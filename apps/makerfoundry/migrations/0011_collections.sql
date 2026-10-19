-- MF-10: saved designs and collections. Each account has at most one Saved list (made the first
-- time something is saved), and collections of its own, private unless made public.
CREATE TABLE mf_collections (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    name TEXT NOT NULL,
    public BOOLEAN NOT NULL DEFAULT false,
    is_saved BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (NOT (is_saved AND public))
);
CREATE UNIQUE INDEX mf_collections_saved ON mf_collections (account_id) WHERE is_saved;
CREATE INDEX mf_collections_account ON mf_collections (account_id);

CREATE TABLE mf_collection_items (
    collection_id BIGINT NOT NULL REFERENCES mf_collections (id) ON DELETE CASCADE,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (collection_id, design_id)
);
CREATE INDEX mf_collection_items_design ON mf_collection_items (design_id);
