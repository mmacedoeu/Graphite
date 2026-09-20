//! Read-only resource for the committed `agent/recipes.json` catalog.
//!
//! This is the MCP companion to the `node-catalog` and `command-catalog`
//! resources. Like them, it is *catalog data, never an executable surface*
//! (INV-8, E-8). Agents read it through the resource path to learn what
//! recipes exist; they apply a recipe by reading the source archive with
//! `document.open` and adapting the per-recipe `template.md`.
//!
//! The catalog is read from `<cwd>/agent/recipes.json` at resource-read time.
//! This file is committed at the workspace root and refreshed by
//! `graphite-agent-descriptors recipes --root <dir> --out <path>`. When the
//! file is missing (e.g. an attached-mode session whose CWD is not a
//! Graphite workspace) the resource returns a valid empty catalog.

use graphite_agent_descriptors::recipes as descriptor_recipes;
use serde_json::Value;
use std::fs;
use std::path::Path;

/// Default path for the committed catalog, relative to the host's current
/// working directory. Both Codex and Claude Code invoke `graphite-agent`
/// with the workspace root as CWD, so the resource resolves to
/// `<workspace>/agent/recipes.json` in production.
const DEFAULT_CATALOG_PATH: &str = "agent/recipes.json";

/// Read and parse the committed catalog. Returns an empty catalog on any I/O
/// or parse error (the resource contract is "always yield a value, never
/// fail"), with the error captured in the response's `error` field for
/// debugging.
pub fn read_catalog() -> Value {
	read_catalog_at(Path::new(DEFAULT_CATALOG_PATH))
}

/// Same as [`read_catalog`] but with a caller-supplied catalog path. Used by
/// the conformance suite to point at a temp-dir catalog.
pub fn read_catalog_at(path: &Path) -> Value {
	let raw = match fs::read_to_string(path) {
		Ok(raw) => raw,
		Err(_) => return empty_catalog_with_note(format!("catalog not found at {}", path.display())),
	};
	let value: Value = match serde_json::from_str(&raw) {
		Ok(value) => value,
		Err(error) => return empty_catalog_with_note(format!("catalog at {} did not parse: {error}", path.display())),
	};
	// Normalize to the canonical shape regardless of how the file was
	// committed. If the file carries `count`, leave it; otherwise add it
	// from the `recipes` array length.
	let mut value = value;
	let needs_count = value.get("count").is_none();
	let count = value.get_mut("recipes").and_then(Value::as_array_mut).map(|recipes| recipes.len());
	if needs_count
		&& let Some(count) = count
		&& let Some(object) = value.as_object_mut()
	{
		object.insert("count".to_string(), Value::Number(count.into()));
	}
	value
}

fn empty_catalog_with_note(note: String) -> Value {
	let recipes: Vec<Value> = Vec::new();
	let note_value = Value::String(note);
	let mut catalog = serde_json::Map::new();
	catalog.insert("schema_version".into(), Value::Number(1.into()));
	catalog.insert("count".into(), Value::Number(0.into()));
	catalog.insert("recipes".into(), Value::Array(recipes));
	catalog.insert("error".into(), note_value);
	Value::Object(catalog)
}

/// Count recipes on disk under `root`. Used by callers who want a live
/// count without paying for the full catalog JSON round-trip.
pub fn count_at(root: &Path) -> usize {
	match descriptor_recipes::load_all_recipes(root) {
		Ok(recipes) => recipes.len(),
		Err(_) => 0,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;

	#[test]
	fn missing_catalog_returns_valid_empty_shape() {
		let value = read_catalog_at(Path::new("/nonexistent/path/agent/recipes.json"));
		assert_eq!(value["count"], Value::Number(0.into()));
		assert!(value["error"].is_string());
	}

	#[test]
	fn well_formed_catalog_is_returned_verbatim_with_count_normalised() {
		let temp = TempDir::new().unwrap();
		let path = temp.path().join("recipes.json");
		let raw = r#"{
			"schema_version": 1,
			"count": 7,
			"recipes": [{}]
		}"#;
		fs::write(&path, raw).unwrap();
		let value = read_catalog_at(&path);
		assert_eq!(value["count"], Value::Number(7.into()));
	}

	#[test]
	fn catalog_missing_count_is_backfilled_from_recipes_length() {
		let temp = TempDir::new().unwrap();
		let path = temp.path().join("recipes.json");
		let raw = r#"{
			"schema_version": 1,
			"recipes": [{}, {}, {}]
		}"#;
		fs::write(&path, raw).unwrap();
		let value = read_catalog_at(&path);
		assert_eq!(value["count"], Value::Number(3.into()));
	}
}
