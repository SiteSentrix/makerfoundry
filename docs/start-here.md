# Start here: notes for a new session

Written 2026-10-19 from the StitchFoundry session where MakerFoundry was decided. Read this, then
`docs/plan.md`.

## The owner
**Bruce Wattendorf**, doing business as **SiteSentrix** (a sole proprietorship/DBA in Rhode
Island; an LLC may come later). He runs StitchFoundry and is building a small family of maker
software. He works from a Windows PC (Claude Code installed, `C:\dev\…`) and the Claude app on his
phone. Plain, step-by-step instructions work best for anything he has to click through.

## What already exists
| Thing | Where | Notes |
|---|---|---|
| StitchFoundry app | github.com/Stitchwright/StitchFoundry | Rust, built on VectorCraft; embroidery digitizing; its `crates/sf-formats` and `crates/sf-core` read and draw embroidery files |
| StitchFoundry cloud service | cloud.stitchfoundry.app (Railway) | Rust (`apps/stitchfoundry-cloud`): accounts (Argon2id), credits, Stripe Checkout, AI through Anthropic, Postgres. A good model for MakerFoundry's accounts and Stripe code |
| Websites | stitchfoundry.app, sitesentrix.com | static pages on Cloudflare Pages from bwattendorf/sitesentrix-web (`stitchfoundry/`, `site/`) |
| Email | support@stitchfoundry.app, anything@sitesentrix.com | Cloudflare Email Routing to the owner's Gmail; Resend for sending |
| Legal | stitchfoundry.app/terms, /privacy | drafted, not lawyer-reviewed; MakerFoundry needs its own Terms, Privacy and Seller Terms |
| Hosting | Railway Pro ($20/mo) | other projects: StitchHub, stitchbench, stitchshop (demos for the StitchBench repair-shop software) |
| Payments | Stripe (owner's account) | StitchFoundry uses Checkout; MakerFoundry needs **Stripe Connect** |
| Domains | Namecheap → Cloudflare DNS | makerfoundry.app and patternfoundry.app to be bought |

## Decided
- Name: **MakerFoundry**. Embroidery designs **and** laser-cut files.
- Free sharing and paid sales; MakerFoundry is the broker and takes a fee on sales.
- Rust, server-rendered, Postgres, Railway + Cloudflare, Stripe Connect, Resend (see the plan).
- Same working rules as StitchFoundry (`CLAUDE.md`); commits authored as Bruce Wattendorf.
- Accounts: the **StitchFoundry cloud accounts**, shared (2026-10-19); design in the plan's
  "Accounts (MF-1)".

## The owner decides (ask, don't assume)
Anything that costs money, takes payment, is legal wording, or goes public: the fee, who may sell,
sales tax, the DMCA agent registration, domain purchases, launching. See "Open decisions" in the
plan.

## First task
**MF-0**: a Cargo workspace with the workspace lints from StitchFoundry's `Cargo.toml` (the
no-panic clippy lints), a CI workflow (fmt, clippy -D warnings, tests), a tiny server with
`GET /health` and a home page, a Dockerfile, and the Railway steps written in `docs/deploy.md`
for the owner to follow.
