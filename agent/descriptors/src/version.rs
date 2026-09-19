//! Catalog versioning (§5.6) and one-phase deprecation support (T3.5, T3.7).
//!
//! §5.6, verbatim:
//! - `ToolDescriptor.version` starts at `1`.
//! - Adding an optional argument → keep version, update `input_schema`.
//! - Removing/renaming an argument or changing result shape → bump by 1, keep the
//!   old descriptor one phase.
//! - Changing an interaction pattern (streaming, subscriptions) requires escalation.
//!
//! Everything here is catalog-side data. A bump changes only the descriptor's own
//! `version` / `input_schema`; it never touches `agent/protocol` (whose
//! [`ToolDescriptor`] shape is fixed by §5.1) and it never touches `agent/mcp`
//! (command descriptors are not tools, INV-8/E-8).

use graphite_agent_protocol::ToolDescriptor;

/// The catalog format version carried by `graphite://command-catalog`.
pub const CATALOG_VERSION: u32 = 1;

/// Every generated descriptor starts at version 1 (§5.6).
pub const VERSION: u32 = 1;

/// The two §5.6 change classes this catalog supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
	/// Adding an optional argument: keep the version, update `input_schema`.
	AdditiveOptional,
	/// Removing/renaming an argument or changing the result shape: bump by 1.
	Breaking,
}

/// Apply a §5.6 change to a descriptor. `updated_input_schema` is the new schema
/// for an additive change; a breaking change must also supply it.
pub fn apply_change(descriptor: &ToolDescriptor, change: Change, updated_input_schema: serde_json::Value) -> ToolDescriptor {
	let mut updated = descriptor.clone();
	updated.input_schema = updated_input_schema;
	if change == Change::Breaking {
		updated.version = descriptor.version.saturating_add(1);
	}
	updated
}

/// One catalog entry, tracking when it became deprecated (if ever).
#[derive(Clone, Debug, PartialEq)]
pub struct VersionedEntry {
	pub descriptor: ToolDescriptor,
	/// The phase in which a newer descriptor replaced this one, or `None` while it
	/// is current.
	pub deprecated_in_phase: Option<u32>,
}

impl VersionedEntry {
	pub fn is_current(&self) -> bool {
		self.deprecated_in_phase.is_none()
	}
}

/// A versioned catalog. At most one current descriptor per name; superseded ones
/// survive one phase for clients pinned to the old shape (T3.7).
#[derive(Debug, Default)]
pub struct Catalog {
	entries: Vec<VersionedEntry>,
	phase: u32,
}

impl Catalog {
	/// The current phase counter.
	pub fn phase(&self) -> u32 {
		self.phase
	}

	/// Every entry, current and still-deprecated, in insertion order.
	pub fn entries(&self) -> &[VersionedEntry] {
		&self.entries
	}

	/// Every current (non-deprecated) descriptor.
	pub fn current(&self) -> Vec<&ToolDescriptor> {
		self.entries.iter().filter(|entry| entry.is_current()).map(|entry| &entry.descriptor).collect()
	}

	/// Every descriptor still readable after a bump, including one-phase-old ones.
	pub fn readable(&self) -> Vec<&ToolDescriptor> {
		self.entries.iter().map(|entry| &entry.descriptor).collect()
	}

	/// Insert a new descriptor, or replace one with the same name. A replacement
	/// keeps the old descriptor, marked deprecated in the current phase.
	pub fn upsert(&mut self, descriptor: ToolDescriptor) {
		let name = descriptor.name.clone();
		let previous_current = self.entries.iter().position(|entry| entry.is_current() && entry.descriptor.name == name);
		if let Some(index) = previous_current {
			self.entries[index].deprecated_in_phase = Some(self.phase);
		}
		self.entries.push(VersionedEntry {
			descriptor,
			deprecated_in_phase: None,
		});
	}

	/// Advance to the next phase and drop entries that have now outlived their one
	/// phase of grace: an entry deprecated in phase `P` is readable during `P` and
	/// `P + 1`, and removed once the catalog reaches `P + 2`.
	pub fn advance_phase(&mut self) {
		self.phase = self.phase.saturating_add(1);
		self.entries.retain(|entry| match entry.deprecated_in_phase {
			Some(phase) => self.phase < phase.saturating_add(2),
			None => true,
		});
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use graphite_agent_protocol::Capability;
	use serde_json::json;

	fn descriptor(name: &str, version: u32) -> ToolDescriptor {
		ToolDescriptor {
			name: name.to_string(),
			description: format!("{name} descriptor"),
			capability: Capability::Read,
			input_schema: json!({ "type": "object", "properties": {} }),
			output_schema: json!({ "type": "object" }),
			version,
			meta: None,
		}
	}

	/// T3.5: the protocol type's §5.1 shape is fixed, except for the additive
	/// optional `meta` field (E-18). This exhaustive destructuring is a compile-time
	/// proof: if a field were added to or removed from `ToolDescriptor`, this test
	/// would stop compiling and force a deliberate review.
	#[test]
	fn protocol_descriptor_shape_is_untouched_by_versioning() {
		let descriptor = descriptor("command.tool.select.pointermove", VERSION);
		let changed = apply_change(
			&descriptor,
			Change::Breaking,
			json!({ "type": "object", "properties": { "modifier_keys": { "x-rust-type": "SelectToolPointerKeys" } } }),
		);

		let ToolDescriptor {
			name,
			description,
			capability,
			input_schema,
			output_schema,
			version,
			meta,
		} = changed;
		assert!(meta.is_none(), "catalog descriptors carry no `_meta`");
		assert_eq!(name, "command.tool.select.pointermove");
		assert!(!description.is_empty());
		assert_eq!(capability, Capability::Read);
		assert!(input_schema.get("properties").is_some());
		assert!(output_schema.is_object());
		assert_eq!(version, VERSION + 1);
	}

	/// T3.5: versioning only rewrites the descriptor's own `version` /
	/// `input_schema`. The original serialized descriptor is byte-identical before
	/// and after a bump, so nothing in `agent/protocol` or `agent/mcp` changes.
	#[test]
	fn a_version_bump_does_not_mutate_the_existing_descriptor() {
		let original = descriptor("command.portfolio.document.undo", VERSION);
		let before = serde_json::to_string(&original).expect("serialize");

		let additive = apply_change(&original, Change::AdditiveOptional, json!({ "type": "object", "properties": { "force": { "type": "boolean" } } }));
		let breaking = apply_change(&original, Change::Breaking, json!({ "type": "object", "properties": {} }));

		// The additive change keeps the version; the breaking change bumps it.
		assert_eq!(additive.version, VERSION);
		assert_eq!(breaking.version, VERSION + 1);
		// The original descriptor was not mutated by either change.
		assert_eq!(serde_json::to_string(&original).expect("serialize"), before);
	}

	/// T3.5: the wire shape of a descriptor is exactly the §5.1 set. A version
	/// bump changes only `version` (and the caller-supplied `input_schema`), so
	/// neither `agent/protocol` nor the MCP adapter needs to change.
	#[test]
	fn serialized_descriptor_keeps_the_frozen_protocol_shape() {
		use std::collections::BTreeSet;

		let value = serde_json::to_value(descriptor("command.portfolio.document.undo", VERSION)).expect("serialize");
		let keys: BTreeSet<&str> = value.as_object().expect("object").keys().map(String::as_str).collect();
		assert_eq!(
			keys,
			BTreeSet::from(["name", "description", "capability", "input_schema", "output_schema", "version"]),
			"the §5.1 ToolDescriptor shape is frozen; versioning must not add or remove fields"
		);
	}

	/// T3.7: an old descriptor survives one phase after the bump, then is dropped.
	#[test]
	fn superseded_descriptor_survives_exactly_one_phase() {
		let mut catalog = Catalog::default();
		catalog.upsert(descriptor("command.portfolio.document.undo", VERSION));
		assert_eq!(catalog.current().len(), 1);

		// A breaking change in phase 0: old kept, marked deprecated; new is current.
		catalog.upsert(descriptor("command.portfolio.document.undo", VERSION + 1));
		assert_eq!(catalog.current().len(), 1);
		assert_eq!(catalog.readable().len(), 2, "the old descriptor survives the phase it was replaced in");
		assert_eq!(catalog.readable()[0].version, VERSION);

		// Phase 1: still readable (this is the "one phase" of grace).
		catalog.advance_phase();
		assert_eq!(catalog.phase(), 1);
		assert_eq!(catalog.readable().len(), 2);
		assert_eq!(catalog.current().len(), 1);

		// Phase 2: the grace phase has elapsed.
		catalog.advance_phase();
		assert_eq!(catalog.phase(), 2);
		assert_eq!(catalog.readable().len(), 1);
		assert_eq!(catalog.readable()[0].version, VERSION + 1);
	}
}
