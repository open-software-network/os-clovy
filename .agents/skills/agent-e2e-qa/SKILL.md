---
name: agent-e2e-qa
description: >-
  Run live, agent-driven end-to-end QA for os-clovy by opening the real app or
  web preview, clicking through changed flows, inspecting visible state,
  recording and compressing video, capturing screenshots/logs, and reporting
  pass/fail evidence. Use when a user asks for full integration testing, live
  app QA, visual inspection, "open the app and use it", "click through it",
  "record the QA run", native Tauri
  verification, WKWebView inspection, onboarding smoke tests, HUD/tray/hotkey
  checks, or manual QA replacement by an agent. Compose Browser, Chrome,
  Computer Use, terminal commands, and repo tests as appropriate.
---

# Agent E2E QA

Use this skill to run the kind of QA a human would do with the product open:
start Clovy, click through the affected workflow, observe the result, and leave
evidence. This is an agent-assisted verification layer, not a substitute for
deterministic unit, Rust, or CI tests.

## Decision Tree

Pick the narrowest surface that proves the behavior.

- **Web preview**: Use `pnpm dev` and Browser for React-only flows, standalone
  preview pages, onboarding preview, HUD demo pages, and visual smoke tests that
  do not require real Tauri commands. The browser-surface technique (faking the
  Tauri IPC bridge, Playwright/CDP driving, screenshot suites, PR-embeddable
  GIFs) is owned by the `browser-test-tauri-fe` skill — read it before driving
  this surface.
- **Background browser video**: Use the bundled Playwright helper for
  web-reachable agent flows when the user is actively using the desktop or does
  not need to watch the run. This avoids foreground macOS screen capture. It may
  shim the Tauri shell while routing prompts through the isolated legacy
  Hermes QA dashboard.
- **Native Tauri app**: Use `pnpm tauri:dev` and Computer Use for WKWebView,
  native windows, tray/menu behavior, macOS permissions, dictation hotkeys,
  microphone/system audio, update prompts, and any flow that depends on Rust
  commands or sidecars.
- **External browser handoff**: Use Chrome when the flow intentionally opens the
  user's real browser, such as OS Accounts login, checkout, account portal, or a
  page that depends on the user's browser session.
- **Deterministic promotion**: Recommend Playwright or Tauri WebDriver only
  after a flow is stable enough to run unattended. Do not add a dependency or
  CI job during a QA pass unless the user asked for that implementation.

If the requested flow touches payments, live accounts, OS permission prompts,
microphone/camera/location access, or sends user data externally, get explicit
confirmation at action time before performing that side effect.

## Setup

1. Read repo instructions first: `AGENTS.md`, `CLAUDE.md`, and relevant specs.
2. Inspect `git status -sb`. Do not overwrite unrelated user changes.
3. Work in the current task worktree. If this is part of `repo-build-pr`, use
   the worktree that skill created.
4. Ensure local development config exists:
   - `.env` should usually match `.env.example`.
   - `clovy-api/.env` should usually match `clovy-api/.env.example`.
   - Local mode uses `OS_CLOVY_LOCAL_DEV=1` and `local-dev-token` so QA does
     not require OS Accounts unless the specific flow is account QA.
5. Choose a dev command:
   - Web only: `pnpm dev`
   - Native app: `pnpm tauri:dev`
   - First-run wizard: `pnpm tauri:dev --replay-onboarding`
6. If `127.0.0.1:1421` or `127.0.0.1:8080` is already in use, verify whether
   the existing service belongs to this repo before reusing it. `pnpm tauri:dev`
   intentionally reuses occupied Vite and API ports.

Keep terminal sessions running only while they are needed. Before ending the
turn, stop or clearly identify any dev server or app process left running.

## Video Recording

Record live QA walkthroughs by default. Start recording before the first app
interaction and stop it only after the final visible pass/fail state is
captured.

- Save recordings under `.tmp/qa-recordings/` so large `.mov` files stay out of
  git. Use a timestamped, descriptive name such as
  `.tmp/qa-recordings/20260627-123000-agent-hi.mov`.
- On macOS, prefer the built-in recorder:
  ```bash
  mkdir -p .tmp/qa-recordings
  screencapture -v -C -k .tmp/qa-recordings/<timestamp>-<slug>.mov
  ```
  Run it as a long-running terminal session and stop it with Ctrl-C when the
  walkthrough is complete. Add `-V <seconds>` only when a hard maximum duration
  is useful.
- For background browser agent runs, prefer the bundled helper instead of
  `screencapture`:
  ```bash
  .agents/skills/agent-e2e-qa/scripts/run_background_agent_prompt.mjs \
    --prompt "hi"
  ```
  The helper starts an isolated tokenized legacy Hermes QA dashboard using its
  local config, opens the Vite app in headless Chrome, records Playwright video
  under `.tmp/qa-recordings/`, shims only the Tauri shell calls needed by the web
  surface, and waits for a visible assistant completion. Playwright comes
  from the repository's pinned `playwright` dev dependency; if it is
  unavailable, restore dependencies with `pnpm install --frozen-lockfile`.
- Do not record microphone audio unless the user explicitly requests and
  approves it; `screencapture -g` uses the default input and may capture private
  speech or room audio.
- If macOS blocks recording behind Screen Recording permission, ask the user to
  grant permission or report video as `BLOCKED`. Do not change OS privacy
  settings through Computer Use without confirmation.
- After stopping, verify the file exists and is non-empty with `ls -lh`. Include
  the recording path in `Artifacts`. If recording is unavailable, keep the run
  going with screenshots/logs and list the missing video in `Gaps`.
- Compress the recording before sharing it. Prefer the bundled helper, which
  creates a 720p, 10 fps, no-audio H.264 MP4 under 300 MiB:
  ```bash
  python3 .agents/skills/agent-e2e-qa/scripts/prepare_qa_video.py \
    .tmp/qa-recordings/<timestamp>-<slug>.mov
  ```
  The helper also accepts Playwright `.webm` recordings from background browser
  runs.
  Use `--max-bytes`, `QA_VIDEO_MAX_BYTES`, `--bitrate-kbps`, or
  `--min-bitrate-kbps` only when the default adaptive compression misses the
  size target. If `ffmpeg` is unavailable, report compressed video as
  `BLOCKED` or a `Gaps` item and keep the QA evidence path moving with raw video
  and screenshots.
- Share a video outside the machine (a PR, an issue, any upload) only when the
  user asked for it; otherwise report the local compressed path.
- Do not commit binary recordings unless the user explicitly asks. Prefer
  attaching or sharing the artifact path outside git.

## Charter

Before clicking, state the QA charter in one or two sentences:

- changed files or feature area under test
- user-visible workflow to prove
- data and environment assumptions
- explicit exclusions, such as live billing or hardware permissions

Use `docs/qa/feature-user-stories.tsv` to map broad checks to existing story
IDs when possible. For bug fixes, reproduce the original sequence first when it
is feasible, then verify the fixed sequence.

## Tool Use

Prefer semantic automation where available, then visual inspection.

- Browser: use Playwright locators, DOM snapshots, console errors, and
  screenshots. Keep the browser visible when the user asked to watch or use the
  page.
- Chrome: use it for flows that rely on the user's browser cookies or external
  login state. Do not enter credentials or complete payment without explicit
  authorization.
- Computer Use: use it for the native Tauri window, macOS permission panes,
  menu bar, tray, overlays, HUDs, hotkeys, drag/drop, file pickers, audio UI,
  and anything Browser cannot see.
- Terminal: use it for starting/stopping dev processes, checking logs, reading
  app data only when needed, and running targeted tests that support the live
  findings.

When a tool-specific skill is available for Browser, Chrome, or Computer Use,
read that skill before using the tool. For the Browser surface in this repo
that is `browser-test-tauri-fe` (fake Tauri IPC bridge, Playwright/CDP,
screenshots and GIFs).

## Walkthrough Loop

1. Start the selected app surface and wait for it to be ready.
2. Start the screen recording and note the output path.
3. Capture baseline evidence:
   - URL or process/app name
   - initial screenshot
   - console/runtime errors if Browser is used
   - relevant terminal log lines
4. Drive the app like a user:
   - click visible controls by accessible name when possible
   - type realistic text
   - use keyboard shortcuts when that is the product behavior
   - wait for visible state changes, not arbitrary sleeps
5. After each meaningful step, verify the expected state using the cheapest
   reliable signal:
   - DOM role/text/state for Browser
   - screenshot plus visible labels for Computer Use
   - app logs or local database only when visible UI cannot prove the state
6. Check for regressions around the touched surface:
   - blank screens
   - modal/popover layering
   - clipped text or overlapping controls
   - stale loading states
   - console errors
   - unexpected account, billing, or permission prompts
7. Stop the screen recording, verify the output file, compress it with
   `prepare_qa_video.py`, and include the raw path, compressed path, and public
   URL or PR comment when one was requested.
8. If something fails, capture the exact repro sequence, screenshot/log proof,
   and likely code owner files. Do not keep clicking until the failure is
   obscured.

## Evidence Format

Report results with this shape:

```text
Environment:
- Worktree/branch:
- Command:
- Surface: web preview | background browser | native Tauri | Chrome handoff
- Data mode:

Checks:
- PASS/FAIL/BLOCKED - story or flow - evidence

Artifacts:
- raw video path, compressed video path, screenshot/log paths,
  or PR comments

Gaps:
- anything not proven and why
```

Use **PASS** only when the live app visibly satisfied the behavior. Use
**BLOCKED** for missing credentials, unavailable tool surfaces, denied
permissions, absent hardware, or a side effect that needs user confirmation.

If you update `docs/qa/feature-user-stories.tsv`, only change rows that this
run actually proved. Keep "manual pending" language when the run did not cover
real accounts, real audio devices, OS permissions, or native overlays.

## Promotion Guidance

Agent-driven QA is the right default for broad product walkthroughs because it
can combine visual judgment, native app control, external browser handoffs, and
repo-specific context. Promote a flow to deterministic automation only when the
same steps are repeated often and can avoid secrets, production accounts,
hardware variability, and subjective visual calls.

Good promotion candidates:

- web-preview onboarding smoke checks
- settings navigation and saved preferences
- empty-state and dialog layering checks
- stable agent composer flows with mocked backend events

Poor promotion candidates:

- macOS microphone, accessibility, system audio, and global hotkey proof
- OS Accounts or Stripe live flows
- update installation and relaunch
- menu bar, tray, HUD, or native permission prompt placement on macOS
