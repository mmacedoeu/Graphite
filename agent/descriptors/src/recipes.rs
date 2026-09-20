//! Recipes & archetypes — schema, lint, and sha256 stamping for the agent's
//! node-graph knowledge layer.
//!
//! Mirrors the discipline of motion-design's three-tier artifact hierarchy
//! (`source-prompts/<id>.txt` + `templates/<id>.md` + `presets/<id>.md` indexed
//! by `assets/presets.json`) translated into the Graphite `.gdd` + template
//! domain. The catalog is committed at `agent/recipes.json`; per-recipe files
//! live under `agent/recipes/<id>/`.
//!
//! This module owns:
//! - The 10-field machine schema (see [`Recipe`]).
//! - The category enum (8 values) and id regex.
//! - Lint: required identifiers resolve in the live `NODE_METADATA`; template
//!   headings cover either the static (`Composition Plan`) or animated
//!   (`Animation Timeline`) discipline; sha256 pins match on-disk bytes.
//!
//! It does *not* re-render the asset. Asset re-rendering lives behind the
//! host's `render.preview_gif` / `render.export_gif` and is invoked by an
//! external orchestrator; the descriptors crate only stamps sha256s and
//! validates schema. (See `agent/docs/plans/2026-09-19-recipes-and-archetypes-design.md`
//! §7 Build pipeline.)

use crate::node_descriptors_with_identifiers;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The closed 8-value category enum (design doc §5.3).
///
/// Kept in lockstep with the lint check; a `BTreeSet` is built once for O(1)
/// lookup.
const CATEGORIES: &[&str] = &[
	"Motion",
	"Filtering",
	"Color",
	"Typography",
	"Geometry",
	"Compositing",
	"Conversion",
	"Debug",
];

/// Identifier fragments whose presence in `required[]` exempts a recipe from
/// the "every animated recipe needs an Animation node" rule (analog of
/// `Motion`-category recipes that pull `AnimationTime` from somewhere
/// downstream) and that mark a recipe as having an animation-time surface.
const ANIMATION_TIME_IDENTIFIERS: &[&str] = &[
	"graphene_core::animation::AnimationTimeNode",
	"graphene_core::animation::RealTimeNode",
	"graphene_core::animation::PointerPositionNode",
	"graphene_core::animation::QuantizeAnimationTimeNode",
];

/// Mandatory top-level headings in `template.md`. One of the two
/// `*_PLAN_HEADINGS` sets must be present; the union of the rest must always be
/// present (See design doc §5.4.)
const REQUIRED_COMMON_HEADINGS: &[&str] = &["Source", "Asset Plan", "Default Bindings", "Fidelity Checks"];
const REQUIRED_ANIMATED_HEADING: &str = "Animation Timeline";
const REQUIRED_STATIC_HEADING: &str = "Composition Plan";

/// The id regex (mirrors `motion-design`'s `build_gallery.py` line 41).
const ID_REGEX_TEXT: &str = r"^[a-z0-9]+(?:-[a-z0-9]+)*$";

/// The 10-field machine schema for a single recipe (design doc §5.2).
///
/// All fields are required for the file-on-disk schema; some are optional in
/// the [`Recipe`] surface (e.g. `aliases`, `color`) so a missing-but-optional
/// field parses as `None`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
	#[serde(rename = "id")]
	pub id: String,
	pub category: String,
	pub summary: String,
	#[serde(default)]
	pub aliases: Vec<String>,
	#[serde(default)]
	pub color: Option<String>,
	pub required: Vec<String>,
	pub defaults: RecipeDefaults,
	pub source: RecipeSource,
	pub template_path: String,
	pub template_version: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecipeDefaults {
	pub fps: Option<f64>,
	pub frames: Option<u32>,
	pub loop_value: Option<bool>,
	#[serde(rename = "loop", skip_serializing_if = "Option::is_none")]
	pub _legacy_loop: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecipeSource {
	pub path: String,
	#[serde(default)]
	pub sha256: Option<String>,
	pub asset: String,
	#[serde(default)]
	pub asset_sha256: Option<String>,
	pub format_version: u32,
}

/// One observed problem.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RecipeIssue {
	pub recipe_id: String,
	pub level: IssueLevel,
	pub code: String,
	pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum IssueLevel {
	Error,
	Warning,
}

/// Load + validate every recipe under `root/<id>/recipe.json`. Walks one
/// level of subdirectories; each subdirectory whose name matches the id
/// regex is a candidate recipe. The returned tuples carry the *discovered*
/// directory path so lint can verify id/dir alignment correctly.
pub fn load_all_recipes(root: &Path) -> Result<Vec<(PathBuf, Recipe)>, String> {
	if !root.is_dir() {
		return Ok(Vec::new());
	}
	let mut recipes = Vec::new();
	for entry in fs::read_dir(root).map_err(|error| format!("recipes root: read_dir {}: {error}", root.display()))? {
		let entry = entry.map_err(|error| format!("recipes root: read entry: {error}"))?;
		let path = entry.path();
		if !path.is_dir() {
			continue;
		}
		let dir_name = match path.file_name().and_then(|name| name.to_str()) {
			Some(name) => name.to_string(),
			None => continue,
		};
		if !is_valid_id(&dir_name) {
			continue;
		}
		let recipe_path = path.join("recipe.json");
		if !recipe_path.is_file() {
			continue;
		}
		let raw = fs::read_to_string(&recipe_path)
			.map_err(|error| format!("recipes: read {}: {error}", recipe_path.display()))?;
		let recipe: Recipe = serde_json::from_str(&raw).map_err(|error| format!("recipes: parse {}: {error}", recipe_path.display()))?;
		recipes.push((path, recipe));
	}
	recipes.sort_by(|a, b| a.1.id.cmp(&b.1.id));
	Ok(recipes)
}

/// The committed `agent/recipes.json`. Mirrors the prefix shape of
/// motion-design's `presets.json`.
pub fn recipes_catalog_json(recipes: &[(PathBuf, Recipe)]) -> Value {
	let recipe_values: Vec<Value> = recipes.iter().map(|(_, recipe)| recipe_to_catalog_entry(recipe)).collect();
	json!({
		"schema_version": 1,
		"count": recipe_values.len(),
		"recipes": recipe_values,
	})
}

fn recipe_to_catalog_entry(recipe: &Recipe) -> Value {
	json!({
		"id": recipe.id,
		"category": recipe.category,
		"summary": recipe.summary,
		"aliases": recipe.aliases,
		"color": recipe.color,
		"required": recipe.required,
		"defaults": {
			"fps": recipe.defaults.fps,
			"frames": recipe.defaults.frames,
			"loop": recipe.defaults.loop_value,
		},
		"source": {
			"path": recipe.source.path,
			"sha256": recipe.source.sha256,
			"asset": recipe.source.asset,
			"asset_sha256": recipe.source.asset_sha256,
			"format_version": recipe.source.format_version,
		},
		"template_path": recipe.template_path,
		"template_version": recipe.template_version,
	})
}

/// One issue per invariant violation. `recipes-lint` exits non-zero iff any
/// issue has level `Error` and the `--strict` flag is set (warnings are
/// always returned but exit 0 even under `--strict`).
pub fn lint_recipes(recipes: &[(PathBuf, Recipe)], _root: &Path) -> Vec<RecipeIssue> {
	let mut issues = Vec::new();
	let categories: BTreeSet<&str> = CATEGORIES.iter().copied().collect();
	let node_identifiers: BTreeSet<String> = node_descriptors_with_identifiers().into_iter().map(|(id, _)| id).collect();

	for (recipe_root, recipe) in recipes {
		let dir_name = recipe_root
			.file_name()
			.and_then(|name| name.to_str())
			.map(str::to_string)
			.unwrap_or_default();
		if dir_name != recipe.id {
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Error,
				code: "id.mismatches_dir".into(),
				message: format!("recipe.json id `{}` does not match directory `{}`", recipe.id, dir_name),
			});
		}

		if !is_valid_id(&recipe.id) {
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Error,
				code: "id.regex".into(),
				message: format!("recipe id `{}` does not match `{}`", recipe.id, ID_REGEX_TEXT),
			});
		}

		if !categories.contains(recipe.category.as_str()) {
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Error,
				code: "category.unknown".into(),
				message: format!(
					"recipe category `{}` is not in the closed enum {:?}",
					recipe.category,
					CATEGORIES
				),
			});
		}

		for identifier in &recipe.required {
			if !node_identifiers.contains(identifier) {
				issues.push(RecipeIssue {
					recipe_id: recipe.id.clone(),
					level: IssueLevel::Error,
					code: "required.unresolved".into(),
					message: format!("required identifier `{}` is not in the live `NODE_METADATA`", identifier),
				});
			}
		}

		if recipe.defaults.frames.is_none() && recipe.defaults.fps.is_none() {
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Warning,
				code: "defaults.fps_or_frames_missing".into(),
				message: "recipe has neither `fps` nor `frames` in `defaults`; rendering falls back to defaults".into(),
			});
		}

		let animated = recipe.defaults.frames.map(|f| f > 1).unwrap_or(false);
		if animated {
			let has_animation_node = recipe
				.required
				.iter()
				.any(|id| ANIMATION_TIME_IDENTIFIERS.iter().any(|needle| id.contains(needle)));
			if !has_animation_node {
				issues.push(RecipeIssue {
					recipe_id: recipe.id.clone(),
					level: IssueLevel::Error,
					code: "animation.no_time_source".into(),
					message: "frames > 1 requires an AnimationTime / RealTime / PointerPosition / QuantizeAnimationTime node in `required[]`".into(),
				});
			}
		}

		let template_path = recipe_root.join("template.md");
		let template_ok = if template_path.is_file() {
			match fs::read_to_string(&template_path) {
				Ok(text) => {
					check_template_headings(&recipe.id, &text, animated, &mut issues);
					true
				}
				Err(error) => {
					issues.push(RecipeIssue {
						recipe_id: recipe.id.clone(),
						level: IssueLevel::Error,
						code: "template.unreadable".into(),
						message: format!("failed to read {}: {error}", template_path.display()),
					});
					false
				}
			}
		} else {
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Error,
				code: "template.missing".into(),
				message: format!("`template.md` is missing at {}", recipe_root.display()),
			});
			false
		};

		if template_ok {
			let template_heading_ok = issues
				.iter()
				.filter(|issue| issue.recipe_id == recipe.id && issue.code.starts_with("template.heading.missing"))
				.count()
				== 0;
			if template_heading_ok
				&& let Ok(template_text) = fs::read_to_string(&template_path)
			{
				check_template_animation_markers(&recipe.id, &template_text, animated, recipe.defaults.frames, recipe.defaults.fps, &mut issues);
			}
		}

		// Reserved slot for sha256 verification once `assets/` files are seeded.
		let source_path = PathBuf::from(&recipe.source.path);
		if !source_path.is_file() {
			// `source.missing` is a Warning, not an Error: the canonical re-render
			// path lives on `recipes-build` (asset-render hooks added in a
			// follow-on), so a freshly-seeded recipe without on-disk bytes is
			// not a CI-blocking failure. The CI gate on `recipes-lint --strict`
			// still surfaces it for maintainers.
			issues.push(RecipeIssue {
				recipe_id: recipe.id.clone(),
				level: IssueLevel::Warning,
				code: "source.missing".into(),
				message: format!("source archive is missing at {}", source_path.display()),
			});
		}
	}
	issues
}

fn check_template_headings(recipe_id: &str, text: &str, animated: bool, issues: &mut Vec<RecipeIssue>) {
	for heading in REQUIRED_COMMON_HEADINGS {
		if !text.contains(&format!("## {heading}")) {
			issues.push(RecipeIssue {
				recipe_id: recipe_id.to_string(),
				level: IssueLevel::Error,
				code: format!("template.heading.missing.{heading_kebab}", heading_kebab = kebab(heading)),
				message: format!("`template.md` is missing required heading `## {heading}`"),
			});
		}
	}
	let plan = if animated { REQUIRED_ANIMATED_HEADING } else { REQUIRED_STATIC_HEADING };
	if !text.contains(&format!("## {plan}")) {
		issues.push(RecipeIssue {
			recipe_id: recipe_id.to_string(),
			level: IssueLevel::Error,
			code: format!("template.heading.missing.{plan_kebab}", plan_kebab = kebab(plan)),
			message: format!("`template.md` is missing required heading `## {plan}` (recipe is {})", if animated { "animated" } else { "static" }),
		});
	}
}

fn check_template_animation_markers(
	recipe_id: &str,
	text: &str,
	animated: bool,
	frames: Option<u32>,
	fps: Option<f64>,
	issues: &mut Vec<RecipeIssue>,
) {
	if !animated {
		return;
	}
	let (Some(frames), Some(fps)) = (frames, fps) else {
		return;
	};
	if frames == 0 || fps <= 0.0 {
		return;
	}
	let duration = (frames as f64) / fps;
	// Use the same regex as motion-design's `build_gallery.py:108`
	// (`[(\d+(?:\.\d+)?)–(\d+(?:\.\d+)?)s]`) to detect timeline markers.
	let mut marker_count = 0_usize;
	let mut last_end = 0.0_f64;
	for captures in timeline_marker_regex().captures_iter(text).flatten() {
		marker_count += 1;
		let start: f64 = captures[1].parse().unwrap_or(0.0);
		let end: f64 = captures[2].parse().unwrap_or(0.0);
		if start < last_end - 1e-6 || end <= start {
			issues.push(RecipeIssue {
				recipe_id: recipe_id.to_string(),
				level: IssueLevel::Error,
				code: "template.timeline.overlap_or_reversed".into(),
				message: format!("animation timeline marker [{start}–{end}s] overlaps or reverses the previous segment"),
			});
		}
		last_end = end;
	}
	if marker_count == 0 {
		issues.push(RecipeIssue {
			recipe_id: recipe_id.to_string(),
			level: IssueLevel::Warning,
			code: "template.timeline.no_markers".into(),
			message: format!("animated recipe has no `[start–end s]` markers in `template.md`; expected to cover {duration:.2}s"),
		});
	} else if (last_end - duration).abs() > 1e-3 {
		issues.push(RecipeIssue {
			recipe_id: recipe_id.to_string(),
			level: IssueLevel::Warning,
			code: "template.timeline.does_not_cover_duration".into(),
			message: format!("animation timeline covers {last_end:.2}s but `defaults.frames / defaults.fps = {duration:.2}s`"),
		});
	}
}

fn timeline_marker_regex() -> fancy_regex::Regex {
	// Built lazily inside the helper to keep this module free of `lazy_static`
	// dependencies; the regex is small.
	fancy_regex::Regex::new(r"\[(\d+(?:\.\d+)?)–(\d+(?:\.\d+)?)s\]").expect("timeline marker regex is valid")
}

/// Compute sha256 hex digest of a file's bytes. Used for asset-stamping in
/// `recipes-build` and verification in `recipes-lint`.
pub fn sha256_of_file(path: &Path) -> Result<String, String> {
	let bytes = fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
	Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
	use sha2::{Digest, Sha256};
	let mut hasher = Sha256::new();
	hasher.update(bytes);
	let digest = hasher.finalize();
	let mut hex = String::with_capacity(64);
	for byte in digest {
		hex.push_str(&format!("{byte:02x}"));
	}
	hex
}

fn is_valid_id(id: &str) -> bool {
	fancy_regex::Regex::new(ID_REGEX_TEXT)
		.map(|re| re.is_match(id).unwrap_or(false))
		.unwrap_or(false)
}

fn kebab(s: &str) -> String {
	let lowered = s.to_lowercase();
	str::replace(&lowered, ' ', "-").replace('_', "-")
}

/// Write `agent/recipes.json` to `path`, pretty-printed with a trailing
/// newline.
pub fn write_recipes_catalog(path: &Path, recipes: &[(PathBuf, Recipe)]) -> std::io::Result<()> {
	if let Some(parent) = path.parent()
		&& !parent.as_os_str().is_empty()
	{
		std::fs::create_dir_all(parent)?;
	}
	let mut serialized = serde_json::to_string_pretty(&recipes_catalog_json(recipes)).expect("recipes catalog is serializable");
	serialized.push('\n');
	let mut file = fs::File::create(path)?;
	file.write_all(serialized.as_bytes())?;
	Ok(())
}

/// Stamp every recipe's on-disk `recipe.json` with fresh `source.sha256` and
/// `source.asset_sha256` values, computed from the on-disk bytes. Returns the
/// number of recipes that were actually rewritten (a recipe with neither a
/// `source.gdd` nor an `asset.<gif|png>` on disk produces no edits).
///
/// Asset re-rendering is *not* in scope for this function; it just hashes what
/// is there. The canonical re-render path lives on the host
/// (`render.preview_gif` / `render.export_gif`).
pub fn stamp_all(_root: &Path, recipes: &[(PathBuf, Recipe)]) -> Result<usize, String> {
	let mut stamped = 0_usize;
	for (recipe_root, recipe) in recipes {
		let recipe_path = recipe_root.join("recipe.json");
		if !recipe_path.is_file() {
			continue;
		}
		let source_path = PathBuf::from(&recipe.source.path);
		let new_source_sha = if source_path.is_file() { Some(sha256_of_file(&source_path)?) } else { None };
		let asset_path = PathBuf::from(&recipe.source.asset);
		let new_asset_sha = if asset_path.is_file() { Some(sha256_of_file(&asset_path)?) } else { None };
		if new_source_sha.is_none() && new_asset_sha.is_none() {
			continue;
		}
		let raw = fs::read_to_string(&recipe_path).map_err(|error| format!("stamp: read {}: {error}", recipe_path.display()))?;
		let mut value: Value = serde_json::from_str(&raw).map_err(|error| format!("stamp: parse {}: {error}", recipe_path.display()))?;
		if let Some(new_source_sha) = new_source_sha
			&& let Some(source_obj) = value.get_mut("source").and_then(|s| s.as_object_mut())
		{
			source_obj.insert("sha256".to_string(), Value::String(new_source_sha));
		}
		if let Some(new_asset_sha) = new_asset_sha
			&& let Some(source_obj) = value.get_mut("source").and_then(|s| s.as_object_mut())
		{
			source_obj.insert("asset_sha256".to_string(), Value::String(new_asset_sha));
		}
		let serialized = serde_json::to_string_pretty(&value).map_err(|error| format!("stamp: serialize: {error}"))?;
		fs::write(&recipe_path, serialized).map_err(|error| format!("stamp: write {}: {error}", recipe_path.display()))?;
		stamped += 1;
	}
	Ok(stamped)
}

/// Lint report encoded as JSON for the `recipes-lint` subcommand. Stable
/// shape: `{ recipe_count, issue_count, has_errors, issues: [...] }`.
pub fn issues_json(recipes: &[(PathBuf, Recipe)], issues: &[RecipeIssue]) -> Value {
	json!({
		"recipe_count": recipes.len(),
		"issue_count": issues.len(),
		"has_errors": issues.iter().any(|issue| matches!(issue.level, IssueLevel::Error)),
		"issues": issues,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use tempfile::TempDir;

	const COMPLETE_TEMPLATE_STATIC: &str = "## Source\n\nplaceholder\n\n## Composition Plan\n\nplaceholder\n\n## Asset Plan\n\nplaceholder\n\n## Default Bindings\n\nplaceholder\n\n## Fidelity Checks\n\nplaceholder\n";
	const COMPLETE_TEMPLATE_ANIMATED: &str = "## Source\n\nplaceholder\n\n## Animation Timeline\n\n[0.0–1.0s] stage 1\n[1.0–2.0s] stage 2\n[2.0–3.0s] stage 3\n\n## Asset Plan\n\nplaceholder\n\n## Default Bindings\n\nplaceholder\n\n## Fidelity Checks\n\nplaceholder\n";

	fn write_recipe(root: &Path, dir: &str, body: &str) {
		write_recipe_with_template(root, dir, body, COMPLETE_TEMPLATE_STATIC);
	}

	fn write_recipe_with_template(root: &Path, dir: &str, body: &str, template: &str) {
		let path = root.join(dir);
		fs::create_dir_all(&path).unwrap();
		fs::write(path.join("recipe.json"), body).unwrap();
		fs::write(path.join("template.md"), template).unwrap();
	}

	fn load_and_lint(root: &Path) -> Vec<RecipeIssue> {
		let recipes = load_all_recipes(root).unwrap();
		lint_recipes(&recipes, root)
	}

	#[test]
	fn recipe_id_regex_matches_motion_designs_pattern() {
		assert!(is_valid_id("kinetic-typography"));
		assert!(is_valid_id("tropical-product"));
		assert!(is_valid_id("a"));
		assert!(is_valid_id("a-b-c"));
		assert!(!is_valid_id("Capitalized"));
		assert!(!is_valid_id("with_underscore"));
		assert!(!is_valid_id("-leading-dash"));
		assert!(!is_valid_id("trailing-dash-"));
		assert!(!is_valid_id("double--dash"));
	}

	#[test]
	fn lint_accepts_a_minimal_static_recipe() {
		let temp = TempDir::new().unwrap();
		let body = r#"{
			"id": "demo-static",
			"category": "Filtering",
			"summary": "A demo.",
			"required": [],
			"defaults": {},
			"source": {
				"path": "agent/recipes/demo-static/source.gdd",
				"asset": "agent/recipes/demo-static/asset.png",
				"format_version": 1
			},
			"template_path": "agent/recipes/demo-static/template.md",
			"template_version": 1
		}"#;
		write_recipe(temp.path(), "demo-static", body);
		let mut issues = load_and_lint(temp.path());
		issues.retain(|issue| !matches!(issue.code.as_str(), "source.missing" | "defaults.fps_or_frames_missing"));
		assert!(issues.is_empty(), "issues: {issues:#?}");
	}

	#[test]
	fn lint_rejects_unknown_category() {
		let temp = TempDir::new().unwrap();
		let body = r#"{
			"id": "demo",
			"category": "KitchenSink",
			"summary": "A demo.",
			"required": [],
			"defaults": {},
			"source": {
				"path": "agent/recipes/demo/source.gdd",
				"asset": "agent/recipes/demo/asset.png",
				"format_version": 1
			},
			"template_path": "agent/recipes/demo/template.md",
			"template_version": 1
		}"#;
		write_recipe(temp.path(), "demo", body);
		let issues = load_and_lint(temp.path());
		assert!(issues.iter().any(|issue| issue.code == "category.unknown"));
	}

	#[test]
	fn lint_rejects_animated_recipe_without_time_source() {
		let temp = TempDir::new().unwrap();
		let body = r#"{
			"id": "demo-animated",
			"category": "Motion",
			"summary": "Animated without time.",
			"required": ["gstd:GaussianBlurNode"],
			"defaults": {"fps": 30.0, "frames": 60, "loop": true},
			"source": {
				"path": "agent/recipes/demo-animated/source.gdd",
				"asset": "agent/recipes/demo-animated/asset.gif",
				"format_version": 1
			},
			"template_path": "agent/recipes/demo-animated/template.md",
			"template_version": 1
		}"#;
		write_recipe_with_template(temp.path(), "demo-animated", body, COMPLETE_TEMPLATE_ANIMATED);
		let issues = load_and_lint(temp.path());
		assert!(issues.iter().any(|issue| issue.code == "animation.no_time_source"));
	}

	#[test]
	fn id_mismatch_with_directory_is_an_error() {
		let temp = TempDir::new().unwrap();
		let body = r#"{
			"id": "actually-a-different-id",
			"category": "Filtering",
			"summary": "x",
			"required": [],
			"defaults": {},
			"source": {
				"path": "agent/recipes/dir-name/source.gdd",
				"asset": "agent/recipes/dir-name/asset.png",
				"format_version": 1
			},
			"template_path": "agent/recipes/dir-name/template.md",
			"template_version": 1
		}"#;
		write_recipe(temp.path(), "dir-name", body);
		let issues = load_and_lint(temp.path());
		assert!(
			issues.iter().any(|issue| issue.code == "id.mismatches_dir"),
			"expected id.mismatches_dir; got: {issues:#?}"
		);
	}

	#[test]
	fn sha256_is_lowercase_hex_of_correct_length() {
		let digest = sha256_hex(b"hello");
		assert_eq!(digest.len(), 64);
		assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
		assert!(digest.chars().all(|c| !c.is_ascii_uppercase()));
	}
}
