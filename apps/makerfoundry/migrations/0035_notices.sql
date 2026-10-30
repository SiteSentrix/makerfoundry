-- Issue #31: email notices. Each member chooses whether to get a daily digest (off unless they
-- turn it on); notices wait here until the digest goes, and are deleted when they turn it off.
CREATE TABLE mf_notice_prefs (
    account_id BIGINT PRIMARY KEY REFERENCES mf_accounts (account_id),
    digest BOOLEAN NOT NULL DEFAULT false,
    last_sent_at TIMESTAMPTZ
);
CREATE TABLE mf_notices (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    kind TEXT NOT NULL CHECK (kind IN ('comment', 'make', 'remix', 'follow')),
    design_id BIGINT REFERENCES mf_designs (id) ON DELETE SET NULL,
    actor_account BIGINT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ
);
CREATE INDEX mf_notices_waiting ON mf_notices (account_id, created_at) WHERE sent_at IS NULL;
