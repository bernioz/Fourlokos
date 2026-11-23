//! Space stations in the game: where they are (each goes round its planet), drawing them,
//! breaking and placing blocks on them, and the core's menu. The stations themselves (blocks,
//! growth, orbit, light) are `orbit_core::station`; the player stands on them through
//! `orbit_core::player`'s station frame. They are put up, listed, packed and brought back with
//! commands (`/station`, see `commands`), the way GalaxyCraft does it with its items.
//!
//! Right click on the core opens its menu (name, size, Pack up); crouching (Shift) and right
//! clicking it places a block instead. The core cannot be broken: it holds the station together.
//! Stations last for the session, like customized planets.

use crate::chat::{ActionBar, Keyboard, Owner, TextField};
use crate::lighting::{LiquidMaterial, VoxelMaterial};
use crate::player::Player;
use crate::textures::Atlas;
use crate::world::{gpu_mesh, Clock, Origin, Space};
use bevy::math::DVec3;
use bevy::picking::hover::Hovered;
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, AsyncComputeTaskPool, Task};
use bevy::ui::Pressed;
use bevy::ui_widgets::{Activate, Button};
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block;
use orbit_core::mesh::{station_meshes, Meshes};
use orbit_core::player::{Body, Pull, StationAt, REACH};
use orbit_core::rng::mix;
use orbit_core::station::{self, find_slot, Lanes, Refusal, Station, StationOrbit, AHEAD, BELOW};
use orbit_core::travel::circular_speed;
use std::sync::{Arc, RwLock, RwLockReadGuard};

pub struct StationsPlugin;

impl Plugin for StationsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Stations>()
            .init_resource::<StationMenu>()
            .add_observer(on_activate)
            .add_systems(PostUpdate, place_stations.before(TransformSystems::Propagate))
            .add_systems(
                Update,
                (mesh_stations, interact_station, menu_keys, run_menu, refresh_menu, style_buttons, draw_shells)
                    .chain()
                    .after(crate::player::sync_camera),
            );
    }
}

/// Every station of the session, placed or packed.
#[derive(Resource, Default)]
pub struct Stations {
    pub list: Vec<Slot>,
    /// The ribbons of the space elevators standing now: planet and the radii each spans (from
    /// the ground to its counterweight). The lanes they cross take no stations (see `lanes_of`).
    pub tethers: Vec<(u64, f32, f32)>,
}

pub struct Slot {
    pub station: Arc<RwLock<Station>>,
    entity: Option<Entity>,
    mat: Option<Handle<VoxelMaterial>>,
    /// Its water: a child of its entity, and the material that draws it.
    water: Option<(Entity, Handle<LiquidMaterial>)>,
    job: Option<Task<Meshes>>,
    /// Time not yet run through the liquids' ticks, seconds.
    flow_time: f32,
    /// Its blocks changed since the mesh was made.
    dirty: bool,
}

impl Slot {
    /// The entity it is drawn as, and its material (for things drawn on it: items lying about).
    pub fn entity(&self) -> Option<Entity> {
        self.entity
    }
    pub fn material(&self) -> Option<Handle<VoxelMaterial>> {
        self.mat.clone()
    }
}

/// Where a station is now, and how it moves.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    pub center: DVec3,
    pub rotation: Quat,
    pub planet: u64,
    pub planet_center: DVec3,
    pub spin: Vec3,
}

impl Frame {
    pub fn at<'a>(&self, station: &'a Station) -> StationAt<'a> {
        StationAt { station, center: self.center, rotation: self.rotation, planet: self.planet, planet_center: self.planet_center, spin: self.spin }
    }
}

/// Physics' view of the stations `Stations::placed` returned.
pub fn located<'a>(v: &'a [(RwLockReadGuard<'_, Station>, Frame)]) -> Vec<StationAt<'a>> {
    v.iter().map(|(s, f)| f.at(s)).collect()
}

/// Where a new station would go: in front of the player and a little below, its floor toward
/// the planet, its front the way they look.
pub struct Spot {
    pub planet: u64,
    /// The station's origin relative to the planet's center, world-oriented.
    pub offset: Vec3,
    pub rotation: Quat,
    /// Which way the player goes round the planet.
    pub heading: Vec3,
}

impl Stations {
    /// A station's frame at time `t`, if it is placed around a planet that is streamed in, or a
    /// rocket's in flight (wherever it is). A rocket parking eases from where it let go into its
    /// orbit.
    pub fn frame(s: &Station, space: &Space, t: f64) -> Option<Frame> {
        if let Some(f) = s.flight {
            let planet_center = space.planets.get(&f.planet).map_or(f.center, |p| p.spec.center);
            return Some(Frame { center: f.center, rotation: f.rotation, planet: f.planet, planet_center, spin: Vec3::ZERO });
        }
        let o = s.orbit?;
        let p = space.planets.get(&o.planet)?;
        let (mut center, mut rotation) = (o.offset(t), o.rotation(t));
        if let Some(k) = s.glide.and_then(|g| g.progress(t).map(|k| (g, k))) {
            let (g, k) = k;
            center = g.from.lerp(center, k);
            rotation = g.turn.slerp(rotation, k as f32);
        }
        Some(Frame { center: p.spec.center + center, rotation, planet: o.planet, planet_center: p.spec.center, spin: o.angular_velocity() })
    }

    /// Placed stations near enough to matter (their planet streamed in), read-locked, with their frames.
    pub fn placed(&self, space: &Space, t: f64) -> Vec<(RwLockReadGuard<'_, Station>, Frame)> {
        self.list
            .iter()
            .filter_map(|slot| {
                let s = slot.station.read().unwrap();
                let f = Self::frame(&s, space, t)?;
                Some((s, f))
            })
            .collect()
    }

    /// The station with this id, or else this name (case-insensitive).
    pub fn find(&self, key: &str) -> Option<usize> {
        let key = key.trim();
        self.list.iter().position(|s| s.station.read().unwrap().id == key).or_else(|| {
            self.list.iter().position(|s| s.station.read().unwrap().name.eq_ignore_ascii_case(key))
        })
    }

    pub fn by_id(&self, id: &str) -> Option<&Slot> {
        self.list.iter().find(|s| s.station.read().unwrap().id == id)
    }

    /// The station the body is on, else the nearest placed one within `range` blocks.
    pub fn near(&self, body: &Body, space: &Space, t: f64, range: f64) -> Option<usize> {
        if let Some(i) = body.station.as_ref().and_then(|id| self.find(id)) {
            return Some(i);
        }
        let at = body.world_point(body.pos);
        self.list
            .iter()
            .enumerate()
            .filter_map(|(i, slot)| {
                let s = slot.station.read().unwrap();
                let f = Self::frame(&s, space, t)?;
                Some((i, f.center.distance(at)))
            })
            .filter(|&(_, d)| d < range)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| i)
    }

    /// Ids of the stations, placed or packed, for suggestions.
    pub fn ids(&self, placed: Option<bool>) -> Vec<String> {
        self.list
            .iter()
            .map(|s| s.station.read().unwrap())
            .filter(|s| placed.is_none_or(|p| s.orbit.is_some() == p))
            .map(|s| s.id.clone())
            .collect()
    }

    /// The spot in front of the player, or why a station cannot go up from where they are.
    pub fn spot(body: &Body, space: &Space) -> Result<Spot, Refusal> {
        if body.gravity_source.is_some() {
            return Err(Refusal::NotInOrbit);
        }
        let id = body.planet().ok_or(Refusal::NotInOrbit)?;
        let p = space.planets.get(&id).ok_or(Refusal::NotInOrbit)?;
        let feet = body.point_from(body.pos, p.spec.center);
        let up = feet.normalize();
        let look = body.world_dir(body.look_dir());
        let forward = look.reject_from(up).try_normalize().unwrap_or_else(|| up.any_orthonormal_vector());
        let right = up.cross(forward);
        let rotation = Quat::from_mat3(&Mat3::from_cols(right, up, forward)).normalize();
        let core = feet + forward * AHEAD - up * BELOW;
        Ok(Spot {
            planet: id,
            offset: core - rotation * Vec3::splat(0.5),
            rotation,
            heading: body.world_vel(),
        })
    }

    /// The orbits of the placed stations around a planet, but station `except`.
    pub fn orbits_around(&self, planet: u64, except: Option<usize>) -> Vec<StationOrbit> {
        self.list.iter().enumerate().filter(|&(i, _)| Some(i) != except).filter_map(|(_, s)| s.station.read().unwrap().orbit).filter(|o| o.planet == planet).collect()
    }

    /// Puts a station up in front of the player: a new one (`None`), or a packed one again. It
    /// goes into the nearest lane with room, as near ahead or behind as it fits. Returns its
    /// index, or why it cannot go up.
    pub fn deploy(&mut self, which: Option<usize>, name: Option<&str>, body: &Body, space: &Space, t: f64) -> Result<usize, Refusal> {
        let spot = Self::spot(body, space)?;
        let (lanes, pull) = lanes_of(space, spot.planet, &self.tethers).ok_or(Refusal::NotInOrbit)?;
        let want = StationOrbit::through(spot.planet, spot.offset, spot.rotation, spot.heading, pull, t);
        let orbit = find_slot(&want, &lanes, &self.orbits_around(spot.planet, which), coasting(pull), None)?;
        let i = match which {
            Some(i) => i,
            None => {
                let id = loop {
                    let id = Station::new_id(mix(t.to_bits() ^ self.list.len() as u64 ^ 0x57A7));
                    if self.find(&id).is_none() {
                        break id;
                    }
                };
                let s = Station::new(id, name.unwrap_or(""), block::SMOOTH_STONE);
                self.list.push(Slot { station: Arc::new(RwLock::new(s)), entity: None, mat: None, water: None, job: None, flow_time: 0.0, dirty: true });
                self.list.len() - 1
            }
        };
        let slot = &mut self.list[i];
        slot.station.write().unwrap().orbit = Some(orbit);
        slot.dirty = true;
        Ok(i)
    }

    /// Moves a placed station to another lane: the one with room nearest `altitude` (blocks over
    /// the planet's mean surface), or with `None` the synchronous one, over the equator, where it
    /// keeps over one spot of the ground. Players on it go with it.
    pub fn move_to(&mut self, i: usize, altitude: Option<f32>, space: &Space, t: f64) -> Result<StationOrbit, Refusal> {
        let o = self.list[i].station.read().unwrap().orbit.ok_or(Refusal::NotInOrbit)?.rebased(t);
        let p = space.planets.get(&o.planet).ok_or(Refusal::NotInOrbit)?;
        let (lanes, pull) = lanes_of(space, o.planet, &self.tethers).ok_or(Refusal::NotInOrbit)?;
        let others = self.orbits_around(o.planet, Some(i));
        let found = match altitude {
            Some(a) => {
                let r = p.spec.radius + a;
                find_slot(&o.raised(r, coasting(pull)(r)), &lanes, &others, coasting(pull), None)?
            }
            None => find_slot(&o, &lanes, &others, coasting(pull), Some((p.spec.spin.axis.as_dvec3(), p.spec.spin.speed as f64)))?,
        };
        self.list[i].station.write().unwrap().orbit = Some(found);
        Ok(found)
    }

    /// Adds a station built elsewhere (from a schematic), packed, with an id no other has.
    /// Returns its index.
    pub fn add(&mut self, schematic: &orbit_core::schematic::Schematic, name: Option<&str>, t: f64) -> usize {
        let id = loop {
            let id = Station::new_id(mix(t.to_bits() ^ self.list.len() as u64 ^ 0x5C4E));
            if self.find(&id).is_none() {
                break id;
            }
        };
        self.push(Station::from_schematic(id, name, schematic))
    }

    /// Adds a station as it is (loaded from a save). Returns its index.
    pub fn push(&mut self, s: Station) -> usize {
        self.list.push(Slot { station: Arc::new(RwLock::new(s)), entity: None, mat: None, water: None, job: None, flow_time: 0.0, dirty: true });
        self.list.len() - 1
    }

    /// Parks station (rocket) `i` in the free place nearest `want` (an orbit as it would have it),
    /// in its lanes, the way a new station goes up. Returns the orbit.
    pub fn park(&mut self, i: usize, want: &StationOrbit, space: &Space) -> Result<StationOrbit, Refusal> {
        let (lanes, pull) = lanes_of(space, want.planet, &self.tethers).ok_or(Refusal::NotInOrbit)?;
        let orbit = find_slot(want, &lanes, &self.orbits_around(want.planet, Some(i)), coasting(pull), None)?;
        self.list[i].station.write().unwrap().orbit = Some(orbit);
        self.list[i].dirty = true;
        Ok(orbit)
    }

    /// Takes station `i` out of the session (a rocket landing becomes ground again). Returns the
    /// entity that drew it, to despawn.
    pub fn remove(&mut self, i: usize) -> Option<Entity> {
        self.list.remove(i).entity
    }

    /// Forgets the last station added (one that could not go up).
    pub fn drop_last(&mut self) {
        self.list.pop();
    }

    /// Packs a station up: out of its orbit, kept with everything on it until it is restored.
    pub fn pack(&mut self, i: usize) {
        let slot = &mut self.list[i];
        slot.station.write().unwrap().orbit = None;
    }

    /// The entity drawing a station in the world (its mesh), if it is drawn.
    pub fn drawn(&self, i: usize) -> Option<Entity> {
        self.list[i].entity
    }

    pub fn mark_dirty(&mut self, i: usize) {
        self.list[i].dirty = true;
    }
}

/// The rate (rad/s) a body coasts round at radius `r` of a planet (or moon) with this pull.
pub fn coasting(pull: Pull) -> impl Fn(f32) -> f64 {
    move |r| (circular_speed(r, pull) / r) as f64
}

/// The station lanes round a planet or moon (streamed in), and its pull. A planet's skip the
/// heights its moons sweep, with their spheres of influence, and those its elevators' ribbons
/// cross (`tethers`, see `Stations::tethers`) but the synchronous one.
pub fn lanes_of(space: &Space, planet: u64, tethers: &[(u64, f32, f32)]) -> Option<(Lanes, Pull)> {
    let p = space.planets.get(&planet)?;
    let pull = p.spec.pull();
    let bands: Vec<(f32, f32)> = tethers.iter().filter(|t| t.0 == planet).map(|t| (t.1, t.2)).collect();
    Some((Lanes::new(pull, p.spec.spin.speed, &moon_bands(space, planet)).without(&bands), pull))
}

/// The heights a planet's moons sweep, with their spheres of influence, in any plane.
pub fn moon_bands(space: &Space, planet: u64) -> Vec<(f32, f32)> {
    space.planets.values().filter_map(|m| m.spec.host.filter(|h| h.id == planet).map(|h| (m.spec.orbit.radius - h.soi, m.spec.orbit.radius + h.soi))).collect()
}

/// Builds station meshes off the main thread (the old mesh stays up meanwhile); takes down the
/// entities of packed stations and those whose planet streamed out.
fn mesh_stations(
    mut commands: Commands,
    mut stations: ResMut<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<VoxelMaterial>>,
    mut water_mats: ResMut<Assets<LiquidMaterial>>,
    atlas: Res<Atlas>,
    time: Res<Time>,
) {
    let pool = AsyncComputeTaskPool::get();
    for slot in &mut stations.list {
        let frame = Stations::frame(&slot.station.read().unwrap(), &space, clock.0);
        let Some(frame) = frame else {
            if let Some(e) = slot.entity.take() {
                commands.entity(e).despawn();
            }
            slot.water = None;
            slot.job = None;
            slot.dirty = true;
            continue;
        };
        if slot.entity.is_none() {
            let star = space.planets.get(&frame.planet).map_or([1.0; 3], |p| p.star_color);
            let m = VoxelMaterial::new(star, atlas.0.clone());
            let water_mat = water_mats.add(LiquidMaterial::new(&m));
            let mat = mats.add(m);
            let e = commands.spawn((Transform::default(), Visibility::default(), MeshMaterial3d(mat.clone()))).id();
            let water = commands.spawn((Transform::default(), Visibility::default(), MeshMaterial3d(water_mat.clone()), ChildOf(e))).id();
            slot.entity = Some(e);
            slot.water = Some((water, water_mat));
            slot.mat = Some(mat);
        }
        // Its water and lava flow.
        slot.flow_time = (slot.flow_time + time.delta_secs()).min(orbit_core::liquid::TICK * 4.0);
        while slot.flow_time >= orbit_core::liquid::TICK {
            slot.flow_time -= orbit_core::liquid::TICK;
            let Ok(mut s) = slot.station.try_write() else { break };
            if s.tick_liquids() {
                slot.dirty = true;
            }
        }
        if let Some(task) = slot.job.as_mut() {
            if let Some(data) = block_on(future::poll_once(task)) {
                slot.job = None;
                let e = slot.entity.unwrap();
                let water = slot.water.as_ref().unwrap().0;
                for (e, data) in [(e, data.solid), (water, data.water)] {
                    if data.is_empty() {
                        commands.entity(e).remove::<Mesh3d>();
                    } else {
                        let (mesh, aabb) = gpu_mesh(&mut meshes, data);
                        commands.entity(e).insert((Mesh3d(mesh), aabb));
                    }
                }
            }
        }
        if slot.dirty && slot.job.is_none() {
            slot.dirty = false;
            let s = slot.station.clone();
            slot.job = Some(pool.spawn(async move { station_meshes(&s.read().unwrap()) }));
        }
    }
}

/// Puts each station where it is in render space, and lights it from its star, dimming it
/// smoothly into and out of its planet's shadow.
#[allow(clippy::too_many_arguments)]
fn place_stations(
    stations: Res<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    origin: Res<Origin>,
    mut tr: Query<&mut Transform>,
    mut mats: ResMut<Assets<VoxelMaterial>>,
    mut water_mats: ResMut<Assets<LiquidMaterial>>,
    submerged: Res<crate::lighting::Submerged>,
) {
    for slot in &stations.list {
        let (Some(e), Some(mat)) = (slot.entity, slot.mat.as_ref()) else { continue };
        let Some(f) = Stations::frame(&slot.station.read().unwrap(), &space, clock.0) else { continue };
        let at = origin.render(f.center);
        if let Ok(mut t) = tr.get_mut(e) {
            t.translation = at;
            t.rotation = f.rotation;
        }
        let Some(p) = space.planets.get(&f.planet) else { continue };
        let sun = (p.spec.star() - f.center).as_vec3().normalize_or(Vec3::Y);
        let rel = (f.center - p.spec.center).as_vec3();
        if let Some(mut m) = mats.get_mut(mat) {
            m.place_flat(at, sun, station::sun_height(rel, sun, p.spec.radius));
            // Its planet's moons (or its moon's planet) can eclipse it too.
            let bodies: Vec<(Vec3, f32)> = space.eclipsers(f.planet).into_iter().map(|(c, r)| (origin.render(c), r)).collect();
            m.eclipses(origin.render(p.spec.star()), p.star_radius, &bodies);
            m.submerge(*submerged);
        }
        if let (Some(m), Some((_, w))) = (mats.get(mat), slot.water.as_ref()) {
            if let Some(mut w) = water_mats.get_mut(w) {
                w.follow(m);
            }
        }
    }
}

/// The edges of a box (local corners `lo`, `hi`) on a station, in render space.
fn box_lines(gizmos: &mut Gizmos, base: Vec3, rotation: Quat, lo: Vec3, hi: Vec3, color: Color) {
    let c = |x: bool, y: bool, z: bool| base + rotation * Vec3::new(if x { hi.x } else { lo.x }, if y { hi.y } else { lo.y }, if z { hi.z } else { lo.z });
    for (a, b) in [((false, false), (true, false)), ((true, false), (true, true)), ((true, true), (false, true)), ((false, true), (false, false))] {
        for y in [false, true] {
            gizmos.line(c(a.0, y, a.1), c(b.0, y, b.1), color);
        }
        gizmos.line(c(a.0, false, a.1), c(a.0, true, a.1), color);
    }
}

/// Faint edges of each nearby station's gravity, so you can see where it holds you.
fn draw_shells(stations: Res<Stations>, space: Res<Space>, clock: Res<Clock>, origin: Res<Origin>, mut gizmos: Gizmos) {
    for (s, f) in stations.placed(&space, clock.0) {
        if s.flight.is_some() || f.center.distance(origin.eye) > 300.0 {
            continue;
        }
        let (lo, hi) = s.gravity_box();
        box_lines(&mut gizmos, origin.render(f.center), f.rotation, lo, hi, Color::srgba(0.45, 0.8, 1.0, 0.12));
    }
}

/// Breaking and placing blocks on the station you are on, and opening its core's menu.
#[allow(clippy::too_many_arguments)]
fn interact_station(
    mut player: Single<&mut Player>,
    mut drops: ResMut<crate::inventory::Drops>,
    mut stations: ResMut<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    origin: Res<Origin>,
    mut bar: ResMut<ActionBar>,
    mut menu: ResMut<StationMenu>,
    keyboard: Res<Keyboard>,
    mut gizmos: Gizmos,
    mut rockets: ResMut<crate::rocket::Rockets>,
) {
    let Some(i) = player.body.station.as_ref().and_then(|id| stations.find(id)) else { return };
    if menu.open.is_some() || keyboard.owner.is_some() {
        return;
    }
    let slot = &stations.list[i];
    let Some(frame) = Stations::frame(&slot.station.read().unwrap(), &space, clock.0) else { return };
    // The body is in the station's frame: eye and look are already station coordinates.
    let hit = slot.station.read().unwrap().raycast(player.body.eye(), player.body.look_dir(), REACH);
    let Some(hit) = hit else { return };
    // Outline what is looked at: its own shape (a door's board, a torch, a flower).
    {
        let s = slot.station.read().unwrap();
        let cell = hit.cell.as_vec3();
        for x in orbit_core::shape::outline(s.get(hit.cell), orbit_core::shape::joined(&*s, hit.cell)).iter() {
            let (lo, hi) = (cell + Vec3::new(x[0], x[2], x[1]) - Vec3::splat(0.002), cell + Vec3::new(x[3], x[5], x[4]) + Vec3::splat(0.002));
            box_lines(&mut gizmos, origin.render(frame.center), frame.rotation, lo, hi, Color::BLACK);
        }
    }

    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    let crouching = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let mut s = slot.station.write().unwrap();
    if mouse.just_pressed(MouseButton::Middle) {
        let b = s.get(hit.cell);
        player.inv.pick(b);
        return;
    }
    let survival = player.inv.mode == orbit_core::inventory::GameMode::Survival;
    if mouse.just_pressed(MouseButton::Left) {
        if s.is_core(hit.cell) {
            bar.show(if s.is_rocket() { "The helm flies the rocket: land it to take it apart" } else { "The core holds the station together: pack it up from its menu" });
            return;
        }
        if !block::breakable(s.get(hit.cell)) {
            return;
        }
        let was = s.get(hit.cell);
        let gone = s.break_block(hit.cell);
        if survival {
            drops.broke_on_station(&s.id, &gone);
        }
        pads(&mut s, hit.cell, was, &mut bar);
    } else if mouse.just_pressed(MouseButton::Right) {
        // A rocket's helm standing on the station: it goes up from here.
        if !s.is_rocket() && s.get(hit.cell) == block::HELM && !crouching {
            rockets.requests.push(crate::rocket::Request::AssembleOnStation { station: s.id.clone(), cell: hit.cell });
            return;
        }
        if s.is_core(hit.cell) && !crouching {
            if s.is_rocket() {
                rockets.requests.push(crate::rocket::Request::Take(s.id.clone()));
            } else {
                menu.wanted = Some(s.id.clone());
            }
            return;
        }
        // A door or trapdoor opens or shuts.
        if block::opens(s.get(hit.cell)) && !crouching {
            for (c, b) in orbit_core::shape::toggle(&*s, hit.cell) {
                s.set(c, b);
            }
            drop(s);
            stations.mark_dirty(i);
            return;
        }
        let Some(at) = hit.before else { return };
        let Some(held) = player.inv.held() else { return };
        let d = hit.cell - at;
        use orbit_core::shape::Against;
        let against = match (d.x, d.y, d.z) {
            (-1, 0, 0) => Against::Side(block::Dir(0)),
            (1, 0, 0) => Against::Side(block::Dir(1)),
            (0, 0, -1) => Against::Side(block::Dir(2)),
            (0, 0, 1) => Against::Side(block::Dir(3)),
            (0, 1, 0) => Against::Above,
            _ => Against::Below,
        };
        let look = player.body.look_dir();
        let look_dir = if look.x.abs() > look.z.abs() { block::Dir(if look.x < 0.0 { 0 } else { 1 }) } else { block::Dir(if look.z < 0.0 { 2 } else { 3 }) };
        let high = hit.point.y - hit.cell.y as f32 > 0.5;
        let Some(puts) = orbit_core::shape::place(&*s, at, held.item, against, look_dir, high) else { return };
        if puts.iter().any(|&(c, _)| !Station::in_limits(c)) {
            bar.show("The station can't grow further");
            return;
        }
        let before: Vec<_> = puts.iter().map(|&(c, _)| (c, s.get(c))).collect();
        for &(c, b) in &puts {
            s.set(c, b);
        }
        if orbit_core::player::collides_flat(&s, player.body.pos) {
            for (c, b) in before {
                s.set(c, b); // would trap the player
            }
            return;
        }
        player.inv.use_held();
        pads(&mut s, at, block::AIR, &mut bar);
    } else {
        return;
    }
    drop(s);
    stations.mark_dirty(i);
}

/// Lights up or puts out the landing pads by a changed block (it was `was`), saying what the
/// pad is now if it was one.
fn pads(s: &mut Station, changed: IVec3, was: block::Block, bar: &mut ActionBar) {
    if s.is_rocket() {
        return;
    }
    let (_, shape) = orbit_core::pad::refresh_station(s, changed);
    if let Some(shape) = shape.filter(|_| block::is_pad(was) || block::is_pad(s.get(changed))) {
        bar.show(&shape.message());
    }
}

// --- The core's menu -------------------------------------------------------------------------

const PANEL: Color = Color::srgba(0.06, 0.07, 0.1, 0.92);
const BUTTON: Color = Color::srgb(0.2, 0.22, 0.28);
const HOVER: Color = Color::srgb(0.3, 0.33, 0.42);
const PRESS: Color = Color::srgb(0.38, 0.5, 0.7);
const MUTED: Color = Color::srgb(0.63, 0.63, 0.63);

/// The core's menu, while open: which station, its name being edited, and the panel.
#[derive(Resource, Default)]
pub struct StationMenu {
    /// A station whose menu should open (right click on its core).
    wanted: Option<String>,
    open: Option<Open>,
    queue: Vec<MenuAction>,
}

impl StationMenu {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }
}

struct Open {
    id: String,
    name: TextField,
    /// Counted once: the menu is open while nothing is built.
    blocks: usize,
    root: Entity,
    /// The mouse was captured when it opened (it is again when it closes).
    regrab: bool,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum MenuAction {
    Rename,
    Pack,
    Done,
}

#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum MenuText {
    Name,
    Info,
}

fn on_activate(ev: On<Activate>, actions: Query<&MenuAction>, mut menu: ResMut<StationMenu>) {
    if let Ok(&a) = actions.get(ev.entity) {
        menu.queue.push(a);
    }
}

fn label(s: &str, size: f32, color: Color) -> (Text, TextFont, TextColor) {
    (Text::new(s), TextFont { font_size: FontSize::Px(size), ..default() }, TextColor(color))
}

fn button(text: &str, action: MenuAction, width: f32) -> impl Bundle {
    (
        Button,
        Hovered::default(),
        action,
        Node { width: px(width), height: px(24), justify_content: JustifyContent::Center, align_items: AlignItems::Center, border_radius: BorderRadius::all(px(5)), ..default() },
        BackgroundColor(BUTTON),
        children![label(text, 14.0, Color::WHITE)],
    )
}

fn spawn_menu(commands: &mut Commands) -> Entity {
    let w = 260.0;
    commands
        .spawn((
            Node { width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, position_type: PositionType::Absolute, ..default() },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
            GlobalZIndex(15),
        ))
        .with_children(|root| {
            root.spawn((
                Node { flex_direction: FlexDirection::Column, row_gap: px(8), padding: UiRect::all(px(14)), border_radius: BorderRadius::all(px(8)), ..default() },
                BackgroundColor(PANEL),
            ))
            .with_children(|p| {
                p.spawn((label("Station", 18.0, Color::WHITE), Node { align_self: AlignSelf::Center, ..default() }));
                p.spawn(label("Name", 14.0, MUTED));
                p.spawn((
                    Node { width: px(w), height: px(24), padding: UiRect::horizontal(px(6)), align_items: AlignItems::Center, border: UiRect::all(px(1)), ..default() },
                    BorderColor::all(Color::srgb(0.65, 0.65, 0.65)),
                    BackgroundColor(Color::BLACK),
                    children![(label("", 15.0, Color::WHITE), MenuText::Name)],
                ));
                p.spawn((label("", 14.0, MUTED), MenuText::Info));
                p.spawn(Node { flex_direction: FlexDirection::Row, column_gap: px(4), ..default() }).with_children(|r| {
                    let bw = (w - 8.0) / 3.0;
                    r.spawn(button("Rename", MenuAction::Rename, bw));
                    r.spawn(button("Pack up", MenuAction::Pack, bw));
                    r.spawn(button("Done", MenuAction::Done, bw));
                });
            });
        })
        .id()
}

/// Opens the menu that was asked for; types into its name; Enter renames and closes, Esc closes.
#[allow(clippy::too_many_arguments)]
fn menu_keys(
    mut commands: Commands,
    mut menu: ResMut<StationMenu>,
    stations: Res<Stations>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    player: Single<&Player>,
    time: Res<Time>,
) {
    if let Some(id) = menu.wanted.take() {
        if let Some(slot) = stations.by_id(&id) {
            let s = slot.station.read().unwrap();
            let root = spawn_menu(&mut commands);
            let mut name = TextField::new(32);
            name.set(&s.name);
            menu.open = Some(Open { id, name, blocks: s.block_count(), root, regrab: cursor.grab_mode != CursorGrabMode::None });
            keyboard.owner = Some(Owner::StationMenu);
            cursor.grab_mode = CursorGrabMode::None;
            cursor.visible = true;
        }
    }
    let menu = &mut *menu;
    let Some(open) = menu.open.as_mut() else { return };
    // Leaving the station (packed, or walked off) closes it.
    if player.body.station.as_deref() != Some(open.id.as_str()) && stations.by_id(&open.id).is_none_or(|s| s.station.read().unwrap().orbit.is_none()) {
        menu.queue.push(MenuAction::Done);
        return;
    }
    let ctrl = keyboard.ctrl;
    for ev in std::mem::take(&mut keyboard.events) {
        match open.name.apply(&ev, ctrl) {
            crate::chat::Edit::Enter => menu.queue.push(MenuAction::Done),
            crate::chat::Edit::Escape => {
                let o = menu.open.take().unwrap();
                close(&mut commands, &mut keyboard, &mut cursor, o, false);
                return;
            }
            _ => {}
        }
    }
    open.name.tick(time.delta_secs());
}

fn close(commands: &mut Commands, keyboard: &mut Keyboard, cursor: &mut CursorOptions, open: Open, _renamed: bool) {
    commands.entity(open.root).despawn();
    keyboard.owner = None;
    if open.regrab {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
}

fn run_menu(
    mut commands: Commands,
    mut menu: ResMut<StationMenu>,
    mut stations: ResMut<Stations>,
    mut keyboard: ResMut<Keyboard>,
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mut bar: ResMut<ActionBar>,
) {
    for action in std::mem::take(&mut menu.queue) {
        let Some(open) = menu.open.take() else { return };
        let Some(i) = stations.find(&open.id) else {
            close(&mut commands, &mut keyboard, &mut cursor, open, false);
            return;
        };
        // Every button keeps the name typed, as the core's menu does.
        {
            let mut s = stations.list[i].station.write().unwrap();
            if open.name.text.trim() != s.name {
                s.name = station::clean_name(&open.name.text);
            }
        }
        match action {
            MenuAction::Rename => {
                menu.open = Some(open);
                continue;
            }
            MenuAction::Pack => {
                let name = stations.list[i].station.read().unwrap().name.clone();
                stations.pack(i);
                bar.show(&format!("Packed {name}"));
            }
            MenuAction::Done => {}
        }
        close(&mut commands, &mut keyboard, &mut cursor, open, true);
    }
}

fn refresh_menu(menu: Res<StationMenu>, stations: Res<Stations>, mut texts: Query<(&MenuText, &mut Text)>) {
    let Some(open) = menu.open.as_ref() else { return };
    let Some(slot) = stations.by_id(&open.id) else { return };
    let s = slot.station.read().unwrap();
    let span = s.bounds.span();
    for (which, mut text) in &mut texts {
        let want = match which {
            MenuText::Name => open.name.shown(),
            MenuText::Info => format!("{} x {} x {}, {} blocks", span.x, span.z, span.y, open.blocks),
        };
        if text.0 != want {
            text.0 = want;
        }
    }
}

fn style_buttons(mut buttons: Query<(&Hovered, Has<Pressed>, &mut BackgroundColor), (With<Button>, With<MenuAction>)>) {
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
