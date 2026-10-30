-- MF-49: what 3D search filters on, kept on each model (refreshed when its files change,
-- src/model3d/search.rs) so a filtered page reads one table through its indexes; and how popular
-- it is lately (likes and makes of the last 30 days, and downloads), refreshed by the sweep.
ALTER TABLE mf_designs ADD COLUMN formats TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE mf_designs ADD COLUMN printers TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE mf_designs ADD COLUMN filament_types TEXT[] NOT NULL DEFAULT '{}';
ALTER TABLE mf_designs ADD COLUMN has_profile BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE mf_designs ADD COLUMN multicolour BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE mf_designs ADD COLUMN popularity BIGINT NOT NULL DEFAULT 0;
CREATE INDEX mf_designs_3d_newest ON mf_designs (published_at DESC, id DESC) WHERE kind = 'model3d' AND status = 'published';
CREATE INDEX mf_designs_3d_popular ON mf_designs (popularity DESC, id DESC) WHERE kind = 'model3d' AND status = 'published';
CREATE INDEX mf_designs_3d_category ON mf_designs (category, published_at DESC) WHERE kind = 'model3d' AND status = 'published';
CREATE INDEX mf_designs_3d_formats ON mf_designs USING GIN (formats) WHERE kind = 'model3d';
CREATE INDEX mf_designs_3d_printers ON mf_designs USING GIN (printers) WHERE kind = 'model3d';
CREATE INDEX mf_designs_3d_filaments ON mf_designs USING GIN (filament_types) WHERE kind = 'model3d';
UPDATE mf_designs d SET
    formats = coalesce((SELECT array_agg(DISTINCT f.format) FROM mf_model_files f WHERE f.design_id = d.id), '{}'),
    printers = coalesce((SELECT array_agg(DISTINCT b) FROM mf_print_profiles p, unnest(p.printers) b WHERE p.design_id = d.id), '{}'),
    filament_types = coalesce((SELECT array_agg(DISTINCT x ->> 'type') FROM mf_print_profiles p, jsonb_array_elements(p.filaments) x WHERE p.design_id = d.id), '{}'),
    has_profile = EXISTS (SELECT 1 FROM mf_print_profiles p WHERE p.design_id = d.id AND p.readable),
    multicolour = EXISTS (SELECT 1 FROM mf_print_profiles p WHERE p.design_id = d.id
                          AND (SELECT count(DISTINCT x ->> 'colour') FROM jsonb_array_elements(p.filaments) x) > 1)
WHERE d.kind = 'model3d';
