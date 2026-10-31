# Asset attribution

Every non-code asset in this repository (images, icons, example files) has a row here with its
author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an image or
icon file has no row, and refuses font files outright: **no fonts in this repository** (pages use
the system's fonts).

Art drawn in code is original and has no file to list. That covers the MakerFoundry mark (a
needle and thread, a laser head and a print nozzle on a red tile, an inline SVG in
`apps/makerfoundry/src/pages.rs`, also the Facebook page's picture) and
the stylesheet (`apps/makerfoundry/static/site.css`), both by SiteSentrix, MIT OR Apache-2.0.

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
