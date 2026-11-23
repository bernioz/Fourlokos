//! Start menu: the game opens on a title screen with single-player, LAN placeholder, and exit
//! options, plus a first-run player-name prompt and title-safe input capture. The screen has
//! the Codex logo. Start (or Enter) begins the fixed solar system (`fixed_gen`: one star, three
//! planets, no other stars) and waits, on this screen, until the home planet is generated;
//! Cerrar closes the game.
//!
//! While the menu is up the game's camera is off and the keyboard and mouse buttons are held
//! back from the rest of the game. Demos (`ORBIT_DEMO`, except `menu`, which presses Start by itself) and `ORBIT_TITLE=0` skip it.

use crate::chat::{Edit, TextField};
use crate::save::{self, SavedWorld, StartWorld};
use crate::sky::SkyCamera;
use bevy::input::keyboard::KeyboardInput;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::inventory::GameMode;
use orbit_core::universe::WorldGen;
use orbit_core::world_file::WorldInfo;
use std::net::SocketAddr;

/// The Codex logo, in `assets/`.
const LOGO: &str = "ui/codex_logo.png";
const BUTTON: Color = Color::srgb(0.16, 0.2, 0.28);
const HOVER: Color = Color::srgb(0.24, 0.3, 0.42);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);
const ACCENT: Color = Color::srgb(0.25, 0.5, 0.35);
const MUTED: Color = Color::srgb(0.62, 0.68, 0.78);
/// Seconds to wait for the home planet before showing the game anyway.
const PATIENCE: f32 = 60.0;

/// How the universe is made: one star with three planets and one moon (the home planet's, Tulio's:
/// `max_moons` 1 and no chance of others), and nothing else.
pub fn fixed_gen() -> WorldGen {
    WorldGen {
        home_only: true,
        planets: (3, 3),
        max_moons: 1,
        moon_chance: 0.0,
        ..WorldGen::default()
    }
}

/// A world made from the create screen: the seed's fixed solar system, kept in `saves/<name>-<seed>`.
fn fixed_world(name: &str, seed: u64, mode: GameMode) -> SavedWorld {
    let mut info = WorldInfo::new(name, seed);
    info.make = fixed_gen();
    info.mode = mode;
    let slug: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    SavedWorld {
        dir: save::saves().join(format!("{slug}-{seed}")),
        info,
    }
}

/// A seed typed by the player: a number is used as is, any other text is hashed (like Minecraft);
/// empty gives a random one.
fn parse_seed(text: &str) -> u64 {
    let t = text.trim();
    if t.is_empty() {
        return std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64 & 0xFFFF_FFFF);
    }
    t.parse().unwrap_or_else(|_| {
        t.bytes().fold(0xcbf29ce484222325u64, |h, b| {
            (h ^ b as u64).wrapping_mul(0x100000001b3)
        }) & 0xFFFF_FFFF
    })
}

/// Whether the game opens on the start menu (not in demos, nor with `ORBIT_TITLE=0`).
pub fn wanted() -> bool {
    std::env::var("ORBIT_DEMO").is_ok_and(|v| v == "menu")
        || std::env::var("ORBIT_DEMO").is_err()
            && !std::env::var("ORBIT_TITLE").is_ok_and(|v| v == "0")
}

pub struct StartMenuPlugin;

impl Plugin for StartMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StartMenu>()
            .add_systems(PostStartup, open)
            .add_observer(on_activate)
            .add_systems(PreUpdate, hold_input.after(bevy::input::InputSystems))
            .add_systems(
                Update,
                (
                    return_to_title,
                    open,
                    name_input,
                    create_input,
                    create_texts,
                    run_actions,
                    refresh_multiplayer,
                    leave_for_client,
                    wait_for_world,
                    style_buttons,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, title_preview.after(crate::world::apply_origin));
    }
}

/// The menu while it is up.
#[derive(Resource, Default)]
pub struct StartMenu {
    open: Option<Screen>,
    /// Clicks waiting to be handled.
    queue: Vec<Action>,
    name_field: TextField,
    name_entry: bool,
    multiplayer: bool,
    reopen: bool,
    manual_field: TextField,
    entry_entities: Vec<Entity>,
    last_found: Vec<String>,
    connect_addr: Option<SocketAddr>,
    create: Option<Create>,
    /// The multiplayer screen's overlay and its status line.
    mp: Option<(Entity, Entity)>,
    /// The menu has opened once at startup.
    started: bool,
}

/// The "create world" screen: name, seed and game mode.
struct Create {
    root: Entity,
    name: TextField,
    seed: TextField,
    mode: GameMode,
    /// Whether the seed field (rather than the name) takes the keys.
    seed_focus: bool,
}

#[derive(Component)]
struct NameText;
#[derive(Component)]
struct SeedText;
#[derive(Component)]
struct ModeText;

impl StartMenu {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
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
    Create,
    CancelCreate,
    CycleMode,
    FocusName,
    FocusSeed,
    Multiplayer,
    CancelMultiplayer,
    ConfirmName,
    ChangeName,
    Fullscreen,
    Close,
    JoinFirst,
    ManualConnect,
}

#[derive(Component)]
struct JoinTarget(SocketAddr);

#[derive(Component)]
struct ManualAddressText;

fn open(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut menu: ResMut<StartMenu>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    settings: Res<crate::settings::Settings>,
) {
    // Only at startup (when wanted) or when sent back to the title: never again right after the
    // game closed the menu on entering a world.
    if menu.open.is_some() || !(menu.reopen || (!menu.started && wanted())) {
        return;
    }
    menu.started = true;
    let camera = commands
        .spawn((
            Camera2d,
            Camera {
                order: 2,
                clear_color: ClearColorConfig::None,
                ..default()
            },
        ))
        .id();
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
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
            GlobalZIndex(50),
        ))
        .with_children(|p| {
            p.spawn((
                ImageNode::new(assets.load(LOGO)),
                Node {
                    width: px(96),
                    height: px(96),
                    ..default()
                },
            ));
            p.spawn(label("Fourlokos", 56.0, Color::WHITE));
            p.spawn((
                label("Built with Codex", 16.0, MUTED),
                Node {
                    margin: UiRect::bottom(px(22)),
                    ..default()
                },
            ));
            if settings.player_name.is_empty() {
                p.spawn(label("Escribe tu nombre (máx. 16)", 18.0, Color::WHITE));
                p.spawn(button("Confirmar nombre", Action::ConfirmName));
            } else {
                p.spawn(button("Un jugador", Action::Start));
                p.spawn(button("Multijugador", Action::Multiplayer));
                p.spawn(button("Cambiar nombre", Action::ChangeName));
                p.spawn(button(
                    if settings.fullscreen {
                        "Pantalla completa: Sí"
                    } else {
                        "Pantalla completa: No"
                    },
                    Action::Fullscreen,
                ));
                p.spawn(button("Salir", Action::Close));
            }
            status = p
                .spawn((
                    label("", 15.0, MUTED),
                    Node {
                        margin: UiRect::top(px(14)),
                        ..default()
                    },
                ))
                .id();
        })
        .id();
    // The 3D camera remains active; title_preview supplies its transform below the UI camera.
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
    if std::env::var("ORBIT_DEMO").is_ok_and(|v| v == "menu") {
        menu.queue.push(Action::Start);
        menu.queue.push(Action::Create);
    }
    menu.name_entry = settings.player_name.is_empty();
    menu.multiplayer = false;
    menu.reopen = false;
    menu.name_field = TextField::new(16);
    menu.manual_field = TextField::new(64);
    menu.entry_entities.clear();
    menu.last_found.clear();
    menu.connect_addr = None;
    menu.create = None;
    menu.mp = None;
    menu.open = Some(Screen {
        camera,
        root,
        status,
        waiting: None,
    });
}

/// Drives the existing world renderer from a title-only camera. The player entity remains a
/// physics placeholder for systems that require `Single<&Player>`, but title input is held and
/// this camera, rather than the player, determines the streaming origin.
fn title_preview(
    menu: Res<StartMenu>,
    time: Res<Time>,
    space: Res<crate::world::Space>,
    clock: Res<crate::world::Clock>,
    mut origin: ResMut<crate::world::Origin>,
    mut camera: Single<&mut Transform, With<SkyCamera>>,
) {
    if menu.open.is_none() {
        return;
    }
    let (mut home, _) = space.universe.spawn();
    home.advance(clock.0);
    let angle = time.elapsed_secs() * 0.05;
    let direction = Vec3::new(angle.cos(), 0.22, angle.sin()).normalize();
    let distance = home.radius * 2.5;
    let eye = home.center + direction.as_dvec3() * distance as f64;
    origin.at = home.center;
    origin.eye = eye;
    **camera = Transform::from_translation(origin.render(eye)).looking_at(Vec3::ZERO, Vec3::Y);
}

fn label(s: &str, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn button(text: &str, action: Action) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node {
            width: px(300),
            height: px(48),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(8)),
            ..default()
        },
        BackgroundColor(BUTTON),
        children![label(text, 22.0, Color::WHITE)],
    )
}

fn multiplayer_screen() -> &'static str {
    "Buscando mundos en la red… (Enter para unirse al primero)"
}

fn on_activate(
    ev: On<Activate>,
    actions: Query<&Action>,
    targets: Query<&JoinTarget>,
    mut menu: ResMut<StartMenu>,
) {
    if let Ok(target) = targets.get(ev.entity) {
        menu.connect_addr = Some(target.0);
        menu.queue.push(Action::JoinFirst);
    } else if let Ok(&a) = actions.get(ev.entity) {
        menu.queue.push(a);
    }
}

/// While the menu is up the rest of the game sees no keys nor mouse buttons; Enter starts.
fn hold_input(
    mut menu: ResMut<StartMenu>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if menu.open.is_none() {
        return;
    }
    if !menu.name_entry
        && menu.open.as_ref().is_some_and(|s| s.waiting.is_none())
        && (keys.just_pressed(KeyCode::Enter) || keys.just_pressed(KeyCode::NumpadEnter))
    {
        let action = if menu.multiplayer {
            Action::JoinFirst
        } else if menu.create.is_some() {
            Action::Create
        } else {
            Action::Start
        };
        menu.queue.push(action);
    }
    keys.reset_all();
    mouse.reset_all();
}

fn name_input(
    mut events: MessageReader<KeyboardInput>,
    mut menu: ResMut<StartMenu>,
    mut settings: ResMut<crate::settings::Settings>,
    mut texts: Query<&mut Text>,
) {
    if menu.create.is_some() {
        return;
    }
    if menu.multiplayer {
        for ev in events.read() {
            if ev.state.is_pressed()
                && !ev.repeat
                && menu.manual_field.apply(ev, false) == Edit::Escape
            {
                menu.queue.push(Action::CancelMultiplayer);
            }
        }
        return;
    }
    if !menu.open.as_ref().is_some_and(|_| menu.name_entry) {
        return;
    }
    let status = menu
        .open
        .as_ref()
        .map(|s| s.status)
        .unwrap_or(Entity::PLACEHOLDER);
    for ev in events.read() {
        if !ev.state.is_pressed() || ev.repeat {
            continue;
        }
        let edit = menu.name_field.apply(ev, false);
        if edit == Edit::Enter && !menu.name_field.text.trim().is_empty() {
            settings.player_name = menu.name_field.text.trim().chars().take(16).collect();
            menu.name_entry = false;
            if let Ok(mut t) = texts.get_mut(status) {
                t.0 = "Nombre guardado".into();
            }
        }
    }
    if let Ok(mut t) = texts.get_mut(status) {
        t.0 = format!("Nombre: {}", menu.name_field.text);
    }
    events.read().for_each(drop);
}

/// Keys typed on the create screen go to the focused field; Tab switches field, Esc goes back.
fn create_input(mut events: MessageReader<KeyboardInput>, mut menu: ResMut<StartMenu>) {
    let Some(c) = menu.create.as_mut() else {
        return;
    };
    let mut cancel = false;
    for ev in events.read() {
        if !ev.state.is_pressed() {
            continue;
        }
        let edit = if c.seed_focus {
            c.seed.apply(ev, false)
        } else {
            c.name.apply(ev, false)
        };
        match edit {
            Edit::Tab => c.seed_focus = !c.seed_focus,
            Edit::Escape => cancel = true,
            _ => {}
        }
    }
    if cancel {
        menu.queue.push(Action::CancelCreate);
    }
}

fn create_texts(
    menu: Res<StartMenu>,
    mut nodes: Query<&mut Node>,
    mut names: Query<&mut Text, (With<NameText>, Without<SeedText>, Without<ModeText>)>,
    mut seeds: Query<&mut Text, (With<SeedText>, Without<NameText>, Without<ModeText>)>,
    mut modes: Query<&mut Text, (With<ModeText>, Without<NameText>, Without<SeedText>)>,
) {
    if let Some(root) = menu.open.as_ref().map(|s| s.root) {
        let hide = menu.create.is_some() || menu.mp.is_some();
        if let Ok(mut n) = nodes.get_mut(root) {
            let want = if hide { Display::None } else { Display::Flex };
            if n.display != want {
                n.display = want;
            }
        }
    }
    let Some(c) = menu.create.as_ref() else {
        return;
    };
    let show = |f: &TextField, focus: bool| {
        format!(
            "{}{}{}",
            f.before(),
            if focus { "|" } else { "" },
            f.after()
        )
    };
    for mut t in &mut names {
        t.0 = format!("Nombre del mundo: {}", show(&c.name, !c.seed_focus));
    }
    for mut t in &mut seeds {
        t.0 = format!("Semilla: {}", show(&c.seed, c.seed_focus));
    }
    for mut t in &mut modes {
        t.0 = format!(
            "Modo de juego: {}",
            if c.mode == GameMode::Survival {
                "Supervivencia"
            } else {
                "Creativo"
            }
        );
    }
}

fn return_to_title(
    mut messages: MessageReader<crate::pause_menu::ReturnToTitle>,
    mut menu: ResMut<StartMenu>,
    mut live: ResMut<crate::world::WorldLive>,
    dir: Res<save::SaveDir>,
    stations: Res<crate::station::Stations>,
    elevators: Res<crate::elevator::Elevators>,
    clock: Res<crate::world::Clock>,
    player: Query<&crate::player::Player>,
    mut main_cam: Single<&mut Camera, With<SkyCamera>>,
) {
    if messages.read().next().is_none() {
        return;
    }
    save::save_now(&dir, &stations, &elevators, &clock, player.single().ok());
    live.0 = true;
    menu.reopen = true;
    main_cam.is_active = true;
}

fn run_actions(
    mut commands: Commands,
    mut menu: ResMut<StartMenu>,
    space: Res<crate::world::Space>,
    mut net: ResMut<crate::netplay::NetPlay>,
    mut role: ResMut<crate::netplay::NetRole>,
    mut starts: MessageWriter<StartWorld>,
    mut exit: MessageWriter<AppExit>,
    mut texts: Query<&mut Text>,
    mut settings: ResMut<crate::settings::Settings>,
) {
    let queue = std::mem::take(&mut menu.queue);
    if menu.open.is_none() {
        return;
    }
    for action in queue {
        let status = menu
            .open
            .as_ref()
            .map(|s| s.status)
            .unwrap_or(Entity::PLACEHOLDER);
        match action {
            Action::ConfirmName => {
                if !menu.name_field.text.trim().is_empty() {
                    settings.player_name = menu.name_field.text.trim().chars().take(16).collect();
                    menu.name_entry = false;
                } else if let Ok(mut t) = texts.get_mut(status) {
                    t.0 = "El nombre no puede estar vacío".into();
                }
            }
            Action::ChangeName => {
                menu.name_entry = true;
                menu.name_field.set(&settings.player_name);
            }
            Action::Fullscreen => {
                settings.fullscreen = !settings.fullscreen;
            }
            Action::Multiplayer if menu.mp.is_none() => {
                menu.multiplayer = true;
                let camera = menu.open.as_ref().map(|s| s.camera).unwrap();
                let mut mp_status = Entity::PLACEHOLDER;
                let overlay = commands
                    .spawn((
                        UiTargetCamera(camera),
                        Node {
                            width: percent(100),
                            height: percent(100),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            row_gap: px(12),
                            position_type: PositionType::Absolute,
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.02, 0.03, 0.07, 0.8)),
                        GlobalZIndex(55),
                    ))
                    .with_children(|p| {
                        p.spawn(label("Multijugador (LAN)", 40.0, Color::WHITE));
                        mp_status = p
                            .spawn((
                                label("Buscando mundos en la red…", 15.0, MUTED),
                                Node {
                                    margin: UiRect::bottom(px(8)),
                                    ..default()
                                },
                            ))
                            .id();
                    })
                    .id();
                menu.mp = Some((overlay, mp_status));
                menu.entry_entities.clear();
                menu.last_found.clear();
            }
            Action::CancelMultiplayer => {
                if let Some((root, _)) = menu.mp.take() {
                    commands.entity(root).despawn();
                    menu.entry_entities.clear();
                    menu.multiplayer = false;
                }
            }
            Action::JoinFirst => {
                let target = menu.connect_addr.take().or_else(|| {
                    net.found
                        .first()
                        .map(|f| SocketAddr::new(f.addr.ip(), f.announce.port))
                });
                if let Some(addr) = target {
                    match crate::net::Client::connect(addr, &settings.player_name) {
                        Ok((client, welcome)) => {
                            *role = crate::netplay::NetRole::Client(client);
                            net.pending_welcome = Some(welcome);
                            info!("LAN discovery join: {:?}", addr);
                        }
                        Err(e) => {
                            if let Ok(mut t) = texts.get_mut(status) {
                                t.0 = format!("No se pudo unir: {e}");
                            }
                        }
                    }
                }
            }
            Action::ManualConnect => {
                let value = menu.manual_field.text.trim().to_owned();
                let parsed = value
                    .parse::<SocketAddr>()
                    .or_else(|_| format!("{value}:{}", crate::net::DEFAULT_PORT).parse());
                match parsed {
                    Ok(addr) => match crate::net::Client::connect(addr, &settings.player_name) {
                        Ok((client, welcome)) => {
                            *role = crate::netplay::NetRole::Client(client);
                            net.pending_welcome = Some(welcome);
                            info!("LAN manual join: {addr}");
                        }
                        Err(e) => {
                            if let Ok(mut t) = texts.get_mut(status) {
                                t.0 = format!("No se pudo conectar: {e}");
                            }
                        }
                    },
                    Err(e) => {
                        if let Ok(mut t) = texts.get_mut(status) {
                            t.0 = format!("Dirección inválida: {e}");
                        }
                    }
                }
            }
            Action::Start
                if menu.open.as_ref().is_some_and(|s| s.waiting.is_none())
                    && menu.create.is_none() =>
            {
                let mut name = TextField::new(24);
                name.set("Fourlokos");
                let (root, camera) = menu.open.as_ref().map(|s| (s.root, s.camera)).unwrap();
                let overlay = commands.spawn((
                    UiTargetCamera(camera),
                    Node { width: percent(100), height: percent(100), flex_direction: FlexDirection::Column, justify_content: JustifyContent::Center, align_items: AlignItems::Center, row_gap: px(14), position_type: PositionType::Absolute, ..default() },
                    BackgroundColor(Color::srgba(0.02, 0.03, 0.07, 0.8)),
                    GlobalZIndex(55),
                )).with_children(|p| {
                    p.spawn(label("Crear mundo", 40.0, Color::WHITE));
                    for (action, marker) in [(Action::FocusName, 0), (Action::FocusSeed, 1), (Action::CycleMode, 2)] {
                        let mut e = p.spawn((Button, Hovered::default(), action, Node { width: px(460), height: px(44), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(8)), ..default() }, BackgroundColor(BUTTON)));
                        e.with_children(|b| {
                            let t = (Text::new(""), TextFont { font_size: FontSize::Px(20.0), ..default() }, TextColor(Color::WHITE));
                            match marker { 0 => { b.spawn((t, NameText)); } 1 => { b.spawn((t, SeedText)); } _ => { b.spawn((t, ModeText)); } }
                        });
                    }
                    p.spawn(label("Tab: cambiar campo | Semilla vacía = aleatoria | Enter: crear | Esc: volver", 14.0, MUTED));
                    p.spawn(button("Crear mundo", Action::Create));
                    p.spawn(button("Cancelar", Action::CancelCreate));
                }).id();
                let _ = root;
                menu.create = Some(Create {
                    root: overlay,
                    name,
                    seed: TextField::new(20),
                    mode: GameMode::Survival,
                    seed_focus: false,
                });
            }
            Action::FocusName => {
                if let Some(c) = menu.create.as_mut() {
                    c.seed_focus = false;
                }
            }
            Action::FocusSeed => {
                if let Some(c) = menu.create.as_mut() {
                    c.seed_focus = true;
                }
            }
            Action::CycleMode => {
                if let Some(c) = menu.create.as_mut() {
                    c.mode = if c.mode == GameMode::Survival {
                        GameMode::Creative
                    } else {
                        GameMode::Survival
                    };
                }
            }
            Action::CancelCreate => {
                if let Some(c) = menu.create.take() {
                    commands.entity(c.root).despawn();
                }
            }
            Action::Create if menu.open.as_ref().is_some_and(|s| s.waiting.is_none()) => {
                let (name, seed, mode) = match menu.create.take() {
                    Some(c) => {
                        commands.entity(c.root).despawn();
                        (
                            c.name.text.trim().to_owned(),
                            parse_seed(&c.seed.text),
                            c.mode,
                        )
                    }
                    None => (
                        "Fourlokos".to_owned(),
                        space.universe.seed,
                        GameMode::Survival,
                    ),
                };
                let name = if name.is_empty() {
                    "Fourlokos".to_owned()
                } else {
                    name
                };
                starts.write(StartWorld(fixed_world(&name, seed, mode)));
                if let Some(s) = menu.open.as_mut() {
                    s.waiting = Some(0.0);
                }
                if let Ok(mut t) = texts.get_mut(status) {
                    t.0 = "Generating the home planet".into();
                }
            }
            Action::Close => {
                exit.write(AppExit::Success);
            }
            Action::Start | Action::Create | Action::Multiplayer => {}
        }
    }
}

fn refresh_multiplayer(
    mut commands: Commands,
    mut menu: ResMut<StartMenu>,
    net: Res<crate::netplay::NetPlay>,
    mut texts: Query<&mut Text, Without<ManualAddressText>>,
    mut manual: Query<&mut Text, With<ManualAddressText>>,
) {
    let Some((root, status)) = menu.mp else {
        return;
    };
    if !menu.multiplayer {
        return;
    }
    let found: Vec<String> = net
        .found
        .iter()
        .map(|f| format!("{}|{}", f.addr, f.announce.world_name))
        .collect();
    if found != menu.last_found || menu.entry_entities.is_empty() {
        for e in menu.entry_entities.drain(..) {
            commands.entity(e).despawn();
        }
        let worlds = net.found.clone();
        let mut spawned = Vec::new();
        commands.entity(root).with_children(|p| {
            for f in worlds {
                let addr = SocketAddr::new(f.addr.ip(), f.announce.port);
                let text = format!(
                    "{} · {} · {} jugadores · {}",
                    f.announce.world_name, f.announce.host_name, f.announce.players, addr
                );
                spawned.push(
                    p.spawn((
                        Button,
                        Hovered::default(),
                        JoinTarget(addr),
                        Node {
                            width: px(420),
                            height: px(38),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(BUTTON),
                        children![label(&text, 15.0, Color::WHITE)],
                    ))
                    .id(),
                );
            }
            p.spawn((
                Text::new(""),
                ManualAddressText,
                TextFont {
                    font_size: FontSize::Px(15.0),
                    ..default()
                },
                TextColor(MUTED),
            ));
            spawned.push(
                p.spawn((
                    Button,
                    Hovered::default(),
                    Action::ManualConnect,
                    Node {
                        width: px(220),
                        height: px(40),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    BackgroundColor(BUTTON),
                    children![label("Conectar", 18.0, Color::WHITE)],
                ))
                .id(),
            );
            spawned.push(p.spawn(button("Volver", Action::CancelMultiplayer)).id());
        });
        menu.entry_entities = spawned;
        menu.last_found = found;
    }
    if let Ok(mut t) = texts.get_mut(status) {
        t.0 = if net.found.is_empty() {
            "Buscando mundos en la red…".into()
        } else {
            "Selecciona un mundo LAN o conecta manualmente".into()
        };
    }
    for mut t in &mut manual {
        t.0 = format!("ip[:port]: {}", menu.manual_field.text);
    }
}

fn leave_for_client(
    mut commands: Commands,
    mut menu: ResMut<StartMenu>,
    role: Res<crate::netplay::NetRole>,
    mut main_cam: Single<&mut Camera, With<SkyCamera>>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if !menu.multiplayer || !matches!(&*role, crate::netplay::NetRole::Client(_)) {
        return;
    }
    let Some(screen) = menu.open.take() else {
        return;
    };
    commands.entity(screen.root).despawn();
    commands.entity(screen.camera).despawn();
    if let Some((root, _)) = menu.mp.take() {
        commands.entity(root).despawn();
    }
    main_cam.is_active = true;
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
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
    let Some(waited) = s.waiting.as_mut() else {
        return;
    };
    *waited += time.delta_secs();
    let pos = player.body.world_point(player.body.pos);
    let ready = !space.waiting_at(pos)
        && space.planets.values().any(|p| {
            p.voxel.is_some() && pos.distance(p.spec.center) < p.spec.gravity_radius() as f64
        });
    if (*waited * 2.0) as u32 != ((*waited - time.delta_secs()) * 2.0) as u32
        && (*waited as u32) % 2 == 0
    {
        info!(
            "start: waiting {:.1}s ready={ready} waiting_at={} voxel_planets={} pos_len={}",
            *waited,
            space.waiting_at(pos),
            space.planets.values().filter(|p| p.voxel.is_some()).count(),
            pos.length()
        );
    }
    if !ready && *waited < PATIENCE {
        if let Ok(mut t) = texts.get_mut(s.status) {
            t.0 = format!(
                "Generating the home planet{}",
                ".".repeat(1 + (*waited * 2.0) as usize % 3)
            );
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

fn style_buttons(
    mut buttons: Query<(&Hovered, Has<Pressed>, &Action, &mut BackgroundColor), With<Button>>,
    menu: Res<StartMenu>,
) {
    let waiting = menu.open.as_ref().is_some_and(|s| s.waiting.is_some());
    for (hovered, pressed, action, mut bg) in &mut buttons {
        let base = if *action == Action::Start && waiting {
            ACCENT
        } else {
            BUTTON
        };
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
