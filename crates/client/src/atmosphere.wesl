// Sky seen from inside a planet's atmosphere, drawn on a huge dome around the camera and blended
// over the star field. Tuned after Chile's sky: a deep, clean blue by day (dry Andean air, pale
// only near the horizon), orange and gold toward the sun at dusk with the pink "arrebol" band on
// the opposite side, and at night almost no veil, so the Milky Way shows as in the Atacama.

import bevy_pbr::render::forward_io::VertexOutput;
import bevy_pbr::render::mesh_view_bindings::view;

struct Atmosphere {
    // xyz: unit direction toward the sun.
    sun_dir: vec4<f32>,
    // xyz: up at the camera; w: how deep in the atmosphere (0 in space, 1 on the ground).
    up: vec4<f32>,
    // rgb: the star's color; w: how much of it is seen (less in an eclipse).
    sun_color: vec4<f32>,
    // This sky's colors (atmosphere.rs `palette`).
    zenith: vec4<f32>,
    horizon: vec4<f32>,
    dusk_zenith: vec4<f32>,
    dusk_sun: vec4<f32>,
    dusk_gold: vec4<f32>,
    arrebol: vec4<f32>,
}

@group(3) @binding(0) var<uniform> atmo: Atmosphere;

// Colors are kept under 1 per channel: past that the tonemapper bends saturated blue toward
// purple.
const NIGHT: vec3<f32> = vec3(0.0015, 0.003, 0.009);
const BRIGHTNESS: f32 = 1.0;

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    let v = normalize(in.world_position.xyz - view.world_position);
    let up = atmo.up.xyz;
    let l = atmo.sun_dir.xyz;
    let h = dot(l, up);         // sun height over the horizon
    let e = dot(v, up);         // view elevation
    let mu = dot(v, l);         // closeness to the sun
    let el = clamp(e, 0.0, 1.0);

    let ZENITH = atmo.zenith.rgb;
    let HORIZON = atmo.horizon.rgb;
    let DUSK_SUN = atmo.dusk_sun.rgb;
    let DUSK_GOLD = atmo.dusk_gold.rgb;
    let ARREBOL = atmo.arrebol.rgb;
    let DUSK_ZENITH = atmo.dusk_zenith.rgb;
    // An eclipse darkens the day: stars come out under a total one.
    let seen = atmo.sun_color.w;

    // Phases of the day by the sun's height (h = sin of its elevation):
    let up_sun = smoothstep(-0.10, 0.04, h) * seen; // sun up: the sky hides the stars
    let day = smoothstep(-0.04, 0.20, h) * seen;    // full daylight
    let twilight = smoothstep(-0.32, -0.02, h);     // glow lasting until ~18° below
    let low = 1.0 - smoothstep(0.05, 0.30, h);      // sun low: warm horizon

    let haze = pow(1.0 - el, 5.0);
    let band = pow(1.0 - el, 4.0);
    let toward = pow(max(mu, 0.0) * 0.5 + 0.5, 4.0);

    // Day: deep blue overhead, paler toward the horizon (and below it, seen from a height).
    let day_sky = mix(ZENITH, HORIZON, haze);
    // Twilight: indigo overhead, a faint pink "arrebol" band low on the side away from the sun.
    let arrebol = ARREBOL * pow(1.0 - el, 7.0) * (1.0 - toward) * 0.4;
    let dusk_sky = mix(DUSK_ZENITH, HORIZON * 0.35, band * 0.5) + arrebol;
    var col = mix(dusk_sky, day_sky, day);
    // The horizon burns orange and gold on the sun's side while it is low.
    col += mix(DUSK_GOLD, DUSK_SUN, band) * band * toward * 1.4 * twilight * low;

    // Glare around the sun while it is up (the disc itself is the star mesh, drawn in front).
    let m = max(mu, 0.0);
    col += atmo.sun_color.rgb * (pow(m, 64.0) * 1.2 + pow(m, 6.0) * 0.18) * up_sun * mix(vec3(1.0), DUSK_SUN * 1.5, low);

    // Opaque while the sun is up, thinning through twilight to a light veil at night.
    let veil = max(up_sun, twilight * 0.9);
    let alpha = clamp(max(veil, 0.12), 0.0, 1.0) * atmo.up.w;
    col = mix(NIGHT, col * BRIGHTNESS, twilight);
    return vec4(col, alpha);
}
