# PRD: Computer use plugin

- **Mode:** CEO
- **Rank:** 5 of 10
- **Score:** 81/100
- **Date:** 2026-07-13
- **Status:** Accepted phase 2 in JUN-278
- **Canonical detailed spec:** [../browser-computer-use-prd.md](../browser-computer-use-prd.md)

## Thesis

Computer use is Clovy's adapter for macOS and Windows work that has no safe API
or browser path. It operates authorized app windows in the background when
supported. An operation that requires focus or pointer input may use visible
mode in the authorized window, selected before delivery. It is strategically
powerful and technically risky, which is why it ranks behind Browser use and
the two dominant work ecosystems.

## Customer and problem

Important follow-through still happens in desktop apps: legacy practice tools,
local files, native communication apps, and proprietary workflows. The user
cannot delegate these jobs to Clovy today. Traditional screen-driving agents
also take over the desktop, expose unrelated content, and make silent mistakes.

## Product promise

Grant each target app deliberately for the current task, preserve background
operation when supported, and revoke the capability at any time. Delivered or
uncertain input is never repeated in another mode.

## V1 experience

- The plugin tile explains Accessibility and Screen recording before macOS
  prompts appear.
- Clovy reports local helper/engine availability and actual platform readiness.
  No account, plan, remote rollout or selected vision model gates it.
- The user chooses a target app/task. Clovy can open a missing app, capture a
  bounded representation, and propose actions.
- The first access to each target app requires one task-scoped decision in
  Clovy's native approval tray. Later captures and actions in that app do not
  ask again until the task ends. Clovy never asks for approval in chat.
- A parked Stage Manager window is restored automatically after the app is
  authorized. Clovy adds it to Clovy's current group without moving the user's
  pointer or asking for a separate bring-forward decision.
- The user sees progress and can stop immediately.
- Disconnect removes Clovy's runtime grant; macOS permissions can also be
  revoked from System Settings.

## Scope

Computer use is limited to attended CLI turns on macOS and Windows, through
Clovy's broker and privately bundled source-pinned agent-desktop engine.
Captures return only to the CLI running that turn. The private helper retains
its fourteen allowlisted operations; the model receives the `computer_use`
tool rather than unrestricted upstream CLI or MCP access.
Routines, credential entry, payment entry, security settings, destructive
system operations and Linux desktop support are out of scope. Visible mode
grants no OS elevation or authentication authority. Native acceptance of the
new engine remains required in [the proof matrix](../qa/computer-use-parity.md).

## Privacy and trust

Screen captures needed for reasoning follow the user's selected inference path.
Clovy must never imply that Screen recording means captures stay local when the
selected model is remote. The driver is pinned and signed; the upstream network
installer never runs. Mutating actions are gated structurally, not by a prompt.

## Local availability

Availability depends on local grants, signed pinned artifacts and real platform
readiness ([ADR-0067](../adr/0067-computer-use-depends-only-on-local-gates.md)).
macOS uses the required Accessibility and Screen Recording grants. Windows uses
the standard interactive desktop with compatible integrity, without TCC or UAC
bypass. Permission education and revocation are always available locally.

## Success measures

| Metric | Target |
| --- | ---: |
| Eligible users completing local setup | 60% |
| Supported tasks completed without user takeover | 65% |
| Apps used without a current-task Clovy authorization | 0 |
| Unexpected cursor/focus theft incidents | 0 |
| Driver crash rate per task | under 1% |
| Median user corrections per completed task | under 1 |

## Risks and gates

- The driver uses private macOS interfaces and may break on OS updates.
- Screen capture has a larger privacy blast radius than structured connectors.
- Model visual mistakes can target the wrong element.
- App sandbox and TCC behavior must be proven on signed release builds.

## Decision

The current private-engine boundary follows
[the 2026-11-01 decision](../adr/ADR-desktop-agent-desktop-private-host.md)
and the D6 OpenSpec design. Source, patch, provenance and binary hashes are
packaged with the signed helper. Signed native self-tests and the full platform
matrix remain release gates; Linux engine evidence does not satisfy them.
Do not expand it to routines.
