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
| Domains | Namecheap → Cloudflare DNS | makerfoundry.app bought 2026-10-19 and live (see `docs/deploy.md`); patternfoundry.app to be bought |

## Decided
- Name: **MakerFoundry**. Embroidery designs **and** laser-cut files.
- Free sharing and paid sales; MakerFoundry is the broker and takes a fee on sales.
- Rust, server-rendered, Postgres, Railway + Cloudflare, Stripe Connect, Resend (see the plan).
- **Fees** (2026-10-19): MakerFoundry keeps 10% of each sale; the card fee (2.9% + 30¢) also comes
  out of the seller's share; Stripe pays sellers monthly.
- **Ordering from shops** (2026-10-31): customers order physical goods (decorated garments and
  the like) from shops through MakerFoundry, live, never a demo; MakerFoundry takes the payment and
  keeps 10% (issue #2, `docs/stitchshop-ordering-audit.md`).
- **Stripe** (2026-10-20): MakerFoundry uses StitchFoundry's Stripe account, with Connect turned
  on, its own restricted key and its own webhooks (`docs/deploy.md` › Stripe).
- Same working rules as StitchFoundry (`CLAUDE.md`); commits authored as Bruce Wattendorf.
- Accounts: the StitchFoundry cloud service's accounts, shared (2026-10-19), called a **Foundry
  Account** on every page (owner's choice of name, 2026-10-19: not "StitchFoundry account"), the
  one sign-in and one credit balance ("Foundry credits") for all of SiteSentrix's software; design
  in the plan's "Accounts (MF-1)". StitchFoundry's app, docs, Terms and Privacy use the name too
  (StitchFoundry commit 53514ec14, 2026-10-19).
- **Hosting** (2026-10-19): Railway runs only the two servers, MakerFoundry and the shared
  StitchFoundry cloud service (cloud.stitchfoundry.app). The apps (StitchFoundry, LaserFoundry,
  PatternFoundry) stay on GitHub Pages and GitHub Releases, and the websites on Cloudflare Pages.
- **AI**: there is one Anthropic key, only in the cloud service's Railway variables. MakerFoundry
  never holds one: AI features, if it gets any, go to the cloud service's `/v1/messages` from the
  server, with the person's session token (which never reaches the browser).
- **Email**: the cloud service's `RESEND_API_KEY` sends both the password-reset emails (CLOUD-07)
  and StitchFoundry's sew-out reports (SO-02, which also need
  `SF_CLOUD_REPORTS_TO=support@stitchfoundry.app`). Both send from @stitchfoundry.app, so that
  domain must show Verified in Resend. MakerFoundry sends no email of its own.
- **Changing the cloud service** (github.com/Stitchwright/StitchFoundry, `apps/stitchfoundry-cloud`)
  is fine, as CLOUD-06 and CLOUD-07 did. Follow StitchFoundry's `CLAUDE.md` there: pull first (the
  StitchFoundry chat pushes to it too), `cargo xtask ci` green before every push, one task id per
  commit (`CLOUD-xx`), and update its `docs/cloud.md`.

## The owner decides (ask, don't assume)
Anything that costs money, takes payment, is legal wording, or goes public: the fee, who may sell,
sales tax, the DMCA agent registration, domain purchases, launching. See "Open decisions" in the
plan.

## First task
**MF-0**: a Cargo workspace with the workspace lints from StitchFoundry's `Cargo.toml` (the
no-panic clippy lints), a CI workflow (fmt, clippy -D warnings, tests), a tiny server with
`GET /health` and a home page, a Dockerfile, and the Railway steps written in `docs/deploy.md`
for the owner to follow.
