//! Rockets, built the way Create's contraptions are: out of blocks, in the world, then assembled
//! into one thing that moves. You build a rocket on the ground with any blocks you like, a
//! **helm** to fly it from and at least one **engine**; right clicking the helm gathers every
//! block you placed that touches it (never the ground itself: only blocks the player put there)
//! into a rocket, which is then a box of blocks of its own (a `Station` of kind `Rocket`) that
//! flies, and that you can walk around inside.
//!
//! A rocket goes up only from a **pad**: under the middle of its base, a flat 3 × 3 of solid
//! ground, all at one height, with nothing on it but the rocket; and open sky over the rocket. It
//! comes down the same way: onto a flat 3 × 3 where its blocks fit, and there it is taken apart
//! into ordinary blocks again (so a landed rocket is part of the planet, to walk in and change).
//!
//! Better than bare ground is a **landing pad** (`pad`): a ready one (a full square of Landing
//! Pad blocks) is where a landing rocket goes first, its middle over the pad's middle. On a
//! **station** a rocket stands only on a ready pad: it lands there (`find_berth`, `berth_cells`),
//! becomes blocks of the station, and goes up from there again (`assemble_on_station`).
//!
//! Rocket coordinates: block (x, y, z) fills [x, x+1) × [y, y+1) × [z, z+1); (0, 0, 0) is the
//! middle block of its lowest layer, so the pad is the 3 × 3 under it at y = -1; +y is up and +z
//! the way it faces. On a planet these lie along the cube-sphere grid (`Lay`): the grid's columns
//! narrow toward the core, but a rocket a few dozen blocks tall barely notices.

use crate::block::{self, Block};
use crate::planet::Planet;
use crate::sphere::CellId;
use crate::station::{MAX_Y, Station};
use glam::{DVec3, IVec3, Mat3, Vec3};
use std::collections::{HashMap, HashSet, VecDeque};

/// The pad is the 3 × 3 under the rocket's middle: -PAD..=PAD on x and z.
pub const PAD: i32 = 1;
/// Most blocks one rocket may have.
pub const MAX_BLOCKS: usize = 8000;
/// How far around a spot a landing looks for a pad, in columns.
pub const PAD_SEARCH: i64 = 14;
/// How far around it a landing looks for a ready landing pad, in columns.
pub const LANDING_PAD_SEARCH: i64 = 48;

/// Why a rocket cannot go up (or come down) here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Problem {
    /// Not one engine on it.
    NoEngine,
    /// Wider than a station may be, or taller than 48 blocks, or too many blocks.
    TooBig,
    /// The 3 × 3 under its middle is not solid ground all at one height.
    NotFlat,
    /// Something stands on the pad besides the rocket.
    PadTaken,
    /// Something (a tree, a roof, the ground) over the rocket or in its way.
    Blocked,
    /// No planet ground to stand on there.
    NoGround,
    /// On a station, not standing on a ready landing pad.
    NoStationPad,
}

impl Problem {
    pub fn message(self) -> &'static str {
        match self {
            Problem::NoEngine => "A rocket needs at least one Engine",
            Problem::TooBig => {
                "Too big for a rocket: at most 128 blocks across, 48 tall, 8000 blocks"
            }
            Problem::NotFlat => {
                "A rocket needs a pad: a flat 3x3 of solid ground under its middle, all at one height"
            }
            Problem::PadTaken => {
                "Something is on the launch pad: the 3x3 under the rocket must be clear but for it"
            }
            Problem::Blocked => "Something is over the rocket or in its way: it needs open sky",
            Problem::NoGround => "No ground there to stand a rocket on",
            Problem::NoStationPad => {
                "On a station a rocket stands on a Landing Pad: a full square of them, lit up, under its middle"
            }
        }
    }
}

/// Why a rocket cannot go up, and the cells to blame (to mark them in the world): what is over
/// it, on its pad, or missing from its pad.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fault {
    pub problem: Problem,
    pub cells: Vec<CellId>,
    /// The nearest of them: its block and how high over the rocket's base layer it is.
    pub first: Option<(Block, i32)>,
}

/// At most this many cells are marked.
const MAX_MARKED: usize = 256;

impl From<Problem> for Fault {
    fn from(problem: Problem) -> Self {
        Self {
            problem,
            cells: Vec::new(),
            first: None,
        }
    }
}

impl Fault {
    /// What went wrong, saying which block is in the way and where.
    pub fn message(&self) -> String {
        let marked = if self.cells.is_empty() {
            ""
        } else {
            " (marked in red)"
        };
        match (self.problem, self.first) {
            (Problem::Blocked, Some((b, y))) => {
                let n = self.cells.len();
                let more = if n > 1 {
                    format!(" and {} more block{}", n - 1, if n == 2 { "" } else { "s" })
                } else {
                    String::new()
                };
                format!(
                    "{} {y} block{} up is over the rocket{more}{marked}: it needs open sky, break it or move the rocket",
                    block::name(b),
                    if y == 1 { "" } else { "s" }
                )
            }
            (Problem::PadTaken, Some((b, _))) => format!(
                "{} is on the launch pad{marked}: the 3x3 under the rocket must be clear but for it",
                block::name(b)
            ),
            (p, _) => format!("{}{marked}", p.message()),
        }
    }
}

/// How rocket coordinates lie on a planet's grid: rocket block (x, y, z) is grid cell
/// (i0 + x·ex.0 + z·ez.0, j0 + x·ex.1 + z·ez.1, k0 + y) of `face` (the grid runs on past the
/// face's edges). `ex` and `ez` are unit steps along the grid's i or j, at right angles, chosen so
/// the rocket is not mirrored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Lay {
    pub face: usize,
    pub i0: i64,
    pub j0: i64,
    pub k0: i64,
    pub ex: (i64, i64),
    pub ez: (i64, i64),
}

impl Lay {
    /// The rocket laid with its origin in the cell holding planet-local point `at`, its +z the
    /// grid direction nearest `forward` (planet-local; laid flat), its +y up.
    pub fn at(planet: &Planet, at: Vec3, forward: Vec3) -> Option<Self> {
        let g = &planet.grid;
        let p = at.as_dvec3();
        let face = g.face_of(p);
        let (a, b) = g.face_coords(face, p);
        let k0 = g.layer_of(at.length());
        if k0 < 0 || k0 >= g.layers as i64 {
            return None;
        }
        Some(Self::snapped(
            planet,
            face,
            a.floor() as i64,
            b.floor() as i64,
            k0,
            forward,
        ))
    }

    /// Origin in grid cell (i0, j0, k0) of `face`, +z the grid direction nearest `forward`.
    pub fn snapped(planet: &Planet, face: usize, i0: i64, j0: i64, k0: i64, forward: Vec3) -> Self {
        let g = &planet.grid;
        let r = g.radius(k0.max(0) as usize) as f64;
        let mid = g.face_point(face, i0 as f64 + 0.5, j0 as f64 + 0.5, r);
        let di = (g.face_point(face, i0 as f64 + 1.5, j0 as f64 + 0.5, r) - mid)
            .normalize()
            .as_vec3();
        let dj = (g.face_point(face, i0 as f64 + 0.5, j0 as f64 + 1.5, r) - mid)
            .normalize()
            .as_vec3();
        let up = mid.normalize().as_vec3();
        let steps = [((1, 0), di), ((-1, 0), -di), ((0, 1), dj), ((0, -1), -dj)];
        let fwd = forward.reject_from(up).normalize_or(di);
        let &(ez, zdir) = steps
            .iter()
            .max_by(|a, b| a.1.dot(fwd).total_cmp(&b.1.dot(fwd)))
            .unwrap();
        // x = y × z: of the two steps across ez, the one that way.
        let want_x = up.cross(zdir);
        let &(ex, _) = steps
            .iter()
            .filter(|s| (s.0.0 != 0) != (ez.0 != 0))
            .max_by(|a, b| a.1.dot(want_x).total_cmp(&b.1.dot(want_x)))
            .unwrap();
        Self {
            face,
            i0,
            j0,
            k0,
            ex,
            ez,
        }
    }

    /// Grid (i, j) of rocket column (x, z).
    pub fn ij(&self, x: i32, z: i32) -> (i64, i64) {
        (
            self.i0 + x as i64 * self.ex.0 + z as i64 * self.ez.0,
            self.j0 + x as i64 * self.ex.1 + z as i64 * self.ez.1,
        )
    }

    /// The planet cell of rocket block `p`, if inside the planet's layers.
    pub fn cell(&self, planet: &Planet, p: IVec3) -> Option<CellId> {
        let (i, j) = self.ij(p.x, p.z);
        planet
            .grid
            .cell_on_face(self.face, i, j, self.k0 + p.y as i64)
    }

    /// Rocket coordinates of a planet cell.
    pub fn local_of(&self, planet: &Planet, c: CellId) -> IVec3 {
        let g = &planet.grid;
        let (a, b) = g.face_coords(self.face, g.center(c).as_dvec3());
        let (di, dj) = (a.floor() as i64 - self.i0, b.floor() as i64 - self.j0);
        let k = g.coords(c).3 as i64;
        // ex and ez are unit steps at right angles: the inverse is the transpose.
        let x = di * self.ex.0 + dj * self.ex.1;
        let z = di * self.ez.0 + dj * self.ez.1;
        IVec3::new(x as i32, (k - self.k0) as i32, z as i32)
    }

    /// The planet-local point at rocket coordinates `p` (any point, not only a block's corner).
    pub fn point(&self, planet: &Planet, p: Vec3) -> Vec3 {
        let (x, z) = (p.x as f64 - 0.5, p.z as f64 - 0.5);
        let a = self.i0 as f64 + 0.5 + x * self.ex.0 as f64 + z * self.ez.0 as f64;
        let b = self.j0 as f64 + 0.5 + x * self.ex.1 as f64 + z * self.ez.1 as f64;
        let r = planet.grid.core as f64 + self.k0 as f64 + p.y as f64;
        planet.grid.face_point(self.face, a, b, r).as_vec3()
    }

    /// Where the middle of the rocket's base is, planet-local: where it stands.
    pub fn base(&self, planet: &Planet) -> Vec3 {
        self.point(planet, Vec3::new(0.5, 0.0, 0.5))
    }

    /// The rocket's turn on the planet (rocket axes → planet-local): up along the planet's
    /// radius at the base, +z along the grid there.
    pub fn turn(&self, planet: &Planet) -> Mat3 {
        let base = self.base(planet);
        let up = base.normalize();
        let z = (self.point(planet, Vec3::new(0.5, 0.0, 1.5)) - base)
            .reject_from(up)
            .normalize();
        Mat3::from_cols(up.cross(z), up, z)
    }
}

/// A rocket gathered from the ground: the rocket, how it lay there, and the cells it took.
pub struct Assembly {
    pub rocket: Station,
    pub lay: Lay,
    pub cells: Vec<CellId>,
}

/// True if the rocket's footprint at `lay` has a pad: the 3 × 3 under its middle solid, and
/// nothing solid on it but the rocket's own blocks (`own`, rocket coordinates). If not, the holes
/// in the pad, or what stands on it.
fn pad_ok(planet: &Planet, lay: &Lay, own: impl Fn(IVec3) -> bool) -> Result<(), Fault> {
    let (mut holes, mut taken) = (Vec::new(), Vec::new());
    for x in -PAD..=PAD {
        for z in -PAD..=PAD {
            let under = lay
                .cell(planet, IVec3::new(x, -1, z))
                .ok_or(Problem::NoGround)?;
            if !block::is_solid(planet.get(under)) {
                holes.push(under);
            }
            let on = IVec3::new(x, 0, z);
            if let Some(c) = lay
                .cell(planet, on)
                .filter(|&c| !own(on) && block::is_solid(planet.get(c)))
            {
                taken.push(c);
            }
        }
    }
    if !holes.is_empty() {
        return Err(Fault {
            problem: Problem::NotFlat,
            cells: holes,
            first: None,
        });
    }
    if let Some(&c) = taken.first() {
        return Err(Fault {
            problem: Problem::PadTaken,
            first: Some((planet.get(c), 0)),
            cells: taken,
        });
    }
    Ok(())
}

/// Gathers the rocket whose helm is in cell `helm`: every block the player placed touching it,
/// through others (faces, not edges). `facing` (planet-local) is the way it will face: its +z.
pub fn assemble(
    planet: &Planet,
    helm: CellId,
    facing: Vec3,
    id: String,
    name: &str,
) -> Result<Assembly, Fault> {
    let g = &planet.grid;
    let (face, i, j, k) = g.coords(helm);
    let at_helm = Lay::snapped(planet, face, i as i64, j as i64, k as i64, facing);
    // Flood fill through placed blocks.
    let mut seen = HashSet::from([helm]);
    let mut queue = VecDeque::from([helm]);
    while let Some(c) = queue.pop_front() {
        for s in crate::sphere::Side::ALL {
            let Some(n) = g.neighbor(c, s) else { continue };
            // Never the pad it stands on.
            if planet.get(n) != block::AIR
                && !block::is_pad(planet.get(n))
                && planet.is_placed(n)
                && seen.insert(n)
            {
                if seen.len() > MAX_BLOCKS {
                    return Err(Problem::TooBig.into());
                }
                queue.push_back(n);
            }
        }
    }
    let cells: Vec<CellId> = seen.into_iter().collect();
    let local: Vec<IVec3> = cells.iter().map(|&c| at_helm.local_of(planet, c)).collect();
    // The origin is where it stands: the block of its lowest layer with the most of that layer
    // in the 3 × 3 around it (its base, not a step or a fin), nearest the middle on a tie.
    let low = local.iter().map(|p| p.y).min().unwrap();
    let bottom: Vec<IVec3> = local.iter().copied().filter(|p| p.y == low).collect();
    let (lo, hi) = bottom
        .iter()
        .fold((bottom[0], bottom[0]), |(a, b), &p| (a.min(p), b.max(p)));
    let mid = (lo + hi).as_vec3() * 0.5;
    let around = |c: IVec3| {
        bottom
            .iter()
            .filter(|p| (p.x - c.x).abs() <= PAD && (p.z - c.z).abs() <= PAD)
            .count()
    };
    let origin = *bottom
        .iter()
        .min_by(|a, b| {
            around(**b)
                .cmp(&around(**a))
                .then(
                    a.as_vec3()
                        .distance_squared(mid)
                        .total_cmp(&b.as_vec3().distance_squared(mid)),
                )
                .then(a.to_array().cmp(&b.to_array()))
        })
        .unwrap();
    let (i0, j0) = at_helm.ij(origin.x, origin.z);
    let lay = Lay {
        i0,
        j0,
        k0: at_helm.k0 + low as i64,
        ..at_helm
    };
    let blocks: HashMap<IVec3, Block> = cells
        .iter()
        .zip(&local)
        .map(|(&c, &p)| (p - origin, planet.get(c)))
        .collect();
    if blocks
        .keys()
        .any(|&p| !Station::in_limits(p) || p.y > MAX_Y)
    {
        return Err(Problem::TooBig.into());
    }
    if !blocks.values().any(|&b| b == block::ENGINE) {
        return Err(Problem::NoEngine.into());
    }
    pad_ok(planet, &lay, |p| blocks.contains_key(&p))?;
    // Open sky over every column of it, and nothing of the ground among its blocks.
    let mut columns: HashMap<(i32, i32), i32> = HashMap::new();
    for p in blocks.keys() {
        let low = columns.entry((p.x, p.z)).or_insert(p.y);
        *low = (*low).min(p.y);
    }
    let top = g.layers as i64 - lay.k0;
    let mut over: Vec<(i32, CellId)> = Vec::new();
    for (&(x, z), &low) in &columns {
        for y in low..top as i32 {
            let p = IVec3::new(x, y, z);
            if blocks.contains_key(&p) {
                continue;
            }
            if let Some(c) = lay
                .cell(planet, p)
                .filter(|&c| block::is_solid(planet.get(c)))
            {
                over.push((y, c));
            }
        }
    }
    if !over.is_empty() {
        over.sort();
        let first = Some((planet.get(over[0].1), over[0].0));
        let cells = over.into_iter().take(MAX_MARKED).map(|(_, c)| c).collect();
        return Err(Fault {
            problem: Problem::Blocked,
            cells,
            first,
        });
    }
    let helm_at = at_helm.local_of(planet, helm) - origin;
    let rocket = Station::rocket(id, name, blocks, helm_at);
    Ok(Assembly { rocket, lay, cells })
}

/// Where a rocket's blocks would go to stand at `lay`, or why they cannot: the pad under it, and
/// every block of it into a cell with nothing solid in it.
pub fn landing(
    planet: &Planet,
    rocket: &Station,
    lay: &Lay,
) -> Result<Vec<(CellId, Block)>, Problem> {
    let own: Vec<(IVec3, Block)> = rocket.cells().filter(|&(_, b)| b != block::AIR).collect();
    pad_ok(planet, lay, |p| rocket.get(p) != block::AIR).map_err(|f| f.problem)?;
    own.into_iter()
        .map(|(p, b)| {
            let c = lay.cell(planet, p).ok_or(Problem::Blocked)?;
            if block::is_solid(planet.get(c)) {
                return Err(Problem::Blocked);
            }
            Ok((c, b))
        })
        .collect()
}

/// The topmost solid layer of a column (grid coordinates on `face`), if any.
pub fn ground(planet: &Planet, face: usize, i: i64, j: i64) -> Option<i64> {
    let top = planet.grid.layers as i64;
    (0..top).rev().find(|&k| {
        planet
            .grid
            .cell_on_face(face, i, j, k)
            .is_some_and(|c| block::is_solid(planet.get(c)))
    })
}

/// The nearest place around the planet-local point `near` where `rocket` can stand facing
/// `forward`: how it would lie there. A ready landing pad (within `LANDING_PAD_SEARCH` columns)
/// first, else flat ground within `PAD_SEARCH`.
pub fn find_pad(planet: &Planet, rocket: &Station, near: Vec3, forward: Vec3) -> Option<Lay> {
    find_landing_pad(planet, rocket, near, forward)
        .map(|(lay, _)| lay)
        .or_else(|| find_ground(planet, rocket, near, forward))
}

/// The nearest ready landing pad around `near` where `rocket` fits, facing `forward`: how it
/// would lie there (its middle over the pad's middle), and the pad's side.
pub fn find_landing_pad(
    planet: &Planet,
    rocket: &Station,
    near: Vec3,
    forward: Vec3,
) -> Option<(Lay, i32)> {
    let g = &planet.grid;
    let p = near.as_dvec3();
    let face = g.face_of(p);
    let (a, b) = g.face_coords(face, p);
    let (ci, cj) = (a.floor() as i64, b.floor() as i64);
    let r = LANDING_PAD_SEARCH;
    let mut spots: Vec<(i64, i64)> = (-r..=r)
        .flat_map(|di| (-r..=r).map(move |dj| (di, dj)))
        .collect();
    spots.sort_by_key(|&(di, dj)| di * di + dj * dj);
    let mut seen: HashSet<CellId> = HashSet::new();
    for (di, dj) in spots {
        let (i, j) = (ci + di, cj + dj);
        let Some(k) = ground(planet, face, i, j) else {
            continue;
        };
        let Some(c) = g.cell_on_face(face, i, j, k) else {
            continue;
        };
        if planet.get(c) != block::PAD_READY || seen.contains(&c) {
            continue;
        }
        let (cells, _, square) = crate::pad::on_planet(planet, c);
        seen.extend(cells);
        let Some(sq) = square else { continue };
        // The square is in the coordinates of a lay with its origin at `c`.
        let (cf, ci0, cj0, ck) = g.coords(c);
        let at = Lay::snapped(planet, cf, ci0 as i64, cj0 as i64, ck as i64, Vec3::Z);
        let (mx, mz) = sq.middle();
        let (mi, mj) = at.ij(mx, mz);
        let lay = Lay::snapped(planet, cf, mi, mj, ck as i64 + 1, forward);
        if landing(planet, rocket, &lay).is_ok() {
            return Some((lay, sq.side));
        }
    }
    None
}

/// The nearest flat ground around `near` (within `PAD_SEARCH` columns) where `rocket` can stand.
fn find_ground(planet: &Planet, rocket: &Station, near: Vec3, forward: Vec3) -> Option<Lay> {
    let g = &planet.grid;
    let p = near.as_dvec3();
    let face = g.face_of(p);
    let (a, b) = g.face_coords(face, p);
    let (ci, cj) = (a.floor() as i64, b.floor() as i64);
    let mut spots: Vec<(i64, i64)> = (-PAD_SEARCH..=PAD_SEARCH)
        .flat_map(|di| (-PAD_SEARCH..=PAD_SEARCH).map(move |dj| (di, dj)))
        .collect();
    spots.sort_by_key(|&(di, dj)| di * di + dj * dj);
    spots.into_iter().find_map(|(di, dj)| {
        let (i, j) = (ci + di, cj + dj);
        let k = ground(planet, face, i, j)? + 1;
        let lay = Lay::snapped(planet, face, i, j, k, forward);
        landing(planet, rocket, &lay).ok().map(|_| lay)
    })
}

// --- On stations ------------------------------------------------------------------------------

/// How a rocket stands on a station: rocket block `r` is station block `origin + turn(r)`, where
/// `turn` turns about +y by `turns` quarter turns (so its front may face any of the station's
/// four ways).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Berth {
    pub origin: IVec3,
    pub turns: u8,
}

/// `p` turned about +y by `k` quarter turns, as `Quat::from_rotation_y(k · 90°)` turns it.
pub fn quarter(p: IVec3, k: u8) -> IVec3 {
    (0..k % 4).fold(p, |p, _| IVec3::new(p.z, p.y, -p.x))
}

impl Berth {
    /// Station block of rocket block `r`.
    pub fn cell(&self, r: IVec3) -> IVec3 {
        self.origin + quarter(r, self.turns)
    }

    /// Rocket block of station block `s`.
    pub fn local(&self, s: IVec3) -> IVec3 {
        quarter(s - self.origin, (4 - self.turns % 4) % 4)
    }

    /// The rocket's turn on the station (rocket axes → station axes).
    pub fn rotation(&self) -> glam::Quat {
        glam::Quat::from_rotation_y(self.turns as f32 * std::f32::consts::FRAC_PI_2)
    }

    /// Station point of rocket point `q` (any point): cells line up, block for block.
    pub fn point(&self, q: Vec3) -> Vec3 {
        let h = Vec3::splat(0.5);
        self.origin.as_vec3() + h + self.rotation() * (q - h)
    }

    /// The turn whose front is nearest `facing` (station coordinates).
    pub fn turns_toward(facing: Vec3) -> u8 {
        (0..4u8)
            .max_by(|&a, &b| {
                quarter(IVec3::Z, a)
                    .as_vec3()
                    .dot(facing)
                    .total_cmp(&quarter(IVec3::Z, b).as_vec3().dot(facing))
            })
            .unwrap()
    }
}

/// A ready pad's blocks under the rocket's middle 3 × 3, at `berth`.
fn on_station_pad(s: &Station, berth: &Berth) -> bool {
    (-PAD..=PAD)
        .all(|x| (-PAD..=PAD).all(|z| s.get(berth.cell(IVec3::new(x, -1, z))) == block::PAD_READY))
}

/// Where a rocket's blocks would go on station `s` at `berth`, or why they cannot: a ready pad
/// under its middle, and every block of it into an empty cell within the station's limits.
pub fn berth_cells(
    s: &Station,
    rocket: &Station,
    berth: &Berth,
) -> Result<Vec<(IVec3, Block)>, Problem> {
    if !on_station_pad(s, berth) {
        return Err(Problem::NoStationPad);
    }
    rocket
        .cells()
        .filter(|&(_, b)| b != block::AIR)
        .map(|(r, b)| {
            let p = berth.cell(r);
            if !Station::in_limits(p) {
                return Err(Problem::TooBig);
            }
            if s.get(p) != block::AIR {
                return Err(Problem::Blocked);
            }
            Ok((p, b))
        })
        .collect()
}

/// The ready pad on station `s` nearest `near` (station coordinates) where `rocket` fits, its
/// front as near `facing` as it can: how it would stand there.
pub fn find_berth(s: &Station, rocket: &Station, near: Vec3, facing: Vec3) -> Option<Berth> {
    let first = Berth::turns_toward(facing);
    let mut pads = crate::pad::station_squares(s);
    let at = |(sq, y): &(crate::pad::Square, i32)| {
        let (x, z) = sq.middle();
        IVec3::new(x, y + 1, z)
    };
    pads.sort_by(|a, b| {
        at(a)
            .as_vec3()
            .distance_squared(near)
            .total_cmp(&at(b).as_vec3().distance_squared(near))
    });
    pads.iter().find_map(|p| {
        (0..4)
            .map(|k| (first + k) % 4)
            .map(|turns| Berth {
                origin: at(p),
                turns,
            })
            .find(|b| berth_cells(s, rocket, b).is_ok())
    })
}

/// A rocket gathered from a station: the rocket, how it stood there, and the station blocks it
/// took.
pub struct StationAssembly {
    pub rocket: Station,
    pub berth: Berth,
    pub cells: Vec<IVec3>,
}

/// Gathers the rocket whose helm is at `helm` on station `s`: every block touching it, through
/// others, but the pad it stands on and the station's core. It must stand on a ready pad, with
/// nothing of the station over it. `facing` (station coordinates) is the way it will face.
pub fn assemble_on_station(
    s: &Station,
    helm: IVec3,
    facing: Vec3,
    id: String,
    name: &str,
) -> Result<StationAssembly, Problem> {
    let mut seen = HashSet::from([helm]);
    let mut queue = VecDeque::from([helm]);
    let sides = [
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Y,
        IVec3::NEG_Y,
        IVec3::Z,
        IVec3::NEG_Z,
    ];
    while let Some(p) = queue.pop_front() {
        for d in sides {
            let n = p + d;
            let b = s.get(n);
            if b != block::AIR && !block::is_pad(b) && !s.is_core(n) && seen.insert(n) {
                if seen.len() > MAX_BLOCKS {
                    return Err(Problem::TooBig);
                }
                queue.push_back(n);
            }
        }
    }
    let cells: Vec<IVec3> = seen.into_iter().collect();
    // Its base, as on a planet: the block of its lowest layer with the most of it around.
    let low = cells.iter().map(|p| p.y).min().unwrap();
    let bottom: Vec<IVec3> = cells.iter().copied().filter(|p| p.y == low).collect();
    let (lo, hi) = bottom
        .iter()
        .fold((bottom[0], bottom[0]), |(a, b), &p| (a.min(p), b.max(p)));
    let mid = (lo + hi).as_vec3() * 0.5;
    let around = |c: IVec3| {
        bottom
            .iter()
            .filter(|p| (p.x - c.x).abs() <= PAD && (p.z - c.z).abs() <= PAD)
            .count()
    };
    let origin = *bottom
        .iter()
        .min_by(|a, b| {
            around(**b)
                .cmp(&around(**a))
                .then(
                    a.as_vec3()
                        .distance_squared(mid)
                        .total_cmp(&b.as_vec3().distance_squared(mid)),
                )
                .then(a.to_array().cmp(&b.to_array()))
        })
        .unwrap();
    let berth = Berth {
        origin,
        turns: Berth::turns_toward(facing),
    };
    let blocks: HashMap<IVec3, Block> = cells.iter().map(|&p| (berth.local(p), s.get(p))).collect();
    if blocks
        .keys()
        .any(|&p| !Station::in_limits(p) || p.y > MAX_Y)
    {
        return Err(Problem::TooBig);
    }
    if !blocks.values().any(|&b| b == block::ENGINE) {
        return Err(Problem::NoEngine);
    }
    if !on_station_pad(s, &berth) {
        return Err(Problem::NoStationPad);
    }
    // Nothing of the station over it.
    let own: HashSet<IVec3> = cells.iter().copied().collect();
    let mut columns: HashMap<(i32, i32), i32> = HashMap::new();
    for p in &cells {
        let low = columns.entry((p.x, p.z)).or_insert(p.y);
        *low = (*low).min(p.y);
    }
    for (&(x, z), &low) in &columns {
        if (low..=crate::station::MAX_Y).any(|y| {
            let p = IVec3::new(x, y, z);
            !own.contains(&p) && s.solid(p)
        }) {
            return Err(Problem::Blocked);
        }
    }
    let rocket = Station::rocket(id, name, blocks, berth.local(helm));
    Ok(StationAssembly {
        rocket,
        berth,
        cells,
    })
}

/// The rocket `/rocket build` puts up: three engines across on a 3 × 3 skirt, a 5 × 5 cabin with
/// windows all round, the helm at the front between two chests, a light in the roof, a nose, two
/// fins, and steps up to the door at the back.
pub fn starter(id: String, name: &str) -> Station {
    let mut b: Vec<(IVec3, Block)> = Vec::new();
    let mut put = |x: i32, y: i32, z: i32, k: Block| b.push((IVec3::new(x, y, z), k));
    for x in -1..=1 {
        for z in -1..=1 {
            put(x, 0, z, block::ENGINE);
        }
    }
    for x in -2i32..=2 {
        for z in -2i32..=2 {
            let ring = x.abs() == 2 || z.abs() == 2;
            let corner = x.abs() == 2 && z.abs() == 2;
            put(x, 1, z, block::HULL);
            put(
                x,
                5,
                z,
                if x == 0 && z == 0 {
                    block::CRYSTAL
                } else {
                    block::HULL
                },
            );
            if ring {
                let door = x == 0 && z == -2;
                if !door {
                    put(x, 2, z, block::HULL);
                    put(x, 4, z, block::HULL);
                }
                // Windows all round at eye height, but at the corners and beside the door.
                if !door {
                    put(
                        x,
                        3,
                        z,
                        if corner || (z == -2 && x.abs() == 1) {
                            block::HULL
                        } else {
                            block::GLASS
                        },
                    );
                }
            }
        }
    }
    for x in -1..=1 {
        for z in -1..=1 {
            put(x, 6, z, block::HULL);
            if x == 0 || z == 0 {
                put(x, 7, z, block::HULL);
            }
        }
    }
    put(0, 8, 0, block::GLASS);
    // Inside: the helm at the front, a chest either side.
    put(0, 2, 1, block::HELM);
    put(-1, 2, 1, block::CHEST);
    put(1, 2, 1, block::CHEST);
    // Fins, and steps up to the door.
    put(-3, 1, 0, block::HULL);
    put(3, 1, 0, block::HULL);
    put(0, 1, -3, block::HULL);
    put(0, 0, -3, block::HULL);
    put(0, 0, -4, block::HULL);
    Station::rocket(id, name, b, IVec3::new(0, 2, 1))
}

/// A rocket's block count and engines, for messages.
pub fn summary(rocket: &Station) -> (usize, usize) {
    rocket.cells().fold((0, 0), |(n, e), (_, b)| {
        (
            n + (b != block::AIR) as usize,
            e + (b == block::ENGINE) as usize,
        )
    })
}

/// Rocket (local) → world turn, from the world directions of its up and its front.
pub fn turn_from(up: Vec3, forward: Vec3) -> glam::Quat {
    let up = up.normalize();
    let z = forward
        .reject_from(up)
        .normalize_or(up.any_orthonormal_vector());
    glam::Quat::from_mat3(&Mat3::from_cols(up.cross(z), up, z)).normalize()
}

/// The rocket's origin (world) given where the middle of its base is (world) and its turn.
pub fn origin_from_base(base: DVec3, turn: glam::Quat) -> DVec3 {
    base - (turn * Vec3::new(0.5, 0.0, 0.5)).as_dvec3()
}

/// Slower than this (blocks/s) a flown rocket's nose does not follow its way: in the air it
/// stands up, in space it keeps pointing where it did. It follows fully from `NOSE_FULL` on.
pub const NOSE_SLOW: f32 = 1.5;
pub const NOSE_FULL: f32 = 8.0;

/// Where a flown rocket's nose should point (world): along the way it goes (`vel`, against its
/// planet or orbit frame). In a planet's air (`in_air`) it never points down: coming down it comes
/// down tail first, as a landing rocket does, and slow or on the ground it stands up along `up`
/// (away from the ground). In space, slow, it keeps pointing as it did (`before`).
pub fn nose_toward(up: Vec3, vel: Vec3, in_air: bool, on_ground: bool, before: Vec3) -> Vec3 {
    let up = up.normalize();
    let speed = vel.length();
    let w = ((speed - NOSE_SLOW) / (NOSE_FULL - NOSE_SLOW)).clamp(0.0, 1.0);
    if !in_air {
        return if w > 0.0 {
            before.lerp(vel / speed, w).normalize_or(before)
        } else {
            before
        };
    }
    if on_ground {
        return up;
    }
    let vy = vel.dot(up);
    let way = (vel - up * vy + up * vy.abs()).normalize_or(up);
    up.lerp(way, w).normalize_or(up)
}

/// The rocket's origin (world) when it turns about its middle: its middle (rocket coordinates
/// `middle`, on the column through the middle of its base) where it would be standing on `base`
/// along `up`, so a rocket tipping over does not swing through the ground.
pub fn origin_about_middle(base: DVec3, up: Vec3, height: f32, turn: glam::Quat) -> DVec3 {
    base + (up.normalize() * height).as_dvec3() - (turn * Vec3::new(0.5, height, 0.5)).as_dvec3()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::{Biome, PlanetSpec};

    fn flat_planet() -> Planet {
        // A Barren planet with no relief: flat ground everywhere.
        let mut spec = PlanetSpec::new(7, "Flat".into(), 90.0, Biome::Barren);
        spec.knobs.relief = 0.0;
        spec.knobs.mountains = 0.0;
        spec.knobs.caves = 0.0;
        spec.knobs.trees = 0.0;
        spec.knobs.grass = 0.0;
        spec.knobs.flowers = 0.0;
        Planet::generate(spec)
    }

    /// The middle of a face, at the ground: (face, i, j, first air layer).
    fn spot(p: &Planet) -> (usize, i64, i64, i64) {
        let n = p.grid.n as i64;
        let (i, j) = (n / 2, n / 2);
        (0, i, j, ground(p, 0, i, j).unwrap() + 1)
    }

    fn build(p: &mut Planet, lay: &Lay, s: &Station) {
        for (q, b) in s.cells().filter(|&(_, b)| b != block::AIR) {
            let c = lay.cell(p, q).unwrap();
            p.set(c, b);
        }
    }

    #[test]
    fn lay_round_trips_and_is_not_mirrored() {
        let p = flat_planet();
        let (f, i, j, k) = spot(&p);
        for fwd in [Vec3::X, Vec3::NEG_Z, Vec3::Y, Vec3::new(0.3, -1.0, 0.2)] {
            let lay = Lay::snapped(&p, f, i, j, k, fwd);
            for q in [IVec3::ZERO, IVec3::new(2, 3, -1), IVec3::new(-2, 0, 2)] {
                let c = lay.cell(&p, q).unwrap();
                assert_eq!(lay.local_of(&p, c), q);
            }
            // The rocket's own axes, measured on the grid, turn the right way: x × y = z.
            let t = lay.turn(&p);
            assert!(t.x_axis.cross(t.y_axis).dot(t.z_axis) > 0.99);
            let x = lay.point(&p, Vec3::new(1.5, 0.0, 0.5)) - lay.base(&p);
            assert!(x.normalize().dot(t.x_axis) > 0.99, "x runs along +x");
        }
    }

    #[test]
    fn a_built_rocket_assembles_and_lands_back() {
        let mut p = flat_planet();
        let (f, i, j, k) = spot(&p);
        let lay = Lay::snapped(&p, f, i, j, k, Vec3::X);
        let kit = starter("00000001".into(), "");
        assert!(
            landing(&p, &kit, &lay).is_ok(),
            "the kit fits on flat ground"
        );
        build(&mut p, &lay, &kit);
        let helm = lay.cell(&p, kit.core).unwrap();
        assert_eq!(p.get(helm), block::HELM);
        let a = assemble(&p, helm, lay.turn(&p).z_axis, "00000002".into(), "Test").unwrap();
        assert_eq!(summary(&a.rocket), summary(&kit));
        assert_eq!(a.rocket.core, kit.core);
        assert_eq!(a.lay.k0, lay.k0);
        assert_eq!(a.cells.len(), summary(&kit).0);
        // Taken off the ground, it can come back where it was.
        for &c in &a.cells {
            p.set(c, block::AIR);
        }
        let back = landing(&p, &a.rocket, &a.lay).unwrap();
        assert_eq!(back.len(), a.cells.len());
    }

    #[test]
    fn natural_ground_is_never_part_of_a_rocket() {
        let mut p = flat_planet();
        let (f, i, j, k) = spot(&p);
        let lay = Lay::snapped(&p, f, i, j, k, Vec3::X);
        build(&mut p, &lay, &starter("00000001".into(), ""));
        let helm = lay.cell(&p, IVec3::new(0, 2, 1)).unwrap();
        let a = assemble(&p, helm, lay.turn(&p).z_axis, "00000002".into(), "").unwrap();
        assert!(a.cells.iter().all(|&c| p.is_placed(c)));
        assert!(a.rocket.cells().all(|(q, _)| q.y >= 0));
    }

    #[test]
    fn needs_an_engine_a_flat_pad_and_open_sky() {
        let (f, i, j, k);
        let base = flat_planet();
        (f, i, j, k) = spot(&base);
        let lay = Lay::snapped(&base, f, i, j, k, Vec3::X);
        let kit = starter("00000001".into(), "");
        let helm = |p: &Planet| lay.cell(p, kit.core).unwrap();
        let facing = lay.turn(&base).z_axis;

        // No engines.
        let mut p = flat_planet();
        build(&mut p, &lay, &kit);
        for x in -1..=1 {
            for z in -1..=1 {
                p.set(lay.cell(&p, IVec3::new(x, 0, z)).unwrap(), block::HULL);
            }
        }
        assert_eq!(
            assemble(&p, helm(&p), facing, "a".into(), "")
                .err()
                .map(|f| f.problem),
            Some(Problem::NoEngine)
        );

        // A hole in the pad.
        let mut p = flat_planet();
        build(&mut p, &lay, &kit);
        p.set(lay.cell(&p, IVec3::new(1, -1, 1)).unwrap(), block::AIR);
        assert_eq!(
            assemble(&p, helm(&p), facing, "a".into(), "")
                .err()
                .map(|f| f.problem),
            Some(Problem::NotFlat)
        );

        // A roof over it (not part of it: not touching).
        let mut p = flat_planet();
        build(&mut p, &lay, &kit);
        let roof = lay.cell(&p, IVec3::new(0, 12, 0)).unwrap();
        p.set(roof, block::STONE);
        let f = assemble(&p, helm(&p), facing, "a".into(), "")
            .err()
            .unwrap();
        assert_eq!(f.problem, Problem::Blocked);
        // It says what and where, and marks it.
        assert_eq!(f.cells, vec![roof]);
        assert_eq!(f.first, Some((block::STONE, 12)));
        assert!(f.message().contains("12 blocks up"), "{}", f.message());
    }

    #[test]
    fn the_nose_points_the_way_it_goes() {
        let (up, fwd) = (Vec3::Y, Vec3::Z);
        // Standing, or slow in the air: upright.
        assert_eq!(nose_toward(up, Vec3::ZERO, true, true, fwd), up);
        assert!(nose_toward(up, Vec3::new(0.0, 0.0, 1.0), true, false, fwd).dot(up) > 0.999);
        // Flying along: along its way; climbing: up its way.
        assert!(nose_toward(up, Vec3::new(0.0, 0.0, 40.0), true, false, up).dot(fwd) > 0.999);
        let climb = Vec3::new(0.0, 30.0, 30.0);
        assert!(nose_toward(up, climb, true, false, up).dot(climb.normalize()) > 0.999);
        // Coming down: tail first, never nose down.
        assert!(nose_toward(up, Vec3::new(0.0, -30.0, 0.0), true, false, up).dot(up) > 0.999);
        assert!(nose_toward(up, Vec3::new(0.0, -30.0, 30.0), true, false, up).y > 0.5);
        // In space: prograde, and as it was when still.
        let v = Vec3::new(300.0, 0.0, 10.0);
        assert!(nose_toward(up, v, false, false, up).dot(v.normalize()) > 0.999);
        assert_eq!(nose_toward(up, Vec3::ZERO, false, false, fwd), fwd);
    }

    #[test]
    fn turning_about_the_middle() {
        let base = DVec3::new(10.0, 20.0, 30.0);
        let (up, fwd) = (Vec3::Y, Vec3::Z);
        // Upright it is where it stands on its base.
        let t = turn_from(up, fwd);
        let o = origin_about_middle(base, up, 4.0, t);
        assert!(o.distance(origin_from_base(base, t)) < 1e-4);
        // Tipped forward its middle stays put, its front tips down and its nose is the nose.
        let nose = Vec3::new(0.0, 0.2, 1.0).normalize();
        let t = turn_from(nose, glam::Quat::from_rotation_arc(up, nose) * fwd);
        assert!((t * Vec3::Y).dot(nose) > 0.9999);
        assert!((t * Vec3::Z).y < -0.9);
        let o = origin_about_middle(base, up, 4.0, t);
        let middle = o + (t * Vec3::new(0.5, 4.0, 0.5)).as_dvec3();
        assert!(middle.distance(base + DVec3::Y * 4.0) < 1e-4);
    }

    #[test]
    fn finds_a_pad_nearby() {
        let mut p = flat_planet();
        let (f, i, j, k) = spot(&p);
        let kit = starter("00000001".into(), "");
        // Rough ground right here: a pillar in the middle.
        let here = Lay::snapped(&p, f, i, j, k, Vec3::X);
        let c = here.cell(&p, IVec3::new(0, 0, 0)).unwrap();
        p.set(c, block::STONE);
        assert!(landing(&p, &kit, &here).is_err());
        let near = here.base(&p) + here.base(&p).normalize() * 20.0;
        let lay = find_pad(&p, &kit, near, here.turn(&p).z_axis).expect("a pad nearby");
        assert!(landing(&p, &kit, &lay).is_ok());
        assert_ne!((lay.i0, lay.j0), (here.i0, here.j0));
    }

    #[test]
    fn lands_on_a_ready_landing_pad_and_never_takes_it_along() {
        let mut p = flat_planet();
        let (f, i, j, k) = spot(&p);
        // A 5 × 5 pad on the ground, a few columns off: laid one by one, it lights up when full.
        let lay = Lay::snapped(&p, f, i + 9, j + 4, k, Vec3::Z);
        let mut last = None;
        for x in 0..5 {
            for z in 0..5 {
                let c = lay.cell(&p, IVec3::new(x, 0, z)).unwrap();
                p.set(c, block::PAD);
                last = crate::pad::refresh_planet(&mut p, c).1;
            }
        }
        assert_eq!(last, Some(crate::pad::Shape::Square { side: 5 }));
        assert_eq!(
            p.get(lay.cell(&p, IVec3::new(2, 0, 2)).unwrap()),
            block::PAD_READY
        );
        // A landing near goes to its middle, though flat ground is nearer.
        let kit = starter("r".into(), "");
        let near = Lay::snapped(&p, f, i, j, k, Vec3::Z).base(&p);
        let (on, side) = find_landing_pad(&p, &kit, near, Vec3::Z).expect("the pad");
        assert_eq!(side, 5);
        let mid = lay.cell(&p, IVec3::new(2, 1, 2)).unwrap();
        assert_eq!(on.cell(&p, IVec3::ZERO), Some(mid));
        assert_eq!(find_pad(&p, &kit, near, Vec3::Z), Some(on));
        // Landed there, it goes up again without its pad.
        for (c, b) in landing(&p, &kit, &on).unwrap() {
            p.set(c, b);
        }
        let helm = on.cell(&p, kit.core).unwrap();
        let a = assemble(&p, helm, on.turn(&p).z_axis, "r".into(), "").expect("goes up");
        assert!(a.cells.iter().all(|&c| !block::is_pad(p.get(c))));
        assert_eq!(a.rocket.block_count(), kit.block_count());
    }

    #[test]
    fn lands_on_a_station_pad_and_goes_up_again() {
        let mut st = Station::new("s".into(), "", block::SMOOTH_STONE);
        // Grown out and a 9 × 9 pad on it, off the core.
        for x in -12..=12 {
            for z in -2..=12 {
                st.set(IVec3::new(x, 0, z), block::SMOOTH_STONE);
            }
        }
        for x in -4..=4 {
            for z in 3..=11 {
                st.set(IVec3::new(x, 1, z), block::PAD);
            }
        }
        crate::pad::refresh_station(&mut st, IVec3::new(0, 1, 3));
        let kit = starter("r".into(), "");
        // Coming from somewhere over it, facing -x.
        let b = find_berth(&st, &kit, Vec3::new(0.0, 30.0, 0.0), Vec3::NEG_X).expect("a berth");
        assert_eq!(b.origin, IVec3::new(0, 2, 7));
        assert_eq!(quarter(IVec3::Z, b.turns), IVec3::NEG_X);
        let cells = berth_cells(&st, &kit, &b).unwrap();
        for &(c, k) in &cells {
            st.set(c, k);
        }
        // The pad is taken now.
        assert!(berth_cells(&st, &kit, &b).is_err());
        let helm = b.cell(kit.core);
        assert_eq!(st.get(helm), block::HELM);
        let a = assemble_on_station(&st, helm, Vec3::NEG_X, "r".into(), "").expect("goes up");
        assert_eq!(a.berth, b);
        assert_eq!(a.cells.len(), cells.len());
        assert_eq!(a.rocket.core, kit.core);
        assert!(kit.cells().all(|(q, k)| a.rocket.get(q) == k));
        // Points line up with cells.
        assert_eq!(b.point(Vec3::new(0.5, 0.0, 0.5)), Vec3::new(0.5, 2.0, 7.5));
        // Not on a pad: no.
        let mut bare = Station::new("t".into(), "", block::SMOOTH_STONE);
        for (q, k) in kit.cells().filter(|&(_, k)| k != block::AIR) {
            bare.set(q + IVec3::new(20, 1, 20), k);
        }
        assert_eq!(
            assemble_on_station(
                &bare,
                kit.core + IVec3::new(20, 1, 20),
                Vec3::Z,
                "r".into(),
                ""
            )
            .err(),
            Some(Problem::NoStationPad)
        );
        // A roof over it: no.
        st.set(helm + IVec3::new(0, 20, 0), block::STONE);
        assert_eq!(
            assemble_on_station(&st, helm, Vec3::NEG_X, "r".into(), "").err(),
            Some(Problem::Blocked)
        );
    }
}
