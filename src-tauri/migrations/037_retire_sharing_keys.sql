-- Sharing is retired. Purge local key material. Keep historical table/index
-- definitions intact so earlier migration stamps remain valid (ADR-0037).
DELETE FROM share_invite_keys;
DELETE FROM share_keys;

-- Give the catalog a distinct schema marker for unversioned database adoption.
CREATE TABLE IF NOT EXISTS sharing_retirement (
  id INTEGER PRIMARY KEY CHECK (id = 1)
);
INSERT OR IGNORE INTO sharing_retirement (id) VALUES (1);
