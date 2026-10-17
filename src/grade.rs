//! The colour grade on the [`crate::AtmosCamera`]: knobs `grade.exposure`
//! (stops), `grade.warmth`, `grade.tint`, `grade.contrast`,
//! `grade.saturation`. Left alone while none is set.

use bevy::prelude::*;
use bevy::render::view::ColorGrading;

pub struct GradePlugin;

impl Plugin for GradePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, grade.after(crate::LookSet));
    }
}

const KNOBS: [&str; 5] = ["grade.exposure", "grade.warmth", "grade.tint", "grade.contrast", "grade.saturation"];

fn grade(mut commands: Commands, mut cameras: Query<(Entity, Option<&mut ColorGrading>), With<crate::AtmosCamera>>, mut graded: Local<bool>) {
    let set = KNOBS.iter().any(|k| crate::knobs::is_set(k));
    if !set && !*graded {
        return;
    }
    *graded = set;
    let want = (
        crate::knobs::get("grade.exposure", 0.0).clamp(-6.0, 6.0),
        crate::knobs::get("grade.warmth", 0.0).clamp(-1.0, 1.0),
        crate::knobs::get("grade.tint", 0.0).clamp(-1.0, 1.0),
        crate::knobs::get("grade.contrast", 1.0).clamp(0.2, 3.0),
        crate::knobs::get("grade.saturation", 1.0).clamp(0.0, 3.0),
    );
    for (e, g) in &mut cameras {
        let Some(mut g) = g else {
            commands.entity(e).insert(ColorGrading::default());
            continue;
        };
        if g.global.exposure != want.0 {
            g.global.exposure = want.0;
        }
        if g.global.temperature != want.1 {
            g.global.temperature = want.1;
        }
        if g.global.tint != want.2 {
            g.global.tint = want.2;
        }
        if g.midtones.contrast != want.3 {
            g.midtones.contrast = want.3;
        }
        if g.global.post_saturation != want.4 {
            g.global.post_saturation = want.4;
        }
    }
}
