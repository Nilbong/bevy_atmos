// The shelter capture's depth (a camera looking straight down) into the
// rain's shelter map: the highest surface over each cell (m), very low
// where nothing is. The map wraps (cell index modulo its size), so each
// texel takes the captured cell that lands on it.

#import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput

@group(0) @binding(0) var depth: texture_depth_2d;
// The modern water's heights (same layout), merged in.
@group(0) @binding(1) var water: texture_2d<f32>;
// x, y: the capture's first cell; z: the camera's height; w: its far plane.
// Second: x cells a side, y 1 with the water's heights.
@group(0) @binding(2) var<uniform> params: array<vec4<f32>, 2>;

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let n = i32(params[1].x + 0.5);
    let s = vec2<i32>(in.position.xy);
    let origin = vec2<i32>(params[0].xy);
    // The world cell that lands on this texel, within the capture.
    let c = origin + ((((s - origin) % n) + n) % n);
    let pixel = c - origin;
    // (Reverse Z, orthographic: 1 at the camera, 0 at the far plane.)
    let d = textureLoad(depth, pixel, 0);
    var h = -1.0e6;
    if (d > 1e-6) {
        h = params[0].z - (1.0 - d) * params[0].w;
    }
    if (params[1].y > 0.5) {
        h = max(h, textureLoad(water, s, 0).r);
    }
    return vec4(h, 0.0, 0.0, 1.0);
}
