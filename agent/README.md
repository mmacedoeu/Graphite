# Graphite Agentic MCP — `agent/`

Internal scaffolding that exposes Graphite to external agents through the Model
Context Protocol. This directory is **not** intended for upstream contribution
while the feature is under construction; do not open PRs with agent-written code
(`website/content/volunteer/guide/`).

## Layering (one direction only)

```
agent/cli  ──▶ agent/mcp  ──▶ agent/host  ──▶ agent/descriptors  ──▶ graphene-std / editor
                     │              │                                    │
                     └──────────────┴──▶ agent/protocol   (leaf; no editor, no MCP)
```

- `protocol` — the frozen transport-agnostic contract (§5.1). Leaf crate: no
  `editor`, no MCP, wasm-safe.
- `descriptors` — derives the node catalog from `NODE_METADATA`, generates the
  command catalog from the live message-action enumeration, and classifies
  allowlisted names.
- `host` — `ToolHost`, the curated tool modules, the session-mode bridges, the
  command-catalog read path, and path confinement.
- `mcp` — the transport adapter. It never touches `editor`.
- `cli` — the `graphite-agent` binary plus the conformance tests.

Rules that are easy to get wrong:

- **Never depend on `agent/host` from `agent/descriptors`.** The dependency
  direction is one-way; the reverse edge is a Cargo cycle (HIGH-1).
- **Never hand-write a node-type descriptor.** Every `node.type.*` entry is
  generated from `NODE_METADATA` by `graphite_agent_descriptors`. If a node is
  missing from the catalog, the node crate is not linked — fix the linkage, do
  not add a literal.
- **Generated `node.type.*` and `command.*` entries are catalog data, not MCP
  tools.** Only curated operations plus `node.list_types` / `node.describe`
  appear in `tools/list` (§6, R2-F, INV-8/E-8).

## Why `shader-nodes` is enabled

`NODE_METADATA` is populated by `#[ctor]` registrations: it is filled only if the
node crates are actually linked into the binary. `graphene-std`'s
`shader-nodes` feature matches what the editor's GPU consumers link, so the
generated catalog is the same one the editor can actually run. Depending on
`core-types` alone yields an **empty catalog**. The precedent for this feature
selection is `desktop/wrapper/Cargo.toml`'s `gpu` feature — *not*
`tools/node-docs`, which builds the website catalog without shader nodes (M-B6).

An empty catalog is treated as a build error: `node_descriptors()` asserts
non-empty.

## `agent_safe` allowlist policy (T3.1)

`agent/allowlist.toml` is **default-deny**. A name is agent-safe if and only if it
literally appears in the `agent_safe` array; there are no subjective rules, score
thresholds, or heuristics (R3-A).

The array holds two kinds of entry:

1. **Curated MCP tool names** (no prefix), e.g. `document.new` — the §17 surface.
   These keep working exactly as in Phase 2.
2. **Generated message-action names**, always `command.<normalized global_name>`.

### The one naming rule (T3.1, §6.3)

An action's allowlist entry **is** its generated command descriptor name:

1. take the action's `AsMessage::global_name()` (e.g. `Portfolio.Document.Undo`);
2. replace `::` with `.`, lowercase, replace every character outside `[a-z0-9.]`
   with `_`;
3. prefix `command.` → `command.portfolio.document.undo`.

So "is this action allowlisted?" is exactly
`classify::is_agent_safe(commands::command_name(action.global_name()))`, exposed
as `classify::is_action_agent_safe`. `classify::is_agent_safe` stays pure
default-deny membership.

**`agent_safe` is descriptive (E-8).** Command descriptors are catalog data, never
MCP tools, and executing a message action by name is forbidden (INV-13). The
allowlist therefore only drives classification and `coverage`, not execution.
`agent/inventory.json` keeps its Phase 0 shape — keys `nodes` and `agent_safe` —
and simply lists the expanded allowlist.

## Enumerating message actions (T3.2)

`Dispatcher::collect_actions()` only advertises document/tool actions when a
document is active (M-A3). `agent/descriptors/src/actions.rs` therefore:

1. constructs the process's one headless `Editor` via `Editor::new_headless`
   (INV-11: exactly one `Editor` per process; the call is cached in a `OnceLock`);
2. dispatches `PortfolioMessage::NewDocumentWithName`;
3. calls `editor.dispatcher.collect_actions()`.

`agent/descriptors` builds this editor itself and **never** depends on
`agent/host` (HIGH-1). It needs a concrete `ResourceStorage`, so it depends on
`graph-craft` (already a workspace dependency) for `HashMapResourceStorage`; the
host's `HeadlessBridge` is in `agent/host`, so using it here would be a cycle.

## Command descriptors are catalog-only (T3.3, E-8)

`agent/descriptors/src/commands.rs` turns the action list plus the field metadata
the macros already emit into one `ToolDescriptor` per action, named per §6.3.
Command descriptors:

- never enter `tools/list`, the host's `module_index`, or any
  `ToolModule::descriptors()`;
- are never executable by name (INV-13);
- have exactly two read paths: the `coverage` subcommand and the read-only
  `graphite://command-catalog` MCP resource (§11).

### Enforcing INV-11 on the resource path

`graphite://command-catalog` must not construct a second `Editor` in the server
process. `HeadlessBridge::open` therefore generates the catalog from the editor it
already owns and installs it (`agent/host/src/modules/command_catalog.rs`); the
MCP adapter only reads the installed value. If nothing was installed, the resource
returns a valid empty catalog instead of panicking.

Because the host owns no document at construction time, the live resource exposes
the actions advertised before a document exists; the **canonical** active-document
catalog (including tool/document actions) is what `coverage` reports from the
descriptors binary's own editor.

## T3.4 finding — the macro is sufficient

The existing derive needs **no extension**. `HierarchicalTree` already emits
`"name: RustType"` field strings for named variants, reachable through the public
`Message::message_tree()`, and `#[message_handler_data]` / `ExtractField` emit
handler/context fields the same way. `commands.rs` consumes them directly.

Two caveats are handled outside `proc-macros` (recorded, not discovered):

1. The metadata is Rust **source-type strings**, so `commands.rs` owns a small
   string→JSON-Schema mapper (`source_type_schema`). The `type_to_schema` in
   `lib.rs` maps `graphene_std::Type`, not source strings.
2. `HierarchicalTree` only recurses into payload type names ending in `Message`;
   nested non-message payload structs (e.g. `DVec2`, `Key`) stay opaque and become
   `x-rust-type`, not a structural schema.

## Catalog versioning and deprecation (T3.5, T3.7)

`agent/descriptors/src/version.rs` implements §5.6:

- descriptors start at version `1`;
- adding an optional argument keeps the version and updates `input_schema`;
- removing/renaming an argument or changing the result shape bumps by 1 and keeps
  the old descriptor for exactly one phase (`Catalog::advance_phase` drops it once
  the grace phase elapses).

Versioning is catalog-side only: it never edits `agent/protocol` or `agent/mcp`,
and `coverage` exposes the current `catalog_version`.

## Coverage (T3.6)

`cargo run -p graphite-agent-descriptors -- coverage` reports generated-vs-total
for nodes and for allowlisted message actions (both 100%), lists every
allowlisted action name, and lists every non-allowlisted message with its status
(`not-allowlisted`).

## Build and test

```sh
cargo build -p graphite-agent-protocol -p graphite-agent-descriptors -p graphene-cli
cargo test  -p graphite-agent-protocol -p graphite-agent-descriptors
cargo run   -p graphite-agent-descriptors -- inventory --out agent/inventory.json
cargo run   -p graphite-agent-descriptors -- coverage
cargo test  -p graphite-agent-cli --test mcp_conformance
```

**Never** run bare `cargo run` at the repository root — it invokes
`tools/cargo-run`. Always pass `-p`.
