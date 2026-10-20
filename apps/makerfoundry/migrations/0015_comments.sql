-- MF-14: comments on designs, and reports of them.
CREATE TABLE mf_comments (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    body TEXT NOT NULL CHECK (length(body) BETWEEN 1 AND 2000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_comments_design ON mf_comments (design_id, created_at);

ALTER TABLE mf_reports DROP CONSTRAINT mf_reports_kind_check;
ALTER TABLE mf_reports ADD CONSTRAINT mf_reports_kind_check
    CHECK (kind IN ('report', 'dmca', 'counter', 'make', 'comment'));
ALTER TABLE mf_reports ADD COLUMN comment_id BIGINT REFERENCES mf_comments (id) ON DELETE SET NULL;
