//! Landing pads: a multiblock. Landing Pad blocks laid side by side on one layer make a pad; when
//! they make a full square (every cell of an N × N, nothing sticking out, N from `MIN_SIDE` to
//! `MAX_SIDE`) the pad is **ready**: its blocks light up (they become `PAD_READY`) and rockets
//! come down on it, on a planet (`rocket::find_pad`) or on a station (`rocket::find_berth`).
//! Break one, or add one that spoils the square, and it goes dark again.
//!
//! The pad's blocks are never part of a rocket standing on it.

use crate::block::{self, Block};
use crate::planet::{ChunkKey, Planet};
use crate::sphere::{CellId, Side};
use crate::station::Station;
use glam::IVec3;
use std::collections::{HashSet, VecDeque};

/// The smallest and largest ready pads, blocks a side.
pub const MIN_SIDE: i32 = 3;
pub const MAX_SIDE: i32 = 32;
/// Most blocks one pad is looked at for: past this it is too big to be a square anyway.
const MAX_CELLS: usize = (MAX_SIDE * MAX_SIDE) as usize + 1;

/// What a pad (the pad blocks joined to one) is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// A full square, `side` blocks a side: ready.
    Square { side: i32 },
    /// Not a full square: how wide and long it is, and how many blocks it has.
    Unfinished { w: i32, l: i32, n: usize },
    /// Bigger than `MAX_SIDE` across.
    TooBig,
}

impl Shape {
    pub fn ready(self) -> bool {
        matches!(self, Shape::Square { .. })
    }

    pub fn message(self) -> String {
        match self {
            Shape::Square { side } => format!("Landing pad ready: {side}x{side}. Rockets can come down on it"),
            Shape::Unfinished { w, l, n } if w == l && n < (w * w) as usize => {
                format!("Landing pad {w}x{l}: {} block{} missing to fill the square", w * w - n as i32, if w * w - n as i32 == 1 { "" } else { "s" })
            }
            Shape::Unfinished { w, l, .. } if w.max(l) < MIN_SIDE => format!("Landing pad {w}x{l}: make it a full square, at least {MIN_SIDE}x{MIN_SIDE}"),
            Shape::Unfinished { w, l, .. } => format!("Landing pad {w}x{l}: make it a full square ({0}x{0} or {1}x{1})", w.min(l).max(MIN_SIDE), w.max(l)),
            Shape::TooBig => format!("Landing pad too big: at most {MAX_SIDE}x{MAX_SIDE}"),
        }
    }
}

/// A ready pad's square, in the coordinates its cells were given: (x0, z0) its first corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Square {
    pub x0: i32,
    pub z0: i32,
    pub side: i32,
}

impl Square {
    /// The block a rocket's middle stands over: the middle one (on even sides, the one before
    /// the middle).
    pub fn middle(&self) -> (i32, i32) {
        (self.x0 + (self.side - 1) / 2, self.z0 + (self.side - 1) / 2)
    }
}

/// What the cells (x, z) of one pad make.
pub fn shape_of(cells: &[(i32, i32)]) -> (Shape, Option<Square>) {
    if cells.len() >= MAX_CELLS {
        return (Shape::TooBig, None);
    }
    let (mut lo, mut hi) = ((i32::MAX, i32::MAX), (i32::MIN, i32::MIN));
    for &(x, z) in cells {
        lo = (lo.0.min(x), lo.1.min(z));
        hi = (hi.0.max(x), hi.1.max(z));
    }
    let (w, l) = (hi.0 - lo.0 + 1, hi.1 - lo.1 + 1);
    if w.max(l) > MAX_SIDE {
        return (Shape::TooBig, None);
    }
    // Cells are distinct, so as many as the box holds fill it.
    if w == l && w >= MIN_SIDE && cells.len() == (w * l) as usize {
        return (Shape::Square { side: w }, Some(Square { x0: lo.0, z0: lo.1, side: w }));
    }
    (Shape::Unfinished { w, l, n: cells.len() }, None)
}

// --- On a planet -------------------------------------------------------------------------

/// The pad blocks joined to `start` (through the four sides on its layer), at most `MAX_CELLS`.
fn planet_pad(planet: &Planet, start: CellId) -> Vec<CellId> {
    let g = &planet.grid;
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(c) = queue.pop_front() {
        for s in [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus] {
            let Some(n) = g.neighbor(c, s) else { continue };
            if block::is_pad(planet.get(n)) && seen.insert(n) {
                if seen.len() >= MAX_CELLS {
                    return seen.into_iter().collect();
                }
                queue.push_back(n);
            }
        }
    }
    seen.into_iter().collect()
}

/// The pad `start` is part of on a planet: its cells, what they make and, if a square, the
/// square in the coordinates of a `rocket::Lay` with its origin at `start`.
pub fn on_planet(planet: &Planet, start: CellId) -> (Vec<CellId>, Shape, Option<Square>) {
    let cells = planet_pad(planet, start);
    let (face, i, j, k) = planet.grid.coords(start);
    let lay = crate::rocket::Lay::snapped(planet, face, i as i64, j as i64, k as i64, glam::Vec3::Z);
    let local: Vec<(i32, i32)> = cells.iter().map(|&c| lay.local_of(planet, c)).map(|p| (p.x, p.z)).collect();
    let (shape, square) = shape_of(&local);
    (cells, shape, square)
}

/// After the block in cell `changed` changed: lights up the pads beside it (or it) that are
/// squares now, and puts out those that are not. The chunks to mesh again, and what the pad the
/// player touched is now (if it touched one).
pub fn refresh_planet(planet: &mut Planet, changed: CellId) -> (Vec<ChunkKey>, Option<Shape>) {
    let g = &planet.grid;
    let mut starts = vec![changed];
    starts.extend([Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus].into_iter().filter_map(|s| g.neighbor(changed, s)));
    let (mut dirty, mut done, mut said) = (Vec::new(), HashSet::new(), None);
    for s in starts {
        if !block::is_pad(planet.get(s)) || done.contains(&s) {
            continue;
        }
        let (cells, shape, _) = on_planet(planet, s);
        if s == changed || said.is_none() {
            said = Some(shape);
        }
        let want = if shape.ready() { block::PAD_READY } else { block::PAD };
        for &c in &cells {
            done.insert(c);
            if planet.get(c) != want {
                for k in planet.set(c, want) {
                    if !dirty.contains(&k) {
                        dirty.push(k);
                    }
                }
            }
        }
    }
    (dirty, said)
}

// --- On a station --------------------------------------------------------------------------

const FLAT: [IVec3; 4] = [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z];

/// The pad blocks joined to `start` on its layer of a station.
fn station_pad(s: &Station, start: IVec3) -> Vec<IVec3> {
    let mut seen = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(p) = queue.pop_front() {
        for d in FLAT {
            let n = p + d;
            if block::is_pad(s.get(n)) && seen.insert(n) {
                if seen.len() >= MAX_CELLS {
                    return seen.into_iter().collect();
                }
                queue.push_back(n);
            }
        }
    }
    seen.into_iter().collect()
}

/// The pad `start` is part of on a station: its cells, what they make and its square (station
/// x and z; it lies on layer `start.y`).
pub fn on_station(s: &Station, start: IVec3) -> (Vec<IVec3>, Shape, Option<Square>) {
    let cells = station_pad(s, start);
    let local: Vec<(i32, i32)> = cells.iter().map(|p| (p.x, p.z)).collect();
    let (shape, square) = shape_of(&local);
    (cells, shape, square)
}

/// `refresh_planet` on a station: true if any block changed.
pub fn refresh_station(s: &mut Station, changed: IVec3) -> (bool, Option<Shape>) {
    let (mut any, mut done, mut said) = (false, HashSet::new(), None);
    for p in std::iter::once(changed).chain(FLAT.map(|d| changed + d)) {
        if !block::is_pad(s.get(p)) || done.contains(&p) {
            continue;
        }
        let (cells, shape, _) = on_station(s, p);
        if p == changed || said.is_none() {
            said = Some(shape);
        }
        let want: Block = if shape.ready() { block::PAD_READY } else { block::PAD };
        for c in cells {
            done.insert(c);
            if s.get(c) != want {
                s.set(c, want);
                any = true;
            }
        }
    }
    (any, said)
}

/// Every ready pad on a station: its square and the layer it lies on.
pub fn station_squares(s: &Station) -> Vec<(Square, i32)> {
    let mut done = HashSet::new();
    let mut out = Vec::new();
    for (p, b) in s.cells() {
        if b != block::PAD_READY || done.contains(&p) {
            continue;
        }
        let (cells, _, square) = on_station(s, p);
        done.extend(cells);
        if let Some(sq) = square {
            out.push((sq, p.y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn square(x0: i32, z0: i32, w: i32, l: i32) -> Vec<(i32, i32)> {
        (x0..x0 + w).flat_map(|x| (z0..z0 + l).map(move |z| (x, z))).collect()
    }

    #[test]
    fn only_full_squares_are_ready() {
        assert_eq!(shape_of(&square(2, -3, 4, 4)).1, Some(Square { x0: 2, z0: -3, side: 4 }));
        assert_eq!(shape_of(&square(0, 0, 5, 5)).0, Shape::Square { side: 5 });
        assert!(!shape_of(&square(0, 0, 5, 4)).0.ready());
        assert!(!shape_of(&square(0, 0, 2, 2)).0.ready());
        let mut holed = square(0, 0, 4, 4);
        holed.retain(|&p| p != (1, 2));
        assert_eq!(shape_of(&holed).0, Shape::Unfinished { w: 4, l: 4, n: 15 });
        assert!(shape_of(&holed).0.message().contains("1 block missing"));
        let mut bump = square(0, 0, 4, 4);
        bump.push((4, 0));
        assert!(!shape_of(&bump).0.ready());
        assert_eq!(shape_of(&square(0, 0, 33, 33)).0, Shape::TooBig);
    }

    #[test]
    fn a_station_pad_lights_up_when_square_and_goes_dark_when_spoiled() {
        let mut s = Station::new("s".into(), "", block::SMOOTH_STONE);
        let at = |x, z| IVec3::new(x, 1, z);
        for x in 0..4 {
            for z in 0..4 {
                if (x, z) != (3, 3) {
                    s.set(at(x, z), block::PAD);
                    refresh_station(&mut s, at(x, z));
                }
            }
        }
        assert_eq!(s.get(at(0, 0)), block::PAD);
        s.set(at(3, 3), block::PAD);
        let (changed, said) = refresh_station(&mut s, at(3, 3));
        assert!(changed);
        assert_eq!(said, Some(Shape::Square { side: 4 }));
        assert!((0..4).all(|x| (0..4).all(|z| s.get(at(x, z)) == block::PAD_READY)));
        assert_eq!(station_squares(&s), vec![(Square { x0: 0, z0: 0, side: 4 }, 1)]);
        // One more beside it: not a square.
        s.set(at(4, 0), block::PAD);
        refresh_station(&mut s, at(4, 0));
        assert_eq!(s.get(at(0, 0)), block::PAD);
        // Gone again: ready again; then one broken: dark.
        s.set(at(4, 0), block::AIR);
        refresh_station(&mut s, at(4, 0));
        assert_eq!(s.get(at(2, 2)), block::PAD_READY);
        s.set(at(1, 1), block::AIR);
        refresh_station(&mut s, at(1, 1));
        assert_eq!(s.get(at(2, 2)), block::PAD);
    }
}
