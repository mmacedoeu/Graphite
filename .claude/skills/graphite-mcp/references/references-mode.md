# References mode

When the user wants the recipes corpus (not a tool, not a graph, not an
asset), the right mode is References. This document describes what the
recipes layer is, why it exists, and how to navigate it without falling
into one of the common traps.

## What the recipes layer is

The recipes layer is a curated, versioned corpus of parameterized graph
intents. Each recipe is a directory under `agent/recipes/<id>/` with three
files:

- `recipe.json` — the structured card: id, category, summary, aliases,
  required node identifiers, defaults, source path, template path, and
  version metadata.
- `template.md` — the prose that names the canonical ports, the bindings
  an adapter should consider, the asset plan, and the fidelity checks.
- `preset.md` — a short marketing-style card that previews the result.

The committed `agent/recipes.json` is the generated catalog. It is what
the host's `recipes.list` tool reads; the per-recipe directories are the
source of truth that `recipes-build` regenerates the catalog from.

## What the recipes layer is not

It is not a generated graph. Calling `recipes.show` returns the
template, not a node network. To turn a recipe into a graph, read the
template and realize the bindings via `graph.add_node`,
`graph.set_input`, and `graph.connect`. See `recipe-adaptation.md` for the
realization protocol.

It is also not an asset. Calling `recipes.show` does not render anything.
To see the canonical preview, follow the recipe's `source.path` (under
`agent/recipes/<id>/source.gdd`) into `render.preview` or
`render.preview_gif`.

## Discovery order

For an unfamiliar user, follow this order:

1. `resources/read` of `graphite://recipe-catalog` — once per session.
   This returns the full catalog with category and summary for every
   recipe, so the agent can pick a candidate before any other call.
2. `recipes.show` on the chosen id — returns the full card, template, and
   preset. The agent should treat this as the authoritative description.
3. Optional `recipes.lint` — surfaces any catalog drift. Run if the user
   asks for a clean check or if any recipe was just edited.

If step 1 returns nothing or fails, the agent process's CWD is not a
Graphite workspace; the host degrades gracefully and returns an empty
catalog with an `error` field. In that case, do not proceed — switch to
the user and ask where the recipes corpus is rooted.

## Traps

- The catalog and `recipes.list` must agree. If they diverge, run
  `recipes-build` to regenerate. The conformance suite has a test that
  catches drift between the two (`graphite_recipe_catalog_resource_is_advertised_and_readable`).
- The `required[]` list is a *minimum* set, not a *maximum* set. A recipe
  may use additional nodes beyond those listed; the list is the
  irreducible core.
- Aliases are not interchangeable with the id. `alpha-composite-stack` is
  the id; `layer-stack` is an alias. `recipes.show` accepts only the id;
  if the user gave an alias, look up the id via `recipes.list` first.
- `defaults.fps_or_frames_missing` and `source.missing` are warnings, not
  errors. The seeded corpus emits them because the asset re-render
  pipeline is a follow-on; `recipes.lint --strict` still passes.
- The recipes layer is read-only through MCP. There is no
  `recipes.create`, `recipes.edit`, or `recipes.delete`. Authoring happens
  on disk and the catalog is regenerated.

## When the corpus is the wrong answer

- The user wants a graph they can adapt to a custom intent. Use References
  to find the closest recipe, then realize it (or switch to Original
  mode if no recipe fits).
- The user wants to audit a recipe's compliance with the schema. Use
  `recipes.lint`; do not manually inspect `recipe.json`.
- The user wants to add a new recipe. This is contributor work, not
  agent work — point them at the contributor workflow in
  `quality-review.md` (the section on "Adding a recipe").