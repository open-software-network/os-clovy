use crate::domain::types::AppError;
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Cursor, Read, Write},
    path::Path,
    sync::Mutex,
};
use tauri_plugin_dialog::DialogExt;

const MAX_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8192;
const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
static OPERATIONS: Mutex<()> = Mutex::new(());

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackgroundImage {
    file_name: String,
    data_url: Option<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    file_name: String,
    stored_name: String,
}
fn error() -> AppError {
    AppError::new(
        "background_invalid",
        "Could not use the local background image",
    )
}
fn read_bytes(path: &Path) -> Result<Vec<u8>, AppError> {
    let file = fs::File::open(path).map_err(|_| error())?;
    let metadata = file.metadata().map_err(|_| error())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(error());
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(error());
    }
    Ok(bytes)
}
fn validate(bytes: &[u8]) -> Result<&'static str, AppError> {
    let format = image::guess_format(bytes).map_err(|_| error())?;
    let mime = match format {
        image::ImageFormat::Png => "image/png",
        image::ImageFormat::Jpeg => "image/jpeg",
        image::ImageFormat::WebP => "image/webp",
        _ => return Err(error()),
    };
    // Bound dimensions and decoded allocation before replacing a valid copy.
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DIMENSION);
    limits.max_image_height = Some(MAX_DIMENSION);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), format);
    reader.limits(limits);
    reader.decode().map_err(|_| error())?;
    Ok(mime)
}
fn read_manifest(directory: &Path) -> Result<Option<Manifest>, AppError> {
    let path = directory.join("current.json");
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(error()),
    };
    let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes) else {
        return Ok(None);
    };
    // Stored names are generated UUIDs, never arbitrary paths from metadata.
    if uuid::Uuid::parse_str(&manifest.stored_name).is_err() {
        return Ok(None);
    }
    Ok(Some(manifest))
}
fn read_image(directory: &Path) -> Result<Option<BackgroundImage>, AppError> {
    let Some(manifest) = read_manifest(directory)? else {
        return Ok(None);
    };
    let data_url = read_bytes(&directory.join(&manifest.stored_name))
        .ok()
        .and_then(|bytes| {
            validate(&bytes)
                .ok()
                .map(|mime| format!("data:{mime};base64,{}", STANDARD.encode(&bytes)))
        });
    Ok(Some(BackgroundImage {
        file_name: manifest.file_name,
        data_url,
    }))
}
fn copy_image(directory: &Path, source: &Path) -> Result<(), AppError> {
    let extension = source
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "png" | "jpg" | "jpeg" | "webp") {
        return Err(error());
    }
    let bytes = read_bytes(source)?;
    validate(&bytes)?;
    fs::create_dir_all(directory).map_err(|_| error())?;
    let stored_name = uuid::Uuid::new_v4().to_string();
    let manifest = Manifest {
        file_name: source
            .file_name()
            .ok_or_else(error)?
            .to_string_lossy()
            .into_owned(),
        stored_name,
    };
    let image_path = directory.join(&manifest.stored_name);
    // NamedTempFile keeps the private copy owner-readable before publication.
    let mut image_copy = tempfile::NamedTempFile::new_in(directory).map_err(|_| error())?;
    image_copy.write_all(&bytes).map_err(|_| error())?;
    image_copy.as_file().sync_all().map_err(|_| error())?;
    image_copy
        .persist_noclobber(&image_path)
        .map_err(|_| error())?;
    let commit = (|| {
        let mut temp = tempfile::NamedTempFile::new_in(directory).map_err(|_| error())?;
        temp.write_all(&serde_json::to_vec(&manifest).map_err(|_| error())?)
            .map_err(|_| error())?;
        temp.as_file().sync_all().map_err(|_| error())?;
        temp.persist(directory.join("current.json"))
            .map_err(|_| error())?;
        Ok::<_, AppError>(())
    })();
    if let Err(error) = commit {
        let _ = fs::remove_file(image_path);
        return Err(error);
    }
    // Also recover orphaned copies after a crash or damaged metadata.
    let _ = remove_copies(directory, Some(&manifest.stored_name));
    Ok(())
}
fn remove_copies(directory: &Path, keep: Option<&str>) -> Result<(), AppError> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(error()),
    };
    for entry in entries {
        let entry = entry.map_err(|_| error())?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        // Only this feature's generated UUID copies; never follow metadata paths
        // or remove unrelated files/directories during recovery.
        if Some(name) != keep && uuid::Uuid::parse_str(name).is_ok() {
            fs::remove_file(entry.path()).map_err(|_| error())?;
        }
    }
    Ok(())
}
fn remove_image(directory: &Path) -> Result<(), AppError> {
    remove_copies(directory, None)?;
    match fs::remove_file(directory.join("current.json")) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(error()),
    }
}

#[tauri::command]
pub async fn choose_background_image(
    app: tauri::AppHandle,
) -> Result<Option<BackgroundImage>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .add_filter("PNG, JPEG, WebP", &["png", "jpg", "jpeg", "webp"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        let source = selected.into_path().map_err(|_| error())?;
        let _guard = OPERATIONS.lock().map_err(|_| error())?;
        let directory = crate::app_paths::app_data_dir(&app)
            .map_err(|_| error())?
            .join("backgrounds");
        copy_image(&directory, &source)?;
        read_image(&directory)
    })
    .await
    .map_err(|_| error())?
}
#[tauri::command]
pub async fn read_background_image(
    app: tauri::AppHandle,
) -> Result<Option<BackgroundImage>, AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATIONS.lock().map_err(|_| error())?;
        let directory = crate::app_paths::app_data_dir(&app)
            .map_err(|_| error())?
            .join("backgrounds");
        read_image(&directory)
    })
    .await
    .map_err(|_| error())?
}
#[tauri::command]
pub async fn remove_background_image(app: tauri::AppHandle) -> Result<(), AppError> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = OPERATIONS.lock().map_err(|_| error())?;
        let directory = crate::app_paths::app_data_dir(&app)
            .map_err(|_| error())?
            .join("backgrounds");
        remove_image(&directory)
    })
    .await
    .map_err(|_| error())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Cursor};
    fn sample(path: &Path, format: image::ImageFormat) {
        let image = image::DynamicImage::new_rgb8(8, 8);
        let mut bytes = Cursor::new(Vec::new());
        image.write_to(&mut bytes, format).unwrap();
        fs::write(path, bytes.into_inner()).unwrap();
    }
    #[test]
    fn copy_replace_remove_preserves_original_and_rejects_invalid() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("backgrounds");
        let source = temp.path().join("first.png");
        sample(&source, image::ImageFormat::Png);
        copy_image(&directory, &source).unwrap();
        assert!(source.exists());
        let first = read_image(&directory).unwrap().unwrap();
        assert_eq!(first.file_name, "first.png");
        assert!(first
            .data_url
            .unwrap()
            .starts_with("data:image/png;base64,"));
        for (name, bytes) in [
            ("wrong.svg", b"<svg/>".to_vec()),
            ("broken.png", b"broken".to_vec()),
            ("large.png", vec![0; MAX_BYTES as usize + 1]),
        ] {
            let rejected = temp.path().join(name);
            fs::write(&rejected, bytes).unwrap();
            assert!(copy_image(&directory, &rejected).is_err());
            assert_eq!(
                read_image(&directory).unwrap().unwrap().file_name,
                "first.png"
            );
        }
        assert!(copy_image(&directory, &temp.path().join("unreadable.png")).is_err());
        for (name, format) in [
            ("second.jpg", image::ImageFormat::Jpeg),
            ("third.webp", image::ImageFormat::WebP),
        ] {
            let replacement = temp.path().join(name);
            sample(&replacement, format);
            copy_image(&directory, &replacement).unwrap();
            assert_eq!(read_image(&directory).unwrap().unwrap().file_name, name);
            assert_eq!(
                fs::read_dir(&directory).unwrap().count(),
                2,
                "only manifest and current copy"
            );
        }
        remove_image(&directory).unwrap();
        assert!(read_image(&directory).unwrap().is_none());
        assert!(source.exists());
        remove_image(&directory).unwrap();
    }
    #[test]
    fn excessive_dimensions_preserve_the_previous_image() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("backgrounds");
        let original = temp.path().join("photo.png");
        sample(&original, image::ImageFormat::Png);
        copy_image(&directory, &original).unwrap();
        for (width, height) in [(8193, 1), (1, 8193)] {
            let oversized = temp.path().join("oversized.png");
            image::DynamicImage::new_rgb8(width, height)
                .save(&oversized)
                .unwrap();
            assert!(fs::metadata(&oversized).unwrap().len() < MAX_BYTES);
            assert!(copy_image(&directory, &oversized).is_err());
            assert_eq!(
                read_image(&directory).unwrap().unwrap().file_name,
                "photo.png"
            );
            assert!(read_image(&directory).unwrap().unwrap().data_url.is_some());
        }
    }

    #[test]
    fn missing_or_corrupt_copy_is_plain_but_keeps_name() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("backgrounds");
        let source = temp.path().join("photo.png");
        sample(&source, image::ImageFormat::Png);
        copy_image(&directory, &source).unwrap();
        let manifest = read_manifest(&directory).unwrap().unwrap();
        fs::write(directory.join(&manifest.stored_name), b"corrupt").unwrap();
        assert!(read_image(&directory).unwrap().unwrap().data_url.is_none());
        fs::remove_file(directory.join(&manifest.stored_name)).unwrap();
        let missing = read_image(&directory).unwrap().unwrap();
        assert_eq!(missing.file_name, "photo.png");
        assert!(missing.data_url.is_none());
    }
    #[test]
    fn corrupt_metadata_can_be_replaced_and_removed_without_touching_other_files() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("backgrounds");
        let source = temp.path().join("photo.png");
        sample(&source, image::ImageFormat::Png);
        copy_image(&directory, &source).unwrap();
        fs::write(directory.join("keep.txt"), b"unrelated").unwrap();
        fs::write(directory.join("current.json"), b"broken JSON").unwrap();
        copy_image(&directory, &source).unwrap();
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 3);
        fs::write(directory.join("current.json"), b"broken again").unwrap();
        remove_image(&directory).unwrap();
        assert!(read_image(&directory).unwrap().is_none());
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        assert_eq!(fs::read(directory.join("keep.txt")).unwrap(), b"unrelated");
        assert!(source.exists());
    }
}
