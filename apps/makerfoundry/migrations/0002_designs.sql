-- MF-2: designs a maker has uploaded (drafts until MF-3 publishes them), and their photos. The
-- files themselves are in storage (R2) under random keys; only the keys are here.
CREATE TABLE mf_designs (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    kind TEXT NOT NULL CHECK (kind IN ('embroidery', 'laser')),
    -- what the uploader called the file (shown escaped, and cleaned for downloads)
    file_name TEXT NOT NULL,
    file_key TEXT NOT NULL,
    file_bytes BIGINT NOT NULL,
    format TEXT NOT NULL,
    -- what reading the file found (size, stitches or lines, colours), as JSON
    facts JSONB NOT NULL,
    -- the preview MakerFoundry drew (its own SVG, never the upload)
    preview_svg TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_designs_account ON mf_designs (account_id, created_at DESC);

CREATE TABLE mf_design_photos (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    photo_key TEXT NOT NULL,
    thumb_key TEXT NOT NULL,
    width INT NOT NULL,
    height INT NOT NULL,
    position INT NOT NULL
);
CREATE INDEX mf_design_photos_design ON mf_design_photos (design_id, position);
