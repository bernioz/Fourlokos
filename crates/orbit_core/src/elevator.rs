//! Space elevators: a ribbon from a point of a planet's equator straight up to a station in the
//! synchronous lane, and on past it to a counterweight; a cabin (a climber) rides it up and down.
//!
//! Why it stands: the station goes round once a planet day, so it keeps over one spot of the
//! ground and the ribbon, hanging in the planet's turning frame, is still. In that frame a piece
//! of ribbon at radius r feels its planet's pull less the centrifugal push, `apparent_gravity`:
//! down below the synchronous orbit, nothing at it, up (outward) past it. The ribbon below pulls
//! the station down, so it does not end there: it goes on up to a counterweight whose outward
//! pull balances the whole ribbon's weight, with some to spare to keep it taut at the anchor
//! (`counterweight_mass`). Nothing here is fiction but the ribbon's strength (carbon nanotubes,
//! graphene) and the game's small, fast-turning planets that make it a few thousand blocks long
//! instead of a hundred thousand kilometers.
//!
//! The ribbon hangs radially, so it must stand on the equator: anywhere else the centrifugal push
//! (away from the axis, not from the center) would lean it toward the equator. `hold` places the
//! station so the ribbon point on its deck is exactly over the anchor, in the equator's plane.
//!
//! It crosses every lane below the synchronous one, in the plane of the equator. A station in
//! any of them (whatever its plane, every orbit crosses the equator's) would meet it sooner or
//! later: they go round faster than the ground turns. So while an elevator stands, the lanes it
//! crosses take no stations (`station::Lanes::without`), and one cannot be built while a station
//! is in them. Stations in the synchronous lane go round with it and never meet it.

use crate::block;
use crate::planet::{ChunkKey, Planet};
use crate::player::{GRAVITY, Pull, gravity_at};
use crate::sphere::{CellId, CubeSphere};
use crate::station::{LANE, Station};
use glam::{DMat3, DQuat, DVec3, IVec3, Vec3};

/// The ribbon's cross-section, blocks: a flat band, wide across the cabin's back.
pub const RIBBON_WIDTH: f32 = 0.625;
pub const RIBBON_THICKNESS: f32 = 0.225;
/// The cabin model (`assets/models/elevator_cabin.glb`), blocks: its origin is the middle of its
/// bottom; the ribbon runs up its back (+z) this far behind the origin; its floor is this high.
pub const CABIN_RIBBON_BACK: f32 = 1.36;
pub const CABIN_FLOOR: f32 = 1.375;
/// Half its width (x, body and side pods) and how far it reaches forward and back (z).
pub const CABIN_HALF_X: f32 = 1.72;
pub const CABIN_FRONT: f32 = 1.55;
pub const CABIN_BACK: f32 = 1.65;
/// Its height to the roof; antennas reach higher.
pub const CABIN_HEIGHT: f32 = 4.5;
/// Where a rider stands in the cabin (model coordinates): by the front glass.
pub const RIDER: Vec3 = Vec3::new(0.0, CABIN_FLOOR, -0.35);
/// Where a rider steps out: past the door, on the cabin's +x side, clear of the shaft the cabin
/// comes up through (it reaches `CABIN_HALF_X + 0.15` out, rounded out to whole blocks).
pub const DOOR_OUT: Vec3 = Vec3::new(CABIN_HALF_X + 1.0, CABIN_FLOOR, 0.0);
/// How near (blocks) the cabin a player must be to get in or call it.
pub const REACH: f32 = 3.5;

/// Climbing speeds (blocks/s): slow in the air (drag, and the ribbon's weakest stretch is low
/// down), fast above it; and how hard it speeds up and slows down (blocks/s², ~0.2 g).
pub const AIR_SPEED: f64 = 25.0;
pub const SPACE_SPEED: f64 = 90.0;
pub const ACCEL: f64 = 6.0;

/// How far over the station the counterweight goes, at most (it stays inside the pull and
/// clear of moons): the higher it is, the lighter it can be.
pub const COUNTERWEIGHT_ABOVE: f32 = 3.0 * LANE;
/// Least height of the counterweight over the station's origin: past its top and its gravity.
pub const COUNTERWEIGHT_MIN: f32 = 80.0;
/// The ribbon's mass per block (1 block = 1 m): a carbon nanotube band, kg/m.
pub const RIBBON_DENSITY: f64 = 1.5;
/// The cabin with a rider, kg; the ribbon is kept taut at the anchor by twice its weight.
pub const CABIN_MASS: f64 = 4000.0;

/// The pull a still thing feels at radius `r` in the frame turning with the planet (`spin`
/// rad/s), blocks/s²: toward the planet (positive) below the synchronous orbit, away past it.
pub fn apparent_gravity(r: f32, pull: Pull, spin: f32) -> f32 {
    gravity_at(r, pull) - spin * spin * r
}

/// The ribbon's weight from radius `lo` to `hi` per kg/block of it: the integral of the apparent
/// gravity (negative where it pulls outward).
pub fn weight_per_density(lo: f32, hi: f32, pull: Pull, spin: f32) -> f64 {
    let n = 2000;
    let dr = (hi - lo) as f64 / n as f64;
    (0..n)
        .map(|i| apparent_gravity(lo + (i as f64 + 0.5) as f32 * dr as f32, pull, spin) as f64 * dr)
        .sum()
}

/// The counterweight (kg) at radius `top` that holds a ribbon from `ground` up to it, `density`
/// kg/block, taut with `anchor` newtons (kg·blocks/s²) at the ground. None if `top` is not past
/// the synchronous orbit (there it pulls inward and holds nothing up).
pub fn counterweight_mass(
    ground: f32,
    top: f32,
    pull: Pull,
    spin: f32,
    density: f64,
    anchor: f64,
) -> Option<f64> {
    let out = -apparent_gravity(top, pull, spin) as f64;
    (out > 1e-9).then(|| (density * weight_per_density(ground, top, pull, spin) + anchor) / out)
}

/// Where the counterweight goes: `COUNTERWEIGHT_ABOVE` over the station's origin `sync`, but
/// inside the pull and clear of the moons' bands (`moons`: radii they sweep). None if there is
/// no room even for `COUNTERWEIGHT_MIN`.
pub fn counterweight_radius(sync: f32, pull: Pull, moons: &[(f32, f32)]) -> Option<f32> {
    let mut top =
        (sync + COUNTERWEIGHT_ABOVE).min(pull.soi - pull.taper() - 2.0 * crate::station::CLEAR);
    for &(lo, _) in moons.iter().filter(|m| m.1 > sync) {
        top = top.min(lo - 2.0 * crate::station::CLEAR);
    }
    (top >= sync + COUNTERWEIGHT_MIN).then_some(top)
}

/// Which way the cabin's front faces on the station, station-local: one of the four sides.
pub fn front_dir(k: u8) -> Vec3 {
    [Vec3::NEG_Z, Vec3::X, Vec3::Z, Vec3::NEG_X][(k % 4) as usize]
}

/// The side (0..4, see `front_dir`) nearest a horizontal direction.
pub fn front_of(dir: Vec3) -> u8 {
    (0..4u8)
        .max_by(|&a, &b| front_dir(a).dot(dir).total_cmp(&front_dir(b).dot(dir)))
        .unwrap()
}

/// An elevator: which station it hangs from (one each), where it stands, and where its cabin is.
#[derive(Clone, Debug, PartialEq)]
pub struct Elevator {
    pub station: String,
    pub planet: u64,
    /// Where the ribbon meets the ground: a unit direction, planet-local, on the equator.
    pub anchor: Vec3,
    /// Where it passes the station's deck the cabin docks on, station-local; y is the deck's top.
    pub ribbon: Vec3,
    /// Which way the cabin faces there (`front_dir`): the ribbon runs up its back.
    pub front: u8,
    /// The cabin rests at the station (else on the ground).
    pub at_top: bool,
}

impl Elevator {
    /// Where the docked cabin's origin is, station-local.
    pub fn dock(&self) -> Vec3 {
        self.ribbon + front_dir(self.front) * CABIN_RIBBON_BACK - Vec3::Y * CABIN_FLOOR
    }

    /// The elevator as saved:
    ///
    /// ```text
    /// b"OBELEV01", u32 station id length, the id, u64 planet, f32 × 3 anchor, f32 × 3 ribbon,
    /// u8 front, u8 1 if the cabin is up
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = b"OBELEV01".to_vec();
        out.extend((self.station.len() as u32).to_le_bytes());
        out.extend(self.station.as_bytes());
        out.extend(self.planet.to_le_bytes());
        for v in self
            .anchor
            .to_array()
            .into_iter()
            .chain(self.ribbon.to_array())
        {
            out.extend(v.to_le_bytes());
        }
        out.push(self.front);
        out.push(self.at_top as u8);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = data.get(at..at + n).ok_or("The file is cut short")?;
            at += n;
            Ok(s)
        };
        if take(8)? != b"OBELEV01" {
            return Err("Not an elevator".into());
        }
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let station = String::from_utf8(take(len)?.to_vec()).map_err(|_| "Bad station id")?;
        let planet = u64::from_le_bytes(take(8)?.try_into().unwrap());
        let mut v = [0f32; 6];
        for x in &mut v {
            *x = f32::from_le_bytes(take(4)?.try_into().unwrap());
        }
        let front = take(1)?[0] % 4;
        let at_top = take(1)?[0] == 1;
        let anchor = Vec3::new(v[0], v[1], v[2])
            .try_normalize()
            .ok_or("Bad anchor")?;
        Ok(Self {
            station,
            planet,
            anchor,
            ribbon: Vec3::new(v[3], v[4], v[5]),
            front,
            at_top,
        })
    }
}

/// The cells of a station to clear so the cabin can come up through it to its dock: under the
/// cabin's footprint (with a little room), from the deck down through the station's bottom,
/// following the ribbon (`down`: its direction down, station-local; it leans by the station's
/// curvature off its core). `dock` is the docked cabin's origin, `front` its facing.
pub fn shaft(dock: Vec3, front: Vec3, down: Vec3, bottom: i32) -> Vec<IVec3> {
    let side = Vec3::Y.cross(front).normalize(); // the cabin's +x, station-local
    let down = down / -down.y.min(-1e-3);
    let floor = (dock.y + CABIN_FLOOR).round() as i32;
    let mut cells = Vec::new();
    for y in bottom..floor {
        // The cabin's origin when its floor passes this layer's middle.
        let at = dock + down * (floor as f32 - (y as f32 + 0.5));
        let mut lo = Vec3::splat(f32::MAX);
        let mut hi = Vec3::splat(f32::MIN);
        for (sx, sz) in [
            (-1.0, -CABIN_FRONT),
            (1.0, -CABIN_FRONT),
            (-1.0, CABIN_BACK),
            (1.0, CABIN_BACK),
        ] {
            // Model -z is the front.
            let p = at + side * sx * (CABIN_HALF_X + 0.15) - front * sz * 1.05;
            lo = lo.min(p);
            hi = hi.max(p);
        }
        for x in lo.x.floor() as i32..=(hi.x - 1e-3).floor() as i32 {
            for z in lo.z.floor() as i32..=(hi.z - 1e-3).floor() as i32 {
                cells.push(IVec3::new(x, y, z));
            }
        }
    }
    cells
}

/// The cells the docked cabin stands in, over the deck: they must be empty to dock there.
pub fn cabin_room(dock: Vec3, front: Vec3) -> Vec<IVec3> {
    let side = Vec3::Y.cross(front).normalize();
    let floor = (dock.y + CABIN_FLOOR).round() as i32;
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for (sx, sz) in [
        (-1.0, -CABIN_FRONT),
        (1.0, -CABIN_FRONT),
        (-1.0, CABIN_BACK),
        (1.0, CABIN_BACK),
    ] {
        let p = dock + side * sx * CABIN_HALF_X - front * sz;
        lo = lo.min(p);
        hi = hi.max(p);
    }
    let top = (dock.y + CABIN_HEIGHT).ceil() as i32;
    let mut cells = Vec::new();
    for y in floor..top {
        for x in lo.x.floor() as i32..=(hi.x - 1e-3).floor() as i32 {
            for z in lo.z.floor() as i32..=(hi.z - 1e-3).floor() as i32 {
                cells.push(IVec3::new(x, y, z));
            }
        }
    }
    cells
}

/// The station's orbit that holds a ribbon over `anchor` (world, unit, in the plane of the
/// equator of a planet turning about `axis` at `spin` rad/s): its origin in the synchronous lane
/// (radius `sync`) and in the equator's plane, going round with the ground, turned about its own
/// up so the ribbon's point on its deck (`ribbon`, station-local) is in that plane too, and
/// moved along the lane so that point is straight over the anchor. `o` is the orbit now (its
/// planet, and its turn to keep its heading as near as it can), `t` the time it holds from.
pub fn hold(
    o: &crate::station::StationOrbit,
    ribbon: Vec3,
    anchor: DVec3,
    axis: DVec3,
    spin: f64,
    sync: f64,
    t: f64,
) -> crate::station::StationOrbit {
    let n = axis.normalize() * spin.signum();
    let a = anchor.reject_from(axis).normalize();
    let d = ribbon.as_dvec3();
    let flat = DVec3::new(d.x, 0.0, d.z);
    let h = flat.length();
    let now = o.rebased(t);
    let turn0 = now.turn;
    let (start, turn) = if h < 1e-6 {
        let up = a;
        (
            up * sync,
            (DQuat::from_rotation_arc(turn0 * DVec3::Y, up) * turn0).normalize(),
        )
    } else {
        // Which way along the lane the ribbon point lies from the origin: the side its heading
        // is on now, so the station turns as little as it can.
        let ahead = n.cross(a);
        let sigma = if (turn0 * (flat / h)).dot(ahead) >= 0.0 {
            1.0
        } else {
            -1.0
        };
        let phi = h.atan2(sync + d.y);
        let up = DQuat::from_axis_angle(n, -sigma * phi) * a;
        let along = n.cross(up) * sigma;
        let local = DMat3::from_cols(flat / h, DVec3::Y, (flat / h).cross(DVec3::Y));
        let world = DMat3::from_cols(along, up, along.cross(up));
        (
            up * sync,
            DQuat::from_mat3(&(world * local.transpose())).normalize(),
        )
    };
    crate::station::StationOrbit {
        planet: o.planet,
        start,
        turn,
        axis: n,
        rate: spin.abs(),
        t0: t,
    }
}

/// Where the cabin is on its ribbon, and how it moves: `h` the height of its origin (its bottom)
/// from the planet's center, along the ribbon; `v` its speed up it (negative going down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Climb {
    pub h: f64,
    pub v: f64,
    /// Going up (to the station) or down.
    pub up: bool,
    pub moving: bool,
}

impl Climb {
    /// At rest at one end.
    pub fn resting(h: f64, up: bool) -> Self {
        Self {
            h,
            v: 0.0,
            up,
            moving: false,
        }
    }

    /// Sets off toward the station (`up`) or the ground; a cabin under way turns round.
    pub fn go(&mut self, up: bool) {
        self.up = up;
        self.moving = true;
    }

    /// Moves it along, between the ground stop `bottom` and the station's `top`, through an
    /// atmosphere that ends at `air`: up to the speed allowed where it is and slowing in time to
    /// stop at the end, or to enter the air no faster than `AIR_SPEED`. True when it arrives.
    pub fn step(&mut self, bottom: f64, top: f64, air: f64, dt: f64) -> bool {
        if !self.moving {
            self.h = if self.up { top } else { bottom };
            self.v = 0.0;
            return false;
        }
        let (end, dir) = if self.up { (top, 1.0) } else { (bottom, -1.0) };
        let left = (end - self.h) * dir;
        let cruise = |h: f64| if h < air { AIR_SPEED } else { SPACE_SPEED };
        // Braking is planned a little softer than the cabin can brake, so it never lags behind.
        let brake = 0.9 * ACCEL;
        let mut allowed = cruise(self.h).min((2.0 * brake * left.max(0.0)).sqrt());
        if !self.up && self.h > air {
            allowed = allowed.min((AIR_SPEED * AIR_SPEED + 2.0 * brake * (self.h - air)).sqrt());
        }
        let want = dir * allowed;
        let dv = (want - self.v).clamp(-ACCEL * dt, ACCEL * dt);
        self.v += dv;
        // The last step lands on the stop instead of past it.
        if self.v * dir * dt > left {
            self.v = dir * left.max(0.0) / dt;
        }
        self.h += self.v * dt;
        let past = (self.h - end) * dir >= -1e-3;
        if past || (left < 0.05 && self.v.abs() < 0.5) {
            self.h = end;
            self.v = 0.0;
            self.moving = false;
            return true;
        }
        false
    }

    /// The acceleration it has now (blocks/s², up the ribbon), for what a rider feels.
    pub fn accel(&self, bottom: f64, top: f64, air: f64, dt: f64) -> f64 {
        let mut c = *self;
        c.step(bottom, top, air, dt);
        (c.v - self.v) / dt
    }
}

/// Seconds a whole trip takes from rest to rest, from `bottom` to `top` (stepping it through).
pub fn trip_time(bottom: f64, top: f64, air: f64) -> f64 {
    let mut c = Climb::resting(bottom, false);
    c.go(true);
    let dt = 1.0 / 30.0;
    let mut t = 0.0;
    while !c.step(bottom, top, air, dt) && t < 3600.0 {
        t += dt;
    }
    t
}

/// What a rider standing in the cabin weighs, as a share of their weight on the ground: the
/// apparent gravity where the cabin is, plus its own acceleration up the ribbon.
pub fn felt_weight(r: f32, pull: Pull, spin: f32, accel: f32) -> f32 {
    (apparent_gravity(r, pull, spin) + accel) / GRAVITY
}

// --- On the ground and at the top: the base and the terminal --------------------------------------

/// The ground platform's radius, blocks, about the docked cabin's middle: room to walk round it.
pub const BASE_RADIUS: f32 = 4.6;
/// Layers cleared over the platform (the cabin is under 5 high) and filled under it.
const BASE_CLEAR: usize = 7;
const BASE_FILL: usize = 4;

/// Ground for the platform to sit on: solid, and not part of a tree.
fn is_ground(b: block::Block) -> bool {
    block::is_solid(b)
        && !matches!(
            b,
            block::LEAVES
                | block::BIRCH_LEAVES
                | block::PINE_LEAVES
                | block::ACACIA_LEAVES
                | block::WOOD
                | block::BIRCH_LOG
                | block::CACTUS
        )
}

/// The layer of the ground's top block in the column under `dir` (planet-local), trees aside.
pub fn surface_layer(planet: &Planet, dir: Vec3) -> Option<usize> {
    let g = &planet.grid;
    let (f, i, j) = g.column_at(dir);
    (0..g.layers)
        .rev()
        .find(|&k| is_ground(planet.get(g.index(f, i, j, k))))
}

/// The columns of the platform round `center` (planet-local direction, the docked cabin's
/// middle), each as its cell in layer `k` and whether it is on the rim (hazard stripes).
pub fn base_cells(g: &CubeSphere, center: Vec3, k: usize) -> Vec<(CellId, bool)> {
    let c = center.normalize();
    let r = g.radius(k) + 0.5;
    let face = g.face_of(c.as_dvec3());
    let (a, b) = g.face_coords(face, (c * r).as_dvec3());
    let reach = BASE_RADIUS.ceil() as i64 + 1;
    let mut cells = Vec::new();
    for di in -reach..=reach {
        for dj in -reach..=reach {
            let Some(cell) =
                g.cell_on_face(face, a.floor() as i64 + di, b.floor() as i64 + dj, k as i64)
            else {
                continue;
            };
            // How far its middle is from the platform's, along the ground.
            let d = (g.center(cell).normalize() - c).length() * r;
            if d <= BASE_RADIUS && !cells.iter().any(|&(x, _)| x == cell) {
                cells.push((cell, d > BASE_RADIUS - 1.0));
            }
        }
    }
    cells
}

/// Lays the platform round `center` with its top on layer `k`: base blocks there, the air over
/// it cleared (trees, hills) and the ground under it filled, so it stands level and solid.
/// Returns the chunks to mesh again.
pub fn lay_base(planet: &mut Planet, center: Vec3, k: usize) -> Vec<ChunkKey> {
    let mut keys: Vec<ChunkKey> = Vec::new();
    let mut put = |planet: &mut Planet, cell: CellId, b: block::Block| {
        if planet.get(cell) != b {
            for key in planet.set(cell, b) {
                if !keys.contains(&key) {
                    keys.push(key);
                }
            }
        }
    };
    for (cell, rim) in base_cells(&planet.grid, center, k) {
        let (f, i, j, _) = planet.grid.coords(cell);
        let at = |planet: &Planet, kk: usize| {
            (kk < planet.grid.layers).then(|| planet.grid.index(f, i, j, kk))
        };
        // From the top down, so nothing is left standing on air.
        for kk in (k + 1..=k + BASE_CLEAR).rev() {
            if let Some(c) = at(planet, kk) {
                put(planet, c, block::AIR);
            }
        }
        put(
            planet,
            cell,
            if rim {
                block::ELEVATOR_EDGE
            } else {
                block::ELEVATOR_BASE
            },
        );
        for kk in k.saturating_sub(BASE_FILL)..k {
            if let Some(c) = at(planet, kk).filter(|&c| !block::is_solid(planet.get(c))) {
                put(planet, c, block::STONE);
            }
        }
    }
    keys
}

/// Where the cabin stands on the terminal's deck (feet, station-local) and which way it faces:
/// at the deck's edge, its glass toward the core, the ribbon just past the edge.
pub const TERMINAL_FEET: Vec3 = Vec3::new(8.5, 1.0, 0.5);
pub const TERMINAL_FRONT: u8 = 3;

/// The station an elevator from the ground goes up to: a round deck of hull plating (smooth
/// stone rim, glass railing with crystal lamps), tapering underneath like a saucer to a crystal
/// beacon that shows from the ground; a ready 5 × 5 landing pad for rockets; the cabin's dock at
/// the edge (`TERMINAL_FEET`), open to the deck.
pub fn terminal(id: String, name: &str) -> Station {
    let mut s = Station::new(id, name, block::HULL);
    let dist = |x: i32, z: i32| ((x * x + z * z) as f32).sqrt();
    let reach = 10;
    for x in -reach..=reach {
        for z in -reach..=reach {
            let d = dist(x, z);
            let mut put = |y: i32, b: block::Block| {
                s.set(IVec3::new(x, y, z), b);
            };
            if d <= 9.5 {
                put(
                    0,
                    if d > 8.5 {
                        block::SMOOTH_STONE
                    } else {
                        block::HULL
                    },
                );
            }
            if d <= 8.0 {
                put(-1, block::SMOOTH_STONE);
            }
            if d <= 6.0 {
                put(-2, block::HULL);
            }
            if d <= 3.5 {
                put(-3, block::BASALT);
            }
            if d <= 1.5 {
                put(-4, block::CRYSTAL);
            }
            // The railing, open on the elevator's side (the cabin and its door).
            let dock_side = x >= 5 && z >= -5;
            if d > 8.5 && d <= 9.5 && !dock_side {
                put(1, block::GLASS);
            }
        }
    }
    for (x, z) in [(7, -6), (0, 9), (-6, 7), (-9, 0), (-6, -7), (0, -9)] {
        s.set(IVec3::new(x, 1, z), block::CRYSTAL);
    }
    // The landing pad, on the far side of the core.
    for x in -7..=-3 {
        for z in -2..=2 {
            s.set(IVec3::new(x, 0, z), block::PAD);
        }
    }
    crate::pad::refresh_station(&mut s, IVec3::new(-5, 0, 0));
    s.set(IVec3::ZERO, block::STATION_CORE);
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::station::{StationOrbit, sync_radius};
    use glam::Quat;

    const GR: f32 = 280.0;

    #[test]
    fn the_terminal_has_room_for_the_cabin_and_its_door_and_a_ready_pad() {
        let s = terminal("t".into(), "T");
        let e = Elevator {
            station: "t".into(),
            planet: 1,
            anchor: Vec3::X,
            ribbon: TERMINAL_FEET - front_dir(TERMINAL_FRONT) * CABIN_RIBBON_BACK,
            front: TERMINAL_FRONT,
            at_top: true,
        };
        let room: Vec<IVec3> = cabin_room(e.dock(), front_dir(e.front))
            .into_iter()
            .filter(|&c| s.solid(c))
            .collect();
        assert!(room.is_empty(), "blocks where the cabin stands: {room:?}");
        // Out of the door: deck underneath, room to stand.
        let back = -front_dir(e.front);
        let door = e.dock() + Vec3::Y.cross(back) * DOOR_OUT.x + Vec3::Y * DOOR_OUT.y;
        let cell = Station::cell_at(door);
        assert!(
            s.solid(cell - IVec3::Y) && !s.solid(cell) && !s.solid(cell + IVec3::Y),
            "door {door:?}"
        );
        assert_eq!(s.get(IVec3::ZERO), block::STATION_CORE);
        assert_eq!(s.get(IVec3::new(-5, 0, 0)), block::PAD_READY);
        // The ribbon goes past the deck's edge, clear of the core.
        assert!(e.ribbon.x > 9.0);
    }

    #[test]
    fn the_base_is_a_disc_round_its_middle_with_a_rim() {
        let g = CubeSphere::new(256, 100.0, 80);
        let center = Vec3::new(0.3, 0.1, 1.0).normalize();
        let cells = base_cells(&g, center, 60);
        let area = std::f32::consts::PI * BASE_RADIUS * BASE_RADIUS;
        assert!(
            (cells.len() as f32 - area).abs() < area * 0.25,
            "{} cells for {area}",
            cells.len()
        );
        let rim = cells.iter().filter(|c| c.1).count();
        assert!(
            rim > 12 && rim < cells.len() / 2,
            "rim {rim} of {}",
            cells.len()
        );
        let r = g.radius(60) + 0.5;
        for (c, _) in cells {
            assert!((g.center(c).normalize() - center).length() * r <= BASE_RADIUS + 1e-3);
        }
    }

    fn spin() -> f32 {
        std::f32::consts::TAU / 450.0
    }

    #[test]
    fn a_held_station_keeps_the_ribbon_over_its_anchor() {
        let axis = DVec3::new(0.2, 1.0, 0.1).normalize();
        let w = spin() as f64;
        let sync = sync_radius(GR, spin()).unwrap() as f64;
        let planet = |t: f64| DQuat::from_axis_angle(axis, w * t);
        // An anchor on the equator, and a station somewhere else in an inclined orbit.
        let anchor_local = axis.any_orthonormal_vector();
        let at = Vec3::new(GR + 90.0, 40.0, 0.0);
        let o = StationOrbit::through(
            1,
            at,
            Quat::from_rotation_arc(Vec3::Y, at.normalize()),
            Vec3::Z,
            Pull::planet(GR),
            3.0,
        );
        let ribbon = Vec3::new(12.5, 1.0, -7.0);
        let t0 = 50.0;
        let held = hold(&o, ribbon, planet(t0) * anchor_local, axis, w, sync, t0);
        assert!((held.radius() as f64 - sync).abs() < 1e-3);
        for t in [t0, 100.0, 1234.5, 50_000.0, 400_000.0] {
            let p = held.offset(t) + (held.rotation(t) * ribbon).as_dvec3();
            let a = planet(t) * anchor_local;
            // Straight over the anchor, in the equator's plane.
            assert!(
                p.normalize().distance(a) * p.length() < 1e-2,
                "{:.4} off at t = {t}",
                p.normalize().distance(a) * p.length()
            );
            assert!(p.dot(axis).abs() < 1e-2);
            // Floor toward the planet.
            let up = held.rotation(t) * Vec3::Y;
            assert!(up.as_dvec3().dot(held.offset(t).normalize()) > 0.9999);
        }
    }

    #[test]
    fn the_counterweight_holds_the_ribbon_up() {
        let pull = Pull::planet(GR);
        let sync = sync_radius(GR, spin()).unwrap();
        let ground = 160.0;
        // Below the synchronous orbit the ribbon weighs down, past it it pulls up.
        assert!(apparent_gravity(ground, pull, spin()) > 0.0);
        assert!(apparent_gravity(sync, pull, spin()).abs() < 1e-3);
        assert!(apparent_gravity(sync + 300.0, pull, spin()) < 0.0);
        // At the station a counterweight cannot hold anything; past it, a finite one does, and
        // higher up a lighter one.
        assert!(counterweight_mass(ground, sync - 10.0, pull, spin(), 1.0, 0.0).is_none());
        let near = counterweight_mass(ground, sync + 120.0, pull, spin(), 1.0, 0.0).unwrap();
        let far = counterweight_mass(ground, sync + 360.0, pull, spin(), 1.0, 0.0).unwrap();
        assert!(far > 0.0 && far < near);
        // Balanced: its outward pull equals the ribbon's weight plus the anchor's tension.
        let m = counterweight_mass(ground, sync + 360.0, pull, spin(), 1.5, 1000.0).unwrap();
        let w = 1.5 * weight_per_density(ground, sync + 360.0, pull, spin()) + 1000.0;
        assert!((m * -apparent_gravity(sync + 360.0, pull, spin()) as f64 - w).abs() < 1e-6 * w);
    }

    #[test]
    fn the_counterweight_stays_clear_of_moons_and_the_pulls_edge() {
        let pull = Pull::planet(GR);
        let sync = sync_radius(GR, spin()).unwrap();
        assert_eq!(
            counterweight_radius(sync, pull, &[]),
            Some(sync + COUNTERWEIGHT_ABOVE)
        );
        let moon = (sync + 200.0, sync + 600.0);
        let r = counterweight_radius(sync, pull, &[moon]).unwrap();
        assert!(r < moon.0 && r >= sync + COUNTERWEIGHT_MIN);
        assert!(counterweight_radius(sync, pull, &[(sync + 50.0, sync + 400.0)]).is_none());
        // Moons below do not matter (lanes and the synchronous orbit already keep clear of them).
        assert_eq!(
            counterweight_radius(sync, pull, &[(400.0, 900.0)]),
            Some(sync + COUNTERWEIGHT_ABOVE)
        );
    }

    #[test]
    fn the_cabin_climbs_and_stops_at_each_end() {
        let (bottom, top, air) = (150.0, 2240.0, 290.0);
        let mut c = Climb::resting(bottom, false);
        c.go(true);
        let dt = 1.0 / 60.0;
        let mut t = 0.0;
        let mut fastest: f64 = 0.0;
        let mut last_v: f64 = 0.0;
        loop {
            let arrived = c.step(bottom, top, air, dt);
            t += dt;
            assert!(c.h >= bottom - 1e-9 && c.h <= top + 1e-9);
            // (arriving, it stops from under a block a second: no jolt either)
            assert!(
                arrived && last_v.abs() < 1.0 || (c.v - last_v).abs() <= ACCEL * dt + 1e-9,
                "a jolt at h = {} (v {} -> {})",
                c.h,
                last_v,
                c.v
            );
            if c.h < air - 1.0 {
                assert!(
                    c.v <= AIR_SPEED + 1e-9,
                    "too fast in the air: {} at {}",
                    c.v,
                    c.h
                );
            }
            fastest = fastest.max(c.v);
            last_v = c.v;
            if arrived {
                break;
            }
            assert!(t < 600.0);
        }
        assert_eq!((c.h, c.v, c.moving), (top, 0.0, false));
        assert!((fastest - SPACE_SPEED).abs() < 1e-6);
        assert!(t > 20.0 && t < 90.0, "{t} s up");
        // And back down, slowing to air speed before it enters the atmosphere.
        c.go(false);
        last_v = 0.0;
        loop {
            let arrived = c.step(bottom, top, air, dt);
            if arrived {
                assert!(last_v.abs() < 1.0);
                break;
            }
            assert!((c.v - last_v).abs() <= ACCEL * dt + 1e-9);
            if c.h < air {
                assert!(c.v >= -AIR_SPEED - 0.2, "into the air at {}", c.v);
            }
            last_v = c.v;
        }
        assert_eq!(c.h, bottom);
        assert!((trip_time(bottom, top, air) - t).abs() < 1.0);
    }

    #[test]
    fn turning_round_halfway() {
        let (bottom, top, air) = (150.0, 2240.0, 290.0);
        let mut c = Climb::resting(bottom, false);
        c.go(true);
        for _ in 0..60 * 20 {
            c.step(bottom, top, air, 1.0 / 60.0);
        }
        let (h, v) = (c.h, c.v);
        assert!(h > air && v > 0.0);
        c.go(false);
        c.step(bottom, top, air, 1.0 / 60.0);
        assert!(c.v < v, "brakes first");
        while !c.step(bottom, top, air, 1.0 / 60.0) {}
        assert_eq!(c.h, bottom);
    }

    #[test]
    fn a_rider_weighs_less_going_up_and_nothing_at_the_top() {
        let pull = Pull::planet(GR);
        let sync = sync_radius(GR, spin()).unwrap();
        assert!((felt_weight(150.0, pull, spin(), 0.0) - 1.0).abs() < 0.01);
        assert!(felt_weight(1000.0, pull, spin(), 0.0) < 0.2);
        assert!(felt_weight(sync, pull, spin(), 0.0).abs() < 1e-3);
    }

    #[test]
    fn saves_and_loads() {
        let e = Elevator {
            station: "0a1b2c3d".into(),
            planet: 77,
            anchor: Vec3::new(0.6, 0.0, 0.8),
            ribbon: Vec3::new(0.5, 1.0, 2.14),
            front: 2,
            at_top: true,
        };
        assert_eq!(Elevator::from_bytes(&e.to_bytes()).unwrap(), e);
        assert!(Elevator::from_bytes(&e.to_bytes()[..12]).is_err());
        assert!(Elevator::from_bytes(b"OBSTAT01xxxx").is_err());
    }

    #[test]
    fn the_shaft_clears_the_cabins_way_and_spares_the_core() {
        // A cabin docked on the starter slab, facing +z, its ribbon between it and the core.
        let front = front_dir(2);
        let ribbon = Vec3::new(0.5, 1.0, 3.5 - CABIN_RIBBON_BACK);
        let e = Elevator {
            station: String::new(),
            planet: 0,
            anchor: Vec3::X,
            ribbon,
            front: 2,
            at_top: true,
        };
        let dock = e.dock();
        assert!((dock - Vec3::new(0.5, 1.0 - CABIN_FLOOR, 3.5)).length() < 1e-5);
        let cells = shaft(dock, front, Vec3::NEG_Y, crate::station::MIN_Y);
        assert!(!cells.contains(&IVec3::ZERO), "the core stays");
        // The deck under the cabin is open, the slab around it not.
        for p in [
            IVec3::new(0, 0, 3),
            IVec3::new(-1, 0, 2),
            IVec3::new(2, 0, 4),
            IVec3::new(0, crate::station::MIN_Y, 3),
        ] {
            assert!(cells.contains(&p), "{p}");
        }
        assert!(!cells.contains(&IVec3::new(0, 0, -2)) && !cells.contains(&IVec3::new(4, 0, 3)));
        assert!(cells.iter().all(|p| p.y < 1));
        // The room the cabin takes over the deck.
        let room = cabin_room(dock, front);
        assert!(room.contains(&IVec3::new(0, 1, 3)) && room.contains(&IVec3::new(0, 4, 3)));
        assert!(room.iter().all(|p| p.y >= 1));
    }
}
