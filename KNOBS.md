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
| Where rain can't fall | Automatic: with no host `weather::Shelter`, while it rains or snows a camera looks straight down over the player for 4 frames, and its depth (roofs, awnings, trees) becomes the shelter map (256 m square, 0.5 m cells, water surfaces merged in), taken again after moving 64 m. A host's own `weather::Shelter` (a heights image round the origin) always wins. Debug: `ATMOS_SHELTER_PNG=<file>` saves the map as an image. |

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
| Clouds | `clouds.quality` (2: 0 off, no clouds at all; 1 low, 2 medium, 3 high: the march's steps and resolution), `clouds.density` (1), `clouds.erosion` (1, detail eating the edges), `clouds.resolution` (unset: the quality's; set, a share of the view's resolution), and the Sky group's cover, darkness, altitude and drift. The same clouds are drawn in the reflection and ambient cube (256 a face), so every cloud knob drives both. Every cloud knob visibly changes them: `clouds.quality` 1..3 also sets their detail and edge sharpness, and `clouds.erosion` makes them wispy. Cloud lighting: `clouds.light` (2.8, 0..4: the sun's light on them), `clouds.shadow` (2, 0..8: self-shadowing, how dark the shaded sides go), `clouds.ambient` (0.45, 0..4: the sky's fill light, mostly from overhead). The clouds are opaque, with grey undersides, bright sun-facing tops and silver linings; `clouds.debug` (1 sun only, 2 ambient only) is a debug view, not in the editor. Some billow erosion is always there; `clouds.erosion` adds wisps. |
| Shadows | Unset, each leaves the host's own setup as it was. `shadows.contact` (0/1, Bevy contact shadows on the camera and sun), `shadows.contact_length` (0.3 m), `shadows.softness` (0.5; below 0.3 hard 2x2 filtering, above Gaussian; with the `pcss` feature, PCSS `soft_shadow_size`), `shadows.distance` (150 m), `shadows.cascades` (4), `shadows.strength` (1; lower, more ambient fill and lighter shadows), `ssao.enabled` (0/1; on, it adds the depth and normal prepasses and turns the camera's MSAA off, restored when off), `ssao.quality` (2: 0 low .. 3 ultra), `ssao.direct` (0.8; how much AO also darkens direct light, so it shows in sunlit places; 0 is Bevy's ambient-only AO), `ssao.contrast` (1.6, 0.5..4) |
| Air | `fog.distance` (0 = the game's fog), `fog.brightness` (1), `shafts.strength` (1, 0 off), `shafts.steps` (24) |
| Grade | `grade.exposure` (0), `grade.warmth` (0, the whole image), `grade.tint` (0, green-magenta), `grade.contrast` (1), `grade.saturation` (1) |
| Weather | `weather.rain` (0), `weather.snow` (0), `weather.wind` (m/s), `weather.wind_dir` (30: rain, clouds and waves), `weather.wetness` (unset: follows the rain), `rain.opacity` (0.35), `rain.brightness` (1), `rain.near` (1), `rain.far` (0.7), `wet.puddles` (1), `wet.darken` (1), `rain.shelter` (1: the automatic shelter map; 0 off: rain everywhere except on water), `rain.shelter_height` (60 m: how far above the camera the roof search starts), `rain.shelter_debug` (debug, not in the editor: 1 draws the snowflakes the shelter map hides in red, to check where rain and snow are kept out; pairs with `ATMOS_SHELTER_PNG`) |
| Water | `water.replace` (1: modern water, the default; 0: keep the level's own water), `water.tint` (0; -1 clear blue .. 1 murky green-brown), `water.brightness` (1), `water.absorption` (1, murk through depth), `water.wave_height` (0.3 m), `water.wave_speed` (1), `water.choppiness` (0.4), `water.detail` (1, scale of the small ripples), `water.clarity` (1, simple water only), `water.waves` (1, simple water only) |
| Finding water | `water.auto` (1: meshes named with a water word become water), `water.guess` (0: also flat, level, smooth or see-through blue-green surfaces; can catch glass), `water.mark.<key>` (1 water, 0 never water: the Mark water tool), `water.kind.<key>` (0 auto, 1 puddle, 2 pond, 3 river, 4 sea: the Mark water tool's kind picker; used before name words and shape). `<key>` is the water's glTF material, mesh or entity name, or `pos_<x*10>_<y*10>_<z*10>` when it has none, so several unnamed meshes at the same origin share one key and one setting. |
| Foam & rain | `water.foam` (1, shore and objects), `water.foam_width` (0.4 m), `water.crest_foam` (0.6, rough water), `water.foam_threshold` (0.5; higher: only the sharpest crests), `water.ripples` (1, rain hitting the water, times the rain), `water.under_fog` (0.15 per m, under the surface) |
| Rivers | `water.flow` (1), `water.flow_speed` (1, times every river), `water.flow_dir` (-1 off; 0..360 turns every river that way), `water.rapids` (2.5 m/s: white water from there; steep slopes count as fast), `water.trail` (4 m of foam behind things crossing the surface), `water.brush_radius` (3 m), `water.brush_speed` (1.5 m/s), `water.sediment` (0.6, 0..1: rivers only, a silty earth colour that absorbs blue first; 0 clear) |
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
