use std::collections::HashSet;

use sqlx::query::query;
use sqlx::row::Row;
use sqlx_sqlite::{SqlitePool, SqliteTransaction};

const SCHEMA_MIGRATIONS_TABLE: &str = "schema_migrations";

#[derive(Clone, Copy)]
struct ColumnDefinition {
    name: &'static str,
    definition: &'static str,
}

#[derive(Clone, Copy)]
enum MigrationStep {
    Sql(&'static str),
    EnsureColumns {
        table: &'static str,
        columns: &'static [ColumnDefinition],
    },
    DropIndex(&'static str),
}

#[derive(Clone, Copy)]
enum SchemaRequirement {
    Table(&'static str),
    Index(&'static str),
    Column {
        table: &'static str,
        column: &'static str,
    },
    MissingIndex(&'static str),
}

struct Migration {
    version: i64,
    name: &'static str,
    requirements: &'static [SchemaRequirement],
    steps: &'static [MigrationStep],
}

const SOURCE_SESSION_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "source_mode",
        definition: "TEXT NOT NULL DEFAULT 'microphone_only'",
    },
    ColumnDefinition {
        name: "permission_summary",
        definition: "TEXT",
    },
];
const SOURCE_ARTIFACT_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "source",
        definition: "TEXT NOT NULL DEFAULT 'microphone'",
    },
    ColumnDefinition {
        name: "partial_path",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "status",
        definition: "TEXT NOT NULL DEFAULT 'valid'",
    },
    ColumnDefinition {
        name: "expected_duration_ms",
        definition: "INTEGER NOT NULL DEFAULT 0",
    },
    ColumnDefinition {
        name: "validation_summary",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "last_error",
        definition: "TEXT",
    },
];
const SOURCE_TRANSCRIPT_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "recording_session_id",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "source_artifact_id",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "source",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "source_mode",
        definition: "TEXT NOT NULL DEFAULT 'microphone_only'",
    },
];
const SOURCE_CHECKPOINT_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "source",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "source_artifact_id",
        definition: "TEXT",
    },
];
const TRANSCRIPT_TURN_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "start_ms",
        definition: "INTEGER",
    },
    ColumnDefinition {
        name: "end_ms",
        definition: "INTEGER",
    },
    ColumnDefinition {
        name: "turn_index",
        definition: "INTEGER",
    },
];
const FOLDER_DESCRIPTION_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "description",
    definition: "TEXT",
}];
const AGENT_TASK_SESSION_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "hermes_session_id",
    definition: "TEXT",
}];
const AGENT_MESSAGE_IDENTITY_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "external_id",
    definition: "TEXT",
}];
const P3A_REPORTING_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "reported_value",
        definition: "INTEGER NOT NULL DEFAULT 0",
    },
    ColumnDefinition {
        name: "reported_at",
        definition: "TEXT",
    },
];
const DICTATION_APP_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "app_bundle_id",
    definition: "TEXT",
}];
const PROFILE_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "profile",
    definition: "TEXT NOT NULL DEFAULT 'default'",
}];
const ROUTINE_APPROVAL_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "approval_since",
    definition: "TEXT",
}];
const ROUTINE_TOOL_CATALOG_VERSION_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "tool_catalog_version",
    definition: "INTEGER NOT NULL DEFAULT 0",
}];
const AGENT_RUN_MCP_SNAPSHOT_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "mcp_policy_snapshotted",
    definition: "INTEGER NOT NULL DEFAULT 0",
}];
const AGENT_RUN_SKILLS_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "enabled_skills_json",
    definition: "TEXT NOT NULL DEFAULT '[]'",
}];
const AGENT_RUN_REASONING_EFFORT_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "reasoning_effort",
    definition: "TEXT",
}];
const AGENT_RUN_CONFIG_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "run_config_json",
    definition: "TEXT",
}];
const FOLDER_MEMORY_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "instructions",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "memory_disabled",
        definition: "INTEGER NOT NULL DEFAULT 0",
    },
];
const TRANSCRIPT_SPAN_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "span_id",
    definition: "TEXT",
}];
const CONNECTOR_METADATA_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "metadata",
    definition: "TEXT NOT NULL DEFAULT '{}'",
}];
const NOTE_CALENDAR_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "calendar_event_id",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "calendar_event_title",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "calendar_event_start_at",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "calendar_event_end_at",
        definition: "TEXT",
    },
    ColumnDefinition {
        name: "calendar_account_email",
        definition: "TEXT",
    },
];
const FOLDER_LOCAL_PATH_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "local_path",
    definition: "TEXT",
}];
const RECORDING_ORIGIN_COLUMNS: &[ColumnDefinition] = &[
    ColumnDefinition {
        name: "recording_origin",
        definition: "TEXT NOT NULL DEFAULT 'other'",
    },
    ColumnDefinition {
        name: "meeting_app_bundle_families",
        definition: "TEXT NOT NULL DEFAULT '[]'",
    },
    ColumnDefinition {
        name: "auto_finish_eligible",
        definition: "INTEGER NOT NULL DEFAULT 0",
    },
];

// IMPORTANT: positions in this catalog are shipped schema versions. They must
// follow the order in which changes reached users, not SQL filename prefixes:
// parallel branches produced duplicate 014 files and later renumbered files
// without changing release chronology. The historical-prefix upgrade test
// models real vintages by slicing this catalog, so reordering an entry would
// invalidate both legacy detection and that test. Append at the end only.
//
// Fresh databases add columns in catalog order, while replay-era databases
// retain the old runner's physical order. In particular,
// transcripts.source_mode is cid 14 on fresh databases but cid 17 on upgraded
// databases; folders.profile precedes local_path when fresh but follows it when
// upgraded. All access must name columns explicitly; never rely on cid order,
// positional decoding, or SELECT *. The integration guard in
// tests/sql_query_guards.rs enforces the projection star-select rule for both
// affected tables, including qualified stars.
const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "initial_schema",
        requirements: &[
            SchemaRequirement::Table("folders"),
            SchemaRequirement::Table("notes"),
            SchemaRequirement::Table("note_folders"),
            SchemaRequirement::Table("recording_sessions"),
            SchemaRequirement::Table("recording_checkpoints"),
            SchemaRequirement::Table("audio_artifacts"),
            SchemaRequirement::Table("transcripts"),
            SchemaRequirement::Table("generation_results"),
            SchemaRequirement::Index("idx_notes_created_at"),
            SchemaRequirement::Index("idx_note_folders_folder"),
            SchemaRequirement::Index("idx_recording_sessions_note"),
            SchemaRequirement::Index("idx_recording_sessions_status"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/001_init.sql"
        ))],
    },
    Migration {
        version: 2,
        name: "source_modes",
        requirements: &[
            SchemaRequirement::Column {
                table: "recording_sessions",
                column: "source_mode",
            },
            SchemaRequirement::Column {
                table: "recording_sessions",
                column: "permission_summary",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "source",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "partial_path",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "status",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "expected_duration_ms",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "validation_summary",
            },
            SchemaRequirement::Column {
                table: "audio_artifacts",
                column: "last_error",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "recording_session_id",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "source_artifact_id",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "source",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "source_mode",
            },
            SchemaRequirement::Column {
                table: "recording_checkpoints",
                column: "source",
            },
            SchemaRequirement::Column {
                table: "recording_checkpoints",
                column: "source_artifact_id",
            },
            SchemaRequirement::Index("idx_audio_artifacts_session_source"),
            SchemaRequirement::Index("idx_transcripts_session_source"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "recording_sessions",
                columns: SOURCE_SESSION_COLUMNS,
            },
            MigrationStep::EnsureColumns {
                table: "audio_artifacts",
                columns: SOURCE_ARTIFACT_COLUMNS,
            },
            MigrationStep::EnsureColumns {
                table: "transcripts",
                columns: SOURCE_TRANSCRIPT_COLUMNS,
            },
            MigrationStep::EnsureColumns {
                table: "recording_checkpoints",
                columns: SOURCE_CHECKPOINT_COLUMNS,
            },
            MigrationStep::Sql(include_str!("../../migrations/002_source_modes.sql")),
        ],
    },
    Migration {
        version: 3,
        name: "transcript_turns",
        requirements: &[
            SchemaRequirement::Column {
                table: "transcripts",
                column: "start_ms",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "end_ms",
            },
            SchemaRequirement::Column {
                table: "transcripts",
                column: "turn_index",
            },
        ],
        steps: &[MigrationStep::EnsureColumns {
            table: "transcripts",
            columns: TRANSCRIPT_TURN_COLUMNS,
        }],
    },
    Migration {
        version: 4,
        name: "generation_blocks",
        requirements: &[
            SchemaRequirement::Table("note_generation_blocks"),
            SchemaRequirement::Index("idx_note_generation_blocks_session"),
            SchemaRequirement::Index("idx_note_generation_blocks_note_order"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/003_generation_blocks.sql"
        ))],
    },
    Migration {
        version: 5,
        name: "folder_descriptions_and_duplicate_names",
        requirements: &[
            SchemaRequirement::Column {
                table: "folders",
                column: "description",
            },
            SchemaRequirement::MissingIndex("idx_folders_active_name"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "folders",
                columns: FOLDER_DESCRIPTION_COLUMN,
            },
            MigrationStep::DropIndex("idx_folders_active_name"),
        ],
    },
    Migration {
        version: 6,
        name: "dictionary",
        requirements: &[
            SchemaRequirement::Table("dictionary_entries"),
            SchemaRequirement::Index("idx_dictionary_entries_active_phrase"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/004_dictionary.sql"
        ))],
    },
    Migration {
        version: 7,
        name: "dictation_history",
        requirements: &[
            SchemaRequirement::Table("dictation_history"),
            SchemaRequirement::Index("idx_dictation_history_created_at"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/005_dictation_history.sql"
        ))],
    },
    Migration {
        version: 8,
        name: "transcript_turn_uniqueness",
        requirements: &[SchemaRequirement::Index(
            "idx_transcripts_session_source_turn",
        )],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/006_transcript_turn_uniqueness.sql"
        ))],
    },
    Migration {
        version: 9,
        name: "agent_workspace",
        requirements: &[
            SchemaRequirement::Table("agent_tasks"),
            SchemaRequirement::Table("agent_messages"),
            SchemaRequirement::Table("agent_tool_events"),
            SchemaRequirement::Index("idx_agent_tasks_updated_at"),
            SchemaRequirement::Index("idx_agent_tasks_status"),
            SchemaRequirement::Index("idx_agent_messages_task_created"),
            SchemaRequirement::Index("idx_agent_tool_events_task_created"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/007_agent.sql"
        ))],
    },
    Migration {
        version: 10,
        name: "agent_task_session_identity",
        requirements: &[SchemaRequirement::Column {
            table: "agent_tasks",
            column: "hermes_session_id",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "agent_tasks",
            columns: AGENT_TASK_SESSION_COLUMN,
        }],
    },
    Migration {
        version: 11,
        name: "agent_message_identity",
        requirements: &[
            SchemaRequirement::Column {
                table: "agent_messages",
                column: "external_id",
            },
            SchemaRequirement::Index("idx_agent_messages_task_external_id"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "agent_messages",
                columns: AGENT_MESSAGE_IDENTITY_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/008_agent_message_identity.sql"
            )),
        ],
    },
    Migration {
        version: 12,
        name: "session_folders",
        requirements: &[
            SchemaRequirement::Table("session_folders"),
            SchemaRequirement::Index("idx_session_folders_folder"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/009_session_folders.sql"
        ))],
    },
    Migration {
        version: 13,
        name: "p3a_counters",
        requirements: &[
            SchemaRequirement::Table("p3a_counters"),
            SchemaRequirement::Column {
                table: "p3a_counters",
                column: "reported_value",
            },
            SchemaRequirement::Column {
                table: "p3a_counters",
                column: "reported_at",
            },
            SchemaRequirement::Index("idx_p3a_counters_epoch"),
        ],
        steps: &[
            MigrationStep::Sql(include_str!("../../migrations/010_p3a_counters.sql")),
            MigrationStep::EnsureColumns {
                table: "p3a_counters",
                columns: P3A_REPORTING_COLUMNS,
            },
        ],
    },
    Migration {
        version: 14,
        name: "connector_accounts",
        requirements: &[
            SchemaRequirement::Table("connector_accounts"),
            SchemaRequirement::Table("routine_trust"),
            SchemaRequirement::Table("connector_triggers"),
            SchemaRequirement::Table("trigger_cursors"),
            SchemaRequirement::Index("idx_connector_triggers_job_id"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/011_connectors.sql"
        ))],
    },
    Migration {
        version: 15,
        name: "connector_grants",
        requirements: &[
            SchemaRequirement::Table("connector_grants"),
            SchemaRequirement::Index("idx_connector_grants_token"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/012_connector_grants.sql"
        ))],
    },
    Migration {
        version: 16,
        name: "connector_credited_runs",
        requirements: &[
            SchemaRequirement::Table("connector_credited_runs"),
            SchemaRequirement::Column {
                table: "routine_trust",
                column: "approval_since",
            },
        ],
        steps: &[
            MigrationStep::Sql(include_str!(
                "../../migrations/013_connector_credited_runs.sql"
            )),
            MigrationStep::EnsureColumns {
                table: "routine_trust",
                columns: ROUTINE_APPROVAL_COLUMN,
            },
        ],
    },
    Migration {
        version: 17,
        name: "note_transcription_jobs",
        requirements: &[
            SchemaRequirement::Column {
                table: "transcripts",
                column: "span_id",
            },
            SchemaRequirement::Table("note_transcription_jobs"),
            SchemaRequirement::Index("idx_note_transcription_jobs_operation"),
            SchemaRequirement::Index("idx_note_transcription_jobs_session_status"),
            SchemaRequirement::Index("idx_note_transcription_jobs_pending"),
            SchemaRequirement::Index("idx_transcripts_span_id"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "transcripts",
                columns: TRANSCRIPT_SPAN_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/014_note_transcription_jobs.sql"
            )),
        ],
    },
    Migration {
        version: 18,
        name: "memories",
        requirements: &[
            SchemaRequirement::Column {
                table: "folders",
                column: "instructions",
            },
            SchemaRequirement::Column {
                table: "folders",
                column: "memory_disabled",
            },
            SchemaRequirement::Table("memories"),
            SchemaRequirement::Table("memory_tombstones"),
            SchemaRequirement::Index("idx_memories_folder_id"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "folders",
                columns: FOLDER_MEMORY_COLUMNS,
            },
            MigrationStep::Sql(include_str!("../../migrations/015_memories.sql")),
        ],
    },
    Migration {
        version: 19,
        name: "share_keys",
        requirements: &[
            SchemaRequirement::Table("share_keys"),
            SchemaRequirement::Table("share_invite_keys"),
            SchemaRequirement::Index("idx_share_keys_item"),
            SchemaRequirement::Index("idx_share_invite_keys_share"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/014_share_keys.sql"
        ))],
    },
    Migration {
        version: 20,
        name: "linear_connector",
        requirements: &[
            SchemaRequirement::Column {
                table: "connector_accounts",
                column: "metadata",
            },
            SchemaRequirement::Table("connector_selected_teams"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "connector_accounts",
                columns: CONNECTOR_METADATA_COLUMN,
            },
            MigrationStep::Sql(include_str!("../../migrations/016_linear_connector.sql")),
        ],
    },
    Migration {
        version: 21,
        name: "connector_actions",
        requirements: &[SchemaRequirement::Table("connector_actions")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/017_connector_actions.sql"
        ))],
    },
    Migration {
        version: 22,
        name: "profile_scoped_data",
        requirements: &[
            SchemaRequirement::Column {
                table: "notes",
                column: "profile",
            },
            SchemaRequirement::Column {
                table: "dictation_history",
                column: "profile",
            },
            SchemaRequirement::Column {
                table: "folders",
                column: "profile",
            },
            SchemaRequirement::Index("idx_notes_profile_created_at"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "notes",
                columns: PROFILE_COLUMN,
            },
            MigrationStep::EnsureColumns {
                table: "dictation_history",
                columns: PROFILE_COLUMN,
            },
            MigrationStep::EnsureColumns {
                table: "folders",
                columns: PROFILE_COLUMN,
            },
            MigrationStep::Sql(
                "CREATE INDEX IF NOT EXISTS idx_notes_profile_created_at
                 ON notes (profile, created_at DESC);",
            ),
        ],
    },
    Migration {
        version: 23,
        name: "session_profiles",
        requirements: &[SchemaRequirement::Table("session_profiles")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/018_session_profiles.sql"
        ))],
    },
    Migration {
        version: 24,
        name: "memory_profiles",
        requirements: &[
            SchemaRequirement::Column {
                table: "memories",
                column: "profile",
            },
            SchemaRequirement::Index("idx_memories_profile_created_at"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "memories",
                columns: PROFILE_COLUMN,
            },
            MigrationStep::Sql(include_str!("../../migrations/019_memory_profiles.sql")),
        ],
    },
    Migration {
        version: 25,
        name: "completed_sessions",
        requirements: &[SchemaRequirement::Table("completed_sessions")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/020_completed_sessions.sql"
        ))],
    },
    Migration {
        version: 26,
        name: "note_calendar_metadata",
        requirements: &[
            SchemaRequirement::Column {
                table: "notes",
                column: "calendar_event_id",
            },
            SchemaRequirement::Column {
                table: "notes",
                column: "calendar_event_title",
            },
            SchemaRequirement::Column {
                table: "notes",
                column: "calendar_event_start_at",
            },
            SchemaRequirement::Column {
                table: "notes",
                column: "calendar_event_end_at",
            },
            SchemaRequirement::Column {
                table: "notes",
                column: "calendar_account_email",
            },
        ],
        steps: &[MigrationStep::EnsureColumns {
            table: "notes",
            columns: NOTE_CALENDAR_COLUMNS,
        }],
    },
    Migration {
        version: 27,
        name: "routine_browser_grants",
        requirements: &[SchemaRequirement::Table("routine_browser_grants")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/021_routine_browser_grants.sql"
        ))],
    },
    Migration {
        version: 28,
        name: "browser_outcome_ledger",
        requirements: &[
            SchemaRequirement::Table("browser_action_outcomes"),
            SchemaRequirement::Table("browser_approval_events"),
            SchemaRequirement::Index("idx_browser_action_outcomes_session"),
            SchemaRequirement::Index("idx_browser_approval_events_session"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/022_browser_outcome_ledger.sql"
        ))],
    },
    Migration {
        version: 29,
        name: "folder_import_paths",
        requirements: &[
            SchemaRequirement::Column {
                table: "folders",
                column: "local_path",
            },
            SchemaRequirement::Index("idx_folders_active_local_path"),
        ],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "folders",
                columns: FOLDER_LOCAL_PATH_COLUMN,
            },
            MigrationStep::Sql(
                "CREATE UNIQUE INDEX IF NOT EXISTS idx_folders_active_local_path
                 ON folders (profile, local_path)
                 WHERE deleted_at IS NULL AND local_path IS NOT NULL;",
            ),
        ],
    },
    Migration {
        version: 30,
        name: "connector_trigger_uniqueness",
        requirements: &[SchemaRequirement::Index(
            "idx_connector_triggers_job_id_unique",
        )],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/023_connector_trigger_uniqueness.sql"
        ))],
    },
    Migration {
        version: 31,
        name: "note_hydration_indexes",
        requirements: &[
            SchemaRequirement::Index("idx_audio_artifacts_note_status_created_at"),
            SchemaRequirement::Index("idx_transcripts_note_created_at"),
            SchemaRequirement::Index("idx_recording_checkpoints_session_kind_created_at"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/024_note_hydration_indexes.sql"
        ))],
    },
    Migration {
        version: 32,
        name: "agent_runtime",
        requirements: &[
            SchemaRequirement::Table("agent_sessions"),
            SchemaRequirement::Table("agent_runs"),
            SchemaRequirement::Table("agent_items"),
            SchemaRequirement::Table("agent_artifacts"),
            SchemaRequirement::Table("agent_skill_settings"),
            SchemaRequirement::Table("agent_migration_manifests"),
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "id",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "title",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "status",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "model",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "safety_mode",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "workspace_path",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "source",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "created_at",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "updated_at",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "completed_at",
            },
            SchemaRequirement::Column {
                table: "agent_sessions",
                column: "last_error",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "id",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "session_id",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "status",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "model",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "started_at",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "updated_at",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "completed_at",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "usage_json",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "interrupted_state_json",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "last_sequence",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "error_code",
            },
            SchemaRequirement::Column {
                table: "agent_runs",
                column: "error_message",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "id",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "session_id",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "run_id",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "sequence",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "kind",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "payload_json",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "external_id",
            },
            SchemaRequirement::Column {
                table: "agent_items",
                column: "created_at",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "id",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "session_id",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "run_id",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "item_id",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "provenance",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "action",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "path",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "original_path",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "mime_type",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "size_bytes",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "available",
            },
            SchemaRequirement::Column {
                table: "agent_artifacts",
                column: "created_at",
            },
            SchemaRequirement::Column {
                table: "agent_skill_settings",
                column: "skill_id",
            },
            SchemaRequirement::Column {
                table: "agent_skill_settings",
                column: "enabled",
            },
            SchemaRequirement::Column {
                table: "agent_skill_settings",
                column: "managed",
            },
            SchemaRequirement::Column {
                table: "agent_skill_settings",
                column: "updated_at",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "migration_key",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "source_path",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "source_fingerprint",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "status",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "source_counts_json",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "imported_counts_json",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "skipped_count",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "errors_json",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "started_at",
            },
            SchemaRequirement::Column {
                table: "agent_migration_manifests",
                column: "completed_at",
            },
            SchemaRequirement::Column {
                table: "session_folders",
                column: "session_id",
            },
            SchemaRequirement::Column {
                table: "session_folders",
                column: "folder_id",
            },
            SchemaRequirement::Column {
                table: "session_folders",
                column: "assigned_at",
            },
            SchemaRequirement::Index("idx_agent_sessions_updated_at"),
            SchemaRequirement::Index("idx_agent_sessions_status"),
            SchemaRequirement::Index("idx_agent_runs_session_started"),
            SchemaRequirement::Index("idx_agent_runs_status"),
            SchemaRequirement::Index("idx_agent_items_session_sequence"),
            SchemaRequirement::Index("idx_agent_items_external_id"),
            SchemaRequirement::Index("idx_agent_artifacts_session_created"),
            SchemaRequirement::Index("idx_session_folders_folder"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/025_agent_runtime.sql"
        ))],
    },
    Migration {
        version: 33,
        name: "agent_routines",
        requirements: &[
            SchemaRequirement::Table("routines"),
            SchemaRequirement::Table("routine_runs"),
            SchemaRequirement::Index("idx_routines_due"),
            SchemaRequirement::Index("idx_routines_claim"),
            SchemaRequirement::Index("idx_routine_runs_routine_started"),
            SchemaRequirement::Index("idx_routine_runs_active"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/026_routines.sql"
        ))],
    },
    Migration {
        version: 34,
        name: "agent_mcp",
        requirements: &[
            SchemaRequirement::Table("agent_mcp_servers"),
            SchemaRequirement::Index("idx_agent_mcp_servers_enabled"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/027_agent_mcp.sql"
        ))],
    },
    Migration {
        version: 35,
        name: "agent_run_mcp_policy",
        requirements: &[
            SchemaRequirement::Table("agent_run_mcp_policies"),
            SchemaRequirement::Index("idx_agent_run_mcp_policies_run"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/028_agent_run_mcp_policy.sql"
        ))],
    },
    Migration {
        version: 36,
        name: "routine_tool_catalog_version",
        requirements: &[SchemaRequirement::Column {
            table: "routines",
            column: "tool_catalog_version",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "routines",
            columns: ROUTINE_TOOL_CATALOG_VERSION_COLUMN,
        }],
    },
    Migration {
        version: 37,
        name: "agent_run_mcp_snapshot",
        requirements: &[SchemaRequirement::Column {
            table: "agent_runs",
            column: "mcp_policy_snapshotted",
        }],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "agent_runs",
                columns: AGENT_RUN_MCP_SNAPSHOT_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/029_agent_run_mcp_snapshot.sql"
            )),
        ],
    },
    Migration {
        version: 38,
        name: "agent_run_skills",
        requirements: &[SchemaRequirement::Column {
            table: "agent_runs",
            column: "enabled_skills_json",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "agent_runs",
            columns: AGENT_RUN_SKILLS_COLUMN,
        }],
    },
    Migration {
        version: 39,
        name: "agent_run_reasoning_effort",
        requirements: &[SchemaRequirement::Column {
            table: "agent_runs",
            column: "reasoning_effort",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "agent_runs",
            columns: AGENT_RUN_REASONING_EFFORT_COLUMN,
        }],
    },
    Migration {
        version: 40,
        name: "agent_run_config",
        requirements: &[SchemaRequirement::Column {
            table: "agent_runs",
            column: "run_config_json",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "agent_runs",
            columns: AGENT_RUN_CONFIG_COLUMN,
        }],
    },
    Migration {
        version: 41,
        name: "companion_devices",
        requirements: &[SchemaRequirement::Table("companion_devices")],
        steps: &[
            // Compare-and-swap revision for remote-safe note edits: linked
            // devices must never overwrite newer local edits (ADR-0048).
            MigrationStep::EnsureColumns {
                table: "notes",
                columns: NOTE_REVISION_COLUMN,
            },
            MigrationStep::Sql(include_str!("../../migrations/030_companion.sql")),
        ],
    },
    Migration {
        version: 42,
        name: "companion_account_scope",
        requirements: &[SchemaRequirement::Column {
            table: "companion_devices",
            column: "account_user_id",
        }],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "companion_devices",
                columns: COMPANION_ACCOUNT_USER_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/031_companion_account_scope.sql"
            )),
        ],
    },
    Migration {
        version: 43,
        name: "companion_operation_state",
        requirements: &[SchemaRequirement::Table("companion_account_state")],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "companion_operations",
                columns: COMPANION_OPERATION_STATE_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/032_companion_operation_state.sql"
            )),
        ],
    },
    // Renumbered from 32 when main advanced past it — positions here are
    // shipped schema versions, so the branch's migration appends after
    // everything main has already stamped (ADR-0037: append-only).
    Migration {
        version: 44,
        name: "meeting_recording_origin",
        requirements: &[
            SchemaRequirement::Column {
                table: "recording_sessions",
                column: "recording_origin",
            },
            SchemaRequirement::Column {
                table: "recording_sessions",
                column: "meeting_app_bundle_families",
            },
            SchemaRequirement::Column {
                table: "recording_sessions",
                column: "auto_finish_eligible",
            },
        ],
        steps: &[MigrationStep::EnsureColumns {
            table: "recording_sessions",
            columns: RECORDING_ORIGIN_COLUMNS,
        }],
    },
    // Renumbered from 44 when meeting_recording_origin merged first (same
    // append-only rule as above).
    Migration {
        version: 45,
        name: "calendar_event_html_link",
        requirements: &[SchemaRequirement::Column {
            table: "notes",
            column: "calendar_event_html_link",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "notes",
            columns: NOTE_CALENDAR_HTML_LINK_COLUMN,
        }],
    },
    // The feature branch used catalog position 44, but main already shipped
    // positions 44 and 45. Preserve those entries and append the file schema
    // after them as required by ADR-0037.
    Migration {
        version: 46,
        name: "companion_files",
        requirements: &[
            SchemaRequirement::Table("companion_browse_roots"),
            SchemaRequirement::Table("companion_uploads"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/033_companion_files.sql"
        ))],
    },
    // The active Computer use fix branch also used catalog position 44.
    // Preserve every earlier position and append its audit schema.
    Migration {
        version: 47,
        name: "companion_computer_use_approval_audit",
        requirements: &[
            SchemaRequirement::Table("companion_computer_use_approval_audit"),
            SchemaRequirement::Index("idx_companion_computer_use_approval_audit_request"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/033_companion_computer_use_approval_audit.sql"
        ))],
    },
    Migration {
        version: 48,
        name: "linear_managed_mcp",
        requirements: &[
            SchemaRequirement::Table("linear_mcp_connection"),
            SchemaRequirement::Column {
                table: "linear_mcp_connection",
                column: "preset_id",
            },
            SchemaRequirement::Column {
                table: "linear_mcp_connection",
                column: "state",
            },
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/033_linear_managed_mcp.sql"
        ))],
    },
    Migration {
        version: 49,
        name: "linear_managed_mcp_repair",
        requirements: &[
            SchemaRequirement::Table("linear_mcp_connection"),
            SchemaRequirement::Column {
                table: "linear_mcp_connection",
                column: "preset_id",
            },
            SchemaRequirement::Column {
                table: "linear_mcp_connection",
                column: "state",
            },
        ],
        // The ledger repair is implemented in
        // repair_prerelease_linear_managed_mcp_stamp. Re-running the
        // compatibility DDL keeps clean and prerelease databases convergent.
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/033_linear_managed_mcp.sql"
        ))],
    },
    Migration {
        version: 50,
        name: "agent_artifact_display_names",
        requirements: &[SchemaRequirement::Column {
            table: "agent_artifacts",
            column: "display_name",
        }],
        steps: &[
            MigrationStep::EnsureColumns {
                table: "agent_artifacts",
                columns: AGENT_ARTIFACT_DISPLAY_NAME_COLUMN,
            },
            MigrationStep::Sql(include_str!(
                "../../migrations/034_agent_artifact_display_names.sql"
            )),
        ],
    },
    Migration {
        version: 51,
        name: "note_transcription_chunks",
        requirements: &[
            SchemaRequirement::Table("note_transcription_chunks"),
            SchemaRequirement::Index("idx_note_transcription_chunks_session"),
        ],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/035_note_transcription_chunks.sql"
        ))],
    },
    Migration {
        version: 52,
        name: "retire_native_chat",
        requirements: &[SchemaRequirement::Table("agent_secret_retirements")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/036_retire_native_chat.sql"
        ))],
    },
    Migration {
        version: 53,
        name: "retire_sharing_keys",
        // A unique marker distinguishes this data cleanup from version 19
        // when adopting an unversioned historical database.
        requirements: &[SchemaRequirement::Table("sharing_retirement")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/037_retire_sharing_keys.sql"
        ))],
    },
    Migration {
        version: 54,
        name: "dictation_app_bundle_id",
        requirements: &[SchemaRequirement::Column {
            table: "dictation_history",
            column: "app_bundle_id",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "dictation_history",
            columns: DICTATION_APP_COLUMN,
        }],
    },
    Migration {
        version: 55,
        name: "agent_session_approval_mode",
        requirements: &[SchemaRequirement::Column {
            table: "agent_sessions",
            column: "approval_mode",
        }],
        steps: &[MigrationStep::EnsureColumns {
            table: "agent_sessions",
            columns: &[ColumnDefinition {
                name: "approval_mode",
                definition: "TEXT NOT NULL DEFAULT 'approvals'",
            }],
        }],
    },
    Migration {
        version: 56,
        name: "interrupt_unresolved_tool_calls",
        requirements: &[SchemaRequirement::Table("agent_tool_terminality_backfill")],
        steps: &[MigrationStep::Sql(include_str!(
            "../../migrations/038_interrupt_unresolved_tool_calls.sql"
        ))],
    },
    Migration {
        version: 57,
        name: "meeting_video",
        requirements: &[
            SchemaRequirement::Table("video_artifacts"),
            SchemaRequirement::Column {
                table: "generation_results",
                column: "video_analysis",
            },
        ],
        steps: &[
            MigrationStep::Sql(include_str!("../../migrations/039_meeting_video.sql")),
            MigrationStep::EnsureColumns {
                table: "generation_results",
                columns: &[ColumnDefinition {
                    name: "video_analysis",
                    definition: "TEXT",
                }],
            },
        ],
    },
    Migration {
        version: 58,
        name: "recording_marks",
        requirements: &[
            SchemaRequirement::Table("recording_marks"),
            SchemaRequirement::Index("idx_recording_marks_note"),
        ],
        steps: &[MigrationStep::Sql(
            "CREATE TABLE IF NOT EXISTS recording_marks (
                id TEXT PRIMARY KEY NOT NULL,
                note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
                session_id TEXT NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
                elapsed_ms INTEGER NOT NULL CHECK (elapsed_ms >= 0),
                label TEXT NOT NULL,
                created_at TEXT NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_recording_marks_note
             ON recording_marks (note_id, created_at);",
        )],
    },
];

const NOTE_REVISION_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "revision",
    definition: "INTEGER NOT NULL DEFAULT 1",
}];
const NOTE_CALENDAR_HTML_LINK_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "calendar_event_html_link",
    definition: "TEXT",
}];
const COMPANION_ACCOUNT_USER_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "account_user_id",
    definition: "TEXT NOT NULL DEFAULT ''",
}];
const COMPANION_OPERATION_STATE_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "operation_state",
    definition: "TEXT NOT NULL DEFAULT 'completed'",
}];
const AGENT_ARTIFACT_DISPLAY_NAME_COLUMN: &[ColumnDefinition] = &[ColumnDefinition {
    name: "display_name",
    definition: "TEXT",
}];

struct AppliedMigration {
    version: i64,
    name: String,
}

#[derive(Default)]
struct SchemaSnapshot {
    tables: HashSet<String>,
    indexes: HashSet<String>,
    columns: HashSet<(String, String)>,
}

impl SchemaSnapshot {
    async fn load(transaction: &mut SqliteTransaction<'_>) -> Result<Self, sqlx::Error> {
        let object_rows = query(
            "SELECT type, name
             FROM sqlite_schema
             WHERE type IN ('table', 'index')",
        )
        .fetch_all(&mut **transaction)
        .await?;
        let column_rows = query(
            "SELECT schema.name AS table_name, column_info.name AS column_name
             FROM sqlite_schema AS schema
             JOIN pragma_table_info(schema.name) AS column_info
             WHERE schema.type = 'table'",
        )
        .fetch_all(&mut **transaction)
        .await?;

        let mut snapshot = Self::default();
        for row in object_rows {
            let object_type: String = row.get("type");
            let name: String = row.get("name");
            match object_type.as_str() {
                "table" => {
                    snapshot.tables.insert(name);
                }
                "index" => {
                    snapshot.indexes.insert(name);
                }
                _ => {}
            }
        }
        for row in column_rows {
            snapshot
                .columns
                .insert((row.get("table_name"), row.get("column_name")));
        }
        Ok(snapshot)
    }

    fn has_application_tables(&self) -> bool {
        self.tables
            .iter()
            .any(|table| table != SCHEMA_MIGRATIONS_TABLE && !table.starts_with("sqlite_"))
    }

    fn satisfies(&self, requirement: SchemaRequirement) -> bool {
        match requirement {
            SchemaRequirement::Table(table) => self.tables.contains(table),
            SchemaRequirement::Index(index) => self.indexes.contains(index),
            SchemaRequirement::Column { table, column } => self
                .columns
                .contains(&(table.to_string(), column.to_string())),
            SchemaRequirement::MissingIndex(index) => !self.indexes.contains(index),
        }
    }
}

pub async fn run_migrations(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    run_migration_catalog(pool, MIGRATIONS).await
}

async fn run_migration_catalog(
    pool: &SqlitePool,
    migrations: &[Migration],
) -> Result<(), sqlx::Error> {
    validate_catalog(migrations)?;

    if let Some(applied) = read_applied_migrations_from_pool(pool).await? {
        if is_prerelease_agent_runtime_stamp(&applied, migrations)
            || is_prerelease_linear_managed_mcp_stamp(&applied, migrations)
        {
            // Repair needs the same write lock and transaction as a normal
            // migration, so continue into migrate_locked.
        } else {
            let current = validate_applied_migrations(&applied, migrations)?;
            if current == migrations.len() {
                return Ok(());
            }
        }
    }

    // Serializing migration writers before inspecting the schema prevents two
    // processes from deriving and stamping the same unversioned database.
    let mut transaction = pool.begin_with("BEGIN IMMEDIATE").await?;
    let migration_result = migrate_locked(&mut transaction, migrations).await;
    match migration_result {
        Ok(()) => transaction.commit().await,
        Err(error) => match transaction.rollback().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(sqlx::Error::Protocol(format!(
                "migration failed ({error}); rollback also failed ({rollback_error})"
            ))),
        },
    }
}

async fn migrate_locked(
    transaction: &mut SqliteTransaction<'_>,
    migrations: &[Migration],
) -> Result<(), sqlx::Error> {
    let mut applied = read_applied_migrations_from_transaction(transaction).await?;
    if let Some(ref stamped) = applied {
        if repair_prerelease_agent_runtime_stamp(transaction, stamped, migrations).await? {
            applied = read_applied_migrations_from_transaction(transaction).await?;
        }
    }
    if let Some(ref stamped) = applied {
        if repair_prerelease_linear_managed_mcp_stamp(transaction, stamped, migrations).await? {
            applied = read_applied_migrations_from_transaction(transaction).await?;
        }
    }
    let current = match applied {
        Some(ref applied) if !applied.is_empty() => {
            validate_applied_migrations(applied, migrations)?
        }
        _ => {
            let snapshot = SchemaSnapshot::load(transaction).await?;
            if adopt_prerelease_agent_runtime_schema(transaction, &snapshot, migrations).await? {
                migrations
                    .iter()
                    .position(|migration| migration.name == "agent_runtime")
                    .map(|index| index + 1)
                    .ok_or_else(|| {
                        sqlx::Error::Protocol(
                            "adopted agent runtime is missing from migration catalog".into(),
                        )
                    })?
            } else {
                let detected = detect_legacy_version(&snapshot, migrations)?;
                create_schema_migrations_table(transaction).await?;
                stamp_legacy_migrations(transaction, &migrations[..detected]).await?;
                detected
            }
        }
    };

    for migration in &migrations[current..] {
        if let Err(error) = apply_migration(transaction, migration).await {
            return Err(sqlx::Error::Protocol(format!(
                "migration {} ({}) failed: {error}",
                migration.version, migration.name
            )));
        }
        query(
            "INSERT INTO schema_migrations (version, name)
             VALUES (?, ?)",
        )
        .bind(migration.version)
        .bind(migration.name)
        .execute(&mut **transaction)
        .await?;
    }

    Ok(())
}

async fn repair_prerelease_agent_runtime_stamp(
    transaction: &mut SqliteTransaction<'_>,
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> Result<bool, sqlx::Error> {
    if !is_prerelease_agent_runtime_stamp(applied, migrations) {
        return Ok(false);
    }
    let Some(runtime_index) = migrations
        .iter()
        .position(|migration| migration.name == "agent_runtime")
    else {
        return Ok(false);
    };
    let runtime = &migrations[runtime_index];
    // PR #920 prerelease builds stamped the runtime at either version 30 or
    // version 31 as main appended migrations underneath the branch. The
    // released runtime belongs at version 32. Preserve main's release order
    // and adopt the already-installed runtime without replaying its SQL.
    let displaced_index = applied
        .last()
        .and_then(|last| usize::try_from(last.version).ok())
        .and_then(|version| version.checked_sub(1))
        .ok_or_else(|| {
            sqlx::Error::Protocol(
                "agent runtime prerelease repair has no displaced migration".into(),
            )
        })?;
    let displaced = &migrations[displaced_index];
    let intervening = &migrations[displaced_index + 1..runtime_index];

    validate_applied_migrations(&applied[..displaced_index], migrations)?;
    let snapshot = SchemaSnapshot::load(transaction).await?;
    if !runtime
        .requirements
        .iter()
        .copied()
        .all(|requirement| snapshot.satisfies(requirement))
    {
        return Ok(false);
    }

    apply_migration(transaction, displaced).await?;
    query(
        "UPDATE schema_migrations
         SET name = ?
         WHERE version = ? AND name = ?",
    )
    .bind(displaced.name)
    .bind(displaced.version)
    .bind(runtime.name)
    .execute(&mut **transaction)
    .await?;
    for migration in intervening {
        apply_migration(transaction, migration).await?;
        stamp_legacy_migrations(transaction, std::slice::from_ref(migration)).await?;
    }
    // The prerelease schema already has the complete runtime. Replaying this
    // SQL would attempt to import tables it retired, so only stamp it after
    // the requirement check above.
    stamp_legacy_migrations(transaction, std::slice::from_ref(runtime)).await?;
    Ok(true)
}

async fn repair_prerelease_linear_managed_mcp_stamp(
    transaction: &mut SqliteTransaction<'_>,
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> Result<bool, sqlx::Error> {
    let Some(displaced_index) = prerelease_linear_managed_mcp_index(applied, migrations) else {
        return Ok(false);
    };

    validate_applied_migrations(&applied[..displaced_index], migrations)?;
    let managed_mcp_index = migrations
        .iter()
        .position(|migration| migration.name == "linear_managed_mcp")
        .ok_or_else(|| {
            sqlx::Error::Protocol(
                "Linear managed MCP prerelease repair is missing its migration".into(),
            )
        })?;
    let managed_mcp = &migrations[managed_mcp_index];
    let managed_mcp_repair = migrations
        .iter()
        .find(|migration| migration.name == "linear_managed_mcp_repair")
        .ok_or_else(|| {
            sqlx::Error::Protocol(
                "Linear managed MCP prerelease repair is missing its repair migration".into(),
            )
        })?;
    let snapshot = SchemaSnapshot::load(transaction).await?;
    if !managed_mcp
        .requirements
        .iter()
        .copied()
        .all(|requirement| snapshot.satisfies(requirement))
    {
        return Ok(false);
    }

    for (old_version, new_version, name) in [
        (
            applied[displaced_index + 1].version,
            managed_mcp_repair.version,
            managed_mcp_repair.name,
        ),
        (
            applied[displaced_index].version,
            managed_mcp.version,
            managed_mcp.name,
        ),
    ] {
        let result = query(
            "UPDATE schema_migrations
             SET version = ?
             WHERE version = ? AND name = ?",
        )
        .bind(new_version)
        .bind(old_version)
        .bind(name)
        .execute(&mut **transaction)
        .await?;
        if result.rows_affected() != 1 {
            return Err(sqlx::Error::Protocol(format!(
                "Linear managed MCP prerelease repair could not move {name}"
            )));
        }
    }
    for migration in &migrations[displaced_index..managed_mcp_index] {
        apply_migration(transaction, migration).await?;
        stamp_legacy_migrations(transaction, std::slice::from_ref(migration)).await?;
    }
    Ok(true)
}

fn prerelease_linear_managed_mcp_index(
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> Option<usize> {
    let calendar_index = migrations
        .iter()
        .position(|migration| migration.name == "calendar_event_html_link")?;
    let calendar = &migrations[calendar_index];
    let displaced_index = if applied.len() == calendar_index + 2 {
        calendar_index
    } else if applied.len() == calendar_index + 3
        && applied.get(calendar_index).is_some_and(|applied| {
            applied.version == calendar.version && applied.name == calendar.name
        })
    {
        calendar_index + 1
    } else {
        return None;
    };
    let managed_mcp = applied.get(displaced_index)?;
    let managed_mcp_repair = applied.get(displaced_index + 1)?;

    // Linear prerelease builds stamped 45/46 before the calendar migration,
    // then 46/47 after the calendar repair. Main subsequently shipped two
    // companion migrations at 46/47, so both exact branch histories must move
    // to the append-only Linear positions without replaying provider state.
    let displaced_version =
        calendar.version + i64::try_from(displaced_index - calendar_index).ok()?;
    (managed_mcp.version == displaced_version
        && managed_mcp.name == "linear_managed_mcp"
        && managed_mcp_repair.version == displaced_version + 1
        && managed_mcp_repair.name == "linear_managed_mcp_repair")
        .then_some(displaced_index)
}

fn is_prerelease_linear_managed_mcp_stamp(
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> bool {
    prerelease_linear_managed_mcp_index(applied, migrations).is_some()
}

fn is_prerelease_agent_runtime_stamp(
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> bool {
    let Some(runtime_index) = migrations
        .iter()
        .position(|migration| migration.name == "agent_runtime")
    else {
        return false;
    };
    let Some(runtime) = migrations.get(runtime_index) else {
        return false;
    };
    let Some(last) = applied.last() else {
        return false;
    };
    let known_displaced_version =
        last.version == runtime.version - 2 || last.version == runtime.version - 1;
    let Some(displaced) = last
        .version
        .checked_sub(1)
        .and_then(|index| usize::try_from(index).ok())
        .and_then(|index| migrations.get(index))
    else {
        return false;
    };
    known_displaced_version
        && matches!(
            displaced.name,
            "connector_trigger_uniqueness" | "note_hydration_indexes"
        )
        && applied.len() == usize::try_from(last.version).unwrap_or_default()
        && last.version == displaced.version
        && last.name == runtime.name
}

/// The runtime branch was exercised before the version catalog reached those
/// installs. Its runtime schema is complete, but it has no ledger and can be
/// missing the two mainline migrations that landed between its old version-30
/// stamp and the released version-32 runtime. Adopt that exact known shape in
/// one transaction: prove the historical prefix, apply or stamp the intervening
/// mainline migrations, then stamp the already-present runtime without replay.
async fn adopt_prerelease_agent_runtime_schema(
    transaction: &mut SqliteTransaction<'_>,
    snapshot: &SchemaSnapshot,
    migrations: &[Migration],
) -> Result<bool, sqlx::Error> {
    let Some(runtime_index) = migrations
        .iter()
        .position(|migration| migration.name == "agent_runtime")
    else {
        return Ok(false);
    };
    let Some(first_intervening_index) = runtime_index.checked_sub(2) else {
        return Ok(false);
    };
    let runtime = &migrations[runtime_index];
    if !migration_requirements_satisfied(snapshot, runtime) {
        return Ok(false);
    }

    // Versions 9 through 11 were retired by the runtime SQL, so their original
    // landmarks are absent on this one known replacement schema. Every other
    // earlier migration must still be directly observable.
    if !migrations[..first_intervening_index]
        .iter()
        .all(|migration| legacy_requirement_satisfied(snapshot, migration, true))
    {
        return Ok(false);
    }

    create_schema_migrations_table(transaction).await?;
    stamp_legacy_migrations(transaction, &migrations[..first_intervening_index]).await?;
    for migration in &migrations[first_intervening_index..runtime_index] {
        if !migration_requirements_satisfied(snapshot, migration) {
            apply_migration(transaction, migration).await?;
        }
        stamp_legacy_migrations(transaction, std::slice::from_ref(migration)).await?;
    }
    stamp_legacy_migrations(transaction, std::slice::from_ref(runtime)).await?;
    Ok(true)
}

fn migration_requirements_satisfied(snapshot: &SchemaSnapshot, migration: &Migration) -> bool {
    migration
        .requirements
        .iter()
        .copied()
        .all(|requirement| snapshot.satisfies(requirement))
}

fn legacy_requirement_satisfied(
    snapshot: &SchemaSnapshot,
    migration: &Migration,
    agent_runtime_installed: bool,
) -> bool {
    migration_requirements_satisfied(snapshot, migration)
        || (agent_runtime_installed
            && matches!(
                migration.name,
                "agent_workspace" | "agent_task_session_identity" | "agent_message_identity"
            ))
}

fn validate_catalog(migrations: &[Migration]) -> Result<(), sqlx::Error> {
    for (index, migration) in migrations.iter().enumerate() {
        let expected_version = index as i64 + 1;
        if migration.version != expected_version || migration.name.is_empty() {
            return Err(sqlx::Error::Protocol(format!(
                "invalid migration catalog entry at position {expected_version}"
            )));
        }
        if migration.requirements.is_empty() {
            return Err(sqlx::Error::Protocol(format!(
                "migration {} ({}) has no schema requirements and cannot be safely detected on \
                 an unversioned database",
                migration.version, migration.name
            )));
        }
    }
    Ok(())
}

fn validate_applied_migrations(
    applied: &[AppliedMigration],
    migrations: &[Migration],
) -> Result<usize, sqlx::Error> {
    // Dev builds on the isolated dev data dir tolerate a database stamped past
    // this catalog: workspaces share one dev database, so an unmerged branch
    // carrying an extra (append-only, additive per ADR-0037) migration would
    // otherwise brick every other workspace's build. Production builds — and
    // dev builds pointed at production data via OS_CLOVY_USE_PROD_DATA_DIR —
    // keep refusing, as downgrade protection for real data.
    let tolerate_newer = cfg!(debug_assertions) && !crate::app_paths::use_prod_data_dir();
    validate_applied_migrations_with_tolerance(applied, migrations, tolerate_newer)
}

fn validate_applied_migrations_with_tolerance(
    applied: &[AppliedMigration],
    migrations: &[Migration],
    tolerate_newer: bool,
) -> Result<usize, sqlx::Error> {
    if applied.len() > migrations.len() {
        if !tolerate_newer {
            return Err(sqlx::Error::Protocol(
                "database schema is newer than this Clovy build".to_string(),
            ));
        }
        eprintln!(
            "warning: database schema is {} migration(s) ahead of this Clovy build; \
             continuing because this is a dev build on the dev data dir",
            applied.len() - migrations.len()
        );
    }
    let known = applied.len().min(migrations.len());
    for (index, applied_migration) in applied[..known].iter().enumerate() {
        let expected = &migrations[index];
        // Names are persisted migration identity, not descriptive labels. A
        // rename makes every stamped install fail this check. Per ADR-0037,
        // existing version/name pairs are append-only and must never be edited.
        if applied_migration.version != expected.version || applied_migration.name != expected.name
        {
            return Err(sqlx::Error::Protocol(format!(
                "schema_migrations diverges at version {}: expected {}, found {} ({})",
                expected.version, expected.name, applied_migration.version, applied_migration.name
            )));
        }
    }
    Ok(known)
}

fn detect_legacy_version(
    snapshot: &SchemaSnapshot,
    migrations: &[Migration],
) -> Result<usize, sqlx::Error> {
    if !snapshot.has_application_tables() {
        return Ok(0);
    }

    let mut detected = 0;
    let mut first_missing: Option<&Migration> = None;
    // The first Clovy-owned agent runtime migration intentionally retires the
    // three Hermes-era agent tables after importing them. Builds that shipped
    // that migration before the version catalog landed therefore have a
    // complete, unversioned runtime schema where migrations 9 through 11 are
    // no longer directly observable. Treat those retired requirements as
    // satisfied only when the complete replacement schema is present. Every
    // unrelated historical requirement is still checked normally.
    let agent_runtime_installed = migrations
        .iter()
        .find(|migration| migration.name == "agent_runtime")
        .is_some_and(|migration| {
            migration
                .requirements
                .iter()
                .copied()
                .all(|requirement| snapshot.satisfies(requirement))
        });
    let agent_runtime_tables_present = migrations
        .iter()
        .find(|migration| migration.name == "agent_runtime")
        .is_some_and(|migration| {
            migration.requirements.iter().any(|requirement| {
                matches!(*requirement, SchemaRequirement::Table(table) if snapshot.tables.contains(table))
            })
        });
    if agent_runtime_tables_present && !agent_runtime_installed {
        return Err(sqlx::Error::Protocol(
            "unversioned database contains an incomplete Clovy agent runtime schema".to_string(),
        ));
    }
    for migration in migrations {
        let applied = migration
            .requirements
            .iter()
            .copied()
            .all(|requirement| snapshot.satisfies(requirement))
            || (agent_runtime_installed
                && matches!(
                    migration.name,
                    "agent_workspace" | "agent_task_session_identity" | "agent_message_identity"
                ));
        if applied {
            if let Some(missing) = first_missing {
                return Err(sqlx::Error::Protocol(format!(
                    "unversioned database has migration {} ({}) but is missing earlier migration {} ({})",
                    migration.version, migration.name, missing.version, missing.name
                )));
            }
            detected = migration.version as usize;
        } else if first_missing.is_none() {
            first_missing = Some(migration);
        }
    }

    if detected == 0 {
        return Err(sqlx::Error::Protocol(
            "unversioned database does not match a known Clovy schema".to_string(),
        ));
    }
    Ok(detected)
}

async fn apply_migration(
    transaction: &mut SqliteTransaction<'_>,
    migration: &Migration,
) -> Result<(), sqlx::Error> {
    for step in migration.steps {
        match *step {
            MigrationStep::Sql(sql) => execute_sql_batch(transaction, sql).await?,
            MigrationStep::EnsureColumns { table, columns } => {
                ensure_columns(transaction, table, columns).await?
            }
            MigrationStep::DropIndex(index) => drop_index(transaction, index).await?,
        }
    }
    Ok(())
}

async fn execute_sql_batch(
    transaction: &mut SqliteTransaction<'_>,
    sql: &str,
) -> Result<(), sqlx::Error> {
    for statement in sql.split(';') {
        let statement = statement.trim();
        if !statement.is_empty() {
            query(statement).execute(&mut **transaction).await?;
        }
    }
    Ok(())
}

async fn ensure_columns(
    transaction: &mut SqliteTransaction<'_>,
    table: &str,
    columns: &[ColumnDefinition],
) -> Result<(), sqlx::Error> {
    let pragma = format!("PRAGMA table_info({})", quote_sqlite_identifier(table));
    let rows = query(&pragma).fetch_all(&mut **transaction).await?;
    let existing: HashSet<String> = rows.into_iter().map(|row| row.get("name")).collect();

    for column in columns {
        if existing.contains(column.name) {
            continue;
        }
        let alter = format!(
            "ALTER TABLE {} ADD COLUMN {} {}",
            quote_sqlite_identifier(table),
            quote_sqlite_identifier(column.name),
            column.definition
        );
        query(&alter).execute(&mut **transaction).await?;
    }
    Ok(())
}

async fn drop_index(
    transaction: &mut SqliteTransaction<'_>,
    index: &str,
) -> Result<(), sqlx::Error> {
    let sql = format!("DROP INDEX IF EXISTS {}", quote_sqlite_identifier(index));
    query(&sql).execute(&mut **transaction).await?;
    Ok(())
}

async fn create_schema_migrations_table(
    transaction: &mut SqliteTransaction<'_>,
) -> Result<(), sqlx::Error> {
    query(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version INTEGER PRIMARY KEY NOT NULL,
            name TEXT NOT NULL UNIQUE,
            applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        )",
    )
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

async fn stamp_legacy_migrations(
    transaction: &mut SqliteTransaction<'_>,
    migrations: &[Migration],
) -> Result<(), sqlx::Error> {
    for migration in migrations {
        query(
            "INSERT INTO schema_migrations (version, name)
             VALUES (?, ?)",
        )
        .bind(migration.version)
        .bind(migration.name)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

async fn read_applied_migrations_from_pool(
    pool: &SqlitePool,
) -> Result<Option<Vec<AppliedMigration>>, sqlx::Error> {
    let exists = query(
        "SELECT 1
         FROM sqlite_schema
         WHERE type = 'table' AND name = 'schema_migrations'",
    )
    .fetch_optional(pool)
    .await?
    .is_some();
    if !exists {
        return Ok(None);
    }
    let rows = query(
        "SELECT version, name
         FROM schema_migrations
         ORDER BY version",
    )
    .fetch_all(pool)
    .await?;
    Ok(Some(
        rows.into_iter()
            .map(|row| AppliedMigration {
                version: row.get("version"),
                name: row.get("name"),
            })
            .collect(),
    ))
}

async fn read_applied_migrations_from_transaction(
    transaction: &mut SqliteTransaction<'_>,
) -> Result<Option<Vec<AppliedMigration>>, sqlx::Error> {
    let exists = query(
        "SELECT 1
         FROM sqlite_schema
         WHERE type = 'table' AND name = 'schema_migrations'",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .is_some();
    if !exists {
        return Ok(None);
    }
    let rows = query(
        "SELECT version, name
         FROM schema_migrations
         ORDER BY version",
    )
    .fetch_all(&mut **transaction)
    .await?;
    Ok(Some(
        rows.into_iter()
            .map(|row| AppliedMigration {
                version: row.get("version"),
                name: row.get("name"),
            })
            .collect(),
    ))
}

fn quote_sqlite_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx_sqlite::SqlitePoolOptions;

    const FAILING_MIGRATIONS: &[Migration] = &[
        Migration {
            version: 1,
            name: "transaction_existing",
            requirements: &[SchemaRequirement::Table("transaction_existing")],
            steps: &[MigrationStep::Sql(
                "CREATE TABLE transaction_existing (id INTEGER PRIMARY KEY);",
            )],
        },
        Migration {
            version: 2,
            name: "transaction_pending_success",
            requirements: &[SchemaRequirement::Table("transaction_pending_success")],
            steps: &[MigrationStep::Sql(
                "CREATE TABLE transaction_pending_success (id INTEGER PRIMARY KEY);",
            )],
        },
        Migration {
            version: 3,
            name: "transaction_failure",
            requirements: &[SchemaRequirement::Table("transaction_failure")],
            steps: &[MigrationStep::Sql(
                "CREATE TABLE transaction_failure (id INTEGER PRIMARY KEY);
                 INSERT INTO table_that_does_not_exist (id) VALUES (1);",
            )],
        },
    ];
    const EMPTY_REQUIREMENTS_MIGRATIONS: &[Migration] = &[Migration {
        version: 1,
        name: "missing_requirements",
        requirements: &[],
        steps: &[],
    }];

    async fn test_pool() -> SqlitePool {
        SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory SQLite")
    }

    #[tokio::test]
    async fn legacy_approval_mode_column_is_retained_with_its_original_default() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..53])
            .await
            .unwrap();
        query("INSERT INTO agent_sessions (id, created_at, updated_at) VALUES ('old-session', 'now', 'now')")
            .execute(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let row = query("SELECT approval_mode FROM agent_sessions WHERE id = 'old-session'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("approval_mode"), "approvals");
    }

    async fn table_exists(pool: &SqlitePool, table: &str) -> bool {
        query(
            "SELECT 1
             FROM sqlite_schema
             WHERE type = 'table' AND name = ?",
        )
        .bind(table)
        .fetch_optional(pool)
        .await
        .expect("table lookup")
        .is_some()
    }

    async fn install_non_replay_guard(pool: &SqlitePool) {
        query(
            "INSERT INTO agent_tasks (
                id, title, prompt, status, safety_profile, created_at, updated_at
             ) VALUES ('task', 'title', 'prompt', 'completed', 'default', 'now', 'now')",
        )
        .execute(pool)
        .await
        .expect("agent task");
        for id in ["message-1", "message-2"] {
            query(
                "INSERT INTO agent_messages (
                    id, task_id, role, content, created_at, external_id
                 ) VALUES (?, 'task', 'assistant', 'same', 'now', NULL)",
            )
            .bind(id)
            .execute(pool)
            .await
            .expect("duplicate legacy message");
        }
        query(
            "CREATE TRIGGER reject_agent_message_replay
             BEFORE DELETE ON agent_messages
             BEGIN
               SELECT RAISE(ABORT, 'destructive migration replayed');
             END",
        )
        .execute(pool)
        .await
        .expect("delete guard");
    }

    async fn install_runtime_non_replay_guard(pool: &SqlitePool) {
        query(
            "INSERT INTO agent_sessions (
                id, title, status, model, safety_mode, source, created_at, updated_at
             ) VALUES ('session', 'title', 'idle', 'auto', 'sandboxed', 'user', 'now', 'now')",
        )
        .execute(pool)
        .await
        .expect("agent session");
        for (sequence, id) in ["message-1", "message-2"].into_iter().enumerate() {
            query(
                "INSERT INTO agent_items (
                    id, session_id, sequence, kind, payload_json, created_at
                 ) VALUES (?, 'session', ?, 'assistant_message', '{}', 'now')",
            )
            .bind(id)
            .bind(sequence as i64)
            .execute(pool)
            .await
            .expect("runtime message");
        }
        query(
            "CREATE TRIGGER reject_agent_runtime_replay
             BEFORE DELETE ON agent_items
             BEGIN
               SELECT RAISE(ABORT, 'destructive runtime migration replayed');
             END",
        )
        .execute(pool)
        .await
        .expect("runtime delete guard");
    }

    async fn assert_latest_stamp(pool: &SqlitePool) {
        let row = query(
            "SELECT COUNT(*) AS count, MAX(version) AS version
             FROM schema_migrations",
        )
        .fetch_one(pool)
        .await
        .expect("migration stamp");
        assert_eq!(row.get::<i64, _>("count"), MIGRATIONS.len() as i64);
        assert_eq!(
            row.get::<i64, _>("version"),
            MIGRATIONS.last().expect("latest migration").version
        );
    }

    #[tokio::test]
    async fn fresh_database_runs_every_migration_and_stamps_latest() {
        let pool = test_pool().await;

        run_migrations(&pool).await.expect("fresh migrations");

        assert!(table_exists(&pool, "browser_action_outcomes").await);
        assert!(table_exists(&pool, "connector_actions").await);
        assert!(table_exists(&pool, "companion_browse_roots").await);
        assert!(table_exists(&pool, "companion_uploads").await);
        assert!(table_exists(&pool, "companion_computer_use_approval_audit").await);
        let companion_tail = query(
            "SELECT version, name
             FROM schema_migrations
             WHERE version >= 46
             ORDER BY version",
        )
        .fetch_all(&pool)
        .await
        .expect("companion migration stamps")
        .into_iter()
        .map(|row| (row.get::<i64, _>("version"), row.get::<String, _>("name")))
        .collect::<Vec<_>>();
        assert_eq!(
            companion_tail,
            vec![
                (46, "companion_files".to_string()),
                (47, "companion_computer_use_approval_audit".to_string()),
                (48, "linear_managed_mcp".to_string()),
                (49, "linear_managed_mcp_repair".to_string()),
                (50, "agent_artifact_display_names".to_string()),
                (51, "note_transcription_chunks".to_string()),
                (52, "retire_native_chat".to_string()),
                (53, "retire_sharing_keys".to_string()),
                (54, "dictation_app_bundle_id".to_string()),
                (55, "agent_session_approval_mode".to_string()),
                (56, "interrupt_unresolved_tool_calls".to_string()),
                (57, "meeting_video".to_string()),
                (58, "recording_marks".to_string()),
            ]
        );
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn dictation_app_migration_preserves_legacy_rows() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..53])
            .await
            .unwrap();
        query("INSERT INTO dictation_history (id, text, language, provider, profile, created_at) VALUES ('legacy', 'old', NULL, 'test', 'default', '2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        run_migration_catalog(&pool, MIGRATIONS).await.unwrap();
        let row = query("SELECT text, app_bundle_id FROM dictation_history WHERE id = 'legacy'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("text"), "old");
        assert_eq!(row.get::<Option<String>, _>("app_bundle_id"), None);
    }

    #[tokio::test]
    async fn meeting_video_upgrades_retired_main_without_replaying_history() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..56])
            .await
            .unwrap();
        let stamps: Vec<(i64, String, String)> = sqlx::query_as::query_as(
            "SELECT version, name, applied_at FROM schema_migrations ORDER BY version",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        install_runtime_non_replay_guard(&pool).await;
        let repos = crate::db::repositories::Repositories::new(pool.clone());
        query("INSERT INTO notes (id, title, processing_status, profile, created_at, updated_at) VALUES ('video-upgrade-note', 'Retained note', 'draft', 'default', 'now', 'now')")
            .execute(&pool).await.unwrap();
        run_migrations(&pool).await.unwrap();
        let after: Vec<(i64, String, String)> = sqlx::query_as::query_as(
            "SELECT version, name, applied_at FROM schema_migrations WHERE version <= 56 ORDER BY version",
        ).fetch_all(&pool).await.unwrap();
        assert_eq!(
            stamps, after,
            "applied main migrations must remain immutable"
        );
        assert!(table_exists(&pool, "video_artifacts").await);
        let notice: Option<String> = sqlx::query_scalar::query_scalar(
            "SELECT video_analysis FROM generation_results LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .unwrap()
        .flatten();
        assert!(notice.is_none());
        assert_eq!(
            repos.get_note("video-upgrade-note").await.unwrap().title,
            "Retained note"
        );
        let count: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_items")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
        run_migrations(&pool).await.unwrap();
        assert_latest_stamp(&pool).await;
        assert!(table_exists(&pool, "recording_marks").await);
    }

    #[tokio::test]
    async fn retired_sharing_keys_are_purged_without_changing_notes_or_history() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..52])
            .await
            .expect("pre-cut schema");
        let repos = crate::db::repositories::Repositories::new(pool.clone());
        // This fixture predates the video column, so insert with its historical schema.
        let note_id = "sharing-retirement-note";
        query("INSERT INTO notes (id, title, processing_status, profile, created_at, updated_at) VALUES (?, 'Retained note', 'draft', 'default', 'now', 'now')")
            .bind(note_id).execute(&pool).await.expect("synthetic historical note");
        install_runtime_non_replay_guard(&pool).await;
        query("INSERT INTO share_keys (share_id, item_kind, item_id, content_key, created_at) VALUES ('old-share', 'note', ?, X'010203', 'now')")
            .bind(note_id).execute(&pool).await.expect("synthetic content key");
        query("INSERT INTO share_invite_keys (invite_id, share_id, invite_key, created_at) VALUES ('old-invite', 'old-share', X'040506', 'now')")
            .execute(&pool).await.expect("synthetic invite key");
        run_migrations(&pool).await.expect("retire sharing");
        for table in ["share_keys", "share_invite_keys"] {
            let count = query(&format!("SELECT COUNT(*) AS count FROM {table}"))
                .fetch_one(&pool)
                .await
                .expect("remaining key count")
                .get::<i64, _>("count");
            assert_eq!(count, 0, "retired key material remains in {table}");
        }
        assert_eq!(
            repos.get_note(note_id).await.expect("note remains").id,
            note_id
        );
        assert_eq!(
            query("SELECT COUNT(*) AS count FROM agent_items")
                .fetch_one(&pool)
                .await
                .expect("history count")
                .get::<i64, _>("count"),
            2
        );
        run_migrations(&pool).await.expect("idempotent restart");
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn artifact_display_name_survives_message_compaction() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..49])
            .await
            .expect("schema before artifact display names");
        query(
            "INSERT INTO agent_sessions (
                id, title, status, model, safety_mode, source, created_at, updated_at
             ) VALUES ('session', 'title', 'idle', 'auto', 'sandboxed', 'user', 'now', 'now')",
        )
        .execute(&pool)
        .await
        .expect("agent session");
        let path = "/workspace/attachments/1fb34e2e-e4e7-45d7-b538-a87aefa4c8e4-9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf";
        let display_name = "9bc2de72-d7b9-4f2a-ae01-cce9bacb2664-notes.pdf";
        let payload = serde_json::json!({
            "role": "user",
            "content": "Review this",
            "attachments": [{
                "id": "attachment",
                "name": display_name,
                "path": path,
                "mimeType": "application/pdf",
                "sizeBytes": 42,
                "available": true,
                "createdAt": "now"
            }]
        })
        .to_string();
        query(
            "INSERT INTO agent_items (
                id, session_id, sequence, kind, payload_json, created_at
             ) VALUES ('message', 'session', 1, 'user_message', ?, 'now')",
        )
        .bind(payload)
        .execute(&pool)
        .await
        .expect("agent message");
        query(
            "INSERT INTO agent_artifacts (
                id, session_id, item_id, provenance, action, path, original_path,
                mime_type, size_bytes, available, created_at
             ) VALUES (
                'artifact', 'session', 'message', 'attachment', 'imported', ?,
                '/tmp/companion/content', 'application/pdf', 42, 1, 'now'
             )",
        )
        .bind(path)
        .execute(&pool)
        .await
        .expect("agent artifact");

        run_migrations(&pool)
            .await
            .expect("artifact display-name migration");
        query("DELETE FROM agent_items WHERE id = 'message'")
            .execute(&pool)
            .await
            .expect("compact message");
        let artifact =
            query("SELECT item_id, display_name FROM agent_artifacts WHERE id = 'artifact'")
                .fetch_one(&pool)
                .await
                .expect("persisted artifact");

        assert_eq!(artifact.get::<Option<String>, _>("item_id"), None);
        assert_eq!(
            artifact.get::<Option<String>, _>("display_name").as_deref(),
            Some(display_name)
        );
    }

    #[tokio::test]
    async fn connector_trigger_uniqueness_migration_keeps_latest_configuration() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..29])
            .await
            .expect("pre-uniqueness schema");
        for (id, kind, config, created_at) in [
            (
                "trigger-old",
                "email_received",
                r#"{"query":"is:unread"}"#,
                "2026-07-24T08:00:00Z",
            ),
            (
                "trigger-new",
                "event_upcoming",
                r#"{"leadMinutes":30}"#,
                "2026-07-24T09:00:00Z",
            ),
        ] {
            query(
                "INSERT INTO connector_triggers
                 (id, job_id, kind, account_id, config, created_at)
                 VALUES (?, 'job-1', ?, 'user@example.com', ?, ?)",
            )
            .bind(id)
            .bind(kind)
            .bind(config)
            .bind(created_at)
            .execute(&pool)
            .await
            .expect("legacy duplicate trigger");
        }

        run_migrations(&pool).await.expect("uniqueness migration");

        let row = query(
            "SELECT id, kind, config
             FROM connector_triggers
             WHERE job_id = 'job-1'",
        )
        .fetch_one(&pool)
        .await
        .expect("preserved trigger");
        assert_eq!(row.get::<String, _>("id"), "trigger-new");
        assert_eq!(row.get::<String, _>("kind"), "event_upcoming");
        assert_eq!(row.get::<String, _>("config"), r#"{"leadMinutes":30}"#);

        let duplicate = query(
            "INSERT INTO connector_triggers
             (id, job_id, kind, account_id, config, created_at)
             VALUES ('trigger-duplicate', 'job-1', 'email_received',
                     'user@example.com', '{}', '2026-07-24T10:00:00Z')",
        )
        .execute(&pool)
        .await;
        assert!(duplicate.is_err(), "job_id uniqueness must be enforced");
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn prerelease_agent_runtime_stamp_repairs_without_replaying_runtime_migration() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("current schema");
        install_runtime_non_replay_guard(&pool).await;
        query("DROP INDEX idx_connector_triggers_job_id_unique")
            .execute(&pool)
            .await
            .expect("restore pre-uniqueness index state");
        for index in [
            "idx_audio_artifacts_note_status_created_at",
            "idx_transcripts_note_created_at",
            "idx_recording_checkpoints_session_kind_created_at",
        ] {
            query(&format!("DROP INDEX {index}"))
                .execute(&pool)
                .await
                .expect("restore pre-hydration index state");
        }
        for (id, created_at) in [
            ("trigger-old", "2026-07-24T08:00:00Z"),
            ("trigger-new", "2026-07-24T09:00:00Z"),
        ] {
            query(
                "INSERT INTO connector_triggers
                 (id, job_id, kind, account_id, config, created_at)
                 VALUES (?, 'job-1', 'email_received', 'user@example.com', '{}', ?)",
            )
            .bind(id)
            .bind(created_at)
            .execute(&pool)
            .await
            .expect("legacy duplicate trigger");
        }
        query("DELETE FROM schema_migrations WHERE version >= 31")
            .execute(&pool)
            .await
            .expect("remove corrected runtime stamp");
        query(
            "UPDATE schema_migrations
             SET name = 'agent_runtime'
             WHERE version = 30",
        )
        .execute(&pool)
        .await
        .expect("restore prerelease runtime stamp");

        run_migrations(&pool)
            .await
            .expect("repair prerelease migration stamps");

        let item_count: i64 = query("SELECT COUNT(*) AS count FROM agent_items")
            .fetch_one(&pool)
            .await
            .expect("preserved runtime messages")
            .get("count");
        assert_eq!(item_count, 2);
        let trigger_id: String = query("SELECT id FROM connector_triggers WHERE job_id = 'job-1'")
            .fetch_one(&pool)
            .await
            .expect("preserved newest trigger")
            .get("id");
        assert_eq!(trigger_id, "trigger-new");
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn prerelease_version_31_runtime_stamp_repairs_hydration_indexes() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("current schema");
        install_runtime_non_replay_guard(&pool).await;
        for index in [
            "idx_audio_artifacts_note_status_created_at",
            "idx_transcripts_note_created_at",
            "idx_recording_checkpoints_session_kind_created_at",
        ] {
            query(&format!("DROP INDEX {index}"))
                .execute(&pool)
                .await
                .expect("restore pre-hydration index state");
        }
        query("DELETE FROM schema_migrations WHERE version >= 32")
            .execute(&pool)
            .await
            .expect("remove corrected runtime stamp");
        query(
            "UPDATE schema_migrations
             SET name = 'agent_runtime'
             WHERE version = 31",
        )
        .execute(&pool)
        .await
        .expect("restore later prerelease runtime stamp");

        run_migrations(&pool)
            .await
            .expect("repair version 31 prerelease migration stamp");

        for index in [
            "idx_audio_artifacts_note_status_created_at",
            "idx_transcripts_note_created_at",
            "idx_recording_checkpoints_session_kind_created_at",
        ] {
            let present: i64 = query(
                "SELECT COUNT(*) AS count FROM sqlite_master WHERE type = 'index' AND name = ?",
            )
            .bind(index)
            .fetch_one(&pool)
            .await
            .expect("hydration index lookup")
            .get("count");
            assert_eq!(present, 1, "{index} should be restored");
        }
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn prerelease_linear_stamps_shift_after_calendar_release_without_replaying() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..44])
            .await
            .expect("schema before calendar release");
        query(
            "CREATE TABLE linear_mcp_connection (
                preset_id TEXT PRIMARY KEY NOT NULL,
                state TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease Linear schema");
        query(
            "INSERT INTO linear_mcp_connection (preset_id, state)
             VALUES ('builtin:linear', 'connected')",
        )
        .execute(&pool)
        .await
        .expect("prerelease Linear state");
        for (version, name) in [
            (45_i64, "linear_managed_mcp"),
            (46_i64, "linear_managed_mcp_repair"),
        ] {
            query(
                "INSERT INTO schema_migrations (version, name)
                 VALUES (?, ?)",
            )
            .bind(version)
            .bind(name)
            .execute(&pool)
            .await
            .expect("prerelease Linear migration stamp");
        }

        run_migrations(&pool)
            .await
            .expect("repair prerelease Linear migration stamps");
        run_migrations(&pool)
            .await
            .expect("idempotent prerelease Linear stamp repair");

        let rows = query(
            "SELECT version, name
             FROM schema_migrations
             WHERE version >= 45
             ORDER BY version",
        )
        .fetch_all(&pool)
        .await
        .expect("repaired migration stamps");
        let stamps = rows
            .iter()
            .map(|row| (row.get::<i64, _>("version"), row.get::<String, _>("name")))
            .collect::<Vec<_>>();
        assert_eq!(
            stamps,
            vec![
                (45, "calendar_event_html_link".to_string()),
                (46, "companion_files".to_string()),
                (47, "companion_computer_use_approval_audit".to_string()),
                (48, "linear_managed_mcp".to_string()),
                (49, "linear_managed_mcp_repair".to_string()),
                (50, "agent_artifact_display_names".to_string()),
                (51, "note_transcription_chunks".to_string()),
                (52, "retire_native_chat".to_string()),
                (53, "retire_sharing_keys".to_string()),
                (54, "dictation_app_bundle_id".to_string()),
                (55, "agent_session_approval_mode".to_string()),
                (56, "interrupt_unresolved_tool_calls".to_string()),
                (57, "meeting_video".to_string()),
                (58, "recording_marks".to_string()),
            ]
        );

        let calendar_column_count: i64 = query(
            "SELECT COUNT(*) AS count
             FROM pragma_table_info('notes')
             WHERE name = 'calendar_event_html_link'",
        )
        .fetch_one(&pool)
        .await
        .expect("calendar column")
        .get("count");
        assert_eq!(calendar_column_count, 1);
        let linear_state: String =
            query("SELECT state FROM linear_mcp_connection WHERE preset_id = 'builtin:linear'")
                .fetch_one(&pool)
                .await
                .expect("preserved Linear state")
                .get("state");
        assert_eq!(linear_state, "connected");
    }

    #[tokio::test]
    async fn clean_database_installs_linear_managed_mcp_compatibility_migrations() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("current schema");
        run_migrations(&pool)
            .await
            .expect("idempotent current schema");

        assert!(table_exists(&pool, "linear_mcp_connection").await);
        let rows: i64 = query("SELECT COUNT(*) AS count FROM linear_mcp_connection")
            .fetch_one(&pool)
            .await
            .expect("compatibility table")
            .get("count");
        assert_eq!(rows, 0, "migration must not invent connection authority");
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn released_version_45_database_advances_through_linear_migrations() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..45])
            .await
            .expect("released version 45 schema");

        run_migrations(&pool)
            .await
            .expect("advance through Linear migrations");

        assert!(table_exists(&pool, "linear_mcp_connection").await);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn repaired_linear_stamps_are_accepted_without_replaying_state() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..45])
            .await
            .expect("released version 45 schema");
        query(
            "CREATE TABLE linear_mcp_connection (
                preset_id TEXT PRIMARY KEY NOT NULL,
                state TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease Linear schema");
        query(
            "INSERT INTO linear_mcp_connection (preset_id, state)
             VALUES ('builtin:linear', 'connected')",
        )
        .execute(&pool)
        .await
        .expect("prerelease Linear state");
        for (version, name) in [
            (46_i64, "linear_managed_mcp"),
            (47_i64, "linear_managed_mcp_repair"),
        ] {
            query("INSERT INTO schema_migrations (version, name) VALUES (?, ?)")
                .bind(version)
                .bind(name)
                .execute(&pool)
                .await
                .expect("repaired Linear migration stamp");
        }

        run_migrations(&pool)
            .await
            .expect("accept repaired Linear stamps");

        assert!(table_exists(&pool, "companion_browse_roots").await);
        assert!(table_exists(&pool, "companion_uploads").await);
        assert!(table_exists(&pool, "companion_computer_use_approval_audit").await);
        let state: String =
            query("SELECT state FROM linear_mcp_connection WHERE preset_id = 'builtin:linear'")
                .fetch_one(&pool)
                .await
                .expect("preserved Linear state")
                .get("state");
        assert_eq!(state, "connected");
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn malformed_linear_prerelease_stamp_is_rejected_transactionally() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..44])
            .await
            .expect("schema before calendar release");
        query(
            "CREATE TABLE linear_mcp_connection (
                preset_id TEXT PRIMARY KEY NOT NULL,
                state TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease Linear schema");
        for (version, name) in [
            (45_i64, "linear_managed_mcp"),
            (46_i64, "linear_managed_mcp_wrong"),
        ] {
            query("INSERT INTO schema_migrations (version, name) VALUES (?, ?)")
                .bind(version)
                .bind(name)
                .execute(&pool)
                .await
                .expect("malformed Linear migration stamp");
        }

        assert!(run_migrations(&pool).await.is_err());
        let calendar_column_count: i64 = query(
            "SELECT COUNT(*) AS count
             FROM pragma_table_info('notes')
             WHERE name = 'calendar_event_html_link'",
        )
        .fetch_one(&pool)
        .await
        .expect("calendar column lookup")
        .get("count");
        assert_eq!(calendar_column_count, 0);
    }

    #[tokio::test]
    async fn malformed_linear_prerelease_schema_is_rejected_transactionally() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..44])
            .await
            .expect("schema before calendar release");
        query(
            "CREATE TABLE linear_mcp_connection (
                wrong_id TEXT PRIMARY KEY NOT NULL,
                state TEXT NOT NULL
             )",
        )
        .execute(&pool)
        .await
        .expect("malformed prerelease Linear schema");
        for (version, name) in [
            (45_i64, "linear_managed_mcp"),
            (46_i64, "linear_managed_mcp_repair"),
        ] {
            query("INSERT INTO schema_migrations (version, name) VALUES (?, ?)")
                .bind(version)
                .bind(name)
                .execute(&pool)
                .await
                .expect("prerelease Linear migration stamp");
        }

        assert!(run_migrations(&pool).await.is_err());
        let calendar_column_count: i64 = query(
            "SELECT COUNT(*) AS count
             FROM pragma_table_info('notes')
             WHERE name = 'calendar_event_html_link'",
        )
        .fetch_one(&pool)
        .await
        .expect("calendar column lookup")
        .get("count");
        assert_eq!(calendar_column_count, 0);
    }

    #[tokio::test]
    async fn unversioned_prerelease_runtime_adopts_intervening_migrations_without_replay() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("current schema");
        install_runtime_non_replay_guard(&pool).await;
        query("DROP INDEX idx_connector_triggers_job_id_unique")
            .execute(&pool)
            .await
            .expect("restore pre-uniqueness index state");
        for index in [
            "idx_audio_artifacts_note_status_created_at",
            "idx_transcripts_note_created_at",
            "idx_recording_checkpoints_session_kind_created_at",
        ] {
            query(&format!("DROP INDEX {index}"))
                .execute(&pool)
                .await
                .expect("restore pre-hydration index state");
        }
        query("DROP TABLE schema_migrations")
            .execute(&pool)
            .await
            .expect("remove version table");

        run_migrations(&pool)
            .await
            .expect("adopt prerelease runtime schema");

        let item_count: i64 = query("SELECT COUNT(*) AS count FROM agent_items")
            .fetch_one(&pool)
            .await
            .expect("preserved runtime messages")
            .get("count");
        assert_eq!(item_count, 2);
        assert_latest_stamp(&pool).await;
    }

    #[test]
    fn catalog_rejects_migrations_without_legacy_requirements() {
        let error = validate_catalog(EMPTY_REQUIREMENTS_MIGRATIONS)
            .expect_err("empty requirements must fail catalog validation");

        assert!(error.to_string().contains("has no schema requirements"));
    }

    #[tokio::test]
    async fn current_replay_database_is_stamped_without_replaying_sql() {
        let pool = test_pool().await;
        run_migrations(&pool).await.expect("build current schema");
        install_runtime_non_replay_guard(&pool).await;
        query("DROP TABLE schema_migrations")
            .execute(&pool)
            .await
            .expect("remove version table");

        run_migrations(&pool)
            .await
            .expect("stamp current replay database");

        let count: i64 = query("SELECT COUNT(*) AS count FROM agent_items")
            .fetch_one(&pool)
            .await
            .expect("runtime messages")
            .get("count");
        assert_eq!(count, 2);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn prerelease_routine_catalog_rows_are_preserved_and_marked_legacy() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..35])
            .await
            .expect("prerelease routine schema");
        query("ALTER TABLE routines DROP COLUMN tool_catalog_version")
            .execute(&pool)
            .await
            .expect("recreate prerelease catalog shape");
        query(
            "INSERT INTO routines (
                id, name, prompt, schedule, timezone, repeat, deliver, model,
                safety_mode, state, enabled, created_at, updated_at, metadata_json
             ) VALUES (
                'routine-prerelease', 'Daily recap', 'Recap my day', '@daily',
                'UTC', 'forever', 'local', 'auto', 'sandboxed', 'scheduled', 1,
                'now', 'now', '{}'
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease routine");

        run_migrations(&pool)
            .await
            .expect("upgrade prerelease routine schema");
        run_migrations(&pool)
            .await
            .expect("idempotent routine catalog upgrade");

        let row = query(
            "SELECT name, tool_catalog_version
             FROM routines
             WHERE id = 'routine-prerelease'",
        )
        .fetch_one(&pool)
        .await
        .expect("preserved routine");
        assert_eq!(row.get::<String, _>("name"), "Daily recap");
        assert_eq!(row.get::<i64, _>("tool_catalog_version"), 0);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn prerelease_agent_runs_get_an_immutable_mcp_snapshot_marker() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..36])
            .await
            .expect("prerelease MCP policy schema");
        query(
            "INSERT INTO agent_sessions (
                id, title, status, model, safety_mode, source, created_at, updated_at
             ) VALUES (
                'session-prerelease', 'Existing session', 'idle', 'auto',
                'sandboxed', 'user', 'now', 'now'
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease session");
        query(
            "INSERT INTO agent_runs (
                id, session_id, status, model, started_at, updated_at
             ) VALUES (
                'run-prerelease', 'session-prerelease', 'interrupted', 'auto',
                'now', 'now'
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease run");
        query("ALTER TABLE agent_runs DROP COLUMN mcp_policy_snapshotted")
            .execute(&pool)
            .await
            .expect("recreate prerelease run shape");

        run_migrations(&pool)
            .await
            .expect("upgrade prerelease MCP snapshot schema");
        run_migrations(&pool)
            .await
            .expect("idempotent MCP snapshot upgrade");

        let snapshotted: i64 = query(
            "SELECT mcp_policy_snapshotted
             FROM agent_runs
             WHERE id = 'run-prerelease'",
        )
        .fetch_one(&pool)
        .await
        .expect("preserved run")
        .get("mcp_policy_snapshotted");
        assert_eq!(snapshotted, 1);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn interrupted_prerelease_runs_gain_nullable_durable_run_configuration() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..39])
            .await
            .expect("migration 39 schema");
        query(
            "INSERT INTO agent_sessions (
                id, title, status, model, safety_mode, source, created_at, updated_at
             ) VALUES (
                'session-config', 'Waiting session', 'waiting_for_user', 'auto',
                'sandboxed', 'routine', 'now', 'now'
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease session");
        query(
            "INSERT INTO agent_runs (
                id, session_id, status, model, started_at, updated_at
             ) VALUES (
                'run-config', 'session-config', 'waiting_for_user', 'auto',
                'now', 'now'
             )",
        )
        .execute(&pool)
        .await
        .expect("prerelease run");

        run_migrations(&pool)
            .await
            .expect("add durable run configuration");
        run_migrations(&pool)
            .await
            .expect("idempotent durable run configuration migration");

        let row = query("SELECT status, run_config_json FROM agent_runs WHERE id = 'run-config'")
            .fetch_one(&pool)
            .await
            .expect("preserved run");
        assert_eq!(row.get::<String, _>("status"), "cancelled");
        assert_eq!(row.get::<Option<String>, _>("run_config_json"), None);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn unversioned_migration_15_database_upgrades_to_latest() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &MIGRATIONS[..15])
            .await
            .expect("migration 15 schema");
        install_non_replay_guard(&pool).await;
        query("DROP TABLE schema_migrations")
            .execute(&pool)
            .await
            .expect("remove version table");

        run_migrations(&pool)
            .await
            .expect("upgrade migration 15 database");

        assert!(table_exists(&pool, "connector_accounts").await);
        assert!(table_exists(&pool, "note_transcription_jobs").await);
        assert!(table_exists(&pool, "browser_action_outcomes").await);
        assert_latest_stamp(&pool).await;
    }

    #[tokio::test]
    async fn every_unversioned_historical_prefix_upgrades_to_latest() {
        for version in 1..MIGRATIONS.len() {
            let pool = test_pool().await;
            run_migration_catalog(&pool, &MIGRATIONS[..version])
                .await
                .unwrap_or_else(|error| panic!("build migration {version} schema: {error}"));
            query("DROP TABLE schema_migrations")
                .execute(&pool)
                .await
                .expect("remove version table");

            run_migrations(&pool)
                .await
                .unwrap_or_else(|error| panic!("upgrade migration {version} schema: {error}"));

            assert_latest_stamp(&pool).await;
        }
    }

    #[tokio::test]
    async fn failed_pending_migration_rolls_back_schema_and_version_stamp() {
        let pool = test_pool().await;
        run_migration_catalog(&pool, &FAILING_MIGRATIONS[..1])
            .await
            .expect("existing migration");

        run_migration_catalog(&pool, FAILING_MIGRATIONS)
            .await
            .expect_err("third migration must fail");

        let row = query(
            "SELECT COUNT(*) AS count, MAX(version) AS version
             FROM schema_migrations",
        )
        .fetch_one(&pool)
        .await
        .expect("preserved migration stamp");
        assert_eq!(row.get::<i64, _>("count"), 1);
        assert_eq!(row.get::<i64, _>("version"), 1);
        assert!(table_exists(&pool, "transaction_existing").await);
        assert!(!table_exists(&pool, "transaction_pending_success").await);
        assert!(!table_exists(&pool, "transaction_failure").await);
    }

    fn stamped(migrations: &[Migration]) -> Vec<AppliedMigration> {
        migrations
            .iter()
            .map(|migration| AppliedMigration {
                version: migration.version,
                name: migration.name.to_string(),
            })
            .collect()
    }

    #[test]
    fn newer_schema_is_refused_without_tolerance() {
        let mut applied = stamped(FAILING_MIGRATIONS);
        applied.push(AppliedMigration {
            version: 4,
            name: "from_an_unmerged_branch".to_string(),
        });

        let error = validate_applied_migrations_with_tolerance(&applied, FAILING_MIGRATIONS, false)
            .expect_err("newer stamp must refuse");
        assert!(error.to_string().contains("newer than this Clovy build"));
    }

    #[test]
    fn newer_schema_is_tolerated_as_fully_migrated_in_dev() {
        let mut applied = stamped(FAILING_MIGRATIONS);
        applied.push(AppliedMigration {
            version: 4,
            name: "from_an_unmerged_branch".to_string(),
        });

        let current =
            validate_applied_migrations_with_tolerance(&applied, FAILING_MIGRATIONS, true)
                .expect("tolerated newer stamp");
        // The build treats the database as fully migrated for its own catalog,
        // so migrate applies nothing and the extra stamp is left untouched.
        assert_eq!(current, FAILING_MIGRATIONS.len());
    }

    #[test]
    fn tolerance_still_refuses_a_diverged_known_prefix() {
        let mut applied = stamped(FAILING_MIGRATIONS);
        applied[1].name = "renamed_migration".to_string();
        applied.push(AppliedMigration {
            version: 4,
            name: "from_an_unmerged_branch".to_string(),
        });

        let error = validate_applied_migrations_with_tolerance(&applied, FAILING_MIGRATIONS, true)
            .expect_err("diverged prefix must still refuse");
        assert!(error.to_string().contains("diverges at version 2"));
    }
}
