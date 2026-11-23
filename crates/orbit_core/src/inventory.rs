//! The player's inventory, as Minecraft's: 36 slots, the first nine the hotbar, stacks of up
//! to 64 of an item (an item is the block it places, `block::item`), and what the mouse carries
//! while the inventory screen is open. In creative, placing takes nothing and the screen has
//! every item; in survival, blocks are placed out of it and broken ones drop into the world.

use crate::block::{self, Block};

pub const HOTBAR: usize = 9;
pub const SLOTS: usize = 36;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum GameMode {
    Survival,
    #[default]
    Creative,
}

impl GameMode {
    pub fn name(self) -> &'static str {
        match self {
            GameMode::Survival => "Survival",
            GameMode::Creative => "Creative",
        }
    }
    /// `/gamemode` takes Minecraft's names, letters and numbers.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "survival" | "s" | "0" => Some(GameMode::Survival),
            "creative" | "c" | "1" => Some(GameMode::Creative),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stack {
    pub item: Block,
    pub count: u8,
    /// Durability used up (tools; 0 otherwise). Tools hold one per slot and never merge.
    pub wear: u16,
}

impl Stack {
    /// `count` of `item`, at most what a slot holds of it, unworn.
    pub fn new(item: Block, count: u8) -> Self {
        let item = block::item(item);
        Self {
            item,
            count: count.min(block::max_stack(item)),
            wear: 0,
        }
    }
    /// The same stack with `count` of it.
    pub fn with_count(self, count: u8) -> Self {
        Self { count, ..self }
    }
    /// Uses of a tool left (0 for what is not one).
    pub fn uses_left(&self) -> u16 {
        crate::items::durability(self.item).saturating_sub(self.wear)
    }
    /// Whether `other` can join this stack: the same item, and one that stacks.
    pub fn merges(&self, other: &Stack) -> bool {
        self.item == other.item && self.max() > 1 && self.wear == other.wear
    }
    pub fn max(&self) -> u8 {
        block::max_stack(self.item)
    }
}

#[derive(Clone, Debug)]
pub struct Inventory {
    pub slots: [Option<Stack>; SLOTS],
    /// The hotbar slot in hand, 0..9.
    pub selected: usize,
    /// What the mouse holds on the inventory screen.
    pub cursor: Option<Stack>,
    /// The left hand (F swaps it with the one in hand; right click falls back to it).
    pub offhand: Option<Stack>,
    pub mode: GameMode,
    /// The crafting grid's cells (3 × 3, row by row; the 2 × 2 grid is its top left corner).
    /// Not saved: what is left in it goes back into the bag when the screen closes.
    pub craft: [Option<Stack>; CRAFT_CELLS],
    /// 2 (the inventory screen's grid) or 3 (a crafting table's).
    pub craft_size: usize,
}

impl Default for Inventory {
    fn default() -> Self {
        let mut inv = Self {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
            offhand: None,
            mode: GameMode::Creative,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        };
        for (i, &b) in block::STARTER.iter().enumerate() {
            inv.slots[i] = Some(Stack::new(b, 64));
        }
        inv
    }
}

/// A hand. A right click uses the main one; if it cannot use what it holds (or holds nothing),
/// the other one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Hand {
    Main,
    Off,
}

impl Hand {
    pub const ORDER: [Hand; 2] = [Hand::Main, Hand::Off];
}

/// The index of the left hand among the slots clicked on the screen (after the 36 of the bag).
pub const OFFHAND: usize = SLOTS;
/// The crafting grid's cells among the clicked slots (after the left hand), and its result.
pub const CRAFT: usize = SLOTS + 1;
pub const CRAFT_CELLS: usize = 9;
pub const RESULT: usize = CRAFT + CRAFT_CELLS;

/// A left click between what the mouse holds and a slot: picks the stack up, puts what the
/// mouse holds down, merges the two or swaps them.
pub fn click_slot(cursor: &mut Option<Stack>, slot: &mut Option<Stack>) {
    match (*cursor, *slot) {
        (None, s) => {
            *cursor = s;
            *slot = None;
        }
        (Some(c), None) => {
            *slot = Some(c);
            *cursor = None;
        }
        (Some(mut c), Some(mut s)) if c.merges(&s) => {
            let n = c.count.min(s.max().saturating_sub(s.count));
            s.count += n;
            c.count -= n;
            *slot = Some(s);
            *cursor = (c.count > 0).then_some(c);
        }
        (Some(c), Some(s)) => {
            *slot = Some(c);
            *cursor = Some(s);
        }
    }
}

/// A right click: half of the stack up (the larger half), or one of what the mouse holds down.
pub fn right_click_slot(cursor: &mut Option<Stack>, slot: &mut Option<Stack>) {
    match (*cursor, *slot) {
        (None, Some(s)) => {
            let half = s.count.div_ceil(2);
            *cursor = Some(s.with_count(half));
            *slot = (s.count > half).then(|| s.with_count(s.count - half));
        }
        (Some(mut c), s) if s.is_none_or(|s| s.merges(&c) && s.count < s.max()) => {
            let mut s = s.unwrap_or(c.with_count(0));
            s.count += 1;
            c.count -= 1;
            *slot = Some(s);
            *cursor = (c.count > 0).then_some(c);
        }
        _ => click_slot(cursor, slot),
    }
}

impl Inventory {
    /// What is in slot `i` (`OFFHAND` is the left hand).
    pub fn at(&self, i: usize) -> Option<Stack> {
        match i {
            OFFHAND => self.offhand,
            CRAFT..RESULT => self.craft[i - CRAFT],
            RESULT => self.craft_result(),
            _ => self.slots[i],
        }
    }

    /// Puts a stack in slot `i` (the result cannot be put into).
    pub fn put(&mut self, i: usize, s: Option<Stack>) {
        match i {
            OFFHAND => self.offhand = s,
            CRAFT..RESULT => self.craft[i - CRAFT] = s,
            RESULT => {}
            _ => self.slots[i] = s,
        }
    }

    /// What the crafting grid makes now.
    pub fn craft_result(&self) -> Option<Stack> {
        let n = self.craft_size;
        let cells: Vec<Option<Block>> = (0..n * n)
            .map(|k| self.craft[k / n * 3 + k % n].map(|s| s.item))
            .collect();
        crate::crafting::find(&cells, n).map(|r| Stack::new(r.result, r.count))
    }

    /// Takes the result into the hand (onto what is there if it is the same), using up one of each
    /// ingredient: whether it did.
    fn craft_into_cursor(&mut self) -> bool {
        let Some(r) = self.craft_result() else {
            return false;
        };
        match self.cursor {
            None => self.cursor = Some(r),
            Some(c) if c.merges(&r) && c.count as u32 + r.count as u32 <= c.max() as u32 => {
                self.cursor = Some(c.with_count(c.count + r.count))
            }
            _ => return false,
        }
        self.use_ingredients();
        true
    }

    fn use_ingredients(&mut self) {
        for c in self.craft.iter_mut() {
            if let Some(s) = c {
                s.count -= 1;
                if s.count == 0 {
                    *c = None;
                }
            }
        }
    }

    /// Room for `count` more of `item` in the bag.
    fn room_for(&self, item: Block, count: u8) -> bool {
        let max = block::max_stack(item) as u32;
        let free: u32 = self
            .slots
            .iter()
            .map(|s| match s {
                None => max,
                Some(s) if s.item == item && max > 1 => max - s.count as u32,
                _ => 0,
            })
            .sum();
        free >= count as u32
    }

    /// Closing the screen: what is on the crafting grid goes back into the bag; what did not fit.
    pub fn return_craft(&mut self) -> Vec<Stack> {
        let cells = std::mem::take(&mut self.craft);
        self.craft_size = 2;
        cells
            .into_iter()
            .flatten()
            .filter_map(|s| self.add_stack(s))
            .collect()
    }

    /// What is in `hand`.
    pub fn in_hand(&self, hand: Hand) -> Option<Stack> {
        match hand {
            Hand::Main => self.held(),
            Hand::Off => self.offhand,
        }
    }

    /// Uses one of what is in `hand` (placing it).
    pub fn use_hand(&mut self, hand: Hand) {
        match hand {
            Hand::Main => self.use_held(),
            Hand::Off => self.use_offhand(),
        }
    }

    /// Eats the food in `hand` if there is any and `health` is not full: it heals and one of it
    /// is used up (none in creative). Whether it ate; when it cannot, the click goes on to the
    /// other hand.
    pub fn eat(&mut self, hand: Hand, health: &mut crate::health::Health) -> bool {
        let Some(heal) = self.in_hand(hand).and_then(|s| crate::items::food(s.item)) else {
            return false;
        };
        if health.is_dead() || health.hp >= health.max {
            return false;
        }
        health.heal(heal);
        self.use_hand(hand);
        true
    }

    /// F: the hands trade what they hold.
    pub fn swap_hands(&mut self) {
        std::mem::swap(&mut self.slots[self.selected], &mut self.offhand);
    }

    /// Uses one of what is in the left hand (placing it): none in creative.
    pub fn use_offhand(&mut self) {
        if self.mode == GameMode::Survival {
            if let Some(s) = self.offhand.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    self.offhand = None;
                }
            }
        }
    }

    /// What is in hand.
    pub fn held(&self) -> Option<Stack> {
        self.slots[self.selected]
    }

    /// Puts `count` of `item` in: onto stacks of it first (hotbar first), then into empty
    /// slots. What did not fit.
    pub fn add(&mut self, item: Block, count: u8) -> u8 {
        let item = block::item(item);
        let mut left = count;
        for s in self.slots.iter_mut().flatten() {
            if left == 0 {
                break;
            }
            if s.item == item && s.count < s.max() {
                let n = left.min(s.max() - s.count);
                s.count += n;
                left -= n;
            }
        }
        for s in self.slots.iter_mut() {
            if left == 0 {
                break;
            }
            if s.is_none() {
                let n = left.min(block::max_stack(item));
                *s = Some(Stack::new(item, n));
                left -= n;
            }
        }
        left
    }

    /// Like `add`, for a stack as it is (a worn tool stays worn): what did not fit.
    pub fn add_stack(&mut self, s: Stack) -> Option<Stack> {
        if s.wear == 0 {
            let left = self.add(s.item, s.count);
            return (left > 0).then(|| s.with_count(left));
        }
        match self.slots.iter_mut().find(|t| t.is_none()) {
            Some(t) => {
                *t = Some(s);
                None
            }
            None => Some(s),
        }
    }

    /// Room for at least one more of `item`.
    pub fn fits(&self, item: Block) -> bool {
        let item = block::item(item);
        self.slots
            .iter()
            .any(|s| s.is_none_or(|s| s.item == item && s.count < s.max()))
    }

    /// Takes up to `n` from slot `i`: what was taken.
    pub fn take(&mut self, i: usize, n: u8) -> Option<Stack> {
        let mut s = self.at(i)?;
        let k = n.min(s.count);
        s.count -= k;
        let taken = s.with_count(k);
        self.put(i, (s.count > 0).then_some(s));
        (k > 0).then_some(taken)
    }

    /// A block broken with the tool in hand: it wears one use, and goes when none are left
    /// (true). Creative never wears it.
    pub fn wear_held(&mut self) -> bool {
        if self.mode != GameMode::Survival {
            return false;
        }
        let Some(s) = self.slots[self.selected]
            .as_mut()
            .filter(|s| crate::items::is_tool(s.item))
        else {
            return false;
        };
        s.wear += 1;
        if s.wear >= crate::items::durability(s.item) {
            self.slots[self.selected] = None;
            return true;
        }
        false
    }

    /// Uses one of what is in hand (placing it): none in creative.
    pub fn use_held(&mut self) {
        if self.mode == GameMode::Survival {
            self.take(self.selected, 1);
        }
    }

    /// Middle click (creative) or picking a block you look at: a slot with it comes into hand,
    /// from the hotbar or from the rest; in creative, a full stack of it if there is none.
    pub fn pick(&mut self, b: Block) {
        let item = block::item(b);
        if let Some(i) = (0..HOTBAR).find(|&i| self.slots[i].is_some_and(|s| s.item == item)) {
            self.selected = i;
            return;
        }
        if let Some(i) = (HOTBAR..SLOTS).find(|&i| self.slots[i].is_some_and(|s| s.item == item)) {
            self.slots.swap(i, self.selected);
            return;
        }
        if self.mode == GameMode::Creative {
            let to = (0..HOTBAR)
                .find(|&i| self.slots[i].is_none())
                .unwrap_or(self.selected);
            self.slots[to] = Some(Stack::new(item, 64));
            self.selected = to;
        }
    }

    /// A left click on slot `i` of the screen: picks the stack up, puts what the mouse holds
    /// down, merges the two or swaps them. On the result: takes what the grid makes.
    pub fn click(&mut self, i: usize) {
        if i == RESULT {
            self.craft_into_cursor();
            return;
        }
        let (mut slot, mut cursor) = (self.at(i), self.cursor);
        click_slot(&mut cursor, &mut slot);
        self.cursor = cursor;
        self.put(i, slot);
    }

    /// A right click: half of the stack up (the larger half), or one of what the mouse holds down.
    pub fn right_click(&mut self, i: usize) {
        if i == RESULT {
            self.craft_into_cursor();
            return;
        }
        let (mut slot, mut cursor) = (self.at(i), self.cursor);
        right_click_slot(&mut cursor, &mut slot);
        self.cursor = cursor;
        self.put(i, slot);
    }

    /// Shift + click: the stack goes over to the hotbar from the rest, or the other way.
    pub fn quick_move(&mut self, i: usize) {
        if i == RESULT {
            // As many as the bag has room for, using the grid up.
            for _ in 0..64 {
                let Some(r) = self
                    .craft_result()
                    .filter(|r| self.room_for(r.item, r.count))
                else {
                    break;
                };
                self.add_stack(r);
                self.use_ingredients();
            }
            return;
        }
        let Some(s) = self.at(i) else { return };
        let range = if i >= OFFHAND {
            0..SLOTS
        } else if i < HOTBAR {
            HOTBAR..SLOTS
        } else {
            0..HOTBAR
        };
        let mut left = s.count;
        for j in range.clone() {
            if let Some(t) = self.slots[j]
                .as_mut()
                .filter(|t| t.merges(&s) && t.count < t.max())
            {
                let n = left.min(t.max() - t.count);
                t.count += n;
                left -= n;
            }
        }
        for j in range {
            if left == 0 {
                break;
            }
            if self.slots[j].is_none() {
                self.slots[j] = Some(s.with_count(left));
                left = 0;
            }
        }
        self.put(i, (left > 0).then(|| s.with_count(left)));
    }

    /// A number key over slot `i`: it swaps with that hotbar slot.
    pub fn swap_hotbar(&mut self, i: usize, hot: usize) {
        if hot < HOTBAR && i != RESULT {
            let (a, b) = (self.at(i), self.at(hot));
            self.put(i, b);
            self.put(hot, a);
        }
    }

    /// The mouse's stack back into the slots (closing the screen): what did not fit.
    pub fn put_back_cursor(&mut self) -> Option<Stack> {
        let c = self.cursor.take()?;
        self.add_stack(c)
    }

    /// Saved: b"OBINV003", u8 mode, u8 selected, then 36 + 1 (the left hand) × (u8 item, u8 count,
    /// u16 wear, little-endian) (count 0: empty).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = b"OBINV003".to_vec();
        v.push(match self.mode {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
        });
        v.push(self.selected as u8);
        for s in self.slots.iter().chain([&self.offhand]) {
            let s = s.unwrap_or(Stack {
                item: 0,
                count: 0,
                wear: 0,
            });
            v.extend([s.item, s.count]);
            v.extend(s.wear.to_le_bytes());
        }
        v
    }

    pub fn from_bytes(d: &[u8]) -> Result<Self, String> {
        if d.len() != 10 + (SLOTS + 1) * 4 || &d[..8] != b"OBINV003" {
            return Err("not an inventory".into());
        }
        let mode = if d[8] == 0 {
            GameMode::Survival
        } else {
            GameMode::Creative
        };
        let mut all = [None; SLOTS + 1];
        for (i, s) in all.iter_mut().enumerate() {
            let at = 10 + i * 4;
            let (item, count, wear) =
                (d[at], d[at + 1], u16::from_le_bytes([d[at + 2], d[at + 3]]));
            if count > 0 && crate::items::valid(item) {
                let mut st = Stack::new(item, count);
                st.wear = wear.min(crate::items::durability(item));
                *s = Some(st);
            }
        }
        let offhand = all[OFFHAND];
        let mut slots = [None; SLOTS];
        slots.copy_from_slice(&all[..SLOTS]);
        Ok(Self {
            slots,
            selected: (d[9] as usize).min(HOTBAR - 1),
            cursor: None,
            offhand,
            mode,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_fills_stacks_then_slots() {
        let mut inv = Inventory {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
            offhand: None,
            mode: GameMode::Survival,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        };
        assert_eq!(inv.add(block::STONE, 100), 0);
        assert_eq!(inv.slots[0], Some(Stack::new(block::STONE, 64)));
        assert_eq!(inv.slots[1], Some(Stack::new(block::STONE, 36)));
        // A wall torch picked up is a torch.
        inv.add(block::wall_torch(block::Dir(2)), 1);
        assert_eq!(inv.slots[2], Some(Stack::new(block::TORCH, 1)));
        inv.use_held();
        assert_eq!(inv.held().unwrap().count, 63);
        let back = Inventory::from_bytes(&inv.to_bytes()).unwrap();
        assert_eq!(back.slots, inv.slots);
        assert_eq!(back.mode, GameMode::Survival);
    }

    #[test]
    fn the_left_hand_swaps_saves_and_clicks_like_a_slot() {
        let mut inv = Inventory {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
            offhand: None,
            mode: GameMode::Survival,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        };
        inv.add(block::STONE, 10);
        inv.offhand = Some(Stack::new(block::TORCH, 5));
        inv.swap_hands();
        assert_eq!(inv.held(), Some(Stack::new(block::TORCH, 5)));
        assert_eq!(inv.offhand, Some(Stack::new(block::STONE, 10)));
        inv.use_offhand();
        assert_eq!(inv.offhand.unwrap().count, 9);
        let back = Inventory::from_bytes(&inv.to_bytes()).unwrap();
        assert_eq!(back.offhand, inv.offhand);
        assert_eq!(back.slots, inv.slots);
        // The screen's click on it picks up and puts down; a number key swaps with the hotbar.
        inv.click(OFFHAND);
        assert!(inv.offhand.is_none() && inv.cursor.unwrap().count == 9);
        inv.click(OFFHAND);
        assert!(inv.cursor.is_none() && inv.offhand.unwrap().count == 9);
        inv.swap_hotbar(OFFHAND, 0);
        assert_eq!(inv.offhand, Some(Stack::new(block::TORCH, 5)));
        // Creative never uses the stack up.
        inv.mode = GameMode::Creative;
        inv.use_offhand();
        assert_eq!(inv.offhand.unwrap().count, 5);
    }

    fn survival() -> Inventory {
        Inventory {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
            offhand: None,
            mode: GameMode::Survival,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        }
    }

    #[test]
    fn saves_round_trip_with_wear_and_items() {
        use crate::items;
        let mut inv = survival();
        inv.slots[0] = Some(Stack {
            item: items::IRON_PICKAXE,
            count: 1,
            wear: 123,
        });
        inv.slots[3] = Some(Stack::new(items::COAL, 40));
        inv.slots[9] = Some(Stack::new(block::IRON_ORE, 7));
        inv.offhand = Some(Stack {
            item: items::WOODEN_SWORD,
            count: 1,
            wear: 59,
        });
        let bytes = inv.to_bytes();
        assert_eq!(&bytes[..8], b"OBINV003");
        let back = Inventory::from_bytes(&bytes).unwrap();
        assert_eq!(back.slots, inv.slots);
        assert_eq!(back.offhand, inv.offhand);
        assert_eq!(back.slots[0].unwrap().uses_left(), 127);
        // Old saves and ids that exist nowhere are refused or dropped.
        let mut old = bytes.clone();
        old[..8].copy_from_slice(b"OBINV002");
        assert!(Inventory::from_bytes(&old).is_err());
        let mut bad = bytes;
        bad[10] = 250;
        assert!(Inventory::from_bytes(&bad).unwrap().slots[0].is_none());
    }

    #[test]
    fn tools_never_stack_or_merge() {
        use crate::items;
        let mut inv = survival();
        assert_eq!(inv.add(items::STONE_PICKAXE, 1), 0);
        assert_eq!(inv.add(items::STONE_PICKAXE, 2), 0);
        assert_eq!(
            inv.slots[..3]
                .iter()
                .flatten()
                .map(|s| s.count)
                .collect::<Vec<_>>(),
            [1, 1, 1]
        );
        assert_eq!(Stack::new(items::IRON_SWORD, 9).count, 1);
        // Clicking one onto another swaps; wear travels with the tool.
        inv.slots[0].as_mut().unwrap().wear = 10;
        inv.click(0);
        inv.click(1);
        assert_eq!(inv.slots[1].unwrap().wear, 10);
        assert_eq!(inv.cursor.unwrap().wear, 0);
        inv.click(1);
        inv.click(0);
        assert_eq!(inv.slots[0].unwrap().wear, 10);
        assert_eq!(inv.slots[1].unwrap().wear, 0);
        // Shift-click and closing the screen keep wear too.
        inv.slots[1].as_mut().unwrap().wear = 5;
        inv.quick_move(1);
        assert!(inv.slots.iter().flatten().any(|s| s.wear == 5));
        inv.cursor = Some(Stack {
            item: items::IRON_PICKAXE,
            count: 1,
            wear: 77,
        });
        assert!(inv.put_back_cursor().is_none());
        assert!(
            inv.slots
                .iter()
                .flatten()
                .any(|s| s.item == items::IRON_PICKAXE && s.wear == 77)
        );
        // Materials stack like blocks.
        inv.add(items::COAL, 70);
        assert_eq!(
            inv.slots
                .iter()
                .flatten()
                .filter(|s| s.item == items::COAL)
                .map(|s| s.count as u32)
                .sum::<u32>(),
            70
        );
    }

    #[test]
    fn food_heals_four_and_is_used_up_only_when_hurt() {
        use crate::health::Health;
        use crate::items;
        let mut inv = survival();
        inv.slots[0] = Some(Stack::new(items::APPLE, 3));
        inv.offhand = Some(Stack::new(block::TORCH, 2));
        let mut hp = Health::default();
        // Full health: not eaten, nothing used up.
        assert!(!inv.eat(Hand::Main, &mut hp) && inv.held().unwrap().count == 3);
        hp.hp = 10.0;
        assert!(inv.eat(Hand::Main, &mut hp));
        assert_eq!((hp.hp, inv.held().unwrap().count), (14.0, 2));
        // Not near full: heals only to the maximum; the left hand holds no food.
        hp.hp = 18.0;
        assert!(inv.eat(Hand::Main, &mut hp) && hp.hp == hp.max);
        assert!(!inv.eat(Hand::Off, &mut hp) && inv.offhand.unwrap().count == 2);
        // The last one empties the slot; creative does not use it up.
        hp.hp = 1.0;
        inv.slots[0].as_mut().unwrap().count = 1;
        assert!(inv.eat(Hand::Main, &mut hp) && inv.held().is_none());
        inv.mode = GameMode::Creative;
        inv.slots[0] = Some(Stack::new(items::APPLE, 1));
        hp.hp = 1.0;
        assert!(inv.eat(Hand::Main, &mut hp) && inv.held().unwrap().count == 1);
        // Tools are not food.
        inv.slots[0] = Some(Stack::new(items::IRON_PICKAXE, 1));
        assert!(!inv.eat(
            Hand::Main,
            &mut Health {
                hp: 1.0,
                ..Health::default()
            }
        ));
    }

    #[test]
    fn tools_wear_out_in_survival_only() {
        use crate::items;
        let mut inv = survival();
        inv.slots[0] = Some(Stack::new(items::WOODEN_PICKAXE, 1));
        for _ in 0..59 {
            assert!(!inv.wear_held());
        }
        assert_eq!(inv.held().unwrap().uses_left(), 1);
        assert!(inv.wear_held() && inv.held().is_none());
        inv.slots[0] = Some(Stack::new(block::STONE, 3));
        assert!(!inv.wear_held() && inv.held().unwrap().wear == 0);
        inv.mode = GameMode::Creative;
        inv.slots[0] = Some(Stack::new(items::WOODEN_PICKAXE, 1));
        inv.wear_held();
        assert_eq!(inv.held().unwrap().wear, 0);
    }

    #[test]
    fn crafting_through_the_grid_slots() {
        use crate::items;
        let mut inv = survival();
        inv.add(block::WOOD, 3);
        inv.add(block::PLANKS, 9);
        // A log into a cell of the 2 × 2 grid: 4 planks show in the result.
        inv.click(0);
        inv.click(CRAFT + 1);
        assert_eq!(inv.at(RESULT), Some(Stack::new(block::PLANKS, 4)));
        assert_eq!(inv.at(CRAFT + 1).unwrap().count, 3);
        // Taking it uses one log up; taking again onto the same stack merges.
        inv.click(RESULT);
        assert_eq!(inv.cursor, Some(Stack::new(block::PLANKS, 4)));
        assert_eq!(inv.at(CRAFT + 1).unwrap().count, 2);
        inv.click(RESULT);
        assert_eq!(inv.cursor.unwrap().count, 8);
        // Something else in hand blocks it; the result cannot be put into.
        inv.click(5);
        inv.cursor = Some(Stack::new(block::DIRT, 1));
        inv.click(RESULT);
        assert_eq!(inv.cursor, Some(Stack::new(block::DIRT, 1)));
        inv.click(RESULT);
        inv.cursor = None;
        // Shift on the result makes all it can: the last log, 4 planks.
        let before: u32 = inv
            .slots
            .iter()
            .flatten()
            .filter(|s| s.item == block::PLANKS)
            .map(|s| s.count as u32)
            .sum();
        inv.quick_move(RESULT);
        let after: u32 = inv
            .slots
            .iter()
            .flatten()
            .filter(|s| s.item == block::PLANKS)
            .map(|s| s.count as u32)
            .sum();
        assert_eq!(after - before, 4);
        assert!(inv.at(CRAFT + 1).is_none() && inv.at(RESULT).is_none());
        // A pickaxe is not made in the 2 × 2 grid, with all it needs laid out; it is in the 3 × 3.
        inv.craft = [None; CRAFT_CELLS];
        for (k, item) in [
            (0, block::PLANKS),
            (1, block::PLANKS),
            (2, block::PLANKS),
            (4, items::STICK),
            (7, items::STICK),
        ] {
            inv.craft[k] = Some(Stack::new(item, 1));
        }
        assert_eq!(inv.at(RESULT), None);
        inv.craft_size = 3;
        assert_eq!(inv.at(RESULT), Some(Stack::new(items::WOODEN_PICKAXE, 1)));
        inv.click(RESULT);
        assert_eq!(inv.cursor.unwrap().item, items::WOODEN_PICKAXE);
        assert!(inv.craft.iter().all(|c| c.is_none()));
        // What is left on the grid when the screen closes goes back into the bag.
        inv.craft[8] = Some(Stack::new(block::DIRT, 5));
        inv.craft_size = 3;
        assert!(inv.return_craft().is_empty());
        assert_eq!(inv.craft_size, 2);
        assert!(
            inv.slots
                .iter()
                .flatten()
                .any(|s| s.item == block::DIRT && s.count == 5)
        );
    }

    #[test]
    fn clicks_move_stacks() {
        let mut inv = Inventory {
            slots: [None; SLOTS],
            selected: 0,
            cursor: None,
            offhand: None,
            mode: GameMode::Survival,
            craft: [None; CRAFT_CELLS],
            craft_size: 2,
        };
        inv.add(block::DIRT, 10);
        inv.right_click(0);
        assert_eq!(inv.cursor.unwrap().count, 5);
        inv.right_click(4);
        assert_eq!(inv.slots[4].unwrap().count, 1);
        inv.click(0);
        assert_eq!(inv.slots[0].unwrap().count, 9);
        assert!(inv.cursor.is_none());
        inv.quick_move(0);
        assert!(inv.slots[0].is_none() && inv.slots[HOTBAR].unwrap().count == 9);
    }
}
