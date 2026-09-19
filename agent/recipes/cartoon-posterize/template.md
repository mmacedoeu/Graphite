# cartoon-posterize — adaptation guide

## Source

A source photo (input raster) is posterized to 4 color bands, then smoothed
with a 3-pixel MedianFilter and finally thresholded to keep edges crisp. The
source archive is `frames = 1` and is intended for static adaptation; the
recipe can be re-rendered under animation by setting `defaults.frames > 1` and
adding a QuantizeAnimationTime to its `required[]`.

## Composition Plan

For an adaptation:

- Set `Posterize Levels` to 4 for a four-colour toon look; 5-7 for a softer
  digital-illustration read. Values above 8 start to look unfiltered.
- Keep `MedianFilter Size` at 3 unless the source has heavy grain (in which
  case 5 reads cleanly without softening critical edges).
- The Threshold at the end is optional. Drop it for a softer toon look or
  keep it to recover hard cell-shaded edges.
- Use this recipe on photographic or high-detail input; applying it to
  vector input produces no visible difference.

## Asset Plan

- One optional input: the source raster. PNG with at least 8-bit per channel.

## Default Bindings

- `Posterize Levels = 4`
- `MedianFilter Size = 3`
- `Threshold = (omitted by default; engagement opt-in)`

## Fidelity Checks

- All `required[]` identifiers still resolve in `NODE_METADATA`.
- The recipe compiles at `frames = 1`; if adapted to animation the
  `Composition Plan` heading must move to an `Animation Timeline`.
