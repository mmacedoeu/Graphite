# Production mode

Walking a recipe from `recipes.show` through to a committed asset. This is
the canonical end-to-end path for a "ship this" request.

## The path

1. **Choose a recipe** — call `resources/read` of `graphite://recipe-catalog`
   to enumerate the corpus, then `recipes.show` on the chosen id to fetch
   the card, template, and preset.

2. **Realize the recipe** — open or create a document, then build the
   graph inside a `history.begin` / `commit` transaction. Realize the
   bindings from the template's **Composition Plan** or **Animation
   Timeline** section by calling `graph.add_node`, `graph.set_input`, and
   `graph.connect`. See `recipe-adaptation.md` for the binding protocol.

3. **Preview** — call `render.preview` first. If the result looks right
   for a static recipe, skip the GIF and move to step 4. For a motion
   recipe, call `render.preview_gif` instead; the GIF path is the only
   one that exercises `AnimationTimeNode` and the per-frame plumbing.

4. **Export** — call `render.export` for a single PNG or `render.export_gif`
   for an animated GIF. Both writes are confined to `--root`; any path
   outside the root surfaces as `PathOutsideRoot`.

5. **Persist the graph** — call `document.save` to write the graph to a
   `.gdd` under `--root`. The save is independent of the export; the
   export is the asset, the save is the document.

6. **Close** — call `document.close` once the user has acknowledged the
   result. Closing releases the document id; subsequent edits to it fail.

## Transaction discipline

The host's transactions are atomic per document. A `history.abort`
rolls back every change since the matching `history.begin`. Use `abort`
whenever:

- `render.preview` reports a compile error that is not worth fixing
  inline.
- The user wants to back out of an in-progress edit.
- A `set_input` call produces an unexpected `Unauthorized` or shape error
  that suggests the binding is wrong, not the user's intent.

`history.undo` is the cheaper alternative for recent single changes; use
it when only the last edit is in question.

## What `render.preview_gif` actually does

The host opens the document, compiles the graph, then walks
`frames` animation times from 0 to (frames-1) at `fps` frames per
second. For each frame, it constructs a `RenderConfig` with the
animation time set to `frame_index / fps`, executes the executor against
the GPU pipeline, and encodes the resulting raster as one frame of the
output GIF.

The GIF encoder is `image::codecs::gif::GifEncoder` with a fixed speed of
10. The output is a single GIF stream with `Loop = Repeat`; the per-frame
delay is `100 / fps` milliseconds.

For a static recipe (no `AnimationTimeNode`), the GIF renders `frames`
identical copies of the same image. Save the render time and call
`render.preview` once instead.

## Idempotency

`render.export` is idempotent: re-running it overwrites the existing
file with the same bytes (modulo timestamps inside the asset). For
bit-identical reproducibility, render once and stash the result; do not
re-render without a reason.

`document.save` is also idempotent for the same `.gdd` content. The
`.gdd` is a CmRDT document; two saves of an unchanged graph produce
byte-identical output.

## Failure modes and recovery

| Symptom | Likely cause | Recovery |
|---|---|---|
| `render.preview` returns `Unauthorized` | Capability was set to `read`, not `all` | Re-authorize with `--capabilities all` (or whatever the call needs) |
| `render.preview` returns GPU adapter error | The host process has no usable GPU | The conformance suite treats this as `PARTIAL`; surface to the user and pause |
| `render.export` returns `PathOutsideRoot` | The export path escapes `--root` | Resolve the path under `--root`; do not try to escape it |
| `render.preview_gif` renders but motion looks wrong | A per-frame dependency is not wired through `AnimationTimeNode` | Audit the wiring with `graph.list_nodes` and `graph.get_node` |
| `document.save` writes 0 bytes | The host has no access to `--root` | Verify `--root` exists and is writable |
| `recipes.show` returns NotFound | The user gave an alias instead of an id | Look up the id via `recipes.list` |

## Stop gates

Production mode ends when all three are true:

1. The user has acknowledged the visual result of `render.preview` (or
   `render.preview_gif`).
2. The export is on disk at the user's chosen path, inside `--root`.
3. The graph is persisted (`document.save`) if the user wants it.

If any stop gate fires, surface it to the user. Do not auto-promote to a
Production commit without acknowledgement.