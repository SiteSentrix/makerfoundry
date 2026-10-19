-- MF-9: whether an embroidery design is also offered converted to the formats its maker didn't
-- upload (on unless its maker turns it off).
ALTER TABLE mf_designs ADD COLUMN convert BOOLEAN NOT NULL DEFAULT true;
