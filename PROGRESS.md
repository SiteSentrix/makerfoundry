# Progress

- 2026-11-02 nightly maintenance: queue empty; `cargo xtask ci` green and stable over 3 test runs.
- Added money.rs edge-case tests (out-of-range prices, half-up rounding, i64 extremes, dollars round-trip); tests only.
- 2026-11-01 nightly maintenance: queue empty; CI and 3 test re-runs green, no flaky tests.
- Added edge-case tests for webhook signature headers (stripe.rs) and money bounds (money.rs); no behaviour change.
