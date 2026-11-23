// Voxel planet lighting and textures. The sun only reaches the half of the planet that faces it (the planet
// shadows its own night side, which shadow maps cannot do at this scale), and the sky light baked
// into vertex alpha dims both sun and ambient in caves and under overhangs.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::{view, globals};

struct VoxelLight {
    // xyz: unit direction toward the sun.
    sun_dir: vec4<f32>,
    // rgb: sun illuminance (lux) per channel.
    sun: vec4<f32>,
    // rgb: ambient sky light by day, and what is left at night (starlight).
    day_ambient: vec4<f32>,
    night_ambient: vec4<f32>,
    // xyz: planet center, world space.
    center: vec4<f32>,
    // xyz: world point, w: radius. Fragments within it are cut (far meshes under loaded chunks).
    hole: vec4<f32>,
    // xyz: the star's center, w: its radius (0: no eclipses).
    star: vec4<f32>,
    // Bodies that can pass in front of the star (moons, a moon's planet): xyz center, w radius
    // (0: none).
    occ0: vec4<f32>,
    occ1: vec4<f32>,
    // xy: the atlas's tiles per row, and rows.
    atlas: vec4<f32>,
    // Eyes under water or in lava: rgb color (times the ambient light), w thickness (0: none).
    fog: vec4<f32>,
}

// Seen through the liquid the eyes are in: fades to its color with distance.
fn fogged(color: vec3<f32>, p: vec3<f32>, ambient: vec3<f32>) -> vec3<f32> {
    if (light.fog.w <= 0.0) {
        return color;
    }
    let d = distance(p, view.world_position.xyz);
    let f = 1.0 - exp(-d * light.fog.w);
    return mix(color, light.fog.rgb * (ambient + vec3<f32>(40.0)) * view.exposure, f);
}

@group(3) @binding(0) var<uniform> light: VoxelLight;
@group(3) @binding(1) var atlas: texture_2d<f32>;
@group(3) @binding(2) var atlas_sampler: sampler;

// Torchlight: warm, about as bright as a cloudy day where it is strongest.
const TORCH: vec3<f32> = vec3<f32>(1.0, 0.72, 0.42) * 320.0;

const PI: f32 = 3.14159265;

// Lava's own glow: brighter than torchlight, enough to bloom.
const LAVA: vec3<f32> = vec3<f32>(1.0, 0.8, 0.62) * 900.0;

// Share of a disc of angular radius a covered by one of radius b, d apart (orbit_core::eclipse).
fn covered(a: f32, b: f32, d: f32) -> f32 {
    if (a <= 0.0 || d >= a + b) {
        return 0.0;
    }
    if (d <= abs(a - b)) {
        let r = min(a, b) / a;
        return r * r;
    }
    let ca = acos(clamp((d * d + a * a - b * b) / (2.0 * d * a), -1.0, 1.0));
    let cb = acos(clamp((d * d + b * b - a * a) / (2.0 * d * b), -1.0, 1.0));
    let k = sqrt(max((-d + a + b) * (d + a - b) * (d - a + b) * (d + a + b), 0.0));
    return clamp((a * a * ca + b * b * cb - 0.5 * k) / (PI * a * a), 0.0, 1.0);
}

// How much of the star `p` sees past body `o` (xyz center, w radius).
fn seen_past(p: vec3<f32>, o: vec4<f32>) -> f32 {
    if (o.w <= 0.0 || light.star.w <= 0.0) {
        return 1.0;
    }
    let to_star = light.star.xyz - p;
    let to_body = o.xyz - p;
    let ds = length(to_star);
    let db = length(to_body);
    if (db <= o.w || db >= ds || dot(to_body, to_star) <= 0.0) {
        return 1.0;
    }
    let a = asin(min(light.star.w / ds, 1.0));
    let b = asin(min(o.w / db, 1.0));
    let sep = acos(clamp(dot(to_body / db, to_star / ds), -1.0, 1.0));
    return 1.0 - covered(a, b, sep);
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // uv: atlas; uv_b: (sky, block) light. Plants and torches are cut out of their tiles.
    // Sampled first, while control flow is still uniform.
    // Lava (vertex alpha 0) creeps: its tile is sampled sliding slowly within itself.
    let t = globals.time;
    let cell = in.uv * light.atlas.xy;
    let corner = floor(cell);
    let local = cell - corner;
    let creep = vec2(t * 0.021, t * 0.013) + 0.03 * vec2(sin(t * 0.6 + local.y * 6.283), cos(t * 0.45 + local.x * 6.283));
    let molten_tex = textureSample(atlas, atlas_sampler, (corner + fract(local + creep)) / light.atlas.xy);
    let plain_tex = textureSample(atlas, atlas_sampler, in.uv);
    let molten = in.color.a < 0.5;
    let tex = select(plain_tex, molten_tex, molten);
    if (distance(in.world_position.xyz, light.hole.xyz) < light.hole.w || tex.a < 0.5) {
        discard;
    }
    let albedo = tex.rgb * in.color.rgb;
    let sky = in.uv_b.x;
    let torch = in.uv_b.y;
    let n = normalize(in.world_normal);
    let up = normalize(in.world_position.xyz - light.center.xyz);
    let l = light.sun_dir.xyz;
    let height = dot(up, l);
    // Sun above this spot's horizon, with a short soft terminator; the sky glow lingers a bit longer.
    let sunlit = smoothstep(-0.06, 0.12, height);
    let twilight = smoothstep(-0.35, 0.2, height);
    // Eclipses: a moon (or a moon's planet) in front of the star dims the sun, and the sky with it.
    let p = in.world_position.xyz;
    let seen = seen_past(p, light.occ0) * seen_past(p, light.occ1);
    let direct = light.sun.rgb * max(dot(n, l), 0.0) * sunlit * seen / PI;
    let ambient = mix(light.night_ambient.rgb, light.day_ambient.rgb, twilight * (0.08 + 0.92 * seen));
    var lit = ((direct + ambient) * sky + TORCH * torch) * view.exposure;
    if (molten) {
        let pulse = 0.82 + 0.18 * sin(t * 1.1 + dot(p, vec3(0.31, 0.17, 0.23)));
        lit += LAVA * pulse * view.exposure;
    }
    return vec4(fogged(albedo * lit, p, ambient), 1.0);
}
