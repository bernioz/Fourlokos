//! Procedural, seeded space backdrop rendered as a cubemap `Skybox`.
//!
//! Everything is a pure function of the seed: a galactic band (great circle with clumps, dust
//! lanes and a brighter core), soft domain-warped nebulae with seeded hues, and a dense star
//! field that thickens along the band. The low-frequency field is evaluated on a coarse
//! per-face grid whose edge samples are shared between faces (so it is seamless), then
//! upsampled; stars are splatted at full resolution onto every face they touch.

use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    light::Skybox,
    math::Vec3,
    prelude::*,
    render::render_resource::{
        Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
    },
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

/// Seed of the sky. Changing it regenerates the backdrop.
#[derive(Resource, Clone, Copy, Debug)]
pub struct SkySeed(pub u64);

/// Cameras tagged with this get the generated `Skybox`.
#[derive(Component, Default)]
pub struct SkyCamera;

pub struct SkyPlugin;

impl Plugin for SkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (start_generation, finish_generation, attach_skybox).chain(),
        );
    }
}

/// Cubemap face resolution in texels.
pub const FACE_SIZE: usize = 1024;
/// `Skybox::brightness`; with Bevy's default camera exposure this maps texel values ~1:1.
pub const SKY_BRIGHTNESS: f32 = 1000.0;
/// Coarse grid cells per face edge for the nebula/band field.
const GRID: usize = 192;

#[derive(Component)]
struct SkyTask(Task<Image>);

#[derive(Resource)]
struct SkyCubemap(Handle<Image>);

fn start_generation(
    mut commands: Commands,
    seed: Option<Res<SkySeed>>,
    tasks: Query<Entity, With<SkyTask>>,
) {
    let Some(seed) = seed else { return };
    if !seed.is_changed() {
        return;
    }
    for e in &tasks {
        commands.entity(e).despawn(); // dropping a Task cancels it
    }
    let seed = seed.0;
    let task = AsyncComputeTaskPool::get().spawn(async move {
        let t = std::time::Instant::now();
        let image = build_image(seed, FACE_SIZE);
        info!(
            "sky: seed {seed} generated in {:.0} ms",
            t.elapsed().as_secs_f32() * 1000.0
        );
        image
    });
    commands.spawn(SkyTask(task));
}

fn finish_generation(
    mut commands: Commands,
    mut tasks: Query<(Entity, &mut SkyTask)>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<&mut Skybox, With<SkyCamera>>,
) {
    for (e, mut task) in &mut tasks {
        let Some(image) = check_ready(&mut task.0) else {
            continue;
        };
        commands.entity(e).despawn();
        let handle = images.add(image);
        for mut skybox in &mut cameras {
            skybox.image = Some(handle.clone());
        }
        commands.insert_resource(SkyCubemap(handle));
    }
}

fn attach_skybox(
    mut commands: Commands,
    cubemap: Option<Res<SkyCubemap>>,
    cameras: Query<Entity, (With<SkyCamera>, Without<Skybox>)>,
) {
    let Some(cubemap) = cubemap else { return };
    for e in &cameras {
        commands.entity(e).insert(Skybox {
            image: Some(cubemap.0.clone()),
            brightness: SKY_BRIGHTNESS,
            ..default()
        });
    }
}

/// Builds the Rgba16Float cube image (6 layers, +X -X +Y -Y +Z -Z).
pub fn build_image(seed: u64, size: usize) -> Image {
    let faces = generate_faces(seed, size);
    let mut data = Vec::with_capacity(faces.len() * 8);
    for px in &faces {
        for c in [px.x, px.y, px.z, 1.0] {
            data.extend_from_slice(&f16_bits(c).to_le_bytes());
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: size as u32,
            height: size as u32,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba16Float,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    // Explicit, so a nearest-filtered default sampler (common in voxel games) doesn't apply.
    image.sampler = ImageSampler::linear();
    image
}

// ---------------------------------------------------------------------------------------------
// Pure generator
// ---------------------------------------------------------------------------------------------

/// Face basis in wgpu cubemap layer order: direction = fwd + s * right + t * down, s,t in [-1, 1].
const FACES: [[Vec3; 3]; 6] = [
    [Vec3::X, Vec3::NEG_Z, Vec3::NEG_Y],
    [Vec3::NEG_X, Vec3::Z, Vec3::NEG_Y],
    [Vec3::Y, Vec3::X, Vec3::Z],
    [Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z],
    [Vec3::Z, Vec3::X, Vec3::NEG_Y],
    [Vec3::NEG_Z, Vec3::NEG_X, Vec3::NEG_Y],
];

struct Nebula {
    color: Vec3,
    offset: Vec3,
    scale: f32,
    strength: f32,
}

/// Everything the seed decides about the sky.
pub struct SkyParams {
    noise_seed: u32,
    band_normal: Vec3,
    core_dir: Vec3,
    band_tint: Vec3,
    clump_offset: Vec3,
    dust_offset: Vec3,
    nebulae: Vec<Nebula>,
}

impl SkyParams {
    pub fn from_seed(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        let band_normal = rng.unit();
        // A point on the band's great circle.
        let r = rng.unit();
        let core_dir = (r - band_normal * r.dot(band_normal))
            .normalize_or(band_normal.any_orthonormal_vector());
        let band_tint = hsv(0.05 + rng.f32() * 0.12, 0.15 + rng.f32() * 0.2, 1.0);
        let base_hue = rng.f32();
        let count = 2 + rng.below(2) as usize;
        let nebulae = (0..count)
            .map(|i| Nebula {
                color: hsv(
                    (base_hue + i as f32 * (0.18 + rng.f32() * 0.25)).fract(),
                    0.6 + rng.f32() * 0.3,
                    1.0,
                ),
                offset: rng.unit() * (20.0 + rng.f32() * 80.0),
                scale: 0.9 + rng.f32() * 1.2,
                strength: 0.025 + rng.f32() * 0.035,
            })
            .collect();
        Self {
            noise_seed: rng.next() as u32,
            band_normal,
            core_dir,
            band_tint,
            clump_offset: rng.unit() * 50.0,
            dust_offset: rng.unit() * 50.0,
            nebulae,
        }
    }

    /// Galactic band density in [0, ~1.5] (used for color and star density).
    fn band(&self, d: Vec3) -> f32 {
        let lat = d.dot(self.band_normal);
        let along = d.dot(self.core_dir).clamp(-1.0, 1.0).acos(); // angle from the core
        let core_long = (-(along / 1.3).powi(2)).exp();
        let thin = (-(lat / (0.10 + 0.06 * core_long)).powi(2)).exp();
        let wide = (-(lat / 0.35).powi(2)).exp();
        let bulge = (-(along / 0.32).powi(2) - (lat / 0.16).powi(2)).exp();
        thin * (0.35 + 0.65 * core_long) + 0.2 * wide * (0.4 + 0.6 * core_long) + 0.9 * bulge
    }

    /// Diffuse sky color (no stars) in direction `d` (unit), linear RGB.
    pub fn background(&self, d: Vec3) -> Vec3 {
        let s = self.noise_seed;
        let mut col = Vec3::new(0.0012, 0.0014, 0.0030);

        // Galactic band: clumpy light cut by dark dust lanes hugging the plane.
        let band = self.band(d);
        if band > 0.002 {
            let clump = smoothstep(0.35, 0.8, fbm(s, d * 3.5 + self.clump_offset, 5));
            let lat = d.dot(self.band_normal);
            let dust = smoothstep(0.48, 0.68, fbm(s ^ 0x9e37, d * 7.0 + self.dust_offset, 5))
                * (-(lat / 0.09).powi(2)).exp();
            let light = band * (0.35 + 0.9 * clump) * (1.0 - 0.85 * dust);
            col += self.band_tint * light * 0.03;
        }

        // Nebulae: domain-warped fbm thresholded into soft clouds, confined to a few regions
        // and favoring the galactic plane.
        let near_band = 0.35 + 0.65 * band.min(1.0);
        for (i, n) in self.nebulae.iter().enumerate() {
            let ns = s.wrapping_add(0x51ed27 * (i as u32 + 1));
            let region = smoothstep(0.52, 0.68, fbm(ns, d * 0.9 + n.offset * 0.5, 2)) * near_band;
            if region <= 0.0 {
                continue;
            }
            let p = d * n.scale * 2.2 + n.offset;
            let warp = Vec3::new(
                fbm(ns ^ 1, p * 0.7, 3),
                fbm(ns ^ 2, p * 0.7, 3),
                fbm(ns ^ 3, p * 0.7, 3),
            );
            let v = fbm(ns, p + warp * 2.2, 5);
            let cloud = smoothstep(0.38, 0.82, v);
            let core = smoothstep(0.6, 0.9, v);
            let tint = n.color.lerp(Vec3::ONE, core * 0.35);
            col += tint * region * (cloud * cloud * n.strength + core * n.strength * 0.8);
        }
        col
    }
}

/// Generates the 6 faces (+X -X +Y -Y +Z -Z), row-major, linear RGB.
pub fn generate_faces(seed: u64, size: usize) -> Vec<Vec3> {
    let p = SkyParams::from_seed(seed);
    let g = GRID.min(size).max(2);
    let mut out = vec![Vec3::ZERO; 6 * size * size];

    for (f, [fwd, right, down]) in FACES.iter().enumerate() {
        // Coarse field sampled at the grid corners, including the exact edges (shared with neighbors).
        let grid: Vec<Vec3> = (0..=g)
            .flat_map(|j| (0..=g).map(move |i| (i, j)))
            .map(|(i, j)| {
                let s = i as f32 / g as f32 * 2.0 - 1.0;
                let t = j as f32 / g as f32 * 2.0 - 1.0;
                p.background((*fwd + *right * s + *down * t).normalize())
            })
            .collect();
        let face = &mut out[f * size * size..(f + 1) * size * size];
        let k = g as f32 / size as f32;
        for y in 0..size {
            let gy = (y as f32 + 0.5) * k;
            let (j, ty) = ((gy as usize).min(g - 1), gy.fract());
            for x in 0..size {
                let gx = (x as f32 + 0.5) * k;
                let (i, tx) = ((gx as usize).min(g - 1), gx.fract());
                let a = grid[j * (g + 1) + i].lerp(grid[j * (g + 1) + i + 1], tx);
                let b = grid[(j + 1) * (g + 1) + i].lerp(grid[(j + 1) * (g + 1) + i + 1], tx);
                face[y * size + x] = a.lerp(b, ty);
            }
        }
    }

    splat_stars(&p, seed, size, &mut out);
    out
}

const STAR_COLORS: [Vec3; 7] = [
    Vec3::new(0.62, 0.74, 1.00),
    Vec3::new(0.80, 0.87, 1.00),
    Vec3::new(1.00, 1.00, 1.00),
    Vec3::new(1.00, 0.95, 0.84),
    Vec3::new(1.00, 0.86, 0.66),
    Vec3::new(1.00, 0.74, 0.52),
    Vec3::new(1.00, 0.62, 0.48),
];

fn splat_stars(p: &SkyParams, seed: u64, size: usize, out: &mut [Vec3]) {
    let mut rng = Rng::new(seed ^ 0x5a17_5eed_0000_0001);
    // Scale the count with resolution so density in texels stays similar.
    let target = 45_000 * size * size / (1024 * 1024) + 64;
    let mut placed = 0;
    while placed < target {
        let d = rng.unit();
        let density = 0.25 + 0.75 * p.band(d).min(1.0);
        if rng.f32() > density {
            continue;
        }
        placed += 1;
        let u = rng.f32();
        // Mostly faint; a rare few are bright.
        let mag = 0.015
            + 0.35 * u.powi(6)
            + if rng.f32() < 0.003 {
                1.5 + 3.0 * rng.f32()
            } else {
                0.0
            };
        let color = STAR_COLORS[rng.below(STAR_COLORS.len() as u32) as usize];
        let sigma = 0.45 + 0.35 * mag.min(2.0).sqrt();
        splat(out, size, d, color * mag, sigma, mag > 1.0);
    }
}

/// Adds a gaussian blob (plus a soft halo for bright stars) centered on direction `d` to every face it touches.
fn splat(out: &mut [Vec3], size: usize, d: Vec3, color: Vec3, sigma: f32, halo: bool) {
    let reach = if halo { 8.0 } else { (sigma * 2.5).ceil() };
    let margin = 2.0 * (reach + 1.0) / size as f32;
    for (f, [fwd, right, down]) in FACES.iter().enumerate() {
        let ma = d.dot(*fwd);
        if ma <= 0.0 {
            continue;
        }
        let (s, t) = (d.dot(*right) / ma, d.dot(*down) / ma);
        if s.abs() > 1.0 + margin || t.abs() > 1.0 + margin {
            continue;
        }
        let cx = (s + 1.0) * 0.5 * size as f32 - 0.5;
        let cy = (t + 1.0) * 0.5 * size as f32 - 0.5;
        let r = reach as i32;
        let inv2s2 = 1.0 / (2.0 * sigma * sigma);
        for y in (cy as i32 - r).max(0)..=(cy as i32 + r + 1).min(size as i32 - 1) {
            for x in (cx as i32 - r).max(0)..=(cx as i32 + r + 1).min(size as i32 - 1) {
                let r2 = (x as f32 - cx).powi(2) + (y as f32 - cy).powi(2);
                let mut w = (-r2 * inv2s2).exp();
                if halo {
                    w += 0.03 * (-r2 / 12.0).exp();
                }
                out[f * size * size + y as usize * size + x as usize] += color * w;
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Small helpers: RNG, value noise, color
// ---------------------------------------------------------------------------------------------

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x2545_f491_4f6c_dd1d)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn f32(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn below(&mut self, n: u32) -> u32 {
        ((self.next() >> 32) * n as u64 >> 32) as u32
    }
    fn unit(&mut self) -> Vec3 {
        let z = self.f32() * 2.0 - 1.0;
        let a = self.f32() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), z, r * a.sin())
    }
}

fn hash3(seed: u32, x: i32, y: i32, z: i32) -> f32 {
    let mut h = seed
        ^ (x as u32).wrapping_mul(0x8da6_b343)
        ^ (y as u32).wrapping_mul(0xd816_3841)
        ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 16)).wrapping_mul(0x7feb_352d);
    h = (h ^ (h >> 15)).wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 / (1u32 << 24) as f32
}

/// Trilinear value noise in [0, 1).
fn value_noise(seed: u32, p: Vec3) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec3::splat(3.0) - 2.0 * f);
    let (x, y, z) = (i.x as i32, i.y as i32, i.z as i32);
    let c = |dx, dy, dz| hash3(seed, x + dx, y + dy, z + dz);
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(c(0, 0, 0), c(1, 0, 0), u.x);
    let x10 = lerp(c(0, 1, 0), c(1, 1, 0), u.x);
    let x01 = lerp(c(0, 0, 1), c(1, 0, 1), u.x);
    let x11 = lerp(c(0, 1, 1), c(1, 1, 1), u.x);
    lerp(lerp(x00, x10, u.y), lerp(x01, x11, u.y), u.z)
}

/// Normalized fbm in [0, 1).
fn fbm(seed: u32, mut p: Vec3, octaves: u32) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    for o in 0..octaves {
        sum += amp * value_noise(seed.wrapping_add(o.wrapping_mul(0x68e3_1da4)), p);
        norm += amp;
        p = p * 2.03 + Vec3::new(17.1, -9.7, 3.3);
        amp *= 0.5;
    }
    sum / norm
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// HSV (all in [0, 1]) to linear-ish RGB.
fn hsv(h: f32, s: f32, v: f32) -> Vec3 {
    let k = |n: f32| {
        let k = (n + h * 6.0) % 6.0;
        v - v * s * (k.min(4.0 - k).clamp(0.0, 1.0))
    };
    Vec3::new(k(5.0), k(3.0), k(1.0))
}

/// f32 -> IEEE half bits (non-negative finite inputs; tiny values flush to zero).
fn f16_bits(v: f32) -> u16 {
    let bits = v.clamp(0.0, 60000.0).to_bits();
    let exp = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if exp <= 0 {
        return 0;
    }
    ((exp as u32) << 10 | (bits & 0x7f_ffff) >> 13) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_per_seed() {
        let dirs = [
            Vec3::X,
            Vec3::new(0.3, -0.5, 0.8).normalize(),
            Vec3::new(-0.7, 0.1, -0.2).normalize(),
        ];
        let (a, b, c) = (
            SkyParams::from_seed(42),
            SkyParams::from_seed(42),
            SkyParams::from_seed(43),
        );
        for d in dirs {
            assert_eq!(a.background(d), b.background(d));
        }
        assert_ne!(a.band_normal, c.band_normal);
        assert_eq!(generate_faces(7, 32), generate_faces(7, 32));
        assert_ne!(generate_faces(7, 32), generate_faces(8, 32));
    }

    #[test]
    fn half_conversion() {
        assert_eq!(f16_bits(1.0), 0x3c00);
        assert_eq!(f16_bits(0.5), 0x3800);
        assert_eq!(f16_bits(0.0), 0);
    }
}
