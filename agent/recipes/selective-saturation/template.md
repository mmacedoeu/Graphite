# selective-saturation — adaptation guide

## Source

Three nodes carry the work: the image enters ImageColorPaletteNode to extract
a palette; the palette feeds a mask via ClippingMaskNode; the mask plus the
original image feed HueSaturationNode with `Saturation = +20` on the masked
region only.

## Composition Plan

- Increase `Saturation` to +30 for fast-cut editorial; cap at +15 for product
  hero shots where accuracy matters.
- The mask comes from the palette's dominant-color threshold by default.
  Switch to a hand-painted mask input via `ClippingMaskNode` when the
  composition has a hard subject/background split.
- Always render the unsaturated backplate first, then the masked Hi-Sat on
  top, so a downstream blend-mode can rescue bad cases.
- The two blend modes that survive a mask failure are `Multiply` (subject
  too dark) and `Screen` (subject too washed). Use them in that order.

## Asset Plan

- Optional inputs: a hand-painted mask (single channel, 0 = subject, 255 = rest).

## Default Bindings

- `HueSaturation Saturation = +20`.
- `ClippingMaskNode` strict-mask mode.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- Mask input is single-channel (mask shape, not raster) when fed as
  `mask_in`. A raster input here will silently produce a tonal desaturation.
