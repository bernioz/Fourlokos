//! The schematics menu (`/schematic`): every saved schematic as a card with its station turning
//! slowly in 3D, its size, and a button that puts it up in front of you.
//!
//! Each preview is the station's real mesh, lit like a station in full sun, on a render layer of
//! its own and drawn by its own camera into an image that the card shows.

use crate::chat::{Chat, Keyboard, Owner};
use crate::commands::{place_schematic, schematic_files};
use crate::lighting::VoxelMaterial;
use crate::player::Player;
use crate::station::Stations;
use crate::textures::Atlas;
use crate::world::{gpu_mesh, Clock, Space};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::picking::hover::Hovered;
use bevy::ui::{Pressed, ScrollPosition};
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::mesh::station_mesh;
use orbit_core::station::Station;

/// Previews' render layers start here (the world is 0, the galaxy map 1).
const FIRST_LAYER: usize = 8;
/// Most cards shown at once.
const MAX_CARDS: usize = 24;
/// Preview size, pixels.
const W: u32 = 240;
const H: u32 = 170;
/// Radians a second the previews turn.
const TURN: f32 = 0.45;

const PANEL: Color = Color::srgba(0.06, 0.07, 0.1, 0.94);
const CARD: Color = Color::srgb(0.11, 0.12, 0.16);
const BUTTON: Color = Color::srgb(0.2, 0.22, 0.28);
const HOVER: Color = Color::srgb(0.3, 0.33, 0.42);
const PRESS: Color = Color::srgb(0.38, 0.5, 0.7);
const MUTED: Color = Color::srgb(0.63, 0.63, 0.63);

pub struct SchematicMenuPlugin;

impl Plugin for SchematicMenuPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SchematicMenu>()
            .add_observer(on_activate)
            .add_systems(Update, (open_menu, menu_keys, run_menu, spin_previews, hide_hud, style_buttons).chain().after(crate::commands::run));
    }
}

#[derive(Resource, Default)]
pub struct SchematicMenu {
    /// `/schematic` asked for it.
    pub wanted: bool,
    open: Option<Open>,
    queue: Vec<Action>,
}

struct Open {
    root: Entity,
    /// Preview cameras and models (not under the UI root).
    scene: Vec<Entity>,
    stems: Vec<String>,
    regrab: bool,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Place(usize),
    Close,
}

/// A preview's turning model.
#[derive(Component)]
struct Spinner;

fn on_activate(ev: On<Activate>, actions: Query<&Action>, mut menu: ResMut<SchematicMenu>) {
    if let Ok(&a) = actions.get(ev.entity) {
        menu.queue.push(a);
    }
}

fn label(s: &str, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (Text::new(s), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(text: &str, action: Action, width: f32) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node { width: px(width), height: px(26), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() },
        BackgroundColor(BUTTON),
        children![label(text, 14.0, Color::WHITE)],
    )
}

/// A preview of `station` on layer `layer`: its model turning about its middle, and a camera
/// framing all of it, drawing into a new image. Returns the image and the entities made.
fn spawn_preview(commands: &mut Commands, images: &mut Assets<Image>, meshes: &mut Assets<Mesh>, mats: &mut Assets<VoxelMaterial>, atlas: &Atlas, station: &Station, layer: usize) -> (Handle<Image>, Vec<Entity>) {
    let image = images.add(Image::new_target_texture(W, H, TextureFormat::Bgra8UnormSrgb, None));
    let layers = RenderLayers::layer(layer);
    let b = station.bounds;
    let (lo, hi) = (b.min.as_vec3(), (b.max + IVec3::ONE).as_vec3());
    let middle = (lo + hi) * 0.5;
    let size = (hi - lo).length().max(4.0);
    // Far from everything, so nothing of the world is near (it is on other layers anyway).
    let at = Vec3::new(0.0, -50_000.0 - layer as f32 * 1000.0, 0.0);
    let mut m = VoxelMaterial::new([1.0, 0.95, 0.85], atlas.0.clone());
    m.place_flat(at, Vec3::new(0.45, 0.8, 0.4).normalize(), 1.0);
    let mat = mats.add(m);
    let pivot = commands.spawn((Spinner, Transform::from_translation(at), Visibility::default(), layers.clone())).id();
    let (mesh, aabb) = gpu_mesh(meshes, station_mesh(station));
    commands.spawn((Mesh3d(mesh), aabb, MeshMaterial3d(mat), Transform::from_translation(-middle), layers.clone(), ChildOf(pivot)));
    // Looking down a little from one side, far enough that the whole box fits.
    let fov = 40f32.to_radians();
    let dist = size * 0.5 / (fov * 0.5).tan() * 1.05;
    let eye = at + Vec3::new(0.0, 0.55, 1.0).normalize() * dist;
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera { order: -1 - layer as isize, clear_color: ClearColorConfig::Custom(Color::srgb(0.03, 0.035, 0.05)), ..default() },
            RenderTarget::Image(ImageRenderTarget { handle: image.clone(), scale_factor: 1.0 }),
            Projection::Perspective(PerspectiveProjection { fov, near: 0.1, far: dist * 4.0, ..default() }),
            Transform::from_translation(eye).looking_at(at, Vec3::Y),
            layers,
        ))
        .id();
    (image, vec![pivot, camera])
}

#[allow(clippy::too_many_arguments)]
fn open_menu(
    mut commands: Commands,
    mut menu: ResMut<SchematicMenu>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<VoxelMaterial>>,
    atlas: Res<Atlas>,
    mut chat: ResMut<Chat>,
) {
    if !std::mem::take(&mut menu.wanted) || menu.open.is_some() {
        return;
    }
    let files = schematic_files();
    if files.is_empty() {
        chat.info("No schematics yet: /schematic save on a station");
        return;
    }
    let mut scene = Vec::new();
    let mut cards = Vec::new();
    for (k, (stem, sc)) in files.iter().take(MAX_CARDS).enumerate() {
        let station = Station::from_schematic(String::new(), None, sc);
        let (image, made) = spawn_preview(&mut commands, &mut images, &mut meshes, &mut mats, &atlas, &station, FIRST_LAYER + k);
        scene.extend(made);
        let span = sc.size;
        cards.push((k, stem.clone(), station.name.clone(), format!("{} blocks  //  {} x {} x {}", sc.solid_cells().count(), span.x, span.z, span.y), image));
    }
    let root = commands
        .spawn((
            Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, position_type: PositionType::Absolute, ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.6)),
            GlobalZIndex(30),
        ))
        .with_children(|root| {
            root.spawn((
                Node { flex_direction: FlexDirection::Column, row_gap: px(10), padding: UiRect::all(px(16)), border_radius: BorderRadius::all(px(8)), max_width: percent(92), max_height: percent(90), ..default() },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn(Node { flex_direction: FlexDirection::Row, justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() }).with_children(|r| {
                    r.spawn(label("Schematics", 20.0, Color::WHITE));
                    r.spawn(button("Close", Action::Close, 80.0));
                });
                p.spawn(label("Place puts a new station up in front of you, in orbit. Esc closes.", 13.0, MUTED));
                p.spawn((
                    Node { flex_direction: FlexDirection::Row, flex_wrap: FlexWrap::Wrap, column_gap: px(10), row_gap: px(10), overflow: Overflow::scroll_y(), ..default() },
                    ScrollPosition::default(),
                ))
                .with_children(|grid| {
                    for (k, stem, name, info, image) in cards {
                        grid.spawn((
                            Node { flex_direction: FlexDirection::Column, row_gap: px(4), padding: UiRect::all(px(8)), border_radius: BorderRadius::all(px(6)), ..default() },
                            BackgroundColor(CARD),
                        ))
                        .with_children(|c| {
                            c.spawn((ImageNode::new(image), Node { width: px(W as f32), height: px(H as f32), border_radius: BorderRadius::all(px(4)), ..default() }));
                            c.spawn(label(&name, 16.0, Color::WHITE));
                            c.spawn(label(&format!("{stem}  //  {info}"), 12.0, MUTED));
                            c.spawn(button("Place", Action::Place(k), W as f32));
                        });
                    }
                });
            });
        })
        .id();
    menu.open = Some(Open { root, scene, stems: files.into_iter().take(MAX_CARDS).map(|(s, _)| s).collect(), regrab: cursor.grab_mode != CursorGrabMode::None });
    keyboard.owner = Some(Owner::Schematics);
    cursor.grab_mode = CursorGrabMode::None;
    cursor.visible = true;
}

/// Esc closes; the wheel scrolls the cards.
fn menu_keys(mut menu: ResMut<SchematicMenu>, mut keyboard: ResMut<Keyboard>) {
    if menu.open.is_none() || keyboard.owner != Some(Owner::Schematics) {
        return;
    }
    for ev in std::mem::take(&mut keyboard.events) {
        if ev.state.is_pressed() && ev.key_code == KeyCode::Escape {
            menu.queue.push(Action::Close);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn run_menu(
    mut commands: Commands,
    mut menu: ResMut<SchematicMenu>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut stations: ResMut<Stations>,
    player: Single<&Player>,
    space: Res<Space>,
    clock: Res<Clock>,
    mut chat: ResMut<Chat>,
) {
    let Some(action) = menu.queue.pop() else { return };
    menu.queue.clear();
    let Some(open) = menu.open.take() else { return };
    if let Action::Place(k) = action {
        match place_schematic(&mut stations, &player.body, &space, clock.0, &open.stems[k], None) {
            Ok(line) => chat.info(&line),
            Err(e) => chat.error(&e),
        }
    }
    commands.entity(open.root).despawn();
    for e in open.scene {
        commands.entity(e).despawn();
    }
    keyboard.owner = None;
    if open.regrab {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

fn spin_previews(menu: Res<SchematicMenu>, time: Res<Time>, mut spinners: Query<&mut Transform, With<Spinner>>) {
    if menu.open.is_none() {
        return;
    }
    for mut t in &mut spinners {
        t.rotate_y(TURN * time.delta_secs());
    }
}

/// The HUD's text and the chat's lines would show through the panel: hidden while it is open.
fn hide_hud(menu: Res<SchematicMenu>, mut hud: Query<&mut Visibility, Or<(With<crate::player::HudText>, With<crate::chat::LinesBox>)>>) {
    let want = if menu.open.is_some() { Visibility::Hidden } else { Visibility::Inherited };
    for mut v in &mut hud {
        if *v != want {
            *v = want;
        }
    }
}

fn style_buttons(mut buttons: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), (With<Button>, With<Action>)>) {
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
