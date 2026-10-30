-- MF-41: a 3D model's files (docs/3d-hub-plan.md § 2): meshes, 3MF projects, STEP sources and
-- text companions, each read and checked (src/model3d/files.rs). A 3MF is kept as rebuilt; what
-- its rebuild dropped is listed for its maker. Meshes and projects get a main picture (a 3MF's own
-- thumbnail, or drawn) and, for meshes, front, side and top views drawn by the server.
CREATE TABLE mf_model_files (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('mesh', 'project', 'source', 'other')),
    format TEXT NOT NULL,
    file_name TEXT NOT NULL,
    file_key TEXT NOT NULL UNIQUE,
    file_bytes BIGINT NOT NULL,
    -- what reading found, for meshes and projects
    triangles BIGINT,
    size_mm DOUBLE PRECISION[],
    thumb_key TEXT,
    thumb_type TEXT,
    view_keys TEXT[] NOT NULL DEFAULT '{}',
    dropped TEXT[] NOT NULL DEFAULT '{}',
    position INTEGER NOT NULL
);
CREATE INDEX mf_model_files_design ON mf_model_files (design_id, position);
