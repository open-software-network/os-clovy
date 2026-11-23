//! Behavior of the one-time identity migration: when it copies, that the copy
//! is complete or absent, and that the source never changes.

use super::*;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;

const RELEASE: &str = "dev.guilhermexp.clovy";

#[derive(Default)]
struct MemoryStore {
    items: Mutex<BTreeMap<(String, String), String>>,
    denied: Mutex<BTreeSet<(String, String)>>,
}

impl MemoryStore {
    fn with(items: &[(&str, &str, &str)]) -> Self {
        let store = Self::default();
        for (service, account, value) in items {
            store.put(service, account, value);
        }
        store
    }

    fn put(&self, service: &str, account: &str, value: &str) {
        self.items
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), value.into());
    }

    fn deny(&self, service: &str, account: &str) {
        self.denied
            .lock()
            .unwrap()
            .insert((service.into(), account.into()));
    }

    fn value(&self, service: &str, account: &str) -> Option<String> {
        self.items
            .lock()
            .unwrap()
            .get(&(service.to_string(), account.to_string()))
            .cloned()
    }

    fn snapshot(&self) -> BTreeMap<(String, String), String> {
        self.items.lock().unwrap().clone()
    }
}

impl CredentialStore for MemoryStore {
    fn accounts(&self, service: &str) -> Result<Vec<String>, String> {
        Ok(self
            .items
            .lock()
            .unwrap()
            .keys()
            .filter(|(item_service, _)| item_service == service)
            .map(|(_, account)| account.clone())
            .collect())
    }

    fn get(&self, service: &str, account: &str) -> Result<Option<String>, String> {
        if self
            .denied
            .lock()
            .unwrap()
            .contains(&(service.to_string(), account.to_string()))
        {
            return Err("user denied access".into());
        }
        Ok(self.value(service, account))
    }

    fn set(&self, service: &str, account: &str, value: &str) -> Result<(), String> {
        self.put(service, account, value);
        Ok(())
    }

    fn delete(&self, service: &str, account: &str) -> Result<(), String> {
        self.items
            .lock()
            .unwrap()
            .remove(&(service.to_string(), account.to_string()));
        Ok(())
    }
}

/// Fails the copy of the `fail_at`-th file, after copying the earlier ones.
struct FailingCopier {
    copied: usize,
    fail_at: usize,
}

impl FileCopier for FailingCopier {
    fn copy_file(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.copied += 1;
        if self.copied == self.fail_at {
            return Err(std::io::Error::other("injected copy failure"));
        }
        std::fs::copy(from, to).map(|_| ())
    }
}

/// Every path under `root` with the SHA-256 of each file's contents.
fn tree_hash(root: &Path) -> BTreeMap<PathBuf, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_path_buf();
            let meta = std::fs::symlink_metadata(&path).unwrap();
            if meta.is_dir() {
                out.insert(relative, "dir".into());
                stack.push(path);
            } else if meta.file_type().is_symlink() {
                out.insert(
                    relative,
                    format!("link:{}", std::fs::read_link(&path).unwrap().display()),
                );
            } else {
                let digest = Sha256::digest(std::fs::read(&path).unwrap());
                out.insert(relative, format!("{digest:x}"));
            }
        }
    }
    out
}

fn write(path: &Path, contents: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// An old profile: notes, an activity database, a recording and settings.
fn old_profile(root: &Path) {
    write(&root.join("notes.sqlite3"), b"notes database bytes");
    write(&root.join("activity.sqlite3"), b"encrypted activity bytes");
    write(
        &root
            .join("recordings")
            .join("note-1")
            .join("s-1")
            .join("mic.wav"),
        &vec![7_u8; 64 * 1024],
    );
    write(&root.join("provider-settings.json"), br#"{"engine":"cli"}"#);
}

fn dir_plan(base: &Path) -> MigrationPlan {
    MigrationPlan {
        data: DirectoryCopy {
            source: base.join("old"),
            target: base.join("new"),
        },
        extra_directories: Vec::new(),
        credentials: Vec::new(),
        source_executables: Vec::new(),
        lock_wait: std::time::Duration::ZERO,
    }
}

fn is_empty_or_absent(path: &Path) -> bool {
    !path.exists() || std::fs::read_dir(path).unwrap().next().is_none()
}

fn leftover_staging(base: &Path) -> Vec<String> {
    std::fs::read_dir(base)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        // The lock file stays on disk by design: only staging is leftover.
        .filter(|name| {
            name != "old" && name != "new" && !name.ends_with(".identity-migration.lock")
        })
        .collect()
}

#[test]
fn decides_to_migrate_only_into_an_absent_or_empty_target() {
    let base = tempfile::tempdir().unwrap();
    let source = base.path().join("old");
    let target = base.path().join("new");

    assert_eq!(decide(&source, &target), Decision::NoSource);

    old_profile(&source);
    assert_eq!(decide(&source, &target), Decision::Migrate);

    std::fs::create_dir_all(&target).unwrap();
    assert_eq!(decide(&source, &target), Decision::Migrate);

    write(&target.join("notes.sqlite3"), b"already in use");
    assert_eq!(decide(&source, &target), Decision::TargetInUse);
}

#[test]
fn copies_the_whole_profile_and_leaves_the_source_unchanged() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    std::os::unix::fs::symlink("notes.sqlite3", plan.data.source.join("notes-link")).unwrap();
    let before = tree_hash(&plan.data.source);

    let outcome = migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    assert!(
        matches!(outcome, Outcome::Migrated { files: 4, .. }),
        "{outcome:?}"
    );
    assert_eq!(tree_hash(&plan.data.target), before);
    assert_eq!(tree_hash(&plan.data.source), before);
    assert!(leftover_staging(base.path()).is_empty());
}

#[test]
fn a_target_in_use_is_left_alone() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    write(
        &plan.data.target.join("notes.sqlite3"),
        b"new identity notes",
    );
    plan.credentials.push(CredentialCopy {
        source_service: "old.activity-db".into(),
        legacy_service: None,
        target_service: "new.activity-db".into(),
    });
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", "k")]);
    let target_before = tree_hash(&plan.data.target);

    let outcome = migrate(&plan, &store, &mut StdCopier).expect("migrate");

    assert_eq!(outcome, Outcome::Skipped(Decision::TargetInUse));
    assert_eq!(tree_hash(&plan.data.target), target_before);
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);
}

#[test]
fn a_copy_failure_leaves_the_target_empty_and_the_source_unchanged() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.extra_directories.push(DirectoryCopy {
        source: base.path().join("old-webkit"),
        target: base.path().join("new-webkit"),
    });
    write(
        &base.path().join("old-webkit").join("LocalStorage"),
        b"ui state",
    );
    plan.credentials.push(CredentialCopy {
        source_service: "old.activity-db".into(),
        legacy_service: None,
        target_service: "new.activity-db".into(),
    });
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", "k")]);
    let before = tree_hash(&plan.data.source);

    // The third file fails after two were already copied.
    let error = migrate(
        &plan,
        &store,
        &mut FailingCopier {
            copied: 0,
            fail_at: 3,
        },
    )
    .expect_err("injected failure");

    assert!(
        error.to_string().contains("injected copy failure"),
        "{error}"
    );
    assert!(is_empty_or_absent(&plan.data.target));
    assert!(is_empty_or_absent(&base.path().join("new-webkit")));
    assert_eq!(tree_hash(&plan.data.source), before);
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);
    let leftovers: Vec<_> = leftover_staging(base.path())
        .into_iter()
        .filter(|name| name != "old-webkit")
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn copies_credentials_including_the_activity_key_without_removing_them() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials = vec![
        CredentialCopy {
            source_service: "old.activity-db".into(),
            legacy_service: None,
            target_service: "new.activity-db".into(),
        },
        CredentialCopy {
            source_service: "old.llm-providers".into(),
            legacy_service: None,
            target_service: "new.llm-providers".into(),
        },
    ];
    let store = MemoryStore::with(&[
        ("old.activity-db", "sqlcipher-key", "ab".repeat(32).as_str()),
        ("old.llm-providers", "endpoint:a", "sk-a"),
        ("old.llm-providers", "endpoint:b", "sk-b"),
        ("unrelated", "x", "y"),
    ]);
    let before = store.snapshot();

    let outcome = migrate(&plan, &store, &mut StdCopier).expect("migrate");

    assert!(
        matches!(outcome, Outcome::Migrated { credentials: 3, .. }),
        "{outcome:?}"
    );
    assert_eq!(
        store.value("new.activity-db", "sqlcipher-key"),
        Some("ab".repeat(32))
    );
    assert_eq!(
        store.value("new.llm-providers", "endpoint:a").as_deref(),
        Some("sk-a")
    );
    assert_eq!(
        store.value("new.llm-providers", "endpoint:b").as_deref(),
        Some("sk-b")
    );
    for (key, value) in before {
        assert_eq!(store.value(&key.0, &key.1), Some(value), "{key:?} changed");
    }
}

#[test]
fn a_denied_credential_fails_the_whole_migration() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials = vec![
        CredentialCopy {
            source_service: "old.llm-providers".into(),
            legacy_service: None,
            target_service: "new.llm-providers".into(),
        },
        CredentialCopy {
            source_service: "old.activity-db".into(),
            legacy_service: None,
            target_service: "new.activity-db".into(),
        },
    ];
    let store = MemoryStore::with(&[
        ("old.llm-providers", "endpoint:a", "sk-a"),
        ("old.activity-db", "sqlcipher-key", "k"),
    ]);
    store.deny("old.activity-db", "sqlcipher-key");
    let before = tree_hash(&plan.data.source);

    let error = migrate(&plan, &store, &mut StdCopier).expect_err("denied");

    assert!(error.to_string().contains("denied"), "{error}");
    assert!(is_empty_or_absent(&plan.data.target));
    assert_eq!(store.value("new.llm-providers", "endpoint:a"), None);
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);
    assert_eq!(tree_hash(&plan.data.source), before);
    assert!(leftover_staging(base.path()).is_empty());
}

#[test]
fn bridged_credentials_migrate_the_value_the_old_bridge_would_read() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials = vec![CredentialCopy {
        source_service: "clovy.google".into(),
        legacy_service: Some("june.google".into()),
        target_service: "new.google".into(),
    }];
    let store = MemoryStore::with(&[
        // Only the June-era service has it: still signed in.
        ("june.google", "legacy-only", "token-1"),
        // Both agree.
        ("clovy.google", "both", "token-2"),
        ("june.google", "both", "token-2"),
        // Only the Clovy service has it: the old bridge read it as signed out.
        ("clovy.google", "signed-out", "stale"),
    ]);

    migrate(&plan, &store, &mut StdCopier).expect("migrate");

    assert_eq!(
        store.value("new.google", "legacy-only").as_deref(),
        Some("token-1")
    );
    assert_eq!(
        store.value("new.google", "both").as_deref(),
        Some("token-2")
    );
    assert_eq!(store.value("new.google", "signed-out"), None);
}

#[tokio::test]
async fn the_migrated_activity_database_opens_with_the_copied_key() {
    use crate::activity::key::{ActivityKeyStore, MemoryKeyStore};
    use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
    use chrono::{TimeZone, Utc};

    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let old_keys = MemoryKeyStore::default();
    let old_db = ActivityStore::open(&plan.data.source.join(ACTIVITY_DB_FILE), &old_keys)
        .await
        .expect("open old");
    old_db
        .insert_frame(&NewFrame {
            captured_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
            app_name: "Zed".into(),
            bundle_id: None,
            window_title: Some("notes.md".into()),
            browser_url: None,
            text_source: TextSource::Accessibility,
            text: Some("migrated activity".into()),
            input_idle_ms: None,
        })
        .await
        .expect("insert");
    old_db.close().await;
    let old_key = old_keys.current().expect("old key");
    plan.credentials.push(CredentialCopy {
        source_service: "old.activity-db".into(),
        legacy_service: None,
        target_service: "new.activity-db".into(),
    });
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", &old_key)]);

    migrate(&plan, &store, &mut StdCopier).expect("migrate");

    let new_keys = MemoryKeyStore::default();
    new_keys
        .store(
            &store
                .value("new.activity-db", "sqlcipher-key")
                .expect("copied key"),
        )
        .unwrap();
    let new_db = ActivityStore::open(&plan.data.target.join(ACTIVITY_DB_FILE), &new_keys)
        .await
        .expect("open migrated");
    assert_eq!(new_db.frames_after(0, 10).await.unwrap().len(), 1);
}

#[test]
fn the_june_profile_migrates_only_into_the_release_identifier() {
    let source = default_source(RELEASE).expect("June source for the release identifier");
    assert_eq!(source.identifier, "co.opensoftware.june");
    assert!(default_source("dev.guilhermexp.clovy.proof").is_none());
}

#[test]
fn the_dev_plan_maps_every_june_name_to_the_new_dev_names() {
    let dirs = PlatformDirs {
        data: PathBuf::from("/home/Library/Application Support"),
        config: PathBuf::from("/home/Library/Application Support"),
        webkit: Some(PathBuf::from("/home/Library/WebKit")),
    };
    let source = default_source(RELEASE).unwrap();

    let plan = plan(&dirs, RELEASE, &source);

    assert_eq!(
        plan.data,
        DirectoryCopy {
            source: dirs.data.join("co.opensoftware.june-dev"),
            target: dirs.data.join("dev.guilhermexp.clovy-dev"),
        }
    );
    assert_eq!(
        plan.extra_directories,
        vec![DirectoryCopy {
            source: PathBuf::from("/home/Library/WebKit/co.opensoftware.june.dev"),
            target: PathBuf::from("/home/Library/WebKit/dev.guilhermexp.clovy.dev"),
        }]
    );
    let activity = plan
        .credentials
        .iter()
        .find(|copy| copy.target_service == "dev.guilhermexp.clovy-dev.activity-db")
        .expect("activity key");
    assert_eq!(
        activity.source_service,
        "co.opensoftware.clovy-dev.activity-db"
    );
    assert_eq!(activity.legacy_service, None);
    let secrets = plan
        .credentials
        .iter()
        .find(|copy| copy.target_service == "dev.guilhermexp.clovy-dev.agent-secrets")
        .expect("agent secrets");
    assert_eq!(
        secrets.source_service,
        "co.opensoftware.clovy.dev.agent-secrets"
    );
    assert_eq!(
        secrets.legacy_service.as_deref(),
        Some("co.opensoftware.june.dev.agent-secrets")
    );
    let google = plan
        .credentials
        .iter()
        .find(|copy| copy.target_service == "dev.guilhermexp.clovy-dev.google")
        .expect("google connector");
    assert_eq!(google.source_service, "co.opensoftware.clovy-dev.google");
    assert_eq!(
        google.legacy_service.as_deref(),
        Some("co.opensoftware.june-dev.google")
    );
}

#[test]
fn a_proof_source_reads_only_its_own_names() {
    let source = source_override("dev.guilhermexp.clovy.proofold").expect("override");
    let dirs = PlatformDirs {
        data: PathBuf::from("/s"),
        config: PathBuf::from("/s"),
        webkit: None,
    };

    let plan = plan(&dirs, "dev.guilhermexp.clovy.proofnew", &source);

    assert_eq!(
        plan.data.source,
        PathBuf::from("/s/dev.guilhermexp.clovy.proofold-dev")
    );
    assert_eq!(
        plan.data.target,
        PathBuf::from("/s/dev.guilhermexp.clovy.proofnew-dev")
    );
    for copy in &plan.credentials {
        assert!(
            copy.source_service
                .starts_with("dev.guilhermexp.clovy.proofold"),
            "{copy:?}"
        );
        assert!(copy.legacy_service.is_none(), "{copy:?}");
        assert!(
            copy.target_service
                .starts_with("dev.guilhermexp.clovy.proofnew-dev."),
            "{copy:?}"
        );
    }
    assert!(source_override("not an identifier").is_none());
}

// --- Data safety (review findings I1 to I5) ---------------------------------

fn activity_copy() -> CredentialCopy {
    CredentialCopy {
        source_service: "old.activity-db".into(),
        legacy_service: None,
        target_service: "new.activity-db".into(),
    }
}

/// A notes database in WAL mode whose last rows live only in the WAL, held
/// open by a connection the caller keeps alive.
async fn notes_with_pending_wal(path: &Path) -> sqlx_sqlite::SqliteConnection {
    use sqlx::connection::ConnectOptions;
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut connection = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(sqlx_sqlite::SqliteJournalMode::Wal)
        .pragma("wal_autocheckpoint", "0")
        .connect()
        .await
        .unwrap();
    sqlx::query::query("CREATE TABLE probe(id INTEGER PRIMARY KEY, body TEXT)")
        .execute(&mut connection)
        .await
        .unwrap();
    for index in 0..50 {
        sqlx::query::query("INSERT INTO probe(body) VALUES (?)")
            .bind(format!("row {index}"))
            .execute(&mut connection)
            .await
            .unwrap();
    }
    connection
}

async fn count_rows(path: &Path, table: &str) -> i64 {
    use sqlx::connection::ConnectOptions;
    use sqlx::row::Row;
    let mut connection = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .unwrap();
    sqlx::query::query(&format!("SELECT count(*) AS n FROM {table}"))
        .fetch_one(&mut connection)
        .await
        .unwrap()
        .get("n")
}

#[test]
fn refuses_to_copy_while_the_old_app_is_running() {
    use std::io::{BufRead, BufReader, Read};
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::process::{Child, Command, Stdio};
    use std::time::Duration;

    struct OldApp(Child);
    impl Drop for OldApp {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    const OLD_APP_SOURCE: &str = r#"
#include <unistd.h>

int main(void) {
    const char ready[] = "migration-fixture-ready\n";
    if (write(STDOUT_FILENO, ready, sizeof ready - 1) != (ssize_t)(sizeof ready - 1)) {
        return 1;
    }
    char stop;
    return read(STDIN_FILENO, &stop, 1) == 0 ? 0 : 2;
}
"#;

    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    // A unique executable name short enough for the process-name detector.
    let name = format!("old{:08x}", rand::random::<u32>());
    let binary = base.path().join(&name);
    let source = base.path().join("old_app.c");
    std::fs::write(&source, OLD_APP_SOURCE).unwrap();
    let compiled = Command::new("/usr/bin/clang")
        .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
        .arg(&source)
        .arg("-o")
        .arg(&binary)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "fixture compile failed: {}; stderr: {}",
        compiled.status,
        String::from_utf8_lossy(&compiled.stderr)
    );
    let (mut control, child_io) = UnixStream::pair().unwrap();
    control
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut old_app = OldApp(
        Command::new(&binary)
            .stdin(Stdio::from(OwnedFd::from(child_io.try_clone().unwrap())))
            .stdout(Stdio::from(OwnedFd::from(child_io)))
            .stderr(Stdio::piped())
            .spawn()
            .unwrap(),
    );
    // The child's marker proves exec completed; open stdin holds it alive.
    let mut ready = String::new();
    let readiness = BufReader::new(&mut control).read_line(&mut ready);
    if readiness.is_err() || ready != "migration-fixture-ready\n" {
        let _ = old_app.0.kill();
        let status = old_app.0.wait().unwrap();
        let mut stderr = String::new();
        old_app
            .0
            .stderr
            .take()
            .unwrap()
            .read_to_string(&mut stderr)
            .unwrap();
        panic!("fixture readiness failed: {readiness:?}; marker: {ready:?}; child: {status}; stderr: {stderr}");
    }
    assert!(old_app.0.try_wait().unwrap().is_none());
    // Use the guard's exact detector and require this child, not just any
    // process matching the name, before attempting migration.
    let detected = Command::new("/usr/bin/pgrep")
        .args(["-x", &name])
        .output()
        .unwrap();
    assert!(detected.status.success(), "{detected:?}");
    assert!(String::from_utf8(detected.stdout)
        .unwrap()
        .lines()
        .any(|pid| pid.parse::<u32>().ok() == Some(old_app.0.id())));
    plan.source_executables = vec![name.clone()];
    let before = tree_hash(&plan.data.source);

    let result = migrate(&plan, &MemoryStore::default(), &mut StdCopier);
    old_app.0.kill().unwrap();
    old_app.0.wait().unwrap();

    let error = result.expect_err("the old app is running");
    assert!(error.blocked, "{error}");
    assert!(error.to_string().contains(&name), "{error}");
    assert!(is_empty_or_absent(&plan.data.target));
    assert_eq!(tree_hash(&plan.data.source), before);
}

#[tokio::test]
async fn databases_are_copied_as_single_file_snapshots() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    let source_db = plan.data.source.join("notes.sqlite3");
    let writer = notes_with_pending_wal(&source_db).await;
    assert!(plan.data.source.join("notes.sqlite3-wal").exists());
    let before = tree_hash(&plan.data.source);

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    for companion in ["notes.sqlite3-wal", "notes.sqlite3-shm"] {
        assert!(
            !plan.data.target.join(companion).exists(),
            "{companion} must not be copied: the snapshot is one file"
        );
    }
    assert_eq!(
        count_rows(&plan.data.target.join("notes.sqlite3"), "probe").await,
        50
    );
    assert_eq!(tree_hash(&plan.data.source), before);
    drop(writer);
}

#[tokio::test]
async fn the_activity_database_is_snapshotted_with_its_key() {
    use crate::activity::key::{ActivityKeyStore, MemoryKeyStore};
    use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
    use chrono::{TimeZone, Utc};

    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let keys = MemoryKeyStore::default();
    // Left open: its WAL holds frames that are not in the main file yet.
    let open_db = ActivityStore::open(&plan.data.source.join(ACTIVITY_DB_FILE), &keys)
        .await
        .unwrap();
    for minute in 0..5 {
        open_db
            .insert_frame(&NewFrame {
                captured_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, minute, 0).unwrap(),
                app_name: "Zed".into(),
                bundle_id: None,
                window_title: None,
                browser_url: None,
                text_source: TextSource::Accessibility,
                text: Some(format!("frame {minute}")),
                input_idle_ms: None,
            })
            .await
            .unwrap();
    }
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[(
        "old.activity-db",
        "sqlcipher-key",
        keys.current().unwrap().as_str(),
    )]);

    migrate(&plan, &store, &mut StdCopier).expect("migrate");

    assert!(!plan.data.target.join("activity.sqlite3-wal").exists());
    let raw = std::fs::read(plan.data.target.join(ACTIVITY_DB_FILE)).unwrap();
    assert!(
        !raw.starts_with(b"SQLite format 3"),
        "the snapshot stays encrypted"
    );
    let new_keys = MemoryKeyStore::default();
    new_keys
        .store(&store.value("new.activity-db", "sqlcipher-key").unwrap())
        .unwrap();
    let copied = ActivityStore::open(&plan.data.target.join(ACTIVITY_DB_FILE), &new_keys)
        .await
        .unwrap();
    assert_eq!(copied.frames_after(0, 10).await.unwrap().len(), 5);
    open_db.close().await;
}

#[test]
fn a_different_target_credential_fails_without_publishing() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[
        ("old.activity-db", "sqlcipher-key", "old-key"),
        // Minted by an earlier launch whose migration failed.
        ("new.activity-db", "sqlcipher-key", "orphan-key"),
    ]);

    let error = migrate(&plan, &store, &mut StdCopier).expect_err("conflicting key");

    assert!(error.to_string().contains("new.activity-db"), "{error}");
    assert!(is_empty_or_absent(&plan.data.target));
    assert_eq!(
        store.value("new.activity-db", "sqlcipher-key").as_deref(),
        Some("orphan-key")
    );
}

#[test]
fn an_equal_target_credential_is_accepted() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[
        ("old.activity-db", "sqlcipher-key", "same-key"),
        ("new.activity-db", "sqlcipher-key", "same-key"),
    ]);

    migrate(&plan, &store, &mut StdCopier).expect("equal key");

    assert!(plan.data.target.join("notes.sqlite3").exists());
}

fn lock_path(plan: &MigrationPlan) -> PathBuf {
    let mut name = plan.data.target.file_name().unwrap().to_os_string();
    name.push(".identity-migration.lock");
    plan.data.target.with_file_name(name)
}

fn staging_for(plan: &MigrationPlan, pid: u32) -> PathBuf {
    let mut name = plan.data.target.file_name().unwrap().to_os_string();
    name.push(format!(".identity-migration-{pid}"));
    plan.data.target.with_file_name(name)
}

fn dead_pid() -> u32 {
    let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
    let pid = child.id();
    child.wait().unwrap();
    pid
}

/// Holds the OS lock on the lock file, the way a live launch does, without
/// ever writing a pid into it.
fn hold_os_lock(path: &Path) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .unwrap();
    // SAFETY: a valid descriptor owned by `file`.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(locked, 0, "the test takes the lock first");
    file
}

#[test]
fn a_live_holder_blocks_even_before_it_wrote_its_pid() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    // Created and locked, pid not written yet.
    let holder = hold_os_lock(&lock_path(&plan));

    let error = migrate(&plan, &MemoryStore::default(), &mut StdCopier)
        .expect_err("a live holder owns the lock");

    assert!(error.blocked, "{error}");
    assert!(is_empty_or_absent(&plan.data.target));
    assert!(lock_path(&plan).exists(), "the holder's lock is left alone");
    drop(holder);
}

#[test]
fn a_lock_that_cannot_be_taken_fails_within_the_deadline() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    // A folder where the lock file goes: it can be neither opened nor removed.
    write(&lock_path(&plan).join("keep"), b"not a lock");

    let (sender, receiver) = std::sync::mpsc::channel();
    let worker_plan = plan.clone();
    std::thread::spawn(move || {
        let _ = sender.send(migrate(
            &worker_plan,
            &MemoryStore::default(),
            &mut StdCopier,
        ));
    });
    let result = receiver
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("the migration returns instead of retrying forever");

    let error = result.expect_err("the lock cannot be taken");
    assert!(error.blocked, "{error}");
    assert!(is_empty_or_absent(&plan.data.target));
}

#[test]
fn a_dead_holder_is_taken_over_and_only_dead_staging_is_removed() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    std::fs::write(lock_path(&plan), dead_pid().to_string()).unwrap();
    let dead_staging = staging_for(&plan, dead_pid());
    write(&dead_staging.join("partial"), b"left by a crashed launch");
    let mut live = std::process::Command::new("/bin/sleep")
        .arg("30")
        .spawn()
        .unwrap();
    let live_staging = staging_for(&plan, live.id());
    write(&live_staging.join("partial"), b"owned by a live process");

    let result = migrate(&plan, &MemoryStore::default(), &mut StdCopier);
    let _ = live.kill();
    let _ = live.wait();

    result.expect("a dead holder does not block");
    assert!(plan.data.target.join("notes.sqlite3").exists());
    assert!(!dead_staging.exists());
    assert!(live_staging.exists());
}

/// Records renames; can panic on the n-th one to stand in for a crash.
struct RenameRecorder {
    renames: Vec<PathBuf>,
    panic_at: Option<usize>,
}

impl FileCopier for RenameRecorder {
    fn copy_file(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to).map(|_| ())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.renames.push(to.to_path_buf());
        if self.panic_at == Some(self.renames.len()) {
            panic!("process died while publishing");
        }
        std::fs::rename(from, to)
    }
}

fn plan_with_webview(base: &Path) -> MigrationPlan {
    let mut plan = dir_plan(base);
    old_profile(&plan.data.source);
    plan.extra_directories.push(DirectoryCopy {
        source: base.join("old-webkit"),
        target: base.join("new-webkit"),
    });
    write(&base.join("old-webkit").join("LocalStorage"), b"ui state");
    plan
}

#[test]
fn the_data_directory_is_published_last() {
    let base = tempfile::tempdir().unwrap();
    let plan = plan_with_webview(base.path());
    let mut recorder = RenameRecorder {
        renames: Vec::new(),
        panic_at: None,
    };

    migrate(&plan, &MemoryStore::default(), &mut recorder).expect("migrate");

    assert_eq!(recorder.renames.last(), Some(&plan.data.target));
}

#[test]
fn an_interrupted_publication_is_finished_by_the_next_launch() {
    let base = tempfile::tempdir().unwrap();
    let plan = plan_with_webview(base.path());
    let store = MemoryStore::default();

    let crashed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut recorder = RenameRecorder {
            renames: Vec::new(),
            panic_at: Some(2),
        };
        migrate(&plan, &store, &mut recorder)
    }));
    assert!(crashed.is_err(), "the first launch died while publishing");

    migrate(&plan, &store, &mut StdCopier).expect("the next launch");

    assert!(plan.data.target.join("notes.sqlite3").exists());
    assert_eq!(
        std::fs::read(base.path().join("new-webkit").join("LocalStorage")).unwrap(),
        b"ui state"
    );
}

// --- I1: paths into the old profile are rewritten in the copy --------------

async fn exec(connection: &mut sqlx_sqlite::SqliteConnection, sql: &str, binds: &[&str]) {
    let mut statement = sqlx::query::query(sql);
    for bind in binds {
        statement = statement.bind(bind.to_string());
    }
    statement.execute(connection).await.unwrap();
}

async fn text(path: &Path, sql: &str) -> String {
    use sqlx::connection::ConnectOptions;
    use sqlx::row::Row;
    let mut connection = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .connect()
        .await
        .unwrap();
    sqlx::query::query(sql)
        .fetch_one(&mut connection)
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn paths_into_the_old_profile_are_rewritten_in_the_copy() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    std::fs::create_dir_all(&plan.data.source).unwrap();
    // The app stores canonical paths: /private/var/... for a /var/... dir.
    let old_canonical = plan.data.source.canonicalize().unwrap();
    let old_canonical = old_canonical.to_string_lossy().into_owned();
    let new_canonical = base
        .path()
        .canonicalize()
        .unwrap()
        .join("new")
        .to_string_lossy()
        .into_owned();
    let db = plan.data.source.join("notes.sqlite3");
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let mut notes = pool.acquire().await.unwrap();
    let workspace = format!("{old}/agent-workspaces/s1");
    let payload = serde_json::json!({
        "attachments": [{ "path": format!("{workspace}/attachments/a.png") }],
        "content": format!("[Clovy attachment manifest v1] {workspace}/attachments/a.png"),
    })
    .to_string();
    let config = serde_json::json!({ "workspace": workspace }).to_string();
    let t = "2026-10-01T09:00:00Z";
    exec(
        &mut notes,
        "INSERT INTO notes(id, created_at, updated_at) VALUES ('n1', ?, ?)",
        &[t, t],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO recording_sessions(id, note_id, status, started_at, partial_path, final_path) VALUES ('r1', 'n1', 'complete', ?, ?, ?)",
        &[t, &format!("{old}/recordings/n1/r1/mic.partial.wav"), &format!("{old_canonical}/recordings/n1/r1/mic.wav")],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO audio_artifacts(id, note_id, recording_session_id, path, format, duration_ms, size_bytes, checksum, created_at) VALUES ('a1', 'n1', 'r1', ?, 'wav', 1, 1, 'x', ?)",
        &[&format!("{old}/recordings/n1/r1/mic.wav"), t],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO agent_sessions(id, workspace_path, created_at, updated_at) VALUES ('s1', ?, ?, ?), ('s2', '/Users/someone/Projects/app', ?, ?), ('s3', ?, ?, ?)",
        &[&workspace, t, t, t, t, &format!("{old}-backup/agent-workspaces/s3"), t, t],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO agent_runs(id, session_id, status, model, started_at, updated_at, run_config_json) VALUES ('run1', 's1', 'completed', 'auto', ?, ?, ?)",
        &[t, t, &config],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO agent_items(id, session_id, run_id, sequence, kind, payload_json, created_at) VALUES ('i1', 's1', 'run1', 0, 'user_message', ?, ?)",
        &[&payload, t],
    )
    .await;
    drop(notes);
    pool.close().await;
    write(
        &plan.data.source.join("extension-host-path"),
        format!("{old}/extension-host.json").as_bytes(),
    );
    let before = tree_hash(&plan.data.source);

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let copied = plan.data.target.join("notes.sqlite3");
    assert_eq!(
        text(&copied, "SELECT partial_path FROM recording_sessions").await,
        format!("{new}/recordings/n1/r1/mic.partial.wav")
    );
    assert_eq!(
        text(&copied, "SELECT final_path FROM recording_sessions").await,
        format!("{new_canonical}/recordings/n1/r1/mic.wav")
    );
    assert_eq!(
        text(&copied, "SELECT path FROM audio_artifacts").await,
        format!("{new}/recordings/n1/r1/mic.wav")
    );
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 's1'"
        )
        .await,
        format!("{new}/agent-workspaces/s1")
    );
    // Paths outside the profile, even a look-alike sibling, stay put.
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 's2'"
        )
        .await,
        "/Users/someone/Projects/app"
    );
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 's3'"
        )
        .await,
        format!("{old}-backup/agent-workspaces/s3")
    );
    let payload: serde_json::Value =
        serde_json::from_str(&text(&copied, "SELECT payload_json FROM agent_items").await).unwrap();
    assert_eq!(
        payload["attachments"][0]["path"],
        format!("{new}/agent-workspaces/s1/attachments/a.png")
    );
    // Free text that only mentions the old folder is the user's content (K4).
    assert_eq!(
        payload["content"],
        format!("[Clovy attachment manifest v1] {old}/agent-workspaces/s1/attachments/a.png")
    );
    let config: serde_json::Value =
        serde_json::from_str(&text(&copied, "SELECT run_config_json FROM agent_runs").await)
            .unwrap();
    assert_eq!(config["workspace"], format!("{new}/agent-workspaces/s1"));
    assert_eq!(
        std::fs::read_to_string(plan.data.target.join("extension-host-path")).unwrap(),
        format!("{new}/extension-host.json")
    );
    assert_eq!(tree_hash(&plan.data.source), before);
}

#[tokio::test]
async fn paths_in_the_encrypted_activity_database_are_rewritten() {
    use crate::activity::key::{ActivityKeyStore, MemoryKeyStore};
    use crate::activity::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};
    use chrono::{TimeZone, Utc};

    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    let keys = MemoryKeyStore::default();
    let db = ActivityStore::open(&plan.data.source.join(ACTIVITY_DB_FILE), &keys)
        .await
        .unwrap();
    db.insert_frame(&NewFrame {
        captured_at: Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap(),
        app_name: "Terminal".into(),
        bundle_id: None,
        window_title: Some(format!("{old}/agent-workspaces/s1")),
        browser_url: None,
        text_source: TextSource::Accessibility,
        text: Some("cd into the workspace".into()),
        input_idle_ms: None,
    })
    .await
    .unwrap();
    db.close().await;
    let key = keys.current().unwrap();
    {
        use sqlx::connection::ConnectOptions;
        let mut raw = sqlx_sqlite::SqliteConnectOptions::new()
            .filename(plan.data.source.join(ACTIVITY_DB_FILE))
            .pragma("key", format!("\"x'{key}'\""))
            .connect()
            .await
            .unwrap();
        sqlx::query::query("INSERT INTO coding_agent_blocks(source, session_id, started_at, ended_at, cwd, transcript, state, updated_at) VALUES ('claude_code', 's1', '2026-10-01T09:00:00Z', '2026-10-01T09:05:00Z', ?, '[]', 'live', '2026-10-01T09:05:00Z')")
            .bind(format!("{old}/agent-workspaces/s1"))
            .execute(&mut raw)
            .await
            .unwrap();
        // Closed for real: a dropped connection checkpoints later, under the
        // copy, like a live writer would.
        sqlx::connection::Connection::close(raw).await.unwrap();
    }
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", key.as_str())]);

    migrate(&plan, &store, &mut StdCopier).expect("migrate");

    let new_keys = MemoryKeyStore::default();
    new_keys
        .store(&store.value("new.activity-db", "sqlcipher-key").unwrap())
        .unwrap();
    let copied = ActivityStore::open(&plan.data.target.join(ACTIVITY_DB_FILE), &new_keys)
        .await
        .unwrap();
    let frames = copied.frames_after(0, 10).await.unwrap();
    // Captured screen text is content: it stays as it was (K4).
    assert_eq!(
        frames[0].window_title.as_deref(),
        Some(format!("{old}/agent-workspaces/s1").as_str())
    );
    copied.close().await;
    use sqlx::connection::ConnectOptions;
    use sqlx::row::Row;
    let new_key = store.value("new.activity-db", "sqlcipher-key").unwrap();
    let mut raw = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(plan.data.target.join(ACTIVITY_DB_FILE))
        .pragma("key", format!("\"x'{new_key}'\""))
        .connect()
        .await
        .unwrap();
    let cwd: String = sqlx::query::query("SELECT cwd FROM coding_agent_blocks")
        .fetch_one(&mut raw)
        .await
        .unwrap()
        .get(0);
    assert_eq!(cwd, format!("{new}/agent-workspaces/s1"));
}

#[tokio::test]
async fn webview_local_storage_values_are_rewritten() {
    use sqlx::connection::ConnectOptions;
    use sqlx::row::Row;
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    plan.extra_directories.push(DirectoryCopy {
        source: base.path().join("old-webkit"),
        target: base.path().join("new-webkit"),
    });
    // WebKit keeps localStorage values as UTF-16 blobs.
    let store_path = base
        .path()
        .join("old-webkit/LocalStorage/localstorage.sqlite3");
    std::fs::create_dir_all(store_path.parent().unwrap()).unwrap();
    let utf16 =
        |value: &str| -> Vec<u8> { value.encode_utf16().flat_map(u16::to_le_bytes).collect() };
    let mut storage = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(&store_path)
        .create_if_missing(true)
        .connect()
        .await
        .unwrap();
    sqlx::query::query("CREATE TABLE ItemTable (key TEXT UNIQUE ON CONFLICT REPLACE PRIMARY KEY NOT NULL, value BLOB NOT NULL ON CONFLICT FAIL)")
        .execute(&mut storage)
        .await
        .unwrap();
    let queued = format!(r#"[{{"attachments":["{old}/agent-attachment-staging/a.png"]}}]"#);
    sqlx::query::query(
        "INSERT INTO ItemTable(key, value) VALUES ('clovy.agent.queuedFollowUps', ?)",
    )
    .bind(utf16(&queued))
    .execute(&mut storage)
    .await
    .unwrap();
    drop(storage);

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let mut copied = sqlx_sqlite::SqliteConnectOptions::new()
        .filename(
            base.path()
                .join("new-webkit/LocalStorage/localstorage.sqlite3"),
        )
        .read_only(true)
        .connect()
        .await
        .unwrap();
    let value: Vec<u8> = sqlx::query::query("SELECT value FROM ItemTable")
        .fetch_one(&mut copied)
        .await
        .unwrap()
        .get(0);
    let units: Vec<u16> = value
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    assert_eq!(
        String::from_utf16(&units).unwrap(),
        format!(r#"[{{"attachments":["{new}/agent-attachment-staging/a.png"]}}]"#)
    );
}

// --- Second review (J2 to J6) ------------------------------------------------

fn journal_for(plan: &MigrationPlan) -> PathBuf {
    let mut name = plan.data.target.file_name().unwrap().to_os_string();
    name.push(".identity-migration.journal");
    plan.data.target.with_file_name(name)
}

/// Fails every publication rename, standing in for a disk that refuses it.
struct RefusingRename;

impl FileCopier for RefusingRename {
    fn copy_file(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to).map(|_| ())
    }

    fn rename(&mut self, _from: &Path, _to: &Path) -> std::io::Result<()> {
        Err(std::io::Error::other("rename refused"))
    }
}

#[test]
fn a_failed_resume_blocks_and_keeps_the_recovery_state() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    let staging = staging_for(&plan, 1);
    write(
        &staging.join("notes.sqlite3"),
        b"staged by an interrupted launch",
    );
    let journal = serde_json::json!({
        "moves": [{ "staging": staging, "target": plan.data.target }],
        "credentials": [],
    });
    std::fs::write(journal_for(&plan), journal.to_string()).unwrap();

    let error = migrate(&plan, &MemoryStore::default(), &mut RefusingRename)
        .expect_err("the resume cannot publish");

    assert!(
        error.blocked,
        "a failed resume must stop the launch: {error}"
    );
    assert!(journal_for(&plan).exists());
    assert!(staging.join("notes.sqlite3").exists());
    assert!(is_empty_or_absent(&plan.data.target));
}

#[test]
fn an_unreadable_journal_blocks_without_being_removed() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    std::fs::write(journal_for(&plan), b"{\"moves\": [{\"stag").unwrap();

    let error = migrate(&plan, &MemoryStore::default(), &mut StdCopier)
        .expect_err("a torn journal is evidence, not garbage");

    assert!(error.blocked, "{error}");
    assert!(journal_for(&plan).exists());
    assert!(is_empty_or_absent(&plan.data.target));
}

#[tokio::test]
async fn structured_values_equal_to_the_root_and_external_look_alikes() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let db = plan.data.source.join("notes.sqlite3");
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let mut notes = pool.acquire().await.unwrap();
    let t = "2026-10-01T09:00:00Z";
    let args = serde_json::json!([old, "--verbose"]).to_string();
    exec(
        &mut notes,
        "INSERT INTO agent_mcp_servers(id, name, enabled, transport, command, args_json, metadata_json, tool_visibility_json, safety_json, created_at, updated_at) VALUES ('m1', 'files', 1, 'stdio', 'serve', ?, '{}', '{}', '{}', ?, ?)",
        &[&args, t, t],
    )
    .await;
    let external = format!("/Volumes/Backup{old}/project");
    exec(
        &mut notes,
        "INSERT INTO agent_sessions(id, workspace_path, created_at, updated_at) VALUES ('ext', ?, ?, ?)",
        &[&external, t, t],
    )
    .await;
    drop(notes);
    pool.close().await;

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let copied = plan.data.target.join("notes.sqlite3");
    let args: serde_json::Value =
        serde_json::from_str(&text(&copied, "SELECT args_json FROM agent_mcp_servers").await)
            .unwrap();
    assert_eq!(args, serde_json::json!([new, "--verbose"]));
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 'ext'"
        )
        .await,
        external,
        "a path that only contains the old folder stays"
    );
}

// --- Third review (K1 to K5) -------------------------------------------------

/// Publishes for real, then fails to flush the folder that names the first
/// published target.
struct FailingFlush {
    renames: usize,
}

impl FileCopier for FailingFlush {
    fn copy_file(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to).map(|_| ())
    }

    fn rename(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        self.renames += 1;
        std::fs::rename(from, to)
    }

    fn sync_directory(&mut self, path: &Path) -> std::io::Result<()> {
        if self.renames == 1 {
            return Err(std::io::Error::other("flush failed"));
        }
        super::sync_directory_now(path)
    }
}

#[test]
fn a_failed_flush_after_publishing_rolls_the_published_folder_back() {
    let base = tempfile::tempdir().unwrap();
    let mut plan = plan_with_webview(base.path());
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", "k")]);

    let error = migrate(&plan, &store, &mut FailingFlush { renames: 0 })
        .expect_err("the flush after the first publication fails");

    assert!(error.to_string().contains("flush failed"), "{error}");
    assert!(is_empty_or_absent(&base.path().join("new-webkit")));
    assert!(is_empty_or_absent(&plan.data.target));
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);
    assert!(!journal_for(&plan).exists());
}

#[test]
fn read_only_files_are_copied_and_stay_read_only() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    let object = plan
        .data
        .source
        .join("agent-workspaces/s1/repo/.git/objects/ab/cdef");
    write(&object, b"git object");
    std::fs::set_permissions(&object, std::fs::Permissions::from_mode(0o444)).unwrap();

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("a read-only file copies");

    let copied = plan
        .data
        .target
        .join("agent-workspaces/s1/repo/.git/objects/ab/cdef");
    assert_eq!(std::fs::read(&copied).unwrap(), b"git object");
    assert_eq!(
        std::fs::metadata(&copied).unwrap().permissions().mode() & 0o777,
        0o444
    );
}

#[tokio::test]
async fn only_known_path_fields_are_rewritten() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(plan.data.source.join("notes.sqlite3"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let mut notes = pool.acquire().await.unwrap();
    let t = "2026-10-01T09:00:00Z";
    let note_text = format!("Ran: cd {old}/agent-workspaces/s1 && ls");
    exec(
        &mut notes,
        "INSERT INTO notes(id, edited_content, created_at, updated_at) VALUES ('n1', ?, ?, ?)",
        &[&note_text, t, t],
    )
    .await;
    let sibling = format!("{old} (backup)/project");
    exec(
        &mut notes,
        "INSERT INTO agent_sessions(id, workspace_path, created_at, updated_at) VALUES ('s1', ?, ?, ?), ('s2', ?, ?, ?)",
        &[&format!("{old}/agent-workspaces/s1"), t, t, &sibling, t, t],
    )
    .await;
    drop(notes);
    pool.close().await;
    // A free-text file inside the profile that mentions the old folder.
    let memo = format!("The workspace lived in {old}/agent-workspaces/s1.");
    write(
        &plan.data.source.join("agent-workspaces/s1/notes.md"),
        memo.as_bytes(),
    );

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let copied = plan.data.target.join("notes.sqlite3");
    assert_eq!(
        text(&copied, "SELECT edited_content FROM notes").await,
        note_text
    );
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 's1'"
        )
        .await,
        format!("{new}/agent-workspaces/s1")
    );
    assert_eq!(
        text(
            &copied,
            "SELECT workspace_path FROM agent_sessions WHERE id = 's2'"
        )
        .await,
        sibling,
        "a sibling that only starts like the old folder stays"
    );
    assert_eq!(
        std::fs::read_to_string(plan.data.target.join("agent-workspaces/s1/notes.md")).unwrap(),
        memo
    );
}

#[test]
fn every_failed_migration_stops_the_launch() {
    let ordinary = MigrationError {
        message: "credential denied".into(),
        blocked: false,
    };
    assert!(stops_the_launch(&Err(ordinary)));
    assert!(stops_the_launch(&Err(MigrationError {
        message: "old app running".into(),
        blocked: true,
    })));
    assert!(!stops_the_launch(&Ok(Outcome::Skipped(
        Decision::TargetInUse
    ))));
    assert!(!stops_the_launch(&Ok(Outcome::Resumed)));
}

// --- Fourth review (L1 to L5) ------------------------------------------------

/// Publishes for real, then fails the flush after the data folder was
/// published and makes part of it impossible to remove, so the rollback
/// stops halfway.
struct StuckRollback {
    stuck: PathBuf,
}

impl FileCopier for StuckRollback {
    fn copy_file(&mut self, from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::copy(from, to).map(|_| ())
    }

    fn sync_directory(&mut self, _path: &Path) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&self.stuck, std::fs::Permissions::from_mode(0o555))?;
        Err(std::io::Error::other("flush failed"))
    }
}

#[test]
fn an_interrupted_rollback_blocks_the_next_launch_until_it_finishes() {
    use std::os::unix::fs::PermissionsExt;
    let base = tempfile::tempdir().unwrap();
    let mut plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    plan.credentials.push(activity_copy());
    let store = MemoryStore::with(&[("old.activity-db", "sqlcipher-key", "k")]);
    let stuck = plan.data.target.join("recordings");

    let first = migrate(
        &plan,
        &store,
        &mut StuckRollback {
            stuck: stuck.clone(),
        },
    )
    .expect_err("the flush fails and the rollback cannot finish");
    assert!(first.blocked, "{first}");
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);
    assert!(
        plan.data.target.exists(),
        "part of the published folder is left"
    );

    // The leftovers are not a finished publication.
    let second = migrate(&plan, &store, &mut StdCopier)
        .expect_err("an unfinished rollback blocks the next launch");
    assert!(second.blocked, "{second}");
    assert!(journal_for(&plan).exists());
    assert_eq!(store.value("new.activity-db", "sqlcipher-key"), None);

    // Once the leftovers can be removed, the rollback finishes and the
    // migration starts over.
    std::fs::set_permissions(&stuck, std::fs::Permissions::from_mode(0o755)).unwrap();
    let third = migrate(&plan, &store, &mut StdCopier).expect("the rollback finishes");
    assert!(matches!(third, Outcome::Migrated { .. }), "{third:?}");
    assert!(!journal_for(&plan).exists());
    assert_eq!(
        tree_hash(&plan.data.target),
        tree_hash(&plan.data.source),
        "the new profile is a whole copy"
    );
    assert_eq!(
        store.value("new.activity-db", "sqlcipher-key").as_deref(),
        Some("k")
    );
}

/// The module and its startup call exist only in a macOS development build.
const DEV_MACOS_ONLY: &str = "#[cfg(all(target_os = \"macos\", debug_assertions))]";

#[test]
fn the_migration_runs_only_on_macos() {
    let lib = include_str!("../lib.rs");
    assert!(
        lib.contains(&format!("{DEV_MACOS_ONLY}\npub mod identity_migration;")),
        "the module is macOS-only"
    );
    assert!(
        lib.contains(&format!(
            "{DEV_MACOS_ONLY}\n    identity_migration::run_at_startup();"
        )),
        "the startup call is macOS-only"
    );
    for (name, source) in [
        ("mod.rs", include_str!("mod.rs")),
        ("guard.rs", include_str!("guard.rs")),
        ("database.rs", include_str!("database.rs")),
    ] {
        for windows_only in [
            "cfg(windows)",
            "target_os = \"windows\"",
            "CredEnumerateW",
            "LockFileEx",
            "tasklist",
            ".exe\"",
            "local_data",
        ] {
            assert!(
                !source.contains(windows_only),
                "{name} still has Windows-only code: {windows_only}"
            );
        }
    }
}

#[test]
fn the_release_build_never_migrates() {
    let lib = include_str!("../lib.rs");
    assert_eq!(
        lib.matches("identity_migration").count(),
        2,
        "only the gated module and its gated call name the migration"
    );
    assert!(lib.contains(&format!("{DEV_MACOS_ONLY}\npub mod identity_migration;")));
    assert!(lib.contains(&format!(
        "{DEV_MACOS_ONLY}\n    identity_migration::run_at_startup();"
    )));
    // No release profile names are left to map: every plan reads `-dev`.
    let module = include_str!("mod.rs");
    for release_only in ["debug: bool", "cfg!(debug_assertions)", "(false, _)"] {
        assert!(
            !module.contains(release_only),
            "the migration still maps a release profile: {release_only}"
        );
    }
}

#[tokio::test]
async fn no_agent_run_stays_resumable_after_the_migration() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    let old = plan.data.source.to_string_lossy().into_owned();
    std::fs::create_dir_all(&plan.data.source).unwrap();
    let pool = sqlx_sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(
            sqlx_sqlite::SqliteConnectOptions::new()
                .filename(plan.data.source.join("notes.sqlite3"))
                .create_if_missing(true),
        )
        .await
        .unwrap();
    crate::db::migrations::run_migrations(&pool).await.unwrap();
    let mut notes = pool.acquire().await.unwrap();
    let t = "2026-10-01T09:00:00Z";
    // The SDK state nests the old workspace inside a serialized string.
    let state = serde_json::Value::String(
        serde_json::json!({
            "clovyVersion": 1,
            "sdkState": serde_json::json!({ "cwd": format!("{old}/agent-workspaces/s1") }).to_string(),
        })
        .to_string(),
    )
    .to_string();
    exec(
        &mut notes,
        "INSERT INTO agent_sessions(id, workspace_path, status, created_at, updated_at) VALUES ('s1', ?, 'waiting_for_user', ?, ?)",
        &[&format!("{old}/agent-workspaces/s1"), t, t],
    )
    .await;
    exec(
        &mut notes,
        "INSERT INTO agent_runs(id, session_id, status, model, started_at, updated_at, interrupted_state_json) VALUES ('waiting', 's1', 'waiting_for_user', 'auto', ?, ?, ?), ('done', 's1', 'completed', 'auto', ?, ?, ?)",
        &[t, t, &state, t, t, &state],
    )
    .await;
    let interruption =
        serde_json::json!({ "id": "approve-1", "kind": "approval", "status": "pending" })
            .to_string();
    exec(
        &mut notes,
        "INSERT INTO agent_items(id, session_id, run_id, sequence, kind, payload_json, created_at) VALUES ('i1', 's1', 'waiting', 0, 'interruption', ?, ?)",
        &[&interruption, t],
    )
    .await;
    drop(notes);
    pool.close().await;

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let copied = plan.data.target.join("notes.sqlite3");
    let count = |sql: &'static str| {
        let copied = copied.clone();
        async move { text(&copied, &format!("SELECT CAST(({sql}) AS TEXT)")).await }
    };
    assert_eq!(
        count("SELECT count(*) FROM agent_runs WHERE interrupted_state_json IS NOT NULL").await,
        "0"
    );
    assert_eq!(
        count("SELECT count(*) FROM agent_runs WHERE status = 'waiting_for_user'").await,
        "0"
    );
    assert_eq!(
        count("SELECT count(*) FROM agent_sessions WHERE status = 'waiting_for_user'").await,
        "0"
    );
    assert_eq!(
        count("SELECT count(*) FROM agent_items WHERE kind = 'interruption' AND json_extract(payload_json, '$.status') = 'pending'").await,
        "0"
    );
    assert_eq!(
        text(&copied, "SELECT status FROM agent_runs WHERE id = 'done'").await,
        "completed",
        "a finished run keeps its outcome"
    );
}

#[test]
fn hermes_import_paths_and_the_obsidian_vault_are_rewritten() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    let old = plan.data.source.to_string_lossy().into_owned();
    let new = plan.data.target.to_string_lossy().into_owned();
    let config = serde_json::json!({
        "model": "auto",
        "mcp_servers": {
            "files": {
                "command": format!("{old}/hermes/bin/files-server"),
                "args": [format!("{old}/hermes/data"), "--verbose", format!("{old} (backup)/data")],
            },
            "legacy": { "cmd": format!("{old}/hermes/bin/legacy"), "args": [] },
            "external": { "command": "/usr/local/bin/tool", "args": ["/tmp/x"] },
        },
    });
    write(
        &plan.data.source.join("hermes/config.yaml"),
        serde_yaml::to_string(&config).unwrap().as_bytes(),
    );
    let jobs = serde_json::json!({
        "jobs": [
            { "id": "j1", "script": format!("{old}/hermes/scripts/daily.sh"), "workdir": format!("{old}/hermes") },
            { "id": "j2", "script": format!("echo {old}/x"), "workdir": "/Users/someone/elsewhere" },
        ],
    });
    write(
        &plan.data.source.join("hermes/cron/jobs.json"),
        jobs.to_string().as_bytes(),
    );
    write(
        &plan.data.source.join("obsidian.json"),
        serde_json::json!({ "vaultPath": format!("{old}/vault") })
            .to_string()
            .as_bytes(),
    );
    let before = tree_hash(&plan.data.source);

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let config: serde_json::Value = serde_yaml::from_str(
        &std::fs::read_to_string(plan.data.target.join("hermes/config.yaml")).unwrap(),
    )
    .unwrap();
    let servers = &config["mcp_servers"];
    assert_eq!(
        servers["files"]["command"],
        format!("{new}/hermes/bin/files-server")
    );
    assert_eq!(
        servers["files"]["args"],
        serde_json::json!([
            format!("{new}/hermes/data"),
            "--verbose",
            format!("{old} (backup)/data")
        ])
    );
    assert_eq!(servers["legacy"]["cmd"], format!("{new}/hermes/bin/legacy"));
    assert_eq!(servers["external"]["command"], "/usr/local/bin/tool");
    assert_eq!(servers["external"]["args"], serde_json::json!(["/tmp/x"]));
    assert_eq!(config["model"], "auto");
    let jobs: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(plan.data.target.join("hermes/cron/jobs.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        jobs["jobs"][0]["script"],
        format!("{new}/hermes/scripts/daily.sh")
    );
    assert_eq!(jobs["jobs"][0]["workdir"], format!("{new}/hermes"));
    assert_eq!(
        jobs["jobs"][1]["script"],
        format!("echo {old}/x"),
        "an inline program is not a path"
    );
    assert_eq!(jobs["jobs"][1]["workdir"], "/Users/someone/elsewhere");
    let obsidian: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(plan.data.target.join("obsidian.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(obsidian["vaultPath"], format!("{new}/vault"));
    assert_eq!(tree_hash(&plan.data.source), before);
}

#[test]
fn an_obsidian_vault_outside_the_profile_stays() {
    let base = tempfile::tempdir().unwrap();
    let plan = dir_plan(base.path());
    old_profile(&plan.data.source);
    let vault = serde_json::json!({ "vaultPath": "/Users/someone/Documents/Vault" }).to_string();
    write(&plan.data.source.join("obsidian.json"), vault.as_bytes());

    migrate(&plan, &MemoryStore::default(), &mut StdCopier).expect("migrate");

    let obsidian: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(plan.data.target.join("obsidian.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(obsidian["vaultPath"], "/Users/someone/Documents/Vault");
}
