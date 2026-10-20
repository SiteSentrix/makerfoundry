-- MF-17: links for buyers to open a paid design in StitchFoundry's web app, which fetches without
-- anyone's sign-in: made once the buyer's right to the file is checked, good for that design for
-- ten minutes, kept only as their hash.
CREATE TABLE mf_open_links (
    token_hash TEXT PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX mf_open_links_expires ON mf_open_links (expires_at);
