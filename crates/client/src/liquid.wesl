// Water: see-through, with moving ripples, the sky mirrored at grazing angles and the sun's glint.
// Lit like the voxels (voxel.wesl, whose uniforms it shares).
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


// The water's own color (linear), under the ripples of its texture.
const WATER: vec3<f32> = vec3<f32>(0.05, 0.22, 0.40);

@fragment
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    // Where the point is in its tile (0..1 across the block), and two ripple layers drifting
    // across it in different directions (sampled within the tile, so they wrap seamlessly).
    let t = globals.time;
    let cell = in.uv * light.atlas.xy;
    let corner = floor(cell);
    let local = cell - corner;
    let ra = textureSample(atlas, atlas_sampler, (corner + fract(local + vec2(t * 0.045, t * 0.028))) / light.atlas.xy).rgb;
    let rb = textureSample(atlas, atlas_sampler, (corner + fract(local + vec2(-t * 0.031, t * 0.052) + 0.5)) / light.atlas.xy).rgb;
    let ripple = (ra + rb) * 0.5;

    let p = in.world_position.xyz;
    var n = normalize(in.world_normal);
    if (!front) {
        n = -n;
    }
    let up = normalize(p - light.center.xyz);
    let v = normalize(view.world_position.xyz - p);
    let top = dot(normalize(in.world_normal), up);
    let surface = smoothstep(0.5, 0.9, abs(top));

    // Waves: the normal sways with a few moving swells (whole periods per block, so they run on
    // from block to block).
    let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(up.x) > 0.9);
    let e1 = normalize(cross(up, side));
    let e2 = cross(up, e1);
    let k = 6.2831853;
    let wx = cos(k * (2.0 * local.x + local.y) + t * 1.6) * 0.5 + cos(k * (local.x - 2.0 * local.y) - t * 1.1) * 0.35 + cos(k * (3.0 * local.x + local.y) + t * 2.3) * 0.15;
    let wy = cos(k * (2.0 * local.y - local.x) + t * 1.3) * 0.5 + cos(k * (local.y + 2.0 * local.x) - t * 1.7) * 0.35 + cos(k * (local.x + 3.0 * local.y) - t * 2.1) * 0.15;
    n = normalize(n + (e1 * wx + e2 * wy) * 0.09 * surface);

    // Light, the way voxel.wesl does it.
    let sky = in.uv_b.x;
    let torch = in.uv_b.y;
    let l = light.sun_dir.xyz;
    let height = dot(up, l);
    let sunlit = smoothstep(-0.06, 0.12, height);
    let twilight = smoothstep(-0.35, 0.2, height);
    let seen = seen_past(p, light.occ0) * seen_past(p, light.occ1);
    let ambient = mix(light.night_ambient.rgb, light.day_ambient.rgb, twilight * (0.08 + 0.92 * seen));
    let direct = light.sun.rgb * max(dot(n, l), 0.0) * sunlit * seen / PI;
    let lit = ((direct * 0.6 + ambient) * sky + TORCH * torch) * view.exposure;
    let body = WATER * (0.55 + ripple * 0.75) * lit;

    // The sky mirrored (paler toward the horizon) and the sun's glint, by Fresnel's law.
    let ndv = clamp(dot(n, v), 0.0, 1.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
    let r = reflect(-v, n);
    let zenith = vec3<f32>(0.32, 0.52, 0.95);
    let horizon = vec3<f32>(0.72, 0.82, 0.98);
    let skycol = mix(horizon, zenith, clamp(dot(r, up) * 1.5, 0.0, 1.0));
    let mirrored = skycol * ambient * 1.15 * sky * view.exposure;
    let h = normalize(l + v);
    let glint = pow(max(dot(n, h), 0.0), 220.0) * light.sun.rgb * 0.5 * sunlit * seen * sky * view.exposure;

    if (!front) {
        // From under the water: the surface glows with the light coming through it.
        return vec4(fogged(body * 1.3 + glint * 0.15, p, ambient), 0.92);
    }
    let color = mix(body, mirrored, fresnel * surface) + glint * surface;
    let alpha = mix(0.72, mix(0.7, 0.97, fresnel), surface);
    return vec4(fogged(color, p, ambient), alpha);
}
