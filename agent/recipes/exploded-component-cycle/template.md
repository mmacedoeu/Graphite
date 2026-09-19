# exploded-component-cycle — adaptation guide

## Source

Five nodes describe the cycle: AnimationTimeNode shares `time` across four
sub-graphs that each scale and translate one component outward; the four
components are drawn by RectangleNode + LineNode with StrokeNode shaping
their geometry; OpacityNode on each sub-graph dampens the cycle's edges so
the explosion reads as a single breath.

## Animation Timeline

[0.00–0.50s] Components fan out along their axes.
[0.50–5.00s] Hold the offsets (the longest beat).
[5.00–6.00s] Components return to the centre.

The full timeline covers 6.0s (frames=180, fps=30) before looping back to 0.

## Asset Plan

- Required inputs: four component shapes (vector).
- Optional inputs: the stroke colour (default `#A57B5C`).

## Default Bindings

- AnimationTimeNode `rate = 1`.
- `frames = 180, fps = 30, loop = true`.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- Timeline markers cover `[0.0, frames/fps) = [0.0, 6.0)` contiguously.
