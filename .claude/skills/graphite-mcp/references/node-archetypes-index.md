# Node archetype index

The seeded recipe corpus at `agent/recipes/`. Each card is one paragraph that
names the recipe's intent, the node classes it leans on, and the kind of
work it shortcuts. Read `recipes.show` for the full template, preset, and
authoritative defaults.

| Recipe | Category | One-line card |
|---|---|---|
| `alpha-composite-stack` | Compositing | Three-layer alpha composite stack with per-layer blend modes; leans on `ClippingMaskNode` + `OpacityNode` + `BlendModeNode`. |
| `cartoon-posterize` | Filtering | Posterize bands + median filter for a flat, edge-clean illustration; uses `PosterizeShaderNodeNode` + `MedianFilterNode` + `ThresholdNode`. |
| `exploded-component-cycle` | Compositing | Four-component vector exploded and reassembled over a 6s loop; uses `AnimationTimeNode` + vector generators + `OpacityNode`. |
| `feedback-trail-color` | Motion | Per-frame feedback trail: a trail-coloured trail behind a moving layer; uses `AnimationTimeNode` + blend + `ThresholdNode`. |
| `kinetic-type-static` | Typography | Single-frame typographic composition with kerned headline + caption; uses `TextToVectorNode` + `FillNode` + `StrokeNode`. |
| `kinetic-typography-loop` | Motion | Animated headline: build, hold, kern-snap, all on a 4s seamless loop; uses `AnimationTimeNode` + `TextToVectorNode` + `RectangleNode` + `OpacityNode`. |
| `lut-grade-warm` | Color | Warm-tone grading with a generated gradient LUT and a soft saturate; uses `EvaluateGradientNode` + `RgbaToColorNode` + `HueSaturationNode`. |
| `pulse-glow` | Motion | Real-time sin-wave brightness modulation: a soft pulse on the subject; uses `RealTimeNode` + `BrightnessContrastShaderNodeNode` + `MathNode` + `SineNode`. |
| `selective-saturation` | Color | Mask-driven selective saturation: pump the subject, leave the rest; uses `ImageColorPaletteNode` + `HueSaturationNode` + `ClippingMaskNode`. |
| `wireframe-progressive-build` | Geometry | Wireframe-to-solid progressive build over 4 seconds; uses `AnimationTimeNode` + vector generators + `StrokeNode` + `FillNode`. |

## Category counts

| Category | Count | Recipes |
|---|---|---|
| Color | 2 | `lut-grade-warm`, `selective-saturation` |
| Compositing | 2 | `alpha-composite-stack`, `exploded-component-cycle` |
| Filtering | 1 | `cartoon-posterize` |
| Geometry | 1 | `wireframe-progressive-build` |
| Motion | 4 | `feedback-trail-color`, `kinetic-typography-loop`, `pulse-glow`, `exploded-component-cycle` (also Compositing) |
| Typography | 1 | `kinetic-type-static` |

Note that `exploded-component-cycle` carries the `Motion` capability on top
of `Compositing` — it is a motion+compositing hybrid.

## Picking a recipe

- The user said "motion" or "loop": start in Motion. If the result needs a
  compositing step on top (alpha + blend modes), use `exploded-component-cycle`
  rather than building two recipes.
- The user said "color" or "grade": start in Color. `lut-grade-warm` is a
  full-grade recipe; `selective-saturation` is a targeted mask-driven
  approach.
- The user said "type" or "text": start in Typography. `kinetic-type-static`
  is single-frame; `kinetic-typography-loop` is the animated counterpart.
- The user said "filter" or "stylize": start in Filtering.
  `cartoon-posterize` is the only seeded Filtering recipe.
- The user said "wireframe" or "build": start in Geometry.
  `wireframe-progressive-build` is the only seeded Geometry recipe.

If none of the recipes fit, switch to Original mode and build from scratch —
do not stretch a recipe into a domain it was not designed for.

## What the index is not

The index does not carry the template's parameterized bindings or the
preset's defaults. Those live in the per-recipe `template.md` and
`preset.md` and are returned by `recipes.show`. The index is a navigation
aid, not a substitute.