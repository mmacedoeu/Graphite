//! Command-catalog read path (T3.6, §11).
//!
//! The generated `command.*` descriptors are **catalog data, never MCP tools**
//! (INV-8, E-8). Their only read path besides the `coverage` subcommand is the
//! read-only `graphite://command-catalog` MCP resource, served from here.
//!
//! INV-11 makes command-catalog generation subtle in the host process: the
//! descriptors crate's canonical enumeration constructs an `Editor`, and the host
//! already owns the process's one `Editor`. So the live catalog is generated from
//! the **host's own** editor (no second construction) and installed here by
//! [`crate::HeadlessBridge`]. If nothing was installed, this returns a valid empty
//! catalog rather than panicking.

use graphite_agent_descriptors::commands;
use serde_json::Value;
use std::sync::OnceLock;

/// The catalog generated from the live host editor, installed once per process.
static INSTALLED: OnceLock<Value> = OnceLock::new();

/// Install the catalog produced from the host's own editor (T3.6). The first
/// installation wins; later calls are ignored.
pub fn install_command_catalog(catalog: Value) {
	let _ = INSTALLED.set(catalog);
}

/// The generated command catalog, for `graphite://command-catalog` (§11).
pub fn command_catalog() -> Value {
	match INSTALLED.get() {
		Some(catalog) => catalog.clone(),
		// No editor-owned catalog was installed: return a valid empty catalog
		// instead of constructing a second `Editor` (INV-11).
		None => commands::command_catalog_json_from_actions(&Vec::new()),
	}
}
