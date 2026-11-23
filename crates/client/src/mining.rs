//! Breaking blocks in survival takes time: hold the left click on a block and a bar fills under
//! the crosshair. Looking away or letting go starts over. The tables are `orbit_core::mining`.

use crate::inventory::Frame;
use bevy::prelude::*;

pub struct MiningPlugin;

impl Plugin for MiningPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mining>()
            .add_systems(Startup, spawn_bar)
            .add_systems(
                Update,
                begin_frame
                    .before(crate::player::interact)
                    .before(crate::station::interact_station),
            )
            .add_systems(
                Update,
                (end_frame, draw_bar)
                    .chain()
                    .after(crate::player::interact)
                    .after(crate::station::interact_station),
            );
    }
}

/// A cell being mined: where, and which one.
pub type Key = (Frame, [i64; 3]);

#[derive(Resource, Default)]
pub struct Mining {
    key: Option<Key>,
    time: f32,
    need: f32,
    /// Set while a system keeps mining this frame; if none does, progress is lost.
    touched: bool,
    /// This frame's length, seconds (see `begin_frame`).
    pub dt: f32,
}

impl Mining {
    /// One more frame on `key`, which takes `need` seconds: true when it breaks.
    pub fn advance(&mut self, key: Key, need: f32) -> bool {
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.time = 0.0;
        }
        self.touched = true;
        self.need = need.max(0.001);
        self.time += self.dt.min(0.1);
        if self.time >= self.need {
            self.reset();
            return true;
        }
        false
    }

    pub fn reset(&mut self) {
        self.key = None;
        self.time = 0.0;
    }

    /// How far along the block is, 0..1 (0 when nothing is being mined).
    pub fn progress(&self) -> f32 {
        if self.key.is_some() {
            (self.time / self.need).clamp(0.0, 1.0)
        } else {
            0.0
        }
    }
}

fn begin_frame(mut mining: ResMut<Mining>, time: Res<Time>) {
    mining.dt = time.delta_secs();
}

fn end_frame(mut mining: ResMut<Mining>) {
    if !mining.touched {
        mining.reset();
    }
    mining.touched = false;
}

#[derive(Component)]
struct BarBack;
#[derive(Component)]
struct BarFill;

fn spawn_bar(mut commands: Commands) {
    commands
        .spawn((
            BarBack,
            Node {
                position_type: PositionType::Absolute,
                left: percent(50),
                top: percent(50),
                margin: UiRect::new(px(-30), px(0), px(22), px(0)),
                width: px(60),
                height: px(6),
                border: UiRect::all(px(1)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            BorderColor::all(Color::srgba(0.0, 0.0, 0.0, 0.8)),
            Visibility::Hidden,
            bevy::ui::FocusPolicy::Pass,
            GlobalZIndex(2),
        ))
        .with_children(|b| {
            b.spawn((
                BarFill,
                Node {
                    width: percent(0),
                    height: percent(100),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.9, 0.9, 0.9)),
            ));
        });
}

fn draw_bar(
    mining: Res<Mining>,
    mut back: Single<&mut Visibility, With<BarBack>>,
    mut fill: Single<&mut Node, With<BarFill>>,
) {
    let p = mining.progress();
    **back = if p > 0.0 {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    fill.width = percent(p * 100.0);
}
