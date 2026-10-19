-- MF-8: a checkout that's never paid for expires (its Stripe page lasts 2 hours), so abandoned
-- orders don't wait for ever.
ALTER TABLE mf_orders DROP CONSTRAINT mf_orders_status_check;
ALTER TABLE mf_orders ADD CONSTRAINT mf_orders_status_check
    CHECK (status IN ('pending', 'paid', 'refunded', 'expired'));
CREATE INDEX mf_orders_pending ON mf_orders (created_at) WHERE status = 'pending';
