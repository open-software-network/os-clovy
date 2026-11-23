-- Finished chunks of a split saved-audio job, so a resumed run sends only
-- the chunks still missing. The key is the chunk's request operation id plus
-- its audio length: a different split never reuses a stale chunk.
CREATE TABLE IF NOT EXISTS note_transcription_chunks (
  chunk_key TEXT PRIMARY KEY NOT NULL,
  recording_session_id TEXT NOT NULL REFERENCES recording_sessions(id) ON DELETE CASCADE,
  text TEXT NOT NULL,
  language TEXT,
  no_speech INTEGER NOT NULL DEFAULT 0 CHECK (no_speech IN (0, 1)),
  created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_note_transcription_chunks_session
ON note_transcription_chunks (recording_session_id);
