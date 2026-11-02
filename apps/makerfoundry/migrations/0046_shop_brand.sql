-- Branded shops (2026-11-02, the owner's decision): a shop's short address
-- (<subdomain>.makerfoundry.app, from its catalogue's shop.brand.handle, first come first served)
-- and its logo, fetched from the hub's files, checked and drawn again, then kept in storage.
ALTER TABLE mf_shops ADD COLUMN subdomain TEXT UNIQUE;
ALTER TABLE mf_shops ADD COLUMN logo_sha256 TEXT;
ALTER TABLE mf_shops ADD COLUMN logo_size BIGINT;
ALTER TABLE mf_shops ADD COLUMN logo_key TEXT;
ALTER TABLE mf_shops ADD COLUMN logo_type TEXT;
