//! Small, human-readable client preferences shared by the title, pause, and camera systems.

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PrimaryWindow, WindowMode};
use std::path::PathBuf;

#[derive(Resource, Clone, Debug)]
pub struct Settings {
    pub fov_deg: f32,
    pub sensitivity: f32,
    pub player_name: String,
    pub fullscreen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            fov_deg: 75.0,
            sensitivity: 1.0,
            player_name: String::new(),
            fullscreen: false,
        }
    }
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::save::saves().join("../settings.cfg")
    }
    pub fn load() -> Self {
        let mut out = Self::default();
        if let Ok(text) = std::fs::read_to_string(Self::path()) {
            for line in text.lines() {
                let Some((key, value)) = line.split_once('=') else {
                    continue;
                };
                match key.trim() {
                    "fov_deg" => out.fov_deg = value.trim().parse().unwrap_or(out.fov_deg),
                    "sensitivity" => {
                        out.sensitivity = value.trim().parse().unwrap_or(out.sensitivity)
                    }
                    "player_name" => out.player_name = value.trim().chars().take(16).collect(),
                    "fullscreen" => out.fullscreen = value.trim() == "1",
                    _ => {}
                }
            }
        }
        out.clamp_values();
        out
    }
    pub fn clamp_values(&mut self) {
        self.fov_deg = self.fov_deg.clamp(50.0, 110.0);
        self.sensitivity = self.sensitivity.clamp(0.1, 5.0);
        self.player_name = self.player_name.chars().take(16).collect();
    }
    pub fn save(&self) {
        let mut saved = self.clone();
        saved.clamp_values();
        if let Some(parent) = Self::path().parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = format!(
            "fov_deg={}\nsensitivity={}\nplayer_name={}\nfullscreen={}\n",
            saved.fov_deg, saved.sensitivity, saved.player_name, saved.fullscreen as u8
        );
        if let Err(e) = std::fs::write(Self::path(), text) {
            warn!("settings: could not save preferences: {e}");
        }
    }
}

pub struct SettingsPlugin;
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Settings::load())
            .add_systems(
                PreUpdate,
                toggle_fullscreen.after(bevy::input::InputSystems),
            )
            .add_systems(Update, (apply_camera_fov, apply_window_mode).chain())
            .add_systems(Last, save_changed.run_if(resource_changed::<Settings>));
    }
}

fn apply_camera_fov(
    settings: Res<Settings>,
    mut camera: Single<&mut Projection, With<crate::sky::SkyCamera>>,
) {
    if let Projection::Perspective(p) = &mut **camera {
        p.fov = settings.fov_deg.to_radians();
    }
}

fn save_changed(settings: Res<Settings>) {
    settings.save();
}

fn toggle_fullscreen(keys: Res<ButtonInput<KeyCode>>, mut settings: ResMut<Settings>) {
    if keys.just_pressed(KeyCode::F11) {
        settings.fullscreen = !settings.fullscreen;
    }
}

fn apply_window_mode(
    settings: Res<Settings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    let desired = if settings.fullscreen {
        WindowMode::BorderlessFullscreen(MonitorSelection::Current)
    } else {
        WindowMode::Windowed
    };
    if window.mode != desired {
        window.mode = desired;
    }
}
