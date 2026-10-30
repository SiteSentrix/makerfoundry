# MakerFoundry release-readiness audit (issue #40)

**Date:** 2026-10-30, at commit `6fcf981` plus the fixes below. The prompt for issue #40 had no
text, so the scope here is the owner's choice: this repository only, covering security, privacy,
legal, operations, tests and the switched-off features. Each sibling repository has its own audit
issue (see `docs/family.md`).

**Verdict:** ready for the launch the owner chose (free designs, soft open), once the owner does
the launch steps in `docs/launch.md` (DMCA agent, admins, `MF_LAUNCHED=yes`). Nothing found blocks
it. MakerFoundry 3D and the outside services stay off until their own steps.

## Fixed in this audit
- **Dependencies:** `cargo audit` (RustSec, 1,395 advisories) found 4. Three were fixed by patch
  updates: `rustls-webpki` 0.103.16 (could accept distrusted intermediates), `webpki-roots` 1.0.11
  (distrusted emSign roots), and `ctutils` 0.4.7 (non-constant-time compares). The fourth, `rsa`
  (RUSTSEC-2023-0071, no fix released), is only in `Cargo.lock` through `sqlx-mysql`, which is
  never built; it's ignored with that reason in `.cargo/audit.toml`.
- **HSTS:** responses had no `Strict-Transport-Security`. Every response now carries
  `max-age=31536000`, not `includeSubDomains`, since other services may share the domain. Tested
  in `tests/pages.rs`, which fails without it.
- **Settings documentation:** four service-address overrides (`THINGIVERSE_WWW_URL`,
  `THINGIVERSE_API_URL`, `META_GRAPH_URL`, `RESEND_API_URL`) weren't in `docs/deploy.md`; now they
  are. All 29 settings the server reads are documented.
- **Privacy wording to come:** the live Privacy Policy doesn't yet name what the switched-off
  services would send where (Resend, Meta, Thingiverse, the converter and OpenSCAD services, API
  keys, contests). Drafts for each, to merge before switching it on, are in
  `docs/legal/3d-draft.md`.

## Checked and fine
| Area | What was checked | Result |
|---|---|---|
| Secrets | `git grep` for live keys, private keys, passwords | Only test placeholders (`sk_test_x`, `whsec_one`) |
| Logs | every `eprintln!` for tokens, emails, passwords, file contents | None logged; secrets' `Debug` prints only the type name |
| No panics | workspace lints | `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented`, `unreachable` are denied, with no `allow` anywhere |
| Forms | all 76 POST routes | Each checks the request's origin (directly or through a shared helper), except the API (bearer tokens) and the one-click unsubscribe (an HMAC token, by RFC 8058) |
| Sessions | cookie | `__Host-` cookie, `HttpOnly; Secure; SameSite=Lax`, only its SHA-256 kept |
| Headers (live) | `curl -I https://makerfoundry.app/` | CSP `default-src 'none'`, `frame-ancestors 'none'`, `X-Frame-Options: DENY`, `nosniff`, referrer policy; HSTS after this deploy |
| Uploads | readers, limits, property tests | Every reader is property-tested with random and damaged files (`tests/untrusted_input.rs`, 30 properties at 20,000 cases under a 3 GB memory cap); size and count caps per route |
| Rate limits | sign-up, sign-in, uploads, reports, comments, likes, API keys, customizing | In place and tested |
| CI | GitHub Actions on `main` | Green on every commit; 2 runs cancelled only because a newer push superseded them |
| Live site | `/health` | `ok 0.1.0 <commit>`; deploys follow green pushes |
| Legal pages | `[OWNER` markers and DRAFT lines in the live documents | None (approved 2026-10-22) |
| Your data | download and delete | Cover every table added this month (notices, API keys, contests, likes, imports, showcase consents) |

## The owner's, before or at launch
1. **DMCA agent:** register at copyright.gov ($6) and set `MF_DMCA_AGENT`. Until then `/dmca`
   asks for the form, and the safe harbour isn't in place (`docs/launch.md` step 3).
2. **Database backups:** turn them on in Railway › Postgres › Backups (`docs/deploy.md`); there
   is no way to check from here that they are on. Files in R2 have no second copy. Consider R2
   object versioning, or a periodic copy of the bucket.
3. **Admins and the launch switch:** `docs/launch.md` steps 5 and 6.
4. **Dependency checks in CI:** `cargo audit` isn't part of `cargo xtask ci` yet (installing it
   adds a few minutes to a run). Suggest a weekly scheduled GitHub Action that runs it.
5. **MakerFoundry 3D:** approve `docs/legal/3d-draft.md` (#26), then `MF_3D=yes`.
6. **Each switched-off service:** its Privacy line (above), its account, then its setting
   (`docs/deploy.md`).

## Not covered
- A penetration test or third-party review; a lawyer's read of the legal pages (advised in
  `docs/legal/README.md`).
- Load beyond the 100,000-model search test.
- The sibling repositories (their own audit issues).
