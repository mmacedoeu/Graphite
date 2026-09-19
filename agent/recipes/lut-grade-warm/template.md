# lut-grade-warm — adaptation guide

## Source

A two-stop gradient (warm amber to deep umber) feeds a color-shift LUT applied
to the image, then a HueSaturationNode lifts saturation by 10 only on warm
hues (mask out the rest via the source's selection input).

## Composition Plan

- For portraits: keep the saturation lift at 5-8; values above 12 produce
  artificial skin tones.
- For food/product photography: increase the lift to 15 and consider tightening
  the gradient to amber-cream.
- The gradient stops drive the look. Pick a left-stop in the 2900-3200K range
  for golden-hour, or shift to 4000K for late-afternoon.
- Do not branch the gradient per-channel; if you need a green-tinted dark,
  layer a separate green-only curve and multiply.

## Asset Plan

- Optional inputs: the source raster.

## Default Bindings

- Left gradient stop: `#F4BFA0` (warm amber).
- Right gradient stop: `#3A1E14` (deep umber).
- `HueSaturationNode` warm-only mask at 10.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- The recipe compiles at `frames = 1`.
