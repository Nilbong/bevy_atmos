//! bevy_atmos's demo: a ground plane, a few boxes, a pool and a sun.
//! The pool's water is marked `AtmosWater`: the modern water replaces it
//! (Water tab, "Modern water").
//! F9 opens the look editor, F8 flies the camera.
//!
//! `cargo run --example demo -p bevy_atmos`
//!
//! For screenshots: `ATMOS_SET="time.hour=17;weather.rain=0.6"` sets
//! knobs, `ATMOS_EDITOR=<tab>` opens the editor on a tab, and
//! `ATMOS_SHOT=<file.png>` saves a screenshot a few seconds in and quits,
//! `ATMOS_CAM="x,y,z,tx,ty,tz"` holds the camera at x,y,z looking at t.
//! `ATMOS_SEQ="a.png@x,y,z,tx,ty,tz@k=v,k=v;b.png@..."` takes several
//! shots in one launch (each 3 s after its camera and knobs are set) and
//! quits; `ATMOS_PERF=1` logs the plugin's GPU pass times at each shot
//! (and turns vsync off); `ATMOS_RES=640x360` a smaller window. In a
//! shot's knobs, `editor=<tab>` opens the editor on that tab (-1 closes it)
//! `editor_adv=1` shows the tab's Advanced settings and `preset=<n>` applies
//! a preset (0 Clear Day .. 5 Snow).

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy_atmos::wet::{Wet, WetMaterial};
use bevy_atmos::{AtmosCamera, AtmosPlugin, AtmosSun};

fn main() {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
            // (`ATMOS_RES=640x360`: a smaller window, for test runs.)
            primary_window: Some(Window {
                title: "bevy_atmos demo".into(),
                resolution: res().into(),
                present_mode: if perf() { bevy::window::PresentMode::AutoNoVsync } else { bevy::window::PresentMode::AutoVsync },
                ..default()
            }),
            ..default()
        }))
        .add_plugins(AtmosPlugin::new("demo", "looks"));
    if perf() {
        app.add_plugins((bevy::render::diagnostic::RenderDiagnosticsPlugin, bevy::diagnostic::FrameTimeDiagnosticsPlugin::default()));
    }
    app.init_resource::<Seq>().add_systems(Startup, setup).add_systems(Update, (sequence, orbit, shoot).chain()).run();
}

fn perf() -> bool {
    std::env::var_os("ATMOS_PERF").is_some()
}

/// `ATMOS_SEQ`: the shots, the one now and when it began.
#[derive(Resource, Default)]
struct Seq {
    shots: Vec<(String, Option<Transform>, Vec<(String, f32)>)>,
    at: usize,
    began: f32,
    camera: Option<Transform>,
    parsed: bool,
    next: bool,
}

fn sequence(mut commands: Commands, time: Res<Time<Real>>, mut seq: ResMut<Seq>, mut editor: ResMut<bevy_atmos::editor::Editor>, diagnostics: Res<bevy::diagnostic::DiagnosticsStore>, mut exit: MessageWriter<AppExit>) {
    if !seq.parsed {
        seq.parsed = true;
        let Ok(text) = std::env::var("ATMOS_SEQ") else { return };
        for shot in text.split(';').filter(|s| !s.trim().is_empty()) {
            let mut parts = shot.split('@');
            let file = parts.next().unwrap_or("shot.png").trim().to_owned();
            let cam = parts.next().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 6);
            let cam = cam.map(|v| Transform::from_xyz(v[0], v[1], v[2]).looking_at(Vec3::new(v[3], v[4], v[5]), Vec3::Y));
            let knobs = parts.next().map(|k| bevy_atmos::knobs::parse(&k.replace(',', "\n")).into_iter().collect()).unwrap_or_default();
            seq.shots.push((file, cam, knobs));
        }
        seq.began = time.elapsed_secs() + 2.0;
        seq.at = usize::MAX;
        return;
    }
    if seq.shots.is_empty() {
        return;
    }
    let t = time.elapsed_secs();
    if seq.at == usize::MAX {
        if t < seq.began {
            return;
        }
        seq.at = 0;
        seq.began = t;
        let (_, cam, knobs) = seq.shots[0].clone();
        seq.camera = cam;
        set_knobs(knobs, &mut editor);
        return;
    }
    if seq.next {
        seq.next = false;
        let (_, cam, knobs) = seq.shots[seq.at].clone();
        seq.camera = cam.or(seq.camera);
        set_knobs(knobs, &mut editor);
        return;
    }
    if seq.at == usize::MAX - 1 {
        if t - seq.began > 1.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    if t - seq.began < 3.0 {
        return;
    }
    let (file, _, _) = seq.shots[seq.at].clone();
    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(file.clone()));
    if perf() {
        let get = |path: &'static str| diagnostics.get(&bevy::diagnostic::DiagnosticPath::const_new(path)).and_then(|d| d.average()).unwrap_or(0.0);
        info!(
            "atmos perf {file}: clouds {:.3} ms, sky {:.3} ms, frame {:.2} ms",
            get("render/atmos_clouds/elapsed_gpu"),
            get("render/atmos_sky/elapsed_gpu"),
            get("frame_time")
        );
    }
    seq.at += 1;
    seq.began = t;
    if seq.at >= seq.shots.len() {
        // (A second for the last screenshot to be written.)
        seq.at = usize::MAX - 1;
        return;
    }
    // (The next shot's camera and knobs next frame: this frame renders the
    // screenshot.)
    seq.next = true;
}

/// A shot's knobs (`editor`, `editor_adv`: the editor).
fn set_knobs(knobs: Vec<(String, f32)>, editor: &mut bevy_atmos::editor::Editor) {
    for (k, v) in knobs {
        match k.as_str() {
            "editor" => editor.set_open(v >= 0.0, v.max(0.0) as usize),
            "editor_adv" => editor.set_advanced(v > 0.0),
            "preset" => bevy_atmos::editor::apply_preset(v as usize),
            _ => bevy_atmos::knobs::set(&k, Some(v)),
        }
    }
}

fn res() -> (u32, u32) {
    std::env::var("ATMOS_RES").ok().and_then(|r| r.split_once('x').and_then(|(w, h)| Some((w.trim().parse().ok()?, h.trim().parse().ok()?)))).unwrap_or((1280, 720))
}

fn setup(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut wet: ResMut<Assets<WetMaterial>>,
    mut water: ResMut<Assets<bevy_atmos::water::WaterMaterial>>,
) {
    let ground = wet.add(WetMaterial { base: StandardMaterial { base_color: Color::srgb(0.42, 0.4, 0.36), perceptual_roughness: 0.9, ..default() }, extension: Wet::ground() });
    let stone = wet.add(WetMaterial { base: StandardMaterial { base_color: Color::srgb(0.36, 0.35, 0.32), perceptual_roughness: 0.85, ..default() }, extension: Wet::surface() });
    // The ground, round a pool sunk in it (x -9..5, z 9..19).
    // (West of the pool, a gap for the river's channel.)
    for (w, d, x, z) in [(200.0, 109.0, 0.0, -45.5), (200.0, 81.0, 0.0, 59.5), (91.0, 3.3, -54.5, 10.65), (91.0, 3.3, -54.5, 17.35), (69.8, 3.4, -65.1, 14.0), (95.0, 10.0, 52.5, 14.0)] {
        commands.spawn((Mesh3d(meshes.add(Plane3d::default().mesh().size(w, d))), MeshMaterial3d(ground.clone()), Transform::from_xyz(x, 0.0, z)));
    }
    // The pool: walls on three sides and a floor sloping up to a beach at
    // the near end (deep 3 m at the far wall), a block standing in it.
    // (The west wall low where the river comes in.)
    for (size, at) in [
        (Vec3::new(0.4, 3.4, 3.5), Vec3::new(-9.2, -1.7, 10.75)),
        (Vec3::new(0.4, 3.4, 3.5), Vec3::new(-9.2, -1.7, 17.25)),
        (Vec3::new(0.4, 1.9, 3.0), Vec3::new(-9.2, -2.45, 14.0)),
        (Vec3::new(0.4, 3.4, 10.0), Vec3::new(5.2, -1.7, 14.0)),
        (Vec3::new(14.8, 3.4, 0.4), Vec3::new(-2.0, -1.7, 8.8)),
    ] {
        commands.spawn((Mesh3d(meshes.add(Cuboid::from_size(size))), MeshMaterial3d(stone.clone()), Transform::from_translation(at)));
    }
    let tilt = (3.05f32 / 10.0).atan();
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(14.0, 10.6))),
        MeshMaterial3d(stone.clone()),
        Transform::from_xyz(-2.0, -1.5, 14.0).with_rotation(Quat::from_rotation_x(-tilt)),
    ));
    commands.spawn((Mesh3d(meshes.add(Cuboid::new(1.6, 4.0, 1.6))), MeshMaterial3d(stone.clone()), Transform::from_xyz(-4.0, -1.0, 12.5)));
    let colours = [Color::srgb(0.7, 0.25, 0.2), Color::srgb(0.25, 0.45, 0.7), Color::srgb(0.75, 0.7, 0.55), Color::srgb(0.3, 0.55, 0.3)];
    for (i, (x, z, h)) in [(-6.0, -4.0, 3.0), (5.0, -7.0, 6.0), (2.0, 4.0, 1.5), (-9.0, 6.0, 4.0), (12.0, 2.0, 9.0), (-14.0, -12.0, 12.0)].into_iter().enumerate() {
        let m = wet.add(WetMaterial { base: StandardMaterial { base_color: colours[i % colours.len()], perceptual_roughness: 0.6, ..default() }, extension: Wet::surface() });
        commands.spawn((Mesh3d(meshes.add(Cuboid::new(3.0, h, 3.0))), MeshMaterial3d(m), Transform::from_xyz(x, h / 2.0, z)));
    }
    // The pool's water: a plain "level" water, marked AtmosWater so the
    // modern water replaces it while `water.replace` is on.
    let pool = water.add(bevy_atmos::water::water_material(Color::srgba(0.05, 0.12, 0.14, 0.85)));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(14.0, 10.0))),
        MeshMaterial3d(pool),
        Transform::from_xyz(-2.0, -0.25, 14.0),
        bevy_atmos::modern_water::AtmosWater,
    ));
    // A river: a sloped channel 21 m long dropping into the pool from the
    // west, two rocks in it. Its water is marked AtmosWater with a
    // WaterFlow (here one world velocity; a game would pass its water's UV
    // scroll, `WaterFlow::uv_scroll`, or `WaterFlow::along_uv`).
    // (Dark wet earth for the bed, 1.2 m under the water, and dark wet
    // stone for the banks and rocks.)
    let earth = wet.add(WetMaterial { base: StandardMaterial { base_color: Color::srgb(0.15, 0.12, 0.09), perceptual_roughness: 0.55, ..default() }, extension: Wet::new(0.0, 0.0) });
    let rock = wet.add(WetMaterial { base: StandardMaterial { base_color: Color::srgb(0.2, 0.19, 0.17), perceptual_roughness: 0.4, ..default() }, extension: Wet::surface() });
    let length = 20.8f32;
    let surface = (1.65f32 / length).atan();
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(length, 3.0))),
        MeshMaterial3d(earth),
        Transform::from_xyz(-19.6, 0.575 - 1.2, 14.0).with_rotation(Quat::from_rotation_z(-surface)),
    ));
    for (size, at) in [(Vec3::new(length, 3.4, 0.2), Vec3::new(-19.6, -0.1, 12.4)), (Vec3::new(length, 3.4, 0.2), Vec3::new(-19.6, -0.1, 15.6)), (Vec3::new(0.2, 3.4, 3.4), Vec3::new(-30.1, -0.1, 14.0))] {
        commands.spawn((Mesh3d(meshes.add(Cuboid::from_size(size))), MeshMaterial3d(rock.clone()), Transform::from_translation(at)));
    }
    for (r, at) in [(0.75, Vec3::new(-22.0, 0.3, 14.3)), (0.6, Vec3::new(-14.5, -0.25, 13.5))] {
        commands.spawn((Mesh3d(meshes.add(Sphere::new(r))), MeshMaterial3d(rock.clone()), Transform::from_translation(at).with_scale(Vec3::new(1.3, 1.0, 1.0))));
    }
    let river = water.add(bevy_atmos::water::water_material(Color::srgba(0.05, 0.12, 0.14, 0.85)));
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(length, 3.0).subdivisions(8))),
        MeshMaterial3d(river),
        Transform::from_xyz(-19.6, 0.575, 14.0).with_rotation(Quat::from_rotation_z(-surface)),
        bevy_atmos::modern_water::AtmosWater,
        bevy_atmos::flow::WaterFlow::world(Vec2::new(1.8, 0.0)),
    ));
    if !bevy_atmos::knobs::is_set("water.replace") {
        bevy_atmos::knobs::set("water.replace", Some(1.0));
    }
    commands.spawn((
        DirectionalLight { illuminance: 12_000.0, shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(Vec3::new(-0.4, -0.6, -0.5), Vec3::Y),
        AtmosSun,
    ));
    commands.insert_resource(GlobalAmbientLight { brightness: 800.0, ..default() });
    commands.spawn((Camera3d::default(), bevy::camera::Hdr, Transform::from_xyz(0.0, 3.0, 26.0).looking_at(Vec3::new(0.0, 3.0, 0.0), Vec3::Y), AtmosCamera));
    // Debug: knobs and the editor from the environment (screenshots).
    if let Ok(set) = std::env::var("ATMOS_SET") {
        for (k, v) in bevy_atmos::knobs::parse(&set.replace(';', "\n")) {
            bevy_atmos::knobs::set(&k, Some(v));
        }
    }
}

/// A slow turn round the scene until the fly camera takes over.
fn orbit(time: Res<Time>, seq: Res<Seq>, mut cameras: Query<&mut Transform, With<AtmosCamera>>) {
    if let Some(at) = seq.camera {
        for mut t in &mut cameras {
            *t = at;
        }
        return;
    }
    if bevy_atmos::flycam::active() || bevy_atmos::editor::open() && std::env::var_os("ATMOS_SHOT").is_none() {
        return;
    }
    // (Debug: `ATMOS_CAM="x,y,z,tx,ty,tz"` holds the camera there.)
    if let Some(v) = std::env::var("ATMOS_CAM").ok().map(|s| s.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 6) {
        for mut t in &mut cameras {
            *t = Transform::from_xyz(v[0], v[1], v[2]).looking_at(Vec3::new(v[3], v[4], v[5]), Vec3::Y);
        }
        return;
    }
    let a = time.elapsed_secs() * 0.05;
    for mut t in &mut cameras {
        *t = Transform::from_xyz(26.0 * a.sin(), 3.0, 26.0 * a.cos()).looking_at(Vec3::new(0.0, 3.0, 0.0), Vec3::Y);
    }
}

fn shoot(mut commands: Commands, time: Res<Time<Real>>, mut editor: ResMut<bevy_atmos::editor::Editor>, mut step: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let Ok(path) = std::env::var("ATMOS_SHOT") else { return };
    let t = time.elapsed_secs();
    if *step == 0 && t > 2.0 {
        if let Some(tab) = std::env::var("ATMOS_EDITOR").ok().and_then(|v| v.parse().ok()) {
            editor.set_open(true, tab);
        }
        *step = 1;
    }
    if *step == 1 && t > 7.0 {
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        *step = 2;
    }
    if *step == 2 && t > 8.5 {
        exit.write(AppExit::Success);
    }
}
