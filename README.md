# Atmos: sky, weather and water for Bevy

**bevy_atmos** gives a Bevy 0.19 game a living outdoor look in one plugin
line. It adds:

- a time of day that moves the sun
- a physical sky with volumetric clouds
- sun shafts and distance fog
- rain and snow that soak the ground into puddles
- modern water with waves, foam and flowing rivers
- a final colour grade

You tune all of it live, in game, from a beginner-friendly editor panel
(F9). Start from a preset like *Golden Hour* or *Storm*, tweak it, and save a
look per level as a small text file. It's free and open source (MIT or
Apache-2.0).

![Golden hour over the demo scene](media/hero.jpg)

## Features

### A look editor anyone can use

Press **F9** in your game. Six tabs (Time & Sun, Sky & Clouds, Weather,
Water, Fog & Light, Colour) hold the everyday settings, with the rest under
*Advanced*.

- Every slider says what it does and shows its unit, and one click resets it.
- Pick a preset, then tweak it.
- Hold *Compare* to see the level without your look, then *Save*.
- The status line always says which level's look is loaded and whether it
  has unsaved changes.
- It works with a mouse, the keyboard or a gamepad.

![The look editor](media/editor.jpg)

### Time of day and sun

Set the hour, or let the clock run. The sun follows a real path for your
latitude, and the moon lights the night. You can also pin the sun anywhere
for a shot.

![Night with the moon](media/night.jpg)

### Sky and volumetric clouds

A physical sky whose colours follow the sun, with 3D clouds raymarched per
pixel. Cover, greyness, wispiness, height and drift are all sliders.

![Overcast sky with volumetric clouds](media/clouds.jpg)

### Rain, snow and wet surfaces

Rain and snow fall round the camera and slant in the wind. Surfaces with
`WetMaterial` darken as they soak and collect puddles. The puddles ripple in
the rain and dry slowly after.

| Storm | Snow |
|---|---|
| ![Storm: rain, wet ground and puddles](media/storm.jpg) | ![Snow](media/snow.jpg) |

### Modern water and rivers

Mark your level's water and Atmos draws it as modern water:

- waves that follow the wind
- shore and wave-top foam
- sky and screen reflections, and sun glitter
- rain rings and underwater fog

Add `WaterFlow` and a river flows downhill. It breaks into white water on
fast stretches and leaves foam trails behind rocks. You can also paint flow
by hand in the editor.

| Rough water with crest foam | Shoreline foam |
|---|---|
| ![Rough water in the pool with foam on the wave crests](media/water.jpg) | ![Waves breaking into foam along the pool's edge](media/shore.jpg) |

| River: foam trailing a rock | River: low sun, looking downstream |
|---|---|
| ![A river flowing past a rock, foam trailing behind it](media/river.jpg) | ![Sun glitter on a river, looking downstream at sunset](media/river_glitter.jpg) |

### Fog, sun shafts and reflections

- Distance fog tinted by the sky.
- Beams of sunlight through the haze.
- Screen-space reflections on wet ground and shiny floors.

### Colour grade

Exposure, warmth, tint, contrast and saturation, applied as the last step.

## Requirements

- **Bevy 0.19**: the plugin depends on `bevy = "0.19.1"`.
- **Rust**: a recent stable toolchain (edition 2024). Install it from
  [rustup.rs](https://rustup.rs).
- **A GPU with compute shaders**: anything that runs Bevy's default renderer
  on Vulkan, DirectX 12 or Metal.
  - WebGL2 is not supported, because the clouds and the sky use compute
    passes.
  - The camera should be `Hdr`.

## Install

1. Add it to your game's `Cargo.toml`:

   ```toml
   [dependencies]
   bevy_atmos = { git = "https://github.com/Nilbong/bevy_atmos" }
   ```

2. Add the plugin (one line), then tag your camera and your sun:

   ```rust
   use bevy::prelude::*;
   use bevy_atmos::{AtmosCamera, AtmosPlugin, AtmosSun};

   fn main() {
       App::new()
           // The level's name, and the folder its saved look goes in.
           .add_plugins((DefaultPlugins, AtmosPlugin::new("my_level", "looks")))
           .add_systems(Startup, setup)
           .run();
   }

   fn setup(mut commands: Commands) {
       commands.spawn((Camera3d::default(), bevy::camera::Hdr, AtmosCamera));
       commands.spawn((DirectionalLight { shadow_maps_enabled: true, ..default() }, AtmosSun));
   }
   ```

3. Run your game and press **F9**.

To try it before adding it to a game, clone the repo and run the demo:

```sh
git clone https://github.com/Nilbong/bevy_atmos
cd bevy_atmos
cargo run --release --example demo
```

## Getting started

| Key | What it does |
|---|---|
| **F9** | Opens and closes the look editor. While it's open, the mouse pointer belongs to the panel. |
| **F8** | The fly camera: WASD to move, Space/E up, Ctrl/Q down, mouse to look, wheel for speed, Shift to go faster. Pad sticks and triggers work too. |

If your game already uses F9 or F8, pick other keys:
`AtmosPlugin::new("my_level", "looks").with_keys(KeyCode::F10, KeyCode::F11)`.

### Using the editor

- **Presets**: Clear Day, Overcast, Golden Hour, Night, Storm and Snow. Each
  one sets the time, sky, weather, fog and colour together. Start from one,
  then change anything.
- **Sliders**: drag them.
  - A changed value turns green and gets a *reset* link. Click it (or the
    value) to go back to the default.
  - Hover a slider to read what it does.
- **Advanced**: each tab's extra settings, such as cloud quality, rain
  streaks, foam and reflections.
- **The buttons along the bottom**:
  - **Save** writes the level's look.
  - **Revert** goes back to the last save.
  - **Hold to compare with original** shows the level with no look at all.
  - **Clear look** starts from scratch.
- **Keyboard**: arrows, Enter, PageUp/PageDown for tabs, Delete to reset,
  Ctrl+S to save, hold H to compare.
- **Gamepad**: Back + D-pad down opens the editor. Then D-pad, A, LB/RB for
  tabs, X to reset, Y to save, hold LT to compare, B to close.

### Where looks are saved

`AtmosPlugin::new(level, save_dir)` saves to `<save_dir>/<level>.txt`,
relative to the folder the game runs from. It's a plain text file you can
read, diff and commit:

```
# my_level's look (bevy_atmos)
time.hour = 17.5
sky.cover = 0.6
weather.rain = 0.4
```

Only the values you changed are written. Everything else stays at its
default, so a level without a saved look looks exactly as your game made it.

- **Changing level**: set the `AtmosLevel` resource to the new level's name
  and its look loads.
- **Shipping looks with your game**: pass them as `AtmosSettings::builtin`
  (`include_str!` of the files). A file in the save folder wins over a
  built-in one.

## Integration

Everything below is optional. The sky, clouds, fog, shafts, rain, snow and
grade work with just the camera and the sun tagged.

| What | How |
|---|---|
| **Wet surfaces** | Use `wet::WetMaterial` (a `StandardMaterial` extension) on surfaces that can get wet. `Wet::ground()` is for floors that collect puddles, `Wet::surface()` for walls and props. `Wet::new(allow, puddles)` lets you choose (allow 0: never wet). |
| **Water** | Put `modern_water::AtmosWater` on each water mesh. With *Modern water* on (Water tab), Atmos hides your mesh and draws modern water over the top of its bounds. Turn it off and your own water shows again. If you have no water material, `water::water_material(colour)` makes a simple one. |
| **Rivers** | Add `flow::WaterFlow` beside `AtmosWater`. The water then keeps the mesh's own (sloped) shape and flows. Give it the direction in one of four ways: `WaterFlow::uv_scroll(scroll)` (your water texture's scroll), `WaterFlow::along_uv(axis, speed)`, `WaterFlow::world(velocity)` or `WaterFlow::map(image, speed)` (a flow map). Flow painted in the editor (Water tab, *Paint river flow*) overrides these and saves as `<level>.flow`. |
| **Shelter map** | Insert `weather::Shelter`, a heights image round the origin, and rain stops at roofs and drips off their edges. Without one, rain falls everywhere. (With modern water on, Atmos makes one from the water, so rain stops at its surface.) |
| **Reflections (SSR)** | On by default for the tagged camera; it adds a depth prepass. Tune it or turn it off on the Fog & Light tab. |
| **Your own systems** | Read any value anywhere with `bevy_atmos::knobs::get("weather.rain", 0.0)`; `knobs::set` changes it. While `editor::open()` or `flycam::active()` is true, ignore your own mouse look and player input. |
| **Keys** | `AtmosSettings::editor_key` and `flycam_key` (default F9 and F8). |

[KNOBS.md](KNOBS.md) lists every knob's name, default and meaning.

## Limitations

Some honest notes, so nothing surprises you:

- **One camera and one sun**: Atmos uses the first `AtmosCamera` and the
  first `AtmosSun`. No split-screen yet.
- **Water is flat water plus waves**:
  - Modern water is a grid over the top of each marked mesh's bounds, in
    quarter-metre cells and at most 256 a side, so big seas get coarser
    waves.
  - Rivers keep their mesh's slope.
  - There are **no waterfalls** yet.
- **GPU-only meshes get no underwater fog in rivers**: finding the river
  surface under the camera needs the mesh's data on the CPU
  (`RenderAssetUsages` that keep the main-world copy).
- **Cloud smear on fast turns**: clouds march a quarter of their pixels each
  frame and reuse the rest, so a very fast camera turn smears briefly at the
  screen's edge.
- **Clouds take ~3 s to appear in full**: their noise is built on a thread at
  startup, and the sky's cheaper clouds show until it's ready.
- **Reflections show only what's on screen**, and are a frame old. Past the
  edge of the screen, water falls back to the sky and wet ground to the
  light probe.
- **Baked lightmaps don't follow the time of day**. Dynamic lights, the sky,
  fog and shafts do.
- **Forward rendering only**: wet and water materials have no deferred path.
- **Bevy internals**: the fog-before-transparents pass finds one of Bevy's
  systems by name, so a Bevy upgrade may need it checked.
- **Cost** (GPU time, measured in the demo at 1280x720):
  - Clouds take about 0.1 to 0.3 ms a frame and the sky about 0.15 ms.
  - Clouds scale with the pixel count, so expect about four times as much at
    1440p. Lower *Cloud quality* to save time.
  - Rain, wet surfaces, water and the grade are cheap by comparison.

## FAQ and troubleshooting

**The sky is black or far too bright.**
Add `bevy::camera::Hdr` to the camera that has `AtmosCamera`. The sky's
brightness is in real units.

**Nothing happens when I press F9.**
- Check that `AtmosPlugin` is added.
- Check that nothing else in your game uses F9. `AtmosSettings::editor_key`
  changes the key.
- The panel draws with `bevy_ui`, so the `bevy_ui` feature must be on. It is
  in `DefaultPlugins`.

**My camera spins when I drag a slider.**
Your mouse look is still running. Skip it while `bevy_atmos::editor::open()`
(or `flycam::active()`) is true.

**My water didn't change.**
Turn on *Modern water* on the Water tab, and make sure the water mesh entity
has `AtmosWater`.

**Rain falls through my roofs.**
Give Atmos a `weather::Shelter` heights map.

**The ground doesn't get wet.**
Only `WetMaterial` surfaces get wet. Swap `StandardMaterial` for
`WetMaterial { base, extension: Wet::ground() }`.

**My saved look didn't load.**
- The file is `<save_dir>/<level>.txt`, relative to the folder the game runs
  from. Check that the game runs from where you think.
- Check that the file name matches `AtmosLevel`.
- On load, the log prints `atmos: N knobs for <level>`.

**The clouds look blurry for the first few seconds.**
Their noise is still being built on a thread. It finishes in about three
seconds.

**It's slow on my laptop.**
- On Sky & Clouds > Advanced, lower *Cloud quality* or turn *Volumetric
  clouds* off.
- On Fog & Light, lower *Sun shaft quality* or turn *Screen reflections* off.

## Licence

bevy_atmos is free and open source. Anyone can use it in any project,
commercial or not, without asking.

It is dual-licensed under either of these, at your option (the same as
Bevy):

- the MIT licence ([LICENSE-MIT](LICENSE-MIT))
- the Apache License 2.0 ([LICENSE-APACHE](LICENSE-APACHE))

Contributions are accepted under the same terms.

## Credits

- Made by [Nilbong](https://github.com/Nilbong). It began as the look tools
  of a Bevy game and was split out so anyone can use it.
- Built on [Bevy](https://bevyengine.org) and its renderer.
- The cloud, sky and water techniques follow published real-time rendering
  work: raymarched cloud layers with temporal reprojection, Rayleigh and Mie sky
  scattering and Gerstner-style waves. The code here is written for Bevy.
