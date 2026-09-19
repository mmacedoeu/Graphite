//! Process-isolation test for INV-11: `Editor::new` installs a process-global
//! `ENVIRONMENT` and panics on a second call.
//!
//! This lives in its own integration-test binary (a separate process from the unit
//! tests) so it can actually construct the first `Editor`.

use graph_craft::application_io::PlatformApplicationIo;
use graph_craft::application_io::resource::HashMapResourceStorage;
use graphite_editor::application::{Editor, Environment, Host, Platform};
use graphite_editor::messages::future::Wake;
use std::sync::Arc;

fn environment() -> Environment {
	Environment {
		platform: Platform::Desktop,
		host: Host::Linux,
	}
}

fn new_editor() -> Editor {
	Editor::new(
		environment(),
		0,
		Arc::new(HashMapResourceStorage::new()),
		None,
		PlatformApplicationIo::default(),
		Arc::new(|| {}) as Wake,
	)
}

#[test]
fn exactly_one_editor_per_process() {
	let _first = new_editor();

	// The panic message is expected; silence the default hook so the test output stays readable.
	let previous_hook = std::panic::take_hook();
	std::panic::set_hook(Box::new(|_| {}));
	let second = std::panic::catch_unwind(new_editor);
	std::panic::set_hook(previous_hook);

	assert!(second.is_err(), "a second `Editor::new` must panic (INV-11)");
}
