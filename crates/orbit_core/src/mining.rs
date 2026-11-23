//! Breaking blocks in survival: how long each takes, which tool and level it needs, and what
//! it leaves. Pure tables and functions; the client keeps the progress and spends durability.

use crate::block::{self, Block};
use crate::items::{self, ToolKind};
use crate::rng::mix;

/// Percent chance that an oak leaf block drops an apple.
pub const APPLE_PERCENT: u64 = 5;

/// The tool a block is worked with (blocks not listed here are broken as well by hand).
pub fn tool_kind(b: Block) -> Option<ToolKind> {
    min_level(b).map(|_| ToolKind::Pick)
}

/// The lowest tool level that harvests a block (see `items::Material::level`); `None`: no
/// tool needed. Deep ores ask one level more than their shallow kind, up to iron's.
pub fn min_level(b: Block) -> Option<u8> {
    use block::*;
    Some(match b {
        STONE | COBBLESTONE | SMOOTH_STONE | DEEPSLATE | BASALT | SANDSTONE | COAL_ORE
        | FURNACE => 0,
        COPPER_ORE | DEEP_COAL_ORE => 1,
        IRON_ORE | DEEP_COPPER_ORE => 2,
        GOLD_ORE | REDSTONE_ORE | DIAMOND_ORE | DEEP_IRON_ORE | DEEP_GOLD_ORE
        | DEEP_REDSTONE_ORE | DEEP_DIAMOND_ORE | OBSIDIAN => 3,
        _ => return None,
    })
}

/// Seconds to break a block with bare hands (a pick takes its speed multiple off that).
pub fn hardness(b: Block) -> f32 {
    use block::*;
    match b {
        CRAFTING_TABLE => 3.5,
        TALL_GRASS | POPPY | DANDELION | CORNFLOWER | DEAD_BUSH | TORCH => 0.05,
        _ if block::is_wall_torch(b) => 0.05,
        LEAVES | BIRCH_LEAVES | PINE_LEAVES | ACACIA_LEAVES => 0.3,
        SNOW => 0.4,
        CACTUS => 0.6,
        DIRT | SAND => 0.75,
        GRASS | GRAVEL | GLASS | GLASS_PANE => 0.9,
        ICE => 1.0,
        SANDSTONE => 4.0,
        STONE | SMOOTH_STONE | BASALT | FURNACE => 7.5,
        COBBLESTONE => 10.0,
        COAL_ORE | COPPER_ORE | IRON_ORE | GOLD_ORE | REDSTONE_ORE | DIAMOND_ORE => 11.0,
        DEEPSLATE => 15.0,
        DEEP_COAL_ORE | DEEP_COPPER_ORE | DEEP_IRON_ORE | DEEP_GOLD_ORE | DEEP_REDSTONE_ORE
        | DEEP_DIAMOND_ORE => 16.0,
        OBSIDIAN => 40.0,
        _ => 3.0,
    }
}

/// The level of the pickaxe `tool` is (`None` for hands, swords, and anything else).
pub fn pick_level(tool: Option<Block>) -> Option<u8> {
    let t = tool?;
    (items::tool_kind(t) == Some(ToolKind::Pick)).then(|| items::material(t).map_or(0, |m| m.level))
}

/// Whether breaking `b` with `tool` leaves its drop.
pub fn can_harvest(b: Block, tool: Option<Block>) -> bool {
    match min_level(b) {
        None => true,
        Some(need) => pick_level(tool).is_some_and(|l| l >= need),
    }
}

/// Seconds to break `b` holding `tool` (nothing: bare hands). A pick speeds up what it is for;
/// a block that needs a better tool than the one in hand takes three times as long.
pub fn mining_time(b: Block, tool: Option<Block>) -> f32 {
    let speed = match (tool_kind(b), pick_level(tool)) {
        (Some(_), Some(_)) => items::material(tool.unwrap()).map_or(1, |m| m.speed) as f32,
        _ => 1.0,
    };
    let slow = if can_harvest(b, tool) { 1.0 } else { 3.0 };
    hardness(b) * slow / speed
}

/// What breaking `b` with `tool` leaves. `key` (the cell, say) makes the chance of an apple
/// out of leaves the same on every run and every machine.
pub fn drops(b: Block, tool: Option<Block>, key: u64) -> Option<Block> {
    if !can_harvest(b, tool) {
        return None;
    }
    match b {
        block::COAL_ORE | block::DEEP_COAL_ORE => Some(items::COAL),
        block::LEAVES => (mix(key ^ 0xA991E) % 100 < APPLE_PERCENT).then_some(items::APPLE),
        block::TALL_GRASS => match mix(key ^ 0xC0FF_EEED) % 4 {
            0 => Some(items::TOMATO_SEEDS),
            1 => Some(items::SUNFLOWER_SEEDS),
            _ => Some(items::WHEAT_SEEDS),
        },
        _ if crate::crop::crop(b).is_some() => crate::crop::drop_for(b),
        _ => block::drops(b),
    }
}

#[cfg(test)]
mod tests {
    use super::drops;
    use super::*;
    use crate::block::{
        BIRCH_LEAVES, Block, COAL_ORE, COBBLESTONE, COPPER_ORE, CORE, COUNT, DEEP_COAL_ORE,
        DEEP_IRON_ORE, DIAMOND_ORE, DIRT, GRASS, IRON_ORE, LEAVES, STONE, breakable, name,
    };

    #[test]
    fn a_pick_is_faster_and_the_level_decides_the_drop() {
        let hand = mining_time(STONE, None);
        assert!(mining_time(STONE, Some(items::WOODEN_PICKAXE)) < hand);
        assert!(
            mining_time(STONE, Some(items::IRON_PICKAXE))
                < mining_time(STONE, Some(items::STONE_PICKAXE))
        );
        // A sword or a stick does not speed up stone; a pick does not speed up dirt.
        assert_eq!(mining_time(STONE, Some(items::IRON_SWORD)), hand);
        assert_eq!(
            mining_time(DIRT, Some(items::IRON_PICKAXE)),
            mining_time(DIRT, None)
        );
        // Iron ore: stone pick breaks it (slowly) for nothing; copper pick gets it.
        assert_eq!(drops(IRON_ORE, Some(items::STONE_PICKAXE), 1), None);
        assert_eq!(drops(IRON_ORE, None, 1), None);
        assert_eq!(
            drops(IRON_ORE, Some(items::COPPER_PICKAXE), 1),
            Some(IRON_ORE)
        );
        assert!(
            mining_time(IRON_ORE, Some(items::STONE_PICKAXE))
                > mining_time(IRON_ORE, Some(items::COPPER_PICKAXE))
        );
        // Stone itself needs any pick.
        assert_eq!(drops(STONE, None, 1), None);
        assert_eq!(
            drops(STONE, Some(items::WOODEN_PICKAXE), 1),
            Some(COBBLESTONE)
        );
        assert_eq!(drops(COPPER_ORE, Some(items::WOODEN_PICKAXE), 1), None);
        assert_eq!(
            drops(COPPER_ORE, Some(items::STONE_PICKAXE), 1),
            Some(COPPER_ORE)
        );
        assert_eq!(
            drops(COAL_ORE, Some(items::WOODEN_PICKAXE), 1),
            Some(items::COAL)
        );
        assert_eq!(
            drops(DEEP_COAL_ORE, Some(items::STONE_PICKAXE), 1),
            Some(items::COAL)
        );
        assert_eq!(drops(DEEP_COAL_ORE, Some(items::WOODEN_PICKAXE), 1), None);
        assert_eq!(
            drops(DEEP_IRON_ORE, Some(items::IRON_PICKAXE), 1),
            Some(DEEP_IRON_ORE)
        );
        assert_eq!(drops(DIAMOND_ORE, Some(items::COPPER_PICKAXE), 1), None);
        assert_eq!(drops(DIRT, None, 1), Some(DIRT));
        assert_eq!(drops(GRASS, None, 1), Some(DIRT));
        assert_eq!(drops(CORE, Some(items::IRON_PICKAXE), 1), None);
    }

    #[test]
    fn leaves_drop_apples_about_five_percent_of_the_time_and_always_the_same() {
        let n = (0..20_000u64)
            .filter(|&k| drops(LEAVES, None, k) == Some(items::APPLE))
            .count();
        assert!((800..1200).contains(&n), "{n} apples of 20000");
        let a: Vec<_> = (0..200u64).map(|k| drops(LEAVES, None, k)).collect();
        let b: Vec<_> = (0..200u64).map(|k| drops(LEAVES, None, k)).collect();
        assert_eq!(a, b);
        assert_eq!(drops(BIRCH_LEAVES, None, 3), None);
    }

    #[test]
    fn hardness_is_positive_where_breakable() {
        for b in 0..COUNT as Block {
            if breakable(b) {
                assert!(
                    hardness(b) > 0.0 && mining_time(b, None).is_finite(),
                    "{}",
                    name(b)
                );
            }
        }
    }
}
