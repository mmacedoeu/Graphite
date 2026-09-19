# wireframe-progressive-build — adaptation guide

## Source

Five nodes describe a 4-second progressive build: LineNode strokes from the
bottom-left, RectangleNode fills along the same time axis, StrokeNode
shapes the line geometry, FillNode paints the solids. AnimationTimeNode
drives all of them via a shared `time -> opacity` curve.

## Animation Timeline

[0.00–1.00s] Line strokes draw at 60% line opacity.
[1.00–2.50s] Rectangle fills ramp in (one per 200ms window).
[2.50–3.50s] Stroke + fill converge to a final straight-line read.
[3.50–4.00s] Hold for half a second, then loop.

The full timeline covers 4.0s (frames=120, fps=30) before looping back to 0.

## Asset Plan

- Required inputs: the build subject geometry (vector).
- Optional inputs: the stroke colour (default `#3DB1D0`) and the fill base.

## Default Bindings

- AnimationTimeNode `rate = 1`.
- AnimationTimeMode = `AnimationTime`.
- `loop = true` so the GIF renders a seamless build+hold cycle.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- Timeline markers cover `[0.0, frames/fps) = [0.0, 4.0)` contiguously.
