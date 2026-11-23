//! Player health in the game: hearts over the hotbar, damage from falls and lava (survival
//! only; creative is invulnerable except to commands), a red flash on every hit, and the
//! "You died" screen. Respawning puts you back at the world's spawn with full health and your
//! inventory.

use crate::player::Player;
use crate::world::{Space, TimeShift};
use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block::Liquid;
use orbit_core::health::{DamageKind, Fall, Health, fall_damage};
use orbit_core::inventory::GameMode;

const HEARTS: usize = 10;
/// Screen pixels per heart pixel.
const SCALE: f32 = 3.0;
const FLASH: f32 = 0.45;
/// Hit points taken by each lava hit (one per invulnerability period: 4 per second).
const LAVA_HIT: f32 = 2.0;

pub struct HealthPlugin;

impl Plugin for HealthPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Vitals>()
            .add_systems(Startup, (make_hearts, spawn_ui).chain())
            .add_systems(PreUpdate, hold_input.after(bevy::input::InputSystems))
            .add_observer(on_activate)
            .add_systems(
                Update,
                (
                    reset_on_start,
                    settle_spawn,
                    take_damage,
                    death,
                    respawn,
                    draw_hearts,
                    draw_flash,
                    style_button,
                )
                    .chain()
                    .after(crate::player::move_player),
            );
    }
}

#[derive(Resource)]
pub struct Vitals {
    pub health: Health,
    fall: Fall,
    /// Falls do not count until the body first stands on something (after spawning from the sky).
    airborne_spawn: bool,
    pub dead: bool,
    respawn: bool,
    /// Seconds of red flash left.
    flash: f32,
    /// The body still has to be put on the ground at the spawn (once the planet is built).
    settle: bool,
}

impl Default for Vitals {
    fn default() -> Self {
        Self {
            health: Health::default(),
            fall: Fall::default(),
            airborne_spawn: true,
            dead: false,
            respawn: false,
            flash: 0.0,
            settle: true,
        }
    }
}

impl Vitals {
    /// A hit from anything; returns what was lost.
    pub fn hurt(&mut self, amount: f32, kind: DamageKind) -> f32 {
        let lost = self.health.damage(amount, kind);
        if lost > 0.0 {
            self.flash = FLASH;
        }
        lost
    }
}

#[derive(Resource)]
struct HeartImages {
    full: Handle<Image>,
    half: Handle<Image>,
    empty: Handle<Image>,
}

#[derive(Component)]
struct HeartsRow;
#[derive(Component)]
struct Heart(usize);
#[derive(Component)]
struct FlashOverlay;
#[derive(Component)]
struct DeathScreen;
#[derive(Component)]
struct RespawnButton;

const W: usize = 9;
const H: usize = 8;
const SHAPE: [&str; H] = [
    ".XXX.XXX.",
    "XXXXXXXXX",
    "XXXXXXXXX",
    "XXXXXXXXX",
    ".XXXXXXX.",
    "..XXXXX..",
    "...XXX...",
    "....X....",
];

/// A 9x8 heart: `fill` is how many of its 9 columns are red (from the left).
fn heart(fill: usize) -> Image {
    let mut data = Vec::with_capacity(W * H * 4);
    for (y, row) in SHAPE.iter().enumerate() {
        for (x, c) in row.bytes().enumerate() {
            let px: [u8; 4] = if c != b'X' {
                [0, 0, 0, 0]
            } else if x < fill {
                if (x, y) == (1, 1) || (x, y) == (2, 1) {
                    [255, 150, 150, 255]
                } else if y >= 5 {
                    [170, 20, 20, 255]
                } else {
                    [225, 35, 35, 255]
                }
            } else {
                [45, 12, 12, 190]
            };
            data.extend_from_slice(&px);
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: W as u32,
            height: H as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

fn make_hearts(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    commands.insert_resource(HeartImages {
        full: images.add(heart(W)),
        half: images.add(heart(W / 2 + 1)),
        empty: images.add(heart(0)),
    });
}

fn spawn_ui(mut commands: Commands, hearts: Res<HeartImages>) {
    let hotbar_w = orbit_core::inventory::HOTBAR as f32 * 40.0 + 8.0;
    commands
        .spawn((
            HeartsRow,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(80),
                left: percent(50),
                margin: UiRect::left(px(-hotbar_w / 2.0 + 4.0)),
                column_gap: px(SCALE),
                ..default()
            },
            Visibility::Hidden,
            bevy::ui::FocusPolicy::Pass,
            GlobalZIndex(2),
        ))
        .with_children(|row| {
            for i in 0..HEARTS {
                row.spawn((
                    Heart(i),
                    ImageNode::new(hearts.full.clone()),
                    Node {
                        width: px(W as f32 * SCALE),
                        height: px(H as f32 * SCALE),
                        ..default()
                    },
                ));
            }
        });
    commands.spawn((
        FlashOverlay,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::NONE),
        bevy::ui::FocusPolicy::Pass,
        GlobalZIndex(3),
    ));
}

/// Falls and lava hurt in survival; time passes (and health regenerates) either way.
fn take_damage(mut vitals: ResMut<Vitals>, player: Single<&Player>, time: Res<Time>) {
    let vitals = &mut *vitals;
    let survival = player.inv.mode == GameMode::Survival;
    let body = &player.body;
    if vitals.dead {
        return;
    }
    vitals.health.tick(time.delta_secs(), survival);
    vitals.flash = (vitals.flash - time.delta_secs()).max(0.0);
    let landed = vitals.fall.track(body);
    if vitals.airborne_spawn {
        if body.on_ground {
            vitals.airborne_spawn = false;
        }
    } else if let (Some(distance), true) = (landed, survival) {
        let hp = fall_damage(distance);
        if hp > 0.0 {
            vitals.hurt(hp, DamageKind::Fall);
        }
    }
    if survival && body.swimming == Some(Liquid::Lava) {
        vitals.hurt(LAVA_HIT, DamageKind::Lava);
    }
}

fn death(
    mut commands: Commands,
    mut vitals: ResMut<Vitals>,
    screen: Query<Entity, With<DeathScreen>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !vitals.health.is_dead() || vitals.dead || !screen.is_empty() {
        return;
    }
    vitals.dead = true;
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    commands
        .spawn((
            DeathScreen,
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: px(18),
                ..default()
            },
            BackgroundColor(Color::srgba(0.35, 0.02, 0.02, 0.6)),
            GlobalZIndex(60),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("You died"),
                TextFont {
                    font_size: FontSize::Px(64.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextShadow::default(),
            ));
            p.spawn((
                RespawnButton,
                Button,
                Hovered::default(),
                Node {
                    width: px(220),
                    height: px(48),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                BackgroundColor(BUTTON),
                children![(
                    Text::new("Respawn"),
                    TextFont {
                        font_size: FontSize::Px(22.0),
                        ..default()
                    },
                    TextColor(Color::WHITE)
                )],
            ));
            p.spawn((
                Text::new("or press Enter"),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::srgb(0.8, 0.7, 0.7)),
            ));
        });
}

const BUTTON: Color = Color::srgb(0.16, 0.2, 0.28);
const HOVER: Color = Color::srgb(0.24, 0.3, 0.42);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);

fn style_button(
    mut buttons: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<RespawnButton>>,
) {
    for (hovered, pressed, mut bg) in &mut buttons {
        let c = if pressed {
            PRESS
        } else if hovered.get() {
            HOVER
        } else {
            BUTTON
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
}

fn on_activate(
    ev: On<Activate>,
    buttons: Query<(), With<RespawnButton>>,
    mut vitals: ResMut<Vitals>,
) {
    if buttons.contains(ev.entity) {
        vitals.respawn = true;
    }
}

/// While you are dead the rest of the game sees no keys nor mouse buttons; Enter respawns.
fn hold_input(
    mut vitals: ResMut<Vitals>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if !vitals.dead {
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        vitals.respawn = true;
    }
    keys.reset_all();
    mouse.reset_all();
}

/// Back at the world's spawn with a full life; the inventory stays.
fn respawn(
    mut commands: Commands,
    mut vitals: ResMut<Vitals>,
    screen: Query<Entity, With<DeathScreen>>,
    mut player: Single<&mut Player>,
    space: Res<Space>,
    shift: Res<TimeShift>,
    time: Res<Time>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !(vitals.dead && vitals.respawn) {
        return;
    }
    let t = shift.0 + time.elapsed_secs_f64();
    let (mut home, at) = space.universe.spawn();
    let local = home.to_local(at);
    home.advance(t);
    let at = home.to_world(local);
    player.body = orbit_core::player::Body::new(at, (at - home.center).as_vec3());
    *vitals = Vitals::default();
    for e in &screen {
        commands.entity(e).despawn();
    }
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

fn draw_hearts(
    vitals: Res<Vitals>,
    player: Single<&Player>,
    images: Res<HeartImages>,
    mut row: Single<&mut Visibility, With<HeartsRow>>,
    menu: Res<crate::start_menu::StartMenu>,
    mut hearts: Query<(&Heart, &mut ImageNode)>,
) {
    // Creative hides them unless something (a command) hurt you.
    let shown = !menu.is_open() && player.inv.mode == GameMode::Survival
        || vitals.health.hp < vitals.health.max;
    let v = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if **row != v {
        **row = v;
    }
    for (Heart(i), mut img) in &mut hearts {
        let left = vitals.health.hp - 2.0 * *i as f32;
        let want = if left >= 2.0 - 0.01 {
            &images.full
        } else if left >= 1.0 {
            &images.half
        } else {
            &images.empty
        };
        if img.image != *want {
            img.image = want.clone();
        }
    }
}

fn draw_flash(vitals: Res<Vitals>, mut overlay: Single<&mut BackgroundColor, With<FlashOverlay>>) {
    let a = 0.45 * (vitals.flash / FLASH).clamp(0.0, 1.0);
    overlay.0 = Color::srgba(0.8, 0.0, 0.0, a);
}

/// The spawn point is up at the sky: once the home planet is built, stands the body on its
/// ground there (first spawn and every respawn), so nobody starts by falling.
fn settle_spawn(
    mut vitals: ResMut<Vitals>,
    mut player: Single<&mut Player>,
    space: Res<Space>,
    live: Res<crate::world::WorldLive>,
) {
    if !vitals.settle || !live.0 || vitals.dead {
        return;
    }
    let (home, at) = space.universe.spawn();
    let Some(slot) = space.planets.get(&home.id) else {
        return;
    };
    let Some(voxel) = slot.voxel.clone() else {
        return;
    };
    let planet = voxel.read().unwrap();
    let top = home.to_local(at);
    let dir = top.normalize();
    // Down from the spawn height to the first solid ground, then up to clear air.
    let mut r = top.length();
    while r > planet.grid.core && !orbit_core::player::collides(&planet, dir * r) {
        r -= 0.25;
    }
    while orbit_core::player::collides(&planet, dir * r) && r < top.length() + 30.0 {
        r += 0.05;
    }
    let here = orbit_core::player::Located {
        planet: &planet,
        center: slot.spec.center,
        rotation: slot.spec.rotation,
    };
    let forward = dir.any_orthonormal_vector();
    player.body = orbit_core::player::Body::on_planet(&here, dir * r, dir, forward);
    info!(
        "health: spawned on the ground at height {r:.2} (spawn point was {:.1})",
        top.length()
    );
    vitals.settle = false;
}

/// A new world starts from scratch: the title screen's physics placeholder already "landed" and
/// settled, so without this the fresh spawn up in the sky counts as a fall and kills the player.
fn reset_on_start(
    mut starts: MessageReader<crate::save::StartWorld>,
    mut vitals: ResMut<Vitals>,
    screen: Query<Entity, With<DeathScreen>>,
    mut commands: Commands,
) {
    if starts.read().last().is_none() {
        return;
    }
    *vitals = Vitals::default();
    for e in &screen {
        commands.entity(e).despawn();
    }
}
