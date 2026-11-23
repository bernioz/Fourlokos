// Models (the elevator: cabin, ribbon, base) lit as the blocks are (voxel.wesl): sun on the side of the
// planet facing it, and out of its shadow higher up; the day sky as ambient, starlight at night.
// They have no baked sky light: they stand in the open.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

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
}

struct ModelLook {
    color: vec4<f32>,
    // x: planet radius, y: glass (1) or a hologram (2), zw: texture repeats along u, v.
    planet: vec4<f32>,
    // An atlas tile repeated over the faces (xy corner, zw size); zero: the texture as it is.
    tile: vec4<f32>,
}

@group(3) @binding(0) var<uniform> light: VoxelLight;
@group(3) @binding(1) var<uniform> look: ModelLook;
@group(3) @binding(2) var tex: texture_2d<f32>;
@group(3) @binding(3) var tex_sampler: sampler;

const PI: f32 = 3.14159265;
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
fn fragment(in: VertexOutput, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var uv = in.uv * look.planet.zw;
    if (look.tile.z > 0.0) {
        uv = look.tile.xy + fract(uv) * look.tile.zw;
    }
    let texel = textureSample(tex, tex_sampler, uv) * look.color;
    let glass = look.planet.y > 0.5;
    if (!glass && texel.a < 0.5) {
        discard;
    }
    var n = normalize(in.world_normal);
    if (!front) {
        n = -n;
    }
    let p = in.world_position.xyz;
    // A hologram (an elevator not built yet): its own light, brighter at its edges, in scanlines.
    if (look.planet.y > 1.5) {
        let to_eye = normalize(view.world_position.xyz - p);
        let rim = pow(1.0 - abs(dot(n, to_eye)), 2.0);
        let scan = 0.7 + 0.3 * step(0.5, fract(in.position.y / 4.0));
        return vec4(look.color.rgb * (0.8 + 1.2 * rim) * scan, clamp(look.color.a * (0.5 + 1.2 * rim), 0.0, 1.0));
    }
    let rel = p - light.center.xyz;
    let r = max(length(rel), 1e-3);
    let l = light.sun_dir.xyz;
    let radius = look.planet.x;
    // On the ground: the sun over this spot's horizon, as on the blocks. Higher up: out of the
    // planet's shadow (orbit_core::station::sun_height, as the stations are lit). Lit if either.
    let ground = dot(rel / r, l);
    let along = dot(rel, l);
    let out = select(length(rel - l * along), r, along >= 0.0);
    let orbit = clamp((out - radius) / (0.6 * radius), -1.0, 1.0);
    let high = smoothstep(0.0, 0.15 * radius, r - radius);
    let height = max(ground, mix(-1.0, orbit, high));
    let sunlit = smoothstep(-0.06, 0.12, height);
    let twilight = smoothstep(-0.35, 0.2, height);
    let seen = seen_past(p, light.occ0) * seen_past(p, light.occ1);
    let direct = light.sun.rgb * max(dot(n, l), 0.0) * sunlit * seen / PI;
    let ambient = mix(light.night_ambient.rgb, light.day_ambient.rgb, twilight * (0.08 + 0.92 * seen));
    var lit = (direct + ambient) * view.exposure;
    var alpha = 1.0;
    if (glass) {
        // A little of the sky off the glass, more at a glance, so it reads as glass from inside too.
        let v = normalize(view.world_position.xyz - p);
        let fresnel = pow(1.0 - abs(dot(n, v)), 4.0);
        alpha = clamp(texel.a + fresnel * 0.5, 0.0, 1.0);
    }
    return vec4(texel.rgb * lit, alpha);
}
