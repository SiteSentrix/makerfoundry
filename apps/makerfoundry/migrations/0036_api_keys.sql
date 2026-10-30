-- Issue #36: keys for the public read-only API for 3D models. Only a key's SHA-256 is kept; the
-- key itself is shown once, when it's made.
CREATE TABLE mf_api_keys (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    name TEXT NOT NULL,
    key_hash BYTEA NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ
);
CREATE INDEX mf_api_keys_account ON mf_api_keys (account_id, created_at DESC);
