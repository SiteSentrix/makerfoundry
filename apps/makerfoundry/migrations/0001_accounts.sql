-- MF-1: who has signed in to MakerFoundry, and their sessions. The accounts themselves (email,
-- password) live in StitchFoundry's account service; MakerFoundry keeps its records under the
-- account's id there.
CREATE TABLE mf_accounts (
    account_id BIGINT PRIMARY KEY,
    email TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- A browser's session: the SHA-256 of the id in its cookie (the id itself is never kept), and the
-- account service's token for the same sign-in, so signing out ends both and a session ended
-- there (a password reset) ends here too.
CREATE TABLE mf_sessions (
    id_hash TEXT PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    cloud_token TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    checked_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_sessions_account ON mf_sessions (account_id);
