//! Root-level JSON Schema combinators (E-20).
//!
//! Claude Code flattens a root-level `anyOf`/`oneOf`/`allOf` before sending a tool
//! to the Claude API, because the API rejects those keywords at the schema root.
//! Flattening turns `oneOf` into a description hint and stops enforcing the
//! branches, so the catalog must never rely on a root combinator.
//!
//! Every generated schema is built as an object literal with `properties`, so this
//! test is a guard: it fails the moment a schema generator starts emitting a
//! combinator at the root, which would silently weaken tool inputs.

use graphite_agent_descriptors::node_descriptors_with_identifiers;
use serde_json::Value;

const COMBINATORS: [&str; 3] = ["anyOf", "oneOf", "allOf"];

fn root_problems(what: &str, schema: &Value, problems: &mut Vec<String>) {
	if schema.get("type").and_then(Value::as_str) != Some("object") {
		problems.push(format!("{what}: root `type` is not `object`"));
	}
	for combinator in COMBINATORS {
		if schema.get(combinator).is_some() {
			problems.push(format!("{what}: root-level `{combinator}`"));
		}
	}
}

#[test]
fn generated_node_schemas_have_no_root_combinator() {
	let nodes = node_descriptors_with_identifiers();
	assert!(!nodes.is_empty(), "the node catalog is empty: the node crates are not linked (see agent/README.md)");

	let mut problems = Vec::new();
	for (identifier, descriptor) in &nodes {
		root_problems(&format!("{identifier} input"), &descriptor.input_schema, &mut problems);
		root_problems(&format!("{identifier} output"), &descriptor.output_schema, &mut problems);
	}

	assert!(problems.is_empty(), "{} generated schema(s) would be flattened by a host:\n{}", problems.len(), problems.join("\n"));
}
