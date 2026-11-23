//! Blocks smaller than their cell, and what holds blocks up, the same on any grid (a planet's
//! cube sphere, a station's box; see `Around`).
//!
//! Shapes are boxes in a cell's own fractions (u, v across it along its two horizontal axes, w
//! up it, each 0..1): doors and trapdoors are boards 3/16 thick, ladders a board on the wall,
//! fences a post with rails to what they join, panes a thin post with glass to what they join.
//! The same boxes stop the player (`collision`), fences a block and a half high.
//!
//! Support, as in Minecraft: grass and flowers stand on grass or dirt, dead bushes on sand,
//! cactus on sand or cactus, torches on a solid block or against a solid wall, ladders against a
//! solid wall, doors on a solid block with their other half over them. A block whose support
//! goes, goes too (`unsupported`).

use crate::block::{self, Block, Dir, Shape};

/// A box in a cell's fractions: [u0, v0, w0, u1, v1, w1].
pub type Cuboid = [f32; 6];

/// Up to ten boxes, without allocating.
#[derive(Clone, Copy, Debug, Default)]
pub struct Boxes {
    n: usize,
    b: [Cuboid; 10],
}

impl Boxes {
    pub fn full() -> Self {
        let mut b = Self::default();
        b.push([0.0, 0.0, 0.0, 1.0, 1.0, 1.0]);
        b
    }
    fn push(&mut self, c: Cuboid) {
        if self.n < self.b.len() {
            self.b[self.n] = c;
            self.n += 1;
        }
    }
    pub fn is_empty(&self) -> bool {
        self.n == 0
    }
    pub fn iter(&self) -> impl Iterator<Item = Cuboid> + '_ {
        self.b[..self.n].iter().copied()
    }
}

/// A grid of cells, seen from one cell: what is there, and the cells beside, over and under it.
pub trait Around {
    type Pos: Copy + Eq;
    fn at(&self, p: Self::Pos) -> Block;
    fn side(&self, p: Self::Pos, d: Dir) -> Option<Self::Pos>;
    fn above(&self, p: Self::Pos) -> Option<Self::Pos>;
    fn below(&self, p: Self::Pos) -> Option<Self::Pos>;
}

fn get<A: Around>(g: &A, p: Option<A::Pos>) -> Block {
    p.map_or(block::AIR, |p| g.at(p))
}

/// A board `t` thick along side `d`, from height w0 to w1.
fn board(d: Dir, t: f32, w0: f32, w1: f32) -> Cuboid {
    match d.0 {
        0 => [0.0, 0.0, w0, t, 1.0, w1],
        1 => [1.0 - t, 0.0, w0, 1.0, 1.0, w1],
        2 => [0.0, 0.0, w0, 1.0, t, w1],
        _ => [0.0, 1.0 - t, w0, 1.0, 1.0, w1],
    }
}

/// An arm from a post (half width `r` about the middle) out to side `d`, `lo..hi` across it.
fn arm(d: Dir, r: f32, lo: f32, hi: f32, w0: f32, w1: f32) -> Cuboid {
    let (near, far) = (0.5 - r, 0.5 + r);
    match d.0 {
        0 => [0.0, lo, w0, near, hi, w1],
        1 => [far, lo, w0, 1.0, hi, w1],
        2 => [lo, 0.0, w0, hi, near, w1],
        _ => [lo, far, w0, hi, 1.0, w1],
    }
}

const BOARD: f32 = 3.0 / 16.0;
/// A fence stops you this high: over the 1.5 blocks a jump reaches here.
pub const FENCE_HEIGHT: f32 = 1.625;

/// Joins what is beside it: a fence or a pane.
pub fn connects(b: Block) -> bool {
    b == block::FENCE || b == block::GLASS_PANE
}

/// Whether a fence or pane `b` joins a neighbor holding `nb`: fences join fences, panes join
/// panes and glass, and both join solid blocks.
pub fn joins(b: Block, nb: Block) -> bool {
    match b {
        block::FENCE => nb == block::FENCE || block::is_solid(nb),
        block::GLASS_PANE => nb == block::GLASS_PANE || block::is_solid(nb),
        _ => false,
    }
}

/// Which sides a fence or pane at `p` joins.
pub fn joined<A: Around>(g: &A, p: A::Pos) -> [bool; 4] {
    let b = g.at(p);
    if !connects(b) {
        return [false; 4];
    }
    Dir::ALL.map(|d| joins(b, get(g, g.side(p, d))))
}

/// The boxes a block is drawn as (`join`: the sides a fence or pane joins). Empty for blocks
/// that are not `Shape::Boxes`.
pub fn boxes(b: Block, join: [bool; 4]) -> Boxes {
    let mut out = Boxes::default();
    match b {
        block::FENCE => {
            let r = 2.0 / 16.0;
            out.push([0.5 - r, 0.5 - r, 0.0, 0.5 + r, 0.5 + r, 1.0]);
            for d in Dir::ALL.into_iter().filter(|d| join[d.0 as usize]) {
                for (w0, w1) in [(6.0 / 16.0, 9.0 / 16.0), (12.0 / 16.0, 15.0 / 16.0)] {
                    out.push(arm(d, r, 7.0 / 16.0, 9.0 / 16.0, w0, w1));
                }
            }
        }
        block::GLASS_PANE => {
            let r = 1.0 / 16.0;
            out.push([0.5 - r, 0.5 - r, 0.0, 0.5 + r, 0.5 + r, 1.0]);
            for d in Dir::ALL.into_iter().filter(|d| join[d.0 as usize]) {
                out.push(arm(d, r, 0.5 - r, 0.5 + r, 0.0, 1.0));
            }
        }
        _ if block::is_ladder(b) => out.push(board(block::facing(b).unwrap(), 1.0 / 16.0, 0.0, 1.0)),
        _ => return collision(b, join),
    }
    out
}

/// The boxes a block stops the player with: a full cell for whole blocks, none for plants,
/// torches and liquids.
pub fn collision(b: Block, join: [bool; 4]) -> Boxes {
    let mut out = Boxes::default();
    match block::shape(b) {
        Shape::Cube => return Boxes::full(),
        Shape::Boxes => {}
        _ => return out,
    }
    let f = block::facing(b).unwrap_or(Dir(0));
    match b {
        block::FENCE => {
            let r = 2.0 / 16.0;
            out.push([0.5 - r, 0.5 - r, 0.0, 0.5 + r, 0.5 + r, FENCE_HEIGHT]);
            for d in Dir::ALL.into_iter().filter(|d| join[d.0 as usize]) {
                out.push(arm(d, r, 0.5 - r, 0.5 + r, 0.0, FENCE_HEIGHT));
            }
        }
        block::GLASS_PANE => return boxes(b, join),
        _ if block::is_ladder(b) => out.push(board(f, BOARD, 0.0, 1.0)),
        _ if block::is_door(b) => out.push(board(if block::is_open(b) { f.turned() } else { f }, BOARD, 0.0, 1.0)),
        _ if block::is_trapdoor(b) => {
            if block::is_open(b) {
                out.push(board(f, BOARD, 0.0, 1.0));
            } else if block::is_top(b) {
                out.push([0.0, 0.0, 1.0 - BOARD, 1.0, 1.0, 1.0]);
            } else {
                out.push([0.0, 0.0, 0.0, 1.0, 1.0, BOARD]);
            }
        }
        _ => {}
    }
    out
}

/// Where a point of a torch (fractions, as it stands on the ground) goes: a torch on a wall
/// stands higher, its foot against the wall, leaning out from it.
pub fn torch_point(b: Block, u: f32, v: f32, w: f32) -> (f32, f32, f32) {
    let Some(d) = block::facing(b) else { return (u, v, w) };
    let (axis, sign) = d.axis();
    let shift = sign * (0.42 - 0.35 * w);
    let (u, v) = if axis == 0 { (u + shift, v) } else { (u, v + shift) };
    (u, v, w + 0.22)
}

/// The boxes a block is outlined by when looked at, and that a look (a click) must go into to
/// reach it: the whole cell for whole blocks, the block's own shape for the rest (a door's board,
/// a torch's stick, a flower's tuft), so you can reach past them.
pub fn outline(b: Block, join: [bool; 4]) -> Boxes {
    let mut out = Boxes::default();
    let px = |n: f32| n / 16.0;
    match block::shape(b) {
        Shape::Cube => return Boxes::full(),
        Shape::Empty | Shape::Liquid => {}
        Shape::Cross => out.push(match b {
            block::TALL_GRASS | block::DEAD_BUSH => [px(2.0), px(2.0), 0.0, px(14.0), px(14.0), px(13.0)],
            _ => [px(5.0), px(5.0), 0.0, px(11.0), px(11.0), px(10.0)],
        }),
        Shape::Torch => {
            // The stick's box, leaned as it is drawn.
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for (u, v, w) in [(px(6.0), px(6.0), 0.0), (px(10.0), px(10.0), 0.0), (px(6.0), px(6.0), px(10.0)), (px(10.0), px(10.0), px(10.0))] {
                let (u, v, w) = torch_point(b, u, v, w);
                for (k, x) in [u, v, w].into_iter().enumerate() {
                    lo[k] = lo[k].min(x);
                    hi[k] = hi[k].max(x);
                }
            }
            out.push([lo[0], lo[1], lo[2], hi[0], hi[1], hi[2]]);
        }
        Shape::Boxes if connects(b) => return boxes(b, join),
        Shape::Boxes => return collision(b, join),
    }
    out
}

/// True if a point (fractions of its cell) is in a block's outline (see `outline`).
pub fn in_outline(b: Block, join: [bool; 4], u: f32, v: f32, w: f32) -> bool {
    block::shape(b) == Shape::Cube || outline(b, join).iter().any(|c| u >= c[0] && u <= c[3] && v >= c[1] && v <= c[4] && w >= c[2] && w <= c[5])
}

/// Where a ray (from `o` along `d`, in a cell's fractions) first goes into a box, if it does.
pub fn ray_box(o: [f32; 3], d: [f32; 3], c: Cuboid) -> Option<f32> {
    let (mut t0, mut t1) = (0.0f32, f32::MAX);
    for k in 0..3 {
        let (lo, hi) = (c[k], c[k + 3]);
        if d[k].abs() < 1e-9 {
            if o[k] < lo || o[k] > hi {
                return None;
            }
            continue;
        }
        let (a, b) = ((lo - o[k]) / d[k], (hi - o[k]) / d[k]);
        t0 = t0.max(a.min(b));
        t1 = t1.min(a.max(b));
    }
    (t0 <= t1).then_some(t0)
}

/// True if some collision box of `b` reaches `w` (fractions) at this point of its cell.
pub fn holds(b: Block, join: [bool; 4], u: f32, v: f32, w: f32) -> bool {
    collision(b, join).iter().any(|c| u >= c[0] && u <= c[3] && v >= c[1] && v <= c[4] && w >= c[2] && w <= c[5])
}

/// Whether `b` at `p` has what it stands on (or hangs from).
pub fn supported<A: Around>(g: &A, p: A::Pos, b: Block) -> bool {
    let below = get(g, g.below(p));
    let wall = |d: Dir| block::is_solid(get(g, g.side(p, d)));
    match b {
        block::TALL_GRASS | block::POPPY | block::DANDELION | block::CORNFLOWER => matches!(below, block::GRASS | block::DIRT),
        block::DEAD_BUSH => matches!(below, block::SAND | block::DIRT | block::GRASS),
        block::CACTUS => matches!(below, block::SAND | block::CACTUS),
        block::TORCH => block::is_solid(below) || below == block::FENCE,
        _ if block::is_wall_torch(b) || block::is_ladder(b) => wall(block::facing(b).unwrap()),
        _ if block::is_door(b) => {
            if block::is_upper(b) {
                block::is_door(below) && !block::is_upper(below)
            } else {
                let above = get(g, g.above(p));
                block::is_solid(below) && block::is_door(above) && block::is_upper(above)
            }
        }
        _ => true,
    }
}

/// The cells next to `p` (over, under and beside it) whose blocks lost their support.
pub fn unsupported<A: Around>(g: &A, p: A::Pos) -> Vec<A::Pos> {
    let mut around = vec![g.above(p), g.below(p)];
    around.extend(Dir::ALL.map(|d| g.side(p, d)));
    let mut out = Vec::new();
    for q in around.into_iter().flatten() {
        let b = g.at(q);
        if b != block::AIR && !block::is_liquid(b) && !supported(g, q, b) && !out.contains(&q) {
            out.push(q);
        }
    }
    out
}

/// Where the block clicked lies from the cell a block goes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Against {
    /// Placing on top of it.
    Below,
    /// Under it (on a ceiling).
    Above,
    Side(Dir),
}

/// What placing `item` at `p` puts where: the blocks (more than one for a door), or None if it
/// cannot go there. `against`: the block clicked; `look`: the side the player looks toward;
/// `high`: the click was on the upper half of a side (a trapdoor then goes at the top).
pub fn place<A: Around>(g: &A, p: A::Pos, item: Block, against: Against, look: Dir, high: bool) -> Option<Vec<(A::Pos, Block)>> {
    let free = |q: Option<A::Pos>| q.is_some_and(|q| block::washable(g.at(q)) || block::is_liquid(g.at(q)));
    if !free(Some(p)) {
        return None;
    }
    let b = match item {
        block::TORCH => match against {
            Against::Below => block::TORCH,
            Against::Side(d) => block::wall_torch(d),
            Against::Above => return None,
        },
        block::LADDER => match against {
            Against::Side(d) => block::ladder(d),
            _ => block::ladder(look),
        },
        block::DOOR => {
            let up = g.above(p)?;
            if !free(Some(up)) {
                return None;
            }
            // Its panel on the side nearest the player, as Minecraft's.
            let f = look.opposite();
            let (lower, upper) = (block::door(f, false, false), block::door(f, false, true));
            if !block::is_solid(get(g, g.below(p))) {
                return None;
            }
            return Some(vec![(p, lower), (up, upper)]);
        }
        block::TRAPDOOR => match against {
            Against::Side(d) => block::trapdoor(d, false, high),
            Against::Below => block::trapdoor(look, false, false),
            Against::Above => block::trapdoor(look, false, true),
        },
        b => b,
    };
    supported(g, p, b).then(|| vec![(p, b)])
}

/// The other half of a door at `p` (holding `b`), if it is there.
pub fn door_pair<A: Around>(g: &A, p: A::Pos, b: Block) -> Option<A::Pos> {
    if !block::is_door(b) {
        return None;
    }
    let q = if block::is_upper(b) { g.below(p) } else { g.above(p) }?;
    let o = g.at(q);
    (block::is_door(o) && block::is_upper(o) != block::is_upper(b)).then_some(q)
}

/// Opening or closing what is at `p`: the cells and what they become (both halves of a door).
pub fn toggle<A: Around>(g: &A, p: A::Pos) -> Vec<(A::Pos, Block)> {
    let b = g.at(p);
    if !block::opens(b) {
        return vec![];
    }
    let mut out = vec![(p, block::toggled(b))];
    if let Some(q) = door_pair(g, p, b) {
        out.push((q, block::toggled(g.at(q))));
    }
    out
}

/// Grass on `p` (holding `b`) next random tick: dirt under the open sky near grass greens over
/// (`lit`: the light over it is 9 or more), grass under something opaque goes back to dirt.
/// `grass_near`: there is grass within a block sideways and from one under to three over.
pub fn grass_tick(b: Block, over: Block, lit: bool, grass_near: bool) -> Option<Block> {
    let covered = block::opaque(over) || block::is_liquid(over);
    match b {
        block::GRASS if covered => Some(block::DIRT),
        block::DIRT if !covered && lit && grass_near => Some(block::GRASS),
        _ => None,
    }
}

/// Whether there is grass that can spread to dirt at `p`: within one cell sideways (and
/// diagonally), from one layer under it to three over it.
pub fn grass_near<A: Around>(g: &A, p: A::Pos) -> bool {
    let mut columns = vec![p];
    for d in Dir::ALL {
        if let Some(q) = g.side(p, d) {
            columns.push(q);
            if let Some(r) = g.side(q, d.turned()) {
                columns.push(r);
            }
        }
    }
    columns.into_iter().any(|c| {
        let mut cells = vec![c];
        let mut up = c;
        for _ in 0..3 {
            match g.above(up) {
                Some(q) => {
                    cells.push(q);
                    up = q;
                }
                None => break,
            }
        }
        cells.extend(g.below(c));
        cells.into_iter().any(|q| g.at(q) == block::GRASS)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A tiny flat world for the rules.
    #[derive(Default)]
    struct Flat(HashMap<(i32, i32, i32), Block>);

    impl Around for Flat {
        type Pos = (i32, i32, i32);
        fn at(&self, p: Self::Pos) -> Block {
            *self.0.get(&p).unwrap_or(&block::AIR)
        }
        fn side(&self, p: Self::Pos, d: Dir) -> Option<Self::Pos> {
            let (a, s) = d.axis();
            let s = s as i32;
            Some(if a == 0 { (p.0 + s, p.1, p.2) } else { (p.0, p.1, p.2 + s) })
        }
        fn above(&self, p: Self::Pos) -> Option<Self::Pos> {
            Some((p.0, p.1 + 1, p.2))
        }
        fn below(&self, p: Self::Pos) -> Option<Self::Pos> {
            Some((p.0, p.1 - 1, p.2))
        }
    }

    #[test]
    fn ids_round_trip() {
        for d in Dir::ALL {
            for open in [false, true] {
                for half in [false, true] {
                    let b = block::door(d, open, half);
                    assert!(block::is_door(b) && block::facing(b) == Some(d) && block::is_open(b) == open && block::is_upper(b) == half);
                    assert_eq!(block::item(b), block::DOOR);
                    let t = block::trapdoor(d, open, half);
                    assert!(block::is_trapdoor(t) && block::facing(t) == Some(d) && block::is_open(t) == open && block::is_top(t) == half);
                    assert_eq!(block::toggled(block::toggled(t)), t);
                }
            }
            assert_eq!(block::facing(block::ladder(d)), Some(d));
            assert_eq!(block::item(block::wall_torch(d)), block::TORCH);
            assert_eq!(d.turned().turned(), d.opposite());
        }
        assert_eq!(block::TRAPDOOR as usize + 16, block::COUNT);
    }

    #[test]
    fn flowers_need_grass_and_doors_two_cells() {
        let mut w = Flat::default();
        w.0.insert((0, 0, 0), block::GRASS);
        w.0.insert((1, 0, 0), block::STONE);
        assert!(place(&w, (0, 1, 0), block::POPPY, Against::Below, Dir(0), false).is_some());
        assert!(place(&w, (1, 1, 0), block::POPPY, Against::Below, Dir(0), false).is_none(), "a flower on stone");
        assert!(place(&w, (5, 5, 5), block::POPPY, Against::Below, Dir(0), false).is_none(), "a flower in the air");
        let door = place(&w, (1, 1, 0), block::DOOR, Against::Below, Dir(1), false).unwrap();
        assert_eq!(door.len(), 2);
        for (p, b) in door {
            w.0.insert(p, b);
        }
        assert!(supported(&w, (1, 1, 0), w.at((1, 1, 0))) && supported(&w, (1, 2, 0), w.at((1, 2, 0))));
        // Opening it opens both halves.
        let t = toggle(&w, (1, 2, 0));
        assert_eq!(t.len(), 2);
        assert!(t.iter().all(|&(_, b)| block::is_open(b)));
        // Its ground goes: both halves lose their support, the lower first.
        w.0.remove(&(1, 0, 0));
        assert_eq!(unsupported(&w, (1, 0, 0)), vec![(1, 1, 0)]);
    }

    #[test]
    fn torches_on_walls_and_fences_join() {
        let mut w = Flat::default();
        w.0.insert((0, 0, 0), block::STONE);
        assert_eq!(place(&w, (1, 0, 0), block::TORCH, Against::Side(Dir(0)), Dir(0), false), Some(vec![((1, 0, 0), block::wall_torch(Dir(0)))]));
        assert!(place(&w, (0, -1, 0), block::TORCH, Against::Above, Dir(0), false).is_none());
        w.0.insert((1, 0, 0), block::FENCE);
        w.0.insert((2, 0, 0), block::FENCE);
        assert_eq!(joined(&w, (1, 0, 0)), [true, true, false, false]);
        let c = collision(block::FENCE, [false; 4]);
        assert!(c.iter().all(|b| b[5] == FENCE_HEIGHT));
        // A closed door is a board on its side; open, on the next side round.
        let shut = collision(block::door(Dir(0), false, false), [false; 4]).iter().next().unwrap();
        let open = collision(block::door(Dir(0), true, false), [false; 4]).iter().next().unwrap();
        assert!(shut[3] < 0.2 && open[4] < 0.2);
    }

    #[test]
    fn grass_spreads_and_dies() {
        let mut w = Flat::default();
        w.0.insert((0, 0, 0), block::GRASS);
        w.0.insert((1, 0, 0), block::DIRT);
        assert!(grass_near(&w, (1, 0, 0)));
        assert!(!grass_near(&w, (5, 0, 0)));
        assert_eq!(grass_tick(block::DIRT, block::AIR, true, true), Some(block::GRASS));
        assert_eq!(grass_tick(block::DIRT, block::AIR, false, true), None);
        assert_eq!(grass_tick(block::GRASS, block::STONE, true, true), Some(block::DIRT));
        assert_eq!(grass_tick(block::GRASS, block::GLASS, true, true), None);
    }
}
