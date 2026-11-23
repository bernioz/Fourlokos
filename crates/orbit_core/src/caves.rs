//! Caves as Minecraft 1.7 digs them (MapGenCaves, MapGenRavine), ported from GalaxyCraft: systems
//! of narrow tunnels (2 to 4 blocks) that wander level with the ground and fork once, a few small
//! rooms, a rare ravine. Systems are spread out so most stand alone rather than form a maze. Space around the
//! planet is cut into 16-block cubes; each that holds ground rolls from the seed and its place
//! whether it starts a system, so the same seed digs the same caves. "Level" is along the sphere:
//! a tunnel's pitch is taken against the up where it is, so it curves with the planet.

use crate::block::{self, Block};
use crate::rng::{Rng, mix};
use crate::sphere::{CellId, CubeSphere, Side};
use glam::{Quat, Vec3};
use std::collections::HashSet;

const CUBE: f32 = 16.0;
/// A tunnel's widest half width, blocks (rooms aside).
const TUNNEL_RADIUS: f32 = 2.0;
const ROOM_RADIUS: f32 = 3.5;

/// The cells to dig out of `blocks`. `amount` scales how many systems start (0 none), `size`
/// how wide they are. A cell's fate depends only on what caves never change (its layer, the
/// bedrock, the sky), so cubes dig at once on every core and in any order.
pub fn carve(
    grid: &CubeSphere,
    depth: usize,
    blocks: &[Block],
    seed: u64,
    amount: f32,
    size: f32,
) -> Vec<CellId> {
    if amount <= 0.0 {
        return Vec::new();
    }
    let (surface, inner, outer) = (
        grid.core + depth as f32,
        grid.core + 1.0,
        grid.core + grid.layers as f32,
    );
    let reach = (outer / CUBE).ceil() as i32;
    let mut cubes = Vec::new();
    for x in -reach..reach {
        for y in -reach..reach {
            for z in -reach..reach {
                let mid = ((Vec3::new(x as f32, y as f32, z as f32) + 0.5) * CUBE).length();
                if mid >= inner - CUBE && mid <= surface + CUBE {
                    cubes.push([x, y, z]);
                }
            }
        }
    }
    let (systems, ravines) = (0.12 * amount, 0.012 * amount);
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let per = cubes.len().div_ceil(threads).max(1);
    let mut dug: Vec<CellId> = std::thread::scope(|s| {
        let jobs: Vec<_> = cubes
            .chunks(per)
            .map(|part| {
                s.spawn(move || {
                    let mut d = Digger {
                        grid,
                        blocks,
                        size,
                        out: Vec::new(),
                        seen: HashSet::new(),
                        queue: Vec::new(),
                    };
                    for &[x, y, z] in part {
                        let mut rng = Rng::new(
                            seed ^ mix((x as i64 as u64).wrapping_mul(341873128712)
                                ^ (y as i64 as u64).wrapping_mul(132897987541)
                                ^ (z as i64 as u64).wrapping_mul(42317861)),
                        );
                        if rng.f32() < systems {
                            if let Some(start) = start(&mut rng, [x, y, z], inner, surface) {
                                let tunnels = rng.int(1, 3);
                                if rng.int(0, 5) == 0 {
                                    let r = ROOM_RADIUS - rng.f32();
                                    d.dig(start, r, r * 0.8);
                                }
                                for _ in 0..tunnels {
                                    let (s, yaw, pitch) = (
                                        rng.next_u64(),
                                        rng.range(0.0, std::f32::consts::TAU),
                                        (rng.f32() - 0.5) / 4.0,
                                    );
                                    let (width, length) = (
                                        0.2 + rng.f32() * (TUNNEL_RADIUS - 1.2),
                                        rng.int(40, 119) as usize,
                                    );
                                    d.tunnel(
                                        s,
                                        start,
                                        heading(start, yaw),
                                        pitch,
                                        width,
                                        length,
                                        1,
                                        false,
                                    );
                                }
                            }
                        }
                        if rng.f32() < ravines {
                            if let Some(start) = start(&mut rng, [x, y, z], inner + 4.0, surface) {
                                let (s, yaw, length) = (
                                    rng.next_u64(),
                                    rng.range(0.0, std::f32::consts::TAU),
                                    rng.int(60, 109) as usize,
                                );
                                d.tunnel(s, start, heading(start, yaw), 0.0, 0.5, length, 0, true);
                            }
                        }
                    }
                    d.out
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().expect("cave thread"))
            .collect()
    });
    dug.sort_unstable();
    dug.dedup();
    dug
}

/// A point in the cube, in the ground (biased deep, as 1.7's), or None.
fn start(rng: &mut Rng, [x, y, z]: [i32; 3], inner: f32, surface: f32) -> Option<Vec3> {
    let p = Vec3::new(
        x as f32 + rng.f32(),
        y as f32 + rng.f32(),
        z as f32 + rng.f32(),
    ) * CUBE;
    let r = inner + 1.0 + rng.f32() * rng.f32() * (surface - inner);
    let p = p.normalize_or_zero() * r;
    let q = (p / CUBE).floor();
    (q == Vec3::new(x as f32, y as f32, z as f32)).then_some(p)
}

/// A heading along the ground at p, turned `yaw` from a fixed reference.
fn heading(p: Vec3, yaw: f32) -> Vec3 {
    let up = p.normalize();
    let h = if up.y.abs() < 0.9 {
        up.cross(Vec3::Y)
    } else {
        up.cross(Vec3::X)
    }
    .normalize();
    Quat::from_axis_angle(up, yaw) * h
}

struct Digger<'a> {
    grid: &'a CubeSphere,
    blocks: &'a [Block],
    /// Width scale.
    size: f32,
    out: Vec<CellId>,
    seen: HashSet<CellId>,
    queue: Vec<CellId>,
}

impl Digger<'_> {
    /// One worm: 1.7's drift of yaw and pitch, a step a block, splitting in two (branches deep).
    #[allow(clippy::too_many_arguments)]
    fn tunnel(
        &mut self,
        seed: u64,
        from: Vec3,
        mut heading: Vec3,
        mut pitch: f32,
        width: f32,
        length: usize,
        branches: u32,
        ravine: bool,
    ) {
        let mut rng = Rng::new(seed);
        let mut p = from;
        let (mut d_yaw, mut d_pitch) = (0.0f32, 0.0f32);
        let branch_at = length / 4 + rng.int(0, (length / 2).max(1) as i64 - 1) as usize;
        let g = self.grid;
        for step in 0..length {
            let radius = 1.0 + (step as f32 * std::f32::consts::PI / length as f32).sin() * width;
            if ravine {
                self.dig(p, radius * 1.5, radius * 4.5);
            } else {
                self.dig(p, radius, radius * 0.85);
            }
            let up = p.normalize();
            // Keep the heading along the ground where it is now, then tilt it by the pitch.
            heading = (heading - up * heading.dot(up)).normalize_or(up.any_orthonormal_vector());
            p += heading * pitch.cos() + up * pitch.sin();
            pitch = pitch * 0.7 + d_pitch * 0.1;
            heading = Quat::from_axis_angle(up, d_yaw * 0.1) * heading;
            d_pitch = d_pitch * (if ravine { 0.8 } else { 0.9 })
                + (rng.f32() - rng.f32()) * rng.f32() * 2.0;
            d_yaw = d_yaw * (if ravine { 0.5 } else { 0.75 })
                + (rng.f32() - rng.f32()) * rng.f32() * 4.0;
            if !ravine && step == branch_at && branches > 0 {
                for side in [-1.0f32, 1.0] {
                    let h = Quat::from_axis_angle(up, side * std::f32::consts::FRAC_PI_2) * heading;
                    self.tunnel(
                        rng.next_u64(),
                        p,
                        h,
                        pitch / 3.0,
                        width * 0.8,
                        (length - step).max(10),
                        branches - 1,
                        false,
                    );
                }
                return; // a worm ends where it splits, as in 1.7
            }
            let r = p.length();
            if r < g.core + 1.0 || r > g.core + g.layers as f32 {
                return;
            }
        }
    }

    /// Digs the cells whose centers lie in the ellipsoid around `at`: `across` half width,
    /// `up_half` half height (along up).
    fn dig(&mut self, at: Vec3, across: f32, up_half: f32) {
        let (across, up_half) = (across * self.size, up_half * self.size);
        let Some(start) = self.grid.cell_at(at) else {
            return;
        };
        let up = at.normalize();
        self.seen.clear();
        self.queue.clear();
        self.queue.push(start);
        self.seen.insert(start);
        while let Some(cell) = self.queue.pop() {
            let rel = self.grid.center(cell) - at;
            let v = rel.dot(up);
            let h2 = rel.length_squared() - v * v;
            if h2 / (across * across) + v * v / (up_half * up_half) >= 1.0 && cell != start {
                continue;
            }
            let (_, _, _, k) = self.grid.coords(cell);
            let b = self.blocks[cell];
            if k > 0 && b != block::AIR && b != block::CORE {
                self.out.push(cell);
            }
            for s in Side::ALL {
                if let Some(nb) = self.grid.neighbor(cell, s) {
                    if self.seen.insert(nb) {
                        self.queue.push(nb);
                    }
                }
            }
        }
    }
}
