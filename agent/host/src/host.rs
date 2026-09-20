//! The session host (T2.2): the mode-agnostic `ToolHost` implementation.
//!
//! One `Host` owns one [`EditorBridge`] (mode A/B/C) and the curated tool modules.
//! It is the single `QueryId` allocator (INV-14) and the only place capabilities are
//! checked against the session grant set (INV-6).

use crate::modules::document::DocumentModule;
use crate::modules::graph::GraphModule;
use crate::modules::history::HistoryModule;
use crate::modules::node_catalog::NodeCatalogModule;
use crate::modules::recipes::RecipesModule;
use crate::modules::registry::RegistryModule;
use crate::modules::render::RenderModule;
use crate::modules::session::SessionModule;
use crate::paths::PathRoot;
use crate::peer::PeerBridge;
use futures::Stream;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use graphite_agent_protocol::{AgentDocumentId, AgentEvent, CapabilitySet, EditorBridge, PendingCall, QueryId, ToolCall, ToolDescriptor, ToolError, ToolHost, ToolModule, ToolOutcome, ToolRequest};
use std::any::Any;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::Notify;

/// Host-local extension of the frozen [`EditorBridge`] contract.
///
/// Phase 5's peer bridge carries a shared, typed [`PeerState`](crate::peer::PeerState) handle
/// that the registry tool module must receive at host construction. The protocol contract
/// (§5.1) is frozen, so this host-local trait provides a type-erased accessor instead of
/// extending it: the blanket impl covers every `EditorBridge`, and [`Host::new`] downcasts the
/// concrete bridge to recover a peer handle when one exists.
pub trait BridgeExt: EditorBridge {
	/// The concrete bridge as `&mut dyn Any`, for host-side downcasting.
	fn as_any_mut(&mut self) -> &mut dyn Any;
	/// Reborrow as the frozen trait object the modules execute against.
	fn as_editor_bridge(&mut self) -> &mut dyn EditorBridge;
}

impl<T: EditorBridge + 'static> BridgeExt for T {
	fn as_any_mut(&mut self) -> &mut dyn Any {
		self
	}

	fn as_editor_bridge(&mut self) -> &mut dyn EditorBridge {
		self
	}
}

/// The host's private, mutex-protected state. Destructured before a module's
/// `execute` so its `'a` borrows can share the lock guard (R2-G).
struct HostInner {
	bridge: Box<dyn BridgeExt>,
	modules: Vec<Box<dyn ToolModule>>,
}

pub struct Host {
	inner: Arc<tokio::sync::Mutex<HostInner>>,
	capabilities: CapabilitySet,
	timeout: Duration,
	/// Cached so `descriptors()` never locks the async mutex synchronously.
	descriptors: Arc<Vec<ToolDescriptor>>,
	/// Descriptor name -> index into `HostInner::modules`.
	module_index: HashMap<String, usize>,
	/// Shared with every in-flight call's poll loop (HIGH-A4).
	cancelled: Arc<Mutex<HashSet<QueryId>>>,
	/// Per-call cancellation signal, so `cancel` can wake exactly one in-flight call.
	notifies: Arc<Mutex<HashMap<QueryId, Arc<Notify>>>>,
	events_sender: UnboundedSender<AgentEvent>,
	events_receiver: Mutex<Option<UnboundedReceiver<AgentEvent>>>,
	/// Per-document `DocumentChanged` coalescing (T4.6), applied to bridge events.
	debouncer: Mutex<DocumentChangeDebouncer>,
	next_id: AtomicU64,
}

/// Coalesces raw [`AgentEvent::DocumentChanged`] events to at most one per
/// `window` per document (T4.6).
///
/// The window is a property of this value, so the behaviour is testable without the
/// desktop: a burst of raw events for one document collapses to a single forwarded
/// event, and a later event after the window elapses is forwarded again. Other
/// event kinds pass through untouched.
pub struct DocumentChangeDebouncer {
	window: Duration,
	last: HashMap<AgentDocumentId, Instant>,
}

impl Default for DocumentChangeDebouncer {
	fn default() -> Self {
		Self::with_window(Duration::from_millis(100))
	}
}

impl DocumentChangeDebouncer {
	/// The production window: ≤ 1 forwarded `DocumentChanged` per 100 ms per document.
	pub fn new() -> Self {
		Self::default()
	}

	pub fn with_window(window: Duration) -> Self {
		Self { window, last: HashMap::new() }
	}

	/// Whether a `DocumentChanged` for `document` should be forwarded now.
	pub fn accept(&mut self, document: AgentDocumentId) -> bool {
		let now = Instant::now();
		match self.last.get(&document) {
			Some(previous) if now.duration_since(*previous) < self.window => false,
			_ => {
				self.last.insert(document, now);
				true
			}
		}
	}

	/// Filter a drained batch, coalescing `DocumentChanged` and passing everything
	/// else through.
	pub fn filter(&mut self, events: Vec<AgentEvent>) -> Vec<AgentEvent> {
		events
			.into_iter()
			.filter(|event| match event {
				AgentEvent::DocumentChanged { document } => self.accept(*document),
				// Forward-compatible: additive `AgentEvent` variants are forwarded.
				_ => true,
			})
			.collect()
	}
}

impl Host {
	/// Build a host over one bridge.
	///
	/// `root` is the path-confinement root required by INV-12 (execution finding
	/// E-6): file-path tools are resolved and prefix-checked against it. It is not
	/// part of the bridge because confinement is a host policy, not a transport
	/// concern — every mode (headless/attached/peer) shares the same rule.
	pub fn new(bridge: Box<dyn BridgeExt>, capabilities: CapabilitySet, timeout: Duration, root: PathBuf) -> Host {
		let paths = Arc::new(PathRoot::new(&root));
		let mut bridge = bridge;
		// Surface C: recover the shared peer handle from the concrete bridge, if it is one.
		// Every other mode yields `None` and the registry tools refuse with InvalidArguments.
		let peer = bridge.as_any_mut().downcast_mut::<PeerBridge>().map(|bridge| bridge.handle());
		let modules: Vec<Box<dyn ToolModule>> = vec![
			Box::new(DocumentModule::new(Arc::clone(&paths))),
			Box::new(NodeCatalogModule),
			Box::new(GraphModule),
			Box::new(HistoryModule),
			Box::new(RenderModule::new(paths)),
			Box::new(SessionModule),
			Box::new(RecipesModule::new()),
			Box::new(RegistryModule::new(peer)),
		];

		let mut module_index = HashMap::new();
		let mut descriptors = Vec::new();
		for (index, module) in modules.iter().enumerate() {
			for descriptor in module.descriptors() {
				module_index.insert(descriptor.name.clone(), index);
				descriptors.push(crate::modules::annotate(descriptor));
			}
		}

		let (events_sender, events_receiver) = unbounded();
		Host {
			inner: Arc::new(tokio::sync::Mutex::new(HostInner { bridge, modules })),
			capabilities,
			timeout,
			descriptors: Arc::new(descriptors),
			module_index,
			cancelled: Arc::new(Mutex::new(HashSet::new())),
			notifies: Arc::new(Mutex::new(HashMap::new())),
			events_sender,
			events_receiver: Mutex::new(Some(events_receiver)),
			debouncer: Mutex::new(DocumentChangeDebouncer::new()),
			next_id: AtomicU64::new(1),
		}
	}

	/// Publish one [`AgentEvent`] to the events stream.
	pub fn emit(&self, event: AgentEvent) -> bool {
		self.events_sender.unbounded_send(event).is_ok()
	}

	/// Drain any events the mode bridge has buffered, debounce them (T4.6), and
	/// publish them on the host's event stream.
	///
	/// The frozen [`ToolHost`] trait is intentionally not extended; the MCP adapters
	/// call this on an idle tick so unsolicited editor changes (for example a human
	/// edit in attached mode) become `notifications/message`.
	pub async fn drain_bridge_events(&self) -> usize {
		let events = {
			let mut guard = self.inner.lock().await;
			guard.bridge.as_editor_bridge().drain_events()
		};
		let events = lock(&self.debouncer).filter(events);
		let count = events.len();
		for event in events {
			let _ = self.emit(event);
		}
		count
	}

	fn immediate(id: QueryId, error: ToolError) -> PendingCall {
		PendingCall {
			id,
			outcome: Box::pin(async move { ToolOutcome::Err { id, error } }),
		}
	}
}

impl ToolHost for Host {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		self.descriptors.as_ref().clone()
	}

	fn call(&self, request: ToolRequest) -> PendingCall {
		let id = self.next_id.fetch_add(1, Ordering::Relaxed);

		let Some(descriptor) = self.descriptors.iter().find(|descriptor| descriptor.name == request.name) else {
			return Self::immediate(
				id,
				ToolError::NotFound {
					what: format!("tool {}", request.name),
				},
			);
		};
		let capability = descriptor.capability;

		// INV-6: capability is host-assigned and checked before execution.
		if let Err(error) = crate::capability::authorize(&self.capabilities, capability) {
			return Self::immediate(id, error);
		}

		let Some(index) = self.module_index.get(&request.name).copied() else {
			return Self::immediate(
				id,
				ToolError::NotFound {
					what: format!("tool {}", request.name),
				},
			);
		};

		let inner = Arc::clone(&self.inner);
		let cancelled = Arc::clone(&self.cancelled);
		let notifies = Arc::clone(&self.notifies);
		let notify = Arc::new(Notify::new());
		lock(&self.notifies).insert(id, Arc::clone(&notify));
		lock(&self.cancelled).remove(&id);
		let timeout = self.timeout;

		let tool_call = ToolCall {
			id,
			name: request.name,
			arguments: request.arguments,
			capability,
			document: request.document,
		};

		let outcome = async move {
			// A cancel issued before this future first polled must not be missed.
			if lock(&cancelled).contains(&id) {
				lock(&notifies).remove(&id);
				return ToolOutcome::Err {
					id,
					error: ToolError::Cancelled { id },
				};
			}

			let mut guard = inner.lock().await;
			let HostInner { bridge, modules } = &mut *guard;
			let module = &mut modules[index];

			// Race the module future against cancellation and the host-owned deadline.
			// No branch is `+ Send`: this all runs on one current-thread runtime (§5.5).
			let result = tokio::select! {
				result = module.execute(tool_call, bridge.as_editor_bridge()) => result,
				() = notify.notified() => Err(ToolError::Cancelled { id }),
				() = tokio::time::sleep(timeout) => Err(ToolError::Timeout { id }),
			};

			// The module future is dropped, so the mutable borrow of the bridge ended.
			if matches!(result, Err(ToolError::Cancelled { .. }) | Err(ToolError::Timeout { .. })) {
				bridge.cancel(id);
			}
			drop(guard);

			lock(&notifies).remove(&id);
			lock(&cancelled).remove(&id);

			match result {
				Ok(result) => ToolOutcome::Ok { id, result },
				Err(error) => ToolOutcome::Err { id, error },
			}
		};

		PendingCall { id, outcome: Box::pin(outcome) }
	}

	fn cancel(&self, id: QueryId) -> bool {
		let in_flight = lock(&self.notifies).get(&id).map(Arc::clone);
		lock(&self.cancelled).insert(id);
		match in_flight {
			Some(notify) => {
				notify.notify_one();
				true
			}
			None => false,
		}
	}

	fn events(&self) -> Pin<Box<dyn Stream<Item = AgentEvent> + Send>> {
		let receiver = lock(&self.events_receiver).take();
		match receiver {
			Some(receiver) => Box::pin(receiver),
			None => Box::pin(futures::stream::empty()),
		}
	}
}

/// Lock a `std::sync::Mutex`, treating poisoning as recoverable: these mutexes only
/// guard plain data and are never held across an `.await`.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
	mutex.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
	use super::*;
	use graphite_agent_protocol::{AgentEvent, BridgeQuery, Capability};
	use serde_json::json;
	use std::future::Future;

	struct NullBridge;

	impl EditorBridge for NullBridge {
		fn submit(&mut self, _id: QueryId, _query: BridgeQuery) -> Result<(), ToolError> {
			Ok(())
		}
		fn poll(&mut self, _id: QueryId) -> Option<Result<serde_json::Value, ToolError>> {
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

	fn host(capabilities: CapabilitySet) -> Host {
		Host::new(Box::new(NullBridge), capabilities, Duration::from_secs(30), std::env::temp_dir().join("graphite-agent-host-test"))
	}

	/// E-20: Claude Code flattens a root-level `anyOf`/`oneOf`/`allOf` before sending
	/// a tool to the API, which degrades the constraint into a description hint. The
	/// curated surface must therefore stay a plain object schema.
	#[test]
	fn curated_schemas_have_no_root_combinator() {
		let host = host(CapabilitySet::default());
		for descriptor in host.descriptors() {
			for (label, schema) in [("input", &descriptor.input_schema), ("output", &descriptor.output_schema)] {
				assert_eq!(
					schema.get("type").and_then(serde_json::Value::as_str),
					Some("object"),
					"{} {label} schema is not an object",
					descriptor.name
				);
				for combinator in ["anyOf", "oneOf", "allOf"] {
					assert!(schema.get(combinator).is_none(), "{} {label} schema has a root-level `{combinator}`", descriptor.name);
				}
			}
		}
	}

	/// E-18: the size annotation reaches `tools/list` only through `annotate`, so the
	/// table and the descriptors must agree exactly.
	#[test]
	fn annotations_cover_exactly_the_large_output_tools() {
		let host = host(CapabilitySet::default());
		let mut annotated: Vec<String> = host
			.descriptors()
			.into_iter()
			.filter(|descriptor| descriptor.meta.is_some())
			.map(|descriptor| descriptor.name)
			.collect();
		let mut expected = crate::modules::annotated_tool_names();
		annotated.sort();
		expected.sort();
		assert_eq!(annotated, expected, "the annotation table and the curated descriptors disagree");
	}

	#[test]
	fn descriptors_are_cached_and_include_the_full_curated_surface() {
		let host = host(CapabilitySet::default());
		let names: HashSet<String> = host.descriptors().into_iter().map(|descriptor| descriptor.name).collect();
		assert_eq!(names.len(), 33, "the curated tool surface has 21 §17 tools plus 3 Phase 4 session tools plus 4 Phase 5 registry tools plus 2 GIF tools plus 3 recipes tools");
		for expected in [
			"document.new",
			"document.open",
			"document.save",
			"document.close",
			"document.list",
			"node.list_types",
			"node.describe",
			"graph.add_node",
			"graph.remove_node",
			"graph.set_input",
			"graph.connect",
			"graph.disconnect",
			"graph.list_nodes",
			"graph.get_node",
			"history.undo",
			"history.redo",
			"history.begin",
			"history.commit",
			"history.abort",
			"render.preview",
			"render.preview_gif",
			"render.export",
			"render.export_gif",
			"session.snapshot",
			"session.selection",
			"session.active_document",
			"registry.apply_delta",
			"registry.query",
			"registry.merge",
			"history.replay",
			"recipes.list",
			"recipes.show",
			"recipes.lint",
		] {
			assert!(names.contains(expected), "missing curated tool {expected}");
		}
	}

	#[test]
	fn document_change_debouncer_coalesces_per_document() {
		let mut debouncer = DocumentChangeDebouncer::with_window(Duration::from_millis(100));
		let batch = vec![
			AgentEvent::DocumentChanged { document: 1 },
			AgentEvent::DocumentChanged { document: 1 },
			AgentEvent::DocumentChanged { document: 2 },
			AgentEvent::DocumentChanged { document: 1 },
		];
		assert_eq!(debouncer.filter(batch).len(), 2, "one event per document per window");

		std::thread::sleep(Duration::from_millis(120));
		let later = debouncer.filter(vec![AgentEvent::DocumentChanged { document: 1 }]);
		assert_eq!(later.len(), 1, "a later change after the window is forwarded again");
	}

	#[test]
	fn document_change_debouncer_passes_other_events_through() {
		let mut debouncer = DocumentChangeDebouncer::with_window(Duration::from_secs(60));
		let filtered = debouncer.filter(vec![
			AgentEvent::Progress {
				id: 1,
				fraction: 0.5,
				message: "working".to_string(),
			},
			AgentEvent::DocumentChanged { document: 1 },
			AgentEvent::DocumentChanged { document: 1 },
		]);
		assert_eq!(filtered.len(), 2);
		assert!(matches!(filtered[0], AgentEvent::Progress { .. }));
		assert!(matches!(filtered[1], AgentEvent::DocumentChanged { document: 1 }));
	}

	#[test]
	fn unknown_tool_is_not_found() {
		let host = host(CapabilitySet(vec![Capability::Read]));
		let pending = host.call(ToolRequest {
			name: "nope".to_string(),
			arguments: json!({}),
			document: None,
		});
		let outcome = futures::executor::block_on(pending.outcome);
		assert!(matches!(
			outcome,
			ToolOutcome::Err {
				error: ToolError::NotFound { .. },
				..
			}
		));
	}

	#[test]
	fn missing_capability_is_unauthorized_before_execution() {
		let host = host(CapabilitySet(vec![Capability::Read]));
		let pending = host.call(ToolRequest {
			name: "graph.add_node".to_string(),
			arguments: json!({ "document_id": 1, "identifier": "x" }),
			document: None,
		});
		let outcome = futures::executor::block_on(pending.outcome);
		assert!(matches!(
			outcome,
			ToolOutcome::Err {
				error: ToolError::Unauthorized { capability: Capability::Author },
				..
			}
		));
	}

	#[test]
	fn events_stream_is_take_once() {
		let host = host(CapabilitySet::default());
		let _first = host.events();
		// The second call must hand back the empty stream, not panic.
		let _second = host.events();
		assert!(host.emit(AgentEvent::Posted { id: 1, message: "x".to_string() }));
	}

	#[test]
	fn cancel_before_the_future_starts_is_not_missed() {
		let host = host(CapabilitySet(vec![Capability::Read, Capability::Author]));
		let pending = host.call(ToolRequest {
			name: "graph.list_nodes".to_string(),
			arguments: json!({ "document_id": 1 }),
			document: None,
		});
		assert!(host.cancel(pending.id));
		let outcome = futures::executor::block_on(pending.outcome);
		assert!(matches!(
			outcome,
			ToolOutcome::Err {
				error: ToolError::Cancelled { .. },
				..
			}
		));
	}
}
