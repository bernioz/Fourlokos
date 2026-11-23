//! The block texture atlas. Every tile (`orbit_core::block::tile`) is painted here as 16×16 pixel
//! art, unless a PNG of its name is found in the texture folder: `$ORBIT_TEXTURES`, else
//! `./textures`, else the crate's `assets/textures`. Any square size works (all tiles are scaled
//! to the largest). `fourlokos --export-textures` writes the painted set there to start from.

use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use orbit_core::block::{self, tile, Shape};
use orbit_core::rng::mix;
use std::path::PathBuf;

const SIZE: usize = 16;

pub struct TexturesPlugin;

impl Plugin for TexturesPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreStartup, build_atlas);
    }
}

/// The atlas every voxel material samples.
#[derive(Resource, Clone)]
pub struct Atlas(pub Handle<Image>);

/// Item icons for the hotbar and the inventory, one `ICON` px square per block id, `ICON_COLUMNS`
/// to a row: whole blocks drawn as little cubes seen from above a corner (as Minecraft's), the
/// rest flat.
#[derive(Resource, Clone)]
pub struct Icons(pub Handle<Image>);

pub const ICON: usize = 32;
pub const ICON_COLUMNS: usize = 16;

impl Icons {
    /// Where an item's icon is in the image, pixels.
    pub fn rect(item: block::Block) -> Rect {
        let (x, y) = ((item as usize % ICON_COLUMNS * ICON) as f32, (item as usize / ICON_COLUMNS * ICON) as f32);
        Rect::new(x, y, x + ICON as f32, y + ICON as f32)
    }
}

type Rgba = [u8; 4];

/// Where PNGs are looked for (and exported to).
pub fn folder() -> PathBuf {
    if let Ok(p) = std::env::var("ORBIT_TEXTURES") {
        return p.into();
    }
    let local = PathBuf::from("textures");
    if local.is_dir() {
        return local;
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/textures")
}

/// Writes the painted tiles as PNGs, for editing.
pub fn export() -> std::io::Result<PathBuf> {
    let dir = std::env::var("ORBIT_TEXTURES").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("textures"));
    std::fs::create_dir_all(&dir)?;
    for (t, name) in tile::NAMES.iter().enumerate() {
        let px = paint(t as u16);
        let img = image::RgbaImage::from_fn(SIZE as u32, SIZE as u32, |x, y| image::Rgba(px[y as usize * SIZE + x as usize]));
        img.save(dir.join(format!("{name}.png"))).map_err(std::io::Error::other)?;
    }
    Ok(dir)
}

fn build_atlas(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    let dir = folder();
    // Tiles from PNGs where there are any.
    let mut loaded: Vec<Option<image::RgbaImage>> = Vec::new();
    for name in tile::NAMES {
        let path = dir.join(format!("{name}.png"));
        let img = path.is_file().then(|| image::open(&path)).and_then(|r| match r {
            Ok(i) => Some(i.to_rgba8()),
            Err(e) => {
                warn!("texture {}: {e}", path.display());
                None
            }
        });
        loaded.push(img);
    }
    let found = loaded.iter().flatten().count();
    let size = loaded.iter().flatten().map(|i| i.width().max(i.height()) as usize).max().unwrap_or(SIZE).max(SIZE);
    info!("textures: {found} of {} from {}, the rest painted; tiles {size}px", tile::COUNT, dir.display());

    let (w, h) = (tile::COLUMNS * size, tile::ROWS * size);
    let mut data = vec![0u8; w * h * 4];
    for t in 0..tile::COUNT {
        let (ox, oy) = ((t % tile::COLUMNS) * size, (t / tile::COLUMNS) * size);
        let painted = paint(t as u16);
        for y in 0..size {
            for x in 0..size {
                // Nearest-neighbor scaling either way keeps the pixel-art look.
                let px = match &loaded[t] {
                    Some(img) => img.get_pixel((x * img.width() as usize / size) as u32, (y * img.height() as usize / size) as u32).0,
                    None => painted[(y * SIZE / size) * SIZE + x * SIZE / size],
                };
                let i = ((oy + y) * w + ox + x) * 4;
                data[i..i + 4].copy_from_slice(&px);
            }
        }
    }
    let icons = icons(&data, w, size);
    let mut image = Image::new(
        Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    commands.insert_resource(Atlas(images.add(image)));
    commands.insert_resource(Icons(images.add(icons)));
}

/// Draws every item's icon from the atlas's pixels (`data`, `w` wide, tiles `size` px).
fn icons(data: &[u8], w: usize, size: usize) -> Image {
    let rows = block::COUNT.div_ceil(ICON_COLUMNS);
    let (iw, ih) = (ICON_COLUMNS * ICON, rows * ICON);
    let mut out = vec![0u8; iw * ih * 4];
    // A tile's texel at (u, v) in 0..1.
    let texel = |t: u16, u: f32, v: f32| -> Rgba {
        let (ox, oy) = ((t as usize % tile::COLUMNS) * size, (t as usize / tile::COLUMNS) * size);
        let (x, y) = (((u * size as f32) as usize).min(size - 1), ((v * size as f32) as usize).min(size - 1));
        let i = ((oy + y) * w + ox + x) * 4;
        [data[i], data[i + 1], data[i + 2], data[i + 3]]
    };
    for &item in &block::ITEMS {
        let (ox, oy) = (item as usize % ICON_COLUMNS * ICON, item as usize / ICON_COLUMNS * ICON);
        let mut put = |x: i32, y: i32, c: Rgba| {
            if c[3] > 0 && (0..ICON as i32).contains(&x) && (0..ICON as i32).contains(&y) {
                let i = ((oy + y as usize) * iw + ox + x as usize) * 4;
                out[i..i + 4].copy_from_slice(&c);
            }
        };
        let tiles = block::tiles(item);
        match block::shape(item) {
            Shape::Cube => {
                // Three faces, splatted texel by texel (oversampled so no pixel is missed).
                // Screen point of a corner (x, y, z) of the unit cube.
                let at = |x: f32, y: f32, z: f32| (16.0 + (x - z) * 13.0, 2.0 + (x + z) * 7.5 + (1.0 - y) * 14.5);
                let n = size * 3;
                for face in 0..3 {
                    for a in 0..n {
                        for b in 0..n {
                            let (u, v) = ((a as f32 + 0.5) / n as f32, (b as f32 + 0.5) / n as f32);
                            let ((sx, sy), t, k) = match face {
                                0 => (at(u, 1.0 - v, 1.0), tiles[1], 0.8),  // left: z = 1
                                1 => (at(1.0, 1.0 - v, 1.0 - u), tiles[1], 0.62), // right: x = 1
                                _ => (at(u, 1.0, v), tiles[0], 1.0),        // top
                            };
                            put(sx as i32, sy as i32, shade(texel(t, u, v), k));
                        }
                    }
                }
            }
            _ if block::is_door(item) => {
                // The door standing: its top half over its bottom half.
                for y in 0..ICON {
                    for x in 8..24 {
                        let (t, v) = if y < 16 { (tile::DOOR_TOP, y as f32 / 16.0) } else { (tile::DOOR_BOTTOM, (y - 16) as f32 / 16.0) };
                        put(x as i32, y as i32, texel(t, (x - 8) as f32 / 16.0, v));
                    }
                }
            }
            _ if item == block::FENCE => {
                // Two posts and two rails between them, in planks.
                for y in 2..32 {
                    for x in 0..32 {
                        let post = (6..11).contains(&x) || (21..26).contains(&x);
                        let rail = (11..21).contains(&x) && ((9..13).contains(&y) || (19..23).contains(&y));
                        if post || rail {
                            let k = if post && (x == 6 || x == 21) { 0.8 } else { 1.0 };
                            put(x, y, shade(texel(tile::PLANKS, x as f32 / 32.0, y as f32 / 32.0), k));
                        }
                    }
                }
            }
            _ => {
                // Flat: the tile, twice as big.
                for y in 0..ICON {
                    for x in 0..ICON {
                        put(x as i32, y as i32, texel(tiles[1], x as f32 / ICON as f32, y as f32 / ICON as f32));
                    }
                }
            }
        }
    }
    let mut image = Image::new(
        Extent3d { width: iw as u32, height: ih as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        out,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// A tiny per-tile RNG: the same pixels every run.
struct Px(u64);

impl Px {
    fn f(&mut self) -> f32 {
        self.0 = mix(self.0);
        (self.0 >> 40) as f32 / (1u64 << 24) as f32
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
}

fn rgb(r: u8, g: u8, b: u8) -> Rgba {
    [r, g, b, 255]
}

/// Scales a color's brightness.
fn shade(c: Rgba, k: f32) -> Rgba {
    [(c[0] as f32 * k).clamp(0.0, 255.0) as u8, (c[1] as f32 * k).clamp(0.0, 255.0) as u8, (c[2] as f32 * k).clamp(0.0, 255.0) as u8, c[3]]
}

const CLEAR: Rgba = [0, 0, 0, 0];

/// A base color with per-pixel brightness noise of ±`amount`.
fn noisy(rng: &mut Px, base: Rgba, amount: f32) -> [Rgba; 256] {
    std::array::from_fn(|_| shade(base, 1.0 + (rng.f() - 0.5) * 2.0 * amount))
}

/// Sprinkles clusters of `color` over `px`.
fn clusters(px: &mut [Rgba; 256], rng: &mut Px, color: Rgba, count: usize, size: usize) {
    for _ in 0..count {
        let (mut x, mut y) = ((rng.f() * 14.0) as i32 + 1, (rng.f() * 14.0) as i32 + 1);
        for _ in 0..size {
            px[(y as usize) * SIZE + x as usize] = shade(color, 0.85 + rng.f() * 0.3);
            x = (x + (rng.f() * 3.0) as i32 - 1).clamp(0, 15);
            y = (y + (rng.f() * 3.0) as i32 - 1).clamp(0, 15);
        }
    }
}

fn stone(rng: &mut Px) -> [Rgba; 256] {
    let mut px = noisy(rng, rgb(125, 125, 128), 0.08);
    clusters(&mut px, rng, rgb(104, 104, 108), 6, 4);
    px
}

fn deepslate(rng: &mut Px) -> [Rgba; 256] {
    let mut px = noisy(rng, rgb(78, 78, 84), 0.07);
    for y in (1..16).step_by(3) {
        let (start, len) = ((rng.f() * 10.0) as usize, 4 + (rng.f() * 6.0) as usize);
        for x in start..(start + len).min(16) {
            px[y * SIZE + x] = shade(px[y * SIZE + x], 0.75);
        }
    }
    px
}

/// Leaves: a body of `c`, darker clumps and a few gaps of deep shade.
fn leaves(rng: &mut Px, c: Rgba) -> [Rgba; 256] {
    let mut px = noisy(rng, c, 0.15);
    clusters(&mut px, rng, shade(c, 0.7), 10, 3);
    for p in px.iter_mut() {
        if rng.chance(0.08) {
            *p = shade(c, 0.45);
        }
    }
    px
}

/// Log end: rings around the middle in `wood`, bark `bark` at the rim.
fn rings(rng: &mut Px, wood: Rgba, bark: Rgba) -> [Rgba; 256] {
    std::array::from_fn(|i| {
        let (x, y) = ((i % SIZE) as f32 - 7.5, (i / SIZE) as f32 - 7.5);
        let r = x.abs().max(y.abs());
        if r > 6.6 {
            shade(bark, 0.9 + rng.f() * 0.2)
        } else {
            shade(wood, if (r as i32) % 2 == 0 { 1.0 } else { 0.86 } * (0.96 + rng.f() * 0.08))
        }
    })
}

/// Bark running up the tile in vertical streaks.
fn bark(rng: &mut Px, c: Rgba) -> [Rgba; 256] {
    let cols: [f32; 16] = std::array::from_fn(|_| 0.8 + rng.f() * 0.35);
    std::array::from_fn(|i| shade(c, cols[i % SIZE] * (0.94 + rng.f() * 0.12)))
}

/// A flower: green stem and leaves, a head of `petal` around a `heart`.
fn flower(rng: &mut Px, petal: Rgba, heart: Rgba) -> [Rgba; 256] {
    let mut px = [CLEAR; 256];
    let stem = rgb(60, 130, 40);
    for y in 7..16 {
        px[y * SIZE + 7] = shade(stem, 0.9 + rng.f() * 0.2);
    }
    for (x, y) in [(6, 11), (5, 10), (8, 12), (9, 11)] {
        px[y * SIZE + x] = stem;
    }
    for y in 3..8 {
        for x in 5..10 {
            let (dx, dy) = (x as i32 - 7, y as i32 - 5);
            if dx * dx + dy * dy <= 5 {
                px[y * SIZE + x] = shade(petal, 0.85 + rng.f() * 0.3);
            }
        }
    }
    px[5 * SIZE + 7] = heart;
    px
}

/// Smooth noise that tiles: a few waves whose periods divide the tile, in -1..1.
fn waves(x: usize, y: usize, terms: &[(f32, f32, f32)]) -> f32 {
    let (x, y) = (x as f32 / SIZE as f32 * std::f32::consts::TAU, y as f32 / SIZE as f32 * std::f32::consts::TAU);
    terms.iter().map(|&(kx, ky, ph)| (kx * x + ky * y + ph).sin()).sum::<f32>() / terms.len() as f32
}

/// Paints one tile.
pub fn paint(t: u16) -> [Rgba; 256] {
    let rng = &mut Px(mix(t as u64 + 0x7E57));
    let dirt_c = rgb(134, 96, 67);
    let grass_c = rgb(96, 160, 56);
    match t {
        tile::WHITE => [rgb(255, 255, 255); 256],
        tile::STONE => stone(rng),
        tile::DIRT => {
            let mut px = noisy(rng, dirt_c, 0.1);
            clusters(&mut px, rng, rgb(108, 76, 52), 7, 2);
            px
        }
        tile::GRASS_TOP => {
            let mut px = noisy(rng, grass_c, 0.13);
            clusters(&mut px, rng, rgb(80, 140, 45), 8, 3);
            px
        }
        tile::GRASS_SIDE => {
            let mut px = noisy(rng, dirt_c, 0.1);
            for x in 0..SIZE {
                let fringe = 3 + (rng.f() * 2.5) as usize;
                for y in 0..fringe {
                    px[y * SIZE + x] = shade(grass_c, 0.85 + rng.f() * 0.25);
                }
            }
            px
        }
        tile::SAND => noisy(rng, rgb(219, 207, 163), 0.05),
        tile::SNOW => noisy(rng, rgb(242, 248, 252), 0.025),
        tile::OAK_LOG => bark(rng, rgb(104, 82, 50)),
        tile::OAK_LOG_TOP => rings(rng, rgb(178, 144, 90), rgb(104, 82, 50)),
        tile::OAK_LEAVES => leaves(rng, rgb(64, 128, 40)),
        tile::PLANKS => std::array::from_fn(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let board = y / 4;
            let seam = (board * 7 + 3) % 16;
            let base = shade(rgb(162, 130, 78), 0.92 + ((mix(board as u64 + 99) % 100) as f32) * 0.0016);
            if y % 4 == 3 || x == seam { shade(base, 0.7) } else { shade(base, 0.95 + rng.f() * 0.1) }
        }),
        tile::CRYSTAL => std::array::from_fn(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let facet = (x + y) % 8 < 2 || (x + 16 - y) % 11 == 0;
            shade(rgb(120, 215, 250), if facet { 1.25 } else { 0.9 + rng.f() * 0.1 })
        }),
        tile::BASALT => std::array::from_fn(|i| shade(rgb(58, 55, 66), if i % SIZE % 4 == 0 { 0.75 } else { 0.95 + rng.f() * 0.12 })),
        tile::BIRCH_LOG => std::array::from_fn(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let mark = (mix(y as u64 * 31 + 5) % 5 == 0) && ((x + y * 3) % 7 < 3);
            if mark { rgb(40, 40, 38) } else { shade(rgb(218, 216, 208), 0.94 + rng.f() * 0.08) }
        }),
        tile::BIRCH_LOG_TOP => rings(rng, rgb(200, 180, 130), rgb(218, 216, 208)),
        tile::BIRCH_LEAVES => leaves(rng, rgb(112, 160, 62)),
        tile::PINE_LEAVES => leaves(rng, rgb(46, 92, 66)),
        tile::ACACIA_LEAVES => leaves(rng, rgb(132, 152, 52)),
        tile::CACTUS_SIDE => std::array::from_fn(|i| {
            let (x, y) = (i % SIZE, i / SIZE);
            let c = rgb(84, 140, 60);
            if x % 4 == 0 { shade(c, 0.72) } else if x % 4 == 2 && y % 4 == 1 { rgb(220, 220, 180) } else { shade(c, 0.95 + rng.f() * 0.1) }
        }),
        tile::CACTUS_TOP => rings(rng, rgb(110, 170, 80), rgb(84, 140, 60)),
        tile::SANDSTONE => std::array::from_fn(|i| shade(rgb(216, 200, 150), if (i / SIZE) % 5 == 4 { 0.86 } else { 0.97 + rng.f() * 0.06 })),
        tile::SANDSTONE_TOP => noisy(rng, rgb(222, 206, 158), 0.04),
        tile::DEEPSLATE => deepslate(rng),
        tile::GRAVEL => std::array::from_fn(|_| {
            let c = [rgb(130, 124, 122), rgb(100, 96, 96), rgb(150, 140, 132), rgb(115, 105, 98)][(rng.f() * 4.0) as usize];
            shade(c, 0.9 + rng.f() * 0.2)
        }),
        tile::COBBLESTONE => std::array::from_fn(|i| {
            // Stones: the nearest of a few fixed points; mortar where two are about as near.
            let (x, y) = ((i % SIZE) as f32, (i / SIZE) as f32);
            let pts = [(2.0, 2.0), (9.0, 1.0), (14.0, 5.0), (5.0, 7.0), (11.0, 9.0), (2.0, 12.0), (8.0, 14.0), (14.0, 14.0)];
            let mut d: Vec<f32> = pts.iter().map(|&(px, py): &(f32, f32)| ((x - px).powi(2) + (y - py).powi(2)).sqrt()).collect();
            d.sort_by(|a, b| a.total_cmp(b));
            if d[1] - d[0] < 1.1 { rgb(70, 70, 72) } else { shade(rgb(128, 128, 128), 0.85 + rng.f() * 0.25) }
        }),
        tile::BEDROCK => std::array::from_fn(|_| shade(rgb(85, 85, 85), 0.35 + rng.f() * 0.9)),
        t if (tile::ORE..tile::ORE + 6).contains(&t) || (tile::DEEP_ORE..tile::DEEP_ORE + 6).contains(&t) => {
            let deep = t >= tile::DEEP_ORE;
            let kind = (t - if deep { tile::DEEP_ORE } else { tile::ORE }) as usize;
            let color = [rgb(30, 30, 32), rgb(200, 112, 72), rgb(216, 175, 145), rgb(250, 214, 60), rgb(220, 24, 20), rgb(92, 236, 226)][kind];
            let mut px = if deep { deepslate(rng) } else { stone(rng) };
            clusters(&mut px, rng, color, 5, 4);
            px
        }
        tile::TORCH => {
            let mut px = [CLEAR; 256];
            for y in 6..16 {
                for x in 7..9 {
                    px[y * SIZE + x] = shade(rgb(130, 96, 52), 0.9 + rng.f() * 0.2);
                }
            }
            for x in 7..9 {
                px[6 * SIZE + x] = rgb(255, 236, 140);
                px[7 * SIZE + x] = rgb(255, 180, 60);
            }
            px
        }
        tile::TALL_GRASS => {
            let mut px = [CLEAR; 256];
            for x in 1..15 {
                if rng.chance(0.6) {
                    let h = 5 + (rng.f() * 10.0) as usize;
                    for y in (16 - h)..16 {
                        px[y * SIZE + x] = shade(grass_c, 0.75 + rng.f() * 0.35);
                    }
                }
            }
            px
        }
        tile::POPPY => flower(rng, rgb(214, 36, 30), rgb(40, 30, 20)),
        tile::DANDELION => flower(rng, rgb(250, 220, 40), rgb(240, 170, 20)),
        tile::CORNFLOWER => flower(rng, rgb(70, 110, 230), rgb(40, 50, 140)),
        tile::DEAD_BUSH => {
            let mut px = [CLEAR; 256];
            let c = rgb(140, 100, 50);
            for (x0, lean) in [(7i32, 0i32), (7, -1), (8, 1), (6, -2), (9, 2)] {
                let mut x = x0 as f32;
                for y in (4..16).rev() {
                    px[y * SIZE + (x as usize).min(15)] = shade(c, 0.85 + rng.f() * 0.3);
                    x = (x + lean as f32 * 0.35).clamp(0.0, 15.0);
                }
            }
            px
        }
        tile::SMOOTH_STONE => std::array::from_fn(|i| {
            // Minecraft's smooth stone: even gray with a lighter rim and a darker inner line.
            let (x, y) = (i % SIZE, i / SIZE);
            let edge = x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1;
            let base = rgb(158, 158, 160);
            if edge { shade(base, 0.82) } else { shade(base, 0.97 + rng.f() * 0.05) }
        }),
        tile::STATION_CORE | tile::STATION_CORE_TOP => {
            // Steel casing, glass panes, and a glowing blue heart (an ender pearl's) in the middle.
            let top = t == tile::STATION_CORE_TOP;
            std::array::from_fn(|i| {
                let (x, y) = ((i % SIZE) as i32, (i / SIZE) as i32);
                let (dx, dy) = (x as f32 - 7.5, y as f32 - 7.5);
                let d = (dx * dx + dy * dy).sqrt();
                let frame = x == 0 || y == 0 || x == 15 || y == 15 || (!top && (x == 5 || x == 10) && (y < 5 || y > 10));
                if frame {
                    shade(rgb(196, 200, 206), 0.8 + rng.f() * 0.1)
                } else if d < 2.6 {
                    shade(rgb(120, 230, 255), 1.0 - d * 0.08)
                } else if d < 4.2 {
                    shade(rgb(40, 110, 150), 0.9 + rng.f() * 0.1)
                } else if (x + y) % 7 == 0 {
                    rgb(200, 225, 235)
                } else {
                    shade(rgb(150, 175, 190), 0.9 + rng.f() * 0.08)
                }
            })
        }
        tile::HULL => std::array::from_fn(|i| {
            // White hull plating: four panels, darker seams, a rivet at each panel's corner.
            let (x, y) = (i % SIZE, i / SIZE);
            let seam = x % 8 == 0 || y % 8 == 0;
            let rivet = (x % 8 == 2 || x % 8 == 6) && (y % 8 == 2 || y % 8 == 6);
            let base = rgb(222, 224, 228);
            if seam {
                shade(base, 0.72)
            } else if rivet {
                shade(base, 0.86)
            } else {
                shade(base, 0.97 + rng.f() * 0.04)
            }
        }),
        tile::GLASS => std::array::from_fn(|i| {
            // A pale frame and two streaks of glare; the rest see-through.
            let (x, y) = (i % SIZE, i / SIZE);
            let frame = x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1;
            let glare = (x + y == 9 || x + y == 10 || x + y == 13) && (3..=12).contains(&x);
            if frame {
                rgb(205, 228, 238)
            } else if glare {
                rgb(232, 246, 252)
            } else {
                CLEAR
            }
        }),
        tile::ENGINE_SIDE => std::array::from_fn(|i| {
            // Dark steel with cooling ribs, scorched toward the bottom, an orange warning band.
            let (x, y) = (i % SIZE, i / SIZE);
            let base = rgb(92, 94, 102);
            if y <= 1 {
                if (x / 2) % 2 == 0 { rgb(232, 120, 40) } else { rgb(40, 40, 44) }
            } else if x % 4 == 0 {
                shade(base, 0.7)
            } else {
                shade(base, 1.0 - (y as f32 / 16.0) * 0.45 + rng.f() * 0.06)
            }
        }),
        tile::ENGINE_NOZZLE => std::array::from_fn(|i| {
            // The bell seen from below: rings round a glowing throat.
            let (x, y) = (i % SIZE, i / SIZE);
            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
            if d < 2.2 {
                rgb(255, 196, 92)
            } else if d < 3.4 {
                rgb(196, 84, 32)
            } else if d < 7.4 {
                shade(rgb(70, 70, 76), if (d as i32) % 2 == 0 { 0.8 } else { 1.0 })
            } else {
                shade(rgb(120, 122, 130), 0.9 + rng.f() * 0.08)
            }
        }),
        tile::HELM_SIDE => std::array::from_fn(|i| {
            // A console: a screen with a green orbit drawn on it, buttons under it.
            let (x, y) = (i % SIZE, i / SIZE);
            let frame = x == 0 || x == SIZE - 1 || y == 0 || y == SIZE - 1;
            let (dx, dy) = (x as f32 - 7.5, (y as f32 - 5.5) * 1.6);
            let ring = ((dx * dx + dy * dy).sqrt() - 4.0).abs() < 0.7;
            if frame {
                rgb(200, 204, 212)
            } else if (2..=9).contains(&y) && (2..=13).contains(&x) {
                if ring { rgb(110, 240, 150) } else if x == 8 && y == 5 { rgb(255, 220, 120) } else { rgb(18, 30, 44) }
            } else if y == 12 && x % 3 == 1 && (2..=13).contains(&x) {
                [rgb(230, 70, 60), rgb(250, 200, 60), rgb(80, 200, 255)][(x / 3) % 3]
            } else {
                shade(rgb(150, 156, 168), 0.92 + rng.f() * 0.06)
            }
        }),
        tile::HELM_TOP => std::array::from_fn(|i| {
            // The control panel: a joystick in the middle, lit keys round it.
            let (x, y) = (i % SIZE, i / SIZE);
            let d = ((x as f32 - 7.5).powi(2) + (y as f32 - 7.5).powi(2)).sqrt();
            if x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1 {
                rgb(200, 204, 212)
            } else if d < 1.6 {
                rgb(220, 60, 50)
            } else if d < 2.6 {
                rgb(40, 40, 46)
            } else if (x == 3 || x == 12) && y % 3 == 0 {
                rgb(80, 200, 255)
            } else {
                shade(rgb(60, 66, 80), 0.9 + rng.f() * 0.1)
            }
        }),
        tile::CHEST_SIDE | tile::CHEST_TOP => {
            // Planks in a dark frame; the sides have the lid's seam and a latch.
            let top = t == tile::CHEST_TOP;
            std::array::from_fn(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let wood = rgb(160, 112, 58);
                if x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1 {
                    rgb(72, 48, 26)
                } else if !top && y == 5 {
                    rgb(72, 48, 26)
                } else if !top && (7..=8).contains(&x) && (4..=7).contains(&y) {
                    rgb(200, 200, 205)
                } else if y % 4 == 0 {
                    shade(wood, 0.8)
                } else {
                    shade(wood, 0.92 + rng.f() * 0.12)
                }
            })
        }
        tile::PAD_TOP | tile::PAD_READY_TOP => {
            // Dark concrete in a rim of hazard stripes; ready, the rim's lights are lit and the
            // stripes glow.
            let ready = t == tile::PAD_READY_TOP;
            std::array::from_fn(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let rim = x < 2 || y < 2 || x >= SIZE - 2 || y >= SIZE - 2;
                let lamp = rim && (x % 5 == 2 || y % 5 == 2) && (x < 2 || x >= SIZE - 2) != (y < 2 || y >= SIZE - 2);
                if rim && ready && lamp {
                    rgb(170, 255, 230)
                } else if rim {
                    let yellow = (x + y) / 3 % 2 == 0;
                    match (yellow, ready) {
                        (true, true) => rgb(255, 214, 70),
                        (true, false) => rgb(196, 160, 40),
                        (false, _) => rgb(34, 34, 38),
                    }
                } else {
                    shade(rgb(92, 94, 100), 0.9 + rng.f() * 0.14)
                }
            })
        }
        tile::PAD_SIDE => std::array::from_fn(|i| {
            // Concrete with a yellow band along the top.
            let y = i / SIZE;
            if y < 2 {
                rgb(196, 160, 40)
            } else {
                shade(rgb(92, 94, 100), 0.85 + rng.f() * 0.14)
            }
        }),
        tile::ELEVATOR_BASE_TOP => std::array::from_fn(|i| {
            // Steel tread plate: raised diamonds on dark steel, a seam round the block.
            let (x, y) = (i % SIZE, i / SIZE);
            let seam = x == 0 || y == 0;
            let diamond = ((x + y) % 4 == 1 && (x / 2 + y / 2) % 2 == 0) || ((x + SIZE - y) % 4 == 3 && (x / 2 + y / 2) % 2 == 1);
            let base = rgb(88, 92, 100);
            if seam {
                shade(base, 0.62)
            } else if diamond {
                shade(base, 1.38)
            } else {
                shade(base, 0.93 + rng.f() * 0.1)
            }
        }),
        tile::ELEVATOR_EDGE_TOP => std::array::from_fn(|i| {
            // Hazard stripes, worn a little.
            let (x, y) = (i % SIZE, i / SIZE);
            if (x + y) / 4 % 2 == 0 {
                shade(rgb(214, 168, 44), 0.9 + rng.f() * 0.12)
            } else {
                shade(rgb(36, 36, 40), 0.9 + rng.f() * 0.15)
            }
        }),
        tile::ELEVATOR_BASE_SIDE => std::array::from_fn(|i| {
            // Concrete footing under a steel lip.
            let (x, y) = (i % SIZE, i / SIZE);
            if y < 3 {
                if y == 2 { rgb(60, 62, 68) } else { shade(rgb(108, 112, 120), 0.95 + rng.f() * 0.08) }
            } else if x % 8 == 0 && y > 4 {
                shade(rgb(120, 120, 118), 0.8)
            } else {
                shade(rgb(140, 140, 136), 0.86 + rng.f() * 0.14)
            }
        }),
        tile::WATER => std::array::from_fn(|i| {
            // Light ripples on blue; the liquid shader tints it by depth and adds the sky's shine.
            let (x, y) = (i % SIZE, i / SIZE);
            let w = waves(x, y, &[(1.0, 2.0, 0.3), (2.0, -1.0, 1.7), (3.0, 1.0, 4.1), (-1.0, 3.0, 2.2)]);
            let crest = (w * 2.2).clamp(-1.0, 1.0);
            let k = 0.86 + crest * 0.12 + (rng.f() - 0.5) * 0.03;
            if crest > 0.72 { shade(rgb(214, 236, 250), 0.98 + rng.f() * 0.04) } else { shade(rgb(150, 190, 232), k) }
        }),
        tile::LAVA => std::array::from_fn(|i| {
            // Molten orange with bright yellow veins and dark cooling crust.
            let (x, y) = (i % SIZE, i / SIZE);
            let w = waves(x, y, &[(1.0, 1.0, 0.0), (2.0, -1.0, 2.0), (-1.0, 2.0, 4.0), (3.0, 2.0, 1.0)]);
            let v = waves(x, y, &[(2.0, 3.0, 0.7), (-3.0, 1.0, 3.3)]);
            if w.abs() < 0.09 {
                shade(rgb(255, 236, 120), 0.96 + rng.f() * 0.04)
            } else if v > 0.62 {
                shade(rgb(120, 30, 8), 0.85 + rng.f() * 0.2)
            } else {
                let c = if w > 0.0 { rgb(244, 120, 24) } else { rgb(214, 70, 14) };
                shade(c, 0.9 + rng.f() * 0.14)
            }
        }),
        tile::OBSIDIAN => std::array::from_fn(|i| {
            // Near-black glass with violet glints.
            let (x, y) = (i % SIZE, i / SIZE);
            let streak = waves(x, y, &[(1.0, 2.0, 0.5), (2.0, 1.0, 2.5)]);
            if streak > 0.82 {
                shade(rgb(120, 82, 168), 0.85 + rng.f() * 0.2)
            } else if streak > 0.6 {
                shade(rgb(58, 36, 86), 0.9 + rng.f() * 0.15)
            } else {
                shade(rgb(22, 16, 34), 0.8 + rng.f() * 0.4)
            }
        }),
        tile::ICE => std::array::from_fn(|i| {
            // Pale blue with white cracks.
            let (x, y) = (i % SIZE, i / SIZE);
            let crack = (x * 3 + y * 5) % 17 == 0 || (x * 7 + 16 - y) % 23 == 0;
            if crack { rgb(236, 246, 255) } else { shade(rgb(158, 200, 240), 0.95 + rng.f() * 0.08) }
        }),
        tile::LADDER => std::array::from_fn(|i| {
            // Two rails and a rung every four pixels; the rest see-through.
            let (x, y) = (i % SIZE, i / SIZE);
            let wood = rgb(150, 112, 62);
            if x == 2 || x == 3 || x == 12 || x == 13 {
                shade(wood, if x == 2 || x == 12 { 0.78 } else { 0.95 + rng.f() * 0.08 })
            } else if (4..12).contains(&x) && y % 4 == 2 {
                shade(wood, 0.92 + rng.f() * 0.12)
            } else if (4..12).contains(&x) && y % 4 == 3 {
                shade(wood, 0.66)
            } else {
                CLEAR
            }
        }),
        tile::DOOR_TOP | tile::DOOR_BOTTOM => {
            // Oak boards running up, in a frame; the top half has two windows, the bottom a handle.
            let top = t == tile::DOOR_TOP;
            std::array::from_fn(|i| {
                let (x, y) = (i % SIZE, i / SIZE);
                let wood = rgb(166, 126, 72);
                let frame = x <= 1 || x >= SIZE - 2 || (top && y <= 1) || (!top && y >= SIZE - 2);
                let window = top && (3..=12).contains(&x) && (3..=11).contains(&y) && x != 7 && x != 8;
                if frame {
                    shade(wood, 0.7 + rng.f() * 0.06)
                } else if window {
                    CLEAR
                } else if !top && x == 12 && (1..=3).contains(&y) {
                    rgb(70, 64, 60)
                } else {
                    shade(wood, if x % 4 == 0 { 0.8 } else { 0.94 + rng.f() * 0.1 })
                }
            })
        }
        tile::TRAPDOOR => std::array::from_fn(|i| {
            // A frame of oak with a cross brace and four holes.
            let (x, y) = (i % SIZE, i / SIZE);
            let wood = rgb(166, 126, 72);
            let frame = x <= 1 || y <= 1 || x >= SIZE - 2 || y >= SIZE - 2;
            let brace = x == 7 || x == 8 || y == 7 || y == 8;
            let hole = !frame && !brace && (3..=5).contains(&(x % 8)) && (3..=5).contains(&(y % 8));
            if frame {
                shade(wood, 0.72 + rng.f() * 0.06)
            } else if hole {
                CLEAR
            } else if brace {
                shade(wood, 0.84)
            } else {
                shade(wood, 0.94 + rng.f() * 0.1)
            }
        }),
        _ => [rgb(255, 0, 255); 256],
    }
}
