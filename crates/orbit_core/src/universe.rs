//! The endless universe: space is cut into sectors; about 7 in 10 hold a star system whose
//! planets are derived from the world seed. Nothing here is stored: same seed, same universe.

use crate::planet::{moon_soi, Biome, Customization, Gen, Host, Orbit, PlanetSpec, Spin, MAX_RADIUS, MIN_RADIUS, MOON_MAX_RADIUS, MOON_MIN_RADIUS};
use crate::station::{sync_radius, LANE};
use crate::travel::mu;
use crate::player::soi;
use crate::rng::{mix, Rng};
use glam::{DVec3, IVec3, Quat, Vec3};

pub const SECTOR: f64 = 288_000.0;
/// Systems keep this far from their sector's walls, so neighbors never overlap.
const MARGIN: f64 = 120_000.0;

#[derive(Clone, Debug)]
pub struct StarSystem {
    pub sector: IVec3,
    pub name: String,
    /// World space, f64 like every absolute position.
    pub center: DVec3,
    pub star_radius: f32,
    pub star_color: [f32; 3],
    pub planets: Vec<PlanetSpec>,
}

/// How a world's universe is generated, chosen when the world is created. The defaults are the
/// universe as it always was: a world made with them is the same as one made before they existed.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldGen {
    /// Only the home planet (and its moons): no other planets, no other stars.
    pub single_planet: bool,
    /// Only the home star system (its planets and moons): no other stars.
    pub home_only: bool,
    /// The home planet as the player shaped it at creation; None: as generated.
    pub spawn: Option<Customization>,
    /// Generation knobs every other planet and moon of each type starts from (by `Biome::ALL`).
    pub profiles: [Gen; 5],
    /// Planet types that may come up (by `Biome::ALL`); none on: all of them.
    pub allowed: [bool; 5],
    /// Fraction of sectors that hold a star system, 0.05..=1.
    pub star_density: f32,
    /// Planets a system has (the home system too), 1..=7.
    pub planets: (u32, u32),
    /// Stars' size, times the usual (800 to 3200 blocks), 0.5..=1.5.
    pub star_size: f32,
    /// How fast planets go round their stars, times the usual, 0..=4.
    pub orbit_speed: f32,
    /// How fast planets turn (shorter days), times the usual, 0.25..=4.
    pub day_speed: f32,
    /// Most moons a planet has, 0..=3, and how likely each is, times the usual, 0..=3.
    pub max_moons: u32,
    pub moon_chance: f32,
}

impl Default for WorldGen {
    fn default() -> Self {
        Self {
            single_planet: false,
            home_only: false,
            spawn: None,
            profiles: [Gen::default(); 5],
            allowed: [true; 5],
            star_density: 0.7,
            planets: (3, 7),
            star_size: 1.0,
            orbit_speed: 1.0,
            day_speed: 1.0,
            max_moons: 2,
            moon_chance: 1.0,
        }
    }
}

impl WorldGen {
    /// Kept to the allowed ranges.
    pub fn clamped(mut self) -> Self {
        self.star_density = self.star_density.clamp(0.05, 1.0);
        let lo = self.planets.0.clamp(1, 7);
        self.planets = (lo, self.planets.1.clamp(lo, 7));
        self.star_size = self.star_size.clamp(0.5, 1.5);
        self.orbit_speed = self.orbit_speed.clamp(0.0, 4.0);
        self.day_speed = self.day_speed.clamp(0.25, 4.0);
        self.max_moons = self.max_moons.min(3);
        self.moon_chance = self.moon_chance.clamp(0.0, 3.0);
        for g in &mut self.profiles {
            *g = g.clamped();
        }
        self
    }

    fn profile(&self, b: Biome) -> Gen {
        // Forest is a column biome of temperate worlds, never a planet type; it shares Meadow's.
        let b = if b == Biome::Forest { Biome::Meadow } else { b };
        Biome::ALL.iter().position(|&x| x == b).map_or_else(Gen::default, |i| self.profiles[i])
    }
}

#[derive(Clone, Debug)]
pub struct Universe {
    pub seed: u64,
    pub make: WorldGen,
}

impl Universe {
    pub fn new(seed: u64) -> Self {
        Self::with(seed, WorldGen::default())
    }

    pub fn with(seed: u64, make: WorldGen) -> Self {
        Self { seed, make: make.clamped() }
    }

    pub fn sector_of(p: DVec3) -> IVec3 {
        (p / SECTOR).floor().as_ivec3()
    }

    fn sector_seed(&self, s: IVec3) -> u64 {
        mix(self.seed ^ mix(s.x as u64 ^ mix(s.y as u64 ^ mix(s.z as u64))))
    }

    pub fn system(&self, s: IVec3) -> Option<StarSystem> {
        let home = s == IVec3::ZERO;
        let g = &self.make;
        let mut rng = Rng::new(self.sector_seed(s));
        if !home && (g.single_planet || g.home_only || rng.f32() >= g.star_density) {
            return None;
        }
        let center = if home {
            DVec3::ZERO
        } else {
            let mut inside = || MARGIN + (SECTOR - 2.0 * MARGIN) * rng.f32() as f64;
            s.as_dvec3() * SECTOR + DVec3::new(inside(), inside(), inside())
        };
        // 4 to 8 times what a star of 200 to 400 blocks would be: 800 to 3200.
        let star_radius = rng.range(200.0, 400.0) * rng.range(4.0, 8.0) * g.star_size;
        let warmth = rng.f32();
        let star_color = [1.0, 0.55 + 0.4 * warmth, 0.3 + 0.7 * warmth * warmth];
        let tilt = Quat::from_axis_angle(rng.dir(), rng.range(0.0, 0.3));

        let count = rng.int(g.planets.0 as i64, g.planets.1 as i64) as usize;
        let count = if g.single_planet { 1 } else { count };
        // The planet types that may come up; with all of them the draw is the one it always was.
        let types: Vec<Biome> = Biome::ALL.iter().zip(g.allowed).filter(|x| x.1).map(|x| *x.0).collect();
        let types = if types.is_empty() { Biome::ALL.to_vec() } else { types };
        let mut planets = Vec::with_capacity(count);
        // Every orbit leaves room for the sphere of influence of the biggest planet a player could
        // customize this one into, so they never overlap, and the first one clears the star by
        // as wide a gap as the others keep between them.
        let reach = soi(PlanetSpec::max_gravity_radius());
        let mut orbit = star_radius * 3.0 + rng.range(500.0, 2000.0);
        for idx in 0..count {
            let (radius, biome) = if home && idx == 0 {
                (150.0, Biome::Meadow)
            } else {
                // Skewed toward the small end: big worlds are the rarer sight.
                let r = MIN_RADIUS + (MAX_RADIUS - MIN_RADIUS) * rng.f32().powf(1.6);
                (r.round(), types[rng.int(0, types.len() as i64 - 1) as usize])
            };
            let id = mix(self.sector_seed(s) ^ idx as u64);
            let mut spec = PlanetSpec::new(id, planet_name(id), radius, biome);
            if !(home && idx == 0) {
                spec.knobs = g.profile(biome);
            }
            orbit += reach;
            // Concentric orbits, one per planet, so planets never meet. Slow: a few blocks per second.
            spec.orbit = Orbit { star: center, radius: orbit, phase: rng.range(0.0, std::f32::consts::TAU), speed: rng.range(1.5, 4.0) / orbit * g.orbit_speed, tilt };
            // Axial tilt up to ~25°, a day of 5 to 10 minutes (short enough for a synchronous
            // orbit inside the sphere of influence).
            let axis = tilt * (Quat::from_axis_angle(rng.dir(), rng.range(0.0, 0.44)) * Vec3::Y);
            spec.spin = Spin { axis: axis.normalize(), speed: std::f32::consts::TAU / rng.range(300.0, 600.0) * g.day_speed, phase: rng.range(0.0, std::f32::consts::TAU) };
            spec.advance(0.0);
            if home && idx == 0 {
                if let Some(c) = g.spawn {
                    c.apply(&mut spec, 0.0);
                }
            }
            orbit += reach + rng.range(500.0, 2000.0);
            planets.push(spec);
        }
        // Moons after all the planets (planets keep their places in the list). Their own random
        // numbers, so planets are what they were before moons.
        for i in 0..planets.len() {
            let host = planets[i].clone();
            let mut mr = Rng::new(mix(self.sector_seed(s) ^ 0x4D00_0000 ^ i as u64));
            let x = mr.f32();
            let k = g.moon_chance;
            let n = match host.radius {
                _ if home && i == 0 && k == 1.0 => 1,
                // (A third, or a second on a smaller planet, only when moons are made likelier.)
                r if r >= 200.0 => (x < 0.8 * k) as usize + (x < 0.35 * k) as usize + (x < 0.1 * (k - 1.0)) as usize,
                r if r >= 120.0 => (x < 0.55 * k) as usize + (x < 0.15 * (k - 1.0)) as usize,
                _ => (x < 0.2 * k) as usize + (x < 0.05 * (k - 1.0)) as usize,
            };
            let n = n.min(g.max_moons as usize);
            let mut taken = Vec::new();
            for k in 0..n {
                if let Some(m) = moon(&host, k, &mut mr, &taken, g) {
                    taken.push((m.orbit.radius - m.host.unwrap().soi, m.orbit.radius + m.host.unwrap().soi));
                    planets.push(m);
                }
            }
        }
        Some(StarSystem { sector: s, name: system_name(self.sector_seed(s), home), center, star_radius, star_color, planets })
    }

    /// Systems in the cube of sectors within `r` of the sector containing `p`.
    pub fn systems_near(&self, p: DVec3, r: i32) -> Vec<StarSystem> {
        let c = Self::sector_of(p);
        let mut out = Vec::new();
        for x in -r..=r {
            for y in -r..=r {
                for z in -r..=r {
                    out.extend(self.system(c + IVec3::new(x, y, z)));
                }
            }
        }
        out
    }

    /// The home planet and a point just above its surface.
    pub fn spawn(&self) -> (PlanetSpec, DVec3) {
        let home = self.system(IVec3::ZERO).expect("home system always exists");
        let p = home.planets[0].clone();
        let up = p.rotation * coast(&p);
        let at = p.center + (up * (p.ceiling() - 4.0)).as_dvec3();
        (p, at)
    }
}

/// Where to start on a planet (planet-local direction): on dry, mild ground by the sea, the
/// nearest such place to its +y; +y itself if there is none.
pub fn coast(p: &PlanetSpec) -> Vec3 {
    use crate::planet::{Biome, Terrain};
    let terrain = Terrain::new(p);
    let step = 3.0 / p.radius; // about three blocks, in radians
    // Directions spiralling out from +y.
    let golden = std::f32::consts::PI * (3.0 - 5f32.sqrt());
    for n in 0..6000 {
        let polar = (n as f32).sqrt() * step * 1.2;
        if polar > std::f32::consts::PI {
            break;
        }
        let (az, d) = (n as f32 * golden, polar);
        let dir = Vec3::new(d.sin() * az.cos(), d.cos(), d.sin() * az.sin());
        let c = terrain.column(dir);
        if c.is_wet() || !(1..=4).contains(&c.height) || !matches!(c.biome, Biome::Meadow | Biome::Forest) {
            continue;
        }
        let t = dir.any_orthonormal_vector();
        let b = dir.cross(t);
        let wet = (0..8).any(|k| {
            let a = k as f32 * std::f32::consts::FRAC_PI_4;
            terrain.column((dir + (t * a.cos() + b * a.sin()) * step * 2.5).normalize()).is_wet()
        });
        if wet {
            return dir;
        }
    }
    Vec3::Y
}

/// Moon `k` of `host`, if one fits: a small world on a circular orbit round its planet, near its
/// equator, tidally locked (one face always toward the planet). Its sphere of influence (where its
/// pull beats the planet's: its Hill sphere) has to fit inside the planet's with room for station
/// lanes below it, clear of the planet's synchronous orbit and of the other moons (`taken`).
fn moon(host: &PlanetSpec, k: usize, rng: &mut Rng, taken: &[(f32, f32)], g: &WorldGen) -> Option<PlanetSpec> {
    const BIOMES: [Biome; 5] = [Biome::Barren, Biome::Barren, Biome::Frozen, Biome::Crystal, Biome::Desert];
    let id = mix(host.id ^ 0x4D00_4E00 ^ k as u64);
    let radius = rng.range(MOON_MIN_RADIUS, MOON_MAX_RADIUS.min(host.radius * 0.5).max(MOON_MIN_RADIUS + 1.0)).round();
    let biome = BIOMES[rng.int(0, BIOMES.len() as i64 - 1) as usize];
    let mut spec = PlanetSpec::new(id, format!("{} {}", host.name, ["I", "II", "III"][k.min(2)]), radius, biome);
    spec.knobs = g.profile(biome);
    let (gm, pull) = (spec.gravity_radius(), host.pull());
    let gp = pull.gravity_radius;
    let sync = sync_radius(gp, host.spin.speed);
    // Every height it could go round at, every 10 blocks; one of those that fit, at random.
    let fits = |a: f32| {
        let soi_m = moon_soi(a, gm, gp);
        soi_m > gm + 2.5 * LANE // room round it for a parking orbit and a few lanes
            && a - soi_m > gp + 2.0 * LANE // a couple of the planet's lanes under it
            && a + soi_m < pull.soi - pull.taper() - LANE
            && sync.is_none_or(|s| (a - s).abs() > soi_m + 2.0 * LANE)
            && taken.iter().all(|&(lo, hi)| a + soi_m + LANE < lo || a - soi_m - LANE > hi)
    };
    let heights: Vec<f32> = (0..).map(|i| gp + i as f32 * 10.0).take_while(|&a| a < pull.soi).filter(|&a| fits(a)).collect();
    if !heights.is_empty() {
        let a = heights[rng.int(0, heights.len() as i64 - 1) as usize];
        let soi_m = moon_soi(a, gm, gp);
        let way = if host.spin.speed < 0.0 { -1.0 } else { 1.0 };
        let tilt = Quat::from_rotation_arc(Vec3::Y, -host.spin.axis) * Quat::from_rotation_x(rng.range(-0.12, 0.12));
        let speed = way * (mu(gp) / (a * a * a)).sqrt();
        spec.orbit = Orbit { star: DVec3::ZERO, radius: a, phase: rng.range(0.0, std::f32::consts::TAU), speed, tilt };
        spec.spin = Spin { axis: spec.orbit.axis(), speed: speed.abs(), phase: 0.0 };
        spec.host = Some(Host { id: host.id, orbit: host.orbit, soi: soi_m });
        spec.advance(0.0);
        return Some(spec);
    }
    None
}

fn system_name(seed: u64, home: bool) -> String {
    if home {
        return "Sol Prima".into();
    }
    const A: [&str; 14] = ["Al", "Ve", "Kor", "Tau", "Ny", "Sar", "Ced", "Ilu", "Pho", "Dra", "Hel", "Mir", "Ost", "Zen"];
    const B: [&str; 10] = ["gol", "thea", "nis", "ara", "cyon", "mus", "dor", "phae", "rion", "ix"];
    const C: [&str; 8] = ["Major", "Minor", "Prime", "Nova", "Deep", "Reach", "Gate", "Drift"];
    let h = mix(seed ^ 0x0051_57e3);
    format!("{}{} {}", A[(h % 14) as usize], B[((h >> 8) % 10) as usize], C[((h >> 16) % 8) as usize])
}

fn planet_name(id: u64) -> String {
    const A: [&str; 12] = ["Ka", "Vo", "Thel", "Ori", "Zan", "Mu", "Eka", "Ryn", "Sol", "Ixa", "Bel", "Qua"];
    const B: [&str; 10] = ["ris", "dor", "nia", "x", "mar", "lune", "tor", "vex", "ra", "dun"];
    let h = mix(id);
    format!("{}{}-{}", A[(h % 12) as usize], B[((h >> 8) % 10) as usize], (h >> 16) % 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_same_universe() {
        let (a, b) = (Universe::new(42), Universe::new(42));
        for s in [IVec3::ZERO, IVec3::new(3, -1, 7), IVec3::new(-20, 5, 2)] {
            let (x, y) = (a.system(s), b.system(s));
            assert_eq!(x.is_some(), y.is_some());
            if let (Some(x), Some(y)) = (x, y) {
                assert_eq!(x.center, y.center);
                assert_eq!(x.planets.len(), y.planets.len());
            }
        }
    }

    #[test]
    fn planets_stay_in_their_sector_and_apart() {
        let u = Universe::new(9);
        for sys in u.systems_near(DVec3::ZERO, 3) {
            let planets: Vec<_> = sys.planets.iter().filter(|p| !p.is_moon()).collect();
            for (i, p) in planets.iter().enumerate() {
                if sys.sector != IVec3::ZERO {
                    assert_eq!(Universe::sector_of(p.center), sys.sector);
                }
                assert!((MIN_RADIUS..=MAX_RADIUS).contains(&p.radius));
                let reach = soi(PlanetSpec::max_gravity_radius());
                assert!(((p.orbit.radius + reach) as f64) < MARGIN, "system reaches into the next sector");
                assert!(p.orbit.radius - reach > sys.star_radius * 3.0, "too near the star");
                for q in &planets[i + 1..] {
                    // Spheres of influence apart even when both are customized to the largest size.
                    assert!(p.center.distance(q.center) > 2.0 * reach as f64);
                }
            }
        }
    }

    #[test]
    fn moons_fit_inside_their_planets_pull() {
        let u = Universe::new(9);
        let mut count = 0;
        for sys in u.systems_near(DVec3::ZERO, 2) {
            for m in sys.planets.iter().filter(|p| p.is_moon()) {
                count += 1;
                let h = m.host.unwrap();
                let host = sys.planets.iter().find(|p| p.id == h.id).unwrap();
                let (pp, pm) = (host.pull(), m.pull());
                assert!((MOON_MIN_RADIUS..=MOON_MAX_RADIUS).contains(&m.radius));
                // Its sphere of influence inside the planet's, above a couple of station lanes.
                assert!(m.orbit.radius + pm.soi < pp.soi - pp.taper());
                assert!(m.orbit.radius - pm.soi > pp.gravity_radius + 2.0 * LANE);
                assert!(pm.soi > pm.gravity_radius + 2.0 * LANE);
                // Clear of the planet's synchronous orbit.
                if let Some(s) = sync_radius(pp.gravity_radius, host.spin.speed) {
                    assert!((m.orbit.radius - s).abs() > pm.soi + LANE);
                }
                // At the distance from its planet it orbits at, going round at circular speed.
                assert!(((m.center.distance(host.center) as f32) - m.orbit.radius).abs() < 1.0);
                let v = m.velocity.length();
                assert!((v - crate::travel::circular_speed(m.orbit.radius, pp)).abs() < 0.01 * v);
                // Tidally locked: it turns once per orbit, the way it goes round.
                assert!((m.spin.speed - m.orbit.speed.abs()).abs() < 1e-6);
                assert!(m.spin.axis.dot(m.orbit.axis()) > 0.9999);
                // Its velocity is how its center moves.
                let mut later = m.clone();
                later.advance(0.5);
                let mut before = m.clone();
                before.advance(-0.5);
                let fd = (later.center - before.center).as_vec3() - (host_at(host, 0.5) - host_at(host, -0.5)).as_vec3();
                assert!((fd - m.velocity).length() < 0.01 * v.max(1.0), "{fd} vs {}", m.velocity);
            }
            // No two moons of a planet share heights.
            for a in sys.planets.iter().filter(|p| p.is_moon()) {
                for b in sys.planets.iter().filter(|p| p.is_moon() && p.id != a.id && p.host.unwrap().id == a.host.unwrap().id) {
                    let (sa, sb) = (a.host.unwrap().soi, b.host.unwrap().soi);
                    assert!((a.orbit.radius - b.orbit.radius).abs() > sa + sb);
                }
            }
        }
        assert!(count > 5, "only {count} moons");
        let home = u.system(IVec3::ZERO).unwrap();
        assert!(home.planets.iter().any(|m| m.host.is_some_and(|h| h.id == home.planets[0].id)), "the home planet has a moon");
    }

    fn host_at(p: &PlanetSpec, t: f64) -> DVec3 {
        let mut p = p.clone();
        p.advance(t);
        p.center
    }

    #[test]
    fn world_gen_knobs() {
        let base = Universe::new(5);
        // The defaults are the universe as it was.
        let a = base.system(IVec3::ZERO).unwrap();
        let b = Universe::with(5, WorldGen::default()).system(IVec3::ZERO).unwrap();
        assert_eq!(a.planets.len(), b.planets.len());
        let one = Universe::with(5, WorldGen { single_planet: true, ..default() });
        assert!(one.systems_near(DVec3::ZERO, 2).len() == 1);
        assert_eq!(one.system(IVec3::ZERO).unwrap().planets.iter().filter(|p| !p.is_moon()).count(), 1);
        let solo = Universe::with(5, WorldGen { home_only: true, planets: (3, 3), max_moons: 0, ..default() });
        assert_eq!(solo.systems_near(DVec3::ZERO, 3).len(), 1);
        assert_eq!(solo.system(IVec3::ZERO).unwrap().planets.len(), 3);
        let none = Universe::with(5, WorldGen { max_moons: 0, ..default() });
        assert!(none.systems_near(DVec3::ZERO, 1).iter().all(|s| s.planets.iter().all(|p| !p.is_moon())));
        // At the extremes systems still fit their sectors.
        let big = Universe::with(5, WorldGen { planets: (7, 7), star_size: 1.5, moon_chance: 3.0, max_moons: 3, star_density: 1.0, ..default() });
        for sys in big.systems_near(DVec3::ZERO, 1) {
            for p in sys.planets.iter().filter(|p| !p.is_moon()) {
                let reach = soi(PlanetSpec::max_gravity_radius());
                assert!(((p.orbit.radius + reach) as f64) < MARGIN, "system reaches into the next sector");
            }
        }
        let fast = Universe::with(5, WorldGen { day_speed: 2.0, orbit_speed: 2.0, ..default() }).spawn().0;
        let slow = base.spawn().0;
        assert!((fast.spin.speed / slow.spin.speed - 2.0).abs() < 1e-4);
    }

    fn default() -> WorldGen {
        WorldGen::default()
    }
}
