//! Space stations: flat platforms that orbit a planet. A station is a box of blocks (its core at
//! station coordinate (0, 0, 0)) with a gravity of its own that always pulls toward its floor,
//! like Minecraft: everything that works on a planet works on it, without the curvature.
//!
//! It starts as a 9 × 9 slab of smooth stone with its core in the middle and grows as blocks go
//! on it: its bounds follow, up to `HALF_SPAN` from the core sideways and from `MIN_Y` to
//! `MAX_Y` in height. Past the bounds is the vacuum: no station gravity, so you float alongside
//! it in the planet's orbit.
//!
//! It orbits its planet on a circle, at the speed a body coasting there would have, and keeps its
//! floor toward the planet (it turns once per orbit). Station coordinates are local: block
//! (x, y, z) fills [x, x+1) × [y, y+1) × [z, z+1); +y is up, away from the planet.

use crate::block::{self, Block};
use crate::light;
use crate::liquid::{self, Basin, Cells, Flow};
use crate::player::{GRAVITY, Pull};
use crate::schematic::Schematic;
use crate::travel::{circular_speed, mu};
use glam::{DQuat, DVec3, IVec3, Quat, Vec3};
use std::collections::VecDeque;

/// Farthest a block may be from the core sideways (x and z).
pub const HALF_SPAN: i32 = 64;
/// Lowest and highest block layers, relative to the core's.
pub const MIN_Y: i32 = -24;
pub const MAX_Y: i32 = 47;
/// Half the starter slab: -START..=START on x and z.
pub const START: i32 = 4;
/// The station's gravity reaches this far past its blocks: above, below, and to the sides.
pub const GRAVITY_ABOVE: f32 = 20.0;
pub const GRAVITY_BELOW: f32 = 4.0;
pub const GRAVITY_SIDE: f32 = 1.0;
/// Stations around one planet at once.
pub const MAX_PER_PLANET: usize = 64;
/// No station goes nearer than this to the atmosphere (its lowest blocks included), blocks.
pub const CLEAR: f32 = 8.0;
/// Where a new station goes from the player: its core this far ahead and below the feet.
pub const AHEAD: f32 = 6.0;
pub const BELOW: f32 = 2.0;
/// The name a station gets when it has none.
pub const DEFAULT_NAME: &str = "Station";

const SIZE_XZ: usize = (2 * HALF_SPAN) as usize;
const SIZE_Y: usize = (MAX_Y - MIN_Y + 1) as usize;

/// The smallest box holding every block a station ever had, inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bounds {
    pub min: IVec3,
    pub max: IVec3,
}

impl Bounds {
    pub fn starter() -> Self {
        Self {
            min: IVec3::new(-START, 0, -START),
            max: IVec3::new(START, 0, START),
        }
    }

    pub fn with(self, p: IVec3) -> Self {
        Self {
            min: self.min.min(p),
            max: self.max.max(p),
        }
    }

    /// Size in blocks (x, y, z).
    pub fn span(&self) -> IVec3 {
        self.max - self.min + IVec3::ONE
    }

    pub fn contains(&self, p: IVec3) -> bool {
        p.cmpge(self.min).all() && p.cmple(self.max).all()
    }

    fn grown(&self, by: i32) -> Self {
        Self {
            min: self.min - IVec3::splat(by),
            max: self.max + IVec3::splat(by),
        }
    }
}

/// Why a station may not go somewhere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// Not in a planet's orbit: on a planet, in its atmosphere, or out between them.
    NotInOrbit,
    /// No lane has room for it anywhere near.
    Full,
    /// The planet has as many as it may.
    TooMany,
    /// The planet turns too slowly for a synchronous orbit inside its pull.
    NoSync,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::NotInOrbit => {
                "Stations go in orbit: above a planet's atmosphere, within its pull"
            }
            Refusal::Full => "No room in the orbits here: every lane near is taken",
            Refusal::TooMany => "Too many stations here",
            Refusal::NoSync => {
                "This planet's day is too long for a synchronous orbit within its pull"
            }
        }
    }
}

/// Station orbits go in lanes: heights this far apart, more than a station's full height (with
/// its gravity) can ever reach, so stations in different lanes never touch, whatever their
/// planes. Within a lane all go round at the same rate, so how they stand to each other repeats
/// every turn and a new one is checked against a whole turn of the others.
pub const LANE: f32 = 120.0;
/// Least distance between the origins of two stations in one lane: two of the widest stations
/// side by side, corner to corner, with their gravity and room to spare.
pub const LANE_GAP: f64 =
    2.0 * (HALF_SPAN as f64 * std::f64::consts::SQRT_2 + GRAVITY_SIDE as f64) + 16.0;
/// How much nearer stations may come between the samples of `StationOrbit::clear_of`.
const SAMPLE_SLACK: f64 = 16.0;
/// Lanes leave this much of the sphere of influence's edge clear, past the station's top.
const LANE_TOP_CLEAR: f32 = 8.0;

/// The lanes around a planet: radii of station origins, lowest first.
#[derive(Clone, Debug)]
pub struct Lanes {
    pub radii: Vec<f32>,
    /// The synchronous lane (one turn per planet day), if it fits; it is in `radii` too.
    pub sync: Option<f32>,
}

impl Lanes {
    /// Lanes around a body with this pull, turning `spin` radians a second. `moons` are the
    /// heights its moons sweep (with their spheres of influence, any plane): no lane reaches them.
    pub fn new(pull: Pull, spin: f32, moons: &[(f32, f32)]) -> Self {
        let low = pull.gravity_radius - MIN_Y as f32 + GRAVITY_BELOW + CLEAR;
        let high = pull.soi - pull.taper() - (MAX_Y + 1) as f32 - GRAVITY_ABOVE - LANE_TOP_CLEAR;
        let (below, above) = reach();
        let free = |r: f32| {
            moons
                .iter()
                .all(|&(lo, hi)| r + above < lo || r - below > hi)
        };
        let sync = sync_radius(pull.gravity_radius, spin)
            .filter(|&r| (low..=high).contains(&r) && free(r));
        let mut radii: Vec<f32> = (0..)
            .map(|k| low + k as f32 * LANE)
            .take_while(|&r| r <= high)
            .filter(|&r| free(r) && sync.is_none_or(|s| (r - s).abs() >= LANE))
            .collect();
        radii.extend(sync);
        radii.sort_by(f32::total_cmp);
        Self { radii, sync }
    }

    /// The lanes, nearest to radius `r` first.
    pub fn nearest(&self, r: f32) -> Vec<f32> {
        let mut v = self.radii.clone();
        v.sort_by(|a, b| (a - r).abs().total_cmp(&(b - r).abs()));
        v
    }

    pub fn is_sync(&self, r: f32) -> bool {
        self.sync.is_some_and(|s| (s - r).abs() < 0.5)
    }

    /// Without the lanes a station in which could touch one of `bands` (radii an elevator's
    /// ribbon spans, from the ground to its counterweight): every one it crosses but the
    /// synchronous lane, whose stations go round with the ribbon and never meet it.
    pub fn without(mut self, bands: &[(f32, f32)]) -> Self {
        let (below, above) = reach();
        let sync = self.sync;
        self.radii.retain(|&r| {
            sync.is_some_and(|s| (s - r).abs() < 0.5)
                || bands
                    .iter()
                    .all(|&(lo, hi)| r + above < lo || r - below > hi)
        });
        self
    }
}

/// How far a lane's station reaches below and above its origin, with room to spare.
fn reach() -> (f32, f32) {
    (
        -MIN_Y as f32 + GRAVITY_BELOW + CLEAR,
        (MAX_Y + 1) as f32 + GRAVITY_ABOVE + CLEAR,
    )
}

/// Radius of the circular orbit that goes round once a planet day (`spin` rad/s), where an
/// orbit keeps over one spot of the equator: what a space elevator hangs from. None for a planet
/// that does not turn.
pub fn sync_radius(gravity_radius: f32, spin: f32) -> Option<f32> {
    (spin.abs() > 1e-6)
        .then(|| (mu(gravity_radius) / (spin * spin)).cbrt())
        .filter(|&r| r > gravity_radius)
}

/// How a station goes around its planet: a circle through `start` (relative to the planet's
/// center, world-oriented) about `axis`, `rate` radians a second, from time `t0`. The rate is
/// always the one a body coasting at that height has: higher is slower.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StationOrbit {
    pub planet: u64,
    pub start: DVec3,
    /// Its turn at `t0` (local → world).
    pub turn: DQuat,
    pub axis: DVec3,
    pub rate: f64,
    pub t0: f64,
}

impl StationOrbit {
    /// A circular orbit through `center` (relative to the planet), turned `turn` there. The orbit
    /// goes the way `heading` points (the player's motion around the planet); with none, any way.
    pub fn through(
        planet: u64,
        center: Vec3,
        turn: Quat,
        heading: Vec3,
        pull: Pull,
        t0: f64,
    ) -> Self {
        let r = center.length().max(1.0);
        let radial = center / r;
        let axis = radial
            .cross(heading)
            .try_normalize()
            .unwrap_or_else(|| radial.any_orthonormal_vector());
        let rate = circular_speed(r, pull) / r;
        Self {
            planet,
            start: center.as_dvec3(),
            turn: turn.as_dquat(),
            axis: axis.as_dvec3(),
            rate: rate as f64,
            t0,
        }
    }

    fn spin(&self, t: f64) -> DQuat {
        DQuat::from_axis_angle(self.axis, self.rate * (t - self.t0))
    }

    /// The same orbit, counted from time `t`: where it is then becomes its start.
    pub fn rebased(&self, t: f64) -> Self {
        let spin = self.spin(t);
        Self {
            start: spin * self.start,
            turn: (spin * self.turn).normalize(),
            t0: t,
            ..*self
        }
    }

    /// The same station moved along its orbit by `angle` (radians, the way it goes).
    pub fn shifted(&self, angle: f64) -> Self {
        let q = DQuat::from_axis_angle(self.axis, angle);
        Self {
            start: q * self.start,
            turn: (q * self.turn).normalize(),
            ..*self
        }
    }

    /// Moved (at its `t0`) to radius `r`, going round at `rate`; turned with the move so its floor
    /// stays toward the planet.
    pub fn raised(&self, r: f32, rate: f64) -> Self {
        Self {
            start: self.start.normalize() * r as f64,
            rate,
            ..*self
        }
    }

    /// Moved (at its `t0`) into the plane of the equator of a planet turning about `axis` (unit)
    /// at `spin` rad/s, going round with it, to the nearest point there.
    pub fn equatorial(&self, axis: DVec3, spin: f64) -> Self {
        let r = self.start.length();
        let flat = self.start.reject_from(axis);
        let dir = flat
            .try_normalize()
            .unwrap_or_else(|| axis.any_orthonormal_vector());
        let q = DQuat::from_rotation_arc(self.start / r, dir);
        Self {
            start: dir * r,
            turn: (q * self.turn).normalize(),
            axis: axis * spin.signum(),
            rate: spin.abs(),
            ..*self
        }
    }

    /// Where the core's corner (station origin) is at time `t`, relative to the planet's center.
    pub fn offset(&self, t: f64) -> DVec3 {
        self.spin(t) * self.start
    }

    /// Its turn at time `t` (station local → world).
    pub fn rotation(&self, t: f64) -> Quat {
        (self.spin(t) * self.turn).normalize().as_quat()
    }

    /// Angular velocity, world, rad/s: every point of the station moves at `spin × (p - planet)`.
    pub fn angular_velocity(&self) -> Vec3 {
        (self.axis * self.rate).as_vec3()
    }

    /// Seconds per orbit.
    pub fn period(&self) -> f64 {
        std::f64::consts::TAU / self.rate.max(1e-12)
    }

    /// Height of the core above the planet's center.
    pub fn radius(&self) -> f32 {
        self.start.length() as f32
    }

    /// True if it never comes nearer than `LANE_GAP` to `other` (same planet), ever. Orbits in
    /// different lanes never meet; in the same lane (same height and rate) how they stand to each
    /// other repeats every turn, so one turn is checked.
    pub fn clear_of(&self, other: &StationOrbit) -> bool {
        let (r, ro) = (self.start.length(), other.start.length());
        if (r - ro).abs() >= LANE as f64 - 1.0 {
            return true;
        }
        if (r - ro).abs() > 0.5 || (self.rate - other.rate).abs() > 1e-9 * self.rate.max(1.0) {
            return false; // overlapping lanes going round at different rates meet sooner or later
        }
        // Sampled finely enough that nothing slips between samples: between two, the stations
        // close in by at most `SAMPLE_SLACK`, which the distance asked for allows for.
        let step = (SAMPLE_SLACK / 2.0 / r).min(0.05);
        let n = (std::f64::consts::TAU / step).ceil() as usize;
        let t = self.t0.max(other.t0);
        let dt = self.period() / n as f64;
        (0..n).all(|i| {
            let ti = t + i as f64 * dt;
            self.offset(ti).distance(other.offset(ti)) >= LANE_GAP + SAMPLE_SLACK
        })
    }
}

/// The nearest free place to `want` (an orbit as the player would have it, at time `want.t0`):
/// in the lane nearest its height, as near along its orbit as there is room, else in the next
/// lanes out. `rate` gives the coasting rate at a radius. With `sync`, only the synchronous lane,
/// in the equator's plane (`equator`: the planet's spin axis and rate).
pub fn find_slot(
    want: &StationOrbit,
    lanes: &Lanes,
    others: &[StationOrbit],
    rate: impl Fn(f32) -> f64,
    equator: Option<(DVec3, f64)>,
) -> Result<StationOrbit, Refusal> {
    if others.len() >= MAX_PER_PLANET {
        return Err(Refusal::TooMany);
    }
    let order = match equator {
        Some(_) => vec![lanes.sync.ok_or(Refusal::NoSync)?],
        None => lanes.nearest(want.radius()),
    };
    for r in order {
        let base = match equator {
            Some((axis, spin)) => want.raised(r, spin.abs()).equatorial(axis, spin),
            None => want.raised(r, rate(r)),
        };
        // Steps of a third of the gap along the orbit, both ways, out to half a turn.
        let step = (LANE_GAP / 3.0 / r as f64).min(0.2);
        let n = (std::f64::consts::PI / step) as i64;
        for k in (0..=n).flat_map(|k| [k, -k]).skip(1) {
            let c = base.shifted(k as f64 * step);
            if others.iter().all(|o| c.clear_of(&o.rebased(c.t0))) {
                return Ok(c);
            }
        }
    }
    Err(Refusal::Full)
}

/// How high the sun stands for a station at `rel` from its planet's center (`sun`: unit
/// direction toward the star), on the scale the voxel shader reads as the sun's height over the
/// horizon: 1 in full sun, falling through dusk near the edge of the planet's shadow, to -1 deep
/// inside it. It goes smoothly, so stations dim into the night instead of switching to it.
pub fn sun_height(rel: Vec3, sun: Vec3, planet_radius: f32) -> f32 {
    // Distance out of the shadow: above the sunlit half, how high over the ground; behind the
    // planet, how far off the shadow's edge (the two agree at the terminator).
    let out = if rel.dot(sun) >= 0.0 {
        rel.length()
    } else {
        rel.reject_from(sun).length()
    };
    // 0.6 radius off the edge counts as the sun (or the night) fully up; the shader dims
    // the sky from about a fifth of a radius inside the shadow to a tenth outside it.
    ((out - planet_radius) / (0.6 * planet_radius)).clamp(-1.0, 1.0)
}

/// A raycast hit: the cell, and the empty cell in front of the face hit (where a block would go).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub cell: IVec3,
    pub before: Option<IVec3>,
    /// Where the ray went in (local).
    pub point: Vec3,
}

/// What a box of blocks in space is: a station (its core holds it in one orbit) or a rocket
/// (its helm flies it; see `rocket`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Station,
    Rocket,
}

/// Where a rocket is while someone flies it: its frame (the origin's world position and its
/// turn), and the planet it is nearest to (whose star lights it). Set every frame by the pilot.
#[derive(Clone, Copy, Debug)]
pub struct Flight {
    pub center: DVec3,
    pub rotation: Quat,
    pub planet: u64,
}

/// A rocket easing into the orbit it parks in: where it was (relative to the planet's center,
/// and its turn) when it let go, at time `t0`. It reaches its orbit `GLIDE` seconds later.
#[derive(Clone, Copy, Debug)]
pub struct Glide {
    pub from: DVec3,
    pub turn: Quat,
    pub t0: f64,
}

/// Seconds a parking rocket takes to ease into its orbit.
pub const GLIDE: f64 = 4.0;

impl Glide {
    /// How far along (0..1, eased) at time `t`; None once there.
    pub fn progress(&self, t: f64) -> Option<f64> {
        let x = ((t - self.t0) / GLIDE).clamp(0.0, 1.0);
        (x < 1.0).then(|| x * x * (3.0 - 2.0 * x))
    }
}

#[derive(Clone, Debug)]
pub struct Station {
    /// 8 hex digits.
    pub id: String,
    pub name: String,
    pub kind: Kind,
    /// Where its core is: (0, 0, 0) on a station, the helm on a rocket.
    pub core: IVec3,
    blocks: Vec<Block>,
    pub bounds: Bounds,
    /// None while packed (or while a rocket flies).
    pub orbit: Option<StationOrbit>,
    /// A rocket in flight: where it is now (it goes where its pilot takes it).
    pub flight: Option<Flight>,
    /// A rocket easing into `orbit`.
    pub glide: Option<Glide>,
    /// Water and lava waiting to flow (see `liquid`).
    flow: Flow<IVec3>,
}

impl Station {
    /// A new station: the starter slab of `slab` blocks, the core in its middle at (0, 0, 0).
    pub fn new(id: String, name: &str, slab: Block) -> Self {
        let mut s = Self::empty(
            id,
            clean_name(name),
            Kind::Station,
            IVec3::ZERO,
            Bounds::starter(),
        );
        for x in -START..=START {
            for z in -START..=START {
                s.put(IVec3::new(x, 0, z), slab);
            }
        }
        s.put(IVec3::ZERO, block::STATION_CORE);
        s
    }

    /// Its blocks within its bounds, as a schematic.
    pub fn to_schematic(&self) -> Schematic {
        let b = self.bounds;
        Schematic {
            name: self.name.clone(),
            min: b.min,
            size: b.span(),
            blocks: self.cells().map(|(_, b)| b).collect(),
        }
    }

    /// A packed station built from a schematic (blocks past the limits are left out). Its core
    /// stays a core, whatever the schematic has there.
    pub fn from_schematic(id: String, name: Option<&str>, s: &Schematic) -> Self {
        let mut st = Self::empty(
            id,
            clean_name(name.unwrap_or(&s.name)),
            Kind::Station,
            IVec3::ZERO,
            Bounds {
                min: IVec3::ZERO,
                max: IVec3::ZERO,
            },
        );
        for (p, b) in s.solid_cells() {
            st.set(p, b);
        }
        st.put(IVec3::ZERO, block::STATION_CORE);
        st
    }

    /// A rocket of these blocks (rocket coordinates: (0, 0, 0) is the middle of its base), flown
    /// from its helm at `helm`. Blocks past the limits are left out.
    pub fn rocket(
        id: String,
        name: &str,
        blocks: impl IntoIterator<Item = (IVec3, Block)>,
        helm: IVec3,
    ) -> Self {
        let mut s = Self::empty(
            id,
            clean_rocket_name(name),
            Kind::Rocket,
            helm,
            Bounds {
                min: helm,
                max: helm,
            },
        );
        for (p, b) in blocks {
            s.set(p, b);
        }
        s
    }

    fn empty(id: String, name: String, kind: Kind, core: IVec3, bounds: Bounds) -> Self {
        Self {
            id,
            name,
            kind,
            core,
            blocks: vec![block::AIR; SIZE_XZ * SIZE_XZ * SIZE_Y],
            bounds,
            orbit: None,
            flight: None,
            glide: None,
            flow: Flow::default(),
        }
    }

    pub fn is_rocket(&self) -> bool {
        self.kind == Kind::Rocket
    }

    /// The station as saved: its id, what it is, its orbit (if placed) and its blocks. (Files
    /// from before rockets start `OBSTAT01` and have no kind and core: they are stations.)
    ///
    /// ```text
    /// b"OBSTAT02", u32 id length, the id,
    /// u8 kind (0 station, 1 rocket), i32 × 3 core,
    /// u8 1 if placed, then: u64 planet, f64 × 3 start, f64 × 4 turn (x y z w), f64 × 3 axis,
    ///     f64 rate, f64 t0
    /// u32 length, the blocks as a schematic (which has the name)
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = b"OBSTAT02".to_vec();
        out.extend((self.id.len() as u32).to_le_bytes());
        out.extend(self.id.as_bytes());
        out.push(self.is_rocket() as u8);
        for v in self.core.to_array() {
            out.extend(v.to_le_bytes());
        }
        match &self.orbit {
            None => out.push(0),
            Some(o) => {
                out.push(1);
                out.extend(o.planet.to_le_bytes());
                for v in o
                    .start
                    .to_array()
                    .into_iter()
                    .chain(o.turn.to_array())
                    .chain(o.axis.to_array())
                    .chain([o.rate, o.t0])
                {
                    out.extend(v.to_le_bytes());
                }
            }
        }
        let sc = self.to_schematic().to_bytes();
        out.extend((sc.len() as u32).to_le_bytes());
        out.extend(sc);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = data.get(at..at + n).ok_or("The file is cut short")?;
            at += n;
            Ok(s)
        };
        let version = match take(8)? {
            b"OBSTAT01" => 1,
            b"OBSTAT02" => 2,
            _ => return Err("Not a station".into()),
        };
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let id = String::from_utf8(take(len)?.to_vec()).map_err(|_| "Bad id")?;
        let (kind, core) = if version >= 2 {
            let kind = if take(1)?[0] == 1 {
                Kind::Rocket
            } else {
                Kind::Station
            };
            let mut c = [0i32; 3];
            for x in &mut c {
                *x = i32::from_le_bytes(take(4)?.try_into().unwrap());
            }
            (kind, IVec3::from_array(c))
        } else {
            (Kind::Station, IVec3::ZERO)
        };
        let orbit = if take(1)?[0] == 1 {
            let planet = u64::from_le_bytes(take(8)?.try_into().unwrap());
            let mut v = [0f64; 12];
            for x in &mut v {
                *x = f64::from_le_bytes(take(8)?.try_into().unwrap());
            }
            let turn = DQuat::from_array([v[3], v[4], v[5], v[6]]);
            Some(StationOrbit {
                planet,
                start: DVec3::new(v[0], v[1], v[2]),
                turn,
                axis: DVec3::new(v[7], v[8], v[9]),
                rate: v[10],
                t0: v[11],
            })
        } else {
            None
        };
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let sc = Schematic::from_bytes(take(len)?)?;
        let mut s = match kind {
            Kind::Station => Self::from_schematic(id, None, &sc),
            Kind::Rocket => Self::rocket(id, &sc.name, sc.solid_cells(), core),
        };
        s.orbit = orbit;
        Ok(s)
    }

    /// A random id: 8 hex digits.
    pub fn new_id(random: u64) -> String {
        format!("{:08x}", random as u32)
    }

    /// Whether a block may be at `p` at all: within the limits around the core.
    pub fn in_limits(p: IVec3) -> bool {
        (-HALF_SPAN..HALF_SPAN).contains(&p.x)
            && (-HALF_SPAN..HALF_SPAN).contains(&p.z)
            && (MIN_Y..=MAX_Y).contains(&p.y)
    }

    fn index(p: IVec3) -> Option<usize> {
        Self::in_limits(p).then(|| {
            let (x, y, z) = (
                (p.x + HALF_SPAN) as usize,
                (p.y - MIN_Y) as usize,
                (p.z + HALF_SPAN) as usize,
            );
            (y * SIZE_XZ + z) * SIZE_XZ + x
        })
    }

    pub fn get(&self, p: IVec3) -> Block {
        Self::index(p).map_or(block::AIR, |i| self.blocks[i])
    }

    fn put(&mut self, p: IVec3, b: Block) {
        if let Some(i) = Self::index(p) {
            self.blocks[i] = b;
        }
    }

    pub fn solid(&self, p: IVec3) -> bool {
        block::is_solid(self.get(p))
    }

    pub fn opaque(&self, p: IVec3) -> bool {
        block::opaque(self.get(p))
    }

    /// The cell holding a local point.
    pub fn cell_at(p: Vec3) -> IVec3 {
        p.floor().as_ivec3()
    }

    /// Puts a block at `p`; a block grows the bounds. False (and nothing done) past the limits:
    /// "the station can't grow further".
    pub fn set(&mut self, p: IVec3, b: Block) -> bool {
        if !Self::in_limits(p) {
            return false;
        }
        self.put(p, b);
        if b != block::AIR {
            self.bounds = self.bounds.with(p);
        }
        let mut flow = std::mem::take(&mut self.flow);
        flow.touch(self, p);
        self.flow = flow;
        true
    }

    /// Where liquids may be (inclusive corners): the station's gravity, around its blocks. Past
    /// it they fall away into space.
    pub fn liquid_box(&self) -> (IVec3, IVec3) {
        (
            self.bounds.min - IVec3::new(1, GRAVITY_BELOW as i32, 1),
            self.bounds.max + IVec3::new(1, GRAVITY_ABOVE as i32, 1),
        )
    }

    /// Liquid cells outside the bounds (falling off the edge, spilling over the rim).
    pub fn stray_liquids(&self) -> impl Iterator<Item = (IVec3, Block)> + '_ {
        let (lo, hi) = self.liquid_box();
        let b = self.bounds;
        (lo.y..=hi.y)
            .flat_map(move |y| {
                (lo.z..=hi.z).flat_map(move |z| (lo.x..=hi.x).map(move |x| IVec3::new(x, y, z)))
            })
            .filter(move |&p| !(p.cmpge(b.min).all() && p.cmple(b.max).all()))
            .map(|p| (p, self.get(p)))
            .filter(|&(_, b)| block::is_liquid(b))
    }

    /// Lets the liquids flow one tick (`liquid::TICK`). True if any block changed.
    pub fn tick_liquids(&mut self) -> bool {
        if self.flow.is_idle() {
            return false;
        }
        let mut flow = std::mem::take(&mut self.flow);
        let changed = flow.tick(&mut StationBasin(self), liquid::BUDGET);
        self.flow = flow;
        changed > 0
    }

    /// Breaks the block at `p`, and what it held up (plants, torches, ladders, the other half of
    /// a door): every block that went (where, what).
    pub fn break_block(&mut self, p: IVec3) -> Vec<(IVec3, Block)> {
        let was = self.get(p);
        let mut gone = vec![(p, was)];
        self.put(p, block::AIR);
        if let Some(q) = crate::shape::door_pair(self, p, was) {
            gone.push((q, self.get(q)));
            self.put(q, block::AIR);
        }
        let mut todo: Vec<IVec3> = gone.iter().map(|g| g.0).collect();
        while let Some(c) = todo.pop() {
            for q in crate::shape::unsupported(self, c) {
                gone.push((q, self.get(q)));
                self.put(q, block::AIR);
                todo.push(q);
            }
        }
        let mut flow = std::mem::take(&mut self.flow);
        for &(c, _) in &gone {
            flow.touch(self, c);
        }
        self.flow = flow;
        gone
    }

    /// A random tick on `p` (see `Planet::random_tick`): true if it changed.
    pub fn random_tick(&mut self, p: IVec3, light: impl Fn(IVec3) -> u8) -> bool {
        let b = self.get(p);
        if b != block::GRASS && b != block::DIRT {
            return false;
        }
        let up = p + IVec3::Y;
        let near = b == block::DIRT && crate::shape::grass_near(self, p);
        match crate::shape::grass_tick(b, self.get(up), light(up) >= 9, near) {
            Some(to) => {
                self.put(p, to);
                true
            }
            None => false,
        }
    }

    /// True at its core: a station's, or a rocket's helm.
    pub fn is_core(&self, p: IVec3) -> bool {
        p == self.core
    }

    pub fn block_count(&self) -> usize {
        self.cells().filter(|&(_, b)| b != block::AIR).count()
    }

    /// Every cell inside the bounds, with its block.
    pub fn cells(&self) -> impl Iterator<Item = (IVec3, Block)> + '_ {
        let b = self.bounds;
        (b.min.y..=b.max.y)
            .flat_map(move |y| {
                (b.min.z..=b.max.z)
                    .flat_map(move |z| (b.min.x..=b.max.x).map(move |x| IVec3::new(x, y, z)))
            })
            .map(|p| (p, self.get(p)))
    }

    /// Where the station's own gravity reaches, local: (min, max) corners.
    pub fn gravity_box(&self) -> (Vec3, Vec3) {
        let (lo, hi) = (
            self.bounds.min.as_vec3(),
            (self.bounds.max + IVec3::ONE).as_vec3(),
        );
        (
            lo - Vec3::new(GRAVITY_SIDE, GRAVITY_BELOW, GRAVITY_SIDE),
            hi + Vec3::new(GRAVITY_SIDE, GRAVITY_ABOVE, GRAVITY_SIDE),
        )
    }

    /// True if a local point is in the station's gravity.
    pub fn in_gravity(&self, p: Vec3) -> bool {
        let (lo, hi) = self.gravity_box();
        p.cmpge(lo).all() && p.cmplt(hi).all()
    }

    /// Gravity on the station, blocks/s²: the same as on a planet's surface.
    pub fn gravity(&self) -> f32 {
        GRAVITY
    }

    /// The first targetable block along a ray (local), within `reach`.
    pub fn raycast(&self, from: Vec3, dir: Vec3, reach: f32) -> Option<Hit> {
        let dir = dir.normalize_or_zero();
        if dir == Vec3::ZERO {
            return None;
        }
        // Voxel traversal (Amanatides & Woo).
        let mut cell = Self::cell_at(from);
        let step = IVec3::new(
            dir.x.signum() as i32,
            dir.y.signum() as i32,
            dir.z.signum() as i32,
        );
        let next = |c: i32, s: i32| (c + (s > 0) as i32) as f32;
        let t_for = |o: f32, d: f32, c: i32, s: i32| {
            if d.abs() < 1e-9 {
                f32::INFINITY
            } else {
                (next(c, s) - o) / d
            }
        };
        let mut t_max = Vec3::new(
            t_for(from.x, dir.x, cell.x, step.x),
            t_for(from.y, dir.y, cell.y, step.y),
            t_for(from.z, dir.z, cell.z, step.z),
        );
        let t_delta = Vec3::new(1.0 / dir.x.abs(), 1.0 / dir.y.abs(), 1.0 / dir.z.abs());
        let mut before = None;
        let mut t = 0.0;
        while t <= reach {
            let b = self.get(cell);
            if block::targetable(b) {
                // Into its shape (a door's board, a flower's tuft), or past it.
                let o = from - cell.as_vec3();
                let into = crate::shape::outline(b, crate::shape::joined(self, cell))
                    .iter()
                    .filter_map(|c| {
                        crate::shape::ray_box([o.x, o.z, o.y], [dir.x, dir.z, dir.y], c)
                    })
                    .reduce(f32::min);
                if let Some(t) = into.filter(|&t| t <= reach) {
                    return Some(Hit {
                        cell,
                        before,
                        point: from + dir * t.max(0.0),
                    });
                }
            } else {
                before = Some(cell);
            }
            if t_max.x < t_max.y && t_max.x < t_max.z {
                cell.x += step.x;
                t = t_max.x;
                t_max.x += t_delta.x;
            } else if t_max.y < t_max.z {
                cell.y += step.y;
                t = t_max.y;
                t_max.y += t_delta.y;
            } else {
                cell.z += step.z;
                t = t_max.z;
                t_max.z += t_delta.z;
            }
        }
        None
    }

    /// Sky and block light around the station's blocks, for meshing.
    pub fn light(&self) -> Light {
        Light::of(self)
    }
}

impl crate::shape::Around for Station {
    type Pos = IVec3;
    fn at(&self, p: IVec3) -> Block {
        self.get(p)
    }
    fn side(&self, p: IVec3, d: block::Dir) -> Option<IVec3> {
        Some(p + [IVec3::NEG_X, IVec3::X, IVec3::NEG_Z, IVec3::Z][d.0 as usize & 3])
    }
    fn above(&self, p: IVec3) -> Option<IVec3> {
        Some(p + IVec3::Y)
    }
    fn below(&self, p: IVec3) -> Option<IVec3> {
        Some(p - IVec3::Y)
    }
}

impl Cells for Station {
    type Pos = IVec3;
    fn block(&self, p: IVec3) -> Block {
        self.get(p)
    }
    fn down(&self, p: IVec3) -> Option<IVec3> {
        Some(p - IVec3::Y)
    }
    fn up(&self, p: IVec3) -> Option<IVec3> {
        Some(p + IVec3::Y)
    }
    fn sides(&self, p: IVec3) -> [Option<IVec3>; 4] {
        [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].map(|d| Some(p + d))
    }
}

/// A station as liquids change it: they never grow its bounds, and past its gravity they are gone.
struct StationBasin<'a>(&'a mut Station);

impl Cells for StationBasin<'_> {
    type Pos = IVec3;
    fn block(&self, p: IVec3) -> Block {
        self.0.get(p)
    }
    fn down(&self, p: IVec3) -> Option<IVec3> {
        self.0.down(p)
    }
    fn up(&self, p: IVec3) -> Option<IVec3> {
        self.0.up(p)
    }
    fn sides(&self, p: IVec3) -> [Option<IVec3>; 4] {
        self.0.sides(p)
    }
}

impl Basin for StationBasin<'_> {
    fn put(&mut self, p: IVec3, b: Block) {
        let (lo, hi) = self.0.liquid_box();
        if p.cmpge(lo).all() && p.cmple(hi).all() {
            self.0.put(p, b);
        }
    }
}

/// A station's name, tidied: trimmed, at most 32 characters, the default when empty.
pub fn clean_name(name: &str) -> String {
    let n: String = name.trim().chars().take(32).collect();
    if n.is_empty() { DEFAULT_NAME.into() } else { n }
}

/// A rocket's name, tidied the same way; "Rocket" when empty.
pub fn clean_rocket_name(name: &str) -> String {
    let n: String = name.trim().chars().take(32).collect();
    if n.is_empty() { "Rocket".into() } else { n }
}

/// Light levels (0..=15) over a box around a station's blocks; outside it, open sky.
pub struct Light {
    lo: IVec3,
    size: IVec3,
    sky: Vec<u8>,
    glow: Vec<u8>,
}

impl Light {
    fn of(s: &Station) -> Self {
        let b = s.bounds.grown(2);
        let (lo, size) = (b.min, b.span());
        let n = (size.x * size.y * size.z) as usize;
        let mut l = Self {
            lo,
            size,
            sky: vec![0; n],
            glow: vec![0; n],
        };
        let mut sky = VecDeque::new();
        let mut glow = VecDeque::new();
        // Sunlight falls straight down each column until something stops it; it comes in from
        // every side of the box too.
        for x in b.min.x..=b.max.x {
            for z in b.min.z..=b.max.z {
                for y in (b.min.y..=b.max.y).rev() {
                    let p = IVec3::new(x, y, z);
                    // Water and lava dim it: light reaches into them from the surface, fading.
                    if s.opaque(p) || block::is_liquid(s.get(p)) {
                        break;
                    }
                    let i = l.index(p).unwrap();
                    l.sky[i] = light::MAX;
                    sky.push_back(p);
                }
            }
        }
        for (p, blk) in s.cells().chain(s.stray_liquids()) {
            let e = block::emission(blk);
            if let (true, Some(i)) = (e > 0, l.index(p)) {
                l.glow[i] = e;
                glow.push_back(p);
            }
        }
        for (field, queue) in [(0, sky), (1, glow)] {
            l.flood(s, field, queue);
        }
        l
    }

    fn index(&self, p: IVec3) -> Option<usize> {
        let d = p - self.lo;
        (d.cmpge(IVec3::ZERO).all() && d.cmplt(self.size).all())
            .then(|| ((d.y * self.size.z + d.z) * self.size.x + d.x) as usize)
    }

    fn flood(&mut self, s: &Station, field: usize, mut queue: VecDeque<IVec3>) {
        const DIRS: [IVec3; 6] = [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ];
        while let Some(p) = queue.pop_front() {
            let level = self.level(field, p);
            if level <= 1 {
                continue;
            }
            for d in DIRS {
                let q = p + d;
                let Some(i) = self.index(q) else { continue };
                let v = if field == 0 {
                    &mut self.sky
                } else {
                    &mut self.glow
                };
                if v[i] + 1 < level && !s.opaque(q) {
                    v[i] = level - 1;
                    queue.push_back(q);
                }
            }
        }
    }

    fn level(&self, field: usize, p: IVec3) -> u8 {
        match self.index(p) {
            Some(i) => {
                if field == 0 {
                    self.sky[i]
                } else {
                    self.glow[i]
                }
            }
            None => {
                if field == 0 {
                    light::MAX
                } else {
                    0
                }
            }
        }
    }

    /// (sky, block) levels of a cell.
    pub fn at(&self, p: IVec3) -> (u8, u8) {
        (self.level(0, p), self.level(1, p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn station() -> Station {
        Station::new(Station::new_id(0xabcdef12), "", block::SMOOTH_STONE)
    }

    #[test]
    fn starts_as_a_slab_with_its_core() {
        let s = station();
        assert_eq!(s.id, "abcdef12");
        assert_eq!(s.name, DEFAULT_NAME);
        assert_eq!(s.get(IVec3::ZERO), block::STATION_CORE);
        assert_eq!(s.get(IVec3::new(4, 0, -4)), block::SMOOTH_STONE);
        assert_eq!(s.get(IVec3::new(5, 0, 0)), block::AIR);
        assert_eq!(s.block_count(), 81);
        assert_eq!(s.bounds.span(), IVec3::new(9, 1, 9));
    }

    #[test]
    fn grows_up_to_its_limits() {
        let mut s = station();
        assert!(s.set(IVec3::new(20, 3, 0), block::STONE));
        assert_eq!(s.bounds.max, IVec3::new(20, 3, 4));
        assert!(
            !s.set(IVec3::new(HALF_SPAN, 0, 0), block::STONE),
            "past the limit"
        );
        assert!(!s.set(IVec3::new(0, MAX_Y + 1, 0), block::STONE));
        assert!(s.set(IVec3::new(-HALF_SPAN, MIN_Y, -HALF_SPAN), block::STONE));
        // Bounds never shrink: breaking a block keeps them.
        s.break_block(IVec3::new(20, 3, 0));
        assert_eq!(s.bounds.max.x, 20);
    }

    #[test]
    fn breaking_takes_what_stands_on_it() {
        let mut s = station();
        s.set(IVec3::new(1, 1, 1), block::TORCH);
        let changed = s.break_block(IVec3::new(1, 0, 1));
        assert_eq!(changed.len(), 2);
        assert_eq!(s.get(IVec3::new(1, 1, 1)), block::AIR);
    }

    #[test]
    fn gravity_over_the_slab_not_past_it() {
        let s = station();
        assert!(s.in_gravity(Vec3::new(0.5, 1.0, 0.5)));
        assert!(s.in_gravity(Vec3::new(0.5, 15.0, 0.5)));
        assert!(!s.in_gravity(Vec3::new(0.5, 30.0, 0.5)));
        assert!(!s.in_gravity(Vec3::new(10.0, 1.0, 0.5)));
    }

    #[test]
    fn raycast_finds_the_slab() {
        let s = station();
        let hit = s
            .raycast(Vec3::new(0.5, 2.6, 2.5), Vec3::new(0.0, -1.0, -0.4), 5.0)
            .unwrap();
        assert_eq!(hit.cell.y, 0);
        assert_eq!(hit.before.unwrap().y, 1);
    }

    #[test]
    fn light_is_full_above_dark_under_a_roof() {
        let mut s = station();
        for x in -1..=1 {
            for z in -1..=1 {
                s.set(IVec3::new(x, 3, z), block::STONE);
            }
        }
        let l = s.light();
        assert_eq!(l.at(IVec3::new(3, 1, 3)).0, light::MAX);
        assert!(l.at(IVec3::new(0, 1, 0)).0 < light::MAX);
        s.set(IVec3::new(0, 1, 0), block::TORCH);
        assert_eq!(
            s.light().at(IVec3::new(0, 1, 0)).1,
            block::emission(block::TORCH)
        );
    }

    #[test]
    fn orbit_keeps_its_height_and_floor_down() {
        let gr = 200.0;
        let center = Vec3::new(0.0, gr + 70.0, 0.0);
        let turn = Quat::IDENTITY; // +y = away from the planet at the start
        let o = StationOrbit::through(1, center, turn, Vec3::X, Pull::planet(gr), 10.0);
        assert!((o.offset(10.0).as_vec3() - center).length() < 1e-3);
        for t in [10.0, 50.0, 400.0, 1234.5] {
            let p = o.offset(t).as_vec3();
            assert!((p.length() - center.length()).abs() < 1e-2);
            let up = o.rotation(t) * Vec3::Y;
            assert!(
                up.dot(p.normalize()) > 0.9999,
                "floor toward the planet at t = {t}"
            );
        }
        // It goes the way it was heading, at circular speed.
        let v = o.angular_velocity().cross(center);
        assert!(v.normalize().dot(Vec3::X) > 0.999);
        assert!((v.length() - circular_speed(center.length(), Pull::planet(gr))).abs() < 1e-2);
    }

    #[test]
    fn sun_height_goes_smoothly_into_the_shadow() {
        let (r, sun) = (200.0, Vec3::X);
        assert_eq!(sun_height(Vec3::new(400.0, 0.0, 0.0), sun, r), 1.0);
        assert_eq!(sun_height(Vec3::new(-400.0, 0.0, 0.0), sun, r), -1.0);
        // Around the orbit, no step bigger than a small one between nearby points.
        let mut last = sun_height(Vec3::new(260.0, 0.0, 0.0), sun, r);
        for i in 1..=2000 {
            let a = i as f32 / 2000.0 * std::f32::consts::TAU;
            let h = sun_height(Vec3::new(a.cos(), a.sin(), 0.0) * 260.0, sun, r);
            assert!((h - last).abs() < 0.01, "step at {a}");
            last = h;
        }
    }

    #[test]
    fn schematics_keep_every_block() {
        let mut s = station();
        s.set(IVec3::new(3, 2, -5), block::TORCH);
        s.set(IVec3::new(-10, -3, 7), block::PLANKS);
        let sc = s.to_schematic();
        let back = Station::from_schematic("00000001".into(), None, &sc);
        assert_eq!(back.bounds, s.bounds);
        assert_eq!(back.name, s.name);
        assert!(s.cells().all(|(p, b)| back.get(p) == b));
        assert_eq!(back.get(IVec3::ZERO), block::STATION_CORE);
    }

    #[test]
    fn saves_and_loads_with_its_orbit() {
        let mut s = station();
        s.name = "Mi Base".into();
        s.set(IVec3::new(5, 3, -7), block::TORCH);
        let at = Vec3::new(400.0, 30.0, -20.0);
        s.orbit = Some(StationOrbit::through(
            42,
            at,
            Quat::from_rotation_arc(Vec3::Y, at.normalize()),
            Vec3::Z,
            Pull::planet(280.0),
            123.5,
        ));
        let back = Station::from_bytes(&s.to_bytes()).unwrap();
        assert_eq!(
            (back.id.as_str(), back.name.as_str()),
            (s.id.as_str(), "Mi Base")
        );
        assert_eq!(back.orbit, s.orbit);
        assert!(s.cells().all(|(p, b)| back.get(p) == b));
        s.orbit = None;
        assert_eq!(Station::from_bytes(&s.to_bytes()).unwrap().orbit, None);
        assert!(Station::from_bytes(&s.to_bytes()[..20]).is_err());
    }

    #[test]
    fn lanes_are_apart_and_inside_the_pull() {
        let gr = 280.0;
        let spin = std::f32::consts::TAU / 450.0;
        let l = Lanes::new(Pull::planet(gr), spin, &[]);
        assert!(l.radii.len() > 10, "{} lanes", l.radii.len());
        assert!(l.radii[0] + (MIN_Y as f32) - GRAVITY_BELOW >= gr + CLEAR - 1e-3);
        assert!(
            *l.radii.last().unwrap() + (MAX_Y + 1) as f32 + GRAVITY_ABOVE
                < Pull::planet(gr).soi - Pull::planet(gr).taper()
        );
        for w in l.radii.windows(2) {
            assert!(w[1] - w[0] >= LANE - 1e-3);
        }
        let s = l.sync.expect("a synchronous lane");
        // One turn a day: the rate of a body coasting there is the planet's.
        assert!((circular_speed(s, Pull::planet(gr)) / s - spin).abs() < 1e-5);
        // A planet with a long day has none inside its pull.
        assert!(
            Lanes::new(Pull::planet(gr), std::f32::consts::TAU / 7200.0, &[])
                .sync
                .is_none()
        );
    }

    #[test]
    fn lanes_keep_clear_of_moons() {
        let gr = 280.0;
        let spin = std::f32::consts::TAU / 450.0;
        let moon = (900.0, 1500.0);
        let l = Lanes::new(Pull::planet(gr), spin, &[moon]);
        assert!(!l.radii.is_empty());
        for &r in &l.radii {
            // A whole station, gravity and all, stays out of the moon's band.
            let (lo, hi) = (
                r + MIN_Y as f32 - GRAVITY_BELOW,
                r + (MAX_Y + 1) as f32 + GRAVITY_ABOVE,
            );
            assert!(hi < moon.0 || lo > moon.1, "lane at {r}");
        }
        assert!(l.radii.iter().any(|&r| r < moon.0) && l.radii.iter().any(|&r| r > moon.1));
        // A moon over the synchronous orbit takes it away.
        let s = Lanes::new(Pull::planet(gr), spin, &[]).sync.unwrap();
        assert!(
            Lanes::new(Pull::planet(gr), spin, &[(s - 100.0, s + 100.0)])
                .sync
                .is_none()
        );
    }

    #[test]
    fn an_elevator_closes_the_lanes_it_crosses_but_the_synchronous_one() {
        let gr = 280.0;
        let spin = std::f32::consts::TAU / 450.0;
        let lanes = Lanes::new(Pull::planet(gr), spin, &[]);
        let s = lanes.sync.unwrap();
        let top = s + 360.0;
        let open = lanes.clone().without(&[(150.0, top)]);
        assert!(open.is_sync(s) && open.sync == Some(s));
        let (below, _) = reach();
        assert!(
            open.radii
                .iter()
                .all(|&r| (r - s).abs() < 0.5 || r - below > top),
            "{:?}",
            open.radii
        );
        assert!(open.radii.len() > 1 && open.radii.len() < lanes.radii.len());
    }

    fn rate_at(gr: f32) -> impl Fn(f32) -> f64 {
        move |r| (circular_speed(r, Pull::planet(gr)) / r) as f64
    }

    #[test]
    fn fifty_stations_never_meet() {
        let gr = 280.0;
        let spin = std::f32::consts::TAU / 450.0;
        let lanes = Lanes::new(Pull::planet(gr), spin, &[]);
        let mut rng = crate::rng::Rng::new(5);
        let mut placed: Vec<StationOrbit> = Vec::new();
        for i in 0..50 {
            // All crowding the same spot, in every plane, at whatever time.
            let t = i as f64 * 37.0;
            let dir = (Vec3::X + rng.dir() * 0.05).normalize();
            let want = StationOrbit::through(
                1,
                dir * (gr + 80.0),
                Quat::IDENTITY,
                rng.dir(),
                Pull::planet(gr),
                t,
            );
            let o = find_slot(
                &want,
                &lanes,
                &placed.iter().map(|o| o.rebased(t)).collect::<Vec<_>>(),
                rate_at(gr),
                None,
            )
            .unwrap();
            placed.push(o);
        }
        // Over a long while, no two come close: neither the same lane (sideways) nor two lanes.
        for k in 0..3000 {
            let t = 2000.0 + k as f64 * 1.3;
            for (i, a) in placed.iter().enumerate() {
                for b in &placed[i + 1..] {
                    let (pa, pb) = (a.offset(t), b.offset(t));
                    let dr = (pa.length() - pb.length()).abs();
                    assert!(
                        dr >= (LANE - 1.0) as f64 || pa.distance(pb) >= LANE_GAP,
                        "met at t = {t}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_full_planet_says_so() {
        let gr = 280.0;
        let lanes = Lanes::new(Pull::planet(gr), 0.01, &[]);
        let want = StationOrbit::through(
            1,
            Vec3::X * (gr + 80.0),
            Quat::IDENTITY,
            Vec3::Z,
            Pull::planet(gr),
            0.0,
        );
        let others = vec![want; MAX_PER_PLANET];
        assert_eq!(
            find_slot(&want, &lanes, &others, rate_at(gr), None),
            Err(Refusal::TooMany)
        );
    }

    #[test]
    fn synchronous_station_keeps_over_one_spot() {
        let gr = 280.0;
        let spin_axis = Vec3::new(0.2, 1.0, 0.1).normalize();
        let spin = std::f32::consts::TAU / 450.0;
        let lanes = Lanes::new(Pull::planet(gr), spin, &[]);
        // Placed from an inclined orbit: it goes into the equator's plane.
        let at = Vec3::new(gr + 90.0, 40.0, 0.0);
        let floor_down = Quat::from_rotation_arc(Vec3::Y, at.normalize());
        let want = StationOrbit::through(
            1,
            at,
            floor_down,
            Vec3::new(0.0, 0.5, 1.0),
            Pull::planet(gr),
            10.0,
        );
        let o = find_slot(
            &want,
            &lanes,
            &[],
            rate_at(gr),
            Some((spin_axis.as_dvec3(), spin as f64)),
        )
        .unwrap();
        assert!((o.radius() - lanes.sync.unwrap()).abs() < 1e-2);
        let ground = |t: f64| DQuat::from_axis_angle(spin_axis.as_dvec3(), spin as f64 * t);
        let over = |t: f64| ground(t).inverse() * o.offset(t);
        for t in [10.0, 300.0, 4000.0, 100_000.0] {
            assert!(over(t).distance(over(10.0)) < 0.5, "drifted at t = {t}");
            let up = o.rotation(t) * Vec3::Y;
            assert!(
                up.dot(o.offset(t).as_vec3().normalize()) > 0.9999,
                "floor toward the planet"
            );
        }
    }
}
