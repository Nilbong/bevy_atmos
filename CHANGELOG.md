# Changelog

## 0.2.0 (2026-10-21)

Grew out of using Atmos inside two real games.

### Clouds
- Clouds now come from a single system, so the tuner drives every cloud you see. 0.1 had two sets of clouds, and the sliders only reached one of them.
- Taller, more varied cloud towers. Smoother edges, with less fuzz and banding.

### Shadows (new tab)
- Contact shadows, ambient occlusion and shadow softness, saved per level.
- Soft, sun-sized shadows (PCSS) with the optional `pcss` feature (Bevy's experimental PCSS).

### Water
- Water detection works in any game, not just the demo.
- **Mark water** in the editor adds a mesh the detection missed.
- Water kinds: lake, river, puddle.
- Rivers flow downhill into white water.
- Higher-resolution foam, and rain rings on the surface.

### Reflections
- Screen-space reflections on water and wet ground.

### Puddles
- Puddles form only where rain can reach. A shelter map keeps them out from under roofs and bridges.
- Smooth puddle edges, with no more square corners.

### Fixes
- No crash when the game despawns an entity Atmos is tracking: commands now use `try_insert`/`try_remove`.
- Docs, screenshots and KNOBS.md updated.

### Upgrading
Pin the version in `Cargo.toml`:

```toml
bevy_atmos = { git = "https://github.com/Nilbong/bevy_atmos", tag = "v0.2.0" }
```

Saved looks from 0.1 still load. New knobs default to "unchanged".

## 0.1.0 (2026-10-16)
First release: time of day, physical sky, volumetric clouds, rain and snow, wet surfaces, modern water, fog and sun shafts, colour grade, and the in-game look editor.
