-- Preserve every observed result/effect. A terminal run without an observed
-- final result has an interrupted call, never a fabricated success or failure.
UPDATE agent_items
SET payload_json = json_set(payload_json, '$.status', 'interrupted')
WHERE kind = 'tool_call'
  AND json_extract(payload_json, '$.status') = 'running'
  AND json_extract(payload_json, '$.result') IS NULL
  AND EXISTS (
    SELECT 1 FROM agent_runs AS run
    WHERE run.id = agent_items.run_id AND run.session_id = agent_items.session_id
      AND run.status IN ('completed', 'cancelled', 'failed', 'interrupted')
  )
  AND NOT EXISTS (
    SELECT 1 FROM agent_items AS result
    WHERE result.session_id = agent_items.session_id
      AND result.run_id = agent_items.run_id
      AND result.kind = 'tool_result'
      AND json_extract(result.payload_json, '$.toolCallId') =
          json_extract(agent_items.payload_json, '$.toolCallId')
  );

-- Distinct schema landmark for safe unversioned adoption (ADR-0037).
CREATE TABLE IF NOT EXISTS agent_tool_terminality_backfill (
  id INTEGER PRIMARY KEY CHECK (id = 1)
);
INSERT OR IGNORE INTO agent_tool_terminality_backfill (id) VALUES (1);
