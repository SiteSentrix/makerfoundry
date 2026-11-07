# Progress

- 2026-11-02 nightly maintenance: queue empty; `cargo xtask ci` green and stable over 3 test runs.
- Added money.rs edge-case tests (out-of-range prices, half-up rounding, i64 extremes, dollars round-trip); tests only.
- 2026-11-01 nightly maintenance: queue empty; CI and 3 test re-runs green, no flaky tests.
- Added edge-case tests for webhook signature headers (stripe.rs) and money bounds (money.rs); no behaviour change.
- 2026-11-05 #84: AGENTS.md (CLAUDE.md now imports it), PR template and label list added. `.claude` hooks + keep-going not done: the session's auto-mode classifier refused to write them (self-modification); needs Bruce to add or approve.
- 2026-11-07 #111 (first slice): search box and Upload button in every header, kind chips, home shelves of design cards (Trending, Newest, Free, From founding creators) with a founding invitation while there are fewer than 8 designs; cards show alt text, kind, founding badge, price.
