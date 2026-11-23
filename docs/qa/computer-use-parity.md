# Computer use parity and acceptance matrix

## Agent-desktop cutover: native acceptance pending

The historical evidence below covers the previous macOS driver only. It does
not establish acceptance of agent-desktop on either platform. Run the following
against the signed packaged candidate, with disposable fixture apps and an
isolated Clovy profile, through both Claude Code and Codex. Record OS version,
bundle and engine hashes, target identities, observed delivery and process
exit evidence for each run. Do not use personal credentials or live data.

| Scenario | macOS | Windows |
| --- | --- | --- |
| Two windows and two concurrent conversations: targets, captures, references and app authorizations remain bound to the owning run; stale references are refused | Pending | Pending |
| Semantic background click and text insertion: observer verifies foreground app and pointer; insertion preserves existing text | Pending | Pending |
| Physical-only click, double-click and drag: visible mode reported before dispatch, exact destination verified, scaled and negative-display coordinates exercised | Pending | Pending |
| Modified click and drag, keyboard chords and Stop during held input: preheld user keys/buttons remain untouched; only posted inputs are released; cancellation acknowledgement follows verified release on the same interactive desktop; no input follows acknowledgement | Pending | Pending |
| Windows shell, task-switching, lock and clipboard shortcuts, including Insert aliases and pointer Windows-key modifiers, are refused before input; ordinary app chords remain available | Not applicable | Pending |
| Exact window screenshot, accessibility tree, visual and SoM output; post-action capture failure retains action receipt without replay | Pending | Pending |
| Sensitive password, OTP and payment fields refused, including after controls or window identity change | Pending | Pending |
| Revoke and Stop during a pending approval and an in-flight action; no late mutation or surviving authorized child/overlay; uncertain delivery is not retried | Pending | Pending |
| Packaged helper and engine signatures, compiled digests, peer identity, private transport and version verified; macOS mapped helper and engine CodeDirectory identity checked before capability/command delivery; direct or untrusted launch refused | Pending | Pending |
| Stop during signature/version verification, helper startup and final prewarm publication: no late helper publication or surviving owned controller; stale work never binds a new epoch | Pending | Pending |
| Stage Manager shelf restoration selects the exact window in the current stage; Screen Recording and Accessibility attributed to the correct signed components | Pending | Not applicable |
| Interactive desktop and integrity checks reject locked, secure or higher-integrity targets without elevation, including a lock during semantic reads/writes, focus or launch; HWND/PID/creation/path and Job Object teardown verified | Not applicable | Pending |

If a permission or interactive session is unavailable, record the case as
blocked. A Linux smoke test, a source inspection or a README claim is not native
acceptance.

## Historical predecessor evidence

The following records describe the previous macOS driver and are retained for
comparison. They are not acceptance evidence for this cutover.

Date: 2026-07-16
Scope: JUN-278 phase 2, JUN-288, JUN-293, JUN-296, JUN-356
Baseline: the attended macOS Computer Use experience documented for Codex on
2026-07-15.

This matrix defines parity by user outcome, not by reusing another product's
implementation. Clovy deliberately applies stricter mutation and sensitive-data
policy where the PRD requires it.

## Outcome matrix

| Outcome | Clovy behavior | Evidence and release gate |
| --- | --- | --- |
| Discover and enable | The top-level Plugins page contains the complete Computer use control, with no duplicate Settings entry. | `PluginsView`, `ComputerUseControl`; component tests. |
| Understand access before prompts | The tile explains Accessibility, Screen Recording, selected-model routing, and that captures are not analytics before the explicit Continue action. Enabling the grant alone never opens a macOS prompt. | `computer-use-control.test.tsx`; manual first-run walkthrough. |
| Install safely | A Clovy-owned helper privately wraps an engine built from the exact reviewed source commit and patch. The upstream public daemon and MCP surface are unavailable. Universal builds merge arm64 and x86_64 slices; each executable is signed before the helper and outer app. | Git/Cargo pin, source fingerprint, SPDX SBOM, license, `prepare-computer-use.mjs`, deterministic self-test. |
| Grant and inspect macOS access | Accessibility and Screen Recording are separate states with direct System Settings links and correctly attributed drag sources: the nested helper for Accessibility, the outer Clovy app for Screen Recording. Readiness requires both preflight and a live ScreenCaptureKit probe for the responsible app. Local dev keeps stable signed identities (`Clovy Dev.app` and a per-worktree helper) and never resets grants on a `make dev` launch. | Native status tests, `computer-use-control.test.tsx`, `tauri-dev-launcher.test.mjs`, `cargo-clovy-runner.test.mjs`, plus signed live release fixture. |
| Meet local readiness | Computer use requires the user's grant, the pinned bundled driver, local permissions, and task-scoped approval; no subscription is required in this fork (ADR-0067). | Desktop capability/policy tests and native readiness evidence. |
| Choose a target app | Clovy can list allowed running apps and select an exact app/window without raising it. Clovy, terminals, security tools, and credential managers are omitted or rejected. | Broker allow/deny tests; live multi-window fixture. |
| Open a missing app | Clovy asks once to use the installed app for the current task, opens it by display name, then returns only verified app/window identity. Opening the same already-approved running identity does not produce a second display-name alias prompt; ambiguous same-name identities fail closed. Paths, URLs, launch arguments, environment, debug ports, and forced new processes are unavailable. | Helper schema/argument tests; broker authorization-alias, blocked-name, and post-launch identity checks; live TextEdit walkthrough. |
| Restore a parked window | Window bounds and visibility are returned. Clovy collapses a full-size hidden window plus its small shelf proxy to the real Accessibility window. After app authorization, it automatically raises that window into Clovy's current Stage Manager group without a second decision or pointer movement, then re-lists and verifies it. | Stage Manager duplicate-collapse and classification tests, exact-PID/window helper contract, task-scoped authorization test, manual Stage Manager walkthrough. |
| Capture a bounded target | Capture addresses one exact process/window and returns a bounded AX tree plus the selected window image. Only the latest local capture remains, with private filesystem permissions. | Broker capture parsing, size/type/digest tests; signed live fixture. |
| Work in the background | Element actions use AX/background delivery; coordinates are window-local and posted to the target. A semi-transparent, click-through Computer use cursor shows the virtual click or drag position, including on scaled and negatively positioned displays. The real pointer and frontmost app must not change. | Driver pointer-notification and coordinate-normalization tests; signed target/observer release test. Active-Space behavior is also checked in the manual support matrix. |
| Act across Mac apps | A task can recapture and retarget multiple allowed apps. The first access to each verified app requires its own authorization; later actions in the same app do not ask again during that task. | Broker authorization-key and stale-target tests; manual two-app walkthrough. |
| See app access before it begins | The always-mounted chat tray names the target app, explains the current-task scope, shows expiry, and offers Deny or Allow for this task before the first capture or action. | Authorization registry and `ComputerUseApprovalsTray` component tests. |
| Keep approval native | The agent invokes the requested operation instead of asking for `yes` in chat. The first blocked call resumes from the native Allow for this task or Deny result. Product-facing copy calls the capability Computer use and does not expose its transport. | Agent schema contract self-test and approval tray test. |
| Approve or deny once per app | Each verified app has one task-scoped decision. There is no cross-task, approve-all, allow-always, or autonomous option. | UI tests and Rust app-authorization registry. |
| Prevent approval races | After approval, the exact window must still exist. Numbered controls must retain role and label; coordinate/key targets must retain the screenshot digest. | Stale capture and element tests. |
| Stop or take over | Stop immediately increments the task epoch, hides the Computer use cursor before teardown, denies parked actions, kills the private driver child, clears the target/captures, and leaves the grant available for a later attended task. Queued cursor updates from the old epoch cannot show it again. | Native stop and cursor epoch tests plus the live driver-exit test. |
| Stay attended | Only a turn submitted from visible Clovy chat receives both the loopback token and a unique native run lease. A routine, manually named MCP server, restored background process, or task after Stop has no lease. | runtime host tests, routine sanitization tests, native lease gate, app lifecycle integration. |
| Revoke | Turning the shared grant off performs the same stop path and removes the tool from the usable runtime. The UI explains that macOS TCC grants remain until removed in System Settings. | Shared grant integration and component tests. |
| Recover from readiness changes | The UI polls while setup is incomplete. A real readiness transition reconfigures the agent runtime once; loss of readiness also stops active work. | Native runtime-readiness transition and UI polling behavior. |
| Recover from driver failure | A failed driver call discards and kills the child. The next eligible request starts a new private child; version/stamp mismatches fail closed. | Native lifecycle code, pin tests, self-test failure modes. |
| Keep routines out | The app-owned MCP server is never included in routine toolsets or earned-autonomy servers. | Rust config tests and runtime capability tests. |
| Stop unsafe execution | Local readiness and task approval govern the capability; losing readiness stops active work. Release rollback is an owner decision, with no remote rollout route. | Desktop capability/policy tests and native gate evidence. |
| Ship only proven builds | Computer use is validated by testing the release candidate in the RC channel before promotion; the signed capture/background-action fixture (`computer-use-release-self-test.sh`) remains available for manual runs. Staging also verifies the signed bundle and contract. | RC-channel validation and desktop release workflows. |

## Deliberately stricter than the baseline

- Every target app requires a separate current-task authorization before Clovy
  captures or changes it.
- Clovy offers no cross-task Allow always, approve-all, or autonomous mode.
- Computer use cannot operate Clovy itself, terminals, System Settings,
  keychains, password managers, installers, security agents, or other blocked
  administration surfaces.
- It cannot enter passwords, secrets, one-time codes, payment details, or use
  clipboard and destructive/system shortcuts.
- It is unavailable to routines and locked or secure desktops. Windows now has a native helper implementation; signed Windows acceptance is pending.
- Its helper rejects direct launch and requires a signed Clovy parent plus a
  fresh in-memory initialization capability.
- It fails a delayed approval when the target image or control changed instead
  of guessing that the action is still safe.

Windows launch acceptance must cover a synthetic asInvoker app with no
compatibility setting and with RUNASADMIN, RUNASHIGHEST and forced
RunAsInvoker. Refused cases create no new app and show no UAC prompt. Registry
fixtures use only run-owned sandbox keys, never the owner's AppCompat Layers.
Check both registry views and environment policy, then verify the real
CreateProcess result against effective OS policy. Verify the exact retained
Explorer parent, no inherited private handles/authority, and that helper Stop
leaves a successfully created app running. The launch receipt's image and
creation time must come from its returned process handle. Also verify that a
minimized elevated target is neither restored nor focused, while an ordinary
authorized target still works. These are pending native Windows acceptance,
not outcomes of Linux decision tests or cross compilation.

These differences preserve the baseline outcomes of app selection, capture,
background control, visible progress, approval, stop/takeover, and revocation
while honoring Clovy's accepted PRD.

## Implementation QA evidence

The implementation pass on 2026-07-16 exercised the real helper against two
signed disposable Mac app fixtures. The live test captured the background
target, clicked and typed through Accessibility, and verified that the
foreground app, pointer, physical key and modifier state, and active Space did
not change. The helper's direct-launch refusal, authenticated MCP handshake,
closed tool schemas, exact upstream pin, source fingerprint, universal
architectures, bundle metadata, and code signature were also checked.

That historical production React surface was then exercised in an isolated Clovy QA app and
browser harness across `off`, `permission_missing`, `model_unsupported`,
`plan_required`, `driver_missing`, `rollout_disabled`, `unsupported`, and
`ready`. The historical `model_unsupported`, `plan_required`, and `rollout_disabled` states are not
current fork gates; local readiness and task approval govern availability. Both Allow for this task and Deny removed the parked app authorization
without a batch or persistent-authorization path.

![Computer use ready state](./jun-278-computer-use-ready.png)

![Computer use macOS access state](./jun-278-computer-use-permissions.png)

![Computer use app authorization](./jun-278-computer-use-approval.png)

## Required walkthrough before release

1. Start with the Clovy grant off and both helper TCC grants absent. Confirm the
   switch shows education without prompting.
2. Enable the grant, choose Continue, deny each macOS prompt once, and verify
   the states remain distinct and the tool stays unavailable.
3. Grant both permissions to the signed helper and verify the UI becomes Ready
   without restarting Clovy.
4. Use a supported vision model to list two fixture apps and capture the
   background target.
5. Ask Clovy to open TextEdit when it is not running. Allow it for the task and
   verify a usable document window is returned without a textual approval
   request.
6. Put TextEdit in the Stage Manager shelf. Verify its thumbnail is marked as
   needing restore, then verify Clovy adds TextEdit to Clovy's current group and
   captures the full document window without a second decision.
7. Click, drag, type, scroll, and recapture TextEdit. Verify the Computer use
   cursor follows each virtual click and drag without intercepting input, while
   the real pointer and active Space do not change.
8. Retarget another allowed app. Deny its first-use authorization and verify the
   app is not captured or changed.
9. End the task, start a new one, and target TextEdit again. Verify Clovy asks
   once again before access.
10. Park an app authorization and press Stop. Verify the card and Computer use
    cursor disappear promptly, the child exits, the action does not run, and
    captures are removed. Verify a delayed pointer update cannot show the
    cursor again.
11. Revoke the Clovy grant during an active task. Verify the same stop behavior
   and that macOS permissions remain independently removable.
12. Repeat model mismatch, driver mismatch, helper crash, app quit, multiple
    windows, modal, menu, text, list, and scrolling cases on the oldest and
    newest supported macOS versions.

The deterministic and signed live fixtures are release blockers. The broader
manual matrix remains the RC dogfood gate because TCC denial UI, cross-Space
placement, and OS update behavior cannot be made reliable on ephemeral hosted
runners.

## Cutover packaging evidence and required native commands

`pnpm computer-use:prepare` is the product entry point. It calls the pinned
engine preparer and authenticated helper preparer on macOS and Windows, and is
wired into development launch, Tauri build hooks, the macOS release builder,
and the Windows builder. The source repository is archived into a separate
build directory before applying the pinned patch; preserved upstream evidence
is not edited. Missing native artifacts fail the desktop build, while the
helper bootstrap only creates the empty Tauri resource directory needed to
compile the helper itself.

Artifact verification tests exercise actual executable bytes, source/protocol/
patch/target mismatches and license changes. Linux builds of the real patched
engine and private supervisor are narrow proof only. The signed helper,
Tauri bundle, TCC attribution, HWND/Job Object enforcement and actual input on
macOS and Windows remain open until run on those platforms.

On macOS, prepare first, build the signed candidate, and run
`computer-use-release-self-test.sh --live --target universal-apple-darwin`
against its nested helper. On Windows, set the signing certificate, build with
`scripts/build-windows.ps1`, verify the extracted installer payload, and run
`computer-use-self-test.mjs --live --bundle <resource-dir> --host <signed-host>`.
The Windows test compiles the checked-in signed synthetic WinForms fixture at
the host's fixed `native/fixtures` QA path. Its automated live scope is exact
capture and semantic click with pointer/focus sentinels. The full matrix above
must still cover text, physical actions, DPI, sensitive controls, two runs,
Stop/revoke, ownership and zero surviving controllers/overlays. Keep native
results pending until their observed artifacts and receipts exist.
