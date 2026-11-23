//! Planet customization menu (P): reshape the planet you stand on, every knob of its generation
//! (size, relief, surface blocks, climate, plants, caves, ores, day and tilt, seed). Changes are
//! drafted in the panel and take effect with Apply, which generates the planet again (edits on it
//! are lost) and sets you down on its new surface. Customizations are kept for the session, so
//! the planet comes back the same after streaming out.
//!
//! Debug: `ORBIT_MENU=1` opens it by itself on the first planet you stand on.

use crate::player::Player;
use crate::world::{Clock, Space, reshape};
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block::{self, Block};
use orbit_core::planet::{Biome, Customization, Gen, MAX_SCALE, Terrain};
use orbit_core::rng::mix;
use orbit_core::universe::Universe;

pub struct MenuPlugin;

impl Plugin for MenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlanetMenu>()
            .add_observer(on_activate)
            .add_systems(
                Update,
                (toggle, run_actions, refresh, style_buttons)
                    .chain()
                    .after(crate::world::MovePlanets),
            );
    }
}

/// The open menu's draft, if any.
#[derive(Resource, Default)]
pub struct PlanetMenu {
    open: Option<Draft>,
    /// Clicks waiting to be handled.
    queue: Vec<Action>,
}

impl PlanetMenu {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

struct Draft {
    planet: u64,
    name: String,
    /// What the planet is now.
    current: Customization,
    /// What it will be on Apply.
    edit: Customization,
    root: Entity,
}

/// One setting of the panel.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Knob {
    Radius,
    Relief,
    Mountains,
    Ranges,
    Roughness,
    Type,
    Top,
    Soil,
    SoilDepth,
    Stone,
    Warmth,
    Moisture,
    Trees,
    Grass,
    Flowers,
    Caves,
    CaveSize,
    Ores,
    Oceans,
    Rivers,
    Day,
    Tilt,
}

pub(crate) type Section = (&'static str, &'static [(Knob, &'static str)]);

/// The panel's sections, in two columns.
pub(crate) const LEFT: [Section; 3] = [
    (
        "Shape",
        &[
            (Knob::Radius, "Radius"),
            (Knob::Relief, "Hills"),
            (Knob::Roughness, "Roughness"),
            (Knob::Mountains, "Mountains"),
            (Knob::Ranges, "Ranges"),
        ],
    ),
    (
        "Surface",
        &[
            (Knob::Type, "Type"),
            (Knob::Top, "Top block"),
            (Knob::Soil, "Soil block"),
            (Knob::SoilDepth, "Soil depth"),
            (Knob::Stone, "Rock block"),
        ],
    ),
    (
        "Climate",
        &[(Knob::Warmth, "Warmth"), (Knob::Moisture, "Moisture")],
    ),
];
pub(crate) const RIGHT: [Section; 4] = [
    (
        "Life",
        &[
            (Knob::Trees, "Trees"),
            (Knob::Grass, "Tall grass"),
            (Knob::Flowers, "Flowers"),
        ],
    ),
    (
        "Water",
        &[(Knob::Oceans, "Oceans"), (Knob::Rivers, "Rivers")],
    ),
    (
        "Underground",
        &[
            (Knob::Caves, "Caves"),
            (Knob::CaveSize, "Cave size"),
            (Knob::Ores, "Ores"),
        ],
    ),
    (
        "Sky",
        &[(Knob::Day, "Day length"), (Knob::Tilt, "Axial tilt")],
    ),
];

/// Blocks the surface pickers run through; AIR stands for "the biome's own".
fn surface_blocks() -> Vec<Block> {
    std::iter::once(block::AIR)
        .chain(
            (0..block::COUNT as Block)
                .filter(|&b| block::is_solid(b) && b != block::CORE && b != block::STATION_CORE),
        )
        .collect()
}

fn round(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

impl Knob {
    /// Moves the setting a step (dir ±1).
    pub(crate) fn step(self, c: &mut Customization, dir: f32) {
        let g = &mut c.knobs;
        let scale = |x: &mut f32, lo: f32, hi: f32| *x = round(*x + dir * 0.1, 0.1).clamp(lo, hi);
        let cycle = |b: &mut Block| {
            let all = surface_blocks();
            let i = all.iter().position(|x| x == b).unwrap_or(0) as i32;
            *b = all[(i + dir as i32).rem_euclid(all.len() as i32) as usize];
        };
        match self {
            Knob::Radius => {
                c.radius = (c.radius + dir * 10.0).clamp(c.radius_range.0, c.radius_range.1)
            }
            Knob::Relief => scale(&mut g.relief, 0.0, MAX_SCALE),
            Knob::Mountains => scale(&mut g.mountains, 0.0, MAX_SCALE),
            Knob::Ranges => scale(&mut g.ranges, 0.0, 1.0),
            Knob::Roughness => g.roughness = round(g.roughness + dir * 0.25, 0.25).clamp(0.25, 3.0),
            Knob::Type => {
                let all = Biome::ALL;
                let i = all.iter().position(|&b| b == c.biome).unwrap_or(0) as i32;
                c.biome = all[(i + dir as i32).rem_euclid(all.len() as i32) as usize];
            }
            Knob::Top => cycle(&mut g.top),
            Knob::Soil => cycle(&mut g.soil),
            Knob::Stone => cycle(&mut g.stone),
            Knob::SoilDepth => scale(&mut g.soil_depth, 0.0, 3.0),
            Knob::Warmth => scale(&mut g.warmth, -1.0, 1.0),
            Knob::Moisture => scale(&mut g.moisture, -1.0, 1.0),
            Knob::Trees => scale(&mut g.trees, 0.0, 3.0),
            Knob::Grass => scale(&mut g.grass, 0.0, 3.0),
            Knob::Flowers => scale(&mut g.flowers, 0.0, 3.0),
            Knob::Caves => scale(&mut g.caves, 0.0, MAX_SCALE),
            Knob::CaveSize => scale(&mut g.cave_size, 0.5, 2.0),
            Knob::Ores => scale(&mut g.ores, 0.0, 3.0),
            Knob::Oceans => scale(&mut g.oceans, 0.0, 2.0),
            Knob::Rivers => scale(&mut g.rivers, 0.0, 2.0),
            Knob::Day => c.day = (round(c.day, 60.0) + dir * 60.0).clamp(60.0, 7200.0),
            Knob::Tilt => c.tilt = (round(c.tilt, 5.0) + dir * 5.0).clamp(0.0, 90.0),
        }
    }

    pub(crate) fn show(self, c: &Customization) -> String {
        let g = &c.knobs;
        let pct = |x: f32| format!("{:.0}%", x * 100.0);
        let signed = |x: f32| {
            if x.abs() < 0.05 {
                "normal".to_string()
            } else {
                format!("{:+.0}%", x * 100.0)
            }
        };
        let blk = |b: Block| {
            if b == block::AIR {
                "Auto".to_string()
            } else {
                block::name(b).to_string()
            }
        };
        match self {
            Knob::Radius => format!("{}", c.radius as i32),
            Knob::Relief => pct(g.relief),
            Knob::Mountains => pct(g.mountains),
            Knob::Ranges => pct(g.ranges),
            Knob::Roughness => format!("{:.2}x", g.roughness),
            Knob::Type => format!("{:?}", c.biome),
            Knob::Top => blk(g.top),
            Knob::Soil => blk(g.soil),
            Knob::Stone => blk(g.stone),
            Knob::SoilDepth => pct(g.soil_depth),
            Knob::Warmth => signed(g.warmth),
            Knob::Moisture => signed(g.moisture),
            Knob::Trees => pct(g.trees),
            Knob::Grass => pct(g.grass),
            Knob::Flowers => pct(g.flowers),
            Knob::Caves => pct(g.caves),
            Knob::CaveSize => pct(g.cave_size),
            Knob::Ores => pct(g.ores),
            Knob::Oceans => pct(g.oceans),
            Knob::Rivers => pct(g.rivers),
            Knob::Day => format!("{:.0} min", c.day / 60.0),
            Knob::Tilt => format!("{:.0} deg", c.tilt),
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum Action {
    Step(Knob, i8),
    NewSeed,
    Defaults,
    Original,
    Revert,
    Apply,
    Close,
}

/// Texts that show the draft.
#[derive(Component, Clone, Copy)]
enum Field {
    Title,
    Value(Knob),
    Seed,
    Status,
}

const PANEL: Color = Color::srgba(0.04, 0.06, 0.1, 0.9);
const BUTTON: Color = Color::srgb(0.16, 0.2, 0.28);
const HOVER: Color = Color::srgb(0.24, 0.3, 0.42);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);
const ACCENT: Color = Color::srgb(0.25, 0.5, 0.35);
const MUTED: Color = Color::srgb(0.62, 0.68, 0.78);
const CHANGED: Color = Color::srgb(1.0, 0.85, 0.4);

fn on_activate(ev: On<Activate>, actions: Query<&Action>, mut menu: ResMut<PlanetMenu>) {
    if let Ok(&a) = actions.get(ev.entity) {
        menu.queue.push(a);
    }
}

/// P opens the menu for the planet underfoot and closes it; it also closes when you leave the
/// planet, or with Esc.
fn toggle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut menu: ResMut<PlanetMenu>,
    space: Res<Space>,
    player: Single<&Player>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut auto: Local<bool>,
) {
    let here = player.body.gravity_source;
    // Debug: ORBIT_MENU=1 opens the menu once, on the first planet stood on.
    let open_now = keys.just_pressed(KeyCode::KeyP)
        || (!*auto && here.is_some() && std::env::var("ORBIT_MENU").is_ok_and(|v| v == "1") && {
            *auto = true;
            true
        });
    let wants_close = keys.just_pressed(KeyCode::Escape)
        || menu.open.as_ref().is_some_and(|d| here != Some(d.planet));
    if menu.is_open() && (keys.just_pressed(KeyCode::KeyP) || wants_close) {
        close(&mut commands, &mut menu);
        return;
    }
    if menu.is_open() || !open_now {
        return;
    }
    let Some(slot) = here.and_then(|id| space.planets.get(&id)) else {
        return;
    };
    let current = Customization::of(&slot.spec);
    let root = spawn_panel(&mut commands);
    menu.open = Some(Draft {
        planet: slot.spec.id,
        name: slot.spec.name.clone(),
        current,
        edit: current,
        root,
    });
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

fn close(commands: &mut Commands, menu: &mut PlanetMenu) {
    if let Some(d) = menu.open.take() {
        commands.entity(d.root).despawn();
    }
    menu.queue.clear();
}

/// The planet as the universe made it, before anyone customized it.
fn original(space: &Space, id: u64) -> Option<Customization> {
    let slot = space.planets.get(&id)?;
    let sys = space
        .universe
        .system(Universe::sector_of(slot.spec.star()))?;
    sys.planets
        .iter()
        .find(|p| p.id == id)
        .map(Customization::of)
}

#[allow(clippy::too_many_arguments)]
fn run_actions(
    mut commands: Commands,
    mut menu: ResMut<PlanetMenu>,
    mut space: ResMut<Space>,
    mut meshes: ResMut<Assets<Mesh>>,
    clock: Res<Clock>,
    mut player: Single<&mut Player>,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
) {
    let queue = std::mem::take(&mut menu.queue);
    // Shift-click takes five steps at once.
    let times = if keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight) {
        5
    } else {
        1
    };
    for action in queue {
        let Some(d) = menu.open.as_mut() else { return };
        match action {
            Action::Step(k, dir) => {
                for _ in 0..times {
                    k.step(&mut d.edit, dir as f32);
                }
            }
            Action::NewSeed => d.edit.seed = mix(d.edit.seed ^ time.elapsed().as_nanos() as u64),
            Action::Defaults => d.edit.knobs = Gen::default(),
            Action::Original => {
                if let Some(o) = original(&space, d.planet) {
                    d.edit = o;
                }
            }
            Action::Revert => d.edit = d.current,
            Action::Close => close(&mut commands, &mut menu),
            Action::Apply if d.edit == d.current => {}
            Action::Apply => {
                let (id, c) = (d.planet, d.edit);
                let Some(slot) = space.planets.get_mut(&id) else {
                    continue;
                };
                let mut spec = slot.spec.clone();
                c.apply(&mut spec, clock.0);
                d.current = Customization::of(&spec);
                d.edit = d.current;
                // Set the player down on the new surface, right where they stood.
                let body = &mut player.body;
                if body.gravity_source == Some(id) {
                    let dir = body.pos.normalize_or(body.up);
                    let ground = spec.radius + Terrain::new(&spec).column(dir).ground() as f32;
                    body.pos = dir * (ground + 1.0);
                    body.vel = Vec3::ZERO;
                }
                reshape(&mut commands, slot, &mut meshes, spec);
                space.custom.insert(id, d.current);
                refit_family(&mut space, id, clock.0);
            }
        }
    }
}

/// Keeps the panel's values in step with the draft; changed values show in yellow.
fn refresh(
    menu: Res<PlanetMenu>,
    mut texts: Query<(&Field, &mut Text, &mut TextColor)>,
    space: Res<Space>,
) {
    let Some(d) = menu.open.as_ref() else { return };
    let (e, c) = (&d.edit, &d.current);
    let generating = space
        .planets
        .get(&d.planet)
        .is_some_and(|p| p.voxel.is_none());
    for (field, mut text, mut color) in &mut texts {
        let (s, want) = match field {
            Field::Title => (format!("Planet {}", d.name), Color::WHITE),
            Field::Value(k) => {
                let now = k.show(e);
                let changed = now != k.show(c);
                (now, if changed { CHANGED } else { Color::WHITE })
            }
            Field::Seed => (
                format!("{:08x}", e.seed as u32),
                if e.seed != c.seed {
                    CHANGED
                } else {
                    Color::WHITE
                },
            ),
            Field::Status => (
                if generating {
                    "Generating...".into()
                } else if e != c {
                    "Apply rebuilds the planet: your builds on it are lost.".into()
                } else {
                    "Shift-click: 5 steps.  P or Esc closes.".into()
                },
                MUTED,
            ),
        };
        if text.0 != s {
            text.0 = s;
        }
        if color.0 != want {
            color.0 = want;
        }
    }
}

fn style_buttons(
    mut buttons: Query<(&Hovered, Has<Pressed>, &Action, &mut BackgroundColor), With<Button>>,
    menu: Res<PlanetMenu>,
) {
    let dirty = menu.open.as_ref().is_some_and(|d| d.edit != d.current);
    for (hovered, pressed, action, mut bg) in &mut buttons {
        let base = if *action == Action::Apply && dirty {
            ACCENT
        } else {
            BUTTON
        };
        let c = if pressed {
            PRESS
        } else if hovered.get() {
            HOVER
        } else {
            base
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
}

fn colored(s: &str, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(s),
        TextFont {
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
    )
}

fn button(label: &str, action: Action, width: f32) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node {
            width: px(width),
            height: px(24),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border_radius: BorderRadius::all(px(5)),
            ..default()
        },
        BackgroundColor(BUTTON),
        children![colored(label, 14.0, Color::WHITE)],
    )
}

fn row(top: f32) -> Node {
    Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: px(6),
        margin: UiRect::top(px(top)),
        ..default()
    }
}

/// A row: label, [-] value [+].
fn stepper(parent: &mut ChildSpawnerCommands, knob: Knob, label: &str) {
    parent.spawn(row(0.0)).with_children(|r| {
        r.spawn((
            colored(label, 14.0, MUTED),
            Node {
                width: px(86),
                ..default()
            },
        ));
        r.spawn(button("-", Action::Step(knob, -1), 26.0));
        r.spawn((
            colored("", 14.0, Color::WHITE),
            Field::Value(knob),
            Node {
                width: px(150),
                justify_content: JustifyContent::Center,
                ..default()
            },
            TextLayout::justify(Justify::Center),
        ));
        r.spawn(button("+", Action::Step(knob, 1), 26.0));
    });
}

fn column(parent: &mut ChildSpawnerCommands, sections: &[Section]) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(5),
            ..default()
        })
        .with_children(|col| {
            for (title, knobs) in sections {
                col.spawn((
                    colored(title, 15.0, Color::srgb(0.55, 0.8, 1.0)),
                    Node {
                        margin: UiRect::top(px(6)),
                        ..default()
                    },
                ));
                for &(k, label) in *knobs {
                    stepper(col, k, label);
                }
            }
        });
}

fn spawn_panel(commands: &mut Commands) -> Entity {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: px(12),
                top: px(12),
                flex_direction: FlexDirection::Column,
                row_gap: px(6),
                padding: UiRect::all(px(12)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            BackgroundColor(PANEL),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            p.spawn((colored("", 18.0, Color::WHITE), Field::Title));
            p.spawn(colored("Customize the planet you stand on", 13.0, MUTED));
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: px(18),
                ..default()
            })
            .with_children(|cols| {
                column(cols, &LEFT);
                column(cols, &RIGHT);
            });
            p.spawn(row(8.0)).with_children(|r| {
                r.spawn(colored("Seed", 14.0, MUTED));
                r.spawn((
                    colored("", 14.0, Color::WHITE),
                    Field::Seed,
                    Node {
                        width: px(76),
                        ..default()
                    },
                ));
                r.spawn(button("New seed", Action::NewSeed, 90.0));
                r.spawn(button("Defaults", Action::Defaults, 90.0));
            });
            p.spawn(row(4.0)).with_children(|r| {
                r.spawn(button("Apply", Action::Apply, 110.0));
                r.spawn(button("Revert", Action::Revert, 110.0));
                r.spawn(button("Original", Action::Original, 110.0));
                r.spawn(button("Close", Action::Close, 110.0));
            });
            p.spawn((colored("", 13.0, MUTED), Field::Status));
        })
        .id()
}

/// After a planet (or moon) changed: its moons, or the moon itself, fit their orbits to it.
fn refit_family(space: &mut Space, id: u64, t: f64) {
    let Some(changed) = space.planets.get(&id).map(|p| p.spec.clone()) else {
        return;
    };
    let host = match changed.host {
        Some(h) => space.planets.get(&h.id).map(|p| p.spec.clone()),
        None => Some(changed.clone()),
    };
    let Some(host) = host else { return };
    for slot in space.planets.values_mut() {
        if slot.spec.host.is_some_and(|h| h.id == host.id) {
            slot.spec.refit(&host, t);
        }
    }
}
