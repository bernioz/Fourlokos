//! Eclipses: how much of its star a point sees when a planet or moon passes in front of it.
//!
//! Seen from the point, the star and the body in front are discs in the sky (their angular radii
//! from their sizes and distances); the light that gets through is the share of the star's disc
//! the body leaves uncovered. Stars here are big, so the shadow has a wide soft penumbra, and a
//! moon smaller in the sky than its star gives an annular eclipse. The voxel shader
//! (`client/src/voxel.wesl`) does the same sums per pixel.

use glam::DVec3;

/// Share of a disc of angular radius `a` covered by one of radius `b` whose center is `d` away
/// (all radians, small or not: treated as flat discs).
pub fn covered(a: f64, b: f64, d: f64) -> f64 {
    if a <= 0.0 || d >= a + b {
        return 0.0;
    }
    if d <= (a - b).abs() {
        return (b.min(a) / a).powi(2);
    }
    // Area of the lens where two circles overlap.
    let ca = ((d * d + a * a - b * b) / (2.0 * d * a)).clamp(-1.0, 1.0).acos();
    let cb = ((d * d + b * b - a * a) / (2.0 * d * b)).clamp(-1.0, 1.0).acos();
    let k = ((-d + a + b) * (d + a - b) * (d - a + b) * (d + a + b)).max(0.0).sqrt();
    let lens = a * a * ca + b * b * cb - 0.5 * k;
    (lens / (std::f64::consts::PI * a * a)).clamp(0.0, 1.0)
}

/// Share of its star (center `star`, radius `star_radius`) that `p` sees past a body (center
/// `body`, radius `radius`). 1 when the body is not in the way.
pub fn sun_seen(p: DVec3, star: DVec3, star_radius: f64, body: DVec3, radius: f64) -> f64 {
    let (to_star, to_body) = (star - p, body - p);
    let (ds, db) = (to_star.length(), to_body.length());
    // Behind the point, or behind the star, or the point is inside it: no eclipse.
    if db <= radius || db >= ds || to_body.dot(to_star) <= 0.0 {
        return 1.0;
    }
    let a = (star_radius / ds).min(1.0).asin();
    let b = (radius / db).min(1.0).asin();
    let sep = (to_body / db).dot(to_star / ds).clamp(-1.0, 1.0).acos();
    1.0 - covered(a, b, sep)
}

/// The next eclipse between a planet and a moon after time `from`, looking up to `within`
/// seconds ahead: when it is deepest (seconds), whether it is solar (the moon in front of the
/// star, seen from the planet) or lunar (the moon in the planet's shadow), and how much of the
/// star is hidden then at the best spot. `at(t)` gives (star, planet, moon) centers at time t;
/// radii are the star's, the planet's and the moon's.
pub fn next_eclipse(at: impl Fn(f64) -> (DVec3, DVec3, DVec3), radii: (f64, f64, f64), from: f64, within: f64) -> Option<(f64, Kind, f64)> {
    let (rs, rp, rm) = radii;
    // How deep each kind is at time t (share of the star hidden, 0 = none).
    let depth = |t: f64| {
        let (star, planet, moon) = at(t);
        // Solar: from the point of the planet right under the moon, toward the star.
        let under = planet + (moon - planet).normalize() * rp;
        let solar = 1.0 - sun_seen(under, star, rs, moon, rm);
        // Lunar: from the middle of the moon.
        let lunar = 1.0 - sun_seen(moon, star, rs, planet, rp);
        (solar, lunar)
    };
    let step = 2.0;
    let mut t = from;
    while t < from + within {
        let (s, l) = depth(t);
        if s > 0.0 || l > 0.0 {
            // Follow it to where it is deepest.
            let kind = if s >= l { Kind::Solar } else { Kind::Lunar };
            let of = |t: f64| if kind == Kind::Solar { depth(t).0 } else { depth(t).1 };
            let (mut best, mut deepest) = (t, of(t));
            let mut u = t;
            while of(u) > 0.0 && u < t + within {
                if of(u) > deepest {
                    (best, deepest) = (u, of(u));
                }
                u += 0.5;
            }
            return Some((best, kind, deepest));
        }
        t += step;
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Solar,
    Lunar,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discs() {
        assert_eq!(covered(1.0, 1.0, 3.0), 0.0);
        assert!((covered(1.0, 2.0, 0.5) - 1.0).abs() < 1e-12); // total
        assert!((covered(1.0, 0.5, 0.1) - 0.25).abs() < 1e-12); // annular
        // Half way across, equal discs: the lens is a bit under 40% of either.
        let half = covered(1.0, 1.0, 1.0);
        assert!((half - 0.391).abs() < 1e-3, "{half}");
        // Coverage only grows as the discs close in.
        let mut last = 0.0;
        for i in (0..=300).rev() {
            let c = covered(1.0, 0.8, i as f64 / 100.0);
            assert!(c >= last - 1e-12);
            last = c;
        }
    }

    #[test]
    fn a_moon_darkens_the_ground_behind_it() {
        let star = DVec3::new(15_000.0, 0.0, 0.0);
        let (p, moon) = (DVec3::ZERO, DVec3::new(1000.0, 0.0, 0.0));
        // Straight in line: a 60-block moon 1000 away (3.4° in radius) in front of a 2500-block
        // star 15000 away (9.6°): smaller in the sky, an annular eclipse, about 1/8 covered.
        let seen = sun_seen(p, star, 2500.0, moon, 60.0);
        assert!(seen > 0.8 && seen < 0.95, "{seen}");
        // A big planet close by covers it all.
        assert!(sun_seen(p, star, 2500.0, DVec3::new(300.0, 0.0, 0.0), 150.0) < 1e-9);
        // Off to the side, or behind: nothing.
        assert_eq!(sun_seen(p, star, 2500.0, DVec3::new(0.0, 1000.0, 0.0), 60.0), 1.0);
        assert_eq!(sun_seen(p, star, 2500.0, -moon, 60.0), 1.0);
    }

    #[test]
    fn the_home_moon_eclipses_now_and_then() {
        let u = crate::universe::Universe::new(1337);
        let sys = u.system(glam::IVec3::ZERO).unwrap();
        let planet = sys.planets[0].clone();
        let moon = sys.planets.iter().find(|m| m.host.is_some_and(|h| h.id == planet.id)).unwrap().clone();
        let at = |t: f64| {
            let (mut p, mut m) = (planet.clone(), moon.clone());
            p.advance(t);
            m.advance(t);
            (sys.center, p.center, m.center)
        };
        let radii = (sys.star_radius as f64, planet.radius as f64, moon.radius as f64);
        // Within a few hours of game time there is one, and it hides some of the star.
        let (t, _, deep) = next_eclipse(at, radii, 0.0, 6.0 * 3600.0).expect("an eclipse");
        assert!(t > 0.0 && deep > 0.05, "{t} {deep}");
    }
}
