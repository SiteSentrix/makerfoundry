-- MF-56: posts the admins made by hand from a showcase consent: where and when, and when taken
-- down. A post whose consent is withdrawn (or whose model is taken down) is flagged until marked
-- taken down.
CREATE TABLE mf_showcase_posts (
    id BIGSERIAL PRIMARY KEY,
    consent_id BIGINT NOT NULL REFERENCES mf_showcase_consents (id),
    platform TEXT NOT NULL CHECK (platform IN ('facebook', 'instagram', 'youtube')),
    url TEXT NOT NULL,
    posted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    posted_by BIGINT NOT NULL,
    taken_down_at TIMESTAMPTZ
);
CREATE INDEX mf_showcase_posts_consent ON mf_showcase_posts (consent_id);
