-- Issue #30: a post made through a platform's API keeps that platform's id for it, so a
-- withdrawal can delete it there.
ALTER TABLE mf_showcase_posts ADD COLUMN remote_id TEXT;
