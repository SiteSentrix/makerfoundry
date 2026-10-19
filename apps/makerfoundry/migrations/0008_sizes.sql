-- MF-8: a design's size as columns (from what reading it found), so browsing can find designs
-- that fit a hoop; and a laser design's material and thickness, as its maker gives them.
ALTER TABLE mf_designs ADD COLUMN width_mm DOUBLE PRECISION;
ALTER TABLE mf_designs ADD COLUMN height_mm DOUBLE PRECISION;
UPDATE mf_designs SET width_mm = (facts ->> 'width_mm')::double precision,
                      height_mm = (facts ->> 'height_mm')::double precision
    WHERE facts ? 'width_mm' AND facts ? 'height_mm';
ALTER TABLE mf_designs ADD COLUMN material TEXT;
ALTER TABLE mf_designs ADD COLUMN thickness_mm DOUBLE PRECISION CHECK (thickness_mm > 0 AND thickness_mm <= 50);
