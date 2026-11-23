//! The warp drive's look, driven by `travel::WarpFx`: while it charges the view shakes, pulls in
//! and the stars stretch into streaks; a flash, and the camera is in a swirling blue hyperspace
//! tunnel with a wide field of view; another flash, and normal space comes back as the streaks
//! shrink to stars again.

use crate::travel::{WarpFx, WarpPhase};
use bevy::asset::embedded_asset;
use bevy::pbr::{Material, MaterialPlugin};
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use orbit_core::rng::Rng;

/// The camera's normal field of view, degrees.
pub const FOV: f32 = 75.0;
/// Stars that streak, and how far from the camera they are drawn.
const STREAKS: usize = 420;
const STREAK_DIST: f32 = 600.0;

pub struct WarpPlugin;

impl Plugin for WarpPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "warp.wesl");
        app.add_plugins(MaterialPlugin::<WarpMaterial>::default())
            .add_systems(Startup, spawn)
            .add_systems(PostUpdate, camera_fx.after(crate::world::apply_origin).before(TransformSystems::Propagate));
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
struct WarpMaterial {
    #[uniform(0)]
    warp: WarpUniform,
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
struct WarpUniform {
    axis: Vec4,
    b1: Vec4,
    b2: Vec4,
}

impl Material for WarpMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fourlokos/warp.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
}

#[derive(Component)]
struct Tunnel(Handle<WarpMaterial>);

#[derive(Component)]
struct Flash;

/// Directions of the streaking stars, in the jump's frame (z ahead), all toward the front.
#[derive(Resource)]
struct Streaks(Vec<(Vec3, f32)>);

fn spawn(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<WarpMaterial>>) {
    // Seen from inside; small, so nothing in the world stands between it and the camera. The
    // shader only uses view directions, so its shape does not show.
    let mesh = Sphere::new(1.0).mesh().uv(64, 32).with_inverted_winding().unwrap();
    let mat = mats.add(WarpMaterial { warp: WarpUniform::default() });
    commands.spawn((Tunnel(mat.clone()), Mesh3d(meshes.add(mesh)), MeshMaterial3d(mat), Transform::default(), Visibility::Hidden));
    commands.spawn((
        Flash,
        Node { position_type: PositionType::Absolute, left: px(0), top: px(0), width: percent(100), height: percent(100), ..default() },
        BackgroundColor(Color::NONE),
        GlobalZIndex(50),
        Pickable::IGNORE,
    ));
    let mut rng = Rng::new(0x3a7f);
    let stars = (0..STREAKS)
        .map(|_| {
            let mut d = rng.dir();
            d.z = d.z.abs() * 0.9 + 0.1;
            (d.normalize(), rng.range(0.4, 1.0))
        })
        .collect();
    commands.insert_resource(Streaks(stars));
}

fn ease(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// How each part of the effect is doing this frame.
#[derive(Default)]
struct Look {
    fov: f32,
    shake: f32,
    tunnel: f32,
    rush: f32,
    flash: f32,
    streak: f32,
}

fn look(fx: &WarpFx) -> Look {
    let p = fx.progress.clamp(0.0, 1.0);
    match fx.phase {
        None | Some(WarpPhase::Depart) => Look { fov: FOV, ..default() },
        Some(WarpPhase::Spool) => {
            let e = p * p;
            Look { fov: FOV - 12.0 * e, shake: 0.035 * e, streak: 260.0 * e.powf(1.4), flash: ease((p - 0.88) / 0.12), ..default() }
        }
        Some(WarpPhase::Tunnel) => Look {
            fov: 118.0 - 14.0 * ease(p * 3.0),
            shake: 0.025 + 0.02 * (1.0 - p),
            tunnel: ease(p / 0.08) * (1.0 - ease((p - 0.9) / 0.1)),
            rush: 1.0 - 0.6 * ease(p * 2.0),
            flash: (1.0 - ease(p / 0.18)) + ease((p - 0.9) / 0.1),
            ..default()
        },
        Some(WarpPhase::Exit) => {
            let q = 1.0 - p;
            Look { fov: FOV + (104.0 - FOV) * q * q * q, shake: 0.02 * q, streak: 300.0 * q * q, flash: 1.0 - ease(p / 0.3), ..default() }
        }
    }
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn camera_fx(
    fx: Res<WarpFx>,
    time: Res<Time>,
    streaks: Res<Streaks>,
    mut cam: Single<(&mut Transform, &mut Projection), (With<crate::sky::SkyCamera>, Without<Tunnel>)>,
    mut tunnel: Single<(&Tunnel, &mut Transform, &mut Visibility), Without<Camera3d>>,
    mut flash: Single<&mut BackgroundColor, With<Flash>>,
    mut mats: ResMut<Assets<WarpMaterial>>,
    mut gizmos: Gizmos,
    rockets: Res<crate::rocket::Rockets>,
) {
    let l = look(&fx);
    let (ref mut t, ref mut proj) = *cam;
    if let Projection::Perspective(p) = &mut **proj {
        let fov = l.fov.to_radians();
        if (p.fov - fov).abs() > 1e-4 {
            p.fov = fov;
        }
    }
    let axis = t.forward().as_vec3();
    let s = time.elapsed_secs();
    if l.shake > 0.0 {
        // A rumble: a few incommensurate wobbles.
        let w = Vec3::new((s * 37.0).sin() + (s * 61.0).sin() * 0.5, (s * 43.0).cos() + (s * 71.0).sin() * 0.5, 0.0) * l.shake;
        t.rotation *= Quat::from_euler(EulerRot::XYZ, w.x * 0.15, w.y * 0.15, (s * 29.0).sin() * l.shake * 0.08);
        let offset = t.rotation * Vec3::new(w.x, w.y, 0.0) * 0.3;
        t.translation += offset;
    }

    let frame = Quat::from_rotation_arc(Vec3::Z, axis);
    let (tun, ref mut tt, ref mut vis) = *tunnel;
    let want = if l.tunnel > 0.001 { Visibility::Visible } else { Visibility::Hidden };
    if **vis != want {
        **vis = want;
    }
    if l.tunnel > 0.001 {
        // Round the camera and, from the chase camera, round the whole rocket too, so the
        // rocket flies inside the tunnel instead of behind its wall.
        let r = if rockets.is_piloting() && !rockets.cockpit { (rockets.reach * 1.25).max(3.0) } else { 3.0 };
        **tt = Transform::from_translation(t.translation).with_rotation(frame).with_scale(Vec3::new(r, r, r * 20.0));
        if let Some(mut m) = mats.get_mut(&tun.0) {
            m.warp = WarpUniform {
                axis: axis.extend(s % 1000.0),
                b1: (frame * Vec3::X).extend(l.tunnel),
                b2: (frame * Vec3::Y).extend(l.rush),
            };
        }
    }

    let a = l.flash.clamp(0.0, 1.0);
    let want = Color::srgba(0.86, 0.93, 1.0, a);
    if flash.0 != want {
        flash.0 = want;
    }

    if l.streak > 0.5 {
        let eye = t.translation;
        for &(d, b) in &streaks.0 {
            let at = eye + frame * d * STREAK_DIST;
            // Lines parallel to the axis: in perspective they all fan out from the point ahead.
            let len = l.streak * b * (1.0 - d.z * 0.5);
            let glow = LinearRgba::rgb(2.6, 3.0, 4.2) * (b * 1.5);
            gizmos.line(at, at - axis * len, Color::LinearRgba(glow));
        }
    }
}
