CREATE TABLE companion_devices (
    device_id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    public_key BYTEA NOT NULL CHECK (octet_length(public_key) = 32),
    display_name TEXT NOT NULL CHECK (octet_length(display_name) BETWEEN 1 AND 128),
    credential_hash BYTEA CHECK (octet_length(credential_hash) = 32),
    apns_token BYTEA CHECK (octet_length(apns_token) BETWEEN 16 AND 256),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    revoked_at TIMESTAMPTZ
);

CREATE INDEX companion_devices_user_id_idx
    ON companion_devices (user_id)
    WHERE revoked_at IS NULL;

CREATE TABLE companion_device_links (
    left_device_id UUID NOT NULL REFERENCES companion_devices(device_id) ON DELETE CASCADE,
    right_device_id UUID NOT NULL REFERENCES companion_devices(device_id) ON DELETE CASCADE,
    user_id TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (left_device_id, right_device_id),
    CHECK (left_device_id <> right_device_id)
);

CREATE INDEX companion_device_links_user_id_idx ON companion_device_links (user_id);
