-- MF-3: makers' public profiles, and what a design needs to be listed: its details, its
-- licence, when it was published, how often it was downloaded, and a search index.
CREATE TABLE mf_makers (
    account_id BIGINT PRIMARY KEY REFERENCES mf_accounts (account_id),
    -- the maker's address on the site, /makers/<handle>
    handle TEXT NOT NULL UNIQUE CHECK (handle ~ '^[a-z0-9][a-z0-9-]{2,29}$'),
    name TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 60),
    bio TEXT NOT NULL DEFAULT '' CHECK (length(bio) <= 1000),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

ALTER TABLE mf_designs
    ADD COLUMN title TEXT NOT NULL DEFAULT '' CHECK (length(title) <= 100),
    ADD COLUMN description TEXT NOT NULL DEFAULT '' CHECK (length(description) <= 5000),
    ADD COLUMN tags TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN licence TEXT,
    ADD COLUMN published_at TIMESTAMPTZ,
    ADD COLUMN downloads BIGINT NOT NULL DEFAULT 0,
    -- title, tags and description, weighted in that order; set whenever the details change
    ADD COLUMN search TSVECTOR;

ALTER TABLE mf_designs DROP CONSTRAINT mf_designs_status_check;
ALTER TABLE mf_designs ADD CONSTRAINT mf_designs_status_check CHECK (status IN ('draft', 'published'));

CREATE INDEX mf_designs_published ON mf_designs (published_at DESC) WHERE status = 'published';
CREATE INDEX mf_designs_search ON mf_designs USING GIN (search);
CREATE INDEX mf_designs_tags ON mf_designs USING GIN (tags);
