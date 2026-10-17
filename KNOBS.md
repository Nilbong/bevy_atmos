# bevy_atmos reference: inputs and knobs

Every value the look uses is a **knob**, read anywhere with
`bevy_atmos::knobs::get(name, default)`. The editor (F9) sets them; a level's
saved look is a text file of the ones it changes. The editor shows each
under a friendlier label (hover it: the help line names its knob); the
file uses these names.

## Inputs from the game

| What | How |
|---|---|
| The camera the look applies to | `AtmosCamera` on it (`Hdr` recommended). Its `Skybox` image, if any, is the sky's base colours. |
| The sun | `AtmosSun` on a `DirectionalLight`. Its direction is the level's own hour. |
| Level name (names the save file) | `AtmosPlugin::new(level, save_dir)`; change `AtmosLevel` when the level changes and its look loads. |
| Save folder, built-in looks | `AtmosSettings { save_dir, builtin, .. }`: `builtin` is `&[(file name, text)]` (e.g. `include_str!`) read when the folder has none. |
| Surfaces that get wet | `wet::WetMaterial` (a `StandardMaterial` extension): `Wet::ground()` (puddles), `Wet::surface()`, `Wet::new(allow, puddles)` (allow 0: never wet). |
| The level's water | `modern_water::AtmosWater` on each water mesh entity. With `water.replace` on, the plugin hides it and draws modern water (a fine grid over the top of its bounds, moved by the waves) in its place; off, the original is shown again. Modern water adds `DepthPrepass` to the camera. |
| Rivers (flowing water) | Add `flow::WaterFlow` beside `AtmosWater` on the river's mesh entity. The modern water then keeps that mesh's own shape (sloped water) and flows. Where the flow comes from: `WaterFlow::uv_scroll(scroll)` takes the original material's texture scroll in UV/s (it flows the way the texture appeared to move, metres from the mesh's UV scale); `WaterFlow::along_uv(axis, speed)` flows along a UV axis of the mesh at `speed` m/s; `WaterFlow::world(velocity)` takes one world velocity (x, z, m/s); `WaterFlow::map(image, speed)` reads a flow map over the mesh's UVs (RG = UV-space direction, 0.5 still). Flow painted in the editor overrides all of these. |
| A simple water material | `water::WaterMaterial`, made with `water::water_material(colour)` (the water you get when `water.replace` is off). |
| Where rain can't fall | `weather::Shelter` (a heights image round the origin). Without one, rain falls everywhere and nothing drips; with modern water on, the plugin makes one from the water so the rain stops and splashes at its surface. |

While `editor::open()` or `flycam::active()` is true, the game should
ignore its own mouse look and player input (and may hide its HUD).

## Knobs

Every value is a knob: `bevy_atmos::knobs::get(name, default)` anywhere.
Unset, a knob is its default, so a level without a saved look looks as the
game made it.

| Group | Knobs (default) |
|---|---|
| Time & sun | `time.hour` (-1: the game's sun), `time.speed` (0, game s per real s), `time.latitude` (35), `sun.azimuth`, `sun.elevation` (both set: the sun there), `sun.brightness` (1), `sun.warmth` (0, the sunlight's colour only), `sky.light` (1, ambient) |
| Sky | `sky.enabled` (1), `sky.physical` (1 without a game skybox), `sky.cover` (0.4), `sky.darkness` (0.3), `sky.altitude` (0 m), `sky.wind_speed` (1, cloud drift; direction: `weather.wind_dir`), `sky.offset` (0 m), `sky.tint` (0, the sky's and the fog's colour), `sky.brightness` (1) |
| Clouds | `sky.screen_clouds` (1: volumetric clouds raymarched per pixel; 0: the sky cube's own, cheaper), `clouds.density` (1), `clouds.erosion` (1, detail eating the edges), `clouds.resolution` (of the view; default half, at most ~960 px across), and the Sky group's cover, darkness, altitude and drift |
| Air | `fog.distance` (0 = the game's fog), `fog.brightness` (1), `shafts.strength` (1, 0 off), `shafts.steps` (24) |
| Grade | `grade.exposure` (0), `grade.warmth` (0, the whole image), `grade.tint` (0, green-magenta), `grade.contrast` (1), `grade.saturation` (1) |
| Weather | `weather.rain` (0), `weather.snow` (0), `weather.wind` (m/s), `weather.wind_dir` (30: rain, clouds and waves), `weather.wetness` (unset: follows the rain), `rain.opacity` (0.35), `rain.brightness` (1), `rain.near` (1), `rain.far` (0.7), `wet.puddles` (1), `wet.darken` (1) |
| Water | `water.replace` (0: keep the level's water, 1: modern water), `water.tint` (0; -1 clear blue .. 1 murky green-brown), `water.brightness` (1), `water.absorption` (1, murk through depth), `water.wave_height` (0.3 m), `water.wave_speed` (1), `water.choppiness` (0.4), `water.detail` (1, scale of the small ripples), `water.clarity` (1, simple water only), `water.waves` (1, simple water only) |
| Foam & rain | `water.foam` (1, shore and objects), `water.foam_width` (0.4 m), `water.crest_foam` (0.6, rough water), `water.foam_threshold` (0.5; higher: only the sharpest crests), `water.ripples` (1, rain hitting the water, times the rain), `water.under_fog` (0.15 per m, under the surface) |
| Rivers | `water.flow` (1), `water.flow_speed` (1, times every river), `water.flow_dir` (-1 off; 0..360 turns every river that way), `water.rapids` (2.5 m/s: white water from there; steep slopes count as fast), `water.trail` (4 m of foam behind things crossing the surface), `water.brush_radius` (3 m), `water.brush_speed` (1.5 m/s) |
| Reflections | `ssr.enabled` (1), `ssr.strength` (1), `ssr.roughness` (0.45, rougher surfaces don't take it), `ssr.distance` (30 m), `water.reflection` (1, sky and screen), `water.glitter` (1, sun or moon), `water.fresnel` (1; higher reflects at steeper angles) |

Merged in this version (old names still read if a saved look sets them):
`fog.warmth` into `sky.tint` (the fog is the sky's horizon colour),
`sky.wind_dir` and `water.wave_dir` into `weather.wind_dir`.

## Saved looks

`<save dir>/<level>.txt`, one knob a line:

```
# my_level's look (bevy_atmos)
time.hour = 17.5
sky.cover = 0.6
weather.rain = 0.4
```

Lines starting `#` are comments. Unknown names are kept and ignored.

## Painted flow

On the Rivers tab, "Paint flow" turns on the brush. Drag across the water with the left
button to paint its flow that way (`water.brush_speed`, `water.brush_radius`); the right
button rubs it out. Save writes it beside the look as `<save dir>/<level>.flow`: one
painted cell per line, `cell x, cell z, m/s x, m/s z, weight`, in 1 m world cells.
