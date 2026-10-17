//! A free-flying camera for judging the look and taking screenshots: the
//! [`crate::AtmosCamera`] leaves whatever placed it and flies. The world
//! keeps running. While [`active`], the host should ignore its own player
//! input and may hide its HUD; on exit the host's camera code takes over
//! again (the camera's transform is only written while flying).
//!
//! Keys (F8 toggles): WASD, Space/E up, Ctrl/Q down, mouse to look, wheel
//! for speed, Shift faster. Pad: left stick moves, right stick looks, RT up,
//! LT down, LB/RB speed, L3 faster. [`Flycam::hold_to_fly`]: the pointer
//! stays free and the camera only flies while the right button is held
//! (for editors that click).

use bevy::input::gamepad::{Gamepad, GamepadAxis, GamepadButton};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use std::sync::atomic::{AtomicBool, Ordering};

pub struct FlycamPlugin;

impl Plugin for FlycamPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Flycam>()
            .add_systems(Update, keys)
            .add_systems(PostUpdate, fly.before(bevy::transform::TransformSystems::Propagate));
    }
}

static ACTIVE: AtomicBool = AtomicBool::new(false);

/// The fly camera has the view: the host's own controls should pause.
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

#[derive(Resource)]
pub struct Flycam {
    want: bool,
    on: bool,
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    at: Option<(Vec3, f32, f32)>,
    /// Metres a second.
    pub speed: f32,
    /// Fly only while the right mouse button is held; the pointer stays free.
    pub hold_to_fly: bool,
}

impl Default for Flycam {
    fn default() -> Self {
        Flycam { want: false, on: false, pos: Vec3::ZERO, yaw: 0.0, pitch: 0.0, at: None, speed: 6.0, hold_to_fly: false }
    }
}

impl Flycam {
    /// Flies from where the camera is.
    pub fn enter(&mut self) {
        self.want = true;
    }

    /// Flies from `pos`, facing `yaw` and `pitch` (radians).
    pub fn enter_at(&mut self, pos: Vec3, yaw: f32, pitch: f32) {
        self.want = true;
        self.at = Some((pos, yaw, pitch));
    }

    pub fn exit(&mut self) {
        self.want = false;
        self.hold_to_fly = false;
        ACTIVE.store(false, Ordering::Relaxed);
    }

    pub fn toggle(&mut self) {
        if self.want { self.exit() } else { self.enter() }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn position(&self) -> Vec3 {
        self.pos
    }
}

fn keys(keys: Res<ButtonInput<KeyCode>>, settings: Res<crate::AtmosSettings>, mut f: ResMut<Flycam>) {
    if keys.just_pressed(settings.flycam_key) && !f.hold_to_fly {
        f.toggle();
    }
}

#[allow(clippy::too_many_arguments)]
fn fly(
    time: Res<Time<Real>>,
    mut f: ResMut<Flycam>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    pads: Query<&Gamepad>,
    mut camera: Query<&mut Transform, With<crate::AtmosCamera>>,
    mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let Ok(mut tf) = camera.single_mut() else { return };
    if f.want && !f.on {
        f.on = true;
        match f.at.take() {
            Some((pos, yaw, pitch)) => (f.pos, f.yaw, f.pitch) = (pos, yaw, pitch),
            None => {
                f.pos = tf.translation;
                let (yaw, pitch, _) = tf.rotation.to_euler(EulerRot::YXZ);
                (f.yaw, f.pitch) = (yaw, pitch);
            }
        }
        if !f.hold_to_fly {
            if let Ok(mut c) = cursor.single_mut() {
                c.grab_mode = CursorGrabMode::Locked;
                c.visible = false;
            }
        }
        ACTIVE.store(true, Ordering::Relaxed);
    }
    if !f.want && f.on {
        f.on = false;
        ACTIVE.store(false, Ordering::Relaxed);
    }
    if !f.on {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let flying = !f.hold_to_fly || buttons.pressed(MouseButton::Right);
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    let pad = pads.iter().next();
    let axis = |a: GamepadAxis| pad.and_then(|p| p.get(a)).filter(|v| v.abs() > 0.15).unwrap_or(0.0);
    let held = |b: GamepadButton| pad.is_some_and(|p| p.pressed(b));
    let pressed = |b: GamepadButton| pad.is_some_and(|p| p.just_pressed(b));
    if flying && (grabbed || f.hold_to_fly) {
        f.yaw -= motion.delta.x * 0.0022;
        f.pitch -= motion.delta.y * 0.0022;
    }
    f.yaw -= axis(GamepadAxis::RightStickX) * 2.2 * dt;
    f.pitch += axis(GamepadAxis::RightStickY) * 1.6 * dt;
    f.pitch = f.pitch.clamp(-1.55, 1.55);
    let notches = if flying { scroll.delta.y } else { 0.0 } + pressed(GamepadButton::RightTrigger) as i32 as f32 - pressed(GamepadButton::LeftTrigger) as i32 as f32;
    if notches != 0.0 {
        f.speed = (f.speed * 1.25f32.powf(notches)).clamp(0.5, 200.0);
    }
    let boost = if keys.pressed(KeyCode::ShiftLeft) || held(GamepadButton::LeftThumb) { 4.0 } else { 1.0 };
    let key = |k: KeyCode| (flying && keys.pressed(k)) as i32 as f32;
    let rot = Quat::from_euler(EulerRot::YXZ, f.yaw, f.pitch, 0.0);
    let up = key(KeyCode::Space) + key(KeyCode::KeyE) - key(KeyCode::ControlLeft) - key(KeyCode::KeyQ)
        + pad.and_then(|p| p.get(GamepadButton::RightTrigger2)).unwrap_or(0.0)
        - pad.and_then(|p| p.get(GamepadButton::LeftTrigger2)).unwrap_or(0.0);
    let ahead = key(KeyCode::KeyW) - key(KeyCode::KeyS) + axis(GamepadAxis::LeftStickY);
    let side = key(KeyCode::KeyD) - key(KeyCode::KeyA) + axis(GamepadAxis::LeftStickX);
    let step = (rot * Vec3::NEG_Z * ahead + rot * Vec3::X * side + Vec3::Y * up).clamp_length_max(1.0) * f.speed * boost * dt;
    f.pos += step;
    tf.translation = f.pos;
    tf.rotation = rot;
}
