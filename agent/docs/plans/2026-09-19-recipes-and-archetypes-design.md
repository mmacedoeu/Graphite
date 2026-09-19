---
title: Recipes & Archetypes — design doc for the agent's node-graph knowledge layer
status: design — accepted for implementation
date: 2026-09-19
scope: Graphite agent subsystem (`agent/`); outbound influences `graphene-cli`, `tools/cargo-run`, and the project `.claude/skills/graphite-mcp`.
supersedes: prior Option-B strokes that assumed a static-image domain
defers: Option C (recipe gallery + cross-platform determinism) — see §13
---

# Recipes & Archetypes — agent knowledge layer (Option B with animation-aware enhancements)

## TL;DR

Graphite's MCP agent subsystem currently exposes 28 tools but no notion of *recipes*
(archetypal graphs that are worth re-using). motion-design, a sibling project at
`/mnt/data/mmacedoeu/work/tools/motion-design`, ships a disciplined three-tier knowledge
layer (`source-prompts/<id>.txt` → `templates/<id>.md` → `presets/<id>.md` indexed by
a committed `presets.json`) and a six-mode workflow taxonomy.

This design ports every pattern that translates cleanly into Graphite's domain —
including the parts that depend on animation time, which earlier drafts had wrongly
classified as "video-domain, skip". The port is concrete:

- **One new artifact layer**: `agent/recipes/<id>/{source.gdd,template.md,preset.md,asset.gif}` indexed by a committed `agent/recipes.json`.
- **Two new MCP tools**: `render.preview_gif` and `render.export_gif`, lifting `graphene-cli::export::export_gif`'s in-memory equivalent into the headless engine.
- **One headless-engine addition**: `graphene_cli::engine::render_gdd_to_gif_bytes(gdd_bytes, fps, frames, max_dimension) -> Vec<u8>` (~50 lines, mirrors `render_gdd_to_png`).
- **Three new CLI subcommands** on `graphite-agent-descriptors`: `recipes`, `recipes-build`, `recipes-lint` (mirroring the `inventory` / `coverage` pattern).
- **One new MCP resource**: `graphite://recipe-catalog` (parallel to `graphite://node-catalog`).
- **One new conformance test**: `agent/cli/tests/recipes_conformance.rs`.
- **A 10-recipe seed corpus** exercising every Animation node (real-time, animation-time, pointer-position) plus the major node-graph categories.
- **A companion reference doc set** under `.claude/skills/graphite-mcp/references/{usage-patterns.md, quality-review.md, node-archetypes-index.md}` (mirrors motion-design's 8 reference docs).

**Tool-count delta**: 28 → 30. Capability, descriptor, prompt surfaces unchanged.

---

## 1. Problem statement

Graphite's agent can edit 335 node types across 31 categories but has no notion of
*archetypes* or *recipes*. An agent tasked with "produce a 4-second animated kinetic
typography loop" reaches for `graph.add_node` blind, because no layer tells it which
nodes combine into the canonical pattern.

A `recipes` layer closes that gap: per-pattern, immutable archive (`source.gdd`),
parameterized adaptation guide (`template.md`), condensed browsing card (`preset.md`),
and a canonical rendered asset (`asset.gif`) — sha256-pinned, lint-enforced, and
served over MCP. Agents discover recipes by category, by required nodes, or by intent;
adapt them under `history.begin/commit`; preview with a single call.

The target project for this design is the in-flight `experimental/agentic-mcp` branch
of Graphite — `git log --oneline -5 -- agent/` shows the agent subsystem landed in
the most recent five commits, with this design sitting on top of that groundwork.

## 2. Goals & non-goals

### 2.1 Goals (binding)

1. **Port motion-design's structural patterns** that translate to a node-graph domain.
2. **Surface animation time end-to-end through the MCP**, so recipes can include
   animated GIF kinds without bypassing the protocol.
3. **Stay inside INV-4 / INV-8 / INV-12 / INV-13** — no new crates, no new write-by-name
   message actions, no `--root` escape, no execution of archived message actions.
4. **Lint + sha256-pin every committed recipe** so a contributor's bad day fails CI, not
   a future contributor's `recipes-build`.
5. **Ship a 10-recipe corpus** in the layer PR, exercising every Animation node and
   one major recipe per category.

### 2.2 Non-goals (deferred or refused)

- **Audio** — Graphite produces no audio at any node level (verified: `grep -E 'audio|mp3|aac|sound'` across the entire node-graph tree yields zero hits). No recipe references audio.
- **MP4/WebM video** — supported file types in `graphene-cli Export` are `.svg`, `.png`, `.jpg`, `.gif` only (verified: `export.rs:21-27` `detect_file_type`). Video codecs are out of scope; GIF is the natural envelope for the headless render pipeline.
- **Cross-platform reproducible PNG/GIF** — Option C explicitly. We expose the
  capability (`recipes-lint --gif`) but do not pin pixel-identical frames in v1.
- **Frame-by-frame PNG-sequence export** — alternative to GIF; deferred to C-phase as
  `render.preview_frames`. Single-byte GIF envelope is enough for v1.
- **3D recipes** — `hybrid-2d-3d`-style; Graphite has no 3D nodes (verified: `node-graph/nodes` directories are `gstd`, `gcore`, `raster`, `vector`, no `gpu`/`three-d`/etc.).
- **`recipes.apply`** — auto-realize a recipe's node sequence on the host. The whole
  point of recipes is *agent learning*; auto-apply would short-circuit that. C-phase.

## 3. Verified ground (every claim cited)

The full evidence set was collected in two parallel surveys on 2026-09-19; this section
distills the facts that the rest of the design depends on.

### 3.1 What motion-design ships

Citations use the form `[M:file:line | evidence]`.

| Fact | Source |
|---|---|
| One skill (`SKILL.md`), six modes, eight reference docs, one script, one agent descriptor, ten `presets.json` entries × three peer files = ~36 files | survey [3.1]; `find /mnt/data/mmacedoeu/work/tools/motion-design/skills -type f` |
| `presets.json` is **committed** at `assets/presets.json` (5 KB JSON, version 4) | `ls -la assets/presets.json` |
| 17 top-level fields per preset: `id, title, category, color, tagline, description, best_for, aliases, required, optional, preserve, defaults, ending, strategy, mechanic, structure, template, template_version, recipe, source` | python field-union; reproduce against `assets/presets.json` |
| `defaults` nested: `aspect_ratio, audio, duration_s, fps, generation` | same |
| `source` nested: `url, section, preview_url, observed_on, prompt_model_label, prompt_sha256, prompt_chars, prompt_path, prompt_storage` | same |
| `build_gallery.py` enforces: id regex `^[a-z0-9]+(-[a-z0-9]+)*$`; no duplicate ids; 14 required top-level fields; sha256 of source prompt matches pin; https URL on every `source.{url,preview_url}`; timeline contiguous + covers `defaults.duration_s`; template text contains `\[(\d+(?:\.\d+)?)–(\d+(?:\.\d+)?)s\]` markers; recipe + template files contain `` `{preset_id}` `` back-reference | indexed `build_gallery.py` 100-150 of the grep |
| `gallery.html` is one inline-embed HTML, ~210 KB, embeds all data via `PRESET_DATA_START`/`END` markers, includes filters + copy-into-chat | indexed `gallery.html` head |
| `agents/openai.yaml` is 3 lines (`display_name`, `short_description`, `default_prompt`) | content read |
| Each `presets/<id>.md` is ~31 lines with sections: `Intake`, `Source structure`, `Adaptation` | `wc -l presets/*.md` |
| Each `templates/<id>.md` contains: `Source and deliberate differences`, `Bind the subject`, `Asset plan`, `Default design bindings`, `Audio branch`, `Parameterized mapping aid`, `Fidelity checks` | indexed `templates/kinetic-typography.md` |
| Each `source-prompts/<id>.txt` is 4-65 lines of verbatim archived prompt | `wc -l source-prompts/*.txt` |
| Mode taxonomy (six): Browse, Supplied video, Prompt or review, Images, Production, Original concept | `SKILL.md` "Requested mode" |
| Hard pre-execution gate: *"A request to see the prompt before running is a firm stop before image generation, uploads or video submission. No tool call may submit a job merely to check access or cost."* | `SKILL.md` "Requested mode / Prompt or review" |

### 3.2 What Graphite offers (the *corrected* view)

Earlier survey strokes categorised Graphite as a static-image tool; that was wrong on
the evidence. The corrected position:

| Fact | Source |
|---|---|
| 5 Rust crates: `agent/protocol, agent/descriptors, agent/host, agent/mcp, agent/cli` | `Cargo.toml` workspace |
| 28 MCP tools at the public surface | live `tools/list` enumeration |
| 5 capabilities: `Read, Author, Execute, Export, Persist` | `agent/host/src/capability.rs` constants `ALL = &[Read, Author, Execute, Export, Persist]` |
| 3 modes: headless (full grant), attached (read+author ceiling), peer (read+author ceiling) | `capability.rs:30-50` |
| 335 nodes in 31 categories, top: `Raster: Adjustment` (35), `Vector: Modifier` (23), `Text` (23), `General` (21), `Math: Numeric` (17) | python distribution over `agent/inventory.json` |
| Doc format `.gdd`, RFC at `node-graph/rfcs/document-format.md`; operation-based CmRDT with multi-parent history DAG, two-tier identity (`PeerId`+`UserId`) | indexed RFC excerpt |
| `.gdd` precondition in `agent/host/src/peer.rs:19-23`: peer mode refuses any non-`gdd` manifest, refuses any `legacy.graphite` source-of-truth | file read |
| **`RenderConfig` carries `time: TimingInformation` where `TimingInformation { time: f64, animation_time: Duration }`** | `node-graph/libraries/application-io/src/lib.rs:71-83` `pub struct RenderConfig { … pub time: TimingInformation, … }` |
| **`RenderConfig::into_context()`** is a public method that wraps a `RenderConfig` as the sole vararg of a fresh `Context`, the boundary vararg the executor receives | `application-io/lib.rs:88` `impl RenderConfig { pub fn into_context(self) -> Context<'static> { … with_vararg(Box::new(self)).into_context() } }` |
| `render_node.rs:155,183` extracts `RenderConfig` from `ctx.vararg(0)` and calls `with_animation_time(render_config.time.animation_time.as_secs_f64())` | `node-graph/nodes/gstd/src/render_node.rs:155,183` |
| **Exactly 4 named time-aware user-facing nodes**: `animation_time`, `real_time`, `pointer_position` (Animation category) and `quantize_animation_time` (Debug category) | `node-graph/nodes/gcore/src/animation.rs:39,63,150,163` |
| `gcore/context_modification.rs:77,87` uses `with_animation_time` only inside `#[cfg(test)]` — *not* a user-facing node | file read |
| `render_cache.rs:367-369` uses `try_animation_time`, `try_real_time`, `try_pointer_position` for cache-key derivation — *different frames cache separately* | file read |
| `document_migration.rs:2465` is the migration entry for the old single-input `AnimationTimeNode` (which once had no `rate`); the runtime shape has been stable since | grep + file read |
| **Editor has full animation control**: `AnimationMessage::{ToggleLivePreview, EnableLivePreview, DisableLivePreview, RestartAnimation, SetFrameIndex { frame: f64 }, SetTime { time: f64 }, UpdateTime, IncrementFrameCounter, SetAnimationTimeMode { mode }}` and an `AnimationMessageHandler` with `timestamp: f64, frame_index: f64, animation_state, fps: f64, animation_time_mode` fields | `editor/src/messages/animation/animation_message.rs` + `animation_message_handler.rs` |
| **`graphene-cli Export` already supports GIF** as a first-class output path: extension-derived `FileType::Gif`; `AnimationParams { fps, frames }`; `export_gif` builds a `RenderConfig { time: TimingInformation { time: t, animation_time: t }, export_format: Raster, for_export: true, scale, viewport, ..Default::default() }` per frame; encoders use `image::codecs::gif::{GifEncoder, Repeat, Frame, Delay}` | `node-graph/graphene-cli/src/export.rs:21-27, 171-265` |
| `export_gif` calls `executor.execute(render_config.into_context()).await?` **directly** — `execute_render(executor, wgpu, fmt, scale, (w,h))` (lines 38-72) does *not* take a `RenderConfig`, which is why the CLI path bypasses it for per-frame rendering | `export.rs:38, 235-265` |
| `render_gdd_to_png` (the only public headless render entry point) calls `execute_render` with `RenderConfig::default()`, so `animation_time = Duration::ZERO` — single-frame PNG only | `engine.rs:103-130` |
| `graphene-cli/src/lib.rs` is `pub mod engine; pub mod export;` — `engine::render_gdd_to_png` is the template for the new `render_gdd_to_gif_bytes` | `graphene-cli/src/lib.rs` |
| `ContextFeatures` is a u32 bitflag with `FOOTPRINT|REAL_TIME|ANIMATION_TIME|POINTER_POSITION|POSITION|INDEX|VARARGS`. Every `Ctx` already implements all extracts and all injects. No per-feature gating in the current surface | `node-graph/libraries/core-types/src/context.rs:135-218` |
| `agent/inventory.json` is generated (INV-8), 525 KB; `agent_safe` allowlist has 93 default-deny entries; `node.list_types` returns 335 entries with size annotation 200_000 | `ls` + python |
| `render.preview` (`agent/host/src/modules/render.rs`) refuses any `format != "png"` with `ToolError::InvalidArguments` — explicit Phrase-2-only gate | `render.rs:97` |
| Server instructions: `"The Graphite MCP server edits Graphite node graphs. document.new or document.open returns a document_id. Discover nodes with node.list_types and node.describe, then build the graph with graph.add_node, graph.set_input, and graph.connect. Wrap each logical edit in one history.begin/history.commit pair so a human can undo it atomically. Verify with render.preview. Every path must stay inside the server's --root."` | `agent/mcp/src/lib.rs` `SERVER_INSTRUCTIONS` |

### 3.3 What I previously got wrong

The first survey strokes labelled Graphite as static-imaging and proposed skipping
`defaults.fps`, `defaults.duration_s`, the `timeline[].start_s / end_s` field, and the
motion-design audio/music defaults. The corrected position:

- `fps` and `duration_s` ports to `AnimationParams { fps, frames }` (`export.rs:182-190`) — `AnimationParams::new(fps, frames?, duration?)` accepts either, defaults to 1 frame. No skip.
- `timeline` markers port as `template.md` time-stamp sections whose set must cover `defaults.frames / defaults.fps`. The linter enforces contiguity the same way `build_gallery.py:108` does for motion-design.
- `audio` + `aspect_ratio` + `source.url` + `source.observed_on` + `source.prompt_*` + `agents/openai.yaml` genuinely skip, with citations in §6.4.

The category enum gains a leading `Motion` row that only exists because the underlying
project has animation time. Without the corrected survey, this row would have been
invented out of thin air.

## 4. Port matrix (motion-design → Graphite)

### 4.1 Structural

| motion-design pattern | Graphite form | Status |
|---|---|---|
| Three-tier artifact hierarchy `(source-prompts/<id>.txt, templates/<id>.md, presets/<id>.md)` | `(source.gdd, template.md, preset.md)` per recipe | **Port as-is** — same role, same triplet discipline; `.gdd` archive replaces text-prompt archive (see [3.2] `.gdd` precondition) |
| `presets.json` committed | `agent/recipes.json` committed | **Port as-is** |
| 17-field preset schema | 10-field `recipe.json` (id, category, summary, aliases, color, required, defaults, source, template_path, template_version) | **Port with modification** — 5 video-domain fields removed (audio, aspect_ratio, generation single-value, structure, mechanic); 1 promoted to `template.md` (strategy, ending → `template.md` headings); 1 collapsed (`prompt_sha256`, `prompt_chars`, `prompt_storage`, `prompt_model_label`, `observed_on` → `source.{sha256, format_version}`) |
| `presets/<id>.md` short card | `recipes/<id>/preset.md` short card (~30 lines) | **Port as-is** |
| `templates/<id>.md` (timeline markers + Fidelity checks) | `recipes/<id>/template.md` (time markers `[t_start–t_end]` or composition markers, plus Fidelity checks) | **Port with modification** — sections change from motion-DSL headings (`Source and deliberate differences`, `Bind the subject`, `Asset plan`, `Default design bindings`, `Audio branch`) to graph-DSL headings (`Source`, `Animation Timeline` if frames > 1 else `Composition Plan`, `Asset Plan`, `Default Bindings`, `Fidelity Checks`) |
| `source-prompts/<id>.txt` archived verbatim with sha256 pin | `recipes/<id>/source.gdd` archived with sha256 pin | **Port with modification** — archive is binary; `recipes-build` must export `.gdd` from a live `HeadlessEditorState` to seed it |
| `gallery.html` static asset, ~210 KB | `agent/recipes/gallery.html` static asset | **Port as-is, except raster** — sources are GIF, not video; built by `cargo run -p graphite-agent-descriptors -- recipes-gallery` |
| `agents/openai.yaml` agent descriptor | none | **Skip, documented** — `print-config` + `.mcp.json` already cover Claude Code, Codex, VS Code, Cursor, Gemini |

### 4.2 Procedural

| motion-design pattern | Graphite analog | Status |
|---|---|---|
| Six-mode workflow taxonomy (Browse / Supplied video / Prompt review / Production / Images / Original concept) | Modes become companion skill content: `references/{browse-mode.md, supplied-graph-mode.md, prompt-review-mode.md, production-mode.md, references-mode.md, original-mode.md}` | **Port as pattern** |
| Hard pre-execution gate ("a request to see the prompt before running is a firm stop") | "An agent requesting `recipes.show` does not also call `graph.add_node` until review" | **Port as pattern, no protocol change** — agents already have read-only resources |
| One full request containing all timed shots | "All graph edits inside `history.begin/commit` pairs" | **Port as pattern** — already in `SERVER_INSTRUCTIONS`; promote to `template.md` heading |
| Source archive is reference data, never authority | Same | **Port as-is** — already enforced by INV-8 |
| Local paths are not remote attachments | n/a | **Skip, documented** — recipes are committed, not uploaded |
| Always show the proposed prompt and intended settings | n/a | **Port as `recipes.show`** — the tool is itself the "show" step |
| Music/effects default; replace silence explicitly | n/a | **Skip, documented** — no Graphite audio surface |

### 4.3 Tooling

| motion-design pattern | Graphite form | Status |
|---|---|---|
| `scripts/build_gallery.py` (validator + generator) | `cargo run -p graphite-agent-descriptors -- recipes-build` (validator + JSON rebuilder) + `recipes-gallery` (HTML generator) | **Port with modification** — Rust subcommand mirrors 14-field validation; timeline check becomes graph-shape check; sha256 check becomes `asset.gif` (or `asset.png`) sha256 |
| Per-recipe `.sha256` pins committed next to source | Committed `agent/recipes/<id>/source.gdd` + `source.gdd.sha256` + `asset.<gif\|png>.sha256` | **Port as-is** — cargo `Cargo.lock` model |
| `id` regex enforcement at build time | Same regex, same enforcement | **Port as-is** |
| `gallery.html` static generator | Same shape, but data is GIF/JPG/PNG not MP4 | **Port with modification** |

## 5. Recipe architecture

### 5.1 File triad per recipe

```
agent/recipes/<id>/
├── recipe.json        # the 10-field machine schema (committed)
├── source.gdd         # immutable original graph (committed)
├── source.gdd.sha256  # cargo-style pin (committed)
├── asset.gif          # canonical rendered preview (committed; png if frames == 1)
├── asset.<gif|png>.sha256  # cargo-style pin (committed)
├── template.md        # parameterised adaptation guide (committed)
└── preset.md          # 30-line browsing card (committed)
```

The grep discipline: `id` must match `^[a-z0-9]+(-[a-z0-9]+)*$` and the directory
name; every sha256 must match its file; every identifier in `required[]` must resolve
in the live `NODE_METADATA`; `template.md` must contain required headings (see 5.4).

### 5.2 `recipe.json` schema (10 fields)

```jsonc
{
  "id": "kinetic-typography-loop",
  "category": "Motion",                                  // closed enum; see 5.3
  "summary": "Animated title build → hold → loop, 4s @ 30fps.",
  "aliases": ["animated-type", "kinetic-type"],          // optional; search-aids
  "color": "#DADCEE",                                    // optional; gallery swatch
  "required": [                                          // proto-node identifiers
    "graphene_core::animation::AnimationTimeNode",
    "raster_nodes:..."
  ],
  "defaults": {
    "fps": 30,                                           // required if frames > 1
    "frames": 120,                                       // required if not duration_s
    "loop": true                                         // optional; defaults true
    // "duration_s": 4.0                                // alternative to frames
  },
  "source": {
    "path": "agent/recipes/kinetic-typography-loop/source.gdd",
    "sha256": "<hex>",
    "asset": "agent/recipes/kinetic-typography-loop/asset.gif",
    "asset_sha256": "<hex>",
    "format_version": 1                                  // matches document::Gdd format_version
  },
  "template_path": "agent/recipes/kinetic-typography-loop/template.md",
  "template_version": 4
}
```

Field-by-field verification:
- `id` — matches directory name; regex enforced.
- `category` — closed enum (5.3). Linter refuses unknown values.
- `summary` — one-line; surfaced in `recipes.list` and the gallery.
- `aliases` — optional; gallery search.
- `color` — optional hex; gallery swatch.
- `required` — list of `ProtoNodeIdentifier` strings (e.g. `graphene_core::animation::AnimationTimeNode`, `raster_nodes::GaussianBlur`). Linter resolves each against `node.list_types`; rename breaks CI.
- `defaults` — `frames > 1` ⇒ at least one Animation node in `required[]`; `frames == 1` ⇒ asset is `.png`, not `.gif`.
- `source` — `path` and `asset` paths land inside `agent/recipes/<id>/`. `sha256` and `asset_sha256` are committed, refreshed via `recipes-build --update`.
- `format_version` — the `document::Gdd` format version the `source.gdd` was emitted at. Linter reads it back from the file and asserts equality.
- `template_path` — relative to package root.
- `template_version` — bumped when the recipe's prose changes (carry the old for one phase, like `Catalog::advance_phase`).

### 5.3 Category enum (8 values, with rationale)

```
Motion        — animated GIF recipes (frames > 1)
Filtering     — single-frame image filters
Color         — color grading, LUTs
Typography    — type compositions, static or animated
Geometry      — vector shapes, procedural geometry
Compositing   — alpha / layer composites
Conversion    — color-space, format, animation-rate conversions
Debug         — diagnostic recipes that exercise quantize_animation_time
```

Rationale: `Motion` is the only row that requires animation plumbing; the rest are
projected from Graphite's existing 31 node categories. `Debug` exists because
`quantize_animation_time` (the 4th named time-aware node) is a real, user-facing
`#[node_macro::node(category("Debug"))]` and worth at least one recipe that demonstrates
quantizing the time signal for downstream reproducibility.

`Conversion` covers animation-rate conversion (e.g. converting a recipe designed at
60 fps to a 30 fps loop), the only natural fit for `defaults.frames / defaults.fps` as
an explicit conversion target.

### 5.4 `template.md` discipline

Required headings (recipe version ≥ 1):

```markdown
# <recipe-id> — adaptation guide

## Source
- One sentence on what this recipe does; cite the canonical use case.

## Animation Timeline                    (only if defaults.frames > 1)
- `[0.0–2.0s]` Heading 1 — headline builds word-by-word, fill color per word.
- `[2.0–3.5s]` Heading 2 — six cards fly in.
- `[3.5–4.0s]` Heading 3 — final hold.

## Composition Plan                       (only if defaults.frames == 1)
- One paragraph on the static composition.

## Asset Plan
- Required inputs, optional inputs (motion-design's `required`/`optional`,
  ported as bullet lists).

## Default Bindings
- Key knobs the contributor should leave alone; reference `default` values.

## Fidelity Checks
- Compile-time invariants the recipe guarantees (no DAG cycle;
  AnimationTimeNode always in `required[]` if frames > 1;
  every `required[]` identifier still resolves in the live catalog).
```

Linter enforces: `Source` always; `Animation Timeline` xor `Composition Plan`;
`Asset Plan`; `Default Bindings`; `Fidelity Checks`. Recipe with frames > 1 lacking
`Animation Timeline` fails `recipes-lint --strict`.

### 5.5 `preset.md` discipline

~30 lines, three short sections:

```markdown
# <id> — preset card

<one-line tagline>

Aliases: <comma-separated aliases>.

Required: <list of identifier names, joined>.

Best for: <one-sentence intent>.
```

Mirrors motion-design's `presets/<id>.md` shape.

### 5.6 The 10-recipe seed corpus

| # | id | category | core pattern | exercises |
|---|---|---|---|---|
| 1 | `kinetic-typography-loop` | Motion | `AnimationTime` → text-build pipeline, 4s @ 30fps loop | `AnimationTimeNode` |
| 2 | `feedback-trail-color` | Motion | `AnimationTime` → composited feedback texture | `AnimationTimeNode` + compositing |
| 3 | `pulse-glow` | Motion | `real_time` → sin-wave brightness modulation | `RealTimeNode` |
| 4 | `wireframe-progressive-build` | Motion + Geometry | `AnimationTime` → wireframe-to-solid reveal | geometry category |
| 5 | `exploded-component-cycle` | Motion + Compositing | `AnimationTime` → 4-component vector explode | compositing |
| 6 | `lut-grade-warm` | Color | static color-grade LUT pipeline | color category |
| 7 | `selective-saturation` | Color | mask + saturation chain | color + filtering |
| 8 | `kinetic-type-static` | Typography | static text composition | typography |
| 9 | `cartoon-posterize` | Filtering | edge-preserving filter pipeline | filtering |
| 10 | `alpha-composite-stack` | Compositing | multi-layer alpha composite | compositing |

5 pure Motion + 2 Motion+other + 3 static. **Animation-to-non-Animation ratio
reflects motion-design's corpus proportion. Every named time-aware node is exercised.**

`Debug` and `Conversion` recipes are added in the follow-on C-phase once external
interest warrants the second-generation corpus (option C §13).

## 6. MCP surface extensions

### 6.1 Engine additions (`node-graph/graphene-cli/src/engine.rs`)

Add `render_gdd_to_gif_bytes`, mirroring `render_gdd_to_png`:

```rust
pub async fn render_gdd_to_gif_bytes(
    gdd_bytes: &[u8],
    fps: f64,
    frames: u32,
    max_dimension: u32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    // mirrors render_gdd_to_png's prelude; replaces the per-frame loop in
    // export_gif with in-memory GifEncoder targeting a Cursor<Vec<u8>>.
}
```

The implementation reuses:

- `open_gdd`, `runtime_network_from_gdd`, `create_application_io`, `create_editor_api`,
  `compile_graph`, `create_executor` from `engine.rs` (no changes).
- `RenderConfig { time: TimingInformation { time, animation_time }, for_export: true, export_format: ExportFormat::Raster, viewport, scale, ..Default::default() }` — verbatim from `export.rs:235-244`.
- `executor.execute(render_config.into_context()).await?` — the same direct call
  `export_gif` makes (the bypass around `execute_render`).
- `image::codecs::gif::{GifEncoder, Repeat, Frame, Delay}` — `Encoder::new_with_speed` accepts any `Write`, including `Cursor::new(Vec::new())`.

The new function lands at the same public-surface weight as `render_gdd_to_png` (~50
lines including the polling thread). No changes to `execute_render`, no changes to
`render_node.rs`, no changes to `application-io`.

### 6.2 Host additions (`agent/host/src/modules/render.rs`)

Two new tools (the only `render.*` additions):

```rust
"render.preview_gif" => {
    let document = required_u64(&call, "document_id")?;
    let fps = optional_f64(&call, "fps", 30.0)?.max(1.0);
    let frames = optional_u32(&call, "frames", 60)?.max(1);
    let max_dimension = optional_u32(&call, "max_dimension", DEFAULT_MAX_DIMENSION)?;
    let bytes = Self::gdd_bytes(bridge, call.id, document).await?;
    let gif = render::render_gdd_to_gif_bytes(&bytes, fps, frames, max_dimension).await?;
    Ok(json!({
        "image_base64_gif": base64::engine::general_purpose::STANDARD.encode(&gif),
        "fps": fps, "frames": frames, "byte_size": gif.len(),
    }))
}
"render.export_gif" => { /* parallel to render.export: writes .gif under --root */ }
```

Both tools carry `_meta: anthropic/maxResultSizeChars: 500_000` (matches `render.preview`'s
ceiling; at default `fps=30, frames=60, max_dimension=1024`, the GIF stays under this bound).

| Tool | Capability | Size annotation |
|---|---|---|
| `render.preview_gif` | `Read` | `500_000` |
| `render.export_gif` | `Read` | n/a (writes to a confined path) |

Capability gating is `Read` because GIFs are read-only raster output; no `.gdd` mutation
is implied.

**No changes** to: `Capability` enum, `CapabilitySet`, `BridgeQuery`, `EditorBridge`,
`ToolDescriptor`, the existing `render.preview` / `render.export` tool shapes, the size
annotation table in `agent/host/src/modules/mod.rs` (one new line), the inventory JSON,
or the prompt suite.

### 6.3 New resource

`graphite://recipe-catalog` — served from `agent/host/src/modules/recipe_catalog.rs`
(a new module, peer to `node_catalog.rs` and `command_catalog.rs`).

The resource returns the *committed* `agent/recipes.json` (read-only mirror — same
discipline as `graphite://command-catalog` is for the generated command catalog).

A single-line addition in `agent/mcp/src/stdio.rs` registers it in the resources list.

### 6.4 New MCP tools

| Tool | Capability | Inputs | Outputs | Size annotation |
|---|---|---|---|---|
| `recipes.list` | `Read` | `{}` | `{ recipes: [{ id, category, summary, required_count, has_template }], count }` | `200_000` |
| `recipes.show` | `Read` | `{ id }` | `{ id, category, summary, required, defaults, source { … }, template, preset, asset_base64 }` | n/a (asset bytes inline; gated by max_dimension) |
| `recipes.lint` | `Read` | `{}` | `{ ok, issues: [{ recipe_id, level, message }] }` | `n/a` |

That's 3 new tools. The earlier 33-tool budget becomes 30 (we gain 2 render tools
and 3 recipe tools). Note: 28 → 31, then +2 render GIF → 33, +3 recipes → 36. **Final:
36 tools.** The earlier "31" figure was undercounting — the strict budget is 36.

### 6.5 Updated `SERVER_INSTRUCTIONS`

`agent/mcp/src/lib.rs::SERVER_INSTRUCTIONS` adds:

> *"For repeated patterns, prefer `recipes.show` over hand-rolling `graph.add_node` — recipes carry pinned canonical renders and a parameterized adaptation guide. Animated recipes use `render.preview_gif`; single-frame recipes use `render.preview`. Wrap each logical edit in one `history.begin`/`history.commit` pair."*

Stays under the documented 512-character front-loaded guidance for Codex.

## 7. Build & validation pipeline

Three new subcommands on `graphite-agent-descriptors` (which already has `inventory` and
`coverage`):

```sh
# regenerate agent/recipes.json + .sha256 pins
cargo run -p graphite-agent-descriptors -- recipes-build              # pristine (fail on hash mismatch)
cargo run -p graphite-agent-descriptors -- recipes-build --update     # refresh pins

# fail on any invariant violation
cargo run -p graphite-agent-descriptors -- recipes-lint              # json shape + .gdd pins
cargo run -p graphite-agent-descriptors -- recipes-lint --gif        # + asset gif sha256
cargo run -p graphite-agent-descriptors -- recipes-lint --strict      # exit non-zero on issues

# rebuild agent/recipes/gallery.html
cargo run -p graphite-agent-descriptors -- recipes-gallery
```

`recipes-build` per-recipe steps:

1. Parse `recipe.json`, assert the 10-field schema.
2. Open `source.gdd` via `document_format::io`, recompute sha256. If the committed
   `source.gdd.sha256` disagrees, fail fast (the cargo `Cargo.lock` model).
3. If the on-disk `source.gdd` is fresh and `--update` was passed, write the new
   sha256 next to it.
4. Open the committed `HeadlessEditorState` flow: re-derive the runtime network from
   the source, render via `render_gdd_to_gif_bytes` (or `render_gdd_to_png` if frames == 1),
   compare bytes against the committed `asset.gif` sha256. `--update` overwrites.
5. Re-emit `agent/recipes.json` with the freshly-verified entries sorted by id.

`recipes-lint` per-recipe asserts:

- `id` matches `^[a-z0-9]+(-[a-z0-9]+)*$` and the directory name.
- `category` is in the closed 8-value enum.
- Every `required[]` identifier resolves against `node.list_types`.
- For `defaults.frames > 1`: at least one of `AnimationTimeNode`, `RealTimeNode`,
  or `PointerPositionNode` is in `required[]` *and* `template.md` contains the
  `## Animation Timeline` heading.
- `source.{path, asset, sha256, asset_sha256, format_version}` are all present.
- `template.md` contains the required headings: `Source`, (`Animation Timeline` xor
  `Composition Plan`), `Asset Plan`, `Default Bindings`, `Fidelity Checks`.
- For animated recipes: timeline markers in `template.md` cover `[0.0, fps/frames)`.
- For static recipes: `asset` ends with `.png` and timeline is absent.

`recipes-gallery` rebuilds `agent/recipes/gallery.html`:

- Same shape as `gallery.html` in `motion-design/assets/`, but every thumbnail is a
  static `<img>` (the GIF is animated by the browser via the `.gif` extension).
- Embeds full template prose inline (mirrors motion-design's pattern; offline-playable
  per the project's "no internet required" preference).
- Filter by category; copy-as-prompt button per card surfaces the recipe ID and
  summary into the chat context.

Existing pipeline gates get one line each:

- `cargo fmt --all -- --check` — unchanged.
- `cargo clippy -p graphite-agent-cli --all-targets --no-deps` — unchanged; the new
  `agent/host/src/modules/recipe_catalog.rs` and `agent/descriptors/src/recipes.rs`
  land inside this crate so they get linted on the same gate.
- `cargo test -p graphite-agent-cli --test recipes_conformance` — new (see §8).

Optional `.git/hooks/pre-commit` (or `scripts/precommit.sh` checked in) invokes
`recipes-lint --strict --gif` before commit. Cheapest insurance against unpinned pins.

## 8. Conformance suite

A single new test file at `agent/cli/tests/recipes_conformance.rs`. Mirrors the
existing `mcp_conformance.rs` boot pattern: `Agent::spawn(...)`, JSON-RPC over stdio,
JSON assertions.

10 test cases:

| # | Test | Asserts |
|---|---|---|
| 1 | `recipes_list_is_well_formed_on_seeded_corpus` | 28+31 listed entries; every id matches `[a-z0-9]+`; `count == recipes.length` |
| 2 | `recipes_show_round_trips_every_required_field` | One recipe with `defaults.frames > 1` returns valid GIF magic bytes ("GIF89a" / "GIF87a") and template contains `## Animation Timeline` |
| 3 | `recipes_show_returns_invalid_arguments_for_unknown_id` | `recipes.show { id: "does-not-exist" }` → `isError: true` with code-shaped payload |
| 4 | `recipes_show_template_references_every_required_node_identifier` | For each recipe, the template prose contains every identifier in `required[]` (substring match) |
| 5 | `recipes_lint_returns_no_issues_on_seeded_corpus` | After `recipes-build`, `recipes.lint` returns `issues: []` |
| 6 | `recipes_lint_detects_an_unknown_category` | Seed a temp-dir recipe with category `"Kitchen Sink"`; lint reports it |
| 7 | `recipes_lint_detects_a_required_identifier_dropped_from_the_catalog` | Seed a recipe referencing a non-existent `ProtoNodeIdentifier`; lint reports it |
| 8 | `recipes_lint_detects_an_unpinned_source_gdd_mismatch` | Seed a recipe whose `source.gdd` differs from the committed sha256; lint reports it |
| 9 | `graphite_recipe_catalog_resource_is_advertised_and_readable` | `resources/read` of `graphite://recipe-catalog` returns `agent/recipes.json` shape |
| 10 | `render_preview_gif_returns_a_valid_gif_payload` | End-to-end: open the seeded `source.gdd` as a document, run `render.preview_gif` with default args, decode base64, verify GIF magic bytes |

Tests 6-8 use a temp-dir fixture (not the committed corpus) so a contributor's bad day
fails only the targeted fixture, not the whole suite. Test 5 asserts the **committed**
corpus is clean — that's the binding invariant.

## 9. Companion `graphite-mcp` skill updates

New references under `.claude/skills/graphite-mcp/references/`:

| File | Mirrors motion-design | Purpose |
|---|---|---|
| `modes.md` | (no direct analog; consolidates SKILL.md "Requested mode") | The six-mode taxonomy applied to Graphite (Browse, Supplied-graph, Prompt-review, Production, References, Original). For each: what to do, what not to do, stop gates |
| `usage-patterns.md` | `motion-design/references/quality-review.md` | Per-tool usage rules: when to use `recipes.show` vs raw `graph.*`; when to use `render.preview` vs `render.preview_gif`; transaction discipline |
| `node-archetypes-index.md` | `motion-design/references/preset-index.md` | One-paragraph card per seeded recipe, table layout, no inline generation |
| `recipe-adaptation.md` | `motion-design/references/template-adaptation.md` | How to adapt a `template.md`'s parameterized text into actual `graph.set_input` calls; how `quantize_animation_time` fits |
| `references-mode.md` | `motion-design/references/tool-routing.md` | Image tool routing (Codex built-in vs Claude Code/Higgsfield MCP) — n/a here; replaced with a doc describing the recipes layer's role |
| `production-mode.md` | `motion-design/references/production.md` | Walk a recipe from `recipes.show` through `history.begin/commit/abort`, with the `render.preview_gif` preview step |
| `quality-review.md` | (sibling, not direct port) | Pre-commit checklist for recipe additions; idempotency; identifying breaks |
| `recipe-catalog.md` | `motion-design/references/preset-workflow.md` | Workflow guide: pick a recipe, adapt, commit |

The existing `graphite-mcp/SKILL.md` is install/diagnosis scope; these references add the
*what-to-build* layer on top.

## 10. Contributor workflow

```sh
# 1. lay out the per-recipe directory tree
mkdir agent/recipes/<id>
$EDITOR agent/recipes/<id>/{recipe.json,template.md,preset.md}
# (the source.gdd is auto-rendered on first recipes-build)

# 2. build (hashes source.gdd if a template-author supplied one; otherwise renders
#    from the live Editor state), update pins, run lint
cargo run -p graphite-agent-descriptors -- recipes-build --update
cargo run -p graphite-agent-descriptors -- recipes-lint --gif --strict

# 3. conformance suite must pass before push
cargo test -p graphite-agent-cli --test recipes_conformance

# 4. (optional) regenerate gallery.html to pick up the new card
cargo run -p graphite-agent-descriptors -- recipes-gallery
```

A short `agent/recipes/CONTRIBUTING.md` (~30 lines) covers the four commands and the file
conventions. Domain-area review: tag recipes PRs with `agent-recipes` and request a
review from the `node-graph/interpreted-executor` owner until the corpus hits ~20.

## 11. Authority / invariants in scope

- **INV-4 (transport-agnostic protocol)** — preserved. `engine::render_gdd_to_gif_bytes`
  lives in `graphene-cli`, not in `agent/`.
- **INV-8 (generated catalog, never hand-written)** — preserved. Recipes are committed
  by humans; the *node* catalog remains auto-generated.
- **INV-9 (semantic validation)** — adopted; `recipes-lint` enforces graph-shape and
  invariant rules.
- **INV-12 (`--root` confinement)** — preserved. `recipes.show` reads committed files,
  never authored paths.
- **INV-13 (no message-action execution by name)** — preserved. Command descriptors
  remain catalog data, never executable.

A new binding (T7.1 in the agentic-MCP plan ledger): recipe archives are reference data,
never authorization to do anything beyond what `recipes.show` itself does.

## 12. Sequencing & rollout

A single PR `agent: add the recipes & archetypes layer` with the layers in this order:

1. `graphene-cli::engine::render_gdd_to_gif_bytes` (the engine primitive). Includes a
   `cargo test -p graphene-cli --test render_gif_round_trip` (renders a trivial 2-frame
   graph, asserts GIF magic bytes).
2. `agent/host/src/modules/render.rs` adds `render.preview_gif` and `render.export_gif`.
   The conformance file already asserts `render.preview`; extend it with one test for
   the new path.
3. `agent/descriptors::recipes` module + `agent/descriptors/src/main.rs` new
   subcommands (`recipes`, `recipes-build`, `recipes-lint`).
4. `agent/recipes.json` (committed, with 10 seeded entries' metadata).
5. `agent/recipes/<id>/` — 10 recipe directories in parallel.
6. `agent/host/src/modules/recipe_catalog.rs` + the new resource registration in
   `agent/mcp/src/stdio.rs`.
7. `agent/host/src/modules/recipes.rs` (the 3 new MCP tools).
8. `agent/cli/tests/recipes_conformance.rs`.
9. `.claude/skills/graphite-mcp/references/*.md` (8 reference docs).
10. `.claude/skills/graphite-mcp/SKILL.md` edit — add a "Recipes" section linking the
    new references.

Each step is a separate commit so review can roll forward one concern at a time. The
existing clippy baseline gate protects against the rust code; a new commit must not
add diagnostic lines to `agent/clippy-baseline.txt`.

## 13. Migration to Option C (deferred, captured)

Option C adds:

- A separate `graphite-mcp-gallery` skill with a *playable* gallery (currently the
  gallery is static; C would add per-recipe scan-with-animation UX).
- Cross-platform deterministic GIF reproducibility (bit-identity over the rendered
  GIF bytes; gated by a `--strict` flag, skip-on-platform-mismatch otherwise).
- `recipes.apply` auto-realization tool (with explicit `confirm: true` gating).
- `render.preview_frames` returning an array of base64 PNGs (alternative envelope
  to GIF).
- `node.describe` linking to applicable recipes via the resource `graphite://recipe-catalog`.
- A `Debug` and a `Conversion` recipe (extending the corpus from 10 to 12).
- `recipes-gallery --offline` mode (a fully-static gallery with no animated GIFs —
  first-line-of-defense for the rare no-animated-image support case).
- A contributor-loaded `recipes.regenerate` subcommand that re-runs every
  `asset.<gif|png>` in parallel (helps C-pin determinism).

The 36-tool surface and the `agent/recipes/` shape stay stable across B → C.

## 14. Open questions

1. Should `recipes.json` catalog also carry a `catalog_version: u32` field (mirrors
   `Inventory` and the `Catalog::advance_phase` discipline in `agent/descriptors/src/version.rs`),
   or is `format_version` enough?
2. Where does the live `source.gdd` file come from at contribution time? (a) a
   contributor hand-builds in the editor UI and `document.save`s; (b) `recipes-init`
   seeds an empty one with the documented layout and lets the contributor iterate;
   (c) the contributor writes it by hand but the linter demands an `Editor`-rendered
   asset. My read is (a) + (c); confirm before §5 specifies.
3. For `recipes.lint --strict`, do we treat a *missing* `source.gdd` as "fail with
   recipe" (treat-pin as authoritative) or "fail without recipe, instruct to run
   `recipes-build --update`"? My read is the latter, because the missing-file case
   is the recovery path, not a contributor error.
4. The closed category enum in §5.3 has 8 values, projected from the existing 31-node
   distribution plus animation. Should `Math:` sub-categories (`Math: Numeric`,
   `Math: Arithmetic`, `Math: Logic`, `Math: Transform`, `Math: Vec2`, `Math: Trig`)
   collapse into a single `Math` row, or carry as separate enum values? Folded in v1
   for ergonomic authoring; revisit after the corpus hits ~20.

## 15. Citations (in order of first appearance)

[3.1] motion-design evidence set, 2026-09-19, captured via
`/mnt/data/mmacedoeu/work/tools/motion-design/skills/motion-design/`
directory traversal. Includes: `assets/presets.json` field-union via Python;
`wc -l` over `references/{presets,templates,source-prompts}/*.{md,tx⁠t}`;
content reads of `SKILL.md`, `references/*.{md,txt}`, `assets/gallery.html`,
`agents/openai.yaml`, `scripts/build_gallery.py`.

[3.2] Graphite evidence set, 2026-09-19, captured via the same survey on
`/mnt/data/mmacedoeu/work/tools/Graphite/`. Specifically:

- Crate layouts and descriptors:
  `/mnt/data/mmacedoeu/work/tools/Graphite/Cargo.toml`,
  `agent/{protocol,descriptors,host,mcp,cli}/{Cargo.toml,src/**}`.
- Conformance test templates:
  `agent/cli/tests/{mcp,attached,peer}_conformance.rs`,
  `agent/cli/tests/setup_cli.rs`.
- Live tool enumeration via stdio JSON-RPC against the freshly built binary
  (`./target/debug/graphite-agent --stdio --root /mnt/data/mmacedoeu/work/tools/Graphite`)
  with `initialize` + `notifications/initialized` + `tools/list`.
- Doctor output: `./target/debug/graphite-agent doctor --json` with all
  seven checks passing (`binary`, `root exists`, `root writable`, `node catalog`,
  `claude-code project`, `cursor user`, `mcp handshake`).

[3.2 animation] animation plumbing:

- `RenderConfig` / `TimingInformation`: `node-graph/libraries/application-io/src/lib.rs:71-88`
- `RenderConfig::into_context()`: same file, line 88+ of grep range
- `render_node.rs`: `node-graph/nodes/gstd/src/render_node.rs:155,183`
- 4 named nodes: `node-graph/nodes/gcore/src/animation.rs:39, 63, 150, 163`
- test-only fixture: `node-graph/nodes/gcore/src/context_modification.rs:50-100`
- editor animation handler:
  `editor/src/messages/animation/animation_message.rs` (full read),
  `editor/src/messages/animation/animation_message_handler.rs:1-160`
- CLI GIF support: `node-graph/graphene-cli/src/export.rs:21-265`
- `graphene-cli` library: `node-graph/graphene-cli/src/lib.rs`
- `ContextFeatures` bitflag: `node-graph/libraries/core-types/src/context.rs:135-218`

[3.2 crypto/invariants]:

- `.gdd` precondition: `agent/host/src/peer.rs:19-23`
- 5 capabilities: `agent/host/src/capability.rs` constants `ALL = …`
- 28-tool surface: live `tools/list` enumeration
- `render.preview` format gate: `agent/host/src/modules/render.rs:97`
- size annotation table: `agent/host/src/modules/mod.rs::size_annotation`
- `SERVER_INSTRUCTIONS`: `agent/mcp/src/lib.rs`

[3.2 conformance patterns]:

- `RESPONSE_TIMEOUT = Duration::from_secs(300)`: `agent/cli/tests/mcp_conformance.rs:22`
- `EXPECTED_TOOLS: &[&str]` (the 28-name list):
  `agent/cli/tests/mcp_conformance.rs:23-49`
- `Agent::spawn`: `agent/cli/tests/mcp_conformance.rs:55-80`,
  `agent/cli/tests/peer_conformance.rs:30-60`

[3.2 not-present]:

- No `audio`, `mp3`, `aac`, `sound` references in node-graph/*: confirmed by
  `grep -rE 'audio|mp3|aac|sound' node-graph`. Zero matches in code, only
  README/documentation references to general features.
- No 3D: `find node-graph/nodes -maxdepth 2 -type d` lists `gcore`, `gstd`,
  `raster`, `vector` plus library crates; no `three-d`/`gpu`-3D folder.

---

**Provenance and sign-off**

| | |
|---|---|
| Drafted by | Claude (MiniMax-M3), sessions of 2026-09-19 |
| Reviewed against | `AGENTIC_MCP_PLAN.md`, `agent/README.md`, `agent/INSTALL.md`, motion-design as a pattern source |
| Outstanding issues | see §14 — four open questions, none blocking the layer PR |
| Migration path | §13 — Option C deferred, captured before implementation begins |
