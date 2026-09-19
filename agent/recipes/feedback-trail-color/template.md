# feedback-trail-color — adaptation guide

## Source

AnimationTimeNode drives a per-frame offset vector; the moving layer is
re-projected at each frame and blended against the previous frame via
BlendMode = Screen; Threshold = (drop) renders only the *new* pixels, so the
trail forms behind the moving edge rather than as a smear.

## Animation Timeline

[0.00–0.50s] Head appears at rest position; nothing else moves.
[0.50–1.50s] Head strokes +1px/frame in `screen` blend.
[1.50–2.00s] Head strokes fade via Opacity 1.0 -> 0.0.

The full timeline covers 2.0s (frames=60, fps=30) before looping back to 0.

## Asset Plan

- Required inputs: the moving subject shape (vector).
- Optional inputs: the trail colour (default `#7AA8FF`).

## Default Bindings

- AnimationTimeNode `rate = 1`.
- BlendMode = `Screen`; Opacity = 1.0 fading to 0.0 over the last beat.
- `loop = true`.

## Fidelity Checks

- All `required[]` identifiers still resolve.
- Timeline markers cover `[0.0, frames/fps) = [0.0, 2.0)` contiguously.
