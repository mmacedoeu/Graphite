# Modes

A request to Graphite via MCP always falls into one of six modes. Pick the mode
before touching any other tool — the right mode gates what is searched for,
how the response is shaped, and what counts as "done". Stop gates are listed
inline; if any stop gate fires, surface it to the user before continuing.

| Mode | Trigger phrase shape | Primary tools |
|---|---|---|
| Browse | "what can Graphite do", "list node types", "show me options" | `node.list_types`, `node.describe`, `recipes.list`, `resources/read` |
| Supplied-graph | "adapt this file", "load `path.gdd`", "patch the existing graph" | `document.open`, `graph.list_nodes`, `graph.get_node`, `graph.set_input`, `graph.connect` |
| Prompt-review | "review this graph", "explain what this does", "find the bug" | `graph.list_nodes`, `graph.get_node`, `render.preview`, `history.undo` |
| Production | "ship this", "commit", "export the asset" | `render.preview_gif`, `render.export`, `render.export_gif`, `history.commit`, `document.save` |
| References | "show recipes", "what is `alpha-composite-stack`", "use the LUT recipe" | `recipes.list`, `recipes.show`, `recipes.lint`, `resources/read` of `graphite://recipe-catalog` |
| Original | "design from scratch", "I have a goal, build a graph" | `node.list_types`, `node.describe`, `graph.add_node`, `graph.connect`, `render.preview` |

## Browse

Read-only exploration of what Graphite can do. Use this mode when the user has
not committed to a graph, asset, or recipe.

- Do: call `node.list_types` first, then `node.describe` for the 2-3 most likely
  identifiers, then summarise the options in prose with one `recipes.list` if a
  recipe would shortcut the work.
- Do not: open a document, write a graph, or render. Browsing is side-effect free.
- Stop gate: the user has chosen a concrete path. Switch to References, Original,
  or Supplied-graph.

## Supplied-graph

The user has handed you a graph (file path or in-memory). All edits go through
`document.open` first.

- Do: open with `document.open`, call `graph.list_nodes` to read the current
  state, and route every change through `history.begin` / `set_input` /
  `connect` / `commit` (or `abort`).
- Do not: assume the structure. The shipped graphs may use node types your
  training set does not remember.
- Stop gate: the graph compiles and `render.preview` succeeds. Switch to
  Production if the user wants the asset written.

## Prompt-review

The user has a graph and wants a critique, an explanation, or a bug-hunt.

- Do: render the current state with `render.preview` and walk node-by-node with
  `graph.get_node`. Reference `node.describe` for any node whose data flow is
  unclear.
- Do not: edit the graph. Review mode is read-only by default; if a fix is
  obvious, ask before switching to Supplied-graph.
- Stop gate: the user agrees with the diagnosis. If a fix is wanted, switch to
  Supplied-graph (apply the patch) or Production (write the result).

## Production

The user wants a final asset: PNG, GIF, or a committed `.gdd`.

- Do: render through `render.preview_gif` first to confirm motion looks right,
  then call `render.export` or `render.export_gif` for the bytes on disk, then
  `document.save` if the user wants the graph itself persisted.
- Do not: skip the preview step. A GIF that renders to disk but never previewed
  is a leak risk — preview is the only place the per-frame wiring is visible.
- Stop gate: every requested artifact is on disk and the user has acknowledged
  the visual result.

## References

The user wants to use or audit the recipes corpus.

- Do: read `graphite://recipe-catalog` once to enumerate the corpus, then call
  `recipes.show` on the chosen id to fetch the card, template, and preset.
  Run `recipes.lint` to audit the corpus if the user asks for a clean check.
- Do not: write to the recipes directory without going through the contributor
  workflow (`recipes-build --update`, lint clean, no diagnostic drift in
  `agent/clippy-baseline.txt`).
- Stop gate: the user has the recipe card they wanted, or the lint report is in
  hand.

## Original

The user has a goal but no starting graph. Build from scratch.

- Do: brainstorm internally, then narrow to 2-4 candidate node types via
  `node.list_types` + `node.describe`. Build the graph in one
  `history.begin`/commit transaction with `graph.add_node` /
  `graph.set_input` / `graph.connect`. Render after every commit to surface
  compile errors early.
- Do not: emit a graph without `render.preview` first. Original mode silently
  fails at compile if a node's input shape is wrong.
- Stop gate: the preview matches the user's intent (or the closest
  recipe in the catalog — switch to References in that case).

## Mode transitions

Modes do not lock. The common transitions:

- Original → References (when an existing recipe covers the goal)
- Supplied-graph → Production (when the patch is approved)
- Production → Supplied-graph (when the export reveals a bug that needs fixing)
- Browse → any (once the user picks a path)

Always state the transition in prose so the user can object.