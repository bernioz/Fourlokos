//! Cultivos: datos puros de etapas y cosechas. El cliente avanza una etapa en cada random tick.

use crate::{block, items};

pub const GROWTH_CHANCE: u64 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crop {
    pub stages: &'static [block::Block],
    pub seed: block::Block,
    pub harvest: block::Block,
}

pub const TOMATO: Crop = Crop {
    stages: &[
        block::TOMATO_0,
        block::TOMATO_1,
        block::TOMATO_2,
        block::TOMATO_3,
    ],
    seed: items::TOMATO_SEEDS,
    harvest: items::TOMATO,
};
pub const SUNFLOWER: Crop = Crop {
    stages: &[
        block::SUNFLOWER_0,
        block::SUNFLOWER_1,
        block::SUNFLOWER_2,
        block::SUNFLOWER_3,
    ],
    seed: items::SUNFLOWER_SEEDS,
    harvest: items::SUNFLOWER_SEEDS,
};
pub const WHEAT: Crop = Crop {
    stages: &[
        block::WHEAT_0,
        block::WHEAT_1,
        block::WHEAT_2,
        block::WHEAT_3,
    ],
    seed: items::WHEAT_SEEDS,
    harvest: items::WHEAT,
};
pub const PALTO: Crop = Crop {
    stages: &[block::PALTO_0, block::PALTO_1],
    seed: items::PALTO_SAPLING,
    harvest: items::AVOCADO,
};

pub fn crop(b: block::Block) -> Option<Crop> {
    [TOMATO, SUNFLOWER, WHEAT, PALTO]
        .into_iter()
        .find(|c| c.stages.contains(&b))
}

pub fn stage(b: block::Block) -> Option<usize> {
    crop(b)?.stages.iter().position(|&x| x == b)
}

pub fn next(b: block::Block) -> Option<block::Block> {
    let c = crop(b)?;
    let i = stage(b)?;
    (i + 1 < c.stages.len()).then_some(c.stages[i + 1])
}

pub fn mature(b: block::Block) -> bool {
    crop(b).is_some_and(|c| c.stages.last() == Some(&b))
}

pub fn drop_for(b: block::Block) -> Option<block::Block> {
    let c = crop(b)?;
    mature(b).then_some(c.harvest).or(Some(c.seed))
}

pub fn plant_block(item: block::Block) -> Option<block::Block> {
    match item {
        items::TOMATO_SEEDS => Some(block::TOMATO_0),
        items::SUNFLOWER_SEEDS => Some(block::SUNFLOWER_0),
        items::WHEAT_SEEDS => Some(block::WHEAT_0),
        items::PALTO_SAPLING => Some(block::PALTO_0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_crops_have_ordered_stages_and_only_the_last_is_mature() {
        for c in [TOMATO, SUNFLOWER, WHEAT] {
            assert_eq!(c.stages.len(), 4);
            for (i, &b) in c.stages.iter().enumerate() {
                assert_eq!(stage(b), Some(i));
                assert_eq!(mature(b), i == 3);
                assert!(block::breakable(b));
            }
        }
        assert_eq!(PALTO.stages.len(), 2);
        assert!(mature(block::PALTO_1));
    }
}
