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

/// A host's own copy of its camera's last lit image, in place of ours.
static HOST_HISTORY: std::sync::RwLock<Option<Handle<Image>>> = std::sync::RwLock::new(None);

/// The camera's last lit image, which the materials bind.
pub fn history() -> Handle<Image> {
    if let Some(h) = HOST_HISTORY.read().ok().and_then(|h| h.clone()) {
        return h;
    }
    HISTORY.get().cloned().unwrap_or_default()
}

/// A host that already keeps its camera's last lit image (as this module
/// does: the frame before post-processing, the camera's own size and
/// format) hands it over: the materials read it and this module copies
/// nothing. `None` goes back to this module's own copy.
pub fn set_host_history(image: Option<Handle<Image>>) {
    if let Ok(mut h) = HOST_HISTORY.write() {
        *h = image;
    }
}

fn host_history() -> bool {
    HOST_HISTORY.read().is_ok_and(|h| h.is_some())
}

/// Our copy of the frame worked (the materials march only then: a blank
/// history would darken every reflection).
static HISTORY_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The knobs as the materials' uniform: x on (0/1) times strength, y the
/// roughness cutoff, z the reach (m), w 1 once there's a frame to reflect.
pub fn params() -> Vec4 {
    let k = crate::knobs::get;
    let on = k("ssr.enabled", 1.0) >= 0.5;
    let ready = host_history() || HISTORY_OK.load(std::sync::atomic::Ordering::Relaxed);
    Vec4::new(if on { k("ssr.strength", 1.0).clamp(0.0, 2.0) } else { 0.0 }, k("ssr.roughness", 0.45).clamp(0.02, 1.0), k("ssr.distance", 30.0).clamp(2.0, 200.0), ready as u32 as f32)
}

/// The camera whose image is kept.
#[derive(Component, Clone, ExtractComponent)]
pub struct SsrSource;

/// The look's camera keeps its image, and gets a depth prepass to march.
fn mark_camera(mut commands: Commands, cameras: Query<(Entity, Has<SsrSource>, Has<DepthPrepass>), With<crate::AtmosCamera>>) {
    if crate::knobs::get("ssr.enabled", 1.0) < 0.5 {
        return;
    }
    let host = host_history();
    for (e, source, depth) in &cameras {
        // (A host's own history: no copy of ours.)
        if host && source {
            commands.entity(e).remove::<SsrSource>();
        }
        if !source && !host {
            commands.entity(e).insert(SsrSource);
        }
        if !depth {
            commands.entity(e).insert(DepthPrepass);
        }
    }
}

/// The history the size and format of the camera's image (the copy needs
/// them equal): HDR's half floats, else the usual 8-bit sRGB.
fn size_history(cameras: Query<(&Camera, Has<bevy::camera::Hdr>), With<SsrSource>>, mut images: ResMut<Assets<Image>>) {
    let (Some((camera, hdr)), Some(handle)) = (cameras.iter().next(), HISTORY.get()) else { return };
    let Some(size) = camera.physical_target_size() else { return };
    let size = Extent3d { width: size.x.max(1), height: size.y.max(1), depth_or_array_layers: 1 };
    let format = if hdr { TextureFormat::Rgba16Float } else { TextureFormat::Rgba8UnormSrgb };
    if images.get(handle).is_some_and(|i| i.texture_descriptor.size != size || i.texture_descriptor.format != format) {
        if let Some(mut image) = images.get_mut(handle) {
            image.texture_descriptor.format = format;
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
        // (Once, say why reflections of the scene are missing.)
        if !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            warn!(
                "bevy_atmos: screen-space reflections off: the camera's image ({:?}, {:?}) and the copy ({:?}, {:?}) differ; they match next frame if the camera just changed",
                source.size(),
                source.format(),
                history.texture.size(),
                history.texture.format()
            );
        }
        HISTORY_OK.store(false, std::sync::atomic::Ordering::Relaxed);
        return;
    }
    ctx.command_encoder().copy_texture_to_texture(source.as_image_copy(), history.texture.as_image_copy(), source.size());
    if !HISTORY_OK.swap(true, std::sync::atomic::Ordering::Relaxed) {
        info!("bevy_atmos: screen-space reflections on ({:?}, {:?})", source.size(), source.format());
    }
}
