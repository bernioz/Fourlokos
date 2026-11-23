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
}

impl Stack {
    pub fn new(item: Block, count: u8) -> Self {
        Self { item: block::item(item), count }
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
    pub mode: GameMode,
}

impl Default for Inventory {
    fn default() -> Self {
        let mut inv = Self { slots: [None; SLOTS], selected: 0, cursor: None, mode: GameMode::Creative };
        for (i, &b) in block::STARTER.iter().enumerate() {
            inv.slots[i] = Some(Stack::new(b, 64));
        }
        inv
    }
}

impl Inventory {
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

    /// Room for at least one more of `item`.
    pub fn fits(&self, item: Block) -> bool {
        let item = block::item(item);
        self.slots.iter().any(|s| s.is_none_or(|s| s.item == item && s.count < s.max()))
    }

    /// Takes up to `n` from slot `i`: what was taken.
    pub fn take(&mut self, i: usize, n: u8) -> Option<Stack> {
        let s = self.slots[i].as_mut()?;
        let k = n.min(s.count);
        s.count -= k;
        let item = s.item;
        if s.count == 0 {
            self.slots[i] = None;
        }
        (k > 0).then(|| Stack::new(item, k))
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
            let to = (0..HOTBAR).find(|&i| self.slots[i].is_none()).unwrap_or(self.selected);
            self.slots[to] = Some(Stack::new(item, 64));
            self.selected = to;
        }
    }

    /// A left click on slot `i` of the screen: picks the stack up, puts what the mouse holds
    /// down, merges the two or swaps them.
    pub fn click(&mut self, i: usize) {
        match (self.cursor, self.slots[i]) {
            (None, s) => {
                self.cursor = s;
                self.slots[i] = None;
            }
            (Some(c), None) => {
                self.slots[i] = Some(c);
                self.cursor = None;
            }
            (Some(mut c), Some(mut s)) if c.item == s.item => {
                let n = c.count.min(s.max().saturating_sub(s.count));
                s.count += n;
                c.count -= n;
                self.slots[i] = Some(s);
                self.cursor = (c.count > 0).then_some(c);
            }
            (Some(c), Some(s)) => {
                self.slots[i] = Some(c);
                self.cursor = Some(s);
            }
        }
    }

    /// A right click: half of the stack up (the larger half), or one of what the mouse holds down.
    pub fn right_click(&mut self, i: usize) {
        match (self.cursor, self.slots[i]) {
            (None, Some(s)) => {
                let half = s.count.div_ceil(2);
                self.cursor = self.take(i, half);
            }
            (Some(mut c), s) if s.is_none_or(|s| s.item == c.item && s.count < s.max()) => {
                let mut s = s.unwrap_or(Stack::new(c.item, 0));
                s.count += 1;
                c.count -= 1;
                self.slots[i] = Some(s);
                self.cursor = (c.count > 0).then_some(c);
            }
            _ => self.click(i),
        }
    }

    /// Shift + click: the stack goes over to the hotbar from the rest, or the other way.
    pub fn quick_move(&mut self, i: usize) {
        let Some(s) = self.slots[i] else { return };
        let range = if i < HOTBAR { HOTBAR..SLOTS } else { 0..HOTBAR };
        let mut left = s.count;
        for j in range.clone() {
            if let Some(t) = self.slots[j].as_mut().filter(|t| t.item == s.item && t.count < t.max()) {
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
                self.slots[j] = Some(Stack::new(s.item, left));
                left = 0;
            }
        }
        self.slots[i] = (left > 0).then(|| Stack::new(s.item, left));
    }

    /// A number key over slot `i`: it swaps with that hotbar slot.
    pub fn swap_hotbar(&mut self, i: usize, hot: usize) {
        if hot < HOTBAR {
            self.slots.swap(i, hot);
        }
    }

    /// The mouse's stack back into the slots (closing the screen): what did not fit.
    pub fn put_back_cursor(&mut self) -> Option<Stack> {
        let c = self.cursor.take()?;
        let left = self.add(c.item, c.count);
        (left > 0).then(|| Stack::new(c.item, left))
    }

    /// Saved: b"OBINV001", u8 mode, u8 selected, then 36 × (u8 item, u8 count) (count 0: empty).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = b"OBINV001".to_vec();
        v.push(match self.mode {
            GameMode::Survival => 0,
            GameMode::Creative => 1,
        });
        v.push(self.selected as u8);
        for s in &self.slots {
            let s = s.unwrap_or(Stack { item: 0, count: 0 });
            v.extend([s.item, s.count]);
        }
        v
    }

    pub fn from_bytes(d: &[u8]) -> Result<Self, String> {
        if d.len() != 10 + SLOTS * 2 || &d[..8] != b"OBINV001" {
            return Err("not an inventory".into());
        }
        let mode = if d[8] == 0 { GameMode::Survival } else { GameMode::Creative };
        let mut slots = [None; SLOTS];
        for (i, s) in slots.iter_mut().enumerate() {
            let (item, count) = (d[10 + i * 2], d[11 + i * 2]);
            if count > 0 && (item as usize) < block::COUNT && item != block::AIR {
                *s = Some(Stack::new(item, count.min(block::max_stack(item))));
            }
        }
        Ok(Self { slots, selected: (d[9] as usize).min(HOTBAR - 1), cursor: None, mode })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adding_fills_stacks_then_slots() {
        let mut inv = Inventory { slots: [None; SLOTS], selected: 0, cursor: None, mode: GameMode::Survival };
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
    fn clicks_move_stacks() {
        let mut inv = Inventory { slots: [None; SLOTS], selected: 0, cursor: None, mode: GameMode::Survival };
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
