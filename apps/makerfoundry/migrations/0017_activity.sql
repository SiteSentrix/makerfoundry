-- MF-22: when a maker last looked at their activity (comments, makes, sales and follows on their
-- designs, drawn from those tables), so what came after shows as new.
CREATE TABLE mf_activity_seen (
    account_id BIGINT PRIMARY KEY REFERENCES mf_accounts (account_id),
    seen_at TIMESTAMPTZ NOT NULL
);
