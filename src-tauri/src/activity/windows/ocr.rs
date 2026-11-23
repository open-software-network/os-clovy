//! In-memory window screenshot capture via Windows Graphics Capture (WGC)
//! and local on-device OCR using Windows.Media.Ocr.

use std::sync::mpsc::sync_channel;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows::core::{factory, Interface, Result, RuntimeType};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
#[cfg(test)]
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Graphics::Imaging::{BitmapBufferAccessMode, BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::{OcrEngine, OcrResult};
#[cfg(test)]
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::System::WinRT::Direct3D11::CreateDirect3D11DeviceFromDXGIDevice;
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{
    IMemoryBufferByteAccess, RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED,
};

use crate::activity::platform::WindowDescriptor;

const GLOBAL_CAPTURE_TIMEOUT: Duration = Duration::from_secs(2);

/// RAII guard to balance RoInitialize and RoUninitialize on the capture thread.
pub(crate) struct RoInitGuard {
    needs_uninit: bool,
}

impl RoInitGuard {
    pub(crate) fn new() -> Self {
        let res = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        Self {
            needs_uninit: res.is_ok(),
        }
    }
}

impl Drop for RoInitGuard {
    fn drop(&mut self) {
        if self.needs_uninit {
            unsafe { RoUninitialize() };
        }
    }
}

/// Holds the in-flight frame and pool lease until the surface copy reaches a terminal state.
struct SurfaceCopyLease {
    frame: Option<Direct3D11CaptureFrame>,
    pool: Option<Direct3D11CaptureFramePool>,
}

impl Drop for SurfaceCopyLease {
    fn drop(&mut self) {
        if let Some(frame) = self.frame.take() {
            let _ = frame.Close();
        }
        if let Some(pool) = self.pool.take() {
            let _ = pool.Close();
        }
    }
}

struct CaptureSessionGuard(GraphicsCaptureSession);

impl Drop for CaptureSessionGuard {
    fn drop(&mut self) {
        let _ = self.0.Close();
    }
}

/// Owns pixels until the native operation is terminal, including after cancellation.
struct PendingImageOperation<T: RuntimeType + 'static, R: Send + 'static> {
    operation: windows_future::IAsyncOperation<T>,
    resource: Option<R>,
}

impl<T: RuntimeType + 'static, R: Send + 'static> PendingImageOperation<T, R> {
    fn new(operation: windows_future::IAsyncOperation<T>, resource: R) -> Self {
        Self {
            operation,
            resource: Some(resource),
        }
    }

    fn is_terminal(&self) -> bool {
        matches!(
            self.operation.Status(),
            Ok(windows_future::AsyncStatus::Completed
                | windows_future::AsyncStatus::Canceled
                | windows_future::AsyncStatus::Error)
        )
    }
}

impl<T: RuntimeType + 'static, R: Send + 'static> Drop for PendingImageOperation<T, R> {
    fn drop(&mut self) {
        if self.is_terminal() {
            return;
        }
        let _ = self.operation.Cancel();
        if self.is_terminal() {
            return;
        }

        // Normally reaped by the next capture tick. At adapter shutdown the
        // native completion must own its pixels rather than close a live frame.
        let resource = Arc::new(Mutex::new(self.resource.take()));
        let completion_resource = Arc::clone(&resource);
        let handler = windows_future::AsyncOperationCompletedHandler::<T>::new(move |_, _| {
            let _released = completion_resource
                .lock()
                .ok()
                .and_then(|mut resource| resource.take());
            Ok(())
        });
        if self.operation.SetCompleted(&handler).is_err() {
            // No safe owner remains if Windows rejects completion registration
            // during shutdown. Quarantine this single lease until process exit;
            // releasing an actively copied GPU surface would be unsafe.
            std::mem::forget(resource);
        }
    }
}

enum PendingCapture {
    Copy(PendingImageOperation<SoftwareBitmap, SurfaceCopyLease>),
    Recognition(PendingImageOperation<OcrResult, (SoftwareBitmap, OcrEngine)>),
}

impl PendingCapture {
    fn is_terminal(&self) -> bool {
        match self {
            Self::Copy(operation) => operation.is_terminal(),
            Self::Recognition(operation) => operation.is_terminal(),
        }
    }
}

pub(crate) struct OcrCapture {
    pending: Option<PendingCapture>,
    device: Option<IDirect3DDevice>,
    engine: Option<OcrEngine>,
    _runtime: RoInitGuard,
}

impl OcrCapture {
    pub(crate) fn new() -> Self {
        Self {
            pending: None,
            device: None,
            engine: None,
            _runtime: RoInitGuard::new(),
        }
    }

    /// Called even on disabled ticks so canceled images do not await another OCR.
    pub(crate) fn reap_completed(&mut self) {
        if self
            .pending
            .as_ref()
            .is_some_and(PendingCapture::is_terminal)
        {
            drop(self.pending.take());
        }
    }

    pub(crate) fn window_text(&mut self, window: &WindowDescriptor) -> Option<String> {
        self.reap_completed();
        if self.pending.is_some() {
            return None;
        }
        let deadline = Instant::now() + GLOBAL_CAPTURE_TIMEOUT;
        let hwnd = HWND((window.window_id? as i32 as isize) as *mut _);
        let bitmap = self.capture_window_bitmap(hwnd, deadline)?;
        self.recognize_bitmap(bitmap, deadline)
    }
}

fn await_operation<T: RuntimeType + 'static>(
    operation: &windows_future::IAsyncOperation<T>,
    deadline: Instant,
) -> Option<T> {
    loop {
        let status = match operation.Status() {
            Ok(status) => status,
            Err(_) => {
                let _ = operation.Cancel();
                return None;
            }
        };
        match status {
            windows_future::AsyncStatus::Completed => return operation.GetResults().ok(),
            windows_future::AsyncStatus::Started => {
                let Some(remaining) = remaining_time(deadline) else {
                    let _ = operation.Cancel();
                    return None;
                };
                std::thread::sleep(Duration::from_millis(2).min(remaining));
            }
            _ => return None,
        }
    }
}

fn remaining_time(deadline: Instant) -> Option<Duration> {
    let now = Instant::now();
    if now >= deadline {
        None
    } else {
        Some(deadline - now)
    }
}

fn create_d3d_device() -> Result<IDirect3DDevice> {
    let mut d3d_device: Option<ID3D11Device> = None;
    unsafe {
        D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut d3d_device),
            None,
            None,
        )?;
    }
    let d3d_device = d3d_device.ok_or_else(windows::core::Error::from_win32)?;
    let dxgi_device: IDXGIDevice = d3d_device.cast()?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device)? };
    inspectable.cast::<IDirect3DDevice>()
}

fn get_ocr_engine() -> Result<OcrEngine> {
    OcrEngine::TryCreateFromUserProfileLanguages()
}

/// Calculates proportional scaled dimensions when dimensions exceed `max_dim`.
/// Preserves the entire aspect ratio so that far-right and bottom content is retained.
pub(crate) fn compute_scaled_dimensions(
    width: u32,
    height: u32,
    max_dim: u32,
) -> Option<(u32, u32)> {
    if width == 0 || height == 0 || max_dim == 0 {
        return None;
    }
    if width <= max_dim && height <= max_dim {
        return None;
    }
    let scale = (max_dim as f64) / ((width.max(height)) as f64);
    let scaled_w = ((width as f64 * scale).round() as u32).max(1);
    let scaled_h = ((height as f64 * scale).round() as u32).max(1);
    Some((scaled_w, scaled_h))
}

/// Resizes BGRA pixels in RAM without intermediate image encoding or disk I/O.
pub(crate) fn resize_software_bitmap(
    bitmap: &SoftwareBitmap,
    scaled_w: u32,
    scaled_h: u32,
) -> Option<SoftwareBitmap> {
    if scaled_w == 0 || scaled_h == 0 {
        return None;
    }

    if bitmap.BitmapPixelFormat().ok()? != BitmapPixelFormat::Bgra8 {
        return None;
    }
    let src_w = usize::try_from(bitmap.PixelWidth().ok()?).ok()?;
    let src_h = usize::try_from(bitmap.PixelHeight().ok()?).ok()?;
    if src_w == 0 || src_h == 0 {
        return None;
    }

    let src_buf = bitmap.LockBuffer(BitmapBufferAccessMode::Read).ok()?;
    let src_plane = src_buf.GetPlaneDescription(0).ok()?;
    let src_ref = src_buf.CreateReference().ok()?;
    let src_access: IMemoryBufferByteAccess = src_ref.cast().ok()?;
    let mut src_ptr: *mut u8 = std::ptr::null_mut();
    let mut src_capacity: u32 = 0;
    unsafe {
        src_access.GetBuffer(&mut src_ptr, &mut src_capacity).ok()?;
    }

    if src_ptr.is_null() {
        return None;
    }

    let dest_bitmap = SoftwareBitmap::Create(
        BitmapPixelFormat::Bgra8,
        i32::try_from(scaled_w).ok()?,
        i32::try_from(scaled_h).ok()?,
    )
    .ok()?;
    let dest_buf = dest_bitmap.LockBuffer(BitmapBufferAccessMode::Write).ok()?;
    let dest_plane = dest_buf.GetPlaneDescription(0).ok()?;
    let dest_ref = dest_buf.CreateReference().ok()?;
    let dest_access: IMemoryBufferByteAccess = dest_ref.cast().ok()?;
    let mut dest_ptr: *mut u8 = std::ptr::null_mut();
    let mut dest_capacity: u32 = 0;
    unsafe {
        dest_access
            .GetBuffer(&mut dest_ptr, &mut dest_capacity)
            .ok()?;
    }

    if dest_ptr.is_null() {
        return None;
    }

    let src_stride = usize::try_from(src_plane.Stride).ok()?;
    let src_start = usize::try_from(src_plane.StartIndex).ok()?;
    let dest_stride = usize::try_from(dest_plane.Stride).ok()?;
    let dest_start = usize::try_from(dest_plane.StartIndex).ok()?;
    let dw = scaled_w as usize;
    let dh = scaled_h as usize;
    let src_row_bytes = src_w.checked_mul(4)?;
    let dest_row_bytes = dw.checked_mul(4)?;
    let src_end = src_start
        .checked_add((src_h - 1).checked_mul(src_stride)?)?
        .checked_add(src_row_bytes)?;
    let dest_end = dest_start
        .checked_add((dh - 1).checked_mul(dest_stride)?)?
        .checked_add(dest_row_bytes)?;
    if src_stride < src_row_bytes
        || dest_stride < dest_row_bytes
        || src_end > src_capacity as usize
        || dest_end > dest_capacity as usize
    {
        return None;
    }

    // Endpoint-aligned nearest neighbors preserve both edges. Bounds were
    // validated once above rather than branching for every pixel.
    for dy in 0..dh {
        let sy = if dh > 1 {
            dy * (src_h - 1) / (dh - 1)
        } else {
            src_h / 2
        };
        let src_row = src_start + sy * src_stride;
        let dest_row = dest_start + dy * dest_stride;
        for dx in 0..dw {
            let sx = if dw > 1 {
                dx * (src_w - 1) / (dw - 1)
            } else {
                src_w / 2
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    src_ptr.add(src_row + sx * 4),
                    dest_ptr.add(dest_row + dx * 4),
                    4,
                );
            }
        }
    }

    Some(dest_bitmap)
}

/// Captures at native size; only the CPU bitmap is downscaled for OCR.
impl OcrCapture {
    fn capture_window_bitmap(&mut self, hwnd: HWND, deadline: Instant) -> Option<SoftwareBitmap> {
        remaining_time(deadline)?;
        if self.device.is_none() {
            self.device = Some(create_d3d_device().ok()?);
        }
        let interop: IGraphicsCaptureItemInterop =
            factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ok()?;
        let item: GraphicsCaptureItem = unsafe { interop.CreateForWindow(hwnd).ok()? };
        let native_size = item.Size().ok()?;
        if native_size.Width <= 0 || native_size.Height <= 0 {
            return None;
        }

        let mut lease = SurfaceCopyLease {
            frame: None,
            pool: Some(
                Direct3D11CaptureFramePool::CreateFreeThreaded(
                    self.device.as_ref()?,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    1,
                    native_size,
                )
                .ok()?,
            ),
        };
        let pool = lease.pool.as_ref()?;
        let session = CaptureSessionGuard(pool.CreateCaptureSession(&item).ok()?);
        let _ = session.0.SetIsBorderRequired(false);
        let _ = session.0.SetIsCursorCaptureEnabled(false);

        let (frame_tx, frame_rx) = sync_channel(1);
        let token = pool
            .FrameArrived(&TypedEventHandler::new(move |_, _| {
                // Never block the WGC callback if another notification is queued.
                let _ = frame_tx.try_send(());
                Ok(())
            }))
            .ok()?;
        session.0.StartCapture().ok()?;
        frame_rx.recv_timeout(remaining_time(deadline)?).ok()?;
        lease.frame = Some(pool.TryGetNextFrame().ok()?);
        let _ = pool.RemoveFrameArrived(token);
        drop(session);

        let surface = lease.frame.as_ref()?.Surface().ok()?;
        let copy = PendingImageOperation::new(
            SoftwareBitmap::CreateCopyFromSurfaceAsync(&surface).ok()?,
            lease,
        );
        let bitmap = await_operation(&copy.operation, deadline);
        let Some(bitmap) = bitmap else {
            if !copy.is_terminal() {
                self.pending = Some(PendingCapture::Copy(copy));
            }
            return None;
        };
        drop(copy);
        remaining_time(deadline)?;

        let max_dim = OcrEngine::MaxImageDimension().ok()?;
        let width = u32::try_from(bitmap.PixelWidth().ok()?).ok()?;
        let height = u32::try_from(bitmap.PixelHeight().ok()?).ok()?;
        match compute_scaled_dimensions(width, height, max_dim) {
            Some((width, height)) => resize_software_bitmap(&bitmap, width, height),
            None => Some(bitmap),
        }
    }
}

impl OcrCapture {
    fn recognize_bitmap(&mut self, bitmap: SoftwareBitmap, deadline: Instant) -> Option<String> {
        remaining_time(deadline)?;
        if self.engine.is_none() {
            self.engine = Some(get_ocr_engine().ok()?);
        }
        let engine = self.engine.as_ref()?;
        let recognition = PendingImageOperation::new(
            engine.RecognizeAsync(&bitmap).ok()?,
            (bitmap, engine.clone()),
        );
        let result = await_operation(&recognition.operation, deadline);
        let Some(result) = result else {
            if !recognition.is_terminal() {
                self.pending = Some(PendingCapture::Recognition(recognition));
            }
            return None;
        };
        let text = result.Text().ok()?.to_string();
        (!text.trim().is_empty()).then_some(text)
    }
}

/// Exercises the same local recognizer over a deterministic, in-memory fixture.
#[cfg(test)]
pub(crate) fn recognize_image_bytes(bytes: &[u8]) -> Option<String> {
    let deadline = Instant::now() + GLOBAL_CAPTURE_TIMEOUT;
    let mut capture = OcrCapture::new();
    let stream = InMemoryRandomAccessStream::new().ok()?;
    let writer = DataWriter::CreateDataWriter(&stream).ok()?;
    writer.WriteBytes(bytes).ok()?;
    await_operation(&writer.StoreAsync().ok()?, deadline)?;
    writer.DetachStream().ok()?;
    stream.Seek(0).ok()?;
    let decoder = await_operation(&BitmapDecoder::CreateAsync(&stream).ok()?, deadline)?;
    let bitmap = await_operation(&decoder.GetSoftwareBitmapAsync().ok()?, deadline)?;
    capture.recognize_bitmap(bitmap, deadline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_windows_ocr_fixture_recognition() {
        let fixture_bytes = include_bytes!("fixtures/ocr-fixture.png");
        let recognized = recognize_image_bytes(fixture_bytes);
        assert!(
            recognized.is_some(),
            "Windows.Media.Ocr must recognize the test fixture image"
        );
        let text = recognized.unwrap();
        assert!(
            text.contains("CLOVY") && text.contains("WINDOWS") && text.contains("ACTIVITY"),
            "Recognized text must contain 'CLOVY WINDOWS ACTIVITY', got: {:?}",
            text
        );
    }

    #[test]
    fn test_resize_software_bitmap_preserves_far_right_content() {
        let _runtime = RoInitGuard::new();
        // Create an actual source SoftwareBitmap: 4000 x 100
        let src_w = 4000;
        let src_h = 100;
        let bitmap = SoftwareBitmap::Create(
            windows::Graphics::Imaging::BitmapPixelFormat::Bgra8,
            src_w,
            src_h,
        )
        .expect("create source bitmap");

        // A visible red region at the right edge, not a one-pixel scanline
        // that a valid downsampling grid may skip.
        {
            let buffer = bitmap
                .LockBuffer(BitmapBufferAccessMode::Write)
                .expect("lock write");
            let plane = buffer.GetPlaneDescription(0).expect("plane");
            let reference = buffer.CreateReference().expect("ref");
            let byte_access: IMemoryBufferByteAccess = reference.cast().expect("byte_access");
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut capacity: u32 = 0;
            unsafe {
                byte_access
                    .GetBuffer(&mut ptr, &mut capacity)
                    .expect("get buffer");
            }

            let stride = plane.Stride as usize;
            let start = plane.StartIndex as usize;

            for y in 0..(src_h as usize) {
                for x in 0..(src_w as usize) {
                    let off = start + y * stride + x * 4;
                    if off + 4 <= capacity as usize {
                        unsafe {
                            *ptr.add(off) = 0; // B
                            *ptr.add(off + 1) = 0; // G
                            *ptr.add(off + 2) = 0; // R
                            *ptr.add(off + 3) = 255;
                        }
                    }
                }
            }

            // Distinct red marker on far-right pixels
            for y in 40..60 {
                for x in 3990..(src_w as usize) {
                    let off = start + y * stride + x * 4;
                    if off + 4 <= capacity as usize {
                        unsafe {
                            *ptr.add(off) = 0;
                            *ptr.add(off + 1) = 0;
                            *ptr.add(off + 2) = 255;
                            *ptr.add(off + 3) = 255;
                        }
                    }
                }
            }
        }

        // Call the REAL resize_software_bitmap with scaled dimensions (2600 x 65)
        let scaled_w = 2600;
        let scaled_h = 65;
        let resized = resize_software_bitmap(&bitmap, scaled_w, scaled_h)
            .expect("resize_software_bitmap must succeed");

        assert_eq!(resized.PixelWidth().unwrap(), 2600);
        assert_eq!(resized.PixelHeight().unwrap(), 65);

        // Read result pixels from the actual resized bitmap
        let res_buf = resized
            .LockBuffer(BitmapBufferAccessMode::Read)
            .expect("lock read");
        let res_plane = res_buf.GetPlaneDescription(0).expect("plane");
        let res_ref = res_buf.CreateReference().expect("ref");
        let res_access: IMemoryBufferByteAccess = res_ref.cast().expect("byte_access");
        let mut res_ptr: *mut u8 = std::ptr::null_mut();
        let mut res_cap: u32 = 0;
        unsafe {
            res_access
                .GetBuffer(&mut res_ptr, &mut res_cap)
                .expect("get buffer");
        }

        let res_stride = res_plane.Stride as usize;
        let res_start = res_plane.StartIndex as usize;

        // The far-right pixel of the resized image (x = 2599, y = 32) must preserve the red marker
        let far_right_off = res_start + 32 * res_stride + 2599 * 4;
        assert!(far_right_off + 4 <= res_cap as usize);
        let r = unsafe { *res_ptr.add(far_right_off + 2) };
        assert_eq!(
            r, 255,
            "Far-right pixel of resized image must preserve the distinct marker, proving content was not clipped"
        );
    }
}
