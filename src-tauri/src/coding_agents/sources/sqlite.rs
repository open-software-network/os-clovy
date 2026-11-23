//! Reading another app's SQLite store without touching it. Opening the live
//! file, even read-only, can make SQLite create or write `-wal`/`-shm` files
//! next to it, and `immutable=1` would miss rows still in the WAL. So the
//! database and its WAL are copied into a private temporary directory and the
//! copy is opened; the agent's files are only ever read.

use std::path::{Path, PathBuf};

use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::{SqliteConnectOptions, SqlitePool, SqlitePoolOptions, SqliteRow};

pub struct Snapshot {
    pool: SqlitePool,
    _dir: tempfile::TempDir,
}

impl Snapshot {
    pub async fn open(path: &Path) -> Result<Self, String> {
        let source = path.to_path_buf();
        let (dir, copy) = tokio::task::spawn_blocking(move || copy_database(&source))
            .await
            .map_err(|error| error.to_string())??;
        let options = SqliteConnectOptions::new()
            .filename(&copy)
            .foreign_keys(false)
            .create_if_missing(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .map_err(|error| error.to_string())?;
        Ok(Self { pool, _dir: dir })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn close(self) {
        crate::db::close_sqlite_pool(&self.pool).await;
    }
}

fn copy_database(source: &Path) -> Result<(tempfile::TempDir, PathBuf), String> {
    let dir = tempfile::Builder::new()
        .prefix("clovy-agent-store-")
        .tempdir()
        .map_err(|error| error.to_string())?;
    let copy = dir.path().join("store.db");
    copy_regular(source, &copy)?;
    let wal = PathBuf::from(format!("{}-wal", source.display()));
    match std::fs::symlink_metadata(&wal) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Ok(metadata) if metadata.file_type().is_file() => {
            copy_regular(&wal, &dir.path().join("store.db-wal"))?;
        }
        _ => return Err(format!("{} is not a regular file", wal.display())),
    }
    Ok((dir, copy))
}

/// Copies a file opened with `O_NOFOLLOW`, so a symlink is never followed.
fn copy_regular(from: &Path, to: &Path) -> Result<(), String> {
    let mut source = super::open_regular(from).map_err(|error| error.to_string())?;
    let mut target = std::fs::File::create(to).map_err(|error| error.to_string())?;
    std::io::copy(&mut source, &mut target).map_err(|error| error.to_string())?;
    Ok(())
}

/// A TEXT or BLOB column as text.
pub fn text_column(row: &SqliteRow, index: usize) -> Option<String> {
    row.try_get::<String, _>(index).ok().or_else(|| {
        row.try_get::<Vec<u8>, _>(index)
            .ok()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    })
}

pub async fn value_for_key(pool: &SqlitePool, sql: &str, key: &str) -> Option<String> {
    query(sql)
        .bind(key)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
        .and_then(|row| text_column(&row, 0))
}
