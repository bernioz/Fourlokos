// Hyperspace tunnel: drawn on a stretched sphere around the camera, its axis the way we jump.
// Everything is worked out from the view direction alone, so the mesh's size does not matter:
// around the axis an angle, along it the cotangent of the angle to the axis (how far down the
// tunnel the wall is seen). Swirling blue walls, white streaks rushing past, a glare ahead.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

struct Warp {
    // xyz: the jump's direction; w: time, seconds.
    axis: vec4<f32>,
    // xyz: a direction across the axis; w: how much tunnel there is (0..1).
    b1: vec4<f32>,
    // xyz: the third direction; w: speed of the rush (0..1).
    b2: vec4<f32>,
}

@group(3) @binding(0) var<uniform> warp: Warp;

const TAU: f32 = 6.2831853;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

fn wrap(x: f32, period: f32) -> f32 {
    return x - period * floor(x / period);
}

// Value noise that repeats every `period` along x (around the tunnel).
fn noise(p: vec2<f32>, period: f32) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let x0 = wrap(i.x, period);
    let x1 = wrap(i.x + 1.0, period);
    let a = hash(vec2(x0, i.y));
    let b = hash(vec2(x1, i.y));
    let c = hash(vec2(x0, i.y + 1.0));
    let d = hash(vec2(x1, i.y + 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let v = normalize(in.world_position.xyz - view.world_position);
    let axis = warp.axis.xyz;
    let t = warp.axis.w;
    let amount = warp.b1.w;
    let rush = warp.b2.w;

    let c = dot(v, axis);
    let lat = v - axis * c;
    let s = max(length(lat), 1e-4);
    let theta = atan2(dot(lat, warp.b2.xyz), dot(lat, warp.b1.xyz));
    let depth = clamp(c / s, -40.0, 40.0);

    // Around the tunnel (periodic), twisting slowly with depth and time.
    let around = (theta / TAU + 0.5) * 64.0 + depth * 1.2 + t * 2.0;
    let along = depth * 0.9 - t * (10.0 + 30.0 * rush);

    let walls = noise(vec2(around * 0.5, along * 0.25), 32.0) * 0.6 + noise(vec2(around, along * 0.5 + 9.0), 64.0) * 0.4;
    let streak = pow(smoothstep(0.62, 1.0, noise(vec2(around * 2.0, along * 0.12 + 3.0), 128.0)), 2.0);
    let fine = pow(smoothstep(0.75, 1.0, noise(vec2(around * 4.0 + 17.0, along * 0.08), 256.0)), 3.0);

    var col = mix(vec3(0.004, 0.012, 0.09), vec3(0.06, 0.28, 1.1), walls * walls * walls);
    col += vec3(0.7, 1.4, 3.2) * streak * 1.3;
    col += vec3(2.4, 2.8, 3.6) * fine * 2.5;
    // White-hot glare down the tunnel ahead; darker behind.
    let ahead = pow(max(c, 0.0), 40.0);
    col += vec3(2.2, 2.6, 3.4) * ahead * 2.0 + vec3(0.6, 0.8, 1.6) * pow(max(c, 0.0), 4.0) * 0.6;
    col *= mix(0.25, 1.0, smoothstep(-0.7, 0.5, c));

    return vec4(col, clamp(amount, 0.0, 1.0));
}
