//! Command descriptors (T3.3, T3.4, §6.3) — **catalog data, never MCP tools**.
//!
//! A command descriptor is generated from a live `collect_actions()` result
//! ([`crate::actions`]) plus the field metadata the `HierarchicalTree` derive
//! already emits on [`Message::message_tree`]. It is produced exactly like a
//! `node.type.*` entry and, per INV-8/E-8, it never enters `tools/list`, the
//! host's `module_index`, or any `ToolModule::descriptors()`. Executing a message
//! action by name remains forbidden (INV-13). Its only read paths are the
//! `coverage` subcommand and the `graphite://command-catalog` MCP resource.
//!
//! ## Naming (the single documented rule)
//!
//! An action's allowlist entry *is* its generated command descriptor name:
//! `command.` + the action's `AsMessage::global_name()` lowercased, with every
//! character outside `[a-z0-9.]` replaced by `_` (a no-op for the dotted
//! `global_name`, but applied so the rule is total). `classify::is_agent_safe`
//! stays default-deny membership against `agent/allowlist.toml`.
//!
//! ## T3.4 finding — the macro is sufficient
//!
//! The existing derive needs no extension. `HierarchicalTree` already emits
//! `"name: RustType"` strings for named variants, reachable through the public
//! `Message::message_tree()`; `#[message_handler_data]` / `ExtractField` cover
//! handler/context fields the same way. Two caveats are handled here rather than
//! by changing `proc-macros`:
//!
//! 1. The metadata yields Rust **source-type strings**, so this module owns a
//!    small string→JSON-Schema mapper (`source_type_schema`); the
//!    `type_to_schema` in `lib.rs` maps `graphene_std::Type`, not source strings.
//! 2. `HierarchicalTree` only recurses into payload type names ending in
//!    `Message`; nested non-message payload structs stay opaque and become
//!    `x-rust-type`, not a structural schema.

use crate::actions;
use crate::version::{CATALOG_VERSION, VERSION};
use graphite_agent_protocol::{Capability, ToolDescriptor};
use graphite_editor::messages::prelude::{AsMessage, Message};
use graphite_editor::utility_traits::ActionList;
use graphite_editor::utility_types::DebugMessageTree;
use serde_json::{Map, Value, json};

/// The prefix that distinguishes a generated command name from a curated tool name.
pub const COMMAND_PREFIX: &str = "command.";

const SCHEMA: &str = "https://json-schema.org/draft/2020-12/schema";

/// One generated command catalog entry: the action's dotted `global_name` and its
/// payload-free catalog descriptor.
#[derive(Clone, Debug, PartialEq)]
pub struct CommandDescriptor {
	pub global_name: String,
	pub descriptor: ToolDescriptor,
}

/// §6.3 naming: `Portfolio.Document.Undo` → `command.portfolio.document.undo`.
pub fn command_name(global_name: &str) -> String {
	let mut name = String::with_capacity(global_name.len() + COMMAND_PREFIX.len());
	name.push_str(COMMAND_PREFIX);
	for character in global_name.replace("::", ".").to_lowercase().chars() {
		if character.is_ascii_lowercase() || character.is_ascii_digit() || character == '.' {
			name.push(character);
		} else {
			name.push('_');
		}
	}
	name
}

/// Generate one descriptor per action in `actions` (groups are flattened first).
/// Sorted by command name so the catalog is deterministic (R8-M3).
pub fn command_descriptors_from_actions(actions: &ActionList) -> Vec<CommandDescriptor> {
	let tree = Message::message_tree();
	let mut descriptors: Vec<CommandDescriptor> = actions
		.iter()
		.flatten()
		.map(|action| {
			let global_name = action.global_name();
			CommandDescriptor {
				global_name: global_name.clone(),
				descriptor: command_descriptor(&tree, &global_name),
			}
		})
		.collect();
	descriptors.sort_by(|a, b| a.descriptor.name.cmp(&b.descriptor.name));
	descriptors.dedup_by(|a, b| a.descriptor.name == b.descriptor.name);
	descriptors
}

/// The canonical descriptors for this process, from the T3.2 active-document
/// enumeration.
pub fn command_descriptors() -> Vec<CommandDescriptor> {
	command_descriptors_from_actions(&actions::enumerate_actions())
}

/// The canonical command catalog as JSON, keyed by `global_name` so the dotted
/// action path (not just its normalized form) survives.
///
/// This constructs the descriptors process's own `Editor` (INV-11). The host,
/// which already owns the process's editor, uses
/// [`command_catalog_json_from_actions`] instead.
pub fn command_catalog_json() -> Value {
	command_catalog_json_from_actions(&actions::enumerate_actions())
}

/// As [`command_catalog_json`], but from an already-collected action list, so a
/// caller that owns the process's live editor (the host, which must not construct
/// a second one — INV-11) can generate the same catalog shape.
pub fn command_catalog_json_from_actions(actions: &ActionList) -> Value {
	let entries = command_descriptors_from_actions(actions);
	let commands: Map<String, Value> = entries
		.iter()
		.filter_map(|entry| serde_json::to_value(&entry.descriptor).ok().map(|value| (entry.global_name.clone(), value)))
		.collect();
	json!({
		"schema_version": 1,
		"catalog_version": CATALOG_VERSION,
		"count": commands.len(),
		"commands": commands,
		"deprecated": [],
	})
}

/// Build the descriptor for one action. The `input_schema` is never empty: a
/// variant with no fields still gets a well-formed object schema.
fn command_descriptor(tree: &DebugMessageTree, global_name: &str) -> ToolDescriptor {
	let mut properties = Map::new();
	for field in fields_for_global_name(tree, global_name) {
		if let Some((name, rust_type)) = parse_field(&field) {
			properties.insert(name.to_string(), source_type_schema(rust_type));
		}
	}

	let input_schema = json!({
		"$schema": SCHEMA,
		"title": command_name(global_name),
		"type": "object",
		"additionalProperties": false,
		"properties": Value::Object(properties),
		"x-command-path": global_name,
	});

	ToolDescriptor {
		name: command_name(global_name),
		description: format!("Command catalog entry for `{global_name}` (catalog data; not an MCP tool)."),
		// Generated catalog entries are read-only catalog data, never tools (§6, INV-8).
		capability: Capability::Read,
		input_schema,
		output_schema: json!({
			"$schema": SCHEMA,
			"type": "object",
			"additionalProperties": true,
			"x-command-path": global_name,
		}),
		version: VERSION,
		// Catalog data never reaches `tools/list`, so it carries no `_meta` (E-18).
		meta: None,
	}
}

/// Locate the `DebugMessageTree` leaf for a dotted `global_name`.
///
/// The tree nests message enums as a single child named `<Enum>Message`, so the
/// walk auto-descends through those wrappers before matching the next segment.
pub fn find_variant<'a>(tree: &'a DebugMessageTree, global_name: &str) -> Option<&'a DebugMessageTree> {
	let mut node = tree;
	for segment in global_name.split('.') {
		// Auto-descend through a single `<Enum>Message` child, if present.
		while let Some(inner) = message_enum_child(node) {
			node = inner;
		}
		node = node.variants()?.iter().find(|variant| variant.name() == segment)?;
	}
	Some(node)
}

/// The single child `DebugMessageTree` that wraps a message enum, if the variant
/// has exactly one such child.
fn message_enum_child(node: &DebugMessageTree) -> Option<&DebugMessageTree> {
	match node.variants()?.as_slice() {
		[single] if single.name().ends_with("Message") => Some(single),
		_ => None,
	}
}

/// The `"name: RustType"` metadata strings for an action, or an empty list for a
/// fieldless variant.
pub fn fields_for_global_name(tree: &DebugMessageTree, global_name: &str) -> Vec<String> {
	find_variant(tree, global_name).and_then(|variant| variant.fields()).cloned().unwrap_or_default()
}

/// Split a `HierarchicalTree` field string (`"frame: f64"`). Returns `None` when
/// the metadata is not in the expected shape.
pub fn parse_field(field: &str) -> Option<(&str, &str)> {
	let (name, rust_type) = field.split_once(':')?;
	let name = name.trim();
	let rust_type = rust_type.trim();
	if name.is_empty() || rust_type.is_empty() { None } else { Some((name, rust_type)) }
}

/// Map a Rust **source-type string** from the derive metadata to a JSON Schema
/// fragment (§6.2 analogue; caveat 1). Unknown types stay opaque via
/// `x-rust-type` rather than being guessed at (caveat 2).
pub fn source_type_schema(rust_type: &str) -> Value {
	let trimmed = rust_type.trim();
	if let Some(inner) = trimmed.strip_prefix("Option<").and_then(|inner| inner.strip_suffix('>')) {
		let mut schema = source_type_schema(inner);
		if let Value::Object(object) = &mut schema {
			object.insert("x-optional".to_string(), Value::Bool(true));
		}
		return schema;
	}
	if let Some(inner) = trimmed.strip_prefix("Vec<").and_then(|inner| inner.strip_suffix('>')) {
		return json!({ "type": "array", "items": source_type_schema(inner) });
	}

	match trimmed {
		"f32" | "f64" => json!({ "type": "number" }),
		"i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => json!({ "type": "integer" }),
		"bool" => json!({ "type": "boolean" }),
		"String" | "&str" | "str" | "PathBuf" => json!({ "type": "string" }),
		_ => json!({ "x-rust-type": trimmed }),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn naming_matches_spec() {
		assert_eq!(command_name("Portfolio.Document.Undo"), "command.portfolio.document.undo");
		assert_eq!(command_name("DocumentMessage::NodeGraphMessage::DeleteNodes"), "command.documentmessage.nodegraphmessage.deletenodes");
		assert_eq!(command_name("Tool.Select.PointerMove"), "command.tool.select.pointermove");
	}

	#[test]
	fn every_generated_command_name_is_prefixed_and_normalized() {
		for global_name in ["appwindow.close", "App-Window.CLOSE!!", "Portfolio.Document.Undo"] {
			let name = command_name(global_name);
			assert!(name.starts_with(COMMAND_PREFIX), "{name}");
			assert!(
				name.chars()
					.all(|character| character.is_ascii_lowercase() || character.is_ascii_digit() || character == '.' || character == '_'),
				"{name} escaped normalization"
			);
		}
	}

	#[test]
	fn source_type_mapping_is_total_and_non_empty() {
		assert_eq!(source_type_schema("f64"), json!({ "type": "number" }));
		assert_eq!(source_type_schema("bool"), json!({ "type": "boolean" }));
		assert_eq!(source_type_schema("u32"), json!({ "type": "integer" }));
		assert_eq!(source_type_schema("String"), json!({ "type": "string" }));
		assert_eq!(source_type_schema("DVec2"), json!({ "x-rust-type": "DVec2" }));
		assert_eq!(source_type_schema("Option<f64>"), json!({ "type": "number", "x-optional": true }));
		assert_eq!(source_type_schema("Vec<f64>"), json!({ "type": "array", "items": { "type": "number" } }));
		// A fieldless variant still passes through the generic branch.
		assert_eq!(
			source_type_schema("Option<for<'a>fn(&'a mut SnappingState)>"),
			json!({ "x-rust-type": "for<'a>fn(&'a mut SnappingState)", "x-optional": true })
		);
	}

	#[test]
	fn field_parsing_handles_the_emitted_shape() {
		assert_eq!(parse_field("frame: f64"), Some(("frame", "f64")));
		assert_eq!(parse_field("delta: DVec2"), Some(("delta", "DVec2")));
		assert_eq!(parse_field("not a field"), None);
		assert_eq!(parse_field(": f64"), None);
	}
}
