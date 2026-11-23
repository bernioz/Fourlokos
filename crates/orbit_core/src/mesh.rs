//! Engine-agnostic meshing. Voxel chunks: one flat quad per exposed cell side, textured from the
//! atlas (`block::tile`), with per-vertex ambient occlusion and a regional tint in the color, and
//! smooth light in `lights` (sky and block brightness). Grass and flowers are two crossed planes,
//! torches a thin stick. Far view: the terrain's columns on a coarser grid, flat-topped with
//! walls where heights step, so planets look blocky from afar too.

use crate::block::{self, tile, Liquid, Shape};
use crate::light;
use crate::planet::{ChunkKey, Planet, PlanetSpec, Terrain};
use crate::rng::mix;
use crate::sphere::{CellId, CubeSphere, Side};
use glam::Vec3;

#[derive(Default)]
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// rgb: tint, shade and occlusion (multiplies the texture); a: 1.
    pub colors: Vec<[f32; 4]>,
    /// Atlas coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// (sky, block) brightness, each 0..1.
    pub lights: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl MeshData {
    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// A quad; `flip` splits it along the other diagonal. Faces the side its corners wind
    /// counter-clockwise around.
    fn quad(&mut self, q: [Vec3; 4], uv: [[f32; 2]; 4], n: Vec3, color: [[f32; 3]; 4], light: [[f32; 2]; 4], flip: bool) {
        let i0 = self.positions.len() as u32;
        for v in 0..4 {
            self.positions.push(q[v].to_array());
            self.normals.push(n.to_array());
            self.colors.push([color[v][0], color[v][1], color[v][2], 1.0]);
            self.uvs.push(uv[v]);
            self.lights.push(light[v]);
        }
        if flip {
            self.indices.extend([i0 + 1, i0 + 2, i0 + 3, i0 + 1, i0 + 3, i0]);
        } else {
            self.indices.extend([i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
        }
    }

    /// Both faces of a quad (plants), lit as if facing `n`.
    fn double_quad(&mut self, q: [Vec3; 4], uv: [[f32; 2]; 4], n: Vec3, color: [f32; 3], light: [f32; 2]) {
        self.quad(q, uv, n, [color; 4], [light; 4], false);
        self.quad([q[0], q[3], q[2], q[1]], [uv[0], uv[3], uv[2], uv[1]], n, [color; 4], [light; 4], false);
    }
}

/// Brightness by how many of a vertex's 3 neighbors in front of the face are solid.
const AO: [f32; 4] = [1.0, 0.8, 0.64, 0.5];

/// Smooth value noise in [0, 1] over planet-local space, for broad color variation.
fn value_noise(p: Vec3, seed: u64) -> f32 {
    let f = p.floor();
    let t = p - f;
    let t = t * t * (Vec3::splat(3.0) - 2.0 * t);
    let h = |x: f32, y: f32, z: f32| {
        let k = mix(seed ^ mix((f.x + x) as i64 as u64 ^ mix((f.y + y) as i64 as u64 ^ mix((f.z + z) as i64 as u64))));
        (k >> 40) as f32 / (1u64 << 24) as f32
    };
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let x00 = lerp(h(0., 0., 0.), h(1., 0., 0.), t.x);
    let x10 = lerp(h(0., 1., 0.), h(1., 1., 0.), t.x);
    let x01 = lerp(h(0., 0., 1.), h(1., 0., 1.), t.x);
    let x11 = lerp(h(0., 1., 1.), h(1., 1., 1.), t.x);
    lerp(lerp(x00, x10, t.y), lerp(x01, x11, t.y), t.z)
}

/// Shifts foliage between a sunny yellow-green and a deep blue-green by region.
fn tint(c: [f32; 3], at: Vec3, seed: u64) -> [f32; 3] {
    let n = value_noise(at / 22.0, seed) * 0.7 + value_noise(at / 7.0, seed ^ 1) * 0.3;
    let warm = [1.12, 1.06, 0.78];
    let cool = [0.82, 0.95, 1.08];
    [0, 1, 2].map(|k| c[k] * (cool[k] + (warm[k] - cool[k]) * n))
}

/// (sky, block) brightness of a cell's own light.
fn own_light(planet: &Planet, c: CellId) -> [f32; 2] {
    [light::brightness(planet.light(c) as f32), light::glow(planet.block_light(c) as f32)]
}

pub fn chunk_mesh(planet: &Planet, key: ChunkKey) -> MeshData {
    mesh_chunk(planet, key, 0)
}

/// A chunk's meshes: its blocks (lava too: it is opaque and glows), and its water, which is
/// drawn see-through with a material of its own.
pub struct Meshes {
    pub solid: MeshData,
    pub water: MeshData,
}

/// Full chunk meshes (`chunk_mesh` and its water).
pub fn chunk_meshes(planet: &Planet, key: ChunkKey) -> Meshes {
    Meshes { solid: mesh_chunk(planet, key, 0), water: water_mesh(planet, key, 0) }
}

/// Shell chunk meshes (`shell_mesh` and the water the sky lights).
pub fn shell_meshes(planet: &Planet, key: ChunkKey) -> Meshes {
    Meshes { solid: mesh_chunk(planet, key, SHELL_SKY), water: water_mesh(planet, key, SHELL_SKY) }
}

/// The water of a chunk, faces looking into at least `min_sky` sky light.
fn water_mesh(planet: &Planet, key: ChunkKey, min_sky: u8) -> MeshData {
    let mut m = MeshData::default();
    for c in planet.chunk_cells(key) {
        let b = planet.get(c);
        if block::liquid(b) == Some(Liquid::Water) {
            planet_liquid(planet, &mut m, c, b, min_sky);
        }
    }
    m
}

/// Height of a liquid's surface in its cell, 0..1: full under more of it, else by how much it
/// holds (a source 8/9, like Minecraft's).
fn surface(b: block::Block, above: block::Block) -> f32 {
    if block::liquid(above).is_some() && block::liquid(above) == block::liquid(b) {
        1.0
    } else {
        block::amount(b) as f32 / 9.0
    }
}

/// Height of a liquid's surface at a corner, from the cells around it (block, block above): the
/// mean of the liquid's heights there and of the open cells beside it (at 0), so the surface
/// slopes down to where it runs out; full if any of them has the liquid over it.
fn corner_height(kind: Liquid, around: impl Iterator<Item = (block::Block, block::Block)>) -> f32 {
    let mut hs: Vec<f32> = Vec::with_capacity(4);
    for (b, above) in around {
        if block::liquid(b) == Some(kind) {
            if block::liquid(above) == Some(kind) {
                return 1.0;
            }
            hs.push(surface(b, above));
        } else if !block::is_solid(b) {
            hs.push(0.0);
        }
    }
    // Summed in a fixed order, so every cell at this corner gets the very same height.
    hs.sort_by(f32::total_cmp);
    if hs.is_empty() { 8.0 / 9.0 } else { hs.iter().sum::<f32>() / hs.len() as f32 }
}

/// A liquid cell's faces, whatever grid it is on. `pos(u, v, w)` is the point at fractions (u, v)
/// across the cell and w up it; `heights` the surface at corners (0,0), (1,0), (1,1), (0,1);
/// each face is drawn if it has a light (the light in front of it): top, the four sides
/// (u = 0, u = 1, v = 0, v = 1), bottom. Lava is flagged (alpha 0) for the shader to make it glow.
struct LiquidCell<'a> {
    kind: Liquid,
    heights: [f32; 4],
    pos: &'a dyn Fn(f32, f32, f32) -> Vec3,
    top: Option<[f32; 2]>,
    sides: [Option<[f32; 2]>; 4],
    bottom: Option<[f32; 2]>,
}

fn emit_liquid(m: &mut MeshData, l: &LiquidCell) {
    let t = block::tiles(l.kind.source())[0];
    let alpha = if l.kind == Liquid::Lava { 0.0 } else { 1.0 };
    // Each face turns toward its own side of the cell (not away from the cell's middle: a thin
    // flowing surface lies below that, and would face down and be culled from above).
    let foot = (l.pos)(0.5, 0.5, 0.0);
    let up = (l.pos)(0.5, 0.5, 1.0) - foot;
    let corners = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let mut face = |mut q: [Vec3; 4], mut uv: [[f32; 2]; 4], out: Vec3, light: [f32; 2], shade: f32| {
        let mut n = (q[2] - q[0]).cross(q[3] - q[1]).normalize_or_zero();
        if n.dot(out) < 0.0 {
            q.swap(1, 3);
            uv.swap(1, 3);
            n = -n;
        }
        let i0 = m.positions.len() as u32;
        for v in 0..4 {
            m.positions.push(q[v].to_array());
            m.normals.push(n.to_array());
            m.colors.push([shade, shade, shade, alpha]);
            m.uvs.push(uv[v]);
            m.lights.push(light);
        }
        m.indices.extend([i0, i0 + 1, i0 + 2, i0, i0 + 2, i0 + 3]);
    };
    if let Some(light) = l.top {
        let q = [0, 1, 2, 3].map(|i| (l.pos)(corners[i].0, corners[i].1, l.heights[i]));
        let uv = corners.map(|(u, v)| tile::uv(t, u, v));
        face(q, uv, up, light, 1.0);
    }
    // Sides: (corner a, corner b) along the side, a's u or v running 0 → 1 to b.
    for (s, (a, b)) in [(0, 3), (1, 2), (0, 1), (3, 2)].into_iter().enumerate() {
        let Some(light) = l.sides[s] else { continue };
        let (ha, hb) = (l.heights[a], l.heights[b]);
        let (ca, cb) = (corners[a], corners[b]);
        let q = [(l.pos)(ca.0, ca.1, 0.0), (l.pos)(cb.0, cb.1, 0.0), (l.pos)(cb.0, cb.1, hb), (l.pos)(ca.0, ca.1, ha)];
        let uv = [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 1.0 - hb), tile::uv(t, 0.0, 1.0 - ha)];
        let mid = [(0.0, 0.5), (1.0, 0.5), (0.5, 0.0), (0.5, 1.0)][s];
        face(q, uv, (l.pos)(mid.0, mid.1, 0.0) - foot, light, 0.8);
    }
    if let Some(light) = l.bottom {
        let q = corners.map(|(u, v)| (l.pos)(u, v, 0.0));
        let uv = corners.map(|(u, v)| tile::uv(t, u, v));
        face(q, uv, -up, light, 0.6);
    }
}

/// A liquid cell on a planet. Faces show against anything but the same liquid and opaque
/// blocks (the top always, unless the liquid goes on up).
fn planet_liquid(planet: &Planet, m: &mut MeshData, c: CellId, b: block::Block, min_sky: u8) {
    let g = &planet.grid;
    let Some(kind) = block::liquid(b) else { return };
    let (f, i, j, k) = g.coords(c);
    let get = |x: Option<CellId>| x.map_or(block::AIR, |x| planet.get(x));
    let lit = |x: Option<CellId>| -> [f32; 2] {
        let x = x.unwrap_or(c);
        let (sky, glow) = (planet.light(x).max(planet.light(c)), planet.block_light(x).max(planet.block_light(c)));
        [light::brightness(sky as f32), light::glow(glow as f32)]
    };
    let shows = |x: Option<CellId>| {
        let nb = get(x);
        block::liquid(nb) != Some(kind) && !block::opaque(nb) && (min_sky == 0 || x.is_none_or(|x| planet.light(x) >= min_sky))
    };
    // The four cells around each corner, found by point a little out from it in the layer.
    let r = g.radius(k) + 0.5;
    let (eu, ev) = ((g.dir(f, i + 1, j) - g.dir(f, i, j)).normalize(), (g.dir(f, i, j + 1) - g.dir(f, i, j)).normalize());
    let heights = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(di, dj)| {
        let at = g.dir(f, i + di, j + dj) * r;
        let around = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].into_iter().map(|(su, sv)| {
            let q = g.cell_at(at + eu * (0.45 * su) + ev * (0.45 * sv));
            (get(q), get(q.and_then(|q| g.neighbor(q, Side::Top))))
        });
        corner_height(kind, around)
    });
    let above = g.neighbor(c, Side::Top);
    let below = g.neighbor(c, Side::Bottom);
    let pos = |u: f32, v: f32, w: f32| g.point(c, u, v, w);
    let side = |s: Side| {
        let nb = g.neighbor(c, s);
        shows(nb).then(|| lit(nb))
    };
    let cell = LiquidCell {
        kind,
        heights,
        pos: &pos,
        top: (block::liquid(get(above)) != Some(kind) && (min_sky == 0 || above.is_none_or(|x| planet.light(x) >= min_sky))).then(|| lit(above)),
        sides: [side(Side::IMinus), side(Side::IPlus), side(Side::JMinus), side(Side::JPlus)],
        bottom: (below.is_some() && shows(below)).then(|| lit(below)),
    };
    emit_liquid(m, &cell);
}

/// Sky light a face must look into to be part of a shell mesh: open sky, or a few blocks from it.
pub const SHELL_SKY: u8 = 10;

/// A chunk's outside only, for seeing it from afar (orbit): the faces that look into air lit by
/// the sky (terrain, trees, plants, the mouths of caves), not the caves underground. Same blocks,
/// same look, a fraction of the triangles.
pub fn shell_mesh(planet: &Planet, key: ChunkKey) -> MeshData {
    mesh_chunk(planet, key, SHELL_SKY)
}

/// A chunk's mesh with the faces that look into air with at least `min_sky` sky light.
fn mesh_chunk(planet: &Planet, key: ChunkKey, min_sky: u8) -> MeshData {
    let g = &planet.grid;
    let seed = planet.spec.seed;
    let mut m = MeshData::default();
    for c in planet.chunk_cells(key) {
        let b = planet.get(c);
        let shape = block::shape(b);
        if min_sky > 0 && !matches!(shape, Shape::Empty | Shape::Cube | Shape::Liquid) && planet.light(c) < min_sky {
            continue;
        }
        match shape {
            Shape::Empty => {}
            Shape::Cube => cube(planet, &mut m, c, b, seed, min_sky),
            Shape::Cross => {
                let up = g.center(c).normalize();
                let color = if block::tinted(b) { tint([1.0; 3], g.center(c), seed) } else { [1.0; 3] };
                let t = block::tiles(b)[1];
                let uv = [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 0.0, 0.0)];
                let light = own_light(planet, c);
                // Corner to corner, a little in from the cell's edges, like Minecraft's.
                let (a, z) = (0.15, 0.85);
                for (u0, v0, u1, v1) in [(a, a, z, z), (z, a, a, z)] {
                    let q = [g.point(c, u0, v0, 0.0), g.point(c, u1, v1, 0.0), g.point(c, u1, v1, 1.0), g.point(c, u0, v0, 1.0)];
                    m.double_quad(q, uv, up, color, light);
                }
            }
            Shape::Torch => torch(planet, &mut m, c, b),
            Shape::Boxes => {
                let open = Side::ALL.map(|s| g.neighbor(c, s).is_none_or(|nb| !block::opaque(planet.get(nb))));
                let pos = |u: f32, v: f32, w: f32| g.point(c, u, v, w);
                let join = crate::shape::joined(planet, c);
                emit_boxes(&mut m, block::tiles(b), crate::shape::boxes(b, join), &pos, own_light(planet, c), open);
            }
            // Lava is drawn with the blocks; water has its own mesh.
            Shape::Liquid => {
                if block::liquid(b) == Some(Liquid::Lava) {
                    planet_liquid(planet, &mut m, c, b, min_sky);
                }
            }
        }
    }
    m
}

/// A full block: every side that faces something see-through.
fn cube(planet: &Planet, m: &mut MeshData, c: CellId, b: block::Block, seed: u64, min_sky: u8) {
    let g = &planet.grid;
    let center = g.center(c);
    let tiles = block::tiles(b);
    let jitter = 0.95 + (mix(c as u64 ^ seed) % 1000) as f32 * 0.0001;
    for s in Side::ALL {
        let exposed = match g.neighbor(c, s) {
            Some(nb) => !block::face_hidden(b, planet.get(nb)) && (min_sky == 0 || planet.light(nb) >= min_sky),
            None => s == Side::Top,
        };
        if !exposed {
            continue;
        }
        let mut q = g.side_quad(c, s);
        let t = match s {
            Side::Top => tiles[0],
            Side::Bottom => tiles[2],
            _ => tiles[1],
        };
        // Side quads run bottom (k) to top: the texture's v points down the side.
        let mut uv = match s {
            Side::Top | Side::Bottom => [tile::uv(t, 0.0, 0.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 0.0, 1.0)],
            _ => [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 0.0, 0.0)],
        };
        let mut nrm = (q[1] - q[0]).cross(q[3] - q[0]).normalize();
        let mid = (q[0] + q[1] + q[2] + q[3]) * 0.25;
        if nrm.dot(mid - center) < 0.0 {
            q.swap(1, 3);
            uv.swap(1, 3);
            nrm = -nrm;
        }
        // Grass tints its top only; its sides are dirt with a green fringe already painted.
        let base = if block::tinted(b) && (b != block::GRASS || s == Side::Top) { tint([1.0; 3], center, seed) } else { [1.0; 3] };
        let shade = jitter
            * match s {
                Side::Top => 1.0,
                Side::Bottom => 0.6,
                _ => 0.82,
            };
        // Ambient occlusion and smooth light: sample the cells around each corner, just in
        // front of the face (its own cell, the two beside it and the diagonal one).
        let mut lights = [[0.0; 2]; 4];
        let ao = [0, 1, 2, 3].map(|i| {
            let v = q[i];
            let u = (q[(i + 1) % 4] - v).normalize();
            let w = (q[(i + 3) % 4] - v).normalize();
            let front = v + nrm * 0.5;
            let s1 = planet.opaque_at(front - u * 0.5 + w * 0.5);
            let s2 = planet.opaque_at(front + u * 0.5 - w * 0.5);
            let corner = planet.opaque_at(front - u * 0.5 - w * 0.5);
            let mut lit = vec![front + (u + w) * 0.5];
            if !s1 {
                lit.push(front - u * 0.5 + w * 0.5);
            }
            if !s2 {
                lit.push(front + u * 0.5 - w * 0.5);
            }
            if !(s1 && s2) && !corner {
                lit.push(front - u * 0.5 - w * 0.5);
            }
            let levels: Vec<(u8, u8)> = lit.iter().filter_map(|&p| planet.light_at(p)).collect();
            let count = levels.len().max(1) as f32;
            let sky = levels.iter().map(|l| l.0 as f32).sum::<f32>() / count;
            let glow = levels.iter().map(|l| l.1 as f32).sum::<f32>() / count;
            lights[i] = [light::brightness(sky), light::glow(glow)];
            if s1 && s2 { 3 } else { s1 as usize + s2 as usize + corner as usize }
        });
        let colors = ao.map(|o| base.map(|x| x * shade * AO[o]));
        // Split along the diagonal that keeps occlusion gradients smooth.
        m.quad(q, uv, nrm, colors, lights, ao[0] + ao[2] > ao[1] + ao[3]);
    }
}

use crate::shape::torch_point as torch_at;

/// A block made of boxes (`shape::boxes`), on any grid: `pos(u, v, w)` is the point at those
/// fractions of its cell, `open` whether each side of the cell (u-, u+, v-, v+, bottom, top) can
/// be seen (faces flush with a hidden side are left out). Each face shows the part of the tile
/// it covers, so a door's panel shows the whole door and its edges a strip of it.
fn emit_boxes(m: &mut MeshData, tiles: [u16; 3], boxes: crate::shape::Boxes, pos: &dyn Fn(f32, f32, f32) -> Vec3, light: [f32; 2], open: [bool; 6]) {
    for x in boxes.iter() {
        let [u0, v0, w0, u1, v1, w1] = x;
        let mid = pos((u0 + u1) * 0.5, (v0 + v1) * 0.5, (w0 + w1) * 0.5);
        // (corners as (u, v, w), their uv in the tile, the cell side it lies on if flush, shade)
        let faces: [([(f32, f32, f32); 4], [(f32, f32); 4], Option<usize>, f32); 6] = [
            ([(u0, v0, w0), (u1, v0, w0), (u1, v1, w0), (u0, v1, w0)], [(u0, v0), (u1, v0), (u1, v1), (u0, v1)], (w0 <= 0.0).then_some(4), 0.6),
            ([(u0, v0, w1), (u1, v0, w1), (u1, v1, w1), (u0, v1, w1)], [(u0, v0), (u1, v0), (u1, v1), (u0, v1)], (w1 >= 1.0).then_some(5), 1.0),
            ([(u0, v0, w0), (u0, v1, w0), (u0, v1, w1), (u0, v0, w1)], [(v0, 1.0 - w0), (v1, 1.0 - w0), (v1, 1.0 - w1), (v0, 1.0 - w1)], (u0 <= 0.0).then_some(0), 0.82),
            ([(u1, v0, w0), (u1, v1, w0), (u1, v1, w1), (u1, v0, w1)], [(v0, 1.0 - w0), (v1, 1.0 - w0), (v1, 1.0 - w1), (v0, 1.0 - w1)], (u1 >= 1.0).then_some(1), 0.82),
            ([(u0, v0, w0), (u1, v0, w0), (u1, v0, w1), (u0, v0, w1)], [(u0, 1.0 - w0), (u1, 1.0 - w0), (u1, 1.0 - w1), (u0, 1.0 - w1)], (v0 <= 0.0).then_some(2), 0.72),
            ([(u0, v1, w0), (u1, v1, w0), (u1, v1, w1), (u0, v1, w1)], [(u0, 1.0 - w0), (u1, 1.0 - w0), (u1, 1.0 - w1), (u0, 1.0 - w1)], (v1 >= 1.0).then_some(3), 0.72),
        ];
        for (f, (corners, uvs, flush, shade)) in faces.into_iter().enumerate() {
            if flush.is_some_and(|s| !open[s]) {
                continue;
            }
            let t = tiles[[2, 0, 1, 1, 1, 1][f]];
            let mut q = corners.map(|(u, v, w)| pos(u, v, w));
            let mut uv = uvs.map(|(a, b)| tile::uv(t, a.clamp(0.0, 1.0), b.clamp(0.0, 1.0)));
            let mut n = (q[1] - q[0]).cross(q[3] - q[0]).normalize_or_zero();
            let fm = (q[0] + q[1] + q[2] + q[3]) * 0.25;
            if n.dot(fm - mid) < 0.0 {
                q.swap(1, 3);
                uv.swap(1, 3);
                n = -n;
            }
            m.quad(q, uv, n, [[shade; 3]; 4], [light; 4], false);
        }
    }
}

/// A torch: a stick 2/16 wide and 10/16 tall in the middle of its cell, its flame on top (on a
/// wall, leaning out from it).
fn torch(planet: &Planet, m: &mut MeshData, c: CellId, b: block::Block) {
    let g = &planet.grid;
    let t = block::tiles(b)[1];
    let (lo, hi, top) = (7.0 / 16.0, 9.0 / 16.0, 10.0 / 16.0);
    let p = |u: f32, v: f32, w: f32| {
        let (u, v, w) = torch_at(b, u, v, w);
        g.point(c, u, v, w)
    };
    let light = own_light(planet, c);
    let center = p(0.5, 0.5, top * 0.5);
    let up = g.center(c).normalize();
    let mut face = |q: [Vec3; 4], uv: [[f32; 2]; 4]| {
        let mut q = q;
        let mut uv = uv;
        let mut n = (q[1] - q[0]).cross(q[3] - q[0]).normalize_or(up);
        let mid = (q[0] + q[1] + q[2] + q[3]) * 0.25;
        if n.dot(mid - center) < 0.0 {
            q.swap(1, 3);
            uv.swap(1, 3);
            n = -n;
        }
        m.quad(q, uv, n, [[1.0; 3]; 4], [light; 4], false);
    };
    // Sides: the stick's column of the tile, v from the flame down to the foot.
    let side = [tile::uv(t, lo, 1.0), tile::uv(t, hi, 1.0), tile::uv(t, hi, 1.0 - top), tile::uv(t, lo, 1.0 - top)];
    face([p(lo, lo, 0.0), p(hi, lo, 0.0), p(hi, lo, top), p(lo, lo, top)], side);
    face([p(lo, hi, 0.0), p(hi, hi, 0.0), p(hi, hi, top), p(lo, hi, top)], side);
    face([p(lo, lo, 0.0), p(lo, hi, 0.0), p(lo, hi, top), p(lo, lo, top)], side);
    face([p(hi, lo, 0.0), p(hi, hi, 0.0), p(hi, hi, top), p(hi, lo, top)], side);
    let tip = [tile::uv(t, lo, 1.0 - top), tile::uv(t, hi, 1.0 - top), tile::uv(t, hi, 1.0 - top + 2.0 / 16.0), tile::uv(t, lo, 1.0 - top + 2.0 / 16.0)];
    face([p(lo, lo, top), p(hi, lo, top), p(hi, hi, top), p(lo, hi, top)], tip);
}

/// An item lying in the world, a quarter of a block: whole blocks as a little cube, doors,
/// fences and the like as their boxes, plants and torches as a picture standing up. Its bottom
/// in the middle at the origin, up +y; lit by `light` (sky, block brightness).
pub fn item_mesh(b: block::Block, light: [f32; 2]) -> MeshData {
    let mut m = MeshData::default();
    let pos = |u: f32, v: f32, w: f32| Vec3::new(u - 0.5, w, v - 0.5) * 0.25;
    match block::shape(b) {
        Shape::Cube => emit_boxes(&mut m, block::tiles(b), crate::shape::Boxes::full(), &pos, light, [true; 6]),
        Shape::Boxes => {
            let join = if crate::shape::connects(b) { [true, true, false, false] } else { [false; 4] };
            let boxes = crate::shape::boxes(b, join);
            // Standing boards (a ladder, a door) stand in the middle.
            emit_boxes(&mut m, block::tiles(b), boxes, &pos, light, [true; 6]);
        }
        _ => {
            let t = block::tiles(b)[1];
            let h = 0.4;
            let q = [Vec3::new(-h / 2.0, 0.0, 0.0), Vec3::new(h / 2.0, 0.0, 0.0), Vec3::new(h / 2.0, h, 0.0), Vec3::new(-h / 2.0, h, 0.0)];
            let uv = [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 0.0, 0.0)];
            m.double_quad(q, uv, Vec3::Z, [1.0; 3], light);
        }
    }
    m
}

/// Meshes every chunk of a planet, spread over all CPU cores.
pub fn planet_meshes(planet: &Planet) -> Vec<(ChunkKey, MeshData)> {
    let keys: Vec<ChunkKey> = planet.all_chunks().collect();
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let per = keys.len().div_ceil(threads);
    std::thread::scope(|s| {
        let jobs: Vec<_> = keys.chunks(per).map(|ks| s.spawn(move || ks.iter().map(|&k| (k, chunk_mesh(planet, k))).collect::<Vec<_>>())).collect();
        jobs.into_iter().flat_map(|j| j.join().expect("mesh thread")).collect()
    })
}

/// A stand-in for a planet seen from afar: its terrain's columns on a `res`×`res`-per-face grid,
/// each a flat top at its height, with a wall down to any lower neighbor, colored by its top
/// block (walls by what lies under it). Untextured (the atlas's white tile), fully sky-lit.
pub fn far_mesh(spec: &PlanetSpec, res: usize) -> MeshData {
    let g = CubeSphere::new(res, 1.0, 1);
    let terrain = Terrain::new(spec);
    let cells = g.cell_count();
    let columns: Vec<_> = (0..cells).map(|c| terrain.column(g.center(c))).collect();
    // Seas stand at their level, over the ground under them.
    let radius = |c: usize| spec.radius + columns[c].ground() as f32;
    let white = tile::uv(tile::WHITE, 0.5, 0.5);
    let lit = [[1.0, 0.0]; 4];
    let color = |b: block::Block, at: Vec3| {
        let c = block::color(b);
        if block::tinted(b) { tint(c, at, spec.seed) } else { c }
    };
    let mut m = MeshData::default();
    for c in 0..cells {
        let (f, i, j, _) = g.coords(c);
        let col = &columns[c];
        let r = radius(c);
        let d = [g.dir(f, i, j), g.dir(f, i + 1, j), g.dir(f, i + 1, j + 1), g.dir(f, i, j + 1)];
        let q = d.map(|x| x * r);
        let mut top = [q[0], q[1], q[2], q[3]];
        if (top[1] - top[0]).cross(top[3] - top[0]).dot(top[0]) < 0.0 {
            top.swap(1, 3);
        }
        let tc = match col.liquid {
            _ if col.frozen => block::color(block::ICE),
            // Turquoise shallows to deep blue.
            block::WATER => {
                let deep = ((-col.height) as f32 / 14.0).clamp(0.0, 1.0);
                let (shallow, abyss) = ([0.16, 0.50, 0.60], [0.04, 0.13, 0.36]);
                [0, 1, 2].map(|k| shallow[k] + (abyss[k] - shallow[k]) * deep)
            }
            block::LAVA => block::color(block::LAVA),
            _ => color(col.top, q[0]),
        };
        // Lava glows (alpha 0 flags it for the shader), day or night.
        let alpha = if col.liquid == block::LAVA { 0.0 } else { 1.0 };
        let i0 = m.positions.len();
        m.quad(top, [white; 4], g.center(c).normalize(), [tc; 4], lit, false);
        for v in &mut m.colors[i0..] {
            v[3] = alpha;
        }
        // Walls down to lower neighbors, on the edge they share.
        let (_, sub, deep) = col.biome.layers();
        let wall = if col.top == block::SNOW { block::SNOW } else if col.rocky { block::STONE } else { sub };
        for (s, (a, b)) in [(Side::IMinus, (0, 3)), (Side::IPlus, (1, 2)), (Side::JMinus, (0, 1)), (Side::JPlus, (3, 2))] {
            let Some(nb) = g.neighbor(c, s) else { continue };
            let low = radius(nb);
            if low >= r {
                continue;
            }
            let wc = color(if r - low > 4.0 { deep } else { wall }, q[a]);
            let mut w = [d[a] * low, d[b] * low, d[b] * r, d[a] * r];
            let mid = (w[0] + w[1] + w[2] + w[3]) * 0.25;
            let mut n = (w[1] - w[0]).cross(w[3] - w[0]).normalize();
            let inward = g.center(c).normalize() * mid.length() - mid;
            if n.dot(inward) > 0.0 {
                w.swap(1, 3);
                n = -n;
            }
            m.quad(w, [white; 4], n, [wc.map(|x| x * 0.85); 4], lit, false);
        }
    }
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::{Biome, PlanetSpec};
    use std::collections::HashMap;

    /// Every edge of a closed surface is shared by an even number of triangles; an odd count is a hole.
    #[test]
    fn voxel_mesh_is_watertight() {
        // Barren: nothing but whole blocks (plants are open planes, not part of a closed surface).
        // No seas or caves (lava lakes): liquid surfaces are open sheets.
        let mut spec = PlanetSpec::new(11, "t".into(), 30.0, Biome::Barren);
        spec.knobs.oceans = 0.0;
        spec.knobs.caves = 0.0;
        let p = Planet::generate(spec);
        let mut edges: HashMap<([i64; 3], [i64; 3]), u32> = HashMap::new();
        let q = |v: [f32; 3]| v.map(|x| x.to_bits() as i64);
        for k in p.all_chunks() {
            let m = chunk_mesh(&p, k);
            for t in m.indices.chunks(3) {
                for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                    let (a, b) = (q(m.positions[a as usize]), q(m.positions[b as usize]));
                    *edges.entry(if a < b { (a, b) } else { (b, a) }).or_default() += 1;
                }
            }
        }
        let open: Vec<_> = edges.iter().filter(|e| e.1 % 2 == 1).collect();
        assert!(open.is_empty(), "{} open edges, e.g. {:?}", open.len(), open.first());
    }

    #[test]
    fn shell_keeps_the_surface_and_drops_the_caves() {
        let p = Planet::generate(PlanetSpec::new(5, "t".into(), 120.0, Biome::Meadow));
        let (mut full, mut shell, mut tops_full, mut tops_shell) = (0, 0, 0, 0);
        for k in p.all_chunks() {
            let (f, s) = (chunk_mesh(&p, k), shell_mesh(&p, k));
            full += f.indices.len();
            shell += s.indices.len();
            // Faces pointing away from the core: the ground and plants seen from above.
            let up = |m: &MeshData| (0..m.normals.len()).step_by(4).filter(|&i| Vec3::from(m.normals[i]).dot(Vec3::from(m.positions[i]).normalize()) > 0.9).count();
            tops_full += up(&f);
            tops_shell += up(&s);
        }
        assert!(shell * 10 < full * 9, "shell {shell} vs full {full} indices");
        assert!(tops_shell * 10 > tops_full * 6, "lost too much surface: {tops_shell} of {tops_full} upward faces");
    }
}

/// A station's blocks, in station coordinates (see `station`): the same look as a planet's,
/// flat. Block light and sky light come from `Station::light`.
pub fn station_mesh(s: &crate::station::Station) -> MeshData {
    station_solid(s, &s.light())
}

/// A station's meshes: its blocks (and lava), and its water.
pub fn station_meshes(s: &crate::station::Station) -> Meshes {
    let light = s.light();
    let mut water = MeshData::default();
    for (p, b) in s.cells().chain(s.stray_liquids()) {
        if block::liquid(b) == Some(Liquid::Water) {
            station_liquid(s, &light, &mut water, p, b);
        }
    }
    Meshes { solid: station_solid(s, &light), water }
}

/// A liquid cell on a station (see `planet_liquid`).
fn station_liquid(s: &crate::station::Station, light: &crate::station::Light, m: &mut MeshData, p: glam::IVec3, b: block::Block) {
    use glam::IVec3;
    let Some(kind) = block::liquid(b) else { return };
    let lit = |q: IVec3| {
        let ((a, g), (c, h)) = (light.at(q), light.at(p));
        [light::brightness(a.max(c) as f32), light::glow(g.max(h) as f32)]
    };
    let shows = |q: IVec3| {
        let nb = s.get(q);
        block::liquid(nb) != Some(kind) && !block::opaque(nb)
    };
    let heights = [(0, 0), (1, 0), (1, 1), (0, 1)].map(|(dx, dz)| {
        let around = [(-1, -1), (0, -1), (0, 0), (-1, 0)].into_iter().map(|(ox, oz)| {
            let q = p + IVec3::new(dx + ox, 0, dz + oz);
            (s.get(q), s.get(q + IVec3::Y))
        });
        corner_height(kind, around)
    });
    let base = p.as_vec3();
    let pos = |u: f32, v: f32, w: f32| base + Vec3::new(u, w, v);
    let side = |d: IVec3| shows(p + d).then(|| lit(p + d));
    let cell = LiquidCell {
        kind,
        heights,
        pos: &pos,
        top: (block::liquid(s.get(p + IVec3::Y)) != Some(kind)).then(|| lit(p + IVec3::Y)),
        sides: [side(IVec3::NEG_X), side(IVec3::X), side(IVec3::NEG_Z), side(IVec3::Z)],
        bottom: side(IVec3::NEG_Y),
    };
    emit_liquid(m, &cell);
}

fn station_solid(s: &crate::station::Station, light: &crate::station::Light) -> MeshData {
    use glam::IVec3;
    let lit = |p: IVec3| {
        let (sky, glow) = light.at(p);
        [light::brightness(sky as f32), light::glow(glow as f32)]
    };
    let mut m = MeshData::default();
    // Corners of each side, counter-clockwise seen from outside, bottom (v = 1) first for the
    // sides so the texture stands upright; and the side's outward normal.
    let sides: [(IVec3, [Vec3; 4]); 6] = [
        (IVec3::Y, [Vec3::new(0., 1., 0.), Vec3::new(0., 1., 1.), Vec3::new(1., 1., 1.), Vec3::new(1., 1., 0.)]),
        (IVec3::NEG_Y, [Vec3::new(0., 0., 0.), Vec3::new(1., 0., 0.), Vec3::new(1., 0., 1.), Vec3::new(0., 0., 1.)]),
        (IVec3::X, [Vec3::new(1., 0., 1.), Vec3::new(1., 0., 0.), Vec3::new(1., 1., 0.), Vec3::new(1., 1., 1.)]),
        (IVec3::NEG_X, [Vec3::new(0., 0., 0.), Vec3::new(0., 0., 1.), Vec3::new(0., 1., 1.), Vec3::new(0., 1., 0.)]),
        (IVec3::Z, [Vec3::new(0., 0., 1.), Vec3::new(1., 0., 1.), Vec3::new(1., 1., 1.), Vec3::new(0., 1., 1.)]),
        (IVec3::NEG_Z, [Vec3::new(1., 0., 0.), Vec3::new(0., 0., 0.), Vec3::new(0., 1., 0.), Vec3::new(1., 1., 0.)]),
    ];
    let seed = 0x57A7_1011u64;
    for (p, b) in s.cells().chain(s.stray_liquids()) {
        let base = p.as_vec3();
        match block::shape(b) {
            Shape::Empty => {}
            Shape::Liquid => {
                if block::liquid(b) == Some(Liquid::Lava) {
                    station_liquid(s, light, &mut m, p, b);
                }
            }
            Shape::Cube => {
                let tiles = block::tiles(b);
                let jitter = 0.95 + (mix((p.x as u64) ^ mix((p.y as u64) ^ mix(p.z as u64))) % 1000) as f32 * 0.0001;
                for (n, corners) in sides {
                    let nb = p + n;
                    if block::face_hidden(b, s.get(nb)) {
                        continue;
                    }
                    let q = corners.map(|c| base + c);
                    let (t, shade) = match n.y {
                        1 => (tiles[0], 1.0),
                        -1 => (tiles[2], 0.6),
                        _ => (tiles[1], 0.82),
                    };
                    let uv = if n.y != 0 {
                        [tile::uv(t, 0.0, 0.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 0.0, 1.0)]
                    } else {
                        [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 0.0, 0.0)]
                    };
                    let center = base + Vec3::splat(0.5);
                    let tinted = block::tinted(b) && (b != block::GRASS || n.y == 1);
                    let color = if tinted { tint([1.0; 3], center, seed) } else { [1.0; 3] };
                    // Occlusion and smooth light from the cells in front of each corner.
                    let nf = n.as_vec3();
                    let mut lights = [[0.0; 2]; 4];
                    let ao = [0, 1, 2, 3].map(|i| {
                        let v = q[i];
                        let u = (q[(i + 1) % 4] - v).normalize();
                        let w = (q[(i + 3) % 4] - v).normalize();
                        let front = v + nf * 0.5;
                        let at = |x: Vec3| crate::station::Station::cell_at(x);
                        let c1 = at(front - u * 0.5 + w * 0.5);
                        let c2 = at(front + u * 0.5 - w * 0.5);
                        let cc = at(front - u * 0.5 - w * 0.5);
                        let (s1, s2, corner) = (s.opaque(c1), s.opaque(c2), s.opaque(cc));
                        let mut cells = vec![nb];
                        if !s1 {
                            cells.push(c1);
                        }
                        if !s2 {
                            cells.push(c2);
                        }
                        if !(s1 && s2) && !corner {
                            cells.push(cc);
                        }
                        let k = cells.len() as f32;
                        let (sky, glow) = cells.iter().fold((0.0, 0.0), |(a, b), &c| {
                            let (x, y) = light.at(c);
                            (a + x as f32, b + y as f32)
                        });
                        lights[i] = [light::brightness(sky / k), light::glow(glow / k)];
                        if s1 && s2 { 3 } else { s1 as usize + s2 as usize + corner as usize }
                    });
                    let colors = ao.map(|o| color.map(|x| x * shade * jitter * AO[o]));
                    m.quad(q, uv, nf, colors, lights, ao[0] + ao[2] > ao[1] + ao[3]);
                }
            }
            Shape::Cross => {
                let center = base + Vec3::splat(0.5);
                let color = if block::tinted(b) { tint([1.0; 3], center, seed) } else { [1.0; 3] };
                let t = block::tiles(b)[1];
                let uv = [tile::uv(t, 0.0, 1.0), tile::uv(t, 1.0, 1.0), tile::uv(t, 1.0, 0.0), tile::uv(t, 0.0, 0.0)];
                let (a, z) = (0.15, 0.85);
                for (x0, z0, x1, z1) in [(a, a, z, z), (z, a, a, z)] {
                    let q = [base + Vec3::new(x0, 0.0, z0), base + Vec3::new(x1, 0.0, z1), base + Vec3::new(x1, 1.0, z1), base + Vec3::new(x0, 1.0, z0)];
                    m.double_quad(q, uv, Vec3::Y, color, lit(p));
                }
            }
            Shape::Boxes => {
                let open = [IVec3::NEG_X, IVec3::X, IVec3::NEG_Z, IVec3::Z, IVec3::NEG_Y, IVec3::Y].map(|d| !block::opaque(s.get(p + d)));
                let pos = |u: f32, v: f32, w: f32| base + Vec3::new(u, w, v);
                let join = crate::shape::joined(s, p);
                emit_boxes(&mut m, block::tiles(b), crate::shape::boxes(b, join), &pos, lit(p), open);
            }
            Shape::Torch => {
                let t = block::tiles(b)[1];
                let (lo, hi, top) = (7.0 / 16.0, 9.0 / 16.0, 10.0 / 16.0);
                let at = |x: f32, y: f32, z: f32| {
                    let (u, v, w) = torch_at(b, x, z, y);
                    base + Vec3::new(u, w, v)
                };
                let l = lit(p);
                let center = at(0.5, top * 0.5, 0.5);
                let mut face = |mut q: [Vec3; 4], mut uv: [[f32; 2]; 4]| {
                    let mut n = (q[1] - q[0]).cross(q[3] - q[0]).normalize_or(Vec3::Y);
                    let mid = (q[0] + q[1] + q[2] + q[3]) * 0.25;
                    if n.dot(mid - center) < 0.0 {
                        q.swap(1, 3);
                        uv.swap(1, 3);
                        n = -n;
                    }
                    m.quad(q, uv, n, [[1.0; 3]; 4], [l; 4], false);
                };
                let side = [tile::uv(t, lo, 1.0), tile::uv(t, hi, 1.0), tile::uv(t, hi, 1.0 - top), tile::uv(t, lo, 1.0 - top)];
                face([at(lo, 0.0, lo), at(hi, 0.0, lo), at(hi, top, lo), at(lo, top, lo)], side);
                face([at(lo, 0.0, hi), at(hi, 0.0, hi), at(hi, top, hi), at(lo, top, hi)], side);
                face([at(lo, 0.0, lo), at(lo, 0.0, hi), at(lo, top, hi), at(lo, top, lo)], side);
                face([at(hi, 0.0, lo), at(hi, 0.0, hi), at(hi, top, hi), at(hi, top, lo)], side);
                let tip = [tile::uv(t, lo, 1.0 - top), tile::uv(t, hi, 1.0 - top), tile::uv(t, hi, 1.0 - top + 2.0 / 16.0), tile::uv(t, lo, 1.0 - top + 2.0 / 16.0)];
                face([at(lo, top, lo), at(hi, top, lo), at(hi, top, hi), at(lo, top, hi)], tip);
            }
        }
    }
    m
}

#[cfg(test)]
mod station_tests {
    use super::*;
    use crate::station::Station;

    /// Thin flowing liquid still faces up (a surface low in its cell once faced down, and was
    /// culled from above).
    #[test]
    fn liquid_faces_point_outward() {
        let mut s = Station::new("00000002".into(), "", block::SMOOTH_STONE);
        s.set(glam::IVec3::new(2, 1, 2), Liquid::Lava.flowing(1));
        s.set(glam::IVec3::new(-2, 1, 2), Liquid::Water.flowing(2));
        let m = station_meshes(&s);
        for mesh in [&m.solid, &m.water] {
            for t in mesh.indices.chunks(3) {
                let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(mesh.positions[i as usize]));
                let n = Vec3::from(mesh.normals[t[0] as usize]);
                assert!((b - a).cross(c - a).normalize().dot(n) > 0.99, "a triangle winds against its normal");
            }
            // The top: its normal points up.
            let tops = mesh.normals.iter().zip(&mesh.positions).filter(|(n, p)| n[1] > 0.9 && p[1] > 1.0 && p[1] < 1.5).count();
            assert!(tops >= 4, "no upward top face");
        }
    }

    #[test]
    fn station_faces_point_outward() {
        let s = Station::new("00000001".into(), "", block::SMOOTH_STONE);
        let m = station_mesh(&s);
        // A 9 × 9 × 1 slab: top and bottom 81 each, 36 around the rim.
        assert_eq!(m.indices.len() / 6, 81 * 2 + 36);
        for t in m.indices.chunks(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(m.positions[i as usize]));
            let n = Vec3::from(m.normals[t[0] as usize]);
            assert!((b - a).cross(c - a).normalize().dot(n) > 0.99, "a triangle winds against its normal");
        }
    }
}
