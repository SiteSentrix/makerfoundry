-- MF-11: makes, photos people post of what they made from a design, with a short note.
CREATE TABLE mf_makes (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    photo_key TEXT NOT NULL,
    thumb_key TEXT NOT NULL,
    width INT NOT NULL,
    height INT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_makes_design ON mf_makes (design_id, created_at DESC);
