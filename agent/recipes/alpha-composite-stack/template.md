# alpha-composite-stack — adaptation guide

## Source

Three source layers compose to one image through OpacityNode + BlendModeNode + a single
ClippingMaskNode. The source archive has the same three nodes in their canonical
ports: backgrounds feeds a BaseLayer, the mid graphic is a SubjectLayer
wearing an OpacityNode at 0.85, and the foreground Drop feeds a HighlightLayer
clipped to a rounded Shape mask. The BlendModeNode is configured as Multiply on the mid
layer and Normal on the highlight.

## Composition Plan

For an adaptation that uses this recipe with your own layers:

- Replace each `<layer>` Render node with your own image source. Keep the
  layer count to three unless the new composition calls for a fourth; each
  additional layer needs its own OpacityNode.
- Set `Opacity` on the mid layer to `0.85` for a typical overlay. Increase to
  `1.0` when the mid layer carries the subject; decrease below `0.5` only when
  you intend the background to read through strongly.
- Use `Multiply` for the mid when it is darker than the background (logos, type,
  patterns on photos). Switch to `Screen` when the mid is brighter (a glow on a
  dark base).
- The clip mask uses a rounded rectangle derived from the foreground path; use
  a circle when the subject is round, an ellipse when a portrait crop is
  desired. The ClippingMaskNode's input is a Boolean, not a Path.

## Asset Plan

- Optional inputs: 3 raster layers (background, mid, foreground). PNG with
  alpha preferred for the foreground so the clip mask aligns to the visible edge.
- Required inputs: the foreground path or shape used by the clip mask.

## Default Bindings

Do not change without re-rendering. Defaults preserve the source composition:

- Layer 1 (background): final render target.
- Layer 2 (mid): `Opacity = 0.85`, `BlendMode = Multiply`.
- Layer 3 (foreground): `ClippingMaskNode = <mask shape>`, `BlendMode = Normal`.

## Fidelity Checks

- No DAG cycle: every node is downstream of a source or constant.
- All `required[]` identifiers still resolve in the live `NODE_METADATA`.
- The ClippingMaskNode receives a Boolean-shaped input (path, boolean, or
  shape-derived boolean). A path-shaped input will fail to compile.
- The recipe compiles with `frames = 1`; if you adapt it for animation
  the static-recipe `Composition Plan` heading must move to an
  `Animation Timeline` heading under the recipe's new author.
