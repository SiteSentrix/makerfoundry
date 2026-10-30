-- Issue #33: themed contests. Admins make a contest (its rules are the owner's text), open it,
-- and announce its winners after it closes; creators enter their published 3D models; members
-- vote when a contest is judged by votes (one vote a contest, changeable until it closes).
CREATE TABLE mf_contests (
    id BIGSERIAL PRIMARY KEY,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    theme TEXT NOT NULL,
    rules TEXT NOT NULL DEFAULT '',
    judging TEXT NOT NULL CHECK (judging IN ('admins', 'votes')),
    closes_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'draft' CHECK (status IN ('draft', 'open', 'announced')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE mf_contest_entries (
    contest_id BIGINT NOT NULL REFERENCES mf_contests (id) ON DELETE CASCADE,
    design_id BIGINT NOT NULL REFERENCES mf_designs (id) ON DELETE CASCADE,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    entered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    place INT CHECK (place BETWEEN 1 AND 3),
    PRIMARY KEY (contest_id, design_id)
);
CREATE TABLE mf_contest_votes (
    contest_id BIGINT NOT NULL,
    account_id BIGINT NOT NULL REFERENCES mf_accounts (account_id),
    design_id BIGINT NOT NULL,
    voted_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (contest_id, account_id),
    FOREIGN KEY (contest_id, design_id) REFERENCES mf_contest_entries (contest_id, design_id) ON DELETE CASCADE
);
