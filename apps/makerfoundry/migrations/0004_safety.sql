-- MF-5: reports, copyright notices and counter-notices, and what admins did about them.

ALTER TABLE mf_designs DROP CONSTRAINT mf_designs_status_check;
ALTER TABLE mf_designs ADD CONSTRAINT mf_designs_status_check CHECK (status IN ('draft', 'published', 'removed'));
-- why a removed design was taken down, as its maker is told
ALTER TABLE mf_designs ADD COLUMN removed_reason TEXT;

CREATE TABLE mf_reports (
    id BIGSERIAL PRIMARY KEY,
    -- the design reported (kept, if the design is deleted, as no design)
    design_id BIGINT REFERENCES mf_designs (id) ON DELETE SET NULL,
    -- 'report' (anyone), 'dmca' (a copyright notice), 'counter' (a maker's counter-notice)
    kind TEXT NOT NULL CHECK (kind IN ('report', 'dmca', 'counter')),
    reason TEXT NOT NULL,
    details TEXT NOT NULL DEFAULT '' CHECK (length(details) <= 5000),
    reporter_account BIGINT,
    -- for notices: who sent them, as the law asks (17 U.S.C. § 512(c)(3), (g)(3))
    contact_name TEXT NOT NULL DEFAULT '',
    contact_email TEXT NOT NULL DEFAULT '',
    contact_address TEXT NOT NULL DEFAULT '',
    contact_phone TEXT NOT NULL DEFAULT '',
    original_work TEXT NOT NULL DEFAULT '',
    signature TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'open' CHECK (status IN ('open', 'actioned', 'dismissed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    resolved_by BIGINT
);
CREATE INDEX mf_reports_open ON mf_reports (created_at) WHERE status = 'open';

CREATE TABLE mf_moderation_log (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT,
    report_id BIGINT,
    admin_account BIGINT NOT NULL,
    action TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
