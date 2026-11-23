---
status: accepted
date: 2026-10-04
---

# The activity database uses SQLCipher, linked app-wide on macOS

## Context

Activity capture (`docs/activity-capture.md`) records window text, URLs, and
redacted clipboard text every 2 seconds. That is the most sensitive data Clovy
stores, so it lives in its own file, `activity.sqlite3`, encrypted at rest with
a random 32-byte key held in the Keychain. Users must be able to delete it, lose
it, or recreate it without touching notes.

Clovy already links SQLite through sqlx (`sqlx-sqlite` with `bundled`), and
Cargo resolves exactly one `libsqlite3-sys` per binary: any SQLite binding a
crate adds shares it, and features unify. There is no way to link SQLCipher for
one database and stock SQLite for another in the same process.

Alternatives considered:

- **Application-level encryption** (encrypt each text column with AES-GCM in
  Rust). Keeps stock SQLite, but leaks structure (row counts, timestamps, app
  names unless those are encrypted too), breaks indexes and the FTS5 search the
  timeline needs, and puts nonce/key handling in our code.
- **A second process** (a helper owning a SQLCipher build). Doubles lifecycle,
  IPC, and TCC identity for a database the app reads constantly.
- **SQLCipher for the whole binary.** One extra feature flag; SQLCipher without
  `PRAGMA key` behaves as plain SQLite, so `notes.sqlite3` keeps working.

## Decision

1. `src-tauri/Cargo.toml` adds `libsqlite3-sys` with
   `bundled-sqlcipher-vendored-openssl` under the macOS target only. Every
   SQLite connection in the macOS app, including the main `notes.sqlite3`
   pool, runs on SQLCipher; the main database never sets a key and is
   unaffected. Windows keeps stock bundled SQLite (activity capture is
   macOS-only, and vendored OpenSSL would add perl/nasm to that lane).
2. `activity.sqlite3` is opened only by `src-tauri/src/activity/store.rs`, with
   the raw key pragma first (sqlx orders `key` before every other pragma).
   Before creating the file the store checks `PRAGMA cipher_version` on an
   in-memory connection and refuses to run without SQLCipher, so no build can
   ever write a plaintext activity database.
3. The key is generated on first enable and stored in the Keychain service
   `co.opensoftware.clovy.activity-db` (`co.opensoftware.clovy-dev.activity-db`
   in debug builds). If the file exists and the key is missing or wrong, Clovy
   never deletes or rekeys on its own: capture reports "key missing" and the
   user may confirm recreating an empty database.

## Consequences

- The full `cargo test` of `src-tauri` (main-database migrations, repositories,
  recovery suites) is the regression gate for this flag: it must stay green on
  the SQLCipher build.
- Build time grows by the vendored OpenSSL and SQLCipher compile on a clean
  target. Release builds ship OpenSSL's libcrypto statically linked.
- Upgrading SQLite now means upgrading the SQLCipher release bundled by
  `libsqlite3-sys`; both pools move together.
- Any future plaintext database still works unchanged; any future encrypted one
  reuses the same mechanism with its own Keychain key.
