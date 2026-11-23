//! Light, the way Minecraft does it, in two channels packed in a byte per cell. Sky light: air
//! open to the sky straight above has full light, and light spreads through air losing one level
//! per cell, so caves and overhangs go dark with distance from the sky. Block light: torches (and
//! other glowing blocks) shine their level into the air around them, fading the same way.
//! Opaque cells hold none.

use crate::block;
use crate::planet::Planet;
use crate::sphere::{CellId, Side};
use glam::Vec3;
use std::collections::VecDeque;

pub const MAX: u8 = 15;

/// Brightness of a light level, in (0, 1]: each level dims by a fifth, like Minecraft's curve.
pub fn brightness(level: f32) -> f32 {
    0.8f32.powf(MAX as f32 - level)
}

/// Brightness of a block-light level: 0 is none at all (there is no ambient torchlight).
pub fn glow(level: f32) -> f32 {
    if level <= 0.0 { 0.0 } else { brightness(level) }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Channel {
    Sky,
    Block,
}

impl Channel {
    fn shift(self) -> u8 {
        match self {
            Channel::Sky => 0,
            Channel::Block => 4,
        }
    }
}

impl Planet {
    /// Sky light of a cell.
    pub fn light(&self, c: CellId) -> u8 {
        self.level(c, Channel::Sky)
    }

    /// Block light of a cell.
    pub fn block_light(&self, c: CellId) -> u8 {
        self.level(c, Channel::Block)
    }

    fn level(&self, c: CellId, ch: Channel) -> u8 {
        (self.light[c] >> ch.shift()) & 0xF
    }

    fn put(&mut self, c: CellId, ch: Channel, v: u8) {
        let s = ch.shift();
        self.light[c] = (self.light[c] & !(0xF << s)) | (v << s);
    }

    /// (sky, block) light at a planet-local point: None inside an opaque cell; full sky and no
    /// block light above the build ceiling.
    pub fn light_at(&self, p: Vec3) -> Option<(u8, u8)> {
        if p.length() < self.grid.core {
            return None;
        }
        match self.grid.cell_at(p) {
            Some(c) if block::opaque(self.get(c)) => None,
            Some(c) => Some((self.light(c), self.block_light(c))),
            None => Some((MAX, 0)),
        }
    }

    fn air(&self, c: CellId) -> bool {
        !block::opaque(self.get(c))
    }

    /// Open to straight-down sunlight: water and lava stop it (light only seeps into them from
    /// their surface, fading a level a cell, so the deep sea floor is dim).
    fn sky_through(&self, c: CellId) -> bool {
        self.air(c) && !block::is_liquid(self.get(c))
    }

    /// Lights every cell from scratch.
    pub(crate) fn compute_light(&mut self) {
        let layers = self.grid.layers;
        let columns = self.grid.cell_count() / layers;
        self.light = vec![0; self.grid.cell_count()];
        // Lowest layer of each column that sees the sky.
        let mut floor = vec![layers; columns];
        for (col, f) in floor.iter_mut().enumerate() {
            for c in (col * layers..(col + 1) * layers).rev() {
                if !self.sky_through(c) {
                    break;
                }
                self.light[c] = MAX;
                *f = c - col * layers;
            }
        }
        // Spread from sky cells beside a column whose sky stops higher up, and down into the
        // liquid a column's sky stops on.
        let mut queue = VecDeque::new();
        for (col, &f) in floor.iter().enumerate() {
            if f > 0 && f < layers && block::is_liquid(self.get(col * layers + f - 1)) {
                queue.push_back(col * layers + f);
            }
        }
        for col in 0..columns {
            for s in [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus] {
                let Some(nb) = self.grid.neighbor(col * layers, s) else { continue };
                let nb_col = nb / layers;
                for k in floor[col]..floor[nb_col] {
                    if self.air(nb_col * layers + k) {
                        queue.push_back(col * layers + k);
                    }
                }
            }
        }
        self.spread(Channel::Sky, queue, &mut Vec::new());
        // Every glowing block shines.
        let mut glowing = VecDeque::new();
        for c in 0..self.grid.cell_count() {
            let e = block::emission(self.get(c));
            if e > 0 {
                self.put(c, Channel::Block, e);
                glowing.push_back(c);
            }
        }
        self.spread(Channel::Block, glowing, &mut Vec::new());
    }

    /// Spreads light outward from the queued cells; records every cell it raises.
    fn spread(&mut self, ch: Channel, mut queue: VecDeque<CellId>, touched: &mut Vec<CellId>) {
        while let Some(c) = queue.pop_front() {
            let l = self.level(c, ch);
            if l <= 1 {
                continue;
            }
            for s in Side::ALL {
                if let Some(nb) = self.grid.neighbor(c, s) {
                    if self.air(nb) && self.level(nb, ch) < l - 1 {
                        self.put(nb, ch, l - 1);
                        touched.push(nb);
                        queue.push_back(nb);
                    }
                }
            }
        }
    }

    /// Takes out the light of `cells` and whatever it alone lit; returns the lit cells around the
    /// dark that must shine back in. Glowing blocks keep their own light.
    fn darken(&mut self, ch: Channel, cells: &[CellId], touched: &mut Vec<CellId>) -> VecDeque<CellId> {
        let mut dark = VecDeque::new();
        let mut bright = VecDeque::new();
        for &x in cells {
            let l = self.level(x, ch);
            if l > 0 {
                dark.push_back((x, l));
                self.put(x, ch, 0);
                touched.push(x);
            }
        }
        while let Some((x, old)) = dark.pop_front() {
            for s in Side::ALL {
                let Some(nb) = self.grid.neighbor(x, s) else { continue };
                let l = self.level(nb, ch);
                if l == 0 {
                    continue;
                }
                if l < old {
                    self.put(nb, ch, 0);
                    touched.push(nb);
                    dark.push_back((nb, l));
                } else {
                    bright.push_back(nb);
                }
            }
        }
        // Lit neighbors shine into what was opened (or re-fill what was darkened).
        for &x in cells {
            bright.extend(Side::ALL.iter().filter_map(|&s| self.grid.neighbor(x, s)).filter(|&nb| self.level(nb, ch) > 0));
        }
        if ch == Channel::Block {
            for &x in touched.iter() {
                let e = block::emission(self.get(x));
                if e > 0 {
                    self.put(x, ch, e);
                    bright.push_back(x);
                }
            }
        }
        bright
    }

    /// Updates the light after cell `c` changed; returns the cells whose light may have changed.
    pub(crate) fn relight(&mut self, c: CellId) -> Vec<CellId> {
        let layers = self.grid.layers;
        let (col, k) = (c / layers * layers, c % layers);
        let mut touched = Vec::new();

        // Sky: the changed cell and the air run under it, whose view of the sky may have changed.
        let mut run = vec![c];
        for below in (col..c).rev() {
            if !self.air(below) {
                break;
            }
            run.push(below);
        }
        let open_above = (k + 1..layers).all(|kk| self.sky_through(col + kk));
        let mut bright = self.darken(Channel::Sky, &run, &mut touched);
        // New sky cells, then spread everything back in.
        if open_above {
            for &x in &run {
                if self.sky_through(x) {
                    self.put(x, Channel::Sky, MAX);
                    bright.push_back(x);
                } else {
                    break;
                }
            }
        }
        self.spread(Channel::Sky, bright, &mut touched);

        // Block light: out with the old, in with the cell's own glow and its lit neighbors'.
        let mut lit = self.darken(Channel::Block, &[c], &mut touched);
        let e = block::emission(self.get(c));
        if e > 0 {
            self.put(c, Channel::Block, e);
            lit.push_back(c);
            touched.push(c);
        }
        self.spread(Channel::Block, lit, &mut touched);
        touched
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::{Biome, PlanetSpec};
    use crate::rng::Rng;

    fn flat(ground: usize) -> Planet {
        let spec = PlanetSpec::new(9, "l".into(), 24.0, Biome::Barren);
        Planet::flat(spec, ground)
    }

    #[test]
    fn sky_is_full_and_sealed_caves_are_dark() {
        let mut p = flat(12);
        let g = p.grid.clone();
        assert_eq!(p.light(g.index(0, 5, 5, 12)), MAX);
        assert_eq!(p.light(g.index(0, 5, 5, 11)), 0); // stone
        // A sealed pocket two cells under the ground.
        for i in 5..8 {
            p.set(g.index(0, i, 5, 9), block::AIR);
        }
        assert_eq!(p.light(g.index(0, 6, 5, 9)), 0);
    }

    #[test]
    fn light_fades_into_a_tunnel() {
        let mut p = flat(12);
        let g = p.grid.clone();
        // A shaft down to layer 8, then a tunnel off its bottom.
        for k in 8..12 {
            p.set(g.index(0, 5, 5, k), block::AIR);
        }
        for i in 6..12 {
            p.set(g.index(0, i, 5, 8), block::AIR);
        }
        assert_eq!(p.light(g.index(0, 5, 5, 8)), MAX, "the shaft sees the sky");
        for i in 6..12 {
            assert_eq!(p.light(g.index(0, i, 5, 8)), MAX - (i - 5) as u8);
        }
        // Capping the shaft seals the cave: it goes black. Reopening it brings the light back.
        p.set(g.index(0, 5, 5, 11), block::STONE);
        assert_eq!(p.light(g.index(0, 5, 5, 8)), 0);
        assert_eq!(p.light(g.index(0, 11, 5, 8)), 0);
        p.set(g.index(0, 5, 5, 11), block::AIR);
        assert_eq!(p.light(g.index(0, 11, 5, 8)), MAX - 6);
    }

    #[test]
    fn edits_match_a_full_relight() {
        let mut p = flat(12);
        let n = p.grid.n;
        let mut rng = Rng::new(5);
        for step in 0..400 {
            let (f, i, j, k) = (rng.int(0, 5) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(6, 14) as usize);
            let b = if step % 3 == 0 { block::STONE } else { block::AIR };
            let c = p.grid.index(f, i, j, k);
            p.set(c, b);
        }
        let incremental = p.light.clone();
        p.compute_light();
        assert!(incremental == p.light, "{} cells differ", incremental.iter().zip(&p.light).filter(|(a, b)| a != b).count());
    }

    #[test]
    fn water_dims_with_depth_and_edits_match_a_full_relight() {
        let mut p = flat(12);
        let g = p.grid.clone();
        // A pool four deep.
        for k in 8..12 {
            p.set(g.index(0, 5, 5, k), block::WATER);
        }
        assert_eq!(p.light(g.index(0, 5, 5, 11)), MAX - 1);
        assert_eq!(p.light(g.index(0, 5, 5, 8)), MAX - 4);
        let n = p.grid.n;
        let mut rng = Rng::new(3);
        for step in 0..500 {
            let (f, i, j, k) = (rng.int(0, 5) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(6, 14) as usize);
            let b = match step % 4 {
                0 => block::STONE,
                1 => block::WATER,
                2 => block::LAVA,
                _ => block::AIR,
            };
            p.set(p.grid.index(f, i, j, k), b);
        }
        let incremental = p.light.clone();
        p.compute_light();
        assert!(incremental == p.light, "{} cells differ", incremental.iter().zip(&p.light).filter(|(a, b)| a != b).count());
    }

    #[test]
    fn torches_light_caves_and_edits_match_a_full_relight() {
        let mut p = flat(12);
        let g = p.grid.clone();
        // A sealed tunnel two cells down, dark until a torch goes in.
        for i in 2..14 {
            p.set(g.index(0, i, 5, 9), block::AIR);
        }
        assert_eq!(p.block_light(g.index(0, 8, 5, 9)), 0);
        p.set(g.index(0, 2, 5, 9), block::TORCH);
        let e = block::emission(block::TORCH);
        assert_eq!(p.block_light(g.index(0, 2, 5, 9)), e);
        assert_eq!(p.block_light(g.index(0, 6, 5, 9)), e - 4);
        assert_eq!(p.light(g.index(0, 6, 5, 9)), 0, "still no sky down there");
        // Random digging, building and torches agree with lighting it all again.
        let n = p.grid.n;
        let mut rng = Rng::new(8);
        for step in 0..500 {
            let (f, i, j, k) = (rng.int(0, 5) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(0, n as i64 - 1) as usize, rng.int(6, 14) as usize);
            let b = match step % 5 {
                0 => block::STONE,
                1 => block::TORCH,
                _ => block::AIR,
            };
            p.set(p.grid.index(f, i, j, k), b);
        }
        let incremental = p.light.clone();
        p.compute_light();
        assert!(incremental == p.light, "{} cells differ", incremental.iter().zip(&p.light).filter(|(a, b)| a != b).count());
        // Taking the torch away darkens the tunnel again.
        let mut q = flat(12);
        for i in 2..14 {
            q.set(g.index(0, i, 5, 9), block::AIR);
        }
        q.set(g.index(0, 2, 5, 9), block::TORCH);
        q.set(g.index(0, 2, 5, 9), block::AIR);
        assert!((2..14).all(|i| q.block_light(g.index(0, i, 5, 9)) == 0));
    }
}

