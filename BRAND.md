# Brand

Visual identity for `scissors`. Light by design; covers what's needed to display
the project consistently across the README, package registries, and external
mentions.

## Logo

The logo derives from "scissors" by `johnny_automatic` on Openclipart (2006),
released into the public domain. See [Sources and licenses](#sources-and-licenses)
for the full provenance. Two variants live in `assets/brand/`:

| File | Purpose |
|------|---------|
| `logo-light.svg` | Charcoal `#1a1a1a` baked in. Use on light backgrounds (default GitHub theme, PyPI, crates.io). |
| `logo-dark.svg` | Off-white `#eaeaea` baked in. Use on dark backgrounds (GitHub dark theme). |

## Palette

| Role | Name | Hex |
|------|------|-----|
| Accent (light bg) | Charcoal | `#1a1a1a` |
| Accent (dark bg) | Off-white | `#eaeaea` |

No secondary colors. If/when needed, extend this table rather than introducing
colors ad hoc in docs or assets.

## Usage rules

- Minimum height: 32 px. Below that, the scissors detail is lost.
- Clear space: leave at least 0.5x the logo's height on every side.
- Do not rotate, skew, or recolor outside the palette.
- Do not place the dark variant on a light background or vice versa.

## Sources and licenses

Base illustration: "scissors" by [`johnny_automatic`][artist] on Openclipart,
uploaded 2006-10-25, released into the public domain (Creative Commons Public
Domain Dedication, declared in the SVG's RDF metadata).

- Original page: https://openclipart.org/detail/903/scissors
- Original drawing: *Home and school sewing* by Frances Patton, Newson & Co.,
  New York, c.1901 (itself public domain by age).

[artist]: https://openclipart.org/artist/johnny_automatic

## Regenerating variants

The committed `assets/brand/logo-*.svg` files were produced from the
original Openclipart SVG by a one-shot normalization (inject
`fill="currentColor"` on the root `<svg>`, recolor `#EFF0F0` highlights at
`fill-opacity="0.15"`, then bake the accent hex into each variant). If the
palette changes, re-fetch the source, re-run the substitution, and overwrite
the light/dark variants. No build pipeline required.
