# Usage patterns

Per-tool rules that the host enforces and that an agent should respect to keep
its output bounded, transactional, and reviewable.

## Discovery: `node.list_types` and `node.describe`

`node.list_types` is large. Call it once per session and cache the result in
the conversation. Never re-issue it inside a loop.

`node.describe` is the only safe way to know an input's name, type, and
default. Before any `graph.set_input`, call `node.describe` once for the node
you are configuring. The result is the source of truth — never infer input
names from a node's display name or category.

Both tools carry a `_meta.anthropic/maxResultSizeChars` annotation because the
node catalog exceeds most clients' result caps; the annotation lifts the cap
when the host honours it. If the host truncates anyway, page through
`resources/read` of `graphite://node-catalog` instead.

## Discovery: `recipes.*` and the recipe-catalog resource

For the recipes layer, the resource read returns the catalog; `recipes.list`
returns per-recipe summaries; `recipes.show` returns the full card plus the
template and preset as text.

- Read the catalog once via `resources/read` of `graphite://recipe-catalog`
  before calling `recipes.list`. The two must agree; if they diverge, the
  build pipeline is out of sync and `recipes-build` needs to run.
- `recipes.show` is the only path that returns the template prose. Do not
  re-author a recipe from `recipes.list` alone; the list only carries summary,
  category, and counts.
- `recipes.lint` returns a structured issue list. Filter by `level == "error"`
  for hard failures; warnings (`source.missing`,
  `defaults.fps_or_frames_missing`) are acceptable until the asset re-render
  pipeline lands.

## Graph authoring: `graph.add_node`, `graph.set_input`, `graph.connect`

Always edit inside a `history.begin` / `commit` (or `abort`) pair. The host's
transactions are atomic per-document: an `abort` rolls back every change since
the matching `begin`. Use `abort` whenever a `render.preview` reports compile
errors that are not worth fixing inline.

`graph.add_node` allocates a fresh node id; never reuse a returned id from a
prior call. `graph.connect` returns success without data; if the connection
fails to compile at render time, the failure surfaces from `render.preview`,
not from `connect`.

`graph.disconnect` and `graph.remove_node` are also transactional, but
prefer `history.undo` if the change is recent — undo is cheaper and the user
can see the diff.

## Rendering: `render.preview` vs `render.preview_gif`

- `render.preview` is for a single frame at a given `max_dimension`. Use it
  during authoring for fast visual feedback.
- `render.preview_gif` is for motion at a given `fps` and `frames`. Use it
  when the graph has `animation_time` consumers or when a single frame cannot
  show the effect.
- A static graph (no `AnimationTimeNode`, no `RealTimeNode`) does not need a
  GIF — the frames are identical. Save the render time and stick to
  `render.preview`.
- The GIF path is GPU-init expensive. Call it once per authoring session
  unless the graph shape changes; re-render after every commit.

## Export: `render.export` vs `render.export_gif`

`render.export` writes a single PNG (or whatever `format` says). `render.export_gif`
writes an animated GIF. Both are confined to `--root` (INV-12); a path
traversal attempt surfaces as `PathOutsideRoot` and must not be retried by
writing outside the root.

If the export is part of a Production-mode commit, call `render.preview_gif`
first and confirm the user is happy before invoking `render.export_gif`.

## Documents and persistence

`document.new` allocates a fresh id; `document.open` reads a `.gdd` from
`--root`. `document.save` writes a `.gdd` to `--root`. The three are
mutually exclusive in intent — never save a new document that the user did
not ask to persist.

`document.close` releases the document; subsequent edits to its id fail.
Close only after the user has acknowledged the result, because the document
is not recoverable from the host after close.

## History: `history.undo`, `history.redo`, `history.replay`

`history.replay` is the long-history primitive for replaying an external
`.gdd` into a different document. It is not a substitute for `document.open`
if you want to edit the original.

`history.undo` returns `{ changed: false }` when there is nothing to undo;
do not interpret that as a failure.

## Selection and active document

`session.active_document` is a hint: it returns the document the host thinks
the user is editing. Treat it as the default `document_id` for any
single-document call. Always pass an explicit `document_id` when in doubt;
the hint can be stale.

`session.selection` returns the node ids currently selected in the editor.
Use it as the input set for batch edits, but never assume it is non-empty.

## Registry: `registry.apply_delta`, `registry.query`, `registry.merge`

The registry is a separate persistent state. `apply_delta` is the only
mutation; `query` is read-only; `merge` reconciles two registry states. Do
not call `apply_delta` without a corresponding `merge` plan in hand; an
unmerged delta will diverge across sessions.