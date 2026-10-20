-- MF-13: who follows which maker, to see their new designs on /following.
CREATE TABLE mf_follows (
    follower_account BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    maker_account BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (follower_account, maker_account),
    CHECK (follower_account <> maker_account)
);
CREATE INDEX mf_follows_maker ON mf_follows (maker_account);
