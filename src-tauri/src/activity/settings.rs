//! `activity-settings.json` in the app config dir: what the user chose in
//! Settings → Activity. Every value is normalized on load and on save, so the
//! engine can trust it without re-validating.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::coding_agents::settings::CodingAgentSources;

pub const ACTIVITY_SETTINGS_FILE: &str = "activity-settings.json";
pub const DEFAULT_RETENTION_DAYS: u32 = 30;
pub const MAX_RETENTION_DAYS: u32 = 365;
const MAX_LIST_ENTRIES: usize = 200;

/// Files saved with a lower `defaultsVersion` (or none) predate capture and
/// the MCP server starting by default; see [`load_at_startup`].
pub const DEFAULTS_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkHours {
    pub enabled: bool,
    /// ISO weekdays, 1 = Monday .. 7 = Sunday.
    pub days: Vec<u8>,
    /// "HH:MM", local time.
    pub start: String,
    /// "HH:MM"; an end at or before the start spans midnight.
    pub end: String,
}

impl Default for WorkHours {
    fn default() -> Self {
        Self {
            enabled: false,
            days: vec![1, 2, 3, 4, 5],
            start: "09:00".into(),
            end: "18:00".into(),
        }
    }
}

/// When the day summary is generated on its own (the day intelligence
/// scheduler; `docs/day-intelligence.md`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DaySummarySettings {
    /// "HH:MM", local time.
    pub time: String,
}

impl Default for DaySummarySettings {
    fn default() -> Self {
        Self {
            time: "18:00".into(),
        }
    }
}

/// Hours during which activity notifications wait instead of showing.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuietHours {
    pub enabled: bool,
    /// "HH:MM"; an end at or before the start spans midnight.
    pub start: String,
    pub end: String,
}

impl Default for QuietHours {
    fn default() -> Self {
        Self {
            enabled: false,
            start: "22:00".into(),
            end: "08:00".into(),
        }
    }
}

/// Activity notifications (day summary ready, activity failures).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityNotificationSettings {
    /// The switch for every activity notification.
    pub enabled: bool,
    pub quiet_hours: QuietHours,
}

impl Default for ActivityNotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            quiet_hours: QuietHours::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivitySettings {
    pub enabled: bool,
    pub secondary_monitors: bool,
    pub input_events: bool,
    pub pause_on_protected_video: bool,
    pub ignored_apps: Vec<String>,
    pub ignored_domains: Vec<String>,
    pub work_hours: WorkHours,
    pub retention_days: u32,
    pub day_summary: DaySummarySettings,
    pub notifications: ActivityNotificationSettings,
    /// The Clovy MCP server (`crate::mcp_server`); only its own command
    /// changes it, `activity_save_settings` keeps the stored value.
    pub mcp_server: bool,
    /// Coding-agent transcripts to ingest (`crate::coding_agents`).
    pub coding_agents: CodingAgentSources,
    /// The defaults this file has been upgraded to; `0` when the file has
    /// none (saved before [`DEFAULTS_VERSION`] 1). Only
    /// [`load_at_startup`] raises it; every save keeps the stored value.
    #[serde(default)]
    pub defaults_version: u32,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            secondary_monitors: false,
            input_events: true,
            pause_on_protected_video: true,
            ignored_apps: Vec::new(),
            ignored_domains: Vec::new(),
            work_hours: WorkHours::default(),
            retention_days: DEFAULT_RETENTION_DAYS,
            day_summary: DaySummarySettings::default(),
            notifications: ActivityNotificationSettings::default(),
            mcp_server: true,
            coding_agents: CodingAgentSources::default(),
            defaults_version: DEFAULTS_VERSION,
        }
    }
}

impl ActivitySettings {
    /// The database is needed while capture or any coding-agent source is on.
    pub fn needs_store(&self) -> bool {
        self.enabled || self.coding_agents.any_enabled()
    }

    pub fn normalized(mut self) -> Self {
        self.ignored_apps = dedupe_case_insensitive(
            self.ignored_apps
                .iter()
                .map(|app| app.trim().to_string())
                .filter(|app| !app.is_empty()),
        );
        self.ignored_domains = dedupe_case_insensitive(
            self.ignored_domains
                .iter()
                .filter_map(|domain| normalize_domain(domain)),
        );
        self.retention_days = self.retention_days.clamp(1, MAX_RETENTION_DAYS);
        let mut days: Vec<u8> = self
            .work_hours
            .days
            .iter()
            .copied()
            .filter(|day| (1..=7).contains(day))
            .collect();
        days.sort_unstable();
        days.dedup();
        self.work_hours.days = days;
        let defaults = WorkHours::default();
        if parse_clock(&self.work_hours.start).is_none() {
            self.work_hours.start = defaults.start;
        }
        if parse_clock(&self.work_hours.end).is_none() {
            self.work_hours.end = defaults.end;
        }
        if parse_clock(&self.day_summary.time).is_none() {
            self.day_summary.time = DaySummarySettings::default().time;
        }
        let quiet = QuietHours::default();
        if parse_clock(&self.notifications.quiet_hours.start).is_none() {
            self.notifications.quiet_hours.start = quiet.start;
        }
        if parse_clock(&self.notifications.quiet_hours.end).is_none() {
            self.notifications.quiet_hours.end = quiet.end;
        }
        self
    }
}

fn dedupe_case_insensitive(values: impl Iterator<Item = String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    values
        .filter(|value| seen.insert(value.to_lowercase()))
        .take(MAX_LIST_ENTRIES)
        .collect()
}

/// `https://www.YouTube.com/watch?v=1` → `youtube.com`. `None` for input
/// with no plausible host.
pub fn normalize_domain(input: &str) -> Option<String> {
    let trimmed = input.trim().to_lowercase();
    let without_scheme = trimmed
        .split_once("://")
        .map_or(trimmed.as_str(), |(_, rest)| rest);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
    let host = host.split(':').next().unwrap_or_default();
    let host = host.trim_start_matches("www.").trim_matches('.');
    let valid = !host.is_empty()
        && host
            .chars()
            .all(|ch| ch.is_alphanumeric() || ch == '-' || ch == '.');
    valid.then(|| host.to_string())
}

/// "HH:MM" → minutes after midnight.
pub fn parse_clock(value: &str) -> Option<u32> {
    let (hours, minutes) = value.trim().split_once(':')?;
    if hours.len() != 2 || minutes.len() != 2 {
        return None;
    }
    let hours: u32 = hours.parse().ok()?;
    let minutes: u32 = minutes.parse().ok()?;
    (hours < 24 && minutes < 60).then_some(hours * 60 + minutes)
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join(ACTIVITY_SETTINGS_FILE)
}

/// Missing or malformed files load the defaults (capture and the MCP server
/// on); missing fields load theirs.
pub fn load(path: &Path) -> ActivitySettings {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<ActivitySettings>(&raw).ok())
        .unwrap_or_default()
        .normalized()
}

/// The settings the app starts with: [`load`], with a file from before
/// capture and the MCP server started by default turned on once (owner
/// decision, 2026-10-07: an old `false` cannot be told apart from an opt-out).
/// The upgrade is saved before the user can change anything; if the save
/// fails, it is reported on stderr and the returned settings still carry
/// the new `defaults_version`, so the next successful save from either
/// switch stores it and a saved opt-out is never undone by the upgrade.
pub fn load_at_startup(path: &Path) -> ActivitySettings {
    let mut settings = load(path);
    if settings.defaults_version < DEFAULTS_VERSION {
        settings.enabled = true;
        settings.mcp_server = true;
        settings.defaults_version = DEFAULTS_VERSION;
        if let Err(error) = save(path, &settings) {
            eprintln!(
                "activity settings upgrade not saved to {}: {error}; capture and MCP server preferences are enabled in memory, the upgrade marker is not persisted, and the next successful settings save stores it",
                path.display()
            );
        }
    }
    settings
}

pub fn save(path: &Path, settings: &ActivitySettings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let serialized = serde_json::to_string_pretty(settings)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serialized)?;
    crate::filesystem::replace_file(&temporary, path).inspect_err(|_| {
        let _ = fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domains_normalize_to_bare_hosts() {
        assert_eq!(
            normalize_domain("https://www.YouTube.com/watch?v=1").as_deref(),
            Some("youtube.com")
        );
        assert_eq!(
            normalize_domain("mail.google.com:443").as_deref(),
            Some("mail.google.com")
        );
        assert_eq!(normalize_domain("  "), None);
        assert_eq!(normalize_domain("not a domain"), None);
    }

    #[test]
    fn normalization_dedupes_and_clamps() {
        let settings = ActivitySettings {
            ignored_apps: vec![" Slack ".into(), "slack".into(), "".into()],
            ignored_domains: vec!["youtube.com".into(), "https://www.youtube.com".into()],
            retention_days: 0,
            work_hours: WorkHours {
                enabled: true,
                days: vec![5, 1, 9, 1],
                start: "25:00".into(),
                end: "17:30".into(),
            },
            ..ActivitySettings::default()
        }
        .normalized();
        assert_eq!(settings.ignored_apps, vec!["Slack"]);
        assert_eq!(settings.ignored_domains, vec!["youtube.com"]);
        assert_eq!(settings.retention_days, 1);
        assert_eq!(settings.work_hours.days, vec![1, 5]);
        assert_eq!(settings.work_hours.start, "09:00");
        assert_eq!(settings.work_hours.end, "17:30");
    }

    #[test]
    fn settings_round_trip_and_malformed_files_load_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = settings_path(dir.path());
        assert_eq!(load(&path), ActivitySettings::default());
        let settings = ActivitySettings {
            enabled: true,
            ignored_domains: vec!["youtube.com".into()],
            ..ActivitySettings::default()
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
        fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), ActivitySettings::default());
    }
}
