# Agent-driven integration QA

## Recommendation

Use a project skill as the first-class interface for full integration QA where
an agent opens Clovy, clicks through the changed flow, inspects visible state,
and reports evidence. Keep deterministic test suites in CI, and promote only
stable, repeatable live checks into Playwright or Tauri WebDriver after they
have proven useful in repeated agent runs.

This gives Clovy three QA layers:

1. Deterministic CI for contracts and logic: `pnpm test`, `pnpm test:rust`, and targeted Rust or Vitest suites.
2. Agent-driven live QA for product workflows: `$agent-e2e-qa` starts the app,
   chooses Browser, Chrome, or Computer Use, records and compresses the
   walkthrough as video when possible, then reports pass/fail evidence.
   Background browser recording is preferred for web-reachable flows when the
   user is actively using their desktop.
3. Promoted browser or native automation for stable walkthroughs that do not
   depend on secrets, production accounts, hardware, subjective layout review,
   or macOS permission prompts.

## Why a skill first

A single script is too narrow for the cases this repo needs to prove. Clovy has
React views, Tauri commands, WKWebView behavior, native windows, tray and menu
behavior, global hotkeys, audio devices, OS permissions, CLI processes, and direct provider calls. Those surfaces need different tools and different
levels of confirmation before side effects.

A skill can make the routing decision at runtime:

- Browser for web preview, DOM assertions, console errors, and screenshots.
- Background Playwright for web-reachable agent flows that should record video
  without bringing Clovy or Chrome to the foreground. The bundled helper can
  shim the Tauri shell for rendering checks; it does not prove native CLI
  turns or provider execution.
- Chrome for flows that intentionally hand off to the user's browser session.
- Computer Use for native Tauri windows, overlays, hotkeys, menu bar, tray,
  file pickers, permission panes, and audio UI.
- Terminal commands for starting local services, reading logs, and running
  targeted tests that support the live finding.

The skill also forces an evidence contract, which matters more than just
"clicked around" completion. A useful live QA result needs the command used, the
surface tested, the data mode, pass/fail checks, a video recording when
available, a compressed MP4 path (shared on the PR only when requested),
screenshots or logs, and clear gaps for anything that was blocked.

## Repo-specific runbook

The desktop has no sign-in, local bearer token, or Clovy inference server.
Use a fresh, synthetic, run-owned debug profile; do not copy or touch the
owner's production profile. Follow AGENTS.md for the exact compiled identity,
permission readiness and single-native-proof rule.

Use these entry points:

- Web preview: `pnpm dev`
- Native Tauri: `pnpm tauri:dev`
- First-run onboarding replay: `pnpm tauri:dev --replay-onboarding`

`pnpm tauri:dev` starts or reuses Vite at `127.0.0.1:1421`. Verify ownership
before reusing an occupied port. For the provider-removal proof, bind a fake
listener to `127.0.0.1:8080` only when free and require zero requests. Tests
may inject an ephemeral retired origin. Dictation with/without a Groq key,
Notes with Claude Code/OpenRouter, and changing the text model while chat
stays on Claude Code need native evidence. Paid media, OpenRouter inference,
and Codex turns require explicit owner authorization; unrun actions remain
unproven.

Use `docs/qa/feature-user-stories.tsv` as the story inventory when a live run
maps to an existing user flow. Only update that tracker when the run actually
proved the row. Keep hardware, real account, and native overlay gaps explicit
when those surfaces were not covered.

Video artifacts should be prepared through the skill helper at
`.agents/skills/agent-e2e-qa/scripts/prepare_qa_video.py`. It transcodes raw
macOS screen recordings or Playwright `.webm` captures to no-audio H.264 MP4,
and keeps the result under 300 MiB. It only compresses: sharing the video
outside the machine is a separate, explicitly requested step.

For the background agent prompt path, use:

```bash
.agents/skills/agent-e2e-qa/scripts/run_background_agent_prompt.mjs --prompt "hi"
python3 .agents/skills/agent-e2e-qa/scripts/prepare_qa_video.py \
  .tmp/qa-recordings/<timestamp>-background-agent-hi.webm \
  --upload --confirm-public --comment-pr <pr-number>
```

## Promotion criteria

Good candidates for deterministic promotion:

- Onboarding preview smoke checks.
- Settings navigation and saved preferences.
- Empty-state and dialog layering checks.
- Stable agent composer flows with mocked backend events.

Poor candidates for deterministic promotion:

- Microphone, system audio, accessibility, and global hotkey proof.
- OS Accounts, checkout, and portal flows.
- Update installation and relaunch.
- Menu bar, tray, HUD, or permission prompt placement on macOS.

When a flow is promoted, Browser-backed Playwright should be the first choice
for web-only behavior. Tauri WebDriver can be considered for stable native
flows, but it should not replace Computer Use for visual or OS-level proof.

## Current implementation

The project skill lives at `.agents/skills/agent-e2e-qa/SKILL.md`. It defines
the decision tree, tool routing, walkthrough loop, and evidence format for live
integration QA. Its bundled `scripts/run_background_agent_prompt.mjs` helper
records headless browser agent runs, and `scripts/prepare_qa_video.py` handles
video compression.

## Example run

A first real run is checked in at
`docs/qa/agent-e2e-qa-runs/2026-06-26-onboarding-preview.md`. It used the new
skill contract to start a PR-branch web preview, drive the onboarding
permissions and trial flow with Chrome plus Computer Use, and record pass/fail
evidence plus gaps. The run intentionally stayed in local stubbed data mode:
no live account, checkout, native app, audio hardware, or macOS permission
prompt was exercised.
