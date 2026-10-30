# MakerFoundry: the plan

**Status:** 2026-10-29. MF-0 to MF-25, MF-34 and MF-54 built at https://makerfoundry.app (the table below);
the legal pages are approved (2026-10-22) for a free-only soft open (`docs/launch.md`: the contact
email, admins, then `MF_LAUNCHED=yes`; the DMCA agent the week after; Stripe keys and sales tax
before paid sales). MF-7's app side is StitchFoundry's
(`docs/api.md`). Next after the soft open: the Preview & Approval Hub, MF-25 to MF-33 (planned
2026-10-28, starting with approval links), and the 3D printing hub, MF-35 to MF-59 (planned
2026-10-30, `docs/3d-hub-plan.md`). Later, each needing the owner's decision: bundles,
discount codes, pay-what-you-want. Owner: Bruce Wattendorf (SiteSentrix).

## What it is
A place, like Thingiverse for 3D prints, where makers find, share and sell files for their
machines:

- **Machine embroidery designs**: DST, PES, PEC, JEF, EXP, VP3, XXX, HUS, U01 and more.
- **Laser-cut and engrave files**: SVG, DXF, PDF, and LightBurn-friendly exports.
- **Free designs** shared openly, and **paid designs** sold through MakerFoundry, which takes a
  small fee on each sale (the owner's income from the marketplace).

Who it is for: home embroiderers and small embroidery shops (StitchFoundry's users), laser hobby
users and small laser businesses (LightFoundry's), and the designers who sell to both.

## How a sale works (first version)
1. A seller connects a **Stripe Connect** account (Express): Stripe checks who they are and pays
   them out; MakerFoundry never holds their bank details.
2. A buyer pays on Stripe Checkout. MakerFoundry keeps its **platform fee** (an application fee
   on the payment); the rest goes to the seller. Stripe's own card fee comes out too.
3. Once Stripe confirms the payment (signed webhook), the buyer's library gets the files:
   download links that last, re-downloadable any time.
4. Refunds and disputes follow a written policy (digital goods: refund when a file is broken or
   isn't what the listing said).

## The first version (MVP)
- Accounts: the StitchFoundry cloud service's accounts, called a **Foundry Account** (owner's
  decisions, 2026-10-19): one email and password, and one credit balance, for every SiteSentrix
  app. See "Accounts (MF-1)" below.
- **Upload a design**: one or more files plus photos of the stitched or cut result, a title,
  description, tags, machine/hoop size or material and size, and a licence.
- **Previews made by the site**: embroidery files drawn as stitches (from StitchFoundry's own
  open-source file readers, used as a Rust dependency), with stitch count, size, colours and
  colour changes; laser files drawn from their paths, with their size. Uploaded files are checked
  as they're read: a file that doesn't read is refused.
- **Browse and search**: by kind (embroidery / laser), tags, hoop size, newest, popular, free/paid.
- **A design page**: previews, photos, details, the licence in plain words, download or buy.
- **Seller pages**: a maker's designs; followers later.
- **Paid designs** with Stripe Connect; a buyer's library; a seller's sales and payouts page.
- **Report a listing** and a DMCA takedown page; an admin queue to act on them.
- Terms of Service, Privacy Policy and Seller Terms pages (drafted like StitchFoundry's).

## Later
- "Open in StitchFoundry" on embroidery designs and "Publish to MakerFoundry" from the app;
  the same for LightFoundry.
- The Foundry Account across LightFoundry and PatternFoundry too.
- Collections, likes, comments, makes (photos of others' sew-outs and cuts), seller follows.
- Bundles, discount codes, "pay what you want".
- Format conversion on download (buy a PES, also get a DST), using StitchFoundry's writers.
- PatternFoundry (sewing patterns) may share the marketplace engine.

## Accounts (MF-1)
The owner chose StitchFoundry's cloud accounts (cloud.stitchfoundry.app) over separate ones.
How it works, given what that service has today (`apps/stitchfoundry-cloud`, `docs/cloud.md`):

- The cloud service owns accounts: emails, Argon2id password hashes, sessions, lock-outs. It keeps
  them in one record owned by its single process, so MakerFoundry **never touches its database**;
  it goes through its API only.
- **Sign up / sign in** on MakerFoundry's own pages: the form posts to MakerFoundry, which calls
  the cloud's `POST /v1/signup` or `/v1/signin` over HTTPS and gets the account's session token.
  The password passes through MakerFoundry's server but is never stored or logged.
- The cloud token can spend the account's AI credits, so it **never goes to the browser**.
  MakerFoundry keeps it server-side in its own Postgres, under a MakerFoundry session id that the
  browser holds in a `__Host-` cookie (HttpOnly, Secure, SameSite=Lax); form posts also check the
  `Origin` header. Signing out ends both sessions (`POST /v1/signout`).
- MakerFoundry's own tables (profile, shop name, later listings and sales) are keyed by the cloud
  account's **id**.
- **Changes needed in StitchFoundry's cloud service first:**
  1. `/v1/account` (and sign-in/up) also return the account's `id`.
  2. **Password reset by email** (Resend): request a link, a single-use token that expires in an
     hour (kept only as its hash), set a new password, end the account's other sessions. Both
     sites link to it.
- Pages call it a **Foundry Account** ("Sign in with your Foundry Account"): the one login for all
  of SiteSentrix's software (owner's decision, 2026-10-19; it replaces "StitchFoundry Account",
  decided earlier the same day). The account service itself is StitchFoundry's, worked on in the
  StitchFoundry repository.
- Accounts made on MakerFoundry get the cloud's usual sign-up credits (100 AI credits, within
  the free-offer budget), the same as accounts made in StitchFoundry (owner's decision, 2026-10-19).
- Later ("One SiteSentrix account" below): a sign-in page of the account service's own, so other
  sites never see passwords.

## Licences a listing can carry
Plain-language choices (final wording is the owner's; the draft wording is in
`apps/makerfoundry/src/licences.rs`, with "small business" at 200 items until the owner says):
- **Personal use**: make it for yourself and gifts.
- **Small business**: sell what you make with it (stitched or cut items), up to a number of items.
- **Commercial**: no limit on items made.
- Free designs may also be **CC BY**, **CC BY-NC** or **CC0**.
Nobody may resell or share the files themselves.

## Tech
- **Rust** web service (axum or similar), server-rendered HTML, Postgres.
- **Files** in object storage (Cloudflare R2: no download-bandwidth charges), served through
  signed, expiring links.
- **Hosting**: Railway (the owner's Pro plan), Cloudflare in front (DNS, TLS, caching).
- **Payments**: Stripe Connect. **Email**: Resend.
- Reuse StitchFoundry's crates for reading and drawing embroidery files (`sf-formats`,
  `sf-core`), as a git dependency pinned by commit.

## Milestones and task ids
| Id | What | Done when |
|---|---|---|
| MF-0 | Workspace: Cargo workspace, CI (fmt, clippy -D warnings, tests), `/health`, a hello page deployed to Railway | the page answers on a Railway address. **Done 2026-10-19**: https://makerfoundry-production.up.railway.app |
| MF-1 | Accounts: sign up, sign in, sessions, password reset by email | a person can make an account. **Done 2026-10-19**, live (`/signup`, `/signin`, `/forgot`, `/account`) with its Postgres on Railway; reset emails wait on the account service's `RESEND_API_KEY` |
| MF-2 | Upload and preview: embroidery and laser files read, checked and drawn; photos; R2 storage | an uploaded PES shows its stitches. **Embroidery done 2026-10-19**: `/preview` reads every StitchFoundry format (`sf-formats`, pinned by commit) and draws the stitches with size, stitch count and colours. **SVG, DXF and PDF done 2026-10-19**: drawn from their shapes in mm, with size, shapes, line length and colours (layers). **Photos and keeping files built 2026-10-19**: `/designs/new` keeps a design file and up to 6 photos (decoded and saved again as JPEG, metadata dropped) in R2, as a draft only its maker sees (`/designs`); live once R2's settings are in Railway (`docs/deploy.md`) |
| MF-3 | Listings: design pages, browse, search, tags, seller pages; free downloads | a free design can be found and downloaded. **Done 2026-10-19**: maker profiles (`/account/maker`, `/makers/<handle>`), design details and licence (`/designs/<id>/edit`), publish with the maker's word it's theirs to share, public pages (`/d/<id>`) with free counted downloads, browse and search (`/browse`: words, tag, kind, newest or most downloaded), newest on the home page. Publishing is open only to `MF_PUBLISHERS` until the launch (MF-5's takedown process first) |
| MF-4 | Selling: Stripe Connect onboarding, checkout with the platform fee, library, sales page, refunds | a test-mode sale pays the seller less the fee. **Built 2026-10-19** (waiting for the Stripe test keys to try it live): sellers set up a Stripe Express account paid monthly (`/account/selling`); a design gets a price ($1 to $200, or free); buyers pay on Stripe Checkout, a destination charge with an application fee of 10% + the card fee (2.9% + 30¢), all in integer cents (`money.rs`); the sale is delivered only when Stripe confirms it, by a signed, de-duplicated webhook (`/stripe/webhook`) or by asking Stripe when the buyer comes back; every sale and refund is three ledger rows (`mf_ledger`); the buyer's library (`/library`), the seller's sales (`/account/sales`), and full refunds by an admin (`/admin/orders`). Paid designs people bought can't be deleted. No sales tax yet (open decision) |
| MF-5 | Safety: report a listing, DMCA page and process, admin queue, upload limits, rate limits | a takedown can be done in minutes. **Built 2026-10-19**: Report this design (`/d/<id>/report`), the copyright policy (`/dmca`, linked from every page), notice and counter-notice forms (17 U.S.C. § 512(c)(3), (g)(3)), the admin queue (`/admin`, accounts in `MF_ADMINS`) with one-click take down, dismiss and restore, all logged, repeat takedowns counted; rate limits on sign-up, sign-in, reset emails, reports, previews and uploads, and 500 designs an account. The policy's wording is a draft for the owner; the designated agent (`MF_DMCA_AGENT`) once registered |
| MF-6 | Launch: Terms, Privacy, Seller Terms, a landing page, makerfoundry.app live | public launch. **Ready to launch, waiting on the owner (2026-10-19)**: `/terms`, `/privacy` and `/seller-terms` are drawn from `docs/legal/*.md` (`legal.rs`); while a document has the owner's markers it's a draft, seen only by admins. The launch switch `MF_LAUNCHED=yes` opens the site only once no markers are left: the legal pages go public, signing up asks for agreement to the Terms and the Privacy Policy (18 or older), publishing and setting up selling to the Seller Terms, each agreement kept with the document's version (`mf_agreements`); publishing opens to everyone; the home page invites people in; `robots.txt` lets search engines in and `/sitemap.xml` lists published designs and makers. Before the launch `robots.txt` keeps them out. The owner's steps: `docs/launch.md` |
| MF-7 | StitchFoundry and LightFoundry integration | publish from the app. **MakerFoundry's side built 2026-10-19**: the apps' API (`POST /api/v1/designs`, `docs/api.md`): an app sends a design with its Foundry Account sign-in (checked with the account service, never kept), it's read and kept as a draft exactly as an upload, and the app opens its page, where signing in comes back to the draft; browsers may call it from the web apps' sites (`MF_API_ORIGINS`). **Next, in the apps' repositories**: StitchFoundry's Publish to MakerFoundry command (the note in `docs/api.md`), then LightFoundry's; **"Open in StitchFoundry" built 2026-10-20**: free, published embroidery designs' pages link to StitchFoundry's web app (`?open=`, its OPEN-01), which fetches the file from `GET /api/v1/designs/{id}/file/{name}` (CORS; `https://stitchwright.github.io` added to the default `MF_API_ORIGINS`) |
| MF-8 | The first version's missing pieces: free/paid and hoop-size browsing, laser material and thickness, several files per design, abandoned checkouts expiring | everything in "The first version (MVP)" is built. **Done 2026-10-19**: free/paid filter on Browse; unpaid checkouts expire (Stripe's page lasts 2 hours, `checkout.session.expired`, and any order still unpaid a day on); embroidery designs found by the hoop they fit (4×4, 5×7, 6×10, 8×8, 8×12 sewing fields, either way round; `sizes.rs`) and shown as "fits a 4×4 in hoop"; laser designs get a material and thickness, shown and searchable; a design can have up to 11 files, one a format (`files.rs`, `mf_design_files`), each read and checked like the first and of the same kind, listed on its page as Formats and downloadable by whoever may download the design (a paid design's buyers get every format, and a sold design's formats can't be removed) |
| MF-9 | Format conversion on download: an embroidery design in every format StitchFoundry writes | buy a PES, also get a DST. **Done 2026-10-19**: a design's page offers the embroidery formats its maker didn't upload (DST, EXP, JEF, PEC, VP3, XXX, U01, PCS, TAP, DSB), each converted from the maker's file by `sf_formats::write` when downloaded (`convert.rs`, `/d/<id>/download?as=dst`), not stored; the same rules as the design's own files (free for anyone, paid for its buyers), counted as downloads, 120 an hour an address; on unless the maker unticks it in the details (Seller Terms § 9 says so, for the owner) |
| MF-10 | Saved designs and collections | people keep designs to come back to, and share lists. **Done 2026-10-19**: Save on a design's page to a private Saved list (made the first time) or to your own collections (`/collections`, up to 100, 1,000 designs each), private unless made public; public ones show on the owner's maker page; a design shows how many people saved it, and Browse can sort by Most saved; unpublished designs drop out of collections (`collections.rs`) |
| MF-11 | Makes: photos of what people made from a design | a design's page shows other people's results. **Done 2026-10-19**: anyone with a maker profile (and, until the launch, allowed to publish) posts a photo and a short note on a published design, with their word it's their own photo of their own make; the photo is cleaned like a design's; the design's page shows the newest 60 and a count; the poster, the design's maker and the admins can take one down; 10 a person a design (`makes.rs`). The Terms' wording about makes is the owner's (`docs/legal/README.md`) |
| MF-12 | Reporting a make | a make that breaks the rules can be flagged and taken down from the admin queue. **Done 2026-10-20**: Report on each make (`/makes/<id>/report`: not theirs, offensive, spam, something else), shown in `/admin` with the photo and Take the make down; the removal is logged; a report of a make already gone is just closed |
| MF-13 | Following makers | people see new designs from the makers they like. **Done 2026-10-20**: Follow on a maker's page (with its follower count; who follows whom isn't shown), `/following` lists the newest 60 designs of the makers you follow, up to 1,000 follows (`follows.rs`) |
| MF-14 | Comments on designs | buyers ask makers questions and share tips under a design. **Done 2026-10-20**: comments under a published design, oldest first, the maker's marked; a maker profile needed (and, until the launch, being allowed to publish); plain text only, nothing becomes a link; 20 an hour; the author, the design's maker and the admins take one down; Report on each, with Take the comment down in `/admin` (`comments.rs`) |
| MF-15 | Your data: a copy and deleting it | what the Privacy Policy promises, done by the person. **Done 2026-10-20**: `/account/data` downloads everything kept about the account as JSON, and deletes it (typing delete to confirm): profile, designs, files and photos, makes, comments, collections, follows and sign-ins, in one transaction, then the files; kept: purchases, sales, ledger, agreements, reports and notices, and bought designs (unpublished, for their buyers) (`data.rs`) |
| MF-16 | The admins' overview | the owner sees the site's numbers at a glance. **Done 2026-10-20**: `/admin/overview` (admins only): accounts, maker profiles, sellers ready, follows; published designs by kind, paid, drafts, taken down, downloads, makes, comments; sales, refunds, and money from the ledger (paid by buyers, to sellers, MakerFoundry's fees, card fees, fees this month); what's waiting in the report queue (`overview.rs`) |
| MF-17 | Open in StitchFoundry for buyers of paid designs | buyers open what they bought in the web app too. **Done 2026-10-20**: a paid embroidery design's page shows Open in StitchFoundry to its buyers, maker and admins; `/d/<id>/open` checks that, makes a link to that design's file good for ten minutes (kept as its hash, `mf_open_links`), and sends them to the web app with it; the file address takes the link for paid designs and refuses them without one (`api.rs`) |
| MF-18 | Open in LaserFoundry: MakerFoundry's side | laser designs open in LaserFoundry's web app. **MakerFoundry's side built 2026-10-20**: SVG and DXF laser files are served at the same file address as embroidery (free for anyone, paid with a buyer's link), and the button (\"Open in LaserFoundry\") is ready but off (`LASERFOUNDRY_OPENS_LINKS`) until LaserFoundry's web app opens `?open=` links and allows makerfoundry.app in its content security policy: the note for its chat is in `docs/api.md` |
| MF-19 | Link previews | a shared design shows its title, words and photo in chats and on social sites. **Done 2026-10-21**: published designs' and makers' pages carry the standard Open Graph tags (title, a line of the description, the page's address, the first photo) and a canonical address; drafts carry none (`pages::page_with`) |
| MF-20 | Property tests of untrusted input | every reader of what people send is tested against random and damaged input, as StitchFoundry's are. **Done 2026-10-21**: `tests/untrusted_input.rs` (proptest) gives the embroidery, SVG, DXF and PDF readers, photos, typed prices and thicknesses, and Stripe's signature header random bytes, damaged copies of good files and generated content (SVG elements, DXF group codes, PDF drawing operators with extreme numbers); each must answer without panicking, and what it accepts must be within the limits (sizes, stitch and shape counts, a preview of numbers and colours only); accepted embroidery converts to every format. It found one bug, fixed: a signature header with a time far in the past overflowed the check of its age (`stripe.rs`). 128 cases a test in CI; more with `PROPTEST_CASES` **2026-10-30**: they found an SVG whose huge arcs (radius 1e30) made usvg allocate gigabytes before our limits were checked; SVGs with a drawing number beyond ±1e7 are now refused before usvg reads them (`laser::MAX_SVG_NUMBER`, `docs/debug-logs/2026-10-30-svg-arc-memory.md`) |
| MF-21 | Signing everyone out after a breach | a session copied from the database stops working. **Done 2026-10-22**: `MF_END_SESSIONS_BEFORE` (a UTC time, a date or Unix seconds) deletes the sessions made before it when the service starts; `docs/deploy.md` › After a breach lists every secret the service holds and where each is renewed |
| MF-22 | Activity | makers learn what people did with their designs without email. **Done 2026-10-22**: `/activity` lists, newest first, the comments and makes on a maker's designs by other people (who, the design, a line of what they wrote), their sales and refunds (what they got or gave back) and new followers (not named); what came since they last looked is marked new, and the account page counts it. Drawn from those tables; only when they last looked is kept (`mf_activity_seen`, in the data download and deleted with the data) (`activity.rs`) |
| MF-23 | Refunds: the seller bears the card fee | a refunded seller is out only the card fee Stripe keeps (the owner's decision, Seller Terms § 7). **Done 2026-10-22**: an admin's refund gives the buyer the whole price back and reverses the seller's transfer, without the application fee; MakerFoundry then gives its 10% back to the seller as a partial refund of that fee (`/v1/application_fees/{id}/refunds`). In the ledger the refund takes the price from the seller (`refund`) and the 10% goes back (`fee_return`); the card fee row stays. If Stripe can't give the fee back at once, `/admin/orders` offers Give the fee back (`fee_returned_at`). Activity and the sales page show what the seller lost. The restricted key needs three more permissions (`docs/deploy.md`) |
| MF-24 | Sales tax with Stripe Tax | the buyer pays the price plus the tax, which is MakerFoundry's, not the seller's. **Built 2026-10-27, off until `MF_STRIPE_TAX=yes`**: checkouts ask Stripe Tax for it (`automatic_tax`, MakerFoundry liable, billing address required, prices exclusive, tax code `txcd_10505001` or `MF_TAX_CODE`); a paid session is believed only if its subtotal is the price and its total the price plus the tax; the tax is taken back from the seller's transfer by a reversal once paid (webhook, return page, sweep; checked with Stripe so it's never taken twice); `tax` ledger rows; a taxed refund takes the price back from the seller first, then refunds the buyer everything; the buyer's library shows the tax, the admins' overview the tax collected (`selling.rs`, `docs/deploy.md` › Sales tax) |
| MF-25 | Approval links (the Preview & Approval Hub, first slice) | a shop sends a customer a private link to a design and the customer approves it. **Built 2026-10-29; its wording approved and in the Terms (§ 9) and the Privacy Policy 2026-10-30.** Admins only until the owner sets `MF_PROOF_KEY` and `MF_PROOFS=yes`: any Foundry shop (any Foundry Account; owner, 2026-10-29) sends a proof from `/proofs/new`: a design file, read and checked as an upload and drawn from what was read, with mock-up photos cleaned as a design's are, its shop name, who it's for (the shop's own note) and a message. The customer's link `/p/<id>/<token>` is an HMAC of the proof's id and link generation with `MF_PROOF_KEY`, so nothing in the database opens it; the shop sees it any time, can make a new one (the old stops working), turn it off, or delete the proof. The customer, with no account, sees the latest version (earlier ones too), comments, approves it or asks for changes under the name they type (a newer version refuses answers to the old one); the page isn't cached, indexed or passed on as a referrer. The shop sees sent / viewed / approved / changes asked for, replies, and sends new versions (up to 30). Who approved which version and when is kept. Proofs are deleted a year after their last activity (owner, 2026-10-29; the 10-minute sweep), are in the data download and go with the account's data. 500 proofs a shop, 100 proofs or versions a day, 30 answers an hour from one address (`proofs.rs`) |
| MF-26 | Product-photo previews | the design drawn onto a product photo on the server, added to a proof. **Planned**: needs MF-25, the stock photos (owner: own photos or CC0, in `ASSETS.md`), the StitchShop prototype's location and licence, a raster library with an allowed licence; placing by typed size and position unless the owner agrees to a little JavaScript for dragging |
| MF-27 | Send to customer from the apps | `POST /api/v1/proofs` and its status, with the Foundry Account token as MF-7. **Planned**: needs MF-25; the button and `stitchfoundry://` links are StitchFoundry's |
| MF-28 | Shop branding and subdomains | a shop's logo, colours and `<shop>.makerfoundry.app` (one Railway wildcard domain); sign-in cookies stay on makerfoundry.app. **Planned**: needs MF-25 |
| MF-29 | Shop plans and the For Shops page | monthly plans on Stripe, tier limits, the demo shop. **Planned**: needs MF-28 and the owner's prices |
| MF-30 | StitchShop order handoff | an approved proof becomes one StitchShop order. **Planned**: needs MF-25 and a StitchShop order API (owner: product or demo) |
| MF-31 | More tools | previews and sending for ScreenFoundry (the owner's screen printing, DTG and sublimation software: previews show ink on fabric, not stitches), PatternFoundry and LightFoundry (laser cuts and engravings; owner, 2026-10-28). **Planned**: needs MF-26, MF-27 and those apps |
| MF-32 | Custom domains | a Top-tier shop's own domain through Cloudflare for SaaS (100 hostnames included, then $0.10 each). **Planned**: needs MF-29 |
| MF-33 | Live camera preview | the design on the real garment through the phone camera. **Planned**: needs MF-26 and the owner's agreement to the JavaScript it needs |
| MF-34 | Report a bug, suggest a feature | at the foot of every page, two buttons open an email to the owner with "[MakerFoundry] Bug report: " or "[MakerFoundry] Feature request: " as the subject and a short template. **Done** (2026-10-30) |
| MF-54 | Founding creators | MakerFoundry's first 100 creators, with a numbered badge. **Built 2026-10-30** (the owner's decisions: 100 spots, makers apply and an admin approves, a numbered badge): `/founding` explains it and shows the spots left (it works before the launch, with a link preview, for the Facebook post); signed-in makers apply (the name to know them by, an optional link to their work, what they make), one application an account; admins approve or decline at `/admin/founding`, approving giving the lowest free number up to 100 (never more, approvals one at a time); "Founding creator #7" on their maker page, their designs' pages and their account page; removing a badge frees its number; applications in the data download and deleted with the data (`founding.rs`). Early access to MakerFoundry 3D comes with the 3D hub (#22) |

## The 3D printing hub (MF-35 onwards, planned 2026-10-30)
A section for 3D-printing models (`/3d`), built on the same engine as the designs: model pages
with several files and images, server-drawn thumbnails, Bambu Studio/OrcaSlicer print profiles
(PrusaSlicer and Cura at a basic level), remixes with their licences enforced, likes, makes,
collections, follows, comments, search with print-profile filters, moderation and DMCA, a bulk
import of a creator's own files (MakerWorld, Printables, Thingiverse), founding-creator invites,
an opt-in social showcase, and Get this printed. The comparison with Thingiverse and Printables is
`docs/3d-hub-feature-matrix.md`; the plan, its data model and the owner's decisions are
`docs/3d-hub-plan.md`; the work is the GitHub issues labelled `3d-hub` (P1 `claude-queue`, in
order; P2 and P3 `claude-proposed`). Off until `MF_3D=yes`; selling, approval links and the
StitchShop ordering work stay as they are.

Progress (test-first, from `docs/3d-hub-test-plan.md`):
- **MF-35 foundation, built 2026-10-30** (#3): designs may be `model3d` (migration 0022: a
  category, print settings, and where an imported model was first posted); the 14 categories
  (`model3d/mod.rs`); 15 more licences for 3D models only (CC BY-SA, BY-NC-SA, BY-ND, BY-NC-ND,
  GPL 2/3 or later, LGPL 2.1/3 or later, BSD 3-Clause, Apache 2.0, MIT, CERN-OHL-S/W/P 2.0, all
  rights reserved), each linked to its official text, never offered on an embroidery or laser
  design (`licences::for_kind`, `find_for`); `/3d` for admins until `MF_3D=yes`, kept out of
  robots and the sitemap until then.

## Open decisions (the owner's)
- ~~**The platform fee**~~: **decided 2026-10-19: 10% of each sale, the card fee (2.9% + 30¢)
  also from the seller's share, payouts monthly** (research: `docs/research/marketplace-fees.md`).
- ~~**The price range**~~: **decided 2026-10-22: $1 to $200.**
- ~~**Refunds and the card fee**~~: **decided 2026-10-22: the seller gets the 10% fee back and
  bears the card fee Stripe keeps** (Seller Terms § 7; built as MF-23).
- ~~**Who may sell**~~: **decided 2026-10-22: anyone with an account (Stripe checks identity).**
- **Sales tax on digital goods**: some US states tax them and treat marketplaces as the seller
  ("marketplace facilitator" laws). Likely answer: Stripe Tax on checkout. Needs the owner's
  decision (and ideally an accountant's word) before paid sales go live. **The launch is free-only
  (owner, 2026-10-22)**: Seller Terms § 5 says paid sales aren't open yet until this is decided.
  **The code is ready (MF-24, 2026-10-27)**: Stripe Tax on checkout, switched on with
  `MF_STRIPE_TAX=yes` once the owner has registered and updated Seller Terms § 5.
- **DMCA designated agent**: register SiteSentrix with the US Copyright Office (a small online
  fee) before the public launch, for safe-harbour protection.
- ~~**Accounts**~~: **decided 2026-10-19: the StitchFoundry cloud accounts** (see "Accounts (MF-1)").
- **Domain**: makerfoundry.app (and patternfoundry.app held for the sewing-pattern app).
