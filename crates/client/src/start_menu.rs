//! Start menu: the game opens on a plain 2D screen with two options, "Start" and "Cerrar", and
//! the Codex logo. Start (or Enter) begins the fixed solar system (`fixed_gen`: one star, three
//! planets, no other stars) and waits, on this screen, until the home planet is generated;
//! Cerrar closes the game.
//!
//! While the menu is up the game's camera is off and the keyboard and mouse buttons are held
//! back from the rest of the game. Demos (`ORBIT_DEMO`) and `ORBIT_TITLE=0` skip it.

use crate::save::{self, SavedWorld, StartWorld};
use crate::sky::SkyCamera;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::universe::WorldGen;
use orbit_core::world_file::WorldInfo;

/// The Codex logo, in `assets/`.
const LOGO: &str = "ui/codex_logo.png";
const BG: Color = Color::srgb(0.03, 0.04, 0.09);
const BUTTON: Color = Color::srgb(0.16, 0.2, 0.28);
const HOVER: Color = Color::srgb(0.24, 0.3, 0.42);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);
const ACCENT: Color = Color::srgb(0.25, 0.5, 0.35);
const MUTED: Color = Color::srgb(0.62, 0.68, 0.78);
/// Seconds to wait for the home planet before showing the game anyway.
const PATIENCE: f32 = 60.0;

/// How the universe is made: one star with three planets (and no moons), and nothing else.
pub fn fixed_gen() -> WorldGen {
    WorldGen { home_only: true, planets: (3, 3), max_moons: 0, ..WorldGen::default() }
}

/// The one world Start begins: the seed's fixed solar system, kept in `saves/fourlokos-<seed>`.
fn fixed_world(seed: u64) -> SavedWorld {
    let mut info = WorldInfo::new("Fourlokos", seed);
    info.make = fixed_gen();
    SavedWorld { dir: save::saves().join(format!("fourlokos-{seed}")), info }
}

/// Whether the game opens on the start menu (not in demos, nor with `ORBIT_TITLE=0`).
pub fn wanted() -> bool {
    std::env::var("ORBIT_DEMO").is_err() && !std::env::var("ORBIT_TITLE").is_ok_and(|v| v == "0")
}

pub struct StartMenuPlugin;

impl Plugin for StartMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StartMenu>()
            .add_systems(PostStartup, open)
            .add_observer(on_activate)
            .add_systems(PreUpdate, hold_input.after(bevy::input::InputSystems))
            .add_systems(Update, (run_actions, wait_for_world, style_buttons).chain());
    }
}

/// The menu while it is up.
#[derive(Resource, Default)]
struct StartMenu {
    open: Option<Screen>,
    /// Clicks waiting to be handled.
    queue: Vec<Action>,
}

struct Screen {
    camera: Entity,
    root: Entity,
    status: Entity,
    /// Seconds waited for the home planet, once Start was pressed.
    waiting: Option<f32>,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Start,
    Close,
}

fn open(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut menu: ResMut<StartMenu>,
    mut main_cam: Single<&mut Camera, With<SkyCamera>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !wanted() {
        return;
    }
    let camera = commands.spawn((Camera2d, Camera { order: 2, clear_color: ClearColorConfig::Custom(BG), ..default() })).id();
    let mut status = Entity::PLACEHOLDER;
    let root = commands
        .spawn((
            UiTargetCamera(camera),
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: px(14),
                ..default()
            },
            BackgroundColor(BG),
            GlobalZIndex(50),
        ))
        .with_children(|p| {
            p.spawn((ImageNode::new(assets.load(LOGO)), Node { width: px(96), height: px(96), ..default() }));
            p.spawn(label("Fourlokos", 56.0, Color::WHITE));
            p.spawn((label("Built with Codex", 16.0, MUTED), Node { margin: UiRect::bottom(px(22)), ..default() }));
            p.spawn(button("Start", Action::Start));
            p.spawn(button("Cerrar", Action::Close));
            status = p.spawn((label("", 15.0, MUTED), Node { margin: UiRect::top(px(14)), ..default() })).id();
        })
        .id();
    main_cam.is_active = false;
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    menu.open = Some(Screen { camera, root, status, waiting: None });
}

fn label(s: &str, size: f32, color: Color) -> impl Bundle {
    (Text::new(s), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(text: &str, action: Action) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node { width: px(220), height: px(48), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(8)), ..default() },
        BackgroundColor(BUTTON),
        children![label(text, 22.0, Color::WHITE)],
    )
}

fn on_activate(ev: On<Activate>, actions: Query<&Action>, mut menu: ResMut<StartMenu>) {
    if let Ok(&a) = actions.get(ev.entity) {
        menu.queue.push(a);
    }
}

/// While the menu is up the rest of the game sees no keys nor mouse buttons; Enter starts.
fn hold_input(mut menu: ResMut<StartMenu>, mut keys: ResMut<ButtonInput<KeyCode>>, mut mouse: ResMut<ButtonInput<MouseButton>>) {
    if menu.open.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter) {
        menu.queue.push(Action::Start);
    }
    keys.reset_all();
    mouse.reset_all();
}

fn run_actions(mut menu: ResMut<StartMenu>, space: Res<crate::world::Space>, mut starts: MessageWriter<StartWorld>, mut exit: MessageWriter<AppExit>, mut texts: Query<&mut Text>) {
    let queue = std::mem::take(&mut menu.queue);
    let Some(s) = menu.open.as_mut() else { return };
    for action in queue {
        match action {
            Action::Start if s.waiting.is_none() => {
                starts.write(StartWorld(fixed_world(space.universe.seed)));
                s.waiting = Some(0.0);
                if let Ok(mut t) = texts.get_mut(s.status) {
                    t.0 = "Generating the home planet".into();
                }
            }
            Action::Close => {
                exit.write(AppExit::Success);
            }
            Action::Start => {}
        }
    }
}

/// After Start: stays up until the home planet is there (gravity pulls from the first frame), then
/// the game takes over with the mouse grabbed.
fn wait_for_world(
    mut commands: Commands,
    mut menu: ResMut<StartMenu>,
    time: Res<Time>,
    space: Res<crate::world::Space>,
    player: Single<&crate::player::Player>,
    mut main_cam: Single<&mut Camera, With<SkyCamera>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut texts: Query<&mut Text>,
) {
    let Some(s) = menu.open.as_mut() else { return };
    let Some(waited) = s.waiting.as_mut() else { return };
    *waited += time.delta_secs();
    let pos = player.body.world_point(player.body.pos);
    let ready = !space.waiting_at(pos) && space.planets.values().any(|p| p.voxel.is_some() && pos.distance(p.spec.center) < p.spec.gravity_radius() as f64);
    if !ready && *waited < PATIENCE {
        if let Ok(mut t) = texts.get_mut(s.status) {
            t.0 = format!("Generating the home planet{}", ".".repeat(1 + (*waited * 2.0) as usize % 3));
        }
        return;
    }
    let (camera, root) = (s.camera, s.root);
    menu.open = None;
    commands.entity(root).despawn();
    commands.entity(camera).despawn();
    main_cam.is_active = true;
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

fn style_buttons(mut buttons: Query<(&Hovered, Has<Pressed>, &Action, &mut BackgroundColor), With<Button>>, menu: Res<StartMenu>) {
    let waiting = menu.open.as_ref().is_some_and(|s| s.waiting.is_some());
    for (hovered, pressed, action, mut bg) in &mut buttons {
        let base = if *action == Action::Start && waiting { ACCENT } else { BUTTON };
        let c = if pressed {
            PRESS
        } else if hovered.get() && !(waiting && *action == Action::Start) {
            HOVER
        } else {
            base
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
}
