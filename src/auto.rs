//! Automatic setup, so most games need nothing beyond the plugin line (tag
//! things only to override):
//! - **The camera**: with no [`crate::AtmosCamera`] tagged and exactly one
//!   `Camera3d`, that one.
//! - **The sun**: with no [`crate::AtmosSun`] tagged, the brightest
//!   `DirectionalLight`.
//! - **Water** (`water.auto`, on): meshes whose `Name`, glTF mesh name or
//!   glTF material name (theirs or their parent's) holds one of
//!   [`crate::AtmosSettings::water_words`]; and with `water.guess` (off: it
//!   can catch glass) meshes that look like water: a smooth or see-through
//!   blue-green `StandardMaterial` on a flat, thin, level mesh. Each gets
//!   [`AtmosWater`] + [`OwnMesh`] (any shape works), its material's colour
//!   ([`WaterTint`]) and a least depth for a sheet over the ground; a
//!   sloped one becomes a river flowing downhill.
//! - **Marked by hand**: the editor's "Mark water" tool (click a surface to
//!   mark or unmark it), saved in the level's look as `water.mark.<name> =
//!   1` (or 0: never water), by its glTF material, mesh or entity name, or
//!   where it is when it has none.
//!
//! A host's own `AtmosWater` (without [`AutoWater`]) is left as it is.

use crate::modern_water::{AtmosWater, DetectedKind, ModernCopy, OwnMesh, WaterDepth, WaterKind, WaterTint};
use bevy::camera::primitives::Aabb;
use bevy::gltf::{GltfMaterialName, GltfMeshName};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::RwLock;

pub struct AutoPlugin;

impl Plugin for AutoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeshShapes>()
            .add_systems(Update, (camera_and_sun, detect, classify, mark_tool, highlight).chain().before(crate::LookSet))
            .add_systems(PostUpdate, remember_shapes.after(bevy::asset::AssetEventSystems));
    }
}

/// The words that make a mesh water by its name (any case).
pub const WATER_WORDS: &[&str] = &["water", "sea", "ocean", "river", "lake", "pool", "pond", "canal", "puddle"];

/// How a water surface was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    ByName,
    ByLook,
    ByHand,
}

/// Water this module marked (and how), so it can take it back.
#[derive(Component, Clone, Copy, Debug)]
pub struct AutoWater(pub Found);

/// Looked at and not water (checked again only when the knobs change).
#[derive(Component)]
struct NotWater;

/// The "Mark water" tool is on.
static MARKING: AtomicBool = AtomicBool::new(false);

pub fn marking() -> bool {
    MARKING.load(Ordering::Relaxed)
}

pub fn set_marking(on: bool) {
    MARKING.store(on, Ordering::Relaxed);
}

/// What a Mark water click does on water: 0 unmarks it, 1..4 sets its kind
/// ([`WaterKind::index`]).
static MARK_KIND: AtomicU32 = AtomicU32::new(0);

/// Mark water clicks on water set its kind (`None`: they unmark it).
pub fn set_mark_kind(kind: Option<WaterKind>) {
    MARK_KIND.store(kind.map_or(0, |k| k.index()), Ordering::Relaxed);
}

pub fn mark_kind() -> Option<WaterKind> {
    WaterKind::from_index(MARK_KIND.load(Ordering::Relaxed))
}

/// The water's kinds, for the editor: "4 puddles, 1 river".
static KINDS: RwLock<String> = RwLock::new(String::new());

pub fn kinds_summary() -> String {
    KINDS.read().map(|s| s.clone()).unwrap_or_default()
}

/// A river's flow this module gave it (taken back if it stops being one).
#[derive(Component)]
struct AutoFlow;

/// The water found, for the editor: "3 by name, 1 marked by hand".
static SUMMARY: RwLock<String> = RwLock::new(String::new());

pub fn summary() -> String {
    SUMMARY.read().map(|s| s.clone()).unwrap_or_default()
}

fn camera_and_sun(
    mut commands: Commands,
    tagged_cameras: Query<(), With<crate::AtmosCamera>>,
    cameras: Query<(Entity, Option<&Name>), (With<Camera3d>, Without<crate::shelter::ShelterCamera>)>,
    tagged_suns: Query<(), With<crate::AtmosSun>>,
    suns: Query<(Entity, &DirectionalLight, Option<&Name>)>,
) {
    if tagged_cameras.is_empty() && cameras.iter().count() == 1 {
        if let Some((e, name)) = cameras.iter().next() {
            commands.entity(e).try_insert(crate::AtmosCamera);
            info!("bevy_atmos: using the only 3D camera ({}) as AtmosCamera", name.map_or(format!("{e}"), |n| n.to_string()));
        }
    }
    if tagged_suns.is_empty() {
        if let Some((e, light, name)) = suns.iter().max_by(|a, b| a.1.illuminance.total_cmp(&b.1.illuminance)) {
            commands.entity(e).try_insert(crate::AtmosSun);
            info!("bevy_atmos: using the brightest directional light ({}, {:.0} lux) as AtmosSun", name.map_or(format!("{e}"), |n| n.to_string()), light.illuminance);
        }
    }
}

/// The name a surface is remembered by: its glTF material's, mesh's, its
/// own or its parent's name; else where it is.
fn key_of(
    e: Entity,
    names: &Query<(Option<&Name>, Option<&GltfMaterialName>, Option<&GltfMeshName>, Option<&ChildOf>)>,
    at: &GlobalTransform,
) -> String {
    let mut key = None;
    if let Ok((name, material, mesh, parent)) = names.get(e) {
        key = material.map(|m| m.0.clone()).or_else(|| mesh.map(|m| m.0.clone())).or_else(|| name.map(|n| n.to_string()));
        if key.is_none() {
            if let Some(p) = parent {
                if let Ok((name, _, mesh, _)) = names.get(p.parent()) {
                    key = mesh.map(|m| m.0.clone()).or_else(|| name.map(|n| n.to_string()));
                }
            }
        }
    }
    let raw = key.filter(|k| !k.trim().is_empty()).unwrap_or_else(|| {
        let p = (at.translation() * 10.0).round();
        format!("pos_{}_{}_{}", p.x as i64, p.y as i64, p.z as i64)
    });
    raw.to_lowercase().chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' }).collect()
}

/// Every name the entity (and its parent) goes by, lowercase.
fn names_of(e: Entity, names: &Query<(Option<&Name>, Option<&GltfMaterialName>, Option<&GltfMeshName>, Option<&ChildOf>)>) -> String {
    let mut all = String::new();
    let mut add = |e: Entity, parent: bool| -> Option<Entity> {
        let (name, material, mesh, up) = names.get(e).ok()?;
        for s in [name.map(|n| n.as_str()), material.map(|m| m.0.as_str()), mesh.map(|m| m.0.as_str())].into_iter().flatten() {
            all.push_str(&s.to_lowercase());
            all.push(' ');
        }
        if parent { None } else { up.map(|p| p.parent()) }
    };
    if let Some(p) = add(e, false) {
        add(p, true);
    }
    all
}

/// Whether a material and its mesh look like water.
fn looks_like_water(m: &StandardMaterial, aabb: &Aabb, at: &GlobalTransform) -> bool {
    let c = m.base_color.to_linear();
    let blended = matches!(m.alpha_mode, AlphaMode::Blend | AlphaMode::Premultiplied | AlphaMode::Add) && c.alpha < 0.95;
    let smooth = m.perceptual_roughness < 0.15;
    let blue_green = c.blue + c.green > c.red * 2.2 && c.blue > c.red;
    let (normal, ax, az) = surface_axes(at, aabb);
    let thin = thinness(at, aabb);
    let flat = thin < 0.08 * ax.length().max(az.length()) && ax.length().max(az.length()) > 1.0;
    (blended || smooth) && blue_green && flat && normal.y > 0.9
}

/// A flat mesh's surface: its normal (from its thinnest local axis, any
/// way the mesh is built: Y or Z up), and its two other axes' world
/// lengths across the ground (x, z).
fn surface_axes(at: &GlobalTransform, aabb: &Aabb) -> (Vec3, Vec2, Vec2) {
    let t = at.compute_transform();
    let size = Vec3::from(aabb.half_extents) * 2.0 * t.scale.abs();
    let axes = [(t.rotation * Vec3::X, size.x), (t.rotation * Vec3::Y, size.y), (t.rotation * Vec3::Z, size.z)];
    let thin = (0..3).min_by(|&a, &b| axes[a].1.total_cmp(&axes[b].1)).unwrap_or(1);
    let mut normal = axes[thin].0;
    if normal.y < 0.0 {
        normal = -normal;
    }
    let others: Vec<Vec2> = (0..3).filter(|&i| i != thin).map(|i| Vec2::new(axes[i].0.x, axes[i].0.z) * axes[i].1).collect();
    (normal, others[0], others[1])
}

/// How thick a mesh is along its thinnest axis (m).
fn thinness(at: &GlobalTransform, aabb: &Aabb) -> f32 {
    (Vec3::from(aabb.half_extents) * 2.0 * at.compute_transform().scale.abs()).min_element()
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn detect(
    mut commands: Commands,
    settings: Res<crate::AtmosSettings>,
    candidates: Query<
        (Entity, &GlobalTransform, Option<&Aabb>, Option<&MeshMaterial3d<StandardMaterial>>, Has<NotWater>, Option<&AutoWater>, Has<AtmosWater>),
        (With<Mesh3d>, Without<ModernCopy>),
    >,
    names: Query<(Option<&Name>, Option<&GltfMaterialName>, Option<&GltfMeshName>, Option<&ChildOf>)>,
    materials: Res<Assets<StandardMaterial>>,
    mut last: Local<Option<(bool, bool, usize)>>,
) {
    let k = crate::knobs::get;
    let (auto, guess) = (k("water.auto", 1.0) >= 0.5, k("water.guess", 0.0) >= 0.5);
    let marks: Vec<(String, f32)> = crate::knobs::all().into_iter().filter_map(|(n, v)| n.strip_prefix("water.mark.").map(|s| (s.to_owned(), v))).collect();
    // (A knob or the hand marks changed: everything is looked at again.)
    let now = (auto, guess, marks.iter().map(|(n, v)| n.len() + (*v as usize) * 7).sum::<usize>() + marks.len() * 1000);
    let redo = *last != Some(now);
    *last = Some(now);
    let (mut by_name, mut by_look, mut by_hand) = (0, 0, 0);
    for (e, at, aabb, material, not_water, found, has_water) in &candidates {
        if let Some(f) = found {
            match f.0 {
                Found::ByName => by_name += 1,
                Found::ByLook => by_look += 1,
                Found::ByHand => by_hand += 1,
            }
        }
        if !redo && (not_water || found.is_some()) {
            continue;
        }
        // (A host's own water stays as the host made it.)
        if has_water && found.is_none() {
            continue;
        }
        let key = key_of(e, &names, at);
        let hand = marks.iter().find(|(n, _)| *n == key).map(|(_, v)| *v >= 0.5);
        let how = match hand {
            Some(true) => Some(Found::ByHand),
            Some(false) => None,
            None if auto && settings.water_words.iter().any(|w| names_of(e, &names).contains(&w.to_lowercase())) => Some(Found::ByName),
            None if auto && guess && aabb.zip(material).and_then(|(a, m)| materials.get(&m.0).map(|m| looks_like_water(m, a, at))).unwrap_or(false) => Some(Found::ByLook),
            None => None,
        };
        let mut entity = commands.entity(e);
        match how {
            Some(how) => {
                if found.map(|f| f.0) == Some(how) {
                    continue;
                }
                entity.try_remove::<NotWater>().try_insert((AtmosWater, OwnMesh, WaterDepth(0.4), AutoWater(how)));
                if let Some(m) = material.and_then(|m| materials.get(&m.0)) {
                    let c = m.base_color.to_linear();
                    entity.try_insert(WaterTint(Color::linear_rgb(c.red * 0.4, c.green * 0.4, c.blue * 0.4)));
                }
                // (A sloped or long, narrow one becomes a river: `classify`.)
                info!("bevy_atmos: water {key} ({how:?})");
            }
            None => {
                if found.is_some() {
                    // (Its modern copy goes with the tag; it's shown again.)
                    entity.try_remove::<(AtmosWater, OwnMesh, WaterDepth, WaterTint, AutoWater, crate::flow::WaterFlow)>().try_insert(Visibility::Inherited);
                }
                entity.try_insert(NotWater);
            }
        }
    }
    let mut parts = Vec::new();
    for (n, what) in [(by_name, "by name"), (by_look, "guessed from materials"), (by_hand, "marked by hand")] {
        if n > 0 {
            parts.push(format!("{n} {what}"));
        }
    }
    if let Ok(mut s) = SUMMARY.write() {
        *s = parts.join(", ");
    }
}

/// "Mark water": with the editor open, a click on a surface marks it as
/// water (or, on water, unmarks it); saved with the look.
#[allow(clippy::too_many_arguments)]
fn mark_tool(
    mut ray_cast: MeshRayCast,
    buttons: Res<ButtonInput<MouseButton>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    cameras: Query<(&Camera, &GlobalTransform), With<crate::AtmosCamera>>,
    copies: Query<&ModernCopy>,
    water: Query<(), With<AtmosWater>>,
    names: Query<(Option<&Name>, Option<&GltfMaterialName>, Option<&GltfMeshName>, Option<&ChildOf>)>,
    transforms: Query<&GlobalTransform>,
) {
    if !marking() || !crate::editor::open() || !buttons.just_pressed(MouseButton::Left) {
        return;
    }
    let (Ok(window), Some((camera, at))) = (windows.single(), cameras.iter().next()) else { return };
    let Some(cursor) = window.cursor_position() else { return };
    // (Not on the editor's panel, down the right side.)
    if cursor.x > window.width() - 440.0 {
        return;
    }
    let Ok(ray) = camera.viewport_to_world(at, cursor) else { return };
    let settings = MeshRayCastSettings::default().with_visibility(RayCastVisibility::Any);
    let Some((hit, _)) = ray_cast.cast_ray(ray, &settings).first() else { return };
    // (A modern water copy stands for its original.)
    let e = copies.get(*hit).map_or(*hit, |c| c.original());
    let Ok(t) = transforms.get(e) else { return };
    let key = key_of(e, &names, t);
    let is_water = water.contains(e);
    // (With a kind chosen, a click on water sets its kind instead.)
    if let (true, Some(kind)) = (is_water, mark_kind()) {
        crate::knobs::set(&format!("water.kind.{key}"), Some(kind.index() as f32));
        info!("bevy_atmos: {key} is a {} (Save to keep it)", kind.name());
        return;
    }
    crate::knobs::set(&format!("water.mark.{key}"), Some(if is_water { 0.0 } else { 1.0 }));
    info!("bevy_atmos: {} {key} by hand (Save to keep it)", if is_water { "unmarked" } else { "marked" });
}

/// While marking: an outline round every water surface.
fn highlight(mut gizmos: Gizmos, water: Query<(&GlobalTransform, &Aabb), (With<AtmosWater>, Without<ModernCopy>)>) {
    if !marking() || !crate::editor::open() {
        return;
    }
    for (at, aabb) in &water {
        let t = at.compute_transform();
        let local = Transform::from_translation(Vec3::from(aabb.center)).with_scale((Vec3::from(aabb.half_extents) * 2.0).max(Vec3::splat(0.05)));
        gizmos.cube(t * local, Color::srgb(0.2, 0.9, 1.0));
    }
}

/// Each water surface's kind (the host's [`WaterKind`], else one set in
/// the editor, else by its name, else by its shape: small a puddle, long
/// and narrow or sloped a river, huge a sea, else a pond), and its waves
/// for its size. A river with no flow of its own flows along its length
/// (downhill when sloped).
#[allow(clippy::type_complexity)]
fn classify(
    mut commands: Commands,
    waters: Query<(Entity, &GlobalTransform, Option<&Aabb>, Option<&WaterKind>, Option<&DetectedKind>, Has<crate::flow::WaterFlow>, Has<AutoFlow>, Has<AutoWater>), (With<AtmosWater>, Without<ModernCopy>)>,
    names: Query<(Option<&Name>, Option<&GltfMaterialName>, Option<&GltfMeshName>, Option<&ChildOf>)>,
    mesh_of: Query<&Mesh3d>,
    meshes: Res<Assets<Mesh>>,
    remembered: Res<MeshShapes>,
    mut shapes: Local<std::collections::HashMap<Entity, Option<(f32, f32)>>>,
) {
    let mut counts = [0u32; 5];
    for (e, at, aabb, host, found, has_flow, auto_flow, auto) in &waters {
        let Some(aabb) = aabb else { continue };
        let (normal, ax, az) = surface_axes(at, aabb);
        let (lx, lz) = (ax.length(), az.length());
        let (long, short) = (lx.max(lz), lx.min(lz).max(0.01));
        let area = lx * lz;
        let up = normal;
        let sloped = up.y < 0.995;
        let key = key_of(e, &names, at);
        let hand = WaterKind::from_index(crate::knobs::get(&format!("water.kind.{key}"), 0.0).round() as u32);
        let named = names_of(e, &names);
        let by_name = if named.contains("puddle") {
            Some(WaterKind::Puddle)
        } else if ["river", "canal", "stream", "creek"].iter().any(|w| named.contains(w)) {
            Some(WaterKind::River)
        } else if ["sea", "ocean"].iter().any(|w| named.contains(w)) {
            Some(WaterKind::Sea)
        } else if ["pond", "lake", "pool", "fountain"].iter().any(|w| named.contains(w)) {
            Some(WaterKind::Pond)
        } else {
            None
        };
        // A winding river's box is squat: its outline says it's long and
        // narrow (width about twice its area over its outline).
        let shape = *shapes.entry(e).or_insert_with(|| {
            let handle = mesh_of.get(e).ok()?;
            // (Measured while its data was still on the CPU, in its own
            // space: scaled to the world by its box.)
            if let Some(&(a, p)) = remembered.0.get(&handle.0.id()) {
                let h = Vec3::from(aabb.half_extents) * 2.0;
                let local = (h.x * h.y).max(h.y * h.z).max(h.x * h.z).max(1e-6);
                let k = (area / local).sqrt();
                return Some((a * k * k, p * k));
            }
            meshes.get(&handle.0).and_then(|m| outline(m, &at.affine()))
        });
        let winding = shape.is_some_and(|(a, p)| {
            let width = 2.0 * a / p.max(0.01);
            a > 200.0 && width < 40.0 && p * 0.5 > 5.0 * width
        });
        let kind = host.copied().or(hand).or(by_name).unwrap_or(if sloped || winding || (long / short > 4.0 && short < 40.0) {
            WaterKind::River
        } else if area < 30.0 && long < 10.0 {
            WaterKind::Puddle
        } else if area > 40_000.0 {
            WaterKind::Sea
        } else {
            WaterKind::Pond
        });
        // Waves for the body's size (its fetch): a pond's small and short,
        // the sea's swells.
        let size = area.sqrt();
        let waves = match kind {
            WaterKind::Puddle => Vec2::new(0.0, 1.0),
            WaterKind::Pond => Vec2::new((size / 50.0).clamp(0.12, 0.6), (size / 40.0).clamp(0.2, 1.0)),
            WaterKind::River => Vec2::new(0.25, 0.4),
            WaterKind::Sea => Vec2::new(1.5, 2.5),
        };
        counts[kind.index() as usize] += 1;
        let detected = DetectedKind(kind, Vec4::new(waves.x, waves.y, kind.index() as f32, 0.0));
        if found != Some(&detected) {
            debug!("bevy_atmos: water {key}: {} at {:.1}, {:.0} x {:.0} m", kind.name(), at.translation(), long, short);
            let mut entity = commands.entity(e);
            entity.insert(detected);
            // (Found by the plugin, its depth unknown (often a sheet just
            // over its bed): coloured as deep as its kind would be, or a
            // river over bright sand read as milky glass.)
            if auto {
                entity.insert(WaterDepth(match kind {
                    // (A puddle's ground shows through, darkened wet.)
                    WaterKind::Puddle => 0.08,
                    WaterKind::Pond | WaterKind::River => 1.5,
                    WaterKind::Sea => 4.0,
                }));
            }
            if kind == WaterKind::River && !has_flow {
                let along = if lx >= lz { ax } else { az };
                let mut dir = along.normalize_or(Vec2::X);
                if sloped {
                    let downhill = Vec2::new(up.x, up.z);
                    if dir.dot(downhill) < 0.0 {
                        dir = -dir;
                    }
                }
                entity.insert((crate::flow::WaterFlow::world(dir * 1.2), AutoFlow));
            } else if kind != WaterKind::River && auto_flow {
                entity.remove::<(crate::flow::WaterFlow, AutoFlow)>();
            }
        }
    }
    let mut parts = Vec::new();
    for (k, n) in counts.iter().enumerate().skip(1) {
        if *n > 0 {
            let name = WaterKind::from_index(k as u32).map_or("", |k| k.name());
            parts.push(format!("{n} {name}{}", if *n == 1 { "" } else { "s" }));
        }
    }
    if let Ok(mut s) = KINDS.write() {
        *s = parts.join(", ");
    }
}

/// A water mesh's area and the length of its outline (m², m), in the world:
/// none if its data isn't kept on the CPU.
fn outline(mesh: &Mesh, to_world: &bevy::math::Affine3A) -> Option<(f32, f32)> {
    // (Not kept on the CPU (render-only meshes): none.)
    let positions = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION).ok()??.as_float3()?;
    let world: Vec<Vec3> = positions.iter().map(|p| to_world.transform_point3(Vec3::from(*p))).collect();
    let indices: Vec<usize> = match mesh.try_indices_option().ok()? {
        Some(i) => i.iter().collect(),
        None => (0..world.len()).collect(),
    };
    // (Vertices split for UVs or normals share a place: joined by it.)
    let key = |p: Vec3| ((p.x * 20.0).round() as i64, (p.y * 20.0).round() as i64, (p.z * 20.0).round() as i64);
    let mut ids = std::collections::HashMap::new();
    let id: Vec<usize> = world.iter().map(|p| {
        let n = ids.len();
        *ids.entry(key(*p)).or_insert(n)
    }).collect();
    let mut edges: std::collections::HashMap<(usize, usize), (u32, f32)> = std::collections::HashMap::new();
    let mut area = 0.0;
    for t in indices.chunks_exact(3) {
        let (a, b, c) = (world[t[0]], world[t[1]], world[t[2]]);
        area += (b - a).cross(c - a).length() * 0.5;
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            let (x, y) = (id[i].min(id[j]), id[i].max(id[j]));
            if x != y {
                edges.entry((x, y)).or_insert((0, world[i].distance(world[j]))).0 += 1;
            }
        }
    }
    // The outline: edges of one triangle only.
    let perimeter: f32 = edges.values().filter(|(n, _)| *n == 1).map(|(_, l)| l).sum();
    (area > 0.0 && perimeter > 0.0).then_some((area, perimeter))
}

/// Flat meshes' area and outline (their own space), measured when loaded:
/// a render-only mesh's data is gone once it's on the GPU.
#[derive(Resource, Default)]
pub struct MeshShapes(std::collections::HashMap<AssetId<Mesh>, (f32, f32)>);

fn remember_shapes(mut events: MessageReader<AssetEvent<Mesh>>, meshes: Res<Assets<Mesh>>, mut shapes: ResMut<MeshShapes>) {
    for ev in events.read() {
        let (AssetEvent::Added { id } | AssetEvent::Modified { id }) = ev else { continue };
        let Some(mesh) = meshes.get(*id) else { continue };
        let Some(Some(positions)) = mesh.try_attribute_option(Mesh::ATTRIBUTE_POSITION).ok().map(|p| p.and_then(|p| p.as_float3())) else { continue };
        // (Only sheets: one axis much thinner than the others.)
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for p in positions {
            lo = lo.min(Vec3::from(*p));
            hi = hi.max(Vec3::from(*p));
        }
        let size = hi - lo;
        if positions.len() < 4 || size.min_element() > size.max_element() * 0.05 {
            continue;
        }
        if let Some(shape) = outline(mesh, &bevy::math::Affine3A::IDENTITY) {
            shapes.0.insert(*id, shape);
        }
    }
}
