-- MF-54: founding creators (the owner's decision, 2026-10-30): makers apply at /founding, an admin
-- approves up to 100, and each approved maker gets a numbered badge ("Founding creator #7").
-- One application an account; the number is given on approval (the lowest free one) and freed
-- if the badge is removed.
CREATE TABLE mf_founding (
    account_id BIGINT PRIMARY KEY REFERENCES mf_accounts (account_id),
    -- what they wrote: the name to know them by, a link to their work, what they make
    name TEXT NOT NULL,
    work_url TEXT NOT NULL DEFAULT '',
    about TEXT NOT NULL DEFAULT '',
    applied_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    status TEXT NOT NULL DEFAULT 'waiting' CHECK (status IN ('waiting', 'approved', 'declined')),
    decided_at TIMESTAMPTZ,
    decided_by BIGINT,
    number INTEGER UNIQUE CHECK (number > 0),
    CHECK ((status = 'approved') = (number IS NOT NULL))
);
CREATE INDEX mf_founding_status ON mf_founding (status, applied_at);
