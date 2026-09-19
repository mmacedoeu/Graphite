//! Peer mode (Surface C, T5.1–T5.6): an attributed CRDT peer over a `.gdd` working copy.
//!
//! [`PeerBridge`] implements the frozen [`EditorBridge`] contract so it can be handed to
//! [`Host::new`](crate::Host) exactly like the headless/attached bridges, but peer mode has
//! no live editor: the graph lives in a [`Session`] over `document/graph-storage`, and the
//! agent participates as its own attributed [`PeerId`].
//!
//! # Typed boundary
//!
//! The shared state is [`PeerState`], a wrapper over `document_graph_storage::Session` exposing
//! delta application ([`PeerState::apply_delta`]), cross-peer integration
//! ([`PeerState::merge`]), typed reads ([`PeerState::query`]), and replay verification
//! ([`PeerState::replay_check`]). Both this bridge and
//! [`RegistryModule`](crate::modules::registry::RegistryModule) hold the same
//! [`PeerHandle`] (`Arc<Mutex<PeerState>>`), so the tool module never reads message-handler
//! private state (INV-2) and never deserializes an arbitrary `Message` (INV-13): it only
//! speaks this typed interface.
//!
//! # `.gdd` precondition (T5.6)
//!
//! [`PeerBridge::open_with_peer`] refuses any target whose manifest does not declare format
//! `gdd`, and any target where a `legacy.graphite` payload would be the source of truth
//! (no `.gdd` registry or history payload present). Both are [`ToolError::InvalidArguments`].
//!
//! # Attribution (T5.2)
//!
//! The first contribution stages a `RegistryDelta::RegisterPeer { peer, user }` retired delta
//! for the agent's `PeerId`; every later apply is a delta authored by that same `PeerId`.

use document_container::backends::folder::FolderBackend;
use document_container::{AnyContainer, AsyncContainer};
use document_format::io as gdd_io;
use document_format::manifest::FORMAT_MAGIC;
use document_format::{GddV1Layout, Layout, MANIFEST_CODEC, Manifest, PayloadCodecs, SessionState};
use document_graph_storage::{CrdtError, Declarations, Delta, HotOp, Implementation, NodeId, PeerId, Registry, RegistryDelta, Rev, Session, TimeStamp, UserId, decode_declaration};
use graphite_agent_protocol::{AgentEvent, BridgeQuery, EditorBridge, QueryId, ToolError};
use serde_json::{Value, json};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

/// Shared, host-owned peer state. The [`PeerBridge`] and the registry tool module both hold
/// an `Arc` clone, so a tool call mutates exactly the session the bridge is built over.
pub type PeerHandle = Arc<Mutex<PeerState>>;

/// The agent's CRDT participation in one `.gdd` document.
pub struct PeerState {
	session: Session,
	peer: PeerId,
	/// Monotonic local counter for hot-op timestamps. Initialized past every loaded timestamp
	/// so the agent never mints a causally-older stamp.
	next_counter: u64,
	/// Proto-node declarations resolved from the working copy at open, used by the semantic
	/// validation pass (INV-9).
	declarations: Declarations,
	/// The working copy this session was loaded from, kept for persistence and resource reads.
	working: AnyContainer,
	layout: GddV1Layout,
	codecs: PayloadCodecs,
}

/// The peer-mode bridge: a `!Send`-free, typed handle over one [`PeerState`].
pub struct PeerBridge {
	state: PeerHandle,
	peer: PeerId,
}

impl PeerBridge {
	/// Open a `.gdd` working copy as a fresh agent peer.
	pub async fn open(path: &Path) -> Result<Self, ToolError> {
		// `Session::new` mints a fresh per-process peer. Two bridges in one process would
		// collide (the UUID generator is seeded once), which is why tests use
		// `open_with_peer`.
		let peer = Session::new().peer();
		Self::open_with_peer(path, peer).await
	}

	/// Open a `.gdd` working copy bound to an explicit peer identity (deterministic tests).
	pub async fn open_with_peer(path: &Path, peer: PeerId) -> Result<Self, ToolError> {
		let state = PeerState::load(path, peer).await?;
		Ok(Self {
			state: Arc::new(Mutex::new(state)),
			peer,
		})
	}

	/// The shared typed handle, for the host to hand to the registry tool module.
	pub fn handle(&self) -> PeerHandle {
		Arc::clone(&self.state)
	}

	/// The agent's attributed peer identity.
	pub fn agent_peer(&self) -> PeerId {
		self.peer
	}
}

impl EditorBridge for PeerBridge {
	fn submit(&mut self, _id: QueryId, _query: BridgeQuery) -> Result<(), ToolError> {
		// Peer mode is not an editor session: it exposes the `registry.*` surface through the
		// typed `PeerState` interface. Editor-shaped bridge queries are unavailable.
		Err(invalid("peer mode exposes the registry.* tools; editor bridge queries are unavailable"))
	}

	fn poll(&mut self, _id: QueryId) -> Option<Result<Value, ToolError>> {
		None
	}

	fn cancel(&mut self, _id: QueryId) {}

	fn drain_events(&mut self) -> Vec<AgentEvent> {
		Vec::new()
	}

	fn pump(&mut self) -> Pin<Box<dyn Future<Output = Result<(), ToolError>> + '_>> {
		Box::pin(async { Ok(()) })
	}
}

impl PeerState {
	/// Open and normalize one `.gdd` working copy.
	async fn load(path: &Path, peer: PeerId) -> Result<Self, ToolError> {
		let working = AnyContainer::Folder(FolderBackend::open(path).map_err(|error| invalid(format!("cannot open .gdd working copy at {}: {error}", path.display())))?);
		let layout = GddV1Layout;

		let manifest: Manifest = gdd_io::read_single(&working, layout.manifest_basename(), MANIFEST_CODEC)
			.await
			.map_err(|error| invalid(format!(".gdd manifest is missing or malformed: {error}")))?;
		if manifest.format != FORMAT_MAGIC {
			return Err(invalid(format!("precondition failed: manifest format is {:?}, expected {FORMAT_MAGIC:?}", manifest.format)));
		}
		let codecs = manifest.codecs;

		let has_registry = gdd_io::exists(&working, layout.registry_basename(), codecs.registry).await;
		let has_history = gdd_io::exists(&working, layout.history_basename(), codecs.history).await;
		// "No legacy.graphite payload is the source of truth": the embedded legacy blob may
		// ride along during the dual-write soak, but it must never be the only data payload.
		if working.exists(layout.legacy_path()).await && !has_registry && !has_history {
			return Err(invalid("precondition failed: a legacy.graphite payload is the source of truth; .gdd must be the sole persisted format"));
		}

		let session_state: SessionState = if gdd_io::exists(&working, layout.session_basename(), codecs.session).await {
			gdd_io::read_single(&working, layout.session_basename(), codecs.session)
				.await
				.map_err(|error| invalid(format!("session payload is malformed: {error}")))?
		} else {
			SessionState::default()
		};

		let mut session = if has_registry && has_history {
			let registry: Registry = gdd_io::read_single(&working, layout.registry_basename(), codecs.registry)
				.await
				.map_err(|error| invalid(format!("registry payload is malformed: {error}")))?;
			let history: Vec<Delta> = gdd_io::iter(&working, layout.history_basename(), codecs.history)
				.await
				.map_err(|error| invalid(format!("history payload is malformed: {error}")))?;
			Session::load(peer, registry, history, session_state.head_rev, session_state.redo_stack.clone(), session_state.next_node_counter)
		} else if has_registry {
			let registry: Registry = gdd_io::read_single(&working, layout.registry_basename(), codecs.registry)
				.await
				.map_err(|error| invalid(format!("registry payload is malformed: {error}")))?;
			Session::bootstrap_from_registry(peer, registry).map_err(crdt_error)?
		} else {
			let history: Vec<Delta> = if has_history {
				gdd_io::iter(&working, layout.history_basename(), codecs.history)
					.await
					.map_err(|error| invalid(format!("history payload is malformed: {error}")))?
			} else {
				Vec::new()
			};
			Session::replay_from_history(peer, history, session_state.next_node_counter).map_err(crdt_error)?
		};

		// Normalize the hot zone: promote every pending hot op to a retired delta so the working
		// registry equals the retired snapshot. Replay verification then compares like for like.
		if let Some(up_to) = session.hot_log().iter().map(|hot| hot.timestamp).max() {
			session.retire(up_to).map_err(crdt_error)?;
		}

		let mut next_counter = 0u64;
		for delta in session.history() {
			next_counter = next_counter.max(delta.timestamp.counter);
		}

		let declarations = resolve_declarations(session.registry(), &working, &layout).await;

		Ok(Self {
			session,
			peer,
			next_counter,
			declarations,
			working,
			layout,
			codecs,
		})
	}

	/// The agent's attributed peer identity.
	pub fn peer(&self) -> PeerId {
		self.peer
	}

	/// The live merged registry.
	pub fn registry(&self) -> &Registry {
		self.session.registry()
	}

	/// The current retired-head rev.
	pub fn head_rev(&self) -> Option<Rev> {
		self.session.head_rev()
	}

	/// The durable delta history in causal order.
	pub fn history(&self) -> impl Iterator<Item = &Delta> {
		self.session.history()
	}

	/// Apply one agent-authored delta as a retired commit, then semantically validate the graph.
	///
	/// On any failure (including [`ToolError::InvalidGraph`]) the session is restored to its
	/// pre-call state, so a rejected delta is never partially applied (T5.4).
	pub fn apply_delta(&mut self, op: RegistryDelta) -> Result<Value, ToolError> {
		let snapshot = self.session.clone();
		match self.try_apply_delta(op) {
			Ok(result) => {
				self.persist()?;
				Ok(result)
			}
			Err(error) => {
				self.session = snapshot;
				Err(error)
			}
		}
	}

	fn try_apply_delta(&mut self, op: RegistryDelta) -> Result<Value, ToolError> {
		self.ensure_self_registered()?;
		let timestamp = self.stamp();
		self.session.apply_hot_op(HotOp { op, timestamp }).map_err(crdt_error)?;
		let revs = self.session.retire(timestamp).map_err(crdt_error)?;
		self.validate()?;
		Ok(json!({
			"applied": true,
			"rev": revs.last().map(ToString::to_string),
			"head": self.session.head_rev().map(|rev| rev.to_string()),
		}))
	}

	/// Integrate incoming retired deltas from another peer, then semantically validate.
	///
	/// On failure the session is restored, so a merge never leaves a half-applied branch.
	pub fn merge(&mut self, deltas: Vec<Delta>) -> Result<Value, ToolError> {
		let snapshot = self.session.clone();
		match self.try_merge(deltas) {
			Ok(result) => {
				self.persist()?;
				Ok(result)
			}
			Err(error) => {
				self.session = snapshot;
				Err(error)
			}
		}
	}

	fn try_merge(&mut self, deltas: Vec<Delta>) -> Result<Value, ToolError> {
		self.ensure_self_registered()?;
		let rev = self.session.merge(deltas).map_err(crdt_error)?;
		self.validate()?;
		Ok(json!({
			"merged": rev.is_some(),
			"rev": rev.map(|rev| rev.to_string()),
			"head": self.session.head_rev().map(|rev| rev.to_string()),
		}))
	}

	/// Typed read of the merged registry (T5.3 `registry.query`).
	pub fn query(&self, node_id: Option<u64>) -> Result<Value, ToolError> {
		let registry = self.session.registry();
		let head = self.session.head_rev().map(|rev| rev.to_string());

		if let Some(id) = node_id {
			let node = registry.node_instances.get(&NodeId(id)).ok_or_else(|| ToolError::NotFound { what: format!("node {id}") })?;
			return Ok(json!({
				"peer_id": self.peer.0,
				"head": head,
				"node_id": id,
				"network": node.network().0,
				"input_count": node.inputs().len(),
				"implementation": implementation_json(node.implementation()),
			}));
		}

		let mut nodes: Vec<Value> = registry
			.node_instances
			.iter()
			.map(|(id, node)| {
				json!({
					"node_id": id.0,
					"network": node.network().0,
					"input_count": node.inputs().len(),
					"implementation": implementation_json(node.implementation()),
				})
			})
			.collect();
		nodes.sort_by_key(|node| node["node_id"].as_u64().unwrap_or(0));

		let mut networks: Vec<Value> = registry
			.networks
			.iter()
			.map(|(id, network)| json!({ "network_id": id.0, "export_count": network.exports.len() }))
			.collect();
		networks.sort_by_key(|network| network["network_id"].as_u64().unwrap_or(0));

		let peer_users: serde_json::Map<String, Value> = registry.peer_users.iter().map(|(peer, user)| (peer.0.to_string(), json!(user.0))).collect();

		Ok(json!({
			"peer_id": self.peer.0,
			"head": head,
			"node_count": registry.node_instances.len(),
			"nodes": nodes,
			"networks": networks,
			"peer_users": peer_users,
			"attributes": registry.attributes,
		}))
	}

	/// Reconstruct the registry from history and assert it equals the live registry **by value**
	/// (T5.5, gate 2). Byte equality is deliberately not used: `HashMap` iteration order makes
	/// serialized output nondeterministic.
	pub fn replay_check(&mut self) -> Result<Value, ToolError> {
		let deltas: Vec<Delta> = self.session.history().cloned().collect();
		let mut replayed = Session::replay_from_history(self.peer, deltas, self.session.next_node_counter()).map_err(crdt_error)?;
		for hot in self.session.hot_log() {
			replayed.replay_hot_op(hot.clone()).map_err(crdt_error)?;
		}

		let live = self.session.registry();
		if !replayed.registry().value_equal(live) {
			return Err(internal("replayed registry diverges from the live registry by value"));
		}
		Ok(json!({
			"equal": true,
			"node_count": live.node_instances.len(),
			"network_count": live.networks.len(),
			"history_length": self.session.history().count(),
		}))
	}

	/// Stage the agent's `PeerId -> UserId` registration as its own retired delta, once. Later
	/// deltas then carry the agent's `PeerId` (T5.2).
	fn ensure_self_registered(&mut self) -> Result<(), ToolError> {
		if self.session.registry().peer_users.contains_key(&self.peer) {
			return Ok(());
		}
		let timestamp = self.stamp();
		self.session
			.apply_hot_op(HotOp {
				op: RegistryDelta::RegisterPeer {
					peer: self.peer,
					user: UserId(self.peer.0),
				},
				timestamp,
			})
			.map_err(crdt_error)?;
		self.session.retire(timestamp).map_err(crdt_error)?;
		Ok(())
	}

	/// Semantic validation (INV-9 / T5.4): convert the registry to a runtime node network and
	/// compile it into a proto graph. An empty graph is trivially valid; a network with no
	/// exports has nothing to compile beyond the conversion itself.
	fn validate(&self) -> Result<(), ToolError> {
		let registry = self.session.registry();
		if registry.networks.is_empty() && registry.node_instances.is_empty() {
			return Ok(());
		}

		let (network, _metadata) = registry
			.to_runtime_with_metadata(&self.declarations)
			.map_err(|error| ToolError::InvalidGraph { message: error.to_string() })?;
		if network.exports.is_empty() {
			return Ok(());
		}

		let application_io = Arc::new(graph_craft::application_io::PlatformApplicationIo::default());
		let editor_api = graphene_cli::engine::create_editor_api(application_io);

		// `graph-craft`'s compiler can assert on malformed networks; a panic is a validation
		// failure, not a process abort (the host's tool call must still answer).
		let compiled = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
			graph_craft::graphene_compiler::Compiler {}
				.compile(interpreted_executor::util::wrap_network_in_scope(network, editor_api))
				.next()
		}));

		match compiled {
			Ok(Some(Ok(_))) => Ok(()),
			Ok(Some(Err(error))) => Err(ToolError::InvalidGraph { message: error }),
			Ok(None) => Err(ToolError::InvalidGraph {
				message: "the graph could not be converted into a proto graph".to_string(),
			}),
			Err(_) => Err(ToolError::InvalidGraph {
				message: "graph compilation panicked".to_string(),
			}),
		}
	}

	/// Mint the next strictly-increasing local timestamp.
	fn stamp(&mut self) -> TimeStamp {
		self.next_counter += 1;
		TimeStamp {
			counter: self.next_counter,
			peer: self.peer,
		}
	}

	/// Mirror the retired registry, history, session cursor, and (empty) hot log back to the
	/// working copy, so the `.gdd` reflects every accepted mutation.
	fn persist(&self) -> Result<(), ToolError> {
		let layout = &self.layout;
		gdd_io::write_single(&self.working, layout.registry_basename(), self.codecs.registry, self.session.retired_registry())
			.map_err(|error| internal(format!("failed to persist registry: {error}")))?;

		let mut buffer = Vec::new();
		for delta in self.session.history() {
			self.codecs.history.append(&mut buffer, delta).map_err(|error| internal(format!("failed to encode history: {error}")))?;
		}
		let history_path = gdd_io::path_for(layout.history_basename(), self.codecs.history);
		self.working
			.write_non_blocking(&history_path, &buffer)
			.map_err(|error| internal(format!("failed to persist history: {error}")))?;

		let state = SessionState {
			peer_id: self.session.peer(),
			head_rev: self.session.head_rev(),
			last_broadcast_rev: self.session.last_broadcast_rev(),
			redo_stack: self.session.redo_stack().to_vec(),
			next_node_counter: self.session.next_node_counter(),
			..Default::default()
		};
		gdd_io::write_single(&self.working, layout.session_basename(), self.codecs.session, &state).map_err(|error| internal(format!("failed to persist session state: {error}")))?;

		// Every accepted mutation retires immediately, so the hot log is always empty.
		let hot_path = gdd_io::path_for(layout.hot_log_basename(), self.codecs.hot_log);
		self.working
			.write_non_blocking(&hot_path, &[])
			.map_err(|error| internal(format!("failed to clear the hot log: {error}")))?;

		Ok(())
	}
}

/// Resolve the `ProtoNode` declarations referenced by `registry` from the working copy's
/// content-addressed resource bytes, for the validation pass.
async fn resolve_declarations(registry: &Registry, working: &AnyContainer, layout: &GddV1Layout) -> Declarations {
	let mut declarations = Declarations::new();
	let mut seen = std::collections::HashSet::new();

	for node in registry.node_instances.values() {
		let Implementation::ProtoNode(id) = node.implementation() else { continue };
		if !seen.insert(*id) {
			continue;
		}
		let Some(hash) = registry.resources.get(id).and_then(|entry| entry.hash) else { continue };
		let Ok(bytes) = working.read(&layout.resource_path(&hash)).await else { continue };
		if let Ok(proto) = decode_declaration(bytes.as_slice()) {
			declarations.insert(*id, proto);
		}
	}

	declarations
}

/// One line of implementation identity, for `registry.query`.
fn implementation_json(implementation: &Implementation) -> Value {
	match implementation {
		Implementation::ProtoNode(id) => json!({ "proto_node": u64::from(*id) }),
		Implementation::Network(id) => json!({ "network": id.0 }),
	}
}

fn crdt_error(error: CrdtError) -> ToolError {
	invalid(format!("CRDT operation rejected: {error}"))
}

fn invalid(message: impl Into<String>) -> ToolError {
	ToolError::InvalidArguments { message: message.into() }
}

fn internal(message: impl Into<String>) -> ToolError {
	ToolError::Internal { message: message.into() }
}

/// Lock a peer handle, treating poisoning as recoverable (the mutex only guards plain data).
pub(crate) fn lock_peer(handle: &PeerHandle) -> MutexGuard<'_, PeerState> {
	handle.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	fn temp_dir(label: &str) -> PathBuf {
		let dir = std::env::temp_dir().join(format!("graphite-agent-peer-{}-{label}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).expect("create temp dir");
		dir
	}

	/// Write the minimal `.gdd` working copy: a JSON manifest. With no registry/history payload
	/// the session loads empty, which is exactly what the precondition/attribution tests need.
	fn write_minimal_gdd(dir: &Path, format: &str) {
		std::fs::create_dir_all(dir).expect("create gdd dir");
		let manifest = json!({
			"format": format,
			"format_version": 1,
			"editor_version": "test",
			"stdlib_version": "test",
			"document_id": 1,
		});
		std::fs::write(dir.join("manifest.json"), serde_json::to_vec_pretty(&manifest).expect("manifest json")).expect("write manifest");
	}

	fn parse(value: Value) -> RegistryDelta {
		serde_json::from_value(value).expect("valid RegistryDelta JSON")
	}

	fn add_root_network() -> RegistryDelta {
		parse(json!({
			"AddNetwork": { "id": 0, "network": { "exports": [], "attributes": {} } }
		}))
	}

	fn change_document_attribute(key: &str, value: &str) -> RegistryDelta {
		parse(json!({
			"ChangeDocumentAttribute": { "delta": { "key": key, "value": value } }
		}))
	}

	fn add_undeclared_node() -> RegistryDelta {
		parse(json!({
			"AddNode": { "id": 1, "node": { "implementation": { "ProtoNode": 12345 }, "inputs": [], "attributes": {}, "network": 0 } }
		}))
	}

	async fn open(dir: &Path, peer: u64) -> PeerBridge {
		PeerBridge::open_with_peer(dir, PeerId(peer)).await.expect("open peer bridge")
	}

	#[test]
	fn precondition_rejects_legacy_format() {
		let dir = temp_dir("legacy-format");
		write_minimal_gdd(&dir, "graphite");

		let error = futures::executor::block_on(PeerBridge::open(&dir)).err().expect("legacy format must be rejected");
		assert!(matches!(error, ToolError::InvalidArguments { .. }), "expected InvalidArguments, got {error:?}");
	}

	#[test]
	fn precondition_rejects_legacy_payload_as_source_of_truth() {
		let dir = temp_dir("legacy-payload");
		write_minimal_gdd(&dir, "gdd");
		std::fs::write(dir.join("legacy.graphite"), b"legacy bytes").expect("write legacy blob");

		let error = futures::executor::block_on(PeerBridge::open(&dir)).err().expect("legacy-only document must be rejected");
		assert!(matches!(error, ToolError::InvalidArguments { .. }), "expected InvalidArguments, got {error:?}");
	}

	#[test]
	fn precondition_rejects_missing_manifest() {
		let dir = temp_dir("no-manifest");
		std::fs::create_dir_all(&dir).expect("create dir");

		let error = futures::executor::block_on(PeerBridge::open(&dir)).err().expect("missing manifest must be rejected");
		assert!(matches!(error, ToolError::InvalidArguments { .. }), "expected InvalidArguments, got {error:?}");
	}

	#[test]
	fn every_applied_delta_is_attributed_to_the_agent_peer() {
		let dir = temp_dir("attribution");
		write_minimal_gdd(&dir, "gdd");
		let bridge = futures::executor::block_on(open(&dir, 11));
		let handle = bridge.handle();

		lock_peer(&handle).apply_delta(add_root_network()).expect("valid apply");
		lock_peer(&handle).apply_delta(change_document_attribute("doc::a", "one")).expect("valid apply");

		let state = lock_peer(&handle);
		assert_eq!(state.peer(), PeerId(11));
		assert!(state.history().count() > 0, "history must be non-empty");
		assert!(state.history().all(|delta| delta.author == PeerId(11)), "every applied delta must be authored by the agent peer");
		assert!(state.registry().peer_users.contains_key(&PeerId(11)), "the agent peer must be registered");
	}

	#[test]
	fn replay_reconstruction_equals_the_live_registry_by_value() {
		let dir = temp_dir("replay");
		write_minimal_gdd(&dir, "gdd");
		let bridge = futures::executor::block_on(open(&dir, 12));
		let handle = bridge.handle();

		lock_peer(&handle).apply_delta(add_root_network()).expect("valid apply");
		lock_peer(&handle).apply_delta(change_document_attribute("doc::b", "two")).expect("valid apply");

		let result = lock_peer(&handle).replay_check().expect("replay must equal the live registry");
		assert_eq!(result["equal"], json!(true));
		assert!(result["history_length"].as_u64().unwrap_or(0) >= 2);
	}

	#[test]
	fn invalid_graph_is_rejected_and_the_session_is_unchanged() {
		let dir = temp_dir("invalid");
		write_minimal_gdd(&dir, "gdd");
		let bridge = futures::executor::block_on(open(&dir, 13));
		let handle = bridge.handle();

		lock_peer(&handle).apply_delta(add_root_network()).expect("valid apply");
		let before_head = lock_peer(&handle).head_rev();
		let before_history = lock_peer(&handle).history().count();
		let before_nodes = lock_peer(&handle).registry().node_instances.len();

		let error = lock_peer(&handle).apply_delta(add_undeclared_node()).expect_err("invalid graph must be rejected");
		assert!(matches!(error, ToolError::InvalidGraph { .. }), "expected InvalidGraph, got {error:?}");

		let state = lock_peer(&handle);
		assert_eq!(state.head_rev(), before_head, "the session head must be unchanged");
		assert_eq!(state.history().count(), before_history, "no partial delta may be committed");
		assert_eq!(state.registry().node_instances.len(), before_nodes, "the registry must be unchanged");
	}

	#[test]
	fn multi_peer_merge_loses_nothing_and_both_are_visible() {
		let agent_dir = temp_dir("multi-agent");
		let human_dir = temp_dir("multi-human");
		write_minimal_gdd(&agent_dir, "gdd");
		write_minimal_gdd(&human_dir, "gdd");

		let agent = futures::executor::block_on(open(&agent_dir, 21));
		let human = futures::executor::block_on(open(&human_dir, 22));
		let agent_handle = agent.handle();
		let human_handle = human.handle();

		lock_peer(&agent_handle).apply_delta(add_root_network()).expect("agent apply");
		lock_peer(&human_handle).apply_delta(change_document_attribute("doc::human", "from-the-human")).expect("human apply");

		let incoming: Vec<Delta> = lock_peer(&human_handle)
			.history()
			.map(|delta| ron::ser::to_string(delta).expect("serialize delta"))
			.map(|encoded| ron::de::from_str(&encoded).expect("round-trip delta"))
			.collect();
		assert!(!incoming.is_empty(), "the human must have produced deltas");

		let merged = lock_peer(&agent_handle).merge(incoming).expect("merge human deltas");
		assert_eq!(merged["merged"], json!(true));

		let query = lock_peer(&agent_handle).query(None).expect("query");
		assert_eq!(query["networks"][0]["network_id"], json!(0), "the agent's network must remain visible");
		let attributes = query["attributes"].as_object().expect("attributes object");
		assert!(attributes.contains_key("doc::human"), "the human's attribute must be visible after the merge");
		let peer_users = query["peer_users"].as_object().expect("peer users object");
		assert!(peer_users.contains_key("21"), "the agent peer must be registered");
		assert!(peer_users.contains_key("22"), "the human peer must be registered after the merge");
	}

	#[test]
	fn multi_peer_merge_then_replay_still_agrees() {
		let agent_dir = temp_dir("multi-replay-agent");
		let human_dir = temp_dir("multi-replay-human");
		write_minimal_gdd(&agent_dir, "gdd");
		write_minimal_gdd(&human_dir, "gdd");

		let agent = futures::executor::block_on(open(&agent_dir, 31));
		let human = futures::executor::block_on(open(&human_dir, 32));
		let agent_handle = agent.handle();
		let human_handle = human.handle();

		lock_peer(&agent_handle).apply_delta(add_root_network()).expect("agent apply");
		lock_peer(&human_handle).apply_delta(change_document_attribute("doc::human", "value")).expect("human apply");

		let incoming: Vec<Delta> = lock_peer(&human_handle)
			.history()
			.map(|delta| ron::ser::to_string(delta).expect("serialize delta"))
			.map(|encoded| ron::de::from_str(&encoded).expect("round-trip delta"))
			.collect();
		lock_peer(&agent_handle).merge(incoming).expect("merge");

		let replay = lock_peer(&agent_handle).replay_check().expect("replay after merge");
		assert_eq!(replay["equal"], json!(true));
	}
}
