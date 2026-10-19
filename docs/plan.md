# MakerFoundry: the plan

**Status:** 2026-10-19. MF-0 done: live at https://makerfoundry-production.up.railway.app.
MF-1 done and live (accounts through StitchFoundry's account service, which gained the account
id, CLOUD-06, and password reset by email, CLOUD-07). Reset emails go out once the account service
has its Resend key. Next: MF-2. Owner: Bruce Wattendorf (SiteSentrix).

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
Plain-language choices (final wording is the owner's):
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
| MF-2 | Upload and preview: embroidery and laser files read, checked and drawn; photos; R2 storage | an uploaded PES shows its stitches. **Embroidery done 2026-10-19**: `/preview` reads every StitchFoundry format (`sf-formats`, pinned by commit) and draws the stitches with size, stitch count and colours. **SVG, DXF and PDF done 2026-10-19**: drawn from their shapes in mm, with size, shapes, line length and colours (layers). Still to do: photos, and keeping files (R2) |
| MF-3 | Listings: design pages, browse, search, tags, seller pages; free downloads | a free design can be found and downloaded |
| MF-4 | Selling: Stripe Connect onboarding, checkout with the platform fee, library, sales page, refunds | a test-mode sale pays the seller less the fee |
| MF-5 | Safety: report a listing, DMCA page and process, admin queue, upload limits, rate limits | a takedown can be done in minutes |
| MF-6 | Launch: Terms, Privacy, Seller Terms, a landing page, makerfoundry.app live | public launch |
| MF-7 | StitchFoundry and LightFoundry integration | publish from the app |

## Open decisions (the owner's)
- **The platform fee**: a percentage of each sale (for example 10%), a minimum, or both.
- **Who may sell**: anyone, or approved sellers first.
- **Sales tax on digital goods**: some US states tax them and treat marketplaces as the seller
  ("marketplace facilitator" laws). Likely answer: Stripe Tax on checkout. Needs the owner's
  decision (and ideally an accountant's word) before paid sales go live.
- **DMCA designated agent**: register SiteSentrix with the US Copyright Office (a small online
  fee) before the public launch, for safe-harbour protection.
- ~~**Accounts**~~: **decided 2026-10-19: the StitchFoundry cloud accounts** (see "Accounts (MF-1)").
- **Domain**: makerfoundry.app (and patternfoundry.app held for the sewing-pattern app).
