//! Test shots in any host (debug, from the environment; nothing happens
//! unless it's set):
//! `ATMOS_SEQ="a.png@x,y,z,tx,ty,tz@k=v,k=v;b.png@..."` takes the shots in
//! one launch: for each, the fly camera at x,y,z looking at tx,ty,tz (the
//! host's camera stands aside, as for F8; leave the camera out to keep the
//! last one), the knobs set, a screenshot 3 s later; then the app quits.
//! Knobs past the look's: `editor` (tab, -1 closed), `editor_adv` (0/1),
//! `preset` (index), `mark` (the Mark water tool, 0/1).
//! `ATMOS_PERF=1` logs the plugin's GPU pass times at each shot.

use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

pub struct TestShotsPlugin;

impl Plugin for TestShotsPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("ATMOS_SEQ").is_none() {
            return;
        }
        if std::env::var_os("ATMOS_PERF").is_some() {
            app.add_plugins((bevy::render::diagnostic::RenderDiagnosticsPlugin, bevy::diagnostic::FrameTimeDiagnosticsPlugin::default()));
        }
        app.init_resource::<Seq>().add_systems(Update, sequence.after(crate::LookSet));
    }
}

type Shot = (String, Option<(Vec3, Vec3)>, Vec<(String, f32)>);

#[derive(Resource, Default)]
struct Seq {
    shots: Vec<Shot>,
    at: usize,
    began: f32,
    parsed: bool,
    next: bool,
    done: bool,
}

fn place(fly: &mut crate::flycam::Flycam, cam: Option<(Vec3, Vec3)>) {
    if let Some((from, to)) = cam {
        let (yaw, pitch, _) = Transform::from_translation(from).looking_at(to, Vec3::Y).rotation.to_euler(EulerRot::YXZ);
        fly.place(from, yaw, pitch);
    }
}

fn set_knobs(knobs: &[(String, f32)], editor: &mut crate::editor::Editor) {
    for (k, v) in knobs {
        match k.as_str() {
            "editor" => editor.set_open(*v >= 0.0, v.max(0.0) as usize),
            "editor_adv" => editor.set_advanced(*v > 0.0),
            "preset" => crate::editor::apply_preset(*v as usize),
            "mark" => crate::auto::set_marking(*v > 0.5),
            _ => crate::knobs::set(k, Some(*v)),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sequence(
    mut commands: Commands,
    time: Res<Time<Real>>,
    mut seq: ResMut<Seq>,
    mut editor: ResMut<crate::editor::Editor>,
    mut fly: ResMut<crate::flycam::Flycam>,
    diagnostics: Res<bevy::diagnostic::DiagnosticsStore>,
    cameras: Query<&GlobalTransform, With<crate::AtmosCamera>>,
    mut exit: MessageWriter<AppExit>,
) {
    let t = time.elapsed_secs();
    if !seq.parsed {
        seq.parsed = true;
        let text = std::env::var("ATMOS_SEQ").unwrap_or_default();
        for shot in text.split(';').filter(|s| !s.trim().is_empty()) {
            let mut parts = shot.split('@');
            let file = parts.next().unwrap_or("shot.png").trim().to_owned();
            let cam = parts.next().map(|c| c.split(',').filter_map(|x| x.trim().parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 6);
            let cam = cam.map(|v| (Vec3::new(v[0], v[1], v[2]), Vec3::new(v[3], v[4], v[5])));
            let knobs = parts.next().map(|k| crate::knobs::parse(&k.replace(',', "\n")).into_iter().collect()).unwrap_or_default();
            seq.shots.push((file, cam, knobs));
        }
        // (A host's world takes a while to load: `ATMOS_SEQ_START` seconds.)
        let start = std::env::var("ATMOS_SEQ_START").ok().and_then(|s| s.parse().ok()).unwrap_or(2.0);
        seq.began = t + start;
        seq.at = usize::MAX;
        return;
    }
    if seq.shots.is_empty() {
        return;
    }
    if seq.done {
        if t - seq.began > 1.0 {
            exit.write(AppExit::Success);
        }
        return;
    }
    if seq.at == usize::MAX {
        if t < seq.began {
            return;
        }
        seq.at = 0;
        seq.began = t;
        let (_, cam, knobs) = seq.shots[0].clone();
        place(&mut fly, cam);
        set_knobs(&knobs, &mut editor);
        return;
    }
    if seq.next {
        seq.next = false;
        let (_, cam, knobs) = seq.shots[seq.at].clone();
        place(&mut fly, cam);
        set_knobs(&knobs, &mut editor);
        return;
    }
    if t - seq.began < 3.0 {
        return;
    }
    let file = seq.shots[seq.at].0.clone();
    commands.spawn(Screenshot::primary_window()).observe(save_to_disk(file.clone()));
    if let Some(c) = cameras.iter().next() {
        info!("atmos shot {file}: camera at {:.1}, looking {:.2}", c.translation(), c.forward().as_vec3());
    }
    if std::env::var_os("ATMOS_PERF").is_some() {
        let get = |path: &'static str| diagnostics.get(&bevy::diagnostic::DiagnosticPath::const_new(path)).and_then(|d| d.average()).unwrap_or(0.0);
        info!(
            "atmos perf {file}: clouds {:.3} ms, sky {:.3} ms, ao {:.3} ms, frame {:.2} ms",
            get("render/atmos_clouds/elapsed_gpu"),
            get("render/atmos_sky/elapsed_gpu"),
            get("render/atmos_ao_direct/elapsed_gpu"),
            get("frame_time")
        );
    }
    seq.at += 1;
    seq.began = t;
    if seq.at >= seq.shots.len() {
        // (A second for the last screenshot to be written.)
        seq.done = true;
        return;
    }
    // (The next shot's camera and knobs next frame: this frame renders the
    // screenshot.)
    seq.next = true;
}
