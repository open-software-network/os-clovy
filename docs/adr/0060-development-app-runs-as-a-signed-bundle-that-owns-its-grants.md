---
status: accepted
date: 2026-10-08
---

# The development app runs as a signed bundle that owns its privacy grants

## Context

macOS keeps a privacy grant (Accessibility, Screen Recording, Microphone, and
the rest of TCC) for a client identity: a bundle identifier, or a path, plus
the designated requirement of the code that was granted. A grant applies to
the process macOS holds responsible for the request, and a process started
from a terminal inherits the terminal as responsible.

Until this decision, `pnpm tauri:dev` ran a hard-linked alias of Cargo's
`os-june` binary from the terminal. The app had no bundle to select in System
Settings, its requests were charged to whichever terminal or agent host
started it, and its ad hoc linker signature changed with every build. To give
Computer use a "fresh walkthrough", the launcher also ran `tccutil reset` on
every start: Accessibility for the debug helper and Screen Recording for the
app identifier, which on a normal branch is the release identifier
`co.opensoftware.june`. `build.rs` deleted and rebuilt the dictation and
system audio helper bundles on every build script run. Developers had to grant
permissions again after restarts and rebuilds.

## Decision

1. The dev launcher never resets privacy grants.
2. The Tauri dev runner (`scripts/cargo-clovy-runner.sh`) runs the app from a
   bundle beside Cargo's output, `target/<profile>/Clovy Dev.app`, with the
   packaged layout (`Contents/MacOS/os-june`) and the bundle identifier
   `<compiled identifier>.dev`, never the release identifier. The compiled
   identifier and the `-dev` data directory do not change. An unchanged Cargo
   output reuses the signed bundle; a changed one replaces it at the same path.
3. One code signing identity signs the bundle and every helper `build.rs` and
   `prepare-cua-driver.mjs` produce: `CLOVY_DEV_SIGNING_IDENTITY`, or the
   single valid Apple Development identity. An Apple Development requirement
   names the identifier and the certificate, not the code hash, so grants
   survive rebuilds. A missing or ambiguous identity stops the launch; ad hoc
   signing happens only when the developer sets `CLOVY_DEV_SIGNING_IDENTITY=-`.
   Unchanged helpers keep their bundles and signatures when the mode is the
   same; a switch between a certificate and an explicit `-` re-signs them in
   the new mode, while builds that select no identity keep any valid
   signature. In these debug builds the selected identity wins over an
   inherited `APPLE_SIGNING_IDENTITY`; release builds keep
   `APPLE_SIGNING_IDENTITY`.
4. Debug builds give every helper `<published identifier>.dev.w<worktree key>`
   (the first 12 hex digits of the SHA-256 of the checkout path): the
   Computer use driver (`scripts/computer-use-dev.mjs`), the dictation helper,
   and the system audio helper (`build.rs`). A development grant or signature
   never lands on an installed release or another worktree. Release builds
   keep the published identifiers.
5. The runner starts the bundle through `clovy-dev-launch`, which execs the app
   in place (`POSIX_SPAWN_SETEXEC`) with the terminal's responsibility
   disclaimed (`responsibility_spawnattrs_setdisclaim`). The app then requests
   and holds its own grants, while Tauri dev keeps supervising the same
   process id, environment, stdio, and exit status.
6. The Computer use helper's development parent check accepts only `os-june`
   in one of this checkout's Cargo profile directories, as Cargo's binary or
   as the main executable of a bundle directly inside that directory. The
   product-named hard-link aliases it accepted before are gone.
7. `CLOVY_DEV_APP_IDENTIFIER` gives a run its own compiled identity, data
   directory, and bundle. `build.rs` compiles the selected identifier into the
   crate, so the browser extension's native host (app and shim) accepts only
   the release descriptor or that identity's `-dev` descriptor.

## Alternatives considered

- **Only remove the reset.** Grants would still belong to the terminal or
  agent host that started the dev app, and nothing selectable would represent
  the app in System Settings.
- **Launch the bundle through LaunchServices (`open`).** Supported, and it
  makes the app responsible for itself, but the app is no longer a child of the
  runner. Tauri dev restarts by killing the runner, which would leave the old
  app running and let the single-instance plugin hand the new launch to it;
  the app would receive launchd's environment instead of the dev environment,
  and its stdio and exit status would not reach Tauri dev.
- **Keep ad hoc signatures.** An ad hoc requirement is the code hash itself,
  so every rebuild is a new identity to macOS.
- **Reuse the release identifier for the dev bundle.** A dev grant would
  replace the installed app's TCC entry, and LaunchServices could resolve the
  release identifier and its URL schemes to the dev bundle.
- **Keep the published identifiers for debug helpers.** A development grant
  or signature would replace the installed release's entry for that helper,
  and two worktrees would overwrite each other's.

## Consequences

- The first launch after this change, a new signing certificate, or a new
  identifier asks for each permission once; the launcher does not fake grants.
- `responsibility_spawnattrs_setdisclaim` is exported by libSystem but not
  documented. It is resolved at runtime and only in the development launcher;
  if a macOS release removes it, `pnpm tauri:dev` fails with a message rather
  than silently charging grants to the terminal. Release builds do not use it.
- Tauri resolves resources from `Contents/Resources` once the app runs from a
  bundle. The runner copies the browser extension files there; helpers keep
  their repository-relative debug paths. WebKit storage moves from
  `~/Library/WebKit/Clovy` to `~/Library/WebKit/<bundle id>`, copied once on
  first launch.

See [docs/development.md](../development.md#the-macos-development-app).

## Addendum - bundle resources and debug helper paths (2026-10-08)

This addendum supersedes the resources consequence above ("The runner copies
the browser extension files there; helpers keep their repository-relative
debug paths"). The rest of this ADR stands.

- The runner stages every resource in the common `bundle.resources` map of
  `tauri.conf.json` (today the browser extension files and agent skills) at
  its bundle path under `Contents/Resources`, and those trees are part of the
  bundle fingerprint, so a changed resource rebuilds the bundle.
- The dev bundle runs without Cargo's runtime environment, so a helper path
  read from a runtime `CARGO_MANIFEST_DIR` finds nothing. Debug builds resolve
  the dictation helper from this checkout at compile time, like the Computer
  use and system audio helpers. Release builds keep the runtime
  `CARGO_MANIFEST_DIR` fallback with the same precedence over the bundled
  candidates.
