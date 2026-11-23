//! Plant models grown on the cube-sphere. Canopies are built as stacks of round disks: cells
//! whose centers lie within a radius of the trunk, so shapes stay round anywhere on the planet.

use crate::block::{self, Block};
use crate::planet::{Biome, Planet};
use crate::rng::Rng;
use crate::sphere::{CellId, Side};

#[derive(Clone, Copy, Debug)]
pub enum Plant {
    Oak,
    Birch,
    Pine,
    Acacia,
    Cactus,
    CrystalSpire,
    Boulder,
}

/// Which plant (if any) sprouts on a column of this biome.
/// `density` scales how often (1 as made, 0 none).
pub fn pick(biome: Biome, rng: &mut Rng, density: f32) -> Option<Plant> {
    let r = rng.f32();
    if density <= 0.0 {
        return None;
    }
    let r = r / density;
    match biome {
        Biome::Forest if r < 0.028 => Some(if rng.f32() < 0.65 {
            Plant::Oak
        } else {
            Plant::Birch
        }),
        Biome::Meadow if r < 0.004 => Some(match rng.int(0, 9) {
            0..=5 => Plant::Oak,
            6..=7 => Plant::Birch,
            _ => Plant::Acacia,
        }),
        Biome::Frozen if r < 0.010 => Some(Plant::Pine),
        Biome::Desert if r < 0.004 => Some(if rng.f32() < 0.8 {
            Plant::Cactus
        } else {
            Plant::Acacia
        }),
        Biome::Crystal if r < 0.008 => Some(Plant::CrystalSpire),
        Biome::Barren if r < 0.005 => Some(Plant::Boulder),
        _ => None,
    }
}

/// Grass and flowers (or a dead bush) for a column with no tree.
/// `grass` and `flowers` scale how often each grows (1 as made, 0 none).
pub fn pick_small(biome: Biome, rng: &mut Rng, grass: f32, flowers: f32) -> Option<Block> {
    let (rate_f, rate_g) = match biome {
        Biome::Meadow => (0.03, 0.21),
        Biome::Forest => (0.012, 0.13),
        Biome::Desert => (0.0, 0.008),
        _ => (0.0, 0.0),
    };
    let (f, g) = ((rate_f * flowers).min(0.5), (rate_g * grass).min(0.9));
    let r = rng.f32();
    if r < f {
        Some([block::POPPY, block::DANDELION, block::CORNFLOWER][rng.int(0, 2) as usize])
    } else if r < f + g {
        Some(if biome == Biome::Desert {
            block::DEAD_BUSH
        } else {
            block::TALL_GRASS
        })
    } else {
        None
    }
}

fn up(p: &Planet, c: CellId, n: usize) -> Option<CellId> {
    (0..n).try_fold(c, |c, _| p.grid.neighbor(c, Side::Top))
}

/// Cells in `c`'s layer whose centers are within `r` blocks of `c`'s center.
fn disk(p: &Planet, c: CellId, r: f32) -> Vec<CellId> {
    let center = p.grid.center(c);
    let mut out = vec![c];
    let mut i = 0;
    while i < out.len() {
        for s in [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus] {
            if let Some(nb) = p.grid.neighbor(out[i], s) {
                if !out.contains(&nb) && p.grid.center(nb).distance(center) <= r {
                    out.push(nb);
                }
            }
        }
        i += 1;
    }
    out
}

/// A round layer of foliage, ragged at the rim so canopies don't look stamped.
fn canopy_layer(p: &mut Planet, c: CellId, r: f32, leaves: Block, rng: &mut Rng) {
    let center = p.grid.center(c);
    for cell in disk(p, c, r) {
        let edge = p.grid.center(cell).distance(center) > r - 0.9;
        if !edge || rng.f32() < 0.6 {
            p.fill(cell, leaves);
        }
    }
}

/// Builds a trunk of `h` cells from `base`; returns the top trunk cell.
fn trunk(p: &mut Planet, base: CellId, h: usize, wood: Block) -> Option<CellId> {
    let mut c = base;
    for i in 0..h {
        p.fill(c, wood);
        if i + 1 < h {
            c = p.grid.neighbor(c, Side::Top)?;
        }
    }
    Some(c)
}

/// Grows a plant whose lowest cell is `base` (the air cell just above the ground).
pub fn grow(p: &mut Planet, base: CellId, kind: Plant, rng: &mut Rng) {
    let _ = grow_inner(p, base, kind, rng);
}

fn grow_inner(p: &mut Planet, base: CellId, kind: Plant, rng: &mut Rng) -> Option<()> {
    match kind {
        Plant::Oak => {
            let h = rng.int(4, 6) as usize;
            let top = trunk(p, base, h, block::WOOD)?;
            // Wide and round: fullest just below the trunk's top.
            for (dy, r) in [(-2i32, 2.6), (-1, 3.1), (0, 2.9), (1, 2.1), (2, 1.2)] {
                let c = if dy < 0 {
                    trunk_cell(p, top, (-dy) as usize)?
                } else {
                    up(p, top, dy as usize)?
                };
                canopy_layer(p, c, r + rng.range(-0.2, 0.3), block::LEAVES, rng);
            }
        }
        Plant::Birch => {
            let h = rng.int(6, 8) as usize;
            let top = trunk(p, base, h, block::BIRCH_LOG)?;
            // Tall and narrow.
            for (dy, r) in [(-3i32, 1.6), (-2, 2.1), (-1, 2.1), (0, 1.7), (1, 1.0)] {
                let c = if dy < 0 {
                    trunk_cell(p, top, (-dy) as usize)?
                } else {
                    up(p, top, dy as usize)?
                };
                canopy_layer(p, c, r, block::BIRCH_LEAVES, rng);
            }
        }
        Plant::Pine => {
            let h = rng.int(8, 12) as usize;
            let top = trunk(p, base, h, block::WOOD)?;
            // A cone of alternating wide and narrow rings, from a few cells up to above the top.
            let start = 3usize;
            let rings = h + 1 - start;
            let mut c = up(p, base, start)?;
            for i in 0..rings {
                let t = i as f32 / rings as f32;
                let r = (1.0 - t) * 3.3 + 0.6;
                canopy_layer(
                    p,
                    c,
                    if i % 2 == 0 { r } else { r * 0.62 },
                    block::PINE_LEAVES,
                    rng,
                );
                match p.grid.neighbor(c, Side::Top) {
                    Some(n) => c = n,
                    None => return Some(()),
                }
            }
            p.fill(c, block::PINE_LEAVES);
            let _ = top;
        }
        Plant::Acacia => {
            let h = rng.int(4, 6) as usize;
            let top = trunk(p, base, h, block::WOOD)?;
            // Flat, wide umbrella.
            canopy_layer(
                p,
                up(p, top, 1)?,
                rng.range(3.2, 4.0),
                block::ACACIA_LEAVES,
                rng,
            );
            canopy_layer(
                p,
                up(p, top, 2)?,
                rng.range(1.8, 2.4),
                block::ACACIA_LEAVES,
                rng,
            );
        }
        Plant::Cactus => {
            trunk(p, base, rng.int(2, 4) as usize, block::CACTUS)?;
        }
        Plant::CrystalSpire => {
            let h = rng.int(3, 8) as usize;
            let top = trunk(p, base, h, block::CRYSTAL)?;
            // A thicker foot so it reads as a growth, not a pole.
            let _ = top;
            for cell in disk(p, base, 1.2) {
                p.fill(cell, block::CRYSTAL);
            }
        }
        Plant::Boulder => {
            let r = rng.range(1.0, 2.2);
            for cell in disk(p, base, r) {
                p.fill(cell, block::BASALT);
            }
            if r > 1.6 {
                for cell in disk(p, up(p, base, 1)?, r - 0.9) {
                    p.fill(cell, block::STONE);
                }
            }
        }
    }
    Some(())
}

/// The trunk cell `n` below `top`.
fn trunk_cell(p: &Planet, top: CellId, n: usize) -> Option<CellId> {
    (0..n).try_fold(top, |c, _| p.grid.neighbor(c, Side::Bottom))
}
