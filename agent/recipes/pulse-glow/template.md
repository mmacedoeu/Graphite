# pulse-glow — adaptation guide

## Source

RealTimeNode's second-of-minute feeds MathNode which scales it to a 2-second
period; the result runs through SinNode to produce a 0..1 oscillation. The
output drives BrightnessContrast on the subject layer; the background layer
is left untouched.

## Animation Timeline

[0.0–0.5s]  Brightness offset +0 (sin ≈ 0); subject at base.
[0.5–1.5s]  Brightness peaks at +0.15; subject brighter.
[1.5–2.0s]  Brightness fades back toward 0; subject returns to base.

The full timeline covers 2.0s (frames=60, fps=30) before looping back to 0.

## Asset Plan

- Optional inputs: the source subject raster.

## Default Bindings

- `RealTimeMode = Second`, period 2.0 (one cycle every two seconds).
- `BrightnessContrastNode Brightness = 0..0.15 * sin(2πt/period)`.
- `loop = true` so the GIF renders a single seamless cycle.

## Fidelity Checks

- All `required[]` identifiers still resolve in the live catalog.
- `frames > 1` requires an Animation node in `required[]` (RealTimeNode here).
- The timeline marks cover `[0.0, frames/fps] = [0.0, 2.0s)` contiguously.
