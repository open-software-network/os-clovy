//! Geometry and decode bounds ported from the fal edit adapter. Source pixels
//! are never cropped or stretched to fit the provider; padding is removed only
//! using the planned content rectangle after validating the returned geometry.
use super::queue::{FalError, ImageAsset, ImageSize};
use crate::domain::types::AppError;
use base64::Engine as _;
use image::{
    imageops::FilterType, metadata::Orientation, DynamicImage, ImageDecoder, ImageError,
    ImageFormat, ImageReader, Rgba, RgbaImage,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, BufReader, Cursor, Read, Seek, SeekFrom};

const MIN_SIDE: u32 = 512;
const MAX_SIDE: u32 = 2048;
const PIXEL_BUDGET: u64 = 1024 * 1024;
const PADDING: Rgba<u8> = Rgba([128, 128, 128, 255]);
const SOURCE_UNSUPPORTED: &str = "image_edit_source_unsupported";
const SOURCE_TOO_LARGE: &str = "image_edit_source_too_large";

pub(super) struct PreparedEdit {
    plan: EditPlan,
}
impl PreparedEdit {
    pub(super) fn input(&self, prompt: &str, safe_mode: Option<bool>) -> Value {
        let (mime, bytes) = match &self.plan {
            EditPlan::AsIs {
                mime_type, source, ..
            } => (*mime_type, source.trim()),
            EditPlan::Reduced { png_base64, .. } | EditPlan::Canvas { png_base64, .. } => {
                ("image/png", png_base64.as_str())
            }
        };
        let size = self.plan.size();
        let mut value = json!({"prompt": prompt, "image_urls": [format!("data:{mime};base64,{bytes}")], "image_size": {"width": size.width, "height": size.height}, "num_images": 1, "output_format": "png", "sync_mode": true});
        if let Some(safe) = safe_mode {
            value["enable_safety_checker"] = json!(safe);
        }
        value
    }
    pub(super) async fn finish(self, asset: ImageAsset) -> Result<ImageAsset, FalError> {
        let (size, content) = match self.plan {
            EditPlan::AsIs { size, .. } | EditPlan::Reduced { size, .. } => (size, None),
            EditPlan::Canvas { size, content, .. } => (size, Some(content)),
        };
        finish(asset, size, observed_result_size(size, Some(32)), content).await
    }
}

pub(super) async fn prepare_request(source: String, cap: u64) -> Result<PreparedEdit, AppError> {
    let encoded = source.trim();
    if encoded.len() as u64 > cap.div_ceil(3) * 4 {
        return Err(invalid_input(SOURCE_TOO_LARGE));
    }
    // Check the exact decoded size too: cap and cap+1 can have equal encoded length.
    let padding = encoded
        .bytes()
        .rev()
        .take_while(|byte| *byte == b'=')
        .count();
    let decoded = (encoded.len() / 4)
        .saturating_mul(3)
        .saturating_sub(padding);
    if decoded as u64 > cap {
        return Err(invalid_input(SOURCE_TOO_LARGE));
    }
    Ok(PreparedEdit {
        plan: plan_edit(source).await?,
    })
}
fn invalid_input(code: &str) -> AppError {
    AppError::new(
        code,
        match code {
            SOURCE_TOO_LARGE => "The source image is too large to edit.",
            _ => "The source image could not be read. Choose a supported image.",
        },
    )
}

enum EditPlan {
    /// The source, within the pixel budget, goes exactly as received (the
    /// request's own text, untrimmed); the result must have `size`.
    AsIs {
        size: ImageSize,
        mime_type: &'static str,
        source: String,
    },
    /// The whole source, over the pixel budget, was scaled to exactly
    /// `size`; the result must have `size`.
    Reduced { size: ImageSize, png_base64: String },
    /// The source was drawn onto a canvas of exactly `size`; the result must
    /// have `size` and is cropped back to `content`.
    Canvas {
        size: ImageSize,
        content: Rect,
        png_base64: String,
    },
}

impl EditPlan {
    fn size(&self) -> ImageSize {
        match self {
            Self::AsIs { size, .. } | Self::Reduced { size, .. } | Self::Canvas { size, .. } => {
                *size
            }
        }
    }
}

/// The part of the requested output that holds the source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Rect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

/// An image's geometry from its header.
struct HeaderView {
    /// Display size: the stored size with a 90/270-degree orientation applied.
    width: u32,
    height: u32,
    format: ImageFormat,
    orientation: Orientation,
}

impl HeaderView {
    /// Whether the source must be redrawn: too elongated for both size limits,
    /// or stored with an orientation the pixels do not carry yet (fal's EXIF
    /// handling is undocumented, so Clovy sends upright pixels).
    fn needs_canvas(&self) -> bool {
        let long = u64::from(self.width.max(self.height));
        let short = u64::from(self.width.min(self.height));
        long > short * u64::from(MAX_SIDE / MIN_SIDE)
            || self.orientation != Orientation::NoTransforms
    }

    /// Pixels as shown.
    fn pixels(&self) -> u64 {
        u64::from(self.width) * u64::from(self.height)
    }
}

/// Why a header could not be read.
enum HeaderError {
    Unreadable,
    OverLimits,
}

/// Plans one edit on a blocking thread that owns the source text (moved in,
/// never copied whole); see the module's notes on blocking work.
async fn plan_edit(source: String) -> Result<EditPlan, AppError> {
    tokio::task::spawn_blocking(move || prepare(source))
        .await
        .map_err(|error| {
            tracing::error!(
                panicked = error.is_panic(),
                "fal image edit: source preparation did not finish"
            );
            super::upstream_error()
        })?
}

/// Keeps the source when it can go as received; otherwise renders the PNG
/// that goes instead: the whole source at the requested size when it is over
/// the pixel budget, or the canvas.
fn prepare(source: String) -> Result<EditPlan, AppError> {
    let text = source.trim();
    let view = Base64Bytes::new(text)
        .ok_or(HeaderError::Unreadable)
        .and_then(|bytes| read_header(BufReader::new(bytes)))
        .map_err(|error| match error {
            HeaderError::Unreadable => invalid_input(SOURCE_UNSUPPORTED),
            HeaderError::OverLimits => invalid_input(SOURCE_TOO_LARGE),
        })?;
    let size = output_size(view.width, view.height);
    if view.needs_canvas() {
        let content = content_rect(view.width, view.height, size);
        let png_base64 = render_canvas(text, view.orientation, size, content)?;
        return Ok(EditPlan::Canvas {
            size,
            content,
            png_base64,
        });
    }
    if view.pixels() > PIXEL_BUDGET {
        let png_base64 = reduce(text, size)?;
        return Ok(EditPlan::Reduced { size, png_base64 });
    }
    // Normalize locally supported formats whose provider input contract is
    // narrower, without resizing or cropping the source frame.
    let (mime_type, source) = if matches!(
        view.format,
        image::ImageFormat::Bmp | image::ImageFormat::Tiff
    ) {
        ("image/png", png_base64(&decode_source(text)?)?)
    } else {
        (view.format.to_mime_type(), source)
    };
    Ok(EditPlan::AsIs {
        size,
        mime_type,
        source,
    })
}

/// Reads format, stored size and orientation without decoding pixels (a JPEG
/// decoder still reads the whole file first).
fn read_header<R: BufRead + Seek>(image: R) -> Result<HeaderView, HeaderError> {
    let reader = ImageReader::new(image)
        .with_guessed_format()
        .map_err(|_| HeaderError::Unreadable)?;
    let format = reader.format().ok_or(HeaderError::Unreadable)?;
    let mut decoder = reader.into_decoder().map_err(|error| {
        tracing::warn!(
            kind = image_error_kind(&error),
            "fal image edit: image header unreadable"
        );
        if matches!(error, ImageError::Limits(_)) {
            HeaderError::OverLimits
        } else {
            HeaderError::Unreadable
        }
    })?;
    let (stored_width, stored_height) = decoder.dimensions();
    if stored_width == 0 || stored_height == 0 {
        return Err(HeaderError::Unreadable);
    }
    // Unreadable orientation metadata leaves the pixels as stored, which is
    // also what a reader ignoring it would show.
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let (width, height) = if swaps_axes(orientation) {
        (stored_height, stored_width)
    } else {
        (stored_width, stored_height)
    };
    Ok(HeaderView {
        width,
        height,
        format,
        orientation,
    })
}

/// The bytes a standard, padded base64 text encodes, readable and seekable
/// without decoding the whole text up front: each read decodes only the
/// 4-character groups it covers, straight into the caller's buffer. A decoder
/// that reads to the end (JPEG) still ends up holding the whole file.
struct Base64Bytes<'a> {
    text: &'a [u8],
    len: usize,
    position: usize,
}

impl<'a> Base64Bytes<'a> {
    /// `None` unless `text` is a non-empty whole number of 4-character groups.
    fn new(text: &'a str) -> Option<Self> {
        let text = text.as_bytes();
        if text.is_empty() || text.len() % 4 != 0 {
            return None;
        }
        let padding = text
            .iter()
            .rev()
            .take(2)
            .take_while(|byte| **byte == b'=')
            .count();
        Some(Self {
            text,
            len: text.len() / 4 * 3 - padding,
            position: 0,
        })
    }
}

impl Read for Base64Bytes<'_> {
    /// Whole groups decode straight into `buf`; a group read only in part (at
    /// either end of the read, or the padded last one) goes through a
    /// three-byte stack buffer. Nothing is allocated.
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let wanted = buf.len().min(self.len.saturating_sub(self.position));
        let whole_groups = self.len / 3;
        let mut read = 0;
        while read < wanted {
            let position = self.position + read;
            let (group, skip) = (position / 3, position % 3);
            let rest = &mut buf[read..wanted];
            let whole = (rest.len() / 3).min(whole_groups.saturating_sub(group));
            let count = if skip == 0 && whole > 0 {
                decode_into(
                    &self.text[group * 4..(group + whole) * 4],
                    &mut rest[..whole * 3],
                )?
            } else {
                let mut decoded = [0_u8; 3];
                let decoded_len = decode_into(&self.text[group * 4..group * 4 + 4], &mut decoded)?;
                let count = decoded_len.saturating_sub(skip).min(rest.len());
                rest[..count].copy_from_slice(&decoded[skip..skip + count]);
                count
            };
            if count == 0 {
                break;
            }
            read += count;
        }
        self.position += read;
        Ok(read)
    }
}

fn decode_into(text: &[u8], output: &mut [u8]) -> io::Result<usize> {
    base64::engine::general_purpose::STANDARD
        .decode_slice(text, output)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

impl Seek for Base64Bytes<'_> {
    fn seek(&mut self, target: SeekFrom) -> io::Result<u64> {
        let outside = || io::Error::new(io::ErrorKind::InvalidInput, "seek outside the source");
        let position = match target {
            SeekFrom::Start(offset) => i128::from(offset),
            SeekFrom::End(offset) => {
                i128::try_from(self.len).map_err(|_| outside())? + i128::from(offset)
            }
            SeekFrom::Current(offset) => {
                i128::try_from(self.position).map_err(|_| outside())? + i128::from(offset)
            }
        };
        self.position = usize::try_from(position).map_err(|_| outside())?;
        u64::try_from(self.position).map_err(|_| outside())
    }
}

fn swaps_axes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

// Exact integer square root for a value bounded by PIXEL_BUDGET. Avoid the
// newer standard-library isqrt API so the desktop source keeps its MSRV.
fn pixel_budget_square_root(value: u64) -> u64 {
    let mut low = 0;
    let mut high = u64::from(MAX_SIDE) + 1;
    while high - low > 1 {
        let middle = low + (high - low) / 2;
        if middle * middle <= value {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

/// The output size for a source of `width` x `height` (display orientation).
/// With `long:short` the source ratio clamped to `MAX_SIDE / MIN_SIDE`, the
/// short side is `floor(sqrt(budget * short / long))` and the long side that
/// ratio applied to it, to the nearest pixel; when that rounding passes the
/// budget, the short side gives up one pixel. Both sides land in 512..=2048.
fn output_size(width: u32, height: u32) -> ImageSize {
    let max_ratio = u64::from(MAX_SIDE / MIN_SIDE);
    let source_long = u64::from(width.max(height));
    let source_short = u64::from(width.min(height));
    let (long, short) = if source_long > source_short * max_ratio {
        (max_ratio, 1)
    } else {
        (source_long, source_short)
    };
    let mut out_short = pixel_budget_square_root(PIXEL_BUDGET * short / long);
    let mut out_long = nearest_pixel(out_short * long, short);
    while out_short * out_long > PIXEL_BUDGET {
        out_short -= 1;
        out_long = nearest_pixel(out_short * long, short);
    }
    let (out_long, out_short) = (side(out_long), side(out_short));
    if width >= height {
        ImageSize {
            width: out_long,
            height: out_short,
        }
    } else {
        ImageSize {
            width: out_short,
            height: out_long,
        }
    }
}

/// `numerator / denominator` rounded to the nearest whole pixel.
fn nearest_pixel(numerator: u64, denominator: u64) -> u64 {
    (2 * numerator + denominator) / (2 * denominator)
}

/// The centered rectangle a `width` x `height` source fills when scaled,
/// without stretching, to fit inside `size`.
fn content_rect(width: u32, height: u32, size: ImageSize) -> Rect {
    let (source_width, source_height) = (u64::from(width), u64::from(height));
    let (out_width, out_height) = (u64::from(size.width), u64::from(size.height));
    let (fit_width, fit_height) = if source_width * out_height >= source_height * out_width {
        let fit_height = nearest_pixel(source_height * out_width, source_width);
        (out_width, fit_height.clamp(1, out_height))
    } else {
        let fit_width = nearest_pixel(source_width * out_height, source_height);
        (fit_width.clamp(1, out_width), out_height)
    };
    Rect {
        x: side((out_width - fit_width) / 2),
        y: side((out_height - fit_height) / 2),
        width: side(fit_width),
        height: side(fit_height),
    }
}

/// Sides here never exceed `MAX_SIDE`.
fn side(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(MAX_SIDE)
}

/// Decodes the whole source from its base64 text, within the decoder's
/// default limits.
fn decode_source(text: &str) -> Result<DynamicImage, AppError> {
    let bytes = Base64Bytes::new(text).ok_or_else(|| invalid_input(SOURCE_UNSUPPORTED))?;
    ImageReader::new(BufReader::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid_input(SOURCE_UNSUPPORTED))?
        .decode()
        .map_err(|error| {
            tracing::warn!(
                kind = image_error_kind(&error),
                "fal image edit: source image unreadable"
            );
            if matches!(error, ImageError::Limits(_)) {
                invalid_input(SOURCE_TOO_LARGE)
            } else {
                invalid_input(SOURCE_UNSUPPORTED)
            }
        })
}

/// Scales the whole upright source to exactly `size`, returned as base64
/// PNG: nothing is cut or padded.
fn reduce(text: &str, size: ImageSize) -> Result<String, AppError> {
    let reduced = decode_source(text)?.resize_exact(size.width, size.height, FilterType::Lanczos3);
    png_base64(&reduced)
}

/// Decodes the source, turns it upright, scales it into `content` and draws
/// it on a neutral canvas of exactly `size`, returned as base64 PNG.
fn render_canvas(
    text: &str,
    orientation: Orientation,
    size: ImageSize,
    content: Rect,
) -> Result<String, AppError> {
    let mut source = decode_source(text)?;
    source.apply_orientation(orientation);
    let fitted = source
        .resize_exact(content.width, content.height, FilterType::Lanczos3)
        .into_rgba8();
    let mut canvas = RgbaImage::from_pixel(size.width, size.height, PADDING);
    image::imageops::replace(
        &mut canvas,
        &fitted,
        i64::from(content.x),
        i64::from(content.y),
    );
    png_base64(&DynamicImage::ImageRgba8(canvas))
}

/// A prepared source as base64 PNG, encoded once.
fn png_base64(image: &DynamicImage) -> Result<String, AppError> {
    encode_png(image)
        .map(|png| base64::engine::general_purpose::STANDARD.encode(png))
        .map_err(|error| {
            tracing::error!(
                kind = image_error_kind(&error),
                "fal image edit: prepared source could not be encoded"
            );
            super::upstream_error()
        })
}

/// The image error's category, for logs: decoder messages can describe the
/// user's image, so only the kind is recorded.
fn image_error_kind(error: &ImageError) -> &'static str {
    if matches!(error, ImageError::Limits(_)) {
        "limits"
    } else if matches!(error, ImageError::Unsupported(_)) {
        "unsupported"
    } else if matches!(error, ImageError::Decoding(_)) {
        "decoding"
    } else if matches!(error, ImageError::Encoding(_)) {
        "encoding"
    } else if matches!(error, ImageError::IoError(_)) {
        "io"
    } else {
        "other"
    }
}

/// The one result size accepted besides the planned `size`: each side rounded
/// up to the endpoint's observed `output_grid`. `None` when the endpoint has
/// no observed grid or `size` already sits on it: only `size` is accepted.
fn observed_result_size(size: ImageSize, output_grid: Option<u32>) -> Option<ImageSize> {
    let grid = output_grid?;
    let rounded = ImageSize {
        width: size.width.next_multiple_of(grid),
        height: size.height.next_multiple_of(grid),
    };
    (rounded != size).then_some(rounded)
}

/// A result for a source sent whole goes back untouched when its header
/// reports the planned `size`. When it reports the `observed` size instead,
/// its whole frame is scaled back to `size`; any other size is not the edit
/// asked for, and is refused before any pixel is decoded.
fn require_size(
    asset: ImageAsset,
    size: ImageSize,
    observed: Option<ImageSize>,
) -> Result<ImageAsset, FalError> {
    let Ok(view) = read_header(Cursor::new(&asset.bytes)) else {
        return Err(FalError::MediaUnavailable);
    };
    let returned = ImageSize {
        width: view.width,
        height: view.height,
    };
    if returned == size {
        return Ok(asset);
    }
    if Some(returned) != observed {
        return Err(size_mismatch(size, returned));
    }
    redraw(&asset.bytes, size, observed, None)
}

/// Checks the result on a blocking thread that owns its bytes and returns it
/// at exactly `size`; a canvas edit is then cropped back to `content`.
async fn finish(
    asset: ImageAsset,
    size: ImageSize,
    observed: Option<ImageSize>,
    content: Option<Rect>,
) -> Result<ImageAsset, FalError> {
    tokio::task::spawn_blocking(move || match content {
        None => require_size(asset, size, observed),
        Some(content) => redraw(&asset.bytes, size, observed, Some(content)),
    })
    .await
    .map_err(|error| {
        tracing::error!(
            panicked = error.is_panic(),
            "fal image edit: result check did not finish"
        );
        FalError::Upstream { retryable: false }
    })?
}

/// Decodes the result and returns it as PNG at exactly `size`: as it came, or,
/// at the `observed` size, with its whole frame scaled back to `size`, nothing
/// cut. A canvas edit is then cropped to `content`, which comes from the
/// planned geometry, never from a guess about a different one.
fn redraw(
    bytes: &[u8],
    size: ImageSize,
    observed: Option<ImageSize>,
    content: Option<Rect>,
) -> Result<ImageAsset, FalError> {
    let view = read_header(Cursor::new(bytes)).map_err(|_| FalError::MediaUnavailable)?;
    let returned = ImageSize {
        width: view.width,
        height: view.height,
    };
    if returned != size && Some(returned) != observed {
        return Err(size_mismatch(size, returned));
    }
    // Decode pixels only after the header matches the bounded output geometry.
    let mut edited = image::load_from_memory(bytes).map_err(|_| FalError::MediaUnavailable)?;
    edited.apply_orientation(view.orientation);
    let edited = if returned == size {
        edited
    } else if Some(returned) == observed {
        tracing::info!(
            requested_width = size.width,
            requested_height = size.height,
            width = returned.width,
            height = returned.height,
            "fal image edit: output at the observed size, scaled back to the requested size"
        );
        edited.resize_exact(size.width, size.height, FilterType::Lanczos3)
    } else {
        return Err(size_mismatch(size, returned));
    };
    let edited = if let Some(content) = content {
        edited.crop_imm(content.x, content.y, content.width, content.height)
    } else {
        edited
    };
    let png = encode_png(&edited).map_err(|error| {
        tracing::error!(
            kind = image_error_kind(&error),
            "fal image edit: result could not be encoded"
        );
        FalError::Upstream { retryable: false }
    })?;
    Ok(ImageAsset {
        bytes: png,
        mime_type: "image/png",
    })
}

/// Logs a result that has neither the planned nor the observed size.
fn size_mismatch(size: ImageSize, returned: ImageSize) -> FalError {
    tracing::error!(
        requested_width = size.width,
        requested_height = size.height,
        width = returned.width,
        height = returned.height,
        "fal image edit: output size differs from the requested size"
    );
    FalError::MediaUnavailable
}

fn encode_png(image: &DynamicImage) -> Result<Vec<u8>, ImageError> {
    let mut png = Cursor::new(Vec::new());
    image.write_to(&mut png, ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct Geometry {
    pub width: u32,
    pub height: u32,
    pub canvas: bool,
}
#[cfg(test)]
pub(super) fn plan_geometry(
    width: u32,
    height: u32,
    orientation: u8,
) -> Result<Geometry, AppError> {
    if width == 0 || height == 0 || !(1..=8).contains(&orientation) {
        return Err(invalid_input(SOURCE_UNSUPPORTED));
    }
    let (width, height) = if (5..=8).contains(&orientation) {
        (height, width)
    } else {
        (width, height)
    };
    let canvas =
        u64::from(width.max(height)) > u64::from(width.min(height)) * 4 || orientation != 1;
    let size = output_size(width, height);
    Ok(Geometry {
        width: size.width,
        height: size.height,
        canvas,
    })
}
#[cfg(test)]
pub(super) fn validate_result_size(
    geometry: &Geometry,
    width: u32,
    height: u32,
) -> Result<(), AppError> {
    let size = ImageSize {
        width: geometry.width,
        height: geometry.height,
    };
    let returned = ImageSize { width, height };
    if returned == size || Some(returned) == observed_result_size(size, Some(32)) {
        Ok(())
    } else {
        Err(AppError::new(
            "image_media_unavailable",
            "The edited image has an unexpected size.",
        ))
    }
}

#[cfg(test)]
mod decoded_tests {
    use super::*;
    use image::{GenericImageView, Rgb, RgbImage};

    const TOP: Rgb<u8> = Rgb([240, 20, 20]);
    const RIGHT: Rgb<u8> = Rgb([20, 240, 20]);
    const BOTTOM: Rgb<u8> = Rgb([20, 20, 240]);
    const LEFT: Rgb<u8> = Rgb([240, 240, 20]);
    const CENTER: Rgb<u8> = Rgb([180, 80, 180]);

    fn framed(width: u32, height: u32, format: ImageFormat) -> Vec<u8> {
        let band = (width.min(height) / 8).max(1);
        let pixels = RgbImage::from_fn(width, height, |x, y| {
            if y < band {
                TOP
            } else if y >= height - band {
                BOTTOM
            } else if x < band {
                LEFT
            } else if x >= width - band {
                RIGHT
            } else {
                CENTER
            }
        });
        let mut bytes = Cursor::new(Vec::new());
        DynamicImage::ImageRgb8(pixels)
            .write_to(&mut bytes, format)
            .unwrap();
        bytes.into_inner()
    }
    fn encoded(bytes: &[u8]) -> String {
        base64::engine::general_purpose::STANDARD.encode(bytes)
    }
    fn sent(prepared: &PreparedEdit) -> DynamicImage {
        let input = prepared.input("change color", Some(true));
        let uri = input["image_urls"][0].as_str().unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(uri.split_once(',').unwrap().1)
            .unwrap();
        image::load_from_memory(&bytes).unwrap()
    }
    fn frame_is_intact(image: &DynamicImage, colors: [Rgb<u8>; 4]) {
        let (w, h) = image.dimensions();
        for ((x, y), wanted) in [(w / 2, 1), (w - 2, h / 2), (w / 2, h - 2), (1, h / 2)]
            .into_iter()
            .zip(colors)
        {
            assert!(
                image
                    .get_pixel(x, y)
                    .0
                    .into_iter()
                    .zip(wanted.0)
                    .all(|(got, want)| got.abs_diff(want) <= 48),
                "border at {x},{y}"
            );
        }
        assert!(image
            .get_pixel(w / 2, h / 2)
            .0
            .into_iter()
            .zip(CENTER.0)
            .all(|(got, want)| got.abs_diff(want) <= 48));
    }
    fn asset(image: DynamicImage) -> ImageAsset {
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, ImageFormat::Png).unwrap();
        ImageAsset {
            bytes: bytes.into_inner(),
            mime_type: "image/png",
        }
    }

    #[tokio::test]
    async fn bmp_and_tiff_sources_are_sent_as_png_with_the_whole_frame() {
        for format in [ImageFormat::Bmp, ImageFormat::Tiff] {
            let prepared = prepare_request(encoded(&framed(160, 90, format)), 50 * 1024 * 1024)
                .await
                .unwrap();
            assert!(prepared.input("change color", None)["image_urls"][0]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,"));
            assert_eq!(sent(&prepared).dimensions(), (160, 90));
            frame_is_intact(&sent(&prepared), [TOP, RIGHT, BOTTOM, LEFT]);
        }
    }

    #[tokio::test]
    async fn small_source_keeps_its_original_pixels_and_encoding() {
        let bytes = framed(160, 90, ImageFormat::Png);
        let source = encoded(&bytes);
        let prepared = prepare_request(source.clone(), 50 * 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            prepared.input("change color", None)["image_urls"][0],
            format!("data:image/png;base64,{source}")
        );
        assert_eq!(sent(&prepared).dimensions(), (160, 90));
        frame_is_intact(&sent(&prepared), [TOP, RIGHT, BOTTOM, LEFT]);
    }

    #[tokio::test]
    async fn oversized_source_scales_the_whole_frame_without_cropping() {
        let prepared = prepare_request(
            encoded(&framed(1536, 1024, ImageFormat::Png)),
            50 * 1024 * 1024,
        )
        .await
        .unwrap();
        let image = sent(&prepared);
        assert_eq!(image.dimensions(), (1254, 836));
        frame_is_intact(&image, [TOP, RIGHT, BOTTOM, LEFT]);
        let result = prepared.finish(asset(image)).await.unwrap();
        assert_eq!(
            image::load_from_memory(&result.bytes).unwrap().dimensions(),
            (1254, 836)
        );
    }

    #[tokio::test]
    async fn extreme_source_is_padded_then_only_padding_is_removed() {
        let prepared = prepare_request(
            encoded(&framed(8000, 100, ImageFormat::Png)),
            50 * 1024 * 1024,
        )
        .await
        .unwrap();
        let image = sent(&prepared);
        assert_eq!(image.dimensions(), (2048, 512));
        assert_eq!(image.get_pixel(0, 0), PADDING);
        let content = match &prepared.plan {
            EditPlan::Canvas { content, .. } => *content,
            _ => panic!("canvas required"),
        };
        let result = prepared.finish(asset(image)).await.unwrap();
        let image = image::load_from_memory(&result.bytes).unwrap();
        assert_eq!(image.dimensions(), (content.width, content.height));
        // The source aspect is retained within one integer output pixel.
        assert!((f64::from(image.width()) / 80.0 - f64::from(image.height())).abs() <= 1.0);
        frame_is_intact(&image, [TOP, RIGHT, BOTTOM, LEFT]);
    }

    #[tokio::test]
    async fn exif_orientation_is_applied_before_upload_and_output_cropping() {
        let jpeg = framed(640, 480, ImageFormat::Jpeg);
        let exif = [
            0xFF, 0xE1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 0x2A, 0, 0x08, 0, 0,
            0, 0x01, 0, 0x12, 0x01, 0x03, 0, 0x01, 0, 0, 0, 0x06, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut bytes = jpeg[..2].to_vec();
        bytes.extend_from_slice(&exif);
        bytes.extend_from_slice(&jpeg[2..]);
        let prepared = prepare_request(encoded(&bytes), 50 * 1024 * 1024)
            .await
            .unwrap();
        let image = sent(&prepared);
        assert!(image.height() > image.width());
        let result = prepared.finish(asset(image)).await.unwrap();
        frame_is_intact(
            &image::load_from_memory(&result.bytes).unwrap(),
            [LEFT, TOP, RIGHT, BOTTOM],
        );
    }

    #[test]
    fn review_delta_provider_output_applies_rotation_and_mirroring() {
        let jpeg = framed(640, 480, ImageFormat::Jpeg);
        for (orientation, size, edges) in [
            (
                6,
                ImageSize {
                    width: 480,
                    height: 640,
                },
                [LEFT, TOP, RIGHT, BOTTOM],
            ),
            (
                2,
                ImageSize {
                    width: 640,
                    height: 480,
                },
                [TOP, LEFT, BOTTOM, RIGHT],
            ),
        ] {
            let mut exif = [
                0xFF, 0xE1, 0x00, 0x22, b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 0x2A, 0, 0x08, 0,
                0, 0, 0x01, 0, 0x12, 0x01, 0x03, 0, 0x01, 0, 0, 0, 0x06, 0, 0, 0, 0, 0, 0, 0,
            ];
            exif[28] = orientation;
            let mut bytes = jpeg[..2].to_vec();
            bytes.extend_from_slice(&exif);
            bytes.extend_from_slice(&jpeg[2..]);
            let result = redraw(&bytes, size, None, None).unwrap();
            let image = image::load_from_memory(&result.bytes).unwrap();
            assert_eq!(image.dimensions(), (size.width, size.height));
            frame_is_intact(&image, edges);
        }
    }

    #[tokio::test]
    async fn exact_decoded_source_limit_distinguishes_equal_base64_lengths() {
        let mut bytes = framed(16, 16, ImageFormat::Png);
        while bytes.len() % 3 != 1 {
            bytes.push(0);
        }
        let cap = bytes.len() as u64;
        assert!(prepare_request(encoded(&bytes), cap).await.is_ok());
        let mut oversized = bytes;
        oversized.push(0);
        assert_eq!(encoded(&oversized).len(), (cap as usize).div_ceil(3) * 4);
        let result = prepare_request(encoded(&oversized), cap).await;
        assert_eq!(result.err().unwrap().code, SOURCE_TOO_LARGE);
    }

    #[tokio::test]
    async fn only_observed_grid_rounding_can_resize_a_returned_image() {
        let source = encoded(&framed(1536, 1024, ImageFormat::Png));
        let prepared = prepare_request(source.clone(), 50 * 1024 * 1024)
            .await
            .unwrap();
        let output = image::load_from_memory(&framed(1280, 864, ImageFormat::Png)).unwrap();
        let result = prepared.finish(asset(output)).await.unwrap();
        let decoded = image::load_from_memory(&result.bytes).unwrap();
        assert_eq!(decoded.dimensions(), (1254, 836));
        frame_is_intact(&decoded, [TOP, RIGHT, BOTTOM, LEFT]);
        let prepared = prepare_request(source, 50 * 1024 * 1024).await.unwrap();
        let wrong = image::load_from_memory(&framed(1255, 836, ImageFormat::Png)).unwrap();
        assert!(matches!(
            prepared.finish(asset(wrong)).await,
            Err(FalError::MediaUnavailable)
        ));
    }
}
