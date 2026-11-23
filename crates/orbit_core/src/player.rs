//! Player movement: walking on spherical planets with radial gravity, a jetpack, free flight, and
//! Newtonian flight in space: orbits around planets, and open space between them. Pure logic so a
//! server can run it too.
//!
//! Three frames, by where the body is:
//! - **Surface** (inside a planet's gravity radius, its atmosphere): the planet's own orbiting,
//!   spinning frame, so standing still is exactly still. Gravity is constant; the air drags.
//! - **Orbit** (inside the planet's sphere of influence, above the atmosphere): a frame that moves
//!   with the planet's center but does not spin. Gravity falls off with the square of the distance
//!   and nothing drags, so orbits are real ellipses (see `travel`).
//! - **Station** (inside a station's gravity, see `station`): the station's own frame, which
//!   orbits its planet and turns with it. Gravity pulls toward the station's floor (-y), and
//!   collision is a plain box against its blocks, as in Minecraft.
//! - **Cabin** (riding a space elevator, see `elevator`): the planet's turning frame again, out
//!   to the station, where the ribbon is still; the cabin carries the body, and the frame's turn
//!   counts in its velocity, so stepping out at the top leaves you going round with the station.
//! - **Space** (anywhere else): world space, no gravity, no drag.
//!
//! Gravity is continuous across the atmosphere's edge, so falling in from orbit or climbing out has
//! no jolt. World positions are f64 (`DVec3`) and only anchor frames: everything the body does is
//! f32 relative to its frame's anchor, which in open space follows the body (re-anchored every
//! `ANCHOR_SPAN` blocks), so its coordinates stay small and exact anywhere in the universe. Collision is Minecraft's box against blocks, done in
//! cell space: the box spans the grid coordinates (a, b) of the cube face under it and the radius,
//! and moves axis by axis, stopping on integer cell boundaries.

use crate::block::{self, Liquid};
use crate::planet::Planet;
use crate::station::Station;
use glam::{DVec3, Quat, Vec2, Vec3};

/// Half the box width, in cells. Cells narrow with depth and so does the box, so it always fits
/// a 1×1 shaft.
pub const HALF_WIDTH: f32 = 0.3;
pub const HEIGHT: f32 = 1.8;
pub const EYE: f32 = 1.62;
pub const REACH: f32 = 5.0;

/// Gravity inside a planet's atmosphere, blocks/s².
pub const GRAVITY: f32 = 28.0;
const JUMP_SPEED: f32 = 9.0;
const WALK: f32 = 4.3;
const SPRINT: f32 = 5.6;
const FLY: f32 = 11.0;
/// Swimming speed, and how fast Space swims up.
const SWIM: f32 = 3.0;
const SWIM_UP: f32 = 4.0;
/// Thrusters in vacuum, and the factor Ctrl boosts them by.
const THRUST: f32 = 18.0;
const BOOST: f32 = 10.0;
/// Jetpack in the atmosphere (on top of holding the body up against gravity), and its boost.
const JET: f32 = 22.0;
const JET_BOOST: f32 = 5.0;
/// Deceleration of the brake (X), blocks/s².
const BRAKE: f32 = 40.0;
/// Terminal fall speed at the surface: the air's drag balances gravity there.
const MAX_FALL: f32 = 60.0;
const AIR_DRAG: f32 = GRAVITY / (MAX_FALL * MAX_FALL);
/// The sphere of influence reaches this many times the gravity radius: far enough that a
/// synchronous orbit (one turn a day, for space elevators) fits inside for days of up to about
/// ten minutes. Orbits leave room for two of them between neighbors (`universe`).
pub const SOI_SCALE: f32 = 13.0;
/// Over this last share of the sphere of influence gravity fades to nothing.
const SOI_TAPER: f32 = 0.06;
/// Overlaps thinner than this (cells, or blocks radially) do not count: float dust at a contact.
const SKIN: f64 = 1e-3;
/// Longest move per collision substep.
const SUBSTEP: f64 = 0.25;
/// In open space the frame's anchor moves to the body once it is this far from it.
pub const ANCHOR_SPAN: f32 = 256.0;
/// Ledges this high (a trapdoor, a slab of ground) are walked up onto without jumping.
const STEP_HEIGHT: f64 = 0.6;
/// Climbing a ladder (pushing forward or Space), and the fastest you slide down one.
const CLIMB: f32 = 2.4;
const CLIMB_FALL: f32 = 3.0;

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Radius of the sphere of influence of a planet with this gravity radius.
pub fn soi(gravity_radius: f32) -> f32 {
    gravity_radius * SOI_SCALE
}

/// The last stretch of the sphere of influence, where gravity fades out.
pub fn soi_taper(gravity_radius: f32) -> f32 {
    soi(gravity_radius) * SOI_TAPER
}

/// A body's gravity: constant in its atmosphere (out to `gravity_radius`), falling off with the
/// square of the distance past it, fading to nothing at the edge of its sphere of influence. A
/// planet's reaches `soi(gravity_radius)`; a moon's ends sooner, where its planet's pull wins.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pull {
    pub gravity_radius: f32,
    pub soi: f32,
}

impl Pull {
    /// A planet's: its sphere of influence as far as `soi` says.
    pub fn planet(gravity_radius: f32) -> Self {
        Self {
            gravity_radius,
            soi: soi(gravity_radius),
        }
    }

    /// The last stretch of the sphere of influence, where gravity fades out.
    pub fn taper(&self) -> f32 {
        self.soi * SOI_TAPER
    }
}

/// Pull toward a body's center at distance `r` (blocks/s²).
pub fn gravity_at(r: f32, pull: Pull) -> f32 {
    if r <= pull.gravity_radius {
        return GRAVITY;
    }
    let k = pull.gravity_radius / r;
    GRAVITY * k * k * (1.0 - smoothstep(pull.soi - pull.taper(), pull.soi, r))
}

/// Air density at `r`: 1 at the mean surface (and below), thinning to 0 at the atmosphere's edge.
fn air_density(r: f32, radius: f32, gravity_radius: f32) -> f32 {
    let x = ((gravity_radius - r) / (gravity_radius - radius)).clamp(0.0, 1.0);
    x * x
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Input {
    /// x = right, y = forward, each in [-1, 1].
    pub movement: Vec2,
    pub jump: bool,
    pub descend: bool,
    pub sprint: bool,
    /// Kill velocity (in the current frame) with the thrusters.
    pub brake: bool,
    /// Autopilot: an acceleration to thrust with, world-oriented, blocks/s². Gravity still
    /// applies on top of it.
    pub thrust: Option<Vec3>,
}

/// A planet where it is now. Its blocks are planet-local and stay put; the frame (center and
/// rotation, world space) moves every frame, so it is passed apart from the block data.
#[derive(Clone, Copy)]
pub struct Located<'a> {
    pub planet: &'a Planet,
    pub center: DVec3,
    pub rotation: Quat,
}

impl<'a> Located<'a> {
    /// At the frame its spec holds.
    pub fn of(planet: &'a Planet) -> Self {
        Self {
            planet,
            center: planet.spec.center,
            rotation: planet.spec.rotation,
        }
    }
}

/// A station where it is now: its blocks, its frame (the station origin and its turn, world),
/// and how that frame moves around its planet.
#[derive(Clone, Copy)]
pub struct StationAt<'a> {
    pub station: &'a Station,
    pub center: DVec3,
    pub rotation: Quat,
    pub planet: u64,
    pub planet_center: DVec3,
    /// Angular velocity about the planet's center, world, rad/s.
    pub spin: Vec3,
}

impl StationAt<'_> {
    /// A world point in station coordinates.
    pub fn to_local(&self, world: DVec3) -> Vec3 {
        self.rotation.inverse() * (world - self.center).as_vec3()
    }

    /// A point in station coordinates, in world space.
    pub fn to_world(&self, local: Vec3) -> DVec3 {
        self.center + (self.rotation * local).as_dvec3()
    }
}

/// An elevator's cabin where it is now, for the body riding it: the planet's frame (center and
/// rotation, world) and how it turns (angular velocity, world) and moves (a moon's drift), and in
/// that frame where the rider stands, the cabin's up and its velocity (planet-local).
#[derive(Clone, Debug)]
pub struct CabinAt {
    pub elevator: String,
    pub planet: u64,
    pub center: DVec3,
    pub rotation: Quat,
    pub spin: Vec3,
    pub drift: Vec3,
    pub feet: Vec3,
    pub up: Vec3,
    pub vel: Vec3,
}

/// A planet's gravity well, as seen from orbit: no blocks needed.
#[derive(Clone, Copy, Debug)]
pub struct Well {
    pub id: u64,
    pub center: DVec3,
    pub radius: f32,
    pub pull: Pull,
    /// How fast its center moves, world: around its planet for a moon; zero for a planet (their
    /// slow way round their star is not counted).
    pub vel: Vec3,
}

impl Well {
    pub fn soi(&self) -> f32 {
        self.pull.soi
    }
}

impl From<&Located<'_>> for Well {
    fn from(p: &Located) -> Self {
        Self {
            id: p.planet.spec.id,
            center: p.center,
            radius: p.planet.spec.radius,
            pull: p.planet.spec.pull(),
            vel: p.planet.spec.velocity,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    /// Feet position. Like `vel`, `up` and `forward`, it is in the body's frame (see the module
    /// docs): planet-local on a surface, relative to the planet's center in orbit, relative to a
    /// nearby anchor in space. Always small numbers.
    pub pos: Vec3,
    pub vel: Vec3,
    pub up: Vec3,
    /// Horizontal facing, always perpendicular to `up`.
    pub forward: Vec3,
    /// Look angle above the horizon, radians.
    pub pitch: f32,
    pub on_ground: bool,
    pub flying: bool,
    /// Jetpack in the atmosphere: Space thrusts up instead of jumping, and it holds you up.
    pub jetpack: bool,
    /// Walking into a one-block ledge climbs it. Off by default.
    pub auto_step: bool,
    /// The liquid the body is in (at its middle), and the one at its eyes (for the view).
    pub swimming: Option<Liquid>,
    pub eyes_in: Option<Liquid>,
    /// Something solid right ahead at the waist: Space climbs out of the water onto it.
    ledge: bool,
    /// On a ladder: forward or Space climbs it, Shift holds on, else you slide down slowly.
    pub climbing: bool,
    /// The planet whose atmosphere (and spinning frame) the body is in.
    pub gravity_source: Option<u64>,
    /// The planet whose sphere of influence (and orbit frame) the body is in, above its atmosphere.
    pub orbiting: Option<u64>,
    /// The station whose gravity (and frame) the body is in, and the planet it orbits.
    pub station: Option<String>,
    pub station_planet: Option<u64>,
    /// The elevator whose cabin carries the body (its station's id), and its planet.
    pub cabin: Option<String>,
    pub cabin_planet: Option<u64>,
    /// That frame's anchor (world) and rotation at the last step; the rotation is identity in
    /// orbit and in space.
    frame: (DVec3, Quat),
    /// How the frame moves against its planet's orbit frame: angular velocity (world) about a
    /// world point. Zero but on a station.
    spin: (Vec3, DVec3),
    /// How fast the frame's body moves (a moon round its planet; zero for planets): kept when
    /// the body goes from one frame to another, so going into a moon's pull or out of it does
    /// not change how fast you go.
    drift: Vec3,
}

impl Body {
    /// A body in open space at a world position; `up` is world space.
    pub fn new(pos: DVec3, up: Vec3) -> Self {
        let up = up.normalize();
        Self {
            pos: Vec3::ZERO,
            vel: Vec3::ZERO,
            up,
            forward: up.any_orthonormal_vector(),
            pitch: 0.0,
            on_ground: false,
            flying: false,
            jetpack: false,
            auto_step: false,
            swimming: None,
            eyes_in: None,
            ledge: false,
            climbing: false,
            gravity_source: None,
            orbiting: None,
            station: None,
            station_planet: None,
            cabin: None,
            cabin_planet: None,
            frame: (pos, Quat::IDENTITY),
            spin: (Vec3::ZERO, DVec3::ZERO),
            drift: Vec3::ZERO,
        }
    }

    pub fn right(&self) -> Vec3 {
        self.forward.cross(self.up)
    }

    /// Eye position, in the body's frame.
    pub fn eye(&self) -> Vec3 {
        self.pos + self.up * EYE
    }

    /// Look direction, in the body's frame.
    pub fn look_dir(&self) -> Vec3 {
        Quat::from_axis_angle(self.right(), self.pitch) * self.forward
    }

    /// A point in the body's frame, in world space.
    pub fn world_point(&self, p: Vec3) -> DVec3 {
        self.frame.0 + (self.frame.1 * p).as_dvec3()
    }

    /// A direction in the body's frame, in world space.
    pub fn world_dir(&self, d: Vec3) -> Vec3 {
        self.frame.1 * d
    }

    /// Where the body's frame is anchored (world): its planet's center, or a point near it in space.
    pub fn frame_center(&self) -> DVec3 {
        self.frame.0
    }

    /// The body's eye relative to a world point (small numbers: for rendering around `origin`).
    pub fn eye_from(&self, origin: DVec3) -> Vec3 {
        (self.frame.0 - origin).as_vec3() + self.frame.1 * self.eye()
    }

    /// A point in the body's frame relative to a world point.
    pub fn point_from(&self, p: Vec3, origin: DVec3) -> Vec3 {
        (self.frame.0 - origin).as_vec3() + self.frame.1 * p
    }

    /// The planet whose frame the body is in (surface or orbit), or whose station it is on.
    pub fn planet(&self) -> Option<u64> {
        self.gravity_source
            .or(self.orbiting)
            .or(self.station_planet)
            .or(self.cabin_planet)
    }

    /// The body's velocity against its planet's orbit frame (or space), world-oriented: its own
    /// plus how its frame carries it (a station going round).
    pub fn world_vel(&self) -> Vec3 {
        self.world_dir(self.vel) + self.frame_motion(self.pos)
    }

    /// How fast the frame carries a point of it (in the frame) against the planet's orbit frame.
    fn frame_motion(&self, p: Vec3) -> Vec3 {
        let (w, pivot) = self.spin;
        if w == Vec3::ZERO {
            return Vec3::ZERO;
        }
        w.cross(self.point_from(p, pivot))
    }

    /// Applies mouse look (radians).
    pub fn turn(&mut self, yaw: f32, pitch: f32) {
        self.forward = (Quat::from_axis_angle(self.up, -yaw) * self.forward).normalize();
        self.pitch = (self.pitch - pitch).clamp(-1.55, 1.55);
    }

    /// Turns the view toward a world direction, `rate` per second (exponential), keeping `up`.
    pub fn look_toward(&mut self, world_dir: Vec3, rate: f32, dt: f32) {
        let d = (self.frame.1.inverse() * world_dir).normalize_or_zero();
        let flat = d.reject_from(self.up).normalize_or_zero();
        if flat == Vec3::ZERO {
            return;
        }
        let k = 1.0 - (-rate * dt).exp();
        self.forward = self
            .forward
            .slerp(flat, k)
            .reject_from(self.up)
            .normalize_or(flat);
        let pitch = d.dot(self.up).clamp(-1.0, 1.0).asin();
        self.pitch += (pitch - self.pitch) * k;
    }

    /// Turns `up` toward a world direction, `rate` per second (exponential).
    pub fn lean_toward(&mut self, world_up: Vec3, rate: f32, dt: f32) {
        let u = (self.frame.1.inverse() * world_up).normalize_or(self.up);
        self.set_up(self.up.slerp(u, 1.0 - (-rate * dt).exp()).normalize());
    }

    fn set_up(&mut self, up: Vec3) {
        if self.up.dot(up) > 1.0 - 1e-7 {
            return;
        }
        let rot = Quat::from_rotation_arc(self.up, up);
        self.up = up;
        self.forward = (rot * self.forward)
            .reject_from(up)
            .normalize_or(up.any_orthonormal_vector());
    }

    /// Puts the body somewhere in open space (world position and velocity): a teleport. The next
    /// step picks the frame it is in.
    pub fn place(&mut self, pos: DVec3, vel: Vec3, up: Vec3, forward: Vec3, pitch: f32) {
        self.frame = (pos, Quat::IDENTITY);
        self.gravity_source = None;
        self.orbiting = None;
        self.station = None;
        self.station_planet = None;
        self.cabin = None;
        self.cabin_planet = None;
        self.spin = (Vec3::ZERO, DVec3::ZERO);
        self.drift = Vec3::ZERO;
        self.pos = Vec3::ZERO;
        self.vel = vel;
        self.up = up.normalize();
        self.forward = forward
            .reject_from(self.up)
            .normalize_or(self.up.any_orthonormal_vector());
        self.pitch = pitch;
        self.on_ground = false;
        self.flying = false;
    }

    /// A body in a planet's own (turning) frame, at `feet` (planet-local), still against the
    /// ground; `up` and `forward` planet-local.
    pub fn on_planet(p: &Located, feet: Vec3, up: Vec3, forward: Vec3) -> Self {
        let mut b = Self::new(p.center, Vec3::Y);
        b.frame = (p.center, p.rotation);
        b.gravity_source = Some(p.planet.spec.id);
        b.drift = p.planet.spec.velocity;
        b.pos = feet;
        b.up = up.normalize();
        b.forward = forward
            .reject_from(b.up)
            .normalize_or(b.up.any_orthonormal_vector());
        b
    }

    /// A body in a planet's orbit frame: `rel` from its center and moving at `vel` against it,
    /// both world-oriented; `up` and `forward` world.
    pub fn in_orbit_of(w: &Well, rel: Vec3, vel: Vec3, up: Vec3, forward: Vec3) -> Self {
        let mut b = Self::new(w.center, Vec3::Y);
        b.frame = (w.center, Quat::IDENTITY);
        b.orbiting = Some(w.id);
        b.drift = w.vel;
        b.pos = rel;
        b.vel = vel;
        b.up = up.normalize();
        b.forward = forward
            .reject_from(b.up)
            .normalize_or(b.up.any_orthonormal_vector());
        b
    }

    /// The turn of the body's frame (frame → world): the planet's on a surface, none elsewhere
    /// but on a station.
    pub fn frame_rotation(&self) -> Quat {
        self.frame.1
    }

    /// Puts the body on a station, at a point in its coordinates, standing still on it and
    /// facing `forward` (station coordinates): a teleport.
    pub fn board(&mut self, st: &StationAt, feet: Vec3, forward: Vec3) {
        self.frame = (st.center, st.rotation);
        self.spin = (st.spin, st.planet_center);
        self.gravity_source = None;
        self.orbiting = None;
        self.station = Some(st.station.id.clone());
        self.station_planet = Some(st.planet);
        self.cabin = None;
        self.cabin_planet = None;
        self.pos = feet;
        self.vel = Vec3::ZERO;
        self.up = Vec3::Y;
        self.forward = forward.reject_from(Vec3::Y).normalize_or(Vec3::Z);
        self.pitch = 0.0;
        self.on_ground = false;
        self.flying = false;
    }

    /// Follows the frame's planet or station to where it is now. `step` does this itself; call
    /// it when skipping a step but still rendering.
    pub fn sync_frame(&mut self, planets: &[Located], wells: &[Well], stations: &[StationAt]) {
        if self.cabin.is_some() {
            return; // `ride` moves the cabin's frame
        }
        if let Some(st) = self
            .station
            .as_ref()
            .and_then(|id| stations.iter().find(|s| &s.station.id == id))
        {
            self.frame = (st.center, st.rotation);
            self.spin = (st.spin, st.planet_center);
        } else if let Some(p) = self
            .gravity_source
            .and_then(|id| planets.iter().find(|p| p.planet.spec.id == id))
        {
            self.frame = (p.center, p.rotation);
        } else if let Some(w) = self
            .orbiting
            .and_then(|id| wells.iter().find(|w| w.id == id))
        {
            self.frame = (w.center, Quat::IDENTITY);
        }
    }

    /// Re-expresses the body in another frame, keeping its motion: what the old frame carried it
    /// at becomes its own velocity, and what the new one carries it at is taken off.
    fn enter_frame(
        &mut self,
        surface: Option<u64>,
        orbiting: Option<u64>,
        frame: (DVec3, Quat),
        drift: Vec3,
    ) {
        self.enter_any(
            surface,
            orbiting,
            None,
            frame,
            (Vec3::ZERO, DVec3::ZERO),
            drift,
        );
    }

    fn enter_any(
        &mut self,
        surface: Option<u64>,
        orbiting: Option<u64>,
        station: Option<&StationAt>,
        frame: (DVec3, Quat),
        spin: (Vec3, DVec3),
        drift: Vec3,
    ) {
        let (pos, vel, up, fwd) = (
            self.world_point(self.pos),
            self.world_vel() + self.drift - drift,
            self.world_dir(self.up),
            self.world_dir(self.forward),
        );
        let inv = frame.1.inverse();
        self.frame = frame;
        self.spin = spin;
        self.drift = drift;
        self.pos = inv * (pos - frame.0).as_vec3();
        self.vel = inv * vel;
        self.vel -= inv * self.frame_motion(self.pos);
        self.up = (inv * up).normalize();
        self.forward = (inv * fwd)
            .reject_from(self.up)
            .normalize_or(self.up.any_orthonormal_vector());
        self.gravity_source = surface;
        self.orbiting = orbiting;
        self.station = station.map(|s| s.station.id.clone());
        self.station_planet = station.map(|s| s.planet);
        self.cabin = None;
        self.cabin_planet = None;
    }

    /// Rides an elevator's cabin: the body stands where `cabin` says, carried along. The first
    /// call takes it into the cabin's frame (the planet's turning frame), keeping how it looks.
    pub fn ride(&mut self, cabin: &CabinAt, dt: f32) {
        if self.cabin.as_deref() != Some(cabin.elevator.as_str()) {
            self.enter_any(
                None,
                None,
                None,
                (cabin.center, cabin.rotation),
                (cabin.spin, cabin.center),
                cabin.drift,
            );
            self.cabin = Some(cabin.elevator.clone());
            self.cabin_planet = Some(cabin.planet);
        }
        self.frame = (cabin.center, cabin.rotation);
        self.spin = (cabin.spin, cabin.center);
        self.drift = cabin.drift;
        self.pos = cabin.feet;
        self.vel = cabin.vel;
        self.set_up(self.up.slerp(cabin.up, 1.0 - (-8.0 * dt).exp()).normalize());
        self.on_ground = true;
        self.flying = false;
    }

    /// Steps out onto a planet's ground (its frame: `center`, `rotation`), feet at `feet`
    /// (planet-local), standing still on it.
    pub fn stand_on(
        &mut self,
        planet: u64,
        center: DVec3,
        rotation: Quat,
        drift: Vec3,
        feet: Vec3,
    ) {
        self.enter_frame(Some(planet), None, (center, rotation), drift);
        self.pos = feet;
        self.vel = Vec3::ZERO;
        self.on_ground = false;
    }

    /// One step with the given voxel planets as the only gravity wells.
    pub fn step(&mut self, input: Input, planets: &[Located], dt: f32) {
        let wells: Vec<Well> = planets.iter().map(Well::from).collect();
        self.step_with(input, planets, &wells, &[], dt);
    }

    /// One step. `planets` are those with blocks loaded (for walking), `wells` every planet near
    /// enough to orbit, loaded or not, and `stations` those near enough to stand on.
    pub fn step_with(
        &mut self,
        input: Input,
        planets: &[Located],
        wells: &[Well],
        stations: &[StationAt],
        dt: f32,
    ) {
        self.sync_frame(planets, wells, stations);
        (self.swimming, self.eyes_in) = (None, None); // set again where there are blocks
        let world = self.world_point(self.pos);
        // A station's gravity first: stations orbit above atmospheres, inside spheres of influence.
        let on = stations
            .iter()
            .find(|s| {
                self.station.as_ref() == Some(&s.station.id)
                    && s.station.in_gravity(s.to_local(world))
            })
            .or_else(|| {
                stations
                    .iter()
                    .find(|s| s.station.in_gravity(s.to_local(world)))
            });
        if let Some(st) = on {
            if self.station.as_ref() != Some(&st.station.id) {
                let drift = wells
                    .iter()
                    .find(|w| w.id == st.planet)
                    .map_or(Vec3::ZERO, |w| w.vel);
                self.enter_any(
                    None,
                    None,
                    Some(st),
                    (st.center, st.rotation),
                    (st.spin, st.planet_center),
                    drift,
                );
            }
            self.step_station(input, st.station, dt);
            return;
        }
        let surface = planets
            .iter()
            .filter(|p| world.distance(p.center) < p.planet.spec.gravity_radius() as f64)
            .min_by(|a, b| {
                (world.distance(a.center) - a.planet.spec.radius as f64)
                    .total_cmp(&(world.distance(b.center) - b.planet.spec.radius as f64))
            });
        if let Some(p) = surface {
            let id = Some(p.planet.spec.id);
            if id != self.gravity_source || self.station.is_some() {
                self.enter_frame(id, None, (p.center, p.rotation), p.planet.spec.velocity);
            }
            self.step_local(input, p.planet, dt);
            return;
        }
        // Inside a moon's sphere of influence, which is inside its planet's: the smallest wins.
        let well = wells
            .iter()
            .filter(|w| world.distance(w.center) < w.soi() as f64)
            .min_by(|a, b| a.soi().total_cmp(&b.soi()));
        match well {
            Some(w) => {
                if self.orbiting != Some(w.id)
                    || self.gravity_source.is_some()
                    || self.station.is_some()
                {
                    self.enter_frame(None, Some(w.id), (w.center, Quat::IDENTITY), w.vel);
                }
                self.step_orbit(input, w, dt);
            }
            None => {
                if self.planet().is_some() || self.station.is_some() {
                    self.enter_frame(None, None, (world, Quat::IDENTITY), Vec3::ZERO);
                }
                let a = self.thrusters(input, dt);
                self.vel += a * dt;
                self.pos += self.vel * dt;
                self.on_ground = false;
                if self.pos.length() > ANCHOR_SPAN {
                    // Keep the local coordinates small: the anchor follows the body.
                    self.frame.0 += self.pos.as_dvec3();
                    self.pos = Vec3::ZERO;
                }
            }
        }
    }

    /// Acceleration from the thrusters in vacuum (manual, autopilot and brake), in the body's frame.
    fn thrusters(&self, input: Input, dt: f32) -> Vec3 {
        let vertical = input.jump as i32 as f32 - input.descend as i32 as f32;
        let wish = self.look_dir() * input.movement.y
            + self.right() * input.movement.x
            + self.up * vertical;
        let thrust = if input.sprint { THRUST * BOOST } else { THRUST };
        let mut a = wish.normalize_or_zero() * thrust + self.autopilot(input);
        if input.brake {
            a += self.braking(dt);
        }
        a
    }

    fn autopilot(&self, input: Input) -> Vec3 {
        input
            .thrust
            .map_or(Vec3::ZERO, |t| self.frame.1.inverse() * t)
    }

    /// Deceleration that stops the body (in its frame) without overshooting.
    fn braking(&self, dt: f32) -> Vec3 {
        let v = self.vel.length();
        if v < 1e-4 {
            return Vec3::ZERO;
        }
        -self.vel / v * BRAKE.min(v / dt)
    }

    /// One step in a planet's orbit frame: inverse-square gravity, thrusters, no drag.
    fn step_orbit(&mut self, input: Input, well: &Well, dt: f32) {
        let r = self.pos.length().max(1.0);
        let radial = self.pos / r;
        let g = -radial * gravity_at(r, well.pull);
        let a = self.thrusters(input, dt);
        // Semi-implicit Euler: velocity first, then position. Keeps orbits closed.
        self.vel += (g + a) * dt;
        self.pos += self.vel * dt;
        self.on_ground = false;
        // The planet becomes "down", gently, so the horizon settles before the atmosphere.
        let lean = 1.5 * (1.0 - smoothstep(well.soi() - well.pull.taper() * 2.0, well.soi(), r));
        self.set_up(self.up.slerp(radial, 1.0 - (-lean * dt).exp()).normalize());
    }

    /// Walking, jumping, flying and the jetpack, standing on ground that is "down" against
    /// `radial`, with `gravity` pulling that way. Sets the velocity; collision is the caller's.
    fn locomotion(&mut self, input: Input, radial: Vec3, gravity: f32, dt: f32) {
        let wish =
            (self.forward * input.movement.y + self.right() * input.movement.x).normalize_or_zero();
        let speed = if input.sprint { SPRINT } else { WALK };
        let vertical = input.jump as i32 as f32 - input.descend as i32 as f32;
        // The jetpack lifts off when Space is held, and walks while standing.
        let jet = self.jetpack && !self.flying && !(self.on_ground && !input.jump);
        let piloted = input.thrust.is_some();

        if let (Some(l), false, false, false) = (self.swimming, self.flying, jet, piloted) {
            // Swimming: water holds you up (you sink slowly), drags you, and Space swims up;
            // lava is thick and slow. At the edge, Space climbs out onto the ledge.
            let (swim, drag, up) = match l {
                Liquid::Water => (if input.sprint { SWIM * 1.4 } else { SWIM }, 2.5, SWIM_UP),
                Liquid::Lava => (SWIM * 0.45, 5.0, SWIM_UP * 0.5),
            };
            let mut vr = self.vel.dot(radial);
            let mut vt = self.vel - radial * vr;
            vt = vt.lerp(wish.reject_from(radial) * swim, 1.0 - (-6.0 * dt).exp());
            vr -= gravity * 0.12 * dt;
            if vertical != 0.0 {
                vr += (vertical * up - vr) * (1.0 - (-6.0 * dt).exp());
            } else {
                vr *= (-drag * dt).exp();
            }
            if input.jump && self.ledge && self.eyes_in.is_none() {
                vr = vr.max(JUMP_SPEED * 0.85);
            }
            self.vel = vt + radial * vr;
        } else if self.flying && !piloted {
            let target =
                (wish * (if input.sprint { FLY * 2.0 } else { FLY })) + radial * vertical * FLY;
            self.vel = self.vel.lerp(target, 1.0 - (-10.0 * dt).exp());
        } else if jet || piloted {
            self.flying = false;
            let mut a = self.autopilot(input) - radial * gravity;
            if jet {
                let look = self.look_dir();
                let dir =
                    (look * input.movement.y + self.right() * input.movement.x + radial * vertical)
                        .normalize_or_zero();
                // Holds you up against gravity, and thrusts where you point.
                a += radial * gravity + dir * if input.sprint { JET * JET_BOOST } else { JET };
            }
            if input.brake {
                a += self.braking(dt);
            }
            self.vel += a * dt;
        } else {
            let mut vr = self.vel.dot(radial);
            let mut vt = self.vel - radial * vr;
            let target = wish.reject_from(radial) * speed;
            if self.on_ground || vt.length() < speed * 1.5 {
                let control = if self.on_ground { 14.0 } else { 3.0 };
                vt = vt.lerp(target, 1.0 - (-control * dt).exp());
            } else {
                // Coming in fast (from orbit, a jetpack sprint): keep the momentum, the air slows it.
                vt += target.normalize_or_zero() * 6.0 * dt;
            }
            if vt.length_squared() < 1e-6 {
                vt = Vec3::ZERO; // come to a full stop instead of creeping forever
            }
            vr -= gravity * dt;
            if input.jump && self.on_ground {
                vr = JUMP_SPEED;
            }
            if self.climbing {
                vr = if input.jump || input.movement.y > 0.0 {
                    CLIMB
                } else if input.descend {
                    vr.max(0.0)
                } else {
                    vr.max(-CLIMB_FALL)
                };
            }
            self.vel = vt + radial * vr;
        }
    }

    /// One step on a station, in its frame: gravity toward its floor, its blocks to stand on.
    fn step_station(&mut self, input: Input, station: &Station, dt: f32) {
        depenetrate_flat(station, &mut self.pos);
        self.set_up(self.up.slerp(Vec3::Y, 1.0 - (-8.0 * dt).exp()).normalize());
        let at = |p: Vec3| station.get(Station::cell_at(p));
        self.swimming = block::liquid(at(self.pos + Vec3::Y * 0.5));
        self.eyes_in = block::liquid(at(self.eye()));
        self.ledge = block::is_solid(at(self.pos + Vec3::Y * 0.6 + self.forward * 0.55));
        self.climbing = block::is_ladder(at(self.pos + Vec3::Y * 0.2))
            || block::is_ladder(at(self.pos + Vec3::Y * 1.0));
        self.locomotion(input, Vec3::Y, station.gravity(), dt);
        let (pos, grounded) = move_flat(
            station,
            self.pos,
            &mut self.vel,
            self.auto_step && !self.flying,
            dt,
        );
        self.pos = pos;
        self.on_ground = grounded;
        if self.on_ground && self.flying && input.descend {
            self.flying = false;
        }
    }

    /// One step in the planet's own frame, where its blocks stay put.
    fn step_local(&mut self, input: Input, planet: &Planet, dt: f32) {
        depenetrate(planet, &mut self.pos);
        let r = self.pos.length();
        let radial = self.pos.normalize_or(self.up);
        let spec = &planet.spec;
        // Turn to stand on the planet: gently high up, quickly near the ground.
        let height = ((r - spec.radius) / (spec.gravity_radius() - spec.radius)).clamp(0.0, 1.0);
        let turn = 8.0 - 6.0 * height;
        self.set_up(self.up.slerp(radial, 1.0 - (-turn * dt).exp()).normalize());

        self.swimming = block::liquid(planet.block_at(self.pos + radial * 0.5));
        self.eyes_in = block::liquid(planet.block_at(self.eye()));
        self.ledge = planet.solid_at(self.pos + radial * 0.6 + self.forward * 0.55);
        self.climbing = block::is_ladder(planet.block_at(self.pos + radial * 0.2))
            || block::is_ladder(planet.block_at(self.pos + radial * 1.0));
        self.locomotion(input, radial, GRAVITY, dt);
        if !self.flying {
            // Quadratic drag, integrated implicitly so it is stable at any speed.
            let k = AIR_DRAG * air_density(r, spec.radius, spec.gravity_radius());
            self.vel /= 1.0 + k * self.vel.length() * dt;
        }

        let (pos, grounded) = move_and_collide(
            planet,
            self.pos,
            &mut self.vel,
            self.auto_step && !self.flying,
            dt,
        );
        self.pos = pos;
        self.on_ground = grounded;
        if self.on_ground && self.flying && input.descend {
            self.flying = false;
        }
    }
}

/// The player's box placed in cell space: `x` = (a, b) grid coordinates on `face`, then the feet
/// radius. It spans a ± HALF_WIDTH, b ± HALF_WIDTH and [r, r + HEIGHT].
#[derive(Clone, Copy, Debug)]
struct Placed {
    face: usize,
    x: [f64; 3],
}

impl Placed {
    fn at(planet: &Planet, p: DVec3) -> Self {
        let g = &planet.grid;
        let face = g.face_of(p);
        let (a, b) = g.face_coords(face, p);
        Self {
            face,
            x: [a, b, p.length()],
        }
    }

    fn point(&self, planet: &Planet) -> DVec3 {
        planet
            .grid
            .face_point(self.face, self.x[0], self.x[1], self.x[2])
    }

    /// Extent on each axis in cell units (i, j, k).
    fn extent(&self, planet: &Planet) -> [(f64, f64); 3] {
        let (w, k) = (HALF_WIDTH as f64, self.x[2] - planet.grid.core as f64);
        [
            (self.x[0] - w, self.x[0] + w),
            (self.x[1] - w, self.x[1] + w),
            (k, k + HEIGHT as f64),
        ]
    }

    /// World-free direction of a grid axis here (unit, planet-local).
    fn axis_dir(&self, planet: &Planet, axis: usize) -> Vec3 {
        let mut ahead = *self;
        ahead.x[axis] += 1e-3;
        (ahead.point(planet) - self.point(planet))
            .normalize()
            .as_vec3()
    }
}

/// A box in grid coordinates on a face: [a0, b0, k0, a1, b1, k1] (k: layers over the core).
type GridBox = [f64; 6];

/// Cells a span overlaps, a hair wider on both sides (for gathering what may be touched).
fn reach((lo, hi): (f64, f64)) -> (i64, i64) {
    ((lo - SKIN).floor() as i64, (hi + SKIN).floor() as i64)
}

/// The collision boxes of the cells in `span` (grid coordinates on `face`; i, j may run past its
/// edges), and of the layer under it (fences stand a block and a half high), in that face's
/// coordinates. Below the layers is the sealed core.
fn boxes_near(planet: &Planet, face: usize, span: [(i64, i64); 3]) -> Vec<GridBox> {
    let g = &planet.grid;
    let mut out = Vec::new();
    for i in span[0].0..=span[0].1 {
        for j in span[1].0..=span[1].1 {
            for k in span[2].0 - 1..=span[2].1 {
                let full = [
                    i as f64,
                    j as f64,
                    k as f64,
                    (i + 1) as f64,
                    (j + 1) as f64,
                    (k + 1) as f64,
                ];
                if k < 0 {
                    out.push(full);
                    continue;
                }
                let Some(c) = g.cell_on_face(face, i, j, k) else {
                    continue;
                };
                let b = planet.get(c);
                match block::shape(b) {
                    block::Shape::Cube => out.push(full),
                    block::Shape::Boxes => {
                        let (f, ci, cj, _) = g.coords(c);
                        let same = f == face && ci as i64 == i && cj as i64 == j;
                        for x in crate::shape::collision(b, crate::shape::joined(planet, c)).iter()
                        {
                            let (w0, w1) = (k as f64 + x[2] as f64, k as f64 + x[5] as f64);
                            if same {
                                out.push([
                                    i as f64 + x[0] as f64,
                                    j as f64 + x[1] as f64,
                                    w0,
                                    i as f64 + x[3] as f64,
                                    j as f64 + x[4] as f64,
                                    w1,
                                ]);
                            } else {
                                // Over a cube edge: the box's corners in this face's coordinates.
                                let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
                                for (u, v) in
                                    [(x[0], x[1]), (x[3], x[1]), (x[3], x[4]), (x[0], x[4])]
                                {
                                    let (a, b) =
                                        g.face_coords(face, g.point(c, u, v, 0.5).as_dvec3());
                                    lo = [lo[0].min(a), lo[1].min(b)];
                                    hi = [hi[0].max(a), hi[1].max(b)];
                                }
                                out.push([lo[0], lo[1], w0, hi[0], hi[1], w1]);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// A box overlaps an extent by more than SKIN on every axis.
fn overlaps(b: &GridBox, e: &[(f64, f64); 3]) -> bool {
    (0..3).all(|x| b[x] < e[x].1 - SKIN && b[x + 3] > e[x].0 + SKIN)
}

fn box_hits(planet: &Planet, at: &Placed) -> bool {
    let e = at.extent(planet);
    boxes_near(planet, at.face, e.map(reach))
        .iter()
        .any(|b| overlaps(b, &e))
}

/// How far an extent can move along `axis` up to `d` before it touches one of `boxes`, and
/// whether it touched one. Boxes it already overlaps along the axis do not stop it.
fn sweep_boxes(boxes: &[GridBox], e: [(f64, f64); 3], axis: usize, d: f64) -> (f64, bool) {
    let (mut best, mut hit) = (d, false);
    for b in boxes {
        if !(0..3)
            .filter(|&x| x != axis)
            .all(|x| b[x] < e[x].1 - SKIN && b[x + 3] > e[x].0 + SKIN)
        {
            continue;
        }
        if d > 0.0 && b[axis] >= e[axis].1 - SKIN {
            let lim = (b[axis] - e[axis].1).max(0.0);
            if lim < best {
                (best, hit) = (lim, true);
            }
        } else if d < 0.0 && b[axis + 3] <= e[axis].0 + SKIN {
            let lim = (b[axis + 3] - e[axis].0).min(0.0);
            if lim > best {
                (best, hit) = (lim, true);
            }
        }
    }
    (best, hit)
}

/// How far the box can move along `axis` (0 = a, 1 = b, 2 = radius), up to `d`, before it
/// touches a block's box; and whether it touched one. Contact is flush on the box.
fn sweep(planet: &Planet, at: &Placed, axis: usize, d: f64) -> (f64, bool) {
    if d == 0.0 {
        return (0.0, false);
    }
    let e = at.extent(planet);
    let mut region = e;
    if d > 0.0 {
        region[axis].1 += d;
    } else {
        region[axis].0 += d;
    }
    sweep_boxes(&boxes_near(planet, at.face, region.map(reach)), e, axis, d)
}

/// Walks the box up onto a low ledge (up to `STEP_HEIGHT`) `amount` along `axis`, if it fits.
fn small_step(planet: &Planet, at: &Placed, axis: usize, amount: f64) -> Option<Placed> {
    let mut r = *at;
    r.x[2] += STEP_HEIGHT;
    if box_hits(planet, &r) {
        return None;
    }
    r.x[axis] += amount;
    if box_hits(planet, &r) {
        return None;
    }
    let (m, _) = sweep(planet, &r, 2, -STEP_HEIGHT);
    r.x[2] += m;
    (r.x[2] > at.x[2] + 1e-4).then_some(r)
}

/// Raises the box onto a one-block ledge if it fits there, both where it is and `amount`
/// further along `axis`.
fn step_up(planet: &Planet, at: &mut Placed, axis: usize, amount: f64) -> bool {
    let core = planet.grid.core as f64;
    let mut raised = *at;
    raised.x[2] = core + (at.x[2] - core + SKIN).floor() + 1.0;
    if box_hits(planet, &raised) {
        return false;
    }
    raised.x[axis] += amount;
    if box_hits(planet, &raised) {
        return false;
    }
    *at = raised;
    true
}

/// True if the player's box with feet at `p` (planet-local) overlaps a solid cell.
pub fn collides(planet: &Planet, p: Vec3) -> bool {
    p.length() < planet.grid.core || box_hits(planet, &Placed::at(planet, p.as_dvec3()))
}

/// A block placed into the body, or the box turning onto the next cube face, can leave it
/// overlapping cells: lift it out onto the next layers, up to three blocks.
fn depenetrate(planet: &Planet, p: &mut Vec3) {
    let mut at = Placed::at(planet, p.as_dvec3());
    if !box_hits(planet, &at) {
        return;
    }
    let core = planet.grid.core as f64;
    for _ in 0..3 {
        at.x[2] = core + (at.x[2] - core + SKIN).floor() + 1.0;
        if !box_hits(planet, &at) {
            *p = at.point(planet).as_vec3();
            return;
        }
    }
}

/// Moves the box through `vel * dt` in short substeps: radially first, then along the two grid
/// axes, each stopping flush against cells and dropping that part of the velocity. While standing
/// (`can_step` and the fall blocked by ground this substep) a wall one block high is stepped onto.
/// Returns the new position and whether the box stands on ground.
fn move_and_collide(
    planet: &Planet,
    p: Vec3,
    vel: &mut Vec3,
    can_step: bool,
    dt: f32,
) -> (Vec3, bool) {
    let delta = (*vel * dt).as_dvec3();
    let steps = (delta.length() / SUBSTEP).ceil().max(1.0) as usize;
    let d = delta / steps as f64;
    let mut cur = p.as_dvec3();
    let mut moved = false;
    let mut grounded = false;
    for _ in 0..steps {
        let mut at = Placed::at(planet, cur);
        let radial = cur / at.x[2];
        let dr = d.dot(radial);
        let tangent = d - radial * dr;
        let mut along = [0.0; 2];
        if tangent.length_squared() > 1e-14 {
            let (a, b) = planet
                .grid
                .face_coords(at.face, (cur + tangent).normalize() * at.x[2]);
            along = [a - at.x[0], b - at.x[1]];
        }

        let (m, hit) = sweep(planet, &at, 2, dr);
        at.x[2] += m;
        grounded = hit && dr < 0.0;
        if hit {
            let r = radial.as_vec3();
            *vel -= r * vel.dot(r);
        }
        for axis in 0..2 {
            let (m, hit) = sweep(planet, &at, axis, along[axis]);
            if hit && grounded {
                if let Some(up) = small_step(planet, &at, axis, along[axis]) {
                    at = up;
                    continue;
                }
                if can_step && step_up(planet, &mut at, axis, along[axis]) {
                    continue;
                }
            }
            at.x[axis] += m;
            if hit {
                let dir = at.axis_dir(planet, axis);
                *vel -= dir * vel.dot(dir);
            }
        }

        let next = at.point(planet);
        if next.distance_squared(cur) > 1e-12 {
            cur = next;
            moved = true;
        }
    }
    (if moved { cur.as_vec3() } else { p }, grounded)
}

/// The player's box with feet at `p` on a station: (min, max) corners.
fn flat_box(p: DVec3) -> (DVec3, DVec3) {
    let w = HALF_WIDTH as f64;
    (
        DVec3::new(p.x - w, p.y, p.z - w),
        DVec3::new(p.x + w, p.y + HEIGHT as f64, p.z + w),
    )
}

/// The collision boxes of the blocks around a box (min, max corners), and of the layer under it
/// (fences stand a block and a half high): [x0, y0, z0, x1, y1, z1].
fn flat_boxes(s: &Station, lo: DVec3, hi: DVec3) -> Vec<GridBox> {
    let (a, b, c) = (
        reach((lo.x, hi.x)),
        reach((lo.y, hi.y)),
        reach((lo.z, hi.z)),
    );
    let mut out = Vec::new();
    for x in a.0..=a.1 {
        for y in b.0 - 1..=b.1 {
            for z in c.0..=c.1 {
                let p = glam::IVec3::new(x as i32, y as i32, z as i32);
                let blk = s.get(p);
                let (fx, fy, fz) = (x as f64, y as f64, z as f64);
                match block::shape(blk) {
                    block::Shape::Cube => out.push([fx, fy, fz, fx + 1.0, fy + 1.0, fz + 1.0]),
                    block::Shape::Boxes => {
                        for q in crate::shape::collision(blk, crate::shape::joined(s, p)).iter() {
                            let q = q.map(|v| v as f64);
                            out.push([
                                fx + q[0],
                                fy + q[2],
                                fz + q[1],
                                fx + q[3],
                                fy + q[5],
                                fz + q[4],
                            ]);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

fn flat_extent(lo: DVec3, hi: DVec3) -> [(f64, f64); 3] {
    [(lo.x, hi.x), (lo.y, hi.y), (lo.z, hi.z)]
}

fn flat_hits(s: &Station, lo: DVec3, hi: DVec3) -> bool {
    let e = flat_extent(lo, hi);
    flat_boxes(s, lo, hi).iter().any(|b| overlaps(b, &e))
}

/// True if the player's box with feet at `p` (station coordinates) overlaps a solid block.
pub fn collides_flat(s: &Station, p: Vec3) -> bool {
    let (lo, hi) = flat_box(p.as_dvec3());
    flat_hits(s, lo, hi)
}

/// How far the box can move along `axis` (0 x, 1 y, 2 z), up to `d`, before it touches a block's
/// box; and whether it touched one.
fn sweep_flat(s: &Station, p: DVec3, axis: usize, d: f64) -> (f64, bool) {
    if d == 0.0 {
        return (0.0, false);
    }
    let (lo, hi) = flat_box(p);
    let (mut rlo, mut rhi) = (lo, hi);
    if d > 0.0 {
        rhi[axis] += d;
    } else {
        rlo[axis] += d;
    }
    sweep_boxes(&flat_boxes(s, rlo, rhi), flat_extent(lo, hi), axis, d)
}

/// A block placed into the body leaves it overlapping: lift it out, up to three blocks.
fn depenetrate_flat(s: &Station, p: &mut Vec3) {
    if !collides_flat(s, *p) {
        return;
    }
    let mut q = *p;
    for _ in 0..3 {
        q.y = (q.y + SKIN as f32).floor() + 1.0;
        if !collides_flat(s, q) {
            *p = q;
            return;
        }
    }
}

/// `move_and_collide` on a station: up and down first, then x and z, in short substeps, stopping
/// flush against blocks; while standing, a one-block ledge is stepped onto.
fn move_flat(s: &Station, p: Vec3, vel: &mut Vec3, can_step: bool, dt: f32) -> (Vec3, bool) {
    let delta = (*vel * dt).as_dvec3();
    let steps = (delta.length() / SUBSTEP).ceil().max(1.0) as usize;
    let d = delta / steps as f64;
    let mut cur = p.as_dvec3();
    let mut grounded = false;
    for _ in 0..steps {
        let (m, hit) = sweep_flat(s, cur, 1, d.y);
        cur.y += m;
        grounded = hit && d.y < 0.0;
        if hit {
            vel.y = 0.0;
        }
        for axis in [0, 2] {
            let (m, hit) = sweep_flat(s, cur, axis, d[axis]);
            if hit && grounded {
                // A low ledge (a trapdoor): up onto it.
                let mut up = cur;
                up.y += STEP_HEIGHT;
                let mut ahead = up;
                ahead[axis] += d[axis];
                let ((lo, hi), (alo, ahi)) = (flat_box(up), flat_box(ahead));
                if !flat_hits(s, lo, hi) && !flat_hits(s, alo, ahi) {
                    let (m, _) = sweep_flat(s, ahead, 1, -STEP_HEIGHT);
                    ahead.y += m;
                    if ahead.y > cur.y + 1e-4 {
                        cur = ahead;
                        continue;
                    }
                }
            }
            if hit && can_step && grounded {
                let mut up = cur;
                up.y = (up.y + SKIN).floor() + 1.0;
                let (lo, hi) = flat_box(up);
                let mut ahead = up;
                ahead[axis] += d[axis];
                let (alo, ahi) = flat_box(ahead);
                if !flat_hits(s, lo, hi) && !flat_hits(s, alo, ahi) {
                    cur = ahead;
                    continue;
                }
            }
            cur[axis] += m;
            if hit {
                vel[axis] = 0.0;
            }
        }
    }
    (cur.as_vec3(), grounded)
}

#[cfg(test)]
mod swim_tests {
    use super::*;
    use crate::planet::{Biome, PlanetSpec};

    #[test]
    fn water_holds_you_up_and_space_swims_up() {
        let spec = PlanetSpec::new(9, "w".into(), 40.0, Biome::Barren);
        let mut p = Planet::flat(spec, 12);
        let g = p.grid.clone();
        // A pool 5 deep over the floor, around column (0, 5, 5).
        for i in 2..9 {
            for j in 2..9 {
                for k in 12..17 {
                    p.set(g.index(0, i, j, k), block::WATER);
                }
            }
        }
        let feet = g.point(g.index(0, 5, 5, 14), 0.5, 0.5, 0.0);
        let mut b = Body::new(feet.as_dvec3(), feet.normalize());
        let planets = [Located::of(&p)];
        b.step(Input::default(), &planets, 1.0 / 60.0);
        b.pos = feet;
        b.vel = Vec3::ZERO;
        for _ in 0..60 {
            b.step(Input::default(), &planets, 1.0 / 60.0);
        }
        assert_eq!(b.swimming, Some(Liquid::Water));
        let sink = b.vel.dot(feet.normalize());
        assert!(sink < 0.0 && sink > -3.0, "sinks slowly: {sink}");
        let r0 = b.pos.length();
        let up = Input {
            jump: true,
            ..Input::default()
        };
        for _ in 0..30 {
            b.step(up, &planets, 1.0 / 60.0);
        }
        assert!(b.pos.length() > r0 + 1.0, "swam up {}", b.pos.length() - r0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::{Biome, Orbit, PlanetSpec, Spin};

    const DT: f32 = 1.0 / 60.0;

    fn spec(radius: f32) -> PlanetSpec {
        PlanetSpec {
            center: DVec3::new(100.0, 0.0, 0.0),
            ..PlanetSpec::new(3, "t".into(), radius, Biome::Barren)
        }
    }

    fn planet() -> Planet {
        Planet::generate(spec(20.0))
    }

    /// Radius 48, solid below layer 16 (= depth): ground at radius 48.
    const GROUND: usize = 16;
    fn flat() -> Planet {
        Planet::flat(spec(48.0), GROUND)
    }

    /// Body standing at grid coordinates (a, b) on `face`, on top of layer `k - 1`, facing +a.
    fn stand(p: &Planet, face: usize, a: f64, b: f64, k: usize) -> Body {
        let at = Placed {
            face,
            x: [a, b, p.grid.radius(k) as f64],
        };
        let local = at.point(p).as_vec3();
        let mut body = Body::new(p.spec.to_world(local), p.spec.rotation * local);
        body.forward = p.spec.rotation * at.axis_dir(p, 0);
        body
    }

    fn placed(p: &Planet, b: &Body) -> Placed {
        assert_eq!(b.gravity_source, Some(p.spec.id));
        Placed::at(p, b.pos.as_dvec3())
    }

    fn run(p: &Planet, b: &mut Body, input: Input, ticks: usize) {
        for i in 0..ticks {
            b.step(input, &[Located::of(p)], DT);
            assert!(
                !collides(p, b.pos),
                "inside a block at tick {i}: {:?}",
                placed(p, b)
            );
        }
    }

    fn set(p: &mut Planet, face: usize, i: usize, j: usize, k: usize, b: crate::block::Block) {
        let c = p.grid.index(face, i, j, k);
        p.set(c, b);
    }

    const FORWARD: Input = Input {
        movement: Vec2::Y,
        jump: false,
        descend: false,
        sprint: false,
        brake: false,
        thrust: None,
    };

    #[test]
    fn falls_and_lands_on_surface() {
        let p = planet();
        let mut b = Body::new(p.spec.to_world(Vec3::Y * p.spec.ceiling()), Vec3::Y);
        for _ in 0..300 {
            b.step(Input::default(), &[Located::of(&p)], DT);
        }
        assert!(b.on_ground);
        assert!(!collides(&p, b.pos));
        let r = b.pos.length();
        assert!((r - p.spec.radius).abs() < 6.0, "r = {r}");
    }

    #[test]
    fn walks_around_without_entering_blocks() {
        let p = planet();
        let mut b = Body::new(p.spec.to_world(Vec3::Y * p.spec.ceiling()), Vec3::Y);
        run(&p, &mut b, Input::default(), 120);
        run(
            &p,
            &mut b,
            Input {
                movement: Vec2::Y,
                sprint: true,
                ..Default::default()
            },
            2880,
        );
        assert!(b.pos.length() < p.spec.ceiling());
    }

    #[test]
    fn walks_a_full_loop_across_cube_edges() {
        let p = flat();
        let n = p.grid.n as f64;
        let mut b = stand(&p, 0, n * 0.3, n * 0.45, GROUND);
        // Head off the grid axes so the loop crosses face edges at an angle.
        b.forward = Quat::from_axis_angle(b.up, 0.4) * b.forward;
        let (mut turned, mut last) = (0.0, None::<Vec3>);
        let sprint = Input {
            movement: Vec2::Y,
            sprint: true,
            ..Default::default()
        };
        for _ in 0..10_000 {
            run(&p, &mut b, sprint, 1);
            let now = b.pos.normalize();
            turned += last.map_or(0.0, |l| l.angle_between(now));
            last = Some(now);
            if turned > std::f32::consts::TAU {
                break;
            }
            assert!(
                b.on_ground && (b.pos.length() - p.grid.radius(GROUND)).abs() < 1e-3,
                "r = {}",
                b.pos.length()
            );
        }
        assert!(turned > std::f32::consts::TAU, "only walked {turned} rad");
    }

    #[test]
    fn going_into_a_moons_pull_keeps_your_speed() {
        // A planet at the origin; a moon 1000 out going round at 45 blocks/s, its pull 400 wide.
        let planet = Well {
            id: 1,
            center: DVec3::ZERO,
            radius: 150.0,
            pull: Pull::planet(280.0),
            vel: Vec3::ZERO,
        };
        let moon_vel = Vec3::new(0.0, 0.0, 45.0);
        let moon = Well {
            id: 2,
            center: DVec3::new(1000.0, 0.0, 0.0),
            radius: 50.0,
            pull: Pull {
                gravity_radius: 150.0,
                soi: 400.0,
            },
            vel: moon_vel,
        };
        let mut b = Body::new(DVec3::new(590.0, 0.0, 0.0), Vec3::Y);
        b.vel = Vec3::new(30.0, 0.0, 0.0);
        let dt = 1.0 / 60.0;
        b.step_with(Input::default(), &[], &[planet, moon], &[], dt);
        assert_eq!(b.orbiting, Some(1));
        let mut before = b.world_vel();
        while b.orbiting != Some(2) {
            before = b.world_vel();
            b.step_with(Input::default(), &[], &[planet, moon], &[], dt);
        }
        // Against the moon, it goes as fast as against the planet, less the moon's own motion.
        assert!(
            (b.world_vel() - (before - moon_vel)).length() < 1.0,
            "{} vs {}",
            b.world_vel(),
            before - moon_vel
        );
        // And back out: as it was.
        b.vel = -b.vel;
        let mut inside = b.world_vel();
        while b.orbiting != Some(1) {
            inside = b.world_vel();
            b.step_with(Input::default(), &[], &[planet, moon], &[], dt);
        }
        assert!((b.world_vel() - (inside + moon_vel)).length() < 1.0);
    }

    #[test]
    fn idle_on_a_spinning_orbiting_planet_does_not_drift() {
        let mut p = flat();
        p.spec.orbit = Orbit {
            star: DVec3::new(-3000.0, 200.0, 1000.0),
            radius: 2500.0,
            speed: 0.05,
            ..Default::default()
        };
        p.spec.spin = Spin {
            axis: Vec3::new(0.3, 1.0, 0.0).normalize(),
            speed: 0.2,
            phase: 0.0,
        };
        p.spec.advance(0.0);
        let n = p.grid.n as f64;
        let mut b = stand(&p, 0, n * 0.37, n * 0.61, GROUND);
        let tick = |p: &mut Planet, b: &mut Body, i: usize| {
            p.spec.advance(i as f64 / 60.0);
            b.step(Input::default(), &[Located::of(&*p)], DT);
        };
        for i in 0..60 {
            tick(&mut p, &mut b, i);
        }
        let start = b.pos;
        for i in 60..60 + 60 * 600 {
            tick(&mut p, &mut b, i);
            assert!(b.on_ground, "left the ground at tick {i}");
        }
        let drift = (b.pos - start).reject_from(start.normalize()).length();
        assert!(drift < 0.01, "drifted {drift} m");
        assert!(!collides(&p, b.pos));
    }

    #[test]
    fn standing_far_from_the_origin_is_perfectly_still() {
        // A star half a million blocks out, where world positions only resolve to centimeters.
        let mut p = flat();
        p.spec.orbit = Orbit {
            star: DVec3::new(480_000.0, 30_000.0, -250_000.0),
            radius: 8000.0,
            speed: 4.0 / 8000.0,
            ..Default::default()
        };
        p.spec.spin = Spin {
            axis: Vec3::new(0.3, 1.0, 0.0).normalize(),
            speed: 0.01,
            phase: 0.0,
        };
        p.spec.advance(0.0);
        let n = p.grid.n as f64;
        let mut b = stand(&p, 0, n * 0.37, n * 0.61, GROUND);
        let ground = p.grid.radius(GROUND);
        for i in 0..60 * 120 {
            p.spec.advance(i as f64 / 60.0);
            b.step(Input::default(), &[Located::of(&p)], DT);
            if i > 60 {
                assert!(b.on_ground, "left the ground at tick {i}");
                assert!(
                    (b.pos.length() - ground).abs() < 1e-4,
                    "bounced to r = {} at tick {i}",
                    b.pos.length()
                );
            }
        }
    }

    #[test]
    fn drifts_smoothly_a_trillion_blocks_out() {
        // Half a meter per second is still exactly that, however far from the universe's center.
        let start = DVec3::new(1e12, -3e11, 7e11);
        let mut b = Body::new(start, Vec3::Y);
        b.vel = Vec3::new(0.5, 0.0, 0.0);
        let mut last = b.world_point(b.pos);
        for i in 0..60 * 600 {
            b.step(Input::default(), &[], DT);
            let now = b.world_point(b.pos);
            let step = (now - last).length();
            // World coordinates out here resolve to about 0.1 mm.
            assert!(
                (step - 0.5 * DT as f64).abs() < 3e-4,
                "moved {step} at tick {i}"
            );
            last = now;
        }
        assert!((last - start).length() - 0.5 * 600.0 < 0.05);
    }

    #[test]
    fn falls_down_a_one_by_one_shaft() {
        let n = flat().grid.n;
        // Mid-face, off-centre in the column, by a face edge and in a cube corner.
        for (face, i, j, off) in [
            (0, n / 2, n / 2, 0.0),
            (2, 7, 9, 0.15),
            (3, n - 1, 5, -0.12),
            (4, 0, 0, 0.1),
        ] {
            let mut p = flat();
            for k in GROUND - 15..GROUND {
                set(&mut p, face, i, j, k, block::AIR);
            }
            let mut b = stand(&p, face, i as f64 + 0.5 + off, j as f64 + 0.5 - off, GROUND);
            run(&p, &mut b, Input::default(), 240);
            let bottom = p.grid.radius(GROUND - 15);
            assert!(b.on_ground, "face {face} ({i}, {j})");
            assert!(
                (b.pos.length() - bottom).abs() < 1e-3,
                "face {face} ({i}, {j}): stuck at r = {}, bottom {bottom}",
                b.pos.length()
            );
        }
    }

    #[test]
    fn walks_into_a_shaft_and_falls_to_the_bottom() {
        let mut p = flat();
        let (f, i, j) = (0, 30, 30);
        for k in GROUND - 15..GROUND {
            set(&mut p, f, i, j, k, block::AIR);
        }
        let mut b = stand(&p, f, i as f64 - 1.5, j as f64 + 0.5, GROUND);
        run(&p, &mut b, FORWARD, 60);
        run(&p, &mut b, Input::default(), 180);
        assert!(
            (b.pos.length() - p.grid.radius(GROUND - 15)).abs() < 1e-3,
            "stuck at r = {}",
            b.pos.length()
        );
    }

    #[test]
    fn walks_through_a_one_by_two_tunnel() {
        let mut p = flat();
        let (f, i, j, k) = (0, 25, 30, GROUND);
        for a in i + 2..i + 10 {
            for (dj, dk) in [(-1, 0), (-1, 1), (1, 0), (1, 1), (0, 2)] {
                set(&mut p, f, a, (j as i64 + dj) as usize, k + dk, block::STONE);
            }
        }
        let mut b = stand(&p, f, i as f64 + 0.5, j as f64 + 0.5, k);
        run(&p, &mut b, FORWARD, 240);
        let at = placed(&p, &b);
        assert!(at.x[0] > (i + 11) as f64, "stopped at a = {}", at.x[0]);
        assert!((b.pos.length() - p.grid.radius(k)).abs() < 1e-3);
    }

    /// A wall two blocks high across the path at column i + 3.
    fn walled() -> (Planet, usize, usize) {
        let mut p = flat();
        let (i, j) = (25, 30);
        for dj in 0..12 {
            for dk in 0..2 {
                set(&mut p, 0, i + 3, j - 6 + dj, GROUND + dk, block::STONE);
            }
        }
        (p, i, j)
    }

    #[test]
    fn stops_cleanly_against_a_wall() {
        let (p, i, j) = walled();
        let mut b = stand(&p, 0, i as f64 + 0.5, j as f64 + 0.5, GROUND);
        run(&p, &mut b, FORWARD, 90);
        let rest = placed(&p, &b);
        assert!(
            (rest.x[0] - (i as f64 + 3.0 - 0.3)).abs() < 1e-3,
            "a = {}",
            rest.x[0]
        );
        assert!(
            (rest.x[2] - p.grid.radius(GROUND) as f64).abs() < 1e-3,
            "climbed a 2-high wall"
        );
        let pos = b.pos;
        run(&p, &mut b, FORWARD, 60);
        assert!(b.pos.distance(pos) < 1e-4, "jitters against the wall");
        assert!(b.on_ground);
    }

    #[test]
    fn slides_along_a_wall_moving_diagonally() {
        let (p, i, j) = walled();
        let mut b = stand(&p, 0, i as f64 + 1.5, j as f64 + 0.5, GROUND);
        let diagonal = Input {
            movement: Vec2::new(1.0, 1.0),
            ..Default::default()
        };
        run(&p, &mut b, diagonal, 20);
        let first = placed(&p, &b);
        run(&p, &mut b, diagonal, 60);
        let then = placed(&p, &b);
        assert!(
            (then.x[0] - (i as f64 + 2.7)).abs() < 1e-3,
            "a = {}",
            then.x[0]
        );
        assert!(
            (then.x[1] - first.x[1]).abs() > 2.0,
            "did not slide: b {} -> {}",
            first.x[1],
            then.x[1]
        );
    }

    #[test]
    fn steps_up_one_block_only_with_auto_step() {
        let mut p = flat();
        let (i, j) = (25, 30);
        for a in i + 3..i + 12 {
            for dj in 0..5 {
                set(&mut p, 0, a, j - 2 + dj, GROUND, block::STONE);
            }
        }
        let mut b = stand(&p, 0, i as f64 + 0.5, j as f64 + 0.5, GROUND);
        run(&p, &mut b, FORWARD, 90);
        let at = placed(&p, &b);
        assert!(
            (at.x[0] - (i as f64 + 2.7)).abs() < 1e-3,
            "climbed without auto-step: a = {}",
            at.x[0]
        );

        let mut b = stand(&p, 0, i as f64 + 0.5, j as f64 + 0.5, GROUND);
        b.auto_step = true;
        run(&p, &mut b, FORWARD, 90);
        let at = placed(&p, &b);
        assert!(at.x[0] > (i + 4) as f64, "a = {}", at.x[0]);
        assert!(
            (at.x[2] - p.grid.radius(GROUND + 1) as f64).abs() < 1e-3,
            "r = {}",
            at.x[2]
        );
    }

    #[test]
    fn space_has_no_gravity() {
        let mut b = Body::new(DVec3::splat(5000.0), Vec3::Y);
        b.step(Input::default(), &[], 1.0);
        assert_eq!(b.vel, Vec3::ZERO);
    }
}

#[cfg(test)]
mod station_tests {
    use super::*;
    use crate::station::{Station, StationOrbit};

    const DT: f32 = 1.0 / 60.0;
    const GR: f32 = 200.0;

    fn setup(t: f64) -> (Station, StationOrbit) {
        let mut s = Station::new("0000abcd".into(), "Test", crate::block::SMOOTH_STONE);
        let o = StationOrbit::through(
            7,
            Vec3::new(0.0, GR + 70.0, 0.0),
            Quat::IDENTITY,
            Vec3::X,
            Pull::planet(GR),
            t,
        );
        s.orbit = Some(o);
        (s, o)
    }

    fn at<'a>(s: &'a Station, o: &StationOrbit, t: f64) -> StationAt<'a> {
        StationAt {
            station: s,
            center: o.offset(t),
            rotation: o.rotation(t),
            planet: 7,
            planet_center: DVec3::ZERO,
            spin: o.angular_velocity(),
        }
    }

    fn well() -> Well {
        Well {
            id: 7,
            center: DVec3::ZERO,
            radius: GR - 40.0,
            pull: Pull::planet(GR),
            vel: Vec3::ZERO,
        }
    }

    #[test]
    fn stands_on_a_moving_station() {
        let (s, o) = setup(0.0);
        let st = at(&s, &o, 0.0);
        let mut b = Body::new(DVec3::ZERO, Vec3::Y);
        b.board(&st, Vec3::new(2.5, 3.0, 2.5), Vec3::Z);
        let mut t = 0.0;
        for _ in 0..240 {
            t += DT as f64;
            let st = at(&s, &o, t);
            b.step_with(Input::default(), &[], &[well()], &[st], DT);
        }
        assert_eq!(b.station.as_deref(), Some("0000abcd"));
        assert!(b.on_ground, "lands on the slab");
        assert!(
            (b.pos.y - 1.0).abs() < 1e-3,
            "feet on top of the slab: {}",
            b.pos.y
        );
        assert!(
            b.pos.distance(Vec3::new(2.5, 1.0, 2.5)) < 1e-3,
            "does not slide while the station turns: {:?}",
            b.pos
        );
        assert_eq!(b.planet(), Some(7));
    }

    #[test]
    fn walking_off_the_edge_keeps_the_orbit() {
        let (s, o) = setup(0.0);
        let mut b = Body::new(DVec3::ZERO, Vec3::Y);
        b.board(&at(&s, &o, 0.0), Vec3::new(0.5, 1.0, 0.5), Vec3::X);
        let mut t = 0.0;
        let walk = Input {
            movement: Vec2::Y,
            ..Default::default()
        };
        let mut speed_before = 0.0;
        while b.station.is_some() && t < 10.0 {
            t += DT as f64;
            speed_before = b.world_vel().length();
            b.step_with(walk, &[], &[well()], &[at(&s, &o, t)], DT);
        }
        assert_eq!(b.orbiting, Some(7), "floats off into the planet's orbit");
        // The station's orbital speed comes along: no jolt.
        let v = b.world_vel().length();
        assert!((v - speed_before).abs() < 2.0, "{speed_before} -> {v}");
        let circular = crate::travel::circular_speed(GR + 70.0, Pull::planet(GR));
        assert!(v > circular * 0.9, "still in orbit: {v} vs {circular}");
        // And coasts alongside the station for a while.
        for _ in 0..120 {
            t += DT as f64;
            b.step_with(Input::default(), &[], &[well()], &[at(&s, &o, t)], DT);
        }
        let d = b.world_point(b.pos).distance(o.offset(t));
        assert!(d < 25.0, "drifted {d} from the station in 2 s");
    }

    /// A planet at the origin turning about +y once in 450 s, its synchronous station over +x.
    fn elevator_setup() -> (Station, StationOrbit, f32) {
        let spin = std::f32::consts::TAU / 450.0;
        let r = crate::station::sync_radius(GR, spin).unwrap();
        let mut s = Station::new("0000e1e7".into(), "Top", crate::block::SMOOTH_STONE);
        let o = StationOrbit {
            planet: 7,
            start: DVec3::new(r as f64, 0.0, 0.0),
            turn: glam::DQuat::from_rotation_arc(glam::DVec3::Y, glam::DVec3::X),
            axis: glam::DVec3::Y,
            rate: spin as f64,
            t0: 0.0,
        };
        s.orbit = Some(o);
        (s, o, spin)
    }

    fn cabin_at(h: f32, v: f32, spin: f32, t: f64) -> CabinAt {
        let rotation = Quat::from_rotation_y(spin * t as f32);
        CabinAt {
            elevator: "0000e1e7".into(),
            planet: 7,
            center: DVec3::ZERO,
            rotation,
            spin: Vec3::Y * spin,
            drift: Vec3::ZERO,
            feet: Vec3::X * h,
            up: Vec3::X,
            vel: Vec3::X * v,
        }
    }

    #[test]
    fn stepping_out_at_the_top_keeps_the_stations_speed() {
        let (s, o, spin) = elevator_setup();
        let r = o.radius();
        let t = 37.0;
        let mut b = Body::new(DVec3::new(GR as f64 - 10.0, 0.0, 0.0), Vec3::X);
        b.ride(&cabin_at(r + 1.0, 0.0, spin, t), DT);
        assert_eq!(b.planet(), Some(7));
        // In the cabin at the top the body goes round with the planet: at the station's speed.
        let st = at(&s, &o, t);
        let station_speed = st
            .spin
            .cross((b.world_point(b.pos) - st.planet_center).as_vec3());
        assert!(
            (b.world_vel() - station_speed).length() < 1e-3,
            "{} vs {station_speed}",
            b.world_vel()
        );
        // That is the circular speed there: the station is in orbit, the rider weightless.
        let circular = crate::travel::circular_speed(r + 1.0, Pull::planet(GR));
        assert!((b.world_vel().length() - circular).abs() < 0.05 * circular);
        // Boarding the station keeps it.
        let before = b.world_vel();
        let feet = st.to_local(b.world_point(b.pos));
        b.board(&st, feet, Vec3::Z);
        assert!(b.cabin.is_none());
        assert!((b.world_vel() - before).length() < 1e-2);
    }

    #[test]
    fn let_go_halfway_up_and_you_fall_back() {
        // Halfway up, the ribbon carries you round faster than you could fall but slower than an
        // orbit there: released, you go round on an ellipse that dips back into the air.
        let (_, o, spin) = elevator_setup();
        let h = (GR + o.radius()) / 2.0;
        let mut b = Body::new(DVec3::ZERO, Vec3::X);
        b.ride(&cabin_at(h, 0.0, spin, 0.0), DT);
        let (p, v) = (b.world_point(b.pos), b.world_vel());
        assert!(v.length() < crate::travel::circular_speed(h, Pull::planet(GR)));
        b.place(p, v, Vec3::X, Vec3::Z, 0.0);
        let mut lowest = f32::MAX;
        for _ in 0..60 * 120 {
            b.step_with(Input::default(), &[], &[well()], &[], DT);
            lowest = lowest.min(b.world_point(b.pos).length() as f32);
        }
        assert!(lowest < GR, "it stays up: lowest {lowest}");
    }

    #[test]
    fn walls_stop_the_player() {
        let (mut s, _) = setup(0.0);
        for y in 1..=3 {
            s.set(glam::IVec3::new(2, y, 0), crate::block::STONE);
        }
        let o = s.orbit.unwrap();
        let mut b = Body::new(DVec3::ZERO, Vec3::Y);
        b.board(&at(&s, &o, 0.0), Vec3::new(0.5, 1.0, 0.5), Vec3::X);
        let mut t = 0.0;
        for _ in 0..120 {
            t += DT as f64;
            b.step_with(
                Input {
                    movement: Vec2::Y,
                    ..Default::default()
                },
                &[],
                &[well()],
                &[at(&s, &o, t)],
                DT,
            );
            assert!(!collides_flat(&s, b.pos));
        }
        assert!(
            (b.pos.x - (2.0 - HALF_WIDTH)).abs() < 1e-3,
            "flush against the wall: {}",
            b.pos.x
        );
    }

    /// Walks +x on the station's deck for `ticks` with `input`, from (0.5, 1, 0.5); where it ends.
    fn walk(s: &Station, input: Input, ticks: usize) -> Body {
        let o = s.orbit.unwrap();
        let mut b = Body::new(DVec3::ZERO, Vec3::Y);
        b.board(&at(s, &o, 0.0), Vec3::new(0.5, 1.0, 0.5), Vec3::X);
        let mut t = 0.0;
        for _ in 0..ticks {
            t += DT as f64;
            b.step_with(input, &[], &[well()], &[at(s, &o, t)], DT);
            assert!(!collides_flat(s, b.pos));
        }
        b
    }

    #[test]
    fn doors_stop_you_shut_and_let_you_through_open() {
        use crate::block::{Dir, door};
        let (mut s, _) = setup(0.0);
        let fwd = Input {
            movement: Vec2::Y,
            ..Default::default()
        };
        // A door in the doorway at x = 2, its panel on the side toward the player (x-).
        s.set(glam::IVec3::new(2, 1, 0), door(Dir(0), false, false));
        s.set(glam::IVec3::new(2, 2, 0), door(Dir(0), false, true));
        let b = walk(&s, fwd, 120);
        assert!(
            (b.pos.x - (2.0 - HALF_WIDTH)).abs() < 1e-3,
            "stops at the shut door: {}",
            b.pos.x
        );
        s.set(glam::IVec3::new(2, 1, 0), door(Dir(0), true, false));
        s.set(glam::IVec3::new(2, 2, 0), door(Dir(0), true, true));
        let b = walk(&s, fwd, 120);
        assert!(b.pos.x > 3.0, "walks through the open door: {}", b.pos.x);
    }

    #[test]
    fn fences_are_too_high_to_jump_and_trapdoors_are_stepped_onto() {
        let (mut s, _) = setup(0.0);
        let jump = Input {
            movement: Vec2::Y,
            jump: true,
            ..Default::default()
        };
        s.set(glam::IVec3::new(2, 1, 0), crate::block::FENCE);
        let b = walk(
            &s,
            Input {
                movement: Vec2::Y,
                ..Default::default()
            },
            60,
        );
        assert!(b.pos.x < 2.1, "walked through a fence: {:?}", b.pos);
        let b = walk(&s, jump, 180);
        assert!(b.pos.x < 2.1, "jumped a fence: {:?}", b.pos);
        s.set(
            glam::IVec3::new(2, 1, 0),
            crate::block::trapdoor(crate::block::Dir(0), false, false),
        );
        let b = walk(
            &s,
            Input {
                movement: Vec2::Y,
                ..Default::default()
            },
            120,
        );
        assert!(
            b.pos.x > 2.5 && b.pos.y > 1.1,
            "on the trapdoor: {:?}",
            b.pos
        );
    }

    #[test]
    fn ladders_are_climbed() {
        let (mut s, _) = setup(0.0);
        for y in 1..=6 {
            s.set(glam::IVec3::new(2, y, 0), crate::block::STONE);
            s.set(
                glam::IVec3::new(1, y, 0),
                crate::block::ladder(crate::block::Dir(1)),
            );
        }
        let b = walk(
            &s,
            Input {
                movement: Vec2::Y,
                ..Default::default()
            },
            240,
        );
        assert!(b.pos.y > 5.0, "climbed to {:?}", b.pos);
    }
}
