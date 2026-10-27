# Deploying MakerFoundry on Railway

MakerFoundry runs on Railway (the owner's Pro plan) as one service built from this repository's
`Dockerfile`. `railway.toml` tells Railway to build with that Dockerfile and to check `/health`
before switching traffic to a new deploy. Nothing secret is in the repository: settings are
Railway **Variables**.

## Where it runs
- Railway project **MakerFoundry**, environment `production`, service `makerfoundry`, deploying
  `main` of `SiteSentrix/makerfoundry`.
- Address: <https://makerfoundry-production.up.railway.app> (first deployed 2026-10-19).
- `/health` answers `ok <version> <commit>` (the commit from Railway's `RAILWAY_GIT_COMMIT_SHA`),
  so `curl https://makerfoundry.app/health` shows which commit is live.
- MakerFoundry has no code signing (a server built by Docker on Railway: no GitHub `environment:`,
  no AZURE_* secrets); the desktop apps sign in their own repository's `release` environment.
- Service `Postgres` (image `postgres:16`, a volume at `/var/lib/postgresql/data`, made
  2026-10-19 for MF-1); the site reads it through `DATABASE_URL` = `${{Postgres.DATABASE_URL}}`
  over Railway's private network. Its password is a Railway variable of that service, generated
  when it was made and never written anywhere else. **Backups:** turn them on in the Postgres
  service's settings (Railway › Postgres › Backups).
- **makerfoundry.app** (bought 2026-10-19; DNS on Cloudflare) and **www.makerfoundry.app** were
  added to the service on 2026-10-19. They answer once these records are in Cloudflare's DNS for
  makerfoundry.app (each **DNS only**, the grey cloud, so Railway can issue the certificate):

  | Type | Name | Content |
  |---|---|---|
  | CNAME | `@` | `ia5fx6io.up.railway.app` |
  | TXT | `_railway-verify` | `railway-verify=2a9a13d59ed3a8efdd77e42fe0504484a5402ea13716bb305a6e7a39d2e9a7b0` |
  | CNAME | `www` | `or0wbw4h.up.railway.app` |
  | TXT | `_railway-verify.www` | `railway-verify=b0e0b032fbbc0ec44f91abba6e8d2e3e05e1528afb7d8118278525fa990049cb` |

- Claude sessions reach this one project through a Railway **project token** in the cloud
  environment's `RAILWAY_TOKEN` variable (Railway's GraphQL API, header `Project-Access-Token`).
  The token can't see the owner's other projects. To revoke it: the project's **Settings → Tokens**.

## First deploy (MF-0)

Done on 2026-10-19 through the API with the project token (the service, `PORT`, the domain). The
steps below are the same thing by hand, for a new project or if it ever needs redoing.

Do these once, in a web browser (the phone works too, but a PC is easier).

1. Go to <https://railway.com> and sign in.
2. Click **New** (top right of the dashboard), then **Project**, then **Deploy from GitHub repo**.
   *Or*, to keep it with your other projects, open an existing project and click **Create** →
   **GitHub Repo**. A project of its own is tidier: call it `makerfoundry`.
3. Pick **sitesentrix/makerfoundry**.
   - If it isn't in the list: click **Configure GitHub App**, choose the **sitesentrix**
     account/organisation, allow access to the `makerfoundry` repository, save, and come back.
4. Railway starts building straight away. The first build takes about 3–5 minutes (it compiles
   the Rust code). Wait for the deploy to show **Active** (green).
5. Click the new service (the box named `makerfoundry`), open the **Variables** tab, click
   **New Variable**, and add:
   - `PORT` = `8080`

   Railway redeploys by itself after a variable changes.
6. Open the **Settings** tab, scroll to **Networking**, and click **Generate Domain**. If it asks
   for a port, type `8080`. You get an address like `makerfoundry-production-xxxx.up.railway.app`.
7. Open `https://<that address>/health` in the browser. It should show `ok 0.1.0`.
   Then open `https://<that address>/` to see the home page.
8. Still in **Settings**, under **Deploy** (or **Source**), turn on **Wait for CI**, so a commit
   only deploys after GitHub's checks pass. Leave the branch as `main`.

Then send the `*.up.railway.app` address back in the chat: MF-0 is done when that page answers.

## After that
- Every push to `main` that passes CI deploys by itself. A failed health check keeps the old
  version running.
- **Logs**: the service's **Deployments** tab → the deploy → **View logs**.
- **Roll back**: the **Deployments** tab → an earlier deploy → **⋮** → **Redeploy**.

## Coming in later tasks
- **MF-1** (accounts): a Postgres database in the same project (**Create** → **Database** →
  **PostgreSQL**), and on the `makerfoundry` service the variable
  `DATABASE_URL` = `${{Postgres.DATABASE_URL}}`. Accounts and their emails (password resets) are
  StitchFoundry's account service's, so MakerFoundry needs no email key of its own.
- **MF-2**: Cloudflare R2 keys for file storage.
- **MF-6**: the `makerfoundry.app` domain (the owner buys it) pointed at the service through
  Cloudflare.

## Stripe (selling)
MakerFoundry uses **StitchFoundry's Stripe account** (owner's decision, 2026-10-20): one set of
books and one name on card statements for both. What keeps the two apart:

- **Its own restricted key**, not StitchFoundry's secret key. In the Stripe Dashboard (test mode
  first): **Developers › API keys › Create restricted key**, named `makerfoundry`, with only these
  permissions, everything else **None**:
  - **Checkout Sessions:** Write (makes and reads a buyer's checkout page)
  - **Refunds:** Write (an admin's full refund)
  - **PaymentIntents:** Read and **Charges:** Read (a refunded sale's application fee)
  - **Connect › Application Fees:** Write (MakerFoundry's 10% given back to the seller on a refund)
  - **Connect › Accounts:** Write (sellers' Express accounts, their set-up and dashboard links)
  - **Connect › Transfers:** Write (only with sales tax on, MF-24: the tax taken back from the
    seller's transfer, and a taxed sale's price before its refund)

  It goes in `STRIPE_SECRET_KEY` (`rk_test_…`, later `rk_live_…`). If it ever leaked, it couldn't
  touch StitchFoundry's products, prices or payouts settings, and it can be rolled on its own.
- **Its own webhook endpoints**, each with its own signing secret, at
  `https://makerfoundry.app/stripe/webhook` (below). StitchFoundry's endpoint stays as it is.
- **Each side ignores the other's payments.** Both endpoints receive every event of the account.
  MakerFoundry labels its checkouts `metadata[app]=makerfoundry` and acts only on those, and only
  for an order of its own at its amount; StitchFoundry adds credits only for checkouts carrying its
  `account` and `credits` labels, which MakerFoundry's never have. Tested both ways
  (`a_shared_stripe_accounts_other_payments_are_left_alone`).
- **Connect** must be on in the account: Marketplace, **Express** accounts, US; the platform's
  name and icon are what sellers see while setting up.
- Everything travels over HTTPS; the keys and secrets live only in Railway's variables.

## Sales tax (MF-24)
Off until `MF_STRIPE_TAX=yes`. MakerFoundry is the marketplace, so it collects and pays the tax
(https://docs.stripe.com/tax/tax-for-marketplaces), never the seller:
1. **Stripe Tax on** in the Stripe Dashboard (**Tax › Get started**): the business address (the
   origin), the default product tax code, and prices **exclusive** of tax.
2. **Registrations** (**Tax › Registrations**): add each state MakerFoundry is registered in with
   the state itself (Rhode Island first). Stripe only collects where a registration is active, and
   its monitoring says when sales in another state come near that state's threshold.
3. **Seller Terms § 5** says how tax works, in place of "Paid sales aren't open yet" (the owner's
   wording, ideally with an accountant's word), before `MF_STRIPE_TAX=yes`.
4. **The restricted key** gets **Connect › Transfers: Write** (above).

What it does: the checkout asks Stripe Tax for the tax (`automatic_tax`, MakerFoundry liable,
the buyer's billing address required, prices exclusive of tax); the buyer pays the price plus the
tax, and the seller's share is worked out from the price alone. A destination charge moves the
whole payment to the seller's account, so once the sale is paid the tax is taken back with a
transfer reversal (from the webhook, the buyer's return, and the 10-minute sweep until it's done;
checked with Stripe first, so never twice). The ledger keeps the tax as MakerFoundry's (`tax`
rows); a refund gives the buyer everything back, the tax too, taking the price back from the
seller first. Stripe records the tax for its reports; filing it with the states is the owner's
(or Stripe's filing add-on).

## Settings the service reads
| Variable | Needed | What |
|---|---|---|
| `PORT` | no (default `8080`) | the port to listen on; Railway sets it |
| `DATABASE_URL` | for accounts | MakerFoundry's Postgres, `${{Postgres.DATABASE_URL}}` on Railway; tables are made when the service starts. Without it the site runs but nobody can sign in |
| `R2_ACCOUNT_ID`, `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`, `R2_BUCKET` | to keep designs | Cloudflare R2: the account's id, an API token's keys (Object Read & Write on the bucket only), and the bucket (`makerfoundry-files`). Without them designs can be previewed but not kept |
| `MF_ADMINS` | for the report queue | account ids apart by commas: who sees `/admin` and can take designs down |
| `MF_DMCA_AGENT` | before launch | the designated agent registered with the US Copyright Office, as shown on `/dmca`: lines apart by `|` (name, address, email, phone) |
| `MF_API_ORIGINS` | no (default `https://stitchfoundry.app`) | the web apps' sites that may call the apps' API from a browser (`docs/api.md`), apart by commas |
| `MF_LAUNCHED` | at the launch | `yes` opens MakerFoundry to everyone (`docs/launch.md`), but only once the legal pages have none of the owner's markers left; until then the log says what's left |
| `MF_PUBLISHERS` | no (nobody; everyone once launched) | who may publish designs until the launch: account ids apart by commas, or `*` for everyone |
| `STRIPE_SECRET_KEY` | to sell | MakerFoundry's restricted key in StitchFoundry's Stripe account (`rk_test_…` until launch; see "Stripe" above). Without it every design is free |
| `STRIPE_WEBHOOK_SECRET` | with the key | the signing secret (`whsec_…`) of the webhook endpoint `https://makerfoundry.app/stripe/webhook` listening to **your account's** events `checkout.session.completed`, `checkout.session.async_payment_succeeded`, `checkout.session.expired`, `charge.refunded` |
| `STRIPE_CONNECT_WEBHOOK_SECRET` | with the key | the signing secret of a second endpoint at the same address listening to **connected accounts'** event `account.updated` (sellers' set-up finishing) |
| `MF_STRIPE_TAX` | no (off) | `yes` charges sales tax with Stripe Tax (MF-24, "Sales tax" below) |
| `MF_TAX_CODE` | no (`txcd_10505001`) | the Stripe tax code designs are sold under: Digital Finished Artwork, downloaded, with permanent rights |
| `MF_PUBLIC_URL` | no (default `https://makerfoundry.app`) | the site's address, for the links Stripe sends people back to and the sitemap |
| `STRIPE_API_URL` | no | a stand-in for Stripe's API (local tests) |
| `STORAGE_ENDPOINT` | no | an S3-compatible address in place of R2's (local tests) |
| `STITCHFOUNDRY_CLOUD_URL` | no (default `https://cloud.stitchfoundry.app`) | StitchFoundry's account service, which MakerFoundry's accounts are |
| `MF_END_SESSIONS_BEFORE` | after a breach | sessions made before this UTC time (`2026-10-22T09:00:00Z`, a date, or Unix seconds) end when the service starts: everyone signs in again (see below) |

## After a breach

If Railway, a key or the database may have been exposed, give every secret a new value and sign
everyone out. For each: make the new one, put it in the Railway variable, let the service
redeploy, then revoke the old one.

1. `STRIPE_SECRET_KEY`: the restricted key rolled in Stripe (Developers › API keys).
2. `STRIPE_WEBHOOK_SECRET` and `STRIPE_CONNECT_WEBHOOK_SECRET`: each endpoint's signing secret
   rolled (Developers › Webhooks): two endpoints, two secrets.
3. `R2_ACCESS_KEY_ID`, `R2_SECRET_ACCESS_KEY`: a new R2 API token (Object Read & Write on the
   bucket only), the old one deleted (Cloudflare › R2 › Manage API tokens).
4. `DATABASE_URL`: the Postgres credentials regenerated in Railway.
5. `MF_END_SESSIONS_BEFORE` set to the time of the change: sessions made before it end on the
   next start. The sessions hold StitchFoundry account tokens too, so end StitchFoundry's sessions
   the same way (`SF_CLOUD_END_SESSIONS_BEFORE`, StitchFoundry's `docs/cloud.md`).
6. Railway itself: a new password, two-factor sign-in, its API tokens revoked, the project's
   members and deploys checked.

## Building the image yourself
From the repository root: `docker build -t makerfoundry .` then
`docker run --rm -p 8080:8080 makerfoundry` and open <http://localhost:8080>.
Without Docker: `cargo run -p makerfoundry`.
