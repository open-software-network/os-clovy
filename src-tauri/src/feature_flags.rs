/// Video kill switch. On now that video generation launches; keep in lockstep
/// with the frontend `VIDEO_GENERATION_ENABLED` in src/lib/feature-flags.ts.
pub const VIDEO_GENERATION_ENABLED: bool = true;

/// Browser use remains experimental while the private pinned browser's
/// native macOS/Windows acceptance is pending. The local experimental
/// setting can enable it with a separate explicit access grant. Keep in lockstep
/// with the frontend `BROWSER_USE_ENABLED` in src/lib/feature-flags.ts.
pub const BROWSER_USE_ENABLED: bool = false;
