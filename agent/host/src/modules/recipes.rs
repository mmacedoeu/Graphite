//! Recipes & archetypes — MCP tools for browsing, reading, and linting the
//! committed recipe corpus (design doc §6.4).
//!
//! All three tools are `Read` capability: a recipe archive is *reference
//! data*, never an executable surface (INV-13, R2-F, E-8). The host serves
//! the corpus; an agent applies a recipe by reading the source archive with
//! `document.open` and adapting the per-recipe `template.md`.
//!
//! The committed catalog is `agent/recipes.json` at the host's working
//! directory. The host reads it at tool-call time via the
//! `recipe_catalog::read_catalog` helper, which silently returns an empty
//! catalog when the file is missing so an attached-mode session whose CWD
//! is not a Graphite workspace stays responsive.

use super::{descriptor, optional_string, required_string, schema_object};
use crate::modules::recipe_catalog;
use graphite_agent_descriptors::recipes as descriptor_recipes;
use graphite_agent_protocol::{Capability, EditorBridge, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;

/// Default cwd-relative recipes directory, used by `recipes.lint` when it
/// needs to walk the corpus on disk.
const DEFAULT_RECIPES_ROOT: &str = "agent/recipes";

/// Look up one recipe entry from the on-disk catalog by id.
fn find_recipe(id: &str) -> Result<Value, ToolError> {
	let catalog = recipe_catalog::read_catalog();
	let recipes = catalog
		.get("recipes")
		.and_then(Value::as_array)
		.ok_or_else(|| ToolError::NotFound { what: format!("recipe catalog `recipes` array missing (catalog error: {})", catalog.get("error").and_then(Value::as_str).unwrap_or("none")) })?;
	let matched = recipes
		.iter()
		.find(|entry| entry.get("id").and_then(Value::as_str) == Some(id))
		.cloned();
	matched.ok_or_else(|| ToolError::NotFound { what: format!("recipe `{id}`") })
}

/// Read text from `path` if it exists, returning `None` silently otherwise.
/// Companion files (`template.md`, `preset.md`) are optional from the
/// agent's point of view.
fn read_optional_text(path: &Path) -> Option<String> {
	std::fs::read_to_string(path).ok()
}

#[derive(Debug, Default)]
pub struct RecipesModule;

impl ToolModule for RecipesModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"recipes.list",
				"Browse the recipes & archetypes corpus (committed at agent/recipes.json).",
				Capability::Read,
				schema_object(&[], json!({})),
				schema_object(
					&["recipes", "count"],
					json!({
						"recipes": {
							"type": "array",
							"items": { "type": "object" },
						},
						"count": { "type": "integer", "minimum": 0 },
					}),
				),
			),
			descriptor(
				"recipes.show",
				"Return the full recipe card (recipe.json + template.md + preset.md) for one id.",
				Capability::Read,
				schema_object(&["id"], json!({ "id": { "type": "string" } })),
				schema_object(
					&["id", "recipe", "template", "preset"],
					json!({
						"id": { "type": "string" },
						"recipe": { "type": "object" },
						"template": { "type": ["string", "null"] },
						"preset": { "type": ["string", "null"] },
					}),
				),
			),
			descriptor(
				"recipes.lint",
				"Lint the committed recipes corpus against the design doc's invariants and return every issue (recipes-and-archetypes plan §5).",
				Capability::Read,
				schema_object(&[], json!({})),
				schema_object(
					&["recipe_count", "issue_count", "has_errors", "issues"],
					json!({
						"recipe_count": { "type": "integer", "minimum": 0 },
						"issue_count": { "type": "integer", "minimum": 0 },
						"has_errors": { "type": "boolean" },
						"issues": { "type": "array", "items": { "type": "object" } },
					}),
				),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, _bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			match call.name.as_str() {
				"recipes.list" => {
					let catalog = recipe_catalog::read_catalog();
					let recipes = catalog
						.get("recipes")
						.and_then(Value::as_array)
						.cloned()
						.unwrap_or_default();
					let summaries: Vec<Value> = recipes
						.into_iter()
						.map(|recipe| {
							let id = recipe.get("id").and_then(Value::as_str).unwrap_or("").to_string();
							let template_path = std::path::PathBuf::from(recipe.get("template_path").and_then(Value::as_str).unwrap_or(""));
							let preset_path = template_path
								.parent()
								.map(|parent| parent.join("preset.md"))
								.unwrap_or_else(|| std::path::PathBuf::new());
							let has_template = !template_path.as_os_str().is_empty() && template_path.is_file();
							let required_count = recipe.get("required").and_then(Value::as_array).map(|a| a.len()).unwrap_or(0);
							json!({
								"id": id,
								"category": recipe.get("category"),
								"summary": recipe.get("summary"),
								"required_count": required_count,
								"has_template": has_template,
								"has_preset": preset_path.is_file(),
							})
						})
						.collect();
					let count = summaries.len();
					Ok(json!({ "recipes": summaries, "count": count }))
				}
				"recipes.show" => {
					let id = required_string(&call, "id")?;
					let recipe = find_recipe(&id)?;
					let template_path = std::path::PathBuf::from(recipe.get("template_path").and_then(Value::as_str).unwrap_or(""));
					let preset_path = template_path
						.parent()
						.map(|parent| parent.join("preset.md"))
						.unwrap_or_else(|| std::path::PathBuf::new());
					Ok(json!({
						"id": id,
						"recipe": recipe,
						"template": read_optional_text(&template_path),
						"preset": read_optional_text(&preset_path),
					}))
				}
				"recipes.lint" => {
					let explicit_root = optional_string(&call, "root").ok().flatten();
					let root_path = Path::new(explicit_root.as_deref().unwrap_or(DEFAULT_RECIPES_ROOT));
					let recipes = descriptor_recipes::load_all_recipes(root_path).map_err(|error| ToolError::Internal {
						message: format!("recipes.lint: failed to walk {root_path:?}: {error}"),
					})?;
					let issues = descriptor_recipes::lint_recipes(&recipes, root_path);
					let issues_json = descriptor_recipes::issues_json(&recipes, &issues);
					Ok(issues_json)
				}
				other => Err(ToolError::NotFound { what: format!("tool {other}") }),
			}
		})
	}
}

impl RecipesModule {
	pub fn new() -> Self {
		Self
	}
}
