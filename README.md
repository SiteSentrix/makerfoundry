# MakerFoundry

A marketplace where makers share and sell **machine-embroidery designs** and **laser-cut
files**: free designs shared openly, paid designs sold through MakerFoundry. By
[SiteSentrix](https://sitesentrix.com), the makers of [StitchFoundry](https://stitchfoundry.app).

## Status
October 2026: **MF-0** (workspace) done: a Rust web service with a home page and `/health`, CI,
and a Dockerfile, live on Railway at <https://makerfoundry-production.up.railway.app>
([`docs/deploy.md`](docs/deploy.md)). **MF-1** (accounts) done and live: sign up, in and out with a Foundry Account, and a
forgotten password. **MF-2** (upload and preview) under way: an
uploaded embroidery file is drawn stitch by stitch, and an SVG, DXF or PDF laser file line by line
(`/preview`, signed in); a maker keeps designs with photos as drafts (`/designs`) 
in Cloudflare R2. **MF-3** (listings) built: maker pages, design details and licences,
publishing, public design pages with free downloads, browse and search; publishing opens at the
launch. **MF-4** (selling) built: sellers set up Stripe payouts (monthly), designs get a price,
buyers pay on Stripe Checkout and find their designs in a library; MakerFoundry keeps 10% and the
card fee, every sale and refund in a ledger; waiting for the Stripe test keys. **MF-5** (safety) built: reporting, the copyright (DMCA) process, an admin queue for
takedowns, and rate limits. **MF-6** (launch) ready: the Terms, Privacy Policy and Seller Terms
pages, agreement on sign-up, publishing and selling, a launch switch that waits for the owner's
approval of the legal wording ([`docs/launch.md`](docs/launch.md)). **MF-7** (apps) under way: the API the apps
publish to ([`docs/api.md`](docs/api.md)); the apps' own command is next. **MF-8** (the first version's missing pieces) done:
free/paid, hoop and material browsing, and several formats per design. **MF-9** done: embroidery designs download converted
to every format StitchFoundry writes. **MF-10** done: saved designs and collections. See [`docs/plan.md`](docs/plan.md).

## Develop
- `cargo run -p makerfoundry`, then open <http://localhost:8080>.
- `cargo xtask ci`: the checks every commit must pass (fmt, clippy `-D warnings`, tests, assets).
- The database tests need a Postgres they may make schemas in:
  `MF_TEST_DATABASE_URL=postgres://user:password@localhost/db cargo xtask ci` (CI has one; without
  it those tests are skipped).
- Signing in locally needs `DATABASE_URL` and, for a test account service,
  `STITCHFOUNDRY_CLOUD_URL` (see `docs/deploy.md`).

## For contributors and agents
Start with [`CLAUDE.md`](CLAUDE.md) and [`docs/start-here.md`](docs/start-here.md).

## Licence
MIT OR Apache-2.0, at your choice ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)).
Designs uploaded by users stay theirs, under the licence chosen for each listing.
