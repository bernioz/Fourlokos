//! Crafting recipes: a table of data, matched against a 2 × 2 or 3 × 3 grid of what the player
//! laid out. A shaped recipe matches its pattern anywhere in the grid (the grid is trimmed to the
//! smallest box round what is in it first); a shapeless one only asks for its ingredients.

use crate::block::{self, Block};
use crate::items;

/// What a recipe needs in a cell.
pub type Ingredient = Block;

#[derive(Clone, Copy, Debug)]
pub enum Pattern {
    /// `w` × `h` cells, row by row; `block::AIR` is an empty cell.
    Shaped {
        w: usize,
        h: usize,
        cells: &'static [Ingredient],
    },
    Shapeless(&'static [Ingredient]),
}

#[derive(Clone, Copy, Debug)]
pub struct Recipe {
    pub pattern: Pattern,
    pub result: Block,
    pub count: u8,
    /// The smallest grid it can be made in: 2 (the inventory's) or 3 (a crafting table's).
    pub grid: usize,
}

/// Whether `item` serves where a recipe asks for `want` (any log is a log).
fn same(want: Ingredient, item: Block) -> bool {
    want == item || (want == block::WOOD && item == block::BIRCH_LOG)
}

use block::{AIR as A, COBBLESTONE as C, PLANKS as P, WOOD as W};
use items::{COAL, SLIMEBALL as SB, STICK as S};

macro_rules! tool {
    ($m:expr, $pick:expr, $sword:expr) => {
        [
            Recipe {
                pattern: Pattern::Shaped {
                    w: 3,
                    h: 3,
                    cells: &[$m, $m, $m, A, S, A, A, S, A],
                },
                result: $pick,
                count: 1,
                grid: 3,
            },
            Recipe {
                pattern: Pattern::Shaped {
                    w: 1,
                    h: 3,
                    cells: &[$m, $m, S],
                },
                result: $sword,
                count: 1,
                grid: 3,
            },
        ]
    };
}

const WOODEN: [Recipe; 2] = tool!(P, items::WOODEN_PICKAXE, items::WOODEN_SWORD);
const STONE: [Recipe; 2] = tool!(C, items::STONE_PICKAXE, items::STONE_SWORD);
const COPPER: [Recipe; 2] = tool!(
    items::COPPER_INGOT,
    items::COPPER_PICKAXE,
    items::COPPER_SWORD
);
const IRON: [Recipe; 2] = tool!(items::IRON_INGOT, items::IRON_PICKAXE, items::IRON_SWORD);

const BASIC: [Recipe; 9] = [
    Recipe {
        pattern: Pattern::Shapeless(&[W]),
        result: P,
        count: 4,
        grid: 2,
    },
    Recipe {
        pattern: Pattern::Shaped {
            w: 1,
            h: 2,
            cells: &[P, P],
        },
        result: items::STICK,
        count: 4,
        grid: 2,
    },
    Recipe {
        pattern: Pattern::Shaped {
            w: 2,
            h: 2,
            cells: &[P, P, P, P],
        },
        result: block::CRAFTING_TABLE,
        count: 1,
        grid: 2,
    },
    Recipe {
        pattern: Pattern::Shaped {
            w: 1,
            h: 2,
            cells: &[COAL, S],
        },
        result: block::TORCH,
        count: 4,
        grid: 2,
    },
    Recipe {
        pattern: Pattern::Shaped {
            w: 3,
            h: 3,
            cells: &[C, C, C, C, A, C, C, C, C],
        },
        result: block::FURNACE,
        count: 1,
        grid: 3,
    },
    // Eight slimeballs round an iron ingot: the core that calls the Slime King.
    Recipe {
        pattern: Pattern::Shaped {
            w: 3,
            h: 3,
            cells: &[SB, SB, SB, SB, items::IRON_INGOT, SB, SB, SB, SB],
        },
        result: items::SLIME_CROWN_CORE,
        count: 1,
        grid: 3,
    },
    Recipe {
        pattern: Pattern::Shapeless(&[items::EGG, items::OIL]),
        result: items::MAYONNAISE,
        count: 1,
        grid: 2,
    },
    Recipe {
        pattern: Pattern::Shaped {
            w: 3,
            h: 1,
            cells: &[items::WHEAT, items::WHEAT, items::WHEAT],
        },
        result: items::BREAD,
        count: 2,
        grid: 3,
    },
    Recipe {
        pattern: Pattern::Shapeless(&[
            items::BREAD,
            items::VIENESA,
            items::AVOCADO,
            items::TOMATO,
            items::MAYONNAISE,
        ]),
        result: items::COMPLETO_ITALIANO,
        count: 1,
        grid: 3,
    },
];

/// Every recipe there is.
pub fn recipes() -> impl Iterator<Item = &'static Recipe> {
    BASIC
        .iter()
        .chain(WOODEN.iter())
        .chain(STONE.iter())
        .chain(COPPER.iter())
        .chain(IRON.iter())
}

/// What the grid makes: `cells` are `size` × `size` cells row by row (`None`: empty).
pub fn find(cells: &[Option<Block>], size: usize) -> Option<&'static Recipe> {
    debug_assert_eq!(cells.len(), size * size);
    let filled: Vec<(usize, usize, Block)> = cells
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.map(|b| (i % size, i / size, b)))
        .collect();
    if filled.is_empty() {
        return None;
    }
    let (x0, x1) = (
        filled.iter().map(|f| f.0).min()?,
        filled.iter().map(|f| f.0).max()?,
    );
    let (y0, y1) = (
        filled.iter().map(|f| f.1).min()?,
        filled.iter().map(|f| f.1).max()?,
    );
    let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
    recipes()
        .filter(|r| r.grid <= size)
        .find(|r| match r.pattern {
            Pattern::Shaped {
                w: rw,
                h: rh,
                cells: want,
            } => {
                rw == w
                    && rh == h
                    && (0..w * h).all(|i| {
                        let have = cells[(y0 + i / w) * size + x0 + i % w];
                        match (want[i], have) {
                            (A, None) => true,
                            (A, Some(_)) | (_, None) => false,
                            (want, Some(b)) => same(want, b),
                        }
                    })
            }
            Pattern::Shapeless(list) => {
                // Every ingredient used once, and nothing else on the grid.
                let mut left: Vec<Block> = filled.iter().map(|f| f.2).collect();
                list.len() == left.len()
                    && list
                        .iter()
                        .all(|&want| match left.iter().position(|&b| same(want, b)) {
                            Some(k) => {
                                left.swap_remove(k);
                                true
                            }
                            None => false,
                        })
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(size: usize, put: &[(usize, usize, Block)]) -> Vec<Option<Block>> {
        let mut g = vec![None; size * size];
        for &(x, y, b) in put {
            g[y * size + x] = Some(b);
        }
        g
    }

    #[test]
    fn shapeless_and_shaped_match_anywhere_in_the_grid() {
        // One log, anywhere: 4 planks (the 2 × 2 grid already does it).
        for (x, y) in [(0, 0), (1, 1), (1, 0)] {
            let r = find(&grid(2, &[(x, y, block::WOOD)]), 2).unwrap();
            assert_eq!((r.result, r.count), (block::PLANKS, 4));
        }
        assert_eq!(
            find(&grid(2, &[(0, 0, block::BIRCH_LOG)]), 2)
                .unwrap()
                .result,
            block::PLANKS
        );
        // Two planks one over the other (trimmed to their box): sticks; side by side: nothing.
        let r = find(&grid(3, &[(2, 1, block::PLANKS), (2, 2, block::PLANKS)]), 3).unwrap();
        assert_eq!((r.result, r.count), (items::STICK, 4));
        assert!(find(&grid(2, &[(0, 0, block::PLANKS), (1, 0, block::PLANKS)]), 2).is_none());
        // Four planks: a table. Three: nothing.
        let four = grid(
            2,
            &[
                (0, 0, block::PLANKS),
                (1, 0, block::PLANKS),
                (0, 1, block::PLANKS),
                (1, 1, block::PLANKS),
            ],
        );
        assert_eq!(find(&four, 2).unwrap().result, block::CRAFTING_TABLE);
        assert!(
            find(
                &grid(
                    2,
                    &[
                        (0, 0, block::PLANKS),
                        (1, 0, block::PLANKS),
                        (0, 1, block::PLANKS)
                    ]
                ),
                2
            )
            .is_none()
        );
        // Coal over a stick: torches. The other way up: nothing.
        assert_eq!(
            find(&grid(2, &[(1, 0, items::COAL), (1, 1, items::STICK)]), 2)
                .unwrap()
                .result,
            block::TORCH
        );
        assert!(find(&grid(2, &[(1, 0, items::STICK), (1, 1, items::COAL)]), 2).is_none());
        // Extra stuff spoils a shapeless recipe.
        assert!(find(&grid(2, &[(0, 0, block::WOOD), (1, 1, block::DIRT)]), 2).is_none());
        assert!(find(&grid(2, &[]), 2).is_none());
    }

    #[test]
    fn tools_need_the_big_grid_and_the_right_material() {
        let pick = |m: Block| {
            grid(
                3,
                &[
                    (0, 0, m),
                    (1, 0, m),
                    (2, 0, m),
                    (1, 1, items::STICK),
                    (1, 2, items::STICK),
                ],
            )
        };
        for (m, tool) in [
            (block::PLANKS, items::WOODEN_PICKAXE),
            (block::COBBLESTONE, items::STONE_PICKAXE),
            (items::COPPER_INGOT, items::COPPER_PICKAXE),
            (items::IRON_INGOT, items::IRON_PICKAXE),
        ] {
            assert_eq!(find(&pick(m), 3).unwrap().result, tool);
        }
        // The same pattern shifted down a row would not fit the 2 × 2 grid at all (3 tall).
        let sword = grid(
            3,
            &[
                (1, 0, items::IRON_INGOT),
                (1, 1, items::IRON_INGOT),
                (1, 2, items::STICK),
            ],
        );
        assert_eq!(find(&sword, 3).unwrap().result, items::IRON_SWORD);
        let small = grid(2, &[(0, 0, block::PLANKS), (1, 0, block::PLANKS)]);
        assert!(find(&small, 2).is_none());
        // Mixed materials make nothing.
        let mixed = grid(
            3,
            &[
                (0, 0, block::PLANKS),
                (1, 0, block::COBBLESTONE),
                (2, 0, block::PLANKS),
                (1, 1, items::STICK),
                (1, 2, items::STICK),
            ],
        );
        assert!(find(&mixed, 3).is_none());
    }

    #[test]
    fn the_furnace_is_a_ring_of_eight_in_the_big_grid() {
        let ring: Vec<_> = (0..9)
            .filter(|&i| i != 4)
            .map(|i| (i % 3, i / 3, block::COBBLESTONE))
            .collect();
        assert_eq!(find(&grid(3, &ring), 3).unwrap().result, block::FURNACE);
        let full: Vec<_> = (0..9).map(|i| (i % 3, i / 3, block::COBBLESTONE)).collect();
        assert!(find(&grid(3, &full), 3).is_none());
        // Not in the 2 × 2 grid, whatever is put in it.
        assert!(
            recipes()
                .filter(|r| r.grid == 2)
                .all(|r| r.result != block::FURNACE)
        );
    }

    #[test]
    fn the_crown_core_is_eight_slimeballs_round_an_iron_ingot() {
        let mut ring: Vec<_> = (0..9)
            .filter(|&i| i != 4)
            .map(|i| (i % 3, i / 3, items::SLIMEBALL))
            .collect();
        ring.push((1, 1, items::IRON_INGOT));
        let r = find(&grid(3, &ring), 3).unwrap();
        assert_eq!((r.result, r.count), (items::SLIME_CROWN_CORE, 1));
        // Not in the 2 x 2 grid, nor with the ingot missing, nor with a gap in the ring.
        assert!(
            recipes()
                .filter(|r| r.grid == 2)
                .all(|r| r.result != items::SLIME_CROWN_CORE)
        );
        let mut no_ingot = ring.clone();
        no_ingot.pop();
        assert!(find(&grid(3, &no_ingot), 3).is_none());
        let gap: Vec<_> = ring.iter().copied().skip(1).collect();
        assert!(find(&grid(3, &gap), 3).is_none());
    }

    #[test]
    fn every_recipe_fits_the_grid_it_says() {
        for r in recipes() {
            if let Pattern::Shaped { w, h, cells } = r.pattern {
                assert_eq!(cells.len(), w * h);
                assert!(
                    w.max(h) <= r.grid,
                    "{} is {w}x{h} but says grid {}",
                    block::name(r.result),
                    r.grid
                );
            }
            assert!(crate::items::valid(r.result) && r.count > 0);
        }
    }

    #[test]
    fn completo_italiano_needs_the_big_grid_for_five_ingredients() {
        let r = find(
            &grid(
                2,
                &[
                    (0, 0, items::BREAD),
                    (1, 0, items::VIENESA),
                    (0, 1, items::AVOCADO),
                    (1, 1, items::TOMATO),
                ],
            ),
            2,
        );
        assert!(r.is_none(), "the completo needs mayonnaise too");
        let r = find(
            &grid(
                3,
                &[
                    (0, 0, items::BREAD),
                    (1, 0, items::VIENESA),
                    (2, 0, items::AVOCADO),
                    (0, 1, items::TOMATO),
                    (1, 1, items::MAYONNAISE),
                ],
            ),
            3,
        )
        .unwrap();
        assert_eq!((r.result, r.count), (items::COMPLETO_ITALIANO, 1));
        assert_eq!(r.grid, 3);
    }
}
