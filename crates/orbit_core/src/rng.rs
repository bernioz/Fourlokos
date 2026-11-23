//! Tiny deterministic RNG (splitmix64): same seed, same universe, on every machine.

pub fn mix(mut z: u64) -> u64 {
    z = z.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(mix(seed))
    }
    pub fn next_u64(&mut self) -> u64 {
        self.0 = mix(self.0);
        self.0
    }
    /// Uniform in [0, 1).
    pub fn f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f32()
    }
    pub fn int(&mut self, lo: i64, hi_incl: i64) -> i64 {
        lo + (self.next_u64() % (hi_incl - lo + 1) as u64) as i64
    }
    /// Uniform unit vector.
    pub fn dir(&mut self) -> glam::Vec3 {
        let z = self.range(-1.0, 1.0);
        let a = self.range(0.0, std::f32::consts::TAU);
        let r = (1.0 - z * z).sqrt();
        glam::Vec3::new(r * a.cos(), z, r * a.sin())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_seed_is_deterministic() {
        let mut a = Rng::new(12345);
        let mut b = Rng::new(12345);
        for _ in 0..100 {
            assert_eq!(a.next_u64(), b.next_u64());
        }
    }

    #[test]
    fn different_seeds_diverge() {
        let mut a = Rng::new(1);
        let mut b = Rng::new(2);

        let a_seq: Vec<u64> = (0..16).map(|_| a.next_u64()).collect();
        let b_seq: Vec<u64> = (0..16).map(|_| b.next_u64()).collect();

        assert_ne!(a_seq, b_seq);
    }

    #[test]
    fn f32_stays_in_unit_interval() {
        let mut rng = Rng::new(7);
        for _ in 0..10_000 {
            let value = rng.f32();
            assert!(value >= 0.0 && value < 1.0, "f32 out of bounds: {value}");
        }
    }

    #[test]
    fn range_stays_within_bounds() {
        let mut rng = Rng::new(99);
        let lo = 2.5f32;
        let hi = 10.0f32;
        for _ in 0..10_000 {
            let value = rng.range(lo, hi);
            assert!(value >= lo && value < hi, "range out of bounds: {value}");
        }
    }

    #[test]
    fn int_stays_in_inclusive_bounds() {
        let mut rng = Rng::new(2024);
        for _ in 0..10_000 {
            let value = rng.int(-5, 5);
            assert!((-5..=5).contains(&value), "int out of bounds: {value}");
        }
    }

    #[test]
    fn dir_is_unit_vector() {
        let mut rng = Rng::new(42);
        for _ in 0..10_000 {
            let v = rng.dir();
            let len = v.length();
            assert!((len - 1.0).abs() < 1e-3, "dir length out of bounds: {len}");
        }
    }
}
