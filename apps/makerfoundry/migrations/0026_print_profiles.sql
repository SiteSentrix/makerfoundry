-- MF-43: print profiles: what a 3MF project's slicer settings say (src/model3d/profile.rs), one
-- per project file. Only these fields are kept, never the settings files' own text.
CREATE TABLE mf_print_profiles (
    id BIGSERIAL PRIMARY KEY,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    file_id BIGINT NOT NULL UNIQUE REFERENCES mf_model_files (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slicer TEXT NOT NULL CHECK (slicer IN ('bambu', 'orca', 'prusa', 'cura', 'unknown')),
    printer TEXT,
    printers TEXT[] NOT NULL DEFAULT '{}',
    nozzle_mm DOUBLE PRECISION,
    layer_mm DOUBLE PRECISION,
    infill_percent DOUBLE PRECISION,
    walls INTEGER,
    supports BOOLEAN,
    plates INTEGER,
    filaments JSONB NOT NULL DEFAULT '[]',
    readable BOOLEAN NOT NULL
);
CREATE INDEX mf_print_profiles_design ON mf_print_profiles (design_id);
