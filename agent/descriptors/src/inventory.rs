//! The `agent/inventory.json` dump (T0.6) and the T3.6 coverage report.
//!
//! `inventory_json` keeps its Phase 0 shape: keys `nodes` (generated descriptors)
//! and `agent_safe` (allowlist result per name). Deliberately **not** included:
//! `editor_commands` and `frontend_messages` (R3-A).
//!
//! `coverage_json` (T3.6) reports generated-vs-total for nodes and for allowlisted
//! message actions, and lists every non-allowlisted message with a status.

use crate::classify;
use crate::commands::{self, COMMAND_PREFIX, CommandDescriptor};
use crate::node_descriptors_with_identifiers;
use serde_json::{Map, Value, json};
use std::collections::HashSet;
use std::io::Write;
use std::path::Path;

/// Build the inventory document. Deterministic: nodes are sorted by descriptor name
/// and `agent_safe` is sorted by name.
pub fn inventory_json() -> Value {
	let nodes: Map<String, Value> = node_descriptors_with_identifiers()
		.into_iter()
		.filter_map(|(identifier, descriptor)| serde_json::to_value(descriptor).ok().map(|value| (identifier, value)))
		.collect();

	let mut names = classify::allowlisted_names();
	names.sort();
	let agent_safe: Map<String, Value> = names
		.into_iter()
		.map(|name| {
			let safe = classify::is_agent_safe(&name);
			(name, Value::Bool(safe))
		})
		.collect();

	json!({
		"nodes": nodes,
		"agent_safe": agent_safe,
	})
}

/// Write the inventory to `path`, pretty-printed with a trailing newline.
pub fn write_inventory(path: &Path) -> std::io::Result<()> {
	if let Some(parent) = path.parent()
		&& !parent.as_os_str().is_empty()
	{
		std::fs::create_dir_all(parent)?;
	}
	let json = inventory_json();
	let mut serialized = serde_json::to_string_pretty(&json).expect("inventory is serializable");
	serialized.push('\n');
	let mut file = std::fs::File::create(path)?;
	file.write_all(serialized.as_bytes())?;
	Ok(())
}

/// T3.6 coverage: nodes and allowlisted message actions, generated vs total, plus
/// every non-allowlisted message and its status.
///
/// Requires the T3.2 active-document enumeration, so it constructs the descriptors
/// process's single `Editor` (INV-11). Do not call this from `inventory_json`.
pub fn coverage_json() -> Value {
	let node_total = node_descriptors_with_identifiers().len();
	// `node_descriptors_with_identifiers` asserts one descriptor per metadata
	// entry, so counting them again would be redundant; keep the two explicit for
	// the report's generated-vs-total shape.
	let node_generated = crate::node_descriptors().len();

	let descriptors = commands::command_descriptors();
	let allowlisted: Vec<String> = {
		let mut names: Vec<String> = classify::allowlisted_names().into_iter().filter(|name| name.starts_with(COMMAND_PREFIX)).collect();
		names.sort();
		names
	};

	let generated_names: HashSet<&str> = descriptors.iter().map(|entry| entry.descriptor.name.as_str()).collect();
	let generated = allowlisted.iter().filter(|name| generated_names.contains(name.as_str())).count();

	let not_allowlisted: Vec<Value> = descriptors.iter().filter(|entry| !classify::is_agent_safe(&entry.descriptor.name)).map(denied_message).collect();

	json!({
		"nodes": {
			"generated": node_generated,
			"total": node_total,
			"coverage_percent": percent(node_generated, node_total),
		},
		"messages": {
			"generated": generated,
			"total": allowlisted.len(),
			"coverage_percent": percent(generated, allowlisted.len()),
			"allowlisted": allowlisted,
			"not_allowlisted": not_allowlisted,
		},
	})
}

/// A non-allowlisted message with its status (T3.6).
fn denied_message(entry: &CommandDescriptor) -> Value {
	json!({
		"name": entry.descriptor.name,
		"global_name": entry.global_name,
		"status": "not-allowlisted",
	})
}

/// `generated / total` as an integer percentage; `100` when nothing is expected.
fn percent(generated: usize, total: usize) -> u64 {
	generated.saturating_mul(100).checked_div(total).unwrap_or(100) as u64
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn inventory_has_nodes_and_agent_safe() {
		let inventory = inventory_json();
		let nodes = inventory.get("nodes").and_then(Value::as_object).expect("nodes object");
		let agent_safe = inventory.get("agent_safe").and_then(Value::as_object).expect("agent_safe object");
		assert!(!nodes.is_empty());
		assert!(!agent_safe.is_empty());
		assert!(agent_safe.values().all(|value| value.as_bool() == Some(true)));
		assert!(inventory.get("editor_commands").is_none());
		assert!(inventory.get("frontend_messages").is_none());
	}

	#[test]
	fn coverage_percent_is_100_when_everything_is_generated() {
		assert_eq!(percent(7, 7), 100);
		assert_eq!(percent(0, 0), 100);
		assert_eq!(percent(1, 2), 50);
	}
}
