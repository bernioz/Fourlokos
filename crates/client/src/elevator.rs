//! Space elevators in the game: building one (`/elevator build`), keeping its station over its
//! anchor, the cabin going up and down its ribbon with or without a rider, and drawing the
//! ribbon, the cabin, the ground pad and the counterweight. The physics (why it stands, the
//! counterweight, the climb) are `orbit_core::elevator`; riding is the body's cabin frame
//! (`orbit_core::player::Body::ride`).
//!
//! E by a cabin at rest gets you in (it sets off once its door has closed) or, by an end it is
//! not at, calls it there; E inside turns it round. It stops at the other end, opens its door
//! and you step out: onto the station's deck at the top, the ground at the bottom.

use crate::chat::{ActionBar, Chat, Keyboard};
use crate::elevator_parts as parts;
use crate::lighting::ModelMaterial;
use crate::map::GalaxyMap;
use crate::menu::PlanetMenu;
use crate::player::Player;
use crate::station::{StationMenu, Stations, lanes_of, moon_bands};
use crate::textures::Atlas;
use crate::world::{Clock, MovePlanets, Origin, Space};
use bevy::math::DVec3;
use bevy::prelude::*;
use bevy::world_serialization::WorldInstanceReady;
use orbit_core::block;
use orbit_core::elevator::{self as el, CABIN_FLOOR, CABIN_RIBBON_BACK, Climb, Elevator};
use orbit_core::planet::Terrain;
use orbit_core::player::{Body, CabinAt, Pull};
use orbit_core::station::{self, Station, StationOrbit};

const MODEL: &str = "models/elevator_cabin.glb";
/// Seconds the door takes to open or close.
const DOOR_TIME: f32 = 0.7;
/// The cabin turns from the ribbon's lean to the station's over this last stretch (blocks).
const DOCKING: f64 = 30.0;
/// Farthest from the equator (blocks along the ground) an anchor may be asked for.
const EQUATOR_SLACK: f32 = 4.0;
/// Where a cabin docks on a station when the elevator is built from the ground: its middle
/// column, facing away from the core, the ribbon between them.
const GROUND_DOCK: Vec3 = Vec3::new(0.5, 1.0, 3.5);

pub struct ElevatorsPlugin;

impl Plugin for ElevatorsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Elevators>()
            .init_resource::<Riding>()
            .init_gizmo_group::<MarkGizmos>()
            .add_systems(Startup, (load_assets, setup_marks, setup_ghost))
            .add_systems(
                Update,
                (keep_held, run_elevators)
                    .chain()
                    .after(MovePlanets)
                    .before(crate::player::move_player),
            )
            .add_systems(
                Update,
                (animate_cabins, draw_ribbons, draw_marks).after(crate::player::sync_camera),
            )
            .add_systems(
                Update,
                collide
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            )
            .add_systems(
                Update,
                run_placing
                    .after(crate::player::interact)
                    .before(crate::player::update_hud),
            )
            .add_systems(
                PostUpdate,
                place_elevators.before(TransformSystems::Propagate),
            )
            .add_observer(cabin_ready)
            .add_observer(ghost_ready);
    }
}

/// Every elevator, and what the HUD says about them.
#[derive(Resource)]
pub struct Elevators {
    pub list: Vec<Slot>,
    /// A line for the HUD: what E does here, or the ride.
    pub hud: String,
    /// What stopped the last `/elevator build`, shown on the station for a while.
    pub marks: Option<Marks>,
    /// Riding, the camera is outside the cabin (V), this far from it (the wheel).
    pub outside: bool,
    pub zoom: f32,
    /// `/elevator build` going on: the hologram.
    pub placing: Option<Placing>,
    /// The hologram's cabin, while it is out.
    ghost: Option<Entity>,
}

impl Default for Elevators {
    fn default() -> Self {
        Self {
            list: vec![],
            hud: String::new(),
            marks: None,
            outside: false,
            zoom: 10.0,
            placing: None,
            ghost: None,
        }
    }
}

/// Riding, the camera outside the cabin: the eye and its up, world. The cabin's middle is
/// `zoom` blocks in front of it, where the mouse looks, and a little under.
pub fn outside_eye(
    elevators: &Elevators,
    riding: &Riding,
    look: Vec3,
) -> Option<(DVec3, Vec3, DVec3)> {
    let c = riding.0.as_ref().filter(|_| elevators.outside)?;
    let up = c.rotation * c.up;
    let mid = c.center + (c.rotation * (c.feet + c.up * 2.2)).as_dvec3();
    let eye = mid - (look * elevators.zoom).as_dvec3() + (up * elevators.zoom * 0.15).as_dvec3();
    Some((eye, up, mid))
}

/// How long the blocks in the way stay marked, seconds.
const MARK_SECS: f32 = 30.0;

/// The blocks that kept a cabin from docking (station-local cells, outlined in red) and the
/// nearest spot it fits: where to stand and which way to face (outlined in green).
pub struct Marks {
    station: String,
    cells: Vec<IVec3>,
    stand: Option<(Vec3, u8)>,
    left: f32,
}

/// What keeps a cabin from docking somewhere.
enum Blocked {
    /// Blocks over the deck where it would stand.
    Room(Vec<IVec3>),
    /// The core, in the shaft it would come up through.
    Core(Vec<IVec3>),
}

/// The cabin the player rides this frame, for `player::move_player`; None while not riding (if
/// the body still thinks it rides, the elevator is gone: it lets go).
#[derive(Resource, Default)]
pub struct Riding(pub Option<CabinAt>);

pub struct Slot {
    pub e: Elevator,
    pub climb: Climb,
    /// Where it all is this frame; None while its station or planet is not around.
    pub now: Option<Shape>,
    /// 0 shut .. 1 open; it moves toward `door_open`.
    door: f32,
    door_open: bool,
    /// The cabin sets off once its door has shut.
    leave: Option<bool>,
    /// The rider steps out once the door is open.
    let_out: bool,
    last_v: f64,
    accel: f64,
    cabin: Option<Entity>,
    parts: Vec<Entity>,
    /// The cabin's materials (its glTF ones, lit as the blocks are) and its parts' (the ribbon's
    /// first): each follows the planet's light.
    mats: Vec<Handle<ModelMaterial>>,
    part_mats: Vec<Handle<ModelMaterial>>,
    anim: Option<(Entity, AnimationNodeIndex, AnimationNodeIndex)>,
    /// The ground stop, worked out from the terrain now and then: (when, radius).
    ground: Option<(f64, f64)>,
    /// Said that it cannot hold (no synchronous orbit).
    slack: bool,
}

impl Slot {
    pub fn new(e: Elevator) -> Self {
        let up = e.at_top;
        Self {
            e,
            climb: Climb::resting(0.0, up),
            now: None,
            door: 1.0,
            door_open: true,
            leave: None,
            let_out: false,
            last_v: 0.0,
            accel: 0.0,
            cabin: None,
            parts: vec![],
            mats: vec![],
            part_mats: vec![],
            anim: None,
            ground: None,
            slack: false,
        }
    }
}

/// An elevator where it is now: its planet's frame, the ribbon (planet-local), the stops.
#[derive(Clone, Copy, Debug)]
pub struct Shape {
    pub planet: u64,
    pub center: DVec3,
    pub rotation: Quat,
    /// The planet's turn: angular velocity, world.
    pub spin: Vec3,
    pub spin_rate: f32,
    pub drift: Vec3,
    pub pull: Pull,
    /// The ribbon's direction (unit, planet-local) and the cabin's front at it (horizontal).
    pub dir: Vec3,
    pub front: Vec3,
    /// The cabin's origin height at the ground and at the station; the top of the air; the
    /// counterweight.
    pub bottom: f64,
    pub top: f64,
    pub air: f64,
    pub counterweight: f32,
    /// The docked cabin, planet-local: its origin and turn.
    pub dock_at: Vec3,
    pub dock_turn: Quat,
    pub radius: f32,
}

impl Shape {
    /// The cabin's turn on the ribbon (planet-local): up the ribbon, its back to it.
    fn ribbon_turn(&self) -> Quat {
        let back = -self.front;
        Quat::from_mat3(&Mat3::from_cols(self.dir.cross(back), self.dir, back)).normalize()
    }

    /// The cabin at height `h`: origin and turn, planet-local. Near the top it eases into the
    /// station's turn and lands exactly on its dock.
    pub fn cabin(&self, h: f64) -> (Vec3, Quat) {
        let on = self.dir * h as f32 + self.front * CABIN_RIBBON_BACK;
        let w = smooth(((h - (self.top - DOCKING)) / DOCKING) as f32);
        let dock =
            self.dock_at + (on - (self.dir * self.top as f32 + self.front * CABIN_RIBBON_BACK));
        (
            on.lerp(dock, w),
            self.ribbon_turn().slerp(self.dock_turn, w),
        )
    }

    pub fn to_world(&self, local: Vec3) -> DVec3 {
        self.center + (self.rotation * local).as_dvec3()
    }
}

fn smooth(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Resource)]
pub(crate) struct CabinAssets {
    scene: Handle<WorldAsset>,
    gltf: Handle<Gltf>,
    ribbon: Handle<Mesh>,
    anchor: Handle<Mesh>,
    rock: Handle<Mesh>,
    ribbon_tex: Handle<Image>,
    atlas: Handle<Image>,
}

/// Marks a cabin's model with its elevator's station id.
#[derive(Component)]
struct CabinOf(String);

fn load_assets(
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    atlas: Res<Atlas>,
) {
    commands.insert_resource(CabinAssets {
        scene: assets.load(GltfAssetLabel::Scene(0).from_asset(MODEL)),
        gltf: assets.load(MODEL),
        ribbon: meshes.add(parts::ribbon_mesh()),
        anchor: meshes.add(parts::anchor_mesh()),
        rock: meshes.add(parts::rock_mesh()),
        ribbon_tex: images.add(parts::ribbon_texture()),
        atlas: atlas.0.clone(),
    });
}

/// The stop on the ground: the highest ground under the cabin's footprint (real blocks when the
/// planet is loaded, else the terrain as generated).
fn ground_stop(space: &Space, planet: u64, dir: Vec3, front: Vec3) -> Option<f64> {
    let p = space.planets.get(&planet)?;
    let side = dir.cross(-front);
    let feet: Vec<Vec3> = [
        (0.0, 0.0),
        (1.0, 1.0),
        (-1.0, 1.0),
        (1.0, -1.0),
        (-1.0, -1.0),
    ]
    .iter()
    .map(|&(a, b)| {
        (dir * p.spec.radius + front * CABIN_RIBBON_BACK + side * a * 1.1 + front * b * 1.1)
            .normalize()
    })
    .collect();
    if let Some(v) = p.voxel.as_ref() {
        let planet = v.read().unwrap();
        let top = feet.iter().filter_map(|d| {
            let mut r = p.spec.ceiling();
            while r > planet.grid.core {
                if planet.solid_at(*d * (r - 0.5)) {
                    return Some(r.floor() as f64 + (planet.grid.core.fract()) as f64);
                }
                r -= 1.0;
            }
            None
        });
        return top.max_by(f64::total_cmp);
    }
    let terrain = Terrain::new(&p.spec);
    feet.iter()
        .map(|d| (p.spec.radius + terrain.column(*d).ground() as f32) as f64)
        .max_by(f64::total_cmp)
}

/// Works out where an elevator is now (None while its station or planet is not around).
fn shape_of(slot: &mut Slot, stations: &Stations, space: &Space, t: f64) -> Option<Shape> {
    let st = stations.by_id(&slot.e.station)?;
    let s = st.station.read().unwrap();
    let f = Stations::frame(&s, space, t)?;
    let p = space.planets.get(&f.planet)?;
    let spec = &p.spec;
    let inv = spec.rotation.inverse();
    let ribbon = inv * (f.center + (f.rotation * slot.e.ribbon).as_dvec3() - spec.center).as_vec3();
    let dir = ribbon.normalize();
    let station_turn = inv * f.rotation;
    let front_s = el::front_dir(slot.e.front);
    let front = (station_turn * front_s)
        .reject_from(dir)
        .normalize_or(dir.any_orthonormal_vector());
    let dock_at =
        inv * (f.center + (f.rotation * slot.e.dock()).as_dvec3() - spec.center).as_vec3();
    let back_s = -front_s;
    let dock_turn =
        station_turn * Quat::from_mat3(&Mat3::from_cols(Vec3::Y.cross(back_s), Vec3::Y, back_s));
    let top = (ribbon.length() - CABIN_FLOOR) as f64;
    let pull = spec.pull();
    let ground = match slot.ground {
        Some((when, r)) if (t - when).abs() < 2.0 => r,
        _ => {
            let r = ground_stop(space, f.planet, dir, front)?;
            slot.ground = Some((t, r));
            r
        }
    };
    let (lanes, _) = lanes_of(space, f.planet, &[])?;
    let counterweight = lanes
        .sync
        .and_then(|sync| el::counterweight_radius(sync, pull, &moon_bands(space, f.planet)))
        .unwrap_or(top as f32 + el::COUNTERWEIGHT_MIN);
    Some(Shape {
        planet: f.planet,
        center: spec.center,
        rotation: spec.rotation,
        spin: spec.spin.axis * spec.spin.speed,
        spin_rate: spec.spin.speed,
        drift: spec.velocity,
        pull,
        dir,
        front,
        bottom: ground.min(top - 1.0),
        top,
        air: spec.gravity_radius() as f64,
        counterweight,
        dock_at,
        dock_turn,
        radius: spec.radius,
    })
}

/// Keeps each elevator's station over its anchor (a customized planet turns at another rate,
/// leans another way, or pulls harder: its synchronous orbit moves, the station with it), takes
/// down the elevators whose station is packed or gone, and tells the stations which lanes the
/// ribbons close.
fn keep_held(
    mut elevators: ResMut<Elevators>,
    mut stations: ResMut<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    mut chat: ResMut<Chat>,
) {
    let t = clock.0;
    let mut gone = Vec::new();
    for (i, slot) in elevators.list.iter_mut().enumerate() {
        let Some(k) = stations.find(&slot.e.station) else {
            gone.push(i);
            continue;
        };
        let orbit = stations.list[k].station.read().unwrap().orbit;
        let Some(o) = orbit else {
            gone.push(i);
            continue;
        };
        let Some(p) = space.planets.get(&o.planet) else {
            continue;
        };
        let axis = p.spec.spin.axis;
        // A tilted planet's equator moved: the anchor goes to it, at the same longitude.
        if slot.e.anchor.dot(axis).abs() > 1e-4 {
            slot.e.anchor = slot
                .e
                .anchor
                .reject_from(axis)
                .normalize_or(axis.any_orthonormal_vector());
        }
        let Some(sync) = lanes_of(&space, o.planet, &[]).and_then(|l| l.0.sync) else {
            if !slot.slack {
                chat.error(&format!("{}'s day is too long now: the elevator to {} has no synchronous orbit to hang from", p.spec.name, slot.e.station));
                slot.slack = true;
            }
            continue;
        };
        slot.slack = false;
        let anchor = (p.spec.rotation * slot.e.anchor).as_dvec3();
        let want = el::hold(
            &o,
            slot.e.ribbon,
            anchor,
            axis.as_dvec3(),
            p.spec.spin.speed as f64,
            sync as f64,
            t,
        );
        let now = o.rebased(t);
        let off = (now.start - want.start).length() > 0.01
            || now.turn.angle_between(want.turn) > 1e-5
            || (now.rate - want.rate).abs() > 1e-9
            || now.axis.distance(want.axis) > 1e-6;
        if off {
            stations.list[k].station.write().unwrap().orbit = Some(want);
        }
    }
    for i in gone.into_iter().rev() {
        let slot = elevators.list.remove(i);
        chat.info(&format!(
            "The elevator to station {} came down with it",
            slot.e.station
        ));
        for e in slot.parts.into_iter().chain(slot.cabin) {
            // despawned in place_elevators' sweep
            let _ = e;
        }
    }
    stations.tethers = elevators
        .list
        .iter()
        .filter_map(|s| s.now.map(|n| (n.planet, n.radius, n.counterweight + 10.0)))
        .collect();
}

/// Where a player's feet stand by the cabin at an end: the door's outside.
fn door_feet(shape: &Shape, h: f64) -> Vec3 {
    let (at, turn) = shape.cabin(h);
    at + turn * el::DOOR_OUT
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn run_elevators(
    mut elevators: ResMut<Elevators>,
    stations: Res<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut player: Single<&mut Player>,
    mut riding: ResMut<Riding>,
    mut bar: ResMut<ActionBar>,
    keyboard: Res<Keyboard>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    menu: Res<PlanetMenu>,
    map: Res<GalaxyMap>,
    station_menu: Res<StationMenu>,
) {
    let t = clock.0;
    let dt = time.delta_secs().min(0.1);
    let ui = keyboard.owner.is_some() || menu.is_open() || map.is_open() || station_menu.is_open();
    let pressed = !ui && keys.just_pressed(KeyCode::KeyE);
    let elevators = &mut *elevators;
    // Riding: V takes the camera outside the cabin or back in; outside, the wheel zooms.
    if player.body.cabin.is_some() && !ui {
        if keys.just_pressed(KeyCode::KeyV) {
            elevators.outside = !elevators.outside;
            bar.show(if elevators.outside {
                "Outside view: mouse looks round, wheel zooms"
            } else {
                "Inside view"
            });
        }
        if elevators.outside && scroll.delta.y != 0.0 {
            elevators.zoom =
                (elevators.zoom * 0.88f32.powf(scroll.delta.y.signum())).clamp(5.0, 60.0);
        }
    }
    elevators.hud.clear();
    for slot in &mut elevators.list {
        slot.now = shape_of(slot, &stations, &space, t);
    }
    let body = &mut player.body;
    let riding_id = body.cabin.clone();
    let mut used = false;
    for slot in &mut elevators.list {
        let Some(shape) = slot.now else { continue };
        let rider = riding_id.as_deref() == Some(slot.e.station.as_str());
        if rider {
            if pressed && slot.leave.is_none() && !slot.let_out {
                slot.climb.go(!slot.climb.up);
                used = true;
            }
        } else if riding_id.is_none() && !used && slot.leave.is_none() {
            // By one end or the other?
            let feet = body.world_point(body.pos);
            let near = |h: f64| {
                shape
                    .to_world(shape.cabin(h).0 + shape.dir * CABIN_FLOOR)
                    .distance(feet)
                    < el::REACH as f64 + 1.0
            };
            let at_top =
                body.station.as_deref() == Some(slot.e.station.as_str()) && near(shape.top);
            let at_bottom = body.gravity_source == Some(shape.planet) && near(shape.bottom);
            if at_top || at_bottom {
                let here_up = at_top;
                let resting_here = !slot.climb.moving && slot.climb.up == here_up;
                if resting_here {
                    elevators.hud = format!(
                        "E: ride the elevator {}",
                        if here_up { "down" } else { "up" }
                    );
                    if pressed {
                        used = true;
                        slot.door_open = false;
                        slot.leave = Some(!here_up);
                        let (at, turn) = shape.cabin(slot.climb.h);
                        body.ride(&cabin_at(slot, &shape, at, turn), dt);
                        bar.show(if here_up {
                            "Going down: E turns round"
                        } else {
                            "Going up: E turns round"
                        });
                    }
                } else {
                    elevators.hud = "E: call the cabin".into();
                    if pressed {
                        used = true;
                        slot.door_open = false;
                        slot.climb.go(here_up);
                        bar.show("The cabin is on its way");
                    }
                }
            }
        }
        // The door, then the climb.
        let target = if slot.door_open { 1.0 } else { 0.0 };
        slot.door += (target - slot.door).clamp(-dt / DOOR_TIME, dt / DOOR_TIME);
        if let Some(up) = slot.leave {
            if slot.door <= 0.0 {
                slot.climb.go(up);
                slot.leave = None;
            }
        }
        if slot.climb.moving {
            slot.door_open = false;
            if slot.door <= 0.0
                && slot
                    .climb
                    .step(shape.bottom, shape.top, shape.air, dt as f64)
            {
                slot.e.at_top = slot.climb.up;
                slot.door_open = true;
                slot.let_out = rider;
            }
        } else {
            slot.climb
                .step(shape.bottom, shape.top, shape.air, dt as f64);
        }
        slot.accel = if dt > 0.0 {
            (slot.climb.v - slot.last_v) / dt as f64
        } else {
            0.0
        };
        slot.last_v = slot.climb.v;
    }
    // The rider: carried, or let out once the door is open.
    riding.0 = None;
    // Read again: E may have just got the player in.
    let Some(id) = body.cabin.clone() else { return };
    let Some(slot) = elevators.list.iter_mut().find(|s| s.e.station == id) else {
        return;
    };
    let Some(shape) = slot.now else { return };
    let (at, turn) = shape.cabin(slot.climb.h);
    if slot.let_out && slot.door >= 1.0 {
        slot.let_out = false;
        let pitch = body.pitch;
        let look = body.world_dir(body.forward);
        if slot.climb.up {
            let placed = stations.placed(&space, t);
            if let Some((s, f)) = placed.iter().find(|(s, _)| s.id == id) {
                let local_turn = f.rotation.inverse() * shape.rotation;
                let mut feet = local_turn
                    * (door_feet(&shape, shape.top)
                        - shape.rotation.inverse() * (f.center - shape.center).as_vec3());
                let deck = Station::cell_at(feet - Vec3::Y * 0.5);
                if s.solid(Station::cell_at(feet + Vec3::Y * 0.5)) || !s.solid(deck) {
                    feet = local_turn
                        * (at + turn * el::RIDER
                            - shape.rotation.inverse() * (f.center - shape.center).as_vec3());
                }
                let at = f.at(s);
                body.board(&at, feet, f.rotation.inverse() * look);
                body.pitch = pitch;
                bar.show(&format!("Arrived at {}", s.name));
            }
        } else {
            let feet = door_feet(&shape, shape.bottom) + shape.dir * 0.6;
            body.stand_on(
                shape.planet,
                shape.center,
                shape.rotation,
                shape.drift,
                feet,
            );
            body.pitch = pitch;
            bar.show("Back on the ground");
        }
        return;
    }
    let cab = cabin_at(slot, &shape, at, turn);
    let r = (slot.climb.h + CABIN_FLOOR as f64) as f32;
    let weight = el::felt_weight(r, shape.pull, shape.spin_rate, slot.accel as f32);
    let alt = r - shape.radius;
    let way = if !slot.climb.moving {
        "waiting"
    } else if slot.climb.up {
        "up"
    } else {
        "down"
    };
    let view = if elevators.outside {
        "inside"
    } else {
        "outside"
    };
    elevators.hud = format!(
        "elevator {way}: {alt:.0} blocks up, {:.0} m/s, you weigh {:.0}% of your weight on the ground  E: turn round  V: {view} view",
        slot.climb.v.abs(),
        (weight * 100.0).max(0.0)
    );
    riding.0 = Some(cab);
}

/// The cabin for the body riding it: where the rider stands in it, its up and its speed.
fn cabin_at(slot: &Slot, shape: &Shape, at: Vec3, turn: Quat) -> CabinAt {
    CabinAt {
        elevator: slot.e.station.clone(),
        planet: shape.planet,
        center: shape.center,
        rotation: shape.rotation,
        spin: shape.spin,
        drift: shape.drift,
        feet: at + turn * el::RIDER,
        up: turn * Vec3::Y,
        vel: shape.dir * slot.climb.v as f32,
    }
}

/// Puts each elevator's cabin, ribbon, pad and counterweight where they are; spawns what is
/// missing and takes down what has no elevator any more. The cabin dims in the planet's shade.
#[allow(clippy::too_many_arguments)]
fn place_elevators(
    mut commands: Commands,
    mut elevators: ResMut<Elevators>,
    assets: Option<Res<CabinAssets>>,
    origin: Res<Origin>,
    space: Res<Space>,
    mut tr: Query<&mut Transform>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    cabins: Query<(Entity, &CabinOf)>,
    parts: Query<(Entity, &PartOf)>,
) {
    let Some(assets) = assets else { return };
    let ids: Vec<String> = elevators.list.iter().map(|s| s.e.station.clone()).collect();
    for (e, of) in &cabins {
        if !ids.contains(&of.0) {
            commands.entity(e).despawn();
        }
    }
    for (e, of) in &parts {
        if !ids.contains(&of.0) {
            commands.entity(e).despawn();
        }
    }
    for slot in &mut elevators.list {
        let Some(s) = slot.now else {
            for e in slot.parts.drain(..).chain(slot.cabin.take()) {
                commands.entity(e).despawn();
            }
            slot.anim = None;
            slot.mats.clear();
            slot.part_mats.clear();
            continue;
        };
        let base = origin.render(s.center);
        if slot.cabin.is_none() {
            slot.cabin = Some(
                commands
                    .spawn((
                        WorldAssetRoot(assets.scene.clone()),
                        CabinOf(slot.e.station.clone()),
                        Transform::default(),
                    ))
                    .id(),
            );
        }
        let planet = space.planets.get(&s.planet);
        if slot.parts.is_empty() {
            let star = planet.map_or([1.0; 3], |p| p.star_color);
            let of = || PartOf(slot.e.station.clone());
            // The ribbon, its anchor on the ground, the counterweight; the ribbon's material first.
            // (Its platform on the ground is blocks: `el::lay_base`.)
            let looks = [
                (
                    assets.ribbon.clone(),
                    ModelMaterial::new(star, Some(assets.ribbon_tex.clone()), false),
                ),
                (
                    assets.anchor.clone(),
                    ModelMaterial::new(star, Some(assets.ribbon_tex.clone()), false),
                ),
                (
                    assets.rock.clone(),
                    ModelMaterial::atlas_tile(star, assets.atlas.clone(), block::tile::BASALT),
                ),
            ];
            for (mesh, look) in looks {
                let m = mats.add(look);
                slot.parts.push(
                    commands
                        .spawn((
                            Mesh3d(mesh),
                            MeshMaterial3d(m.clone()),
                            Transform::default(),
                            of(),
                        ))
                        .id(),
                );
                slot.part_mats.push(m);
            }
        }
        let radial = s.ribbon_turn();
        let (lo, hi) = (s.bottom as f32 - 3.0, s.counterweight);
        let set =
            |tr: &mut Query<&mut Transform>, e: Entity, local: Vec3, turn: Quat, scale: Vec3| {
                if let Ok(mut t) = tr.get_mut(e) {
                    *t = Transform {
                        translation: base + s.rotation * local,
                        rotation: s.rotation * turn,
                        scale,
                    };
                }
            };
        set(
            &mut tr,
            slot.parts[0],
            s.dir * (lo + hi) / 2.0,
            radial,
            Vec3::new(1.0, hi - lo, 1.0),
        );
        set(
            &mut tr,
            slot.parts[1],
            s.dir * s.bottom as f32,
            radial,
            Vec3::ONE,
        );
        set(
            &mut tr,
            slot.parts[2],
            s.dir * (hi + 5.0),
            radial,
            Vec3::ONE,
        );
        let (at, turn) = s.cabin(slot.climb.h);
        if let Some(e) = slot.cabin {
            set(&mut tr, e, at, turn, Vec3::ONE);
        }
        // Light: from the planet's star, as the blocks around it are lit (`lighting::ModelMaterial`).
        if let Some(p) = planet {
            let star = origin.render(p.spec.star());
            let bodies: Vec<(Vec3, f32)> = space
                .eclipsers(s.planet)
                .into_iter()
                .map(|(c, r)| (origin.render(c), r))
                .collect();
            for (i, h) in slot.part_mats.iter().chain(&slot.mats).enumerate() {
                if let Some(mut m) = mats.get_mut(h) {
                    m.place(base, s.radius, star, p.star_radius, &bodies);
                    if i == 0 {
                        m.look.planet.w = (hi - lo) / parts::RIBBON_REPEAT;
                    }
                }
            }
        }
    }
}

/// Marks a part (ribbon, pad, counterweight) of an elevator.
#[derive(Component)]
struct PartOf(String);

/// The cabin's model is in: its materials lit as the blocks are (its glass see-through), and its
/// animations (door, lights) hooked up.
#[allow(clippy::too_many_arguments)]
fn cabin_ready(
    ev: On<WorldInstanceReady>,
    cabins: Query<&CabinOf>,
    children: Query<&Children>,
    names: Query<&Name>,
    parents: Query<&ChildOf>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
    standard: Res<Assets<StandardMaterial>>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    space: Res<Space>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    gltfs: Res<Assets<Gltf>>,
    assets: Res<CabinAssets>,
    mut elevators: ResMut<Elevators>,
) {
    let Ok(of) = cabins.get(ev.entity) else {
        return;
    };
    let Some(slot) = elevators.list.iter_mut().find(|s| s.e.station == of.0) else {
        return;
    };
    let glass = |mut e: Entity| {
        for _ in 0..3 {
            if names.get(e).is_ok_and(|n| {
                n.as_str().starts_with("glass") || n.as_str().starts_with("door_glass")
            }) {
                return true;
            }
            match parents.get(e) {
                Ok(p) => e = p.parent(),
                Err(_) => return false,
            }
        }
        false
    };
    slot.mats.clear();
    let star = slot
        .now
        .and_then(|s| space.planets.get(&s.planet))
        .map_or([1.0; 3], |p| p.star_color);
    let mut copies: Vec<(AssetId<StandardMaterial>, bool, Handle<ModelMaterial>)> = Vec::new();
    let mut player = None;
    for e in children.iter_descendants(ev.entity) {
        if players.get(e).is_ok() && player.is_none() {
            player = Some(e);
        }
        let Ok(m) = meshes.get(e) else { continue };
        let see_through = glass(e);
        let handle = match copies
            .iter()
            .find(|c| c.0 == m.0.id() && c.1 == see_through)
        {
            Some(c) => c.2.clone(),
            None => {
                let Some(gltf) = standard.get(&m.0) else {
                    continue;
                };
                let mut look =
                    ModelMaterial::new(star, gltf.base_color_texture.clone(), see_through);
                look.look.color = gltf.base_color.to_linear().to_vec4();
                let h = mats.add(look);
                slot.mats.push(h.clone());
                copies.push((m.0.id(), see_through, h.clone()));
                h
            }
        };
        commands
            .entity(e)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
    let Some(player) = player else { return };
    let Some(gltf) = gltfs.get(&assets.gltf) else {
        return;
    };
    let (Some(door), Some(lights)) = (
        gltf.named_animations.get("animation.door_open"),
        gltf.named_animations.get("animation.lights"),
    ) else {
        return;
    };
    let (graph, idx) = AnimationGraph::from_clips([door.clone(), lights.clone()]);
    commands
        .entity(player)
        .insert(AnimationGraphHandle(graphs.add(graph)));
    if let Ok(mut p) = players.get_mut(player) {
        p.play(idx[0]).set_speed(0.0);
        p.play(idx[1]).repeat();
    }
    slot.anim = Some((player, idx[0], idx[1]));
}

/// The door follows the slot's door (0 shut .. 1 open), and the lights blink while it moves.
fn animate_cabins(elevators: Res<Elevators>, mut players: Query<&mut AnimationPlayer>) {
    for slot in &elevators.list {
        let Some((e, door, lights)) = slot.anim else {
            continue;
        };
        let Ok(mut p) = players.get_mut(e) else {
            continue;
        };
        p.play(door)
            .set_speed(0.0)
            .seek_to(slot.door.clamp(0.0, 1.0));
        let blink = p.play(lights).repeat();
        blink.set_speed(if slot.climb.moving { 1.0 } else { 0.0 });
    }
}

/// A thin line along each ribbon, so it shows from far away where its model is under a pixel.
fn draw_ribbons(elevators: Res<Elevators>, origin: Res<Origin>, mut gizmos: Gizmos) {
    for slot in &elevators.list {
        let Some(s) = slot.now else { continue };
        let a = origin.render(s.to_world(s.dir * s.bottom as f32));
        let b = origin.render(s.to_world(s.dir * s.counterweight));
        gizmos.line(a, b, Color::srgba(0.16, 0.17, 0.2, 0.75));
    }
}

/// The lines of `draw_marks`: drawn over the blocks, so marks under the deck (the core) show.
#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct MarkGizmos;

fn setup_marks(mut store: ResMut<GizmoConfigStore>) {
    let (config, _) = store.config_mut::<MarkGizmos>();
    config.depth_bias = -1.0;
    config.line.width = 2.5;
}

/// Outlines, blinking, the blocks that kept the last `/elevator build` from going up (red) and
/// the spot where it fits (green, with an arrow the way to face), and draws a line from your eye
/// to the spot (or the first block) so it is found wherever it is.
fn draw_marks(
    mut elevators: ResMut<Elevators>,
    stations: Res<Stations>,
    space: Res<Space>,
    clock: Res<Clock>,
    origin: Res<Origin>,
    time: Res<Time>,
    player: Single<&Player>,
    mut gizmos: Gizmos<MarkGizmos>,
) {
    let Some(m) = elevators.marks.as_mut() else {
        return;
    };
    m.left -= time.delta_secs();
    if m.left <= 0.0 {
        elevators.marks = None;
        return;
    }
    let Some(slot) = stations.by_id(&m.station) else {
        return;
    };
    let Some(f) = Stations::frame(&slot.station.read().unwrap(), &space, clock.0) else {
        return;
    };
    let at = |p: Vec3| origin.render(f.center + (f.rotation * p).as_dvec3());
    let blink = 0.55 + 0.45 * (time.elapsed_secs() * 6.0).sin().abs();
    let red = Color::srgba(1.0, 0.15, 0.1, blink);
    let cube = |gizmos: &mut Gizmos<MarkGizmos>, lo: Vec3, hi: Vec3, color: Color| {
        let c = |i: usize| {
            Vec3::new(
                if i & 1 == 0 { lo.x } else { hi.x },
                if i & 2 == 0 { lo.y } else { hi.y },
                if i & 4 == 0 { lo.z } else { hi.z },
            )
        };
        for (a, b) in [
            (0, 1),
            (2, 3),
            (4, 5),
            (6, 7),
            (0, 2),
            (1, 3),
            (4, 6),
            (5, 7),
            (0, 4),
            (1, 5),
            (2, 6),
            (3, 7),
        ] {
            gizmos.line(at(c(a)), at(c(b)), color);
        }
    };
    for &cell in &m.cells {
        let mid = cell.as_vec3() + 0.5;
        cube(&mut gizmos, mid - 0.53, mid + 0.53, red);
    }
    let target = if let Some((feet, front)) = m.stand {
        let green = Color::srgba(0.2, 1.0, 0.35, blink);
        // The block to stand in, a post over it to see from afar, and the way to face.
        cube(
            &mut gizmos,
            feet - Vec3::new(0.5, 0.0, 0.5),
            feet + Vec3::new(0.5, 1.8, 0.5),
            green,
        );
        gizmos.line(at(feet), at(feet + Vec3::Y * 6.0), green);
        let dir = el::front_dir(front);
        let side = Vec3::Y.cross(dir);
        let (base, tip) = (feet + Vec3::Y * 0.05, feet + Vec3::Y * 0.05 + dir * 1.6);
        gizmos.line(at(base), at(tip), green);
        gizmos.line(at(tip), at(tip - dir * 0.5 + side * 0.4), green);
        gizmos.line(at(tip), at(tip - dir * 0.5 - side * 0.4), green);
        Some((feet + Vec3::Y * 0.9, green))
    } else {
        m.cells.first().map(|&c| (c.as_vec3() + 0.5, red))
    };
    if let Some((to, color)) = target {
        let b = &player.body;
        let eye = origin.render(b.world_point(b.eye()));
        let to = at(to);
        gizmos.line(
            eye + (to - eye).normalize_or_zero() * 1.5,
            to,
            color.with_alpha(0.6 * blink),
        );
    }
}

/// The nearest spot on the deck around `feet` (station-local) where a cabin fits (`fits`: by the
/// feet of who builds it and the way they face), facing `front` if it can: where to stand and
/// which way to face. Only spots you can stand on, a few blocks around.
fn stand_spot(
    s: &Station,
    feet: Vec3,
    front: u8,
    fits: impl Fn(Vec3, u8) -> bool,
) -> Option<(Vec3, u8)> {
    const REACH: i32 = 10;
    let here = feet.floor().as_ivec3();
    let mut spots = Vec::new();
    for dy in -1..=1 {
        for dz in -REACH..=REACH {
            for dx in -REACH..=REACH {
                let c = here + IVec3::new(dx, dy, dz);
                if s.solid(c - IVec3::Y) && !s.solid(c) && !s.solid(c + IVec3::Y) {
                    spots.push((dx * dx + dz * dz + 4 * dy * dy, c));
                }
            }
        }
    }
    spots.sort_by_key(|&(d, _)| d);
    let fronts = [front, (front + 1) % 4, (front + 3) % 4, (front + 2) % 4];
    spots.into_iter().find_map(|(_, c)| {
        let at = c.as_vec3() + Vec3::new(0.5, 0.0, 0.5);
        fronts.into_iter().find(|&f| fits(at, f)).map(|f| (at, f))
    })
}

// --- Building -----------------------------------------------------------------------------------

/// The other stations around a planet whose lanes a ribbon from `lo` to `hi` would cross (all but
/// the synchronous lane's): they would meet it sooner or later.
fn in_the_way(
    stations: &Stations,
    planet: u64,
    except: &str,
    lanes: &station::Lanes,
    lo: f32,
    hi: f32,
) -> Vec<String> {
    stations
        .list
        .iter()
        .map(|s| s.station.read().unwrap())
        .filter(|s| s.id != except)
        .filter_map(|s| {
            let o = s.orbit.filter(|o| o.planet == planet)?;
            let r = o.radius();
            let reach = (
                r + station::MIN_Y as f32 - station::GRAVITY_BELOW - station::CLEAR,
                r + (station::MAX_Y + 1) as f32 + station::GRAVITY_ABOVE + station::CLEAR,
            );
            (!lanes.is_sync(r) && reach.1 > lo && reach.0 < hi)
                .then(|| format!("{} ({})", s.name, s.id))
        })
        .collect()
}

/// Builds an elevator from the station `i` the player is on or names: from the deck where the
/// player stands (`from_deck`: its cell and which way the cabin faces), or else from the ground
/// at `anchor` (planet-local, the player's spot on the equator) with the cabin at `GROUND_DOCK`.
/// Says what it did, or why not.
pub fn build(
    elevators: &mut Elevators,
    stations: &mut Stations,
    space: &Space,
    body: &Body,
    t: f64,
    i: usize,
    from_deck: Option<(Vec3, u8)>,
    anchor: Option<Vec3>,
) -> Result<String, String> {
    let (id, name, orbit) = {
        let s = stations.list[i].station.read().unwrap();
        (s.id.clone(), s.name.clone(), s.orbit)
    };
    if elevators.list.iter().any(|s| s.e.station == id) {
        return Err(format!("{name} already has an elevator: /elevator info"));
    }
    let o = orbit.ok_or(format!("{name} is packed: /station restore {id}"))?;
    let p = space
        .planets
        .get(&o.planet)
        .ok_or("Its planet is too far away")?;
    if p.spec.is_moon() {
        return Err("A moon turns once a month, so its synchronous orbit is far outside its pull: no elevator stands on a moon (a real lunar one would hang through the L1 point instead)".into());
    }
    let (lanes, pull) =
        lanes_of(space, o.planet, &stations.tethers).ok_or("Its planet is too far away")?;
    let sync = lanes.sync.ok_or(station::Refusal::NoSync.message())?;
    if anchor.is_none() && !lanes.is_sync(o.radius()) {
        return Err(format!(
            "Elevators hang from the synchronous lane, over one spot of the equator: move {name} there first with /station height sync"
        ));
    }
    let counterweight = el::counterweight_radius(sync, pull, &moon_bands(space, o.planet))
        .ok_or("A moon goes round just over the synchronous orbit: no room for a counterweight")?;
    let (ribbon, front) = match from_deck {
        Some((feet, front)) => {
            let floor = feet.y.round();
            let center = Vec3::new(feet.x.floor() + 0.5, floor, feet.z.floor() + 0.5);
            (center - el::front_dir(front) * CABIN_RIBBON_BACK, front)
        }
        None => (GROUND_DOCK - el::front_dir(2) * CABIN_RIBBON_BACK, 2),
    };
    let e = Elevator {
        station: id.clone(),
        planet: o.planet,
        anchor: Vec3::X,
        ribbon,
        front,
        at_top: from_deck.is_some(),
    };
    let axis = p.spec.spin.axis;
    // Where it stands, for a ribbon at `ribbon` on the deck: straight under that point, on the
    // equator; or where the player stands on the equator. And the orbit that holds it there.
    let place = |ribbon: Vec3| {
        let anchor_local = match anchor {
            Some(a) => a,
            None => {
                let now = o.rebased(t);
                let world = now.start + (now.rotation(t) * ribbon).as_dvec3();
                (p.spec.rotation.inverse() * world.as_vec3())
                    .reject_from(axis)
                    .normalize()
            }
        };
        (
            anchor_local,
            el::hold(
                &o,
                ribbon,
                (p.spec.rotation * anchor_local).as_dvec3(),
                axis.as_dvec3(),
                p.spec.spin.speed as f64,
                sync as f64,
                t,
            ),
        )
    };
    // The shaft the cabin comes up through, along the ribbon (it leans off the core's up).
    let shaft_of = |e: &Elevator, held: &station::StationOrbit| {
        let w = held.offset(t) + (held.rotation(t) * e.ribbon).as_dvec3();
        let down = (held.rotation(t).inverse() * -w.normalize().as_vec3()).normalize();
        el::shaft(e.dock(), el::front_dir(e.front), down, station::MIN_Y)
    };
    // The blocks in the way of a cabin there: over the deck, or the core in its shaft.
    let in_way = |s: &Station, e: &Elevator| -> Result<(), Blocked> {
        let room: Vec<IVec3> = el::cabin_room(e.dock(), el::front_dir(e.front))
            .into_iter()
            .filter(|&c| s.solid(c))
            .collect();
        if !room.is_empty() {
            return Err(Blocked::Room(room));
        }
        let core: Vec<IVec3> = shaft_of(e, &place(e.ribbon).1)
            .into_iter()
            .filter(|&c| s.is_core(c))
            .collect();
        if !core.is_empty() {
            return Err(Blocked::Core(core));
        }
        Ok(())
    };
    // Room for the cabin over the deck, and none of it on the core.
    {
        let s = stations.list[i].station.read().unwrap();
        if let Err(b) = in_way(&s, &e) {
            // Mark the blocks to blame, and the nearest spot to stand on where it fits.
            let stand = from_deck.and_then(|(feet, front)| {
                let fits = |at: Vec3, f: u8| {
                    let center = Vec3::new(at.x.floor() + 0.5, at.y.round(), at.z.floor() + 0.5);
                    in_way(
                        &s,
                        &Elevator {
                            ribbon: center - el::front_dir(f) * CABIN_RIBBON_BACK,
                            front: f,
                            ..e.clone()
                        },
                    )
                    .is_ok()
                };
                stand_spot(&s, feet, front, fits)
            });
            let (cells, why) = match b {
                Blocked::Room(c) => {
                    let why = format!("No room for the cabin there: {} blocks in the way, marked in red (it stands {:.0} wide and {:.0} high)", c.len(), 2.0 * el::CABIN_HALF_X, el::CABIN_HEIGHT);
                    (c, why)
                }
                Blocked::Core(c) => (c, "The core (marked in red) is in the way of the cabin's shaft: the cabin is 4 blocks wide and 3 deep, its back to the ribbon".to_string()),
            };
            let hint = match (&stand, from_deck.is_some()) {
                (Some(_), _) => {
                    ". Stand on the green mark, facing where its arrow points, and build again"
                }
                (None, true) => {
                    ". No spot near you fits it: clear the red blocks, or stand 3 or more blocks from the core facing away from it"
                }
                (None, false) => "",
            };
            elevators.marks = Some(Marks {
                station: id.clone(),
                cells,
                stand,
                left: MARK_SECS,
            });
            return Err(format!("{why}{hint}"));
        }
    }
    let ground = p.spec.radius - 10.0;
    let others = in_the_way(
        stations,
        o.planet,
        &id,
        &lanes,
        ground,
        counterweight + 10.0,
    );
    if !others.is_empty() {
        return Err(format!(
            "The ribbon would cross the orbits of {}: they would hit it sooner or later. Move them over {:.0} blocks up (/station height) or pack them",
            others.join(", "),
            counterweight + 10.0 - p.spec.radius
        ));
    }
    let (anchor_local, held) = place(ribbon);
    let others: Vec<_> = stations
        .list
        .iter()
        .enumerate()
        .filter(|&(k, _)| k != i)
        .filter_map(|(_, s)| s.station.read().unwrap().orbit)
        .filter(|x| x.planet == o.planet)
        .collect();
    if let Some(x) = others.iter().find(|x| !held.clear_of(&x.rebased(t))) {
        let who = stations
            .list
            .iter()
            .map(|s| s.station.read().unwrap())
            .find(|s| s.orbit.as_ref() == Some(x))
            .map(|s| s.name.clone())
            .unwrap_or_default();
        return Err(format!(
            "{who} is in the way there, in the synchronous lane: pick another spot on the equator"
        ));
    }
    if from_deck.is_none() && body.station.as_deref() == Some(id.as_str()) {
        return Err("Build it from the station's deck: /elevator build".into());
    }
    let cells = shaft_of(&e, &held);
    let mut s = stations.list[i].station.write().unwrap();
    let mut opened = 0;
    for c in cells {
        if s.get(c) != block::AIR {
            s.set(c, block::AIR);
            opened += 1;
        }
    }
    s.orbit = Some(held);
    drop(s);
    stations.mark_dirty(i);
    elevators.marks = None;
    let mut slot = Slot::new(Elevator {
        anchor: anchor_local,
        ..e
    });
    slot.climb = Climb::resting(0.0, slot.e.at_top);
    elevators.list.push(slot);
    let long = counterweight - p.spec.radius;
    let shaft = if opened > 0 {
        format!(" Opened a shaft of {opened} blocks through the deck for the cabin.")
    } else {
        String::new()
    };
    Ok(format!(
        "Built an elevator from {} to {name}: a ribbon {long:.0} blocks long, up to a counterweight {:.0} blocks over the station.{shaft} E by the cabin to ride",
        p.spec.name,
        counterweight - sync
    ))
}

/// Where you step out of a cabin docked on the ground, planet-local (for the demo).
pub fn ground_door(slot: &Slot) -> Option<Vec3> {
    slot.now.map(|s| door_feet(&s, s.bottom))
}

/// The middle of the floor of a cabin docked on the ground, planet-local (for the demo).
pub fn ground_middle(slot: &Slot) -> Option<Vec3> {
    slot.now.map(|s| {
        let (at, turn) = s.cabin(s.bottom);
        at + turn * Vec3::Y * CABIN_FLOOR
    })
}

/// What an elevator is, in words: lines for `/elevator info`.
pub fn describe(slot: &Slot, space: &Space, stations: &Stations) -> Vec<String> {
    let name = stations
        .by_id(&slot.e.station)
        .map(|s| s.station.read().unwrap().name.clone())
        .unwrap_or_else(|| slot.e.station.clone());
    let Some(s) = slot.now else {
        return vec![format!("Elevator to {name}: too far away to see")];
    };
    let planet = space
        .planets
        .get(&s.planet)
        .map_or(String::new(), |p| p.spec.name.clone());
    let long = s.counterweight - s.bottom as f32;
    let ground_g = el::apparent_gravity(s.bottom as f32, s.pull, s.spin_rate);
    let anchor = 2.0 * el::CABIN_MASS * ground_g as f64;
    let mass = el::counterweight_mass(
        s.bottom as f32,
        s.counterweight,
        s.pull,
        s.spin_rate,
        el::RIBBON_DENSITY,
        anchor,
    );
    let ribbon_t = el::RIBBON_DENSITY * long as f64 / 1000.0;
    let mut out = vec![
        format!(
            "Elevator from {planet} to {name} ({}): the cabin is {}",
            slot.e.station,
            if slot.climb.moving {
                "under way"
            } else if slot.climb.up {
                "at the station"
            } else {
                "on the ground"
            }
        ),
        format!(
            "Ribbon {long:.0} blocks long ({ribbon_t:.1} t), {:.0} to the station, {:.0} on to the counterweight",
            s.top - s.bottom,
            s.counterweight as f64 - s.top
        ),
        format!(
            "A trip takes {:.0} s: {:.0} m/s in the air, {:.0} above it",
            el::trip_time(s.bottom, s.top, s.air),
            el::AIR_SPEED,
            el::SPACE_SPEED
        ),
    ];
    if let Some(m) = mass {
        out.push(format!("Counterweight {:.1} t: its outward pull holds the ribbon up and keeps it taut at the anchor", m / 1000.0));
    }
    out
}

// --- Building from the ground: the hologram, then the real thing ------------------------------

/// `/elevator build` going on: a hologram of the elevator where you look (on the equator), built
/// with a right click, put away with a left click.
#[derive(Default)]
pub struct Placing {
    ok: Option<bool>,
}

/// An elevator as it would stand from a spot of the ground: where its ribbon meets the ground,
/// which way its cabin faces there, the platform's top (the cabin's floor) and the orbit that
/// would hold its station up.
struct Plan {
    planet: u64,
    anchor: Vec3,
    front: Vec3,
    layer: usize,
    bottom: f32,
    held: StationOrbit,
}

impl Plan {
    /// The platform's middle (the docked cabin's), planet-local.
    fn middle(&self) -> Vec3 {
        self.anchor * self.bottom + self.front * CABIN_RIBBON_BACK
    }

    /// The docked cabin's turn (planet-local), as `Shape::ribbon_turn`.
    fn turn(&self) -> Quat {
        let back = -self.front;
        Quat::from_mat3(&Mat3::from_cols(self.anchor.cross(back), self.anchor, back)).normalize()
    }
}

/// The orbit a new terminal starts from before it is held over `anchor` (world): any will do, but
/// the hologram and the build must agree, as `el::hold` turns the station the least from it.
fn first_orbit(
    planet: u64,
    anchor: Vec3,
    axis: Vec3,
    spin: f32,
    sync: f32,
    t: f64,
) -> StationOrbit {
    StationOrbit {
        planet,
        start: (anchor * sync).as_dvec3(),
        turn: bevy::math::DQuat::IDENTITY,
        axis: (axis * spin.signum()).as_dvec3(),
        rate: spin.abs() as f64,
        t0: t,
    }
}

/// The ribbon's point on the terminal's deck, station-local.
fn terminal_ribbon() -> Vec3 {
    el::TERMINAL_FEET - el::front_dir(el::TERMINAL_FRONT) * CABIN_RIBBON_BACK
}

/// The elevator that would stand from the equator nearest `target` (planet-local) on planet `id`:
/// the plan (when there is one to show) and why it cannot be built, if it cannot.
fn plan(
    space: &Space,
    stations: &Stations,
    id: u64,
    target: Vec3,
    t: f64,
) -> (Option<Plan>, Result<(), String>) {
    let Some(p) = space.planets.get(&id) else {
        return (None, Err("No planet here".into()));
    };
    if p.spec.is_moon() {
        return (None, Err("A moon turns once a month, so its synchronous orbit is far outside its pull: no elevator stands on a moon".into()));
    }
    let Some(voxels) = p.voxel.as_ref() else {
        return (None, Err("Too far from the ground".into()));
    };
    let Some((lanes, pull)) = lanes_of(space, id, &stations.tethers) else {
        return (None, Err("No planet here".into()));
    };
    let Some(sync) = lanes.sync else {
        return (None, Err(station::Refusal::NoSync.message().into()));
    };
    let axis = p.spec.spin.axis;
    let anchor = target
        .reject_from(axis)
        .normalize_or(axis.any_orthonormal_vector());
    let world = p.spec.rotation * anchor;
    let held = el::hold(
        &first_orbit(id, world, axis, p.spec.spin.speed, sync, t),
        terminal_ribbon(),
        world.as_dvec3(),
        axis.as_dvec3(),
        p.spec.spin.speed as f64,
        sync as f64,
        t,
    );
    let front = (p.spec.rotation.inverse() * held.rotation(t) * el::front_dir(el::TERMINAL_FRONT))
        .reject_from(anchor)
        .normalize_or(anchor.any_orthonormal_vector());
    let planet = voxels.read().unwrap();
    let ground = |r: f32| {
        let mid = (anchor * r + front * CABIN_RIBBON_BACK).normalize();
        el::surface_layer(&planet, mid)
    };
    let Some(layer) = ground(p.spec.radius) else {
        return (None, Err("No ground here".into()));
    };
    let bottom = planet.grid.radius(layer + 1);
    let plan = Plan {
        planet: id,
        anchor,
        front,
        layer,
        bottom,
        held,
    };
    // On the equator?
    let off = target.normalize().dot(axis).clamp(-1.0, 1.0).asin() * p.spec.radius;
    if off.abs() > EQUATOR_SLACK {
        let way = if off > 0.0 { "south" } else { "north" };
        return (
            Some(plan),
            Err(format!(
                "Elevators stand on the equator (anywhere else the planet's turn would lean the ribbon over): it is {:.0} blocks {way}",
                off.abs()
            )),
        );
    }
    let Some(counterweight) = el::counterweight_radius(sync, pull, &moon_bands(space, id)) else {
        return (
            Some(plan),
            Err(
                "A moon goes round just over the synchronous orbit: no room for a counterweight"
                    .into(),
            ),
        );
    };
    let others = in_the_way(
        stations,
        id,
        "",
        &lanes,
        p.spec.radius - 10.0,
        counterweight + 10.0,
    );
    if !others.is_empty() {
        return (
            Some(plan),
            Err(format!(
                "The ribbon would cross the orbits of {}: move them over {:.0} blocks up (/station height) or pack them",
                others.join(", "),
                counterweight + 10.0 - p.spec.radius
            )),
        );
    }
    let around: Vec<StationOrbit> = stations
        .list
        .iter()
        .filter_map(|s| s.station.read().unwrap().orbit)
        .filter(|o| o.planet == id)
        .collect();
    if around.len() >= station::MAX_PER_PLANET {
        return (Some(plan), Err(station::Refusal::TooMany.message().into()));
    }
    if around.iter().any(|o| !plan.held.clear_of(&o.rebased(t))) {
        return (Some(plan), Err("Another station is in the way up there, in the synchronous lane: pick another spot on the equator".into()));
    }
    (Some(plan), Ok(()))
}

/// Builds it: the terminal goes up over the spot, held there; the ribbon down to the ground, the
/// cabin waiting at the bottom on its platform, laid in blocks. What to say, or why not.
fn build_from_ground(
    elevators: &mut Elevators,
    stations: &mut Stations,
    space: &mut Space,
    body: &Body,
    plan: &Plan,
    t: f64,
) -> Result<String, String> {
    let name = format!(
        "{} Skyport",
        space
            .planets
            .get(&plan.planet)
            .map_or("Elevator".into(), |p| p.spec.name.clone())
    );
    let id = loop {
        let id = Station::new_id(orbit_core::rng::mix(
            t.to_bits() ^ stations.list.len() as u64 ^ 0xE1E7,
        ));
        if stations.find(&id).is_none() {
            break id;
        }
    };
    let mut terminal = el::terminal(id, &name);
    let p = space.planets.get(&plan.planet).ok_or("No planet here")?;
    let (lanes, _) = lanes_of(space, plan.planet, &stations.tethers).ok_or("No planet here")?;
    let world = p.spec.rotation * plan.anchor;
    terminal.orbit = Some(first_orbit(
        plan.planet,
        world,
        p.spec.spin.axis,
        p.spec.spin.speed,
        lanes.sync.ok_or("No synchronous orbit")?,
        t,
    ));
    let i = stations.push(terminal);
    let line = match build(
        elevators,
        stations,
        space,
        body,
        t,
        i,
        Some((el::TERMINAL_FEET, el::TERMINAL_FRONT)),
        Some(plan.anchor),
    ) {
        Ok(line) => line,
        Err(e) => {
            stations.list.pop();
            return Err(e);
        }
    };
    // The cabin waits on the ground, where you are.
    if let Some(slot) = elevators.list.last_mut() {
        slot.e.at_top = false;
        slot.climb = Climb::resting(0.0, false);
    }
    // The platform, in blocks.
    if let Some(slot) = space.planets.get_mut(&plan.planet) {
        if let Some(v) = slot.voxel.clone() {
            let keys = el::lay_base(&mut v.write().unwrap(), plan.middle(), plan.layer);
            slot.edited = true;
            crate::world::remesh(slot, &keys);
        }
    }
    Ok(line)
}

/// The hologram's model is in: every part of it glows in the hologram's color.
fn ghost_ready(
    ev: On<WorldInstanceReady>,
    ghosts: Query<(), With<Ghost>>,
    children: Query<&Children>,
    meshes: Query<(), With<MeshMaterial3d<StandardMaterial>>>,
    mut commands: Commands,
    look: Res<GhostLook>,
) {
    if ghosts.get(ev.entity).is_err() {
        return;
    }
    for e in children.iter_descendants(ev.entity) {
        if meshes.get(e).is_ok() {
            commands
                .entity(e)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(look.0.clone()));
        }
    }
}

/// Marks the hologram's cabin.
#[derive(Component)]
struct Ghost;

/// The hologram's material (its color says whether it can be built there).
#[derive(Resource)]
pub(crate) struct GhostLook(Handle<ModelMaterial>);

const GHOST_OK: Vec4 = Vec4::new(0.35, 0.85, 1.0, 0.42);
const GHOST_NO: Vec4 = Vec4::new(1.0, 0.32, 0.25, 0.42);

fn setup_ghost(mut commands: Commands, mut mats: ResMut<Assets<ModelMaterial>>) {
    let mut m = ModelMaterial::new([1.0; 3], None, true);
    m.look.planet.y = 2.0;
    m.look.color = GHOST_OK;
    commands.insert_resource(GhostLook(mats.add(m)));
}

/// While `/elevator build` goes on: the hologram where you look, and the clicks.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_placing(
    mut commands: Commands,
    mut elevators: ResMut<Elevators>,
    mut stations: ResMut<Stations>,
    mut space: ResMut<Space>,
    player: Single<&Player>,
    clock: Res<Clock>,
    origin: Res<Origin>,
    mouse: Res<ButtonInput<MouseButton>>,
    (keyboard, menu, map): (Res<Keyboard>, Res<PlanetMenu>, Res<GalaxyMap>),
    assets: Option<Res<CabinAssets>>,
    look: Res<GhostLook>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    mut tr: Query<&mut Transform>,
    (mut bar, mut chat): (ResMut<ActionBar>, ResMut<Chat>),
    mut gizmos: Gizmos<MarkGizmos>,
) {
    let body = &player.body;
    let on_ground = body
        .gravity_source
        .filter(|_| body.station.is_none() && body.cabin.is_none());
    if elevators.placing.is_none() || on_ground.is_none() {
        // Put away, built, or off the ground: the hologram goes.
        elevators.placing = None;
        if let Some(e) = elevators.ghost.take() {
            commands.entity(e).despawn();
        }
        return;
    }
    let Some(id) = on_ground else { return };
    let Some(assets) = assets else { return };
    let ghost = *elevators.ghost.get_or_insert_with(|| {
        commands
            .spawn((
                WorldAssetRoot(assets.scene.clone()),
                Ghost,
                Transform::default(),
                Visibility::Hidden,
            ))
            .id()
    });
    let Some(placing) = elevators.placing.as_mut() else {
        return;
    };
    // Where you look on the ground, near by; else where you stand.
    let target = space
        .planets
        .get(&id)
        .and_then(|p| p.voxel.as_ref())
        .and_then(|v| {
            let planet = v.read().unwrap();
            planet
                .raycast(body.eye(), body.look_dir(), 48.0)
                .map(|h| planet.grid.center(h.cell))
        });
    let t = clock.0;
    let (plan, ok) = plan(&space, &stations, id, target.unwrap_or(body.pos), t);
    let Some(p) = space.planets.get(&id) else {
        return;
    };
    let base = origin.render(p.spec.center);
    let Some(plan) = plan else {
        if let Ok(mut t) = tr.get_mut(ghost) {
            t.scale = Vec3::ZERO;
        }
        elevators.hud = format!("{}  Left click: put it away", ok.err().unwrap_or_default());
        if mouse.just_pressed(MouseButton::Left) {
            elevators.placing = None;
        }
        return;
    };
    // The hologram: the cabin, its platform's blocks outlined, the ribbon going up.
    let good = ok.is_ok();
    if placing.ok != Some(good) {
        placing.ok = Some(good);
        if let Some(mut m) = mats.get_mut(&look.0) {
            m.look.color = if good { GHOST_OK } else { GHOST_NO };
        }
    }
    let color = if good {
        Color::srgb(0.35, 0.85, 1.0)
    } else {
        Color::srgb(1.0, 0.32, 0.25)
    };
    commands.entity(ghost).insert(Visibility::Inherited);
    if let Ok(mut t) = tr.get_mut(ghost) {
        *t = Transform {
            translation: base + p.spec.rotation * plan.middle(),
            rotation: p.spec.rotation * plan.turn(),
            scale: Vec3::ONE,
        };
    }
    if let Some(v) = p.voxel.as_ref() {
        let planet = v.read().unwrap();
        let g = &planet.grid;
        for (cell, _) in el::base_cells(g, plan.middle(), plan.layer) {
            let c = |di, dj| base + p.spec.rotation * g.corner(cell, di, dj, 1);
            for (a, b) in [
                ((0, 0), (1, 0)),
                ((1, 0), (1, 1)),
                ((1, 1), (0, 1)),
                ((0, 1), (0, 0)),
            ] {
                gizmos.line(c(a.0, a.1), c(b.0, b.1), color);
            }
        }
    }
    gizmos.line(
        base + p.spec.rotation * (plan.anchor * plan.bottom),
        base + p.spec.rotation * (plan.anchor * (plan.bottom + 400.0)),
        color,
    );
    elevators.hud = match &ok {
        Ok(()) => "Right click: build the elevator here (its station goes up over it)  Left click: put it away".into(),
        Err(e) => format!("{e}  Left click: put it away"),
    };
    let ui = keyboard.owner.is_some() || menu.is_open() || map.is_open();
    if ui {
        return;
    }
    if mouse.just_pressed(MouseButton::Left) {
        elevators.placing = None;
        bar.show("Elevator put away");
    } else if mouse.just_pressed(MouseButton::Right) {
        match ok.and_then(|()| {
            build_from_ground(&mut elevators, &mut stations, &mut space, body, &plan, t)
        }) {
            Ok(line) => {
                elevators.placing = None;
                bar.show("Elevator built");
                chat.info(&line);
            }
            Err(e) => chat.error(&e),
        }
    }
}

// --- Collision ----------------------------------------------------------------------------------

/// What of an elevator the player bumps into, in the docked cabin's frame (origin under its
/// middle, +z its back, toward the ribbon): the cabin, and the ribbon with its anchor housing.
const SOLIDS: [(Vec3, Vec3); 3] = [
    (
        Vec3::new(-el::CABIN_HALF_X, 0.0, -el::CABIN_FRONT),
        Vec3::new(el::CABIN_HALF_X, el::CABIN_HEIGHT + 0.2, el::CABIN_BACK),
    ),
    (
        Vec3::new(-0.9, 0.0, CABIN_RIBBON_BACK - 0.2),
        Vec3::new(0.9, 1.55, CABIN_RIBBON_BACK + 1.05),
    ),
    (
        Vec3::new(-0.45, -40.0, CABIN_RIBBON_BACK - 0.2),
        Vec3::new(0.45, 60.0, CABIN_RIBBON_BACK + 0.2),
    ),
];

/// Keeps the player out of a docked cabin, its ribbon and its anchor: pushed out the shortest
/// way, or stood on top when coming down on them.
fn push_out(
    pos: &mut Vec3,
    vel: &mut Vec3,
    on_ground: &mut bool,
    at: Vec3,
    turn: Quat,
    solids: &[(Vec3, Vec3)],
) {
    use orbit_core::player::{HALF_WIDTH, HEIGHT};
    let inv = turn.inverse();
    let mut p = inv * (*pos - at);
    let mut v = inv * *vel;
    let mut moved = false;
    for &(lo, hi) in solids {
        let (lo, hi) = (
            lo - Vec3::new(HALF_WIDTH, HEIGHT, HALF_WIDTH),
            hi + Vec3::new(HALF_WIDTH, 0.0, HALF_WIDTH),
        );
        if p.cmple(lo).any() || p.cmpge(hi).any() {
            continue;
        }
        let up = hi.y - p.y;
        let ways = [
            (p.x - lo.x, 0, -1.0),
            (hi.x - p.x, 0, 1.0),
            (p.z - lo.z, 2, -1.0),
            (hi.z - p.z, 2, 1.0),
        ];
        let (d, axis, sign) = ways.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
        if up < 0.6 && up < d {
            p.y = hi.y;
            v.y = v.y.max(0.0);
            *on_ground = true;
        } else {
            p[axis] += d * sign;
            if v[axis] * sign < 0.0 {
                v[axis] = 0.0;
            }
        }
        moved = true;
    }
    if moved {
        *pos = at + turn * p;
        *vel = turn * v;
    }
}

fn collide(elevators: Res<Elevators>, mut player: Single<&mut Player>) {
    let b = &mut player.body;
    if b.cabin.is_some() {
        return;
    }
    for slot in &elevators.list {
        let Some(s) = slot.now else { continue };
        if slot.climb.moving {
            continue;
        }
        let (at, turn) = if slot.climb.up {
            // On its station's deck, in the station's frame.
            if b.station.as_deref() != Some(slot.e.station.as_str()) {
                continue;
            }
            let back = -el::front_dir(slot.e.front);
            (
                slot.e.dock(),
                Quat::from_mat3(&Mat3::from_cols(Vec3::Y.cross(back), Vec3::Y, back)),
            )
        } else {
            // On the ground, in the planet's frame.
            if b.station.is_some() || b.gravity_source != Some(s.planet) {
                continue;
            }
            s.cabin(slot.climb.h)
        };
        let (mut pos, mut vel, mut on) = (b.pos, b.vel, b.on_ground);
        push_out(&mut pos, &mut vel, &mut on, at, turn, &SOLIDS);
        (b.pos, b.vel, b.on_ground) = (pos, vel, on);
    }
}
