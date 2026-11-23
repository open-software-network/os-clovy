// Kill switch from the RC hide (PR #555). On again now that JUN-171 closes the
// gaps that kept image generation out of the release: fast-path images enter
// the model's context, and generation/editing work as LLM tools.
export const IMAGE_GENERATION_ENABLED = true;

// Kill switch for the /video fast path and the agent video tools. On now that
// video generation launches; keep in lockstep with the Rust
// VIDEO_GENERATION_ENABLED in src-tauri/src/feature_flags.rs.
export const VIDEO_GENERATION_ENABLED = true;

// Public rollout switch for the Clovy-owned browser. Local experiments can
// enable it before rollout; no browser extension or pairing is required.
// Keep in lockstep with the native BROWSER_USE_ENABLED switch.
export const BROWSER_USE_ENABLED = false;
