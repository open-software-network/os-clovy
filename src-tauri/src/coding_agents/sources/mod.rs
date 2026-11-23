//! Read-only readers for the seven agents' local stores.
//!
//! Boundaries:
//! - Every source has a configured location (its **root**). The root itself
//!   may be a symlink (dotfiles setups); it is resolved once, and nothing
//!   below it may be: directory listings skip symlinked entries, candidates
//!   must be regular files (and so must a SQLite WAL next to them), and right
//!   before reading, the candidate is re-checked to still be a regular file
//!   whose canonical path lies under the canonical root. Files are opened
//!   with `O_NOFOLLOW`. Content outside a source's root is never read.
//! - Files are opened for reading only; nothing under an agent's directories
//!   is created, renamed, or touched (SQLite stores are read from a copy,
//!   `sqlite.rs`).
//! - Readers stream: JSONL is read line by line (lines over
//!   [`MAX_LINE_BYTES`] are skipped unparsed) and each normalized record goes
//!   straight into the session's [`BlockBuilder`], which keeps memory bounded.
//!
//! Discovery is cheap (directory listing and metadata); a candidate is loaded
//! only when its fingerprint (size and mtime of the file and its WAL) changed
//! since the last scan.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod copilot_cli;
pub mod copilot_vscode;
pub mod cursor;
pub mod cursor_cli;
pub mod sqlite;

use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::record::{RecordSink, SessionInfo};
use super::segment::BlockBuilder;
use super::SourceId;

/// JSONL lines longer than this (a dumped file inside one record) are
/// skipped without being kept in memory or parsed.
pub const MAX_LINE_BYTES: usize = 4 * 1024 * 1024;

/// Where the agents keep their data. Production uses the user's home;
/// tests point it at a temporary directory.
#[derive(Clone, Debug)]
pub struct SourceRoots {
    pub home: PathBuf,
    app_support_dir: Option<PathBuf>,
}

impl SourceRoots {
    pub fn from_home(home: impl Into<PathBuf>) -> Self {
        Self {
            home: home.into(),
            app_support_dir: None,
        }
    }

    pub fn from_windows(profile: impl Into<PathBuf>, appdata: impl Into<PathBuf>) -> Self {
        Self {
            home: profile.into(),
            app_support_dir: Some(appdata.into()),
        }
    }

    pub fn current() -> Option<Self> {
        let home = std::env::var_os("HOME")
            .filter(|home| !home.is_empty())
            .map(PathBuf::from)
            .or_else(dirs::home_dir)
            .or_else(|| {
                std::env::var_os("USERPROFILE")
                    .filter(|p| !p.is_empty())
                    .map(PathBuf::from)
            })?;
        let app_support_dir = if cfg!(target_os = "macos") {
            home.join("Library").join("Application Support")
        } else {
            dirs::config_dir()
                .or_else(|| {
                    std::env::var_os("APPDATA")
                        .filter(|appdata| !appdata.is_empty())
                        .map(PathBuf::from)
                })
                .unwrap_or_else(|| home.join("AppData").join("Roaming"))
        };
        Some(Self {
            home,
            app_support_dir: Some(app_support_dir),
        })
    }

    pub fn app_support(&self, app: &str) -> PathBuf {
        if let Some(dir) = &self.app_support_dir {
            dir.join(app)
        } else {
            self.home
                .join("Library")
                .join("Application Support")
                .join(app)
        }
    }

    /// The directory whose existence means the agent was used on this Mac.
    pub fn presence_path(&self, source: SourceId) -> PathBuf {
        match source {
            SourceId::ClaudeCode => claude::root(self),
            SourceId::Codex => codex::root(self),
            SourceId::CopilotCli => copilot_cli::root(self),
            SourceId::CopilotVscode => copilot_vscode::user_dir(self),
            SourceId::Cursor => cursor::root(self),
            SourceId::CursorCli => cursor_cli::root(self),
            SourceId::Antigravity => antigravity::cli_root(self),
        }
    }

    pub fn is_present(&self, source: SourceId) -> bool {
        let present = self.presence_path(source).exists();
        if source == SourceId::Antigravity {
            return present || antigravity::ide_root(self).exists();
        }
        present
    }
}

/// Size and mtime of a store file plus its SQLite WAL, if any.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fingerprint(Vec<(u64, Option<SystemTime>)>);

/// One store entry that may hold new turns.
#[derive(Clone, Debug)]
pub struct Candidate {
    pub source: SourceId,
    /// Canonical root of the source the entry was found under.
    pub root: PathBuf,
    pub path: PathBuf,
    pub modified: SystemTime,
    pub fingerprint: Fingerprint,
}

/// The canonical directory of a configured source location, if it exists.
pub fn source_root(location: &Path) -> Option<PathBuf> {
    fs::canonicalize(location).ok().filter(|root| root.is_dir())
}

/// Metadata of `path` when it is a regular file and not a symlink.
fn regular_file(path: &Path) -> Option<fs::Metadata> {
    fs::symlink_metadata(path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
}

pub fn is_regular_file(path: &Path) -> bool {
    regular_file(path).is_some()
}

/// Entries of `dir` that are not symlinks, sorted; empty when unreadable.
pub fn list_dir(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
                .map(|entry| entry.path())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    entries
}

/// Real subdirectories of `dir` (symlinks excluded).
pub fn list_subdirs(dir: &Path) -> Vec<PathBuf> {
    list_dir(dir)
        .into_iter()
        .filter(|entry| is_real_dir(entry))
        .collect()
}

/// A directory that is not a symlink.
pub fn is_real_dir(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
}

/// The canonical path of `path` when it is a regular file under `root`.
pub fn inside_root(root: &Path, path: &Path) -> Option<PathBuf> {
    regular_file(path)?;
    fs::canonicalize(path)
        .ok()
        .filter(|canonical| canonical.starts_with(root))
}

/// Opens a file for reading without following a symlink in its last
/// component.
pub fn open_regular(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(path)?;
    #[cfg(windows)]
    {
        use std::mem::size_of;
        use std::os::windows::io::AsRawHandle;
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::Storage::FileSystem::{
            FileAttributeTagInfo, GetFileInformationByHandleEx, FILE_ATTRIBUTE_DIRECTORY,
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
        };

        let handle = HANDLE(file.as_raw_handle());
        let mut attributes = FILE_ATTRIBUTE_TAG_INFO::default();
        // SAFETY: `handle` remains owned by `file`; `attributes` points to a
        // correctly sized writable FILE_ATTRIBUTE_TAG_INFO value.
        unsafe {
            GetFileInformationByHandleEx(
                handle,
                FileAttributeTagInfo,
                std::ptr::addr_of_mut!(attributes).cast(),
                size_of::<FILE_ATTRIBUTE_TAG_INFO>() as u32,
            )
        }
        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;

        if (attributes.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0)
            || (attributes.FileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0)
        {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "reparse points and directories are not accepted",
            ));
        }
    }
    Ok(file)
}

fn wal_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}-wal", path.display()))
}

/// A candidate for `path` when it (or its WAL) changed at or after `since`.
/// Symlinked files, or a symlinked WAL, are never candidates.
pub fn candidate(
    source: SourceId,
    root: &Path,
    path: PathBuf,
    since: SystemTime,
) -> Option<Candidate> {
    let main = regular_file(&path)?;
    let wal = match fs::symlink_metadata(wal_path(&path)) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Ok(metadata) if metadata.file_type().is_file() => Some(metadata),
        _ => return None,
    };
    let parts: Vec<(u64, Option<SystemTime>)> = std::iter::once(&main)
        .chain(wal.as_ref())
        .map(|metadata| (metadata.len(), metadata.modified().ok()))
        .collect();
    let modified = parts.iter().filter_map(|(_, modified)| *modified).max()?;
    if modified < since {
        return None;
    }
    Some(Candidate {
        source,
        root: root.to_path_buf(),
        path,
        modified,
        fingerprint: Fingerprint(parts),
    })
}

/// Store entries of `source` changed at or after `since`.
pub fn discover(source: SourceId, roots: &SourceRoots, since: SystemTime) -> Vec<Candidate> {
    let found: Vec<(PathBuf, PathBuf)> = match source {
        SourceId::ClaudeCode => claude::discover(roots),
        SourceId::Codex => codex::discover(roots),
        SourceId::CopilotCli => copilot_cli::discover(roots),
        SourceId::CopilotVscode => copilot_vscode::discover(roots),
        SourceId::Cursor => cursor::discover(roots),
        SourceId::CursorCli => cursor_cli::discover(roots),
        SourceId::Antigravity => antigravity::discover(roots),
    };
    found
        .into_iter()
        .filter_map(|(root, path)| candidate(source, &root, path, since))
        .collect()
}

/// Every entry of `paths` paired with its source root.
pub fn under(root: &Path, paths: Vec<PathBuf>) -> Vec<(PathBuf, PathBuf)> {
    paths
        .into_iter()
        .map(|path| (root.to_path_buf(), path))
        .collect()
}

/// Loads the sessions of one candidate into block builders. `window_start`
/// bounds history (and multi-session stores such as Cursor's database). An
/// error (a store caught mid-write) leaves the candidate to the next scan.
pub async fn load(
    candidate: &Candidate,
    now: DateTime<Utc>,
    window_start: DateTime<Utc>,
) -> Result<Vec<(SessionInfo, BlockBuilder)>, String> {
    inside_root(&candidate.root, &candidate.path).ok_or_else(|| {
        format!(
            "{} is no longer a regular file under {}",
            candidate.path.display(),
            candidate.root.display()
        )
    })?;
    let source = candidate.source;
    let new_builder = move || BlockBuilder::new(source, now, window_start);
    match source {
        SourceId::Cursor => cursor::load(&candidate.path, window_start, new_builder).await,
        SourceId::CursorCli => {
            cursor_cli::load(&candidate.path, candidate.modified, new_builder).await
        }
        source => {
            let path = candidate.path.clone();
            let root = candidate.root.clone();
            tokio::task::spawn_blocking(move || {
                let mut builder = new_builder();
                let info = read_file(source, &root, &path, &mut |record| builder.push(record))
                    .map_err(|error| error.to_string())?;
                Ok(vec![(info, builder)])
            })
            .await
            .map_err(|error| error.to_string())?
        }
    }
}

fn read_file(
    source: SourceId,
    root: &Path,
    path: &Path,
    sink: &mut RecordSink<'_>,
) -> io::Result<SessionInfo> {
    match source {
        SourceId::ClaudeCode => claude::read(path, sink),
        SourceId::Codex => codex::read(path, sink),
        SourceId::CopilotCli => copilot_cli::read(path, sink),
        SourceId::CopilotVscode => copilot_vscode::read(root, path, sink),
        SourceId::Antigravity => antigravity::read(path, sink),
        SourceId::Cursor | SourceId::CursorCli => Err(io::Error::other("not a file source")),
    }
}

/// Outcome of reading one line with a size cap.
enum Line {
    Complete,
    TooLong,
    End,
}

/// Reads one line into `out` (without the newline), never buffering more than
/// `cap` bytes: a longer line is consumed and reported as [`Line::TooLong`].
fn read_capped_line(reader: &mut impl BufRead, out: &mut Vec<u8>, cap: usize) -> io::Result<Line> {
    out.clear();
    let mut read_any = false;
    let mut too_long = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(match (read_any, too_long) {
                (false, _) => Line::End,
                (true, true) => Line::TooLong,
                (true, false) => Line::Complete,
            });
        }
        read_any = true;
        let newline = available.iter().position(|byte| *byte == b'\n');
        let chunk = &available[..newline.unwrap_or(available.len())];
        if !too_long {
            if out.len() + chunk.len() > cap {
                too_long = true;
                out.clear();
            } else {
                out.extend_from_slice(chunk);
            }
        }
        let used = newline.map_or(available.len(), |index| index + 1);
        reader.consume(used);
        if newline.is_some() {
            return Ok(if too_long {
                Line::TooLong
            } else {
                Line::Complete
            });
        }
    }
}

/// Calls `visit` with every line (raw bytes, at most [`MAX_LINE_BYTES`])
/// until it returns `false`.
pub fn for_each_line(path: &Path, mut visit: impl FnMut(&[u8]) -> bool) -> io::Result<()> {
    let mut reader = BufReader::new(open_regular(path)?);
    let mut line = Vec::new();
    loop {
        match read_capped_line(&mut reader, &mut line, MAX_LINE_BYTES)? {
            Line::End => return Ok(()),
            Line::TooLong => continue,
            Line::Complete => {
                if !visit(&line) {
                    return Ok(());
                }
            }
        }
    }
}

/// Calls `visit` with every well-formed JSON object line until it returns
/// `false`. Partial last lines (the agent is mid-write), invalid lines, and
/// oversized lines are skipped; the next scan sees completed lines.
pub fn for_each_json_line(path: &Path, mut visit: impl FnMut(&Value) -> bool) -> io::Result<()> {
    for_each_line(path, |bytes| {
        let text = String::from_utf8_lossy(bytes);
        let text = text.trim();
        if text.is_empty() {
            return true;
        }
        match serde_json::from_str::<Value>(text) {
            Ok(value @ Value::Object(_)) => visit(&value),
            _ => true,
        }
    })
}

pub fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut current = value;
    for key in path {
        current = current.get(key)?;
    }
    current.as_str()
}

pub fn file_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// `file:///Users/me/My%20Repo` → `/Users/me/My Repo`.
pub fn path_from_file_uri(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    urlencoding::decode(path)
        .ok()
        .map(|decoded| decoded.into_owned())
        .filter(|decoded| decoded.starts_with('/'))
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::super::record::Record;
    use super::*;

    pub fn write_lines(dir: &Path, relative: &str, values: &[Value]) -> PathBuf {
        let path = dir.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let body: String = values.iter().map(|value| format!("{value}\n")).collect();
        fs::write(&path, body).unwrap();
        path
    }

    /// Runs a streaming reader and collects what it emits.
    pub fn collect(
        read: impl FnOnce(&mut RecordSink<'_>) -> io::Result<SessionInfo>,
    ) -> (SessionInfo, Vec<Record>) {
        let mut records = Vec::new();
        let info = read(&mut |record| {
            records.push(record);
            true
        })
        .unwrap();
        (info, records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_uris_decode_to_paths() {
        assert_eq!(
            path_from_file_uri("file:///Users/me/My%20Repo").as_deref(),
            Some("/Users/me/My Repo")
        );
        assert_eq!(path_from_file_uri("vscode-remote://x/y"), None);
    }

    #[test]
    fn json_lines_skip_partial_malformed_and_oversized_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        let huge = format!("{{\"big\":\"{}\"}}", "x".repeat(MAX_LINE_BYTES + 10));
        fs::write(
            &path,
            format!("{{\"a\":1}}\nnot json\n\n[1]\n{huge}\n{{\"b\":2}}\n{{\"partial\":"),
        )
        .unwrap();
        let mut keys = Vec::new();
        for_each_json_line(&path, |value| {
            keys.extend(value.as_object().unwrap().keys().cloned());
            true
        })
        .unwrap();
        assert_eq!(keys, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn reading_stops_when_the_visitor_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.jsonl");
        fs::write(&path, "{\"n\":1}\n{\"n\":2}\n{\"n\":3}\n").unwrap();
        let mut seen = 0;
        for_each_json_line(&path, |_| {
            seen += 1;
            seen < 2
        })
        .unwrap();
        assert_eq!(seen, 2);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_below_a_source_root_are_never_candidates_or_read() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let secret = outside.path().join("secret.jsonl");
        fs::write(&secret, "{\"type\":\"user\"}\n").unwrap();
        fs::create_dir_all(outside.path().join("project")).unwrap();
        fs::write(outside.path().join("project/linked-dir.jsonl"), "{}\n").unwrap();

        let projects = home.path().join(".claude/projects");
        fs::create_dir_all(projects.join("real")).unwrap();
        fs::write(projects.join("real/session.jsonl"), "{}\n").unwrap();
        symlink(&secret, projects.join("real/linked-file.jsonl")).unwrap();
        symlink(
            outside.path().join("project"),
            projects.join("linked-project"),
        )
        .unwrap();

        let roots = SourceRoots::from_home(home.path());
        let found: Vec<String> = discover(SourceId::ClaudeCode, &roots, SystemTime::UNIX_EPOCH)
            .iter()
            .map(|candidate| {
                candidate
                    .path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(found, vec!["session.jsonl".to_string()]);

        // A file swapped for a symlink after discovery is not read either.
        let root = source_root(&projects).unwrap();
        assert_eq!(
            inside_root(&root, &projects.join("real/linked-file.jsonl")),
            None
        );
        assert!(open_regular(&projects.join("real/linked-file.jsonl")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_source_location_is_resolved_once() {
        use std::os::unix::fs::symlink;
        let home = tempfile::tempdir().unwrap();
        let dotfiles = tempfile::tempdir().unwrap();
        fs::create_dir_all(dotfiles.path().join("projects/p")).unwrap();
        fs::write(dotfiles.path().join("projects/p/s.jsonl"), "{}\n").unwrap();
        symlink(dotfiles.path(), home.path().join(".claude")).unwrap();
        let roots = SourceRoots::from_home(home.path());
        let found = discover(SourceId::ClaudeCode, &roots, SystemTime::UNIX_EPOCH);
        assert_eq!(found.len(), 1);
        assert!(inside_root(&found[0].root, &found[0].path).is_some());
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_wal_disqualifies_a_sqlite_store() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("store.db");
        fs::write(&db, "x").unwrap();
        let elsewhere = tempfile::NamedTempFile::new().unwrap();
        symlink(elsewhere.path(), dir.path().join("store.db-wal")).unwrap();
        assert!(candidate(SourceId::CursorCli, dir.path(), db, SystemTime::UNIX_EPOCH).is_none());
    }

    #[test]
    fn windows_source_roots_resolve_cursor_and_copilot_from_appdata_and_cli_from_profile() {
        let profile = PathBuf::from(r"C:\Users\tester");
        let appdata = PathBuf::from(r"C:\Users\tester\AppData\Roaming");
        let roots = SourceRoots::from_windows(&profile, &appdata);
        assert_eq!(
            cursor::root(&roots),
            appdata.join("Cursor").join("User").join("globalStorage")
        );
        assert_eq!(
            copilot_vscode::user_dir(&roots),
            appdata.join("Code").join("User")
        );
        assert_eq!(
            claude::root(&roots),
            profile.join(".claude").join("projects")
        );
        assert_eq!(codex::root(&roots), profile.join(".codex").join("sessions"));
        assert_eq!(
            copilot_cli::root(&roots),
            profile.join(".copilot").join("session-state")
        );
        assert_eq!(
            cursor_cli::root(&roots),
            profile.join(".cursor").join("chats")
        );
        assert_eq!(
            antigravity::cli_root(&roots),
            profile.join(".gemini").join("antigravity-cli")
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_source_roots_current_resolves_without_home() {
        const SENTINEL: &str = "CLOVY_TEST_WINDOWS_SOURCE_ROOTS_CURRENT_CHILD";
        if std::env::var_os(SENTINEL).is_some() {
            let roots =
                SourceRoots::current().expect("SourceRoots::current() must resolve on Windows");
            let userprofile = std::env::var_os("USERPROFILE")
                .map(PathBuf::from)
                .expect("USERPROFILE must be set on Windows");
            let appdata = std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .expect("APPDATA must be set on Windows");
            assert_eq!(roots.home, userprofile);
            assert_eq!(roots.app_support("Cursor"), appdata.join("Cursor"));
            return;
        }

        let current_exe = std::env::current_exe().expect("current_exe");
        let status = std::process::Command::new(current_exe)
            .arg(
                "coding_agents::sources::tests::windows_source_roots_current_resolves_without_home",
            )
            .arg("--exact")
            .arg("--nocapture")
            .env(SENTINEL, "1")
            .env_remove("HOME")
            .status()
            .expect("child process failed to start");
        assert!(status.success(), "child process failed: {status:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_open_regular_refuses_reparse_points() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.jsonl");
        fs::write(&target, "{}\n").unwrap();
        let link = dir.path().join("link.jsonl");
        match std::os::windows::fs::symlink_file(&target, &link) {
            Ok(()) => {
                assert!(
                    open_regular(&link).is_err(),
                    "open_regular must refuse reparse points on Windows"
                );
            }
            Err(error) => {
                panic!(
                    "failed to create symlink for reparse point test: {error} (os error {:?})",
                    error.raw_os_error()
                );
            }
        }
    }

    #[test]
    fn from_home_confines_app_support_under_supplied_home() {
        let temp = tempfile::tempdir().unwrap();
        let roots = SourceRoots::from_home(temp.path());
        let cursor_dir = roots.app_support("Cursor");
        assert!(
            cursor_dir.starts_with(temp.path()),
            "app_support for from_home must stay under supplied home ({:?}), got {:?}",
            temp.path(),
            cursor_dir
        );
        assert_eq!(
            cursor_dir,
            temp.path()
                .join("Library")
                .join("Application Support")
                .join("Cursor")
        );
    }
}
