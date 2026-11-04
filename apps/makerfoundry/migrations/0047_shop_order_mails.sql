-- Issue #82: the customer is emailed when the shop reports a status on their order. One row per
-- `order.status` message applied (by the hub message's id, so a message applied twice is mailed
-- once), sent by the sweep (`sent_at`), tried a few times (`tries`), and forgotten after 30 days.
CREATE TABLE mf_shop_order_mails (
    message_id TEXT PRIMARY KEY,
    order_id BIGINT NOT NULL REFERENCES mf_shop_orders (id) ON DELETE CASCADE,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sent_at TIMESTAMPTZ,
    tries INT NOT NULL DEFAULT 0
);
CREATE INDEX mf_shop_order_mails_due ON mf_shop_order_mails (created_at) WHERE sent_at IS NULL;
