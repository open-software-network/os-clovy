//! Host-owned browser image and namespace identity.
use std::{
    io,
    path::{Path, PathBuf},
};

/// Paths are supplied by the host's browser configuration, never by a tool call.
/// Resolution is deferred so absent assets do not prevent application startup.
pub(crate) struct OwnedBrowserRuntime {
    namespace: PathBuf,
    image: PathBuf,
}

impl OwnedBrowserRuntime {
    pub(crate) fn new(namespace: PathBuf, image: PathBuf) -> Self {
        Self { namespace, image }
    }

    pub(crate) fn paths(&self) -> (&Path, &Path) {
        (&self.namespace, &self.image)
    }

    /// Returns whether an executable belongs to the private runtime. An
    /// indeterminate identity is an error, never evidence of an external app.
    pub(crate) fn owns(&self, candidate: &Path) -> io::Result<bool> {
        if !self.namespace.is_absolute() || !self.image.is_absolute() || !candidate.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Absolute paths required",
            ));
        }
        let namespace = std::fs::canonicalize(&self.namespace)?;
        let image = std::fs::canonicalize(&self.image)?;
        let candidate = std::fs::canonicalize(candidate)?;
        let namespace_info = file_info(&namespace)?;
        let image_info = file_info(&image)?;
        let candidate_info = file_info(&candidate)?;
        if !namespace_info.is_dir || !image_info.is_file || !candidate_info.is_file {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid runtime identity",
            ));
        }
        let namespace_id = namespace_info.identity;
        if !has_ancestor_identity(&image, namespace_id)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Image outside runtime namespace",
            ));
        }
        Ok(candidate_info.identity == image_info.identity
            || has_ancestor_identity(&candidate, namespace_id)?)
    }
}

fn has_ancestor_identity(path: &Path, identity: FileIdentity) -> io::Result<bool> {
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Missing parent"))?;
    for directory in parent.ancestors() {
        if file_info(directory)?.identity == identity {
            return Ok(true);
        }
    }
    Ok(false)
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileIdentity(u64, u64);

struct FileInfo {
    identity: FileIdentity,
    is_file: bool,
    is_dir: bool,
}

#[cfg(unix)]
fn file_info(path: &Path) -> io::Result<FileInfo> {
    use std::os::unix::fs::MetadataExt;
    let metadata = std::fs::metadata(path)?;
    Ok(FileInfo {
        identity: FileIdentity(metadata.dev(), metadata.ino()),
        is_file: metadata.is_file(),
        is_dir: metadata.is_dir(),
    })
}

#[cfg(windows)]
fn file_info(path: &Path) -> io::Result<FileInfo> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{
            GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_DEVICE,
            FILE_ATTRIBUTE_DIRECTORY, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
    };

    let file = std::fs::OpenOptions::new()
        .access_mode(0)
        .share_mode((FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE).0)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0)
        .open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }
        .map_err(io::Error::other)?;
    let is_dir = info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0;
    let is_device = info.dwFileAttributes & FILE_ATTRIBUTE_DEVICE.0 != 0;
    Ok(FileInfo {
        identity: FileIdentity(
            info.dwVolumeSerialNumber as u64,
            ((info.nFileIndexHigh as u64) << 32) | info.nFileIndexLow as u64,
        ),
        is_file: !is_dir && !is_device,
        is_dir,
    })
}

#[cfg(test)]
mod owned_browser_runtime_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "clovy-browser-policy-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }

    #[test]
    fn private_namespace_and_image_identity_are_not_external_apps() {
        let fixture = Fixture::new();
        let namespace = fixture.0.join("chrome");
        let sibling = fixture.0.join("chrome-other");
        std::fs::create_dir(&namespace).unwrap();
        std::fs::create_dir(&sibling).unwrap();
        let image = namespace.join("browser");
        let other_private = namespace.join("helper");
        let external = sibling.join("browser");
        let hardlink = fixture.0.join("browser-alias");
        for file in [&image, &other_private, &external] {
            std::fs::write(file, b"fixture").unwrap();
        }
        std::fs::hard_link(&image, &hardlink).unwrap();
        let policy = OwnedBrowserRuntime::new(namespace, image.clone());
        assert!(policy.owns(&image).unwrap());
        assert!(policy.owns(&other_private).unwrap());
        assert!(policy.owns(&hardlink).unwrap());
        assert!(!policy.owns(&external).unwrap());
        #[cfg(unix)]
        {
            let alias = fixture.0.join("symlink");
            std::os::unix::fs::symlink(&image, &alias).unwrap();
            assert!(policy.owns(&alias).unwrap());
            let directory_alias = fixture.0.join("directory-alias");
            std::os::unix::fs::symlink(&policy.namespace, &directory_alias).unwrap();
            assert!(policy.owns(&directory_alias.join("helper")).unwrap());
        }
        assert!(policy.owns(Path::new("relative")).is_err());
        assert!(policy.owns(&fixture.0.join("missing")).is_err());
        // Reclassification observes a changed file identity instead of
        // carrying an earlier decision across an intervening wait.
        std::fs::remove_file(&external).unwrap();
        std::fs::hard_link(&image, &external).unwrap();
        assert!(policy.owns(&external).unwrap());
        std::fs::remove_file(&image).unwrap();
        assert!(policy.owns(&external).is_err());
    }

    #[test]
    fn invalid_configuration_never_classifies_an_external_file() {
        let fixture = Fixture::new();
        let namespace = fixture.0.join("namespace");
        std::fs::create_dir(&namespace).unwrap();
        let external = fixture.0.join("external");
        std::fs::write(&external, b"fixture").unwrap();
        let missing = OwnedBrowserRuntime::new(namespace.clone(), namespace.join("missing"));
        assert!(missing.owns(&external).is_err());
        let outside = OwnedBrowserRuntime::new(namespace, external.clone());
        assert!(outside.owns(&external).is_err());
    }
}
