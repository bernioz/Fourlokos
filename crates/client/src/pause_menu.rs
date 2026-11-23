//! In-game pause and client settings panel.

use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};

const BG: Color = Color::srgba(0.03, 0.04, 0.09, 0.94);
const BUTTON: Color = Color::srgb(0.16, 0.2, 0.28);
const HOVER: Color = Color::srgb(0.24, 0.3, 0.42);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);

#[derive(Resource, Default)]
pub struct PauseState {
    pub open: bool,
    settings: bool,
    root: Option<Entity>,
    queue: Vec<Action>,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct ReturnToTitle;

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Action {
    Continue,
    Settings,
    Back,
    FovUp,
    FovDown,
    SensUp,
    SensDown,
    Fullscreen,
    Return,
    HostLan,
    CloseLan,
}

pub struct PauseMenuPlugin;
impl Plugin for PauseMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PauseState>()
            .add_message::<ReturnToTitle>()
            .add_observer(on_activate)
            .add_systems(Update, (toggle, run_actions, style, hold_input).chain());
    }
}

fn text(s: &str, size: f32) -> impl Bundle {
    (
        Text::new(s),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(Color::WHITE),
    )
}
fn button(s: &str, a: Action) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        a,
        Node {
            width: px(260),
            height: px(42),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(7)),
            ..default()
        },
        BackgroundColor(BUTTON),
        children![text(s, 18.0)],
    )
}

fn toggle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    start: Res<crate::start_menu::StartMenu>,
    menu: Res<crate::menu::PlanetMenu>,
    map: Res<crate::map::GalaxyMap>,
    keyboard: Res<crate::chat::Keyboard>,
    station: Res<crate::station::StationMenu>,
    inventory: Res<crate::inventory::Screen>,
    settings: Res<crate::settings::Settings>,
    mut pause: ResMut<PauseState>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if start.is_open()
        || menu.is_open()
        || map.is_open()
        || keyboard.owner.is_some()
        || station.is_open()
        || inventory.is_open()
    {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        if pause.open {
            close(&mut commands, &mut pause, &mut cursor);
        } else {
            open(&mut commands, &mut pause, &mut cursor, &settings);
        }
    }
}

fn open(
    commands: &mut Commands,
    pause: &mut PauseState,
    cursor: &mut CursorOptions,
    settings: &crate::settings::Settings,
) {
    pause.open = true;
    pause.settings = false;
    rebuild(commands, pause, settings);
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

fn rebuild(commands: &mut Commands, pause: &mut PauseState, settings: &crate::settings::Settings) {
    if let Some(root) = pause.root.take() {
        commands.entity(root).despawn();
    }
    let settings_screen = pause.settings;
    let root = commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(Color::srgba(0., 0., 0., 0.35)),
            GlobalZIndex(40),
        ))
        .with_children(|p| {
            p.spawn((
                Node {
                    width: px(360),
                    padding: UiRect::all(px(24)),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: px(10),
                    border_radius: BorderRadius::all(px(10)),
                    ..default()
                },
                BackgroundColor(BG),
            ))
            .with_children(|p| {
                p.spawn(text(
                    if settings_screen { "Ajustes" } else { "Pausa" },
                    30.0,
                ));
                if settings_screen {
                    p.spawn(button("FOV -", Action::FovDown));
                    p.spawn(button("FOV +", Action::FovUp));
                    p.spawn(button("Sensibilidad -", Action::SensDown));
                    p.spawn(button("Sensibilidad +", Action::SensUp));
                    p.spawn(button(
                        if settings.fullscreen {
                            "Pantalla completa: Sí"
                        } else {
                            "Pantalla completa: No"
                        },
                        Action::Fullscreen,
                    ));
                    p.spawn(button("Volver", Action::Back));
                } else {
                    p.spawn(button("Continuar", Action::Continue));
                    p.spawn(button("Ajustes", Action::Settings));
                    p.spawn(button("Abrir a LAN", Action::HostLan));
                    p.spawn(button("Cerrar LAN", Action::CloseLan));
                    p.spawn(button("Volver al menú de inicio", Action::Return));
                }
            });
        })
        .id();
    pause.root = Some(root);
}

fn close(commands: &mut Commands, pause: &mut PauseState, cursor: &mut CursorOptions) {
    if let Some(root) = pause.root.take() {
        commands.entity(root).despawn();
    }
    pause.open = false;
    pause.settings = false;
    cursor.grab_mode = CursorGrabMode::Locked;
    cursor.visible = false;
}

fn on_activate(ev: On<Activate>, actions: Query<&Action>, mut pause: ResMut<PauseState>) {
    if let Ok(&a) = actions.get(ev.entity) {
        pause.queue.push(a);
    }
}

fn run_actions(
    mut commands: Commands,
    mut pause: ResMut<PauseState>,
    mut settings: ResMut<crate::settings::Settings>,
    mut returns: MessageWriter<ReturnToTitle>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut role: ResMut<crate::netplay::NetRole>,
    space: Res<crate::world::Space>,
) {
    let actions = std::mem::take(&mut pause.queue);
    for action in actions {
        match action {
            Action::Continue => close(&mut commands, &mut pause, &mut cursor),
            Action::Back => {
                if pause.settings {
                    pause.settings = false;
                    rebuild(&mut commands, &mut pause, &settings);
                } else {
                    close(&mut commands, &mut pause, &mut cursor);
                }
            }
            Action::Settings => {
                pause.settings = true;
                rebuild(&mut commands, &mut pause, &settings);
            }
            Action::FovUp => {
                settings.fov_deg = (settings.fov_deg + 5.).min(110.);
            }
            Action::FovDown => {
                settings.fov_deg = (settings.fov_deg - 5.).max(50.);
            }
            Action::SensUp => {
                settings.sensitivity = (settings.sensitivity + 0.1).min(5.0);
            }
            Action::SensDown => {
                settings.sensitivity = (settings.sensitivity - 0.1).max(0.1);
            }
            Action::Fullscreen => {
                settings.fullscreen = !settings.fullscreen;
            }
            Action::HostLan => {
                if matches!(&*role, crate::netplay::NetRole::Offline) {
                    match crate::net::Host::start(
                        crate::net::DEFAULT_PORT,
                        "Fourlokos",
                        space.universe.seed,
                        &settings.player_name,
                    ) {
                        Ok(host) => {
                            *role = crate::netplay::NetRole::Host(host);
                            info!("LAN host started on port {}", crate::net::DEFAULT_PORT);
                        }
                        Err(e) => warn!("LAN host failed: {e}"),
                    }
                }
            }
            Action::CloseLan => {
                if !matches!(&*role, crate::netplay::NetRole::Offline) {
                    *role = crate::netplay::NetRole::Offline;
                    info!("LAN session closed");
                }
            }
            Action::Return => {
                *role = crate::netplay::NetRole::Offline;
                returns.write(ReturnToTitle);
                close(&mut commands, &mut pause, &mut cursor);
            }
        }
    }
}

fn style(mut q: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), With<Action>>) {
    for (hover, pressed, mut bg) in &mut q {
        bg.0 = if pressed {
            PRESS
        } else if hover.get() {
            HOVER
        } else {
            BUTTON
        };
    }
}

fn hold_input(
    pause: Res<PauseState>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
) {
    if pause.open {
        keys.reset_all();
        mouse.reset_all();
    }
}
