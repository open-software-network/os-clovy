# Computer use runs a pinned engine behind its authenticated helper

Status: accepted, 2026-11-01. Implements the owner's D6 decision in
`openspec/changes/melhorias-agente-audio-automacao/design.md`. The integrator
assigns the final ADR number. Supersedes ADR-0028's engine and macOS-only
implementation details; its local approval and private-helper boundary remain.

The desktop needs one engine for macOS and Windows while retaining exact
window identity, task ownership, app authorization, sensitive-field checks,
and Stop. Loading the upstream FFI in Tauri would place native control in the
main process, omit window scoping in its snapshot API, and leave destruction
unable to cancel an in-flight call. Invoking the normal upstream CLI on macOS
would instead create a public same-user daemon outside the helper's ownership.

Clovy builds `vercel-labs/agent-desktop` at the exact source commit in
`src-tauri/agent-desktop-pin.json` and retains the Clovy-owned authenticated
helper. On macOS that helper starts the internal command host directly over
private stdio. On Windows each invocation belongs to a non-breakaway Job
Object with kill-on-close. The agent receives the existing 14 closed helper
operations through Clovy's broker; it cannot choose arbitrary argv, an
upstream MCP server, a CLI home, or another daemon.

The reviewed downstream patch keeps macOS controllers and overlays in the
helper's process group and revalidates physical targets under the engine's
interaction lease. Stop invalidates run/epoch authority and terminates owned
processes without waiting for the request mutex. Real applications opened
through the OS application broker remain the user's applications.

Every package includes the real engine, its manifest, Apache-2.0 notice,
source SPDX record, and source qualification. Preparation uses a separate
archive of the immutable source and applies only the pinned patch. Mac engine
and helper executables are signed before the outer app. Windows engine,
helper and host require valid signatures from the same publisher. The signed
helper embeds the actual prepared engine SHA-256 at compile time, and the
signed host embeds the staged helper SHA-256. Editable adjacent manifests
cannot substitute an older executable from the same publisher. Missing
artifacts, a mismatched source/patch/protocol, or a changed binary hash fail
closed. Ordinary native builds require `pnpm computer-use:prepare`; the
explicit helper-bootstrap build flag only breaks preparation recursion and
never creates a placeholder executable. There is no runtime engine download,
global installation, or fallback to the retired engine.

Semantic actions use background mode when supported. The owner authorized
visible input when background delivery is impossible; that switch happens
before dispatch or after a definitive unsupported/not-attempted result.
Delivered or uncertain input is never replayed in another mode. Visible mode
still requires target identity and task authority and grants no OS elevation,
UAC, TCC, login, password, or OTP authority.

The trade-off is a small maintained engine patch and two signed artifacts
instead of the FFI's simpler call boundary. The source pin is newer than seven
days for specific Windows corrections; its reviewed provenance and unchanged
registry dependency cooldown are recorded in
`src-tauri/agent-desktop/source-review.md`. Linux engine/supervisor tests are
narrow proof. Signed macOS and Windows acceptance remains required in
`docs/qa/computer-use-parity.md`.

## Addendum, 2026-11-01: Windows application launch without elevation

Windows application launch uses `CreateProcessW` with an exact, locked
executable and a retained, qualified Explorer process as
`PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`. The parent must be the OS shell from
the Windows directory returned by the native API, with the same user/session,
non-elevated compatible integrity, stable image/birth identity and a fresh
shell-window check. This parent attribute inherits the specified parent's
token and job, so real applications remain outside the helper's controller
job. Controller creation retains its existing suspended `JOB_LIST` path.
This inheritance is the documented Windows contract, with native runtime
acceptance still required. [Parent-process attributes](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-updateprocthreadattribute).

An `asInvoker` manifest alone cannot establish effective launch policy.
Preflight also reads per-user and machine compatibility Layers in both
registry views, using canonical, ordinary DOS and executable-name candidates,
and checks the actual explicit launch environment. Elevating, forced
`RunAsInvoker`, unknown or unreadable layers are refused. The environment
preserves `__COMPAT_LAYER` unchanged and removes only private helper/engine
authority and run variables. Clovy never inserts an override to bypass OS
policy. Windows then evaluates effective AppCompat, Fusion and installer
policy in `CreateProcess` itself. A demand for elevation fails with
`ERROR_ELEVATION_REQUIRED`; there is no `ShellExecute` or AppInfo fallback that
could turn that failure into a UAC prompt. Manual Layer checks supplement
this OS decision; they do not claim to reproduce every SDB rule.
[UAC process creation](https://learn.microsoft.com/en-gb/windows-server/security/user-account-control/how-user-account-control-works),
[CreateProcess and elevation](https://learn.microsoft.com/en-us/windows/win32/dxtecharts/user-account-control-for-game-developers).

Real applications start without `CREATE_SUSPENDED`: stopping the helper must
not strand an unstarted application outside its job between creation and
resume. Once creation succeeds, the application belongs to the user and is
never killed by receipt validation or helper Stop. Its returned process
handle supplies the authoritative image and creation time; a PID found by
image name cannot substitute another process. Any later cancellation,
desktop loss or identity-query failure preserves the created effect and an
unsafe receipt. The trade-off is a small native process-creation path and
conservative refusal of indeterminate compatibility settings, while retaining
ordinary installed app launch without elevation or new model arguments.
