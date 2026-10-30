-- MF-42: a published 3D model's files as one ZIP, made on its first download and kept.
ALTER TABLE mf_designs ADD COLUMN zip_key TEXT;
