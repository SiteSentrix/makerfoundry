-- Picks of the week (2026-11-01): one design a week for each craft, shown on the home page and
-- kept for the past picks' page. An admin picks it on the design's page; when none has by the
-- time the home page first shows that week, the week's most liked and made design of that craft
-- is kept as its pick (`picked_by` NULL), so it stays the same all week.
CREATE TABLE mf_picks (
    week DATE NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('embroidery', 'laser', 'model3d')),
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    picked_by BIGINT,
    picked_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (week, kind)
);
CREATE INDEX mf_picks_design ON mf_picks (design_id);
