//! Shadows: the sun's and the camera's shadow settings as knobs, saved per
//! level with the rest. Each applies to the [`crate::AtmosSun`] and
//! [`crate::AtmosCamera`] only while its knob is set; unset, the host's own
//! setup is put back as it was.
//!
//! Knobs:
//! - `shadows.contact` (0/1) and `shadows.contact_length` (m, 0.3): Bevy's
//!   screen-space contact shadows (`ContactShadows` on the camera, the
//!   sun's `contact_shadows_enabled`), for the small shadows where things
//!   meet the ground. Needs the depth prepass (added with them).
//! - `shadows.softness` (0..1): the sun's shadow edges. With the
//!   `pcss` feature, percentage-closer soft shadows (`soft_shadow_size`);
//!   without, the filter (0 hard 2x2, above that Gaussian).
//! - `shadows.distance` (m, 150) and `shadows.cascades` (1..4): how far the
//!   sun's shadows reach, in how many cascades.
//! - `shadows.strength` (0..1, 1): how dark shadows are against the sky's
//!   fill light (lower: more ambient fill, lighter shadows).
//! - `ssao.enabled` (0/1) and `ssao.quality` (0 low .. 3 ultra): Bevy's
//!   screen-space ambient occlusion on the camera (needs MSAA off).
//! - `ssao.direct` (0..1, 0.8) and `ssao.contrast` (1.6): Bevy's AO darkens
//!   only the ambient light, so in sunlit places it hardly shows; this lays
//!   it over all of the light too (under awnings, at wall bases, round
//!   props), drawn over the lit scene before the transparents. 0: Bevy's
//!   alone.

use bevy::light::{CascadeShadowConfig, CascadeShadowConfigBuilder, ShadowFilteringMethod};
use bevy::core_pipeline::prepass::{DepthPrepass, NormalPrepass};
use bevy::pbr::{ContactShadows, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_resource::binding_types::{texture_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::diagnostic::RecordDiagnostics;
use bevy::render::view::ViewTarget;
use std::collections::HashMap;
use std::sync::Mutex;

pub struct ShadowsPlugin;

impl Plugin for ShadowsPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "ao_direct.wgsl");
        app.add_plugins(ExtractComponentPlugin::<AoDirect>::default()).add_systems(Update, apply.after(crate::LookSet));
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::core_pipeline::core_3d::{main_opaque_pass_3d, main_transparent_pass_3d};
            use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
            render_app
                .add_systems(bevy::render::RenderStartup, init_ao_direct)
                .add_systems(Core3d, ao_direct.in_set(Core3dSystems::MainPass).after(main_opaque_pass_3d).before(main_transparent_pass_3d));
        }
    }
}

/// The sky's fill light against the shadows (`shadows.strength`): the
/// ambient light's multiplier (1 unset).
pub fn fill() -> f32 {
    if crate::knobs::is_set("shadows.strength") {
        1.0 + (1.0 - crate::knobs::get("shadows.strength", 1.0).clamp(0.0, 1.0)) * 3.0
    } else {
        1.0
    }
}

/// The host's own settings, kept while a knob overrides them.
#[derive(Default)]
struct Saved {
    contact: Option<(Option<ContactShadows>, bool)>,
    filter: Option<Option<ShadowFilteringMethod>>,
    #[cfg(feature = "pcss")]
    soft: Option<Option<f32>>,
    cascades: Option<Option<CascadeShadowConfig>>,
    ssao: Option<Option<ScreenSpaceAmbientOcclusion>>,
    msaa: Option<Option<Msaa>>,
}

#[allow(clippy::type_complexity)]
fn apply(
    mut commands: Commands,
    cameras: Query<(Entity, Option<&ContactShadows>, Option<&ShadowFilteringMethod>, Option<&ScreenSpaceAmbientOcclusion>, Option<&Msaa>), With<crate::AtmosCamera>>,
    mut suns: Query<(Entity, &mut DirectionalLight, Option<&CascadeShadowConfig>), With<crate::AtmosSun>>,
    mut saved: Local<Saved>,
    mut last: Local<Option<[f32; 11]>>,
) {
    let (k, set) = (crate::knobs::get, crate::knobs::is_set);
    let flag = |name: &str| if set(name) { 1.0 } else { 0.0 };
    // (Only when a knob changes: the components stay as set between.)
    let now = [
        flag("shadows.contact") + k("shadows.contact", 0.0) * 2.0,
        k("shadows.contact_length", 0.3),
        flag("shadows.softness") + k("shadows.softness", 0.5) * 2.0,
        flag("shadows.distance") + flag("shadows.cascades") * 2.0,
        k("shadows.distance", 150.0),
        k("shadows.cascades", 4.0),
        flag("ssao.enabled") + k("ssao.enabled", 0.0) * 2.0,
        k("ssao.quality", 2.0),
        cameras.iter().count() as f32 + suns.iter().count() as f32 * 10.0,
        k("ssao.direct", 0.8),
        k("ssao.contrast", 1.6),
    ];
    if *last == Some(now) {
        return;
    }
    *last = Some(now);
    let (Some((cam, contact, filter, ssao, msaa)), Some((sun, mut light, cascades))) = (cameras.iter().next(), suns.iter_mut().next()) else { return };

    // Contact shadows.
    if set("shadows.contact") {
        saved.contact.get_or_insert((contact.copied(), light.contact_shadows_enabled));
        if k("shadows.contact", 0.0) >= 0.5 {
            commands.entity(cam).try_insert(ContactShadows { length: k("shadows.contact_length", 0.3).clamp(0.02, 5.0), ..default() });
            light.contact_shadows_enabled = true;
        } else {
            commands.entity(cam).try_remove::<ContactShadows>();
            light.contact_shadows_enabled = false;
        }
    } else if let Some((was, enabled)) = saved.contact.take() {
        match was {
            Some(c) => {
                commands.entity(cam).try_insert(c);
            }
            None => {
                commands.entity(cam).try_remove::<ContactShadows>();
            }
        }
        light.contact_shadows_enabled = enabled;
    }

    // Softness.
    if set("shadows.softness") {
        let s = k("shadows.softness", 0.5).clamp(0.0, 1.0);
        saved.filter.get_or_insert(filter.copied());
        commands.entity(cam).try_insert(if s < 0.3 { ShadowFilteringMethod::Hardware2x2 } else { ShadowFilteringMethod::Gaussian });
        #[cfg(feature = "pcss")]
        {
            saved.soft.get_or_insert(light.soft_shadow_size);
            light.soft_shadow_size = (s > 0.0).then_some(s * 2.0);
        }
    } else {
        if let Some(was) = saved.filter.take() {
            match was {
                Some(f) => {
                    commands.entity(cam).try_insert(f);
                }
                None => {
                    commands.entity(cam).try_remove::<ShadowFilteringMethod>();
                }
            }
        }
        #[cfg(feature = "pcss")]
        if let Some(was) = saved.soft.take() {
            light.soft_shadow_size = was;
        }
    }

    // Distance and cascades.
    if set("shadows.distance") || set("shadows.cascades") {
        saved.cascades.get_or_insert(cascades.cloned());
        let distance = k("shadows.distance", 150.0).clamp(10.0, 2000.0);
        let config = CascadeShadowConfigBuilder {
            num_cascades: k("shadows.cascades", 4.0).round().clamp(1.0, 4.0) as usize,
            maximum_distance: distance,
            first_cascade_far_bound: (distance * 0.06).max(2.0),
            ..default()
        }
        .build();
        commands.entity(sun).try_insert(config);
    } else if let Some(was) = saved.cascades.take() {
        match was {
            Some(c) => {
                commands.entity(sun).try_insert(c);
            }
            None => {
                commands.entity(sun).try_insert(CascadeShadowConfigBuilder::default().build());
            }
        }
    }

    // Ambient occlusion: Bevy's needs the depth and normal prepasses and
    // MSAA off, or it silently does nothing: the prepasses are added, and
    // MSAA turned off while it's on (put back after).
    let restore_msaa = |saved: &mut Saved, commands: &mut Commands| {
        if let Some(was) = saved.msaa.take() {
            match was {
                Some(m) => {
                    commands.entity(cam).try_insert(m);
                }
                None => {
                    commands.entity(cam).try_remove::<Msaa>();
                }
            }
            AO_NOTE.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    };
    if set("ssao.enabled") {
        saved.ssao.get_or_insert(ssao.cloned());
        if k("ssao.enabled", 0.0) >= 0.5 {
            let quality_level = match k("ssao.quality", 2.0).round() as i32 {
                0 => ScreenSpaceAmbientOcclusionQualityLevel::Low,
                1 => ScreenSpaceAmbientOcclusionQualityLevel::Medium,
                3 => ScreenSpaceAmbientOcclusionQualityLevel::Ultra,
                _ => ScreenSpaceAmbientOcclusionQualityLevel::High,
            };
            if msaa.is_none_or(|m| *m != Msaa::Off) {
                saved.msaa.get_or_insert(msaa.copied());
                commands.entity(cam).try_insert(Msaa::Off);
                AO_NOTE.store(true, std::sync::atomic::Ordering::Relaxed);
                info!("bevy_atmos: ambient occlusion on: the camera's MSAA turned off (Bevy's AO needs it off)");
            }
            commands.entity(cam).try_insert((ScreenSpaceAmbientOcclusion { quality_level, ..default() }, DepthPrepass, NormalPrepass));
            let direct = k("ssao.direct", 0.8).clamp(0.0, 1.0);
            if direct > 0.0 {
                commands.entity(cam).try_insert(AoDirect(direct, k("ssao.contrast", 1.6).clamp(0.5, 4.0)));
            } else {
                commands.entity(cam).try_remove::<AoDirect>();
            }
        } else {
            commands.entity(cam).try_remove::<(ScreenSpaceAmbientOcclusion, AoDirect)>();
            restore_msaa(&mut saved, &mut commands);
        }
    } else if let Some(was) = saved.ssao.take() {
        commands.entity(cam).try_remove::<AoDirect>();
        match was {
            Some(s) => {
                commands.entity(cam).try_insert(s);
            }
            None => {
                commands.entity(cam).try_remove::<ScreenSpaceAmbientOcclusion>();
            }
        }
        restore_msaa(&mut saved, &mut commands);
    }
}

/// Ambient occlusion turned the camera's MSAA off (the editor says so).
static AO_NOTE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A note for the editor while ambient occlusion has the camera's MSAA off.
pub fn ao_note() -> Option<&'static str> {
    AO_NOTE.load(std::sync::atomic::Ordering::Relaxed).then_some("Ambient occlusion needs MSAA off: it's off while AO is on.")
}

/// AO over all of the light on this camera (`ssao.direct`): strength,
/// contrast.
#[derive(Component, Clone, Copy, ExtractComponent)]
pub struct AoDirect(f32, f32);

#[derive(Resource)]
struct AoDirectPipeline {
    layout: BindGroupLayoutDescriptor,
    vertex: VertexState,
    shader: Handle<Shader>,
    /// One per camera image format.
    ids: Mutex<HashMap<TextureFormat, CachedRenderPipelineId>>,
}

fn init_ao_direct(mut commands: Commands, fullscreen: Res<bevy::core_pipeline::FullscreenShader>, asset_server: Res<AssetServer>) {
    let layout = BindGroupLayoutDescriptor::new(
        "atmos_ao_direct_layout",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (texture_2d(TextureSampleType::Float { filterable: false }), uniform_buffer_sized(false, None))),
    );
    commands.insert_resource(AoDirectPipeline {
        layout,
        vertex: fullscreen.to_vertex_state(),
        shader: asset_server.load("embedded://bevy_atmos/ao_direct.wgsl"),
        ids: Mutex::new(HashMap::new()),
    });
}

/// After the opaque pass: the camera's image multiplied by the AO.
fn ao_direct(
    view: ViewQuery<(&ViewTarget, &bevy::pbr::ScreenSpaceAmbientOcclusionResources, &AoDirect)>,
    pipe: Res<AoDirectPipeline>,
    cache: Res<PipelineCache>,
    mut ctx: RenderContext,
) {
    let (target, ssao, ao) = view.into_inner();
    let format = target.main_texture_format();
    let id = *pipe.ids.lock().unwrap().entry(format).or_insert_with(|| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some("atmos_ao_direct".into()),
            layout: vec![pipe.layout.clone()],
            vertex: pipe.vertex.clone(),
            fragment: Some(FragmentState {
                shader: pipe.shader.clone(),
                targets: vec![Some(ColorTargetState {
                    format,
                    // (Multiply: the image times the shader's output.)
                    blend: Some(BlendState {
                        color: BlendComponent { src_factor: BlendFactor::Dst, dst_factor: BlendFactor::Zero, operation: BlendOperation::Add },
                        alpha: BlendComponent { src_factor: BlendFactor::Zero, dst_factor: BlendFactor::One, operation: BlendOperation::Add },
                    }),
                    write_mask: ColorWrites::ALL,
                })],
                ..default()
            }),
            ..default()
        })
    });
    let Some(pipeline) = cache.get_render_pipeline(id) else { return };
    let bytes: Vec<u8> = [ao.0, ao.1, 0.0f32, 0.0].iter().flat_map(|f| f.to_ne_bytes()).collect();
    let buffer = ctx.render_device().create_buffer_with_data(&BufferInitDescriptor { label: Some("atmos_ao_direct_params"), contents: &bytes, usage: BufferUsages::UNIFORM });
    let bind_group = ctx.render_device().create_bind_group(
        "atmos_ao_direct",
        &cache.get_bind_group_layout(&pipe.layout),
        &BindGroupEntries::sequential((&ssao.screen_space_ambient_occlusion_texture.default_view, buffer.as_entire_binding())),
    );
    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("atmos_ao_direct"),
        color_attachments: &[Some(target.get_unsampled_color_attachment())],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    let span = diagnostics.pass_span(&mut pass, "atmos_ao_direct");
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
    span.end(&mut pass);
}
