//! Modern water: replaces a level's own water. The host marks its water
//! meshes with [`AtmosWater`]; while `water.replace` is on, each one is
//! hidden and a fine grid over the top of its bounds (flat water) drawn
//! with [`ModernWaterMaterial`] in its place (off: the original is shown
//! again, untouched).
//!
//! The surface: Gerstner waves moving the grid (height, speed, the wind's
//! direction, choppiness) with layered detail waves in the normals (detail
//! scale), screen-space reflections over the look's sky by Fresnel, the
//! sun's (or moon's) glitter, colour by absorption through the water's
//! depth (from the depth prepass, which the plugin adds to the camera),
//! foam on steep wave crests and where the water is shallow (shorelines,
//! round anything crossing the surface), rain hitting it (rings and small
//! splashes; the rain stops at the surface, see [`AutoShelter`]), and fog
//! under the surface when the camera goes below it.
//!
//! Rivers: with a [`crate::flow::WaterFlow`] beside the `AtmosWater`, the
//! modern water is drawn on the original mesh itself (sloped water follows
//! it, no flat grid, no Gerstner waves) and flows (`crate::flow`).
//!
//! Knobs (all saved with the look): `water.replace` (0 keep the original,
//! 1 replace), `water.tint`, `water.brightness`, `water.absorption`,
//! `water.wave_height`, `water.wave_speed`, `water.choppiness`,
//! `water.detail`, `water.reflection`, `water.glitter`, `water.fresnel`,
//! `water.foam`, `water.foam_width`, `water.crest_foam`,
//! `water.foam_threshold`, `water.ripples`, `water.under_fog`; the waves
//! run with `weather.wind_dir` (`water.wave_dir` overrides).

use bevy::camera::primitives::Aabb;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

pub struct ModernWaterPlugin;

impl Plugin for ModernWaterPlugin {
    fn build(&self, app: &mut App) {
        bevy::asset::embedded_asset!(app, "modern_water.wgsl");
        // (Usable on its own, without the rest of `AtmosPlugin`: the weather
        // and the time of day it reads, at their defaults unless the host or
        // the full plugin drives them.)
        app.init_resource::<Underwater>()
            .init_resource::<crate::weather::Weather>()
            .init_resource::<crate::time::TimeOfDay>()
            .add_plugins(MaterialPlugin::<ModernWaterMaterial>::default())
            .add_systems(Update, (replace, kinds, look, auto_shelter).chain().after(crate::LookSet))
            .add_systems(PostUpdate, (follow, under).chain().after(bevy::transform::TransformSystems::Propagate))
            .init_resource::<FlatMeshes>()
            .add_systems(PostUpdate, (remember_flat.after(bevy::asset::AssetEventSystems), edge_meshes).chain());
    }
}

/// The host's water: a mesh entity (`Mesh3d`) whose top is the surface.
/// With `water.replace` on, the plugin draws its modern water there instead.
#[derive(Component, Clone, Copy, Default)]
pub struct AtmosWater;

/// Draw the modern water on this water's own mesh (any shape, several
/// pools in one mesh) instead of a flat grid over its bounds: its normals
/// carry the waves, nothing is displaced.
#[derive(Component, Clone, Copy, Default)]
pub struct OwnMesh;

/// The host's colour for this water (its own material's, linear): the
/// modern water's body colour, in place of `water.tint`'s.
#[derive(Component, Clone, Copy)]
pub struct WaterTint(pub Color);

/// The depth (m) to colour and foam this water by at least: for water drawn
/// as a sheet just over the ground (a puddle decal, a game's pool surface)
/// whose real depth below is a few centimetres.
#[derive(Component, Clone, Copy)]
pub struct WaterDepth(pub f32);

/// What kind of water a surface is: how it moves. Set by the host (or the
/// editor's Mark water tool), else worked out from its size, shape, slope
/// and name (`crate::auto`).
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaterKind {
    /// Small: still and mirror-like, no waves, rain rings only.
    Puddle,
    /// A pond, lake or fountain: small waves, by its size.
    Pond,
    /// Long and narrow or sloped: flows along its length.
    River,
    /// Large: swells.
    Sea,
}

impl WaterKind {
    pub fn from_index(i: u32) -> Option<Self> {
        Some(match i {
            1 => WaterKind::Puddle,
            2 => WaterKind::Pond,
            3 => WaterKind::River,
            4 => WaterKind::Sea,
            _ => return None,
        })
    }

    pub fn index(self) -> u32 {
        match self {
            WaterKind::Puddle => 1,
            WaterKind::Pond => 2,
            WaterKind::River => 3,
            WaterKind::Sea => 4,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            WaterKind::Puddle => "puddle",
            WaterKind::Pond => "pond",
            WaterKind::River => "river",
            WaterKind::Sea => "sea",
        }
    }
}

/// The kind a surface was found to be, and its waves for its size (x wave
/// height scale, y wave length scale, z the kind's index): written by
/// `crate::auto`, read into the water's material.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub struct DetectedKind(pub WaterKind, pub Vec4);

/// The mesh's vertex colour alpha fades the water (its edges): for a host
/// whose water meshes carry a fade there (not by default: in many games
/// that alpha means something else, and faded water in squares).
#[derive(Component, Clone, Copy, Default)]
pub struct FadeEdges;

/// Light on the water's body beyond Bevy's lights (linear, in the lights'
/// units before exposure, like an ambient light's): a host lighting its
/// world some other way (baked lightmaps) says how lit its water is.
#[derive(Resource, Clone, Copy, Default)]
pub struct WaterAmbient(pub Vec3);

/// Whether the camera is under a modern water surface (and how deep, m).
#[derive(Resource, Clone, Copy, Default, Debug)]
pub struct Underwater {
    pub under: bool,
    pub depth: f32,
}

pub type ModernWaterMaterial = ExtendedMaterial<StandardMaterial, ModernWater>;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct ModernWater {
    /// x: wave height (m), y: wave speed, z: choppiness, w: detail scale.
    #[uniform(100)]
    pub waves: Vec4,
    /// xy: wave direction (Bevy x, z), z: rain ripples, w: 1 seen from below.
    #[uniform(101)]
    pub wind: Vec4,
    /// x: sky reflection, y: glitter, z: Fresnel, w: the skybox's brightness.
    #[uniform(102)]
    pub light: Vec4,
    /// x: foam, y: foam width (m), z, w: -.
    #[uniform(103)]
    pub body: Vec4,
    /// The water's own colour (light scattered back out of it), linear.
    #[uniform(104)]
    pub scatter: Vec4,
    /// Absorption per metre, per channel.
    #[uniform(105)]
    pub absorb: Vec4,
    /// xyz: towards the sun (the moon by night), w: -.
    #[uniform(106)]
    pub sun_dir: Vec4,
    /// The sun's colour times its illuminance (lux), linear.
    #[uniform(107)]
    pub sun_color: Vec4,
    #[texture(108, dimension = "cube")]
    #[sampler(109)]
    pub sky: Option<Handle<Image>>,
    /// x: crest foam, y: foam threshold, z, w: -.
    #[uniform(110)]
    pub foam: Vec4,
    /// Screen-space reflections ([`crate::ssr::params`]).
    #[uniform(111)]
    pub ssr: Vec4,
    #[texture(112)]
    #[sampler(113)]
    pub history: Option<Handle<Image>>,
    /// Rivers ([`crate::flow::WaterFlow::uniform`]): x mode (0 still), yz
    /// the vector, w speed.
    #[uniform(114)]
    pub flow: Vec4,
    /// The flow knobs ([`crate::flow::look`]).
    #[uniform(115)]
    pub flow_look: Vec4,
    /// A host's flow map (mode 3).
    #[texture(116)]
    #[sampler(117)]
    pub flow_map: Option<Handle<Image>>,
    /// The painted flow ([`crate::flow::FlowPaint`]).
    #[texture(118, sample_type = "float", filterable = false)]
    pub paint: Option<Handle<Image>>,
    /// x the painted cells' size (m), y how many a side, z 1 if any.
    #[uniform(119)]
    pub paint_info: Vec4,
    /// The host's colour ([`WaterTint`]): rgb linear, w 1 if given.
    #[uniform(120)]
    pub tint: Vec4,
    /// The reflected sky's rotation (quaternion xyzw; set each frame).
    #[uniform(121)]
    pub sky_rot: Vec4,
    /// x: the least depth to colour by ([`WaterDepth`], m); yzw: the
    /// host's light on the body ([`WaterAmbient`]; set each frame).
    #[uniform(122)]
    pub extra: Vec4,
    /// x: wave height scale, y: wave length scale (the body's size), z: the
    /// kind ([`WaterKind::index`], 0 unknown), w: 1 to fade by the vertex
    /// alpha ([`FadeEdges`]).
    #[uniform(123)]
    pub kind: Vec4,
}

impl MaterialExtension for ModernWater {
    fn vertex_shader() -> ShaderRef {
        "embedded://bevy_atmos/modern_water.wgsl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://bevy_atmos/modern_water.wgsl".into()
    }
}

/// The modern copy of a [`AtmosWater`] entity.
#[derive(Component)]
pub struct ModernCopy {
    original: Entity,
    /// The original's visibility before it was hidden.
    was: Visibility,
    /// A river (the original mesh, sloped, flowing).
    river: bool,
}

impl ModernCopy {
    /// The host's water entity this draws.
    pub fn original(&self) -> Entity {
        self.original
    }
}

#[derive(Resource)]
struct Shared(Handle<ModernWaterMaterial>);

/// The level has water marked [`AtmosWater`] (the editor's Water tab says
/// so when it hasn't).
pub fn has_water() -> bool {
    HAS_WATER.load(std::sync::atomic::Ordering::Relaxed)
}

static HAS_WATER: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn on() -> bool {
    crate::knobs::get("water.replace", 1.0) >= 0.5
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn replace(
    mut commands: Commands,
    shared: Option<Res<Shared>>,
    mut materials: ResMut<Assets<ModernWaterMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut originals: Query<
        (Entity, Option<&Aabb>, &GlobalTransform, &mut Visibility, &Mesh3d, Option<&crate::flow::WaterFlow>, Has<OwnMesh>, Option<&WaterTint>, Option<&WaterDepth>),
        (With<AtmosWater>, Without<ModernCopy>),
    >,
    modern: Query<(Entity, &ModernCopy)>,
    cameras: Query<(Entity, Has<DepthPrepass>), With<crate::AtmosCamera>>,
    fades: Query<(), With<FadeEdges>>,
    host_materials: (Query<&MeshMaterial3d<StandardMaterial>>, Res<Assets<StandardMaterial>>),
    edges: Query<(Option<&EdgeMesh>, Has<EdgeChecked>)>,
) {
    HAS_WATER.store(!originals.is_empty() || !modern.is_empty(), std::sync::atomic::Ordering::Relaxed);
    let on = on();
    if !on {
        for (e, m) in &modern {
            if let Ok((_, _, _, mut vis, _, _, _, _, _)) = originals.get_mut(m.original) {
                *vis = m.was;
            }
            commands.entity(e).despawn();
        }
        return;
    }
    let handle = match shared {
        Some(s) => s.0.clone(),
        None => {
            let h = materials.add(ExtendedMaterial {
                base: StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 1.0, reflectance: 0.0, alpha_mode: AlphaMode::Premultiplied, double_sided: true, cull_mode: None, ..default() },
                extension: ModernWater::default(),
            });
            commands.insert_resource(Shared(h.clone()));
            h
        }
    };
    let done: Vec<Entity> = modern.iter().map(|(_, m)| m.original).collect();
    let mut any = !done.is_empty();
    for (e, aabb, at, mut vis, mesh, flow, own, tint, least) in &mut originals {
        // (Its bounds come a frame after it's spawned.)
        let Some(aabb) = aabb else { continue };
        if done.contains(&e) {
            continue;
        }
        // (Its own mesh: once its soft-edged copy is made, or found not to
        // be possible, `edge_meshes`.)
        let (edge_mesh, checked) = edges.get(e).map_or((None, true), |(m, c)| (m.map(|m| m.0.clone()), c));
        if own && !checked {
            continue;
        }
        let soft = edge_mesh.is_some();
        let was = *vis;
        *vis = Visibility::Hidden;
        // A river or `OwnMesh` water: its own mesh, and its own material
        // (its flow, or mode 5: still water on its mesh; its colour).
        let on_mesh = flow.is_some() || own;
        // (Each its own material: its kind and size set its waves.)
        let material = {
            let mut m = materials.get(&handle).cloned().unwrap_or_else(|| ExtendedMaterial { base: StandardMaterial::default(), extension: ModernWater::default() });
            // (The original's depth bias, and a little more: water laid
            // over the ground as a sheet would sink into it without.)
            let bias = host_materials.0.get(e).ok().and_then(|h| host_materials.1.get(&h.0)).map_or(0.0, |o| o.depth_bias);
            m.base.depth_bias = bias + 2.0;
            if let Some(flow) = flow {
                m.extension.flow = flow.uniform();
                m.extension.flow_map = flow.map_image();
            } else if own {
                m.extension.flow = Vec4::new(5.0, 0.0, 0.0, 0.0);
            }
            if let Some(t) = tint {
                m.extension.tint = t.0.to_linear().to_vec3().extend(1.0);
            }
            if let Some(d) = least {
                m.extension.extra.x = d.0.max(0.0);
            }
            m.extension.kind = Vec4::new(1.0, 1.0, 0.0, (fades.contains(e) || soft) as u32 as f32);
            materials.add(m)
        };
        let mesh = if on_mesh { edge_mesh.unwrap_or_else(|| mesh.0.clone()) } else { meshes.add(grid(aabb)) };
        commands.spawn((
            Name::new("atmos modern water"),
            ModernCopy { original: e, was, river: on_mesh },
            Mesh3d(mesh),
            MeshMaterial3d(material),
            at.compute_transform(),
            bevy::light::NotShadowCaster,
        ));
        any = true;
    }
    // Depth through the water (absorption, foam) comes from the prepass.
    if any {
        for (c, has) in &cameras {
            if !has {
                commands.entity(c).try_insert(DepthPrepass);
            }
        }
    }
    // (Originals gone: their copies go too.)
    for (e, m) in &modern {
        if originals.get(m.original).is_err() {
            commands.entity(e).despawn();
        }
    }
}

/// A grid over the top of `aabb` (the original's local bounds), cells of
/// a quarter metre up to 256 a side, for the waves to move.
fn grid(aabb: &Aabb) -> Mesh {
    let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
    let size = Vec2::new(h.x * 2.0, h.z * 2.0).max(Vec2::splat(0.01));
    let n = (size / 0.25).ceil().clamp(Vec2::ONE, Vec2::splat(256.0)).as_uvec2();
    let mut positions = Vec::with_capacity(((n.x + 1) * (n.y + 1)) as usize);
    let mut uvs = Vec::with_capacity(positions.capacity());
    for z in 0..=n.y {
        for x in 0..=n.x {
            let f = Vec2::new(x as f32 / n.x as f32, z as f32 / n.y as f32);
            positions.push([c.x - h.x + f.x * size.x, c.y + h.y, c.z - h.z + f.y * size.y]);
            uvs.push([f.x, f.y]);
        }
    }
    let mut indices = Vec::with_capacity((n.x * n.y * 6) as usize);
    for z in 0..n.y {
        for x in 0..n.x {
            let i = z * (n.x + 1) + x;
            indices.extend([i, i + n.x + 1, i + 1, i + 1, i + n.x + 1, i + n.x + 2]);
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut mesh = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}

/// Each copy's material takes its original's kind (and a river's flow, if
/// it came later).
fn kinds(
    copies: Query<(&ModernCopy, &MeshMaterial3d<ModernWaterMaterial>)>,
    originals: Query<(Option<&DetectedKind>, Option<&crate::flow::WaterFlow>, Has<FadeEdges>, Option<&WaterDepth>, Has<EdgeMesh>)>,
    mut materials: ResMut<Assets<ModernWaterMaterial>>,
) {
    for (copy, handle) in &copies {
        let Ok((kind, flow, fade, least, soft)) = originals.get(copy.original) else { continue };
        // (Its least coloured depth, if it changed since: the kind's.)
        if let Some(d) = least {
            if materials.get(&handle.0).is_some_and(|m| m.extension.extra.x != d.0.max(0.0)) {
                if let Some(mut m) = materials.get_mut(&handle.0) {
                    m.extension.extra.x = d.0.max(0.0);
                }
            }
        }
        let want = kind.map_or(Vec4::new(1.0, 1.0, 0.0, 0.0), |k| k.1).truncate().extend((fade || soft) as u32 as f32);
        let want_flow = flow.map_or(Vec4::ZERO, |f| f.uniform());
        let Some(m) = materials.get(&handle.0) else { continue };
        let flow_now = if m.extension.flow.x == 5.0 && want_flow == Vec4::ZERO { m.extension.flow } else { want_flow };
        if m.extension.kind != want || m.extension.flow != flow_now {
            if let Some(mut m) = materials.get_mut(&handle.0) {
                m.extension.kind = want;
                m.extension.flow = flow_now;
            }
        }
    }
}

/// The copies stay where their originals are.
fn follow(originals: Query<&GlobalTransform, With<AtmosWater>>, mut modern: Query<(&ModernCopy, &mut Transform), Without<AtmosWater>>) {
    for (m, mut t) in &mut modern {
        if let Ok(g) = originals.get(m.original) {
            let now = g.compute_transform();
            if *t != now {
                *t = now;
            }
        }
    }
}

/// Without a host [`crate::weather::Shelter`], the plugin makes one from
/// the modern water, so rain stops at its surface and splashes there.
#[derive(Resource)]
pub struct AutoShelter {
    key: Vec<[i32; 5]>,
}

/// The modern water's surfaces as shelter heights (the same layout as the
/// shelter map), for the scene's shelter (`crate::shelter`) to merge in.
#[derive(Resource)]
pub struct WaterShelter {
    pub image: Handle<Image>,
}

const SHELTER_CELL: f32 = 0.5;
const SHELTER_CELLS: u32 = 512;

fn auto_shelter(
    mut commands: Commands,
    shelter: Option<Res<crate::weather::Shelter>>,
    auto: Option<Res<AutoShelter>>,
    water: Option<Res<WaterShelter>>,
    mut images: ResMut<Assets<Image>>,
    modern: Query<(&GlobalTransform, &Aabb, &ModernCopy)>,
) {
    // (A host's own shelter wins; the scene's (`crate::shelter`, while
    // `rain.shelter` is on) takes the water's in.)
    let mine = |s: &crate::weather::Shelter| water.as_ref().is_some_and(|w| w.image == s.heights);
    let scene = crate::knobs::get("rain.shelter", 1.0) >= 0.5;
    if shelter.as_ref().is_some_and(|s| !mine(s)) && auto.is_none() {
        return;
    }
    if scene && auto.is_some() && shelter.as_ref().is_some_and(|s| mine(s)) {
        commands.remove_resource::<crate::weather::Shelter>();
    }
    // The water's boxes: world cells, and the surface's height (cm).
    let mut boxes: Vec<[i32; 5]> = modern
        .iter()
        .map(|(g, aabb, m)| {
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            let (a, b) = (g.transform_point(c - h), g.transform_point(c + h));
            let (lo, hi) = (a.min(b), a.max(b));
            // (A sloped river: its middle height.)
            let y = if m.river { (lo.y + hi.y) * 0.5 } else { hi.y };
            [(lo.x / SHELTER_CELL).floor() as i32, (lo.z / SHELTER_CELL).floor() as i32, (hi.x / SHELTER_CELL).ceil() as i32, (hi.z / SHELTER_CELL).ceil() as i32, (y * 100.0) as i32]
        })
        .collect();
    boxes.sort();
    if auto.as_ref().is_some_and(|a| a.key == boxes) {
        return;
    }
    if boxes.is_empty() {
        if auto.is_some() {
            commands.remove_resource::<AutoShelter>();
            commands.remove_resource::<WaterShelter>();
            if shelter.as_ref().is_some_and(|s| mine(s)) {
                commands.remove_resource::<crate::weather::Shelter>();
            }
        }
        return;
    }
    let n = SHELTER_CELLS as i32;
    let mut data = vec![-1.0e6f32; (n * n) as usize];
    for b in &boxes {
        let top = b[4] as f32 / 100.0;
        for z in b[1]..b[3].min(b[1] + n) {
            for x in b[0]..b[2].min(b[0] + n) {
                data[(z.rem_euclid(n) * n + x.rem_euclid(n)) as usize] = top;
            }
        }
    }
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_le_bytes()).collect();
    let image = Image::new(
        bevy::render::render_resource::Extent3d { width: SHELTER_CELLS, height: SHELTER_CELLS, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        bytes,
        bevy::render::render_resource::TextureFormat::R32Float,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    let image = images.add(image);
    if !scene && shelter.as_ref().is_none_or(|s| mine(s)) {
        commands.insert_resource(crate::weather::Shelter { heights: image.clone(), cell: SHELTER_CELL, cells: SHELTER_CELLS });
    }
    commands.insert_resource(WaterShelter { image });
    commands.insert_resource(AutoShelter { key: boxes });
}

/// The colours from `water.tint` (-1 clear blue .. 0 sea .. 1 murky green-
/// brown) and `water.absorption`.
fn colours(tint: f32, absorption: f32, bright: f32) -> (Vec3, Vec3) {
    let blue = (Vec3::new(0.004, 0.025, 0.04), Vec3::new(0.45, 0.07, 0.04));
    let sea = (Vec3::new(0.008, 0.045, 0.05), Vec3::new(0.40, 0.11, 0.09));
    let murk = (Vec3::new(0.025, 0.03, 0.012), Vec3::new(0.55, 0.45, 0.65));
    let (s, a) = if tint < 0.0 { (sea.0.lerp(blue.0, -tint), sea.1.lerp(blue.1, -tint)) } else { (sea.0.lerp(murk.0, tint), sea.1.lerp(murk.1, tint)) };
    (s * bright, a * absorption)
}

#[allow(clippy::too_many_arguments)]
fn look(
    paint: Option<Res<crate::flow::FlowPaint>>,
    ambient: Option<Res<WaterAmbient>>,
    host: Option<Res<crate::clouds::HostSky>>,
    mut materials: ResMut<Assets<ModernWaterMaterial>>,
    sky: Option<Res<crate::sky::DynamicSky>>,
    weather: Res<crate::weather::Weather>,
    underwater: Res<Underwater>,
    cameras: Query<&bevy::light::Skybox, With<crate::AtmosCamera>>,
    suns: Query<(&DirectionalLight, &GlobalTransform), With<crate::AtmosSun>>,
) {
    let k = crate::knobs::get;
    // The sky reflected: the look's own sky, else the camera's skybox (a
    // host drawing its own sky).
    let host_sky = cameras.iter().next();
    // (A host's own sky: its reflections cube if it gives one.)
    let host_cube = host.as_ref().and_then(|h| h.reflections.clone());
    let sky_image = sky.as_ref().map(|s| s.image()).or(host_cube).or_else(|| host_sky.and_then(|s| s.image.clone()));
    let sky_rot = if sky.is_some() { Quat::IDENTITY } else { host_sky.map_or(Quat::IDENTITY, |s| s.rotation) };
    let dir = k("water.wave_dir", k("weather.wind_dir", 30.0)).to_radians();
    let (scatter, absorb) = colours(k("water.tint", 0.0).clamp(-1.0, 1.0), k("water.absorption", 1.0).clamp(0.0, 4.0), k("water.brightness", 1.0).clamp(0.0, 4.0));
    let (sun_dir, sun_color) = suns.iter().next().map_or((Vec3::Y, Vec3::ZERO), |(l, g)| (-g.forward().as_vec3(), l.color.to_linear().to_vec3() * l.illuminance));
    let ids: Vec<_> = materials.ids().collect();
    for id in ids {
    let Some(mut m) = materials.get_mut(id) else { continue };
    let w = &mut m.extension;
    w.flow_look = crate::flow::look();
    if let Some(p) = &paint {
        w.paint = Some(p.image.clone());
        w.paint_info = Vec4::new(crate::flow::CELL, crate::flow::CELLS as f32, crate::flow::has_paint(p) as u32 as f32, 0.0);
    }
    w.waves = Vec4::new(k("water.wave_height", 0.3).clamp(0.0, 3.0), k("water.wave_speed", 1.0).clamp(0.0, 4.0), k("water.choppiness", 0.4).clamp(0.0, 1.0), k("water.detail", 1.0).clamp(0.1, 4.0));
    w.wind = Vec4::new(dir.cos(), dir.sin(), weather.rain * k("water.ripples", 1.0).clamp(0.0, 3.0), underwater.under as u32 as f32);
    w.light = Vec4::new(k("water.reflection", 1.0).clamp(0.0, 3.0), k("water.glitter", 1.0).clamp(0.0, 4.0), k("water.fresnel", 1.0).clamp(0.1, 3.0), cameras.iter().next().map_or(1000.0, |s| s.brightness));
    w.body = Vec4::new(k("water.foam", 1.0).clamp(0.0, 3.0), k("water.foam_width", 0.4).clamp(0.01, 4.0), k("water.sediment", 0.6).clamp(0.0, 1.0), 0.0);
    w.foam = Vec4::new(k("water.crest_foam", 0.6).clamp(0.0, 3.0), k("water.foam_threshold", 0.5).clamp(0.0, 1.5), 0.0, 0.0);
    w.scatter = if w.tint.w > 0.5 { (w.tint.truncate() * k("water.brightness", 1.0).clamp(0.0, 4.0)).extend(1.0) } else { scatter.extend(1.0) };
    // (A host's own colour: murky water of that colour, absorbing evenly.)
    w.absorb = if w.tint.w > 0.5 { (Vec3::splat(1.5) * k("water.absorption", 1.0).clamp(0.0, 8.0)).extend(0.0) } else { absorb.extend(0.0) };
    w.sun_dir = sun_dir.extend(0.0);
    w.sun_color = sun_color.extend(0.0);
    w.sky = sky_image.clone();
    w.sky_rot = Vec4::from(sky_rot);
    let a = ambient.as_ref().map_or(Vec3::ZERO, |a| a.0);
    w.extra = Vec4::new(w.extra.x, a.x, a.y, a.z);
    w.ssr = crate::ssr::params();
    if w.history.is_none() {
        w.history = Some(crate::ssr::history());
    }
    }
}

/// Under the surface: whether the camera is inside a modern water's box,
/// below its top; there, the camera's fog turns to the water's (and back
/// as it was on leaving).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn under(
    mut commands: Commands,
    mut underwater: ResMut<Underwater>,
    mut ray_cast: bevy::picking::mesh_picking::ray_cast::MeshRayCast,
    rivers: Query<&ModernCopy>,
    modern: Query<(&GlobalTransform, Option<&Aabb>, &ModernCopy)>,
    mut cameras: Query<(Entity, &GlobalTransform, Option<&mut DistanceFog>), With<crate::AtmosCamera>>,
    tod: Res<crate::time::TimeOfDay>,
    mut saved: Local<Option<Option<DistanceFog>>>,
) {
    let Some((cam, eye, fog)) = cameras.iter_mut().next() else { return };
    let eye = eye.translation();
    let mut found = None;
    if on() {
        // A river (sloped): under it if a ray straight up meets it (needs
        // the mesh's data on the CPU; without it, no fog there).
        if rivers.iter().any(|m| m.river) {
            let filter = |e: Entity| rivers.get(e).is_ok_and(|m| m.river);
            let settings = bevy::picking::mesh_picking::ray_cast::MeshRayCastSettings::default()
                .with_filter(&filter)
                .with_visibility(bevy::picking::mesh_picking::ray_cast::RayCastVisibility::Any);
            if let Some((_, hit)) = ray_cast.cast_ray(Ray3d::new(eye, Dir3::Y), &settings).first() {
                if hit.distance < 60.0 {
                    found = Some(hit.distance);
                }
            }
        }
        for (g, aabb, m) in &modern {
            if m.river || found.is_some() {
                continue;
            }
            let Some(aabb) = aabb else { continue };
            let local = g.affine().inverse().transform_point3(eye);
            let (c, h) = (Vec3::from(aabb.center), Vec3::from(aabb.half_extents));
            let top = c.y + h.y;
            if (local.x - c.x).abs() <= h.x && (local.z - c.z).abs() <= h.z && local.y < top && local.y > top - 200.0 {
                found = Some(g.transform_point(Vec3::new(local.x, top, local.z)).y - eye.y);
                break;
            }
        }
    }
    underwater.under = found.is_some();
    underwater.depth = found.unwrap_or(0.0);
    let density = crate::knobs::get("water.under_fog", 0.15).clamp(0.0, 4.0);
    if let Some(depth) = found {
        if saved.is_none() {
            *saved = Some(fog.as_deref().cloned());
        }
        let (scatter, _) = colours(crate::knobs::get("water.tint", 0.0).clamp(-1.0, 1.0), 1.0, crate::knobs::get("water.brightness", 1.0).clamp(0.0, 4.0));
        // (The fog's colour is as the frame shows it, after exposure.)
        // Darker the deeper and by night.
        let c = scatter * 6.0 * (-depth * 0.08).exp() * (1.0 - 0.9 * tod.night);
        let new = DistanceFog { color: Color::linear_rgb(c.x, c.y, c.z), falloff: FogFalloff::Exponential { density }, ..default() };
        match fog {
            Some(mut f) => *f = new,
            None => {
                commands.entity(cam).try_insert(new);
            }
        }
    } else if let Some(was) = saved.take() {
        match was {
            Some(f) => {
                commands.entity(cam).try_insert(f);
            }
            None => {
                commands.entity(cam).try_remove::<DistanceFog>();
            }
        }
    }
}

/// A soft-edged copy of a water surface's own mesh (`edge_meshes`): its
/// vertex alpha the distance in from the mesh's boundary.
#[derive(Component, Clone)]
pub struct EdgeMesh(pub Handle<Mesh>);

/// `edge_meshes` has looked at this surface.
#[derive(Component)]
pub struct EdgeChecked;

/// Water on its own mesh, as it arrives: while its data is still on the CPU
/// (a host's render-only mesh keeps it only for the frame it's added), a
/// copy subdivided so it has inner vertices, its vertex alpha fading in
/// from the boundary, so puddles and ponds get soft edges with no host
/// data. (Not where the host fades its own, `FadeEdges`.)
#[allow(clippy::type_complexity)]
fn edge_meshes(
    mut commands: Commands,
    waters: Query<(Entity, &Mesh3d, Has<FadeEdges>), (With<AtmosWater>, With<OwnMesh>, Without<EdgeChecked>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    flat: Res<FlatMeshes>,
) {
    for (e, handle, host_fades) in &waters {
        if host_fades {
            commands.entity(e).try_insert(EdgeChecked);
            continue;
        }
        // (A host's render-only mesh has given up its data by now: the
        // copy `remember_flat` kept as it arrived.)
        let soft = match meshes.get(&handle.0) {
            Some(mesh) => soft_edged(mesh).or_else(|| flat.0.get(&handle.0.id()).and_then(|(m, _)| soft_edged(m))),
            None => continue,
        };
        let mut entity = commands.entity(e);
        entity.try_insert(EdgeChecked);
        match soft {
            Some(m) => {
                entity.try_insert(EdgeMesh(meshes.add(m)));
                EDGES.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
            None => {
                EDGES.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let (a, b) = (EDGES.0.load(std::sync::atomic::Ordering::Relaxed), EDGES.1.load(std::sync::atomic::Ordering::Relaxed));
        debug!("bevy_atmos: soft water edges on {a} surfaces ({b} without: data not on the CPU or too big)");
    }
}

/// `mesh` subdivided to some hundreds of triangles, its vertex alpha the
/// distance in from its boundary (smoothstep over a fade width set by its
/// size); none if its data isn't on the CPU or it has no boundary.
fn soft_edged(mesh: &Mesh) -> Option<Mesh> {
    use bevy::mesh::VertexAttributeValues as V;
    let pos: Vec<Vec3> = match mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION).ok()?? {
        V::Float32x3(p) => p.iter().map(|p| Vec3::from(*p)).collect(),
        _ => return None,
    };
    let nor: Option<Vec<Vec3>> = match mesh.try_attribute_option(Mesh::ATTRIBUTE_NORMAL).ok().flatten() {
        Some(V::Float32x3(n)) => Some(n.iter().map(|n| Vec3::from(*n)).collect()),
        _ => None,
    };
    let uv: Option<Vec<Vec2>> = match mesh.try_attribute_option(Mesh::ATTRIBUTE_UV_0).ok().flatten() {
        Some(V::Float32x2(u)) => Some(u.iter().map(|u| Vec2::from(*u)).collect()),
        _ => None,
    };
    let mut tris: Vec<[u32; 3]> = match mesh.try_indices_option().ok()? {
        Some(i) => i.iter().collect::<Vec<usize>>().chunks_exact(3).map(|c| [c[0] as u32, c[1] as u32, c[2] as u32]).collect(),
        None => (0..pos.len() as u32 / 3).map(|t| [t * 3, t * 3 + 1, t * 3 + 2]).collect(),
    };
    if tris.is_empty() || tris.len() > 4_000 {
        return None;
    }
    let (mut p, mut n, mut u) = (pos, nor, uv);
    // Subdivide (each triangle into four) until there are inner vertices
    // enough to fade across.
    while tris.len() < 600 {
        let mut mid: std::collections::HashMap<(u32, u32), u32> = std::collections::HashMap::new();
        let mut next = Vec::with_capacity(tris.len() * 4);
        for [a, b, c] in tris {
            let mut midpoint = |a: u32, b: u32| -> u32 {
                *mid.entry((a.min(b), a.max(b))).or_insert_with(|| {
                    p.push((p[a as usize] + p[b as usize]) * 0.5);
                    if let Some(n) = n.as_mut() {
                        let m = (n[a as usize] + n[b as usize]).normalize_or(Vec3::Y);
                        n.push(m);
                    }
                    if let Some(u) = u.as_mut() {
                        let m = (u[a as usize] + u[b as usize]) * 0.5;
                        u.push(m);
                    }
                    p.len() as u32 - 1
                })
            };
            let ab = midpoint(a, b);
            let bc = midpoint(b, c);
            let ca = midpoint(c, a);
            next.extend([[a, ab, ca], [ab, b, bc], [ca, bc, c], [ab, bc, ca]]);
        }
        tris = next;
    }
    // The distance in from the edge, from the mesh drawn flat into 10 cm
    // cells (on its two widest axes), kept sparse: a host's puddles are
    // often loose triangles or T-junctions (where each triangle's own edges
    // read as the boundary), and one mesh may hold puddles scattered over a
    // whole street (a coarse grid missed the small ones and faded them out).
    let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), v| (l.min(*v), h.max(*v)));
    let size = hi - lo;
    let thin = if size.x <= size.y && size.x <= size.z { 0 } else if size.y <= size.z { 1 } else { 2 };
    let (ia, ib) = match thin {
        0 => (1, 2),
        1 => (0, 2),
        _ => (0, 1),
    };
    const CELL: f32 = 0.1;
    let flat = |v: Vec3| Vec2::new(v[ia], v[ib]) / CELL;
    let mut inside: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    for t in &tris {
        let (a, b, c) = (flat(p[t[0] as usize]), flat(p[t[1] as usize]), flat(p[t[2] as usize]));
        let (mn, mx) = (a.min(b).min(c), a.max(b).max(c));
        let area = (b - a).perp_dot(c - a);
        // (Its corners' cells always: a sliver still marks its cells.)
        for q in [a, b, c, (a + b + c) / 3.0] {
            inside.insert((q.x.floor() as i32, q.y.floor() as i32));
        }
        if area.abs() < 1e-8 {
            continue;
        }
        for y in mn.y.floor() as i32..=mx.y.ceil() as i32 {
            for x in mn.x.floor() as i32..=mx.x.ceil() as i32 {
                let q = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let w0 = (b - q).perp_dot(c - q) / area;
                let w1 = (c - q).perp_dot(a - q) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 > -0.05 && w1 > -0.05 && w2 > -0.05 {
                    inside.insert((x, y));
                }
            }
        }
        if inside.len() > 3_000_000 {
            return None;
        }
    }
    // (Fade over up to 60 cm: a vertex's distance to the nearest cell
    // outside the water, searched out that far.)
    const REACH: i32 = 6;
    let reach_m = REACH as f32 * CELL;
    let alpha: Vec<[f32; 4]> = p
        .iter()
        .map(|v| {
            let f = flat(*v);
            let (cx, cy) = (f.x.floor() as i32, f.y.floor() as i32);
            let mut best = reach_m;
            for dy in -REACH..=REACH {
                for dx in -REACH..=REACH {
                    if !inside.contains(&(cx + dx, cy + dy)) {
                        let d = Vec2::new(dx as f32, dy as f32).length() * CELL;
                        best = best.min((d - CELL * 0.5).max(0.0));
                    }
                }
            }
            let x = (best / reach_m).clamp(0.0, 1.0);
            [1.0, 1.0, 1.0, x * x * (3.0 - 2.0 * x)]
        })
        .collect();
    let mut out = Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, bevy::asset::RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.iter().map(|v| v.to_array()).collect::<Vec<_>>());
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, n.map_or_else(|| vec![[0.0, 1.0, 0.0]; p.len()], |n| n.iter().map(|v| v.to_array()).collect()));
    if let Some(u) = u {
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, u.iter().map(|v| v.to_array()).collect::<Vec<_>>());
    }
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, alpha);
    out.insert_indices(bevy::mesh::Indices::U32(tris.into_iter().flatten().collect()));
    Some(out)
}

/// Soft-edged water surfaces made, and those that couldn't be.
static EDGES: (std::sync::atomic::AtomicU32, std::sync::atomic::AtomicU32) = (std::sync::atomic::AtomicU32::new(0), std::sync::atomic::AtomicU32::new(0));

/// Small flat meshes as they arrive (the CPU copy a host's render-only mesh
/// gives up when it goes to the GPU), kept a few seconds for water surfaces
/// tagged after: their soft edges (`edge_meshes`).
#[derive(Resource, Default)]
struct FlatMeshes(std::collections::HashMap<AssetId<Mesh>, (Mesh, u32)>, u32);

fn remember_flat(mut events: MessageReader<AssetEvent<Mesh>>, meshes: Res<Assets<Mesh>>, mut flat: ResMut<FlatMeshes>) {
    flat.1 = flat.1.wrapping_add(1);
    let now = flat.1;
    for ev in events.read() {
        let AssetEvent::Added { id } = ev else { continue };
        let Some(mesh) = meshes.get(*id) else { continue };
        let Some(Some(bevy::mesh::VertexAttributeValues::Float32x3(p))) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION).ok() else { continue };
        if p.len() < 3 || p.len() > 6_000 {
            continue;
        }
        let (lo, hi) = p.iter().fold((Vec3::MAX, Vec3::MIN), |(l, h), v| (l.min(Vec3::from(*v)), h.max(Vec3::from(*v))));
        let size = hi - lo;
        // (Sheets only: one axis much thinner than the others.)
        if size.min_element() > size.max_element() * 0.05 {
            continue;
        }
        flat.0.insert(*id, (mesh.clone(), now));
    }
    // (Kept ten seconds or so.)
    flat.0.retain(|_, (_, at)| now.wrapping_sub(*at) < 600);
}
