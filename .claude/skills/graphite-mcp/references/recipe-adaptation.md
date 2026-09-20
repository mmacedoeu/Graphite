# Recipe adaptation

How to take a recipe's `template.md` (the prose returned by `recipes.show`)
and turn it into concrete `graph.set_input` and `graph.connect` calls.
Recipes are not generated graphs; they are parameterized cards an agent
realizes on demand.

## The five template sections

Every template has the same five headings, in this order:

1. **Source** — what the canonical `.gdd` looks like, in prose. Names the
   source archive and the node ports the recipe's author used.
2. **Composition Plan** (static recipes) or **Animation Timeline** (motion
   recipes) — the parameterized adaptations a downstream author would
   consider. This is where most agent edits will land.
3. **Asset Plan** — what inputs the user is expected to provide (raster
   files, fonts, paths) and which are optional.
4. **Default Bindings** — the values the recipe's author committed to. Do
   not change these without re-rendering and updating the catalog.
5. **Fidelity Checks** — invariants the recipe must preserve. A failure
   here means the recipe has drifted and the catalog needs a refresh.

The lint passes today because every recipe in the corpus ships with all
five headings. If a template loses one, `recipes.lint` reports it as an
error.

## From prose to graph

Treat each bullet under **Composition Plan** or **Animation Timeline** as
one binding. For `alpha-composite-stack` the bullets map roughly to:

| Bullet | Realization |
|---|---|
| "Replace each `<layer>` Render node with your own image source" | `graph.add_node` with the user's raster node, then `graph.set_input` on the upstream `Render` node |
| "Set `Opacity` on the mid layer to `0.85`" | `graph.set_input` on the mid layer's `OpacityNode` input |
| "Use `Multiply` for the mid, `Screen` when brighter" | `graph.set_input` on the `BlendModeNode` input (enum value: `Multiply` or `Screen`) |
| "The clip mask uses a rounded rectangle derived from the foreground path" | `graph.set_input` on the `ClippingMaskNode` input with a Boolean shape |

The exact input names come from `node.describe`, not from prose. Always
call `node.describe` for each `required[]` node before realizing a binding.

## Animation Timeline bindings

Motion recipes carry an **Animation Timeline** section instead of (or in
addition to) a **Composition Plan** section. The Timeline names frames,
phases, and easing; each phase typically maps to one or more `set_input`
calls on a downstream node.

`AnimationTimeNode` provides a 0..1 normalized time. Wire its output into
any node that should track the loop. For a per-phase effect, gate with a
`QuantizeAnimationTimeNode` (Debug) or a `Range` comparison on the
animation time, so the binding holds within `[phase_start, phase_end]`
and falls back outside.

Recipes that use `RealTimeNode` (e.g. `pulse-glow`) do not loop — the
animation time is the wall-clock time since the editor opened the document.
Do not use these recipes when the user wants a reproducible GIF; switch to
a `AnimationTimeNode` recipe or skip the GIF export.

## Defaults and overrides

The **Default Bindings** section lists values that are part of the recipe's
identity. Changing them without re-rendering the asset and stamping the
new SHA256 (via `recipes-build`) will leave `recipes-build --strict`
failing. The right sequence is:

1. Edit `agent/recipes/<id>/template.md` to reflect the new defaults.
2. Re-render the source `.gdd` to PNG/GIF, replacing the existing asset.
3. Run `recipes-build --update` to refresh the catalog and stamp new
   SHA256 hashes.
4. Run `recipes.lint` to confirm the corpus is still clean.

If the user only wants a one-off adaptation (not a recipe change),
override the binding in their working document; do not touch the recipe.

## Asset plan realizations

The **Asset Plan** section lists what the recipe expects. For an
`alpha-composite-stack` adaptation the user supplies three rasters; for a
`kinetic-typography-loop` adaptation the user supplies a font file or a
string for `TextToVectorNode` to rasterize.

Each asset becomes one input node in the graph (a raster, a string, a
shape). When the asset is missing, the recipe renders with a placeholder
(usually a constant of the right type). The placeholder is part of the
**Default Bindings**, not the user's input.

## Fidelity checks

The **Fidelity Checks** section is a list of invariants. If any of them
fails after a change, the change is a regression. The most common checks:

- No DAG cycle: every node is downstream of a source or constant.
- All `required[]` identifiers still resolve in the live `NODE_METADATA`.
- The recipe compiles with `frames = 1` (static) or `frames > 1` (motion).
- For motion recipes, `AnimationTimeNode` (or `RealTimeNode`) is wired
  into every per-frame dependency.

The conformance suite (`agent/cli/tests/recipes_conformance.rs`) verifies
the third check for every seeded recipe via the
`recipes_show_template_references_every_required_node_identifier` test;
the others are run-time checks surfaced by `render.preview`.