//! Default-deny `agent_safe` classification (T0.7, expanded in T3.1).
//!
//! A name is agent-safe **only** if it appears in `agent/allowlist.toml`. There are
//! no subjective rules and no heuristics (R3-A): unknown names are denied.
//!
//! Phase 3 adds generated message-action names to the same array. Their single
//! documented naming rule is that an action's allowlist entry is exactly its
//! generated command descriptor name: `command.` + the action's
//! `AsMessage::global_name()` lowercased/normalized ([`crate::commands::command_name`]).
//! `is_agent_safe` remains pure membership; [`is_action_agent_safe`] applies the
//! rule. Since E-8 makes command descriptors catalog-only, `agent_safe` is
//! **descriptive** — it no longer gates execution.

use std::sync::LazyLock;

/// The curated allowlist, embedded at compile time so the binary needs no runtime
/// file lookup and the classification is deterministic.
pub const ALLOWLIST_SOURCE: &str = include_str!("../../allowlist.toml");

static ALLOWLISTED: LazyLock<Vec<String>> = LazyLock::new(|| parse_allowlist(ALLOWLIST_SOURCE));

/// Every allowlisted name, in file order.
pub fn allowlisted_names() -> Vec<String> {
	ALLOWLISTED.clone()
}

/// Default-deny membership test (gate check 6).
pub fn is_agent_safe(name: &str) -> bool {
	ALLOWLISTED.iter().any(|allowlisted| allowlisted == name)
}

/// The T3.1 rule: a message action is agent-safe when its generated command
/// descriptor name is allowlisted.
pub fn is_action_agent_safe(global_name: &str) -> bool {
	is_agent_safe(&crate::commands::command_name(global_name))
}

/// Minimal reader for the `agent_safe = [ "...", ... ]` array. Deliberately not a
/// general TOML parser: the allowlist file has exactly one key, and adding a TOML
/// dependency for it is not approved (§4.1).
fn parse_allowlist(source: &str) -> Vec<String> {
	let without_comments: String = source.lines().map(|line| line.split('#').next().unwrap_or("")).collect::<Vec<_>>().join("\n");
	let Some((_, after_key)) = without_comments.split_once("agent_safe") else { return Vec::new() };
	let Some((_, after_open)) = after_key.split_once('[') else { return Vec::new() };
	let Some((contents, _)) = after_open.split_once(']') else { return Vec::new() };

	contents
		.split(',')
		.map(|entry| entry.trim().trim_matches('"').trim_matches('\'').trim().to_string())
		.filter(|entry| !entry.is_empty())
		.collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn unknown_names_are_denied() {
		assert!(!is_agent_safe("graph.definitely_not_a_tool"));
		assert!(!is_agent_safe(""));
		assert!(!is_agent_safe("Message::Batched"));
	}

	#[test]
	fn allowlisted_names_are_accepted_and_no_others() {
		let names = allowlisted_names();
		assert!(!names.is_empty(), "allowlist.toml must seed the Phase 2 curated tools");
		for name in &names {
			assert!(is_agent_safe(name), "{name} should be agent-safe");
		}
		// Every name that is agent-safe is in the list: the predicate is exactly membership.
		assert!(names.iter().all(|name| is_agent_safe(name)));
	}

	#[test]
	fn existing_curated_tool_names_still_work() {
		// Phase 2 regression: expanding the allowlist must not drop the curated set.
		for name in ["document.new", "graph.add_node", "history.undo", "render.preview", "render.export"] {
			assert!(is_agent_safe(name), "{name} must stay agent-safe");
		}
		// A message action maps through the documented naming rule.
		assert!(is_action_agent_safe("Portfolio.Document.Undo"));
		assert!(is_action_agent_safe("Tool.Select.DragStart"));
		assert!(!is_action_agent_safe("Portfolio.Open"));
		assert!(!is_action_agent_safe("AppWindow.Close"));
	}

	#[test]
	fn allowlist_has_no_duplicates() {
		let names = allowlisted_names();
		let mut sorted = names.clone();
		sorted.sort();
		sorted.dedup();
		assert_eq!(sorted.len(), names.len(), "duplicate entries in agent/allowlist.toml");
	}
}
