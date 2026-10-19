# MakerFoundry — instructions for agents

MakerFoundry is a marketplace by **SiteSentrix** where makers share and sell **machine-embroidery
designs** (DST, PES, JEF, EXP, VP3, XXX…) and **laser-cut files** (SVG, DXF, PDF…). Free designs
are shared openly; paid designs are sold with MakerFoundry as the broker, taking a small fee on
each sale. Sibling products: [StitchFoundry](https://github.com/Stitchwright/StitchFoundry)
(embroidery digitizing app, https://stitchfoundry.app) and LightFoundry (laser cutting). A later
sibling, PatternFoundry, is reserved for sewing patterns.

These rules come from the VectorCraft/StitchFoundry `CLAUDE.md` (kept for reference in
[`docs/vectorcraft-CLAUDE.md`](docs/vectorcraft-CLAUDE.md)), adapted to a web service.

## Start every session here
1. Read [`docs/start-here.md`](docs/start-here.md): who the owner is, what exists already
   (accounts, hosting, domains), the decisions made and the ones still open.
2. Read [`docs/plan.md`](docs/plan.md): the product, the milestones and their task ids. Unless the
   user gives you a task, take the next unfinished one.
3. Orient → plan → implement + test → verify → record (update `docs/plan.md` and the README's
   Status) → commit. Don't stop to ask unless the decision is listed as the owner's in
   `docs/start-here.md` (money, legal wording, anything public, anything that costs).

`plan/` is gitignored (local working notes only).

## Non-negotiables
- **Clean room.** Behaviour comes only from public documentation, cited by URL. Never copy code,
  text, page designs, icons or images from Thingiverse, Etsy, Cults, Creative Fabrica, Embroidery
  Library or any other site or app. Never copy GPL/AGPL code. Never copy code from Roidery
  (github.com/lork-fork/Roidery) (owner's decision, 2026-10-16).
- **Assets.** Every image, icon, font or other asset is original work (MIT OR Apache-2.0), OSI open
  source, public domain/CC0 or Creative Commons with redistribution allowed, and has a row in
  `ASSETS.md` (path, author, source URL, licence). Prefer art generated in code. If an asset's
  provenance is unclear, don't add it. No fonts in this repo.
- **Users' uploads are untrusted input.** Every uploaded file (embroidery files, SVG, DXF, PDF,
  images, zips) is parsed defensively: size caps, count caps, no unbounded allocation or loops, no
  zip bombs, no path traversal, SVG never served inline from our origin unsanitized (scripts,
  external references). Parsers get fuzz/property tests, as StitchFoundry's do.
- **Never crash: no panics in shipped code.** Fallible work returns `Result`/`Option` and is
  handled or propagated with `?`. Workspace clippy lints ban `unwrap()`, `expect()`, `panic!`,
  `unreachable!`, `todo!`, `unimplemented!` outside tests. Use `.get(i)` for indexes from data,
  never divide by something that can be zero, check floats are finite before casting. Tests may
  unwrap.
- **Money is exact.** Prices and fees are integer cents (never floats); every charge, fee, payout
  and refund is a ledger row; Stripe webhooks are verified and idempotent; a sale is delivered
  once its payment is confirmed, never before.
- **Privacy.** Collect the least needed. Never log passwords, tokens, card data or file contents.
  Server secrets live in the host's configuration (Railway variables), never in the repository.
  Never paste secrets into chat or commits.
- **Copyright safety.** Sellers confirm they own what they upload; there is a takedown process
  (DMCA) from the first public release; listings of other people's logos, characters and
  commercial designs are refused or removed.
- **Rust.** The service is Rust (server-rendered HTML pages; small hand-written CSS). No
  JavaScript frameworks; a little progressive-enhancement JS only if a page can't work without it
  and the owner agrees.
- **Quality gates before every commit:** `cargo fmt --check`, `cargo clippy --all-targets -- -D
  warnings`, `cargo test` (a `cargo xtask ci` once the workspace has one). One task id per commit
  (`MF-1.2: upload a design`). Never push to `main` with a red check; never force-push or amend
  pushed commits.
- **Look at what you build.** For page work, render the page (Playwright/Chromium is installed in
  cloud sessions) and look at it at phone and desktop widths before calling it done.

## Commit authorship
Commits are authored by **Bruce Wattendorf**, so the work shows on his GitHub profile. AI
assistance is credited with a trailer, not as the author.
- Before the first commit: `git config user.name "Bruce Wattendorf"` and
  `git config user.email "bwattendorf@gmail.com"` (or pass
  `--author="Bruce Wattendorf <bwattendorf@gmail.com>"` on every commit).
- Never author or commit as `Claude <noreply@anthropic.com>`.
- End every commit message with: `Co-Authored-By: Claude <noreply@anthropic.com>`
- No model names in commits, code or docs.

## Licence
Code and original assets are MIT OR Apache-2.0 (`LICENSE-MIT`, `LICENSE-APACHE`). Designs that
users upload stay theirs, under the licence they choose for each listing (see `docs/plan.md`).
