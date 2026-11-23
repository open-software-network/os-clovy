# Building Clovy for Windows

Windows releases come from `.github/workflows/release.yml` (see
[ADR-0069](adr/0069-the-owner-publishes-own-releases-with-a-one-channel-updater.md)
and [release-macos.md](release-macos.md)): its `windows` job runs
`scripts/build-windows.ps1` with the updater key and publishes the NSIS
installer and its `.sig`. You can also build the installer locally with
`scripts/build-windows.ps1` (prerequisites in `AGENTS.md`) and validate it
with the checks below.

The workflow defaults to both platforms (`CLOVY_RELEASE_PLATFORMS=all`, also
when the non-secret repository variable is unset). For the temporary macOS-only
0.1.6 publication, the owner selects `macos` and preserves/restores the previous
variable state. The version job validates this choice once and passes its
output to the build/publication jobs and manifest generator. In `macos`, the
entire Windows job, including certificate preparation, is skipped. Windows
awaits a valid signing certificate; its signing and verification requirements
remain unchanged. This release has no Windows 0.1.6 installer or updater entry
and does not reuse the Windows 0.1.5 package under a newer version. In `all`,
Windows must build successfully with its certificate and updater signature;
a skipped, failed or cancelled required job cannot publish a partial release.
See [release-macos.md](release-macos.md) for the selection and restoration steps.

The canonical installer directory is `artifacts/windows-signed`; the desktop
workflow uploads `clovy-windows-signed-release-nsis` from that directory. There
is no unsigned output alias.

Computer use now requires signed native artifacts. Set
`WINDOWS_CERTIFICATE_THUMBPRINT` to an available current-user signing
certificate and provide `signtool.exe` on PATH (or `WINDOWS_SIGNTOOL_PATH`).
The builder signs the bundled `clovy-mcp.exe` relay, agent-desktop engine and authenticated helper
before Tauri signs Clovy and the installer. Payload verification requires a
valid matching publisher on the host, MCP relay, Computer use helper and engine, plus
matching engine manifest, source, protocol, patch, target, binary and license
hashes. Missing signing credentials fail the build; unsigned executables cannot
satisfy the private helper's peer contract.

`scripts/tauri-build.mjs` stages the MCP relay through
`prepare-windows-mcp.mjs` after preparing the existing native engines. This
invokes Cargo with the requested target/profile and honors `CARGO_TARGET_DIR`,
selects the executable from Cargo's artifact events, signs it with the existing
certificate, and writes `.tauri-helper/mcp-windows/clovy-mcp-manifest.json`.
The Windows config maps that directory to `native/bin`. `build.rs` may create
the directory for bootstrap, but never a placeholder executable.
`verify-windows-artifacts.ps1` requires exactly one extracted relay and manifest
at that location, identical staged/extracted hashes, current-source provenance,
the release MSVC target/profile, and the same publisher as app/installer.
Run the builder and final verifier on native Windows; a GNU metadata/typecheck
run is not proof of signing or NSIS extraction.

`pnpm tauri:dev` uses a native development runner around Cargo. On every rebuild,
it takes the executable paths reported by Cargo, signs the newly built host and
CLI relays with this same certificate, verifies their publisher, and only then
launches Clovy. Signing failure stops launch. The runner keeps the selected dev
identity and arguments and owns its child process tree across rebuilds. It does
not import a certificate or relax the private helper's signature checks. Native
Windows launch, handshake and rebuild acceptance is still required.

Updater minisign signing remains separate: with `TAURI_SIGNING_PRIVATE_KEY`
(and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) the installer also has a `.sig`.
Without that key the Authenticode-signed installer has no updater `.sig`.
The hosted Windows release workflow imports the owner-provided
`WINDOWS_CERTIFICATE` (base64 PFX) with `WINDOWS_CERTIFICATE_PASSWORD` into
its disposable current-user certificate store and exports the resulting
thumbprint. The import script refuses local or self-hosted execution using
[GitHub's runner environment marker](https://docs.github.com/en/actions/reference/workflows-and-actions/variables). Those
real owner secrets must be configured before the workflow can produce this
candidate; no certificate was imported on the implementation box. This native build and its
interactive Computer use acceptance have not run on the Linux implementation
box; see [computer-use-support.md](computer-use-support.md).

The installer installs `clovy.exe` under `$env:LOCALAPPDATA\Clovy` with the owner
as publisher, per
[ADR-0068](adr/0068-clovy-takes-its-own-identity-with-a-one-time-migration.md);
`scripts/verify-windows-artifacts.ps1` checks the payload.

The one-time identity migration of ADR-0068 runs only on macOS: Clovy on
Windows always starts with a new profile.

## Windows support

The Windows installer supports the app shell, microphone
recording, dictation shortcuts and paste, note generation, folders, and settings
using direct user-key providers or the selected CLI/endpoint Notes engine. macOS system-audio capture and Seatbelt
sandbox features are macOS-only.

Chat requires an installed CLI and its own login. Claude Code is the default,
with Codex as the existing local fallback. The installer bundles the `clovy-mcp`
relay for Clovy tools, not a model-and-tool loop. CLI execution policy and
Clovy's local approval broker govern each turn; see
[llm-providers.md](llm-providers.md#cli-chat-engine).

## Qualifying a signed build on native Windows

Before merging Windows build changes, run the tracked qualification helper in
PowerShell 7 on a native x64 Windows machine. Pass the standalone repository,
branch, and exact 40-character commit SHA to qualify:

```powershell
$repo = "E:\clones\work\os-june-butler"
$branch = "<branch-to-qualify>"
$expectedSha = "<exact-40-character-commit-sha>"

& "$repo\scripts\qualify-windows-build.ps1" `
  -RepoPath $repo `
  -Branch $branch `
  -ExpectedSha $expectedSha
```

The helper fetches `origin`, synchronizes the named GitButler branch to its
remote state, and fails unless both resolve to the expected SHA.
It then runs two complete signed builders and an additional explicit artifact
verification after each build. It does not install dependencies or upload
artifacts, push branches, or discard source changes.

The default evidence directory is
`.tmp/windows-qualification/<branch>/<short-sha>/`. Pass
`-EvidenceDirectory` to use another location. Each run writes `evidence.md` and
`build-transcript.txt`; certificate values are not included.

## Validation

Copy a locally built `Clovy_x64-setup.exe` to a clean Windows 11 VM and run:

```powershell
$installer = "$env:USERPROFILE\Downloads\Clovy_x64-setup.exe"
Get-AuthenticodeSignature $installer | Format-List
Start-Process -FilePath $installer -ArgumentList "/S" -Wait
Start-Process "$env:LOCALAPPDATA\Clovy\clovy.exe"
```

The installed app is branded as Clovy, but the current Windows binary on disk is
`clovy.exe` under `$env:LOCALAPPDATA\Clovy`.

Confirm the app launches as Clovy and an installed CLI starts with its own login, and Windows dictation can record from the microphone and
paste into a normal app. Record from the microphone and generate a note against
the configured Groq key and selected Notes engine. `Get-AuthenticodeSignature` must report `Valid` for this candidate; the
Computer use peer contract also requires the helper and engine to match the
host signing publisher. Record the signed native acceptance matrix separately.

## Private browser resources

The build wrapper prepares the pinned Windows x64 agent-browser host and Chrome
for Testing before the desktop build. Resources are installed beneath
`native/agent-browser/win64/`, including the private `clovy-agent-browser.exe`,
`chrome/chrome-win64/chrome.exe`, licenses, dependency SBOM and provenance.
The source commit, patches and immutable Chrome archive digest are recorded in
`scripts/agent-browser/manifest.json`; no runtime installer or global browser
lookup is used. The private host receives Authenticode signing through
`scripts/windows-sign.ps1` when the Windows signing certificate is configured;
Chrome keeps its vendor signatures. Unsigned local packaging follows the
existing Windows release policy. `pnpm browser:prepare -- --release --check`
verifies the exact staged files before bundling. Native installation, signature,
process-job teardown and persistent-login acceptance remain Windows checks.
