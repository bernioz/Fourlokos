//! The inventory, as Minecraft's (the slots and their rules are `orbit_core::inventory`):
//!
//! - The **hotbar** along the bottom of the screen: 1-9 or the wheel pick the slot in hand, its
//!   name shows over it for a moment when it changes.
//! - **E** opens the inventory screen: the 27 slots and the hotbar, and in creative every item
//!   over them (the wheel scrolls them). Left click picks a stack up or puts it down (merging,
//!   swapping), right click takes half or puts one down, Shift + click sends a stack between the
//!   hotbar and the rest, 1-9 over a slot swap it with that hotbar slot, Q over one throws one
//!   (Ctrl + Q all). Clicking outside the panel throws what the mouse holds. E or Esc closes it.
//! - **Items in the world** (`orbit_core::item`): in survival, broken blocks drop what they give
//!   (stone cobblestone, grass dirt...), and Q throws what is in hand (Ctrl + Q the stack); they
//!   fall, rest, float in water, and are picked up by walking over them. They spin and bob.
//! - Middle click on a block takes it in hand (in creative, a full stack if you have none).
//! - **Grass** spreads over dirt near it under the open sky, and dies back under blocks, by
//!   random ticks round the player (as Minecraft's), on planets and stations.

use crate::chat::{Keyboard, Owner};
use crate::lighting::VoxelMaterial;
use crate::map::GalaxyMap;
use crate::menu::PlanetMenu;
use crate::player::Player;
use crate::station::{StationMenu, Stations};
use crate::textures::{ICON, Icons};
use crate::world::{Clock, Space, gpu_mesh, remesh};
use bevy::input::mouse::AccumulatedMouseScroll;
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block::{self, Block};
use orbit_core::inventory::{GameMode, HOTBAR, Stack};
use orbit_core::item::{ItemDrop, LIFETIME, PICKUP_RANGE};
use orbit_core::rng::Rng;
use orbit_core::station::Station;

pub struct InventoryPlugin;

impl Plugin for InventoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Drops>()
            .init_resource::<Screen>()
            .init_resource::<Ticks>()
            .add_systems(Startup, (load_drop_assets, spawn_hotbar))
            .add_systems(
                Update,
                (
                    open_screen.after(crate::elevator::run_elevators),
                    run_screen,
                    throw,
                    run_drops,
                    draw_drops,
                    draw_hotbar,
                    draw_durability,
                    draw_furnace_screen,
                    grass,
                )
                    .chain()
                    .after(crate::player::interact),
            );
    }
}

const SLOT: f32 = 36.0;
const GAP: f32 = 2.0;
const PAD: f32 = 10.0;
const PALETTE_ROWS: usize = 5;
const SLOT_BG: Color = Color::srgba(0.16, 0.17, 0.2, 0.92);
const SLOT_HOVER: Color = Color::srgba(0.32, 0.34, 0.4, 0.95);
const PANEL: Color = Color::srgba(0.07, 0.08, 0.1, 0.94);
const HOTBAR_BG: Color = Color::srgba(0.05, 0.05, 0.07, 0.6);
const SELECTED: Color = Color::srgb(0.95, 0.95, 0.95);
const MUTED: Color = Color::srgb(0.63, 0.63, 0.63);
/// Most items lying about at once (the oldest go first).
const MAX_DROPS: usize = 400;
const EGG_MODEL: &str = "models/egg.gltf";
/// `egg.gltf` is 16 pixels tall; 1/80 makes the loose egg about 0.2 m high.
const EGG_MODEL_SCALE: f32 = 1.0 / 80.0;

#[cfg(test)]
mod egg_model_scale_tests {
    use super::*;

    #[test]
    fn egg_pixel_extent_is_about_twenty_centimetres() {
        assert!((16.0 * EGG_MODEL_SCALE - 0.2).abs() < f32::EPSILON);
    }
}

#[derive(Resource)]
struct DropAssets {
    egg: Handle<WorldAsset>,
}

fn load_drop_assets(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(DropAssets {
        egg: assets.load(GltfAssetLabel::Scene(0).from_asset(EGG_MODEL)),
    });
}

// --- Items in the world ---------------------------------------------------------------------

/// Whose frame an item lies in.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Frame {
    Planet(u64),
    Station(String),
}

pub struct Dropped {
    pub frame: Frame,
    pub d: ItemDrop,
    entity: Option<(Entity, Entity)>,
    /// The light level its mesh was made with.
    light: (u8, u8),
    spin: f32,
    model: bool,
}

#[derive(Resource, Default)]
pub struct Drops {
    pub list: Vec<Dropped>,
    rng: Option<Rng>,
}

impl Drops {
    fn rand3(&mut self) -> [f32; 3] {
        let rng = self.rng.get_or_insert_with(|| Rng::new(0xD20B5));
        [rng.f32(), rng.f32(), rng.f32()]
    }

    /// One of `item` pops out of a cell (its middle `center`, `up` away from the ground).
    pub fn pop(&mut self, frame: Frame, center: Vec3, up: Vec3, item: Block) {
        let r = self.rand3();
        let d = ItemDrop::popped(Stack::new(item, 1), center, up, r);
        self.push(frame, d);
    }

    /// What breaking blocks gives in survival: each block's drop, where it was.
    pub fn broke_on_planet(
        &mut self,
        planet: u64,
        g: &orbit_core::sphere::CubeSphere,
        gone: &[(usize, Block)],
        tool: Option<Block>,
    ) {
        for &(c, b) in gone {
            if let Some(item) =
                orbit_core::mining::drops(b, tool, orbit_core::rng::mix(planet ^ c as u64))
            {
                let center = g.center(c);
                self.pop(Frame::Planet(planet), center, center.normalize(), item);
            }
        }
    }

    pub fn broke_on_station(
        &mut self,
        station: &str,
        gone: &[(IVec3, Block)],
        tool: Option<Block>,
    ) {
        for &(p, b) in gone {
            let key = orbit_core::rng::mix(
                (p.x as i64 as u64) ^ ((p.y as i64 as u64) << 20) ^ ((p.z as i64 as u64) << 40),
            );
            if let Some(item) = orbit_core::mining::drops(b, tool, key) {
                self.pop(
                    Frame::Station(station.into()),
                    Station::cell_center(p),
                    Vec3::Y,
                    item,
                );
            }
        }
    }

    /// A whole stack pops out of a cell.
    pub fn pop_stack(&mut self, frame: Frame, center: Vec3, up: Vec3, stack: Stack) {
        let r = self.rand3();
        self.push(frame, ItemDrop::popped(stack, center, up, r));
    }

    pub fn push(&mut self, frame: Frame, d: ItemDrop) {
        if self.list.len() >= MAX_DROPS {
            self.list.remove(0); // its entity is cleaned up by `draw_drops` (it looks for orphans)
        }
        let spin = self.rand3()[0] * std::f32::consts::TAU;
        self.list.push(Dropped {
            frame,
            d,
            entity: None,
            light: (255, 255),
            spin,
            model: false,
        });
    }
}

/// Marks an item's entity (so ones whose item is gone are found and despawned).
#[derive(Component)]
struct DropMesh;

/// Where a frame is now: (world anchor, rotation), None if it is not around.
fn frame_of(frame: &Frame, space: &Space, stations: &Stations, t: f64) -> Option<(DVec3, Quat)> {
    match frame {
        Frame::Planet(id) => space
            .planets
            .get(id)
            .filter(|p| p.voxel.is_some())
            .map(|p| (p.spec.center, p.spec.rotation)),
        Frame::Station(id) => stations
            .by_id(id)
            .and_then(|s| Stations::frame(&s.station.read().unwrap(), space, t))
            .map(|f| (f.center, f.rotation)),
    }
}

/// Items fall and rest; near the player they are picked up; close ones of a kind merge; old
/// ones (and those whose planet or station has gone) are gone.
fn run_drops(
    mut drops: ResMut<Drops>,
    mut player: Single<&mut Player>,
    space: Res<Space>,
    stations: Res<Stations>,
    clock: Res<Clock>,
    time: Res<Time>,
) {
    let dt = time.delta_secs().min(0.05);
    let drops = &mut *drops;
    let body = &player.body;
    let mid = body.world_point(body.pos + body.up * 0.9);
    let mut picked: Vec<(usize, Stack)> = Vec::new();
    let mut keep = vec![true; drops.list.len()];
    for (i, d) in drops.list.iter_mut().enumerate() {
        let moved = match &d.frame {
            Frame::Planet(id) => space
                .planets
                .get(id)
                .and_then(|p| p.voxel.as_ref())
                .map(|v| d.d.step_planet(&v.read().unwrap(), dt)),
            Frame::Station(id) => stations
                .by_id(id)
                .map(|s| d.d.step_station(&s.station.read().unwrap(), dt)),
        };
        if moved.is_none() || d.d.age > LIFETIME {
            keep[i] = false;
            continue;
        }
        let Some((center, rot)) = frame_of(&d.frame, &space, &stations, clock.0) else {
            continue;
        };
        let at = center + (rot * (d.d.pos + d.up() * 0.12)).as_dvec3();
        if d.d.delay <= 0.0 && at.distance(mid) < PICKUP_RANGE as f64 && body.cabin.is_none() {
            picked.push((i, d.d.stack));
        }
    }
    for (i, s) in picked {
        match player.inv.add_stack(s) {
            None => keep[i] = false,
            Some(rest) => drops.list[i].d.stack = rest,
        }
    }
    let mut k = keep.iter();
    drops.list.retain(|_| *k.next().unwrap());
    // Merge neighbors of a kind (few items: the simple way).
    let n = drops.list.len();
    let mut gone = vec![false; n];
    for a in 0..n {
        for b in a + 1..n {
            if gone[a] || gone[b] || drops.list[a].frame != drops.list[b].frame {
                continue;
            }
            let (x, y) = drops.list.split_at_mut(b);
            if x[a].d.merge(&mut y[0].d) {
                gone[b] = true;
            }
        }
    }
    let mut g = gone.iter();
    drops.list.retain(|_| !*g.next().unwrap());
}

impl Dropped {
    fn up(&self) -> Vec3 {
        match self.frame {
            Frame::Planet(_) => self.d.pos.normalize_or(Vec3::Y),
            Frame::Station(_) => Vec3::Y,
        }
    }
}

/// Each item drawn as a child of its planet or station, lit by the light where it lies.
#[allow(clippy::too_many_arguments)]
fn draw_drops(
    mut commands: Commands,
    mut drops: ResMut<Drops>,
    assets: Res<DropAssets>,
    server: Res<AssetServer>,
    space: Res<Space>,
    stations: Res<Stations>,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut tr: Query<&mut Transform, With<DropMesh>>,
    marked: Query<Entity, (With<DropMesh>, With<ChildOf>)>,
    parents: Query<&ChildOf>,
) {
    let t = time.elapsed_secs();
    // Entities kept, and those despawned here (still in the query until the commands run).
    let mut alive: Vec<Entity> = Vec::new();
    for d in &mut drops.list {
        let (parent, mat, light): (Option<Entity>, Option<Handle<VoxelMaterial>>, (u8, u8)) =
            match &d.frame {
                Frame::Planet(id) => match space.planets.get(id) {
                    Some(p) => {
                        let l = p
                            .voxel
                            .as_ref()
                            .and_then(|v| v.read().unwrap().light_at(d.d.pos + d.up() * 0.1))
                            .unwrap_or((15, 0));
                        (Some(p.root), Some(p.material()), l)
                    }
                    None => (None, None, (15, 0)),
                },
                Frame::Station(id) => stations.by_id(id).map_or((None, None, (15, 0)), |s| {
                    (s.entity(), s.material(), (15, 0))
                }),
            };
        let (Some(parent), Some(mat)) = (parent, mat) else {
            continue;
        };
        // Rebuilt when its parent changed (a station drawn again) or its light did.
        let stale = d
            .entity
            .is_some_and(|(e, p)| p != parent || parents.get(e).map_or(true, |c| c.parent() != p));
        let egg_model = d.d.stack.item == orbit_core::items::EGG
            && matches!(
                server.get_load_state(assets.egg.id()),
                Some(bevy::asset::LoadState::Loaded)
            );
        if stale || (d.entity.is_some() && d.light != light) || (d.entity.is_some() && d.model != egg_model) {
            if let Some((e, _)) = d.entity.take() {
                commands.entity(e).despawn();
                alive.push(e);
            }
        }
        if d.entity.is_none() {
            let lit = [
                orbit_core::light::brightness(light.0 as f32),
                orbit_core::light::glow(light.1 as f32),
            ];
            let e = if egg_model {
                commands
                    .spawn((
                        DropMesh,
                        WorldAssetRoot(assets.egg.clone()),
                        Transform::from_scale(Vec3::splat(EGG_MODEL_SCALE)),
                        Visibility::default(),
                        ChildOf(parent),
                    ))
                    .id()
            } else {
                let (mesh, aabb) = gpu_mesh(
                    &mut meshes,
                    orbit_core::mesh::item_mesh(d.d.stack.item, lit),
                );
                commands
                    .spawn((
                        DropMesh,
                        Mesh3d(mesh),
                        aabb,
                        MeshMaterial3d(mat),
                        Transform::default(),
                        Visibility::default(),
                        ChildOf(parent),
                    ))
                    .id()
            };
            d.entity = Some((e, parent));
            d.light = light;
            d.model = egg_model;
        }
        let (e, _) = d.entity.unwrap();
        alive.push(e);
        let up = d.up();
        let bob = 0.1 + 0.06 * (t * 2.2 + d.spin).sin();
        let turn = Quat::from_rotation_arc(Vec3::Y, up) * Quat::from_rotation_y(t * 1.6 + d.spin);
        if let Ok(mut x) = tr.get_mut(e) {
            x.translation = d.d.pos + up * bob;
            x.rotation = turn;
            // A stack of more than one shows a little bigger.
            let stack_scale = if d.d.stack.count > 1 { 1.15 } else { 1.0 };
            x.scale = Vec3::splat(if d.model {
                EGG_MODEL_SCALE * stack_scale
            } else {
                stack_scale
            });
        }
    }
    for e in &marked {
        if !alive.contains(&e) {
            commands.entity(e).despawn();
        }
    }
}

/// Q throws one of what is in hand (Ctrl + Q the whole stack), where you look.
fn throw(
    mut player: Single<&mut Player>,
    keys: Res<ButtonInput<KeyCode>>,
    keyboard: Res<Keyboard>,
    mut drops: ResMut<Drops>,
    space: Res<Space>,
    rockets: Res<crate::rocket::Rockets>,
) {
    if keyboard.owner.is_some() || !keys.just_pressed(KeyCode::KeyQ) || rockets.is_piloting() {
        return;
    }
    let all = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let p = &mut **player;
    let Some(frame) = body_frame(&p.body, &space) else {
        return;
    };
    let n = if all { 64 } else { 1 };
    let sel = p.inv.selected;
    let Some(stack) = p.inv.take(sel, n) else {
        return;
    };
    let up = if matches!(frame, Frame::Station(_)) {
        Vec3::Y
    } else {
        p.body.up
    };
    drops.push(
        frame,
        ItemDrop::thrown(stack, p.body.eye(), p.body.look_dir(), up),
    );
}

/// The frame the body stands in, if items can lie there (a planet with its blocks, a station).
fn body_frame(body: &orbit_core::player::Body, space: &Space) -> Option<Frame> {
    if let Some(id) = &body.station {
        return Some(Frame::Station(id.clone()));
    }
    let id = body.gravity_source?;
    space
        .planets
        .get(&id)
        .filter(|p| p.voxel.is_some())
        .map(|_| Frame::Planet(id))
}

// --- The hotbar -----------------------------------------------------------------------------

#[derive(Component)]
struct HotbarRoot;
#[derive(Component)]
struct HotSlot(usize);
#[derive(Component)]
struct HotIcon(usize);
#[derive(Component)]
struct HotCount(usize);
#[derive(Component)]
struct HeldName;
/// The left hand's slot, left of the hotbar.
#[derive(Component)]
struct OffIcon;
#[derive(Component)]
struct OffCount;

/// What the hotbar showed, to show the item's name when it changes.
#[derive(Default)]
struct Shown {
    held: Option<(usize, Option<Block>)>,
    since: f32,
}

/// Where a durability bar reads its tool from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DuraOf {
    Hot(usize),
    Off,
    Screen(SlotRef),
}

#[derive(Component)]
struct DuraBack(DuraOf);
#[derive(Component)]
struct DuraFill(DuraOf);

const DURA_W: f32 = 28.0;

/// A thin bar under a slot's icon, there while its tool is worn.
fn spawn_dura(s: &mut ChildSpawnerCommands, of: DuraOf, left: f32) {
    s.spawn((
        DuraBack(of),
        Node {
            position_type: PositionType::Absolute,
            left: px(left),
            bottom: px(2),
            width: px(DURA_W + 2.0),
            height: px(5),
            padding: UiRect::all(px(1)),
            ..default()
        },
        BackgroundColor(Color::BLACK),
        Visibility::Hidden,
    ))
    .with_children(|b| {
        b.spawn((
            DuraFill(of),
            Node {
                width: px(DURA_W),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::srgb(0.3, 0.9, 0.3)),
        ));
    });
}

/// The furnace screen's arrow and flame.
fn draw_furnace_screen(
    screen: Res<Screen>,
    furnaces: Res<crate::furnace::Furnaces>,
    mut fills: Query<(&FurnaceFill, &mut Node)>,
) {
    let f = screen
        .furnace
        .as_ref()
        .filter(|_| screen.open.is_some() && screen.kind == Kind::Furnace)
        .and_then(|k| furnaces.map.get(k));
    for (fill, mut n) in &mut fills {
        let Some(f) = f else { continue };
        if fill.flame {
            n.height = percent(
                100.0 * f.burn_left
                    / (orbit_core::furnace::ITEMS_PER_FUEL * orbit_core::furnace::SMELT_TIME),
            );
        } else {
            n.width = percent(100.0 * f.fraction());
        }
    }
}

fn draw_durability(
    player: Single<&Player>,
    mut back: Query<(&DuraBack, &mut Visibility)>,
    mut fill: Query<(&DuraFill, &mut Node, &mut BackgroundColor)>,
) {
    let inv = &player.inv;
    let worn = |of: DuraOf| -> Option<f32> {
        let s = match of {
            DuraOf::Hot(i) => inv.slots[i],
            DuraOf::Off => inv.offhand,
            DuraOf::Screen(SlotRef::Inv(i)) => inv.at(i),
            DuraOf::Screen(_) => None,
        }?;
        let max = orbit_core::items::durability(s.item);
        (max > 0 && s.wear > 0).then(|| s.uses_left() as f32 / max as f32)
    };
    for (d, mut v) in &mut back {
        let want = if worn(d.0).is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *v != want {
            *v = want;
        }
    }
    for (d, mut n, mut c) in &mut fill {
        if let Some(f) = worn(d.0) {
            n.width = px(DURA_W * f);
            c.0 = Color::srgb((2.0 * (1.0 - f)).min(1.0), (2.0 * f).min(1.0), 0.15);
        }
    }
}

fn icon(icons: &Icons, item: Option<Block>) -> (ImageNode, Visibility) {
    let mut img = ImageNode::new(icons.0.clone());
    img.rect = item.map(Icons::rect);
    (
        img,
        if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        },
    )
}

fn count_text(n: u8) -> (Text, TextFont, TextColor, TextShadow) {
    (
        Text::new(if n > 1 { n.to_string() } else { String::new() }),
        TextFont {
            font_size: FontSize::Px(14.0),
            ..default()
        },
        TextColor(Color::WHITE),
        TextShadow::default(),
    )
}

fn spawn_hotbar(mut commands: Commands, icons: Res<Icons>) {
    let w = HOTBAR as f32 * (SLOT + 4.0) + 8.0;
    commands
        .spawn((
            HotbarRoot,
            Node {
                position_type: PositionType::Absolute,
                bottom: px(6),
                left: percent(50),
                margin: UiRect::left(px(-w / 2.0)),
                width: px(w),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                ..default()
            },
            bevy::ui::FocusPolicy::Pass,
            GlobalZIndex(2),
        ))
        .with_children(|root| {
            // The left hand: its own slot, to the left of the bar.
            root.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(-(SLOT + 14.0)),
                    bottom: px(0),
                    padding: UiRect::all(px(4)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(HOTBAR_BG),
            ))
            .with_children(|o| {
                o.spawn((
                    Node {
                        width: px(SLOT),
                        height: px(SLOT),
                        border_radius: BorderRadius::all(px(3)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                ))
                .with_children(|s| {
                    let (img, vis) = icon(&icons, None);
                    s.spawn((
                        OffIcon,
                        img,
                        vis,
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(0),
                            top: px(0),
                            width: px(ICON as f32),
                            height: px(ICON as f32),
                            ..default()
                        },
                    ));
                    s.spawn((
                        OffCount,
                        count_text(0),
                        Node {
                            position_type: PositionType::Absolute,
                            right: px(1),
                            bottom: px(-2),
                            ..default()
                        },
                    ));
                    spawn_dura(s, DuraOf::Off, 2.0);
                });
            });
            root.spawn((
                HeldName,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextShadow::default(),
                Node {
                    margin: UiRect::bottom(px(6)),
                    ..default()
                },
            ));
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    padding: UiRect::all(px(4)),
                    column_gap: px(4),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                BackgroundColor(HOTBAR_BG),
            ))
            .with_children(|bar| {
                for i in 0..HOTBAR {
                    bar.spawn((
                        HotSlot(i),
                        Node {
                            width: px(SLOT),
                            height: px(SLOT),
                            border: UiRect::all(px(2)),
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BorderColor::all(Color::NONE),
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                    ))
                    .with_children(|s| {
                        let (img, vis) = icon(&icons, None);
                        s.spawn((
                            HotIcon(i),
                            img,
                            vis,
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(0),
                                top: px(0),
                                width: px(ICON as f32),
                                height: px(ICON as f32),
                                ..default()
                            },
                        ));
                        s.spawn((
                            HotCount(i),
                            count_text(0),
                            Node {
                                position_type: PositionType::Absolute,
                                right: px(1),
                                bottom: px(-2),
                                ..default()
                            },
                        ));
                        spawn_dura(s, DuraOf::Hot(i), 1.0);
                    });
                }
            });
        });
}

#[allow(clippy::too_many_arguments)]
fn draw_hotbar(
    player: Single<&Player>,
    map: Res<GalaxyMap>,
    screen: Res<Screen>,
    time: Res<Time>,
    mut shown: Local<Shown>,
    mut root: Single<&mut Visibility, With<HotbarRoot>>,
    mut slots: Query<(&HotSlot, &mut BorderColor)>,
    mut icons: Query<(&HotIcon, &mut ImageNode, &mut Visibility), Without<HotbarRoot>>,
    mut counts: Query<(&HotCount, &mut Text), Without<HeldName>>,
    mut name: Single<(&mut Text, &mut TextColor), With<HeldName>>,
    mut off_icon: Single<
        (&mut ImageNode, &mut Visibility),
        (With<OffIcon>, Without<HotbarRoot>, Without<HotIcon>),
    >,
    mut off_count: Single<&mut Text, (With<OffCount>, Without<HeldName>, Without<HotCount>)>,
    rockets: Res<crate::rocket::Rockets>,
    menu: Res<crate::start_menu::StartMenu>,
) {
    let hide = menu.is_open() || map.is_open() || screen.open.is_some() || rockets.is_piloting();
    let want = if hide {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    if **root != want {
        **root = want;
    }
    let inv = &player.inv;
    for (s, mut b) in &mut slots {
        let c = if s.0 == inv.selected {
            SELECTED
        } else {
            Color::NONE
        };
        if b.top != c {
            *b = BorderColor::all(c);
        }
    }
    for (i, mut img, mut vis) in &mut icons {
        let item = inv.slots[i.0].map(|s| s.item);
        let rect = item.map(Icons::rect);
        if img.rect != rect {
            img.rect = rect;
        }
        let v = if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != v {
            *vis = v;
        }
    }
    {
        let item = inv.offhand.map(|s| s.item);
        let rect = item.map(Icons::rect);
        if off_icon.0.rect != rect {
            off_icon.0.rect = rect;
        }
        let v = if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *off_icon.1 != v {
            *off_icon.1 = v;
        }
        let n = inv.offhand.map_or(0, |s| s.count);
        let txt = if n > 1 { n.to_string() } else { String::new() };
        if off_count.0 != txt {
            off_count.0 = txt;
        }
    }
    for (i, mut t) in &mut counts {
        let n = inv.slots[i.0].map_or(0, |s| s.count);
        let s = if n > 1 { n.to_string() } else { String::new() };
        if t.0 != s {
            t.0 = s;
        }
    }
    // The name of what is in hand, for two seconds after it changes, then fading.
    let held = (inv.selected, inv.held().map(|s| s.item));
    if shown.held != Some(held) {
        shown.held = Some(held);
        shown.since = time.elapsed_secs();
    }
    let age = time.elapsed_secs() - shown.since;
    let alpha = (1.0 - (age - 2.0) / 0.5).clamp(0.0, 1.0);
    let text = held
        .1
        .map(|b| block::name(b).to_string())
        .unwrap_or_default();
    let (t, c) = &mut *name;
    if t.0 != text {
        t.0 = text;
    }
    c.0 = Color::srgba(1.0, 1.0, 1.0, alpha);
}

// --- The inventory screen -------------------------------------------------------------------

/// A slot on the screen: one of the inventory's, or one of the creative palette's (by its place
/// on the screen; what it holds depends on the scroll).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SlotRef {
    Inv(usize),
    Palette(usize),
    Furnace(orbit_core::furnace::Slot),
}

/// The arrow and the flame of a furnace's screen: the fill grows with the work / shrinks with the fuel.
#[derive(Component)]
struct FurnaceFill {
    flame: bool,
}

#[derive(Component)]
struct ScreenSlot(SlotRef);
#[derive(Component)]
struct ScreenIcon(SlotRef);
#[derive(Component)]
struct ScreenCount(SlotRef);
#[derive(Component)]
struct CursorIcon;
#[derive(Component)]
struct CursorCount;
#[derive(Component)]
struct Tooltip;
#[derive(Component)]
struct ScreenPanel;

struct Open {
    root: Entity,
    /// Slots and where they are in the panel (top left, px).
    slots: Vec<(SlotRef, Vec2)>,
    size: Vec2,
    regrab: bool,
}

/// Which screen: the inventory's (E), with its 2 × 2 grid, or a crafting table's 3 × 3.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Kind {
    #[default]
    Inventory,
    Table,
    /// A furnace's: the one in `Screen::furnace`.
    Furnace,
}

#[derive(Resource, Default)]
pub struct Screen {
    open: Option<Open>,
    kind: Kind,
    /// A screen something asked for (right clicking a table); `open_screen` opens it.
    pub request: Option<Kind>,
    /// The furnace whose screen is, or is asked to be, open.
    pub furnace: Option<crate::furnace::Key>,
    scroll: usize,
    shift: bool,
}

impl Screen {
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }
    pub(crate) fn kind(&self) -> Kind {
        self.kind
    }
    /// Right clicking a furnace: its screen.
    pub fn request_furnace(&mut self, key: crate::furnace::Key) {
        self.request = Some(Kind::Furnace);
        self.furnace = Some(key);
    }
}

fn palette_rows() -> usize {
    orbit_core::items::creative().len().div_ceil(HOTBAR)
}

/// E opens the screen (unless E is for an elevator just now).
#[allow(clippy::too_many_arguments)]
pub(crate) fn open_screen(
    mut commands: Commands,
    mut screen: ResMut<Screen>,
    keys: Res<ButtonInput<KeyCode>>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut player: Single<&mut Player>,
    icons: Res<Icons>,
    elevators: Res<crate::elevator::Elevators>,
    menu: Res<PlanetMenu>,
    map: Res<GalaxyMap>,
    station_menu: Res<StationMenu>,
    rockets: Res<crate::rocket::Rockets>,
    mut furnaces: ResMut<crate::furnace::Furnaces>,
) {
    if screen.open.is_some()
        || keyboard.owner.is_some()
        || menu.is_open()
        || map.is_open()
        || station_menu.is_open()
        || rockets.is_piloting()
    {
        screen.request = None;
        return;
    }
    let kind = match screen.request.take() {
        Some(k) => k,
        None if keys.just_pressed(KeyCode::KeyE)
            && player.body.cabin.is_none()
            && !elevators.hud.starts_with("E") =>
        {
            Kind::Inventory
        }
        None => return,
    };
    let creative = player.inv.mode == GameMode::Creative && kind == Kind::Inventory;
    let furnace = kind == Kind::Furnace;
    if furnace {
        let Some(key) = screen.furnace.clone() else {
            return;
        };
        furnaces.map.entry(key).or_default();
    }
    let table = kind == Kind::Table;
    player.inv.craft_size = if table { 3 } else { 2 };
    // The furnace screen's arrow and flame: where, how big.
    let mut bars: Vec<(Vec2, Vec2, bool)> = Vec::new();
    // Where the arrows between a grid and its result go.
    let mut arrows = Vec::new();
    let mut slots = Vec::new();
    let step = SLOT + GAP;
    let mut y = PAD + 24.0;
    if creative {
        for r in 0..PALETTE_ROWS {
            for c in 0..HOTBAR {
                slots.push((
                    SlotRef::Palette(r * HOTBAR + c),
                    Vec2::new(PAD + c as f32 * step, y + r as f32 * step),
                ));
            }
        }
        y += PALETTE_ROWS as f32 * step + 10.0;
    }
    if table {
        // A table's grid on top, in the middle: 3 × 3, an arrow and the result.
        let x0 = PAD + 1.5 * step;
        for r in 0..3 {
            for c in 0..3 {
                slots.push((
                    SlotRef::Inv(orbit_core::inventory::CRAFT + r * 3 + c),
                    Vec2::new(x0 + c as f32 * step, y + r as f32 * step),
                ));
            }
        }
        arrows.push(Vec2::new(x0 + 3.0 * step + 6.0, y + step + 6.0));
        slots.push((
            SlotRef::Inv(orbit_core::inventory::RESULT),
            Vec2::new(x0 + 3.0 * step + 44.0, y + step),
        ));
        y += 3.0 * step + 10.0;
    }
    if furnace {
        use orbit_core::furnace::Slot;
        let x0 = PAD + 2.0 * step;
        slots.push((SlotRef::Furnace(Slot::Input), Vec2::new(x0, y)));
        slots.push((SlotRef::Furnace(Slot::Fuel), Vec2::new(x0, y + 2.0 * step)));
        slots.push((
            SlotRef::Furnace(Slot::Output),
            Vec2::new(x0 + 4.2 * step, y + step),
        ));
        bars.push((
            Vec2::new(x0 + 6.0, y + step + 2.0),
            Vec2::new(SLOT - 12.0, step - 4.0),
            true,
        ));
        bars.push((
            Vec2::new(x0 + 1.4 * step, y + step + 10.0),
            Vec2::new(2.0 * step, 16.0),
            false,
        ));
        y += 3.0 * step + 10.0;
    }
    let bag_y = y;
    for r in 0..3 {
        for c in 0..HOTBAR {
            slots.push((
                SlotRef::Inv(HOTBAR + r * HOTBAR + c),
                Vec2::new(PAD + c as f32 * step, y + r as f32 * step),
            ));
        }
    }
    y += 3.0 * step + 8.0;
    for c in 0..HOTBAR {
        slots.push((SlotRef::Inv(c), Vec2::new(PAD + c as f32 * step, y)));
    }
    // The left hand, a little apart at the end of the hotbar row.
    let off_x = PAD + HOTBAR as f32 * step + 10.0;
    slots.push((
        SlotRef::Inv(orbit_core::inventory::OFFHAND),
        Vec2::new(off_x, y),
    ));
    y += step + PAD - GAP;
    let mut width = off_x + SLOT + PAD;
    if !table && !furnace {
        // The inventory's grid, 2 × 2, to the right of the bag.
        let x0 = off_x + SLOT + 30.0;
        for r in 0..2 {
            for c in 0..2 {
                slots.push((
                    SlotRef::Inv(orbit_core::inventory::CRAFT + r * 3 + c),
                    Vec2::new(x0 + c as f32 * step, bag_y + r as f32 * step),
                ));
            }
        }
        arrows.push(Vec2::new(x0 + 2.0 * step + 6.0, bag_y + 0.5 * step + 6.0));
        slots.push((
            SlotRef::Inv(orbit_core::inventory::RESULT),
            Vec2::new(x0 + 2.0 * step + 44.0, bag_y + 0.5 * step),
        ));
        width = x0 + 2.0 * step + 44.0 + SLOT + PAD;
    }
    let size = Vec2::new(width, y);
    let title = if furnace {
        "Furnace"
    } else if table {
        "Crafting"
    } else if creative {
        "Creative  (wheel scrolls)"
    } else {
        "Inventory"
    };
    let root = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.45)),
            GlobalZIndex(20),
        ))
        .with_children(|root| {
            root.spawn((
                ScreenPanel,
                Node {
                    position_type: PositionType::Absolute,
                    width: px(size.x),
                    height: px(size.y),
                    border_radius: BorderRadius::all(px(6)),
                    ..default()
                },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn((
                    Text::new(title),
                    TextFont {
                        font_size: FontSize::Px(15.0),
                        ..default()
                    },
                    TextColor(MUTED),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(PAD),
                        top: px(PAD - 2.0),
                        ..default()
                    },
                ));
                for &(at, size, flame) in &bars {
                    p.spawn((
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(at.x),
                            top: px(at.y),
                            width: px(size.x),
                            height: px(size.y),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.5)),
                    ))
                    .with_children(|b| {
                        let color = if flame {
                            Color::srgb(1.0, 0.55, 0.1)
                        } else {
                            Color::srgb(0.9, 0.9, 0.9)
                        };
                        let node = if flame {
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(0),
                                bottom: px(0),
                                width: percent(100),
                                height: percent(0),
                                ..default()
                            }
                        } else {
                            Node {
                                width: percent(0),
                                height: percent(100),
                                ..default()
                            }
                        };
                        b.spawn((FurnaceFill { flame }, node, BackgroundColor(color)));
                    });
                }
                for &a in &arrows {
                    p.spawn((
                        Text::new("->"),
                        TextFont {
                            font_size: FontSize::Px(20.0),
                            ..default()
                        },
                        TextColor(MUTED),
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(a.x),
                            top: px(a.y),
                            ..default()
                        },
                    ));
                }
                for &(r, at) in &slots {
                    p.spawn((
                        ScreenSlot(r),
                        Node {
                            position_type: PositionType::Absolute,
                            left: px(at.x),
                            top: px(at.y),
                            width: px(SLOT),
                            height: px(SLOT),
                            border_radius: BorderRadius::all(px(3)),
                            ..default()
                        },
                        BackgroundColor(SLOT_BG),
                    ))
                    .with_children(|s| {
                        let (img, vis) = icon(&icons, None);
                        s.spawn((
                            ScreenIcon(r),
                            img,
                            vis,
                            Node {
                                position_type: PositionType::Absolute,
                                left: px(2),
                                top: px(2),
                                width: px(ICON as f32),
                                height: px(ICON as f32),
                                ..default()
                            },
                        ));
                        s.spawn((
                            ScreenCount(r),
                            count_text(0),
                            Node {
                                position_type: PositionType::Absolute,
                                right: px(2),
                                bottom: px(0),
                                ..default()
                            },
                        ));
                        spawn_dura(s, DuraOf::Screen(r), 3.0);
                    });
                }
            });
            // What the mouse holds, and the name of the item under it.
            let (img, vis) = icon(&icons, None);
            root.spawn((
                CursorIcon,
                img,
                vis,
                Node {
                    position_type: PositionType::Absolute,
                    width: px(ICON as f32),
                    height: px(ICON as f32),
                    ..default()
                },
                GlobalZIndex(22),
            ))
            .with_children(|c| {
                c.spawn((
                    CursorCount,
                    count_text(0),
                    Node {
                        position_type: PositionType::Absolute,
                        right: px(-2),
                        bottom: px(-2),
                        ..default()
                    },
                ));
            });
            root.spawn((
                Tooltip,
                Text::new(""),
                TextFont {
                    font_size: FontSize::Px(14.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                Node {
                    position_type: PositionType::Absolute,
                    padding: UiRect::axes(px(5), px(2)),
                    border_radius: BorderRadius::all(px(3)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.1, 0.02, 0.16, 0.92)),
                Visibility::Hidden,
                GlobalZIndex(23),
            ));
        })
        .id();
    screen.open = Some(Open {
        root,
        slots,
        size,
        regrab: cursor.grab_mode != CursorGrabMode::None,
    });
    screen.kind = kind;
    screen.shift = false;
    screen.scroll = 0;
    keyboard.owner = Some(Owner::Inventory);
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

/// The screen while open: keys and clicks, then what every slot shows.
#[allow(clippy::too_many_arguments)]
fn run_screen(
    mut commands: Commands,
    mut screen: ResMut<Screen>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    window: Single<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
    mut player: Single<&mut Player>,
    mut drops: ResMut<Drops>,
    space: Res<Space>,
    mut furnaces: ResMut<crate::furnace::Furnaces>,
    mut ui: ParamSet<(
        Query<&mut Node, With<ScreenPanel>>,
        Query<(&ScreenSlot, &mut BackgroundColor)>,
        Query<(&ScreenIcon, &mut ImageNode, &mut Visibility)>,
        Query<(&ScreenCount, &mut Text)>,
        Query<(&mut Node, &mut ImageNode, &mut Visibility), With<CursorIcon>>,
        Query<&mut Text, With<CursorCount>>,
        Query<(&mut Node, &mut Text, &mut Visibility), With<Tooltip>>,
    )>,
) {
    let Some(open) = screen.open.as_ref() else {
        return;
    };
    if keyboard.owner != Some(Owner::Inventory) {
        return;
    }
    let win = Vec2::new(window.width(), window.height());
    let origin = ((win - open.size) * 0.5).floor();
    let mouse_at = window.cursor_position();
    let hovered = mouse_at.and_then(|m| {
        open.slots
            .iter()
            .find(|(_, at)| {
                let r = Rect::from_corners(origin + *at, origin + *at + Vec2::splat(SLOT));
                r.contains(m)
            })
            .map(|s| s.0)
    });
    let in_panel =
        mouse_at.is_some_and(|m| Rect::from_corners(origin, origin + open.size).contains(m));
    let creative = player.inv.mode == GameMode::Creative;
    let max_scroll = palette_rows().saturating_sub(PALETTE_ROWS);
    let scroll_now = screen.scroll.min(max_scroll);
    let palette_item = |k: usize| {
        orbit_core::items::creative()
            .get(scroll_now * HOTBAR + k)
            .copied()
    };
    let throw_frame = body_frame(&player.body, &space);

    let mut close = false;
    for ev in std::mem::take(&mut keyboard.events) {
        if matches!(ev.key_code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
            screen.shift = ev.state.is_pressed();
        }
        if !ev.state.is_pressed() || ev.repeat {
            continue;
        }
        match ev.key_code {
            KeyCode::Escape | KeyCode::KeyE => close = true,
            KeyCode::KeyQ => {
                if let Some(SlotRef::Inv(i)) = hovered
                    .filter(|h| matches!(h, SlotRef::Inv(i) if *i < orbit_core::inventory::SLOTS))
                {
                    let n = if keyboard.ctrl { 64 } else { 1 };
                    if let (Some(s), Some(f)) = (player.inv.take(i, n), throw_frame.clone()) {
                        let b = &player.body;
                        let up = if matches!(f, Frame::Station(_)) {
                            Vec3::Y
                        } else {
                            b.up
                        };
                        drops.push(f, ItemDrop::thrown(s, b.eye(), b.look_dir(), up));
                    }
                }
            }
            k => {
                let digits = [
                    KeyCode::Digit1,
                    KeyCode::Digit2,
                    KeyCode::Digit3,
                    KeyCode::Digit4,
                    KeyCode::Digit5,
                    KeyCode::Digit6,
                    KeyCode::Digit7,
                    KeyCode::Digit8,
                    KeyCode::Digit9,
                ];
                if let Some(n) = digits.iter().position(|&d| d == k) {
                    match hovered {
                        Some(SlotRef::Inv(i)) => player.inv.swap_hotbar(i, n),
                        Some(SlotRef::Palette(p)) if creative => {
                            if let Some(item) = palette_item(p) {
                                player.inv.slots[n] =
                                    Some(Stack::new(item, block::max_stack(item)));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    if creative && scroll.delta.y != 0.0 {
        let s = scroll_now as i32 - scroll.delta.y.signum() as i32;
        screen.scroll = s.clamp(0, max_scroll as i32) as usize;
    }
    let (left, right) = (
        mouse.just_pressed(MouseButton::Left),
        mouse.just_pressed(MouseButton::Right),
    );
    if left || right {
        let inv = &mut player.inv;
        match hovered {
            Some(SlotRef::Furnace(s)) => {
                if let Some(f) = screen
                    .furnace
                    .as_ref()
                    .and_then(|k| furnaces.map.get_mut(k))
                {
                    if left && screen.shift {
                        // The whole stack to the bag; what does not fit stays.
                        let mut cur = None;
                        f.click(s, &mut cur);
                        if let Some(rest) = cur.and_then(|c| inv.add_stack(c)) {
                            match s {
                                orbit_core::furnace::Slot::Output => f.output = Some(rest),
                                _ => f.click(s, &mut Some(rest)),
                            }
                        }
                    } else {
                        f.click(s, &mut inv.cursor);
                    }
                }
            }
            Some(SlotRef::Inv(i))
                if left
                    && screen.shift
                    && screen.kind == Kind::Furnace
                    && i < orbit_core::inventory::SLOTS
                    && inv.at(i).is_some_and(|st| {
                        orbit_core::furnace::Furnace::accepts(
                            orbit_core::furnace::Slot::Input,
                            st.item,
                        ) || orbit_core::furnace::Furnace::accepts(
                            orbit_core::furnace::Slot::Fuel,
                            st.item,
                        )
                    }) =>
            {
                // Into the furnace's input or fuel, as Minecraft does.
                let st = inv.at(i).unwrap();
                let slot = if orbit_core::furnace::Furnace::accepts(
                    orbit_core::furnace::Slot::Input,
                    st.item,
                ) {
                    orbit_core::furnace::Slot::Input
                } else {
                    orbit_core::furnace::Slot::Fuel
                };
                if let Some(f) = screen
                    .furnace
                    .as_ref()
                    .and_then(|k| furnaces.map.get_mut(k))
                {
                    let mut tmp = inv.take(i, st.count);
                    f.click(slot, &mut tmp);
                    if let Some(rest) = tmp.and_then(|r| inv.add_stack(r)) {
                        inv.put(i, Some(rest));
                    }
                }
            }
            Some(SlotRef::Inv(i)) if left && screen.shift => inv.quick_move(i),
            Some(SlotRef::Inv(i)) if left => inv.click(i),
            Some(SlotRef::Inv(i)) => inv.right_click(i),
            Some(SlotRef::Palette(p)) => {
                // Holding something: the palette takes it away. Else a stack (or one) of the item.
                if inv.cursor.is_some() {
                    inv.cursor = None;
                } else if let Some(item) = palette_item(p) {
                    let n = if right { 1 } else { block::max_stack(item) };
                    if screen.shift {
                        inv.add(item, n);
                    } else {
                        inv.cursor = Some(Stack::new(item, n));
                    }
                }
            }
            None if !in_panel => {
                // Out of the panel: what the mouse holds is thrown (one with a right click).
                if let Some(c) = inv.cursor {
                    let n = if right { 1 } else { c.count };
                    let rest = c.count - n;
                    inv.cursor = (rest > 0).then(|| Stack::new(c.item, rest));
                    if let Some(f) = throw_frame.clone() {
                        let b = &player.body;
                        let up = if matches!(f, Frame::Station(_)) {
                            Vec3::Y
                        } else {
                            b.up
                        };
                        drops.push(
                            f,
                            ItemDrop::thrown(Stack::new(c.item, n), b.eye(), b.look_dir(), up),
                        );
                    }
                }
            }
            None => {}
        }
    }
    if close {
        let open = screen.open.take().unwrap();
        commands.entity(open.root).despawn();
        // The mouse's stack and what is on the crafting grid go back into the bag; what does not fit is thrown.
        let mut rest: Vec<Stack> = player.inv.put_back_cursor().into_iter().collect();
        rest.extend(player.inv.return_craft());
        if let Some(f) = throw_frame {
            for r in rest {
                let b = &player.body;
                let up = if matches!(f, Frame::Station(_)) {
                    Vec3::Y
                } else {
                    b.up
                };
                drops.push(f.clone(), ItemDrop::thrown(r, b.eye(), b.look_dir(), up));
            }
        }
        keyboard.owner = None;
        if open.regrab {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
        return;
    }

    // What everything shows.
    let inv = &player.inv;
    let furnace = screen
        .furnace
        .as_ref()
        .and_then(|k| furnaces.map.get(k))
        .cloned();
    let content = |r: SlotRef| -> Option<Stack> {
        match r {
            SlotRef::Inv(i) => inv.at(i),
            SlotRef::Palette(p) => palette_item(p).map(|b| Stack::new(b, 1)),
            SlotRef::Furnace(s) => furnace.as_ref().and_then(|f| f.slot(s)),
        }
    };
    if let Ok(mut n) = ui.p0().single_mut() {
        n.left = px(origin.x);
        n.top = px(origin.y);
    }
    for (s, mut bg) in &mut ui.p1() {
        let c = if hovered == Some(s.0) {
            SLOT_HOVER
        } else {
            SLOT_BG
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    for (s, mut img, mut vis) in &mut ui.p2() {
        let item = content(s.0).map(|x| x.item);
        let rect = item.map(Icons::rect);
        if img.rect != rect {
            img.rect = rect;
        }
        let v = if item.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != v {
            *vis = v;
        }
    }
    for (s, mut t) in &mut ui.p3() {
        let n = content(s.0).map_or(0, |x| x.count);
        let want = if n > 1 { n.to_string() } else { String::new() };
        if t.0 != want {
            t.0 = want;
        }
    }
    let held = inv.cursor;
    let m = mouse_at.unwrap_or(Vec2::splat(-100.0));
    if let Ok((mut n, mut img, mut vis)) = ui.p4().single_mut() {
        n.left = px(m.x - ICON as f32 / 2.0);
        n.top = px(m.y - ICON as f32 / 2.0);
        img.rect = held.map(|s| Icons::rect(s.item));
        *vis = if held.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
    if let Ok(mut t) = ui.p5().single_mut() {
        let n = held.map_or(0, |s| s.count);
        t.0 = if n > 1 { n.to_string() } else { String::new() };
    }
    let tip = if held.is_none() {
        hovered.and_then(content).map(|s| block::name(s.item))
    } else {
        None
    };
    if let Ok((mut n, mut t, mut vis)) = ui.p6().single_mut() {
        n.left = px(m.x + 14.0);
        n.top = px(m.y - 26.0);
        if let Some(name) = tip {
            if t.0 != name {
                t.0 = name.into();
            }
            *vis = Visibility::Inherited;
        } else {
            *vis = Visibility::Hidden;
        }
    }
}

// --- Grass ----------------------------------------------------------------------------------

/// Time not yet run through random ticks, and the dice.
#[derive(Resource)]
struct Ticks {
    time: f32,
    rng: Rng,
}

impl Default for Ticks {
    fn default() -> Self {
        Self {
            time: 0.0,
            rng: Rng::new(0x6A55),
        }
    }
}

/// Random ticks per tick (0.05 s) in the region round the player: 64 × 64 cells across, 24 high.
const RANDOM_TICKS: usize = 100;
const REGION: i64 = 32;
const REGION_UP: i64 = 12;

/// Random ticks round the player: grass creeping over dirt, dying back under blocks.
fn grass(
    mut ticks: ResMut<Ticks>,
    time: Res<Time>,
    player: Single<&Player>,
    mut space: ResMut<Space>,
    mut stations: ResMut<Stations>,
) {
    let tick = orbit_core::liquid::TICK;
    ticks.time = (ticks.time + time.delta_secs()).min(tick * 4.0);
    let body = &player.body;
    while ticks.time >= tick {
        ticks.time -= tick;
        if let Some(i) = body.station.as_ref().and_then(|id| stations.find(id)) {
            let s = stations.list[i].station.clone();
            let Ok(mut s) = s.try_write() else { continue };
            let (lo, hi) = (s.bounds.min, s.bounds.max);
            let mut changed = false;
            for _ in 0..RANDOM_TICKS / 4 {
                let r = &mut ticks.rng;
                let p = IVec3::new(
                    r.int(lo.x as i64, hi.x as i64) as i32,
                    r.int(lo.y as i64, hi.y as i64) as i32,
                    r.int(lo.z as i64, hi.z as i64) as i32,
                );
                // The station's light is worked out for its mesh only: here, open sky over it or not.
                let open = !(p.y + 1..=hi.y).any(|y| block::opaque(s.get(IVec3::new(p.x, y, p.z))));
                let l = if open { 15 } else { 0 };
                changed |= s.random_tick(p, |_| l);
            }
            drop(s);
            if changed {
                stations.mark_dirty(i);
            }
            continue;
        }
        let Some(id) = body.gravity_source else {
            return;
        };
        let Some(slot) = space.planets.get_mut(&id) else {
            return;
        };
        let Some(v) = slot.voxel.clone() else { return };
        let Ok(mut planet) = v.try_write() else {
            return;
        };
        let g = &planet.grid;
        let p = body.pos.as_dvec3();
        let face = g.face_of(p);
        let (a, b) = g.face_coords(face, p);
        let k = g.layer_of(body.pos.length());
        let mut dirty = Vec::new();
        for _ in 0..RANDOM_TICKS {
            let r = &mut ticks.rng;
            let (i, j, kk) = (
                a as i64 + r.int(-REGION, REGION),
                b as i64 + r.int(-REGION, REGION),
                k + r.int(-REGION_UP, REGION_UP),
            );
            let Some(c) = planet.grid.cell_on_face(face, i, j, kk) else {
                continue;
            };
            for key in planet.random_tick(c) {
                if !dirty.contains(&key) {
                    dirty.push(key);
                }
            }
        }
        drop(planet);
        if !dirty.is_empty() {
            slot.edited = true;
            remesh(slot, &dirty);
        }
    }
}
