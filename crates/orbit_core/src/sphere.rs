//! Cube-sphere cell grid: a cube projected onto a sphere (equal-angle mapping), each face an
//! n×n grid of columns, radial layers one block thick. Cell (face, i, j, k) spans radii
//! [core + k, core + k + 1]. Neighbors share corners exactly, so meshes are watertight.

use glam::{DVec3, Vec3};

pub type CellId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    IMinus,
    IPlus,
    JMinus,
    JPlus,
    Bottom,
    Top,
}

impl Side {
    pub const ALL: [Side; 6] = [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus, Side::Bottom, Side::Top];
}

/// Per face: normal N, then U and V with U × V = N.
const BASIS: [[[f64; 3]; 3]; 6] = [
    [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
    [[-1., 0., 0.], [0., 0., 1.], [0., 1., 0.]],
    [[0., 1., 0.], [0., 0., 1.], [1., 0., 0.]],
    [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]],
    [[0., 0., 1.], [1., 0., 0.], [0., 1., 0.]],
    [[0., 0., -1.], [0., 1., 0.], [1., 0., 0.]],
];

fn v(a: [f64; 3]) -> DVec3 {
    DVec3::from_array(a)
}

#[derive(Clone, Debug)]
pub struct CubeSphere {
    pub n: usize,
    pub core: f32,
    pub layers: usize,
    /// Unit directions of every face's (n+1)² grid vertices.
    dirs: Vec<Vec3>,
}

impl CubeSphere {
    pub fn new(n: usize, core: f32, layers: usize) -> Self {
        let mut dirs = Vec::with_capacity(6 * (n + 1) * (n + 1));
        for f in 0..6 {
            for i in 0..=n {
                for j in 0..=n {
                    let b = BASIS[f];
                    let d = v(b[0]) + v(b[1]) * tan_grid(n, i) + v(b[2]) * tan_grid(n, j);
                    dirs.push(d.normalize().as_vec3());
                }
            }
        }
        Self { n, core, layers, dirs }
    }

    pub fn cell_count(&self) -> usize {
        6 * self.n * self.n * self.layers
    }

    pub fn index(&self, face: usize, i: usize, j: usize, k: usize) -> CellId {
        ((face * self.n + i) * self.n + j) * self.layers + k
    }

    /// (face, i, j, k)
    pub fn coords(&self, c: CellId) -> (usize, usize, usize, usize) {
        let k = c % self.layers;
        let col = c / self.layers;
        let j = col % self.n;
        let i = (col / self.n) % self.n;
        (col / (self.n * self.n), i, j, k)
    }

    pub fn radius(&self, k: usize) -> f32 {
        self.core + k as f32
    }

    pub fn dir(&self, face: usize, i: usize, j: usize) -> Vec3 {
        self.dirs[(face * (self.n + 1) + i) * (self.n + 1) + j]
    }

    pub fn corner(&self, c: CellId, di: usize, dj: usize, dk: usize) -> Vec3 {
        let (f, i, j, k) = self.coords(c);
        self.dir(f, i + di, j + dj) * self.radius(k + dk)
    }

    /// A point inside cell `c` at fractions (u along i, v along j, w up), each 0..1: the
    /// directions blend across the column, the radius runs through the layer.
    pub fn point(&self, c: CellId, u: f32, v: f32, w: f32) -> Vec3 {
        let (f, i, j, k) = self.coords(c);
        let d = self.dir(f, i, j) * (1.0 - u) * (1.0 - v) + self.dir(f, i + 1, j) * u * (1.0 - v) + self.dir(f, i, j + 1) * (1.0 - u) * v + self.dir(f, i + 1, j + 1) * u * v;
        d.normalize() * (self.radius(k) + w)
    }

    pub fn center(&self, c: CellId) -> Vec3 {
        let (f, i, j, k) = self.coords(c);
        let d = (self.dir(f, i, j) + self.dir(f, i + 1, j) + self.dir(f, i, j + 1) + self.dir(f, i + 1, j + 1)).normalize();
        d * (self.radius(k) + 0.5)
    }

    /// The 4 corners of a side, in no particular winding.
    pub fn side_quad(&self, c: CellId, s: Side) -> [Vec3; 4] {
        let q = |a: (usize, usize, usize)| self.corner(c, a.0, a.1, a.2);
        match s {
            Side::IMinus => [q((0, 0, 0)), q((0, 1, 0)), q((0, 1, 1)), q((0, 0, 1))],
            Side::IPlus => [q((1, 0, 0)), q((1, 1, 0)), q((1, 1, 1)), q((1, 0, 1))],
            Side::JMinus => [q((0, 0, 0)), q((1, 0, 0)), q((1, 0, 1)), q((0, 0, 1))],
            Side::JPlus => [q((0, 1, 0)), q((1, 1, 0)), q((1, 1, 1)), q((0, 1, 1))],
            Side::Bottom => [q((0, 0, 0)), q((1, 0, 0)), q((1, 1, 0)), q((0, 1, 0))],
            Side::Top => [q((0, 0, 1)), q((1, 0, 1)), q((1, 1, 1)), q((0, 1, 1))],
        }
    }

    /// Column (face, i, j) under a planet-local direction.
    pub fn column_at(&self, p: Vec3) -> (usize, usize, usize) {
        let p = p.as_dvec3();
        let (mut face, mut best) = (0, f64::MIN);
        for (f, b) in BASIS.iter().enumerate() {
            let d = v(b[0]).dot(p);
            if d > best {
                best = d;
                face = f;
            }
        }
        let b = BASIS[face];
        (face, grid(self.n, v(b[1]).dot(p) / best), grid(self.n, v(b[2]).dot(p) / best))
    }

    /// Face whose square a planet-local direction points through.
    pub fn face_of(&self, p: DVec3) -> usize {
        (0..6).max_by(|&a, &b| v(BASIS[a][0]).dot(p).total_cmp(&v(BASIS[b][0]).dot(p))).unwrap()
    }

    /// Continuous grid coordinates (a, b) of a planet-local point on `face`: the point lies in
    /// column (floor a, floor b). Past [0, n] the same equal-angle grid runs on over the edge, so
    /// a box near an edge can be handled in one face's coordinates. `p` must be in front of the face.
    pub fn face_coords(&self, face: usize, p: DVec3) -> (f64, f64) {
        let b = BASIS[face];
        let d = v(b[0]).dot(p);
        (grid_coord(self.n, v(b[1]).dot(p) / d), grid_coord(self.n, v(b[2]).dot(p) / d))
    }

    /// The point at grid coordinates (a, b) on `face` and radius r; inverse of `face_coords`.
    pub fn face_point(&self, face: usize, a: f64, b: f64, r: f64) -> DVec3 {
        let f = BASIS[face];
        (v(f[0]) + v(f[1]) * grid_tan(self.n, a) + v(f[2]) * grid_tan(self.n, b)).normalize() * r
    }

    /// Cell at integer grid coordinates on `face`; i and j may run past the face's edge onto its
    /// neighbors. None outside the layers.
    pub fn cell_on_face(&self, face: usize, i: i64, j: i64, k: i64) -> Option<CellId> {
        if k < 0 || k >= self.layers as i64 {
            return None;
        }
        let n = self.n as i64;
        if (0..n).contains(&i) && (0..n).contains(&j) {
            return Some(self.index(face, i as usize, j as usize, k as usize));
        }
        let c = self.face_point(face, i as f64 + 0.5, j as f64 + 0.5, self.radius(k as usize) as f64 + 0.5);
        self.cell_at(c.as_vec3())
    }

    /// Layer index containing radius r (may be out of range).
    pub fn layer_of(&self, r: f32) -> i64 {
        (r - self.core).floor() as i64
    }

    /// The cell containing a planet-local point, or None outside the layers.
    pub fn cell_at(&self, p: Vec3) -> Option<CellId> {
        let k = self.layer_of(p.length());
        if p == Vec3::ZERO || k < 0 || k as usize >= self.layers {
            return None;
        }
        let (f, i, j) = self.column_at(p);
        Some(self.index(f, i, j, k as usize))
    }

    /// Neighbor across a side, or None past the innermost or outermost layer.
    pub fn neighbor(&self, c: CellId, s: Side) -> Option<CellId> {
        let (_, i, j, k) = self.coords(c);
        let (n, l) = (self.n, self.layers);
        match s {
            Side::Top => return (k + 1 < l).then_some(c + 1),
            Side::Bottom => return (k > 0).then(|| c - 1),
            Side::IMinus if i > 0 => return Some(c - n * l),
            Side::IPlus if i + 1 < n => return Some(c + n * l),
            Side::JMinus if j > 0 => return Some(c - l),
            Side::JPlus if j + 1 < n => return Some(c + l),
            _ => {}
        }
        // Across a cube edge: probe just past the side's midpoint, keeping the shell radius.
        let q = self.side_quad(c, s);
        let mid = (q[0] + q[1] + q[2] + q[3]) * 0.25;
        let out = (mid + (mid - self.center(c)) * 0.05).normalize() * (self.radius(k) + 0.5);
        self.cell_at(out)
    }
}

/// tan of grid line i's angle, exactly odd about the middle and exactly ±1 on the edges so
/// faces meeting on an edge compute bit-identical directions.
fn tan_grid(n: usize, i: usize) -> f64 {
    if 2 * i > n {
        return -tan_grid(n, n - i);
    }
    if i == 0 {
        return -1.0;
    }
    if 2 * i == n {
        return 0.0;
    }
    ((-1.0 + 2.0 * i as f64 / n as f64) * std::f64::consts::FRAC_PI_4).tan()
}

/// Continuous grid coordinate of a tangent; `grid` is its floor, clamped to the face.
fn grid_coord(n: usize, tan: f64) -> f64 {
    (tan.atan() * 4.0 / std::f64::consts::PI + 1.0) / 2.0 * n as f64
}

fn grid_tan(n: usize, a: f64) -> f64 {
    ((2.0 * a / n as f64 - 1.0) * std::f64::consts::FRAC_PI_4).tan()
}

fn grid(n: usize, tan: f64) -> usize {
    let g = ((tan.atan() * 4.0 / std::f64::consts::PI + 1.0) / 2.0 * n as f64).floor() as i64;
    g.clamp(0, n as i64 - 1) as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_maps_back_to_cell() {
        let s = CubeSphere::new(20, 10.0, 8);
        for c in (0..s.cell_count()).step_by(7) {
            assert_eq!(s.cell_at(s.center(c)), Some(c));
        }
    }

    #[test]
    fn face_coords_round_trip_and_match_cells() {
        let s = CubeSphere::new(20, 10.0, 8);
        for c in (0..s.cell_count()).step_by(5) {
            let (f, i, j, k) = s.coords(c);
            let p = s.center(c).as_dvec3();
            let (a, b) = s.face_coords(f, p);
            assert_eq!((a.floor() as usize, b.floor() as usize), (i, j));
            assert!(s.face_point(f, a, b, p.length()).distance(p) < 1e-9);
            assert_eq!(s.cell_on_face(f, i as i64, j as i64, k as i64), Some(c));
        }
        // Past the edge the grid continues onto the neighbor face.
        let c = s.index(0, 19, 4, 2);
        assert_eq!(s.cell_on_face(0, 20, 4, 2), s.neighbor(c, Side::IPlus));
    }

    #[test]
    fn neighbors_are_symmetric_everywhere() {
        let s = CubeSphere::new(12, 6.0, 3);
        for c in 0..s.cell_count() {
            for side in [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus] {
                let nb = s.neighbor(c, side).expect("horizontal neighbor always exists");
                assert_ne!(nb, c);
                let back = Side::ALL.iter().any(|&b| s.neighbor(nb, b) == Some(c));
                assert!(back, "cell {c} side {side:?}");
            }
        }
    }
}

#[cfg(test)]
mod seam_tests {
    use super::*;

    /// Across every side, the neighbor must share that exact face (same 4 corners).
    #[test]
    fn neighbors_share_the_same_face() {
        let s = CubeSphere::new(126, 64.0, 3);
        let key = |v: Vec3| v.to_array().map(|x| x.to_bits());
        let mut bad = 0;
        for c in 0..s.cell_count() {
            for side in [Side::IMinus, Side::IPlus, Side::JMinus, Side::JPlus] {
                let nb = s.neighbor(c, side).unwrap();
                let mut a: Vec<_> = s.side_quad(c, side).map(key).to_vec();
                a.sort();
                let shared = Side::ALL.iter().any(|&t| {
                    let mut b: Vec<_> = s.side_quad(nb, t).map(key).to_vec();
                    b.sort();
                    a == b
                });
                if !shared {
                    bad += 1;
                }
            }
        }
        assert_eq!(bad, 0);
    }
}
