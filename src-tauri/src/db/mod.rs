pub mod migrations;
pub mod repositories;

/// Close SQLite workers, including returns racing SQLx 0.8's final idle drain.
pub(crate) async fn close_sqlite_pool(pool: &sqlx_sqlite::SqlitePool) {
    // SQLx 0.8 can release an idle connection's already-available permit
    // again while closing it. Its semaphore wait can then finish before
    // another SQLite worker has shut down. Drain until the live count is
    // zero; yield so asynchronous returns can finish, without file retries.
    loop {
        pool.close().await;
        if pool.size() == 0 {
            break;
        }
        tokio::task::yield_now().await;
    }
}
