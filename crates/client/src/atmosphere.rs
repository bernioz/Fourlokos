//! The sky inside a planet's atmosphere: a dome around the camera, blended over the star field.
//! Its color follows the sun at the camera (day, dusk, night) and fades out on the way to space.

use crate::world::{Origin, Space};
use bevy::asset::embedded_asset;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use orbit_core::eclipse::sun_seen;
use orbit_core::planet::PlanetSpec;

/// Dome radius: beyond the planet's own star (planets are up to about 120 000 blocks from it; it
/// is drawn in front of the sky), nearer than other systems' stars (240 000 away at least), which
/// the daytime sky hides as it should.
const DOME: f32 = 150_000.0;

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "atmosphere.wesl");
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .add_systems(Startup, spawn_dome)
            .add_systems(PostUpdate, follow_camera.before(TransformSystems::Propagate));
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct SkyMaterial {
    #[uniform(0)]
    atmo: AtmosphereUniform,
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct AtmosphereUniform {
    sun_dir: Vec4,
    up: Vec4,
    /// rgb: the star's color; w: how much of it is seen (less in an eclipse).
    sun_color: Vec4,
    /// The sky's colors: overhead and at the horizon by day, overhead at dusk, the glow toward
    /// the setting sun and the band opposite it.
    zenith: Vec4,
    horizon: Vec4,
    dusk_zenith: Vec4,
    dusk_sun: Vec4,
    dusk_gold: Vec4,
    arrebol: Vec4,
}

/// A sky's colors (see `AtmosphereUniform`).
struct Palette {
    zenith: Vec3,
    horizon: Vec3,
    dusk_zenith: Vec3,
    dusk_sun: Vec3,
    dusk_gold: Vec3,
    arrebol: Vec3,
}

/// Planets have Chile's sky: deep blue by day, orange and gold at dusk with the pink "arrebol".
/// Moons have thinner, stranger air, colored by what they are made of, turned a little by their
/// own seed so no two are quite alike.
fn palette(spec: &PlanetSpec) -> Palette {
    use orbit_core::planet::Biome;
    let chile = Palette {
        zenith: Vec3::new(0.04, 0.2, 0.72),
        horizon: Vec3::new(0.42, 0.63, 0.93),
        dusk_zenith: Vec3::new(0.025, 0.06, 0.26),
        dusk_sun: Vec3::new(1.0, 0.36, 0.08),
        dusk_gold: Vec3::new(1.0, 0.68, 0.25),
        arrebol: Vec3::new(0.78, 0.36, 0.48),
    };
    if !spec.is_moon() {
        return chile;
    }
    let p = match spec.biome {
        // Like Mars: butterscotch by day, and blue round the setting sun.
        Biome::Desert => Palette {
            zenith: Vec3::new(0.55, 0.36, 0.2),
            horizon: Vec3::new(0.8, 0.6, 0.42),
            dusk_zenith: Vec3::new(0.12, 0.07, 0.06),
            dusk_sun: Vec3::new(0.25, 0.5, 0.95),
            dusk_gold: Vec3::new(0.55, 0.7, 0.95),
            arrebol: Vec3::new(0.5, 0.3, 0.25),
        },
        // Crystal: teal by day, violet at dusk.
        Biome::Crystal => Palette {
            zenith: Vec3::new(0.05, 0.42, 0.48),
            horizon: Vec3::new(0.45, 0.8, 0.78),
            dusk_zenith: Vec3::new(0.08, 0.03, 0.2),
            dusk_sun: Vec3::new(0.75, 0.3, 0.95),
            dusk_gold: Vec3::new(0.95, 0.6, 0.95),
            arrebol: Vec3::new(0.3, 0.75, 0.7),
        },
        // Ice: pale lavender, rose at dusk.
        Biome::Frozen => Palette {
            zenith: Vec3::new(0.32, 0.3, 0.62),
            horizon: Vec3::new(0.75, 0.72, 0.9),
            dusk_zenith: Vec3::new(0.07, 0.05, 0.16),
            dusk_sun: Vec3::new(0.95, 0.45, 0.55),
            dusk_gold: Vec3::new(1.0, 0.75, 0.75),
            arrebol: Vec3::new(0.55, 0.45, 0.85),
        },
        // Rock: a hazy amber like Titan's, deep red at dusk.
        _ => Palette {
            zenith: Vec3::new(0.42, 0.3, 0.12),
            horizon: Vec3::new(0.78, 0.62, 0.35),
            dusk_zenith: Vec3::new(0.1, 0.05, 0.03),
            dusk_sun: Vec3::new(0.9, 0.25, 0.08),
            dusk_gold: Vec3::new(0.95, 0.55, 0.2),
            arrebol: Vec3::new(0.6, 0.3, 0.2),
        },
    };
    // A small turn of hue (and depth) from the moon's own seed.
    let h = orbit_core::rng::mix(spec.id ^ 0x5C1E);
    let t = ((h & 0xFF) as f32 / 255.0 - 0.5) * 0.25;
    let turn = |c: Vec3| Vec3::new(c.x * (1.0 + t), c.y, c.z * (1.0 - t)).min(Vec3::splat(0.98));
    Palette { zenith: turn(p.zenith), horizon: turn(p.horizon), dusk_zenith: turn(p.dusk_zenith), dusk_sun: p.dusk_sun, dusk_gold: p.dusk_gold, arrebol: turn(p.arrebol) }
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fourlokos/atmosphere.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

#[derive(Component)]
struct Dome(Handle<SkyMaterial>);

fn spawn_dome(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<SkyMaterial>>) {
    // Seen from inside: flip the sphere's triangles.
    let mesh = Sphere::new(DOME).mesh().ico(4).unwrap().with_inverted_winding().unwrap();
    let mat = mats.add(SkyMaterial { atmo: AtmosphereUniform::default() });
    commands.spawn((Dome(mat.clone()), Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat), Transform::default(), Visibility::Hidden));
}

/// Centers the dome on the camera and points its sky at the nearest planet's sun.
fn follow_camera(
    space: Res<Space>,
    origin: Res<Origin>,
    cam: Query<&Transform, (With<crate::sky::SkyCamera>, Without<Dome>)>,
    mut dome: Query<(&Dome, &mut Transform, &mut Visibility)>,
    mut mats: ResMut<Assets<SkyMaterial>>,
) {
    let (Ok(cam), Ok((dome, mut t, mut vis))) = (cam.single(), dome.single_mut()) else { return };
    t.translation = cam.translation; // render space
    let eye = origin.eye; // world, for the planets
    let near = space.planets.values().min_by(|a, b| {
        let da = eye.distance(a.spec.center) as f32 - a.spec.radius;
        let db = eye.distance(b.spec.center) as f32 - b.spec.radius;
        da.total_cmp(&db)
    });
    let Some(p) = near else {
        *vis = Visibility::Hidden;
        return;
    };
    // Full sky up to a third of the way to the gravity edge, gone at the edge.
    let top = p.spec.gravity_radius() - p.spec.radius;
    let alt = eye.distance(p.spec.center) as f32 - p.spec.radius;
    let depth = 1.0 - ((alt - top / 3.0) / (top * 2.0 / 3.0)).clamp(0.0, 1.0);
    let depth = depth * depth * (3.0 - 2.0 * depth);
    *vis = if depth > 0.0 { Visibility::Visible } else { Visibility::Hidden };
    if let Some(mut m) = mats.get_mut(&dome.0) {
        let up = (eye - p.spec.center).as_vec3().normalize_or(Vec3::Y);
        // In an eclipse the sky darkens with the sun.
        let seen = space.eclipsers(p.spec.id).iter().map(|&(c, r)| sun_seen(eye, p.spec.star(), p.star_radius as f64, c, r as f64)).product::<f64>() as f32;
        let c = palette(&p.spec);
        m.atmo = AtmosphereUniform {
            sun_dir: (p.spec.star() - p.spec.center).as_vec3().normalize_or(Vec3::Y).extend(0.0),
            up: up.extend(depth),
            sun_color: Vec3::ONE.lerp(Vec3::from_array(p.star_color), 0.35).extend(seen),
            zenith: c.zenith.extend(0.0),
            horizon: c.horizon.extend(0.0),
            dusk_zenith: c.dusk_zenith.extend(0.0),
            dusk_sun: c.dusk_sun.extend(0.0),
            dusk_gold: c.dusk_gold.extend(0.0),
            arrebol: c.arrebol.extend(0.0),
        };
    }
}
