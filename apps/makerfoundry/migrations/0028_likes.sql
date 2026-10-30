-- MF-48: likes on 3D models: one an account a model.
CREATE TABLE mf_likes (
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (account_id, design_id)
);
CREATE INDEX mf_likes_design ON mf_likes (design_id, created_at);
