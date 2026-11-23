//! Travel between planets with the player's own thrusters: the autopilot flies the guidance in
//! `orbit_core::travel`, and the warp drive jumps through a hyperspace tunnel (visuals in `warp`).
//!
//! You travel from orbit: on a planet, launch to orbit first (a gravity turn into a circular
//! parking orbit), then pick a destination in the galaxy map (M). Flying there is a real burn:
//! out of the planet's sphere of influence, a cruise across space that speeds up and brakes,
//! and a capture burn into the destination's parking orbit. Warping starts the same way, breaking
//! orbit toward the destination; once under way the drive charges, the hyperspace tunnel opens
//! ahead, and you drop out of it near the destination still moving fast, to brake into its orbit. From orbit, Land comes down gently onto the surface.

use crate::player::Player;
use crate::world::{Clock, Space};
use bevy::math::DVec3;
use bevy::prelude::*;
use orbit_core::planet::{PlanetSpec, Terrain};
use orbit_core::player::{gravity_at, Body};
use orbit_core::travel::{circularize, circularized, cruise, cruise_time, descend, in_orbit, orbit_radius, PILOT_ACCEL};

/// Cruise thrust between planets, blocks/s², and the speed it arrives at its aim point with.
pub const CRUISE_ACCEL: f32 = 120.0;
const ARRIVE_SPEED: f32 = 60.0;
/// Warp: the speed (blocks/s) to be making toward the destination before the drive charges, the
/// least time spent breaking orbit first, and where and how fast you come out of hyperspace.
const WARP_ENTRY: f32 = 450.0;
const DEPART_MIN: f32 = 2.5;
const EXIT_DIST: f32 = 2600.0;
const EXIT_SPEED: f32 = 500.0;
/// Warp sequence, seconds per phase.
pub const SPOOL: f32 = 2.2;
pub const TUNNEL: f32 = 3.4;
pub const EXIT: f32 = 1.6;

pub struct TravelPlugin;

impl Plugin for TravelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Travel>()
            .init_resource::<Pilot>()
            .init_resource::<WarpFx>()
            .add_systems(Update, run_travel.after(crate::world::MovePlanets).before(crate::player::move_player));
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Warp,
    Fly,
}

/// A planet somewhere in the universe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    pub sector: IVec3,
    pub planet: u64,
}

#[derive(Clone, Copy, Debug)]
pub enum Request {
    Launch,
    Land,
    Go(Target, Mode),
    Cancel,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WarpPhase {
    /// Breaking orbit, burning toward the destination.
    Depart,
    /// The drive charges: stars stretch.
    Spool,
    /// In hyperspace.
    Tunnel,
    /// Back in normal space, streaks shrinking.
    Exit,
}

#[derive(Clone, Debug)]
pub enum Job {
    /// Up to a circular orbit of radius `target`.
    Launch { planet: u64, target: f32 },
    /// Down onto the ground below, or a rocket onto its pad: where the middle of its base goes
    /// and the way it faces there (planet-local).
    Land { planet: u64, pad: Option<(Vec3, Vec3)> },
    /// A rocket onto a ready landing pad of a station: over it, then down onto it.
    Dock { station: String, berth: orbit_core::rocket::Berth },
    /// `side` offsets the aim so the arrival is beside the planet, not into it; `capture` once
    /// the burn into orbit has begun.
    Cruise { target: Target, side: Vec3, top: f32, capture: bool },
    /// `dir`: the way the jump goes, fixed when the tunnel opens.
    Warp { target: Target, side: Vec3, phase: WarpPhase, t: f32, dir: Vec3 },
}

#[derive(Resource, Default)]
pub struct Travel {
    pub job: Option<Job>,
    pub requests: Vec<Request>,
    /// The last thing the autopilot has to say.
    pub message: String,
    /// A landing just touched down (a rocket comes apart then; see `rocket`).
    pub landed: bool,
    /// A rocket just came down on a station's pad: the station and how it stands there (it
    /// becomes blocks of the station then; see `rocket`).
    pub docked: Option<(String, orbit_core::rocket::Berth)>,
    /// While docking, the way the rocket's nose and front should point (world).
    pub align: Option<(Vec3, Vec3)>,
}

impl Travel {
    /// True in the part of a warp that cannot be stopped.
    pub fn locked(&self) -> bool {
        matches!(self.job, Some(Job::Warp { phase: WarpPhase::Spool | WarpPhase::Tunnel, .. }))
    }
}

/// What the autopilot asks of the player's physics this frame.
#[derive(Resource, Default)]
pub struct Pilot {
    pub thrust: Option<Vec3>,
    /// No physics at all (the warp moves the body itself).
    pub frozen: bool,
    /// Movement keys do nothing (they would cancel the autopilot).
    pub hands_off: bool,
}

/// The warp effect's state, for `warp`.
#[derive(Resource, Default)]
pub struct WarpFx {
    pub phase: Option<WarpPhase>,
    /// 0..1 through the phase.
    pub progress: f32,
}

/// A body's position and velocity relative to a planet's center, world-oriented.
pub fn relative(body: &Body, center: DVec3) -> (Vec3, Vec3) {
    (body.point_from(body.pos, center), body.world_vel())
}

/// Why the body cannot leave for another planet now, if it cannot.
pub fn travel_blocker(body: &Body, space: &Space) -> Option<String> {
    if body.gravity_source.is_some() {
        return Some("You must be in orbit to travel: launch to orbit first.".into());
    }
    let id = body.planet()?;
    let p = space.planets.get(&id)?;
    let (pos, vel) = relative(body, p.spec.center);
    (!in_orbit(pos, vel, p.spec.pull())).then(|| "Not in a stable orbit yet: launch to orbit (or circularize by hand).".into())
}

/// Estimated seconds to fly (cruise) from `from` to a planet.
pub fn fly_time(from: DVec3, spec: &PlanetSpec) -> f32 {
    let d = from.distance(spec.center) as f32;
    cruise_time(d, cruise_top(d), CRUISE_ACCEL) + 15.0
}

pub fn warp_time() -> f32 {
    6.0 + SPOOL + TUNNEL + EXIT + 12.0
}

/// Cruise speed for a trip of length `d`: long trips go faster.
fn cruise_top(d: f32) -> f32 {
    (d / 10.0).clamp(250.0, 5000.0)
}

/// Thrust to leave the planet we orbit toward `dir` at `speed`: steered around the planet if the
/// way lies through it, and against its pull. Plain thrust toward `dir` away from planets.
fn departure(body: &Body, space: &Space, dir: Vec3, speed: f32) -> Vec3 {
    let v = body.world_vel();
    let mut d = dir.normalize_or(Vec3::X);
    let mut hold = Vec3::ZERO;
    if let Some(from) = body.orbiting.and_then(|id| space.planets.get(&id)) {
        let (p, _) = relative(body, from.spec.center);
        let radial = p.normalize_or_zero();
        // Always climbing a little: never into the planet.
        d = (d - radial * (d.dot(radial) - 0.3).min(0.0)).normalize_or(radial);
        hold = radial * gravity_at(p.length(), from.spec.pull());
    }
    ((d * speed - v) * 1.5).clamp_length_max(CRUISE_ACCEL) + hold
}

/// Thrust to bring a rocket down onto its pad (`target`: where the middle of its base goes,
/// relative to the planet's center, world-oriented): across to over it, then down, slowing to a
/// couple of blocks a second at the end.
fn onto_pad(p: Vec3, v: Vec3, target: Vec3, pull: orbit_core::player::Pull) -> Vec3 {
    let r = p.length().max(1.0);
    let radial = p / r;
    let across = (target - p).reject_from(radial);
    let off = across.length();
    let alt = r - target.length();
    let sideways = across.normalize_or_zero() * (off * 0.6).min(40.0);
    // Down fast while high; low down, only once over the pad.
    let over = (1.0 - off / 20.0).clamp(0.0, 1.0);
    let sink = if alt > 60.0 { 1.0 } else { over };
    let down = (alt * 0.35).clamp(2.0, 50.0) * sink;
    let want = sideways - radial * down;
    ((want - v) * 1.5 + radial * gravity_at(r, pull)).clamp_length_max(PILOT_ACCEL)
}

/// The radius of the free lane nearest a rocket's parking orbit round planet `id`: where K takes
/// it, clear of every station and parked rocket.
fn free_lane(body: &Body, space: &Space, stations: &crate::station::Stations, id: u64, t: f64) -> Option<f32> {
    let p = space.planets.get(&id)?;
    let (lanes, pull) = crate::station::lanes_of(space, id, &stations.tethers)?;
    let (pos, vel) = relative(body, p.spec.center);
    let heading = if vel.reject_from(pos).length() > 1.0 { vel } else { body.world_dir(body.forward) };
    let at = pos.normalize_or(Vec3::Y) * orbit_radius(p.spec.gravity_radius());
    let want = orbit_core::station::StationOrbit::through(id, at, Quat::IDENTITY, heading, pull, t);
    let others = stations.orbits_around(id, None);
    orbit_core::station::find_slot(&want, &lanes, &others, crate::station::coasting(pull), None).ok().map(|o| o.radius())
}

/// The pad nearest below a rocket (station `rocket`) on planet `id`: where the middle of its base
/// would stand, and the way it would face (planet-local).
fn find_pad(body: &Body, space: &Space, stations: &crate::station::Stations, rocket: &str, id: u64) -> Option<(Vec3, Vec3)> {
    let p = space.planets.get(&id)?;
    let ground = p.voxel.as_ref()?.read().unwrap();
    let r = stations.by_id(rocket)?.station.read().unwrap();
    let below = p.spec.to_local(body.world_point(body.pos));
    // The way the rocket faces, not the camera.
    let face = p.spec.rotation.inverse() * r.flight.map_or(body.world_dir(body.forward), |f| f.rotation * Vec3::Z);
    let lay = orbit_core::rocket::find_pad(&ground, &r, below, face)?;
    Some((lay.base(&ground), lay.turn(&ground).z_axis))
}

/// How near a station a rocket must be for L to bring it down on one of its pads, blocks.
pub const DOCK_RANGE: f64 = 2500.0;

/// The nearest station (within `DOCK_RANGE`) with a ready landing pad where the rocket `rocket`
/// fits: the station's id and how the rocket would stand on it.
fn find_berth(body: &Body, space: &Space, stations: &crate::station::Stations, rocket: &str, t: f64) -> Option<(String, orbit_core::rocket::Berth)> {
    let r = stations.by_id(rocket)?.station.read().unwrap().clone();
    let here = body.world_point(body.pos);
    let front = r.flight.map_or(body.world_dir(body.forward), |f| f.rotation * Vec3::Z);
    let mut best: Option<(f64, String, orbit_core::rocket::Berth)> = None;
    for (s, f) in stations.placed(space, t) {
        let d = f.center.distance(here);
        if s.is_rocket() || s.flight.is_some() || d > DOCK_RANGE || best.as_ref().is_some_and(|b| b.0 < d) {
            continue;
        }
        let near = f.at(&s).to_local(here);
        if let Some(b) = orbit_core::rocket::find_berth(&s, &r, near, f.rotation.inverse() * front) {
            best = Some((d, s.id.clone(), b));
        }
    }
    best.map(|(_, id, b)| (id, b))
}

/// Thrust to bring a rocket onto a station's pad. `p`, `v`: where the middle of its base is and
/// how it moves, against the station (station coordinates); `target`: where it goes. Up to a
/// little over the pad's level first, across to over it, then down slowly.
fn onto_berth(p: Vec3, v: Vec3, target: Vec3) -> Vec3 {
    let d = target - p;
    let across = Vec3::new(d.x, 0.0, d.z);
    let off = across.length();
    let alt = p.y - target.y;
    const OVER: f32 = 8.0;
    let want_y = if off > 1.5 { ((OVER - alt) * 0.8).clamp(-40.0, 40.0) } else { -(alt * 0.5).clamp(0.8, 15.0) };
    // Across only once high enough not to sweep through the station (slower when low).
    let high = ((alt + 2.0) / OVER).clamp(0.15, 1.0);
    let sideways = across.normalize_or_zero() * (off * 0.5).min(120.0) * high;
    ((Vec3::new(sideways.x, want_y, sideways.z) - v) * 1.5).clamp_length_max(PILOT_ACCEL)
}

fn name_of(space: &Space, id: u64) -> String {
    space.planets.get(&id).map_or_else(|| "the planet".into(), |p| p.spec.name.clone())
}

#[allow(clippy::too_many_arguments)]
fn run_travel(
    mut travel: ResMut<Travel>,
    mut pilot: ResMut<Pilot>,
    mut fx: ResMut<WarpFx>,
    mut player: Single<&mut Player>,
    space: Res<Space>,
    clock: Res<Clock>,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    map: Res<crate::map::GalaxyMap>,
    rockets: Res<crate::rocket::Rockets>,
    stations: Res<crate::station::Stations>,
) {
    *pilot = Pilot::default();
    fx.phase = None;
    travel.align = None;
    let body = &mut player.body;
    let travel = &mut *travel;

    for req in std::mem::take(&mut travel.requests) {
        if travel.locked() {
            continue;
        }
        match req {
            Request::Cancel => {
                if travel.job.take().is_some() {
                    travel.message = "Autopilot off.".into();
                }
            }
            Request::Launch => match body.planet().and_then(|id| space.planets.get(&id)) {
                Some(slot) => {
                    let id = slot.spec.id;
                    body.flying = false;
                    let parking = orbit_radius(slot.spec.gravity_radius());
                    // A rocket goes up into a lane with room, so it meets no station.
                    let lane = rockets.is_piloting().then(|| free_lane(body, &space, &stations, id, clock.0)).flatten();
                    let target = lane.unwrap_or(parking);
                    travel.job = Some(Job::Launch { planet: id, target });
                    travel.message = match lane {
                        Some(r) => format!("Launching into a free orbit lane {:.0} blocks over {}...", r - slot.spec.radius, slot.spec.name),
                        None => format!("Launching to orbit around {}...", slot.spec.name),
                    };
                }
                None => travel.message = "No planet here to orbit.".into(),
            },
            Request::Land => match body.planet() {
                // A rocket out in orbit near a station with a pad comes down on that.
                _ if let Some((station, berth)) = body.gravity_source.is_none().then(|| rockets.piloting.as_ref().and_then(|p| find_berth(body, &space, &stations, &p.id, clock.0))).flatten() => {
                    let name = stations.by_id(&station).map_or_else(String::new, |s| s.station.read().unwrap().name.clone());
                    travel.message = format!("Coming down on a landing pad of {name}...");
                    travel.job = Some(Job::Dock { station, berth });
                }
                Some(id) => {
                    body.flying = false;
                    let pad = rockets.piloting.as_ref().and_then(|p| find_pad(body, &space, &stations, &p.id, id));
                    travel.job = Some(Job::Land { planet: id, pad });
                    travel.message = match (rockets.is_piloting(), pad) {
                        (true, Some(_)) => format!("Coming down onto a pad on {}...", name_of(&space, id)),
                        (true, None) => format!("No landing pad or flat 3x3 below: coming straight down on {}...", name_of(&space, id)),
                        _ => format!("Descending to {}...", name_of(&space, id)),
                    };
                }
                None => travel.message = "No planet below to land on.".into(),
            },
            Request::Go(target, mode) => {
                if let Some(why) = travel_blocker(body, &space) {
                    travel.message = why;
                } else if body.planet() == Some(target.planet) {
                    travel.message = "You are already orbiting it.".into();
                } else if let Some(spec) = space.planet_spec(target.sector, target.planet, clock.0) {
                    let here = body.world_point(body.pos);
                    let dir = (spec.center - here).as_vec3().normalize_or(Vec3::X);
                    let side = dir.cross(spec.orbit.tilt * Vec3::Y).normalize_or(dir.any_orthonormal_vector());
                    travel.job = Some(match mode {
                        Mode::Fly => Job::Cruise { target, side, top: cruise_top(here.distance(spec.center) as f32), capture: false },
                        Mode::Warp => Job::Warp { target, side, phase: WarpPhase::Depart, t: 0.0, dir },
                    });
                    travel.message = match mode {
                        Mode::Fly => format!("Burning for {}...", spec.name),
                        Mode::Warp => format!("Warp to {}: breaking orbit...", spec.name),
                    };
                }
            }
        }
    }

    // Taking the controls switches the autopilot off (not mid-warp).
    let manual = !map.is_open() && [KeyCode::KeyW, KeyCode::KeyA, KeyCode::KeyS, KeyCode::KeyD, KeyCode::Space, KeyCode::ShiftLeft, KeyCode::KeyX].iter().any(|k| keys.just_pressed(*k));
    if manual && travel.job.is_some() && !travel.locked() && !matches!(travel.job, Some(Job::Warp { phase: WarpPhase::Exit, .. })) {
        travel.job = None;
        travel.message = "Manual control.".into();
    }

    let dt = time.delta_secs().min(0.1);
    let mut next = None;
    let Some(job) = travel.job.as_mut() else { return };
    let (done, message): (bool, Option<String>) = match job {
        Job::Launch { planet, target } => match space.planets.get(planet) {
            None => (true, Some("Lost the planet.".into())),
            Some(slot) => {
                let pull = slot.spec.pull();
                let (p, v) = relative(body, slot.spec.center);
                if circularized(p, v, *target, pull) {
                    let rocket = if rockets.is_piloting() { " R to park the rocket here," } else { "" };
                    (true, Some(format!("In orbit around {}.{rocket} Open the map (M) to travel, L to land.", slot.spec.name)))
                } else {
                    pilot.thrust = Some(circularize(p, v, *target, pull, body.world_dir(body.forward), PILOT_ACCEL));
                    pilot.hands_off = true;
                    (false, None)
                }
            }
        },
        Job::Land { planet, pad } => match space.planets.get(planet) {
            None => (true, Some("Lost the planet.".into())),
            Some(_) if body.on_ground => {
                travel.landed = true;
                (true, Some(format!("Landed on {}.", name_of(&space, *planet))))
            }
            Some(slot) if pad.is_some() => {
                let (base, face) = pad.unwrap();
                let spec = &slot.spec;
                let (p, v) = relative(body, spec.center);
                pilot.thrust = Some(onto_pad(p, v, spec.rotation * base, spec.pull()));
                pilot.hands_off = true;
                // Turned to stand square on the pad.
                body.look_toward(spec.rotation * face, 1.5, dt);
                (false, None)
            }
            Some(slot) => {
                let spec = &slot.spec;
                let (p, v) = relative(body, spec.center);
                let below = (spec.rotation.inverse() * p).normalize_or(Vec3::Y);
                let ground = spec.radius + Terrain::new(spec).column(below).ground() as f32 + 1.0;
                pilot.thrust = Some(descend(p, v, ground, spec.pull(), PILOT_ACCEL));
                pilot.hands_off = true;
                (false, None)
            }
        },
        Job::Dock { station, berth } => {
            let at = stations.by_id(station).and_then(|sl| {
                let s = sl.station.read().unwrap();
                crate::station::Stations::frame(&s, &space, clock.0)
            });
            match at {
                None => (true, Some("Lost the station.".into())),
                Some(f) => {
                    let world = body.world_point(body.pos);
                    let inv = f.rotation.inverse();
                    let p = inv * (world - f.center).as_vec3();
                    // The station goes round its planet as one rigid thing: this is how fast
                    // its point where the rocket is moves.
                    let carried = f.spin.cross((world - f.planet_center).as_vec3());
                    let v = inv * (body.world_vel() - carried);
                    let target = berth.point(Vec3::new(0.5, 0.0, 0.5));
                    travel.align = Some((f.rotation * Vec3::Y, f.rotation * (berth.rotation() * Vec3::Z)));
                    pilot.hands_off = true;
                    if p.distance(target) < 0.4 && v.length() < 1.5 {
                        travel.docked = Some((station.clone(), *berth));
                        (true, None)
                    } else if p.distance(target) as f64 > DOCK_RANGE * 1.5 {
                        (true, Some("Too far from the station: docking off.".into()))
                    } else {
                        pilot.thrust = Some(f.rotation * onto_berth(p, v, target));
                        (false, None)
                    }
                }
            }
        }
        Job::Cruise { target, side, top, capture } => match space.planet_spec(target.sector, target.planet, clock.0) {
            None => (true, Some("Lost the destination.".into())),
            Some(ts) => {
                let gr = ts.gravity_radius();
                let pull = ts.pull();
                let world = body.world_point(body.pos);
                let aim = ts.center + (*side * orbit_radius(gr)).as_dvec3();
                // Offsets as f32 once taken in f64: small or not, they are exact enough.
                let to_aim = (aim - world).as_vec3();
                let v = body.world_vel();
                if !*capture && (body.planet() == Some(target.planet) || world.distance(aim) < 150.0) {
                    *capture = true;
                }
                pilot.hands_off = true;
                if *capture {
                    let (p, v) = relative(body, ts.center);
                    if circularized(p, v, orbit_radius(gr), pull) {
                        (true, Some(format!("Arrived: in orbit around {}. L to land.", ts.name)))
                    } else {
                        pilot.thrust = Some(circularize(p, v, orbit_radius(gr), pull, -*side, PILOT_ACCEL * 1.5));
                        body.look_toward((ts.center - world).as_vec3(), 1.0, dt);
                        (false, None)
                    }
                } else {
                    // Out of the planet we leave first, then the cruise proper.
                    pilot.thrust = Some(if body.orbiting.is_some() {
                        departure(body, &space, to_aim, top.min(400.0))
                    } else {
                        cruise(to_aim, v, ARRIVE_SPEED, *top, CRUISE_ACCEL)
                    });
                    body.look_toward(if v.length() > 20.0 { v } else { to_aim }, 1.2, dt);
                    (false, None)
                }
            }
        },
        Job::Warp { target, side, phase, t, dir } => match space.planet_spec(target.sector, target.planet, clock.0) {
            None => (true, Some("Lost the destination.".into())),
            Some(ts) => {
                *t += dt;
                let world = body.world_point(body.pos);
                let v = body.world_vel();
                let aim = ts.center + (*side * orbit_radius(ts.gravity_radius())).as_dvec3();
                let to_aim = (aim - world).as_vec3();
                let up = Vec3::Y.reject_from(*dir).normalize_or(dir.any_orthonormal_vector());
                let mut message = None;
                pilot.hands_off = true;
                match *phase {
                    WarpPhase::Depart => {
                        pilot.thrust = Some(departure(body, &space, to_aim, WARP_ENTRY + 250.0));
                        body.look_toward(if v.length() > 30.0 { v } else { to_aim }, 1.5, dt);
                        let clear = body.gravity_source.is_none() && (body.orbiting.is_none() || *t > 12.0);
                        if clear && *t > DEPART_MIN && v.length() > WARP_ENTRY {
                            (*phase, *t) = (WarpPhase::Spool, 0.0);
                            message = Some(format!("Warp to {}: charging the drive...", ts.name));
                        }
                    }
                    WarpPhase::Spool => {
                        // Still burning while the drive charges.
                        pilot.thrust = Some(departure(body, &space, to_aim, WARP_ENTRY + 250.0));
                        body.look_toward(v, 2.0, dt);
                        fx.progress = *t / SPOOL;
                        if *t > SPOOL {
                            *dir = body.world_dir(body.look_dir()).normalize_or(to_aim.normalize_or(Vec3::X));
                            (*phase, *t) = (WarpPhase::Tunnel, 0.0);
                            message = Some(format!("Hyperspace: {}...", ts.name));
                        }
                    }
                    WarpPhase::Tunnel => {
                        // Already where we will come out, so the destination streams in unseen.
                        // The jump bends toward the destination while in the tunnel.
                        let toward = to_aim.normalize_or(*dir);
                        *dir = dir.slerp(toward, (dt * 2.0).min(1.0)).normalize();
                        pilot.frozen = true;
                        // Out of hyperspace this far short of the destination, on the line of the jump.
                        let exit = aim - (*dir * EXIT_DIST).as_dvec3();
                        body.place(exit, Vec3::ZERO, up, *dir, 0.0);
                        fx.progress = *t / TUNNEL;
                        if *t > TUNNEL {
                            body.place(exit, *dir * EXIT_SPEED, up, *dir, 0.0);
                            (*phase, *t) = (WarpPhase::Exit, 0.0);
                        }
                    }
                    WarpPhase::Exit => {
                        pilot.thrust = Some(cruise(to_aim, v, ARRIVE_SPEED, EXIT_SPEED, CRUISE_ACCEL));
                        body.look_toward(to_aim, 1.0, dt);
                        fx.progress = *t / EXIT;
                        if *t > EXIT {
                            next = Some(Job::Cruise { target: *target, side: *side, top: EXIT_SPEED, capture: false });
                            message = Some(format!("Out of hyperspace: braking into orbit around {}...", ts.name));
                        }
                    }
                }
                fx.phase = Some(*phase).filter(|p| *p != WarpPhase::Depart);
                (false, message)
            }
        },
    };
    if let Some(m) = message {
        travel.message = m;
    }
    if let Some(j) = next {
        travel.job = Some(j);
    } else if done {
        travel.job = None;
    }
}
