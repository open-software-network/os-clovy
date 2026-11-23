---
status: accepted
date: 2026-10-21
supersedes: 0055
---

# Clovy takes its own identity with a one-time migration

## Context

ADR-0055 kept the June-era technical identities (`co.opensoftware.june`,
`June.app`, `os-june`, the `co.opensoftware.*` credential services and native
messaging hosts, the Web Store extension id) immutable, and bridged everything
else through dual reads and dual writes, because released June builds and their
users had to keep working through upgrades and rollbacks.

This fork has no published users: its only install is the owner's. It has
already left every Open Software service (no OS Accounts, billing, updater,
companion or telemetry), so a rollback to a released June build is not a path
anyone takes. The remaining June identity is pure cost: every new feature has to
reason about two names, and the app still presents a third party's identifier
to macOS, Windows, browsers and credential stores.

## Decision

- The app is `dev.guilhermexp.clovy` (development bundle
  `dev.guilhermexp.clovy.dev`, development profile `dev.guilhermexp.clovy-dev`).
  It ships as `Clovy.app` with executable `clovy` (`clovy.exe` on Windows),
  helpers `Clovy Computer Use Driver.app`, `Clovy Dictation Helper.app`,
  `Clovy Audio Capture.app`, `clovy-dictation-helper`, `clovy-nm-shim`, helper
  identifiers `dev.guilhermexp.clovy.<helper>`, the login item "Clovy", and the
  owner as publisher and author. Only `clovy://` is registered.
- `src-tauri/src/app_identity.rs` is the one place Rust spells the identifier.
  Credential services are `<compiled identifier>[-dev].<name>`, so a proof
  build with its own `CLOVY_DEV_APP_IDENTIFIER` never shares credentials with
  the owner's profile.
- The browser extension carries the owner's own key pair: the manifest `key` is
  its public half (extension id `cmfpkfaibipkocbjcpgamaednndpopcp`); the
  private half stays outside the repository. The native host is
  `dev.guilhermexp.clovy.extension`.
- The first launch under the new identity runs `identity_migration` before the
  Tauri builder (Tauri creates windows, and so the WebView store, before the
  setup hook). When the new data directory is absent or empty and the June-era
  one exists, it copies the data and config directories, the WebView store and
  every credential item (including the SQLCipher key of `activity.sqlite3`)
  into the new names. The copy is staged beside the target and verified by
  file count and size. Databases travel as `VACUUM INTO` snapshots (the
  SQLCipher one with the copied key) taken from a private scratch copy, so
  the source is never opened. Only the places where the app stores paths into
  its own profile are rewritten: a fixed list of columns (recordings, audio
  artifacts, agent workspaces and artifacts, MCP server command and
  arguments, coding-agent cwd), JSON path keys (attachments, workspace,
  path), the WebKit queued follow-ups, `extension-host-path` and symlink
  targets, and only when the value is the old root or the root followed by a
  separator. Notes, transcripts, chat messages, captured activity and any
  other file keep their text, even where it mentions the old folder. The
  credentials are written, refusing a target credential that
  already holds a different value, and only then are the staged folders
  renamed into place, the data folder last, under a journal that lets the
  next launch finish or undo an interrupted publication; staging, journal and
  folders are flushed to disk before publishing. A resume that cannot finish,
  or a journal that cannot be read, stops the launch and keeps everything as
  it is. An operating-system lock per target (flock, LockFileEx) keeps two
  launches apart. Nothing is copied while the old app runs. Any failure
  removes everything the run created (unless a journal must be kept to
  finish it) and stops the launch with an alert, so the new profile stays
  empty and nobody starts using it by mistake. The source is never moved,
  changed or deleted.
- The June-era names survive only as migration sources: the source list in
  `identity_migration`, and one-way reads of a June-era persona file, Obsidian
  skill copy or skill setting row. Nothing June-era is written again. The
  rollback bridges of ADR-0055 (credential, browser storage, persona, skill,
  environment variables, IPC aliases, `june-nm-shim`, the June native host) are
  removed.

## Consequences

- The owner grants the macOS permissions again to the new app and helpers,
  re-pastes the MCP configuration (the data directory path changed), removes
  the old extension and loads the new id, and re-enables launch at login if it
  was on.
- A development build migrates the owner's `co.opensoftware.june-dev` profile
  only under the default identifier. Proofs use their own identifier plus the
  debug-only `CLOVY_DEV_IDENTITY_MIGRATION_SOURCE`, which names a synthetic
  source profile and never falls back to the June one.
- A migration that fails stops the launch with an alert and leaves the new
  profile empty and the June-era one intact; the error is logged beside the
  new folder (`<identifier>[-dev].identity-migration.log`). The owner fixes the
  cause and relaunches. Only when the log names a conflicting credential does
  the owner delete that new `<identifier>[-dev].*` item; with a journal
  present, nothing is cleaned up by hand.
- Released June-era wire and persisted names that are not identity (Clovy API
  headers and error codes, toolset ids such as `june_gmail`, the
  `__june_*__` model prefixes) are out of scope and stay as they are.

## Addendum (2026-10-21): macOS only, recorded rollbacks, paused runs

- **The migration runs only on macOS.** The owner has no June-era profile on
  Windows, so Clovy on Windows always starts with a new profile. The module
  and its startup call compile only on macOS; the Windows credential listing,
  `LockFileEx` lock, `tasklist` checks, WebView2 folder and backslash path
  handling were removed. The lock is `flock`. This also retires a Windows-only
  defect: the SQL prefilter searched for the raw root while JSON stores it
  escaped (`C:\\Users\\…`), so listed JSON fields were never rewritten there.
- **A rollback is recorded before anything is removed.** A failed run writes
  the rollback (the folders it created and the credentials it wrote) to the
  journal, flushed like the rest of it, and only then deletes. The next launch
  finishes a recorded rollback, or stops if it cannot remove everything; the
  leftovers of a rollback are never taken for a finished publication. A
  publication that cannot be finished (a staged folder is gone) is rolled back
  the same way. If the rollback cannot be recorded, nothing is removed and the
  launch stops with the publication journal intact.
- **Paused agent runs are not resumed after the migration.** Their serialized
  SDK state nests paths to the old profile inside an opaque string, which is
  not rewritten. The staged notes database expires pending interruption cards,
  marks waiting runs and sessions `interrupted` (`identity_migrated`) and
  clears every `interrupted_state_json`; the routine projection follows at the
  next startup's reconciliation.
- **The path list also covers the legacy Hermes import and Obsidian.**
  `hermes/config.yaml` (each MCP server's `command`/`cmd` and `args`),
  `hermes/cron/jobs.json` (each job's `script` and `workdir`) and
  `obsidian.json` (`vaultPath`), with the same rule: only a value that is the
  root or starts with the root and a separator. A rewritten YAML file loses
  its comments and formatting; a file its reader could not parse stays as it
  is.

## Addendum (2026-10-22): only the development build migrates

The owner's own release (`release-propria`) starts with an empty profile: the
production June profile has been idle since September and is not carried over.
`identity_migration` and its startup call are compiled only under
`cfg(all(target_os = "macos", debug_assertions))`, and the plan maps only the
`-dev` names (`co.opensoftware.june-dev` to `dev.guilhermexp.clovy-dev`, the
`.dev` WebKit store, the `-dev`/`.dev` credential services). A release binary
contains no migration code and never reads `co.opensoftware.june`. Guarded by
`the_release_build_never_migrates`.
