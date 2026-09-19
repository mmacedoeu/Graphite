//! Rendering tools (T2.7).
//!
//! Both tools obtain the document bytes through `BridgeQuery::Document(ExportGdd)`
//! (INV-15) and then call the single public headless render entry point,
//! `graphene_cli::engine::render_gdd_to_png` (T0.3, R9-M3). Output paths are
//! confined to `--root` (INV-12).
//!
//! Two animation-aware tools (the recipes-and-archetypes layer) layer on top of the
//! same headless engine: `render.preview_gif` and `render.export_gif`, which call
//! `graphene_cli::engine::render_gdd_to_gif_bytes` and advance animation time per
//! frame via `RenderConfig.time.animation_time`.

use super::{descriptor, optional_f64, optional_string, optional_u32, query, required_string, required_u64, schema_object};
use crate::modules::document::decode_gdd;
use crate::paths::PathRoot;
use base64::Engine as _;
use graphite_agent_protocol::{BridgeQuery, Capability, DocumentOperation, EditorBridge, ToolCall, ToolDescriptor, ToolError, ToolModule};
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

/// The default bound (in each axis) for `render.preview`.
const DEFAULT_MAX_DIMENSION: u32 = 512;
/// The nominal side length `scale` multiplies for `render.export`.
const EXPORT_BASE_DIMENSION: f64 = 1024.0;

/// Defaults for the `*_gif` tools. `fps` defaults to 30 (matching
/// `graphene_cli`'s `--fps 30`); `frames` defaults to 60 (two seconds at 30 fps)
/// which keeps the encoded GIF under the `anthropic/maxResultSizeChars` ceiling
/// for any reasonable palette.
const DEFAULT_GIF_FPS: f64 = 30.0;
const DEFAULT_GIF_FRAMES: u32 = 60;

#[derive(Debug)]
pub struct RenderModule {
	paths: Arc<PathRoot>,
}

impl RenderModule {
	pub fn new(paths: Arc<PathRoot>) -> Self {
		Self { paths }
	}

	async fn gdd_bytes(bridge: &mut dyn EditorBridge, id: u64, document: u64) -> Result<Vec<u8>, ToolError> {
		let exported = query(
			bridge,
			id,
			BridgeQuery::Document {
				operation: DocumentOperation::ExportGdd { document },
			},
		)
		.await?;
		decode_gdd(&exported)
	}
}

impl ToolModule for RenderModule {
	fn descriptors(&self) -> Vec<ToolDescriptor> {
		vec![
			descriptor(
				"render.preview",
				"Render a document to a base64 PNG preview bounded by `max_dimension`.",
				Capability::Execute,
				schema_object(
					&["document_id"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"max_dimension": { "type": "integer", "minimum": 1 }
					}),
				),
				schema_object(&["image_base64_png"], json!({ "image_base64_png": { "type": "string" } })),
			),
			descriptor(
				"render.preview_gif",
				"Render a document to a base64 animated GIF preview at `fps` for `frames`, bounded by `max_dimension`.",
				Capability::Execute,
				schema_object(
					&["document_id"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"max_dimension": { "type": "integer", "minimum": 1 },
						"fps": { "type": "number", "minimum": 1 },
						"frames": { "type": "integer", "minimum": 1 }
					}),
				),
				schema_object(
					&["image_base64_gif", "fps", "frames", "byte_size"],
					json!({
						"image_base64_gif": { "type": "string" },
						"fps": { "type": "number" },
						"frames": { "type": "integer", "minimum": 1 },
						"byte_size": { "type": "integer", "minimum": 0 }
					}),
				),
			),
			descriptor(
				"render.export",
				"Render a document and write the image under the configured root.",
				Capability::Export,
				schema_object(
					&["document_id", "path"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"path": { "type": "string" },
						"format": { "type": "string", "enum": ["png"] },
						"scale": { "type": "number", "minimum": 0 }
					}),
				),
				schema_object(&["path"], json!({ "path": { "type": "string" } })),
			),
			descriptor(
				"render.export_gif",
				"Render a document to an animated GIF and write it under the configured root at `fps` for `frames`.",
				Capability::Export,
				schema_object(
					&["document_id", "path"],
					json!({
						"document_id": { "type": "integer", "minimum": 0 },
						"path": { "type": "string" },
						"fps": { "type": "number", "minimum": 1 },
						"frames": { "type": "integer", "minimum": 1 },
						"max_dimension": { "type": "integer", "minimum": 1 }
					}),
				),
				schema_object(&["path"], json!({ "path": { "type": "string" } })),
			),
		]
	}

	fn execute<'a>(&'a mut self, call: ToolCall, bridge: &'a mut dyn EditorBridge) -> Pin<Box<dyn Future<Output = Result<Value, ToolError>> + 'a>> {
		Box::pin(async move {
			let document = required_u64(&call, "document_id")?;
			match call.name.as_str() {
				"render.preview" => {
					let max_dimension = optional_u32(&call, "max_dimension", DEFAULT_MAX_DIMENSION)?;
					let bytes = Self::gdd_bytes(bridge, call.id, document).await?;
					let png = render_png(bytes, max_dimension).await?;
					Ok(json!({ "image_base64_png": base64::engine::general_purpose::STANDARD.encode(png) }))
				}
				"render.preview_gif" => {
					let max_dimension = optional_u32(&call, "max_dimension", DEFAULT_MAX_DIMENSION)?;
					let fps = optional_f64(&call, "fps", DEFAULT_GIF_FPS)?;
					let frames = optional_u32(&call, "frames", DEFAULT_GIF_FRAMES)?;
					let bytes = Self::gdd_bytes(bridge, call.id, document).await?;
					let gif = render_gif_bytes(bytes, fps, frames, max_dimension).await?;
					Ok(json!({
						"image_base64_gif": base64::engine::general_purpose::STANDARD.encode(&gif),
						"fps": fps,
						"frames": frames,
						"byte_size": gif.len(),
					}))
				}
				"render.export" => {
					let requested = required_string(&call, "path")?;
					// INV-12: reject an escaping path before doing any expensive work.
					let target = self.paths.resolve(&requested)?;
					let format = optional_string(&call, "format")?.unwrap_or_else(|| "png".to_string());
					if format != "png" {
						return Err(ToolError::InvalidArguments {
							message: format!("render.export format `{format}` is not supported in Phase 2 (only `png`)"),
						});
					}
					let scale = optional_f64(&call, "scale", 1.0)?;
					let max_dimension = (scale * EXPORT_BASE_DIMENSION).round().clamp(1.0, u32::MAX as f64) as u32;
					let bytes = Self::gdd_bytes(bridge, call.id, document).await?;
					let png = render_png(bytes, max_dimension).await?;
					if let Some(parent) = target.parent() {
						std::fs::create_dir_all(parent).map_err(|error| ToolError::Internal {
							message: format!("failed to create {}: {error}", parent.display()),
						})?;
					}
					std::fs::write(&target, png).map_err(|error| ToolError::Internal {
						message: format!("failed to write {}: {error}", target.display()),
					})?;
					Ok(json!({ "path": target, "document_id": document }))
				}
				"render.export_gif" => {
					let requested = required_string(&call, "path")?;
					// INV-12: reject an escaping path before doing any expensive work.
					let target = self.paths.resolve(&requested)?;
					let fps = optional_f64(&call, "fps", DEFAULT_GIF_FPS)?;
					let frames = optional_u32(&call, "frames", DEFAULT_GIF_FRAMES)?;
					let max_dimension = optional_u32(&call, "max_dimension", DEFAULT_MAX_DIMENSION)?;
					let bytes = Self::gdd_bytes(bridge, call.id, document).await?;
					let gif = render_gif_bytes(bytes, fps, frames, max_dimension).await?;
					if let Some(parent) = target.parent() {
						std::fs::create_dir_all(parent).map_err(|error| ToolError::Internal {
							message: format!("failed to create {}: {error}", parent.display()),
						})?;
					}
					std::fs::write(&target, gif).map_err(|error| ToolError::Internal {
						message: format!("failed to write {}: {error}", target.display()),
					})?;
					Ok(json!({ "path": target, "document_id": document }))
				}
				other => Err(ToolError::NotFound { what: format!("tool {other}") }),
			}
		})
	}
}

async fn render_png(gdd_bytes: Vec<u8>, max_dimension: u32) -> Result<Vec<u8>, ToolError> {
	graphene_cli::engine::render_gdd_to_png(&gdd_bytes, max_dimension).await.map_err(|error| ToolError::Internal {
		message: format!("render failed: {error}"),
	})
}

async fn render_gif_bytes(gdd_bytes: Vec<u8>, fps: f64, frames: u32, max_dimension: u32) -> Result<Vec<u8>, ToolError> {
	graphene_cli::engine::render_gdd_to_gif_bytes(&gdd_bytes, fps, frames, max_dimension)
		.await
		.map_err(|error| ToolError::Internal {
			message: format!("render failed: {error}"),
		})
}
