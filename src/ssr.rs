//! Screen-space reflections for the look's wet surfaces, puddles and
//! water. Bevy's own SSR needs the deferred renderer; this works forward:
//! the [`crate::AtmosCamera`]'s lit image (before post-processing) is
//! copied after its transparent pass each frame ([`history`]), and the
//! materials march the reflected ray through the depth prepass
//! (`ssr.wgsl`, `bevy_atmos::ssr`), taking that image's colour where the
//! ray passes behind something. A reflection is a frame old. Where the ray
//! leaves the screen or finds nothing, each material falls back on its own
//! (the sky for water, the light probe for wet ground).
//!
//! Knobs: `ssr.enabled` (1), `ssr.strength` (1), `ssr.roughness` (0.45:
//! rougher surfaces don't take it), `ssr.distance` (30 m: the longest ray).

use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::{Extent3d, TextureFormat};
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;
use bevy::render::view::ViewTarget;
use std::sync::OnceLock;

pub struct SsrPlugin;

impl Plugin for SsrPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "ssr.wgsl");
        let image = app.world_mut().resource_mut::<Assets<Image>>().add(Image::new_target_texture(1280, 720, TextureFormat::Rgba16Float, None));
        HISTORY.set(image).ok();
        app.add_plugins(ExtractComponentPlugin::<SsrSource>::default()).add_systems(Update, (mark_camera, size_history).after(crate::LookSet));
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::core_pipeline::core_3d::main_transparent_pass_3d;
            use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
            render_app.add_systems(Core3d, copy_history.in_set(Core3dSystems::MainPass).after(main_transparent_pass_3d));
        }
    }
}

static HISTORY: OnceLock<Handle<Image>> = OnceLock::new();

/// The camera's last lit image, which the materials bind.
pub fn history() -> Handle<Image> {
    HISTORY.get().cloned().unwrap_or_default()
}

/// The knobs as the materials' uniform: x on (0/1) times strength, y the
/// roughness cutoff, z the reach (m), w -.
pub fn params() -> Vec4 {
    let k = crate::knobs::get;
    let on = k("ssr.enabled", 1.0) >= 0.5;
    Vec4::new(if on { k("ssr.strength", 1.0).clamp(0.0, 2.0) } else { 0.0 }, k("ssr.roughness", 0.45).clamp(0.02, 1.0), k("ssr.distance", 30.0).clamp(2.0, 200.0), 0.0)
}

/// The camera whose image is kept.
#[derive(Component, Clone, ExtractComponent)]
pub struct SsrSource;

/// The look's camera keeps its image, and gets a depth prepass to march.
fn mark_camera(mut commands: Commands, cameras: Query<(Entity, Has<SsrSource>, Has<DepthPrepass>), With<crate::AtmosCamera>>) {
    if crate::knobs::get("ssr.enabled", 1.0) < 0.5 {
        return;
    }
    for (e, source, depth) in &cameras {
        if !source {
            commands.entity(e).insert(SsrSource);
        }
        if !depth {
            commands.entity(e).insert(DepthPrepass);
        }
    }
}

/// The history the size of the camera's image (the copy needs them equal).
fn size_history(cameras: Query<&Camera, With<SsrSource>>, mut images: ResMut<Assets<Image>>) {
    let (Some(size), Some(handle)) = (cameras.iter().next().and_then(|c| c.physical_target_size()), HISTORY.get()) else { return };
    let size = Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 };
    if images.get(handle).is_some_and(|i| i.texture_descriptor.size != size) {
        if let Some(mut image) = images.get_mut(handle) {
            image.resize(size);
        }
    }
}

/// After the camera's transparent pass: its image into the history.
fn copy_history(view: ViewQuery<&ViewTarget, With<SsrSource>>, images: Res<RenderAssets<GpuImage>>, mut ctx: RenderContext) {
    let target = view.into_inner();
    let Some(history) = HISTORY.get().and_then(|h| images.get(h)) else { return };
    let source = target.main_texture();
    if source.size() != history.texture.size() || source.format() != history.texture.format() {
        return;
    }
    ctx.command_encoder().copy_texture_to_texture(source.as_image_copy(), history.texture.as_image_copy(), source.size());
}
