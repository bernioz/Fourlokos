//! A voxel planet: block data on a cube-sphere, terrain generation, edits and raycasts.
//! All positions here are planet-local (origin at the planet's center), in blocks.

use crate::block::{self, Block};
use crate::caves;
use crate::liquid::{self, Basin, Cells, Flow};
use crate::player::Pull;
use crate::rng::Rng;
use crate::sphere::{CellId, CubeSphere, Side};
use crate::trees;
use glam::{DVec3, Quat, Vec3};
use noise::{Fbm, MultiFractal, NoiseFn, Perlin};
use std::collections::HashSet;

/// Layers of air above the highest terrain, room to build.
pub const HEADROOM: usize = 64;
/// Extra distance past the build ceiling where the planet's gravity still holds.
pub const GRAVITY_MARGIN: f32 = 24.0;
/// Columns per chunk side.
pub const CHUNK: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Biome {
    Meadow,
    Forest,
    Desert,
    Frozen,
    Barren,
    Crystal,
}

impl Biome {
    /// Planet types. Meadow, Desert and Frozen planets are temperate worlds with a climate (their
    /// columns get Forest, Meadow, Desert or Frozen by latitude, warmth and moisture); the
    /// others are one biome throughout.
    pub const ALL: [Biome; 5] = [
        Biome::Meadow,
        Biome::Desert,
        Biome::Frozen,
        Biome::Barren,
        Biome::Crystal,
    ];

    /// (top, subsurface, deep)
    pub fn layers(self) -> (Block, Block, Block) {
        use block::*;
        match self {
            Biome::Meadow | Biome::Forest => (GRASS, DIRT, STONE),
            Biome::Desert => (SAND, SANDSTONE, STONE),
            Biome::Frozen => (SNOW, DIRT, STONE),
            Biome::Barren => (STONE, STONE, BASALT),
            Biome::Crystal => (BASALT, BASALT, CRYSTAL),
        }
    }

    /// Representative color for far views.
    pub fn color(self) -> [f32; 3] {
        block::color(self.layers().0)
    }
}

/// A circular orbit around a star. Position depends only on time, so every client agrees.
#[derive(Clone, Copy, Debug, Default)]
pub struct Orbit {
    /// The star's position, world space (f64: the universe is far bigger than f32 can place).
    pub star: DVec3,
    pub radius: f32,
    pub phase: f32,
    /// Radians per second.
    pub speed: f32,
    pub tilt: Quat,
}

impl Orbit {
    pub fn position(&self, t: f64) -> DVec3 {
        let a = self.phase as f64 + (self.speed as f64 * t).rem_euclid(std::f64::consts::TAU);
        self.star + self.tilt.as_dquat() * (DVec3::new(a.cos(), 0.0, a.sin()) * self.radius as f64)
    }

    /// How fast it goes along the orbit at time `t`, world.
    pub fn velocity(&self, t: f64) -> Vec3 {
        let a = self.phase as f64 + (self.speed as f64 * t).rem_euclid(std::f64::consts::TAU);
        self.tilt * (Vec3::new(-a.sin() as f32, 0.0, a.cos() as f32) * self.radius * self.speed)
    }

    /// The axis it goes round (unit, world): its angular velocity's direction.
    pub fn axis(&self) -> Vec3 {
        // (cos a, 0, sin a) turns from x toward z as a grows: about -y.
        self.tilt * Vec3::NEG_Y * self.speed.signum()
    }
}

/// Spin about the planet's own axis.
#[derive(Clone, Copy, Debug)]
pub struct Spin {
    pub axis: Vec3,
    /// Radians per second.
    pub speed: f32,
    pub phase: f32,
}

impl Default for Spin {
    fn default() -> Self {
        Self {
            axis: Vec3::Y,
            speed: 0.0,
            phase: 0.0,
        }
    }
}

impl Spin {
    pub fn rotation(&self, t: f64) -> Quat {
        let a = self.phase + (self.speed as f64 * t).rem_euclid(std::f64::consts::TAU) as f32;
        Quat::from_axis_angle(self.axis, a)
    }
}

#[derive(Clone, Debug)]
pub struct PlanetSpec {
    pub id: u64,
    pub name: String,
    /// Current center, world space; follow `orbit` with `PlanetSpec::advance`.
    pub center: DVec3,
    /// How fast the center moves now, world: a moon's way round its planet; zero for a planet
    /// (its slow way round the star is not counted).
    pub velocity: Vec3,
    /// Round the star; for a moon, round its planet (`star` is then zero: relative to it).
    pub orbit: Orbit,
    /// For a moon: the planet it goes round.
    pub host: Option<Host>,
    pub spin: Spin,
    /// Current orientation; world = center + rotation * local.
    pub rotation: Quat,
    /// Mean surface radius in blocks, `MIN_RADIUS..=MAX_RADIUS`.
    pub radius: f32,
    pub biome: Biome,
    /// Terrain seed: shapes relief, climate and plants. Starts as `id`; customizing can reroll it.
    pub seed: u64,
    /// Everything else that shapes the generated planet.
    pub knobs: Gen,
}

pub const MIN_RADIUS: f32 = 80.0;
pub const MAX_RADIUS: f32 = 300.0;
/// Moons are smaller.
pub const MOON_MIN_RADIUS: f32 = 40.0;
pub const MOON_MAX_RADIUS: f32 = 90.0;

/// A moon's sphere of influence at `a` from its planet (gravity radii `gm`, `gp`): its Hill
/// sphere, a · cbrt(m / 3M), masses going as gravity radius squared (GM = g R²), and never past
/// what a planet of its size would have.
pub fn moon_soi(a: f32, gm: f32, gp: f32) -> f32 {
    crate::player::soi(gm).min(a * (gm * gm / (3.0 * gp * gp)).cbrt())
}

/// Fits every moon in a system's list to its planet as it is (see `PlanetSpec::refit`).
pub fn refit_moons(planets: &mut [PlanetSpec], t: f64) {
    for i in 0..planets.len() {
        let Some(h) = planets[i].host else { continue };
        if let Some(host) = planets.iter().find(|p| p.id == h.id).cloned() {
            planets[i].refit(&host, t);
        }
    }
}

/// The planet a moon goes round.
#[derive(Clone, Copy, Debug)]
pub struct Host {
    pub id: u64,
    /// The planet's orbit round the star.
    pub orbit: Orbit,
    /// Where the moon's pull gives way to the planet's: its sphere of influence.
    pub soi: f32,
}
pub const MAX_SCALE: f32 = 2.0;

/// The knobs of planet generation. 1 (or 0 for offsets, AIR for blocks) is the planet as the
/// universe makes it; each is clamped to its range by `Gen::clamped`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gen {
    /// Plains-and-hills relief height, 0..=2.
    pub relief: f32,
    /// Mountain height, 0..=2.
    pub mountains: f32,
    /// How much of the planet mountain ranges cover, 0..=1 (0.5 as made).
    pub ranges: f32,
    /// How tightly hills follow each other, 0.25..=3.
    pub roughness: f32,
    /// Climate offsets, -1..=1: warmer (deserts) or colder (ice), wetter (forests) or drier.
    pub warmth: f32,
    pub moisture: f32,
    /// Blocks that replace the biomes' own: top, soil under it, rock under that (AIR: the biome's).
    pub top: Block,
    pub soil: Block,
    pub stone: Block,
    /// Soil thickness, 0..=3.
    pub soil_depth: f32,
    /// How many trees, tall grass and flowers grow, 0..=3 each.
    pub trees: f32,
    pub grass: f32,
    pub flowers: f32,
    /// How many caves, 0..=2, and how wide, 0.5..=2.
    pub caves: f32,
    pub cave_size: f32,
    /// How many ore veins, 0..=3.
    pub ores: f32,
    /// How much of the planet seas cover, 0..=2, and how many rivers run, 0..=2.
    pub oceans: f32,
    pub rivers: f32,
}

impl Default for Gen {
    fn default() -> Self {
        Self {
            relief: 1.0,
            mountains: 1.0,
            ranges: 0.5,
            roughness: 1.0,
            warmth: 0.0,
            moisture: 0.0,
            top: block::AIR,
            soil: block::AIR,
            stone: block::AIR,
            soil_depth: 1.0,
            trees: 1.0,
            grass: 1.0,
            flowers: 1.0,
            caves: 1.0,
            cave_size: 1.0,
            ores: 1.0,
            oceans: 1.0,
            rivers: 1.0,
        }
    }
}

impl Gen {
    pub fn clamped(mut self) -> Self {
        self.relief = self.relief.clamp(0.0, MAX_SCALE);
        self.mountains = self.mountains.clamp(0.0, MAX_SCALE);
        self.ranges = self.ranges.clamp(0.0, 1.0);
        self.roughness = self.roughness.clamp(0.25, 3.0);
        self.warmth = self.warmth.clamp(-1.0, 1.0);
        self.moisture = self.moisture.clamp(-1.0, 1.0);
        for b in [&mut self.top, &mut self.soil, &mut self.stone] {
            if !block::is_solid(*b) || *b == block::CORE {
                *b = block::AIR;
            }
        }
        self.soil_depth = self.soil_depth.clamp(0.0, 3.0);
        self.trees = self.trees.clamp(0.0, 3.0);
        self.grass = self.grass.clamp(0.0, 3.0);
        self.flowers = self.flowers.clamp(0.0, 3.0);
        self.caves = self.caves.clamp(0.0, MAX_SCALE);
        self.cave_size = self.cave_size.clamp(0.5, 2.0);
        self.ores = self.ores.clamp(0.0, 3.0);
        self.oceans = self.oceans.clamp(0.0, 2.0);
        self.rivers = self.rivers.clamp(0.0, 2.0);
        self
    }
}

/// What a player can change about a planet. Applied to a spec, it gives the planet a new shape;
/// the planet must then be generated again.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Customization {
    pub radius: f32,
    pub biome: Biome,
    pub seed: u64,
    pub knobs: Gen,
    /// Length of a day, seconds.
    pub day: f32,
    /// Axial tilt against the orbit, degrees, 0..=90.
    pub tilt: f32,
    /// The sizes `radius` may take (smaller for a moon).
    pub radius_range: (f32, f32),
}

impl Customization {
    pub fn of(spec: &PlanetSpec) -> Self {
        let normal = spec.orbit.tilt * Vec3::Y;
        let tilt = spec
            .spin
            .axis
            .dot(normal)
            .clamp(-1.0, 1.0)
            .acos()
            .to_degrees();
        Self {
            radius: spec.radius,
            biome: spec.biome,
            seed: spec.seed,
            knobs: spec.knobs,
            day: std::f32::consts::TAU / spec.spin.speed.max(1e-6),
            tilt,
            radius_range: spec.radius_range(),
        }
    }

    /// Applies to `spec`, clamped to the allowed ranges. `t` is the universe time: the spin
    /// keeps its current angle so the planet does not jump when the day length changes.
    pub fn apply(&self, spec: &mut PlanetSpec, t: f64) {
        let (lo, hi) = spec.radius_range();
        spec.radius = self.radius.clamp(lo, hi).round();
        spec.biome = self.biome;
        spec.seed = self.seed;
        spec.knobs = self.knobs.clamped();
        // Tilt the axis away from the orbit's normal, keeping the direction it leans in.
        let normal = spec.orbit.tilt * Vec3::Y;
        let lean = (spec.spin.axis - normal * spec.spin.axis.dot(normal))
            .normalize_or(normal.any_orthonormal_vector());
        let tilt = self.tilt.clamp(0.0, 90.0).to_radians();
        let axis = (normal * tilt.cos() + lean * tilt.sin()).normalize();
        if axis.angle_between(spec.spin.axis) > 1e-3 {
            spec.spin.axis = axis;
        }
        let speed = std::f32::consts::TAU / self.day.clamp(60.0, 7200.0);
        let turned = |s: f32| (s as f64 * t).rem_euclid(std::f64::consts::TAU);
        spec.spin.phase = (spec.spin.phase as f64 + turned(spec.spin.speed) - turned(speed))
            .rem_euclid(std::f64::consts::TAU) as f32;
        spec.spin.speed = speed;
        spec.advance(t);
    }
}

impl PlanetSpec {
    /// A planet at rest at the origin, as generated (no customization).
    pub fn new(id: u64, name: String, radius: f32, biome: Biome) -> Self {
        Self {
            id,
            name,
            center: DVec3::ZERO,
            velocity: Vec3::ZERO,
            orbit: Orbit::default(),
            host: None,
            spin: Spin::default(),
            rotation: Quat::IDENTITY,
            radius,
            biome,
            seed: id,
            knobs: Gen::default(),
        }
    }

    /// The largest gravity radius any customization can give a planet: orbits are spaced by it,
    /// so a planet can grow to the maximum without reaching its neighbors.
    pub fn max_gravity_radius() -> f32 {
        Self::largest(MAX_RADIUS)
    }

    /// The largest gravity radius any customization can give a moon.
    pub fn max_moon_gravity_radius() -> f32 {
        Self::largest(MOON_MAX_RADIUS)
    }

    fn largest(radius: f32) -> f32 {
        let mut s = Self::new(0, String::new(), radius, Biome::Barren);
        s.knobs.relief = MAX_SCALE;
        s.knobs.mountains = MAX_SCALE;
        s.gravity_radius()
    }

    /// A moon's orbit and sphere of influence made to fit its planet as it is now (after either
    /// was customized: a bigger planet pulls harder, so the moon goes round faster). It stays
    /// where it is at time `t`; a tidally locked moon stays locked.
    pub fn refit(&mut self, host: &PlanetSpec, t: f64) {
        if self.host.is_none() {
            return;
        }
        let (gp, gm, a) = (
            host.gravity_radius(),
            self.gravity_radius(),
            self.orbit.radius,
        );
        self.host = Some(Host {
            id: host.id,
            orbit: host.orbit,
            soi: moon_soi(a, gm, gp),
        });
        let speed = self.orbit.speed.signum() * (crate::travel::mu(gp) / (a * a * a)).sqrt();
        let locked = (self.spin.speed - self.orbit.speed.abs()).abs() < 1e-6;
        let turned = |s: f32| (s as f64 * t).rem_euclid(std::f64::consts::TAU);
        let keep = |phase: f32, old: f32, new: f32| {
            (phase as f64 + turned(old) - turned(new)).rem_euclid(std::f64::consts::TAU) as f32
        };
        self.orbit.phase = keep(self.orbit.phase, self.orbit.speed, speed);
        if locked {
            self.spin.phase = keep(self.spin.phase, self.spin.speed, speed.abs());
            self.spin.speed = speed.abs();
        }
        self.orbit.speed = speed;
        self.advance(t);
    }

    pub fn is_moon(&self) -> bool {
        self.host.is_some()
    }

    /// The sizes it may be customized to: smaller for a moon.
    pub fn radius_range(&self) -> (f32, f32) {
        if self.is_moon() {
            (MOON_MIN_RADIUS, MOON_MAX_RADIUS)
        } else {
            (MIN_RADIUS, MAX_RADIUS)
        }
    }

    /// Where its star is, world space.
    pub fn star(&self) -> DVec3 {
        self.host.map_or(self.orbit.star, |h| h.orbit.star)
    }

    /// Its gravity: a planet's reaches `player::soi`; a moon's ends sooner, at its own sphere of
    /// influence inside its planet's.
    pub fn pull(&self) -> Pull {
        let p = Pull::planet(self.gravity_radius());
        match self.host {
            Some(h) => Pull {
                soi: p.soi.min(h.soi),
                ..p
            },
            None => p,
        }
    }

    /// Moves along the orbit and spins to time `t`.
    pub fn advance(&mut self, t: f64) {
        self.center = match &self.host {
            Some(h) => h.orbit.position(t) + self.orbit.position(t),
            None => self.orbit.position(t),
        };
        self.velocity = match self.host {
            Some(_) => self.orbit.velocity(t),
            None => Vec3::ZERO,
        };
        self.rotation = self.spin.rotation(t);
    }

    pub fn to_local(&self, world: DVec3) -> Vec3 {
        self.rotation.inverse() * (world - self.center).as_vec3()
    }

    pub fn to_world(&self, local: Vec3) -> DVec3 {
        self.center + (self.rotation * local).as_dvec3()
    }

    /// Layers of ground under the mean surface, bedrock included: room for caves and ores.
    pub fn depth(&self) -> usize {
        ((self.radius * 0.15) as usize)
            .clamp(16, 40)
            .min((self.radius * 0.5) as usize)
            .max(4)
    }
    /// Height of the gentle base relief (plains and hills).
    pub fn amplitude(&self) -> f32 {
        (self.radius * 0.05).clamp(3.0, 8.0) * self.knobs.relief
    }
    /// Extra height mountain ranges reach above the base relief.
    pub fn mountain_height(&self) -> f32 {
        let base = match self.biome {
            Biome::Crystal | Biome::Barren => (self.radius * 0.25).clamp(8.0, 40.0),
            _ => (self.radius * 0.22).clamp(8.0, 36.0),
        };
        base * self.knobs.mountains
    }
    /// Highest terrain above the mean radius (land stands a little over the sea, see `Terrain`).
    pub fn max_height(&self) -> f32 {
        self.amplitude() * 1.6 + 1.0 + self.mountain_height()
    }
    /// How far the ocean basins sink under the land, blocks.
    pub fn ocean_depth(&self) -> f32 {
        (self.radius * 0.08)
            .clamp(6.0, 22.0)
            .min(self.depth() as f32 * 0.6)
    }
    /// What fills the low ground: water on living worlds, lava on barren ones, nothing on crystal
    /// worlds; with how much of the planet it covers (before the `oceans` knob).
    pub fn sea(&self) -> (Block, f32) {
        match self.biome {
            Biome::Meadow | Biome::Frozen => (block::WATER, 1.0),
            Biome::Desert => (block::WATER, 0.45),
            Biome::Forest => (block::WATER, 1.0),
            Biome::Barren => (block::LAVA, 0.35),
            Biome::Crystal => (block::AIR, 0.0),
        }
    }
    /// Radius of the build ceiling.
    pub fn ceiling(&self) -> f32 {
        self.radius + self.max_height() + HEADROOM as f32
    }
    pub fn gravity_radius(&self) -> f32 {
        self.ceiling() + GRAVITY_MARGIN
    }
}

/// The terrain's shape and climate, shared by voxel generation and far views.
pub struct Terrain {
    height: Fbm<Perlin>,
    ranges: Fbm<Perlin>,
    ridges: Perlin,
    warmth: Fbm<Perlin>,
    moisture: Fbm<Perlin>,
    soil: Perlin,
    continents: Fbm<Perlin>,
    rivers: Perlin,
    /// The liquid of its seas (AIR: none), how much they cover, and how deep the basins go.
    sea: Block,
    coverage: f32,
    ocean_depth: f32,
    amp: f32,
    mountains: f32,
    kind: Biome,
    /// Planet-local spin axis: latitude is measured from it.
    axis: Vec3,
    /// Warmth at the equator, before noise (0 cold .. 1 hot).
    base_warmth: f32,
    knobs: Gen,
}

/// The sea's surface: liquid fills every column below it, up to the mean radius (offset 0).
pub const SEA_LEVEL: i64 = 0;

/// One surface column: height offset from the mean radius, its biome, and its top block.
#[derive(Clone, Copy, Debug)]
pub struct Column {
    pub height: i64,
    /// The liquid over it, up to `SEA_LEVEL` (AIR: dry land); ICE for water frozen over.
    pub liquid: Block,
    pub frozen: bool,
    /// Sea floor or beach: the top block runs a couple of blocks deep (sand, gravel).
    pub bed: bool,
    pub biome: Biome,
    pub top: Block,
    /// On a mountain's rocky upper slopes: rock under the surface, not soil.
    pub rocky: bool,
    /// Blocks of soil (the biome's subsurface) under the top, 1.7's surface run: 0 bares the rock.
    pub soil: usize,
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl Column {
    /// Under a sea, lake or river.
    pub fn is_wet(&self) -> bool {
        self.liquid != block::AIR
    }

    /// Height of what you would stand on (or float on): the ground, or the sea over it.
    pub fn ground(&self) -> i64 {
        if self.is_wet() {
            self.height.max(SEA_LEVEL)
        } else {
            self.height
        }
    }
}

impl Terrain {
    pub fn new(spec: &PlanetSpec) -> Self {
        let seed = spec.seed as u32;
        // Planets close to their star run warmer.
        let star_heat = (1.0 - spec.orbit.radius / 14000.0).clamp(0.0, 1.0) * 0.25;
        let base_warmth = match spec.biome {
            Biome::Desert => 0.85,
            Biome::Frozen => 0.15,
            _ => 0.55,
        } + star_heat
            + spec.knobs.warmth * 0.5;
        Self {
            height: Fbm::<Perlin>::new(seed)
                .set_octaves(4)
                .set_persistence(0.45),
            ranges: Fbm::<Perlin>::new(seed.wrapping_add(3)).set_octaves(2),
            ridges: Perlin::new(seed.wrapping_add(4)),
            warmth: Fbm::<Perlin>::new(seed.wrapping_add(1)).set_octaves(3),
            moisture: Fbm::<Perlin>::new(seed.wrapping_add(2)).set_octaves(3),
            soil: Perlin::new(seed.wrapping_add(5)),
            continents: Fbm::<Perlin>::new(seed.wrapping_add(6))
                .set_octaves(4)
                .set_persistence(0.5),
            rivers: Perlin::new(seed.wrapping_add(7)),
            sea: spec.sea().0,
            coverage: spec.sea().1 * spec.knobs.oceans,
            ocean_depth: spec.ocean_depth(),
            amp: spec.amplitude(),
            mountains: spec.mountain_height(),
            kind: spec.biome,
            axis: spec.spin.axis,
            base_warmth,
            knobs: spec.knobs,
        }
    }

    /// A biome's (top, soil, rock), with the planet's chosen blocks in their place.
    pub fn layers(&self, biome: Biome) -> (Block, Block, Block) {
        let (t, s, r) = biome.layers();
        let pick = |own: Block, chosen: Block| if chosen == block::AIR { own } else { chosen };
        (
            pick(t, self.knobs.top),
            pick(s, self.knobs.soil),
            pick(r, self.knobs.stone),
        )
    }

    fn sample(f: &impl NoiseFn<f64, 3>, d: Vec3, freq: f64) -> f32 {
        let d = d.as_dvec3() * freq;
        f.get([d.x, d.y, d.z]) as f32
    }

    pub fn column(&self, d: Vec3) -> Column {
        let d = d.normalize();
        // Broad, gentle plains and hills...
        let rough = self.knobs.roughness as f64;
        let base = (Self::sample(&self.height, d, 1.4 * rough) * 1.5).clamp(-1.0, 1.0) * self.amp;
        // ...and mountain ranges only where the range mask says so: sharp ridged crests that
        // rise smoothly out of the plains.
        // `ranges` slides the mask's threshold: 0 leaves almost none, 1 almost everywhere.
        let lo = 0.05 + (0.5 - self.knobs.ranges) * 0.9;
        let mask = smoothstep(lo, lo + 0.4, Self::sample(&self.ranges, d, 1.1));
        let ridge = 1.0 - Self::sample(&self.ridges, d, 3.2 * rough.sqrt()).abs();
        let detail = 1.0 - Self::sample(&self.ridges, d, 7.0 * rough).abs();
        let peak = mask * (ridge * ridge * 0.8 + detail * detail * ridge * 0.2);
        // Ocean basins where the continents noise runs low, their shelves sloping down from the
        // coast; the land stands a little over the sea, with lakes in its lowest hollows.
        let cont = Self::sample(&self.continents, d, 0.8);
        let ocean = if self.coverage > 0.0 {
            smoothstep(0.0, 0.28, (self.coverage - 1.0) * 0.22 - 0.03 - cont)
        } else {
            0.0
        };
        let lift = if self.sea == block::AIR {
            0.0
        } else {
            self.amp * 0.6 + 1.0
        };
        let mut h = base * (1.0 - 0.5 * ocean)
            + lift * (1.0 - ocean)
            + peak * self.mountains * (1.0 - 0.8 * ocean)
            - ocean * self.ocean_depth;
        // Rivers: thin bands of noise cut valleys down to the sea's level and a channel under it,
        // through the lowlands (they die out climbing into the mountains).
        let wet = self.sea == block::WATER;
        if wet && self.knobs.rivers > 0.0 {
            let w = 0.034 * self.knobs.rivers.sqrt();
            let r = Self::sample(&self.rivers, d, 2.4 * rough.sqrt()).abs();
            let fade = (1.0 - smoothstep(0.08, 0.3, peak)) * (1.0 - ocean);
            let valley = (1.0 - r / (w * 4.0)).max(0.0);
            h = h + (h.min(SEA_LEVEL as f32 + 1.5) - h) * valley * valley * fade;
            let bank = (1.0 - r / w).max(0.0);
            let bed = SEA_LEVEL as f32 - 1.0 - bank * 2.5;
            h = h + (h.min(bed) - h) * smoothstep(0.0, 0.5, bank) * fade;
        }
        let height = h.round() as i64;

        let lofty = peak; // 0 on plains, ~1 on the crests
        let biome = match self.kind {
            Biome::Barren | Biome::Crystal => self.kind,
            _ => {
                let lat = d.dot(self.axis);
                let warmth = self.base_warmth - lat * lat * 0.75
                    + Self::sample(&self.warmth, d, 2.0) * 0.2
                    - lofty * 0.35;
                let moisture =
                    Self::sample(&self.moisture, d, 2.5) * 0.5 + 0.5 + self.knobs.moisture * 0.4;
                if warmth < 0.2 {
                    Biome::Frozen
                } else if warmth > 0.7 && moisture < 0.55 {
                    Biome::Desert
                } else if moisture > 0.55 {
                    Biome::Forest
                } else {
                    Biome::Meadow
                }
            }
        };
        let natural = biome.layers().0;
        let top = match self.kind {
            _ if self.knobs.top != block::AIR => self.knobs.top,
            Biome::Barren | Biome::Crystal => natural,
            _ if lofty > 0.62 => block::SNOW, // snowy peaks
            _ if lofty > 0.38 && biome != Biome::Frozen => block::STONE, // bare rocky slopes
            _ => natural,
        };
        let soil = ((Self::sample(&self.soil, d, 9.0) * 2.5 + 3.2).clamp(0.0, 6.0)
            * self.knobs.soil_depth) as usize;
        let liquid = if height < SEA_LEVEL {
            self.sea
        } else {
            block::AIR
        };
        // Sea floors: sand in the shallows, gravel deeper (basalt under lava); sandy beaches.
        let (top, bed) = if liquid == block::LAVA {
            (block::BASALT, false)
        } else if liquid == block::WATER && self.knobs.top != block::AIR {
            (self.knobs.top, false)
        } else if liquid == block::WATER {
            let n = Self::sample(&self.soil, d, 23.0);
            (
                if SEA_LEVEL - height <= 3 || (n > 0.25 && SEA_LEVEL - height <= 8) {
                    block::SAND
                } else {
                    block::GRAVEL
                },
                true,
            )
        } else if wet
            && height <= SEA_LEVEL + 1
            && lofty < 0.3
            && biome != Biome::Frozen
            && self.knobs.top == block::AIR
        {
            (block::SAND, true)
        } else {
            (top, false)
        };
        let frozen = liquid == block::WATER && biome == Biome::Frozen;
        Column {
            height,
            liquid,
            frozen,
            bed,
            biome,
            top,
            rocky: lofty > 0.38 && !bed,
            soil,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ChunkKey {
    pub face: u8,
    pub ci: u16,
    pub cj: u16,
}

pub struct RayHit {
    pub cell: CellId,
    /// Last empty cell before the hit, where a block would be placed.
    pub before: Option<CellId>,
    /// Where the ray went in (planet-local).
    pub point: Vec3,
}

pub struct Planet {
    pub spec: PlanetSpec,
    pub grid: CubeSphere,
    blocks: Vec<Block>,
    /// Sky light per cell, 0..=light::MAX (see `light`).
    pub(crate) light: Vec<u8>,
    /// Cells holding blocks the player put there (not generated): what a rocket is built of.
    placed: HashSet<CellId>,
    /// Water and lava waiting to flow (see `liquid`).
    flow: Flow<CellId>,
}

impl Planet {
    pub fn generate(spec: PlanetSpec) -> Self {
        let depth = spec.depth();
        let layers = depth + spec.max_height().ceil() as usize + HEADROOM;
        let n = (std::f32::consts::FRAC_PI_2 * spec.radius).round().max(4.0) as usize;
        let grid = CubeSphere::new(n, spec.radius - depth as f32, layers);
        let mut blocks = vec![block::AIR; grid.cell_count()];
        let terrain = Terrain::new(&spec);
        let columns = 6 * n * n;
        // The deepest third of the crust is deepslate.
        let deep_top = depth / 3;

        // Ground: bedrock, deepslate, stone, a run of soil, the top block.
        let mut biomes = Vec::with_capacity(columns);
        let mut seas = Vec::with_capacity(columns);
        for f in 0..6 {
            for i in 0..n {
                for j in 0..n {
                    let col = terrain.column(grid.center(grid.index(f, i, j, 0)));
                    let (top, sub, deep) = terrain.layers(col.biome);
                    let surface = (depth as i64 + col.height).clamp(2, layers as i64 - 1) as usize; // layers below are solid
                    let stone = |k: usize| {
                        if deep == block::STONE && k <= deep_top {
                            block::DEEPSLATE
                        } else {
                            deep
                        }
                    };
                    for k in 0..surface {
                        let below = surface - 1 - k; // 0: the top block
                        let b = if k == 0 {
                            block::CORE
                        } else if below == 0 {
                            if col.soil == 0 && col.top == top && top != block::SAND {
                                block::STONE
                            } else {
                                col.top
                            }
                        } else if below <= col.soil {
                            if col.bed && below <= 2 {
                                col.top
                            } else if col.rocky {
                                block::STONE
                            } else if sub == block::SANDSTONE && below <= 2 {
                                block::SAND
                            } else {
                                sub
                            }
                        } else {
                            stone(k)
                        };
                        blocks[grid.index(f, i, j, k)] = b;
                    }
                    biomes.push(col.biome);
                    seas.push(col);
                }
            }
        }

        // Caves, then each column's ground top again (caves open some to the sky).
        for c in caves::carve(
            &grid,
            depth,
            &blocks,
            spec.seed,
            spec.knobs.caves,
            spec.knobs.cave_size,
        ) {
            blocks[c] = block::AIR;
        }
        let mut tops = vec![0usize; columns];
        for (col, t) in tops.iter_mut().enumerate() {
            let base = col * layers;
            let mut k = layers - 1;
            while k > 0 && blocks[base + k] == block::AIR {
                k -= 1;
            }
            *t = k;
            // Soil a cave left bare under the sky grows its biome's top again.
            let (top, sub, _) = terrain.layers(biomes[col]);
            if blocks[base + k] == sub && sub != top && block::tinted(top) && !seas[col].is_wet() {
                blocks[base + k] = top;
            }
        }

        // Seas, lakes and rivers: liquid over every low column up to the sea's level (down any
        // cave mouth on the bottom too), frozen over where it is cold.
        let sea_top = (depth as i64 + SEA_LEVEL).clamp(1, layers as i64 - 1) as usize; // first dry layer
        for (col, c) in seas.iter().enumerate() {
            if !c.is_wet() {
                continue;
            }
            let base = col * layers;
            for k in (tops[col] + 1)..sea_top {
                blocks[base + k] = if c.frozen && k + 1 == sea_top {
                    block::ICE
                } else {
                    c.liquid
                };
            }
        }
        // Deep down, the lowest caves are lakes of lava, like Minecraft's.
        let lava_top = (depth / 8).clamp(2, 5);
        for col in 0..columns {
            for k in 1..=lava_top {
                if blocks[col * layers + k] == block::AIR {
                    blocks[col * layers + k] = block::LAVA;
                }
            }
        }

        // Ore veins in the stone and deepslate, deeper ores further down.
        let mut rng = Rng::new(spec.seed ^ 0x0E5);
        let crust = columns as f32 * (depth - 1) as f32;
        for ore in &block::ORES {
            let veins = (crust / 1000.0 * ore.rate * spec.knobs.ores) as usize;
            for _ in 0..veins {
                let col = rng.int(0, columns as i64 - 1) as usize;
                let top = tops[col].min(depth + 4);
                let f = rng.range(ore.from, ore.to);
                let k =
                    ((top as f32 - f * (top as f32 - 1.0)).round() as usize).clamp(1, layers - 1);
                let mut cell = col * layers + k;
                for _ in 0..ore.size {
                    let b = blocks[cell];
                    if b == block::DEEPSLATE {
                        blocks[cell] = ore.deep;
                    } else if b != block::AIR
                        && b != block::CORE
                        && b != block::DEEPSLATE
                        && block::is_solid(b)
                        && cell % layers + 4 < tops[cell / layers]
                    {
                        blocks[cell] = ore.block; // veins run through whatever the rock is
                    }
                    match grid.neighbor(cell, Side::ALL[rng.int(0, 5) as usize]) {
                        Some(nb) if nb % layers > 0 => cell = nb,
                        _ => break,
                    }
                }
            }
        }

        // Trees first, then grass and flowers around them; only on the biome's own ground.
        let knobs = spec.knobs;
        let grounds: Vec<Block> = biomes.iter().map(|&b| terrain.layers(b).0).collect();
        let mut p = Self {
            spec,
            grid,
            blocks,
            light: Vec::new(),
            placed: HashSet::new(),
            flow: Flow::default(),
        };
        let mut rng = Rng::new(p.spec.seed ^ 0x7EE5);
        let mut small = Vec::new();
        for col in 0..columns {
            let (k, layers) = (tops[col], p.grid.layers);
            let biome = biomes[col];
            if k + 1 >= layers || p.blocks[col * layers + k] != grounds[col] || seas[col].is_wet() {
                continue;
            }
            let base = col * layers + k + 1;
            if let Some(kind) = trees::pick(biome, &mut rng, knobs.trees) {
                trees::grow(&mut p, base, kind, &mut rng);
            } else if let Some(b) = trees::pick_small(biome, &mut rng, knobs.grass, knobs.flowers) {
                small.push((base, b));
            }
        }
        for (c, b) in small {
            p.fill(c, b);
        }
        p.compute_light();
        p
    }

    /// A smooth test planet: every column solid below layer `ground`, air above.
    #[cfg(test)]
    pub(crate) fn flat(spec: PlanetSpec, ground: usize) -> Self {
        let depth = spec.depth();
        let layers = depth + spec.max_height().ceil() as usize + HEADROOM;
        let n = (std::f32::consts::FRAC_PI_2 * spec.radius).round().max(4.0) as usize;
        let grid = CubeSphere::new(n, spec.radius - depth as f32, layers);
        let blocks = (0..grid.cell_count())
            .map(|c| {
                if grid.coords(c).3 < ground {
                    block::STONE
                } else {
                    block::AIR
                }
            })
            .collect();
        let mut p = Self {
            spec,
            grid,
            blocks,
            light: Vec::new(),
            placed: HashSet::new(),
            flow: Flow::default(),
        };
        p.compute_light();
        p
    }

    /// Sets a block only where there is air (plants never cut into terrain or each other's trunks).
    pub(crate) fn fill(&mut self, c: CellId, b: Block) {
        if self.blocks[c] == block::AIR {
            self.blocks[c] = b;
        }
    }

    pub fn get(&self, c: CellId) -> Block {
        self.blocks[c]
    }

    /// True if the block in `c` was put there by the player (not generated).
    pub fn is_placed(&self, c: CellId) -> bool {
        self.placed.contains(&c)
    }

    /// Block at a planet-local point. Inside the core is solid, above the ceiling is air.
    pub fn block_at(&self, p: Vec3) -> Block {
        if p.length() < self.grid.core {
            return block::CORE;
        }
        self.grid.cell_at(p).map_or(block::AIR, |c| self.blocks[c])
    }

    pub fn solid_at(&self, p: Vec3) -> bool {
        block::is_solid(self.block_at(p))
    }

    pub fn opaque_at(&self, p: Vec3) -> bool {
        block::opaque(self.block_at(p))
    }

    /// Breaks a block, and with it what it held up (grass, flowers, torches, ladders, the other
    /// half of a door): the chunks to mesh again, and every block that went (where, what).
    pub fn break_block(&mut self, c: CellId) -> (Vec<ChunkKey>, Vec<(CellId, Block)>) {
        let was = self.get(c);
        let mut keys = self.set(c, block::AIR);
        let mut gone = vec![(c, was)];
        // The other half of a door goes with it.
        if let Some(q) = crate::shape::door_pair(self, c, was) {
            gone.push((q, self.get(q)));
            merge(&mut keys, self.set(q, block::AIR));
        }
        let (more, fell) = self.settle(c);
        merge(&mut keys, more);
        gone.extend(fell);
        if let Some(&(q, _)) = gone.get(1) {
            merge(&mut keys, self.settle(q).0);
        }
        (keys, gone)
    }

    /// Takes away whatever lost its support around `c` (and what that held up in turn).
    pub fn settle(&mut self, c: CellId) -> (Vec<ChunkKey>, Vec<(CellId, Block)>) {
        let (mut keys, mut gone) = (Vec::new(), Vec::new());
        let mut todo = vec![c];
        while let Some(p) = todo.pop() {
            for q in crate::shape::unsupported(self, p) {
                gone.push((q, self.get(q)));
                merge(&mut keys, self.set(q, block::AIR));
                todo.push(q);
            }
        }
        (keys, gone)
    }

    /// A random tick (as Minecraft's) on cell `c`: grass spreading over dirt near it under the
    /// open sky, or dying back under something opaque. The chunks changed.
    pub fn random_tick(&mut self, c: CellId) -> Vec<ChunkKey> {
        let b = self.blocks[c];
        if let Some(next) = crate::crop::next(b) {
            return self.put_block(c, next);
        }
        if b != block::GRASS && b != block::DIRT {
            return Vec::new();
        }
        let Some(up) = self.grid.neighbor(c, Side::Top) else {
            return Vec::new();
        };
        let lit = self.light(up).max(self.block_light(up)) >= 9;
        let near = b == block::DIRT && crate::shape::grass_near(self, c);
        match crate::shape::grass_tick(b, self.blocks[up], lit, near) {
            Some(to) => self.put_block(c, to),
            None => Vec::new(),
        }
    }

    /// Sets a block and returns the chunks whose meshes changed (its own, its neighbors', and
    /// those whose light changed). Liquids around it get to flow.
    pub fn set(&mut self, c: CellId, b: Block) -> Vec<ChunkKey> {
        if b == block::AIR || block::is_liquid(b) {
            self.placed.remove(&c);
        } else {
            self.placed.insert(c);
        }
        let keys = self.put_block(c, b);
        let mut flow = std::mem::take(&mut self.flow);
        flow.touch(self, c);
        self.flow = flow;
        keys
    }

    /// Lets the liquids flow one tick (`liquid::TICK`); the chunks whose meshes changed.
    pub fn tick_liquids(&mut self) -> Vec<ChunkKey> {
        if self.flow.is_idle() {
            return Vec::new();
        }
        let mut flow = std::mem::take(&mut self.flow);
        let mut basin = PlanetBasin {
            planet: self,
            dirty: HashSet::new(),
        };
        flow.tick(&mut basin, liquid::BUDGET);
        let dirty = basin.dirty;
        self.flow = flow;
        dirty.into_iter().collect()
    }

    /// Where a planet-local point is in cell `c`: fractions (u along i, v along j, w up).
    pub fn fractions(&self, c: CellId, p: Vec3) -> (f32, f32, f32) {
        let (face, i, j, k) = self.grid.coords(c);
        let (a, b) = self.grid.face_coords(face, p.as_dvec3());
        (
            (a - i as f64) as f32,
            (b - j as f64) as f32,
            p.length() - self.grid.radius(k),
        )
    }

    /// Liquid cells waiting to flow.
    pub fn flowing(&self) -> usize {
        self.flow.pending()
    }

    /// Sets a block and relights; the chunks whose meshes changed.
    fn put_block(&mut self, c: CellId, b: Block) -> Vec<ChunkKey> {
        self.blocks[c] = b;
        let mut cells = self.relight(c);
        cells.push(c);
        let mut keys = Vec::new();
        for cell in cells {
            // Faces sample the light of the cells in front of them, which may sit in a neighbor chunk.
            let around = Side::ALL
                .into_iter()
                .filter_map(|s| self.grid.neighbor(cell, s));
            for x in std::iter::once(cell).chain(around) {
                let k = self.chunk_of(x);
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    }

    pub fn chunks_per_side(&self) -> usize {
        self.grid.n.div_ceil(CHUNK)
    }

    pub fn chunk_of(&self, c: CellId) -> ChunkKey {
        let (f, i, j, _) = self.grid.coords(c);
        ChunkKey {
            face: f as u8,
            ci: (i / CHUNK) as u16,
            cj: (j / CHUNK) as u16,
        }
    }

    pub fn all_chunks(&self) -> impl Iterator<Item = ChunkKey> + '_ {
        let m = self.chunks_per_side() as u16;
        (0..6u8).flat_map(move |face| {
            (0..m).flat_map(move |ci| (0..m).map(move |cj| ChunkKey { face, ci, cj }))
        })
    }

    /// Cells of a chunk, column by column.
    pub fn chunk_cells(&self, key: ChunkKey) -> impl Iterator<Item = CellId> + '_ {
        let n = self.grid.n;
        let (i0, j0) = (key.ci as usize * CHUNK, key.cj as usize * CHUNK);
        (i0..(i0 + CHUNK).min(n)).flat_map(move |i| {
            (j0..(j0 + CHUNK).min(n)).flat_map(move |j| {
                (0..self.grid.layers).map(move |k| self.grid.index(key.face as usize, i, j, k))
            })
        })
    }

    /// Steps along a ray (planet-local) and returns the first cell holding anything (plants and
    /// torches too), breakable or not.
    pub fn raycast(&self, origin: Vec3, dir: Vec3, reach: f32) -> Option<RayHit> {
        const STEP: f32 = 0.02;
        let mut before = None;
        let mut t = 0.0;
        while t <= reach {
            let p = origin + dir * t;
            if let Some(c) = self.grid.cell_at(p) {
                let b = self.blocks[c];
                if block::targetable(b) {
                    // Into its shape (a door's board, a flower's tuft), or past it.
                    let (u, v, w) = self.fractions(c, p);
                    if crate::shape::in_outline(b, crate::shape::joined(self, c), u, v, w) {
                        return Some(RayHit {
                            cell: c,
                            before,
                            point: p,
                        });
                    }
                } else {
                    before = Some(c);
                }
            } else if p.length() < self.grid.core {
                return None;
            }
            t += STEP;
        }
        None
    }
}

impl crate::shape::Around for Planet {
    type Pos = CellId;
    fn at(&self, p: CellId) -> Block {
        self.blocks[p]
    }
    fn side(&self, p: CellId, d: block::Dir) -> Option<CellId> {
        self.grid.neighbor(
            p,
            [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus][d.0 as usize & 3],
        )
    }
    fn above(&self, p: CellId) -> Option<CellId> {
        self.grid.neighbor(p, Side::Top)
    }
    fn below(&self, p: CellId) -> Option<CellId> {
        self.grid.neighbor(p, Side::Bottom)
    }
}

/// Adds chunks not there yet.
fn merge(keys: &mut Vec<ChunkKey>, more: Vec<ChunkKey>) {
    for k in more {
        if !keys.contains(&k) {
            keys.push(k);
        }
    }
}

impl Cells for Planet {
    type Pos = CellId;
    fn block(&self, p: CellId) -> Block {
        self.blocks[p]
    }
    fn down(&self, p: CellId) -> Option<CellId> {
        self.grid.neighbor(p, Side::Bottom)
    }
    fn up(&self, p: CellId) -> Option<CellId> {
        self.grid.neighbor(p, Side::Top)
    }
    fn sides(&self, p: CellId) -> [Option<CellId>; 4] {
        [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus].map(|s| self.grid.neighbor(p, s))
    }
}

/// A planet as liquids change it, gathering the chunks to mesh again.
struct PlanetBasin<'a> {
    planet: &'a mut Planet,
    dirty: HashSet<ChunkKey>,
}

impl Cells for PlanetBasin<'_> {
    type Pos = CellId;
    fn block(&self, p: CellId) -> Block {
        self.planet.blocks[p]
    }
    fn down(&self, p: CellId) -> Option<CellId> {
        self.planet.down(p)
    }
    fn up(&self, p: CellId) -> Option<CellId> {
        self.planet.up(p)
    }
    fn sides(&self, p: CellId) -> [Option<CellId>; 4] {
        self.planet.sides(p)
    }
}

impl Basin for PlanetBasin<'_> {
    fn put(&mut self, p: CellId, b: Block) {
        // Never over bedrock, or into the top layer (nothing above it to flow from).
        if self.planet.blocks[p] == block::CORE {
            return;
        }
        self.planet.placed.remove(&p);
        self.dirty.extend(self.planet.put_block(p, b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> PlanetSpec {
        PlanetSpec::new(7, "t".into(), 24.0, Biome::Meadow)
    }

    #[test]
    fn a_moon_refits_when_its_planet_grows() {
        let u = crate::universe::Universe::new(9);
        let sys = u.system(glam::IVec3::ZERO).unwrap();
        let mut planets = sys.planets.clone();
        let t = 1234.5;
        for p in &mut planets {
            p.advance(t);
        }
        let k = planets.iter().position(|p| p.is_moon()).unwrap();
        let host = planets
            .iter()
            .position(|p| p.id == planets[k].host.unwrap().id)
            .unwrap();
        let before = (
            planets[k].center - planets[host].center,
            planets[k].rotation,
            planets[k].orbit.speed,
            planets[k].host.unwrap().soi,
        );
        let mut c = Customization::of(&planets[host]);
        c.radius = MAX_RADIUS;
        c.apply(&mut planets[host], t);
        refit_moons(&mut planets, t);
        let m = &planets[k];
        // Where it was, turned as it was; faster round a heavier planet, and its pull smaller.
        assert!((m.center - planets[host].center - before.0).length() < 1e-3);
        assert!(m.rotation.angle_between(before.1) < 1e-3);
        assert!(m.orbit.speed.abs() > before.2.abs());
        assert!(m.host.unwrap().soi < before.3);
        // Still locked.
        assert!((m.spin.speed - m.orbit.speed.abs()).abs() < 1e-6);
    }

    #[test]
    fn surface_is_near_radius_everywhere() {
        let p = Planet::generate(spec());
        let amp = p.spec.max_height() + 14.0; // mountains and trees
        for d in [
            Vec3::X,
            Vec3::NEG_Y,
            Vec3::new(1.0, 1.0, 1.0).normalize(),
            Vec3::new(-0.3, 0.2, -0.9).normalize(),
        ] {
            let mut r = p.spec.ceiling();
            while !p.solid_at(d * r) {
                r -= 0.1;
            }
            assert!((r - p.spec.radius).abs() <= amp, "dir {d} surface {r}");
        }
    }

    #[test]
    fn raycast_hits_and_places_outside() {
        let mut p = Planet::generate(spec());
        let o = Vec3::Y * p.spec.ceiling();
        let hit = p.raycast(o, Vec3::NEG_Y, 200.0).expect("hit");
        let before = hit.before.expect("air above ground");
        assert!(block::targetable(p.get(hit.cell)));
        assert_eq!(p.get(before), block::AIR);
        let dirty = p.set(before, block::PLANKS);
        assert!(!dirty.is_empty());
        assert_eq!(p.get(before), block::PLANKS);
    }

    #[test]
    fn customization_reshapes_and_keeps_the_spin_angle() {
        let mut s = spec();
        s.spin = Spin {
            axis: Vec3::Y,
            speed: 0.01,
            phase: 0.3,
        };
        let t = 1234.5;
        s.advance(t);
        let before = s.rotation;
        let mut c = Customization::of(&s);
        assert!((c.day - std::f32::consts::TAU / 0.01).abs() < 0.1);
        c.radius = 999.0;
        c.knobs.mountains = 1.5;
        c.tilt = 40.0;
        c.day = 300.0;
        c.seed = 99;
        c.apply(&mut s, t);
        assert_eq!(s.radius, MAX_RADIUS);
        assert_eq!(s.seed, 99);
        assert!((Customization::of(&s).tilt - 40.0).abs() < 0.1);
        let _ = before;
        assert!(s.gravity_radius() <= PlanetSpec::max_gravity_radius());
        // A new seed gives new terrain; the same seed, the same terrain.
        let col = |s: &PlanetSpec| Terrain::new(s).column(Vec3::new(0.3, 0.8, -0.2)).height;
        let mut a = spec();
        let heights: Vec<i64> = (0..6)
            .map(|k| {
                a.seed = k;
                col(&a)
            })
            .collect();
        assert!(heights.iter().any(|&h| h != heights[0]));
        a.seed = 3;
        assert_eq!(col(&a), heights[3]);
    }

    #[test]
    fn ground_has_caves_ores_deepslate_and_plants() {
        let p = Planet::generate(PlanetSpec::new(21, "g".into(), 90.0, Biome::Meadow));
        let count = |b: Block| p.blocks.iter().filter(|&&x| x == b).count();
        let (layers, depth) = (p.grid.layers, p.spec.depth());
        // Air below the mean surface, under rock: caves.
        let caves = (0..p.grid.cell_count())
            .filter(|&c| c % layers > 0 && c % layers < depth - 6 && p.blocks[c] == block::AIR)
            .count();
        assert!(caves > 300, "caves {caves}");
        assert!(count(block::COAL_ORE) + count(block::DEEP_COAL_ORE) > 100);
        assert!(count(block::IRON_ORE) + count(block::DEEP_IRON_ORE) > 30);
        assert!(count(block::DEEPSLATE) > 1000);
        assert!(
            count(block::TALL_GRASS) > 100,
            "grass {}",
            count(block::TALL_GRASS)
        );
        assert!(count(block::POPPY) + count(block::DANDELION) + count(block::CORNFLOWER) > 10);
        assert!(
            (0..p.grid.cell_count() / layers).all(|col| p.blocks[col * layers] == block::CORE),
            "bedrock floor"
        );
        let mut flat = PlanetSpec::new(21, "g".into(), 90.0, Biome::Meadow);
        flat.knobs.caves = 0.0;
        let q = Planet::generate(flat);
        assert_eq!(
            (0..q.grid.cell_count())
                .filter(|&c| c % layers > 0 && c % layers < depth - 6 && q.blocks[c] == block::AIR)
                .count(),
            0
        );
    }

    #[test]
    fn knobs_reshape_the_ground() {
        let mut spec = PlanetSpec::new(4, "k".into(), 80.0, Biome::Meadow);
        spec.knobs.top = block::SAND;
        spec.knobs.stone = block::BASALT;
        spec.knobs.trees = 0.0;
        spec.knobs.grass = 0.0;
        spec.knobs.flowers = 0.0;
        spec.knobs.caves = 0.0;
        let p = Planet::generate(spec);
        let layers = p.grid.layers;
        let mut tops = std::collections::HashMap::new();
        for col in 0..p.grid.cell_count() / layers {
            let ground = |b: Block| b != block::AIR && !block::is_liquid(b) && b != block::ICE;
            let k = (0..layers)
                .rev()
                .find(|&k| ground(p.get(col * layers + k)))
                .unwrap();
            *tops.entry(p.get(col * layers + k)).or_insert(0) += 1;
        }
        assert_eq!(tops.len(), 1, "only sand on top: {tops:?}");
        assert!(tops.contains_key(&block::SAND));
        assert!(p.blocks.iter().filter(|&&b| b == block::BASALT).count() > 10_000);
        assert_eq!(
            p.blocks
                .iter()
                .filter(|&&b| b == block::TALL_GRASS || b == block::WOOD)
                .count(),
            0
        );
    }

    #[test]
    fn living_worlds_have_seas_rivers_and_beaches() {
        let p = Planet::generate(PlanetSpec::new(7, "s".into(), 150.0, Biome::Meadow));
        let terrain = Terrain::new(&p.spec);
        let g = &p.grid;
        let columns = g.cell_count() / g.layers;
        let cols: Vec<Column> = (0..columns)
            .map(|col| terrain.column(g.center(col * g.layers)))
            .collect();
        let wet = cols.iter().filter(|c| c.is_wet()).count() as f32 / columns as f32;
        assert!((0.2..0.7).contains(&wet), "seas cover {:.0}%", wet * 100.0);
        assert!(
            cols.iter().filter(|c| c.height < -8).count() > columns / 20,
            "the seas are deep in places"
        );
        let sea = p.blocks.iter().filter(|&&b| b == block::WATER).count();
        assert!(sea > columns / 5, "{sea} water cells");
        assert!(
            p.blocks.iter().filter(|&&b| b == block::SAND).count() > columns / 20,
            "beaches and shallows"
        );
        // Water stands still where it was made: nothing is flowing yet.
        assert!(p.flow.is_idle());
        // The sea's surface is level: every wet column's water reaches the sea's level, no higher.
        let top = p.spec.depth() as i64 + SEA_LEVEL;
        for (col, c) in cols.iter().enumerate().step_by(37) {
            if c.is_wet() {
                let b = p.blocks[col * g.layers + top as usize - 1];
                assert!(
                    b == block::WATER || b == block::ICE,
                    "column {col}: {}",
                    block::name(b)
                );
                assert!(!block::is_liquid(p.blocks[col * g.layers + top as usize]));
            }
        }
        // Lava lies in the deepest caves.
        assert!(p.blocks.iter().any(|&b| b == block::LAVA));
        // A barren world has seas of lava, a crystal one none.
        let b = Planet::generate(PlanetSpec::new(8, "b".into(), 90.0, Biome::Barren));
        let lava = b.blocks.iter().filter(|&&x| x == block::LAVA).count();
        assert!(
            lava > 1000 && !b.blocks.contains(&block::WATER),
            "{lava} lava"
        );
    }

    #[test]
    fn generation_is_deterministic() {
        let a = Planet::generate(spec());
        let b = Planet::generate(spec());
        assert!(a.blocks == b.blocks);
    }
}
