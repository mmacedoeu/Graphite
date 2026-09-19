//! Library surface for the `graphene-cli` package.
//!
//! The binary in `main.rs` is a thin CLI wrapper around these modules. Other crates (for example the
//! agent bridge) depend on this library to drive the headless engine without shelling out to the CLI.

pub mod engine;
pub mod export;
