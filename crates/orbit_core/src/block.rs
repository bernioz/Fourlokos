//! Block types: shape, how they meet light and the player, and which texture tiles they wear.
//! Textures are named tiles in an atlas (see `tile`); the client paints them, or loads PNGs.

pub type Block = u8;

pub const AIR: Block = 0;
pub const CORE: Block = 1; // bedrock: unbreakable
pub const STONE: Block = 2;
pub const DIRT: Block = 3;
pub const GRASS: Block = 4;
pub const SAND: Block = 5;
pub const SNOW: Block = 6;
pub const WOOD: Block = 7;
pub const LEAVES: Block = 8;
pub const PLANKS: Block = 9;
pub const CRYSTAL: Block = 10;
pub const BASALT: Block = 11;
pub const BIRCH_LOG: Block = 12;
pub const BIRCH_LEAVES: Block = 13;
pub const PINE_LEAVES: Block = 14;
pub const ACACIA_LEAVES: Block = 15;
pub const CACTUS: Block = 16;
pub const SANDSTONE: Block = 17;
pub const DEEPSLATE: Block = 18;
pub const GRAVEL: Block = 19;
pub const COBBLESTONE: Block = 20;
pub const COAL_ORE: Block = 21;
pub const COPPER_ORE: Block = 22;
pub const IRON_ORE: Block = 23;
pub const GOLD_ORE: Block = 24;
pub const REDSTONE_ORE: Block = 25;
pub const DIAMOND_ORE: Block = 26;
pub const DEEP_COAL_ORE: Block = 27;
pub const DEEP_COPPER_ORE: Block = 28;
pub const DEEP_IRON_ORE: Block = 29;
pub const DEEP_GOLD_ORE: Block = 30;
pub const DEEP_REDSTONE_ORE: Block = 31;
pub const DEEP_DIAMOND_ORE: Block = 32;
pub const TORCH: Block = 33;
pub const TALL_GRASS: Block = 34;
pub const POPPY: Block = 35;
pub const DANDELION: Block = 36;
pub const CORNFLOWER: Block = 37;
pub const DEAD_BUSH: Block = 38;
pub const SMOOTH_STONE: Block = 39;
/// Holds a station together: unbreakable, opens the station's menu.
pub const STATION_CORE: Block = 40;
/// Rocket parts (see `rocket`): white hull plating, see-through glass, an engine (its nozzle
/// underneath), the helm that flies the rocket, and a chest.
pub const HULL: Block = 41;
pub const GLASS: Block = 42;
pub const ENGINE: Block = 43;
pub const HELM: Block = 44;
pub const CHEST: Block = 45;
/// Landing pads (see `pad`): laid in a full square, at least 3 × 3, they light up (`PAD_READY`)
/// and rockets come down on them, on planets and on stations.
pub const PAD: Block = 46;
pub const PAD_READY: Block = 47;
/// A space elevator's ground platform (see `elevator::base_cells`): steel tread plate, in a
/// ring of hazard-striped edge blocks.
pub const ELEVATOR_BASE: Block = 48;
pub const ELEVATOR_EDGE: Block = 49;
/// Liquids (see `liquid`): each kind takes `LIQUID_STATES` ids from its first: the source, a
/// falling column, then flowing levels 1 (thinnest) to 7.
pub const WATER: Block = 50;
pub const LAVA: Block = 59;
pub const LIQUID_STATES: Block = 9;
/// What lava turns into where water meets it: a source becomes obsidian.
pub const OBSIDIAN: Block = 68;
/// The frozen top of cold seas.
pub const ICE: Block = 69;
/// A thin pane of glass: a post that joins the panes, glass and solid blocks beside it.
pub const GLASS_PANE: Block = 70;
/// An oak fence: a post with rails to the fences and solid blocks beside it; 1.5 blocks high to
/// jump over.
pub const FENCE: Block = 71;
/// Blocks that face one of a cell's four sides (`Dir`) take four ids from their first, by
/// side: a torch on a wall (the wall on that side), a ladder (against the wall on that side).
pub const WALL_TORCH: Block = 72;
pub const LADDER: Block = 76;
/// An oak door: 16 ids, `DOOR + side + 4 open + 8 upper half` (see `door`). Closed, its panel
/// lies along `side`; open, along the next side round (`Dir::turned`).
pub const DOOR: Block = 80;
/// An oak trapdoor: 16 ids, `TRAPDOOR + side + 4 open + 8 top half` (see `trapdoor`). Closed, a
/// slab at the bottom (or top) of its cell; open, standing up along `side`, where its hinge is.
pub const TRAPDOOR: Block = 96;

pub const COUNT: usize = 112;

/// How a block is drawn and collided.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Empty,
    /// A full cell: collides, hides its neighbors' faces, stops light.
    Cube,
    /// Two crossed planes (grass, flowers): walk-through, let light by.
    Cross,
    /// A thin stick standing in the cell.
    Torch,
    /// Water or lava: a surface at its level's height, walk-through (you swim), lets light by.
    Liquid,
    /// Made of boxes smaller than the cell (doors, trapdoors, ladders, fences, panes; see
    /// `shape::boxes`): collides by them, lets light by.
    Boxes,
}

/// Texture tiles of the atlas, by index. `NAMES` gives each its file name (`<name>.png`).
pub mod tile {
    pub const WHITE: u16 = 0;
    pub const STONE: u16 = 1;
    pub const DIRT: u16 = 2;
    pub const GRASS_TOP: u16 = 3;
    pub const GRASS_SIDE: u16 = 4;
    pub const SAND: u16 = 5;
    pub const SNOW: u16 = 6;
    pub const OAK_LOG: u16 = 7;
    pub const OAK_LOG_TOP: u16 = 8;
    pub const OAK_LEAVES: u16 = 9;
    pub const PLANKS: u16 = 10;
    pub const CRYSTAL: u16 = 11;
    pub const BASALT: u16 = 12;
    pub const BIRCH_LOG: u16 = 13;
    pub const BIRCH_LOG_TOP: u16 = 14;
    pub const BIRCH_LEAVES: u16 = 15;
    pub const PINE_LEAVES: u16 = 16;
    pub const ACACIA_LEAVES: u16 = 17;
    pub const CACTUS_SIDE: u16 = 18;
    pub const CACTUS_TOP: u16 = 19;
    pub const SANDSTONE: u16 = 20;
    pub const SANDSTONE_TOP: u16 = 21;
    pub const DEEPSLATE: u16 = 22;
    pub const GRAVEL: u16 = 23;
    pub const COBBLESTONE: u16 = 24;
    pub const BEDROCK: u16 = 25;
    /// Ores on stone, then on deepslate, in `ORES` order.
    pub const ORE: u16 = 26;
    pub const DEEP_ORE: u16 = 32;
    pub const TORCH: u16 = 38;
    pub const TALL_GRASS: u16 = 39;
    pub const POPPY: u16 = 40;
    pub const DANDELION: u16 = 41;
    pub const CORNFLOWER: u16 = 42;
    pub const DEAD_BUSH: u16 = 43;
    pub const SMOOTH_STONE: u16 = 44;
    pub const STATION_CORE: u16 = 45;
    pub const STATION_CORE_TOP: u16 = 46;
    pub const HULL: u16 = 47;
    pub const GLASS: u16 = 48;
    pub const ENGINE_SIDE: u16 = 49;
    pub const ENGINE_NOZZLE: u16 = 50;
    pub const HELM_SIDE: u16 = 51;
    pub const HELM_TOP: u16 = 52;
    pub const CHEST_SIDE: u16 = 53;
    pub const CHEST_TOP: u16 = 54;
    pub const PAD_TOP: u16 = 55;
    pub const PAD_READY_TOP: u16 = 56;
    pub const PAD_SIDE: u16 = 57;
    pub const ELEVATOR_BASE_TOP: u16 = 58;
    pub const ELEVATOR_EDGE_TOP: u16 = 59;
    pub const ELEVATOR_BASE_SIDE: u16 = 60;
    pub const WATER: u16 = 61;
    pub const LAVA: u16 = 62;
    pub const OBSIDIAN: u16 = 63;
    pub const ICE: u16 = 64;
    pub const LADDER: u16 = 65;
    pub const DOOR_TOP: u16 = 66;
    pub const DOOR_BOTTOM: u16 = 67;
    pub const TRAPDOOR: u16 = 68;

    pub const COUNT: usize = 69;
    pub const NAMES: [&str; COUNT] = [
        "white", "stone", "dirt", "grass_top", "grass_side", "sand", "snow", "oak_log", "oak_log_top", "oak_leaves", "planks",
        "crystal", "basalt", "birch_log", "birch_log_top", "birch_leaves", "pine_leaves", "acacia_leaves", "cactus_side",
        "cactus_top", "sandstone", "sandstone_top", "deepslate", "gravel", "cobblestone", "bedrock", "coal_ore", "copper_ore",
        "iron_ore", "gold_ore", "redstone_ore", "diamond_ore", "deepslate_coal_ore", "deepslate_copper_ore", "deepslate_iron_ore",
        "deepslate_gold_ore", "deepslate_redstone_ore", "deepslate_diamond_ore", "torch", "tall_grass", "poppy", "dandelion",
        "cornflower", "dead_bush", "smooth_stone", "station_core", "station_core_top",
        "hull", "glass", "engine_side", "engine_nozzle", "helm_side", "helm_top", "chest_side", "chest_top",
        "landing_pad", "landing_pad_ready", "landing_pad_side", "elevator_base", "elevator_edge", "elevator_base_side",
        "water_still", "lava_still", "obsidian", "ice",
        "ladder", "oak_door_top", "oak_door_bottom", "oak_trapdoor",
    ];

    /// Atlas layout: `COLUMNS` tiles per row.
    pub const COLUMNS: usize = 8;
    pub const ROWS: usize = COUNT.div_ceil(COLUMNS);

    /// Atlas UV of a point (u, v in 0..1, v down) inside a tile, pulled in a hair from the edges
    /// so neighbors never bleed in.
    pub fn uv(t: u16, u: f32, v: f32) -> [f32; 2] {
        const INSET: f32 = 0.002;
        let (col, row) = ((t as usize % COLUMNS) as f32, (t as usize / COLUMNS) as f32);
        let (u, v) = (INSET + u * (1.0 - 2.0 * INSET), INSET + v * (1.0 - 2.0 * INSET));
        [(col + u) / COLUMNS as f32, (row + v) / ROWS as f32]
    }
}

struct Info {
    name: &'static str,
    /// Average color, for far views.
    color: [f32; 3],
    shape: Shape,
    /// Tiles for top, sides, bottom.
    tiles: [u16; 3],
    /// Block light it gives off, 0..=15.
    light: u8,
}

const fn cube(name: &'static str, color: [f32; 3], t: u16) -> Info {
    Info { name, color, shape: Shape::Cube, tiles: [t, t, t], light: 0 }
}
const fn column(name: &'static str, color: [f32; 3], ends: u16, side: u16) -> Info {
    Info { name, color, shape: Shape::Cube, tiles: [ends, side, ends], light: 0 }
}
const fn plant(name: &'static str, color: [f32; 3], t: u16) -> Info {
    Info { name, color, shape: Shape::Cross, tiles: [t, t, t], light: 0 }
}

use tile as t;
const INFO: [Info; COUNT] = [
    Info { name: "Air", color: [0.0; 3], shape: Shape::Empty, tiles: [0; 3], light: 0 },
    cube("Bedrock", [0.16, 0.12, 0.18], t::BEDROCK),
    cube("Stone", [0.52, 0.52, 0.55], t::STONE),
    cube("Dirt", [0.50, 0.35, 0.23], t::DIRT),
    Info { name: "Grass", color: [0.42, 0.68, 0.30], shape: Shape::Cube, tiles: [t::GRASS_TOP, t::GRASS_SIDE, t::DIRT], light: 0 },
    cube("Sand", [0.92, 0.85, 0.62], t::SAND),
    cube("Snow", [0.95, 0.97, 1.00], t::SNOW),
    column("Oak Log", [0.42, 0.29, 0.18], t::OAK_LOG_TOP, t::OAK_LOG),
    cube("Oak Leaves", [0.25, 0.55, 0.22], t::OAK_LEAVES),
    cube("Planks", [0.76, 0.60, 0.38], t::PLANKS),
    Info { name: "Crystal", color: [0.50, 0.85, 0.98], shape: Shape::Cube, tiles: [t::CRYSTAL; 3], light: 9 },
    cube("Basalt", [0.22, 0.20, 0.25], t::BASALT),
    column("Birch Log", [0.88, 0.86, 0.80], t::BIRCH_LOG_TOP, t::BIRCH_LOG),
    cube("Birch Leaves", [0.52, 0.72, 0.30], t::BIRCH_LEAVES),
    cube("Pine Needles", [0.17, 0.38, 0.27], t::PINE_LEAVES),
    cube("Acacia Leaves", [0.55, 0.64, 0.20], t::ACACIA_LEAVES),
    column("Cactus", [0.30, 0.58, 0.28], t::CACTUS_TOP, t::CACTUS_SIDE),
    Info { name: "Sandstone", color: [0.84, 0.72, 0.50], shape: Shape::Cube, tiles: [t::SANDSTONE_TOP, t::SANDSTONE, t::SANDSTONE_TOP], light: 0 },
    cube("Deepslate", [0.30, 0.30, 0.33], t::DEEPSLATE),
    cube("Gravel", [0.50, 0.47, 0.46], t::GRAVEL),
    cube("Cobblestone", [0.48, 0.48, 0.48], t::COBBLESTONE),
    cube("Coal Ore", [0.42, 0.42, 0.43], t::ORE),
    cube("Copper Ore", [0.55, 0.50, 0.46], t::ORE + 1),
    cube("Iron Ore", [0.58, 0.54, 0.50], t::ORE + 2),
    cube("Gold Ore", [0.60, 0.57, 0.45], t::ORE + 3),
    Info { name: "Redstone Ore", color: [0.58, 0.42, 0.42], shape: Shape::Cube, tiles: [t::ORE + 4; 3], light: 4 },
    cube("Diamond Ore", [0.48, 0.62, 0.64], t::ORE + 5),
    cube("Deepslate Coal Ore", [0.25, 0.25, 0.27], t::DEEP_ORE),
    cube("Deepslate Copper Ore", [0.36, 0.33, 0.32], t::DEEP_ORE + 1),
    cube("Deepslate Iron Ore", [0.38, 0.35, 0.34], t::DEEP_ORE + 2),
    cube("Deepslate Gold Ore", [0.40, 0.37, 0.30], t::DEEP_ORE + 3),
    Info { name: "Deepslate Redstone Ore", color: [0.40, 0.28, 0.29], shape: Shape::Cube, tiles: [t::DEEP_ORE + 4; 3], light: 4 },
    cube("Deepslate Diamond Ore", [0.30, 0.42, 0.45], t::DEEP_ORE + 5),
    Info { name: "Torch", color: [1.0, 0.8, 0.4], shape: Shape::Torch, tiles: [t::TORCH; 3], light: 14 },
    plant("Grass", [0.40, 0.66, 0.28], t::TALL_GRASS),
    plant("Poppy", [0.85, 0.20, 0.15], t::POPPY),
    plant("Dandelion", [0.95, 0.85, 0.20], t::DANDELION),
    plant("Cornflower", [0.35, 0.45, 0.90], t::CORNFLOWER),
    plant("Dead Bush", [0.55, 0.40, 0.22], t::DEAD_BUSH),
    cube("Smooth Stone", [0.62, 0.62, 0.63], t::SMOOTH_STONE),
    Info { name: "Station Core", color: [0.55, 0.75, 0.95], shape: Shape::Cube, tiles: [t::STATION_CORE_TOP, t::STATION_CORE, t::STATION_CORE_TOP], light: 7 },
    cube("Hull", [0.86, 0.87, 0.89], t::HULL),
    cube("Glass", [0.75, 0.88, 0.95], t::GLASS),
    Info { name: "Engine", color: [0.40, 0.40, 0.44], shape: Shape::Cube, tiles: [t::HULL, t::ENGINE_SIDE, t::ENGINE_NOZZLE], light: 0 },
    Info { name: "Helm", color: [0.35, 0.45, 0.60], shape: Shape::Cube, tiles: [t::HELM_TOP, t::HELM_SIDE, t::HULL], light: 5 },
    Info { name: "Chest", color: [0.62, 0.45, 0.25], shape: Shape::Cube, tiles: [t::CHEST_TOP, t::CHEST_SIDE, t::CHEST_TOP], light: 0 },
    Info { name: "Landing Pad", color: [0.36, 0.37, 0.40], shape: Shape::Cube, tiles: [t::PAD_TOP, t::PAD_SIDE, t::PAD_SIDE], light: 0 },
    Info { name: "Landing Pad", color: [0.38, 0.40, 0.44], shape: Shape::Cube, tiles: [t::PAD_READY_TOP, t::PAD_SIDE, t::PAD_SIDE], light: 6 },
    Info { name: "Elevator Base", color: [0.40, 0.42, 0.46], shape: Shape::Cube, tiles: [t::ELEVATOR_BASE_TOP, t::ELEVATOR_BASE_SIDE, t::ELEVATOR_BASE_SIDE], light: 0 },
    Info { name: "Elevator Base", color: [0.62, 0.52, 0.22], shape: Shape::Cube, tiles: [t::ELEVATOR_EDGE_TOP, t::ELEVATOR_BASE_SIDE, t::ELEVATOR_BASE_SIDE], light: 0 },
    water("Water"), water("Falling Water"), water("Water"), water("Water"), water("Water"), water("Water"), water("Water"), water("Water"), water("Water"),
    lava("Lava"), lava("Falling Lava"), lava("Lava"), lava("Lava"), lava("Lava"), lava("Lava"), lava("Lava"), lava("Lava"), lava("Lava"),
    cube("Obsidian", [0.10, 0.07, 0.16], t::OBSIDIAN),
    cube("Ice", [0.62, 0.78, 0.95], t::ICE),
    Info { name: "Glass Pane", color: [0.75, 0.88, 0.95], shape: Shape::Boxes, tiles: [t::GLASS; 3], light: 0 },
    Info { name: "Oak Fence", color: [0.76, 0.60, 0.38], shape: Shape::Boxes, tiles: [t::PLANKS; 3], light: 0 },
    wall_torch_info(), wall_torch_info(), wall_torch_info(), wall_torch_info(),
    ladder_info(), ladder_info(), ladder_info(), ladder_info(),
    door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM),
    door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM), door_info(t::DOOR_BOTTOM),
    door_info(t::DOOR_TOP), door_info(t::DOOR_TOP), door_info(t::DOOR_TOP), door_info(t::DOOR_TOP),
    door_info(t::DOOR_TOP), door_info(t::DOOR_TOP), door_info(t::DOOR_TOP), door_info(t::DOOR_TOP),
    trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(),
    trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(), trapdoor_info(),
];

const fn wall_torch_info() -> Info {
    Info { name: "Torch", color: [1.0, 0.8, 0.4], shape: Shape::Torch, tiles: [t::TORCH; 3], light: 14 }
}
const fn ladder_info() -> Info {
    Info { name: "Ladder", color: [0.62, 0.48, 0.28], shape: Shape::Boxes, tiles: [t::LADDER; 3], light: 0 }
}
const fn door_info(t: u16) -> Info {
    Info { name: "Oak Door", color: [0.62, 0.48, 0.28], shape: Shape::Boxes, tiles: [t; 3], light: 0 }
}
const fn trapdoor_info() -> Info {
    Info { name: "Oak Trapdoor", color: [0.62, 0.48, 0.28], shape: Shape::Boxes, tiles: [t::TRAPDOOR; 3], light: 0 }
}

const fn water(name: &'static str) -> Info {
    Info { name, color: [0.16, 0.36, 0.70], shape: Shape::Liquid, tiles: [t::WATER; 3], light: 0 }
}
const fn lava(name: &'static str) -> Info {
    Info { name, color: [0.95, 0.42, 0.08], shape: Shape::Liquid, tiles: [t::LAVA; 3], light: 15 }
}

/// Ores from shallow to deep, with where they lie (fraction of the crust from the surface: 0 top,
/// 1 bedrock), veins per 1000 crust cells and cells per vein; the deepslate kind is `deep`.
pub struct Ore {
    pub block: Block,
    pub deep: Block,
    pub from: f32,
    pub to: f32,
    pub rate: f32,
    pub size: usize,
}

pub const ORES: [Ore; 6] = [
    Ore { block: COAL_ORE, deep: DEEP_COAL_ORE, from: 0.05, to: 0.6, rate: 1.2, size: 8 },
    Ore { block: COPPER_ORE, deep: DEEP_COPPER_ORE, from: 0.15, to: 0.6, rate: 0.8, size: 6 },
    Ore { block: IRON_ORE, deep: DEEP_IRON_ORE, from: 0.2, to: 0.9, rate: 1.0, size: 5 },
    Ore { block: GOLD_ORE, deep: DEEP_GOLD_ORE, from: 0.6, to: 1.0, rate: 0.3, size: 4 },
    Ore { block: REDSTONE_ORE, deep: DEEP_REDSTONE_ORE, from: 0.7, to: 1.0, rate: 0.5, size: 5 },
    Ore { block: DIAMOND_ORE, deep: DEEP_DIAMOND_ORE, from: 0.85, to: 1.0, rate: 0.15, size: 3 },
];

/// Every item there is, in the creative inventory's order (an item is a block: what it places).
/// Water and lava go down as sources, like emptying a bucket.
pub const ITEMS: [Block; 55] = [
    STONE, COBBLESTONE, SMOOTH_STONE, DEEPSLATE, BASALT, OBSIDIAN, DIRT, GRASS, SAND, SANDSTONE, GRAVEL, SNOW, ICE, CRYSTAL,
    WOOD, BIRCH_LOG, PLANKS, LEAVES, BIRCH_LEAVES, PINE_LEAVES, ACACIA_LEAVES, CACTUS,
    GLASS, GLASS_PANE, FENCE, DOOR, TRAPDOOR, LADDER, TORCH,
    TALL_GRASS, POPPY, DANDELION, CORNFLOWER, DEAD_BUSH,
    COAL_ORE, COPPER_ORE, IRON_ORE, GOLD_ORE, REDSTONE_ORE, DIAMOND_ORE,
    DEEP_COAL_ORE, DEEP_COPPER_ORE, DEEP_IRON_ORE, DEEP_GOLD_ORE, DEEP_REDSTONE_ORE, DEEP_DIAMOND_ORE,
    HULL, ENGINE, HELM, CHEST, PAD, ELEVATOR_BASE, ELEVATOR_EDGE, WATER, LAVA,
];

/// What the creative hotbar starts with.
pub const STARTER: [Block; 9] = [STONE, DIRT, GRASS, PLANKS, WOOD, GLASS, TORCH, WATER, LAVA];

/// The item a block is (what picking it gives, the stack it goes in): a wall torch is a torch,
/// either half of a door the door, a lit landing pad a pad, flowing water water.
pub fn item(b: Block) -> Block {
    match b {
        _ if (WALL_TORCH..WALL_TORCH + 4).contains(&b) => TORCH,
        _ if (LADDER..LADDER + 4).contains(&b) => LADDER,
        _ if (DOOR..DOOR + 16).contains(&b) => DOOR,
        _ if (TRAPDOOR..TRAPDOOR + 16).contains(&b) => TRAPDOOR,
        PAD_READY => PAD,
        _ => match liquid(b) {
            Some(l) => l.source(),
            None => b,
        },
    }
}

/// What breaking a block leaves to pick up in survival, as Minecraft has it: stone gives
/// cobblestone, grass dirt; glass, ice, leaves and grass give nothing; a door gives itself from
/// its lower half only.
pub fn drops(b: Block) -> Option<Block> {
    match b {
        STONE => Some(COBBLESTONE),
        GRASS => Some(DIRT),
        GLASS | GLASS_PANE | ICE | TALL_GRASS | LEAVES | BIRCH_LEAVES | PINE_LEAVES | ACACIA_LEAVES | AIR | CORE | STATION_CORE => None,
        _ if (DOOR + 8..DOOR + 16).contains(&b) => None,
        _ if is_liquid(b) => None,
        _ => Some(item(b)),
    }
}

/// Most of one item a slot holds.
pub fn max_stack(_item: Block) -> u8 {
    64
}

fn info(b: Block) -> &'static Info {
    INFO.get(b as usize).unwrap_or(&INFO[STONE as usize])
}

pub fn name(b: Block) -> &'static str {
    info(b).name
}

pub fn color(b: Block) -> [f32; 3] {
    info(b).color
}

pub fn shape(b: Block) -> Shape {
    info(b).shape
}

/// Tiles for (top, sides, bottom).
pub fn tiles(b: Block) -> [u16; 3] {
    info(b).tiles
}

/// Block light given off.
pub fn emission(b: Block) -> u8 {
    info(b).light
}

/// Collides with the player.
pub fn is_solid(b: Block) -> bool {
    shape(b) == Shape::Cube
}

/// Hides the faces behind it and stops light.
pub fn opaque(b: Block) -> bool {
    shape(b) == Shape::Cube && b != GLASS
}

/// A face of `b` against `next` is not drawn: `next` hides it, or both are glass (one pane).
pub fn face_hidden(b: Block, next: Block) -> bool {
    opaque(next) || (b == GLASS && next == GLASS)
}

/// Something a ray stops at (to break it, or place against it). Rays go through liquids, and
/// a block placed there takes the liquid's place.
pub fn targetable(b: Block) -> bool {
    b != AIR && !is_liquid(b)
}

/// The two liquids.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Liquid {
    Water,
    Lava,
}

impl Liquid {
    pub fn base(self) -> Block {
        match self {
            Liquid::Water => WATER,
            Liquid::Lava => LAVA,
        }
    }
    /// The still source block.
    pub fn source(self) -> Block {
        self.base()
    }
    /// A falling column (full, not a source).
    pub fn falling(self) -> Block {
        self.base() + 1
    }
    /// Flowing at `level` 1..=7 (7 next to a source).
    pub fn flowing(self, level: u8) -> Block {
        self.base() + 1 + level.clamp(1, 7)
    }
}

pub fn is_liquid(b: Block) -> bool {
    shape(b) == Shape::Liquid
}

pub fn liquid(b: Block) -> Option<Liquid> {
    match b {
        _ if (WATER..WATER + LIQUID_STATES).contains(&b) => Some(Liquid::Water),
        _ if (LAVA..LAVA + LIQUID_STATES).contains(&b) => Some(Liquid::Lava),
        _ => None,
    }
}

pub fn is_source(b: Block) -> bool {
    b == WATER || b == LAVA
}

pub fn is_falling(b: Block) -> bool {
    b == WATER + 1 || b == LAVA + 1
}

/// How much liquid a cell holds: 8 for a source or a falling column, else its flowing level
/// 1..=7; 0 for anything else.
pub fn amount(b: Block) -> u8 {
    match liquid(b) {
        Some(l) => match b - l.base() {
            0 | 1 => 8,
            n => n - 1,
        },
        None => 0,
    }
}

/// A liquid flowing in washes it away: air, plants, torches.
pub fn washable(b: Block) -> bool {
    b == AIR || matches!(shape(b), Shape::Cross | Shape::Torch)
}

/// Part of a landing pad, lit or not.
pub fn is_pad(b: Block) -> bool {
    b == PAD || b == PAD_READY
}

pub fn breakable(b: Block) -> bool {
    b != AIR && b != CORE && b != STATION_CORE && !is_liquid(b)
}

/// Stands on the block below and goes when that one does (see `shape::supported` for what
/// each needs, side walls too).
pub fn needs_support(b: Block) -> bool {
    matches!(shape(b), Shape::Cross | Shape::Torch) || is_door(b) || is_ladder(b) || b == CACTUS
}

/// A horizontal side of a cell: along the grid's first axis (u: a planet face's i, a station's
/// x) down or up, or its second (v: j, z). `turned` goes round a quarter.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Dir(pub u8);

impl Dir {
    pub const ALL: [Dir; 4] = [Dir(0), Dir(1), Dir(2), Dir(3)];
    pub fn opposite(self) -> Dir {
        Dir(self.0 ^ 1)
    }
    /// A quarter turn: u- → v- → u+ → v+ → u-.
    pub fn turned(self) -> Dir {
        Dir([2, 3, 1, 0][self.0 as usize & 3])
    }
    /// (axis 0 = u or 1 = v, sign ±1).
    pub fn axis(self) -> (usize, f32) {
        ((self.0 >> 1) as usize, if self.0 & 1 == 1 { 1.0 } else { -1.0 })
    }
}

/// The side a wall torch, ladder, door or trapdoor faces.
pub fn facing(b: Block) -> Option<Dir> {
    match b {
        _ if (WALL_TORCH..WALL_TORCH + 4).contains(&b) => Some(Dir(b - WALL_TORCH)),
        _ if (LADDER..LADDER + 4).contains(&b) => Some(Dir(b - LADDER)),
        _ if is_door(b) => Some(Dir((b - DOOR) & 3)),
        _ if is_trapdoor(b) => Some(Dir((b - TRAPDOOR) & 3)),
        _ => None,
    }
}

pub fn wall_torch(d: Dir) -> Block {
    WALL_TORCH + (d.0 & 3)
}
pub fn ladder(d: Dir) -> Block {
    LADDER + (d.0 & 3)
}
pub fn door(d: Dir, open: bool, upper: bool) -> Block {
    DOOR + (d.0 & 3) + 4 * open as u8 + 8 * upper as u8
}
pub fn trapdoor(d: Dir, open: bool, top: bool) -> Block {
    TRAPDOOR + (d.0 & 3) + 4 * open as u8 + 8 * top as u8
}
pub fn is_door(b: Block) -> bool {
    (DOOR..DOOR + 16).contains(&b)
}
pub fn is_trapdoor(b: Block) -> bool {
    (TRAPDOOR..TRAPDOOR + 16).contains(&b)
}
pub fn is_ladder(b: Block) -> bool {
    (LADDER..LADDER + 4).contains(&b)
}
pub fn is_wall_torch(b: Block) -> bool {
    (WALL_TORCH..WALL_TORCH + 4).contains(&b)
}
/// Open (a door or a trapdoor).
pub fn is_open(b: Block) -> bool {
    (is_door(b) && (b - DOOR) & 4 != 0) || (is_trapdoor(b) && (b - TRAPDOOR) & 4 != 0)
}
/// The upper half of a door.
pub fn is_upper(b: Block) -> bool {
    is_door(b) && (b - DOOR) & 8 != 0
}
/// A trapdoor in the top half of its cell.
pub fn is_top(b: Block) -> bool {
    is_trapdoor(b) && (b - TRAPDOOR) & 8 != 0
}
/// A door or trapdoor opened (closed if open); anything else as it is.
pub fn toggled(b: Block) -> Block {
    if is_door(b) || is_trapdoor(b) { b ^ 4 } else { b }
}

/// Something a hand opens and closes (right click).
pub fn opens(b: Block) -> bool {
    is_door(b) || is_trapdoor(b)
}

/// Foliage and grass take a regional tint so planets don't look flat-colored.
pub fn tinted(b: Block) -> bool {
    matches!(b, GRASS | LEAVES | BIRCH_LEAVES | PINE_LEAVES | ACACIA_LEAVES | TALL_GRASS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_line_up() {
        assert_eq!(tile::NAMES.len(), tile::COUNT);
        for b in 0..COUNT as Block {
            for t in tiles(b) {
                assert!((t as usize) < tile::COUNT, "{} tile {t}", name(b));
            }
        }
        assert_eq!(tile::NAMES[tile::ORE as usize], "coal_ore");
        assert_eq!(tile::NAMES[tile::DEEP_ORE as usize + 5], "deepslate_diamond_ore");
        assert_eq!(tile::NAMES[tile::DEAD_BUSH as usize], "dead_bush");
        assert_eq!(name(DEAD_BUSH), "Dead Bush");
        assert!(emission(TORCH) > 0 && !is_solid(TORCH) && !opaque(TALL_GRASS));
        assert_eq!(tile::NAMES[tile::ICE as usize], "ice");
        assert_eq!(name(OBSIDIAN), "Obsidian");
    }

    #[test]
    fn liquid_states() {
        for l in [Liquid::Water, Liquid::Lava] {
            assert_eq!(liquid(l.source()), Some(l));
            assert!(is_source(l.source()) && is_falling(l.falling()));
            assert_eq!(amount(l.source()), 8);
            assert_eq!(amount(l.falling()), 8);
            for n in 1..=7 {
                assert_eq!(amount(l.flowing(n)), n);
                assert_eq!(liquid(l.flowing(n)), Some(l));
            }
        }
        assert_eq!(Liquid::Lava.flowing(7), OBSIDIAN - 1);
        assert!(liquid(OBSIDIAN).is_none() && !targetable(WATER) && !is_solid(LAVA) && !opaque(WATER));
        assert!(emission(LAVA) == 15 && washable(TORCH) && !washable(STONE));
    }
}
