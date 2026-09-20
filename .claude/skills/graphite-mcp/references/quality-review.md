# Quality review

Pre-commit checklist for adding or editing a recipe, plus rules for keeping
the corpus stable as the host evolves. This is a contributor's reference,
not an agent's — agent code does not edit recipes.

## Adding a recipe

1. Pick the directory name to match the id: `mkdir agent/recipes/<id>/`.
   The id must satisfy `^[a-z0-9]+(?:-[a-z0-9]+)*$`; the conformance test
   `recipes_list_is_well_formed_on_seeded_corpus` rejects anything else.

2. Author `recipe.json` with the eight documented fields: `id`,
   `category`, `summary`, `aliases` (optional), `required[]`,
   `defaults{}`, `source{path,asset,format_version}` (or empty for a
   prose-only recipe), and `template_path` + `template_version`.

3. Author `template.md` with all five headings, in order: **Source**,
   **Composition Plan** (static) or **Animation Timeline** (motion),
   **Asset Plan**, **Default Bindings**, **Fidelity Checks**. The lint
   rejects any missing heading.

4. Author `preset.md` — a short marketing card. The lint does not enforce
   content here, but `recipes.show` returns the preset verbatim, so make
   it presentable.

5. If the recipe is a static recipe that should compile at
   `frames = 1`, prefer the static template; if it loops, use the motion
   template with `defaults.frames` and `defaults.fps` set.

6. Run `cargo run -p graphite-agent-descriptors -- recipes --root
   agent/recipes --out agent/recipes.json`. The committed catalog must be
   regenerated whenever a recipe changes.

7. Run `cargo run -p graphite-agent-descriptors -- recipes-lint --root
   agent/recipes --strict`. The seeded corpus must lint clean.

8. Run `cargo test -p graphite-agent-cli --test recipes_conformance`. All
   six tests must pass — the conformance suite is the binding invariant
   for the corpus.

9. Commit with a descriptive message naming the recipe id and category.
   No code-style backticks in the message body; the commit hook refuses
   them.

## Editing an existing recipe

The same flow, scoped to one directory:

1. Edit `recipe.json` (or `template.md` or `preset.md`) under
   `agent/recipes/<id>/`.

2. Re-run `recipes-build` to refresh `agent/recipes.json`.

3. Re-run `recipes-lint --strict` and the conformance suite.

4. Commit. Do not squash into a single commit if the change spans more
   than one recipe.

## Stamp discipline

The first `recipes-build` writes `source.gdd.sha256` and
`asset.<gif|png>.sha256` into the recipe's `source{}` block.
Subsequent builds fail on mismatch unless `--update` is passed.

- `--update` re-renders the source `.gdd` and re-stamps the new hash.
- Without `--update`, a drift between the rendered asset and the
  recorded hash is treated as a hard error. This is intentional: an
  unannounced asset change is the most common way a recipe stops
  matching its preview.

The seeded corpus today carries no `source.gdd` and no asset; the lint
reports `source.missing` and `defaults.fps_or_frames_missing` as
warnings, not errors. This is the designed-as-correct interim state; the
asset re-render pipeline is a follow-on.

## Common breaks

- A recipe's `required[]` names a node type that no longer exists in
  `NODE_METADATA`. The conformance suite's
  `recipes_show_template_references_every_required_node_identifier`
  asserts every required identifier appears in the template; it does
  not assert the identifier is still in the catalog. Catalog drift
  surfaces only at `render.preview` time. Run `recipes.lint` and check
  the live `node.list_types` output before promoting a recipe.
- A recipe's `template.md` loses a heading. The lint catches this with
  a `missing_heading` issue at Error level.
- A recipe's id is changed after commit. The id must match the directory
  name; the lint catches `id_mismatches_dir` at Error level. Changing an
  id is a breaking change to anyone reading `recipes.list`; do not do it
  unless you are also retiring the old id.
- A new field appears in `recipe.json` and the schema is not updated.
  The conformance suite does not assert a schema version. Today, every
  recipe carries `format_version: 1`; if you bump it, the schema needs
  a corresponding bump and a migration note.

## Diagnostic drift

The clippy baseline (`agent/clippy-baseline.txt`) is a guardrail. A new
commit must not add diagnostic lines; if it does, the CI gate fails.
This is mostly a Rust concern, but a recipe that triggers new clippy
warnings in the descriptors crate (for example by adding a `clone` that
the baseline missed) is a regression.

Run `cargo clippy -p graphite-agent-descriptors --all-targets -- -D
warnings` locally before committing; the host's CI runs the same gate.

## Identifying breaks

When a recipe breaks after a host change, walk this checklist:

1. Does `recipes.show` still return a card? If not, the catalog is out
   of date. Re-run `recipes-build`.
2. Does the template still name every `required[]` node? If not, the
   template has drifted; the conformance test catches this.
3. Does `render.preview` on the source `.gdd` succeed? If not, the
   recipe's graph is no longer compatible with the current
   `NODE_METADATA`. Update the recipe or retire it.
4. Does `render.preview_gif` (for motion recipes) match the asset
   on disk? If not, the source and asset are out of sync; re-render.

The conformance suite covers the first two; the latter two are run-time
checks that the contributor must verify before commit.