//! A furnace: one input, one fuel, one output, and the clock that smelts. Plain state with a
//! `tick(dt)`, so a host can run every furnace of a world, open or not.

use crate::block::{self, Block};
use crate::inventory::{Stack, click_slot};
use crate::items;

/// Seconds to smelt one item.
pub const SMELT_TIME: f32 = 8.0;
/// Items one fuel item smelts.
pub const ITEMS_PER_FUEL: f32 = 8.0;
/// The most the output slot holds.
pub const OUTPUT_MAX: u8 = 64;

/// What smelting `item` gives (one of).
pub fn smelts_to(item: Block) -> Option<Block> {
    match item {
        block::COPPER_ORE | block::DEEP_COPPER_ORE => Some(items::COPPER_INGOT),
        block::IRON_ORE | block::DEEP_IRON_ORE => Some(items::IRON_INGOT),
        items::SUNFLOWER_SEEDS => Some(items::OIL),
        items::RAW_CHICKEN => Some(items::VIENESA),
        _ => None,
    }
}

/// Whether `item` burns: only coal does.
pub fn is_fuel(item: Block) -> bool {
    item == items::COAL
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    Input,
    Fuel,
    Output,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Furnace {
    pub input: Option<Stack>,
    pub fuel: Option<Stack>,
    pub output: Option<Stack>,
    /// Seconds into the item being smelted, 0..`SMELT_TIME`.
    pub progress: f32,
    /// Seconds of fuel left in what was lit already.
    pub burn_left: f32,
}

impl Furnace {
    pub fn slot(&self, s: Slot) -> Option<Stack> {
        match s {
            Slot::Input => self.input,
            Slot::Fuel => self.fuel,
            Slot::Output => self.output,
        }
    }

    fn slot_mut(&mut self, s: Slot) -> &mut Option<Stack> {
        match s {
            Slot::Input => &mut self.input,
            Slot::Fuel => &mut self.fuel,
            Slot::Output => &mut self.output,
        }
    }

    /// Whether `slot` takes `item` being put in (the output takes nothing).
    pub fn accepts(slot: Slot, item: Block) -> bool {
        match slot {
            Slot::Input => smelts_to(item).is_some(),
            Slot::Fuel => is_fuel(item),
            Slot::Output => false,
        }
    }

    /// The item now being smelted, and whether it can be: the output has room for it.
    fn job(&self) -> Option<Block> {
        let to = smelts_to(self.input?.item)?;
        match self.output {
            None => Some(to),
            Some(o) if o.item == to && o.count < OUTPUT_MAX => Some(to),
            _ => None,
        }
    }

    /// Whether it is burning now.
    pub fn lit(&self) -> bool {
        self.burn_left > 0.0 && self.job().is_some()
    }

    /// How far along the item is, 0..1.
    pub fn fraction(&self) -> f32 {
        self.progress / SMELT_TIME
    }

    /// `dt` seconds go by. Smelts while there is an item to smelt, room for it and fuel to burn;
    /// stops (keeping where it was) when the output holds something else or is full, or the fuel
    /// is out; goes back to the start when the input is not smeltable.
    pub fn tick(&mut self, dt: f32) {
        let mut left = dt;
        while left > 1e-6 {
            if self.input.and_then(|s| smelts_to(s.item)).is_none() {
                self.progress = 0.0;
                return;
            }
            let Some(to) = self.job() else { return };
            if self.burn_left <= 1e-6 {
                match self.fuel.as_mut().filter(|f| is_fuel(f.item)) {
                    Some(f) => {
                        f.count -= 1;
                        if f.count == 0 {
                            self.fuel = None;
                        }
                        self.burn_left += ITEMS_PER_FUEL * SMELT_TIME;
                    }
                    None => return,
                }
            }
            let step = left.min(self.burn_left).min(SMELT_TIME - self.progress);
            self.burn_left -= step;
            self.progress += step;
            left -= step;
            if self.progress >= SMELT_TIME - 1e-6 {
                self.progress = 0.0;
                let i = self.input.as_mut().unwrap();
                i.count -= 1;
                if i.count == 0 {
                    self.input = None;
                }
                match self.output.as_mut() {
                    Some(o) => o.count += 1,
                    None => self.output = Some(Stack::new(to, 1)),
                }
            }
        }
    }

    /// A left click on one of its slots with the mouse's stack: input and fuel take what they
    /// accept; the output only gives.
    pub fn click(&mut self, slot: Slot, cursor: &mut Option<Stack>) {
        match slot {
            Slot::Output => {
                let Some(o) = self.output else { return };
                match *cursor {
                    None => {
                        *cursor = Some(o);
                        self.output = None;
                    }
                    Some(c) if c.merges(&o) => {
                        let n = o.count.min(c.max() - c.count);
                        *cursor = Some(c.with_count(c.count + n));
                        self.output = (o.count > n).then(|| o.with_count(o.count - n));
                    }
                    Some(_) => {}
                }
            }
            _ => {
                if cursor.is_some_and(|c| !Self::accepts(slot, c.item)) {
                    return;
                }
                click_slot(cursor, self.slot_mut(slot));
            }
        }
        if self.input.is_none() {
            self.progress = 0.0;
        }
    }

    /// Every item it holds (when it is broken).
    pub fn contents(&self) -> Vec<Stack> {
        [self.input, self.fuel, self.output]
            .into_iter()
            .flatten()
            .collect()
    }

    /// 3 slots × (u8 item, u8 count, u16 wear), f32 progress, f32 burn_left, little-endian.
    pub const BYTES: usize = 12 + 8;

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(Self::BYTES);
        for s in [self.input, self.fuel, self.output] {
            let s = s.unwrap_or(Stack {
                item: 0,
                count: 0,
                wear: 0,
            });
            v.extend([s.item, s.count]);
            v.extend(s.wear.to_le_bytes());
        }
        v.extend(self.progress.to_le_bytes());
        v.extend(self.burn_left.to_le_bytes());
        v
    }

    pub fn from_bytes(d: &[u8]) -> Option<Self> {
        if d.len() != Self::BYTES {
            return None;
        }
        let stack = |i: usize| {
            let (item, count) = (d[i * 4], d[i * 4 + 1]);
            (count > 0 && items::valid(item)).then(|| Stack::new(item, count))
        };
        let f = |at: usize| f32::from_le_bytes([d[at], d[at + 1], d[at + 2], d[at + 3]]);
        let (progress, burn_left) = (f(12), f(16));
        (progress.is_finite() && burn_left.is_finite()).then(|| Self {
            input: stack(0),
            fuel: stack(1),
            output: stack(2),
            progress: progress.clamp(0.0, SMELT_TIME),
            burn_left: burn_left.clamp(0.0, ITEMS_PER_FUEL * SMELT_TIME),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn loaded(ore: Block, n: u8, coal: u8) -> Furnace {
        Furnace {
            input: Some(Stack::new(ore, n)),
            fuel: (coal > 0).then(|| Stack::new(items::COAL, coal)),
            ..Default::default()
        }
    }

    #[test]
    fn smelts_one_item_in_eight_seconds_and_burns_a_coal_for_eight() {
        let mut f = loaded(block::IRON_ORE, 10, 1);
        f.tick(7.9);
        assert!(
            f.output.is_none() && f.fuel.is_none(),
            "the coal is lit as work starts"
        );
        f.tick(0.2);
        assert_eq!(f.output, Some(Stack::new(items::IRON_INGOT, 1)));
        assert_eq!(f.input.unwrap().count, 9);
        // One coal is eight items: 64 s in all; the ninth waits for fuel.
        f.tick(1000.0);
        assert_eq!(f.output.unwrap().count, 8);
        assert_eq!(f.input.unwrap().count, 2);
        assert!(f.burn_left <= 1e-3 && f.progress == 0.0);
        // More coal: it goes on.
        f.fuel = Some(Stack::new(items::COAL, 1));
        f.tick(16.0);
        assert_eq!(f.output.unwrap().count, 10);
        assert!(
            f.input.is_none() && f.fuel.is_none() && f.burn_left > 0.0,
            "left burning for later"
        );
    }

    #[test]
    fn the_time_adds_up_however_it_is_sliced() {
        let mut a = loaded(block::COPPER_ORE, 5, 1);
        let mut b = a.clone();
        a.tick(30.0);
        for _ in 0..3000 {
            b.tick(0.01);
        }
        assert_eq!(a.output.unwrap().count, 3);
        assert_eq!(b.output.unwrap().count, 3);
        assert!((a.progress - b.progress).abs() < 0.05);
        // Deep ores melt too.
        let mut d = loaded(block::DEEP_IRON_ORE, 1, 1);
        d.tick(8.0);
        assert_eq!(d.output.unwrap().item, items::IRON_INGOT);
    }

    #[test]
    fn it_pauses_without_fuel_or_with_the_output_taken_and_keeps_its_place() {
        let mut f = loaded(block::IRON_ORE, 3, 0);
        f.tick(20.0);
        assert!(f.output.is_none() && f.progress == 0.0);
        f.fuel = Some(Stack::new(items::COAL, 1));
        f.tick(4.0);
        assert!((f.progress - 4.0).abs() < 1e-3);
        // The output holds something else: pause, progress kept.
        f.output = Some(Stack::new(items::COPPER_INGOT, 1));
        f.tick(20.0);
        assert!((f.progress - 4.0).abs() < 1e-3 && f.output.unwrap().item == items::COPPER_INGOT);
        // A full output pauses too; once taken, it goes on.
        f.output = Some(Stack::new(items::IRON_INGOT, OUTPUT_MAX));
        f.tick(20.0);
        assert!((f.progress - 4.0).abs() < 1e-3 && f.input.unwrap().count == 3);
        f.output = None;
        f.tick(4.0);
        assert_eq!(f.output, Some(Stack::new(items::IRON_INGOT, 1)));
        // What cannot be smelted never starts, and nothing burns.
        let mut g = loaded(block::STONE, 4, 3);
        g.tick(100.0);
        assert!(g.output.is_none() && g.fuel.unwrap().count == 3);
    }

    #[test]
    fn slots_take_only_what_they_accept() {
        let mut f = Furnace::default();
        let mut cursor = Some(Stack::new(block::STONE, 5));
        f.click(Slot::Input, &mut cursor);
        f.click(Slot::Fuel, &mut cursor);
        assert!(f.input.is_none() && f.fuel.is_none() && cursor.unwrap().count == 5);
        cursor = Some(Stack::new(block::IRON_ORE, 5));
        f.click(Slot::Fuel, &mut cursor);
        assert!(f.fuel.is_none());
        f.click(Slot::Input, &mut cursor);
        assert_eq!(f.input, Some(Stack::new(block::IRON_ORE, 5)));
        assert!(cursor.is_none());
        cursor = Some(Stack::new(items::COAL, 2));
        f.click(Slot::Fuel, &mut cursor);
        f.tick(8.0);
        // Take the ingot; the output takes nothing in.
        cursor = Some(Stack::new(block::DIRT, 1));
        f.click(Slot::Output, &mut cursor);
        assert_eq!(cursor, Some(Stack::new(block::DIRT, 1)));
        cursor = None;
        f.click(Slot::Output, &mut cursor);
        assert_eq!(cursor, Some(Stack::new(items::IRON_INGOT, 1)));
        assert!(f.output.is_none());
        // Taking the input away resets the work.
        f.tick(3.0);
        let mut c = None;
        f.click(Slot::Input, &mut c);
        assert!(f.input.is_none() && f.progress == 0.0 && c.unwrap().count == 4);
    }

    #[test]
    fn saves_round_trip() {
        let mut f = loaded(block::COPPER_ORE, 7, 2);
        f.tick(11.0);
        let back = Furnace::from_bytes(&f.to_bytes()).unwrap();
        assert_eq!(back, f);
        assert_eq!(f.contents().len(), 3);
        assert!(Furnace::from_bytes(&[1, 2, 3]).is_none());
    }

    #[test]
    fn farm_food_smelt_results_are_defined() {
        assert_eq!(smelts_to(items::SUNFLOWER_SEEDS), Some(items::OIL));
        assert_eq!(smelts_to(items::RAW_CHICKEN), Some(items::VIENESA));
    }
}
