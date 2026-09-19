//! Message-action enumeration with an active document (T3.2).
//!
//! `Dispatcher::collect_actions` only advertises the document/tool actions when a
//! document is active (M-A3), so the enumeration must first open one. The
//! procedure, exactly as the Phase 3 plan prescribes:
//!
//! 1. Construct the process's one headless [`Editor`] via
//!    [`Editor::new_headless`] (INV-11: exactly one `Editor` per process).
//! 2. Dispatch [`PortfolioMessage::NewDocumentWithName`] so an active document
//!    exists.
//! 3. Call `editor.dispatcher.collect_actions()`.
//!
//! `agent/descriptors` must **never** depend on `agent/host` (HIGH-1): the
//! reverse edge would be a Cargo cycle. It already depends on `editor`,
//! `graphene-std`, and `graph-craft`, which is all this needs.
//!
//! Because `Editor::new_headless` panics on a second call in the same process,
//! the one construction is guarded by a [`OnceLock`]; every later call reuses it.

use graph_craft::application_io::resource::HashMapResourceStorage;
use graphite_editor::application::{Editor, HeadlessEditorState};
use graphite_editor::messages::portfolio::PortfolioMessage;
use graphite_editor::utility_traits::{ActionList, AsMessage};
use std::sync::{Arc, OnceLock};

/// The process's single catalog enumeration (INV-11). `MessageDiscriminant` is a
/// payload-free `Copy` name, so caching the list is cheap and thread-safe.
static ACTION_LIST: OnceLock<ActionList> = OnceLock::new();

/// The raw `collect_actions` result, groups intact, with the active-document
/// procedure already applied. Cached for the process.
pub fn collect_action_list() -> ActionList {
	ACTION_LIST
		.get_or_init(|| {
			let state = HeadlessEditorState {
				resource_storage: Arc::new(HashMapResourceStorage::new()),
				working_copy_root: working_copy_root(),
				// Deterministic by construction: no wall-clock or random input.
				uuid_random_seed: 0,
			};
			let (mut editor, _wake) = Editor::new_headless(state);

			// T3.2 step 2: open the first document so document/tool actions are advertised.
			let _ = editor.handle_message(PortfolioMessage::NewDocumentWithName { name: "Agent Catalog".to_string() });

			editor.dispatcher.collect_actions()
		})
		.clone()
}

/// The flattened, de-duplicated, name-sorted set of every advertised action.
///
/// `collect_actions` groups actions into menu-like sections; the catalog only
/// cares about the individual actions, so the groups are flattened. Discriminants
/// are payload-free names, so ordering is done on the generated command name.
pub fn enumerate_actions() -> ActionList {
	let mut flattened: Vec<_> = collect_action_list().into_iter().flatten().collect();
	flattened.sort_by_key(|action| action.global_name().to_lowercase());
	flattened.dedup();
	vec![flattened]
}

/// Every action's `global_name`, e.g. `Portfolio.Document.Undo`.
pub fn action_global_names() -> Vec<String> {
	enumerate_actions().into_iter().flatten().map(|action| action.global_name()).collect()
}

/// A stable scratch working-copy directory for the throwaway catalog editor.
fn working_copy_root() -> std::path::PathBuf {
	let root = std::env::temp_dir().join("graphite-agent-descriptors-catalog");
	let _ = std::fs::create_dir_all(&root);
	root
}
