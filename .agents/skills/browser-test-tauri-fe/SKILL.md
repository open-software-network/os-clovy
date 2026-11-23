---
name: browser-test-tauri-fe
description: Test, screenshot, and debug a Tauri or Vite frontend in a real browser using Playwright or Chrome DevTools MCP. Use when asked to run/screenshot a desktop-app or web UI, verify a frontend change visually, debug rendering without the native build, or when the native webview (macOS WKWebView) can't be driven by Playwright/CDP. Covers faking the Tauri IPC bridge so surfaces render with no backend.
---

# Browser-testing a Tauri / Vite frontend

Drive a frontend in headless/real Chromium for screenshots, interaction tests, and UI debugging. Two tools, one strategy.

## First decision: can you drive the real app?

- **Plain web app / Vite dev server** → yes. Point Playwright or Chrome DevTools MCP at the dev URL and go.
- **Tauri app on macOS** → NO. The window is WKWebView, which speaks no CDP, and `tauri-driver` doesn't support macOS. You **cannot** attach Playwright/DevTools to the real `.app` window. Instead, run the frontend's **Vite dev server** (`pnpm dev` / `npm run dev`) and drive that in Chromium. The catch: Tauri `invoke()`/`listen()` calls have no backend in a plain browser, so you must fake the IPC bridge (below).
- **Tauri on Windows** → WebView2 is Chromium; `tauri-driver` + WebdriverIO works. On Linux, WebKitGTK + `tauri-driver`.

## Faking the Tauri IPC bridge (the key technique)

`@tauri-apps/api` reads `window.__TAURI_INTERNALS__` at load time. Define it before the app module loads and the React/Vue/etc. app can't tell it isn't in a webview — every `invoke()`/`listen()` resolves against your fake.

Recipe — a dev-only HTML entry (e.g. `app-preview.html`) NOT listed in `vite.config` `rollupOptions.input` (so it never ships):

```html
<script type="module">
  // 1. install the bridge, 2. THEN import the app (dynamic import guarantees order)
  window.__TAURI_INTERNALS__ = {
    transformCallback(cb) { /* store cb, return id */ },
    unregisterCallback(id) { /* delete */ },
    convertFileSrc(p) { return p; },
    async invoke(cmd, args = {}) {
      switch (cmd) {
        case "plugin:event|listen":   /* register args.handler under args.event; return it */
        case "plugin:event|unlisten": return null;
        /* ...stub each command the app calls at boot, return realistic shapes... */
        default: return null;
      }
    },
  };
  // @tauri-apps/api >= v2.11: listener teardown uses a SEPARATE global.
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
    unregisterListener(event, eventId) { /* drop the handler */ },
  };
  await import("/src/main.tsx");
</script>
```

### Gotchas that each cost a debug cycle

1. **Missing `__TAURI_EVENT_PLUGIN_INTERNALS__`** (api ≥ v2.11): every `listen()` cleanup throws on React StrictMode's mount→unmount→mount and the render goes blank. Always define `unregisterListener`.
2. **Boot stubs**: stub every command the app fires on mount with the *shape the reducer/hook expects*, not `null`. A `null` where the app expects `{items:[]}` or a DTO crashes a reducer and blanks the page. Instrument: log `(cmd, args)` in `invoke`, load the page, watch the sequence, fix the one that precedes the crash.
3. **Reuse the project's own test doubles.** If the app already has a fake server / fixtures for its unit tests (e.g. a `FakeHermesServer` injected as `fetch`), route the relevant `invoke` to it instead of hand-writing JSON — the shapes already match. Watch for its expectations (auth headers, absolute-URL `new URL(input)` → prepend a base, body re-stringification).
4. **Find the navigation hook.** Apps often expose state changes via events (`listen("app://open-settings")`) — emit those from the bridge instead of clicking. Inspect the real DOM to learn selectors (settings nav may be `<button>`s, not `role=tab`).
5. **Skip first-run gates** by pre-setting the localStorage keys the app checks (onboarding-complete version, signed-in account stub).

## Driving + screenshots

**Playwright** (scripted, reproducible, can inject IPC before load via `addInitScript` or the HTML above) — best for screenshot suites and CI. See `screenshot-harness.template.mjs` in this skill dir. Always:
- viewport matching the app window (e.g. Tauri window size), `deviceScaleFactor: 2` for crisp shots
- capture `page.on("console", ...errors)` and `page.on("pageerror", ...)` — these surface the real bugs
- wait for a *proof-of-data* text (a value only present once the backend response rendered), not a fixed sleep
- exit non-zero if a surface never rendered

**Chrome DevTools MCP** (interactive: `take_snapshot`, `take_screenshot`, `list_console_messages`, `evaluate_script`) — best for ad-hoc debugging and a11y/perf. User prefers the testing Chrome profile, not the personal Claude-in-Chrome extension.

**Tools (check first, prompt before installing).** Driving/screenshots need Playwright + a Chromium build. Playwright is a pinned dev dependency; if `pnpm exec playwright --version` is missing, restore it with `pnpm install --frozen-lockfile`. If the Chromium build is missing, ask the user before running `pnpm exec playwright install chromium`.

## Recording walkthroughs (video / GIF)

Scope note: this pipeline produces lightweight, PR-embeddable GIFs of a
browser session. For QA-grade evidence — full walkthrough charter, compressed
MP4, PASS/FAIL report — use the `agent-e2e-qa` skill,
which owns that process and its scripts.

**Recording needs more than screenshots do, so if the user asks for a recording, first prompt to install every tool it needs** — check each, and install only with the user's go-ahead (one of these failing midway wastes the slow recording pass):

- **Playwright + Chromium** (drives the page, captures the `.webm`): restore the pinned dependency with `pnpm install --frozen-lockfile` if needed, then run `pnpm exec playwright install chromium` with the user's approval
- **ffmpeg** (converts `.webm` → a PR-embeddable `.gif`, and extracts verify frames): check `ffmpeg -version` → `brew install ffmpeg` (macOS) / `apt-get install -y ffmpeg` (Debian/Ubuntu)

Don't assume any are present. A missing ffmpeg only fails at the convert step, *after* the recording runs, so confirm all of them up front.

To record a feature walkthrough, drive the page **slowly with deliberate pauses** and let Playwright capture video at the context level:

```js
const context = await browser.newContext({
  viewport: { width: 1180, height: 780 },
  recordVideo: { dir: outDir, size: { width: 1180, height: 780 } },
});
// ...goto, navigate each surface, page.waitForTimeout(~1500) between stops...
await context.close(); // finalizes the .webm (NOT before this)
const webm = await page.video()?.path(); // or read the random-named .webm from outDir
```

The pacing you script is the pacing in the video. Then convert `.webm` → `.gif` so it **embeds in a PR comment via a raw URL** (`.webm`/`.mp4` only render through GitHub's attachment upload, unreachable by token auth). Two-pass palette keeps quality up and size sane, and trim the blank pre-mount lead-in so the poster frame is real UI:

```sh
VF="fps=12,scale=1000:-1:flags=lanczos"
ffmpeg -y -ss 1.3 -i in.webm -vf "$VF,palettegen" pal.png
ffmpeg -y -ss 1.3 -i in.webm -i pal.png -lavfi "$VF [x]; [x][1:v] paletteuse" out.gif
```

See `walkthrough-recorder.template.mjs` in this skill dir for a full recorder (records + converts). Keep GIFs ≲ a few MB (drop fps / scale width if larger). Note: this records the **browser** session — it can't screen-record a native Tauri window; for that you'd use macOS `screencapture`/QuickTime out of band.

## Verify your screenshots / recordings
Use the **Read** tool on a PNG — it renders the image so you can confirm the surface looks right (not just "a file exists"). For a GIF, Read shows only the **first frame**, so verify a recording by extracting mid-points with `ffmpeg -ss <t> -i out.gif -frames:v 1 frame.png` and reading those. This is how you catch blank/broken renders, error toasts, and blank poster frames.

## Worked example
`os-clovy` (Clovy, a Tauri 2 + React app): instantiate the
two templates in this skill dir as a dev-only `*-preview.html` that fakes the
bridge and routes the single `hermes_admin_request` command to the repo's
`FakeHermesServer` (`src/test/fixtures/`), plus a Playwright driver that visits
MCP servers / Skills hub / Toolsets / Installed skills and screenshots each. The
repo's unit tests exercise the same fake, so the preview proves the rendered
wiring.

These preview + recorder files are **dev-only and not committed**: generate them
per feature from the templates here, attach the screenshots/GIF to the PR, then
leave them out of the repo. Playwright is a standing pinned dev dependency;
restore it with `pnpm install --frozen-lockfile`. Install its Chromium build on
demand with `pnpm exec playwright install chromium` after the user approves the
download.
