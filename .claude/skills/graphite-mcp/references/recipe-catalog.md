# Recipe catalog workflow

End-to-end guide: pick a recipe, adapt it to the user's intent, commit
the result. This is the user-facing path; for contributor work, see
`quality-review.md`.

## Pick

1. Read the catalog once per session via `resources/read` of
   `graphite://recipe-catalog`. The catalog returns every recipe's
   summary, category, and required-count, so the agent can pick a
   candidate before any other call.

2. If the catalog is empty, the host's CWD is not a Graphite workspace.
   Stop and ask the user where the recipes corpus is rooted.

3. Pick by category first, then by summary. The `node-archetypes-index.md`
   reference is a navigation aid that names each recipe's intent in one
   paragraph.

4. If no recipe fits, switch to Original mode and build from scratch. Do
   not stretch a recipe into a domain it was not designed for.

## Show

Call `recipes.show` on the chosen id:

```json
{
  "id": "alpha-composite-stack",
  "recipe": { "id": "...", "category": "Compositing", ... },
  "template": "# ... adaptation guide\n## Source\n...",
  "preset": "alpha-composite-stack ..."
}
```

The response carries the structured card plus the on-disk `template.md`
and `preset.md`. Treat the template as authoritative for the bindings,
the preset as the marketing card.

If the user gave an alias (e.g. `layer-stack`) instead of the id, look
up the id via `recipes.list` first. `recipes.show` accepts only the id.

## Adapt

Realize the recipe's bindings into a graph. Open or create a document
first:

```json
{"tool": "document.new", "arguments": {"name": "user-asset"}}
```

Then build inside a `history.begin` / `commit` transaction. Each binding
from the template's **Composition Plan** (static) or **Animation
Timeline** (motion) section becomes one or more `graph.add_node`,
`graph.set_input`, and `graph.connect` calls. See
`recipe-adaptation.md` for the binding protocol.

For per-node input names, call `node.describe` once per `required[]`
node. Never infer input names from a node's display name or category.

## Preview

Static recipe: `render.preview` once. The result should match the
recipe's intent; if not, audit the wiring with `graph.list_nodes` and
`graph.get_node`.

Motion recipe: `render.preview_gif` once. The GIF is the only artifact
that exercises the per-frame plumbing, so it is the binding check for
motion. If the GIF looks wrong, audit `AnimationTimeNode` (or
`RealTimeNode`) and the per-frame dependencies.

For a static recipe, do not render a GIF — the frames are identical and
the render is wasted time.

## Export

`render.export` for a single PNG (or `render.export` with a `format`
other than PNG if the host supports it). `render.export_gif` for an
animated GIF. Both writes are confined to `--root`; any path outside
the root surfaces as `PathOutsideRoot`.

If the user wants the graph itself persisted, call `document.save` with
a `.gdd` path. The save is independent of the export.

## Commit

The MCP host does not have a `recipes.commit` or `recipes.apply`. The
commit is a graph-level operation: `history.commit` finalizes the
transaction the agent built during adaptation.

After commit, call `document.close` once the user has acknowledged the
result. Closing releases the document id.

## Persistence

The recipes layer is committed to the repository as
`agent/recipes/<id>/`. The catalog `agent/recipes.json` is generated
from those directories by `recipes-build`. Editing a recipe is a
contributor task; an agent that needs a one-off adaptation should
override the binding in the user's working document, not in the
committed recipe.

## When to use which mode

| The user wants... | Mode |
|---|---|
| A list of recipes | References |
| The full card for one recipe | References |
| A recipe realized as a graph | References → Supplied-graph (or Original if no recipe fits) |
| A rendered asset from a recipe | References → Production |
| A new recipe added to the corpus | (contributor task — see `quality-review.md`) |
| An audit of the corpus for errors | References (call `recipes.lint`) |

## Stop gates

Stop when all of the following are true:

1. The user has acknowledged the preview (or skipped the preview).
2. The export is on disk at the user's chosen path, inside `--root`.
3. The graph is committed (`history.commit`) and the document is closed
   (`document.close`).

If any stop gate fires, surface it. Do not auto-promote to a commit
without explicit acknowledgement from the user.