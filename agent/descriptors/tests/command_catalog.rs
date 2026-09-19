//! T3.3/T3.4/T3.6 integration tests for the generated command catalog.
//!
//! These construct the descriptors process's single headless `Editor` (INV-11)
//! through `graphite_agent_descriptors::actions`, which caches it with a
//! `OnceLock`, so any number of tests in this binary is safe.

use graphite_agent_descriptors::classify;
use graphite_agent_descriptors::commands::{self, COMMAND_PREFIX};
use graphite_agent_descriptors::inventory::coverage_json;
use serde_json::Value;

fn descriptors() -> Vec<commands::CommandDescriptor> {
	commands::command_descriptors()
}

#[test]
fn every_action_yields_a_validator_passing_descriptor_with_a_non_empty_schema() {
	let descriptors = descriptors();
	assert!(!descriptors.is_empty(), "the active-document enumeration must produce actions");

	for entry in &descriptors {
		assert!(entry.descriptor.name.starts_with(COMMAND_PREFIX), "{} is not a command name", entry.descriptor.name);
		let schema = &entry.descriptor.input_schema;
		let object = schema.as_object().expect("input_schema is an object");
		assert!(!object.is_empty(), "{} has an empty input_schema", entry.descriptor.name);
		assert_eq!(object.get("type").and_then(Value::as_str), Some("object"), "{}", entry.descriptor.name);
		assert!(object.contains_key("properties"), "{} must carry a properties map", entry.descriptor.name);
		assert_eq!(entry.descriptor.version, 1, "descriptors start at version 1 (§5.6)");
		jsonschema::validator_for(schema).unwrap_or_else(|error| panic!("{} produced an invalid schema: {error}", entry.descriptor.name));
	}
}

/// T3.4: the existing `HierarchicalTree` derive is sufficient — a known message's
/// generated command `input_schema` has exactly the expected properties.
#[test]
fn known_message_fields_become_the_expected_properties() {
	let descriptors = descriptors();
	let by_name = |name: &str| descriptors.iter().find(|entry| entry.descriptor.name == name).unwrap_or_else(|| panic!("missing {name}"));

	// `AnimationMessage::SetFrameIndex { frame: f64 }`.
	let frame_index = by_name("command.animation.setframeindex");
	assert_eq!(frame_index.global_name, "Animation.SetFrameIndex");
	let properties = frame_index.descriptor.input_schema.get("properties").and_then(Value::as_object).expect("properties");
	assert_eq!(properties.len(), 1, "SetFrameIndex has exactly one field");
	assert_eq!(properties.get("frame").and_then(|schema| schema.get("type")).and_then(Value::as_str), Some("number"));

	// `SelectMessage::DragStart { extend_selection: Key, ... }` — five fields whose
	// Rust source type `Key` is opaque (caveat 2: non-`Message` payload types).
	let drag_start = by_name("command.tool.select.dragstart");
	let properties = drag_start.descriptor.input_schema.get("properties").and_then(Value::as_object).expect("properties");
	assert_eq!(properties.len(), 5);
	assert_eq!(properties.get("extend_selection").and_then(|schema| schema.get("x-rust-type")).and_then(Value::as_str), Some("Key"));

	// `CanvasTiltSet { angle_radians: f64 }`.
	let tilt = by_name("command.portfolio.document.navigation.canvastiltset");
	assert_eq!(
		tilt.descriptor
			.input_schema
			.get("properties")
			.and_then(|value| value.get("angle_radians"))
			.and_then(|schema| schema.get("type"))
			.and_then(Value::as_str),
		Some("number")
	);

	// A fieldless variant still receives a well-formed, non-empty object schema.
	let noop = by_name("command.portfolio.document.noop");
	assert_eq!(noop.descriptor.input_schema.get("properties").and_then(Value::as_object).map(|properties| properties.len()), Some(0));
	assert!(noop.descriptor.input_schema.as_object().is_some_and(|object| !object.is_empty()));
}

/// T3.2 proof: document/tool actions are only advertised with an active document,
/// so their presence proves the enumeration opened one (M-A3).
#[test]
fn active_document_actions_are_present() {
	let entries = descriptors();
	let names: Vec<&str> = entries.iter().map(|entry| entry.descriptor.name.as_str()).collect();
	assert!(
		names.iter().any(|name| name.starts_with("command.tool.activate")),
		"tool actions are only advertised with an active document"
	);
	assert!(
		names.iter().any(|name| name.starts_with("command.portfolio.document.")),
		"document actions are only advertised with an active document"
	);
}

#[test]
fn command_descriptors_are_never_curated_tools() {
	// The §17 curated names are tools; a command descriptor must not collide with
	// one, and must never be promoted to a tool (INV-8, E-8).
	let curated = [
		"document.new",
		"document.open",
		"document.save",
		"document.close",
		"document.list",
		"graph.add_node",
		"render.preview",
		"render.export",
	];
	for entry in descriptors() {
		assert!(!curated.contains(&entry.descriptor.name.as_str()), "{} collides with a curated tool", entry.descriptor.name);
	}
}

/// T3.6: coverage is 100% for nodes and for allowlisted messages, and every other
/// message is listed with a status.
#[test]
fn coverage_is_complete_and_lists_every_other_message() {
	let coverage = coverage_json();
	assert_eq!(coverage.get("nodes").and_then(|nodes| nodes.get("coverage_percent")).and_then(Value::as_u64), Some(100));
	let nodes = coverage.get("nodes").expect("nodes");
	assert!(nodes.get("total").and_then(Value::as_u64).unwrap_or(0) > 0);

	let messages = coverage.get("messages").expect("messages");
	assert_eq!(messages.get("coverage_percent").and_then(Value::as_u64), Some(100));
	assert!(messages.get("total").and_then(Value::as_u64).unwrap_or(0) > 0);

	let allowlisted = messages.get("allowlisted").and_then(Value::as_array).expect("allowlisted");
	assert!(allowlisted.iter().all(|name| name.as_str().is_some_and(classify::is_agent_safe)));

	// Gate 3: every allowlisted message action yields a descriptor with a
	// non-empty `input_schema` (including document/tool actions, which exist only
	// because T3.2 opened a document).
	let entries = descriptors();
	for name in allowlisted {
		let name = name.as_str().expect("allowlisted name");
		let entry = entries
			.iter()
			.find(|entry| entry.descriptor.name == name)
			.unwrap_or_else(|| panic!("allowlisted action `{name}` has no generated descriptor"));
		assert!(
			entry.descriptor.input_schema.as_object().is_some_and(|object| !object.is_empty()),
			"`{name}` must have a non-empty input_schema"
		);
	}

	let denied = messages.get("not_allowlisted").and_then(Value::as_array).expect("not_allowlisted");
	assert!(!denied.is_empty(), "some messages are expected to be denied");
	for entry in denied {
		assert_eq!(entry.get("status").and_then(Value::as_str), Some("not-allowlisted"));
		assert!(!classify::is_agent_safe(entry.get("name").and_then(Value::as_str).expect("name")));
	}

	// Coverage is exactly the set of collected, non-allowlisted actions: none missing.
	let generated = entries.len();
	let denied_count = denied.len();
	let allowlisted_generated = allowlisted
		.iter()
		.filter(|name| name.as_str().is_some_and(|name| entries.iter().any(|entry| entry.descriptor.name == name)))
		.count();
	assert_eq!(allowlisted_generated + denied_count, generated, "every collected action is either allowlisted or listed as denied");
}
