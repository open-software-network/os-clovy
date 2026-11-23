//! One scan: seal idle live blocks, then, for every enabled source only,
//! discover store entries changed inside the window, stream the changed ones
//! into block builders (which drop Clovy's own CLI conversations), and upsert
//! the blocks.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

use chrono::{DateTime, Utc};

use super::segment::IDLE_GAP;
use super::settings::CodingAgentSources;
use super::sources::{self, Fingerprint, SourceRoots};
use super::SourceId;
use crate::activity::store::{ActivityStore, StoreError};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScanReport {
    /// Store entries opened, per source (unchanged entries are not opened).
    pub files_read: BTreeMap<SourceId, usize>,
    pub sessions: usize,
    pub clovy_calls_skipped: usize,
    pub blocks_written: usize,
    pub blocks_sealed_idle: u64,
    pub load_errors: usize,
}

impl ScanReport {
    pub fn changed(&self) -> bool {
        self.blocks_written > 0 || self.blocks_sealed_idle > 0
    }
}

pub struct Scanner {
    roots: SourceRoots,
    /// Fingerprint of every entry loaded successfully, so unchanged files are
    /// not parsed again.
    seen: HashMap<PathBuf, Fingerprint>,
}

impl Scanner {
    pub fn new(roots: SourceRoots) -> Self {
        Self {
            roots,
            seen: HashMap::new(),
        }
    }

    /// `window_start` bounds history: entries last changed before it are not
    /// read, and blocks that ended before it are not stored.
    pub async fn scan(
        &mut self,
        store: &ActivityStore,
        enabled: &CodingAgentSources,
        now: DateTime<Utc>,
        window_start: DateTime<Utc>,
    ) -> Result<ScanReport, StoreError> {
        let mut report = ScanReport {
            blocks_sealed_idle: store
                .seal_idle_coding_agent_blocks(now - IDLE_GAP, now)
                .await?,
            ..ScanReport::default()
        };
        for source in SourceId::ALL {
            if !enabled.is_enabled(source) {
                continue;
            }
            // Listing and stat-ing every transcript is blocking file IO.
            let roots = self.roots.clone();
            let candidates = tokio::task::spawn_blocking(move || {
                sources::discover(source, &roots, window_start.into())
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "coding agents: discovery stopped");
                Vec::new()
            });
            for candidate in candidates {
                if self.seen.get(&candidate.path) == Some(&candidate.fingerprint) {
                    continue;
                }
                *report.files_read.entry(source).or_default() += 1;
                let sessions = match sources::load(&candidate, now, window_start).await {
                    Ok(sessions) => sessions,
                    Err(error) => {
                        tracing::debug!(%error, path = %candidate.path.display(), "coding agents: store not readable");
                        report.load_errors += 1;
                        continue;
                    }
                };
                for (info, builder) in sessions {
                    report.sessions += 1;
                    if builder.is_clovy_call() {
                        report.clovy_calls_skipped += 1;
                        continue;
                    }
                    for block in builder.finish(&info) {
                        if store.upsert_coding_agent_block(&block, now).await? {
                            report.blocks_written += 1;
                        }
                    }
                }
                self.seen.insert(candidate.path, candidate.fingerprint);
            }
        }
        Ok(report)
    }
}
