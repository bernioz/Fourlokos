//! Items lying in the world, as Minecraft's: what a broken block drops in survival, or what Q
//! throws. They fall (toward the planet's middle, or a station's floor), float in water, rest
//! on whatever holds them (the boxes of fences and trapdoors too), and the player picks them up
//! by walking over them, after a moment.

use crate::block;
use crate::inventory::Stack;
use crate::planet::Planet;
use crate::station::Station;
use glam::{IVec3, Vec3};

/// Seconds before a dropped item can be picked up (thrown ones wait longer: you just threw it).
pub const PICKUP_DELAY: f32 = 0.5;
pub const THROWN_DELAY: f32 = 2.0;
/// Seconds an item lies before it is gone.
pub const LIFETIME: f32 = 300.0;
/// How near the player's middle an item is picked up from.
pub const PICKUP_RANGE: f32 = 1.4;
const GRAVITY: f32 = 20.0;

#[derive(Clone, Copy, Debug)]
pub struct ItemDrop {
    pub stack: Stack,
    /// Its bottom, in its frame (planet-local or station coordinates).
    pub pos: Vec3,
    pub vel: Vec3,
    pub age: f32,
    /// Seconds until it can be picked up.
    pub delay: f32,
    pub resting: bool,
}

impl ItemDrop {
    /// An item popping out of the cell around `center`, with a little random toss (`r` in 0..1 ×3).
    pub fn popped(stack: Stack, center: Vec3, up: Vec3, r: [f32; 3]) -> Self {
        let side = up.any_orthonormal_vector();
        let other = up.cross(side);
        let toss = side * (r[0] - 0.5) * 2.0 + other * (r[1] - 0.5) * 2.0 + up * (2.0 + r[2] * 1.5);
        Self { stack, pos: center - up * 0.15, vel: toss, age: 0.0, delay: PICKUP_DELAY, resting: false }
    }

    /// Thrown from the eye along `look`.
    pub fn thrown(stack: Stack, eye: Vec3, look: Vec3, up: Vec3) -> Self {
        Self { stack, pos: eye - up * 0.3, vel: look * 6.0 + up * 1.2, age: 0.0, delay: THROWN_DELAY, resting: false }
    }

    /// One step, `down` the way gravity pulls (unit), `solid` whether a point is inside
    /// something that holds items up, `wet` whether it is in water or lava.
    pub fn step(&mut self, down: Vec3, solid: impl Fn(Vec3) -> bool, wet: bool, dt: f32) {
        self.age += dt;
        self.delay = (self.delay - dt).max(0.0);
        if solid(self.pos) {
            // A block went where it lies: it works its way out, upward.
            self.pos -= down * 3.0 * dt;
            self.vel = Vec3::ZERO;
            return;
        }
        self.vel += down * GRAVITY * dt;
        if wet {
            // Floats up, slowly, and the water slows it.
            self.vel -= down * GRAVITY * 1.15 * dt;
            self.vel *= (-4.0 * dt).exp();
        } else {
            self.vel *= (-0.3 * dt).exp();
        }
        let vr = self.vel.dot(down);
        let vt = self.vel - down * vr;
        let next = self.pos + down * vr * dt;
        self.resting = false;
        if solid(next) {
            self.vel = vt;
            self.resting = vr > 0.0;
        } else {
            self.pos = next;
        }
        let next = self.pos + vt * dt;
        if solid(next) {
            self.vel -= vt;
        } else {
            self.pos = next;
        }
        if self.resting {
            let vt = self.vel - down * self.vel.dot(down);
            self.vel -= vt * (1.0 - (-10.0 * dt).exp());
        }
    }

    /// One step on a planet (its frame).
    pub fn step_planet(&mut self, planet: &Planet, dt: f32) {
        let down = -self.pos.normalize_or(Vec3::Y);
        let wet = block::is_liquid(planet.block_at(self.pos + down * -0.1));
        self.step(down, |p| planet.holds_at(p), wet, dt);
    }

    /// One step on a station (its frame): it falls toward the floor (-y).
    pub fn step_station(&mut self, s: &Station, dt: f32) {
        let wet = block::is_liquid(s.get(Station::cell_at(self.pos + Vec3::Y * 0.1)));
        self.step(Vec3::NEG_Y, |p| s.holds_at(p), wet, dt);
    }

    /// Two drops of the same item close together become one (as much as one stack holds).
    pub fn merge(&mut self, other: &mut ItemDrop) -> bool {
        if self.stack.item != other.stack.item || self.pos.distance_squared(other.pos) > 0.5 * 0.5 {
            return false;
        }
        let n = other.stack.count.min(self.stack.max().saturating_sub(self.stack.count));
        self.stack.count += n;
        other.stack.count -= n;
        other.stack.count == 0
    }
}

impl Planet {
    /// True if a planet-local point is inside a block's collision boxes.
    pub fn holds_at(&self, p: Vec3) -> bool {
        if p.length() < self.grid.core {
            return true;
        }
        let Some(c) = self.grid.cell_at(p) else { return false };
        let b = self.get(c);
        match block::shape(b) {
            block::Shape::Cube => true,
            block::Shape::Boxes => {
                let (face, i, j, k) = self.grid.coords(c);
                let (a, bb) = self.grid.face_coords(face, p.as_dvec3());
                let w = p.length() - self.grid.radius(k);
                crate::shape::holds(b, crate::shape::joined(self, c), (a - i as f64) as f32, (bb - j as f64) as f32, w)
            }
            _ => false,
        }
    }
}

impl Station {
    /// True if a local point is inside a block's collision boxes.
    pub fn holds_at(&self, p: Vec3) -> bool {
        let c = Station::cell_at(p);
        let b = self.get(c);
        match block::shape(b) {
            block::Shape::Cube => true,
            block::Shape::Boxes => {
                let f = p - c.as_vec3();
                crate::shape::holds(b, crate::shape::joined(self, c), f.x, f.z, f.y)
            }
            _ => false,
        }
    }

    /// The middle of a cell.
    pub fn cell_center(c: IVec3) -> Vec3 {
        c.as_vec3() + Vec3::splat(0.5)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::SMOOTH_STONE;

    #[test]
    fn falls_and_rests_on_a_station_floor() {
        let s = Station::new("00000003".into(), "", SMOOTH_STONE);
        let mut d = ItemDrop::popped(Stack::new(block::DIRT, 1), Vec3::new(2.5, 3.5, 2.5), Vec3::Y, [0.5; 3]);
        for _ in 0..300 {
            d.step_station(&s, 1.0 / 60.0);
        }
        assert!(d.resting && (d.pos.y - 1.0).abs() < 0.2, "rests at {:?}", d.pos);
    }

    #[test]
    fn looks_go_past_an_open_door_and_stop_at_a_shut_one() {
        use crate::block::{door, Dir};
        let mut s = Station::new("00000004".into(), "", SMOOTH_STONE);
        s.set(IVec3::new(3, 1, 0), block::STONE);
        // A door at x = 2 whose panel, shut, lies on its x- side; open, on its z- side.
        s.set(IVec3::new(2, 1, 0), door(Dir(0), false, false));
        s.set(IVec3::new(2, 2, 0), door(Dir(0), false, true));
        let (eye, look) = (Vec3::new(0.5, 1.5, 0.5), Vec3::X);
        let hit = s.raycast(eye, look, 5.0).unwrap();
        assert_eq!(hit.cell, IVec3::new(2, 1, 0));
        assert!((hit.point.x - 2.0).abs() < 1e-3, "on the board: {:?}", hit.point);
        s.set(IVec3::new(2, 1, 0), door(Dir(0), true, false));
        let hit = s.raycast(eye, look, 5.0).unwrap();
        assert_eq!(hit.cell, IVec3::new(3, 1, 0), "through the open doorway");
    }
}
