# Building Clovy for macOS

Releases come from `.github/workflows/release.yml`
([ADR-0069](adr/0069-the-owner-publishes-own-releases-with-a-one-channel-updater.md)):
by default, a tag `vX.Y.Z` (or a manual run naming it) builds the universal Mac
app and the Windows installer, signs the updater packages with the owner's key and publishes
a GitHub release on `guilhermexp/os-clovy` with `Clovy_universal.dmg`,
`Clovy.app.tar.gz`, its `.sig`, the Windows `*-setup.exe`, its `.sig` and
`latest.json`. A manual run defaults to `dry_run=true`: it builds and keeps the
files as run artifacts and publishes nothing.

The build is `Clovy.app` (bundle identifier `dev.guilhermexp.clovy`,
executable `clovy`), per
[ADR-0068](adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md).
`node scripts/verify-macos-app-name.mjs <path-to-Clovy.app>` checks the bundle
name, identifier, executable and the name Finder shows. A release starts with
an empty profile: only a development build migrates the June-era dev profile.

## Publishing a release

1. On `main`: `pnpm version:bump X.Y.Z`, commit `release: vX.Y.Z`.
2. Push the tag `vX.Y.Z`. The `version` job fails before any build when the tag
   differs from the four version files or a release for it already exists.
3. When all selected builds pass, `publish` creates the release. A failed or
   cancelled required job publishes nothing; fix it before retrying.

The non-secret repository variable `CLOVY_RELEASE_PLATFORMS` selects `all`
(the default when unset) or `macos`. The `version` job captures and validates it
once. Windows job selection, publication eligibility and manifest generation
all use that validated output, even if the variable changes during the run.
Unknown values stop the workflow before building. In `all`, both platform jobs
must succeed. In `macos`, macOS must succeed and Windows must be skipped;
Windows certificate preparation does not run. Manual `dry_run=true` still
builds and keeps artifacts without publishing.

Version 0.1.6 is temporarily selected for macOS only while Windows awaits a
valid signing certificate. Before triggering that publication, the owner records
the previous value (or absence) of `CLOVY_RELEASE_PLATFORMS`, selects `macos`,
and restores the recorded state after the run. This is a CI selection, with no
change to signing requirements. The release contains `Clovy_universal.dmg`,
`Clovy.app.tar.gz`, its `.sig` and `latest.json` with exactly `darwin-aarch64`
and `darwin-x86_64`. It contains no new Windows installer or Windows manifest
entry, and never relabels a previous Windows package as 0.1.6.

For local metadata generation, `scripts/release-manifest.mjs --platforms macos`
requires all three Mac artifacts. Omitting the option uses `all` and still
requires the Windows installer and its signature; missing artifacts never
silently reduce the selected platforms. Updater signing remains required in CI.

The macOS job installs both Apple target standard libraries for stable and for
the desktop engine's pinned Rust 1.89.0 (`agent-desktop` source commit
`9eb9b296`). Its own `rust-toolchain.toml` selects that version: installing
the universal targets only on stable does not supply them to the engine.
The subsequent stable setup remains the default for the Clovy host.

The MCP relay's architecture check passes the binary path before
`-verify_arch` and its architecture list. Xcode 16's `lipo` treats a trailing
path as another architecture, even though newer Xcode versions accept it.
Keep that check enabled: both real slices must validate before combining
and signing the universal relay.

To roll back, delete the bad release (`gh release delete vX.Y.Z`): `latest`
falls back to the previous one. Installs that already updated reinstall the
previous `.dmg` by hand.

## Updater key

The updater packages are signed with the owner's minisign key. The private key
and its password are the repository secrets `TAURI_SIGNING_PRIVATE_KEY` and
`TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, with a local copy in
`~/.config/clovy/updater.key` and `~/.config/clovy/updater.key.password`
(mode 0600). Keep a backup of both files outside this machine: an install only
accepts packages signed by the key whose public half is in
`plugins.updater.pubkey`, so losing it means reinstalling Clovy by hand
everywhere. The public key also sits in `~/.config/clovy/updater.key.pub`.

## Apple signing is optional

`scripts/build-macos-release.sh` (`pnpm tauri:build:release-mac`) decides from
the variables present, read from the environment (CI) or `.env.signing`
(local). `scripts/build-macos-release.sh --check` prints the plan without
building:

| Variables | Result |
|---|---|
| none | ad hoc signature, no notarization |
| `APPLE_SIGNING_IDENTITY` | app, helpers and DMG signed with that identity |
| identity plus `APPLE_API_ISSUER`, `APPLE_API_KEY` and one of `APPLE_API_KEY_PATH`, `APPLE_API_KEY_P8`, `APPLE_API_KEY_P8_BASE64` | signed, then the DMG is notarized and stapled |
| `TAURI_SIGNING_PRIVATE_KEY` (+ `_PASSWORD`) | adds `src-tauri/tauri.release.conf.json`: `Clovy.app.tar.gz` and its `.sig` |

In CI, `APPLE_CERTIFICATE` (base64 `.p12`) and `APPLE_CERTIFICATE_PASSWORD`
import the identity into a throwaway keychain; locally the identity is read
from the login keychain as it is. Notarization settings without an identity, or
an incomplete set, fail before the build. Adding the Apple secrets to the
repository is the only change needed to ship signed and notarized builds.

The script exports `CARGO_PROFILE_RELEASE_BUILD_OVERRIDE_STRIP=none` (the Xcode
27 proc-macro workaround) and leaves the files in
`src-tauri/target/release-artifacts/macos/` (or `CLOVY_RELEASE_OUT_DIR`).

## Installing an ad hoc build

Until the Apple secrets exist, the release is signed ad hoc and not notarized:

- **First download.** A `.dmg` downloaded in a browser is quarantined, and
  macOS refuses to open the app. Open it once, then go to System Settings >
  Privacy & Security and choose "Open Anyway" for Clovy. Updates installed by
  the app itself are not downloaded through a browser.
- **Permissions after every update.** An ad hoc signature changes with every
  build, so macOS treats each update as a new app: it asks again for
  Microphone, Screen Recording (system audio), Accessibility and the other
  grants. Grant them again after updating. This goes away once releases are
  signed with an Apple identity.
- **No Computer use.** The Computer use driver only accepts a host signed by
  an Apple team (`computer_use_driver.rs`), so an ad hoc release cannot start
  it; the release self-test checks only the bundled helper in that case.

## macOS support

Clovy supports macOS 14.0 and later on Apple Silicon and Intel Macs, including
macOS 15. Release builds are universal macOS apps. System
audio capture uses Core Audio process taps and is available only on macOS 14.2
and later. On macOS 14.0 or 14.1, recording falls back to microphone-only mode.
Chat uses the user's installed CLI. The app bundles the `clovy-mcp` relay and
native helpers, without a separate model-and-tool loop executable.

## Build

The local release recipe (frontend tooling and the Xcode strip workaround) is
in `AGENTS.md` under **Build**; it produces an
unsigned-updater build for local use. For the release files, with the updater
key and an ad hoc signature unless Apple variables are set:

```sh
PATH="$HOME/.local/share/node-official/node24/bin:$PATH" pnpm tauri:build:release-mac
```

Release builds are universal, so the `rustc` first on `PATH` must have the
`x86_64-apple-darwin` standard library. Homebrew's `rustc` does not; put the
rustup toolchain first (`PATH="$(dirname "$(rustup which rustc)"):$PATH"`) after
`rustup target add x86_64-apple-darwin`.

With Finder's "Show all filename extensions" on, macOS displays every app as
`<name>.app`; `verify-macos-app-name.mjs` then expects `Clovy.app`.

## Validating a built app

Run these against the built app (an ad hoc build fails the `spctl` and
`stapler` lines by design; they apply to a notarized build):

```sh
APP="<path to the built Clovy.app>"
DMG="<path to the built Clovy_universal.dmg>"

codesign --verify --deep --strict --verbose=2 "$APP"
spctl --assess --type execute --verbose "$APP"
spctl --assess --type install --verbose "$DMG"
xcrun stapler validate "$APP"
xcrun stapler validate "$DMG"
plutil -extract CFBundleURLTypes xml1 -o - "$APP/Contents/Info.plist"
lipo -archs "$APP/Contents/MacOS/clovy"
lipo -archs "$APP/Contents/Resources/native/bin/Clovy Dictation Helper.app/Contents/MacOS/clovy-dictation-helper"
lipo -archs "$APP/Contents/Resources/native/bin/Clovy Audio Capture.app/Contents/MacOS/clovy-system-audio-recorder"
```

Confirm `clovy` appears in `CFBundleURLSchemes` and each `lipo` command
prints both `x86_64` and `arm64`. Validate CLI chat independently with an
installed, signed-in CLI and the isolated debug-app proof described in
`AGENTS.md`; a bundle architecture check does not prove a model turn.

Also confirm the compatibility path resolves to the Clovy presentation name:

```sh
node scripts/verify-macos-app-name.mjs "$APP"
```

Do not replace this with a raw `CFBundleDisplayName` assertion. Finder requires
the base display name to match the physical bundle name before it applies the
localized Clovy value.

The desktop no longer needs V8 executable-memory entitlements for an agent
process. Preserve the entitlements required by the remaining native helpers.

## Private browser resources

The build wrapper prepares both pinned browser artifacts before the desktop
crate or native helpers are built. `pnpm browser:prepare -- --release` compiles
the audited agent-browser source commit with the upstream lockfile and Clovy's
private host patches; it downloads Chrome only from the fixed, digest-checked
Chrome for Testing URL in `scripts/agent-browser/manifest.json`.
For a universal bundle, pass `--target universal-apple-darwin` to prepare both
native platform subtrees. Each architecture keeps its matching Chrome tree and
engine executable under `native/agent-browser/<platform>/`.

The macOS overlay removes the inherited browser entry from `bundle.resources`
with a JSON merge patch `null` and copies `../resources/agent-browser` through
`bundle.macOS.files` to `Resources/native/agent-browser`. Tauri CLI 2.11.3
preserves file and directory symlinks on this copy path, including the versioned
Chrome framework links; the generic resources copier does not. Other native
helpers keep their existing resource entries. Runtime paths stay the same.
Tauri copies these files before signing the enclosing app, then uses that same
final app for the DMG and updater tar archive (which does not follow symlinks).
Do not repair or re-sign just one packaged artifact after bundling.

Preparation signs Mach-O files, nested frameworks and apps from the inside out
with `APPLE_SIGNING_IDENTITY`, or ad hoc when absent, preserving Chrome's
entitlements. The engine is signed with the same build identity. The staged
provenance hashes cover the signed output, including symlinks and permissions.
The dependency SBOM and license notices ship beside the executable. Release
Cargo builds reject missing or modified resources; plain debug Cargo directory
creation cannot make a release browser-ready. Validate the prepared resources
with `pnpm browser:prepare -- --release --check` (and the matching `--target`
for a universal release). macOS signature/notarization and live persistent-login
acceptance must be performed on a Mac; Linux preparation does not prove them.

Signing does not change Chromium's native Safe Storage namespace. The current
pinned source/packaging does not establish the required `-dev` Keychain service
for persistent development/proof runs. Keep those runs blocked before launch
until a supported namespace integration is reviewed and qualified; changing
only the bundle id/profile path or using persistent mock/basic crypto does not
meet that boundary. See the development runbook's pinned-browser section.
