//! `graphite-agent-descriptors` — the Phase 0 catalog/inventory binary (T0.5),
//! extended in Phase 3 (T3.6), and the recipes & archetypes layer (T7.x).
//!
//! Subcommands:
//! - `inventory --out <path>`: write `agent/inventory.json` (T0.6).
//! - `coverage`: print node + message-action coverage, including every
//!   non-allowlisted message and its status (T3.6).
//! - `recipes --out <path>`: validate every recipe under `--root` and write
//!   the committed `agent/recipes.json` (T7.2).
//! - `recipes-build --root <dir>`: re-stamp per-recipe sha256 pins. Asset
//!   re-rendering is *not* in scope for the descriptors crate (which avoids a
//!   `graphene-cli` dependency); the host's `render.preview_gif` /
//!   `render.export_gif` are the canonical re-render path. (T7.3)
//! - `recipes-lint --root <dir> [--strict]`: emit every invariant violation
//!   as a structured issue; exit non-zero under `--strict` iff any issue has
//!   level `Error`. (T7.4)

use clap::{Parser, Subcommand};
use graphite_agent_descriptors::{inventory, node_catalog_json, recipes};
use std::error::Error;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[clap(name = "graphite-agent-descriptors", version)]
struct App {
	#[clap(subcommand)]
	command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
	/// Write the generated node + allowlist inventory as JSON.
	Inventory {
		/// Output path (for example `agent/inventory.json`).
		#[clap(long)]
		out: PathBuf,
	},
	/// Print node-type and allowlist coverage.
	Coverage,
	/// Validate every recipe under --root and write the committed recipes catalog.
	Recipes {
		/// Recipe root (for example `agent/recipes/`).
		#[clap(long)]
		root: PathBuf,
		/// Output path (for example `agent/recipes.json`).
		#[clap(long)]
		out: PathBuf,
	},
	/// Re-stamp every recipe's sha256 pins by hashing on-disk `source.gdd` and
	/// `asset.<gif|png>`. Asset re-rendering is the host's job.
	RecipesBuild {
		/// Recipe root (for example `agent/recipes/`).
		#[clap(long)]
		root: PathBuf,
	},
	/// Lint every recipe under --root and emit a structured issue list. Exits
	/// non-zero under `--strict` iff any issue has level `Error`.
	RecipesLint {
		/// Recipe root (for example `agent/recipes/`).
		#[clap(long)]
		root: PathBuf,
		/// Exit non-zero iff any issue has level `Error`. Without this flag,
		/// warnings do not affect the exit code.
		#[clap(long)]
		strict: bool,
	},
}

fn main() -> Result<(), Box<dyn Error>> {
	let app = App::parse();

	match app.command {
		Command::Inventory { out } => {
			inventory::write_inventory(&out)?;
			let catalog = node_catalog_json();
			let node_count = catalog.get("count").and_then(serde_json::Value::as_u64).unwrap_or(0);
			eprintln!("wrote {} node descriptors to {}", node_count, out.display());
		}
		Command::Coverage => {
			println!("{}", serde_json::to_string_pretty(&inventory::coverage_json())?);
		}
		Command::Recipes { root, out } => {
			let recipe_list = recipes::load_all_recipes(&root)?;
			recipes::write_recipes_catalog(&out, &recipe_list)?;
			eprintln!("wrote {} recipes to {}", recipe_list.len(), out.display());
		}
		Command::RecipesBuild { root } => {
			let recipe_list = recipes::load_all_recipes(&root)?;
			let stamped = recipes::stamp_all(&root, &recipe_list)?;
			eprintln!("stamped {stamped} recipe{} under {}", if stamped == 1 { "" } else { "s" }, root.display());
		}
		Command::RecipesLint { root, strict } => {
			let recipe_list = recipes::load_all_recipes(&root)?;
			let issues = recipes::lint_recipes(&recipe_list, &root);
			println!("{}", serde_json::to_string_pretty(&recipes::issues_json(&recipe_list, &issues))?);
			let has_errors = issues.iter().any(|issue| matches!(issue.level, recipes::IssueLevel::Error));
			if strict && has_errors {
				std::process::exit(2);
			}
		}
	}

	Ok(())
}
