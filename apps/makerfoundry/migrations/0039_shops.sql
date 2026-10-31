-- MF-62: shops that take orders through StitchHub, each with the catalogue it last published
-- (checked on arrival). A shop is offered to customers only while its Foundry Account can be paid.
CREATE TABLE mf_shops (
    party TEXT PRIMARY KEY,
    handle TEXT NOT NULL UNIQUE,
    account_id BIGINT NOT NULL,
    name TEXT NOT NULL,
    town TEXT NOT NULL DEFAULT '',
    ships BOOLEAN NOT NULL DEFAULT false,
    pickup BOOLEAN NOT NULL DEFAULT false,
    catalog JSONB NOT NULL,
    shipping_cents BIGINT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mf_shops_name ON mf_shops (name);
