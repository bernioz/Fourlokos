//! Preview of the procedural skybox: rotates a camera and saves screenshots, then exits.
//!
//! `SKY_SEED=123 SKY_OUT=/tmp/sky cargo run -p fourlokos --example sky_preview`
//! writes `<SKY_OUT>_<seed>_<n>.png` for a few view directions.

#[allow(dead_code)]
#[path = "../src/sky.rs"]
mod sky;

use bevy::{
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
};
use sky::{SkyCamera, SkyPlugin, SkySeed};

/// Yaw/pitch (radians) of each screenshot.
const VIEWS: [(f32, f32); 4] = [(0.0, 0.0), (1.6, 0.3), (3.2, -0.4), (4.7, 1.2)];

fn main() {
    let seed = std::env::var("SKY_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                resolution: (1280, 720).into(),
                title: "sky preview".into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(SkySeed(seed))
        .add_plugins(SkyPlugin)
        .add_systems(Startup, |mut commands: Commands| {
            commands.spawn((
                Camera3d::default(),
                Projection::Perspective(PerspectiveProjection {
                    fov: 70f32.to_radians(),
                    ..default()
                }),
                SkyCamera,
            ));
        })
        .add_systems(Update, drive)
        .run();
}

fn drive(
    mut commands: Commands,
    seed: Res<SkySeed>,
    time: Res<Time>,
    mut camera: Single<&mut Transform, With<SkyCamera>>,
    ready: Query<(), (With<SkyCamera>, With<bevy::light::Skybox>)>,
    mut frames_ready: Local<u32>,
    mut exit: MessageWriter<AppExit>,
) {
    if ready.is_empty() {
        return;
    }
    *frames_ready += 1;
    let n = *frames_ready as usize;
    let view = (n / 20).min(VIEWS.len());
    if view == VIEWS.len() {
        if n > VIEWS.len() * 20 + 30 {
            exit.write(AppExit::Success);
        }
        return;
    }
    // Slow drift around the current view.
    let (yaw, pitch) = VIEWS[view];
    let drift = time.elapsed_secs() * 0.02;
    camera.rotation = Quat::from_euler(EulerRot::YXZ, yaw + drift, pitch, 0.0);
    if n % 20 == 19 {
        let out = std::env::var("SKY_OUT").unwrap_or_else(|_| "/tmp/sky".into());
        commands
            .spawn(Screenshot::primary_window())
            .observe(save_to_disk(format!("{out}_{}_{view}.png", seed.0)));
    }
}
