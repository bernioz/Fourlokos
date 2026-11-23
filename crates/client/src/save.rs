//! Worlds and autosave. Each world has a folder in `saves/`, named after it; the universe clock
//! and every station (blocks, name, orbit, packed or placed) are kept there every `EVERY` seconds
//! and when the game closes, and are loaded when the world starts. The rest of the world comes
//! from its seed and generation settings. The start menu begins the fixed solar system
//! (`StartWorld`, in `saves/fourlokos-<seed>/`); without it (`ORBIT_TITLE=0`) the world of the seed
//! given on the command line starts at once, in `saves/world-<seed>/` (folders of that name from
//! before worlds had names are listed as "World <seed>"). Demos (`ORBIT_DEMO`) neither load nor save, unless
//! `ORBIT_SAVE=<dir>` names a folder to use instead (for trying it).
//!
//! - `world.cfg`: name, seed, game mode, dates and generation (`orbit_core::world_file`)
//! - `world.bin`: b"OBWORLD1", f64 universe clock (seconds)
//! - `stations/<id>.obst`: one per station (`orbit_core::station::Station::to_bytes`)
//! - `player.bin`: the inventory and game mode (`orbit_core::inventory::Inventory::to_bytes`)
//! - `elevators/<id>.obel`: one per space elevator, by its station's id
//!   (`orbit_core::elevator::Elevator::to_bytes`)

use crate::chat::Chat;
use crate::elevator::{Elevators, Slot as ElevatorSlot};
use crate::station::Stations;
use crate::world::{Clock, TimeShift};
use bevy::prelude::*;
use orbit_core::elevator::Elevator;
use orbit_core::station::Station;
use orbit_core::world_file::{self, WorldInfo};
use std::path::{Path, PathBuf};

/// Where the worlds are: `saves/`, or `ORBIT_SAVES=<dir>` (for trying things out).
pub fn saves() -> PathBuf {
    std::env::var("ORBIT_SAVES").map_or_else(|_| PathBuf::from("saves"), PathBuf::from)
}

/// Seconds between autosaves.
const EVERY: f32 = 20.0;
const STATION_EXT: &str = "obst";
const ELEVATOR_EXT: &str = "obel";

pub struct SavePlugin;

impl Plugin for SavePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StartWorld>()
            .add_systems(
                PreUpdate,
                (
                    start_world,
                    (load, load_player)
                        .chain()
                        .run_if(resource_changed::<SaveDir>),
                )
                    .chain(),
            )
            .add_systems(Update, autosave)
            .add_systems(Last, save_on_exit);
    }
}

/// Where this world is saved; None: not saved (demos).
#[derive(Resource)]
pub struct SaveDir(pub Option<PathBuf>);

impl SaveDir {
    pub fn for_seed(seed: u64) -> Self {
        if let Ok(dir) = std::env::var("ORBIT_SAVE") {
            return Self(Some(dir.into()));
        }
        let demo = std::env::var("ORBIT_DEMO").is_ok();
        Self((!demo).then(|| saves().join(format!("world-{seed}"))))
    }
}

/// A world on disk.
#[derive(Clone, Debug)]
pub struct SavedWorld {
    pub dir: PathBuf,
    pub info: WorldInfo,
}

pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

pub fn write_info(dir: &Path, info: &WorldInfo) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_atomic(&dir.join(world_file::FILE), info.to_text().as_bytes())
}

/// Starts this world: the universe is made from it and the player set down on its home planet.
#[derive(Message, Clone, Debug)]
pub struct StartWorld(pub SavedWorld);

/// The universe clock a save was left at.
fn saved_shift(dir: &Path) -> Option<f64> {
    let data = std::fs::read(dir.join("world.bin")).ok()?;
    (data.len() == 16 && &data[..8] == b"OBWORLD1")
        .then(|| f64::from_le_bytes(data[8..16].try_into().unwrap()))
}

#[allow(clippy::too_many_arguments)]
fn start_world(
    mut starts: MessageReader<StartWorld>,
    mut commands: Commands,
    mut space: ResMut<crate::world::Space>,
    mut live: ResMut<crate::world::WorldLive>,
    mut shift: ResMut<TimeShift>,
    time: Res<Time>,
    mut player: Single<&mut crate::player::Player>,
    role: Res<crate::netplay::NetRole>,
) {
    let Some(StartWorld(world)) = starts.read().last().cloned() else {
        return;
    };
    let mut info = world.info.clone();
    info.played = now();
    let ephemeral = matches!(&*role, crate::netplay::NetRole::Client(_));
    if std::env::var("ORBIT_DEMO").is_err() && !ephemeral {
        if let Err(e) = write_info(&world.dir, &info) {
            warn!("save: could not write {}: {e}", world.dir.display());
        }
    }
    space.despawn_all(&mut commands);
    *space = crate::world::Space::with(info.seed, info.make.clone());
    commands.insert_resource(crate::sky::SkySeed(info.seed));
    // The planets have gone on round their stars since the clock started: the spawn point is
    // where it is now.
    shift.0 = saved_shift(&world.dir).unwrap_or(0.0);
    let t = shift.0 + time.elapsed_secs_f64();
    let (mut home, at) = space.universe.spawn();
    let local = home.to_local(at);
    home.advance(t);
    let at = home.to_world(local);
    player.body = orbit_core::player::Body::new(at, (at - home.center).as_vec3());
    player.inv = orbit_core::inventory::Inventory {
        mode: info.mode,
        ..default()
    };
    // Survival starts empty-handed: the creative hotbar's blocks are only for creative.
    if info.mode == orbit_core::inventory::GameMode::Survival {
        player.inv.slots = [None; orbit_core::inventory::SLOTS];
    }
    info!(
        "save: starting world {:?} (seed {}) from {}",
        info.name,
        info.seed,
        world.dir.display()
    );
    commands.insert_resource(SaveDir((!ephemeral).then_some(world.dir)));
    live.0 = true;
}

fn load(
    dir: Res<SaveDir>,
    mut stations: ResMut<Stations>,
    mut elevators: ResMut<Elevators>,
    mut shift: ResMut<TimeShift>,
    mut chat: ResMut<Chat>,
) {
    let Some(dir) = &dir.0 else { return };
    if let Ok(data) = std::fs::read(dir.join("world.bin")) {
        if data.len() == 16 && &data[..8] == b"OBWORLD1" {
            shift.0 = f64::from_le_bytes(data[8..16].try_into().unwrap());
        }
    }
    let mut n = 0;
    let entries = std::fs::read_dir(dir.join("stations"))
        .into_iter()
        .flatten();
    for path in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == STATION_EXT))
    {
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| Station::from_bytes(&d))
        {
            Ok(s) if stations.find(&s.id).is_none() => {
                stations.push(s);
                n += 1;
            }
            Ok(_) => {}
            Err(e) => {
                warn!("save: could not load {}: {e}", path.display());
                chat.error(&format!("Could not load {}: {e}", path.display()));
            }
        }
    }
    if n > 0 {
        info!("save: loaded {n} stations from {}", dir.display());
        chat.info(&format!(
            "Loaded {n} station{}",
            if n == 1 { "" } else { "s" }
        ));
    }
    // Elevators, once their stations are in.
    let entries = std::fs::read_dir(dir.join("elevators"))
        .into_iter()
        .flatten();
    for path in entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == ELEVATOR_EXT))
    {
        match std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|d| Elevator::from_bytes(&d))
        {
            Ok(e)
                if stations.find(&e.station).is_some()
                    && !elevators.list.iter().any(|s| s.e.station == e.station) =>
            {
                elevators.list.push(ElevatorSlot::new(e))
            }
            Ok(e) => warn!("save: elevator {} has no station, left out", e.station),
            Err(e) => {
                warn!("save: could not load {}: {e}", path.display());
                chat.error(&format!("Could not load {}: {e}", path.display()));
            }
        }
    }
}

/// The inventory and game mode, once the player is there.
fn load_player(dir: Res<SaveDir>, mut player: Single<&mut crate::player::Player>) {
    let Some(dir) = &dir.0 else { return };
    if let Ok(data) = std::fs::read(dir.join("player.bin")) {
        match orbit_core::inventory::Inventory::from_bytes(&data) {
            Ok(inv) => player.inv = inv,
            Err(e) => warn!("save: could not load the inventory: {e}"),
        }
    }
}

fn save_player(dir: &Path, player: &crate::player::Player) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    write_atomic(&dir.join("player.bin"), &player.inv.to_bytes())
}

fn autosave(
    dir: Res<SaveDir>,
    stations: Res<Stations>,
    elevators: Res<Elevators>,
    clock: Res<Clock>,
    time: Res<Time>,
    mut since: Local<f32>,
    player: Query<&crate::player::Player>,
) {
    *since += time.delta_secs();
    if *since < EVERY {
        return;
    }
    *since = 0.0;
    if let Some(dir) = &dir.0 {
        if let Err(e) = save(dir, &stations, &elevators, clock.0) {
            warn!("save: could not save to {}: {e}", dir.display());
        }
        if let Some(Err(e)) = player.single().ok().map(|p| save_player(dir, p)) {
            warn!("save: could not save the inventory: {e}");
        }
    }
}

/// Saves everything as the game closes (the window shut, or anything asking it to quit).
fn save_on_exit(
    mut exit: MessageReader<AppExit>,
    dir: Res<SaveDir>,
    stations: Res<Stations>,
    elevators: Res<Elevators>,
    clock: Res<Clock>,
    player: Query<&crate::player::Player>,
) {
    if exit.read().count() == 0 {
        return;
    }
    let Some(dir) = &dir.0 else { return };
    if let Some(Err(e)) = player.single().ok().map(|p| save_player(dir, p)) {
        error!("save: could not save the inventory: {e}");
    }
    match save(dir, &stations, &elevators, clock.0) {
        Ok(()) => info!("save: saved to {}", dir.display()),
        Err(e) => error!("save: could not save to {}: {e}", dir.display()),
    }
}

fn save(dir: &Path, stations: &Stations, elevators: &Elevators, t: f64) -> std::io::Result<()> {
    let sdir = dir.join("stations");
    std::fs::create_dir_all(&sdir)?;
    let mut world = b"OBWORLD1".to_vec();
    world.extend(t.to_le_bytes());
    write_atomic(&dir.join("world.bin"), &world)?;
    let files = stations.list.iter().map(|slot| {
        let s = slot.station.read().unwrap();
        (s.id.clone(), s.to_bytes())
    });
    save_all(&sdir, STATION_EXT, files)?;
    let edir = dir.join("elevators");
    std::fs::create_dir_all(&edir)?;
    save_all(
        &edir,
        ELEVATOR_EXT,
        elevators
            .list
            .iter()
            .map(|s| (s.e.station.clone(), s.e.to_bytes())),
    )
}

/// Flushes the active world before a menu transition.
pub fn save_now(
    dir: &SaveDir,
    stations: &Stations,
    elevators: &Elevators,
    clock: &Clock,
    player: Option<&crate::player::Player>,
) {
    let Some(path) = &dir.0 else { return };
    if let Some(p) = player {
        if let Err(e) = save_player(path, p) {
            warn!("save: could not save the inventory: {e}");
        }
    }
    if let Err(e) = save(path, stations, elevators, clock.0) {
        warn!("save: could not save to {}: {e}", path.display());
    }
}

/// Writes `<id>.<ext>` for each one into `dir`, and removes those of ones that are gone.
fn save_all(
    dir: &Path,
    ext: &str,
    files: impl Iterator<Item = (String, Vec<u8>)>,
) -> std::io::Result<()> {
    let mut kept = Vec::new();
    for (id, data) in files {
        let name = format!("{id}.{ext}");
        write_atomic(&dir.join(&name), &data)?;
        kept.push(name);
    }
    for e in std::fs::read_dir(dir)?.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name.ends_with(&format!(".{ext}")) && !kept.contains(&name) {
            std::fs::remove_file(e.path())?;
        }
    }
    Ok(())
}

/// Writes a file whole or not at all: a crash mid-write leaves the old one.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}
