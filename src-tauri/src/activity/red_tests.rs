//! Behavioral tests kept in their own file so the test text stays fixed while
//! the implementation it guards changes (the ticket's RED/GREEN receipts hash
//! this file).

use chrono::{TimeZone, Utc};

use super::key::MemoryKeyStore;
use super::store::{ActivityStore, NewFrame, TextSource, ACTIVITY_DB_FILE};

/// The activity database is unreadable without its Keychain key: a plain
/// SQLite client sees "file is not a database", no captured text appears in
/// the file, and the same key opens it again.
#[tokio::test]
async fn encrypted_file_is_unreadable_without_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join(ACTIVITY_DB_FILE);
    let keys = MemoryKeyStore::default();
    let store = ActivityStore::open(&path, &keys).await.expect("open");
    store
        .insert_frame(&NewFrame {
            captured_at: Utc.with_ymd_and_hms(2026, 9, 1, 10, 0, 0).unwrap(),
            app_name: "Zed".into(),
            bundle_id: Some("dev.zed.Zed".into()),
            window_title: Some("main.rs".into()),
            browser_url: None,
            text_source: TextSource::Accessibility,
            text: Some("secret meeting notes".into()),
            input_idle_ms: None,
        })
        .await
        .expect("insert");
    store.close().await;

    // A plain SQLite client (no PRAGMA key) must not recognize the file.
    let plain = sqlx_sqlite::SqliteConnectOptions::new().filename(&path);
    let plain_read = match sqlx_sqlite::SqlitePool::connect_with(plain).await {
        Ok(pool) => sqlx::query::query("SELECT count(*) FROM frames")
            .fetch_one(&pool)
            .await
            .map(|_| ()),
        Err(error) => Err(error),
    };
    let error = plain_read.expect_err("plain SQLite must not read the activity database");
    assert!(
        error.to_string().contains("file is not a database"),
        "unexpected error: {error}"
    );
    let raw = std::fs::read(&path).expect("read file");
    assert!(!raw.starts_with(b"SQLite format 3"));
    assert!(!raw.windows(6).any(|window| window == b"secret"));

    // On Windows, also use the operating system's stock SQLite DLL, not
    // sqlx's app-wide SQLCipher linkage.
    #[cfg(target_os = "windows")]
    assert_stock_windows_sqlite_rejects(&path);

    // The same key reopens it.
    let reopened = ActivityStore::open(&path, &keys).await.expect("reopen");
    assert_eq!(reopened.frames_after(0, 10).await.unwrap().len(), 1);
}

#[cfg(target_os = "windows")]
fn assert_stock_windows_sqlite_rejects(path: &std::path::Path) {
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::os::windows::ffi::OsStrExt;

    // Importing winsqlite3.lib would collide with the app's SQLite symbols.
    // Resolve the system DLL's own exports instead, without app linkage.
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        fn GetProcAddress(module: *mut c_void, name: *const c_char) -> *mut c_void;
        fn FreeLibrary(module: *mut c_void) -> c_int;
    }
    struct Library(*mut c_void);
    impl Drop for Library {
        fn drop(&mut self) {
            // SAFETY: this handle comes from the successful LoadLibraryExW below.
            unsafe { FreeLibrary(self.0) };
        }
    }
    type Open = unsafe extern "C" fn(*const c_void, *mut *mut c_void) -> c_int;
    type Prepare = unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        c_int,
        *mut *mut c_void,
        *mut *const c_char,
    ) -> c_int;
    type Finalize = unsafe extern "C" fn(*mut c_void) -> c_int;
    type Close = unsafe extern "C" fn(*mut c_void) -> c_int;

    let dll_name: Vec<u16> = "winsqlite3.dll".encode_utf16().chain(Some(0)).collect();
    // SAFETY: null-terminated name; LOAD_LIBRARY_SEARCH_SYSTEM32 refuses DLLs
    // planted beside the test binary or in the working directory.
    let dll = unsafe { LoadLibraryExW(dll_name.as_ptr(), std::ptr::null_mut(), 0x800) };
    assert!(!dll.is_null(), "Windows stock SQLite DLL must be available");
    let library = Library(dll);
    let symbol = |name: &CStr| {
        // SAFETY: library stays loaded until every SQLite handle is closed.
        let address = unsafe { GetProcAddress(library.0, name.as_ptr()) };
        assert!(!address.is_null(), "missing SQLite export {name:?}");
        address
    };
    // SAFETY: these signatures match the documented stock SQLite C exports.
    let open: Open = unsafe { std::mem::transmute(symbol(c"sqlite3_open16")) };
    let prepare: Prepare = unsafe { std::mem::transmute(symbol(c"sqlite3_prepare_v2")) };
    let finalize: Finalize = unsafe { std::mem::transmute(symbol(c"sqlite3_finalize")) };
    let close: Close = unsafe { std::mem::transmute(symbol(c"sqlite3_close")) };
    let filename: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut db = std::ptr::null_mut();
    // SAFETY: buffers live through the calls; the same DLL owns and frees
    // every handle, and stays loaded until after they have been freed.
    unsafe {
        let opened = open(filename.as_ptr().cast(), &mut db);
        if opened != 0 {
            if !db.is_null() {
                close(db);
            }
            panic!("stock SQLite could not open the test file: {opened}");
        }
        let mut statement = std::ptr::null_mut();
        let result = prepare(
            db,
            c"SELECT count(*) FROM frames".as_ptr(),
            -1,
            &mut statement,
            std::ptr::null_mut(),
        );
        if !statement.is_null() {
            finalize(statement);
        }
        close(db);
        assert_eq!(result, 26, "stock SQLite must report SQLITE_NOTADB");
    }
}
