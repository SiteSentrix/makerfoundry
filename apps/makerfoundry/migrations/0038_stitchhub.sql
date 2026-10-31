-- MF-81: the StitchHub client's own records (issues #1 and #2). Messages to shops wait in the
-- outbox until the hub takes them, so an order change and its message are saved together and
-- survive a restart or the hub being down. Messages from shops are applied once (by id) and in
-- order (per order and sender, by seq), held while an earlier one hasn't come yet.
CREATE TABLE mf_hub_outbox (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    order_ref TEXT,
    seq BIGINT NOT NULL DEFAULT 0,
    to_party TEXT NOT NULL,
    envelope JSONB NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    attempts INT NOT NULL DEFAULT 0,
    next_try_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    failed TEXT
);
CREATE UNIQUE INDEX mf_hub_outbox_seq ON mf_hub_outbox (order_ref, to_party, seq) WHERE order_ref IS NOT NULL;
CREATE INDEX mf_hub_outbox_due ON mf_hub_outbox (next_try_at) WHERE sent_at IS NULL AND failed IS NULL;

-- Ids of messages already applied (kept 30 days), so the same message twice is applied once.
CREATE TABLE mf_hub_seen (
    id TEXT PRIMARY KEY,
    seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- The last seq applied, per order and sender.
CREATE TABLE mf_hub_seq (
    order_ref TEXT NOT NULL,
    from_party TEXT NOT NULL,
    last_seq BIGINT NOT NULL,
    PRIMARY KEY (order_ref, from_party)
);

-- Messages that came before an earlier one of their order.
CREATE TABLE mf_hub_held (
    id TEXT PRIMARY KEY,
    order_ref TEXT NOT NULL,
    from_party TEXT NOT NULL,
    seq BIGINT NOT NULL,
    envelope JSONB NOT NULL,
    held_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (order_ref, from_party, seq)
);

-- The client's state: the hub's cursor, whether this version is too old, deprecations.
CREATE TABLE mf_hub_state (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
