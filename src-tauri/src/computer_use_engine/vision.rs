//! Local OCR over the exact engine screenshot. Text observations carry image
//! coordinates; OCR does not manufacture editable Accessibility references.
use serde_json::Value;

pub(super) async fn recognize(png: Vec<u8>) -> Result<Vec<Value>, String> {
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        tokio::task::spawn_blocking(move || native(&png))
            .await
            .map_err(|_| "Local text recognition stopped unexpectedly.".to_owned())?
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = png;
        Err("Local text recognition is supported on macOS and Windows.".into())
    }
}

#[cfg(target_os = "macos")]
fn native(png: &[u8]) -> Result<Vec<Value>, String> {
    use objc2::{
        msg_send,
        runtime::{AnyClass, AnyObject},
    };
    use objc2_foundation::{NSArray, NSData, NSDictionary, NSRect};
    use serde_json::json;
    #[link(name = "Vision", kind = "framework")]
    unsafe extern "C" {}
    struct Owned(*mut AnyObject);
    impl Drop for Owned {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    let _: () = msg_send![self.0, release];
                }
            }
        }
    }
    let unavailable = || "Local text recognition is unavailable.".to_owned();
    let handler_class = AnyClass::get(c"VNImageRequestHandler").ok_or_else(unavailable)?;
    let request_class = AnyClass::get(c"VNRecognizeTextRequest").ok_or_else(unavailable)?;
    let data = NSData::with_bytes(png);
    let options = NSDictionary::<AnyObject, AnyObject>::new();
    let handler: *mut AnyObject = unsafe { msg_send![handler_class, alloc] };
    let handler = Owned(unsafe { msg_send![handler,initWithData:&*data,options:&*options] });
    let request: *mut AnyObject = unsafe { msg_send![request_class, alloc] };
    let request = Owned(unsafe { msg_send![request, init] });
    if handler.0.is_null() || request.0.is_null() {
        return Err(unavailable());
    }
    unsafe {
        let _: () = msg_send![request.0,setRecognitionLevel:0isize];
        let _: () = msg_send![request.0,setUsesLanguageCorrection:true];
    }
    let requests = NSArray::from_slice(&[unsafe { &*request.0 }]);
    let mut error: *mut AnyObject = std::ptr::null_mut();
    let success: bool = unsafe { msg_send![handler.0,performRequests:&*requests,error:&mut error] };
    if !success {
        return Err("Local text recognition failed.".into());
    }
    let results: *mut AnyObject = unsafe { msg_send![request.0, results] };
    if results.is_null() {
        return Ok(vec![]);
    }
    let count: usize = unsafe { msg_send![results, count] };
    let mut out = Vec::new();
    for i in 0..count.min(2000) {
        let observation: *mut AnyObject = unsafe { msg_send![results,objectAtIndex:i] };
        if observation.is_null() {
            continue;
        }
        let bounds: NSRect = unsafe { msg_send![observation, boundingBox] };
        let candidates: *mut AnyObject = unsafe { msg_send![observation,topCandidates:1usize] };
        if candidates.is_null() {
            continue;
        }
        let count: usize = unsafe { msg_send![candidates, count] };
        if count == 0 {
            continue;
        }
        let candidate: *mut AnyObject = unsafe { msg_send![candidates,objectAtIndex:0usize] };
        if candidate.is_null() {
            continue;
        }
        let string: *mut AnyObject = unsafe { msg_send![candidate, string] };
        if string.is_null() {
            continue;
        }
        let text: *const i8 = unsafe { msg_send![string, UTF8String] };
        if text.is_null() {
            continue;
        }
        let text = unsafe { std::ffi::CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned();
        out.push(json!({"text":text,"bounds_normalized":{"x":bounds.origin.x,"y":1.0-bounds.origin.y-bounds.size.height,"width":bounds.size.width,"height":bounds.size.height},"source":"apple_vision"}));
    }
    Ok(out)
}

#[cfg(target_os = "windows")]
fn native(png: &[u8]) -> Result<Vec<Value>, String> {
    use serde_json::json;
    use windows::{
        Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap},
        Media::Ocr::OcrEngine,
        Storage::Streams::DataWriter,
        Win32::System::WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
    };
    let fail =
        || "Local text recognition is unavailable for this screenshot or language.".to_owned();
    struct Apartment(bool);
    impl Drop for Apartment {
        fn drop(&mut self) {
            if self.0 {
                unsafe { RoUninitialize() }
            }
        }
    }
    let _apartment = Apartment(unsafe { RoInitialize(RO_INIT_MULTITHREADED) }.is_ok());
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|_| fail())?
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut bgra = image.into_raw();
    for pixel in bgra.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
    let writer = DataWriter::new().map_err(|_| fail())?;
    writer.WriteBytes(&bgra).map_err(|_| fail())?;
    let buffer = writer.DetachBuffer().map_err(|_| fail())?;
    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        i32::try_from(width).map_err(|_| fail())?,
        i32::try_from(height).map_err(|_| fail())?,
    )
    .map_err(|_| fail())?;
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|_| fail())?;
    let max = OcrEngine::MaxImageDimension().map_err(|_| fail())?;
    if width > max || height > max {
        return Err("The screenshot exceeds the local text recognition size limit.".into());
    }
    let operation = engine.RecognizeAsync(&bitmap).map_err(|_| fail())?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        match operation.Status().map_err(|_| fail())? {
            windows_future::AsyncStatus::Completed => break,
            windows_future::AsyncStatus::Error | windows_future::AsyncStatus::Canceled => {
                return Err(fail())
            }
            _ if std::time::Instant::now() >= deadline => {
                let _ = operation.Cancel();
                return Err("Local text recognition timed out.".into());
            }
            _ => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    let result = operation.GetResults().map_err(|_| fail())?;
    let mut out = Vec::new();
    for line in result.Lines().map_err(|_| fail())? {
        for word in line.Words().map_err(|_| fail())? {
            if out.len() >= 2000 {
                break;
            }
            let b = word.BoundingRect().map_err(|_| fail())?;
            out.push(json!({"text":word.Text().map_err(|_|fail())?.to_string(),"bounds_normalized":{"x":b.X as f64/width as f64,"y":b.Y as f64/height as f64,"width":b.Width as f64/width as f64,"height":b.Height as f64/height as f64},"source":"windows_ocr"}));
        }
    }
    let _ = bitmap.Close();
    Ok(out)
}
