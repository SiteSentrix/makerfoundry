-- MF-68: the app a design was sent from (`app` in POST /api/v1/designs, its name without the
-- version: "ScreenFoundry"), so admins can find designs by the app that made them. NULL for
-- designs uploaded on the site.
ALTER TABLE mf_designs ADD COLUMN from_app TEXT CHECK (length(from_app) BETWEEN 1 AND 40);
CREATE INDEX mf_designs_from_app ON mf_designs (from_app, created_at DESC) WHERE from_app IS NOT NULL;
