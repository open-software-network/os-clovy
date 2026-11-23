-- Private sharing (JUN-308). The server stores ciphertext, per-recipient key
-- envelopes, and ACL metadata only; there is no June-side decryption path.
CREATE TABLE shares (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    share_id TEXT NOT NULL UNIQUE CHECK (share_id LIKE 'shr_%'),
    owner_user_id TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('note', 'session')),
    ciphertext BYTEA NOT NULL,
    iv BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    deleted_at TIMESTAMPTZ
);

CREATE INDEX idx_shares_owner ON shares (owner_user_id) WHERE deleted_at IS NULL;

CREATE TABLE share_invites (
    id BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    invite_id TEXT NOT NULL UNIQUE CHECK (invite_id LIKE 'shi_%'),
    share_id BIGINT NOT NULL REFERENCES shares(id) ON DELETE CASCADE,
    -- Lowercased before insert; matched against the caller's verified emails.
    email TEXT NOT NULL CHECK (email = lower(email)),
    envelope BYTEA NOT NULL,
    envelope_iv BYTEA NOT NULL,
    recipient_user_id TEXT,
    accepted_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    last_access_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_share_invites_share ON share_invites (share_id);
-- At most one active (non-revoked) invite per email per share. The viewer
-- authorizes by any active invite matching a verified email, so a duplicate
-- active row would survive revoking the first. Enforced in the database so it
-- holds under concurrent add-invite requests regardless of isolation level;
-- revoked rows are excluded so re-inviting after a revoke is allowed.
CREATE UNIQUE INDEX idx_share_invites_active_email
    ON share_invites (share_id, email) WHERE revoked_at IS NULL;
