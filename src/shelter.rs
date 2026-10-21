//! Where rain can't fall, worked out from the scene itself: with no host
//! [`crate::weather::Shelter`], while it rains or snows, a camera looks
//! straight down over the [`crate::AtmosCamera`] for a few frames and its
//! depth (the highest surface over each half-metre cell, roofs, awnings,
//! trees) becomes the shelter map, the modern water's surfaces merged in.
//! It's taken again when the camera has moved a quarter of the way to its
//! edge (128 m each way of the camera), so rain doesn't fall indoors, under
//! arches or awnings without any host code.
//!
//! Knobs: `rain.shelter` (1; 0 none of ours: rain everywhere but on the
//! water), `rain.shelter_height` (60 m: how far over the camera the capture
//! starts; higher roofs than that don't count).

use bevy::camera::{ClearColorConfig, RenderTarget, ScalingMode};
use bevy::core_pipeline::prepass::{DepthPrepass, ViewPrepassTextures};
use bevy::prelude::*;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{texture_2d, texture_depth_2d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, ViewQuery};
use bevy::render::texture::GpuImage;

pub struct ShelterPlugin;

impl Plugin for ShelterPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "shelter.wgsl");
        app.add_plugins(ExtractComponentPlugin::<ShelterCamera>::default()).add_systems(Update, capture.after(crate::LookSet));
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            use bevy::core_pipeline::core_3d::main_opaque_pass_3d;
            use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
            render_app
                .add_systems(bevy::render::RenderStartup, init_pipeline)
                .add_systems(Core3d, write_heights.in_set(Core3dSystems::MainPass).after(main_opaque_pass_3d));
        }
    }
}

/// Cells a side and their size (m): the map covers 256 m.
const CELLS: u32 = 512;
const CELL: f32 = 0.5;
/// Frames the capture camera stays (its first may not have drawn yet).
const FRAMES: u32 = 4;

/// The camera taking the capture, and what its depth is turned into.
#[derive(Component, Clone, ExtractComponent)]
pub struct ShelterCamera {
    /// Into these heights (the map; R32Float).
    heights: Handle<Image>,
    /// The modern water's heights (the same layout), merged in.
    water: Handle<Image>,
    /// The capture's first cell (x, z), the camera's height, its far plane.
    origin: IVec2,
    top: f32,
    far: f32,
}

/// Ours, while it's the shelter in use.
#[derive(Resource)]
struct GeoShelter {
    heights: Handle<Image>,
    /// Where the last capture was centred (cells).
    centre: Option<IVec2>,
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn capture(
    mut commands: Commands,
    weather: Res<crate::weather::Weather>,
    shelter: Option<Res<crate::weather::Shelter>>,
    geo: Option<ResMut<GeoShelter>>,
    water: Option<Res<crate::modern_water::WaterShelter>>,
    eye: Query<&GlobalTransform, With<crate::AtmosCamera>>,
    mut cams: Query<(Entity, &mut ShelterCamera)>,
    mut frames: Local<u32>,
    mut images: ResMut<Assets<Image>>,
) {
    // The capture camera's frames, then it goes.
    if let Some((e, mut cam)) = cams.iter_mut().next() {
        *frames += 1;
        // (The water's heights may have come since.)
        if let Some(w) = &water {
            if cam.water != w.image {
                cam.water = w.image.clone();
            }
        }
        if *frames >= FRAMES {
            // (Debug: `ATMOS_SHELTER_PNG=<file>` saves the map, white the
            // capture's height, black 80 m below it or open.)
            if let Some(path) = std::env::var_os("ATMOS_SHELTER_PNG") {
                let top = cam.top;
                commands.spawn(bevy::render::gpu_readback::Readback::texture(cam.heights.clone())).observe(
                    move |ev: On<bevy::render::gpu_readback::ReadbackComplete>, mut commands: Commands| {
                        let bytes: Vec<u8> = ev.data.chunks_exact(4).map(|c| (((f32::from_le_bytes([c[0], c[1], c[2], c[3]]) - (top - 80.0)) / 80.0).clamp(0.0, 1.0) * 255.0) as u8).collect();
                        let img = Image::new(Extent3d { width: CELLS, height: CELLS, depth_or_array_layers: 1 }, TextureDimension::D2, bytes, TextureFormat::R8Unorm, bevy::asset::RenderAssetUsages::MAIN_WORLD);
                        if let Ok(d) = img.try_into_dynamic() {
                            let _ = d.save(&path);
                            info!("bevy_atmos: shelter map saved to {:?}", path);
                        }
                        commands.entity(ev.entity).despawn();
                    },
                );
            }
            commands.entity(e).despawn();
        }
        return;
    }
    let wanted = crate::knobs::get("rain.shelter", 1.0) >= 0.5 && (weather.rain > 0.0 || weather.snow > 0.0);
    // (A host's own shelter wins.)
    let host = shelter.is_some() && geo.as_ref().is_none_or(|g| shelter.as_ref().is_some_and(|s| s.heights != g.heights));
    if host || !wanted {
        if let Some(g) = &geo {
            if !host && shelter.as_ref().is_some_and(|s| s.heights == g.heights) {
                commands.remove_resource::<crate::weather::Shelter>();
            }
            commands.remove_resource::<GeoShelter>();
        }
        return;
    }
    let Some(at) = eye.iter().next().map(|g| g.translation()) else { return };
    let here = (Vec2::new(at.x, at.z) / CELL).floor().as_ivec2();
    let mut geo = match geo {
        Some(g) => g,
        None => {
            let mut image = Image::new_target_texture(CELLS, CELLS, TextureFormat::R32Float, None);
            image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
            image.data = Some((0..CELLS * CELLS).flat_map(|_| (-1.0e6f32).to_le_bytes()).collect());
            let heights = images.add(image);
            commands.insert_resource(crate::weather::Shelter { heights: heights.clone(), cell: CELL, cells: CELLS });
            commands.insert_resource(GeoShelter { heights, centre: None });
            return;
        }
    };
    // Again once the camera's a quarter of the way to the map's edge.
    if geo.centre.is_some_and(|c| (c - here).abs().max_element() < CELLS as i32 / 4) {
        return;
    }
    geo.centre = Some(here);
    let half = CELLS as i32 / 2;
    let origin = here - IVec2::splat(half);
    let top = at.y + crate::knobs::get("rain.shelter_height", 60.0).clamp(5.0, 500.0);
    let far = 1000.0;
    let centre = (origin.as_vec2() + Vec2::splat(half as f32)) * CELL;
    // (Its colour isn't used: a small target of its own.)
    let color = images.add(Image::new_target_texture(CELLS, CELLS, TextureFormat::Rgba8Unorm, None));
    let water = water.map_or(Handle::default(), |w| w.image.clone());
    *frames = 0;
    commands.spawn((
        Name::new("atmos shelter capture"),
        Camera3d::default(),
        Camera { order: -50, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::Image(color.into()),
        Projection::Orthographic(OrthographicProjection {
            near: 0.0,
            far,
            scaling_mode: ScalingMode::Fixed { width: CELLS as f32 * CELL, height: CELLS as f32 * CELL },
            ..OrthographicProjection::default_3d()
        }),
        // (Looking down, screen right +x, screen up -z: the image's rows
        // run with z.)
        Transform::from_xyz(centre.x, top, centre.y).looking_to(Vec3::NEG_Y, Vec3::NEG_Z),
        Msaa::Off,
        DepthPrepass,
        ShelterCamera { heights: geo.heights.clone(), water, origin, top, far },
    ));
}

#[derive(Resource)]
struct ShelterPipeline {
    layout: BindGroupLayoutDescriptor,
    id: CachedRenderPipelineId,
}

fn init_pipeline(mut commands: Commands, fullscreen: Res<bevy::core_pipeline::FullscreenShader>, asset_server: Res<AssetServer>, cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new(
        "atmos_shelter_layout",
        &BindGroupLayoutEntries::sequential(ShaderStages::FRAGMENT, (texture_depth_2d(), texture_2d(TextureSampleType::Float { filterable: false }), uniform_buffer_sized(false, None))),
    );
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("atmos_shelter".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: asset_server.load("embedded://bevy_atmos/shelter.wgsl"),
            targets: vec![Some(ColorTargetState { format: TextureFormat::R32Float, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(ShelterPipeline { layout, id });
}

/// The capture's depth into the shelter's heights.
fn write_heights(view: ViewQuery<(&ViewPrepassTextures, &ShelterCamera)>, pipe: Option<Res<ShelterPipeline>>, cache: Res<PipelineCache>, images: Res<RenderAssets<GpuImage>>, mut ctx: RenderContext) {
    let (prepass, cam) = view.into_inner();
    let Some(pipe) = pipe else { return };
    let Some(pipeline) = cache.get_render_pipeline(pipe.id) else { return };
    let Some(depth) = &prepass.depth else { return };
    let Some(heights) = images.get(&cam.heights) else { return };
    // (No water yet: a 1x1 open cell, Bevy's default image, read as open.)
    let water = images.get(&cam.water);
    let has_water = water.is_some_and(|w| w.texture_descriptor.size.width == CELLS);
    let Some(water) = water.or_else(|| images.get(&Handle::<Image>::default())) else { return };
    let params = [cam.origin.x as f32, cam.origin.y as f32, cam.top, cam.far, CELLS as f32, if has_water { 1.0 } else { 0.0 }, 0.0, 0.0];
    let bytes: Vec<u8> = params.iter().flat_map(|f| f.to_ne_bytes()).collect();
    let buffer = ctx.render_device().create_buffer_with_data(&BufferInitDescriptor { label: Some("atmos_shelter_params"), contents: &bytes, usage: BufferUsages::UNIFORM });
    let bind_group = ctx.render_device().create_bind_group(
        "atmos_shelter",
        &cache.get_bind_group_layout(&pipe.layout),
        &BindGroupEntries::sequential((&depth.texture.default_view, &water.texture_view, buffer.as_entire_binding())),
    );
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("atmos_shelter"),
        color_attachments: &[Some(RenderPassColorAttachment { view: &heights.texture_view, depth_slice: None, resolve_target: None, ops: Operations { load: LoadOp::Load, store: StoreOp::Store } })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_render_pipeline(pipeline);
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
}
