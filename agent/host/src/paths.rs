//! INV-12 path confinement.
//!
//! Every file-path tool argument is resolved against a configured root and
//! prefix-checked before the host touches the filesystem. The check is lexical
//! (it must work for output paths that do not exist yet), so it cannot be a
//! `fs::canonicalize` of the candidate; the root itself is canonicalized once at
//! construction.

use graphite_agent_protocol::ToolError;
use std::path::{Component, Path, PathBuf};

/// A confined root directory. Cloning is cheap (`Arc` it at the call sites that need it).
#[derive(Clone, Debug)]
pub struct PathRoot {
	root: PathBuf,
}

impl PathRoot {
	/// Canonicalize the configured root. Falls back to an absolutized, lexically
	/// normalized root when the directory does not exist yet or cannot be
	/// canonicalized, so `Host::new` (whose signature per the plan returns `Host`,
	/// not a `Result`) never fails here; a later write simply fails with an IO error.
	pub fn new(root: &Path) -> Self {
		let root = std::fs::canonicalize(root).unwrap_or_else(|_| {
			let absolute = if root.is_absolute() {
				root.to_path_buf()
			} else {
				std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")).join(root)
			};
			normalize(&absolute)
		});
		Self { root }
	}

	pub fn root(&self) -> &Path {
		&self.root
	}

	/// Resolve a tool-supplied path against the root, rejecting anything that
	/// lexically escapes it (INV-12).
	pub fn resolve(&self, requested: &str) -> Result<PathBuf, ToolError> {
		if requested.is_empty() {
			return Err(ToolError::InvalidArguments {
				message: "path must not be empty".to_string(),
			});
		}

		let requested_path = Path::new(requested);
		let joined = if requested_path.is_absolute() {
			requested_path.to_path_buf()
		} else {
			self.root.join(requested_path)
		};
		let normalized = normalize(&joined);

		if normalized == self.root || normalized.starts_with(&self.root) {
			Ok(normalized)
		} else {
			Err(ToolError::PathOutsideRoot { path: requested.to_string() })
		}
	}

	/// A generated output path directly under the root, for tools that allow the
	/// caller to omit a path (`generated_document_path`, T2.3). The stem is
	/// sanitized so it cannot introduce separators or traversal.
	pub fn generated(&self, stem: &str, extension: &str) -> PathBuf {
		let sanitized: String = stem
			.chars()
			.map(|character| {
				if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
					character
				} else {
					'-'
				}
			})
			.collect();
		let sanitized = sanitized.trim_matches('-');
		let sanitized = if sanitized.is_empty() { "document" } else { sanitized };
		self.root.join(format!("{sanitized}.{extension}"))
	}
}

/// Lexically normalize a path: collapse `.` and `..` without touching the
/// filesystem. Leading `..` components that would escape an absolute path are
/// preserved (they make the prefix check fail, which is the point).
pub fn normalize(path: &Path) -> PathBuf {
	let mut normalized = PathBuf::new();
	for component in path.components() {
		match component {
			Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
			Component::RootDir => normalized.push(Component::RootDir.as_os_str()),
			Component::CurDir => {}
			Component::ParentDir => {
				if !normalized.pop() {
					normalized.push("..");
				}
			}
			Component::Normal(part) => normalized.push(part),
		}
	}
	normalized
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn lexical_normalization_collapses_dot_segments() {
		assert_eq!(normalize(Path::new("/a/b/./c/../d")), PathBuf::from("/a/b/d"));
		assert_eq!(normalize(Path::new("/a/../b")), PathBuf::from("/b"));
	}

	#[test]
	fn relative_paths_resolve_inside_the_root() {
		let root = std::env::temp_dir();
		let paths = PathRoot::new(&root);
		let resolved = paths.resolve("nested/./out/../image.png").expect("inside root");
		assert!(resolved.starts_with(paths.root()));
		assert_eq!(resolved, paths.root().join("nested/image.png"));
	}

	#[test]
	fn traversal_is_rejected() {
		let root = std::env::temp_dir().join("graphite-agent-paths-test");
		let paths = PathRoot::new(&root);
		match paths.resolve("../escape.png") {
			Err(ToolError::PathOutsideRoot { path }) => assert_eq!(path, "../escape.png"),
			other => panic!("expected PathOutsideRoot, got {other:?}"),
		}
	}

	#[test]
	fn absolute_paths_outside_the_root_are_rejected() {
		let root = std::env::temp_dir().join("graphite-agent-paths-test");
		let paths = PathRoot::new(&root);
		assert!(matches!(paths.resolve("/etc/passwd"), Err(ToolError::PathOutsideRoot { .. })));
	}

	#[test]
	fn generated_names_are_sanitized() {
		let root = std::env::temp_dir().join("graphite-agent-paths-test");
		let paths = PathRoot::new(&root);
		let generated = paths.generated("../../evil name", "gdd");
		assert!(generated.starts_with(paths.root()));
		assert_eq!(generated.file_name().and_then(|name| name.to_str()), Some("evil-name.gdd"));
	}
}
