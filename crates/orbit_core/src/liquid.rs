//! Flowing water and lava, the way Minecraft does it, on any grid of cells (a planet's cube
//! sphere, a station's box): see `Cells`.
//!
//! A source holds its liquid for good. Liquid falls straight down while it can (a falling
//! column is full but no source), and where it rests it spreads sideways, one level thinner per
//! cell (water reaches 7 cells, lava 3), heading for the nearest way down within a few cells
//! when there is one. Cut from its source it dries up again, from the top of the run. Water
//! between two sources on solid ground becomes a source itself (an endless pool).
//!
//! Where they meet: lava touching water turns to obsidian (a source) or cobblestone (flowing),
//! and lava falling into water turns it to stone.
//!
//! Nothing flows until something changes: edits `touch` the cells around them, which puts the
//! liquids there on the schedule, and each update touches what it changed in turn. Water moves
//! every `WATER_TICKS` ticks, lava every `LAVA_TICKS` (ticks are `TICK` seconds).

use crate::block::{self, Block, Liquid};
use std::collections::{BTreeMap, HashSet};
use std::hash::Hash;

/// Seconds per tick, like Minecraft's 20 a second.
pub const TICK: f32 = 0.05;
pub const WATER_TICKS: u64 = 5;
pub const LAVA_TICKS: u64 = 30;
/// Most cell updates in one tick (the rest wait for the next).
pub const BUDGET: usize = 3000;

/// A grid that liquids flow in. Positions outside it read as air; `down` may be None at its
/// floor (bedrock: nothing below).
pub trait Cells {
    type Pos: Copy + Eq + Hash;
    fn block(&self, p: Self::Pos) -> Block;
    fn down(&self, p: Self::Pos) -> Option<Self::Pos>;
    fn up(&self, p: Self::Pos) -> Option<Self::Pos>;
    fn sides(&self, p: Self::Pos) -> [Option<Self::Pos>; 4];
}

/// A grid the liquids can change. `put` may refuse (leave the cell as it was) where liquid
/// cannot go: past a station's edge it falls away into space.
pub trait Basin: Cells {
    fn put(&mut self, p: Self::Pos, b: Block);
}

/// How much thinner a liquid gets per cell it spreads.
fn drop(l: Liquid) -> u8 {
    match l {
        Liquid::Water => 1,
        Liquid::Lava => 2,
    }
}

/// How far (cells) a liquid looks for a way down before spreading.
fn search(l: Liquid) -> usize {
    match l {
        Liquid::Water => 4,
        Liquid::Lava => 2,
    }
}

fn ticks(l: Liquid) -> u64 {
    match l {
        Liquid::Water => WATER_TICKS,
        Liquid::Lava => LAVA_TICKS,
    }
}

/// Liquid of kind `l` can pour down into a cell holding `b`.
fn pours_into(l: Liquid, b: Block) -> bool {
    block::washable(b)
        || (block::liquid(b) == Some(l) && !block::is_source(b) && !block::is_falling(b))
        || (l == Liquid::Lava && block::liquid(b) == Some(Liquid::Water))
}

/// Liquid of kind `l` spreading sideways at `level` can go into a cell holding `b`.
fn spreads_into(l: Liquid, b: Block, level: u8) -> bool {
    block::washable(b) || (block::liquid(b) == Some(l) && !block::is_source(b) && !block::is_falling(b) && block::amount(b) < level)
}

/// The cells waiting to update, by the tick they are due.
#[derive(Clone, Debug)]
pub struct Flow<P> {
    now: u64,
    due: BTreeMap<u64, Vec<P>>,
    queued: HashSet<P>,
}

impl<P> Default for Flow<P> {
    fn default() -> Self {
        Self { now: 0, due: BTreeMap::new(), queued: HashSet::new() }
    }
}

impl<P: Copy + Eq + Hash> Flow<P> {
    /// Nothing to do.
    pub fn is_idle(&self) -> bool {
        self.queued.is_empty()
    }

    pub fn pending(&self) -> usize {
        self.queued.len()
    }

    fn schedule(&mut self, p: P, delay: u64) {
        if self.queued.insert(p) {
            self.due.entry(self.now + delay.max(1)).or_default().push(p);
        }
    }

    /// Something changed at `p`: the liquids there and around it get to react.
    pub fn touch<C: Cells<Pos = P>>(&mut self, w: &C, p: P) {
        let around = w.sides(p).into_iter().chain([w.up(p), w.down(p)]).flatten();
        for q in std::iter::once(p).chain(around) {
            if let Some(l) = block::liquid(w.block(q)) {
                let delay = if l == Liquid::Lava && touches_water(w, q) { 2 } else { ticks(l) };
                self.schedule(q, delay);
            }
        }
    }

    /// Runs one tick: updates what is due (at most `budget` cells). Returns how many changed.
    pub fn tick<B: Basin<Pos = P>>(&mut self, w: &mut B, budget: usize) -> usize {
        self.now += 1;
        let mut work = Vec::new();
        while let Some(entry) = self.due.first_entry() {
            if *entry.key() > self.now || work.len() >= budget {
                break;
            }
            work.extend(entry.remove());
        }
        if work.len() > budget {
            let rest = work.split_off(budget);
            self.due.entry(self.now + 1).or_default().extend(rest);
        }
        let mut changed = 0;
        for p in work {
            self.queued.remove(&p);
            changed += self.update(w, p);
        }
        changed
    }

    /// Puts a block and touches around it, if it went in. 1 if it did.
    fn set<B: Basin<Pos = P>>(&mut self, w: &mut B, p: P, b: Block) -> usize {
        if w.block(p) == b {
            return 0;
        }
        w.put(p, b);
        if w.block(p) != b {
            return 0; // it fell away
        }
        self.touch(w, p);
        1
    }

    fn update<B: Basin<Pos = P>>(&mut self, w: &mut B, p: P) -> usize {
        let b = w.block(p);
        let Some(l) = block::liquid(b) else { return 0 };
        // Lava meeting water hardens.
        if l == Liquid::Lava && touches_water(w, p) {
            return self.set(w, p, if block::is_source(b) { block::OBSIDIAN } else { block::COBBLESTONE });
        }
        let mut changed = 0;
        let mut state = b;
        if !block::is_source(b) {
            let new = settle(w, p, l);
            if new != b {
                changed += self.set(w, p, new);
                if new == block::AIR {
                    return changed;
                }
                state = new;
            }
        }
        // Down first, while it can.
        if let Some(d) = w.down(p) {
            let below = w.block(d);
            if pours_into(l, below) {
                let to = if block::liquid(below) == Some(Liquid::Water) && l == Liquid::Lava { block::STONE } else { l.falling() };
                return changed + self.set(w, d, to);
            }
        }
        // Then sideways, toward the nearest way down if there is one close.
        let a = block::amount(state);
        if a <= drop(l) || !spreads_sideways(w, p, state, l) {
            return changed;
        }
        let level = (a - drop(l)).min(7);
        let open: Vec<P> = w.sides(p).into_iter().flatten().filter(|&s| spreads_into(l, w.block(s), level)).collect();
        if open.is_empty() {
            return changed;
        }
        let dist: Vec<usize> = open.iter().map(|&s| hole_distance(w, s, l, p)).collect();
        let best = *dist.iter().min().unwrap();
        for (s, d) in open.into_iter().zip(dist) {
            if d == best {
                changed += self.set(w, s, l.flowing(level));
            }
        }
        changed
    }
}

/// What a liquid cell that is not a source should be now, from what feeds it.
fn settle<C: Cells>(w: &C, p: C::Pos, l: Liquid) -> Block {
    if w.up(p).is_some_and(|u| block::liquid(w.block(u)) == Some(l)) {
        return l.falling();
    }
    let (mut best, mut sources) = (0, 0);
    for s in w.sides(p).into_iter().flatten() {
        let nb = w.block(s);
        if block::liquid(nb) != Some(l) {
            continue;
        }
        if block::is_source(nb) {
            sources += 1;
        }
        if spreads_sideways(w, s, nb, l) {
            best = best.max(block::amount(nb));
        }
    }
    let floor = w.down(p).is_none_or(|d| {
        let b = w.block(d);
        block::is_solid(b) || (block::is_source(b) && block::liquid(b) == Some(l))
    });
    if l == Liquid::Water && sources >= 2 && floor {
        l.source()
    } else if best > drop(l) {
        l.flowing((best - drop(l)).min(7))
    } else {
        block::AIR
    }
}

/// Liquid `b` at `p` spreads sideways: it rests on something (it cannot pour down), and it is a
/// source or does not stand on more of itself (a falling column only spreads where it lands).
fn spreads_sideways<C: Cells>(w: &C, p: C::Pos, b: Block, l: Liquid) -> bool {
    let below = w.down(p).map(|d| w.block(d));
    !below.is_some_and(|d| pours_into(l, d)) && (block::is_source(b) || !below.is_some_and(|d| block::liquid(d) == Some(l)))
}

/// Lava at `p` has water beside or above it.
fn touches_water<C: Cells>(w: &C, p: C::Pos) -> bool {
    w.sides(p).into_iter().chain([w.up(p)]).flatten().any(|q| block::liquid(w.block(q)) == Some(Liquid::Water))
}

/// Cells from `start` (one step from `from`) to the nearest cell the liquid could pour down
/// from, going sideways through cells it could spread into; `usize::MAX` if none is in reach.
fn hole_distance<C: Cells>(w: &C, start: C::Pos, l: Liquid, from: C::Pos) -> usize {
    let can_pour = |q: C::Pos| w.down(q).is_some_and(|d| pours_into(l, w.block(d)));
    let mut seen = HashSet::from([from, start]);
    let mut ring = vec![start];
    for dist in 0..search(l) {
        if ring.iter().any(|&q| can_pour(q)) {
            return dist;
        }
        let mut next = Vec::new();
        for q in ring {
            for s in w.sides(q).into_iter().flatten() {
                if seen.insert(s) && spreads_into(l, w.block(s), 8) {
                    next.push(s);
                }
            }
        }
        ring = next;
    }
    usize::MAX
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::IVec3;
    use std::collections::HashMap;

    /// A flat test grid: stone floor at y = 0 over x, z in 0..16 unless set otherwise.
    #[derive(Default)]
    struct Box {
        cells: HashMap<IVec3, Block>,
    }

    impl Box {
        fn floor() -> Self {
            let mut b = Self::default();
            for x in 0..16 {
                for z in 0..16 {
                    b.cells.insert(IVec3::new(x, 0, z), block::STONE);
                }
            }
            b
        }
    }

    impl Cells for Box {
        type Pos = IVec3;
        fn block(&self, p: IVec3) -> Block {
            *self.cells.get(&p).unwrap_or(&block::AIR)
        }
        fn down(&self, p: IVec3) -> Option<IVec3> {
            (p.y > -8).then(|| p - IVec3::Y)
        }
        fn up(&self, p: IVec3) -> Option<IVec3> {
            Some(p + IVec3::Y)
        }
        fn sides(&self, p: IVec3) -> [Option<IVec3>; 4] {
            [IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z].map(|d| {
                let q = p + d;
                ((0..16).contains(&q.x) && (0..16).contains(&q.z)).then_some(q)
            })
        }
    }

    impl Basin for Box {
        fn put(&mut self, p: IVec3, b: Block) {
            self.cells.insert(p, b);
        }
    }

    fn place(w: &mut Box, f: &mut Flow<IVec3>, p: IVec3, b: Block) {
        w.put(p, b);
        f.touch(w, p);
    }

    fn run(w: &mut Box, f: &mut Flow<IVec3>, ticks: usize) {
        for _ in 0..ticks {
            f.tick(w, BUDGET);
        }
    }

    #[test]
    fn water_spreads_seven_cells_and_settles() {
        let (mut w, mut f) = (Box::floor(), Flow::default());
        place(&mut w, &mut f, IVec3::new(8, 1, 8), block::WATER);
        run(&mut w, &mut f, 200);
        assert!(f.is_idle(), "still flowing");
        assert_eq!(w.block(IVec3::new(9, 1, 8)), Liquid::Water.flowing(7));
        assert_eq!(w.block(IVec3::new(14, 1, 8)), Liquid::Water.flowing(2));
        assert_eq!(w.block(IVec3::new(15, 1, 8)), Liquid::Water.flowing(1));
        // Diagonals take the long way round: 2 + 2 cells.
        assert_eq!(w.block(IVec3::new(10, 1, 10)), Liquid::Water.flowing(4));
        // Taking the source away dries it all up.
        place(&mut w, &mut f, IVec3::new(8, 1, 8), block::AIR);
        run(&mut w, &mut f, 400);
        assert!(w.cells.values().all(|&b| !block::is_liquid(b)), "puddles left");
    }

    #[test]
    fn water_falls_and_spreads_where_it_lands() {
        let (mut w, mut f) = (Box::floor(), Flow::default());
        place(&mut w, &mut f, IVec3::new(8, 5, 8), block::WATER);
        run(&mut w, &mut f, 200);
        for y in 1..5 {
            assert_eq!(w.block(IVec3::new(8, y, 8)), Liquid::Water.falling(), "column at {y}");
        }
        assert!(block::is_liquid(w.block(IVec3::new(12, 1, 8))), "spreads where it lands");
        // Like Minecraft's, a source over its own fall opens into a cross of little falls.
        assert_eq!(w.block(IVec3::new(9, 5, 8)), Liquid::Water.flowing(7));
        assert_eq!(w.block(IVec3::new(10, 5, 8)), block::AIR);
    }

    #[test]
    fn water_heads_for_the_nearest_drop() {
        let (mut w, mut f) = (Box::floor(), Flow::default());
        w.cells.remove(&IVec3::new(10, 0, 8)); // a hole two cells east
        w.put(IVec3::new(10, -1, 8), block::STONE);
        place(&mut w, &mut f, IVec3::new(8, 1, 8), block::WATER);
        run(&mut w, &mut f, 12);
        assert!(block::is_liquid(w.block(IVec3::new(9, 1, 8))));
        assert_eq!(w.block(IVec3::new(7, 1, 8)), block::AIR, "spread away from the hole");
        run(&mut w, &mut f, 20);
        assert!(block::is_liquid(w.block(IVec3::new(10, 0, 8))), "poured into the hole");
    }

    #[test]
    fn two_sources_make_an_endless_pool() {
        let (mut w, mut f) = (Box::floor(), Flow::default());
        // A 3-long trench walled in.
        for x in 6..=10 {
            for z in 7..=9 {
                if !(7..=9).contains(&x) || z != 8 {
                    w.put(IVec3::new(x, 1, z), block::STONE);
                }
            }
        }
        place(&mut w, &mut f, IVec3::new(7, 1, 8), block::WATER);
        place(&mut w, &mut f, IVec3::new(9, 1, 8), block::WATER);
        run(&mut w, &mut f, 40);
        assert_eq!(w.block(IVec3::new(8, 1, 8)), block::WATER);
    }

    #[test]
    fn lava_meets_water() {
        // A lava source with water poured next to it: obsidian.
        let (mut w, mut f) = (Box::floor(), Flow::default());
        for x in 0..16 {
            w.put(IVec3::new(x, 1, 7), block::STONE);
            w.put(IVec3::new(x, 1, 9), block::STONE);
        }
        place(&mut w, &mut f, IVec3::new(4, 1, 8), block::LAVA);
        run(&mut w, &mut f, 100);
        assert_eq!(block::liquid(w.block(IVec3::new(6, 1, 8))), Some(Liquid::Lava), "lava spreads (slowly)");
        place(&mut w, &mut f, IVec3::new(5, 1, 8), block::WATER);
        run(&mut w, &mut f, 400);
        assert_eq!(w.block(IVec3::new(4, 1, 8)), block::OBSIDIAN, "the source hardens to obsidian");
        assert_eq!(w.block(IVec3::new(6, 1, 8)), block::COBBLESTONE, "flowing lava to cobblestone");
        assert!(w.cells.values().all(|&b| block::liquid(b) != Some(Liquid::Lava)), "no lava left");

        // Flowing lava meeting water: cobblestone. Lava falling into water: stone.
        let (mut w, mut f) = (Box::floor(), Flow::default());
        place(&mut w, &mut f, IVec3::new(8, 1, 8), Liquid::Water.source());
        place(&mut w, &mut f, IVec3::new(8, 4, 8), block::LAVA);
        run(&mut w, &mut f, 300);
        assert_eq!(w.block(IVec3::new(8, 1, 8)), block::STONE, "lava poured on water");
    }

    #[test]
    fn liquid_washes_away_plants_and_torches() {
        let (mut w, mut f) = (Box::floor(), Flow::default());
        w.put(IVec3::new(9, 1, 8), block::TORCH);
        w.put(IVec3::new(7, 1, 8), block::POPPY);
        place(&mut w, &mut f, IVec3::new(8, 1, 8), block::WATER);
        run(&mut w, &mut f, 20);
        assert!(block::is_liquid(w.block(IVec3::new(9, 1, 8))) && block::is_liquid(w.block(IVec3::new(7, 1, 8))));
    }
}
