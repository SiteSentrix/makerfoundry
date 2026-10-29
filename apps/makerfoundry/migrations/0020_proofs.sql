-- MF-25: approval links. A shop (any Foundry Account; the owner's decision, 2026-10-29) sends a
-- customer a proof: a design, read and checked like an upload, with mock-up photos. The customer
-- opens a private link without an account, comments, and approves the latest version or asks for
-- changes. The link isn't kept: it's made from the proof's id and `link_generation` with the
-- server's key (`MF_PROOF_KEY`), so a new link (the generation + 1) ends the old one. A customer's
-- name and words are the shop's (the owner's decision, 2026-10-29); a proof is deleted a year
-- after its last activity (`active_at`).
CREATE TABLE mf_proofs (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    title TEXT NOT NULL,
    -- the name the customer sees it from, and the shop's own note of who it's for
    shop_name TEXT NOT NULL,
    customer TEXT NOT NULL DEFAULT '',
    message TEXT NOT NULL DEFAULT '',
    link_generation INTEGER NOT NULL DEFAULT 1 CHECK (link_generation > 0),
    link_on BOOLEAN NOT NULL DEFAULT true,
    -- of the latest version: sent, viewed by the customer, approved, or changes asked for
    status TEXT NOT NULL DEFAULT 'sent' CHECK (status IN ('sent', 'viewed', 'approved', 'changes')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    active_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_proofs_shop ON mf_proofs (account_id, active_at DESC);
CREATE INDEX mf_proofs_active ON mf_proofs (active_at);

CREATE TABLE mf_proof_versions (
    id BIGSERIAL PRIMARY KEY,
    proof_id BIGINT NOT NULL REFERENCES mf_proofs (id) ON DELETE CASCADE,
    number INTEGER NOT NULL CHECK (number > 0),
    kind TEXT NOT NULL CHECK (kind IN ('embroidery', 'laser')),
    file_name TEXT NOT NULL,
    file_key TEXT NOT NULL,
    file_bytes BIGINT NOT NULL,
    format TEXT NOT NULL,
    facts JSONB NOT NULL,
    preview_svg TEXT NOT NULL,
    width_mm DOUBLE PRECISION,
    height_mm DOUBLE PRECISION,
    -- what the shop says about this version (what changed)
    note TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (proof_id, number)
);

CREATE TABLE mf_proof_photos (
    id BIGSERIAL PRIMARY KEY,
    version_id BIGINT NOT NULL REFERENCES mf_proof_versions (id) ON DELETE CASCADE,
    photo_key TEXT NOT NULL,
    thumb_key TEXT NOT NULL,
    width INTEGER NOT NULL,
    height INTEGER NOT NULL,
    position INTEGER NOT NULL
);
CREATE INDEX mf_proof_photos_version ON mf_proof_photos (version_id, position);

-- The conversation under a proof: comments from either side, and the customer's decisions on a
-- version (approved, or changes asked for), each with the name they typed.
CREATE TABLE mf_proof_replies (
    id BIGSERIAL PRIMARY KEY,
    proof_id BIGINT NOT NULL REFERENCES mf_proofs (id) ON DELETE CASCADE,
    version INTEGER NOT NULL,
    by_shop BOOLEAN NOT NULL,
    name TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL DEFAULT '',
    decision TEXT NOT NULL CHECK (decision IN ('comment', 'approved', 'changes')),
    at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_proof_replies_proof ON mf_proof_replies (proof_id, at);
