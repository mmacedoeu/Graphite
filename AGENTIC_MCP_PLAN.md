# Graphite Agentic MCP — Implementation Plan

**Version:** 1.13 (rounds 1–9 adversarially reviewed and corrected; 1.10–1.13 record execution findings E-1…E-17 from implementing Phases 0–5)
**Audience:** substrate worker agents executing one phase at a time.
**Status:** prescriptive. Do not redesign. Every open choice has already been made below.

### Revision history

- **1.1** — Round-1. Tool-module/bridge cycle; client-supplied capability; arbitrary `Message` injection; missing pump; wrong paths; non-importable `graphene-cli` export; dialog-bound save; Phase 0 binary; tool assertion; tsify; `DocumentId`; one-Editor-per-process; subjective `agent_safe`; missing history tool; `NODE_METADATA` linkage; no-op `Wake`; 12 MEDIUM.
- **1.2** — Round-2. Node-id list-diff (R2-A); reply-leak into `FrontendMessage` (R2-B); `DocumentQuery` acquisition (R2-C); multi-document addressing (R2-D); `SetInput` decode (R2-E); catalog-vs-tools (R2-F); borrow split (R2-G); `Platform::Desktop` caveat (R2-H); mount messages (R2-I); typed document extraction (R2-J); pump re-dispatch (R2-K); document-id discovery (R2-L); resources/prompts tasks (R2-M); single-editor test isolation.
- **1.3** — Round-3. Inventory surfaces unreachable (R3-A); `collect_actions` path (R3-B); pump snippet crate paths (R3-C).
- **1.4** — Round-4. Async `pump` contradicting sync trait (CRIT-A1); contract could not express document lifecycle/persistence (HIGH-A2); pump treated `"No active document"` as fatal (HIGH-A3); cancellation unreachable while `call` borrowed `&mut self` (HIGH-A4/B-M9); async `export_gdd` on a sync trait (HIGH-B1); `CARGO_BIN_EXE` in the wrong package (HIGH-B2); QueryId allocator ambiguity (M-A5); write scopes missing manifest/registration files (M-A6/B-M2); `collect_actions` incomplete without an active document (M-A7); `HeadlessEditorState` undefined (M-A8/B-M7); `shader-nodes` feature not enabled (M-A9); `jsonschema` not approved (M-B3); HTTP deps not approved (M-B4); socket is fire-and-forget and `interprocess` unapproved (M-B5); `agent/host` deps incomplete (M-B6); single-editor test not run by its gate (M-B8).
- **1.5** — Round-5. `agent/host` ↔ `agent/descriptors` Cargo cycle in T3.2 (HIGH-1); frozen §5.1 types lacked `PartialEq`, which `Message` requires, so Phase 1 could not compile (HIGH-2); dependency matrix incomplete and non-workspace crates incorrectly marked `{ workspace = true }` (M-1); `graphene_resource::ResourceStorage` is not a direct `editor` dependency (M-2); `Snapshot.document = None` also covers `ActiveDocument` (M-3); `DocumentOperation::Open` path-vs-bytes contradiction (M-4); Appendix D Phase 4 omitted `desktop/src/app.rs`/`event.rs` (M-5); `tools/node-docs` was cited as the `shader-nodes` precedent but does not enable it (M-6).
- **1.6** — Round-6. `ToolModule::execute` returned a future bound to `&mut self` but captured `bridge`; no conforming impl could compile, fixed with a shared `'a` lifetime (R6-C1); the normative pump snippet's `FutureMessage::Wake.into()` was type-ambiguous, removed `.into()` (R6-M1); §5.2 still said `Snapshot.document = None` was only for `DocumentList` (R6-M2); Phase 3/4/5 gates did not run the tests their tasks add (R6-M3); §21 allowed Phases 4 and 5 in parallel despite both editing `agent/host` registration files (R6-M4); the three MCP prompts had no content, now defined verbatim in §11.1 (R6-M5).
- **1.7** — Round-7. `session.selection` had no projection and Phase 4 could not edit `agent/protocol`/`editor`; `SnapshotProjection::Selection` and `DocumentQuery::selection` now land in Phase 1 (R7-H1); `tokio/io-util` was not granted though the stdio adapter needs it (R7-M2); the shipped binary had no mode selection, leaving Modes B/C and HTTP unreachable outside tests — added `--mode`/`--attach`/`--gdd`/`--http`, `Host::new`, wiring tasks T4.10/T5.7, and launch assertions in the Phase 4/5 gates (R7-M3).
- **1.8** — Round-8. `ToolHost::events` prescribed a tokio broadcast receiver, which is not a `futures::Stream`; specification now requires `futures::channel::mpsc` (R8-H1); MCP `logging` capability was missing while events use `notifications/message` (R8-M1); `--capabilities` default contradicted itself between T2.10 and T4.7 (R8-M2); Phase 5 gate demanded byte-identical serialization of `HashMap`-backed registry (R8-M3); the new attached/peer launch commands omitted the required `--root` flag (R8-M4); §4.3 listed `AgentReplySink` as a protocol export though it is editor-local (R8-M5).
- **1.9** — Round-9. `run_node_graph()` is `!Send` (it holds a `std::sync::MutexGuard` across `.await`), so the `+ Send` bounds on `pump`/`execute`/`PendingCall` made them unimplementable; removed, and the host is pinned to one dedicated current-thread runtime (R9-H1); the attached socket had no event frame, so `drain_events`/`DocumentChanged` had no transport — frames are now `Request`/`Response`/`Event` (R9-M1); `agent/mcp` lacked `futures` to poll `ToolHost::events()` (R9-M2); the render path had no GPU context or `.gdd`→runtime conversion — T0.3 now exposes `engine::render_gdd_to_png` (R9-M3).
- **1.10** — Execution findings (E-x), recorded while implementing Phases 0–2. **E-1:** every `cargo clippy … -- -D warnings` gate was unpassable, because `-D warnings` also reached out-of-scope *dependency* workspace members and the current stable clippy flags a pre-existing `proc-macros` lint; all clippy gates now pass `--no-deps` (§3.1). **E-2:** `jsonschema`'s current release line is 0.56 (the descriptors dev-dependency; `jsonschema::validator_for` is the stable schema-wellformedness entry point), and `tiny_http` resolves to 0.12 for Phase 4. **E-3:** §5.2's `DocumentQuery::active_document` is shadowed on `PortfolioMessageHandler` by an inherent `active_document() -> Option<&DocumentMessageHandler>`; call it fully qualified as `DocumentQuery::active_document(portfolio)`. **E-4:** the clippy gate is baseline-relative, not `-D warnings` — the `editor` crate already reports 13 pre-existing diagnostics in files outside every phase's write scope, recorded in `agent/clippy-baseline.txt`; a phase must add no `file:line` entry and introduce no diagnostic in a file it creates (§3.1, every phase gate). **E-5:** §5.4's pump loop as first written skipped `handle_message(FutureMessage::Wake)` whenever `run_node_graph` reported no further work, so an async `ExportGdd` reply was never drained; the wake drain now runs unconditionally at the top of every iteration (§5.4). **E-6:** `Host::new` also needs the INV-12 confinement `root`, giving `Host::new(bridge, capabilities, timeout, root)` (T2.2). **E-7:** Phase 2's `render.export` is PNG-only: `graphene_cli` exposes no public byte-returning SVG entry point and `node-graph/**` is out of scope, so non-PNG `format` values return `InvalidArguments` (T2.7). **E-8:** T3.3's "command **tools**" was ambiguous; a read-only survey of the plan and the implemented code resolved it as **catalog-only, no contradiction**. INV-8 is restated to say generated node-type and command descriptors "are never MCP tools"; §6.3 now fixes command naming (`command.<global_name lowercased/dotted>`), the source-type-string and non-`Message`-payload caveats, and the `graphite://command-catalog` read path; T3.3, T3.6, the Phase 3 gate (new item 7) and Appendix D now all state that a command descriptor must never enter `tools/list`, the host's `module_index`, or a `ToolModule`, because dispatching an arbitrary `Message` remains forbidden (INV-13). `collect_actions()` returns payload-free discriminants and needs an active document; the `HierarchicalTree` derive already emits `"name: RustType"` field metadata, so T3.4 needs no `proc-macros` change. Also verified while building: `futures`' `UnboundedReceiver::try_next` returns `Err(TryRecvError)` for "currently empty" (unlike tokio); a new document's working copy is mounted, so `ExportGdd` round-trips in-process; and `render.preview` produced a real PNG in this environment (no PARTIAL was needed).

- **1.11** — Phase 3 execution findings. **E-9:** `agent/descriptors` also needs `graph-craft` (§4.1): T3.2's `HeadlessEditorState.resource_storage` requires a concrete `ResourceStorage` and `HashMapResourceStorage` is not reachable through `graphene-std`. **E-10:** §6.3's illustrative command name was wrong — the naming rule applies to the runtime `global_name()` (parent `Message` enum variants), e.g. `Portfolio.Document.NodeGraph.DeleteNodes` → `command.portfolio.document.nodegraph.deletenodes`; also, `NodeGraph` actions are only collected while the graph-view overlay is open. **E-11:** the `graphite://command-catalog` resource is generated from the host's **own** `Editor` (never a second one — INV-11), so before any document is open it exposes only the document-less action subset; the canonical active-document catalog is what `coverage` reports. Allowlist rule adopted: an action's allowlist entry **is** its command descriptor name, so `classify::is_action_agent_safe(global_name)` is `is_agent_safe(command_name(global_name))`; `agent_safe` stays default-deny and remains descriptive, never an execution gate (E-8).
- **1.12** — Phase 4 execution findings. **E-12 (environment):** `graphite-desktop` had never been built here and `cef-dll-sys` downloads a ~1 GB CEF distribution, but the crate turned out to be cached and **`cargo check -p graphite-desktop --all-targets` passes**, so the desktop-side tasks (T4.1–T4.3) are compile-verified. Two environment caveats remain: the check requires the gitignored, generated `desktop/third-party-licenses.txt.xz` (normally produced by `cargo run -p third-party-licenses --features desktop`, which needs `cargo-about` and npm — neither is installed here; a placeholder was used for the check only), and the **runtime** desktop↔agent live session plus the plan's gate 8 (desktop regression without `--agent-bridge`) are still unexercised because they need a real CEF/GUI process. The agent-side tasks are gated against a scripted server that speaks the identical framing. **E-13:** T4.1's framing is `Request { id, query }` plus a fourth `Cancel { id }` control frame — a bare `Request(BridgeQuery)` cannot be correlated, and `EditorBridge::cancel` (a frozen signature) has no query representation. **E-14:** `--http` mode serves request/response but does not stream `AgentEvent` (no SSE GET stream), so `DocumentChanged` notifications are stdio-only; `--http` and `--stdio` are mutually exclusive in the shipped binary. Also: the desktop cannot derive the active document id without a public editor accessor (`editor/**` is out of scope), so the desktop forwarder attributes `DocumentChanged` from the most recent request's document id.
- **1.13** — Phase 5 execution findings. **E-15:** `registry.merge`'s wire encoding is **RON**, not JSON — a `RegistryDelta`'s `id`/`parent`/`extra_parents` carry `Rev(NonZeroU128)`, which `serde_json` cannot represent, and `ron` needs its `integer128` feature; `registry.apply_delta` remains JSON because it only carries `u64` (§17 specifies no encoding for either, so this is additive). **E-16:** `--gdd <path>` denotes a `.gdd` **working-copy directory** (manifest/registry/history files), matching `Gdd::open` semantics, not a single archive file. **E-17:** the compile sub-step of INV-9's semantic validation only runs when the root network has at least one export; the tested rejection path is caught earlier, by `to_runtime_with_metadata` on an unresolved `ProtoNode` declaration. No `document/graph-storage` change was needed: the existing public `Session` API already provides the typed boundary, and `Host::new` recovers the peer handle through a host-local downcast rather than changing the frozen §5.1 contract.

---

## 0. How to use this document

You are a worker agent. You will be assigned **one phase**. Do this:

1. Read §1–§7 once. They are binding for every phase.
2. Read only your phase section, Appendix D (write scopes), and the phases it depends on.
3. Execute the phase's numbered tasks **in order**. Do not skip, reorder, or merge tasks.
4. Run the **Gate** commands. The phase is done only when every gate check passes.
5. If a gate fails twice, follow §16 (Escalation). Do not improvise a workaround.
6. Edit only files inside your phase's write scope (Appendix D).

**Rule zero:** if you are about to make a design decision, stop. The decision is already in this document. If it genuinely is not, escalate per §16.

---

## 1. Mission and end state

Expose Graphite to external agents through the Model Context Protocol (MCP), built on one shared contract, with three session modes:

- **Mode A (headless):** the host owns an editor instance; no GUI, no human.
- **Mode B (attached):** the host fronts a live editor session a human is also editing.
- **Mode C (peer):** the host participates in the `.gdd` CRDT as an attributed `PeerId`.

A, B, C share one **contract** (`agent/protocol`) and one **descriptor catalog** (`agent/descriptors`). Transport (MCP) is an adapter at the edge.

End state: an external MCP client can list tools, create/open a document, author a node graph, render a preview, and export — without any hand-written per-tool transport code.

---

## 2. Objective function (binding)

In priority order. Earlier items dominate.

1. **Principles are gates, not trade-offs.** A phase that requires reading a message handler's private state outside the typed query interface, bypassing the protocol boundary, or building a parallel abstraction is REJECTED regardless of speed.
2. Minimize time to the first real agent action.
3. Risk must increase monotonically across phases.
4. Every surface reuses the same contract and the same descriptor catalog.
5. No catalog is finalized before a real consumer exercises it.

### 2.1 Non-negotiable invariants

| ID | Invariant |
|----|-----------|
| INV-1 | Tool calls enter the system only through the `agent/protocol` contract. |
| INV-2 | State is obtained only through a **typed query interface** (`DocumentQuery`) or through a correlated `AgentMessage` reply. Direct reads of message-handler private fields are forbidden. |
| INV-3 | `agent/protocol` must not depend on MCP or on the `editor` crate. |
| INV-4 | `agent/mcp` must not depend on the `editor` crate. It depends on `agent/protocol` + `agent/host`. |
| INV-5 | Every mutating tool runs inside an editor transaction so undo is always available. |
| INV-6 | Capability is **host-assigned**, never client-supplied. Each descriptor declares one required `Capability`; the host checks it against the session grant set before executing. |
| INV-7 | No file in `agent/` exceeds 3000 lines. Split before that. |
| INV-8 | Node-type and command descriptors are generated from metadata **as catalog data only — they are never MCP tools**. No hand-maintained node-type list. Only curated operations are enumerated in the one static host registry, and they are the only entries in `tools/list` besides `node.list_types` / `node.describe`. |
| INV-9 | Mutations that could produce an invalid graph are followed by a validation pass (compile or render). |
| INV-10 | In stdio mode, **stdout carries JSON-RPC only**. All logging goes to stderr. |
| INV-11 | **Exactly one `Editor` per process.** `Editor::new` installs a process-global `ENVIRONMENT` and panics on a second call. Multi-session = multiple processes. |
| INV-12 | File-path tools are confined to a configured root directory; paths are canonicalized and prefix-checked. |
| INV-13 | The agent subsystem executes only curated enums (`AgentOperation`, `DocumentOperation`). It must never deserialize and dispatch an arbitrary `Message`. |
| INV-14 | The **host** is the single `QueryId` allocator. Nothing else allocates ids (M-A5). |
| INV-15 | `.gdd` export is **async** and is performed by the document subsystem, not by a synchronous query method (HIGH-B1). |

---

## 3. Repository orientation (read before your phase)

| Symbol / concept | Location |
|---|---|
| Top-level `Message` enum, `Dispatcher` | `editor/src/messages/message.rs`, `editor/src/dispatcher.rs` |
| `Dispatcher::collect_actions` (public; needs an active document for document/tool actions) | `editor/src/dispatcher.rs` |
| Message handler contract | `editor/src/utility_traits.rs` (`MessageHandler`) |
| Adding a child message subsystem | `editor/src/messages/mod.rs`, `editor/src/messages/prelude.rs`, `#[impl_message]` / `#[child]` in `proc-macros` |
| Async work re-entering the dispatcher | `editor/src/messages/future/` (`MessageFuture`, `FutureMessage`, `Wake`) |
| Canonical async pump reference | `frontend/wrapper/src/helpers.rs` (`poll_node_graph_evaluation`) |
| Editor construction / environment | `editor/src/application.rs` (`Editor::new`, `poll_node_graph_evaluation`, `Environment`, `Platform`, `Host`) |
| Node-graph driver | `editor/src/node_graph_executor/runtime.rs` (`pub async fn run_node_graph`) |
| `DocumentId` | `editor/src/messages/portfolio/document/utility_types/misc.rs` (`pub struct DocumentId(pub u64)`) |
| Transactions / history | `editor/src/messages/portfolio/document/document_message.rs` (`AddTransaction`, `CommitTransaction`, `AbortTransaction`, `DocumentHistoryBackward/Forward`) |
| Async `.gdd` export (reference) | `editor/src/messages/portfolio/document/document_message_handler.rs` (`SaveDocument` async closure) |
| Node metadata | `node-graph/libraries/core-types/src/registry.rs`; access via `graphene_std::registry::NODE_METADATA.lock()` |
| Compiler + executor | `node-graph/graph-craft/src/graphene_compiler.rs`, `node-graph/interpreted-executor/src/dynamic_executor.rs` |
| Platform IO (headless default exists) | `node-graph/graph-craft/src/application_io.rs` (`PlatformApplicationIo::default()`) |
| Document format `.gdd` | `document/format/src/` (`Gdd::create_in`, `open_in`, `async export_to_bytes`) |
| Container (`AnyContainer`) | `document/container/src/` |
| CRDT storage + deltas | `document/graph-storage/src/` (`Registry`, `RegistryDelta::RegisterPeer`, `Session`, `PeerId`) |
| Headless engine CLI + export | `node-graph/graphene-cli/src/main.rs`, `src/export.rs` (needs a lib target — T0.3) |
| Desktop IPC socket (currently one-way) | `desktop/src/socket.rs` |
| Metadata generator precedent | `tools/node-docs`, `tools/editor-message-tree` |

### 3.1 Build rules for workers

- Use `cargo build -p <crate>`, `cargo test -p <crate>`, `cargo clippy -p <crate>`.
- **NEVER run bare `cargo run`** at the repository root (it invokes `tools/cargo-run`). Use explicit `-p`.
- `cargo fmt --all -- --check` and `cargo clippy -p <crate> --all-targets --no-deps` are mandatory.
  - **`--no-deps` is required.** `-D warnings` also reaches *dependency* workspace members, and the
    current stable clippy already flags `proc-macros/src/message_handler_data_attr.rs` (a build-dependency
    of `editor`). `--no-deps` lints exactly the crates the gate names (execution finding E-1).
  - **The clippy gate is baseline-relative, not `-D warnings`.** The `editor` crate itself already reports
    13 pre-existing diagnostics in files outside every phase's write scope (recorded in
    `agent/clippy-baseline.txt`), so `-D warnings` on it can never pass. A phase passes when:
    (a) `cargo clippy -p <crate> --all-targets --no-deps 2>&1 | grep -E '^[[:space:]]+--> ' | sed 's/^ *--> //' | sort -u`
    adds **no** `file:line` entry to that crate's section of `agent/clippy-baseline.txt`; and
    (b) no diagnostic points at a file the phase created. Regenerating the baseline is an escalation (E-4).
- Follow `website/content/volunteer/guide/starting-a-task/code-quality-guidelines.md`.
- Do **not** open upstream Graphite PRs with agent-written code (`.../ai-contribution-policy.md`). Internal scaffolding only.

---

## 4. Target crate layout

```
agent/
  protocol/       crate graphite-agent-protocol   (types + traits; no editor, no MCP)
  descriptors/    crate graphite-agent-descriptors (NODE_METADATA -> descriptors)
  host/           crate graphite-agent-host        (ToolHost, ToolModules, mode bridges)
  mcp/            crate graphite-agent-mcp         (MCP transport adapter)
  cli/            crate graphite-agent-cli         (binary graphite-agent + conformance tests)
```

Add to root `Cargo.toml` `members` (and **not** `default-members`):

```toml
    "agent/protocol",
    "agent/descriptors",
    "agent/host",
    "agent/mcp",
    "agent/cli",
```

### 4.1 Dependency matrix (complete — M-B6)

| Crate | Local dependencies | Third-party dependencies | Notes |
|---|---|---|---|
| `agent/protocol` | none | `serde`, `serde_json`, `futures` | |
| `agent/descriptors` | `graphene-std` (`features = ["shader-nodes"]`), `editor` (`path = "../../editor", package = "graphite-editor"`), `graphite-agent-protocol`, `graph-craft` | `serde`, `serde_json`, `futures`, `clap`; dev: `jsonschema` | `shader-nodes` matches the editor's GPU consumers (M-A9). `graph-craft` is required from Phase 3 by T3.2: `HeadlessEditorState.resource_storage` needs a concrete `ResourceStorage`, and `HashMapResourceStorage` is reachable only through `graph-craft`, not `graphene-std` (E-9). Must never depend on `agent/host` (HIGH-1). |
| `agent/host` | `graphite-editor`, `graphite-agent-protocol`, `graphite-agent-descriptors`, `document-format`, `document-container`, `document-graph-storage`, `graph-craft`, `interpreted-executor`, `wgpu-executor`, `graphene-std` (`shader-nodes`), `graphene-cli` | `serde`, `serde_json`, `futures`, `base64`, `tokio` (`sync`, `rt-multi-thread`, `time`), `interprocess` | `document-graph-storage` for Phase 5; `base64` for `render.preview` (M-B1). |
| `agent/mcp` | `graphite-agent-protocol`, `graphite-agent-host` | `serde`, `serde_json`, `futures`, `tokio` (`io-util`), `tiny_http` | `io-util` for the stdio reader and `futures` to poll `ToolHost::events()` (R7-M2, R9-M2); `tiny_http` only from Phase 4 (M-B4). |
| `agent/cli` | `graphite-agent-host`, `graphite-agent-mcp`, `graphene-cli` | `serde_json`, `tokio`, `clap`, `anyhow` | Binary `graphite-agent`; hosts the conformance tests so `CARGO_BIN_EXE_*` resolves (HIGH-B2). |

**Workspace vs path vs version deps (M-B1).** Only crates already listed in root `[workspace.dependencies]` may use `{ workspace = true }`. Most members used here are already listed there (`graphite-editor`, `graphene-std`, `graph-craft`, `interpreted-executor`, `wgpu-executor`, `document-format`, `document-container`, `document-graph-storage`, `graphene-resource`, `serde`, `serde_json`, `futures`, `clap`, `tokio`, `base64`, `anyhow`). `graphene-cli` is **not** listed: declare it as a path dependency (`graphene-cli = { path = "../../node-graph/graphene-cli" }`). When the local package name differs from the desired crate name, use `package =` (e.g. `editor = { path = "../../editor", package = "graphite-editor" }`). `interprocess`, `tiny_http`, `jsonschema`, and `uuid` are not workspace dependencies: declare them as version dependencies in the owning crate (`interprocess = "2.4.2"`, matching `desktop/Cargo.toml`). The first build of Phase 0/Phase 4 needs network access to fetch `jsonschema`/`tiny_http`.

Pre-approved third-party additions: `uuid`, `jsonschema` (dev only), `tiny_http`, `interprocess`. `tokio` features may be extended with `sync`, `io-util`, `net`, `time` (R7-M2). No other new dependency without escalation.

### 4.2 Node-code linkage

- `NODE_METADATA` is filled by `#[ctor]` registrations. A crate that wants the non-empty GPU node catalog must link the node crates with `graphene-std = { workspace = true, features = ["shader-nodes"] }`. The precedent is `desktop/wrapper/Cargo.toml`'s `gpu` feature (`graphene-std/shader-nodes`), **not** `tools/node-docs` (which builds the website catalog without shader nodes — a known divergence, M-B6). Depending on `core-types` alone yields an **empty catalog**.
- `agent/descriptors` also depends on `graphite-editor` so it can enumerate message actions and build a temporary headless editor (T3.2).
- **Dependency direction is one-way: `agent/host` → `agent/descriptors`.** `agent/descriptors` must never depend on `agent/host` (HIGH-1).

### 4.3 `editor` gains one dependency

`editor` depends on `graphite-agent-protocol` (for `AgentOperation`, `DocumentOperation`, `SnapshotProjection`, `ToolOutcome`, `Capability`). `AgentReplySink` is **not** in the protocol crate — the editor defines it locally from `futures` (§5.2, R8-M5). The protocol crate is a leaf with no editor dependency, so there is no cycle, and it uses only wasm-safe crates. This requires editing `editor/Cargo.toml` (in Phase 1 scope, M-B2).

---

## 5. Frozen interfaces — contract v1

Copy exactly. Do not change names, field shapes, or signatures after the Phase 1 gate. Later phases may only **add** variants to `#[non_exhaustive]` enums.

### 5.1 `agent/protocol/src/lib.rs`

```rust
//! Transport-agnostic agent contract. No MCP, no editor dependencies.

use futures::Stream;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;

/// Monotonic, host-assigned call identity (INV-14). The unit of correlation.
pub type QueryId = u64;

/// Document identity as exposed to agents: the inner `u64` of the editor's
/// `DocumentId(pub u64)`. Converted at the host boundary; never passed raw.
pub type AgentDocumentId = u64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability { Read, Author, Execute, Export, Persist }

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CapabilitySet(pub Vec<Capability>);

impl CapabilitySet {
    pub fn grants(&self, required: Capability) -> bool { self.0.contains(&required) }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolDescriptor {
    pub name: String,
    pub description: String,
    pub capability: Capability,
    pub input_schema: serde_json::Value,   // JSON Schema draft 2020-12
    pub output_schema: serde_json::Value,
    pub version: u32,
}

/// Adapter-facing request. Carries no id and no capability (INV-6, INV-14).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolRequest {
    pub name: String,
    pub arguments: serde_json::Value,
    pub document: Option<AgentDocumentId>,
}

/// Module-facing call. Constructed only by the host, which assigns id + capability.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: QueryId,
    pub name: String,
    pub arguments: serde_json::Value,
    pub capability: Capability,
    pub document: Option<AgentDocumentId>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ToolOutcome {
    Ok { id: QueryId, result: serde_json::Value },
    Err { id: QueryId, error: ToolError },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ToolError {
    InvalidArguments { message: String },
    Unauthorized { capability: Capability },
    NotFound { what: String },
    Timeout { id: QueryId },
    Cancelled { id: QueryId },
    InvalidGraph { message: String },
    PathOutsideRoot { path: String },
    Internal { message: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentEvent {
    Progress { id: QueryId, fraction: f32, message: String },
    DocumentChanged { document: AgentDocumentId },
    Posted { id: QueryId, message: String },
}

/// An accepted, in-flight call. Owned and `'static`, so `cancel` remains
/// reachable while it is outstanding (HIGH-A4 / B-M9).
///
/// NOTE: the future is deliberately NOT `+ Send`. `editor::node_graph_executor::run_node_graph()`
/// holds a `std::sync::MutexGuard` across an `.await`, so it is `!Send` (R9-H1).
/// The host and the MCP adapter must therefore run on ONE dedicated thread with a
/// current-thread runtime (§5.5).
pub struct PendingCall {
    pub id: QueryId,
    pub outcome: Pin<Box<dyn Future<Output = ToolOutcome> + 'static>>,
}

/// The tool-execution boundary. Implementations are `Send + Sync` and use
/// interior mutability (`tokio::sync::Mutex<HostInner>`) so that `call` and
/// `cancel` can be invoked concurrently.
pub trait ToolHost: Send + Sync {
    fn descriptors(&self) -> Vec<ToolDescriptor>;
    fn call(&self, request: ToolRequest) -> PendingCall;
    fn cancel(&self, id: QueryId) -> bool;
    /// Owned stream. Implementations must use `futures::channel::mpsc` (whose
    /// `UnboundedReceiver` implements `Stream`), NOT a tokio broadcast receiver
    /// (which is not a `futures::Stream`; `tokio-stream` is not approved) — R8-H1.
    fn events(&self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>>;
}

pub trait ToolModule: Send {
    fn descriptors(&self) -> Vec<ToolDescriptor>;
    /// `'a` must be shared by `self` and `bridge`: the returned future captures
    /// both. With elided lifetimes, `'_` binds to `&mut self` only and no
    /// conforming impl can compile (R6-C1). Do not add `+ Send` (R9-H1).
    fn execute<'a>(
        &'a mut self,
        call: ToolCall,
        bridge: &'a mut dyn EditorBridge,
    ) -> Pin<Box<dyn Future<Output = Result<serde_json::Value, ToolError>> + 'a>>;
}

/// The typed query/response boundary over the editor. Implementations submit
/// curated queries and await correlated replies. They must NOT (INV-2, INV-13):
///   - read message-handler state directly,
///   - deserialize and dispatch an arbitrary `Message`.
pub trait EditorBridge: Send {
    /// Submit under a host-allocated id (INV-14).
    fn submit(&mut self, id: QueryId, query: BridgeQuery) -> Result<(), ToolError>;
    fn poll(&mut self, id: QueryId) -> Option<Result<serde_json::Value, ToolError>>;
    fn cancel(&mut self, id: QueryId);
    fn drain_events(&mut self) -> Vec<AgentEvent>;
    /// Advance async editor work. MUST be async (CRIT-A1): the only way to drive
    /// node-graph execution is `run_node_graph().await`. Do not add `+ Send`:
    /// `run_node_graph()` holds a `std::sync::MutexGuard` across `.await` and is
    /// therefore `!Send` (R9-H1).
    fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>>;
}

/// A curated query (INV-13). `Snapshot.document` is `None` only for the
/// document-independent projections: `DocumentList` and `ActiveDocument`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BridgeQuery {
    Document { operation: DocumentOperation },
    Operation { document: AgentDocumentId, operation: AgentOperation },
    Snapshot { document: Option<AgentDocumentId>, projection: SnapshotProjection },
}

/// Document lifecycle + persistence (HIGH-A2). Export is async on the editor side (INV-15).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum DocumentOperation {
    New { name: String },
    /// `path` MUST already be canonicalized and prefix-checked by the host
    /// against its configured root (INV-12). The editor reads the file itself;
    /// the host does not read bytes for open (the bytes never leave the process,
    /// so no bytes variant is needed). Result: `{ "document_id": <id> }`.
    Open { path: String },
    Close { document: AgentDocumentId },
    /// Export the document to `.gdd` bytes. Result: `{ "gdd_base64": "..." }`.
    ExportGdd { document: AgentDocumentId },
}

/// The ENTIRE graph mutation surface exposed to agents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AgentOperation {
    AddNode { identifier: String, x: f64, y: f64 },
    RemoveNode { node_id: u64 },
    /// `value_json` is a JSON-encoded `graph_craft::document::value::TaggedValue`.
    SetInput { node_id: u64, input_index: u32, value_json: String },
    Connect { from_node: u64, from_output: u32, to_node: u64, to_input: u32 },
    Disconnect { to_node: u64, to_input: u32 },
    BeginTransaction,
    CommitTransaction,
    AbortTransaction,
    Undo,
    Redo,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum SnapshotProjection {
    DocumentList,
    DocumentSummary,
    NodeList,
    Node { node_id: u64 },
    ActiveDocument,
    /// Currently selected layer/node ids in the document (R7-H1).
    Selection,
}
```

### 5.2 Editor-side boundary

Create `editor/src/messages/agent/{mod.rs, agent_message.rs, agent_message_handler.rs, test.rs}` and `editor/src/messages/portfolio/document/query.rs`.

```rust
// agent_message.rs — wire with the standard #[impl_message] / #[child] pattern.
// Derive EXACTLY this set: the top-level `Message` derives `PartialEq` and
// `dispatcher.rs` compares queued messages, so a child enum without `PartialEq`
// fails to compile the whole editor (HIGH-2). All §5.1 payload types therefore
// also derive `PartialEq`.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum AgentMessage {
    /// Lifecycle/persistence. Handled by the document subsystem where async export lives.
    Document { id: QueryId, operation: DocumentOperation },
    /// Graph/history mutation. Executed by the agent handler via curated messages.
    Execute { id: QueryId, document: u64, operation: AgentOperation },
    /// Typed read. `document: None` is valid only for `DocumentList` and `ActiveDocument`.
    Snapshot { id: QueryId, document: Option<u64>, projection: SnapshotProjection },
    Cancel { id: QueryId },
    /// Internal: a subsystem finished async work and reports the correlated result.
    Reply { id: QueryId, outcome: ToolOutcome },
}
```

`AgentMessageHandler` responsibilities (it owns **no** tool modules):

1. Map `Execute` to concrete editor `Message`s pushed onto `responses`.
2. Map `Snapshot` to `DocumentQuery` calls (sync reads only).
3. Forward `Document` operations to the document subsystem (for `ExportGdd`, to `DocumentMessage::ExportGdd { request_id }`).
4. On completion, send exactly one `ToolOutcome` to the **reply sink**.
5. Track cancelled ids; a cancelled id replies `Cancelled` at most once.

```rust
/// Sync typed reads only (INV-15: export is NOT here).
pub trait DocumentQuery {
    fn document_list(&self) -> Result<serde_json::Value, ToolError>;
    fn summary(&self, document: u64) -> Result<serde_json::Value, ToolError>;
    fn node_list(&self, document: u64) -> Result<serde_json::Value, ToolError>;
    fn node(&self, document: u64, node_id: u64) -> Result<serde_json::Value, ToolError>;
    fn active_document(&self) -> Result<Option<u64>, ToolError>;
    /// Selected layer/node ids for the document (R7-H1). Needed by `session.selection`.
    fn selection(&self, document: u64) -> Result<serde_json::Value, ToolError>;
}
```

```rust
/// Installed into the editor at construction. The host owns the receiver.
pub type AgentReplySink = futures::channel::mpsc::UnboundedSender<ToolOutcome>;
```

`Editor` gains a setter (does NOT change `Editor::new`'s signature — avoids out-of-scope caller edits, B-M1):

```rust
impl Editor {
    pub fn set_agent_reply_sink(&mut self, sink: AgentReplySink);
}
```

**Reply transport:** do NOT add a `FrontendMessage` variant (R2-B). `AgentMessageHandler` sends to the sink. `document-format`'s async export runs inside the document subsystem and emits `AgentMessage::Reply { id, outcome }` when done (INV-15).

**Acquisition of `DocumentQuery`:** the dispatcher builds `AgentMessageContext { portfolio: &mut PortfolioMessageHandler }` for `Message::Agent` (mirroring `ToolMessageContext`). `DocumentQuery` is implemented for `PortfolioMessageHandler` inside the portfolio module.

> **Node-id note (R2-A).** `NodeId` is hash-derived, not monotonic. The handler allocates `let id = NodeId::new();` **before** pushing `NodeGraphMessage::CreateNodeFromContextMenu { node_id: Some(id), node_type, xy, add_transaction: true }`, then replies with that exact `id`. Never infer an id by diffing lists.

> **Import ordering.** Each phase reads its own write scope in Appendix D; the recurring files are `editor/src/messages/prelude.rs` and `editor/src/messages/portfolio/document/mod.rs`, which must export the new `agent` / `query` modules (M-A6).

### 5.3 `HeadlessEditorState` (M-A8 / B-M7 — defined here, not invented by workers)

```rust
/// Everything `Editor::new_headless` needs, so the host constructs it without
/// touching editor internals.
pub struct HeadlessEditorState {
    /// Use the type `editor/src/application.rs` already imports; `graphene-resource`
    /// is NOT a direct dependency of `editor` (M-B2).
    pub resource_storage: std::sync::Arc<dyn graph_craft::application_io::resource::ResourceStorage>,
    pub working_copy_root: std::path::PathBuf,
    pub uuid_random_seed: u64,
}

impl Editor {
    /// Uses `Environment { platform: Platform::Desktop, host: <compile-time host> }`,
    /// `PlatformApplicationIo::default()`, and a real signaling `Wake`.
    /// Install the reply sink via `set_agent_reply_sink` before or after.
    pub fn new_headless(state: HeadlessEditorState) -> (Self, Wake);
}
```

### 5.4 Headless pump loop (normative — CRIT-4, R2-K, CRIT-A1, HIGH-A3)

`Editor::poll_node_graph_evaluation` only collects `Message`s; it does not dispatch them. `run_node_graph` is `async`. `pump` MUST run this exact loop, mirroring `frontend/wrapper/src/helpers.rs`:

```rust
// In agent/host; `editor` is the alias for package `graphite-editor`.
//   use editor::messages::future::FutureMessage;
//   use editor::messages::prelude::Message;
//   use std::collections::VecDeque;
//   use futures::FutureExt; // for .boxed()
loop {
    // 1. Drain deferred future results FIRST, unconditionally (E-5). `run_node_graph`
    //    returning `!more` must not skip this: an async `ExportGdd` reply arrives as a
    //    `FutureMessage` result, and `handle_message` is what drains it into the queue.
    let _ = self.editor.handle_message(FutureMessage::Wake);

    // 2. Advance async node-graph execution.
    let (more, _texture) = editor::node_graph_executor::run_node_graph().await;
    if !more { break; }

    // 3. Collect and RE-DISPATCH evaluated messages.
    let mut messages = VecDeque::new();
    if let Err(e) = self.editor.poll_node_graph_evaluation(&mut messages) {
        // HIGH-A3: this is the normal "no documents open yet" state, not a failure.
        if e != "No active document" {
            return Err(ToolError::Internal { message: e });
        }
    }
    if messages.is_empty() { break; }
    let _ = self.editor.handle_message(Message::Batched { messages: messages.into_iter().collect() });
}

// 4. Always drain the reply sink afterwards.
self.drain_reply_sink();
```

Steps 1 and 2 must both run every iteration: a headless pump that only drains when
`run_node_graph` reports more work never delivers an async `ExportGdd` reply (E-5).
A caller that needs to wait for a pending reply loops `submit → poll → pump` with a
short sleep; the host deadline (§5.5) bounds the wait.

The `Wake` installed by `new_headless` must signal this loop (e.g. `tokio::sync::Notify`), exactly as the web wrapper's wake dispatches `FutureMessage::Wake`.

### 5.5 Timeout and cancellation ownership (HIGH-A4 / B-M9)

- The **host** owns timeouts: `ToolHost::call` starts a deadline; on expiry it calls `bridge.cancel(id)` and resolves `ToolError::Timeout`.
- Cancellation is delivered through `ToolHost::cancel(&self, id)`, which sets a flag in a `Arc<Mutex<HashSet<QueryId>>>` **shared** with the in-flight call's poll loop. The loop observes the flag each iteration and resolves `ToolError::Cancelled`. This is why `ToolHost::call` takes `&self` and returns a `'static` `PendingCall` (HIGH-A4).
- The default timeout is 30 s (`--timeout-seconds`).
- The bridge and the editor handler do not implement timeouts.

**Runtime requirement (R9-H1).** `editor::node_graph_executor::run_node_graph()` holds a `std::sync::MutexGuard` across an `.await`, so its future is `!Send`. Therefore the host, its bridges, the tool modules, and the MCP adapter MUST all run on **one dedicated thread** driving a current-thread runtime:

```rust
tokio::runtime::Builder::new_current_thread().enable_all().build()?.block_on(async { /* adapter + host */ })
```

`AgentEvent` (not `AgentReplySink`) is the only cross-thread channel; it is `Send`. Do not spawn tool work on a multi-thread runtime.

### 5.6 Versioning rule

- `ToolDescriptor.version` starts at `1`.
- Adding an optional argument → keep version, update `input_schema`.
- Removing/renaming an argument or changing result shape → bump by 1, keep the old descriptor one phase.
- Changing an interaction pattern (streaming, subscriptions) requires escalation.

---

## 6. Descriptor format

Every descriptor is generated or declared in the single static registry.

**Two kinds of catalog entry (R2-F).** Generated `node.type.*` entries are **catalog data** surfaced by `node.describe` and the `graphite://node-catalog` resource; they are **not** MCP tools. Curated operation entries are the MCP tools. `tools/list` returns curated tools plus exactly `node.list_types` and `node.describe`.

Canonical shape (example is curated):

```json
{
  "name": "graph.add_node",
  "description": "Add a node of a given type to a document's graph.",
  "capability": "Author",
  "version": 1,
  "input_schema": {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "additionalProperties": false,
    "required": ["document_id", "identifier"],
    "properties": {
      "document_id": { "type": "integer", "minimum": 0 },
      "identifier": { "type": "string", "description": "Proto node identifier." },
      "x": { "type": "number" },
      "y": { "type": "number" }
    }
  },
  "output_schema": {
    "$schema": "https://json-schema.org/draft/2020-12/schema",
    "type": "object",
    "additionalProperties": false,
    "required": ["node_id"],
    "properties": { "node_id": { "type": "integer", "minimum": 0 } }
  }
}
```

### 6.1 Generated node-type descriptor naming

1. Take the `ProtoNodeIdentifier` string, e.g. `graphene_core::raster::OpacityNode`.
2. Replace every `::` with `.`.
3. Lowercase the whole string.
4. Replace every character that is not `[a-z0-9.]` with `_`.
5. Prefix with `node.type.`.

Example: `graphene_core::raster::OpacityNode` → `node.type.graphene_core.raster.opacitynode`.

### 6.2 `FieldMetadata` → JSON Schema mapping

| `FieldMetadata` | JSON Schema |
|---|---|
| `name` | property key |
| `description` | `description` |
| `default_type` / value type | `type` / `enum` / `oneOf` |
| `number_soft_min` / `number_soft_max` | `x-soft-min` / `x-soft-max` |
| `number_hard_min` / `number_hard_max` | `minimum` / `maximum` |
| `unit` | `x-unit` |
| `hidden` | omit |
| `RegistryWidgetOverride::Custom` | `x-widget` |

> `NODE_METADATA` is a `LazyLock<Mutex<HashMap<...>>>`. Lock it; treat poisoning as an error; assert non-empty before generating.

### 6.3 Generated command descriptor naming and status (E-8)

Command descriptors are **catalog data**, exactly like `node.type.*` entries. They are produced by
`agent/descriptors/src/commands.rs` from a live `collect_actions()` result plus the message field
metadata the macros already emit, and they **never** appear in `tools/list`, in the host's
`module_index`, or in any `ToolModule::descriptors()`. Executing a message action by name is
forbidden (INV-13); the only executable entries are the curated host modules.

1. Take the action's `AsMessage::global_name()` (e.g. `NodeGraphMessage::DeleteNodes` →
   the dotted hierarchical name).
2. Replace every `::` with `.`, lowercase, and replace every character that is not `[a-z0-9.]` with `_`.
3. Prefix with `command.`.

Example: `Portfolio.Document.NodeGraph.DeleteNodes` → `command.portfolio.document.nodegraph.deletenodes` (E-10: the rule is applied to the runtime `global_name()` string, whose segments are the parent `Message` enum variants, not to a Rust path). Note that `NodeGraph` actions are not collected unless the graph-view overlay is open, so no `command.…nodegraph…` entry is allowlisted in the default document state; use a collected name such as `command.animation.setframeindex` in tests.

Two caveats the worker must record rather than discover:

- The macro metadata yields Rust **source-type strings** (`"name: RustType"`), so `commands.rs`
  needs its own string→JSON-Schema mapper; `type_to_schema` in `agent/descriptors/src/lib.rs` maps
  `graphene_std::Type`, not source strings.
- `HierarchicalTree` only recurses into payload types whose name ends in `Message`; nested non-message
  payload structs therefore appear as opaque type strings and get `x-rust-type`, not a structural schema.

Command descriptors have exactly one read path: the `coverage` subcommand and the
`graphite://command-catalog` resource (§11). They are not tools.

---

## 7. Global rules of engagement

**ALWAYS**
- Keep `agent/protocol` free of `editor` and MCP dependencies.
- Go through the contract for every call (INV-1).
- Let the host assign capability and `QueryId` (INV-6, INV-14).
- Put every mutation in an editor transaction (INV-5).
- Generate node/command descriptors from metadata (INV-8).
- Confine file paths to the configured root (INV-12).
- Edit only files in Appendix D for your phase.

**NEVER**
- Read `PortfolioMessageHandler` / `DocumentMessageHandler` private fields (INV-2).
- Deserialize and dispatch an arbitrary `Message` (INV-13).
- Add a second tool registry or a second schema path.
- Put transport code inside `agent/host` or tool modules.
- Hand-write a node-type descriptor that could be generated.
- Add agent variants to `FrontendMessage` (R2-B).
- Write to stdout in stdio mode except JSON-RPC (INV-10).
- Initialize more than one `Editor` per process (INV-11).
- Run bare `cargo run`.
- Change a frozen interface after the Phase 1 gate.

---

## 8. Phase 0 — Inventory, derivation, and prerequisite fixes

**Goal:** produce the node catalog and unblock the reuse path. Zero runtime risk to the editor.
**Depends on:** nothing.
**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T0.1 | Create the `agent/` crate tree with stub sources and the §4.1 dependency matrix. Add to `members` only. | `agent/*`, `Cargo.toml` |
| T0.2 | Implement §5.1 types verbatim. Add serde round-trip tests for `ToolRequest`, `ToolCall`, `ToolOutcome`, `AgentOperation`, `DocumentOperation`, `BridgeQuery`. | `agent/protocol/src/lib.rs` |
| T0.3 | Give `graphene-cli` a library target: create `src/lib.rs` with `pub mod export; pub mod engine;`, move `compile_graph` + `create_executor` from `main.rs` into `engine.rs` as `pub fn`. Additionally move the `.gdd`→runtime setup that currently lives only in `main.rs` (`PlatformApplicationIo::new().await`, `registry().to_runtime_with_metadata(...)`, building `PlatformEditorApi`, obtaining the `WgpuExecutor`) into `engine.rs`, and expose **one** entry point: `pub async fn render_gdd_to_png(gdd_bytes: &[u8], max_dimension: u32) -> Result<Vec<u8>, Box<dyn Error>>` (R9-M3). The `NodeGraphUpdateSender` used must write to **stderr or be a no-op**, never stdout (INV-10); replace `UpdateLogger`'s `println!`. `main.rs` uses the library. Behavior identical. | `node-graph/graphene-cli/src/lib.rs`, `src/engine.rs`, `src/main.rs` |
| T0.4 | Implement the node descriptor generator: depend on `graphene-std` (`shader-nodes`), lock `graphene_std::registry::NODE_METADATA`, assert non-empty, emit one descriptor per node type per §6.1/§6.2. | `agent/descriptors/src/lib.rs` |
| T0.5 | Implement binary `graphite-agent-descriptors` with `inventory --out <path>` and `coverage`. | `agent/descriptors/src/main.rs`, `Cargo.toml` |
| T0.6 | Inventory dump `agent/inventory.json` with keys `nodes` (generated) and `agent_safe` (allowlist result per action). **Do not** include `editor_commands` or `frontend_messages` (R3-A). Message-action enumeration moves to Phase 3, where an active document exists (M-A3). | `agent/descriptors/src/inventory.rs` |
| T0.7 | Implement `agent_safe` as **default-deny against `agent/allowlist.toml`**. Seed it with the §17 Phase 2 curated tool names only. Do not write subjective rules. | `agent/allowlist.toml`, `agent/descriptors/src/classify.rs` |
| T0.8 | Expose the catalog to other crates: `pub fn node_descriptors() -> Vec<ToolDescriptor>` and `pub fn node_catalog_json() -> serde_json::Value`. `agent/host` consumes these (M-A8). | `agent/descriptors/src/lib.rs` |
| T0.9 | Write `agent/README.md`: layering, "never hand-write a node descriptor", allowlist policy, and why `shader-nodes` is enabled (M-A9). | `agent/README.md` |

### Gate (Definition of Done)

1. `cargo build -p graphite-agent-protocol -p graphite-agent-descriptors -p graphene-cli` → success.
2. `cargo test -p graphite-agent-protocol -p graphite-agent-descriptors` → all pass.
3. `cargo run -p graphite-agent-descriptors -- inventory --out agent/inventory.json` → file exists.
4. Test: every `NODE_METADATA` entry produced exactly one descriptor; count **> 0**; each `input_schema` validates with the `jsonschema` dev-dependency (M-B3).
5. `agent/inventory.json` has non-empty `nodes` and an `agent_safe` object.
6. Test: `agent_safe` returns `false` for an unknown name, `true` only for allowlisted names.
7. `cargo fmt --all -- --check` and `cargo clippy -p graphite-agent-protocol -p graphite-agent-descriptors -p graphene-cli --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).

### Rollback
Delete `agent/` and revert `members`; revert the `graphene-cli` lib extraction.

---

## 9. Phase 1 — D0: the contract primitive

**Goal:** correlation, typed snapshots, async export plumbing, cancellation, events. No surface yet.
**Depends on:** Phase 0 gated.
**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T1.1 | Create the `agent` message subsystem per §5.2 (`#[impl_message]`/`#[child]`), add handler to `DispatcherMessageHandlers`, and export it from `messages/mod.rs` + `messages/prelude.rs`. | `editor/src/messages/agent/*`, `editor/src/messages/mod.rs`, `editor/src/messages/prelude.rs`, `editor/src/dispatcher.rs`, `editor/src/messages/message.rs` |
| T1.2 | Define `AgentReplySink` and store it on `AgentMessageHandler`. Add `Editor::set_agent_reply_sink` (no `Editor::new` signature change). | `editor/src/messages/agent/agent_message_handler.rs`, `editor/src/application.rs` |
| T1.3 | Define `pub trait DocumentQuery` and implement it for `PortfolioMessageHandler` inside the portfolio module (sync reads only; no export). Add `pub mod query;` to the document module. | `editor/src/messages/portfolio/document/query.rs`, `editor/src/messages/portfolio/document/mod.rs` |
| T1.4 | Build `AgentMessageContext { portfolio: &mut PortfolioMessageHandler }` in the dispatcher for `Message::Agent`. | `editor/src/dispatcher.rs` |
| T1.5 | Implement `Execute`: map each `AgentOperation` to concrete messages on `responses`; allocate `NodeId::new()` for `AddNode` and reply with it (R2-A). Wrap mutations in transactions unless a `BeginTransaction` is open. | `agent_message_handler.rs` |
| T1.6 | Implement `Snapshot` via `DocumentQuery`, including `DocumentList`/`ActiveDocument` with `document: None` and `Selection` with a document id (R7-H1). | `agent_message_handler.rs` |
| T1.7 | Implement `DocumentOperation` routing: `New`/`Open`/`Close` to `PortfolioMessage` (using `DocumentPassMessage` for document-scoped work); `ExportGdd` to a new `DocumentMessage::ExportGdd { request_id }`. | `agent_message_handler.rs`, `editor/src/messages/portfolio/document/document_message.rs` |
| T1.8 | Implement `DocumentMessage::ExportGdd` in the document subsystem as an async closure mirroring the existing save path (`responses.add(async move { ... resources.embed_resources().await ... storage.export_to_bytes().await ... })`), then emit `AgentMessage::Reply { id, outcome: base64 }` (INV-15, HIGH-B1). | `editor/src/messages/portfolio/document/document_message_handler.rs` |
| T1.9 | Implement cancellation: `Cancel { id }` records the id; forward to the in-flight async export so it resolves `Cancelled` at most once. | `agent_message_handler.rs` |
| T1.10 | Forward every terminal `AgentMessage::Reply` to the reply sink exactly once. | `agent_message_handler.rs` |
| T1.11 | Add `Editor::new_headless(state: HeadlessEditorState) -> (Self, Wake)` per §5.3. Use a real signaling `Wake` (HIGH-12). | `editor/src/application.rs` |
| T1.12 | Write unit tests in `agent/test.rs`; write the single-editor process-isolation test in `editor/tests/single_editor.rs`. | `editor/src/messages/agent/test.rs`, `editor/tests/single_editor.rs` |

### Gate (Definition of Done)

1. `cargo build -p graphite-editor -p graphite-agent-protocol` → success.
2. `cargo test -p graphite-editor agent::` → all pass: correlation (two concurrent ids), cancellation, snapshot isolation (including `SnapshotProjection::Selection`), no-arbitrary-Message static check, export round-trip (`ExportGdd` then `document-format` re-open).
3. `cargo test -p graphite-editor --test single_editor` → pass (separate command; the `agent::` filter does not match integration-test paths, M-B8).
4. `cargo clippy -p graphite-editor --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).
5. No §5.1/§5.2 signature changed except additive `#[non_exhaustive]` variants.

### Rollback
Revert the `agent` subsystem, `DocumentQuery`, `ExportGdd`, and the headless constructor; `editor/Cargo.toml` dependency reverted.

---

## 10. Phase 2 — Surface A (headless) + session host

**Goal:** first real agent action, end to end, through the contract.
**Depends on:** Phase 1 gated.
**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T2.1 | Implement `HeadlessBridge`: owns `Editor` + `Wake` + the `AgentReplySink` receiver; `submit(id, query)` sends an `AgentMessage`; `poll(id)` drains the receiver; `cancel(id)` sends `AgentMessage::Cancel`; `pump()` returns the boxed async future running §5.4; `drain_events`. Construct the editor via `new_headless` with `HeadlessEditorState { resource_storage: folder storage, working_copy_root: temp, uuid_random_seed: 0 }`, then `set_agent_reply_sink`. | `agent/host/src/headless.rs` |
| T2.2 | Implement `ToolHost` for `Host`, plus a mode-agnostic `Host::new(bridge: Box<dyn EditorBridge>, capabilities: CapabilitySet, timeout: Duration, root: PathBuf) -> Host`. The extra `root` is the INV-12 confinement policy point the file tools need (E-6). `inner: Arc<tokio::sync::Mutex<HostInner>>`. `call(&self)` **clones the Arc** (so the returned future is `'static` and holds no borrow of `self`), allocates the `QueryId`, builds `ToolCall` with the descriptor's capability, checks the `CapabilitySet`, starts the deadline, and returns a `PendingCall` whose future locks the mutex, runs the module + bridge (§5.4), and owns a clone of the cancellation set. `cancel(&self)` sets the flag in the shared `Arc<Mutex<HashSet<QueryId>>>`. `events(&self)` returns the stored owned `futures::channel::mpsc::UnboundedReceiver<AgentEvent>` created at construction (the internal sender is kept and cloned for emits); if the receiver was already taken, return `futures::stream::empty()`. **Do not use a tokio broadcast receiver** (R8-H1). Destructure host fields before calling a module's `execute` (R2-G). | `agent/host/src/host.rs` |
| T2.3 | Implement `DocumentModule`: `document.new`, `document.open`, `document.save`, `document.close`, `document.list`. `save` = `BridgeQuery::Document(ExportGdd)` → host writes the returned bytes under `--root` (INV-12); `open` = host canonicalizes and prefix-checks the path, then `DocumentOperation::Open { path }` (the editor reads it; the host does not read bytes — M-B4); `new` = `DocumentOperation::New` then id discovery via `Snapshot(DocumentList)` diff (R2-L); `close` = `DocumentOperation::Close`; `list` = `Snapshot(DocumentList)`. | `agent/host/src/modules/document.rs` |
| T2.4 | Implement `NodeCatalogModule`: `node.list_types`, `node.describe`, sourced from `graphite_agent_descriptors::{node_descriptors, node_catalog_json}` (M-A8). | `agent/host/src/modules/node_catalog.rs` |
| T2.5 | Implement `GraphModule`: `graph.add_node`, `graph.remove_node`, `graph.set_input`, `graph.connect`, `graph.disconnect`, `graph.list_nodes`, `graph.get_node`. Mutations via `BridgeQuery::Operation`; reads via `BridgeQuery::Snapshot`. `graph.add_node`'s `node_id` is the handler-allocated id (R2-A). | `agent/host/src/modules/graph.rs` |
| T2.6 | Implement `HistoryModule`: `history.undo`, `history.redo`, `history.begin`, `history.commit`, `history.abort`. | `agent/host/src/modules/history.rs` |
| T2.7 | Implement `RenderModule`: `render.preview`, `render.export`. Obtain bytes via `BridgeQuery::Document(ExportGdd)`, then call **only** `graphene_cli::engine::render_gdd_to_png(bytes, max_dimension)` for preview (T0.3 owns the GPU context and `.gdd`→runtime conversion, R9-M3); for `render.export`, write the produced PNG under `--root`, confined by INV-12. **Phase 2 is PNG-only for `render.export`** (E-7): `graphene_cli` exposes no public byte-returning SVG entry point, `node-graph/**` is out of Phase 2 scope, and the non-PNG `format` values return `InvalidArguments`; adding an SVG byte path is a Phase 3+ task. | `agent/host/src/modules/render.rs` |
| T2.8 | Implement path confinement (canonicalize + prefix check; `PathOutsideRoot`). | `agent/host/src/paths.rs` |
| T2.9 | Implement the MCP stdio adapter per §11: `initialize`, `notifications/initialized`, `tools/list`, `tools/call`, `notifications/cancelled`, `resources/list`, `resources/read`, `prompts/list`, `prompts/get`, `logging/setLevel` (accepted; threshold ignored, R8-M1). JSON-RPC only on stdout (INV-10). | `agent/mcp/src/stdio.rs`, `agent/mcp/src/lib.rs` |
| T2.10 | Implement binary `graphite-agent` in `agent/cli`. Flags: `--mode headless\|attached\|peer` (default `headless`), `--stdio`, `--http <addr>` (off by default), `--root <path>` (required), `--storage <path>`, `--attach <socket>` (attached mode), `--gdd <path>` (peer mode), `--timeout-seconds` (default 30), `--capabilities <csv>` (default: `all` in headless mode, `read,author` in attached and peer modes — R8-M2). In Phase 2, `attached`/`peer`/`--http` exit with a clear "not implemented until Phase 4/5" error; only `--mode headless` works (R7-M3). Run the adapter + host on one dedicated thread with a current-thread runtime (§5.5, R9-H1). | `agent/cli/src/main.rs` |
| T2.11 | Write the MCP conformance test **in the package that declares the binary** (HIGH-B2). | `agent/cli/tests/mcp_conformance.rs` |

### Gate (Definition of Done)

1. `cargo test -p graphite-agent-host` → all pass.
2. `cargo test -p graphite-agent-cli --test mcp_conformance` → pass (HIGH-B2).
3. The conformance test drives `initialize` → `tools/list` → `document.new` → `graph.add_node` ×2 → `graph.connect` → `render.preview` → `render.export` → `history.undo`, asserting correlated ids and a non-empty PNG.
4. **Generated-vs-curated check:** `tools/list` == §17 curated tools + `node.list_types` + `node.describe`, and nothing else. Catalog entry count > 0 and equals the `NODE_METADATA` count.
5. Capability: `--capabilities read` → `graph.add_node` returns `Unauthorized`; `read,author` → succeeds.
6. Cancellation: issuing `notifications/cancelled` for a long `render.preview` resolves `Cancelled` (proves `ToolHost::cancel(&self)` works in flight — HIGH-A4).
7. Path confinement: `render.export` with `../escape.png` returns `PathOutsideRoot`.
8. Undo/redo via `history.*` removes and restores a node.
9. `render.preview` returns a non-empty PNG when an adapter is available; otherwise mark that assertion `PARTIAL` and still assert compilation.
10. `cargo clippy -p graphite-agent-host -p graphite-agent-mcp -p graphite-agent-cli --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).
11. No file in `agent/` exceeds 3000 lines.

### Rollback
Delete `agent/host`, `agent/mcp`, `agent/cli` from `members`. Phases 0–1 remain valid.

---

## 11. MCP wire mapping (v1)

| Concern | Decision |
|---|---|
| Protocol version | `2025-06-18` |
| `initialize` result | `protocolVersion: "2025-06-18"`, `capabilities: { tools: {}, resources: {}, prompts: {}, logging: {} }`, `serverInfo: { name: "graphite-agent", version: env!("CARGO_PKG_VERSION") }`. `logging: {}` is REQUIRED because events are delivered as `notifications/message` (R8-M1). |
| `notifications/initialized` | Accepted, ignored. |
| `tools/list` | `{ tools: [ { name, description, inputSchema } ] }` |
| `tools/call` params | `{ name, arguments }` |
| Success result | `{ content: [ { type: "text", text: "<compact JSON>" } ], structuredContent: <result>, isError: false }` |
| Tool failure | `{ content: [ { type: "text", text: "<message>" } ], isError: true }` — not a JSON-RPC error |
| Unknown tool / malformed params | JSON-RPC error `-32602` |
| Internal panic | JSON-RPC error `-32603` |
| `notifications/cancelled` | `{ requestId, reason? }` → map JSON-RPC id to `QueryId` per connection → `host.cancel(id)` |
| JSON-RPC id ↔ `QueryId` | Host allocates `QueryId` (INV-14); connection keeps `HashMap<JsonRpcId, QueryId>`; never reuse a live id |
| Events | `notifications/message` with `{ level, data: <AgentEvent JSON> }`; accept `logging/setLevel` and ignore the threshold (R8-M1) |
| Resources | `graphite://node-catalog`, `graphite://command-catalog` (generated command descriptors, read-only catalog data — never tools, E-8), `graphite://document/{id}/registry`, `graphite://document/{id}/preview` |
| Prompts | Three prompts defined verbatim in §11.1 |
| stdout | JSON-RPC only (INV-10) |

### 11.1 Prompt definitions (M-5 — copy verbatim)

`prompts/get` returns exactly these `messages[]` contents. Arguments are substituted with `{{name}}` before returning. Do not invent additional prompts.

```json
[
  {
    "name": "author_procedural_graph",
    "description": "Draft a node graph for a procedural pattern.",
    "arguments": [
      { "name": "goal", "description": "What the artwork should depict.", "required": true }
    ],
    "template": "You are authoring a Graphite node graph. Goal: {{goal}}. Use node.list_types and node.describe to discover nodes. Build the graph with graph.add_node, graph.set_input, and graph.connect inside a history.begin/history.commit pair. Render with render.preview and correct any compile errors before finishing."
  },
  {
    "name": "explain_graph",
    "description": "Explain an existing graph in plain language.",
    "arguments": [
      { "name": "document_id", "description": "Document to inspect.", "required": true }
    ],
    "template": "Read document {{document_id}} with graph.list_nodes and graph.get_node. Explain, in plain language, what the graph computes from inputs to the exported output. Name each node's role and describe the data flowing between them."
  },
  {
    "name": "repair_graph",
    "description": "Diagnose and fix a graph that fails to compile or render.",
    "arguments": [
      { "name": "document_id", "description": "Document to repair.", "required": true }
    ],
    "template": "Inspect document {{document_id}}. Call render.preview; if it reports errors, use graph.list_nodes and graph.get_node to locate the broken or unconnected inputs. Repair them with graph.connect and graph.set_input inside a single transaction, then re-render until the preview succeeds."
  }
]
```

---

## 12. Phase 3 — D2: catalog generalization and curation

**Goal:** full, versioned, agent-safe catalog driven by Phase 2's real usage.
**Depends on:** Phase 2 gated.
**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T3.1 | Expand `agent/allowlist.toml` to the reviewed agent-safe set. Every entry needs a one-line justification. Default-deny remains. | `agent/allowlist.toml` |
| T3.2 | Enumerate message actions **with an active document**, so document/tool actions are included (M-A3). `agent/descriptors` already depends on `editor`, so construct `Editor::new_headless(...)` directly, dispatch `PortfolioMessage::NewDocumentWithName` to open the first document, then call `editor.dispatcher.collect_actions()`. **Do NOT depend on `agent/host` here** — that would create a `host → descriptors → host` Cargo cycle (HIGH-1). Record the procedure in `agent/README.md`. | `agent/descriptors/src/actions.rs` |
| T3.3 | Extend descriptor **generation** to **command descriptors**: `ToolDescriptor` catalog entries derived from a live `collect_actions()` result plus message field metadata, exactly like `node.type.*`. They are **catalog data** (§6.3, E-8) — they never enter `tools/list`, the host's `module_index`, or any `ToolModule::descriptors()`, and executing a message action by name remains forbidden (INV-13). | `agent/descriptors/src/commands.rs` |
| T3.4 | **Verify first:** check whether the existing `HierarchicalTree` derive already emits field names and types. Add a test asserting the generated command `input_schema` for one known message has the expected properties. Only extend the macro if proven insufficient; record the finding. | `agent/descriptors/src/commands.rs`, possibly `proc-macros/src/` |
| T3.5 | Implement catalog versioning per §5.6 with a test proving a version bump does not change `agent/protocol` or the MCP adapter. | `agent/descriptors/src/version.rs` |
| T3.6 | Add `coverage` output: generated vs total for nodes and for allowlisted messages; list every non-allowlisted message with status. Also expose the generated command catalog at `graphite://command-catalog` (§11) so the descriptors have a real read path. | `agent/descriptors/src/main.rs` |
| T3.7 | Add deprecation support (keep an old descriptor one phase after a bump). | `agent/descriptors/src/version.rs` |

### Gate (Definition of Done)

1. `cargo test -p graphite-agent-descriptors` → all pass (runs the T3.4/T3.5/T3.7 tests, M-3).
2. Every `NODE_METADATA` entry yields a validator-passing descriptor; count > 0.
3. Every allowlisted message action yields a descriptor with a non-empty `input_schema`, **including document/tool actions** (proves T3.2 used an active document, M-A3).
4. Coverage shows 100% for nodes and for allowlisted messages; all others listed with status.
5. A version-bump test passes without touching `agent/protocol` or `agent/mcp`.
6. `cargo clippy -p graphite-agent-descriptors --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).
7. **No command descriptor is a tool (E-8).** `tools/list` still equals the §17 curated tools plus `node.list_types` and `node.describe`, and contains no `command.*` name — re-run the Phase 2 gate-4 comparison. Every generated command descriptor is reachable only through `graphite://command-catalog` / `coverage`.

### Rollback
Revert `agent/descriptors` and the allowlist to the Phase 2 state.

---

## 13. Phase 4 — Surface B (attached live session)

**Goal:** an agent and a human edit one live document with one undo stack.
**Depends on:** Phase 3 gated.
**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T4.1 | Extend the desktop socket to **bidirectional, length-prefixed, correlated** framing with **three frame kinds: `Request`, `Response(ToolOutcome)`, `Event(AgentEvent)`** (R9-M1), plus a fourth control frame `Cancel { id }` (E-13). Frame = 4-byte big-endian length prefix + RON-encoded payload; `Request` **must carry the host-allocated `QueryId`** (`Request { id, query }`) or correlation is impossible, and `Cancel` cannot be a `BridgeQuery` because the frozen `EditorBridge::cancel` has no query representation. Current `desktop/src/socket.rs` is one-way (`Message::OpenFiles`) with no reply path; keep that behavior working (its RON payload is distinguishable from a length prefix). Give the server access to the editor (via `DesktopWrapper`). | `desktop/src/socket.rs`, `desktop/src/lib.rs` |
| T4.2 | Add a desktop enablement flag `--agent-bridge` (default off). Without it, behavior is byte-for-byte unchanged (regression-tested). | `desktop/src/cli.rs`, `desktop/src/lib.rs` |
| T4.3 | Install an `AgentReplySink` on the desktop editor via `Editor::set_agent_reply_sink` (call site is `desktop/wrapper/src/lib.rs`, in scope) and add a forwarder task that relays sink output to the socket. Handle inbound frames by dispatching `AgentMessage`s. Message system only (INV-2). | `desktop/wrapper/src/lib.rs`, `desktop/wrapper/Cargo.toml`, `desktop/src/agent_bridge.rs` |
| T4.4 | Implement `AttachedBridge` in `agent/host` over the socket: `submit` writes a `Request` frame, `poll` reads the correlated `Response`, `cancel` writes a cancel frame, `drain_events` buffers the `Event` frames received on the socket (R9-M1). Add `interprocess` to `agent/host`. | `agent/host/src/attached.rs`, `agent/host/Cargo.toml` |
| T4.5 | Live projections: `session.snapshot`, `session.selection`, `session.active_document`, backed by `SnapshotProjection`. | `agent/host/src/modules/session.rs` |
| T4.6 | Forward editor state changes as `AgentEvent::DocumentChanged`, debounced to ≤ 1 per 100 ms per document. | `desktop/src/agent_bridge.rs`, `agent/host/src/host.rs` |
| T4.7 | Capability attenuation per connection: grant set configured at launch, default `read,author` in attached mode (matching T2.10's per-mode default, R8-M2); refuse others with `Unauthorized`. | `agent/host/src/capability.rs` |
| T4.8 | Add the MCP Streamable HTTP adapter using `tiny_http`, bound to `127.0.0.1` by default (M-B4). Reuses the same `ToolHost`. | `agent/mcp/src/http.rs` |
| T4.9 | Attached conformance test. | `agent/cli/tests/attached_conformance.rs` |
| T4.10 | Wire mode selection into the shipped binary: `graphite-agent --mode attached --attach <socket> --root <path> [--http <addr>]`. Construct `Host::new(Box::new(AttachedBridge::connect(socket)?), capabilities, timeout, root)` — the 4th `root` argument is the INV-12 confinement point (E-6, as in T2.2) — and serve stdio and/or HTTP. Without this task the Modes B/C surfaces are reachable only from tests (R7-M3). | `agent/cli/src/main.rs`, `agent/host/src/attached.rs` |

### Gate (Definition of Done)

1. `cargo test -p graphite-agent-host attached::` → pass.
2. `cargo test -p graphite-agent-cli --test attached_conformance` → pass (runs T4.9, M-3).
3. Live-session test: a tool mutation is visible through `session.snapshot`; the agent can read state it did not cause.
4. Atomic undo: an agent transaction undoes as one step.
5. Concurrency: with an interleaved human-side mutation, responses correlate and state is uncorrupted.
6. Notifications: a human-side change arrives as one debounced `DocumentChanged` event.
7. Capability refusal: an `Execute` call on a `read,author` connection returns `Unauthorized`.
8. Regression: desktop without `--agent-bridge` behaves as before.
9. HTTP adapter serves `tools/list` on localhost and refuses non-loopback binds unless explicitly configured.
10. `cargo clippy -p graphite-agent-host -p graphite-agent-mcp --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).
11. The shipped binary actually starts attached mode: `graphite-agent --mode attached --attach <test-socket> --root <test-root> --stdio` answers `tools/list` (R7-M3, R8-M4).

### Rollback
Revert `agent/host/attached.rs`, `desktop/src/agent_bridge.rs`, the socket extension, the `--agent-bridge` flag, and the `desktop/wrapper` sink installation.

---

## 14. Phase 5 — Surface C (CRDT peer)

**Goal:** an attributed agent peer on `.gdd`, with replay and semantic validation.
**Depends on:** Phase 3 gated. **Precondition:** target documents use `.gdd` as the sole persisted format. Assert at open time that the manifest declares format `gdd` and that no `legacy.graphite` payload is the source of truth; otherwise `ToolError::InvalidArguments`.

**Write scope:** see Appendix D.

### Tasks

| ID | Do exactly this | Files |
|---|---|---|
| T5.1 | Implement `PeerBridge` over `document/graph-storage`: open a `Session`, expose delta application and queries through a typed interface. | `agent/host/src/peer.rs` |
| T5.2 | Register the agent as an attributed peer: `RegistryDelta::RegisterPeer { peer, user }` on first contribution; later deltas carry the agent's `PeerId`. | `agent/host/src/peer.rs` |
| T5.3 | Implement `registry.apply_delta`, `registry.query`, `registry.merge`, `history.replay`. | `agent/host/src/modules/registry.rs` |
| T5.4 | Semantic validation (INV-9): after every accepted apply/merge, compile the graph; on failure return `InvalidGraph` and leave the session unchanged. | `agent/host/src/modules/registry.rs` |
| T5.5 | Replay tooling: reconstruct a registry from history and assert equality with the live registry. | `agent/host/src/peer.rs`, tests |
| T5.6 | Assert the precondition at open time. | `agent/host/src/peer.rs` |
| T5.7 | Wire peer mode into the shipped binary: `graphite-agent --mode peer --gdd <path> --root <path>`, constructing `Host::new(Box::new(PeerBridge::open(path)?), capabilities, timeout)` and serving stdio (R7-M3). | `agent/cli/src/main.rs`, `agent/host/src/peer.rs` |

### Gate (Definition of Done)

1. `cargo test -p graphite-agent-host` → all pass, unfiltered, so the `modules::registry::*` tests from T5.3/T5.4 run (M-3). The `peer::` filter alone misses them.
2. Replay reconstruction equals the live registry: assert `replayed == live` (or `Registry::value_equal`), **not** byte equality of serialized output — `HashMap` iteration order makes byte equality flaky (R8-M3).
3. Every applied delta is attributed to the agent's `PeerId`.
4. A semantically invalid graph is rejected with `InvalidGraph`; the session is unchanged.
5. Multi-peer test: agent + simulated human deltas merge without loss; both visible via `registry.query`.
6. The precondition assertion rejects a legacy `.graphite` document.
7. `cargo clippy -p graphite-agent-host --all-targets --no-deps` → no new diagnostics vs `agent/clippy-baseline.txt` (E-4).
8. The shipped binary actually starts peer mode: `graphite-agent --mode peer --gdd <test.gdd> --root <test-root> --stdio` answers `tools/list` (R7-M3, R8-M4).

### Rollback
Revert `agent/host/src/peer.rs` and `modules/registry.rs`. Modes A and B unaffected.

---

## 15. Cross-phase checklist

- [ ] Every task in the phase table is complete, in order.
- [ ] Every gate command ran; output recorded.
- [ ] `cargo fmt --all -- --check` clean.
- [ ] `cargo clippy -p <touched crates> --all-targets --no-deps` adds no diagnostics vs `agent/clippy-baseline.txt`.
- [ ] No frozen interface changed (only additive `#[non_exhaustive]` variants).
- [ ] No file in `agent/` exceeds 3000 lines.
- [ ] No handler private fields read; only `DocumentQuery`/`AgentMessage::Reply`.
- [ ] No arbitrary `Message` deserialized or dispatched.
- [ ] No client-supplied capability trusted; only the host allocated `QueryId`s.
- [ ] No hand-written node-type descriptor added.
- [ ] No bare `cargo run` used.
- [ ] Only Appendix D files for this phase changed.

---

## 16. Escalation and blocker protocol

Stop and escalate when any of these is true:

1. A gate command failed twice with the same error.
2. A frozen interface appears insufficient.
3. A needed third-party crate is not on the pre-approved list (§4.1).
4. A task requires editing a file outside Appendix D.
5. Two tasks in the same phase contradict each other.

**Procedure:** create `agent/BLOCKED-<phase>.md` with the phase, task id, exact command, full output, hypothesis, and the smallest unblocking question. Then stop. Do not attempt an alternative design. Report the blocker path to the Lead.

---

## 17. Appendix A — Phase 2 MCP tool list (v1)

| Tool | Capability | Notable arguments | Result | Source |
|---|---|---|---|---|
| `document.new` | Persist | `name` | `document_id` | curated |
| `document.open` | Read | `path` | `document_id` | curated |
| `document.save` | Persist | `document_id`, `path` | `saved` | curated (ExportGdd + host write) |
| `document.close` | Persist | `document_id` | `closed` | curated |
| `document.list` | Read | — | `documents[]` | curated |
| `node.list_types` | Read | — | `types[]` | generated |
| `node.describe` | Read | `identifier` | descriptor | generated |
| `graph.add_node` | Author | `document_id`, `identifier`, `x`, `y` | `node_id` | curated |
| `graph.remove_node` | Author | `document_id`, `node_id` | `removed` | curated |
| `graph.set_input` | Author | `document_id`, `node_id`, `input_index`, `value` | `node_id` | curated |
| `graph.connect` | Author | `document_id`, `from_node`, `from_output`, `to_node`, `to_input` | `connected` | curated |
| `graph.disconnect` | Author | `document_id`, `to_node`, `to_input` | `disconnected` | curated |
| `graph.list_nodes` | Read | `document_id` | `nodes[]` | curated |
| `graph.get_node` | Read | `document_id`, `node_id` | node | curated |
| `history.undo` / `history.redo` | Author | `document_id` | `changed` | curated |
| `history.begin` / `history.commit` / `history.abort` | Author | `document_id` | `ok` | curated |
| `render.preview` | Execute | `document_id`, `max_dimension` | `image_base64_png` | curated |
| `render.export` | Export | `document_id`, `path`, `format`, `scale` | `path` | curated |

**Phase 4 additions:** `session.snapshot`, `session.selection`, `session.active_document`.
**Phase 5 additions:** `registry.apply_delta`, `registry.query`, `registry.merge`, `history.replay`.

---

## 18. Appendix B — Command cheat sheet

```sh
# Phase 0
cargo build -p graphite-agent-protocol -p graphite-agent-descriptors -p graphene-cli
cargo test  -p graphite-agent-protocol -p graphite-agent-descriptors
cargo run   -p graphite-agent-descriptors -- inventory --out agent/inventory.json

# Phase 1
cargo test -p graphite-editor agent::
cargo test -p graphite-editor --test single_editor

# Phase 2
cargo test -p graphite-agent-host
cargo test -p graphite-agent-cli --test mcp_conformance

# Phase 3
cargo run -p graphite-agent-descriptors -- coverage

# Phase 4 / 5
cargo test -p graphite-agent-host attached::
cargo test -p graphite-agent-cli --test attached_conformance
cargo test -p graphite-agent-host peer::

# Always before declaring done
cargo fmt --all -- --check
cargo clippy -p <crate> --all-targets --no-deps  # must add no diagnostics vs agent/clippy-baseline.txt
```

**Forbidden:** `cargo run` with no `-p`.

---

## 19. Appendix C — Known hard constraints

| Constraint | Consequence |
|---|---|
| `Editor::new` installs a process-global `ENVIRONMENT` and panics on a second call | One editor per process. Multi-session = multiple processes. |
| `NODE_METADATA` filled by `#[ctor]` registrations | Link node crates (`graphene-std` with `shader-nodes`); `core-types` alone yields an empty catalog. |
| `shader-nodes` gates GPU node registration | Descriptors/host must enable it or the catalog/renders diverge from the live editor (M-A9). |
| `NODE_METADATA` is behind a `Mutex` | Every read locks; handle poisoning. |
| `Message` derives `Serialize`/`Deserialize` | Only curated enums may be dispatched (INV-13). |
| `DocumentMessage::SaveDocument` opens a frontend dialog | Headless persistence uses `ExportGdd` + host-side write. |
| `Gdd::export_to_bytes` is `async` | Export runs on the document subsystem via an async closure and replies through `AgentMessage::Reply` (INV-15). |
| `run_node_graph` is `async` and `!Send` (holds a std `MutexGuard` across `.await`) | `EditorBridge::pump`, `ToolModule::execute`, and `PendingCall.outcome` must NOT be `+ Send`; run the host and adapter on one dedicated thread with a current-thread runtime (R9-H1). |
| `poll_node_graph_evaluation` returns `Err("No active document")` before any document exists | Treat that exact string as non-fatal (HIGH-A3). |
| `poll_node_graph_evaluation` collects but does not dispatch | Re-dispatch collected messages as a batch (R2-K). |
| `ToolHost::call` must be cancellable in flight | `call(&self)` returns a `'static` `PendingCall`; the host uses interior mutability and a shared cancel set (HIGH-A4). |
| `CARGO_BIN_EXE_*` is only set for the declaring package's tests | Conformance tests live in `agent/cli/tests/` (HIGH-B2). |
| `graphite-cli` was binary-only | Phase 0 T0.3 adds a library target. |
| `desktop/src/socket.rs` is one-way and `desktop/ui` has no editor access | Phase 4 T4.1 adds bidirectional correlated framing and routes through `DesktopWrapper` (M-B5). |
| stdio MCP shares stdout | Logging to stderr (INV-10). |
| `NodeId` is hash-derived | Allocate `NodeId::new()` and supply it (R2-A). |
| `Platform::Desktop` is the only viable native headless environment | Desktop-only paths are active; bypass dialogs and use `document-format` (R2-H). |

---

## 20. Appendix D — Phase write scopes (M-A6, M-B2)

Edit **only** these paths, plus the phase's newly created `agent/` files and any `Cargo.toml` listed.

| Phase | Permitted existing paths |
|---|---|
| 0 | root `Cargo.toml`; `node-graph/graphene-cli/{Cargo.toml, src/main.rs, src/lib.rs, src/engine.rs, src/export.rs}`; `agent/**` |
| 1 | `editor/Cargo.toml`; `editor/src/application.rs`; `editor/src/dispatcher.rs`; `editor/src/messages/message.rs`; `editor/src/messages/mod.rs`; `editor/src/messages/prelude.rs`; `editor/src/messages/agent/**`; `editor/src/messages/portfolio/document/mod.rs`; `editor/src/messages/portfolio/document/query.rs`; `editor/src/messages/portfolio/document/document_message.rs`; `editor/src/messages/portfolio/document/document_message_handler.rs`; `editor/tests/single_editor.rs`; `agent/protocol/**`; root `Cargo.toml` |
| 2 | `agent/**`; root `Cargo.toml` |
| 3 | `agent/**`; `proc-macros/src/**` (only if T3.4 proves it necessary); root `Cargo.toml`. **T3.3's files are `agent/descriptors/src/commands.rs` only (E-8):** `agent/host/**` is in this phase's scope only for T3.6's read path, and command descriptors must never be wired into `Host::new`, `module_index`, or a `ToolModule`. |
| 4 | `agent/host/**`; `agent/mcp/**`; `agent/cli/**`; `agent/*/Cargo.toml`; `desktop/src/{socket.rs, cli.rs, lib.rs, app.rs, event.rs, agent_bridge.rs}`; `desktop/wrapper/src/lib.rs`; `desktop/wrapper/Cargo.toml`; root `Cargo.toml` |
| 5 | `agent/host/**`; `document/graph-storage/**` (only if a typed query boundary is missing); root `Cargo.toml` |

Any path not listed here is out of scope for that phase (§16.4).

---

## 21. Phase dependency graph

```
Phase 0 ──▶ Phase 1 ──▶ Phase 2 ──▶ Phase 3 ──▶ Phase 4 ──▶ Phase 5
                 │                      │
                 └── contract frozen ───┘
                 └── no redesign after Phase 1 gate ──┘
```

Phases 0–3 are strictly sequential. **Phases 4 and 5 must also be serialized (4 before 5, or 5 before 4):** although they share only the contract logically, both edit the `agent/host` registration files (`agent/host/src/lib.rs`, `agent/host/src/modules/mod.rs`), `agent/host/Cargo.toml`, and root `Cargo.toml`. Running them concurrently guarantees a write collision (M-4). They may be developed in parallel only if T2.1 first creates the complete module skeleton so neither later phase opens those files.
