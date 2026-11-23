CREATE TABLE IF NOT EXISTS video_artifacts (
  recording_session_id TEXT PRIMARY KEY NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
  note_id TEXT NOT NULL REFERENCES notes(id) ON DELETE CASCADE,
  partial_path TEXT NOT NULL,
  path TEXT NOT NULL,
  status TEXT NOT NULL CHECK (status IN ('recording', 'valid', 'failed', 'recovered')),
  offset_ms INTEGER NOT NULL DEFAULT 0,
  app TEXT NOT NULL DEFAULT '',
  title TEXT NOT NULL DEFAULT '',
  width INTEGER,
  height INTEGER,
  duration_ms INTEGER,
  failure_reason TEXT
);
CREATE INDEX IF NOT EXISTS idx_video_artifacts_note ON video_artifacts(note_id);

-- Reverse scan also satisfies the latest generation rowid tie-break.
CREATE INDEX IF NOT EXISTS idx_generation_results_note_created_at
ON generation_results(note_id, created_at);
