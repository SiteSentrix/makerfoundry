-- Issue #28: importing from a linked Thingiverse account. The one-time state of a sign-in at
-- Thingiverse (used once, within 10 minutes), and what each imported file's thing said about
-- where it was first published: its address, date and licence. Tokens are never stored.
CREATE TABLE mf_oauth_states (
    state TEXT PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE mf_import_files ADD COLUMN origin_url TEXT;
ALTER TABLE mf_import_files ADD COLUMN origin_date DATE;
ALTER TABLE mf_import_files ADD COLUMN origin_licence TEXT;
