//! Streams the universe around the player: distant systems as glowing stars, nearby systems with
//! their orbiting planets, and voxel planets built off the main thread when close.
//!
//! Levels of detail, all blocky: the chunks around the camera are real voxels (caves and all);
//! from there out to the horizon (seen from orbit, the whole face of the planet) chunks are shells,
//! the same blocks, trees and plants without the caves underground; past them, and for every
//! other planet, a far mesh of the terrain's columns stands in, a
//! detailed one (about two blocks a column) when the planet is near and a coarse one when it is
//! far. A planet whose voxels stay loaded (edited) but that is far away draws only its far mesh.

use crate::lighting::{LiquidMaterial, VoxelMaterial};
use crate::textures::Atlas;
use bevy::asset::RenderAssetUsages;
use bevy::camera::primitives::Aabb;
use bevy::math::DVec3;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::platform::collections::HashMap;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, block_on, futures_lite::future};
use orbit_core::mesh::{MeshData, Meshes, chunk_meshes, far_mesh, shell_meshes};
use orbit_core::planet::{ChunkKey, Customization, Planet, PlanetSpec};
use orbit_core::player::{Located, Well};
use orbit_core::universe::{StarSystem, Universe};
use std::sync::{Arc, RwLock, RwLockReadGuard};

/// Physics' view of the planets `Space::voxels` returned.
pub fn located<'a>(voxels: &'a [(RwLockReadGuard<'_, Planet>, DVec3, Quat)]) -> Vec<Located<'a>> {
    voxels
        .iter()
        .map(|(p, center, rotation)| Located {
            planet: p,
            center: *center,
            rotation: *rotation,
        })
        .collect()
}

/// Sectors around the player whose stars are drawn.
const STAR_RANGE: i32 = 1;
/// Systems whose star is closer than this get their planets: all of a system you are in (they
/// reach about 120 000 blocks from their star) and none of its neighbors (240 000 away at least).
const DETAIL_DIST: f32 = 150_000.0;
/// Build voxels when this close to a planet's gravity edge; drop them (if unedited) at twice that.
const VOXEL_LOAD: f32 = 600.0;
/// Chunks whose middle is this close to the camera are meshed in full.
const CHUNK_VIEW: f32 = 192.0;
/// Past that, out to the horizon but no further than this, they are shells.
const SHELL_VIEW_MAX: f32 = 1400.0;
/// Chunks go when this much past the view.
const CHUNK_SLACK: f32 = 40.0;
/// Chunks are shown while the camera is this close to a planet's atmosphere (orbits are well
/// inside); past it the far mesh takes over.
const CHUNK_RANGE: f32 = 350.0;
/// The far mesh is cut away up to this much short of the nearest chunk not meshed yet (chunks reach
/// past their middle sideways and up their mountains).
const HOLE_MARGIN: f32 = 40.0;
/// Within this distance of a planet's surface its detailed far mesh is used (and built); it falls
/// back to the coarse one past `DETAIL_DROP`.
const DETAIL_NEAR: f32 = 3000.0;
const DETAIL_DROP: f32 = 4000.0;
/// Coarse far mesh: columns per cube face side.
const COARSE_RES: usize = 40;
/// Chunk meshing jobs in flight per planet.
const MAX_JOBS: usize = 32;
/// While voxels are loaded the far mesh sinks this far below them, out of sight.
const FAR_SINK: f32 = 2.0;

pub struct WorldPlugin;

/// Floating origin. World positions are f64 (`DVec3`, exact to well under a millimeter anywhere
/// you could fly), but the GPU works in f32, so nothing is drawn at its world position: render
/// space is world space shifted by `at`, the center of the planet you are on or orbiting (else
/// your own eye). The camera and everything near it then sit at small, exact coordinates, so the
/// view never jitters, however far you go. `eye` is the camera's world position, for streaming.
#[derive(Resource, Default, Clone, Copy, Debug)]
pub struct Origin {
    pub at: DVec3,
    pub eye: DVec3,
}

impl Origin {
    /// A world point in render space.
    pub fn render(&self, world: DVec3) -> Vec3 {
        (world - self.at).as_vec3()
    }
}

/// Something drawn at a fixed world position (stars).
#[derive(Component)]
pub struct WorldPos(pub DVec3);

#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct MovePlanets;

/// False while no world has been started (the title screen is up): nothing streams in until one is.
#[derive(Resource, Clone, Copy)]
pub struct WorldLive(pub bool);

fn live(l: Res<WorldLive>) -> bool {
    l.0
}

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Clock>()
            .init_resource::<TimeShift>()
            .init_resource::<Origin>()
            .add_systems(PostUpdate, apply_origin.before(TransformSystems::Propagate))
            .add_systems(Startup, setup_assets)
            .add_systems(Update, move_planets.in_set(MovePlanets).run_if(live))
            .add_systems(
                Update,
                (
                    stream_systems,
                    stream_voxels,
                    finish_voxels,
                    flow_liquids,
                    stream_chunks,
                    aim_sunlight,
                )
                    .chain()
                    .after(MovePlanets)
                    .run_if(live),
            );
    }
}

/// The shared universe clock, seconds.
#[derive(Resource, Default)]
pub struct Clock(pub f64);

/// Seconds the clock has been moved on by (`/time`).
#[derive(Resource, Default)]
pub struct TimeShift(pub f64);

/// A generated planet, shared with the meshing jobs.
pub type Voxels = Arc<RwLock<Planet>>;

#[derive(Resource)]
pub struct Space {
    pub universe: Universe,
    pub planets: HashMap<u64, PlanetSlot>,
    systems: HashMap<IVec3, SystemSlot>,
    /// Players' changes to planets, kept while the planets stream in and out.
    pub custom: HashMap<u64, Customization>,
}

impl Space {
    /// What can pass between a planet (or moon) and its star: a planet's moons; a moon's planet
    /// and its other moon. Centers (world) and radii, up to two.
    pub fn eclipsers(&self, id: u64) -> Vec<(DVec3, f32)> {
        let Some(p) = self.planets.get(&id) else {
            return vec![];
        };
        let host = p.spec.host.map(|h| h.id);
        let mut v: Vec<(DVec3, f32)> = host
            .and_then(|h| self.planets.get(&h))
            .map(|h| (h.spec.center, h.spec.radius))
            .into_iter()
            .collect();
        // Moons of this planet, or the other moons of this moon's planet.
        let family = host.unwrap_or(id);
        v.extend(
            self.planets
                .values()
                .filter(|m| m.spec.id != id && m.spec.host.is_some_and(|h| h.id == family))
                .map(|m| (m.spec.center, m.spec.radius)),
        );
        v.truncate(2);
        v
    }

    pub fn new(seed: u64) -> Self {
        Self::with(seed, orbit_core::universe::WorldGen::default())
    }

    /// Despawns everything this space put in the scene (stars, planets and their chunks), for when
    /// it is about to be replaced by another world's: otherwise the old planets stay behind.
    pub fn despawn_all(&mut self, commands: &mut Commands) {
        for (_, slot) in self.systems.drain() {
            for e in slot.entities {
                commands.entity(e).despawn();
            }
        }
        for (_, p) in self.planets.drain() {
            commands.entity(p.root).despawn();
        }
    }

    pub fn with(seed: u64, make: orbit_core::universe::WorldGen) -> Self {
        Self {
            universe: Universe::with(seed, make),
            planets: HashMap::default(),
            systems: HashMap::default(),
            custom: HashMap::default(),
        }
    }

    /// Loaded voxel planets with where they are now, for physics (see `located`).
    pub fn voxels(&self) -> Vec<(RwLockReadGuard<'_, Planet>, DVec3, Quat)> {
        self.planets
            .values()
            .filter_map(|p| {
                Some((
                    p.voxel.as_ref()?.read().unwrap(),
                    p.spec.center,
                    p.spec.rotation,
                ))
            })
            .collect()
    }

    /// Every streamed planet's gravity well (orbits need no voxels).
    pub fn wells(&self) -> Vec<Well> {
        self.planets
            .values()
            .map(|p| Well {
                id: p.spec.id,
                center: p.spec.center,
                radius: p.spec.radius,
                pull: p.spec.pull(),
                vel: p.spec.velocity,
            })
            .collect()
    }

    /// A planet anywhere in the universe as it is at time `t`, customizations included: the
    /// streamed one if it is loaded, else worked out from the seed.
    pub fn planet_spec(&self, sector: IVec3, id: u64, t: f64) -> Option<PlanetSpec> {
        if let Some(p) = self.planets.get(&id) {
            return Some(p.spec.clone());
        }
        // The whole system: a moon goes round its planet as customized.
        let mut planets = self.universe.system(sector)?.planets;
        for p in &mut planets {
            if let Some(c) = self.custom.get(&p.id) {
                c.apply(p, t);
            }
        }
        orbit_core::planet::refit_moons(&mut planets, t);
        let mut spec = planets.into_iter().find(|p| p.id == id)?;
        spec.advance(t);
        Some(spec)
    }

    /// True if the point is inside a planet's gravity whose voxels are not ready yet.
    pub fn waiting_at(&self, pos: DVec3) -> bool {
        self.planets.values().any(|p| {
            p.voxel.is_none() && pos.distance(p.spec.center) < p.spec.gravity_radius() as f64
        })
    }
}

impl PlanetSlot {
    /// The material its chunks are drawn with (for things drawn among them: items lying about).
    pub fn material(&self) -> Handle<VoxelMaterial> {
        self.mat.clone()
    }

    /// Chunks currently represented by near voxel meshes. Decorative model props should only
    /// follow these cells; the voxel mesh remains the fallback outside this set.
    pub(crate) fn loaded_chunks(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        self.chunks.keys().copied()
    }
}

pub struct PlanetSlot {
    pub spec: PlanetSpec,
    pub root: Entity,
    far: Entity,
    core: Entity,
    /// Lights this planet's chunks from its star.
    mat: Handle<VoxelMaterial>,
    /// The same for the far mesh, which also cuts a hole where chunks are.
    far_mat: Handle<VoxelMaterial>,
    /// Its water, lit like the chunks.
    water_mat: Handle<LiquidMaterial>,
    /// Time not yet run through the liquids' ticks, seconds.
    flow_time: f32,
    pub star_color: [f32; 3],
    pub star_radius: f32,
    pub voxel: Option<Voxels>,
    pub edited: bool,
    task: Option<Task<Planet>>,
    pub chunks: HashMap<ChunkKey, (Lod, Entity)>,
    /// Meshing jobs, by chunk.
    jobs: HashMap<ChunkKey, (Lod, Task<Meshes>)>,
    /// Every chunk with a point near its middle on the surface, planet-local.
    chunk_centers: Vec<(ChunkKey, Vec3)>,
    /// Far meshes: coarse (always there) and detailed (built when near).
    coarse: (Handle<Mesh>, Aabb),
    detail: Option<(Handle<Mesh>, Aabb)>,
    detail_task: Option<Task<MeshData>>,
    showing_detail: bool,
    /// How far around the camera the far mesh is cut away (chunks stand there).
    hole: f32,
}

/// How much of a chunk is meshed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lod {
    /// Everything, caves too: near the camera.
    Full,
    /// Only what the sky lights: from afar.
    Shell,
}

fn mesh_job(v: &Voxels, k: ChunkKey, lod: Lod) -> Task<Meshes> {
    let v = v.clone();
    AsyncComputeTaskPool::get().spawn(async move {
        let p = v.read().unwrap();
        match lod {
            Lod::Full => chunk_meshes(&p, k),
            Lod::Shell => shell_meshes(&p, k),
        }
    })
}

struct SystemSlot {
    star: Option<(DVec3, [f32; 3])>,
    entities: Vec<Entity>,
    planets: Vec<u64>,
    detailed: bool,
}

#[derive(Resource)]
pub struct Assets3d {
    core_mat: Handle<StandardMaterial>,
    sphere: Handle<Mesh>,
}

#[derive(Component)]
pub struct Sun;

fn setup_assets(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let sphere = meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap());
    let c = orbit_core::block::color(orbit_core::block::CORE);
    let core_mat = mats.add(StandardMaterial {
        base_color: rgb(c),
        perceptual_roughness: 1.0,
        ..default()
    });
    commands.insert_resource(Assets3d { core_mat, sphere });
    commands.spawn((
        Sun,
        DirectionalLight {
            illuminance: 9000.0,
            ..default()
        },
        Transform::default(),
    ));
    commands.insert_resource(GlobalAmbientLight {
        color: Color::srgb(0.6, 0.65, 0.9),
        brightness: 60.0,
        ..default()
    }); // planets light themselves (lighting.rs)
}

/// Planet meshes live on the GPU only (they run to hundreds of MB), with bounds computed here: the
/// main world never holds the mesh to compute them from.
pub(crate) fn gpu_mesh(meshes: &mut Assets<Mesh>, m: MeshData) -> (Handle<Mesh>, Aabb) {
    let aabb = Aabb::enclosing(m.positions.iter().map(|&p| Vec3::from(p))).unwrap_or_default();
    let colors: Vec<[f32; 4]> = m
        .colors
        .into_iter()
        .map(|c| LinearRgba::from(Color::srgba(c[0], c[1], c[2], c[3])).to_f32_array())
        .collect();
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, m.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, m.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_COLOR, colors)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, m.uvs)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, m.lights)
    .with_inserted_indices(Indices::U32(m.indices));
    (meshes.add(mesh), aabb)
}

pub(crate) fn rgb(c: [f32; 3]) -> Color {
    Color::srgb(c[0], c[1], c[2])
}

/// Detailed far mesh resolution (columns per cube face side): two blocks a column, within limits.
fn detail_res(spec: &PlanetSpec) -> usize {
    ((std::f32::consts::FRAC_PI_4 * spec.radius) as usize).clamp(48, 200)
}

fn core_radius(spec: &PlanetSpec) -> f32 {
    spec.radius - spec.depth() as f32
}

/// Moves every planet along its orbit; time is the shared universe clock.
fn move_planets(
    mut space: ResMut<Space>,
    time: Res<Time>,
    mut clock: ResMut<Clock>,
    shift: Res<TimeShift>,
    mut offset: Local<Option<f64>>,
) {
    // Debug: ORBIT_TIME=<seconds> starts the universe clock later (another time of day).
    let start = *offset.get_or_insert_with(|| {
        std::env::var("ORBIT_TIME")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0)
    });
    let t = start + shift.0 + time.elapsed_secs_f64();
    clock.0 = t;
    for slot in space.planets.values_mut() {
        slot.spec.advance(t);
    }
}

/// Puts everything where it is in render space (see `Origin`), once the frame's origin is known.
pub(crate) fn apply_origin(
    space: Res<Space>,
    origin: Res<Origin>,
    mut roots: Query<&mut Transform, Without<WorldPos>>,
    mut placed: Query<(&WorldPos, &mut Transform)>,
    mut mats: ResMut<Assets<VoxelMaterial>>,
    mut water_mats: ResMut<Assets<LiquidMaterial>>,
    submerged: Res<crate::lighting::Submerged>,
) {
    for slot in space.planets.values() {
        let center = origin.render(slot.spec.center);
        for h in [&slot.mat, &slot.far_mat] {
            if let Some(mut m) = mats.get_mut(h) {
                m.place(center, origin.render(slot.spec.star()));
                let bodies: Vec<(Vec3, f32)> = space
                    .eclipsers(slot.spec.id)
                    .into_iter()
                    .map(|(c, r)| (origin.render(c), r))
                    .collect();
                m.eclipses(origin.render(slot.spec.star()), slot.star_radius, &bodies);
                m.submerge(*submerged);
            }
        }
        if let (Some(m), Some(mut w)) = (mats.get(&slot.mat), water_mats.get_mut(&slot.water_mat)) {
            w.follow(m);
        }
        if let Some(mut m) = mats.get_mut(&slot.far_mat) {
            m.cut(origin.render(origin.eye), slot.hole);
        }
        if let Ok(mut tr) = roots.get_mut(slot.root) {
            tr.translation = center;
            tr.rotation = slot.spec.rotation;
        }
    }
    for (w, mut tr) in &mut placed {
        tr.translation = origin.render(w.0);
    }
}

fn stream_systems(
    mut commands: Commands,
    mut space: ResMut<Space>,
    clock: Res<Clock>,
    assets: Res<Assets3d>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut voxel_mats: ResMut<Assets<VoxelMaterial>>,
    mut water_mats: ResMut<Assets<LiquidMaterial>>,
    atlas: Res<Atlas>,
    origin: Res<Origin>,
) {
    let eye = origin.eye;
    let here = Universe::sector_of(eye);
    let space = &mut *space;

    // Drop systems out of range, and any whose level of detail no longer fits (rebuilt below).
    space.systems.retain(|s, slot| {
        let in_range = (*s - here).abs().max_element() <= STAR_RANGE;
        let wants_detail = slot
            .star
            .is_some_and(|(c, _)| c.distance(eye) < DETAIL_DIST as f64);
        if in_range && slot.detailed == wants_detail {
            return true;
        }
        for e in slot.entities.drain(..) {
            commands.entity(e).despawn();
        }
        for id in slot.planets.drain(..) {
            if space.planets.get(&id).is_some_and(|p| !p.edited) {
                let p = space.planets.remove(&id).unwrap();
                commands.entity(p.root).despawn();
            }
        }
        false
    });

    for x in -STAR_RANGE..=STAR_RANGE {
        for y in -STAR_RANGE..=STAR_RANGE {
            for z in -STAR_RANGE..=STAR_RANGE {
                let s = here + IVec3::new(x, y, z);
                if space.systems.contains_key(&s) {
                    continue;
                }
                let slot = match space.universe.system(s) {
                    Some(mut sys) => {
                        for p in &mut sys.planets {
                            if let Some(c) = space.custom.get(&p.id) {
                                c.apply(p, clock.0);
                            }
                        }
                        orbit_core::planet::refit_moons(&mut sys.planets, clock.0);
                        let detailed = sys.center.distance(eye) < DETAIL_DIST as f64;
                        spawn_system(
                            &mut commands,
                            &mut space.planets,
                            &assets,
                            &atlas,
                            &mut meshes,
                            &mut mats,
                            (&mut voxel_mats, &mut water_mats),
                            sys,
                            detailed,
                        )
                    }
                    None => SystemSlot {
                        star: None,
                        entities: vec![],
                        planets: vec![],
                        detailed: false,
                    },
                };
                space.systems.insert(s, slot);
            }
        }
    }
}

fn spawn_system(
    commands: &mut Commands,
    planets: &mut HashMap<u64, PlanetSlot>,
    assets: &Assets3d,
    atlas: &Atlas,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    (voxel_mats, water_mats): (&mut Assets<VoxelMaterial>, &mut Assets<LiquidMaterial>),
    sys: StarSystem,
    detailed: bool,
) -> SystemSlot {
    // HDR color well above 1.0 so bloom makes it glow like a sun.
    let glow = LinearRgba::from(rgb(sys.star_color)) * 12.0;
    let star_mat = mats.add(StandardMaterial {
        base_color: Color::LinearRgba(glow),
        unlit: true,
        ..default()
    });
    let star = commands
        .spawn((
            Mesh3d(assets.sphere.clone()),
            MeshMaterial3d(star_mat),
            WorldPos(sys.center),
            Transform::from_scale(Vec3::splat(sys.star_radius)),
        ))
        .id();
    let mut slot = SystemSlot {
        star: Some((sys.center, sys.star_color)),
        entities: vec![star],
        planets: vec![],
        detailed,
    };
    if !detailed {
        return slot;
    }
    for spec in sys.planets {
        slot.planets.push(spec.id);
        if planets.contains_key(&spec.id) {
            continue; // kept alive because it was edited
        }
        let root = commands
            .spawn((
                Transform::from_rotation(spec.rotation),
                Visibility::default(),
            ))
            .id();
        // Solid core under the voxel layers: seals the planet (nothing behind it shows through
        // pixel-wide gaps) and is the floor when digging all the way down.
        let core = commands
            .spawn((
                Mesh3d(assets.sphere.clone()),
                MeshMaterial3d(assets.core_mat.clone()),
                Transform::from_scale(Vec3::splat(core_radius(&spec))),
                ChildOf(root),
            ))
            .id();
        let mut m = VoxelMaterial::new(sys.star_color, atlas.0.clone());
        m.place(Vec3::ZERO, (spec.star() - spec.center).as_vec3()); // placed for real in `apply_origin`
        let mat = voxel_mats.add(m.clone());
        let water_mat = water_mats.add(LiquidMaterial::new(&m));
        let far_mat = voxel_mats.add(m);
        let coarse = gpu_mesh(meshes, far_mesh(&spec, COARSE_RES));
        let far = commands
            .spawn((
                Mesh3d(coarse.0.clone()),
                coarse.1,
                MeshMaterial3d(far_mat.clone()),
                Transform::default(),
                ChildOf(root),
            ))
            .id();
        planets.insert(
            spec.id,
            PlanetSlot {
                spec,
                root,
                far,
                core,
                mat,
                far_mat,
                water_mat,
                flow_time: 0.0,
                star_color: sys.star_color,
                star_radius: sys.star_radius,
                voxel: None,
                edited: false,
                task: None,
                chunks: HashMap::default(),
                jobs: HashMap::default(),
                chunk_centers: Vec::new(),
                coarse,
                detail: None,
                detail_task: None,
                showing_detail: false,
                hole: 0.0,
            },
        );
    }
    slot
}

/// Drops a planet's chunk meshes (the voxels stay).
fn drop_chunks(commands: &mut Commands, slot: &mut PlanetSlot) {
    slot.jobs.clear();
    for (_, (_, e)) in slot.chunks.drain() {
        commands.entity(e).despawn();
    }
}

/// Drops a planet's voxels and chunks; the far mesh stands in until they are rebuilt.
fn unload(commands: &mut Commands, slot: &mut PlanetSlot) {
    slot.voxel = None;
    slot.task = None;
    slot.chunk_centers.clear();
    drop_chunks(commands, slot);
}

/// Gives a planet a new shape (after customizing it): new far mesh and core, and its voxels are
/// generated again (edits are lost).
pub fn reshape(
    commands: &mut Commands,
    slot: &mut PlanetSlot,
    meshes: &mut Assets<Mesh>,
    spec: PlanetSpec,
) {
    unload(commands, slot);
    slot.edited = false;
    slot.coarse = gpu_mesh(meshes, far_mesh(&spec, COARSE_RES));
    slot.detail = None;
    slot.detail_task = None;
    slot.showing_detail = false;
    commands
        .entity(slot.far)
        .insert((Mesh3d(slot.coarse.0.clone()), slot.coarse.1));
    commands
        .entity(slot.core)
        .insert(Transform::from_scale(Vec3::splat(core_radius(&spec))));
    slot.spec = spec;
}

fn stream_voxels(mut commands: Commands, mut space: ResMut<Space>, origin: Res<Origin>) {
    let eye = origin.eye;
    let pool = AsyncComputeTaskPool::get();
    for slot in space.planets.values_mut() {
        let d = eye.distance(slot.spec.center) as f32 - slot.spec.gravity_radius();
        if d < VOXEL_LOAD && slot.voxel.is_none() && slot.task.is_none() {
            let spec = slot.spec.clone();
            slot.task = Some(pool.spawn(async move { Planet::generate(spec) }));
        } else if d > VOXEL_LOAD * 2.0
            && (slot.voxel.is_some() || slot.task.is_some())
            && !slot.edited
        {
            unload(&mut commands, slot);
        }
    }
}

fn finish_voxels(mut space: ResMut<Space>) {
    for slot in space.planets.values_mut() {
        let Some(task) = slot.task.as_mut() else {
            continue;
        };
        let Some(planet) = block_on(future::poll_once(task)) else {
            continue;
        };
        slot.task = None;
        // Where it is now lives in `slot.spec`; the voxels' own spec frame is not kept up to date.
        let g = &planet.grid;
        let (size, half) = (orbit_core::planet::CHUNK, orbit_core::planet::CHUNK / 2);
        let surface = planet.spec.depth();
        slot.chunk_centers = planet
            .all_chunks()
            .map(|k| {
                let (i, j) = (
                    (k.ci as usize * size + half).min(g.n - 1),
                    (k.cj as usize * size + half).min(g.n - 1),
                );
                (k, g.center(g.index(k.face as usize, i, j, surface)))
            })
            .collect();
        slot.voxel = Some(Arc::new(RwLock::new(planet)));
    }
}

/// Picks each planet's level of detail: voxel chunks near the camera (meshed nearest first,
/// dropped when they fall behind or the planet is far), the far mesh everywhere else, cut away
/// where the chunks stand, detailed or coarse by distance.
fn stream_chunks(
    mut commands: Commands,
    mut space: ResMut<Space>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut far: Query<&mut Transform>,
    origin: Res<Origin>,
) {
    let eye = origin.eye;
    let pool = AsyncComputeTaskPool::get();
    for slot in space.planets.values_mut() {
        // Finished jobs.
        let done: Vec<ChunkKey> = slot
            .jobs
            .iter_mut()
            .filter_map(|(k, (_, t))| t.is_finished().then_some(*k))
            .collect();
        for k in done {
            let (lod, task) = slot.jobs.remove(&k).unwrap();
            let data = block_on(task);
            spawn_chunk(&mut commands, slot, &mut meshes, k, lod, data);
        }

        // Far mesh: detailed when near (built off the main thread), coarse when far.
        let altitude = eye.distance(slot.spec.center) as f32 - slot.spec.radius;
        if altitude < DETAIL_NEAR && slot.detail.is_none() && slot.detail_task.is_none() {
            let spec = slot.spec.clone();
            slot.detail_task = Some(pool.spawn(async move { far_mesh(&spec, detail_res(&spec)) }));
        }
        if let Some(task) = slot.detail_task.as_mut() {
            if let Some(data) = block_on(future::poll_once(task)) {
                slot.detail_task = None;
                slot.detail = Some(gpu_mesh(&mut meshes, data));
            }
        }
        if altitude > DETAIL_DROP {
            slot.detail = None;
            slot.detail_task = None;
        }
        let detail = altitude < DETAIL_DROP && slot.detail.is_some();
        if detail != slot.showing_detail {
            slot.showing_detail = detail;
            let (h, aabb) = if detail {
                slot.detail.clone().unwrap()
            } else {
                slot.coarse.clone()
            };
            commands.entity(slot.far).insert((Mesh3d(h), aabb));
        }

        // Chunks, only while the camera is near enough to the planet to see them.
        let near =
            (eye.distance(slot.spec.center) as f32) < slot.spec.gravity_radius() + CHUNK_RANGE;
        let mut hole: f32 = 0.0;
        match slot.voxel.clone() {
            Some(v) if near => {
                let local = slot.spec.to_local(eye);
                // Out to the horizon (and the mountains standing over it), at least the full view.
                let r = slot.spec.radius;
                let alt = (local.length() - r).max(0.0);
                let horizon = (2.0 * r * alt + alt * alt).sqrt() + slot.spec.max_height() + 48.0;
                let view = horizon.clamp(CHUNK_VIEW, SHELL_VIEW_MAX);
                let behind: Vec<ChunkKey> = slot
                    .chunk_centers
                    .iter()
                    .filter(|(_, c)| c.distance(local) > view + CHUNK_SLACK)
                    .map(|(k, _)| *k)
                    .collect();
                for k in behind {
                    if let Some((_, e)) = slot.chunks.remove(&k) {
                        commands.entity(e).despawn();
                    }
                    slot.jobs.remove(&k);
                }
                // Missing chunks, and shells near enough to be made whole. The far mesh is cut
                // out up to the nearest chunk in view with nothing meshed yet.
                let mut missing = Vec::new();
                let mut gap = view;
                for (k, c) in &slot.chunk_centers {
                    let d = c.distance(local);
                    if d >= view {
                        continue;
                    }
                    let want = if d < CHUNK_VIEW {
                        Lod::Full
                    } else {
                        Lod::Shell
                    };
                    let have = slot.chunks.get(k).map(|c| c.0);
                    if have.is_none() {
                        gap = gap.min(d);
                    }
                    let needed = have.is_none() || (want == Lod::Full && have == Some(Lod::Shell));
                    let queued = slot
                        .jobs
                        .get(k)
                        .is_some_and(|j| j.0 == want || j.0 == Lod::Full);
                    if needed && !queued {
                        missing.push((d, *k, want));
                    }
                }
                hole = (gap - HOLE_MARGIN).max(0.0);
                missing.sort_by(|a, b| a.0.total_cmp(&b.0));
                let free = MAX_JOBS.saturating_sub(slot.jobs.len());
                for (_, k, lod) in missing.into_iter().take(free) {
                    slot.jobs.insert(k, (lod, mesh_job(&v, k, lod)));
                }
            }
            _ if !slot.chunks.is_empty() || !slot.jobs.is_empty() => {
                drop_chunks(&mut commands, slot)
            }
            _ => {}
        }

        slot.hole = hole;
        if let Ok(mut t) = far.get_mut(slot.far) {
            // Under the chunks, out of sight, while there are any.
            let s = if slot.chunks.is_empty() {
                1.0
            } else {
                (slot.spec.radius - FAR_SINK) / slot.spec.radius
            };
            if t.scale.x != s {
                t.scale = Vec3::splat(s);
            }
        }
    }
}

fn spawn_chunk(
    commands: &mut Commands,
    slot: &mut PlanetSlot,
    meshes: &mut Assets<Mesh>,
    key: ChunkKey,
    lod: Lod,
    data: Meshes,
) {
    if let Some((_, old)) = slot.chunks.remove(&key) {
        commands.entity(old).despawn();
    }
    // An empty chunk still counts as loaded (an entity with no mesh), so it is not asked for again.
    let mut e = commands.spawn((
        Transform::default(),
        Visibility::default(),
        ChildOf(slot.root),
    ));
    if !data.solid.is_empty() {
        let (mesh, aabb) = gpu_mesh(meshes, data.solid);
        e.insert((Mesh3d(mesh), aabb, MeshMaterial3d(slot.mat.clone())));
    }
    let id = e.id();
    // Its water, a child drawn see-through.
    if !data.water.is_empty() {
        let (mesh, aabb) = gpu_mesh(meshes, data.water);
        commands.spawn((
            Mesh3d(mesh),
            aabb,
            MeshMaterial3d(slot.water_mat.clone()),
            Transform::default(),
            Visibility::default(),
            ChildOf(id),
        ));
    }
    slot.chunks.insert(key, (lod, id));
}

/// Lets water and lava flow on the loaded planets, `liquid::TICK` at a time, and meshes what
/// they change again.
fn flow_liquids(mut space: ResMut<Space>, time: Res<Time>) {
    for slot in space.planets.values_mut() {
        let Some(v) = slot.voxel.clone() else {
            continue;
        };
        slot.flow_time = (slot.flow_time + time.delta_secs()).min(orbit_core::liquid::TICK * 4.0);
        if v.read().unwrap().flowing() == 0 {
            slot.flow_time = 0.0;
            continue;
        }
        // Not while meshing jobs read it: the main thread would wait on them. Next frame, then.
        let Ok(mut planet) = v.try_write() else {
            continue;
        };
        let mut dirty: Vec<ChunkKey> = Vec::new();
        while slot.flow_time >= orbit_core::liquid::TICK {
            slot.flow_time -= orbit_core::liquid::TICK;
            for k in planet.tick_liquids() {
                if !dirty.contains(&k) {
                    dirty.push(k);
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

/// Rebuilds the given chunks of a planet after an edit (off the main thread; the old meshes stay
/// up until the new ones are ready). Chunks not loaded are left for streaming.
pub fn remesh(slot: &mut PlanetSlot, keys: &[ChunkKey]) {
    let Some(v) = slot.voxel.clone() else { return };
    for &k in keys {
        // At the detail it has (or is being built at); replacing a job drops the stale one.
        if let Some(lod) = slot
            .jobs
            .get(&k)
            .map(|j| j.0)
            .or(slot.chunks.get(&k).map(|c| c.0))
        {
            slot.jobs.insert(k, (lod, mesh_job(&v, k, lod)));
        }
    }
}

/// Points the sunlight from the nearest star toward the player.
fn aim_sunlight(
    space: Res<Space>,
    origin: Res<Origin>,
    mut sun: Query<(&mut Transform, &mut DirectionalLight), With<Sun>>,
) {
    let Ok((mut t, mut light)) = sun.single_mut() else {
        return;
    };
    let eye = origin.eye;
    let near = space
        .systems
        .values()
        .filter_map(|s| s.star)
        .min_by(|a, b| a.0.distance(eye).total_cmp(&b.0.distance(eye)));
    if let Some((center, color)) = near {
        let dir = (eye - center).as_vec3().normalize_or(Vec3::Z);
        *t = Transform::from_translation(origin.render(center))
            .looking_to(dir, dir.any_orthonormal_vector());
        light.color = Color::WHITE.mix(&rgb(color), 0.35);
    }
}
