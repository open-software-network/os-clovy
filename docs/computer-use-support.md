# Computer use support runbook

Computer use now packages the pinned agent-desktop engine behind the authenticated
Clovy helper on macOS and Windows. The source commit, protocol, target, reviewed
patch and actual signed executable digest are in the bundled
`agent-desktop-manifest.json`; upstream version 1.0.2 alone does not identify
this post-tag source. Source qualification and Apache-2.0 licensing are shipped
with the engine. The app never downloads a control engine at runtime.

Signed native acceptance remains pending for this cutover. The previous
macOS driver evidence does not establish agent-desktop behavior. Follow
[the native acceptance matrix](qa/computer-use-parity.md) with a fresh synthetic
profile and run-owned app identity before accepting a candidate.

Background semantic delivery is the default. An operation that cannot run in
background may use visible mode, as authorized by the owner; Clovy reports the
effective mode and still validates the exact window. A delivered or uncertain
action is not replayed in another mode. Visible mode grants no administrator,
UAC, TCC, login, password or OTP authority.

Computer use has independent local gates: Clovy's stored grant, the pinned
signed helper and engine, native access in an interactive session, and a visible attended CLI turn
([ADR-0067](adr/0067-computer-use-depends-only-on-local-gates.md): no plan,
Clovy vision model or remote rollout). Do not treat one as proof that the
others are enabled.

Names and identifiers below follow
[ADR-0068](adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md).

## State guide

| State | Meaning | Action |
| --- | --- | --- |
| `unsupported` | This platform is neither macOS nor Windows. | Use a supported native platform; the Linux engine smoke does not add Linux desktop support. |
| `off` | The Clovy grant is off. Native permissions may remain enabled. | Enable from Plugins after reading the permission explanation. |
| `driver_missing` | The signed app does not contain the pinned helper/stamp. | Reinstall or update Clovy. Do not run an upstream installer. |
| `driver_mismatch` | A helper, engine, source/patch/protocol, signature or compiled digest does not match the prepared build. | Hold the candidate and inspect its manifest and signed executable bytes. |
| `permission_missing` | A required native permission or readiness probe is unavailable. | On macOS, inspect the separate privacy grants; on Windows, use an unlocked, non-elevated interactive session. |
| `ready` | Every local gate is satisfied. | Start an attended task on an agent CLI with Clovy's MCP server. |
| `error` | The helper could not start, handshake, or probe. | Stop the task, inspect logs and signature, then run the self-test. |

## macOS recovery

1. Press Stop current task before changing permissions or reinstalling.
2. Confirm the intended helper exists in the installed app at
   `Contents/Resources/native/bin/Clovy Computer Use Driver.app` and has bundle
   identifier `dev.guilhermexp.clovy.computer-use-driver`. A packaged helper
   also requires the signed outer Clovy app with the same Developer ID team.
3. In System Settings > Privacy & Security, inspect Accessibility and Screen
   Recording separately. Accessibility names `Clovy Computer Use Driver`;
   Screen Recording names the outer `Clovy` app because macOS assigns capture to
   the responsible launcher. If either entry is absent, drag the matching card
   from Clovy into that pane: the helper card for Accessibility, or the outer
   Clovy card for Screen Recording. Removing a macOS grant may require Clovy to
   be quit and reopened before macOS reports the new state.
4. Return to Clovy. The visible setup page polls one signed-helper probe at a
   time while incomplete and reconfigures the runtime when the helper becomes
   capturable. Polling pauses while the page is hidden and refreshes when it
   becomes visible again.
5. If the driver crashed, Stop clears its private child and the next eligible
   task starts a new one. Never start an upstream daemon beside Clovy.
6. The first access to each target app asks once for access during the current
   task. Click Allow for this task. Captures and actions in that verified app do
   not ask again until the task ends or Stop is pressed. If Clovy later asks to
   open that same already-approved running app by display name, the broker
   reuses its exact verified identity instead of showing a second alias prompt.
7. A Stage Manager shelf window is restored automatically after the app is
   allowed. Clovy first activates its own window, then raises the exact target
   window into Clovy's current group without a separate approval. If restore
   fails, Clovy stops retrying that window for the current task. Press Stop and
   start a new task rather than switching Spaces manually.
8. If the target changed while an authorization card waited, ask Clovy to retry
   the app. A stale-target failure is expected safety behavior.

Turning the Clovy switch off removes Clovy's runtime grant and immediately stops
the task. It cannot silently remove macOS TCC entries; the user removes those
from System Settings.

## Local worktree permission testing

Every macOS `make dev` launch gives the debug Computer use helper a stable
bundle identifier derived from the current worktree path and signs it with the
same development certificate as `Clovy Dev.app`
([development.md](development.md#the-macos-development-app),
[ADR-0060](adr/0060-development-app-runs-as-a-signed-bundle-that-owns-its-grants.md)).
The launch reuses an unchanged helper bundle and never resets privacy grants:
Accessibility granted to the helper and Screen Recording granted to the outer
`Clovy Dev` app survive restarts and rebuilds. A new worktree, a new signing
certificate, or `CLOVY_DEV_SIGNING_IDENTITY=-` is a new identity for macOS and
asks again. To repeat the first-permission walkthrough on purpose, remove the
entries in System Settings, or run `tccutil reset Accessibility
dev.guilhermexp.clovy.computer-use-driver.dev.w<worktree>` and `tccutil reset
ScreenCapture dev.guilhermexp.clovy.dev` yourself.

The Tauri dev runner starts the app from
`target/<profile>/Clovy Dev.app/Contents/MacOS/clovy`. The authenticated
debug helper accepts only `clovy` in one of this checkout's Cargo profile
directories (marked by Cargo's `.cargo-lock`): Cargo's own binary, or the main
executable of a bundle directly inside that directory. Other names, deeper
bundles, and paths outside this checkout's target tree are refused. If the
Plugins surface reports that the driver stopped while the default self-test
passes, rerun the self-test through the exact live launcher to check this
boundary:

```sh
node scripts/computer-use-self-test.mjs --permissions-only \
  --host "src-tauri/target/debug/Clovy Dev.app/Contents/MacOS/clovy"
```

The production helper always keeps `dev.guilhermexp.clovy.computer-use-driver`
and its release signature.

## Build and release diagnosis

Run the deterministic gate after a normal macOS Rust build has signed the
staging helper:

```sh
pnpm computer-use:prepare
cargo build --manifest-path src-tauri/Cargo.toml --bin clovy --locked
pnpm computer-use:self-test
```

Preparation builds the engine from the pinned source lock plus its reviewed
patch, then builds and signs the helper. Ordinary macOS and Windows native
Cargo builds and tests require this preparation first. The helper-only
bootstrap flag is internal to that build; it never creates a placeholder
executable.

The gate checks the exact upstream commit, Clovy helper source fingerprint,
bundle metadata, declared architecture slices, signature, SPDX source record, Apache-2.0
notice, direct-launch rejection, authenticated Clovy-parent handshake, narrow
driver schemas, and Clovy MCP schema. A schema failure means the pin cannot be
upgraded without reviewing `computer_use.rs`, `computer_use_driver.rs`, and
`computer-use-contract.json` together.

After a signed universal app build, run the actual nested helper gate:

```sh
./scripts/computer-use-release-self-test.sh --live --target universal-apple-darwin
```

Live mode requires an interactive login plus Accessibility and Screen Recording
for the stable Developer ID helper. Set explicit `APPLE_SIGNING_IDENTITY` or
`CLOVY_DEV_SIGNING_IDENTITY` for the fixture; absent or ad hoc identities are
refused. The fixture must carry its exact signed fixture identifier and the
same non-ad-hoc Apple Team as the host. Renaming another app or changing its
Info.plist cannot satisfy that check. An ad hoc release supports static checks
only and an explicit live request fails clearly. On a new desktop release runner, use the
same command with `--prompt-permissions` passed directly to
`computer-use-self-test.mjs`, approve the two macOS dialogs, then rerun without
prompting. macOS does not provide a supported command that fabricates these
user grants.

The live fixture runs through the real signed Clovy executable's fixed QA host,
which accepts only the bundled helper and the two disposable fixture bundle
identifiers. It captures the target, clicks a numbered button while the observer
remains frontmost, verifies the real pointer and current-Space window flags did
not change, kills both fixtures, and exits the private driver. The manual
Stage Manager pass additionally verifies that the exact shelf window joins
Clovy's current group without moving the pointer or showing another decision.
The release workflows do not run this fixture; computer use is validated on the
release-candidate channel before promotion. Run it manually when validating a
release runner or diagnosing a signed-build regression.

## OS update or regression

- Run the live fixture on the oldest and newest supported macOS releases.
- Recheck both TCC states after an OS update; a preflight `true` with live
  capture `false` is treated as missing permission.
- Record driver version, macOS version, operation class, and failure class only.
  Never attach the screenshot, AX tree, typed value, app title, or approval text
  to analytics or an issue without explicit user attachment.
- If focus, pointer, Space, target binding, or unapproved-mutation behavior
  regresses, hold the promotion and keep the previous release live.

## Local capability policy

Computer use depends on local readiness and approval (ADR-0067). The retired
server rollout route is gone; there is no server override or deployment step.
Release decisions use the native proof and rollback criteria above.

## Windows preparation and proof

Set `WINDOWS_CERTIFICATE_THUMBPRINT` to a usable current-user signing
certificate, and put `signtool.exe` on PATH (or set `WINDOWS_SIGNTOOL_PATH`).
The helper, engine and Clovy host must all have valid Authenticode signatures
from that publisher. The helper also embeds the prepared engine digest at
compile time, and Clovy embeds the staged helper digest, so editing an adjacent
manifest cannot substitute an older signed executable. `scripts/build-windows.ps1` prepares and signs the nested
artifacts before Tauri signs the host and installer. It fails clearly when the
certificate is unavailable. An updater minisign key is a separate contract.

The staged `.tauri-helper/computer-use` directory is bundled at
`native/bin/computer-use`; it contains the helper executable, engine,
`agent-desktop-manifest.json`, the helper source stamp, Apache-2.0 notice and
source SPDX record. Native peer verification also binds the helper to the
expected parent process image and creation identity. Readiness requires an
unlocked, ordinary interactive desktop and a non-elevated process. There are
no macOS privacy buttons or UAC elevation path on Windows.

Window restoration and focus also qualify the target process token, so an
elevated or more privileged window stays unavailable even if Windows would
allow it to be raised. Opening an app requires a standard-user manifest and
compatible effective launch policy. Elevating, forced RunAsInvoker, unknown
or unreadable compatibility layers are refused before creation. The native
launch preserves the existing compatibility environment and uses Windows'
non-elevating process-creation path, with the qualified Explorer token/job
parent. It never retries through a UAC-capable shell operation. A successful
creation receipt carries image and creation identity from that exact process
handle; later failure remains unsafe to retry, and Stop does not close the
user's application. Opening an app manually does not authorize elevated
control. Signed native launch, integrity and job-inheritance acceptance is
still pending on Windows.

Run the deterministic self-test with the exact signed host and helper resource
directory:

```powershell
node scripts/computer-use-self-test.mjs --bundle <computer-use-resource-directory> --host <signed-clovy.exe>
node scripts/computer-use-self-test.mjs --live --bundle <computer-use-resource-directory> --host <signed-clovy.exe>
```

Without `--host`, discovery uses the native Cargo output under `CARGO_TARGET_DIR`
or `src-tauri/target`. For a build made with an explicit Cargo target, pass the
same target to the self-test, for example `--target x86_64-pc-windows-msvc`.
The self-test does not select a different architecture if that host is missing.

The live mode compiles the checked-in WinForms fixture through the installed
.NET Framework C# compiler, signs it with the same certificate, and stages it
at the fixed QA-only `native/fixtures/clovy-computer-use-fixture.exe` beside
the specified host. It does not ship that fixture in the production package.
The host also verifies the fixture's signed product/title markers, so renaming
another signed executable to the fixture filename is refused.
Use a fresh run-owned build directory: the test refuses to overwrite a prior
fixture. It proves two synthetic windows, an exact-window capture and a
semantic click without moving the pointer or foreground app. It preserves
its synthetic state directory as evidence. Text insertion, physical input,
mixed DPI, locked/secure desktops, identity reuse, Stop and Job Object teardown
still require the broader native acceptance matrix. These tests have not run
on the Linux implementation box.
