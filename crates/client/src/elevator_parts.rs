//! What a space elevator is built of besides its cabin: the ribbon the cabin climbs (a carbon
//! band with steel guide rails at its edges and a rack down its back, spliced every 4 blocks), the
//! steel anchor housing behind the ribbon's foot on its platform (the platform itself is blocks:
//! `orbit_core::elevator::lay_base`) and the captured rock that is its counterweight (basalt, block
//! by block). Meshes are in
//! the ribbon's frame: x across it, y along it (up), z away from the cabin; meters.

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use orbit_core::elevator::{RIBBON_THICKNESS, RIBBON_WIDTH};

/// The ribbon texture: 64 × 64 pixels, 16 to a block across, 4 blocks of ribbon down it.
const TEX: usize = 64;
/// Blocks of ribbon one texture covers.
pub const RIBBON_REPEAT: f32 = 4.0;

/// Columns of the texture: (first, width) in pixels.
const BAND: (f32, f32) = (0.0, 10.0);
const BAND_SIDE: (f32, f32) = (16.0, 4.0);
const RAIL: (f32, f32) = (24.0, 8.0);
const RACK: (f32, f32) = (40.0, 6.0);
const PLATE: (f32, f32) = (48.0, 16.0);

/// A mesh being built from boxes.
#[derive(Default)]
struct Boxes {
    pos: Vec<[f32; 3]>,
    normal: Vec<[f32; 3]>,
    uv: Vec<[f32; 2]>,
    index: Vec<u32>,
}

impl Boxes {
    /// A box from `lo` to `hi`, each face's texture coordinates from `uv(point, normal)`.
    fn cuboid(&mut self, lo: Vec3, hi: Vec3, uv: impl Fn(Vec3, Vec3) -> [f32; 2]) {
        for axis in 0..3 {
            for sign in [-1.0f32, 1.0] {
                let n = Vec3::AXES[axis] * sign;
                let (b, c) = (Vec3::AXES[(axis + 1) % 3], Vec3::AXES[(axis + 2) % 3]);
                let at = |i: f32, j: f32| {
                    let mut p = lo + (hi - lo) * (b * i + c * j);
                    p[axis] = if sign > 0.0 { hi[axis] } else { lo[axis] };
                    p
                };
                let base = self.pos.len() as u32;
                for (i, j) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
                    let p = at(i, j);
                    self.pos.push(p.to_array());
                    self.normal.push(n.to_array());
                    self.uv.push(uv(p, n));
                }
                // Wound to face out (both sides are drawn, but normals and order agree).
                if sign > 0.0 {
                    self.index
                        .extend([base, base + 1, base + 2, base, base + 2, base + 3]);
                } else {
                    self.index
                        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
                }
            }
        }
    }

    fn mesh(self) -> Mesh {
        Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        )
        .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, self.pos)
        .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, self.normal)
        .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, self.uv)
        .with_inserted_indices(Indices::U32(self.index))
    }
}

/// Texture coordinates for a ribbon part: across a face, the columns `front` (faces toward or
/// away from the cabin) or `side`; down it, `v(point)`.
fn ribbon_uv(
    lo: Vec3,
    hi: Vec3,
    front: (f32, f32),
    side: (f32, f32),
    v: impl Fn(Vec3) -> f32,
) -> impl Fn(Vec3, Vec3) -> [f32; 2] {
    move |p, n| {
        let (cols, t) = if n.z != 0.0 {
            (front, (p.x - lo.x) / (hi.x - lo.x).max(1e-6))
        } else if n.x != 0.0 {
            (side, (p.z - lo.z) / (hi.z - lo.z).max(1e-6))
        } else {
            // The ends: hardly ever seen.
            (side, 0.5)
        };
        [(cols.0 + t * cols.1) / TEX as f32, v(p)]
    }
}

/// The ribbon, one block long (y from -0.5 to 0.5): it is stretched to its length, and its
/// material repeats the texture `length / RIBBON_REPEAT` times down it.
pub fn ribbon_mesh() -> Mesh {
    let mut b = Boxes::default();
    let v = |p: Vec3| p.y + 0.5;
    let (w, t) = (RIBBON_WIDTH / 2.0, RIBBON_THICKNESS / 2.0);
    // The band itself.
    let (lo, hi) = (Vec3::new(-w, -0.5, -t), Vec3::new(w, 0.5, t));
    b.cuboid(lo, hi, ribbon_uv(lo, hi, BAND, BAND_SIDE, v));
    // A guide rail along each edge (the cabin's clamps ride them).
    for s in [-1.0, 1.0] {
        let (x0, x1) = (w * s, (w + 0.0875) * s);
        let (lo, hi) = (
            Vec3::new(x0.min(x1), -0.5, -t * 0.55),
            Vec3::new(x0.max(x1), 0.5, t + 0.05),
        );
        b.cuboid(lo, hi, ribbon_uv(lo, hi, RAIL, RAIL, v));
    }
    // The rack down its back that the cabin's drive climbs.
    let (lo, hi) = (Vec3::new(-0.08, -0.5, t), Vec3::new(0.08, 0.5, t + 0.075));
    b.cuboid(lo, hi, ribbon_uv(lo, hi, RACK, RACK, v));
    b.mesh()
}

/// Texture coordinates repeating a block tile once per block: on each face, its two other axes.
fn block_uv(p: Vec3, n: Vec3) -> [f32; 2] {
    if n.y != 0.0 {
        [p.x, p.z]
    } else if n.x != 0.0 {
        [p.z, -p.y]
    } else {
        [p.x, -p.y]
    }
}

/// The anchor at the ribbon's foot: a steel housing behind it (clear of the cabin), and two cheeks
/// either side of the ribbon tying it in. Its material repeats the texture once (not stretched).
pub fn anchor_mesh() -> Mesh {
    let mut b = Boxes::default();
    let v = |p: Vec3| 0.999 - p.y / RIBBON_REPEAT;
    let parts = [
        (Vec3::new(-0.9, 0.0, 0.25), Vec3::new(0.9, 1.4, 1.05)),
        (Vec3::new(-0.75, 1.4, 0.35), Vec3::new(0.75, 1.55, 0.95)),
        (Vec3::new(-0.7, 0.0, -0.1), Vec3::new(-0.46, 0.45, 0.25)),
        (Vec3::new(0.46, 0.0, -0.1), Vec3::new(0.7, 0.45, 0.25)),
    ];
    for (lo, hi) in parts {
        b.cuboid(lo, hi, ribbon_uv(lo, hi, PLATE, PLATE, v));
    }
    b.mesh()
}

/// The counterweight: a captured rock, a few blocky lumps of it.
pub fn rock_mesh() -> Mesh {
    let mut b = Boxes::default();
    for (at, size) in [
        (Vec3::ZERO, Vec3::new(14.0, 10.0, 12.0)),
        (Vec3::new(5.0, 4.0, -3.0), Vec3::new(8.0, 7.0, 8.0)),
        (Vec3::new(-6.0, -2.0, 4.0), Vec3::new(7.0, 6.0, 9.0)),
        (Vec3::new(1.0, -6.0, 1.0), Vec3::splat(3.0)),
    ] {
        b.cuboid(at - size / 2.0, at + size / 2.0, block_uv);
    }
    b.mesh()
}

/// A small hash for the texture's noise.
fn noise(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B1) ^ (y as u32).wrapping_mul(0x85EB_CA77);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Paints the ribbon's texture (pixel art, like the blocks), repeating both ways.
pub fn ribbon_texture() -> Image {
    let mut px = vec![[0u8; 4]; TEX * TEX];
    let rgb = |r: u8, g: u8, b: u8| [r, g, b, 255];
    let shade = |c: [u8; 4], k: f32| {
        [
            (c[0] as f32 * k).min(255.0) as u8,
            (c[1] as f32 * k).min(255.0) as u8,
            (c[2] as f32 * k).min(255.0) as u8,
            255,
        ]
    };
    let hazard = |x: usize, y: usize| {
        if (x + y) % 6 < 3 {
            rgb(222, 170, 38)
        } else {
            rgb(34, 34, 36)
        }
    };
    for y in 0..TEX {
        for x in 0..TEX {
            let n = 0.94 + noise(x, y) * 0.12;
            let c = match x {
                // The band: woven carbon, lighter at its edges; a steel splice plate every 4 blocks
                // with a hazard band under it (it shows how fast you go), a faint seam every block.
                0..10 => {
                    if y < 6 {
                        let edge = y == 0 || y == 5 || x == 0 || x == 9;
                        let rivet = (x == 2 || x == 7) && (y == 2 || y == 3);
                        if rivet {
                            rgb(70, 72, 78)
                        } else if edge {
                            rgb(96, 100, 108)
                        } else {
                            shade(rgb(132, 136, 144), n)
                        }
                    } else if y < 8 {
                        hazard(x, y)
                    } else if x == 0 || x == 9 {
                        rgb(74, 78, 88)
                    } else if y % 16 == 15 {
                        rgb(28, 29, 33)
                    } else if (x / 2 + y / 2) % 2 == 0 {
                        shade(rgb(48, 51, 58), n)
                    } else {
                        shade(rgb(37, 39, 45), n)
                    }
                }
                16..20 => {
                    if y < 6 {
                        shade(rgb(112, 116, 124), n)
                    } else if y < 8 {
                        hazard(x, y)
                    } else {
                        shade(rgb(32, 34, 38), n)
                    }
                }
                // The guide rails: polished steel, rounded by shading; a joint every 4 blocks
                // and a bolt every block.
                24..32 => {
                    let tone: f32 =
                        [104.0, 138.0, 166.0, 184.0, 184.0, 166.0, 138.0, 104.0][x - 24];
                    if y < 2 {
                        rgb(60, 62, 68)
                    } else if y % 16 == 8 && (x == 27 || x == 28) {
                        rgb(84, 86, 92)
                    } else {
                        shade(
                            rgb(
                                tone as u8,
                                (tone + 4.0) as u8,
                                (tone + 12.0).min(255.0) as u8,
                            ),
                            0.97 + noise(x, y) * 0.06,
                        )
                    }
                }
                // The rack: teeth.
                40..46 => {
                    if y % 4 < 2 {
                        shade(rgb(100, 104, 112), n)
                    } else {
                        rgb(50, 53, 60)
                    }
                }
                // Steel plate (the anchor housing): panels, rivets, a hazard band at its foot.
                48..64 => {
                    let (u, v) = (x - 48, y);
                    let row = TEX - 1 - v; // from the bottom of the housing up
                    if row < 3 {
                        hazard(u, v)
                    } else if u == 0 || u == 15 || v % 16 == 0 {
                        rgb(64, 67, 74)
                    } else if (u == 2 || u == 13) && v % 16 == 2 {
                        rgb(150, 154, 162)
                    } else {
                        shade(rgb(96, 100, 108), n)
                    }
                }
                _ => rgb(56, 58, 64),
            };
            px[y * TEX + x] = c;
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: TEX as u32,
            height: TEX as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        px.into_iter().flatten().collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Nearest,
        min_filter: ImageFilterMode::Nearest,
        ..default()
    });
    image
}
