//! Items that are not blocks: ids from `block::ITEM_BASE` up in the same `u8` as blocks, each
//! with a kind (material, tool, food) and, for tools, a level, a mining speed, a durability and
//! a sword's damage. Pure data: the client only reads it.

use crate::block::{self, Block, ITEM_BASE};

pub const STICK: Block = ITEM_BASE;
pub const COAL: Block = ITEM_BASE + 1;
pub const COPPER_INGOT: Block = ITEM_BASE + 2;
pub const IRON_INGOT: Block = ITEM_BASE + 3;
pub const APPLE: Block = ITEM_BASE + 4;
pub const WOODEN_PICKAXE: Block = ITEM_BASE + 5;
pub const STONE_PICKAXE: Block = ITEM_BASE + 6;
pub const COPPER_PICKAXE: Block = ITEM_BASE + 7;
pub const IRON_PICKAXE: Block = ITEM_BASE + 8;
pub const WOODEN_SWORD: Block = ITEM_BASE + 9;
pub const STONE_SWORD: Block = ITEM_BASE + 10;
pub const COPPER_SWORD: Block = ITEM_BASE + 11;
pub const IRON_SWORD: Block = ITEM_BASE + 12;
pub const SLIMEBALL: Block = ITEM_BASE + 13;
/// Right-clicked on the ground it spends itself to summon the Slime King.
pub const SLIME_CROWN_CORE: Block = ITEM_BASE + 14;
pub const ROCKET_KIT: Block = ITEM_BASE + 15;
pub const EGG: Block = ITEM_BASE + 16;
pub const RAW_CHICKEN: Block = ITEM_BASE + 17;
pub const COOKED_CHICKEN: Block = ITEM_BASE + 18;
pub const FEATHER: Block = ITEM_BASE + 19;
pub const TOMATO_SEEDS: Block = ITEM_BASE + 20;
pub const SUNFLOWER_SEEDS: Block = ITEM_BASE + 21;
pub const WHEAT_SEEDS: Block = ITEM_BASE + 22;
pub const AVOCADO: Block = ITEM_BASE + 23;
pub const TOMATO: Block = ITEM_BASE + 24;
pub const SUNFLOWER: Block = ITEM_BASE + 25;
pub const OIL: Block = ITEM_BASE + 26;
pub const MAYONNAISE: Block = ITEM_BASE + 27;
pub const BREAD: Block = ITEM_BASE + 28;
pub const VIENESA: Block = ITEM_BASE + 29;
pub const COMPLETO_ITALIANO: Block = ITEM_BASE + 30;
pub const PALTO_SAPLING: Block = ITEM_BASE + 31;
pub const WHEAT: Block = ITEM_BASE + 32;

/// How many non-block items there are (ids `ITEM_BASE..ITEM_BASE + COUNT`).
pub const COUNT: usize = 33;

/// Every non-block item, in the creative palette's order.
pub const ALL: [Block; COUNT] = [
    STICK,
    COAL,
    COPPER_INGOT,
    IRON_INGOT,
    APPLE,
    WOODEN_PICKAXE,
    STONE_PICKAXE,
    COPPER_PICKAXE,
    IRON_PICKAXE,
    WOODEN_SWORD,
    STONE_SWORD,
    COPPER_SWORD,
    IRON_SWORD,
    SLIMEBALL,
    SLIME_CROWN_CORE,
    ROCKET_KIT,
    EGG,
    RAW_CHICKEN,
    COOKED_CHICKEN,
    FEATHER,
    TOMATO_SEEDS,
    SUNFLOWER_SEEDS,
    WHEAT_SEEDS,
    AVOCADO,
    TOMATO,
    SUNFLOWER,
    OIL,
    MAYONNAISE,
    BREAD,
    VIENESA,
    COMPLETO_ITALIANO,
    PALTO_SAPLING,
    WHEAT,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Places a block.
    Block,
    Material,
    Tool(ToolKind),
    Food,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolKind {
    Pick,
    Sword,
}

/// What a tool is made of: its level, mining speed over bare hands, uses, and sword damage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Material {
    pub name: &'static str,
    pub level: u8,
    pub speed: u8,
    pub durability: u16,
    pub damage: u8,
}

pub const WOOD: Material = Material {
    name: "Wooden",
    level: 0,
    speed: 2,
    durability: 60,
    damage: 4,
};
pub const STONE: Material = Material {
    name: "Stone",
    level: 1,
    speed: 4,
    durability: 130,
    damage: 5,
};
pub const COPPER: Material = Material {
    name: "Copper",
    level: 2,
    speed: 5,
    durability: 180,
    damage: 6,
};
pub const IRON: Material = Material {
    name: "Iron",
    level: 3,
    speed: 7,
    durability: 250,
    damage: 7,
};
pub const MATERIALS: [Material; 4] = [WOOD, STONE, COPPER, IRON];

/// The id of a non-block item, as an index into the item tables.
fn index(item: Block) -> Option<usize> {
    (item >= ITEM_BASE && ((item - ITEM_BASE) as usize) < COUNT)
        .then(|| (item - ITEM_BASE) as usize)
}

/// Whether `item` is a non-block item that exists.
pub fn is_item(item: Block) -> bool {
    index(item).is_some()
}

/// Whether `item` is something a stack can hold: a block (not air) or one of these.
pub fn valid(item: Block) -> bool {
    is_item(item) || ((item as usize) < block::COUNT && item != block::AIR)
}

pub fn kind(item: Block) -> Kind {
    match index(item) {
        None => Kind::Block,
        Some(0..=3 | 13..=16 | 19..=22 | 26) => Kind::Material,
        Some(4 | 17..=18 | 23..=25 | 27..=30 | 32) => Kind::Food,
        Some(5..=8) => Kind::Tool(ToolKind::Pick),
        Some(9..=12) => Kind::Tool(ToolKind::Sword),
        Some(_) => Kind::Material,
    }
}

/// The material a tool is made of.
pub fn material(item: Block) -> Option<Material> {
    match (index(item)?, kind(item)) {
        (i, Kind::Tool(ToolKind::Pick)) => Some(MATERIALS[i - 5]),
        (i, Kind::Tool(ToolKind::Sword)) => Some(MATERIALS[i - 9]),
        _ => None,
    }
}

pub fn tool_kind(item: Block) -> Option<ToolKind> {
    match kind(item) {
        Kind::Tool(k) => Some(k),
        _ => None,
    }
}

/// The atlas tile of an item's picture.
pub fn tile(item: Block) -> u16 {
    block::tile::ITEM_BASE + item.saturating_sub(ITEM_BASE) as u16
}

/// Everything that can be in a creative palette: the blocks, then these items.
pub fn creative() -> &'static [Block] {
    static ALL_ITEMS: std::sync::OnceLock<Vec<Block>> = std::sync::OnceLock::new();
    ALL_ITEMS.get_or_init(|| block::ITEMS.iter().copied().chain(ALL).collect())
}

pub fn is_tool(item: Block) -> bool {
    tool_kind(item).is_some()
}

/// Uses a tool has before it breaks (0 for what is not a tool).
pub fn durability(item: Block) -> u16 {
    material(item).map_or(0, |m| m.durability)
}

/// A sword's damage (0 for anything else).
pub fn damage(item: Block) -> u8 {
    match tool_kind(item) {
        Some(ToolKind::Sword) => material(item).map_or(0, |m| m.damage),
        _ => 0,
    }
}

/// Health food restores when eaten (half hearts, as `health`).
pub fn food(item: Block) -> Option<f32> {
    match item {
        APPLE | RAW_CHICKEN | TOMATO | AVOCADO | BREAD => Some(4.0),
        COOKED_CHICKEN | MAYONNAISE | VIENESA => Some(6.0),
        COMPLETO_ITALIANO => Some(10.0),
        _ => None,
    }
}

pub fn max_stack(item: Block) -> u8 {
    if is_tool(item) { 1 } else { 64 }
}

pub fn name(item: Block) -> &'static str {
    const NAMES: [&str; COUNT] = [
        "Stick",
        "Coal",
        "Copper Ingot",
        "Iron Ingot",
        "Apple",
        "Wooden Pickaxe",
        "Stone Pickaxe",
        "Copper Pickaxe",
        "Iron Pickaxe",
        "Wooden Sword",
        "Stone Sword",
        "Copper Sword",
        "Iron Sword",
        "Bola de slime",
        "Núcleo de corona de slime",
        "Kit de cohete",
        "Huevo",
        "Pollo crudo",
        "Pollo cocido",
        "Pluma",
        "Semillas de tomate",
        "Semillas de girasol",
        "Semillas de trigo",
        "Palta",
        "Tomate",
        "Girasol",
        "Aceite de girasol",
        "Mayonesa",
        "Pan",
        "Vienesa",
        "Completo italiano",
        "Brote de palto",
        "Trigo",
    ];
    index(item).map_or("Unknown", |i| NAMES[i])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_apart_from_blocks() {
        assert!(block::COUNT <= ITEM_BASE as usize);
        for (i, &it) in ALL.iter().enumerate() {
            assert_eq!(it, ITEM_BASE + i as u8);
            assert!(is_item(it) && valid(it));
            assert_eq!(block::item(it), it);
            assert_ne!(name(it), "Unknown");
            assert_eq!(block::name(it), name(it));
        }
        for &it in &ALL {
            let t = block::tiles(it)[1];
            assert!(block::tile::NAMES[t as usize].starts_with("item_"));
        }
        assert!(
            creative().len() == block::ITEMS.len() + COUNT
                && creative().contains(&IRON_PICKAXE)
                && creative().contains(&block::FURNACE)
        );
        assert!(
            !is_item(ITEM_BASE + COUNT as u8)
                && !valid(ITEM_BASE + COUNT as u8)
                && !valid(block::AIR)
        );
        // Block ids did not move.
        assert_eq!(
            (block::COAL_ORE, block::COPPER_ORE, block::IRON_ORE),
            (21, 22, 23)
        );
        assert_eq!(block::item(block::STONE), block::STONE);
        assert_eq!(kind(block::STONE), Kind::Block);
    }

    #[test]
    fn sword_damage_by_level() {
        let d: Vec<u8> = [WOODEN_SWORD, STONE_SWORD, COPPER_SWORD, IRON_SWORD]
            .iter()
            .map(|&s| damage(s))
            .collect();
        assert_eq!(d, [4, 5, 6, 7]);
        assert_eq!(
            (damage(IRON_PICKAXE), damage(block::AIR), damage(STICK)),
            (0, 0, 0)
        );
    }

    #[test]
    fn tools_have_materials() {
        assert_eq!(material(WOODEN_PICKAXE), Some(WOOD));
        assert_eq!(material(IRON_SWORD), Some(IRON));
        assert_eq!(tool_kind(STONE_PICKAXE), Some(ToolKind::Pick));
        assert_eq!(tool_kind(COPPER_SWORD), Some(ToolKind::Sword));
        assert_eq!(durability(COPPER_PICKAXE), 180);
        assert_eq!(
            (
                damage(WOODEN_SWORD),
                damage(IRON_SWORD),
                damage(IRON_PICKAXE)
            ),
            (4, 7, 0)
        );
        assert_eq!(max_stack(IRON_PICKAXE), 1);
        assert_eq!(max_stack(STICK), 64);
        assert_eq!(food(APPLE), Some(4.0));
        assert_eq!(kind(COAL), Kind::Material);
        assert_eq!(
            (kind(SLIMEBALL), kind(SLIME_CROWN_CORE)),
            (Kind::Material, Kind::Material)
        );
        assert_eq!((SLIMEBALL, SLIME_CROWN_CORE), (141, 142));
        assert!(material(SLIMEBALL).is_none() && damage(SLIMEBALL) == 0);
        assert_eq!(kind(APPLE), Kind::Food);
    }
}
