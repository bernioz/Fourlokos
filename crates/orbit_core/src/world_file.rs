//! A saved world's description (`world.cfg` in its folder): its name, seed, game mode and how its
//! universe is generated. Plain text, one `key = value` per line, so it can be read and fixed by
//! hand; unknown keys are skipped and missing ones keep their defaults, so older and newer files
//! load.
//!
//! ```text
//! # Fourlokos world
//! version = 1
//! name = My world
//! seed = 1337
//! mode = creative
//! created = 1763412345
//! played = 1763419000
//! single_planet = false
//! planets = 3 7
//! profile.desert.trees = 0.5
//! spawn.biome = meadow
//! spawn.relief = 1.4
//! ```

use crate::inventory::GameMode;
use crate::planet::{Biome, Customization, Gen};
use crate::universe::WorldGen;

pub const FILE: &str = "world.cfg";
const VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct WorldInfo {
    pub name: String,
    pub seed: u64,
    /// The mode a new player starts in (the player's own file keeps theirs after).
    pub mode: GameMode,
    /// Unix seconds.
    pub created: u64,
    pub played: u64,
    pub make: WorldGen,
}

impl WorldInfo {
    pub fn new(name: &str, seed: u64) -> Self {
        Self {
            name: name.into(),
            seed,
            mode: GameMode::default(),
            created: 0,
            played: 0,
            make: WorldGen::default(),
        }
    }

    pub fn to_text(&self) -> String {
        let mut out = String::from("# Fourlokos world\n");
        let mut put = |k: &str, v: String| {
            out.push_str(k);
            out.push_str(" = ");
            out.push_str(&v);
            out.push('\n');
        };
        put("version", VERSION.to_string());
        put("name", self.name.replace('\n', " "));
        put("seed", self.seed.to_string());
        put("mode", self.mode.name().to_ascii_lowercase());
        put("created", self.created.to_string());
        put("played", self.played.to_string());
        let g = &self.make;
        put("single_planet", g.single_planet.to_string());
        put("home_only", g.home_only.to_string());
        put("star_density", g.star_density.to_string());
        put("planets", format!("{} {}", g.planets.0, g.planets.1));
        put("star_size", g.star_size.to_string());
        put("orbit_speed", g.orbit_speed.to_string());
        put("day_speed", g.day_speed.to_string());
        put("max_moons", g.max_moons.to_string());
        put("moon_chance", g.moon_chance.to_string());
        put(
            "allowed",
            g.allowed
                .iter()
                .map(|&a| if a { "1" } else { "0" })
                .collect::<Vec<_>>()
                .join(" "),
        );
        let default = Gen::default();
        for (b, p) in Biome::ALL.iter().zip(&g.profiles) {
            // Only what differs from the defaults.
            let mut p = *p;
            let mut d = default;
            for ((k, v), (_, dv)) in gen_fields(&mut p).into_iter().zip(gen_fields(&mut d)) {
                if v != dv {
                    put(&format!("profile.{}.{k}", biome_key(*b)), v.to_string());
                }
            }
            for ((k, v), (_, dv)) in gen_blocks(&mut p).into_iter().zip(gen_blocks(&mut d)) {
                if v != dv {
                    put(&format!("profile.{}.{k}", biome_key(*b)), v.to_string());
                }
            }
        }
        if let Some(mut c) = g.spawn {
            put("spawn.biome", biome_key(c.biome).into());
            put("spawn.radius", c.radius.to_string());
            put("spawn.seed", c.seed.to_string());
            put("spawn.day", c.day.to_string());
            put("spawn.tilt", c.tilt.to_string());
            for (k, v) in gen_fields(&mut c.knobs) {
                put(&format!("spawn.{k}"), v.to_string());
            }
            for (k, v) in gen_blocks(&mut c.knobs) {
                put(&format!("spawn.{k}"), v.to_string());
            }
        }
        out
    }

    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut w = WorldInfo::new("", 0);
        let mut spawn: Option<Customization> = None;
        let mut has_seed = false;
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else {
                return Err(format!("line {}: no '='", n + 1));
            };
            let (k, v) = (k.trim(), v.trim());
            let bad = || format!("line {}: bad value for {k}: {v}", n + 1);
            let f = || v.parse::<f32>().map_err(|_| bad());
            let g = &mut w.make;
            match k {
                "version" => {}
                "name" => w.name = v.to_string(),
                "seed" => {
                    w.seed = v.parse().map_err(|_| bad())?;
                    has_seed = true;
                }
                "mode" => w.mode = GameMode::parse(v).ok_or_else(bad)?,
                "created" => w.created = v.parse().map_err(|_| bad())?,
                "played" => w.played = v.parse().map_err(|_| bad())?,
                "single_planet" => g.single_planet = v.parse().map_err(|_| bad())?,
                "home_only" => g.home_only = v.parse().map_err(|_| bad())?,
                "star_density" => g.star_density = f()?,
                "planets" => {
                    let mut it = v.split_whitespace().map(|x| x.parse::<u32>());
                    match (it.next(), it.next()) {
                        (Some(Ok(a)), Some(Ok(b))) => g.planets = (a, b),
                        _ => return Err(bad()),
                    }
                }
                "star_size" => g.star_size = f()?,
                "orbit_speed" => g.orbit_speed = f()?,
                "day_speed" => g.day_speed = f()?,
                "max_moons" => g.max_moons = v.parse().map_err(|_| bad())?,
                "moon_chance" => g.moon_chance = f()?,
                "allowed" => {
                    for (a, x) in g.allowed.iter_mut().zip(v.split_whitespace()) {
                        *a = x == "1";
                    }
                }
                _ => {
                    if let Some(rest) = k.strip_prefix("profile.") {
                        let Some((b, field)) = rest.split_once('.') else {
                            continue;
                        };
                        let Some(i) = Biome::ALL.iter().position(|&x| biome_key(x) == b) else {
                            continue;
                        };
                        set_gen(&mut g.profiles[i], field, v).map_err(|_| bad())?;
                    } else if let Some(field) = k.strip_prefix("spawn.") {
                        let c = spawn.get_or_insert(Customization {
                            radius: 150.0,
                            biome: Biome::Meadow,
                            seed: 0,
                            knobs: Gen::default(),
                            day: 450.0,
                            tilt: 0.0,
                            radius_range: (crate::planet::MIN_RADIUS, crate::planet::MAX_RADIUS),
                        });
                        match field {
                            "biome" => {
                                c.biome = Biome::ALL
                                    .into_iter()
                                    .find(|&x| biome_key(x) == v)
                                    .ok_or_else(bad)?
                            }
                            "radius" => c.radius = f()?,
                            "seed" => c.seed = v.parse().map_err(|_| bad())?,
                            "day" => c.day = f()?,
                            "tilt" => c.tilt = f()?,
                            _ => set_gen(&mut c.knobs, field, v).map_err(|_| bad())?,
                        }
                    }
                }
            }
        }
        if !has_seed {
            return Err("no seed".into());
        }
        w.make.spawn = spawn;
        w.make = w.make.clamped();
        Ok(w)
    }
}

fn biome_key(b: Biome) -> &'static str {
    match b {
        Biome::Meadow => "meadow",
        Biome::Forest => "forest",
        Biome::Desert => "desert",
        Biome::Frozen => "frozen",
        Biome::Barren => "barren",
        Biome::Crystal => "crystal",
    }
}

/// The number knobs of a generation, by name.
fn gen_fields(g: &mut Gen) -> [(&'static str, &mut f32); 15] {
    [
        ("relief", &mut g.relief),
        ("mountains", &mut g.mountains),
        ("ranges", &mut g.ranges),
        ("roughness", &mut g.roughness),
        ("warmth", &mut g.warmth),
        ("moisture", &mut g.moisture),
        ("soil_depth", &mut g.soil_depth),
        ("trees", &mut g.trees),
        ("grass", &mut g.grass),
        ("flowers", &mut g.flowers),
        ("caves", &mut g.caves),
        ("cave_size", &mut g.cave_size),
        ("ores", &mut g.ores),
        ("oceans", &mut g.oceans),
        ("rivers", &mut g.rivers),
    ]
}

fn gen_blocks(g: &mut Gen) -> [(&'static str, &mut u8); 3] {
    [
        ("top", &mut g.top),
        ("soil", &mut g.soil),
        ("stone", &mut g.stone),
    ]
}

fn set_gen(g: &mut Gen, field: &str, v: &str) -> Result<(), ()> {
    if let Some((_, x)) = gen_fields(g).into_iter().find(|(k, _)| *k == field) {
        *x = v.parse().map_err(|_| ())?;
    } else if let Some((_, x)) = gen_blocks(g).into_iter().find(|(k, _)| *k == field) {
        *x = v.parse().map_err(|_| ())?;
    }
    Ok(())
}

/// A folder name for a world: its name in lowercase letters, digits and dashes.
pub fn slug(name: &str) -> String {
    let mut s = String::new();
    for c in name.trim().chars() {
        if c.is_alphanumeric() {
            s.extend(c.to_lowercase());
        } else if !s.ends_with('-') && !s.is_empty() {
            s.push('-');
        }
    }
    let s = s.trim_end_matches('-').to_string();
    if s.is_empty() { "world".into() } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::universe::Universe;

    #[test]
    fn round_trip() {
        let mut w = WorldInfo::new("Mi mundo", 42);
        w.mode = GameMode::Survival;
        w.created = 10;
        w.played = 20;
        w.make.single_planet = true;
        w.make.home_only = true;
        w.make.planets = (2, 5);
        w.make.moon_chance = 2.5;
        w.make.max_moons = 3;
        w.make.allowed = [true, false, true, true, false];
        w.make.profiles[1].trees = 2.0;
        w.make.profiles[3].top = 7;
        let home = Universe::new(42).spawn().0;
        let mut c = Customization::of(&home);
        c.knobs.relief = 1.7;
        c.biome = Biome::Frozen;
        w.make.spawn = Some(c);
        let back = WorldInfo::from_text(&w.to_text()).unwrap();
        assert_eq!(back.name, w.name);
        assert_eq!(back.seed, 42);
        assert_eq!(back.mode, GameMode::Survival);
        let (a, b) = (back.make.spawn.unwrap(), c);
        assert_eq!(
            (a.biome, a.radius, a.seed, a.knobs, a.day, a.tilt),
            (b.biome, b.radius, b.seed, b.knobs, b.day, b.tilt)
        );
        let mut g = back.make.clone();
        g.spawn = w.make.spawn;
        assert_eq!(g, w.make.clone().clamped());
    }

    #[test]
    fn defaults_are_the_old_universe() {
        let w = WorldInfo::from_text("seed = 7\n").unwrap();
        assert_eq!(w.make, WorldGen::default());
        assert!(WorldInfo::from_text("name = x\n").is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slug("  Mi Mundo #2! "), "mi-mundo-2");
        assert_eq!(slug("???"), "world");
    }
}
