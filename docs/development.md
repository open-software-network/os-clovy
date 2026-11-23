# Local development

Day-to-day development reference for the desktop app.
Configuration details (environment, provider keys, and text engines)
live in [configuration.md](configuration.md); when the two disagree, the env
example files win.

## Checks in nested worktrees

Biome excludes the checkout's own `.tmp` with the root-relative `!!.tmp`
pattern. Keep this exclusion relative to the project root: `!!**/.tmp` also
prunes a checkout located under an ancestor `.tmp/worktrees`, causing
`pnpm check` to process zero files. The local temporary evidence directory
remains excluded while the source files are checked normally.

## Quick start

Clone the repo, copy the desktop env example, and run the app:

```sh
cp .env.example .env
sfw pnpm install
pnpm tauri:dev
```

`pnpm tauri:dev` starts or reuses Vite at `127.0.0.1:1421`; `VITE_PORT`
changes that port. Verify ownership before reusing an occupied port. It does
not start an inference server. Restart it after changing the root `.env`.

Replay first-run onboarding without wiping app data:

```sh
pnpm tauri:dev --replay-onboarding
```

The app has no sign-in or local bearer token. Chat uses installed CLI engines,
with Claude Code as default and Codex as the existing local fallback. Save
Groq, OpenRouter, and fal keys through Settings, never in the desktop `.env`.
Missing keys fail clearly without contacting another service. See
[configuration.md](configuration.md) for catalogs and note-engine selection.

## Local connector OAuth

Connector OAuth application identifiers belong in the root desktop `.env`.
They identify the installed client but do not grant access to user data by
themselves; user grants stay in Keychain.

Create a Google Desktop OAuth client and set `GOOGLE_OAUTH_CLIENT_ID` and
`GOOGLE_OAUTH_CLIENT_SECRET`. Restart `pnpm tauri:dev` after changing either
value.


## Local data

The app data directory is resolved by Tauri at runtime. In development, inspect
the platform app data path for:

- `notes.sqlite3`
- `recordings/{note_id}/{session_id}.wav`
- `recordings/{note_id}/{session_id}/microphone.wav`
- `recordings/{note_id}/{session_id}/system.wav` when `Microphone + system audio`
  is selected

Saved audio is the source of truth for retry. If transcription or generation
fails after capture, Clovy keeps the audio and processing metadata so work can be
retried without recording again.

## Agent skills

The agent loads skills from its managed `skills` folder and, when the folder
exists, from `~/.agents/skills` in your home directory (the same location the
`skills` CLI installs into). Drop a skill folder there and every agent session
picks it up the next time it starts. Home-folder skills load read-only: the
macOS write-jail grants writes only under Clovy's own data directory, so the
agent can use these skills but cannot modify them.

## Permissions

Clovy asks for permissions only where the feature needs them:

- **Microphone:** required for meeting notes and dictation.
- **Accessibility:** required for dictation paste into the previously focused
  app and for activity capture.
- **Screen and system audio recording:** required when using
  `Microphone + system audio` on macOS, for activity text recognition, and for
  Computer use captures.
- **File access:** requested by agent workflows when a task needs a specific
  scope.

The macOS bundle includes `NSMicrophoneUsageDescription`,
`NSAudioCaptureUsageDescription`, and `NSScreenCaptureUsageDescription` in
[src-tauri/Info.plist](../src-tauri/Info.plist).

### The macOS development app

`pnpm tauri:dev` (and `make dev`) runs the app from a signed bundle beside
Cargo's output, so macOS can show it in System Settings and keep its grants
across restarts and rebuilds
([ADR-0060](adr/0060-development-app-runs-as-a-signed-bundle-that-owns-its-grants.md)).
The launcher never resets permissions.

- **Bundle:** `src-tauri/target/debug/Clovy Dev.app`, bundle identifier
  `<identifier>.dev` (`dev.guilhermexp.clovy.dev` on a normal branch). The
  compiled Tauri identifier, and with it the `<identifier>-dev` data
  directory, does not change. A branch identity (`codex/jun-278-…`) or
  `CLOVY_DEV_APP_IDENTIFIER` gets its own bundle, for example
  `Clovy Dev permproof.app` for `dev.guilhermexp.clovy.permproof`. The browser
  profile belongs to that isolated app identity.
- **Bundle contents:** besides `clovy` and `clovy-mcp`,
  the bundle carries the resources `tauri.conf.json` declares for every
  platform (private browser resources, agent skills) at their bundle paths, and
  rebuilds when they change. The Computer use, dictation, and system audio
  helpers resolve from `.tauri-helper` at compile time, since the bundle runs
  without Cargo's runtime environment.
- **Signing identity:** `CLOVY_DEV_SIGNING_IDENTITY` (a SHA-1 or exact name
  from `security find-identity -v -p codesigning`), otherwise the single valid
  Apple Development identity. Several Apple Development identities, or none,
  stop the launch with the next step to take; nothing falls back silently.
  `CLOVY_DEV_SIGNING_IDENTITY=-` opts into ad hoc signing, which macOS ties to
  the exact code hash: every rebuild then asks for each permission again. In
  these debug builds the selected identity also wins over an
  `APPLE_SIGNING_IDENTITY` inherited from the shell; release builds keep
  signing with `APPLE_SIGNING_IDENTITY`.
- **One certificate everywhere:** the same identity signs `Clovy Dev.app`
  (with `clovy-mcp` beside `clovy` in
  `Contents/MacOS`) and the helpers in `.tauri-helper` (Computer use driver,
  dictation helper, system audio helper). Their designated requirement is
  `identifier "<bundle id>" and anchor apple generic and certificate
  leaf[subject.CN] = "<certificate>" and …`, so a rebuild changes the code
  hash but not the requirement macOS checks a grant against. Changing the
  certificate or the bundle identifier is a new identity for macOS.
- **Reuse:** an unchanged Cargo output reuses the signed bundle as is, and
  unchanged helpers keep their bundles and signatures. A changed binary
  replaces the bundle at the same path. Switching between a certificate and
  `CLOVY_DEV_SIGNING_IDENTITY=-` re-signs the app and the helpers in the new
  mode even when their content is unchanged. Plain `cargo build` and
  `cargo test`, which select no identity, keep any valid helper signature.
- **First authorization:** macOS asks once for each permission the dev app
  uses, or add `Clovy Dev.app` with + in System Settings > Privacy & Security.
  When the checkout lives in a protected folder (`~/Documents`, `~/Desktop`,
  `~/Downloads`), macOS may also ask for access to that folder, possibly before
  the main window appears. Check which request macOS actually shows and the
  state under Files and Folders; whether to allow it is the owner's decision.
  Folder access alone does not make the app ready. macOS may also ask once to
  let the newly signed app read an existing `-dev` Keychain item; that is an
  authorization to read the item, never an unlock or change of the Keychain,
  and Always Allow is recorded against the same requirement.
- **Web storage:** the unbundled app kept its WebKit storage in
  `~/Library/WebKit/Clovy`; the first bundled launch copies it to
  `~/Library/WebKit/<bundle id>` and leaves the original in place.

What macOS sees in development:

| Component | Identity | Started by |
| --- | --- | --- |
| `Clovy Dev.app` | `<identifier>.dev` | the runner, responsible for its own requests |
| `.tauri-helper/Clovy Dictation Helper.app` | `dev.guilhermexp.clovy.dictation-helper.dev.w<worktree>` | Clovy Dev, as a child process |
| `.tauri-helper/Clovy Audio Capture.app` (system audio) | `dev.guilhermexp.clovy.audio-capture.dev.w<worktree>` | Clovy Dev, through LaunchServices |
| `.tauri-helper/Clovy Computer Use Driver.app` | `dev.guilhermexp.clovy.computer-use-driver.dev.w<worktree>` | Clovy Dev, through LaunchServices |

`w<worktree>` is the first 12 hex digits of the SHA-256 of the checkout path,
the same key for the three helpers, so helper grants never land on an
installed release or on another worktree. Release builds keep the published
identifiers (`dev.guilhermexp.clovy.dictation-helper`,
`dev.guilhermexp.clovy.audio-capture`,
`dev.guilhermexp.clovy.computer-use-driver`). Requests Clovy Dev makes itself
(microphone for notes, Accessibility and Screen Recording for activity
capture, notifications) are listed as Clovy Dev. Computer use Accessibility
belongs to the driver and its Screen Recording to the outer responsible app,
Clovy Dev
([ADR-0028 addendum](adr/0028-private-stdio-broker-for-computer-use.md)).

macOS attributes a privacy request to the process responsible for it, and a
process started from a terminal inherits the terminal as responsible. The
runner therefore starts the bundle through `clovy-dev-launch`
([src-tauri/native/dev-launcher](../src-tauri/native/dev-launcher/clovy-dev-launch.c)),
which execs the app in place with the terminal's responsibility disclaimed.
The process id, arguments, environment, stdio, and exit status stay the ones
Tauri dev supervises. It uses `responsibility_spawnattrs_setdisclaim`, which
libSystem exports but Apple does not document; if a macOS release removes it,
the launch fails with a message instead of inheriting the terminal's grants.

To reset a development grant on purpose, name the dev bundle, never the
release identifier:

```sh
tccutil reset Microphone dev.guilhermexp.clovy.dev
```

## Verification commands

```sh
pnpm check
pnpm typecheck
pnpm test
pnpm test:rust
pnpm build
pnpm tauri:build
```

`make verify` mirrors CI.

Useful validation docs:

- [specs/001-tauri-note-mvp/manual-validation.md](../specs/001-tauri-note-mvp/manual-validation.md)
- [specs/002-system-audio-source-mode/quickstart.md](../specs/002-system-audio-source-mode/quickstart.md)
- [specs/003-conversation-turns/quickstart.md](../specs/003-conversation-turns/quickstart.md)

## Dictation destination HUD verification

The native path is `src-tauri/native/mac-dictation-helper/main.swift`
(`FocusTargetController` and `DictationController`) →
`src-tauri/src/dictation.rs` (take correlation and `latest_hud_snapshot`) →
`src/hud.ts` / `hud.html` / `src/styles/hud.css`. The existing
`src-tauri/src/activity/app_icon.rs` command resolves local PNG icons, through
`activityAppIcon` in `src/lib/activity-timeline.ts`; custom app commands use
the existing Tauri command registration, so no new window capability is
required. `focus_target` is visual metadata, not a workflow start or finish.
The replay snapshot retains a listening lifecycle across meter/focus updates
and carries the pinned target forward through processing. No per-level icon
lookup or PNG logging is added. See [ADR-0014](adr/0014-pinned-dictation-paste-target.md).

A live `final_transcript` without `target` may supersede the startup snapshot.
In that case `paste_target` can still hydrate the authoritative destination.
Focus updates are accepted only during listening, so this hydration window
cannot replace the destination with an unrelated foreground app. An explicit
target (including null) or a previously pinned destination stays authoritative.

`src/test/hud-target.test.ts` tests the actual HUD markup and consumer across
candidate changes, stop, paste, error, cancellation, delayed icons and legacy
fallback. `i18n-hud.test.ts` covers labels and late-subscriber hydration;
`hud-target-hydration.test.ts` covers a missed stop snapshot followed by live
transcript/paste events without changing the original RED witness;
`dictation.rs` tests snapshot reduction. Browser smoke with a controlled Tauri
bridge proves rendering and control requests only, not Swift focus, Fn,
permissions or paste. Keep screenshots and the bridge script in the run's
ignored evidence folder; never ship the shim.

Native proof belongs to the operator of an isolated debug identity. Build,
but do not launch, with the official Node 24 PATH and a run-owned identifier:

```sh
PATH="$HOME/.local/share/node-official/node24/bin:/opt/homebrew/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
  pnpm tauri:build -- --debug --bundles app \
  --config src-tauri/tauri.macos.conf.json \
  --config '{"identifier":"dev.guilhermexp.clovy.targetproof20261012"}'
```

This compiles a separate data identity; editing Info.plist is insufficient.
The release build receipt is packaging evidence, not the binary to open for
native proof. Follow AGENTS → Verification: evidence-owned copy of the bundle,
normal HOME, isolated copied profile excluding live MCP sockets, and `lsof`
confirmation of that exact process's database paths before interactions. The
worker does not copy the owner's profile or open any native app. The operator
must stop if protected-folder, microphone, Accessibility or Keychain access is
missing; only the owner decides those prompts. Never alter TCC or Keychains.

A separate bundle/data identity does not isolate global Fn listeners. The
owner's dev helper and the proof helper can react to the same key. Before Fn
proof, the operator must isolate shortcut activation using supported controls,
or temporarily suspend only the dev instance started by Main for this proof.
Record which exact process/listener was suspended and its restoration; do not
stop an unrelated owner's instance or assume the proof bundle owns Fn. Restore
the prior activation/dev state after the run. The worker neither suspends dev
processes nor exercises Fn. If activation cannot be isolated within this
authority, record the native Fn proof as blocked.

Use two run-owned scratch text destinations and a third app for focus drift:

1. With existing permissions and shortcut activation isolated as above,
   start Fn dictation in A. Observe A's real icon/name on the first useful HUD
   frame; switch to B while listening and observe B. Record helper events with
   the same takeId, without audio, transcript or credential content in evidence.
2. Release Fn or click Stop in B, then focus C. Observe the correlated
   `finalizing_transcript.target` for B, and B beside the processing spinner.
   If a local provider stub is used to avoid remote inference, label it as
   such; it must return synthetic text through the real transcription path,
   never emit helper/HUD events or paste commands as a substitute for proof.
   Without an authorized configured route, full processing/paste remains
   unproven. Do not change provider settings in the owner's profile.
3. If authorized transcription is available, verify only B receives the text,
   and `paste_target` and the visible indicator agree. Close B after stop in a
   separate take: observe B on the error, clipboard recovery, and no paste in C.
4. Cancel with Esc and the HUD cancel button in separate takes. Start a new
   take in A immediately; verify no B icon or delayed delivery reappears.
   Check drag, Stop, Esc hint, EN/pt-BR tooltip and accessible destination name.
5. Capture the real HUD and read back the isolated take outcome using the
   app's own supported export/read path. Check run-owned autostart cleanup by
   exact ProgramArguments, and stop only this run's processes. Preserve the
   evidence report's explicit limits (native, liveGroq, Keychain, deployment).

A fresh debug identity may be blocked on permissions; that is a recorded
blocker, not permission to use release data or synthesize native success.

## Releases

There are no published releases or release workflows (removed by the
`desvincular-repo-ci` change). To build and validate the desktop app locally,
see [release-macos.md](release-macos.md) and
[release-windows.md](release-windows.md).

Chat uses installed CLI engines and the bundled `clovy-mcp` relay. Run
`cargo test chat_engine`, `cargo test mcp_server`, and `cargo test approval_broker`
when changing that path, then the full `make verify` gate. Session persistence
lives in `src-tauri/src/agent_sessions/`; no JavaScript agent executable is
built or packaged. Native proof uses an isolated run-owned debug identity and
the data-safety rules in `AGENTS.md`. A CLI login and authorized model usage
are separate from a successful desktop build.

### Pinned browser resources

`pnpm browser:prepare` builds the private `clovy-agent-browser` executable and
prepares Chrome for Testing in `resources/agent-browser/<platform>/`. The dev
launcher runs it automatically. Source commit, typed host protocol, local patches,
Chrome version, official archive URLs and SHA-256 digests are fixed in
`scripts/agent-browser/manifest.json`; preparation does not resolve a latest release.
The source build uses the upstream lockfile and includes the Apache-2.0 license,
Chrome ABOUT notice, dependency SBOM and per-file provenance in the resource tree.
The build cache is `.tauri-agent-browser/` and can be regenerated from the pins.
Linux builds use the shared ordered heavy-build locks and require 20 GiB free.

The packaged app resolves `native/agent-browser/<platform>/` in its Tauri resources.
The debug adapter may resolve the prepared repository resources. There is no
runtime download or global executable lookup. Plain debug Cargo checks create
only the empty resource directory; they do not prepare a usable browser.
Use `pnpm browser:prepare -- --check` to verify a prepared debug resource tree.

Persistent macOS browser proofs are blocked before launch until native storage
has a supported development namespace. The pinned CFT source uses Chromium's
default Safe Storage service/account; a separate profile directory, bundle id
or ad-hoc signature does not establish the required `-dev` Keychain service.
Do not launch it to discover or create an owner's item, change Keychains, or
substitute mock/basic storage for a persistent profile. The supported namespace
integration requires a separate owner-coordinated decision and native proof.
Anonymous run-owned browser tests use ephemeral profiles and do not qualify
persistent native storage or account login.
On macOS the dev launcher signs nested Chrome code and the private executable
with the same selected development identity as the app.
