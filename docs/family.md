# The family: MakerFoundry's sibling projects (issue #41)

**Status:** 2026-10-30, from each repository's README and agent instructions (read, not copied).
Every project belongs to the same owner (Bruce Wattendorf, SiteSentrix) and follows the same
broad rules: clean room, MIT OR Apache-2.0, Rust, no panics in shipped code, tests before claims.
They share standards and decisions, **never code**: don't copy source between repositories; link to
the sibling's own docs and write it here.

| Project | Repository | What it is | How it touches MakerFoundry |
|---|---|---|---|
| **MakerFoundry** (this) | [SiteSentrix/makerfoundry](https://github.com/SiteSentrix/makerfoundry) | Marketplace for embroidery designs, laser files and 3D models; MakerFoundry brokers paid sales | n/a |
| **StitchFoundry** | [Stitchwright/StitchFoundry](https://github.com/Stitchwright/StitchFoundry) | Open-source embroidery digitizing app (desktop and browser), built on VectorCraft; <https://stitchfoundry.app> | Its cloud service runs the **Foundry Account** sign-in MakerFoundry uses (`cloud.stitchfoundry.app`); publishes designs to MakerFoundry's API (`docs/api.md`); **Open in StitchFoundry** from design pages. Pre-release (0.14 RC). |
| **LaserFoundry** | [Stitchwright/laserfoundry](https://github.com/Stitchwright/laserfoundry) | Open-source laser engraver/cutter app (LightBurn-class workflow, GRBL) | Will publish SVG/DXF to the same API; **Open in LaserFoundry** is built on our side and switched off until its web app opens links (`LASERFOUNDRY_OPENS_LINKS`, `docs/api.md`). Early. |
| **PatternFoundry** | not started | Reserved for sewing patterns (`patternfoundry.app` to be bought) | Would publish to MakerFoundry like the others. |
| **StitchBench** | [Stitchwright/stitchbench](https://github.com/Stitchwright/stitchbench) | Repair-bench software for sewing/embroidery machine techs: machines, factory specs, tickets, invoices, parts; with **StitchShop** for the sewing and quilt shops they work with | None in code. Shares the Railway account (its demos). A 3D-printed part in its inventory could one day link to a MakerFoundry model. |
| **StitchHub** | [Stitchwright/stitchhub](https://github.com/Stitchwright/stitchhub) | The community server StitchBench/StitchShop shops sync with: manual links, tech notes, parts cross-reference, a parts marketplace, partner links | None in code. This repo's issues #1 and #2 (a StitchHub sync client, a StitchShop demo storefront) belong to that family, not to MakerFoundry; see below. |
| **BuildKiosk** | [SiteSentrix/buildkiosk](https://github.com/SiteSentrix/buildkiosk) | Self-hosted construction-management software (Procore-class), no-JavaScript web UI, API, CLI and MCP server | None. Same owner and conventions. |

## Differences that matter when working across them
- **Commit authorship differs.** Here commits are authored by **Claude** with Bruce as co-author
  (owner's decision, 2026-10-21; `CLAUDE.md`). StitchBench, StitchHub and BuildKiosk author commits
  as **Bruce** with a Claude trailer. Follow each repository's own `CLAUDE.md`.
- **Agent rules live in different files:** here `CLAUDE.md`; StitchBench and BuildKiosk keep them in
  `AGENTS.md`; StitchFoundry and LaserFoundry in `CLAUDE.md` with `ROADMAP.md` and `docs/gaps.md`.
- **Who owns the contract:** MakerFoundry owns its API (`docs/api.md`) and Open-in links; the apps
  own their publish commands and their link handling. A change to the API is announced to the apps'
  repositories, not made in them from here.

## Open questions for the owner
- Issues #1 (StitchHub sync client) and #2 (Main Street Threadworks demo storefront) describe
  StitchHub/StitchShop work. Should they move to those repositories?
