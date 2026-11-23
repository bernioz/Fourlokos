//! Schematics: a station's blocks saved to a file, to keep and to put up again as a new station.
//!
//! The file (`.obsc`) is little-endian:
//!
//! ```text
//! b"OBSCHEM1"
//! u32 name length, the name (UTF-8)
//! i32 × 3  min: the lowest corner, relative to the station's core (0, 0, 0)
//! i32 × 3  size: blocks along x, y, z
//! u8 × (size.x · size.y · size.z)  the blocks, x fastest, then z, then y
//! ```

use crate::block::{self, Block};
use glam::IVec3;

const MAGIC: &[u8; 8] = b"OBSCHEM1";

/// File extension, without the dot.
pub const EXT: &str = "obsc";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Schematic {
    pub name: String,
    /// The lowest corner, relative to the core.
    pub min: IVec3,
    pub size: IVec3,
    pub blocks: Vec<Block>,
}

impl Schematic {
    fn index(&self, p: IVec3) -> Option<usize> {
        let l = p - self.min;
        (l.cmpge(IVec3::ZERO).all() && l.cmplt(self.size).all()).then(|| ((l.y * self.size.z + l.z) * self.size.x + l.x) as usize)
    }

    /// The block at `p` (relative to the core); air outside.
    pub fn get(&self, p: IVec3) -> Block {
        self.index(p).map_or(block::AIR, |i| self.blocks[i])
    }

    /// Every block that is not air, with where it is.
    pub fn solid_cells(&self) -> impl Iterator<Item = (IVec3, Block)> + '_ {
        let (min, s) = (self.min, self.size);
        (0..s.y)
            .flat_map(move |y| (0..s.z).flat_map(move |z| (0..s.x).map(move |x| min + IVec3::new(x, y, z))))
            .map(|p| (p, self.get(p)))
            .filter(|&(_, b)| b != block::AIR)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        out.extend((self.name.len() as u32).to_le_bytes());
        out.extend(self.name.as_bytes());
        for v in self.min.to_array().into_iter().chain(self.size.to_array()) {
            out.extend(v.to_le_bytes());
        }
        out.extend(&self.blocks);
        out
    }

    pub fn from_bytes(data: &[u8]) -> Result<Self, String> {
        let mut at = 0;
        let mut take = |n: usize| -> Result<&[u8], String> {
            let s = data.get(at..at + n).ok_or("The file is cut short")?;
            at += n;
            Ok(s)
        };
        if take(8)? != MAGIC {
            return Err("Not a schematic".into());
        }
        let len = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
        let name = String::from_utf8(take(len)?.to_vec()).map_err(|_| "Bad name")?;
        let mut v = [0i32; 6];
        for x in &mut v {
            *x = i32::from_le_bytes(take(4)?.try_into().unwrap());
        }
        let (min, size) = (IVec3::new(v[0], v[1], v[2]), IVec3::new(v[3], v[4], v[5]));
        if size.cmplt(IVec3::ONE).any() || size.cmpgt(IVec3::splat(1024)).any() {
            return Err("Bad size".into());
        }
        let blocks = take((size.x * size.y * size.z) as usize)?.to_vec();
        Ok(Self { name, min, size, blocks })
    }
}

/// A name made safe to be a file name: letters, digits, `-` and `_` (others become `_`).
pub fn file_stem(name: &str) -> String {
    let s: String = name.trim().chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c.to_ascii_lowercase() } else { '_' }).collect();
    if s.is_empty() { "schematic".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_bytes() {
        let s = Schematic { name: "Mi Base ñ".into(), min: IVec3::new(-3, -1, -2), size: IVec3::new(4, 2, 3), blocks: (0..24).collect() };
        assert_eq!(Schematic::from_bytes(&s.to_bytes()).unwrap(), s);
        assert_eq!(s.get(IVec3::new(-3, -1, -2)), 0);
        assert_eq!(s.get(IVec3::new(-2, -1, -2)), 1);
        assert_eq!(s.get(IVec3::new(-3, -1, -1)), 4);
        assert_eq!(s.get(IVec3::new(-3, 0, -2)), 12);
        assert_eq!(s.get(IVec3::new(9, 9, 9)), block::AIR);
        assert!(Schematic::from_bytes(&s.to_bytes()[..30]).is_err());
    }

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("Mi Base/../x"), "mi_base____x");
        assert_eq!(file_stem("  "), "schematic");
    }
}
