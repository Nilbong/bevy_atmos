//! The air: distance fog and sun shafts on the [`crate::AtmosCamera`].
//!
//! - Fog (`fog.distance`, metres to half-hidden; 0 or unset leaves the
//!   host's `DistanceFog` alone): an exponential fog from `fog.start`
//!   metres, its colour from the sky's horizon toward the sun (and
//!   `sky.tint`, the sky and fog colour),
//!   and `fog.brightness` on it.
//! - Sun shafts (`shafts.strength`, 0 off): Bevy's volumetric fog in a
//!   volume round the camera lit by the sun, `shafts.steps` raymarch steps.
//!   The pass is moved before the transparent one ([`FogBeforeTransparents`]),
//!   so smoke and glass draw over the shafts and not the shafts of what's
//!   behind them over them.

use bevy::light::{FogVolume, VolumetricFog, VolumetricLight};
use bevy::prelude::*;

pub struct AirPlugin;

impl Plugin for AirPlugin {
    fn build(&self, app: &mut App) {
        if let Some(render_app) = app.get_sub_app_mut(bevy::render::RenderApp) {
            render_app.edit_schedule(bevy::core_pipeline::schedule::Core3d, |s| {
                s.add_build_pass(FogBeforeTransparents);
            });
        }
        app.add_systems(Update, (fog, shafts).after(crate::LookSet))
            .add_systems(PostUpdate, follow.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// The horizon's colour (linear, the frame's units before exposure) the sky
/// last drew: the fog takes it.
#[derive(Resource, Clone, Copy, Default)]
pub struct Horizon(pub Vec3);

fn fog(mut commands: Commands, tod: Res<crate::time::TimeOfDay>, horizon: Option<Res<Horizon>>, mut cameras: Query<(Entity, Option<&mut DistanceFog>), With<crate::AtmosCamera>>) {
    let distance = crate::knobs::get("fog.distance", 0.0);
    if distance <= 0.0 {
        return;
    }
    let start = crate::knobs::get("fog.start", 0.0).max(0.0);
    // (The fog is the sky's colour at the horizon: the sky tint warms or
    // cools both.)
    let warm = crate::knobs::get("sky.tint", 0.0).clamp(-1.0, 1.0);
    let bright = crate::knobs::get("fog.brightness", 1.0).clamp(0.0, 4.0);
    // The horizon's colour, the day's light on it.
    let base = horizon.map_or(Vec3::new(0.55, 0.62, 0.72), |h| h.0) * (1.0 - 0.85 * tod.night).max(0.05);
    let c = base * Vec3::new(1.0 + 0.3 * warm, 1.0, 1.0 - 0.3 * warm) * bright;
    let color = Color::linear_rgba(c.x, c.y, c.z, 1.0);
    let sun = tod.sun_color * tod.sun_share * 0.4;
    for (e, f) in &mut cameras {
        let falloff = FogFalloff::Exponential { density: std::f32::consts::LN_2 / distance.max(1.0) };
        match f {
            Some(mut f) => {
                f.color = color;
                f.falloff = falloff;
                f.directional_light_color = Color::linear_rgba(sun.x, sun.y, sun.z, 0.5);
                f.directional_light_exponent = 12.0;
            }
            None => {
                commands.entity(e).insert(DistanceFog { color, falloff, ..default() });
            }
        }
        let _ = start;
    }
}

/// The shafts' volume round the camera.
#[derive(Component)]
struct ShaftAir;

/// How far round the camera the shafts reach (m).
const REACH: f32 = 60.0;

#[allow(clippy::type_complexity)]
fn shafts(
    mut commands: Commands,
    cameras: Query<(Entity, Option<&VolumetricFog>), With<crate::AtmosCamera>>,
    suns: Query<(Entity, Has<VolumetricLight>), With<crate::AtmosSun>>,
    mut air: Query<(Entity, &mut FogVolume), With<ShaftAir>>,
    tod: Res<crate::time::TimeOfDay>,
) {
    let strength = crate::knobs::get("shafts.strength", 1.0).clamp(0.0, 5.0);
    let steps = crate::knobs::get("shafts.steps", 24.0).clamp(4.0, 96.0) as u32;
    let on = strength > 0.0;
    for (e, fog) in &cameras {
        match (on, fog) {
            (true, Some(f)) if f.step_count == steps => {}
            (true, _) => {
                commands.entity(e).insert(VolumetricFog { ambient_intensity: 0.0, jitter: 0.0, step_count: steps, ..default() });
            }
            (false, Some(_)) => {
                commands.entity(e).remove::<VolumetricFog>();
            }
            _ => {}
        }
    }
    for (e, has) in &suns {
        if on && !has {
            commands.entity(e).insert(VolumetricLight);
        } else if !on && has {
            commands.entity(e).remove::<VolumetricLight>();
        }
    }
    let density = 0.02 * strength * (1.0 - 0.7 * tod.night);
    match (on, air.single_mut()) {
        (true, Ok((_, mut v))) => {
            if (v.density_factor - density).abs() > 1e-5 {
                v.density_factor = density;
            }
        }
        (true, Err(_)) => {
            commands.spawn((
                Name::new("atmos shafts"),
                ShaftAir,
                FogVolume { density_factor: density, absorption: 0.05, scattering: 0.4, scattering_asymmetry: 0.6, ..default() },
                Transform::from_scale(Vec3::splat(REACH * 2.0)),
            ));
        }
        (false, Ok((e, _))) => {
            commands.entity(e).despawn();
        }
        _ => {}
    }
}

/// The shafts' volume stays round the camera.
fn follow(cameras: Query<&GlobalTransform, With<crate::AtmosCamera>>, mut air: Query<(&mut Transform, &mut GlobalTransform), (With<ShaftAir>, Without<crate::AtmosCamera>)>) {
    let Some(at) = cameras.iter().next().map(|g| g.translation()) else { return };
    for (mut t, mut g) in &mut air {
        t.translation = at;
        *g = GlobalTransform::from(*t);
    }
}

/// Moves Bevy's `volumetric_fog` render system from after the main pass to
/// before its transparent part, so transparent things draw over the fog.
/// (Bevy's system isn't public: found by name in the flattened graph.)
#[derive(Debug)]
pub struct FogBeforeTransparents;

impl bevy::ecs::schedule::ScheduleBuildPass for FogBeforeTransparents {
    type EdgeOptions = ();

    fn add_dependency(&mut self, _from: bevy::ecs::schedule::NodeId, _to: bevy::ecs::schedule::NodeId, _options: Option<&Self::EdgeOptions>) {}

    fn collapse_set(
        &mut self,
        _set: bevy::ecs::schedule::SystemSetKey,
        _systems: &indexmap::IndexSet<bevy::ecs::schedule::SystemKey, bevy::platform::hash::FixedHasher>,
        _dependency_flattening: &bevy::ecs::schedule::graph::DiGraph<bevy::ecs::schedule::NodeId>,
    ) -> impl Iterator<Item = (bevy::ecs::schedule::NodeId, bevy::ecs::schedule::NodeId)> {
        std::iter::empty()
    }

    fn build(
        &mut self,
        _world: &mut World,
        graph: &mut bevy::ecs::schedule::ScheduleGraph,
        mut deps: bevy::ecs::schedule::FlattenedDependencies<'_>,
    ) -> Result<(), bevy::ecs::schedule::ScheduleBuildError> {
        use bevy::ecs::schedule::NodeId;
        let name = |k| graph.get_node_name(&NodeId::System(k));
        let (fog, transparent) = {
            let Ok((order, _)) = deps.toposort_and_graph() else { return Ok(()) };
            let order = order.to_vec();
            let fog = order.iter().copied().find(|&k| {
                let n = name(k);
                n.contains("volumetric_fog::render") && n.ends_with("::volumetric_fog")
            });
            let transparent = order.iter().copied().find(|&k| name(k).ends_with("::main_transparent_pass_3d"));
            (fog, transparent)
        };
        let (Some(fog), Some(transparent)) = (fog, transparent) else { return Ok(()) };
        let after: Vec<_> = {
            let Ok((_, g)) = deps.toposort_and_graph() else { return Ok(()) };
            let mut seen = vec![transparent];
            let mut i = 0;
            while i < seen.len() {
                for n in g.neighbors_directed(seen[i], bevy::ecs::schedule::graph::Direction::Outgoing) {
                    if n != fog && !seen.contains(&n) {
                        seen.push(n);
                    }
                }
                i += 1;
            }
            seen.into_iter().filter(|&k| g.contains_edge(k, fog)).collect()
        };
        for k in after {
            deps.remove_edge(k, fog);
        }
        deps.add_edge(fog, transparent);
        Ok(())
    }
}
