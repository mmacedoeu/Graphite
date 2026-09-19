//! Generates the agent tool descriptor catalog from Graphene node metadata.
//!
//! Two kinds of catalog entry exist (§6, R2-F):
//! - **Generated** `node.type.*` entries: catalog data surfaced by `node.describe`
//!   and the `graphite://node-catalog` resource. They are NOT MCP tools.
//! - **Generated** `command.*` entries ([`commands`]): catalog data generated from
//!   the live message-action enumeration ([`actions`]), surfaced only by the
//!   `coverage` subcommand and the `graphite://command-catalog` resource. They are
//!   NOT MCP tools (INV-8, E-8).
//! - **Curated** operation entries: the MCP tools, declared in the host modules.
//!
//! Never hand-write a node descriptor that can be generated here (INV-8).

pub mod actions;
pub mod classify;
pub mod commands;
pub mod inventory;
pub mod version;

use graphene_std::Type;
use graphene_std::registry::{FieldMetadata, NODE_METADATA, NodeMetadata, RegistryWidgetOverride};
use graphite_agent_protocol::{Capability, ToolDescriptor};
use serde_json::{Map, Value, json};

/// One descriptor per `NODE_METADATA` entry, keyed by its raw `ProtoNodeIdentifier`
/// string. Sorted by descriptor name so output is deterministic (M-B6 / R8-M3).
pub fn node_descriptors_with_identifiers() -> Vec<(String, ToolDescriptor)> {
	let metadata = NODE_METADATA.lock().expect("NODE_METADATA mutex is poisoned");
	let mut entries: Vec<(String, ToolDescriptor)> = metadata
		.iter()
		.map(|(identifier, metadata)| (identifier.as_str().to_string(), node_descriptor(identifier.as_str(), metadata)))
		.collect();
	entries.sort_by(|a, b| a.1.name.cmp(&b.1.name));
	assert!(!entries.is_empty(), "NODE_METADATA is empty: the node crates are not linked (see agent/README.md)");
	assert_eq!(entries.len(), metadata.len(), "one descriptor per node metadata entry");
	entries
}

/// Every generated node-type descriptor (T0.8), sorted by `name`.
pub fn node_descriptors() -> Vec<ToolDescriptor> {
	node_descriptors_with_identifiers().into_iter().map(|(_, descriptor)| descriptor).collect()
}

/// The catalog as JSON (T0.8), for the `graphite://node-catalog` resource.
pub fn node_catalog_json() -> Value {
	let nodes: Map<String, Value> = node_descriptors_with_identifiers()
		.into_iter()
		.filter_map(|(identifier, descriptor)| serde_json::to_value(descriptor).ok().map(|value| (identifier, value)))
		.collect();
	json!({
		"schema_version": 1,
		"count": nodes.len(),
		"nodes": nodes,
	})
}

/// §6.1 descriptor naming.
///
/// `graphene_core::raster::OpacityNode` -> `node.type.graphene_core.raster.opacitynode`.
pub fn descriptor_name(identifier: &str) -> String {
	let mut name = String::with_capacity(identifier.len() + "node.type.".len());
	name.push_str("node.type.");
	for character in identifier.replace("::", ".").to_lowercase().chars() {
		if character.is_ascii_lowercase() || character.is_ascii_digit() || character == '.' {
			name.push(character);
		} else {
			name.push('_');
		}
	}
	name
}

fn node_descriptor(identifier: &str, metadata: &NodeMetadata) -> ToolDescriptor {
	let mut properties = Map::new();
	for field in &metadata.fields {
		if field.hidden {
			continue;
		}
		properties.insert(field.name.to_string(), field_schema(field));
	}

	let description = if metadata.description.is_empty() { metadata.display_name } else { metadata.description };

	let input_schema = json!({
		"$schema": "https://json-schema.org/draft/2020-12/schema",
		"title": descriptor_name(identifier),
		"type": "object",
		"additionalProperties": false,
		"properties": Value::Object(properties),
		"x-node-identifier": identifier,
		"x-node-display-name": metadata.display_name,
		"x-node-category": metadata.category,
		"x-context-features": metadata.context_features.iter().map(|feature| format!("{feature:?}")).collect::<Vec<_>>(),
		"x-memoize": metadata.memoize,
		"x-inject-scope": metadata.inject_scope,
	});

	let output_schema = json!({
		"$schema": "https://json-schema.org/draft/2020-12/schema",
		"type": "object",
		"additionalProperties": true,
		"x-node-identifier": identifier,
	});

	ToolDescriptor {
		name: descriptor_name(identifier),
		description: description.to_string(),
		// Generated catalog entries are not MCP tools; they are read-only catalog data (§6).
		capability: Capability::Read,
		input_schema,
		output_schema,
		version: 1,
	}
}

/// §6.2 `FieldMetadata` -> JSON Schema.
fn field_schema(field: &FieldMetadata) -> Value {
	let mut schema = field.default_type.as_ref().map(type_to_schema).unwrap_or_else(|| json!({}));
	let Value::Object(object) = &mut schema else {
		return schema;
	};

	if !field.description.is_empty() {
		object.insert("description".to_string(), json!(field.description));
	}
	if let Some(unit) = field.unit {
		object.insert("x-unit".to_string(), json!(unit));
	}
	if let Some(minimum) = field.number_hard_min {
		object.insert("minimum".to_string(), json!(minimum));
	}
	if let Some(maximum) = field.number_hard_max {
		object.insert("maximum".to_string(), json!(maximum));
	}
	if let Some(minimum) = field.number_soft_min {
		object.insert("x-soft-min".to_string(), json!(minimum));
	}
	if let Some(maximum) = field.number_soft_max {
		object.insert("x-soft-max".to_string(), json!(maximum));
	}
	if let Some(step) = field.number_step {
		object.insert("x-step".to_string(), json!(step));
	}
	if let RegistryWidgetOverride::Custom(widget) = &field.widget_override {
		object.insert("x-widget".to_string(), json!(widget));
	}
	schema
}

fn type_to_schema(ty: &Type) -> Value {
	match ty {
		Type::Generic(_) => json!({}),
		Type::Concrete(descriptor) => concrete_type_schema(descriptor.name.as_ref()),
		Type::Fn(_, _) => json!({}),
		Type::Future(inner) => type_to_schema(inner),
		Type::Item(inner) | Type::List(inner) => json!({ "type": "array", "items": type_to_schema(inner) }),
	}
}

fn concrete_type_schema(name: &str) -> Value {
	let leaf = name.rsplit("::").next().unwrap_or(name);
	match leaf {
		"f32" | "f64" => json!({ "type": "number" }),
		"i8" | "i16" | "i32" | "i64" | "i128" | "isize" | "u8" | "u16" | "u32" | "u64" | "u128" | "usize" => json!({ "type": "integer" }),
		"bool" => json!({ "type": "boolean" }),
		"String" | "str" | "Cow" => json!({ "type": "string" }),
		"Vec" => json!({ "type": "array" }),
		// Unknown concrete types are described by their Rust path rather than guessed at.
		_ => json!({ "x-rust-type": name }),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashSet;

	#[test]
	fn descriptor_naming_matches_spec() {
		assert_eq!(descriptor_name("graphene_core::raster::OpacityNode"), "node.type.graphene_core.raster.opacitynode");
		assert_eq!(descriptor_name("graphene_core::raster::OpacityNode"), "node.type.graphene_core.raster.opacitynode");
		assert_eq!(descriptor_name("Core::Foo-Bar"), "node.type.core.foo_bar");
	}

	#[test]
	fn every_metadata_entry_produces_exactly_one_descriptor() {
		let entries = node_descriptors_with_identifiers();
		assert!(!entries.is_empty(), "catalog must be non-empty (shader-nodes must be enabled)");
		let identifiers: HashSet<&str> = entries.iter().map(|(identifier, _)| identifier.as_str()).collect();
		let names: HashSet<&str> = entries.iter().map(|(_, descriptor)| descriptor.name.as_str()).collect();
		assert_eq!(identifiers.len(), entries.len(), "identifier collision");
		assert_eq!(names.len(), entries.len(), "descriptor name collision");
	}

	#[test]
	fn every_input_schema_validates_as_json_schema() {
		// The schemas must be well-formed draft 2020-12 documents, not merely JSON.
		for (identifier, descriptor) in node_descriptors_with_identifiers() {
			assert_valid_schema(&descriptor.input_schema, &identifier);
			assert_valid_schema(&descriptor.output_schema, &identifier);
		}
	}

	fn assert_valid_schema(schema: &Value, identifier: &str) {
		jsonschema::validator_for(schema).unwrap_or_else(|error| panic!("{identifier} produced an invalid schema: {error}"));
	}

	#[test]
	fn catalog_json_is_non_empty_and_consistent() {
		let catalog = node_catalog_json();
		let nodes = catalog.get("nodes").and_then(Value::as_object).expect("nodes object");
		assert!(!nodes.is_empty());
		assert_eq!(catalog.get("count").and_then(Value::as_u64), Some(nodes.len() as u64));
	}
}
