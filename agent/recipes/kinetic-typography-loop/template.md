# kinetic-typography-loop — adaptation guide

## Source

AnimationTimeNode advances the timeline; its output branches to a text-build
curve for the headline and an OpacityNode for kern-snap. The five Rectangle
nodes are background cards that slide in behind the headline during the
build phase.

## Animation Timeline

[0.00–1.00s] Cards slide in (5 stages, one per 200ms slot).
[1.00–2.00s] Headline builds word-by-word.
[2.00–3.00s] Static hold; the cursor blinks on the last word (24-frame cycle).
[3.00–3.50s] Kern-snap pulls the centre pair tighter by 0.1em.
[3.50–4.00s] Last-frame cleanup, return to base.

The full timeline covers 4.0s (frames=120, fps=30) before looping back to 0.

## Asset Plan

- Required inputs: the headline string and the type face.
- Optional inputs: the brand colour for the highlight word.

## Default Bindings

- AnimationTimeNode `rate = 1`.
- AnimationTimeMode = `AnimationTime`.
- `loop = true` so the GIF renders a seamless 4-second cycle.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- `frames > 1` requires an Animation node in `required[]` (AnimationTime here).
- Timeline markers cover `[0.0, frames/fps) = [0.0, 4.0)` contiguously without
  overlap.
