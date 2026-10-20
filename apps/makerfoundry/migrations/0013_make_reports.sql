-- MF-12: a make can be reported. Its report keeps the design, and the make while it's up.
ALTER TABLE mf_reports DROP CONSTRAINT mf_reports_kind_check;
ALTER TABLE mf_reports ADD CONSTRAINT mf_reports_kind_check CHECK (kind IN ('report', 'dmca', 'counter', 'make'));
ALTER TABLE mf_reports ADD COLUMN make_id BIGINT REFERENCES mf_makes (id) ON DELETE SET NULL;
