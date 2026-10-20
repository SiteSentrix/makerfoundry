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
| `STRIPE_SECRET_KEY` | to sell | MakerFoundry's Stripe secret key (`sk_test_…` until launch), from Stripe › Developers › API keys, with Connect switched on. Without it every design is free |
| `STRIPE_WEBHOOK_SECRET` | with the key | the signing secret (`whsec_…`) of the webhook endpoint `https://makerfoundry.app/stripe/webhook` listening to **your account's** events `checkout.session.completed`, `checkout.session.async_payment_succeeded`, `checkout.session.expired`, `charge.refunded` |
| `STRIPE_CONNECT_WEBHOOK_SECRET` | with the key | the signing secret of a second endpoint at the same address listening to **connected accounts'** event `account.updated` (sellers' set-up finishing) |
| `MF_PUBLIC_URL` | no (default `https://makerfoundry.app`) | the site's address, for the links Stripe sends people back to and the sitemap |
| `STRIPE_API_URL` | no | a stand-in for Stripe's API (local tests) |
| `STORAGE_ENDPOINT` | no | an S3-compatible address in place of R2's (local tests) |
| `STITCHFOUNDRY_CLOUD_URL` | no (default `https://cloud.stitchfoundry.app`) | StitchFoundry's account service, which MakerFoundry's accounts are |

## Building the image yourself
From the repository root: `docker build -t makerfoundry .` then
`docker run --rm -p 8080:8080 makerfoundry` and open <http://localhost:8080>.
Without Docker: `cargo run -p makerfoundry`.
