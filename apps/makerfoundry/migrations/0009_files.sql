-- MF-8: a design's other files: the same design in more formats, each read and checked.
CREATE TABLE mf_design_files (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    file_name TEXT NOT NULL,
    file_key TEXT NOT NULL UNIQUE,
    file_bytes BIGINT NOT NULL,
    format TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_design_files_design ON mf_design_files (design_id);
