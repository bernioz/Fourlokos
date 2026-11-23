//! Galaxy map (M): a 3D star map with its own camera and scene (render layer `LAYER`).
//!
//! - **Galaxy**: the star systems around you as glowing stars over a nebula, each with a ring and
//!   its name; faint lanes join neighbors. Hover a star for its card; click it to pick it, click
//!   again (or "Enter system") to dive in.
//! - **System**: the star, the planets' orbits, and the planets themselves (their real blocky far
//!   meshes, lit by their star, turning and going round in real time), with the stations going
//!   round them, drawn from their own blocks. Your position is marked. Pick a planet to follow it
//!   and Warp or Fly there; Backspace (or "Galaxy") goes back out.
//!
//! The camera orbits what it looks at and moves smoothly: drag to turn it, the wheel zooms, WASD
//! pans, Space comes back to you. It opens on your own system when you are in one. M or Esc closes.

use crate::lighting::VoxelMaterial;
use crate::player::Player;
use crate::station::Stations;
use crate::textures::Atlas;
use crate::travel::{fly_time, travel_blocker, warp_time, Mode, Request, Target, Travel};
use crate::world::{gpu_mesh, Clock, Space};
use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::RenderLayers;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::math::DVec3;
use bevy::picking::hover::Hovered;
use bevy::picking::Pickable;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::mesh::{far_mesh, MeshData};
use orbit_core::planet::PlanetSpec;
use orbit_core::rng::mix;
use orbit_core::universe::{StarSystem, Universe};
use std::f32::consts::{FRAC_PI_2, PI, TAU};

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GalaxyMap>()
            .init_gizmo_group::<MapGizmos>()
            .add_observer(on_activate)
            .add_systems(Startup, setup)
            .add_systems(Update, reseed.run_if(resource_changed::<crate::sky::SkySeed>))
            .add_systems(
                Update,
                (toggle, run_actions, swap_cameras, controls, transition, finish_models, place_scene, place_markers, draw_lines, card, refresh_texts, style_buttons, finish_nebula)
                    .chain()
                    .after(crate::player::move_player),
            );
    }
}

/// The map's own render layer: nothing of the world shows in it, nor it in the world.
const LAYER: usize = 1;
/// Sectors shown around you, sideways and up/down.
const RANGE: i32 = 3;
const RANGE_Y: i32 = 1;
/// Galaxy view: map units per block.
const GALAXY_SCALE: f64 = 1.0 / 4500.0;
/// System view: the outermost orbit is this far from the star, map units.
const SYSTEM_SIZE: f32 = 120.0;
/// Station models are drawn this many times their true size (true distances, a little bigger).
const STATION_SCALE: f32 = 2.0;
/// Seconds to dive into a system (or pull out of one), and to come out of the fade.
const DIVE: f32 = 0.5;
const SURFACE: f32 = 0.45;

const ACCENT: Color = Color::srgb(1.0, 0.72, 0.28);
const TEXT: Color = Color::srgb(0.92, 0.95, 1.0);
const MUTED: Color = Color::srgb(0.62, 0.68, 0.78);
const CARD: Color = Color::srgba(0.02, 0.04, 0.06, 0.82);
const CARD_EDGE: Color = Color::srgba(0.75, 0.85, 1.0, 0.25);
const BUTTON: Color = Color::srgba(0.12, 0.16, 0.22, 0.9);
const HOVER: Color = Color::srgba(0.22, 0.3, 0.42, 0.95);
const PRESS: Color = Color::srgb(0.35, 0.55, 0.85);
const GO: Color = Color::srgba(0.16, 0.38, 0.26, 0.95);
const STATION: Color = Color::srgb(0.45, 0.85, 1.0);

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct MapGizmos;

#[derive(Resource, Default)]
pub struct GalaxyMap {
    open: Option<View>,
    queue: Vec<MapAction>,
}

impl GalaxyMap {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Presses the map's Launch button.
    pub fn press_launch(&mut self) {
        self.queue.push(MapAction::Launch);
    }

    /// Picks and follows a station (by its index in `Stations`), as clicking it does.
    pub fn follow_station(&mut self, k: usize) {
        self.queue.push(MapAction::Station(k));
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Level {
    Galaxy,
    System(usize),
}

/// What the camera keeps in the middle of the view as it moves.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Follow {
    Free,
    You,
    Star,
    Planet(u64),
    Station(usize),
}

/// A camera orbiting a point: where it looks, which way round, how far.
#[derive(Clone, Copy, Debug)]
struct Orbit {
    focus: Vec3,
    yaw: f32,
    pitch: f32,
    dist: f32,
}

impl Orbit {
    fn eye(&self) -> Vec3 {
        self.focus + Vec3::new(self.yaw.sin() * self.pitch.cos(), self.pitch.sin(), self.yaw.cos() * self.pitch.cos()) * self.dist
    }

    fn approach(&mut self, to: &Orbit, k: f32) {
        self.focus = self.focus.lerp(to.focus, k);
        self.yaw += (to.yaw - self.yaw) * k;
        self.pitch += (to.pitch - self.pitch) * k;
        self.dist = (self.dist.ln() + (to.dist.ln() - self.dist.ln()) * k).exp();
    }
}

struct Transition {
    to: Level,
    t: f32,
    swapped: bool,
}

struct View {
    root: Entity,
    markers: Entity,
    card: Entity,
    card_body: Entity,
    fade: Entity,
    /// The 3D things of the level shown, and its markers.
    scene: Vec<Entity>,
    level: Level,
    systems: Vec<StarSystem>,
    /// Where you were when the map opened (world): the galaxy view is laid out around it.
    here: DVec3,
    home: Option<usize>,
    /// Picked on the map.
    system: Option<usize>,
    planet: Option<u64>,
    station: Option<usize>,
    cam: Orbit,
    target: Orbit,
    follow: Follow,
    transition: Option<Transition>,
    /// The card shows this; rebuilt when it changes.
    card_for: Option<MapAction>,
    card_shown: f32,
    regrab: bool,
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
enum MapAction {
    System(usize),
    Enter(usize),
    Planet(u64),
    Station(usize),
    Go(Mode),
    Launch,
    Land,
    Cancel,
    Back,
    Center,
    Close,
}

/// The map's camera.
#[derive(Component)]
pub struct MapCamera;

/// The nebula behind everything (follows the camera).
#[derive(Component)]
struct Backdrop;

/// The far stars, points in front of the nebula (follow the camera too).
#[derive(Component)]
struct Dust;

/// Far stars as tiny quads on a big sphere facing its middle, where the camera stays: a few
/// pixels each, some bright enough to bloom, more along the galaxy's band.
fn dust(seed: u64) -> Mesh {
    const N: usize = 6000;
    const R: f32 = 9000.0;
    let band_axis = Vec3::new(0.2, 1.0, 0.1).normalize();
    let (mut pos, mut col, mut idx) = (Vec::new(), Vec::new(), Vec::new());
    let mut k = 0u64;
    let mut rnd = || {
        k += 1;
        (mix(seed ^ 0xD057 ^ k.wrapping_mul(0x9E37_79B9)) >> 40) as f32 / (1u64 << 24) as f32
    };
    while pos.len() < N * 4 {
        let (z, a) = (rnd() * 2.0 - 1.0, rnd() * TAU);
        let d = Vec3::new((1.0 - z * z).sqrt() * a.cos(), z, (1.0 - z * z).sqrt() * a.sin());
        let band = (-(d.dot(band_axis) * 3.2).powi(2)).exp();
        if rnd() > 0.35 + 0.65 * band {
            continue;
        }
        let b = rnd().powf(5.0);
        let size = R * (0.00045 + 0.0009 * b);
        let warm = rnd() < 0.3;
        let tint = if warm { Vec3::new(1.0, 0.78, 0.55) } else { Vec3::new(0.8, 0.88, 1.0) };
        let c = tint * (0.3 + 3.0 * b);
        let (u, w) = (d.any_orthonormal_vector() * size, d.cross(d.any_orthonormal_vector()) * size);
        let base = pos.len() as u32;
        for q in [-u - w, u - w, u + w, -u + w] {
            pos.push((d * R + q).to_array());
            col.push([c.x, c.y, c.z, 1.0]);
        }
        idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    Mesh::new(bevy::mesh::PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD)
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, col)
        .with_inserted_indices(bevy::mesh::Indices::U32(idx))
}

#[derive(Resource)]
struct MapAssets {
    sphere: Handle<Mesh>,
    nebula_task: Option<Task<Image>>,
    nebula_mat: Handle<StandardMaterial>,
}

/// A 3D thing on the map and where it is.
#[derive(Component, Clone, Copy, PartialEq, Debug)]
enum Thing {
    Star(usize),
    Planet(u64),
    Station(usize),
}

/// A planet's model being built.
#[derive(Component)]
struct ModelJob(Task<MeshData>);

/// A marker over something on the map: its ring and label follow it on screen.
#[derive(Component)]
struct Marker {
    of: Mark,
    /// Grows when hovered or picked.
    scale: f32,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Mark {
    Thing(Thing),
    You,
}

#[derive(Component)]
struct MarkerRing;

#[derive(Component, Clone, Copy)]
enum MapText {
    Title,
    Hint,
    Status,
}

fn on_activate(ev: On<Activate>, actions: Query<&MapAction>, mut map: ResMut<GalaxyMap>) {
    if let Ok(&a) = actions.get(ev.entity) {
        map.queue.push(a);
    }
}

fn layer() -> RenderLayers {
    RenderLayers::layer(LAYER)
}

fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, mut gizmo: ResMut<GizmoConfigStore>, seed: Res<crate::sky::SkySeed>) {
    commands.spawn((
        MapCamera,
        Camera3d::default(),
        Camera { order: 1, is_active: false, clear_color: ClearColorConfig::Custom(Color::srgb(0.004, 0.006, 0.012)), ..default() },
        Projection::Perspective(PerspectiveProjection { fov: 50f32.to_radians(), near: 0.0005, far: 30_000.0, ..default() }),
        bevy::camera::Hdr,
        bevy::post_process::bloom::Bloom { intensity: 0.3, ..bevy::post_process::bloom::Bloom::NATURAL },
        Transform::default(),
        layer(),
    ));
    let (config, _) = gizmo.config_mut::<MapGizmos>();
    config.render_layers = layer();
    config.line.width = 1.5;

    let sphere = meshes.add(Sphere::new(1.0).mesh().uv(64, 32));
    let nebula_mat = mats.add(StandardMaterial { base_color: Color::BLACK, unlit: true, cull_mode: None, ..default() });
    commands.spawn((Backdrop, Mesh3d(sphere.clone()), MeshMaterial3d(nebula_mat.clone()), Transform::from_scale(Vec3::splat(20_000.0)), layer()));
    let dust_mesh = meshes.add(dust(seed.0));
    let dust_mat = mats.add(StandardMaterial { base_color: Color::WHITE, unlit: true, cull_mode: None, ..default() });
    commands.spawn((Dust, Mesh3d(dust_mesh), MeshMaterial3d(dust_mat), Transform::default(), layer()));
    let s = seed.0;
    let task = AsyncComputeTaskPool::get().spawn(async move { nebula(s) });
    commands.insert_resource(MapAssets { sphere, nebula_task: Some(task), nebula_mat });
}

/// Another world was started: its own dust and nebula (the first frame's are the ones `setup` made).
fn reseed(seed: Res<crate::sky::SkySeed>, mut assets: ResMut<MapAssets>, dust: Query<&Mesh3d, With<Dust>>, mut meshes: ResMut<Assets<Mesh>>, mut first: Local<bool>) {
    if !std::mem::replace(&mut *first, true) {
        return;
    }
    for m in &dust {
        if let Some(mut mesh) = meshes.get_mut(&m.0) {
            *mesh = self::dust(seed.0);
        }
    }
    let s = seed.0;
    assets.nebula_task = Some(AsyncComputeTaskPool::get().spawn(async move { nebula(s) }));
}

fn finish_nebula(mut assets: ResMut<MapAssets>, mut images: ResMut<Assets<Image>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let Some(task) = assets.nebula_task.as_mut() else { return };
    let Some(img) = block_on(future::poll_once(task)) else { return };
    assets.nebula_task = None;
    let h = images.add(img);
    if let Some(mut m) = mats.get_mut(&assets.nebula_mat) {
        m.base_color = Color::WHITE;
        m.base_color_texture = Some(h);
    }
}

/// Smooth value noise over 3D space, 0..1.
fn noise3(p: Vec3, seed: u64) -> f32 {
    let f = p.floor();
    let t = p - f;
    let t = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let h = |x: f32, y: f32, z: f32| {
        let k = mix(seed ^ mix((f.x + x) as i64 as u64 ^ mix((f.y + y) as i64 as u64 ^ mix((f.z + z) as i64 as u64))));
        (k >> 40) as f32 / (1u64 << 24) as f32
    };
    let l = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = l(h(0., 0., 0.), h(1., 0., 0.), t.x);
    let x10 = l(h(0., 1., 0.), h(1., 1., 0.), t.x);
    let x01 = l(h(0., 0., 1.), h(1., 0., 1.), t.x);
    let x11 = l(h(0., 1., 1.), h(1., 1., 1.), t.x);
    l(l(x00, x10, t.y), l(x01, x11, t.y), t.z)
}

fn fbm(p: Vec3, seed: u64, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut freq, mut norm) = (0.0, 1.0, 1.0, 0.0);
    for o in 0..octaves {
        sum += noise3(p * freq, seed ^ o as u64) * amp;
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    sum / norm
}

/// The map's sky: dark space, a band of the galaxy's glow, and nebulae in two colors from the
/// seed (the far stars are `dust`, sharp points in front of it).
fn nebula(seed: u64) -> Image {
    let (w, h) = (2048usize, 1024usize);
    let hue = |k: u64| {
        let a = (mix(seed ^ k) % 1000) as f32 / 1000.0 * TAU;
        Vec3::new(0.5 + 0.5 * a.cos(), 0.5 + 0.5 * (a + 2.1).cos(), 0.5 + 0.5 * (a + 4.2).cos())
    };
    let (c1, c2) = (hue(1) * Vec3::new(0.5, 1.0, 0.6), hue(2));
    let band_axis = Vec3::new(0.2, 1.0, 0.1).normalize();
    let mut data = vec![0u8; w * h * 4];
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let (th, ph) = (u * TAU, v * PI);
            let d = Vec3::new(ph.sin() * th.cos(), ph.cos(), ph.sin() * th.sin());
            let band = (-(d.dot(band_axis) * 3.2).powi(2)).exp();
            let cloud = fbm(d * 2.2 + Vec3::splat(7.0), seed, 6);
            let wisps = fbm(d * 5.0 + Vec3::splat(cloud * 2.0), seed ^ 9, 5);
            let a = ((cloud - 0.45) * 2.4).clamp(0.0, 1.0).powf(1.6) * (0.35 + 0.65 * band);
            let b = ((wisps - 0.5) * 3.0).clamp(0.0, 1.0).powf(2.0) * 0.6;
            let col = Vec3::new(0.008, 0.01, 0.02) + c1 * a * 0.22 + c2 * b * 0.12 * (0.4 + band) + Vec3::new(0.9, 0.85, 0.8) * band * 0.05 * cloud;
            let i = (y * w + x) * 4;
            for k in 0..3 {
                data[i + k] = (col[k].clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
            }
            data[i + 3] = 255;
        }
    }
    Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD)
}

/// Systems around `here`, their planets where they are now (customizations included).
fn systems_around(space: &Space, here: DVec3, t: f64) -> Vec<StarSystem> {
    let c = Universe::sector_of(here);
    let mut out = Vec::new();
    for x in -RANGE..=RANGE {
        for y in -RANGE_Y..=RANGE_Y {
            for z in -RANGE..=RANGE {
                let Some(mut sys) = space.universe.system(c + IVec3::new(x, y, z)) else { continue };
                for p in &mut sys.planets {
                    if let Some(spec) = space.planet_spec(sys.sector, p.id, t) {
                        *p = spec;
                    }
                }
                out.push(sys);
            }
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn toggle(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    mut map: ResMut<GalaxyMap>,
    menu: Res<crate::menu::PlanetMenu>,
    space: Res<Space>,
    clock: Res<Clock>,
    player: Single<&Player>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
) {
    if map.is_open() {
        if keys.just_pressed(KeyCode::KeyM) || keys.just_pressed(KeyCode::Escape) {
            close(&mut commands, &mut map, &mut cursor);
        }
    } else if keys.just_pressed(KeyCode::KeyM) && !menu.is_open() {
        let b = &player.body;
        let here = b.world_point(b.pos);
        let systems = systems_around(&space, here, clock.0);
        let home_sector = b.planet().and_then(|id| space.planets.get(&id)).map(|p| Universe::sector_of(p.spec.star()));
        let home = home_sector.and_then(|s| systems.iter().position(|x| x.sector == s));
        let (root, markers, card, card_body, fade) = spawn_ui(&mut commands);
        // Open on your own system, coming out from where you are; else on the galaxy.
        let level = home.map_or(Level::Galaxy, Level::System);
        let start = Orbit { focus: Vec3::ZERO, yaw: 0.6, pitch: 0.5, dist: 4.0 };
        let mut v = View {
            root,
            markers,
            card,
            card_body,
            fade,
            scene: Vec::new(),
            level,
            systems,
            here,
            home,
            system: home,
            planet: b.planet(),
            station: None,
            cam: start,
            target: start,
            follow: Follow::You,
            transition: None,
            card_for: None,
            card_shown: 0.0,
            regrab: cursor.grab_mode != CursorGrabMode::None,
        };
        v.target.dist = match level {
            Level::Galaxy => 90.0,
            Level::System(_) => 70.0,
        };
        map.open = Some(v);
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

/// The map's camera takes the screen (and the UI) while the map is open, however it opened or
/// closed (M, Esc, or a button that leaves it to watch a launch or a trip).
fn swap_cameras(
    mut commands: Commands,
    map: Res<GalaxyMap>,
    mut main_cam: Single<(Entity, &mut Camera), (With<crate::sky::SkyCamera>, Without<MapCamera>)>,
    mut map_cam: Single<(Entity, &mut Camera), With<MapCamera>>,
) {
    let open = map.is_open();
    if map_cam.1.is_active == open {
        return;
    }
    main_cam.1.is_active = !open;
    map_cam.1.is_active = open;
    if open {
        commands.entity(main_cam.0).remove::<IsDefaultUiCamera>();
        commands.entity(map_cam.0).insert(IsDefaultUiCamera);
    } else {
        commands.entity(map_cam.0).remove::<IsDefaultUiCamera>();
        commands.entity(main_cam.0).insert(IsDefaultUiCamera);
    }
}

fn close(commands: &mut Commands, map: &mut GalaxyMap, cursor: &mut CursorOptions) {
    if let Some(v) = map.open.take() {
        commands.entity(v.root).despawn();
        for e in v.scene {
            commands.entity(e).despawn();
        }
        if v.regrab {
            cursor.grab_mode = CursorGrabMode::Locked;
            cursor.visible = false;
        }
    }
    map.queue.clear();
}

/// The map's own position of a world point, at the current level.
struct Layout {
    level: Level,
    here: DVec3,
    star: DVec3,
    scale: f32,
}

impl Layout {
    fn of(v: &View) -> Self {
        match v.level {
            Level::Galaxy => Self { level: v.level, here: v.here, star: DVec3::ZERO, scale: 0.0 },
            Level::System(i) => {
                let sys = &v.systems[i];
                let far = sys.planets.iter().filter(|p| !p.is_moon()).map(|p| p.orbit.radius).fold(1.0, f32::max);
                Self { level: v.level, here: v.here, star: sys.center, scale: SYSTEM_SIZE / far }
            }
        }
    }

    fn galaxy(&self, world: DVec3) -> Vec3 {
        ((world - self.here) * GALAXY_SCALE).as_vec3()
    }

    fn system(&self, world: DVec3) -> Vec3 {
        (world - self.star).as_vec3() * self.scale
    }
}

/// Where a planet is now (live if streamed in, else from its orbit).
fn planet_now(space: &Space, spec: &PlanetSpec, t: f64) -> PlanetSpec {
    match space.planets.get(&spec.id) {
        Some(p) => p.spec.clone(),
        None => {
            let mut s = spec.clone();
            s.advance(t);
            s
        }
    }
}

/// Builds a level's 3D scene and markers.
#[allow(clippy::too_many_arguments)]
fn build_level(
    commands: &mut Commands,
    v: &mut View,
    assets: &MapAssets,
    mats: &mut Assets<StandardMaterial>,
    voxel_mats: &mut Assets<VoxelMaterial>,
    atlas: &Atlas,
    stations: &Stations,
    space: &Space,
    t: f64,
) {
    for e in v.scene.drain(..) {
        commands.entity(e).despawn();
    }
    commands.entity(v.markers).despawn_children();
    let mut markers = Vec::new();
    match v.level {
        Level::Galaxy => {
            for (i, sys) in v.systems.iter().enumerate() {
                let glow = LinearRgba::from(rgb(sys.star_color)) * 18.0;
                let m = mats.add(StandardMaterial { base_color: Color::LinearRgba(glow), unlit: true, ..default() });
                let size = 0.35 + (sys.star_radius - 800.0) / 4800.0;
                v.scene.push(commands.spawn((Thing::Star(i), Mesh3d(assets.sphere.clone()), MeshMaterial3d(m), Transform::from_scale(Vec3::splat(size)), layer())).id());
                let n = stations_in(stations, sys);
                let note = if n > 0 { format!("  [{n} station{}]", if n == 1 { "" } else { "s" }) } else { String::new() };
                markers.push((Mark::Thing(Thing::Star(i)), MapAction::System(i), format!("{}{note}", sys.name), rgb(sys.star_color), 18.0));
            }
        }
        Level::System(i) => {
            let sys = &v.systems[i];
            let lay = Layout::of(v);
            let glow = LinearRgba::from(rgb(sys.star_color)) * 14.0;
            let m = mats.add(StandardMaterial { base_color: Color::LinearRgba(glow), unlit: true, ..default() });
            let size = (sys.star_radius * lay.scale).max(0.4);
            v.scene.push(commands.spawn((Thing::Star(i), Mesh3d(assets.sphere.clone()), MeshMaterial3d(m), Transform::from_scale(Vec3::splat(size)), layer())).id());
            markers.push((Mark::Thing(Thing::Star(i)), MapAction::System(i), sys.name.clone(), rgb(sys.star_color), 22.0));
            let pool = AsyncComputeTaskPool::get();
            for p in &sys.planets {
                let spec = planet_now(space, p, t);
                let mat = voxel_mats.add(VoxelMaterial::new(sys.star_color, atlas.0.clone()));
                let s = spec.clone();
                let job = pool.spawn(async move { far_mesh(&s, 40) });
                v.scene.push(commands.spawn((Thing::Planet(p.id), ModelJob(job), MeshMaterial3d(mat), Transform::default(), Visibility::default(), layer())).id());
                markers.push((Mark::Thing(Thing::Planet(p.id)), MapAction::Planet(p.id), p.name.clone(), rgb(p.biome.color()), 20.0));
            }
            for (k, slot) in stations.list.iter().enumerate() {
                let s = slot.station.read().unwrap();
                let Some(o) = s.orbit else { continue };
                if !sys.planets.iter().any(|p| p.id == o.planet) {
                    continue;
                }
                let mat = voxel_mats.add(VoxelMaterial::new(sys.star_color, atlas.0.clone()));
                let mut e = commands.spawn((Thing::Station(k), MeshMaterial3d(mat), Transform::default(), Visibility::default(), layer()));
                match stations.drawn(k) {
                    Some(mesh) => {
                        e.insert(MeshFrom(mesh));
                    }
                    None => {
                        // Not drawn in the world (its planet is far): meshed here.
                        let st = slot.station.clone();
                        e.insert(ModelJob(pool.spawn(async move { orbit_core::mesh::station_mesh(&st.read().unwrap()) })));
                    }
                }
                v.scene.push(e.id());
                markers.push((Mark::Thing(Thing::Station(k)), MapAction::Station(k), s.name.clone(), STATION, 14.0));
            }
        }
    }
    commands.entity(v.markers).with_children(|m| {
        for (of, action, label, color, size) in markers {
            spawn_marker(m, of, Some(action), &label, color, size);
        }
        spawn_marker(m, Mark::You, None, "You", ACCENT, 12.0);
    });
}

/// A station model shows the mesh the world draws it with.
#[derive(Component)]
struct MeshFrom(Entity);

fn stations_in(stations: &Stations, sys: &StarSystem) -> usize {
    stations.list.iter().filter(|s| s.station.read().unwrap().orbit.is_some_and(|o| sys.planets.iter().any(|p| p.id == o.planet))).count()
}

fn spawn_marker(parent: &mut ChildSpawnerCommands, of: Mark, action: Option<MapAction>, label: &str, color: Color, size: f32) {
    let you = of == Mark::You;
    let mut e = parent.spawn((
        Marker { of, scale: 1.0 },
        Node { position_type: PositionType::Absolute, width: px(size), height: px(size), ..default() },
        UiTransform::default(),
        Visibility::Hidden,
    ));
    match action {
        Some(a) => {
            e.insert((Button, Hovered::default(), a));
        }
        None => {
            e.insert(Pickable::IGNORE);
        }
    }
    e.with_children(|m| {
        // The ring, a dot in it; you are a diamond.
        m.spawn((
            MarkerRing,
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                top: px(0),
                width: percent(100),
                height: percent(100),
                border: UiRect::all(px(if you { 2.0 } else { 1.5 })),
                border_radius: if you { BorderRadius::ZERO } else { BorderRadius::all(percent(50)) },
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            UiTransform { rotation: if you { Rot2::degrees(45.0) } else { Rot2::IDENTITY }, ..default() },
            BorderColor::all(if you { color } else { Color::srgba(1.0, 1.0, 1.0, 0.8) }),
            Pickable::IGNORE,
            children![(
                Node { width: px(size * 0.28), height: px(size * 0.28), border_radius: BorderRadius::all(percent(50)), ..default() },
                BackgroundColor(color),
                Pickable::IGNORE,
            )],
        ));
        m.spawn((
            Text::new(label),
            TextFont { font_size: FontSize::Px(if you { 13.0 } else { 14.0 }), ..default() },
            TextColor(if you { ACCENT } else { TEXT }),
            TextShadow { offset: Vec2::new(1.0, 1.0), color: Color::BLACK.with_alpha(0.9) },
            Node { position_type: PositionType::Absolute, left: px(size + 4.0), bottom: px(size * 0.55), ..default() },
            TextLayout::no_wrap(),
            Pickable::IGNORE,
        ));
    });
}

fn spawn_ui(commands: &mut Commands) -> (Entity, Entity, Entity, Entity, Entity) {
    let (mut markers, mut card, mut body, mut fade) = (Entity::PLACEHOLDER, Entity::PLACEHOLDER, Entity::PLACEHOLDER, Entity::PLACEHOLDER);
    let root = commands
        .spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, GlobalZIndex(20), Pickable::IGNORE))
        .with_children(|r| {
            markers = r.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, Pickable::IGNORE)).id();
            // Top: where you are on the map, and how to move it.
            r.spawn((Node { position_type: PositionType::Absolute, left: px(18), top: px(14), flex_direction: FlexDirection::Column, row_gap: px(2), ..default() }, Pickable::IGNORE))
                .with_children(|t| {
                    t.spawn((Text::default(), TextFont { font_size: FontSize::Px(22.0), ..default() }, TextColor(TEXT), MapText::Title, Pickable::IGNORE));
                    t.spawn((Text::default(), TextFont { font_size: FontSize::Px(13.0), ..default() }, TextColor(MUTED), MapText::Hint, Pickable::IGNORE));
                });
            // The card of what is hovered or picked.
            card = r
                .spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(330),
                        flex_direction: FlexDirection::Column,
                        row_gap: px(4),
                        padding: UiRect::all(px(12)),
                        border: UiRect::left(px(2)),
                        ..default()
                    },
                    BackgroundColor(CARD),
                    BorderColor::all(CARD_EDGE),
                    UiTransform::default(),
                    Visibility::Hidden,
                ))
                .with_children(|c| {
                    body = c.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(4), ..default() }).id();
                })
                .id();
            // Bottom bar: where you are, and what you can do.
            r.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    right: px(0),
                    bottom: px(0),
                    padding: UiRect::axes(px(16), px(8)),
                    column_gap: px(8),
                    align_items: AlignItems::Center,
                    flex_wrap: FlexWrap::Wrap,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            ))
            .with_children(|b| {
                b.spawn((Text::default(), TextFont { font_size: FontSize::Px(14.0), ..default() }, TextColor(ACCENT), MapText::Status, Node { flex_grow: 1.0, ..default() }));
                for (label, a) in [("Center on me", MapAction::Center), ("Galaxy", MapAction::Back), ("Launch (K)", MapAction::Launch), ("Land (L)", MapAction::Land), ("Stop autopilot", MapAction::Cancel), ("Leave (M)", MapAction::Close)] {
                    b.spawn(button(label, a));
                }
            });
            fade = r.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::NONE), Pickable::IGNORE)).id();
        })
        .id();
    (root, markers, card, body, fade)
}

fn colored(s: &str, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (Text::new(s), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(label: &str, action: MapAction) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node { min_height: px(26), padding: UiRect::horizontal(px(10)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(4)), ..default() },
        BackgroundColor(BUTTON),
        children![colored(label, 14.0, TEXT)],
    )
}

fn run_actions(mut commands: Commands, mut map: ResMut<GalaxyMap>, mut travel: ResMut<Travel>, space: Res<Space>, player: Single<&Player>, mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>) {
    for action in std::mem::take(&mut map.queue) {
        let Some(v) = map.open.as_mut() else { return };
        let mut leave = false;
        match action {
            MapAction::System(i) => {
                if v.system == Some(i) && v.level == Level::Galaxy {
                    v.transition = Some(Transition { to: Level::System(i), t: 0.0, swapped: false });
                } else if v.level == Level::Galaxy {
                    v.system = Some(i);
                    v.planet = None;
                    v.follow = Follow::Star;
                } else {
                    v.follow = Follow::Star;
                    v.planet = None;
                    v.station = None;
                }
            }
            MapAction::Enter(i) => v.transition = Some(Transition { to: Level::System(i), t: 0.0, swapped: false }),
            MapAction::Planet(id) => {
                v.planet = Some(id);
                v.station = None;
                v.follow = Follow::Planet(id);
                v.target.dist = v.target.dist.min(3.0);
            }
            MapAction::Station(k) => {
                v.station = Some(k);
                v.follow = Follow::Station(k);
                v.target.dist = v.target.dist.min(0.25);
            }
            MapAction::Go(mode) => {
                let target = v.system.zip(v.planet).map(|(i, p)| Target { sector: v.systems[i].sector, planet: p });
                if let Some(t) = target {
                    travel.requests.push(Request::Go(t, mode));
                    leave = travel_blocker(&player.body, &space).is_none() && player.body.planet() != Some(t.planet);
                    if !leave {
                        travel.message = travel_blocker(&player.body, &space).unwrap_or_else(|| "You are already orbiting it.".into());
                    }
                }
            }
            MapAction::Launch => {
                travel.requests.push(Request::Launch);
                leave = true;
            }
            MapAction::Land => {
                travel.requests.push(Request::Land);
                leave = true;
            }
            MapAction::Cancel => travel.requests.push(Request::Cancel),
            MapAction::Back => {
                if matches!(v.level, Level::System(_)) {
                    v.transition = Some(Transition { to: Level::Galaxy, t: 0.0, swapped: false });
                }
            }
            MapAction::Center => v.follow = Follow::You,
            MapAction::Close => leave = true,
        }
        if leave {
            close(&mut commands, &mut map, &mut cursor);
            return;
        }
    }
}

/// Drag turns the camera, the wheel zooms, WASD pans, Space comes back to you, Backspace
/// goes out to the galaxy. Then the camera eases toward where it is wanted.
#[allow(clippy::too_many_arguments)]
fn controls(
    mut map: ResMut<GalaxyMap>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    time: Res<Time>,
    mut cam: Single<&mut Transform, (With<MapCamera>, Without<Backdrop>, Without<Dust>)>,
    mut backdrop: Single<&mut Transform, (With<Backdrop>, Without<Dust>)>,
    mut dust: Single<&mut Transform, With<Dust>>,
) {
    let map = &mut *map;
    let Some(v) = map.open.as_mut() else { return };
    let dt = time.delta_secs().min(0.1);
    if v.transition.is_none() {
        if mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right) {
            v.target.yaw -= motion.delta.x * 0.006;
            v.target.pitch = (v.target.pitch + motion.delta.y * 0.005).clamp(-1.45, 1.45);
        }
        if scroll.delta.y != 0.0 {
            let (lo, hi) = match v.level {
                Level::Galaxy => (3.0, 700.0),
                Level::System(_) => (0.02, 420.0),
            };
            v.target.dist = (v.target.dist * 1.18f32.powf(-scroll.delta.y.signum())).clamp(lo, hi);
        }
        let axis = |pos, neg| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
        let pan = Vec2::new(axis(KeyCode::KeyD, KeyCode::KeyA), axis(KeyCode::KeyW, KeyCode::KeyS));
        if pan != Vec2::ZERO {
            v.follow = Follow::Free;
            let fwd = -Vec3::new(v.cam.yaw.sin(), 0.0, v.cam.yaw.cos());
            let right = Vec3::new(fwd.z * -1.0, 0.0, fwd.x);
            v.target.focus += (fwd * pan.y + right * pan.x) * v.target.dist * 0.9 * dt;
        }
        if keys.just_pressed(KeyCode::Space) {
            v.follow = Follow::You;
        }
        if keys.just_pressed(KeyCode::Backspace) {
            map.queue.push(MapAction::Back);
        }
    }
    let Some(v) = map.open.as_mut() else { return };
    let k = 1.0 - (-7.0 * dt).exp();
    let target = v.target;
    v.cam.approach(&target, k);
    let eye = v.cam.eye();
    **cam = Transform::from_translation(eye).looking_at(v.cam.focus, Vec3::Y);
    backdrop.translation = eye;
    dust.translation = eye;
}

/// Diving into a system and pulling out of it: the camera rushes in (or out) and the view fades,
/// the level changes behind the fade, and the camera eases out into the new one.
#[allow(clippy::too_many_arguments)]
fn transition(
    mut commands: Commands,
    mut map: ResMut<GalaxyMap>,
    assets: Res<MapAssets>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut voxel_mats: ResMut<Assets<VoxelMaterial>>,
    atlas: Res<Atlas>,
    stations: Res<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    time: Res<Time>,
    mut fades: Query<&mut BackgroundColor>,
) {
    let Some(v) = map.open.as_mut() else { return };
    // First frame: the level it opened on.
    if v.scene.is_empty() && v.transition.is_none() {
        build_level(&mut commands, v, &assets, &mut mats, &mut voxel_mats, &atlas, &stations, &space, clock.0);
        return;
    }
    let dt = time.delta_secs().min(0.1);
    let mut alpha = 0.0;
    if let Some(mut tr) = v.transition.take() {
        tr.t += dt;
        let mut keep = true;
        if !tr.swapped {
            alpha = (tr.t / DIVE).min(1.0);
            match tr.to {
                Level::System(i) => {
                    let at = Layout::of(v).galaxy(v.systems[i].center);
                    v.target.focus = at;
                    v.target.dist = 0.6;
                    v.follow = Follow::Free;
                }
                Level::Galaxy => v.target.dist = 900.0,
            }
            if tr.t >= DIVE {
                let (from, to) = (v.level, tr.to);
                v.level = to;
                if let Level::System(i) = to {
                    v.system = Some(i);
                    v.planet = None;
                }
                v.station = None;
                tr.swapped = true;
                tr.t = 0.0;
                build_level(&mut commands, v, &assets, &mut mats, &mut voxel_mats, &atlas, &stations, &space, clock.0);
                // The new level starts close in and eases out.
                match (from, to) {
                    (_, Level::System(_)) => {
                        v.cam.focus = Vec3::ZERO;
                        v.cam.dist = 4.0;
                        v.target = Orbit { focus: Vec3::ZERO, dist: 110.0, ..v.cam };
                        v.follow = Follow::Star;
                    }
                    (Level::System(i), Level::Galaxy) => {
                        let at = Layout::of(v).galaxy(v.systems[i].center);
                        v.cam.focus = at;
                        v.cam.dist = 1.5;
                        v.target = Orbit { focus: at, dist: 45.0, ..v.cam };
                        v.follow = Follow::Free;
                    }
                    _ => {}
                }
                alpha = 1.0;
            }
        } else {
            alpha = 1.0 - (tr.t / SURFACE).min(1.0);
            keep = tr.t < SURFACE;
        }
        if keep {
            v.transition = Some(tr);
        }
    }
    if let Ok(mut bg) = fades.get_mut(v.fade) {
        let c = Color::srgba(0.0, 0.0, 0.0, alpha * alpha * (3.0 - 2.0 * alpha));
        if bg.0 != c {
            bg.0 = c;
        }
    }
}

fn finish_models(mut commands: Commands, mut jobs: Query<(Entity, &mut ModelJob)>, mut meshes: ResMut<Assets<Mesh>>, from: Query<(Entity, &MeshFrom)>, world_meshes: Query<&Mesh3d, Without<MeshFrom>>) {
    for (e, mut job) in &mut jobs {
        if let Some(data) = block_on(future::poll_once(&mut job.0)) {
            let (mesh, aabb) = gpu_mesh(&mut meshes, data);
            commands.entity(e).remove::<ModelJob>().insert((Mesh3d(mesh), aabb));
        }
    }
    // Station models share the world's mesh of the station (it follows what is built).
    for (e, src) in &from {
        if let Ok(m) = world_meshes.get(src.0) {
            commands.entity(e).insert(Mesh3d(m.0.clone()));
        }
    }
}

/// Where everything is on the map now: planets and stations go round in real time.
#[allow(clippy::too_many_arguments)]
fn place_scene(
    mut map: ResMut<GalaxyMap>,
    space: Res<Space>,
    stations: Res<Stations>,
    clock: Res<Clock>,
    player: Single<&Player>,
    mut things: Query<(&Thing, &mut Transform, Option<&MeshMaterial3d<VoxelMaterial>>)>,
    mut voxel_mats: ResMut<Assets<VoxelMaterial>>,
) {
    let Some(v) = map.open.as_mut() else { return };
    let lay = Layout::of(v);
    let t = clock.0;
    let you = you_on_map(v, &lay, &space, &stations, &player.body, t);
    let mut focus = None;
    for (thing, mut tr, mat) in &mut things {
        match (*thing, lay.level) {
            (Thing::Star(i), Level::Galaxy) => {
                tr.translation = lay.galaxy(v.systems[i].center);
                if v.follow == Follow::Star && v.system == Some(i) {
                    focus = Some(tr.translation);
                }
            }
            (Thing::Star(_), Level::System(_)) => {
                tr.translation = Vec3::ZERO;
                if v.follow == Follow::Star {
                    focus = Some(Vec3::ZERO);
                }
            }
            (Thing::Planet(id), Level::System(i)) => {
                let Some(p) = v.systems[i].planets.iter().find(|p| p.id == id) else { continue };
                let spec = planet_now(&space, p, t);
                let at = lay.system(spec.center);
                *tr = Transform { translation: at, rotation: spec.rotation, scale: Vec3::splat(lay.scale) };
                if let Some(mut m) = mat.and_then(|m| voxel_mats.get_mut(&m.0)) {
                    m.place(at, Vec3::ZERO);
                }
                if v.follow == Follow::Planet(id) {
                    focus = Some(at);
                }
            }
            (Thing::Station(k), Level::System(i)) => {
                let Some(slot) = stations.list.get(k) else { continue };
                let s = slot.station.read().unwrap();
                let Some(o) = s.orbit else {
                    tr.scale = Vec3::ZERO;
                    continue;
                };
                let Some(p) = v.systems[i].planets.iter().find(|p| p.id == o.planet) else { continue };
                let spec = planet_now(&space, p, t);
                let off = o.offset(t).as_vec3();
                let at = lay.system(spec.center) + off * lay.scale;
                *tr = Transform { translation: at, rotation: o.rotation(t), scale: Vec3::splat(lay.scale * STATION_SCALE) };
                if let Some(mut m) = mat.and_then(|m| voxel_mats.get_mut(&m.0)) {
                    m.place_flat(at, (-at).normalize_or(Vec3::Y), 1.0);
                }
                if v.follow == Follow::Station(k) {
                    focus = Some(at);
                }
            }
            _ => {}
        }
    }
    if v.follow == Follow::You {
        focus = you;
    }
    if let (Some(f), None) = (focus, v.transition.as_ref()) {
        v.target.focus = f;
    }
}

/// You on the map, at the current level.
fn you_on_map(v: &View, lay: &Layout, space: &Space, stations: &Stations, body: &orbit_core::player::Body, t: f64) -> Option<Vec3> {
    let world = body.world_point(body.pos);
    match lay.level {
        Level::Galaxy => Some(lay.galaxy(world)),
        Level::System(i) => {
            let sys = &v.systems[i];
            // On a station: where its model is.
            if let Some(o) = body.station.as_ref().and_then(|id| stations.by_id(id)).and_then(|s| s.station.read().unwrap().orbit) {
                let p = sys.planets.iter().find(|p| p.id == o.planet)?;
                let spec = planet_now(space, p, t);
                let off = o.offset(t).as_vec3();
                return Some(lay.system(spec.center) + off * lay.scale);
            }
            match body.planet().and_then(|id| sys.planets.iter().find(|p| p.id == id)) {
                Some(p) => {
                    let spec = planet_now(space, p, t);
                    let rel = (world - spec.center).as_vec3();
                    Some(lay.system(spec.center) + rel * lay.scale)
                }
                None if world.distance(sys.center) < 150_000.0 => Some(lay.system(world)),
                None => None,
            }
        }
    }
}

/// Puts each marker over what it marks, grows it when hovered or picked, fades far labels.
#[allow(clippy::too_many_arguments)]
fn place_markers(
    map: Res<GalaxyMap>,
    cam: Single<(&Camera, &Transform), With<MapCamera>>,
    things: Query<(&Thing, &Transform), Without<MapCamera>>,
    space: Res<Space>,
    stations: Res<Stations>,
    clock: Res<Clock>,
    player: Single<&Player>,
    time: Res<Time>,
    mut markers: Query<(&mut Marker, &mut Node, &mut UiTransform, &mut Visibility, Option<&Hovered>, Option<&MapAction>)>,
) {
    let Some(v) = map.open.as_ref() else { return };
    let (camera, ct) = *cam;
    let gt = GlobalTransform::from(*ct);
    let lay = Layout::of(v);
    let you = you_on_map(v, &lay, &space, &stations, &player.body, clock.0);
    let k = 1.0 - (-12.0 * time.delta_secs()).exp();
    let pulse = 1.0 + 0.12 * (time.elapsed_secs() * 4.0).sin();
    for (mut m, mut node, mut ut, mut vis, hovered, action) in &mut markers {
        let at = match m.of {
            Mark::You => you,
            Mark::Thing(th) => things.iter().find(|(x, _)| **x == th).map(|(_, t)| t.translation),
        };
        let screen = at.and_then(|p| camera.world_to_viewport(&gt, p).ok().map(|s| (s, p.distance(ct.translation))));
        let Some((s, d)) = screen else {
            *vis = Visibility::Hidden;
            continue;
        };
        *vis = Visibility::Inherited;
        let picked = match action {
            Some(MapAction::System(i)) => v.system == Some(*i) && v.level == Level::Galaxy,
            Some(MapAction::Planet(id)) => v.planet == Some(*id),
            Some(MapAction::Station(k)) => v.station == Some(*k),
            _ => false,
        };
        let hover = hovered.is_some_and(Hovered::get);
        let want = if hover { 1.45 } else if picked { 1.25 } else { 1.0 } * if m.of == Mark::You { pulse } else { 1.0 };
        m.scale += (want - m.scale) * k;
        // Far stars' markers shrink a little.
        let far = match lay.level {
            Level::Galaxy => (1.0 - (d / 400.0)).clamp(0.55, 1.0),
            Level::System(_) => 1.0,
        };
        let size = match node.width {
            Val::Px(w) => w,
            _ => 16.0,
        };
        let (l, t) = (px(s.x - size * 0.5), px(s.y - size * 0.5));
        if node.left != l || node.top != t {
            node.left = l;
            node.top = t;
        }
        ut.scale = Vec2::splat(m.scale * far);
    }
}

/// Lanes between neighboring stars; orbits of planets and stations.
fn draw_lines(map: Res<GalaxyMap>, stations: Res<Stations>, elevators: Res<crate::elevator::Elevators>, space: Res<Space>, clock: Res<Clock>, mut gizmos: Gizmos<MapGizmos>) {
    let Some(v) = map.open.as_ref() else { return };
    let lay = Layout::of(v);
    match lay.level {
        Level::Galaxy => {
            let pts: Vec<Vec3> = v.systems.iter().map(|s| lay.galaxy(s.center)).collect();
            for (i, a) in pts.iter().enumerate() {
                let mut near: Vec<(f32, usize)> = pts.iter().enumerate().filter(|(j, _)| *j != i).map(|(j, b)| (a.distance(*b), j)).collect();
                near.sort_by(|x, y| x.0.total_cmp(&y.0));
                for &(d, j) in near.iter().take(3) {
                    if j > i || near.len() < 2 {
                        let alpha = (0.22 * (1.0 - d / 140.0)).max(0.03);
                        gizmos.line(*a, pts[j], Color::srgba(0.6, 0.85, 1.0, alpha));
                    }
                }
            }
            if let Some(i) = v.system {
                let c = pts[i];
                gizmos.circle(Isometry3d::new(c, Quat::from_rotation_x(FRAC_PI_2)), 2.2, ACCENT.with_alpha(0.7)).resolution(48);
            }
        }
        Level::System(i) => {
            let sys = &v.systems[i];
            let t = clock.0;
            for p in &sys.planets {
                let rot = p.orbit.tilt * Quat::from_rotation_x(FRAC_PI_2);
                let sel = v.planet == Some(p.id);
                // Round the star, or a moon's round its planet.
                let around = match p.host.and_then(|h| sys.planets.iter().find(|q| q.id == h.id)) {
                    Some(host) => lay.system(planet_now(&space, host, t).center),
                    None => Vec3::ZERO,
                };
                gizmos
                    .circle(Isometry3d::new(around, rot), p.orbit.radius * lay.scale, Color::srgba(0.55, 0.8, 1.0, if sel { 0.55 } else { 0.18 }))
                    .resolution(160);
                // How far its pull reaches (where orbits around it can be), facing the camera.
                let spec = planet_now(&space, p, t);
                let at = lay.system(spec.center);
                let face = Quat::from_rotation_arc(Vec3::Z, (v.cam.eye() - at).normalize_or(Vec3::Z));
                gizmos.circle(Isometry3d::new(at, face), spec.pull().soi * lay.scale, Color::srgba(0.55, 0.8, 1.0, if sel { 0.3 } else { 0.1 })).resolution(64);
            }
            for s in &stations.list {
                let s = s.station.read().unwrap();
                let Some(o) = s.orbit else { continue };
                let Some(p) = sys.planets.iter().find(|p| p.id == o.planet) else { continue };
                let spec = planet_now(&space, p, t);
                let r = o.radius() * lay.scale;
                let rot = Quat::from_rotation_arc(Vec3::Z, o.axis.as_vec3().normalize_or(Vec3::Y));
                gizmos.circle(Isometry3d::new(lay.system(spec.center), rot), r, STATION.with_alpha(0.35)).resolution(64);
            }
            // Space elevators: their ribbons, from the ground up to the counterweight.
            for e in &elevators.list {
                let Some(n) = e.now else { continue };
                let Some(p) = sys.planets.iter().find(|p| p.id == n.planet) else { continue };
                let c = lay.system(planet_now(&space, p, t).center);
                let d = n.rotation * n.dir;
                gizmos.line(c + d * n.bottom as f32 * lay.scale, c + d * n.counterweight * lay.scale, STATION.with_alpha(0.8));
            }
        }
    }
}

/// The card of what is hovered (else picked): beside its marker, sliding and fading in.
#[allow(clippy::too_many_arguments)]
fn card(
    mut commands: Commands,
    mut map: ResMut<GalaxyMap>,
    markers: Query<(&Hovered, &MapAction, &Node), With<Marker>>,
    mut cards: Query<(&mut Node, &mut UiTransform, &mut Visibility), Without<Marker>>,
    space: Res<Space>,
    stations: Res<Stations>,
    player: Single<&Player>,
    clock: Res<Clock>,
    window: Single<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
) {
    let Some(v) = map.open.as_mut() else { return };
    let hovered = markers.iter().find(|(h, _, _)| h.get()).map(|(_, a, n)| (*a, n.left, n.top));
    let picked = if let Some(k) = v.station {
        Some(MapAction::Station(k))
    } else if let Some(p) = v.planet.filter(|_| matches!(v.level, Level::System(_))) {
        Some(MapAction::Planet(p))
    } else {
        v.system.map(MapAction::System)
    };
    let picked_at = picked.and_then(|a| markers.iter().find(|(_, x, _)| **x == a).map(|(_, _, n)| (n.left, n.top)));
    let (what, at) = match hovered {
        Some((a, l, t)) => (Some(a), Some((l, t))),
        None => (picked, picked_at),
    };
    if what != v.card_for {
        v.card_for = what;
        v.card_shown = 0.0;
        commands.entity(v.card_body).despawn_children();
        if let Some(a) = what {
            let body = v.card_body;
            let lines = card_lines(v, a, &space, &stations, &player.body, clock.0);
            commands.entity(body).with_children(|c| fill_card(c, lines));
        }
    }
    v.card_shown = (v.card_shown + time.delta_secs() * 6.0).min(1.0);
    let Ok((mut node, mut ut, mut vis)) = cards.get_mut(v.card) else { return };
    let (Some(_), Some((Val::Px(x), Val::Px(y)))) = (what, at.map(|(l, t)| (l, t))) else {
        *vis = Visibility::Hidden;
        return;
    };
    *vis = Visibility::Inherited;
    let e = v.card_shown;
    let ease = 1.0 - (1.0 - e) * (1.0 - e);
    // To the right of the marker, unless that runs off the screen.
    let (w, h) = (window.width(), window.height());
    let left = if x + 40.0 + 330.0 < w { x + 40.0 } else { (x - 340.0).max(8.0) };
    let top = (y - 30.0).clamp(60.0, (h - 300.0).max(60.0));
    node.left = px(left);
    node.top = px(top);
    ut.translation = Val2::px(-14.0 * (1.0 - ease), 0.0);
    ut.scale = Vec2::splat(0.94 + 0.06 * ease);
}

enum CardLine {
    Title(String, Color),
    Sub(String),
    Row(String, String),
    Buttons(Vec<(String, MapAction, bool)>),
}

fn card_lines(v: &View, a: MapAction, space: &Space, stations: &Stations, body: &orbit_core::player::Body, t: f64) -> Vec<CardLine> {
    let here = body.world_point(body.pos);
    let mut out = Vec::new();
    match a {
        MapAction::System(i) | MapAction::Enter(i) => {
            let sys = &v.systems[i];
            let mine = v.home == Some(i);
            out.push(CardLine::Title(format!("{}{}", sys.name, if mine { " (you are here)" } else { "" }), rgb(sys.star_color)));
            let planets: Vec<_> = sys.planets.iter().filter(|p| !p.is_moon()).collect();
            let moons = sys.planets.len() - planets.len();
            out.push(CardLine::Sub(format!("{:.1} km  //  {} planets, {moons} moons  //  star r {:.0}", sys.center.distance(here) / 1000.0, planets.len(), sys.star_radius)));
            for p in planets.iter().take(7) {
                let m = sys.planets.iter().filter(|q| q.host.is_some_and(|h| h.id == p.id)).count();
                let note = if m > 0 { format!(", {m} moon{}", if m == 1 { "" } else { "s" }) } else { String::new() };
                out.push(CardLine::Row(p.name.clone(), format!("{:?}, r {}{note}", p.biome, p.radius as i32)));
            }
            let n = stations_in(stations, sys);
            if n > 0 {
                out.push(CardLine::Row("Stations".into(), n.to_string()));
            }
            if v.level == Level::Galaxy {
                out.push(CardLine::Buttons(vec![("Enter system".into(), MapAction::Enter(i), false)]));
            }
        }
        MapAction::Planet(id) => {
            let Some((sys, p)) = v.systems.iter().find_map(|s| s.planets.iter().find(|p| p.id == id).map(|p| (s, p))) else { return out };
            let spec = planet_now(space, p, t);
            let orbiting = body.planet() == Some(id);
            out.push(CardLine::Title(spec.name.clone(), rgb(spec.biome.color())));
            out.push(CardLine::Sub(format!("{:.1} km  //  {:?}  //  {}", spec.center.distance(here) / 1000.0, spec.biome, sys.name)));
            out.push(CardLine::Row("Radius".into(), format!("{} blocks", spec.radius as i32)));
            if spec.spin.speed.abs() > 1e-6 {
                out.push(CardLine::Row("Day".into(), fmt_time(TAU / spec.spin.speed.abs())));
            }
            match spec.host.and_then(|h| sys.planets.iter().find(|q| q.id == h.id)) {
                Some(host) => {
                    out.push(CardLine::Row("Moon of".into(), host.name.clone()));
                    out.push(CardLine::Row("Month".into(), fmt_time(TAU / spec.orbit.speed.abs().max(1e-9))));
                    out.push(CardLine::Row("Distance".into(), format!("{:.0} blocks", spec.orbit.radius)));
                }
                None => out.push(CardLine::Row("Year".into(), fmt_time(TAU / spec.orbit.speed.abs().max(1e-9)))),
            }
            let n = stations.list.iter().filter(|s| s.station.read().unwrap().orbit.is_some_and(|o| o.planet == id)).count();
            if n > 0 {
                out.push(CardLine::Row("Stations".into(), n.to_string()));
            }
            if orbiting {
                out.push(CardLine::Sub("You are here.".into()));
                out.push(CardLine::Buttons(vec![("Land (L)".into(), MapAction::Land, false)]));
            } else {
                out.push(CardLine::Buttons(vec![
                    (format!("Warp  ~{}", fmt_time(warp_time())), MapAction::Go(Mode::Warp), true),
                    (format!("Fly  ~{}", fmt_time(fly_time(here, &spec))), MapAction::Go(Mode::Fly), true),
                ]));
            }
        }
        MapAction::Station(k) => {
            let Some(slot) = stations.list.get(k) else { return out };
            let s = slot.station.read().unwrap();
            out.push(CardLine::Title(s.name.clone(), STATION));
            let span = s.bounds.span();
            out.push(CardLine::Sub(format!("Station {}  //  {} blocks  //  {} x {} x {}", s.id, s.block_count(), span.x, span.z, span.y)));
            if let Some(o) = s.orbit {
                let planet = v.systems.iter().flat_map(|s| &s.planets).find(|p| p.id == o.planet);
                if let Some(p) = planet {
                    out.push(CardLine::Row("Orbits".into(), p.name.clone()));
                    out.push(CardLine::Row("Altitude".into(), format!("{:.0} blocks", o.radius() - p.radius)));
                }
                out.push(CardLine::Row("Period".into(), fmt_time(o.period() as f32)));
            }
            if body.station.as_deref() == Some(s.id.as_str()) {
                out.push(CardLine::Sub("You are on it.".into()));
            }
        }
        _ => {}
    }
    out
}

fn fill_card(c: &mut ChildSpawnerCommands, lines: Vec<CardLine>) {
    for l in lines {
        match l {
            CardLine::Title(s, color) => {
                c.spawn(colored(&s, 20.0, color));
            }
            CardLine::Sub(s) => {
                c.spawn((colored(&s, 13.0, ACCENT), Node { margin: UiRect::bottom(px(4)), ..default() }));
            }
            CardLine::Row(k, val) => {
                c.spawn(Node { column_gap: px(8), ..default() }).with_children(|r| {
                    r.spawn((colored(&k, 14.0, MUTED), Node { width: px(110), ..default() }));
                    r.spawn(colored(&val, 14.0, TEXT));
                });
            }
            CardLine::Buttons(bs) => {
                c.spawn(Node { column_gap: px(6), margin: UiRect::top(px(6)), ..default() }).with_children(|r| {
                    for (label, a, go) in bs {
                        let mut b = r.spawn(button(&label, a));
                        if go {
                            b.insert(BackgroundColor(GO));
                        }
                    }
                });
            }
        }
    }
}

pub fn fmt_time(s: f32) -> String {
    if s < 90.0 {
        format!("{:.0} s", s)
    } else if s < 5400.0 {
        format!("{:.0} min {:02.0} s", (s / 60.0).floor(), s % 60.0)
    } else {
        format!("{:.1} h", s / 3600.0)
    }
}

fn refresh_texts(map: Res<GalaxyMap>, travel: Res<Travel>, space: Res<Space>, player: Single<&Player>, stations: Res<Stations>, mut texts: Query<(&MapText, &mut Text)>) {
    let Some(v) = map.open.as_ref() else { return };
    for (kind, mut text) in &mut texts {
        let s = match kind {
            MapText::Title => match v.level {
                Level::Galaxy => "GALAXY".into(),
                Level::System(i) => format!("GALAXY  >  {}", v.systems[i].name.to_uppercase()),
            },
            MapText::Hint => match v.level {
                Level::Galaxy => "Drag: turn  |  Wheel: zoom  |  WASD: pan  |  Click a star, click again to enter  |  Space: you  |  M / Esc: leave".into(),
                Level::System(_) => "Drag: turn  |  Wheel: zoom  |  Click a planet or station to follow it  |  Backspace: galaxy  |  Space: you".into(),
            },
            MapText::Status => {
                let b = &player.body;
                let name = |id: u64| space.planets.get(&id).map_or("?".to_string(), |p| p.spec.name.clone());
                let on = b.station.as_ref().and_then(|id| stations.by_id(id)).map(|s| s.station.read().unwrap().name.clone());
                let place = match (b.gravity_source, b.orbiting, on) {
                    (Some(id), _, _) => format!("On {}", name(id)),
                    (_, _, Some(st)) => format!("On station {st}, around {}", b.planet().map(name).unwrap_or_default()),
                    (None, Some(id), _) => format!("In orbit around {}", name(id)),
                    _ => "In deep space".into(),
                };
                let why = travel_blocker(b, &space).map(|w| format!("  //  {w}")).unwrap_or_default();
                let msg = if travel.message.is_empty() { String::new() } else { format!("  //  {}", travel.message) };
                format!("{place}{why}{msg}")
            }
        };
        if text.0 != s {
            text.0 = s;
        }
    }
}

#[allow(clippy::type_complexity)]
fn style_buttons(map: Res<GalaxyMap>, time: Res<Time>, mut buttons: Query<(&Hovered, Has<Pressed>, &MapAction, &mut BackgroundColor), (With<Button>, Without<Marker>)>) {
    if !map.is_open() {
        return;
    }
    let k = 1.0 - (-14.0 * time.delta_secs()).exp();
    for (hovered, pressed, action, mut bg) in &mut buttons {
        let base = if matches!(action, MapAction::Go(_)) { GO } else { BUTTON };
        let want = if pressed {
            PRESS
        } else if hovered.get() {
            HOVER
        } else {
            base
        };
        // Colors ease, so buttons light up rather than snap.
        let now = bg.0.to_srgba();
        let w = want.to_srgba();
        let c = Color::srgba(now.red + (w.red - now.red) * k, now.green + (w.green - now.green) * k, now.blue + (w.blue - now.blue) * k, now.alpha + (w.alpha - now.alpha) * k);
        bg.0 = c;
    }
}
