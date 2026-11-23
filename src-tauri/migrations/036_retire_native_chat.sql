-- Preserve chat history while retiring execution state owned by the removed engine.
CREATE TABLE IF NOT EXISTS agent_secret_retirements (
    secret_ref TEXT PRIMARY KEY,
    completed_at TEXT
);
INSERT OR IGNORE INTO agent_secret_retirements(secret_ref)
SELECT json_extract(payload_json, '$.secretRef') FROM agent_items
WHERE kind = 'interruption' AND json_valid(payload_json)
  AND json_type(payload_json, '$.secretRef') = 'text';
UPDATE agent_items
SET payload_json = json_set(payload_json, '$.status', 'cancelled', '$.resolvedAt', strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
WHERE kind = 'interruption' AND json_valid(payload_json)
  AND json_extract(payload_json, '$.status') = 'pending'
  AND run_id IN (SELECT id FROM agent_runs WHERE model NOT GLOB '__clovy_cli_engine__:*');
UPDATE agent_runs
SET status = 'cancelled', interrupted_state_json = NULL,
    completed_at = COALESCE(completed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now'),
    error_code = 'native_engine_retired',
    error_message = 'The previous chat engine was removed. Retry with Claude Code.'
WHERE model NOT GLOB '__clovy_cli_engine__:*'
  AND status IN ('queued', 'running', 'waiting_for_user');
UPDATE agent_sessions
SET status = 'idle', last_error = NULL
WHERE status IN ('queued', 'running', 'waiting_for_user')
  AND NOT EXISTS (SELECT 1 FROM agent_runs WHERE session_id = agent_sessions.id
    AND status IN ('queued', 'running', 'waiting_for_user'));
UPDATE routine_runs SET status = 'cancelled', completed_at = COALESCE(completed_at, strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    error_code = 'native_engine_retired', error_message = 'The previous chat engine was removed. Retry with Claude Code.'
WHERE agent_run_id IN (SELECT id FROM agent_runs WHERE error_code = 'native_engine_retired')
  AND status IN ('queued', 'running', 'waiting_for_user');
