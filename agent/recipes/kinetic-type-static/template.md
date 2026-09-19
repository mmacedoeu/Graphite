# kinetic-type-static — adaptation guide

## Source

A single-frame composition: one TextToVectorNode feeds a FillNode, a second
TextToVectorNode feeds the caption and renders below the headline. No
animation; `defaults = {}` keeps `frames = 1`.

## Composition Plan

- Replace the headline text with the user-supplied title string.
- The caption is anchored 80px below the headline baseline. Move it to 60px for
  tight compositions, 120px for posters with breathing room.
- Switch the headline Fill from near-black to the brand colour only when the
  background is white; on dark backgrounds, set Fill to white and stroke to a
  dim warm gray.
- Do not branch the TextToVector output: the caption is rendered once, then
  composited below.

## Asset Plan

- Optional inputs: a brand colour for the headline.
- Required inputs: text strings (the title and one caption line).

## Default Bindings

- Headline `Fill = #1A1A18`, `Stroke = none`.
- Caption `Fill = #4A4A45`, type weight 0.4 of the headline weight.

## Fidelity Checks

- All `required[]` identifiers still resolve in `NODE_METADATA`.
- The recipe compiles at `frames = 1`.
