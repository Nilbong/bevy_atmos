// bevy_atmos's screen-space clouds laid over the sky (`clouds.rs`): a
// fullscreen quad at the far plane, so it only covers pixels nothing was
// drawn on (reverse Z: depth 0 passes the depth test only there), the
// clouds texture stretched over the view.

#import bevy_pbr::mesh_view_bindings::view

@group(#{MATERIAL_BIND_GROUP}) @binding(0) var clouds: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var clouds_sampler: sampler;
// x: the skybox's brightness.
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var<uniform> look: vec4<f32>;

struct Out {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vertex(@location(0) position: vec3<f32>) -> Out {
    var out: Out;
    // (The quad is -1..1: straight to clip space, at the far plane.)
    out.clip = vec4(position.xy, 0.0, 1.0);
    out.uv = vec2(position.x * 0.5 + 0.5, 0.5 - position.y * 0.5);
    return out;
}

@fragment
fn fragment(in: Out) -> @location(0) vec4<f32> {
    // (A small tent over the half-resolution march: its last noise showed as
    // a fine grid when scaled up with one tap.)
    let px = 0.5 / vec2<f32>(textureDimensions(clouds));
    let c = (textureSampleLevel(clouds, clouds_sampler, in.uv + vec2(px.x, px.y), 0.0)
        + textureSampleLevel(clouds, clouds_sampler, in.uv + vec2(-px.x, px.y), 0.0)
        + textureSampleLevel(clouds, clouds_sampler, in.uv + vec2(px.x, -px.y), 0.0)
        + textureSampleLevel(clouds, clouds_sampler, in.uv + vec2(-px.x, -px.y), 0.0)) * 0.25;
    // Premultiplied: the clouds' light added, the sky behind dimmed by
    // their transmittance.
    return vec4(c.rgb * look.x * view.exposure, 1.0 - saturate(c.a));
}
