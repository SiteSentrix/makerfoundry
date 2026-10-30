# 3D printing hub: feature matrix

**Status:** 2026-10-30, research for the 3D hub (plan: [`3d-hub-plan.md`](3d-hub-plan.md)). Thingiverse
and Printables were studied for **what they offer and how people use it**, from public help pages,
published terms, press and general knowledge, never by downloading, mirroring or copying their
models, files, images, text, code, styles or branding (CLAUDE.md › Clean room). MakerWorld (Bambu
Lab) is noted where it sets the bar (print profiles), on the same terms; it has no official public
API, and none of its endpoints are used.

**Priority:** P1 = at the hub's launch, P2 = soon after, P3 = later. "Yes"/"No" is whether the site
offers it today as far as public sources show; a mark of "partly" is explained in the notes.
Complaints are the ones users commonly raise in public forums and press; they're general knowledge,
not measured, and are listed as what MakerFoundry should beat, not as claims about either site.

## The matrix

| Feature | Thingiverse | Printables | MakerFoundry | Notes, and the complaints to beat |
|---|---|---|---|---|
| Upload STL | Yes | Yes | **P1** | Read and checked on upload (size, triangle count, finite numbers), never just stored. |
| Upload 3MF | Yes | Yes | **P1** | A ZIP of XML: read with limits (zip bombs, paths, no XML entities), rebuilt with only known parts, so nothing executable rides along. |
| Upload OBJ | Yes | Yes | **P1** | Geometry only; materials files kept as downloads. |
| Upload STEP | Yes (as a file) | Yes (as a file) | **P1** download only; preview **P3** | No Rust STEP kernel: checked as STEP text and offered for download; a preview needs a converter (P3). |
| Several files per model | Yes | Yes | **P1** | Parts, plates, variants and source files in one model. |
| Images per model | Yes | Yes | **P1** | Through the existing photo cleaner (location data removed). |
| In-browser 3D viewer | Yes | Yes | **P1** (approach is the owner's call: JavaScript) | A common complaint is slow, heavy model pages. Plan: server-drawn thumbnails and views with no JavaScript, plus an optional viewer (`3d-hub-plan.md` › Viewer). |
| Auto thumbnails | Yes | Yes | **P1** | Drawn on the server from the mesh, or the 3MF's own plate thumbnail when it has one. |
| Description and print settings | Yes | Yes | **P1** | Plain text with paragraphs, plus structured print settings (layer height, infill, supports, material). |
| Licences: Creative Commons | Yes (CC, also GPL, LGPL, BSD, public domain) | Yes (CC, GNU, BSD, Apache, CERN OHL, Standard Digital File, Commercial) [^pt] | **P1**: Creative Commons, CC0, GNU GPL and LGPL, BSD, Apache, MIT, CERN OHL, MakerFoundry's own (personal, small business, commercial) and "All rights reserved" | The owner chose licences like both sites' (2026-10-30). Public licences linked to their official texts; Printables' own licence text not copied (MakerFoundry's own stand in). Every model shows its licence in plain words. |
| Remixes and attribution tree | Yes | Yes | **P1** | Licence disputes over remixes come up often [^benchy]. MakerFoundry **enforces** compatibility at publish time (no remix of a NoDerivatives model, ShareAlike and NonCommercial carried over) and shows the whole tree. |
| Makes (photos of prints) | Yes | Yes | **P1** | Reuses MakerFoundry's makes (MF-11). |
| Collections | Yes | Yes | **P1** | Reuses MF-10. |
| Likes | Yes | Yes | **P1** | New: one per account per model, counted. |
| Follows | Yes | Yes | **P1** | Reuses MF-13. |
| Comments, with Report | Yes | Yes | **P1** | Reuses MF-14 and its report button. |
| Search | Yes | Yes | **P1** | Weak search is a frequent Thingiverse complaint. MakerFoundry: Postgres full-text search with weighted title, tags and description, and filters below; aim for under 100 ms a page. |
| Filters (category, licence, file type, has makes, recent, popular) | Partly | Yes | **P1** | Plus the print-profile filters: has a profile, printer, multi-colour, filament type. |
| Tags and categories | Yes | Yes | **P1** | A fixed category list (ours), free tags. |
| Creator profiles | Yes | Yes | **P1** | Reuses maker pages (MF-3), with models, makes, counts and the founding-creator badge. |
| Contests | Occasional | Yes, regular | **P3** | Prizes cost money: the owner's call. |
| Points and rewards | No | Yes (Prusameters, exchanged for filament and goods) [^pm] | **P3** | Points invite gaming (re-uploads, low-effort makes); revisit with the owner. |
| Print profiles (3MF with slicer settings) | No | Partly (print files, Prusa-centred) | **P1**, Bambu Studio/OrcaSlicer first | MakerWorld sets the bar here. MakerFoundry reads Bambu Studio and OrcaSlicer projects (plates, AMS colours, printer, nozzle, filament, settings) and PrusaSlicer and Cura projects at a basic level; several profiles per model, each its own 3MF download. |
| Printer compatibility badge | No | Partly | **P1** | X1C, P1S, A1, A1 mini, H2D and others, read from the 3MF. |
| Downloads, and all files as a ZIP | Yes | Yes | **P1** | Per file, and the whole model as one ZIP made once per version. |
| Customizer-style parametric models | Yes (OpenSCAD Customizer) | No | **P3** | Thingiverse's Customizer is often reported slow or broken. Doing it well needs a sandboxed OpenSCAD service: later, and the owner's call. |
| Moderation and reporting | Yes | Yes | **P1** | Reuses MF-5: reports, the copyright notice and counter-notice process, one-click takedown, repeat infringers counted. |
| Rate limits on uploads | Unpublished | Unpublished | **P1** | Per account per day, as for designs today. |
| Public API | Yes (OAuth 2.0 REST) [^tv] | No documented public API | **P3** | MakerFoundry's own API (MF-7) grows as the apps need it. |
| Import your own models | Partly (through its API, with your sign-in) | No official route | **P1** files you upload; **P2** Thingiverse account link | P1: the creator uploads their own files (many at once, or a ZIP), grouped into drafts they review. P2: link a Thingiverse account through its official OAuth API to prove ownership, if its terms allow. No crawling, no import by someone else's address. |
| "Originally published on" link | No | No | **P1** | With the original date; shown on the model, never fetched by us. |
| Ads | Yes (a common complaint) | Few | **None** | No ads. |
| Uptime and speed | Often criticised (outages, slow pages) | Generally good | **P1** | Server-rendered pages, no heavy scripts, cached thumbnails. |
| Opt-in social showcase | No | No | **P1** | Unticked consent on every upload and make, withdrawn at any time; an admin queue that drafts a caption crediting the creator's chosen name. |
| Founding creator badge | No | No | **P1** | For the first invited creators. |
| Get it printed (order a print) | No | Partly (Prusa's shop sells printers and filament, not prints) | **P1** hook | Links to MakerFoundry's ordering where the licence allows printing for sale (`3d-hub-plan.md` › Cross-sell). |

## Sources

Opened or found through public search on 2026-10-30. They describe features and terms only.

- Printables' terms of service (licence types, Sec. 2.3): <https://www.prusa3d.com/page/terms-of-service-of-printablescom_231249/>
- Prusameters, Printables' points: <https://help.prusa3d.com/article/how-do-prusameters-on-printables-work_365065>
- Thingiverse API, getting started (OAuth, `/users/me`): <https://www.thingiverse.com/developers/getting-started> (not reachable from here, found through search; to check before P2)
- A remix-licence dispute on Printables (the 3DBenchy remix contest): <https://all3dp.com/4/printables-celebrates-3dbenchy-with-remix-contest/>
- Creative Commons FAQ (NoDerivatives, ShareAlike): <https://creativecommons.org/faq/>, and the 4.0 compatibility notes: <https://wiki.creativecommons.org/wiki/4.0/Compatibility>
- Thingiverse licences and Customizer (secondary: a library guide and a CHI 2015 study): <https://library.macewan.ca/node/338>, <https://ilab.ucalgary.ca/static/publications/chi-2015-oehlberg.pdf>

[^pt]: Printables' terms, Sec. 2.3.
[^pm]: Prusa's help article on Prusameters.
[^benchy]: all3dp.com on the 3DBenchy remix contest, which followed a licence dispute.
[^tv]: Thingiverse's developer getting-started page.
