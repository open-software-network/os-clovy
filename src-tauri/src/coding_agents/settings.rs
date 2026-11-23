//! Which agents' transcripts Clovy reads. Part of `activity-settings.json`
//! (`codingAgents`); every source is off until the user turns it on.

use serde::{Deserialize, Serialize};

use super::SourceId;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CodingAgentSources {
    pub claude_code: bool,
    pub codex: bool,
    pub copilot_cli: bool,
    pub copilot_vscode: bool,
    pub cursor: bool,
    pub cursor_cli: bool,
    pub antigravity: bool,
}

impl CodingAgentSources {
    pub fn is_enabled(&self, source: SourceId) -> bool {
        match source {
            SourceId::ClaudeCode => self.claude_code,
            SourceId::Codex => self.codex,
            SourceId::CopilotCli => self.copilot_cli,
            SourceId::CopilotVscode => self.copilot_vscode,
            SourceId::Cursor => self.cursor,
            SourceId::CursorCli => self.cursor_cli,
            SourceId::Antigravity => self.antigravity,
        }
    }

    pub fn any_enabled(&self) -> bool {
        SourceId::ALL
            .into_iter()
            .any(|source| self.is_enabled(source))
    }
}
