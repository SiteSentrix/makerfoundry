-- MF-35: MakerFoundry 3D. A design may now be a printable model ('model3d'), with a category
-- (src/model3d: CATEGORIES), the print settings read from its 3MF (MF-43/44, as JSON), and, for a
-- model a maker imported from their own account elsewhere (MF-52/53), where it was first posted.
ALTER TABLE mf_designs DROP CONSTRAINT mf_designs_kind_check;
ALTER TABLE mf_designs ADD CONSTRAINT mf_designs_kind_check
    CHECK (kind IN ('embroidery', 'laser', 'model3d'));
ALTER TABLE mf_designs ADD COLUMN category TEXT;
ALTER TABLE mf_designs ADD COLUMN print_settings JSONB;
ALTER TABLE mf_designs ADD COLUMN original_site TEXT;
ALTER TABLE mf_designs ADD COLUMN original_url TEXT;
ALTER TABLE mf_designs ADD COLUMN original_published_at DATE;
CREATE INDEX mf_designs_category ON mf_designs (kind, category) WHERE status = 'published';
