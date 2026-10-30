-- MF-52: bulk imports of a creator's own models. Each file sent (or found in a ZIP sent) is kept
-- as a pending import file until its group becomes a draft; a file that can't be used keeps its
-- row, without a stored copy, to say why. Pending files and unpublished imported drafts go after
-- 30 days (the owner's decision, 2026-10-30).
CREATE TABLE mf_imports (
    id BIGSERIAL PRIMARY KEY,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_imports_account ON mf_imports (account_id, created_at DESC);
CREATE TABLE mf_import_files (
    id BIGSERIAL PRIMARY KEY,
    import_id BIGINT NOT NULL REFERENCES mf_imports (id) ON DELETE CASCADE,
    -- as sent, with its folders inside a ZIP (only for grouping and showing)
    path TEXT NOT NULL,
    file_key TEXT UNIQUE,
    file_bytes BIGINT NOT NULL,
    image BOOLEAN NOT NULL,
    group_name TEXT NOT NULL DEFAULT '',
    design_id BIGINT REFERENCES mf_designs (id) ON DELETE SET NULL,
    problem TEXT,
    position INTEGER NOT NULL
);
CREATE INDEX mf_import_files_import ON mf_import_files (import_id, position);
ALTER TABLE mf_designs ADD COLUMN import_id BIGINT REFERENCES mf_imports (id) ON DELETE SET NULL;
