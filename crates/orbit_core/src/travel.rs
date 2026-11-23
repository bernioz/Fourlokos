//! Orbital mechanics and autopilot guidance for the player's thrusters.
//!
//! Everything works on a state relative to one planet's center (position and velocity,
//! world-oriented), the way `Body::world_dir` gives it. Guidance functions return the
//! acceleration to thrust with (`player::Input::thrust`); gravity is not part of it, they
//! compensate for it themselves.

use crate::player::{gravity_at, Pull, GRAVITY};
use glam::Vec3;

/// Circular orbits the autopilot flies sit this far above the atmosphere.
pub const ORBIT_ALTITUDE: f32 = 70.0;
/// Thrust the autopilot uses near planets, blocks/s².
pub const PILOT_ACCEL: f32 = 60.0;

/// Radius of the parking orbit around a planet with this gravity radius.
pub fn orbit_radius(gravity_radius: f32) -> f32 {
    gravity_radius + ORBIT_ALTITUDE
}

/// Gravitational parameter (G·M) of the inverse-square field past the atmosphere.
pub fn mu(gravity_radius: f32) -> f32 {
    GRAVITY * gravity_radius * gravity_radius
}

/// Speed of a circular orbit at radius `r`.
pub fn circular_speed(r: f32, pull: Pull) -> f32 {
    (gravity_at(r, pull) * r).sqrt()
}

/// The conic the body is on, ignoring the atmosphere and the fade at the sphere's edge.
#[derive(Clone, Copy, Debug)]
pub struct Elements {
    pub periapsis: f32,
    /// Infinite when not bound.
    pub apoapsis: f32,
    pub eccentricity: f32,
    pub period: f32,
}

pub fn elements(p: Vec3, v: Vec3, gravity_radius: f32) -> Elements {
    let mu = mu(gravity_radius);
    let r = p.length().max(1e-3);
    let energy = v.length_squared() * 0.5 - mu / r;
    let h = p.cross(v).length();
    let e = (1.0 + 2.0 * energy * h * h / (mu * mu)).max(0.0).sqrt();
    let periapsis = h * h / (mu * (1.0 + e));
    if energy >= 0.0 || e >= 1.0 {
        return Elements { periapsis, apoapsis: f32::INFINITY, eccentricity: e, period: f32::INFINITY };
    }
    let a = -mu / (2.0 * energy);
    Elements { periapsis, apoapsis: a * (1.0 + e), eccentricity: e, period: std::f32::consts::TAU * (a * a * a / mu).sqrt() }
}

/// True if the body circles the planet for good: above the atmosphere all the way round and
/// inside its sphere of influence.
pub fn in_orbit(p: Vec3, v: Vec3, pull: Pull) -> bool {
    let e = elements(p, v, pull.gravity_radius);
    e.periapsis > pull.gravity_radius + 2.0 && e.apoapsis < pull.soi - pull.taper()
}

/// Horizontal (perpendicular to `radial`) direction of travel, or `fallback` laid flat.
fn horizontal(radial: Vec3, v: Vec3, fallback: Vec3) -> Vec3 {
    let t = v.reject_from(radial);
    if t.length() > 1.0 {
        return t.normalize();
    }
    fallback.reject_from(radial).normalize_or(radial.any_orthonormal_vector())
}

fn clamp_len(a: Vec3, max: f32) -> Vec3 {
    a.clamp_length_max(max)
}

/// Thrust to reach a circular orbit of radius `target` from wherever the body is: straight up
/// out of the atmosphere, turning over to horizontal on the way (a gravity turn), then holding
/// the orbit's speed. Works the same for launching and for capture on arrival. `fallback` is the
/// way to head when the body has no horizontal motion yet.
pub fn circularize(p: Vec3, v: Vec3, target: f32, pull: Pull, fallback: Vec3, max_accel: f32) -> Vec3 {
    let gravity_radius = pull.gravity_radius;
    let r = p.length().max(1.0);
    let radial = p / r;
    let tangent = horizontal(radial, v, fallback);
    // The turn begins a while below the atmosphere's edge and ends at the orbit.
    let start = gravity_radius - 60.0;
    let turn = ((r - start) / (target - start)).clamp(0.0, 1.0);
    let vt_want = circular_speed(r.max(target), pull) * turn.sqrt();
    let vr_want = ((target - r) * 0.6).clamp(-25.0, 45.0);
    let want = radial * vr_want + tangent * vt_want;
    let vt = v.reject_from(radial).length();
    // Hold against gravity, less what the horizontal speed already does (centripetal).
    let hold = radial * (gravity_at(r, pull) - vt * vt / r);
    clamp_len((want - v) * 1.2 + hold, max_accel)
}

/// True once `circularize` has done its job.
pub fn circularized(p: Vec3, v: Vec3, target: f32, pull: Pull) -> bool {
    (p.length() - target).abs() < 15.0 && in_orbit(p, v, pull) && elements(p, v, pull.gravity_radius).eccentricity < 0.06
}

/// Thrust to come down gently over the point below: horizontal speed killed, descent slowing to
/// a few blocks a second near `ground` (radius of the surface there).
pub fn descend(p: Vec3, v: Vec3, ground: f32, pull: Pull, max_accel: f32) -> Vec3 {
    let r = p.length().max(1.0);
    let radial = p / r;
    let alt = (r - ground).max(0.0);
    let want = -radial * (alt * 0.3).clamp(4.0, 60.0);
    clamp_len((want - v) * 1.0 + radial * gravity_at(r, pull), max_accel)
}

/// Thrust to fly to a point `to` (relative to the body) and get there at `arrive` speed: full
/// acceleration up to `max_speed`, then a braking curve that ends on the point.
pub fn cruise(to: Vec3, v: Vec3, arrive: f32, max_speed: f32, max_accel: f32) -> Vec3 {
    let d = to.length();
    let dir = to.normalize_or_zero();
    let speed = (arrive * arrive + 2.0 * max_accel * 0.5 * d).sqrt().min(max_speed);
    clamp_len((dir * speed - v) * 3.0, max_accel)
}

/// Seconds `cruise` takes over `distance`, roughly.
pub fn cruise_time(distance: f32, max_speed: f32, max_accel: f32) -> f32 {
    let a = max_accel * 0.5;
    let ramp = max_speed * max_speed / a; // distance spent speeding up and slowing down
    if distance < ramp {
        2.0 * (distance / a).sqrt()
    } else {
        2.0 * max_speed / a + (distance - ramp) / max_speed
    }
}

/// Free-fall path from a state, for drawing: points every `dt` seconds until it hits the
/// atmosphere, leaves the sphere of influence, or closes the orbit.
pub fn predict(mut p: Vec3, mut v: Vec3, pull: Pull, dt: f32, max_points: usize) -> Vec<Vec3> {
    let mut out = vec![p];
    let start = p;
    let (s, gravity_radius) = (pull.soi, pull.gravity_radius);
    for i in 0..max_points {
        // A few substeps per point, the same integrator as the body.
        for _ in 0..4 {
            let r = p.length().max(1.0);
            v -= p / r * gravity_at(r, pull) * (dt / 4.0);
            p += v * (dt / 4.0);
        }
        out.push(p);
        let r = p.length();
        if r < gravity_radius || r > s || (i > 8 && p.distance(start) < v.length() * dt) {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::planet::{Biome, Planet, PlanetSpec};
    use crate::player::{Body, Input, Located, Well};

    const DT: f32 = 1.0 / 60.0;

    fn planet() -> Planet {
        let spec = PlanetSpec { center: glam::DVec3::new(500.0, 0.0, 0.0), ..PlanetSpec::new(7, "t".into(), 80.0, Biome::Barren) };
        Planet::generate(spec)
    }

    fn rel(b: &Body, p: &Planet) -> (Vec3, Vec3) {
        (b.point_from(b.pos, p.spec.center), b.world_dir(b.vel))
    }

    #[test]
    fn circular_orbit_stays_circular() {
        let gr = 200.0;
        let r = orbit_radius(gr);
        let well = Well { id: 1, center: glam::DVec3::ZERO, radius: 100.0, pull: Pull::planet(gr), vel: Vec3::ZERO };
        let mut b = Body::new(glam::DVec3::new(r as f64, 0.0, 0.0), Vec3::X);
        b.vel = Vec3::new(0.0, 0.0, circular_speed(r, Pull::planet(gr)));
        let period = elements(Vec3::new(r, 0.0, 0.0), b.vel, gr).period;
        for _ in 0..(period * 3.0 / DT) as usize {
            b.step_with(Input::default(), &[], &[well], &[], DT);
            assert_eq!(b.orbiting, Some(1));
            assert!((b.pos.length() - r).abs() < 3.0, "r = {}", b.pos.length());
        }
        assert!(in_orbit(b.pos, b.vel, Pull::planet(gr)));
    }

    #[test]
    fn launches_from_the_ground_into_orbit() {
        let p = planet();
        let gr = p.spec.gravity_radius();
        let mut b = Body::new(p.spec.to_world(Vec3::Y * p.spec.ceiling()), Vec3::Y);
        let locs = [Located::of(&p)];
        for _ in 0..300 {
            b.step(Input::default(), &locs, DT);
        }
        assert!(b.on_ground);
        let target = orbit_radius(gr);
        let mut done = false;
        for _ in 0..60 * 120 {
            let (pos, vel) = rel(&b, &p);
            if circularized(pos, vel, target, Pull::planet(gr)) {
                done = true;
                break;
            }
            let thrust = circularize(pos, vel, target, Pull::planet(gr), Vec3::X, PILOT_ACCEL);
            b.step(Input { thrust: Some(thrust), ..Default::default() }, &locs, DT);
        }
        assert!(done, "never reached orbit");
        assert_eq!(b.orbiting, Some(p.spec.id));
        // And it stays up without thrust.
        for _ in 0..60 * 30 {
            b.step(Input::default(), &locs, DT);
        }
        let (pos, vel) = rel(&b, &p);
        assert!(in_orbit(pos, vel, Pull::planet(gr)), "decayed: r = {}", pos.length());
    }

    #[test]
    fn descends_from_orbit_and_lands_softly() {
        let p = planet();
        let gr = p.spec.gravity_radius();
        let r = orbit_radius(gr);
        let mut b = Body::new(p.spec.to_world(Vec3::Y * r), Vec3::Y);
        b.vel = Vec3::X * circular_speed(r, Pull::planet(gr));
        let locs = [Located::of(&p)];
        let mut landed = None;
        for i in 0..60 * 120 {
            let (pos, vel) = rel(&b, &p);
            let thrust = descend(pos, vel, p.spec.radius + p.spec.max_height(), Pull::planet(gr), PILOT_ACCEL);
            let before = b.vel.length();
            b.step(Input { thrust: Some(thrust), ..Default::default() }, &locs, DT);
            if b.on_ground {
                landed = Some((i, before));
                break;
            }
        }
        let (_, speed) = landed.expect("never landed");
        assert!(speed < 10.0, "hit the ground at {speed}");
    }

    #[test]
    fn cruise_arrives_slowly() {
        let mut p = Vec3::ZERO;
        let mut v = Vec3::ZERO;
        let to = Vec3::new(20_000.0, 3000.0, -5000.0);
        let mut t = 0.0;
        while p.distance(to) > 20.0 && t < 600.0 {
            let a = cruise(to - p, v, 50.0, 3000.0, 120.0);
            v += a * DT;
            p += v * DT;
            t += DT;
        }
        assert!(p.distance(to) <= 20.0, "stopped at {}", p.distance(to));
        assert!(v.length() < 120.0, "arrived at {}", v.length());
        let est = cruise_time(to.length(), 3000.0, 120.0);
        assert!((t - est).abs() < est * 0.35, "took {t}, estimated {est}");
    }

    #[test]
    fn falling_in_from_orbit_has_no_jolt() {
        // Gravity is continuous at the atmosphere's edge.
        let gr = 200.0;
        assert!((gravity_at(gr - 0.01, Pull::planet(gr)) - gravity_at(gr + 0.01, Pull::planet(gr))).abs() < 0.01);
        assert_eq!(gravity_at(Pull::planet(gr).soi + 1.0, Pull::planet(gr)), 0.0);
    }
}
