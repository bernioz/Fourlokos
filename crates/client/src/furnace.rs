//! Furnaces in the world: each one placed and opened has its state (`orbit_core::furnace`), run
//! every frame whether its screen is open or not. Breaking one drops what it holds. They are
//! saved next to the world as `furnaces.bin`.

use crate::inventory::{Drops, Frame};
use crate::save::SaveDir;
use crate::station::Stations;
use crate::world::Space;
use bevy::prelude::*;
use orbit_core::block;
use orbit_core::furnace::Furnace;
use std::collections::HashMap;

pub struct FurnacePlugin;

impl Plugin for FurnacePlugin {
    fn build(&self, app: &mut App) {
        // `ORBIT_SMELT_SPEED=<n>` (demos): time runs n times faster inside furnaces.
        let speed = std::env::var("ORBIT_SMELT_SPEED")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
            .unwrap_or(1.0);
        app.init_resource::<Furnaces>()
            .insert_resource(SmeltSpeed(speed))
            .add_systems(PreUpdate, load.run_if(resource_changed::<SaveDir>))
            .add_systems(Update, (run_furnaces, reap_furnaces).chain())
            .add_systems(Update, autosave)
            .add_systems(Last, save_on_exit);
    }
}

#[derive(Resource)]
struct SmeltSpeed(f32);

/// Where a furnace is: the planet or station, and its cell.
pub type Key = (Frame, [i64; 3]);

#[derive(Resource, Default)]
pub struct Furnaces {
    pub map: HashMap<Key, Furnace>,
}

fn run_furnaces(mut furnaces: ResMut<Furnaces>, time: Res<Time>, speed: Res<SmeltSpeed>) {
    let dt = time.delta_secs().min(0.25) * speed.0;
    for f in furnaces.map.values_mut() {
        f.tick(dt);
    }
}

/// A furnace whose block is gone (broken) drops what it held and is forgotten.
fn reap_furnaces(
    mut furnaces: ResMut<Furnaces>,
    mut drops: ResMut<Drops>,
    space: Res<Space>,
    stations: Res<Stations>,
) {
    let mut gone: Vec<(Key, Vec3, Vec3)> = Vec::new();
    for key in furnaces.map.keys() {
        let (frame, c) = key;
        match frame {
            Frame::Planet(id) => {
                let Some(v) = space.planets.get(id).and_then(|p| p.voxel.as_ref()) else {
                    continue;
                };
                let v = v.read().unwrap();
                let cell = c[0] as usize;
                if cell < v.grid.cell_count() && v.get(cell) != block::FURNACE {
                    let at = v.grid.center(cell);
                    gone.push((key.clone(), at, at.normalize()));
                }
            }
            Frame::Station(id) => {
                let Some(s) = stations.by_id(id) else {
                    continue;
                };
                let s = s.station.read().unwrap();
                let p = IVec3::new(c[0] as i32, c[1] as i32, c[2] as i32);
                if s.get(p) != block::FURNACE {
                    gone.push((
                        key.clone(),
                        orbit_core::station::Station::cell_center(p),
                        Vec3::Y,
                    ));
                }
            }
        }
    }
    for (key, at, up) in gone {
        if let Some(f) = furnaces.map.remove(&key) {
            for s in f.contents() {
                drops.pop_stack(key.0.clone(), at, up, s);
            }
        }
    }
}

// --- Saving -----------------------------------------------------------------------------------

const MAGIC: &[u8; 8] = b"OBFUR001";

/// b"OBFUR001", u32 count, then per furnace: u8 0 + u64 planet id, or u8 1 + u16 length + the
/// station's id; 3 × i64 cell; the furnace (`Furnace::BYTES`).
fn encode(map: &HashMap<Key, Furnace>) -> Vec<u8> {
    let mut v = MAGIC.to_vec();
    v.extend((map.len() as u32).to_le_bytes());
    for ((frame, c), f) in map {
        match frame {
            Frame::Planet(id) => {
                v.push(0);
                v.extend(id.to_le_bytes());
            }
            Frame::Station(id) => {
                v.push(1);
                v.extend((id.len() as u16).to_le_bytes());
                v.extend(id.as_bytes());
            }
        }
        for n in c {
            v.extend(n.to_le_bytes());
        }
        v.extend(f.to_bytes());
    }
    v
}

fn decode(d: &[u8]) -> Option<HashMap<Key, Furnace>> {
    if d.get(..8)? != MAGIC {
        return None;
    }
    let n = u32::from_le_bytes(d.get(8..12)?.try_into().ok()?) as usize;
    let mut at = 12;
    let mut take = |len: usize| -> Option<&[u8]> {
        let s = d.get(at..at + len)?;
        at += len;
        Some(s)
    };
    let mut map = HashMap::new();
    for _ in 0..n {
        let frame = match take(1)?[0] {
            0 => Frame::Planet(u64::from_le_bytes(take(8)?.try_into().ok()?)),
            1 => {
                let len = u16::from_le_bytes(take(2)?.try_into().ok()?) as usize;
                Frame::Station(String::from_utf8(take(len)?.to_vec()).ok()?)
            }
            _ => return None,
        };
        let mut c = [0i64; 3];
        for n in &mut c {
            *n = i64::from_le_bytes(take(8)?.try_into().ok()?);
        }
        map.insert((frame, c), Furnace::from_bytes(take(Furnace::BYTES)?)?);
    }
    Some(map)
}

fn load(dir: Res<SaveDir>, mut furnaces: ResMut<Furnaces>) {
    let Some(dir) = &dir.0 else { return };
    furnaces.map.clear();
    if let Ok(data) = std::fs::read(dir.join("furnaces.bin")) {
        match decode(&data) {
            Some(map) => furnaces.map = map,
            None => warn!("save: could not read the furnaces"),
        }
    }
}

fn write(dir: &SaveDir, furnaces: &Furnaces) {
    let Some(dir) = &dir.0 else { return };
    if furnaces.map.is_empty() && !dir.join("furnaces.bin").exists() {
        return;
    }
    let path = dir.join("furnaces.bin");
    let tmp = path.with_extension("tmp");
    let r = std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&tmp, encode(&furnaces.map)))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = r {
        warn!("save: could not save the furnaces: {e}");
    }
}

fn autosave(dir: Res<SaveDir>, furnaces: Res<Furnaces>, time: Res<Time>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since >= 30.0 {
        *since = 0.0;
        write(&dir, &furnaces);
    }
}

fn save_on_exit(mut exit: MessageReader<AppExit>, dir: Res<SaveDir>, furnaces: Res<Furnaces>) {
    if exit.read().count() > 0 {
        write(&dir, &furnaces);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_core::inventory::Stack;

    #[test]
    fn furnaces_save_and_load() {
        let mut map = HashMap::new();
        let mut f = Furnace::default();
        f.input = Some(Stack::new(block::IRON_ORE, 3));
        f.fuel = Some(Stack::new(orbit_core::items::COAL, 2));
        f.tick(5.0);
        map.insert((Frame::Planet(7), [1234, 0, 0]), f.clone());
        map.insert(
            (Frame::Station("Base 1".into()), [-3, 4, 5]),
            Furnace::default(),
        );
        let back = decode(&encode(&map)).unwrap();
        assert_eq!(back, map);
        assert!(decode(b"nonsense").is_none());
    }
}
