# MakerFoundry

A marketplace where makers share and sell **machine-embroidery designs** and **laser-cut
files**: free designs shared openly, paid designs sold through MakerFoundry. By
[SiteSentrix](https://sitesentrix.com), the makers of [StitchFoundry](https://stitchfoundry.app).

## Status
October 2026: **MF-0** (workspace) done: a Rust web service with a home page and `/health`, CI,
and a Dockerfile, live on Railway at <https://makerfoundry-production.up.railway.app>
([`docs/deploy.md`](docs/deploy.md)). Next: **MF-1** (accounts, shared with StitchFoundry). See [`docs/plan.md`](docs/plan.md).

## Develop
- `cargo run -p makerfoundry`, then open <http://localhost:8080>.
- `cargo xtask ci`: the checks every commit must pass (fmt, clippy `-D warnings`, tests, assets).

## For contributors and agents
Start with [`CLAUDE.md`](CLAUDE.md) and [`docs/start-here.md`](docs/start-here.md).

## Licence
MIT OR Apache-2.0, at your choice ([`LICENSE-MIT`](LICENSE-MIT), [`LICENSE-APACHE`](LICENSE-APACHE)).
Designs uploaded by users stay theirs, under the licence chosen for each listing.
