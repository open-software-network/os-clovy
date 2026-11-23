//! Process-local identity for custody associated with one SQLite partition.
use super::partition_file::partition_file_identity;
use sqlx::{query::query, row::Row};
use sqlx_sqlite::{SqliteConnectOptions, SqlitePool};
use std::{fs::File, sync::Arc};

pub(super) struct CandidatePartition {
    options: Arc<SqliteConnectOptions>,
    // Retain the open object to prevent its native identity from being reused.
    file: Option<File>,
    identity: Option<[u64; 2]>,
    memory: bool,
}
impl CandidatePartition {
    pub(super) async fn new(pool: &SqlitePool) -> Self {
        let options = pool.connect_options();
        // SQLite itself reports an empty main path for in-memory databases;
        // connect options alone can name a file used only as a memory URI.
        let name = query("PRAGMA database_list")
            .fetch_all(pool)
            .await
            .ok()
            .and_then(|rows| {
                rows.into_iter()
                    .find(|row| row.try_get::<String, _>("name").ok().as_deref() == Some("main"))
            })
            .and_then(|row| row.try_get::<String, _>("file").ok());
        let memory = name.as_deref() == Some("");
        let file = name
            .filter(|name| !name.is_empty())
            .and_then(|name| File::open(name).ok());
        let identity = file
            .as_ref()
            .and_then(|file| partition_file_identity(file).ok());
        Self {
            options,
            file,
            identity,
            memory,
        }
    }
    pub(super) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.options, &other.options)
            || (self.file.is_some()
                && other.file.is_some()
                && self.identity.is_some()
                && self.identity == other.identity)
    }
    pub(super) fn definitely_different(&self, other: &Self) -> bool {
        if self.same(other) {
            return false;
        }
        (self.memory && other.identity.is_some())
            || (other.memory && self.identity.is_some())
            || (self.identity.is_some() && other.identity.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    #[tokio::test]
    async fn partition_identity_tracks_real_database_object_and_separates_memory_pools() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.sqlite");
        let other = dir.path().join("other.sqlite");
        let open = |path: &std::path::Path| {
            SqliteConnectOptions::new()
                .filename(path)
                .create_if_missing(true)
        };
        let pool_a = SqlitePoolOptions::new()
            .connect_with(open(&first))
            .await
            .unwrap();
        let pool_b = SqlitePoolOptions::new()
            .connect_with(open(&other))
            .await
            .unwrap();
        let a = CandidatePartition::new(&pool_a).await;
        let b = CandidatePartition::new(&pool_b).await;
        assert!(a.same(&CandidatePartition::new(&pool_a.clone()).await));
        assert!(!a.same(&b));
        assert!(a.definitely_different(&b));
        pool_a.close().await;
        let reopened = SqlitePoolOptions::new()
            .connect_with(open(&first))
            .await
            .unwrap();
        assert!(a.same(&CandidatePartition::new(&reopened).await));
        #[cfg(unix)]
        {
            reopened.close().await;
            std::fs::rename(&first, dir.path().join("moved.sqlite")).unwrap();
            let replacement = SqlitePoolOptions::new()
                .connect_with(open(&first))
                .await
                .unwrap();
            assert!(!a.same(&CandidatePartition::new(&replacement).await));
        }
        let memory_a = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let memory_b = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .unwrap();
        assert!(!CandidatePartition::new(&memory_a)
            .await
            .same(&CandidatePartition::new(&memory_b).await));
        pool_b.close().await;
        let unavailable = CandidatePartition::new(&pool_b).await;
        assert!(!a.same(&unavailable));
        assert!(!a.definitely_different(&unavailable));
    }
}
