# activity/windows — Windows capture adapter

Implements `ActivityPlatform` for the shared text-only capture engine
(`../engine.rs`). Contract: [docs/activity-capture.md](../../../../docs/activity-capture.md);
spec: [openspec/specs/activity-capture/spec.md](../../../../openspec/specs/activity-capture/spec.md).

| File | Role |
|---|---|
| `mod.rs` | `WinActivityPlatform`; `system_idle_ms` via `GetLastInputInfo` + `GetTickCount` (u32 wraparound) |
| `window.rs` | Foreground window identity; packaged apps keyed by AUMID |
| `uia.rs` | UI Automation text, browser URL, private-window detection |
| `ocr.rs` | Windows Graphics Capture frame + `Windows.Media.Ocr`, in memory only |
| `input.rs` | Raw Input timings (key-down, clicks); never keys, scancodes or characters |
| `clipboard.rs` | Clipboard events, history/concealment markers checked before plaintext |
| `disk.rs` | Free space of the capture volume |

Conventions:
- No permission prompts on Windows; the UI follows native `status.supported`.
- UIA reads are bounded (node, depth and character caps, 250 ms soft deadline)
  and skip offscreen subtrees, so text stays tied to the filtered document.
- Clipboard open/read/close runs off the capture thread with a bounded wait;
  late results are discarded.
- COM/WinRT interfaces are declared before their apartment guard so they drop
  first.
- `LockApp`/`LogonUI` frames count as away, not idle
  (`../timeline/builder.rs`).

Tests run only on a Windows host (`cargo test --manifest-path src-tauri/Cargo.toml --locked activity`);
`make verify` on the Mac does not compile this module. Clipboard fixtures run in
a child process with a private window station; the installed-package icon test
is `--include-ignored` (see root AGENTS.md).
