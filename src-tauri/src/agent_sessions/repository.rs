use super::domain::{
    AgentApprovalMode, AgentArtifactDto, AgentItemDto, AgentItemPayload, AgentRunDto,
    AgentSafetyMode, AgentSessionDto, AgentSkillDto,
};
use super::skill_identity::{
    canonical_skill_id, canonical_skill_ids, write_skill_ids, CLOVY_OBSIDIAN_SKILL_ID,
    LEGACY_OBSIDIAN_SKILL_ID,
};
use crate::domain::types::AppError;
use sqlx::{query::query, row::Row};
use sqlx_sqlite::{SqlitePool, SqliteRow};
use std::collections::BTreeSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct AgentRepository {
    pub(crate) pool: SqlitePool,
}

pub(crate) enum ContextSummaryReplacement {
    Applied(Box<AgentItemDto>),
    Noop,
}

impl AgentRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create_session(
        &self,
        title: &str,
        model: &str,
        safety_mode: AgentSafetyMode,
        workspace_path: Option<&str>,
    ) -> Result<AgentSessionDto, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let now = crate::db::repositories::timestamp();
        query(
            "INSERT INTO agent_sessions
             (id, title, status, model, safety_mode, workspace_path, source, created_at, updated_at)
             VALUES (?, ?, 'idle', ?, ?, ?, 'user', ?, ?)",
        )
        .bind(&id)
        .bind(title.trim())
        .bind(model)
        .bind(safety_mode.as_db())
        .bind(workspace_path)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.get_session(&id).await
    }

    pub async fn create_session_in_profile(
        &self,
        title: &str,
        model: &str,
        safety_mode: AgentSafetyMode,
        workspace_path: Option<&str>,
        profile: &str,
    ) -> Result<AgentSessionDto, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let now = crate::db::repositories::timestamp();
        let profile = profile.trim();
        let profile = if profile.is_empty() {
            "default"
        } else {
            profile
        };
        let mut transaction = self.pool.begin().await?;
        query(
            "INSERT INTO agent_sessions
             (id, title, status, model, safety_mode, workspace_path, source, created_at, updated_at)
             VALUES (?, ?, 'idle', ?, ?, ?, 'user', ?, ?)",
        )
        .bind(&id)
        .bind(title.trim())
        .bind(model)
        .bind(safety_mode.as_db())
        .bind(workspace_path)
        .bind(&now)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        query(
            "INSERT INTO session_profiles (session_id, profile, assigned_at)
             VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(profile)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        self.get_session(&id).await
    }

    pub async fn get_session(&self, id: &str) -> Result<AgentSessionDto, sqlx::Error> {
        query(
            "SELECT id, title, status, model, safety_mode, workspace_path, source,
                    created_at, updated_at, completed_at, last_error
             FROM agent_sessions WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map(session_from_row)
    }

    pub async fn list_sessions(&self) -> Result<Vec<AgentSessionDto>, sqlx::Error> {
        query(
            "SELECT id, title, status, model, safety_mode, workspace_path, source,
                    created_at, updated_at, completed_at, last_error
             FROM agent_sessions ORDER BY updated_at DESC, id ASC",
        )
        .fetch_all(&self.pool)
        .await
        .map(|rows| rows.into_iter().map(session_from_row).collect())
    }

    pub async fn rename_session(
        &self,
        id: &str,
        title: &str,
    ) -> Result<AgentSessionDto, sqlx::Error> {
        query("UPDATE agent_sessions SET title = ?, updated_at = ? WHERE id = ?")
            .bind(title.trim())
            .bind(crate::db::repositories::timestamp())
            .bind(id)
            .execute(&self.pool)
            .await?;
        self.get_session(id).await
    }

    pub async fn delete_session(&self, id: &str) -> Result<(), AppError> {
        self.delete_session_for_turn(id, None).await
    }

    pub(crate) async fn delete_session_for_turn(
        &self,
        id: &str,
        turn: Option<(
            &str,
            &str,
            &str,
            &crate::domain::turn_authority::TurnAuthority,
        )>,
    ) -> Result<(), AppError> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        Self::delete_session_in_transaction(&mut tx, id, turn).await?;
        tx.commit().await?;
        Ok(())
    }

    /// UI and MCP deletion share one write reservation with run admission.
    /// The optional partition and caller identity come only from the host.
    pub(crate) async fn delete_session_in_transaction(
        tx: &mut sqlx::transaction::Transaction<'_, sqlx_sqlite::Sqlite>,
        id: &str,
        turn: Option<(
            &str,
            &str,
            &str,
            &crate::domain::turn_authority::TurnAuthority,
        )>,
    ) -> Result<(), AppError> {
        if let Some((profile, session_id, run_id, authority)) = turn {
            if id == session_id {
                return Err(AppError::new(
                    "conversation_self_delete",
                    "A turn cannot delete its own conversation.",
                ));
            }
            crate::db::repositories::guard_bound_turn(tx, profile, session_id, run_id, authority)
                .await?;
        }
        let profile = turn.map(|(profile, _, _, _)| profile);
        // Acquire the writer even when the supplied transaction was deferred.
        // A concurrent run INSERT is wholly before or after this admission.
        let claimed = query("UPDATE agent_sessions SET id=id WHERE id=? AND (? IS NULL OR COALESCE((SELECT profile FROM session_profiles WHERE session_id=agent_sessions.id),'default')=?)")
            .bind(id).bind(profile).bind(profile).execute(&mut **tx).await?.rows_affected();
        if claimed == 0 {
            return Err(AppError::new(
                "conversation_not_found",
                "Conversation was not found.",
            ));
        }
        let active: i64 = sqlx::query_scalar::query_scalar("SELECT COUNT(*) FROM agent_runs WHERE session_id=? AND status IN ('queued','running','waiting_for_user')")
            .bind(id).fetch_one(&mut **tx).await?;
        if active != 0 {
            return Err(AppError::new(
                "agent_run_active",
                "Stop or resolve the current run before deleting this conversation.",
            ));
        }
        query("DELETE FROM completed_sessions WHERE session_id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        query("DELETE FROM session_profiles WHERE session_id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        query("DELETE FROM agent_sessions WHERE id=?")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        Ok(())
    }

    pub async fn latest_run(&self, session_id: &str) -> Result<AgentRunDto, sqlx::Error> {
        query(
            "SELECT id, session_id, status, model, reasoning_effort, started_at, updated_at, completed_at,
                      usage_json, interrupted_state_json, last_sequence, error_code, error_message, json_extract(run_config_json, '$.approvalMode') AS approval_mode
               FROM agent_runs WHERE session_id = ? ORDER BY started_at DESC LIMIT 1",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await
        .map(run_from_row)
    }

    pub async fn create_run(
        &self,
        session_id: &str,
        model: &str,
        reasoning_effort: Option<&str>,
    ) -> Result<AgentRunDto, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let now = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        query(
            "INSERT INTO agent_runs
             (id, session_id, status, model, reasoning_effort, started_at, updated_at, run_config_json)
             SELECT ?, s.id, 'running', ?, ?, ?, ?,
                    json_object('approvalMode', CASE WHEN s.safety_mode = 'unrestricted' THEN 'bypass' ELSE 'approvals' END,
                                'dataPartition', COALESCE(p.profile, 'default'))
             FROM agent_sessions s LEFT JOIN session_profiles p ON p.session_id = s.id
             WHERE s.id = ?",
        )
        .bind(&id)
        .bind(model)
        .bind(reasoning_effort)
        .bind(&now)
        .bind(&now)
        .bind(session_id)
        .execute(&mut *transaction)
        .await?;
        query("UPDATE agent_sessions SET status = 'running', updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        self.get_run(&id).await
    }

    pub async fn get_run(&self, id: &str) -> Result<AgentRunDto, sqlx::Error> {
        let row = query(
            "SELECT id, session_id, status, model, reasoning_effort, started_at, updated_at, completed_at,
                    usage_json, interrupted_state_json, last_sequence, error_code, error_message, json_extract(run_config_json, '$.approvalMode') AS approval_mode
             FROM agent_runs WHERE id = ?",
        )
        .bind(id)
        .fetch_one(&self.pool)
        .await?;
        Ok(run_from_row(row))
    }

    pub async fn pending_interruption_ids(&self, run_id: &str) -> Result<Vec<String>, sqlx::Error> {
        let rows = query(
            "SELECT json_extract(payload_json, '$.id') AS interruption_id
             FROM agent_items
             WHERE run_id = ?
               AND kind = 'interruption'
               AND json_extract(payload_json, '$.status') = 'pending'
             ORDER BY interruption_id",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.get::<Option<String>, _>("interruption_id"))
            .filter(|id| !id.is_empty())
            .collect())
    }

    pub async fn reset_run_sequence_for_resume(&self, run_id: &str) -> Result<(), sqlx::Error> {
        query("UPDATE agent_runs SET last_sequence = 0, updated_at = ? WHERE id = ?")
            .bind(crate::db::repositories::timestamp())
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_run_enabled_skills(
        &self,
        run_id: &str,
        skill_ids: &[String],
    ) -> Result<(), sqlx::Error> {
        let compatible = canonical_skill_ids(skill_ids.iter().map(String::as_str));
        query("UPDATE agent_runs SET enabled_skills_json = ? WHERE id = ?")
            .bind(
                serde_json::to_string(&compatible)
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
            )
            .bind(run_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn run_enabled_skills(&self, run_id: &str) -> Result<Vec<String>, sqlx::Error> {
        let row = query("SELECT enabled_skills_json FROM agent_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await?;
        let stored: Vec<String> =
            serde_json::from_str(&row.get::<String, _>("enabled_skills_json"))
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
        let compatible = canonical_skill_ids(stored.iter().map(String::as_str));
        let stored_set = stored.iter().cloned().collect::<BTreeSet<_>>();
        if compatible != stored_set {
            query("UPDATE agent_runs SET enabled_skills_json = ? WHERE id = ?")
                .bind(
                    serde_json::to_string(&compatible)
                        .map_err(|error| sqlx::Error::Decode(Box::new(error)))?,
                )
                .bind(run_id)
                .execute(&self.pool)
                .await?;
        }
        Ok(canonical_skill_ids(stored.iter().map(String::as_str))
            .into_iter()
            .collect())
    }

    pub async fn set_run_config(
        &self,
        run_id: &str,
        config: &serde_json::Value,
    ) -> Result<(), sqlx::Error> {
        query(
            "UPDATE agent_runs SET run_config_json = json_patch(json_remove(?, '$.approvalMode', '$.dataPartition'), COALESCE(run_config_json, '{}')), updated_at = ?
             WHERE id = ?",
        )
        .bind(config.to_string())
        .bind(crate::db::repositories::timestamp())
        .bind(run_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Retain retry ancestry even if CLI discovery or spawning fails before
    /// a conversation id exists. The parent is assigned once per run.
    pub(crate) async fn set_run_retry_parent(
        &self,
        run_id: &str,
        previous_run_id: &str,
    ) -> Result<(), sqlx::Error> {
        query("UPDATE agent_runs SET run_config_json = json_set(COALESCE(run_config_json, '{}'), '$.retryOfRunId', ?), updated_at = ? WHERE id = ? AND json_extract(run_config_json, '$.retryOfRunId') IS NULL")
            .bind(previous_run_id).bind(crate::db::repositories::timestamp()).bind(run_id)
            .execute(&self.pool).await?;
        Ok(())
    }

    /// Save a CLI conversation once while retaining retry ancestry and any
    /// other run metadata recorded before the process was launched.
    pub(crate) async fn set_cli_conversation(
        &self,
        run_id: &str,
        cli: &str,
        conversation_id: &str,
        profile_id: Option<&str>,
    ) -> Result<(), sqlx::Error> {
        let origin = profile_id
            .map(|id| serde_json::json!({"cliProfileId":id}))
            .unwrap_or_else(|| serde_json::json!({}));
        query("UPDATE agent_runs SET run_config_json = json_patch(json_set(COALESCE(run_config_json, '{}'), '$.engine', 'cli', '$.cli', ?, '$.conversationId', ?), json(?)), updated_at = ? WHERE id = ? AND json_extract(run_config_json, '$.conversationId') IS NULL")
            .bind(cli).bind(conversation_id).bind(origin.to_string()).bind(crate::db::repositories::timestamp()).bind(run_id)
            .execute(&self.pool).await?;
        Ok(())
    }

    pub async fn run_config(&self, run_id: &str) -> Result<Option<serde_json::Value>, sqlx::Error> {
        let value = query("SELECT run_config_json FROM agent_runs WHERE id = ?")
            .bind(run_id)
            .fetch_one(&self.pool)
            .await?
            .get::<Option<String>, _>("run_config_json");
        value
            .map(|value| {
                serde_json::from_str(&value).map_err(|error| sqlx::Error::Decode(Box::new(error)))
            })
            .transpose()
    }

    /// Coalesces streamed reasoning into one durable row while preserving the
    /// runtime's monotonic sequence guard.
    pub async fn append_reasoning_delta(
        &self,
        session_id: &str,
        run_id: &str,
        sequence: i64,
        delta: &str,
        external_id: &str,
    ) -> Result<Option<AgentItemDto>, sqlx::Error> {
        let now = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        let updated = query(
            "UPDATE agent_runs SET last_sequence = ?, updated_at = ?
             WHERE id = ? AND last_sequence < ?",
        )
        .bind(sequence)
        .bind(&now)
        .bind(run_id)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }

        if let Some(row) = query(
            "SELECT id, sequence, payload_json, created_at
             FROM agent_items WHERE external_id = ?",
        )
        .bind(external_id)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let id: String = row.get("id");
            let display_sequence: i64 = row.get("sequence");
            let created_at: String = row.get("created_at");
            let mut payload: super::domain::TextPayload =
                serde_json::from_str(&row.get::<String, _>("payload_json"))
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
            payload.text.push_str(delta);
            query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
                .bind(
                    serde_json::to_string(&payload)
                        .map_err(|error| sqlx::Error::Encode(Box::new(error)))?,
                )
                .bind(&id)
                .execute(&mut *transaction)
                .await?;
            query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(session_id)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Ok(Some(AgentItemDto {
                id,
                session_id: session_id.to_string(),
                run_id: Some(run_id.to_string()),
                sequence: display_sequence,
                payload: AgentItemPayload::Reasoning(payload),
                external_id: Some(external_id.to_string()),
                created_at,
            }));
        }

        let id = Uuid::new_v4().to_string();
        let display_sequence: i64 = query(
            "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence
             FROM agent_items WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(&mut *transaction)
        .await?
        .get("next_sequence");
        let payload = super::domain::TextPayload {
            text: delta.to_string(),
            metadata: None,
        };
        query(
            "INSERT INTO agent_items
             (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
             VALUES (?, ?, ?, ?, 'reasoning', ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(run_id)
        .bind(display_sequence)
        .bind(
            serde_json::to_string(&payload)
                .map_err(|error| sqlx::Error::Encode(Box::new(error)))?,
        )
        .bind(external_id)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(AgentItemDto {
            id,
            session_id: session_id.to_string(),
            run_id: Some(run_id.to_string()),
            sequence: display_sequence,
            payload: AgentItemPayload::Reasoning(payload),
            external_id: Some(external_id.to_string()),
            created_at: now,
        }))
    }

    /// Coalesces streamed assistant output into one durable row. Persisting
    /// partial output lets another window hydrate the full response-so-far
    /// instead of only receiving deltas emitted after it subscribed.
    pub async fn append_assistant_message_delta(
        &self,
        session_id: &str,
        run_id: &str,
        sequence: i64,
        delta: &str,
        external_id: &str,
    ) -> Result<Option<AgentItemDto>, sqlx::Error> {
        let now = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        let updated = query(
            "UPDATE agent_runs SET last_sequence = ?, updated_at = ?
             WHERE id = ? AND last_sequence < ?",
        )
        .bind(sequence)
        .bind(&now)
        .bind(run_id)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }

        if let Some(row) = query(
            "SELECT id, sequence, payload_json, created_at
             FROM agent_items
             WHERE external_id = ? AND session_id = ? AND run_id = ?
               AND kind = 'assistant_message'",
        )
        .bind(external_id)
        .bind(session_id)
        .bind(run_id)
        .fetch_optional(&mut *transaction)
        .await?
        {
            let id: String = row.get("id");
            let display_sequence: i64 = row.get("sequence");
            let created_at: String = row.get("created_at");
            let mut payload: super::domain::MessagePayload =
                serde_json::from_str(&row.get::<String, _>("payload_json"))
                    .map_err(|error| sqlx::Error::Decode(Box::new(error)))?;
            payload.content.push_str(delta);
            query("UPDATE agent_items SET payload_json = ? WHERE id = ?")
                .bind(
                    serde_json::to_string(&payload)
                        .map_err(|error| sqlx::Error::Encode(Box::new(error)))?,
                )
                .bind(&id)
                .execute(&mut *transaction)
                .await?;
            query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
                .bind(&now)
                .bind(session_id)
                .execute(&mut *transaction)
                .await?;
            transaction.commit().await?;
            return Ok(Some(AgentItemDto {
                id,
                session_id: session_id.to_string(),
                run_id: Some(run_id.to_string()),
                sequence: display_sequence,
                payload: AgentItemPayload::AssistantMessage(payload),
                external_id: Some(external_id.to_string()),
                created_at,
            }));
        }

        let id = Uuid::new_v4().to_string();
        let display_sequence: i64 = query(
            "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence
             FROM agent_items WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(&mut *transaction)
        .await?
        .get("next_sequence");
        let payload = super::domain::MessagePayload {
            role: "assistant".into(),
            content: delta.to_string(),
            attachments: Vec::new(),
        };
        query(
            "INSERT INTO agent_items
             (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
             VALUES (?, ?, ?, ?, 'assistant_message', ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(run_id)
        .bind(display_sequence)
        .bind(
            serde_json::to_string(&payload)
                .map_err(|error| sqlx::Error::Encode(Box::new(error)))?,
        )
        .bind(external_id)
        .bind(&now)
        .execute(&mut *transaction)
        .await?;
        query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(AgentItemDto {
            id,
            session_id: session_id.to_string(),
            run_id: Some(run_id.to_string()),
            sequence: display_sequence,
            payload: AgentItemPayload::AssistantMessage(payload),
            external_id: Some(external_id.to_string()),
            created_at: now,
        }))
    }

    /// Replaces the coalesced response-so-far with the SDK's authoritative
    /// completed text without creating a duplicate assistant message.
    pub async fn complete_assistant_message(
        &self,
        session_id: &str,
        run_id: &str,
        sequence: i64,
        text: &str,
        external_id: &str,
    ) -> Result<Option<AgentItemDto>, sqlx::Error> {
        let now = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        let updated = query(
            "UPDATE agent_runs SET last_sequence = ?, updated_at = ?
             WHERE id = ? AND last_sequence < ?",
        )
        .bind(sequence)
        .bind(&now)
        .bind(run_id)
        .bind(sequence)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }

        let payload = super::domain::MessagePayload {
            role: "assistant".into(),
            content: text.to_string(),
            attachments: Vec::new(),
        };
        let payload_json = serde_json::to_string(&payload)
            .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;
        let item = if let Some(row) =
            query("SELECT id FROM agent_items WHERE external_id = ? AND session_id = ? AND run_id = ? AND kind = 'assistant_message'")
                .bind(external_id)
                .bind(session_id)
                .bind(run_id)
                .fetch_optional(&mut *transaction)
                .await?
        {
            let id: String = row.get("id");
            let display_sequence: i64 = query(
                "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence
                 FROM agent_items WHERE session_id = ?",
            )
            .bind(session_id)
            .fetch_one(&mut *transaction)
            .await?
            .get("next_sequence");
            query("UPDATE agent_items SET sequence = ?, payload_json = ?, external_id = NULL, created_at = ? WHERE id = ?")
                .bind(display_sequence)
                .bind(&payload_json)
                .bind(&now)
                .bind(&id)
                .execute(&mut *transaction)
                .await?;
            AgentItemDto {
                id,
                session_id: session_id.to_string(),
                run_id: Some(run_id.to_string()),
                sequence: display_sequence,
                payload: AgentItemPayload::AssistantMessage(payload),
                external_id: None,
                created_at: now.clone(),
            }
        } else {
            let id = Uuid::new_v4().to_string();
            let display_sequence: i64 = query(
                "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence
                 FROM agent_items WHERE session_id = ?",
            )
            .bind(session_id)
            .fetch_one(&mut *transaction)
            .await?
            .get("next_sequence");
            query(
                "INSERT INTO agent_items
                 (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
                 VALUES (?, ?, ?, ?, 'assistant_message', ?, NULL, ?)",
            )
            .bind(&id)
            .bind(session_id)
            .bind(run_id)
            .bind(display_sequence)
            .bind(&payload_json)
            .bind(&now)
            .execute(&mut *transaction)
            .await?;
            AgentItemDto {
                id,
                session_id: session_id.to_string(),
                run_id: Some(run_id.to_string()),
                sequence: display_sequence,
                payload: AgentItemPayload::AssistantMessage(payload),
                external_id: None,
                created_at: now.clone(),
            }
        };
        query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(item))
    }

    /// Persists one runtime event. Duplicate or out-of-order sequence numbers
    /// are ignored so reconnect/replay cannot duplicate transcript items.
    pub async fn append_item(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        sequence: i64,
        payload: &AgentItemPayload,
        external_id: Option<&str>,
    ) -> Result<Option<AgentItemDto>, sqlx::Error> {
        let id = Uuid::new_v4().to_string();
        let now = crate::db::repositories::timestamp();
        let payload_json = payload
            .value()
            .map_err(|error| sqlx::Error::Decode(Box::new(error)))?
            .to_string();
        let mut transaction = self.pool.begin().await?;
        if let Some(run_id) = run_id {
            let running_tool_call = matches!(payload, AgentItemPayload::ToolCall(tool) if tool.status.as_deref() == Some("running"));
            let updated = query(
                "UPDATE agent_runs SET last_sequence = ?, updated_at = ?
                 WHERE id = ? AND session_id = ? AND last_sequence < ?
                   AND (? = 0 OR status NOT IN ('completed','cancelled','failed','interrupted'))",
            )
            .bind(sequence)
            .bind(&now)
            .bind(run_id)
            .bind(session_id)
            .bind(sequence)
            .bind(running_tool_call)
            .execute(&mut *transaction)
            .await?
            .rows_affected();
            if updated == 0 {
                transaction.rollback().await?;
                return Ok(None);
            }
        }
        let display_sequence: i64 = query(
            "SELECT COALESCE(MAX(sequence), -1) + 1 AS next_sequence FROM agent_items WHERE session_id = ?",
        )
        .bind(session_id)
        .fetch_one(&mut *transaction)
        .await?
        .get("next_sequence");
        let inserted = query(
            "INSERT OR IGNORE INTO agent_items
             (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(run_id)
        .bind(display_sequence)
        .bind(payload.kind())
        .bind(payload_json)
        .bind(external_id)
        .bind(&now)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if inserted == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }
        query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
            .bind(&now)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(Some(AgentItemDto {
            id,
            session_id: session_id.to_string(),
            run_id: run_id.map(ToString::to_string),
            sequence: display_sequence,
            payload: payload.clone(),
            external_id: external_id.map(ToString::to_string),
            created_at: now,
        }))
    }

    pub async fn items(&self, session_id: &str) -> Result<Vec<AgentItemDto>, sqlx::Error> {
        let rows = query(
            "SELECT id, session_id, run_id, sequence, kind, payload_json, external_id, created_at
             FROM agent_items WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(item_from_row).collect()
    }

    /// Atomically replaces compacted transcript items with one visible context
    /// summary at the earliest removed position. A replay is a no-op because
    /// the source item ids have already been removed.
    pub async fn replace_items_with_context_summary(
        &self,
        session_id: &str,
        run_id: &str,
        summary_text: &str,
        summary_metadata: Option<&serde_json::Value>,
        removed_item_ids: &[String],
    ) -> Result<Option<AgentItemDto>, sqlx::Error> {
        match self
            .replace_items_with_context_summary_inner(
                session_id,
                run_id,
                summary_text,
                summary_metadata,
                removed_item_ids,
            )
            .await?
        {
            ContextSummaryReplacement::Applied(item) => Ok(Some(*item)),
            ContextSummaryReplacement::Noop => Ok(None),
        }
    }

    async fn replace_items_with_context_summary_inner(
        &self,
        session_id: &str,
        run_id: &str,
        summary_text: &str,
        summary_metadata: Option<&serde_json::Value>,
        removed_item_ids: &[String],
    ) -> Result<ContextSummaryReplacement, sqlx::Error> {
        if removed_item_ids.is_empty() {
            return Ok(ContextSummaryReplacement::Noop);
        }
        let mut transaction = self.pool.begin().await?;
        let mut earliest_sequence: Option<i64> = None;
        for item_id in removed_item_ids {
            let row = query("SELECT sequence FROM agent_items WHERE session_id = ? AND id = ?")
                .bind(session_id)
                .bind(item_id)
                .fetch_optional(&mut *transaction)
                .await?;
            if let Some(row) = row {
                let sequence: i64 = row.get("sequence");
                earliest_sequence =
                    Some(earliest_sequence.map_or(sequence, |current| current.min(sequence)));
            }
        }
        let Some(sequence) = earliest_sequence else {
            transaction.rollback().await?;
            return Ok(ContextSummaryReplacement::Noop);
        };
        for item_id in removed_item_ids {
            query("DELETE FROM agent_items WHERE session_id = ? AND id = ?")
                .bind(session_id)
                .bind(item_id)
                .execute(&mut *transaction)
                .await?;
        }
        let id = Uuid::new_v4().to_string();
        let created_at = crate::db::repositories::timestamp();
        let payload = AgentItemPayload::ContextSummary(super::domain::TextPayload {
            text: summary_text.to_string(),
            metadata: summary_metadata.cloned(),
        });
        query(
            "INSERT INTO agent_items
             (id, session_id, run_id, sequence, kind, payload_json, external_id, created_at)
             VALUES (?, ?, ?, ?, 'context_summary', ?, ?, ?)",
        )
        .bind(&id)
        .bind(session_id)
        .bind(run_id)
        .bind(sequence)
        .bind(
            payload
                .value()
                .map_err(|error| sqlx::Error::Decode(Box::new(error)))?
                .to_string(),
        )
        .bind(format!("context-summary:{run_id}"))
        .bind(&created_at)
        .execute(&mut *transaction)
        .await?;
        query("UPDATE agent_sessions SET updated_at = ? WHERE id = ?")
            .bind(&created_at)
            .bind(session_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(ContextSummaryReplacement::Applied(Box::new(AgentItemDto {
            id,
            session_id: session_id.to_string(),
            run_id: Some(run_id.to_string()),
            sequence,
            payload,
            external_id: Some(format!("context-summary:{run_id}")),
            created_at,
        })))
    }

    pub async fn update_run_status(
        &self,
        run_id: &str,
        status: &str,
        usage: Option<&serde_json::Value>,
        interrupted_state: Option<&serde_json::Value>,
        error: Option<(&str, &str)>,
    ) -> Result<AgentRunDto, sqlx::Error> {
        let now = crate::db::repositories::timestamp();
        let terminal = matches!(status, "completed" | "cancelled" | "failed" | "interrupted");
        let run = self.get_run(run_id).await?;
        let updated = query("UPDATE agent_runs SET status = ?, updated_at = ?, completed_at = ?, usage_json = COALESCE(?, usage_json), interrupted_state_json = COALESCE(?, interrupted_state_json), error_code = ?, error_message = ? WHERE id = ? AND status NOT IN ('completed', 'cancelled', 'failed', 'interrupted')")
            .bind(status).bind(&now).bind(terminal.then_some(now.as_str()))
            .bind(usage.map(serde_json::Value::to_string))
            .bind(interrupted_state.map(serde_json::Value::to_string))
            .bind(error.map(|v| v.0)).bind(error.map(|v| v.1)).bind(run_id)
            .execute(&self.pool).await?.rows_affected();
        if updated == 0 {
            return self.get_run(run_id).await;
        }
        let session_status = match status {
            "waiting_for_user" => "waiting_for_user",
            "failed" => "failed",
            "interrupted" => "interrupted",
            "completed" | "cancelled" => "idle",
            _ => "running",
        };
        query("UPDATE agent_sessions SET status = ?, updated_at = ?, last_error = ? WHERE id = ?")
            .bind(session_status)
            .bind(&now)
            .bind(error.map(|v| v.1))
            .bind(&run.session_id)
            .execute(&self.pool)
            .await?;
        self.get_run(run_id).await
    }

    /// Cancel a run that is durably paused for user input. Paused runs are no
    /// awaiting input rather than running, so they cannot use the ordinary
    /// `run.cancel` path. Retire their pending cards in the same transaction
    /// that releases the session for another turn.
    pub async fn cancel_waiting_run(&self, run_id: &str) -> Result<Option<AgentRunDto>, AppError> {
        let run = self.get_run(run_id).await?;
        if run.status != "waiting_for_user" {
            return Ok(None);
        }
        let timestamp = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        let retired = query(
            "UPDATE agent_items
             SET payload_json = json_set(payload_json, '$.status', 'expired', '$.resolvedAt', ?)
             WHERE run_id = ?
               AND kind = 'interruption'
               AND json_extract(payload_json, '$.status') = 'pending'",
        )
        .bind(&timestamp)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if retired == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }
        let updated = query(
            "UPDATE agent_runs
             SET status = 'cancelled', updated_at = ?, completed_at = ?, error_code = NULL, error_message = NULL
             WHERE id = ? AND status = 'waiting_for_user'",
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(run_id)
        .execute(&mut *transaction)
        .await?
        .rows_affected();
        if updated == 0 {
            transaction.rollback().await?;
            return Ok(None);
        }
        query(
            "UPDATE agent_sessions
             SET status = 'idle', updated_at = ?, last_error = NULL
             WHERE id = ?
               AND NOT EXISTS (
                   SELECT 1 FROM agent_runs
                   WHERE session_id = ?
                     AND status IN ('queued', 'running', 'waiting_for_user')
               )",
        )
        .bind(&timestamp)
        .bind(&run.session_id)
        .bind(&run.session_id)
        .execute(&mut *transaction)
        .await?;
        crate::routines::project_agent_run_terminal(&mut transaction, run_id, &timestamp).await?;
        transaction.commit().await?;
        self.get_run(run_id).await.map(Some).map_err(Into::into)
    }

    /// Repairs non-routine work left active by a previous app process. Waiting
    /// runs keep their serialized interruption state and routine runs are
    /// reconciled by the scheduler's lease-aware recovery path.
    pub async fn reconcile_non_routine_runs_after_restart(&self) -> Result<u64, sqlx::Error> {
        let timestamp = crate::db::repositories::timestamp();
        let message = "Clovy restarted before this run completed.";
        let mut transaction = self.pool.begin().await?;
        let result = query(
            "UPDATE agent_runs
             SET status = 'interrupted', updated_at = ?, completed_at = COALESCE(completed_at, ?),
                 error_code = COALESCE(error_code, 'runtime_restarted'),
                 error_message = COALESCE(error_message, ?)
             WHERE status IN ('queued', 'running')
               AND NOT EXISTS (
                 SELECT 1 FROM routine_runs WHERE routine_runs.agent_run_id = agent_runs.id
               )",
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(message)
        .execute(&mut *transaction)
        .await?;
        query(
            "UPDATE agent_sessions
             SET status = 'interrupted', updated_at = ?, last_error = ?
             WHERE EXISTS (
               SELECT 1 FROM agent_runs
               WHERE agent_runs.session_id = agent_sessions.id
                 AND agent_runs.status = 'interrupted'
                 AND agent_runs.error_code = 'runtime_restarted'
                 AND agent_runs.updated_at = ?
             )
               AND NOT EXISTS (
                 SELECT 1 FROM agent_runs
                 WHERE agent_runs.session_id = agent_sessions.id
                   AND agent_runs.status IN ('queued', 'running', 'waiting_for_user')
             )",
        )
        .bind(&timestamp)
        .bind(message)
        .bind(&timestamp)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(result.rows_affected())
    }

    pub async fn artifacts(&self, session_id: &str) -> Result<Vec<AgentArtifactDto>, sqlx::Error> {
        let rows = query("SELECT id, session_id, run_id, item_id, provenance, action, path, original_path, display_name, mime_type, size_bytes, available, created_at FROM agent_artifacts WHERE session_id = ? ORDER BY created_at ASC")
            .bind(session_id).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentArtifactDto {
                id: row.get("id"),
                session_id: row.get("session_id"),
                run_id: row.get("run_id"),
                item_id: row.get("item_id"),
                provenance: row.get("provenance"),
                action: row.get("action"),
                path: row.get("path"),
                original_path: row.get("original_path"),
                display_name: row.get("display_name"),
                mime_type: row.get("mime_type"),
                size_bytes: row.get("size_bytes"),
                available: row.get::<i64, _>("available") != 0,
                created_at: row.get("created_at"),
            })
            .collect())
    }

    pub async fn skills(&self) -> Result<Vec<AgentSkillDto>, sqlx::Error> {
        self.reconcile_obsidian_skill_settings().await?;
        let rows = query("SELECT skill_id, enabled, managed, updated_at FROM agent_skill_settings ORDER BY skill_id")
            .fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|row| AgentSkillDto {
                id: row.get("skill_id"),
                enabled: row.get::<i64, _>("enabled") != 0,
                managed: row.get::<i64, _>("managed") != 0,
                updated_at: row.get("updated_at"),
            })
            .collect())
    }

    pub async fn set_skill_enabled(
        &self,
        id: &str,
        enabled: bool,
        managed: bool,
    ) -> Result<AgentSkillDto, sqlx::Error> {
        let canonical_id = canonical_skill_id(id);
        let updated_at = crate::db::repositories::timestamp();
        let mut transaction = self.pool.begin().await?;
        for write_id in write_skill_ids(canonical_id) {
            query("INSERT INTO agent_skill_settings(skill_id, enabled, managed, updated_at) VALUES (?, ?, ?, ?) ON CONFLICT(skill_id) DO UPDATE SET enabled = excluded.enabled, managed = excluded.managed, updated_at = excluded.updated_at")
                .bind(write_id)
                .bind(enabled)
                .bind(managed)
                .bind(&updated_at)
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(AgentSkillDto {
            id: canonical_id.into(),
            enabled,
            managed,
            updated_at,
        })
    }

    /// A June-era row is read once into the Clovy id; it is never written.
    async fn reconcile_obsidian_skill_settings(&self) -> Result<(), sqlx::Error> {
        let canonical = query("SELECT 1 FROM agent_skill_settings WHERE skill_id = ?")
            .bind(CLOVY_OBSIDIAN_SKILL_ID)
            .fetch_optional(&self.pool)
            .await?;
        if canonical.is_some() {
            return Ok(());
        }
        query("INSERT INTO agent_skill_settings(skill_id, enabled, managed, updated_at) SELECT ?, enabled, managed, updated_at FROM agent_skill_settings WHERE skill_id = ?")
            .bind(CLOVY_OBSIDIAN_SKILL_ID)
            .bind(LEGACY_OBSIDIAN_SKILL_ID)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn session_from_row(row: SqliteRow) -> AgentSessionDto {
    AgentSessionDto {
        id: row.get("id"),
        title: row.get("title"),
        status: row.get("status"),
        model: row.get("model"),
        safety_mode: AgentSafetyMode::from(row.get::<String, _>("safety_mode").as_str()),
        workspace_path: row.get("workspace_path"),
        source: row.get("source"),
        created_at: row.get("created_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
        last_error: row.get("last_error"),
    }
}

fn run_from_row(row: SqliteRow) -> AgentRunDto {
    AgentRunDto {
        id: row.get("id"),
        session_id: row.get("session_id"),
        status: row.get("status"),
        model: row.get("model"),
        approval_mode: AgentApprovalMode::from(
            row.get::<Option<String>, _>("approval_mode")
                .as_deref()
                .unwrap_or("approvals"),
        ),
        reasoning_effort: row.get("reasoning_effort"),
        started_at: row.get("started_at"),
        updated_at: row.get("updated_at"),
        completed_at: row.get("completed_at"),
        usage: json_column(&row, "usage_json"),
        interrupted_state: json_column(&row, "interrupted_state_json"),
        last_sequence: row.get("last_sequence"),
        error_code: row.get("error_code"),
        error_message: row.get("error_message"),
    }
}

fn item_from_row(row: SqliteRow) -> Result<AgentItemDto, sqlx::Error> {
    let kind: String = row.get("kind");
    let payload_json: String = row.get("payload_json");
    let value = serde_json::from_str(&payload_json).map_err(decode_error)?;
    let payload = match kind.as_str() {
        "user_message" => {
            AgentItemPayload::UserMessage(serde_json::from_value(value).map_err(decode_error)?)
        }
        "assistant_message" => {
            AgentItemPayload::AssistantMessage(serde_json::from_value(value).map_err(decode_error)?)
        }
        "system_message" => {
            AgentItemPayload::SystemMessage(serde_json::from_value(value).map_err(decode_error)?)
        }
        "reasoning" => {
            AgentItemPayload::Reasoning(serde_json::from_value(value).map_err(decode_error)?)
        }
        "steering" => {
            AgentItemPayload::Steering(serde_json::from_value(value).map_err(decode_error)?)
        }
        "context_summary" => {
            AgentItemPayload::ContextSummary(serde_json::from_value(value).map_err(decode_error)?)
        }
        "tool_call" => {
            AgentItemPayload::ToolCall(serde_json::from_value(value).map_err(decode_error)?)
        }
        "tool_result" => {
            AgentItemPayload::ToolResult(serde_json::from_value(value).map_err(decode_error)?)
        }
        "interruption" => AgentItemPayload::Interruption(value),
        _ => AgentItemPayload::Error(value),
    };
    Ok(AgentItemDto {
        id: row.get("id"),
        session_id: row.get("session_id"),
        run_id: row.get("run_id"),
        sequence: row.get("sequence"),
        payload,
        external_id: row.get("external_id"),
        created_at: row.get("created_at"),
    })
}

fn json_column(row: &SqliteRow, column: &str) -> Option<serde_json::Value> {
    row.get::<Option<String>, _>(column)
        .and_then(|value| serde_json::from_str(&value).ok())
}

fn decode_error(error: serde_json::Error) -> sqlx::Error {
    sqlx::Error::Decode(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_sessions::domain::MessagePayload;
    use sqlx_sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn approval_mode_is_derived_from_safety_and_snapshotted_for_retries() {
        let repo = repository().await;
        let session = repo
            .create_session("Chat", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .unwrap();
        // A legacy preference cannot bypass sandboxed tools.
        query("UPDATE agent_sessions SET approval_mode = 'bypass' WHERE id = ?")
            .bind(&session.id)
            .execute(&repo.pool)
            .await
            .unwrap();
        let sandboxed = repo.create_run(&session.id, "auto", None).await.unwrap();
        assert_eq!(sandboxed.approval_mode, AgentApprovalMode::Approvals);
        query("UPDATE agent_sessions SET safety_mode = 'unrestricted', approval_mode = 'approvals' WHERE id = ?")
            .bind(&session.id).execute(&repo.pool).await.unwrap();
        let unrestricted = repo.create_run(&session.id, "auto", None).await.unwrap();
        assert_eq!(unrestricted.approval_mode, AgentApprovalMode::Bypass);
        repo.update_run_status(&unrestricted.id, "failed", None, None, None)
            .await
            .unwrap();
        let retry = repo.create_run(&session.id, "auto", None).await.unwrap();
        assert_eq!(retry.approval_mode, AgentApprovalMode::Bypass);
        query("UPDATE agent_sessions SET safety_mode = 'sandboxed' WHERE id = ?")
            .bind(&session.id)
            .execute(&repo.pool)
            .await
            .unwrap();
        assert_eq!(
            repo.get_run(&retry.id).await.unwrap().approval_mode,
            AgentApprovalMode::Bypass
        );
        let next = repo.create_run(&session.id, "auto", None).await.unwrap();
        assert_eq!(next.approval_mode, AgentApprovalMode::Approvals);
        repo.set_run_config(
            &next.id,
            &serde_json::json!({"approvalMode":"bypass", "surface":"chat"}),
        )
        .await
        .unwrap();
        assert_eq!(
            repo.get_run(&next.id).await.unwrap().approval_mode,
            AgentApprovalMode::Approvals
        );
    }

    async fn repository() -> AgentRepository {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("memory database");
        crate::db::migrations::run_migrations(&pool)
            .await
            .expect("migrations");
        AgentRepository::new(pool)
    }

    #[tokio::test]
    async fn terminal_run_statuses_cannot_return_to_running() {
        let repository = repository().await;
        let failed_session = repository
            .create_session("Failed", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("failed session");
        let failed_run = repository
            .create_run(&failed_session.id, "auto", None)
            .await
            .expect("failed run");
        repository
            .update_run_status(
                &failed_run.id,
                "failed",
                None,
                None,
                Some(("dispatch_failed", "dispatch timed out")),
            )
            .await
            .expect("failed status");

        let failed = repository
            .update_run_status(&failed_run.id, "running", None, None, None)
            .await
            .expect("late running status");
        assert_eq!(failed.status, "failed");
        assert_eq!(
            repository
                .get_session(&failed_session.id)
                .await
                .expect("failed session state")
                .status,
            "failed"
        );

        let cancelled_session = repository
            .create_session("Cancelled", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("cancelled session");
        let cancelled_run = repository
            .create_run(&cancelled_session.id, "auto", None)
            .await
            .expect("cancelled run");
        repository
            .update_run_status(&cancelled_run.id, "cancelled", None, None, None)
            .await
            .expect("cancelled status");

        let cancelled = repository
            .update_run_status(&cancelled_run.id, "running", None, None, None)
            .await
            .expect("late running status");
        assert_eq!(cancelled.status, "cancelled");
        assert_eq!(
            repository
                .get_session(&cancelled_session.id)
                .await
                .expect("cancelled session state")
                .status,
            "idle"
        );
    }

    #[tokio::test]
    async fn cancelling_a_waiting_run_expires_its_interruption_and_releases_the_session() {
        let repository = repository().await;
        let session = repository
            .create_session("Waiting", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("waiting session");
        let run = repository
            .create_run(&session.id, "auto", None)
            .await
            .expect("waiting run");
        repository
            .append_item(
                &session.id,
                Some(&run.id),
                1,
                &AgentItemPayload::Interruption(serde_json::json!({
                    "id": "clarify-1",
                    "kind": "clarification",
                    "status": "pending",
                    "question": "Which project?"
                })),
                Some("interruption:clarify-1"),
            )
            .await
            .expect("pending interruption");
        repository
            .update_run_status(
                &run.id,
                "waiting_for_user",
                None,
                Some(&serde_json::json!("serialized")),
                None,
            )
            .await
            .expect("waiting status");

        let cancelled = repository
            .cancel_waiting_run(&run.id)
            .await
            .expect("cancel waiting run")
            .expect("cancellation won the pending interruption race");

        assert_eq!(cancelled.status, "cancelled");
        assert!(cancelled.completed_at.is_some());
        assert_eq!(
            repository
                .get_session(&session.id)
                .await
                .expect("released session")
                .status,
            "idle"
        );
        let items = repository.items(&session.id).await.expect("retired items");
        assert!(matches!(
            &items[0].payload,
            AgentItemPayload::Interruption(value)
                if value.get("status").and_then(serde_json::Value::as_str) == Some("expired")
                    && value.get("resolvedAt").and_then(serde_json::Value::as_str).is_some()
        ));
    }

    #[tokio::test]
    async fn waiting_run_cancellation_loses_to_an_already_resolved_interruption() {
        let repository = repository().await;
        let session = repository
            .create_session("Resuming", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("resuming session");
        let run = repository
            .create_run(&session.id, "auto", None)
            .await
            .expect("resuming run");
        repository
            .append_item(
                &session.id,
                Some(&run.id),
                1,
                &AgentItemPayload::Interruption(serde_json::json!({
                    "id": "clarify-1",
                    "kind": "clarification",
                    "status": "resolved",
                    "question": "Which project?",
                    "answer": "June"
                })),
                Some("interruption:clarify-1"),
            )
            .await
            .expect("resolved interruption");
        repository
            .update_run_status(
                &run.id,
                "waiting_for_user",
                None,
                Some(&serde_json::json!("serialized")),
                None,
            )
            .await
            .expect("waiting status");

        let cancelled = repository
            .cancel_waiting_run(&run.id)
            .await
            .expect("race check");

        assert!(cancelled.is_none());
        assert_eq!(
            repository
                .get_run(&run.id)
                .await
                .expect("unchanged run")
                .status,
            "waiting_for_user"
        );
        assert_eq!(
            repository
                .get_session(&session.id)
                .await
                .expect("unchanged session")
                .status,
            "waiting_for_user"
        );
    }

    #[tokio::test]
    async fn waiting_run_cancellation_rolls_back_when_routine_claim_release_fails() {
        let repository = repository().await;
        crate::routines::create(
            &repository.pool,
            crate::routines::CreateAgentRoutineRequest {
                id: Some("routine-atomic".into()),
                legacy_job_id: None,
                name: Some("Atomic cancellation".into()),
                prompt: "Prepare a brief".into(),
                schedule: "every 1h".into(),
                timezone: "UTC".into(),
                repeat: None,
                deliver: None,
                model: "auto".into(),
                safety_mode: Some(AgentSafetyMode::Sandboxed),
                state: None,
                enabled: None,
                metadata: serde_json::json!({}),
                enabled_toolsets: None,
            },
        )
        .await
        .expect("routine");
        let claim = crate::routines::claim(&repository.pool, "routine-atomic", "manual", false)
            .await
            .expect("claim")
            .expect("available claim");
        let session = repository
            .create_session(
                "Atomic cancellation",
                "auto",
                AgentSafetyMode::Sandboxed,
                None,
            )
            .await
            .expect("session");
        let run = repository
            .create_run(&session.id, "auto", None)
            .await
            .expect("run");
        repository
            .append_item(
                &session.id,
                Some(&run.id),
                1,
                &AgentItemPayload::Interruption(serde_json::json!({
                    "id": "clarify-atomic",
                    "kind": "clarification",
                    "status": "pending",
                    "question": "Which project?"
                })),
                Some("interruption:clarify-atomic"),
            )
            .await
            .expect("pending interruption");
        repository
            .update_run_status(
                &run.id,
                "waiting_for_user",
                None,
                Some(&serde_json::json!("serialized")),
                None,
            )
            .await
            .expect("waiting run");
        let timestamp = crate::db::repositories::timestamp();
        query(
            "UPDATE routine_runs
             SET agent_session_id = ?, agent_run_id = ?, status = 'waiting_for_user',
                 started_at = ?, updated_at = ?
             WHERE id = ? AND claim_token = ?",
        )
        .bind(&session.id)
        .bind(&run.id)
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(&claim.routine_run_id)
        .bind(&claim.token)
        .execute(&repository.pool)
        .await
        .expect("attach run");
        query("UPDATE routines SET schedule = 'not a schedule' WHERE id = 'routine-atomic'")
            .execute(&repository.pool)
            .await
            .expect("invalid schedule");

        let error = repository
            .cancel_waiting_run(&run.id)
            .await
            .expect_err("claim release must fail");

        assert_eq!(error.code, "routine_schedule_invalid");
        assert_eq!(
            repository
                .get_run(&run.id)
                .await
                .expect("unchanged run")
                .status,
            "waiting_for_user"
        );
        assert!(matches!(
            &repository.items(&session.id).await.expect("items")[0].payload,
            AgentItemPayload::Interruption(value)
                if value.get("status").and_then(serde_json::Value::as_str) == Some("pending")
        ));
        assert_eq!(
            repository
                .get_session(&session.id)
                .await
                .expect("unchanged session")
                .status,
            "waiting_for_user"
        );
        let routine_run = crate::routines::list_runs(&repository.pool, Some("routine-atomic"))
            .await
            .expect("routine runs")
            .remove(0);
        assert_eq!(routine_run.status, "waiting_for_user");
        let claim_token: Option<String> =
            query("SELECT claim_token FROM routines WHERE id = 'routine-atomic'")
                .fetch_one(&repository.pool)
                .await
                .expect("routine claim")
                .get("claim_token");
        assert_eq!(claim_token.as_deref(), Some(claim.token.as_str()));
    }

    #[tokio::test]
    async fn context_summary_metadata_is_persisted_with_the_replacement() {
        let repository = repository().await;
        let session = repository
            .create_session("Summary", "auto", AgentSafetyMode::Sandboxed, None)
            .await
            .expect("session");
        let run = repository
            .create_run(&session.id, "auto", None)
            .await
            .expect("run");
        let first = repository
            .append_item(
                &session.id,
                Some(&run.id),
                1,
                &AgentItemPayload::UserMessage(MessagePayload {
                    role: "user".into(),
                    content: "Earlier question".into(),
                    attachments: Vec::new(),
                }),
                Some("summary-source-1"),
            )
            .await
            .expect("first source")
            .expect("inserted first source");
        let second = repository
            .append_item(
                &session.id,
                Some(&run.id),
                2,
                &AgentItemPayload::UserMessage(MessagePayload {
                    role: "user".into(),
                    content: "Later question".into(),
                    attachments: Vec::new(),
                }),
                Some("summary-source-2"),
            )
            .await
            .expect("second source")
            .expect("inserted second source");
        repository
            .update_run_status(&run.id, "completed", None, None, None)
            .await
            .expect("completed run");
        let metadata = serde_json::json!({ "fallback": true });
        let replacement = repository
            .replace_items_with_context_summary(
                &session.id,
                &run.id,
                "Bounded deterministic context",
                Some(&metadata),
                &[first.id, second.id],
            )
            .await
            .expect("replace history");
        let Some(summary) = replacement else {
            panic!("unchanged idle history must be replaced");
        };

        let AgentItemPayload::ContextSummary(payload) = summary.payload else {
            panic!("replacement must be a context summary");
        };
        assert_eq!(payload.metadata, Some(metadata));
        assert_eq!(repository.items(&session.id).await.expect("items").len(), 1);
    }
}
