//! Player health: hit points, damage kinds, invulnerability after a hit, slow regeneration and
//! fall damage. No engine; the client feeds it what happened.

use crate::player::Body;

/// Hit points of a full life (10 hearts, 2 points each).
pub const MAX_HP: f32 = 20.0;
/// Seconds after a hit in which further hits (but falls and commands) do nothing.
pub const INVULNERABLE: f32 = 0.5;
/// Seconds without damage before health starts to come back.
pub const REGEN_DELAY: f32 = 8.0;
/// Hit points per second regenerated.
pub const REGEN_RATE: f32 = 0.5;
/// Blocks you can fall without getting hurt.
pub const SAFE_FALL: f32 = 3.0;
/// A change of height in one step above this is a teleport, not a fall.
const JUMP: f32 = 20.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DamageKind {
    Fall,
    Lava,
    Mob,
    Player,
    Command,
}

impl DamageKind {
    /// Whether the hit gets through the invulnerability after a previous one.
    pub fn ignores_cooldown(self) -> bool {
        matches!(self, DamageKind::Fall | DamageKind::Command)
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Health {
    pub hp: f32,
    pub max: f32,
    /// Seconds of invulnerability left.
    pub cooldown: f32,
    /// Seconds since the last damage.
    pub since_hit: f32,
}

impl Default for Health {
    fn default() -> Self {
        Self {
            hp: MAX_HP,
            max: MAX_HP,
            cooldown: 0.0,
            since_hit: REGEN_DELAY,
        }
    }
}

impl Health {
    /// Takes `amount` hit points; returns what was really lost (0 if the cooldown absorbed it).
    pub fn damage(&mut self, amount: f32, kind: DamageKind) -> f32 {
        if amount <= 0.0 || self.is_dead() || (self.cooldown > 0.0 && !kind.ignores_cooldown()) {
            return 0.0;
        }
        let lost = amount.min(self.hp);
        self.hp -= lost;
        self.cooldown = INVULNERABLE;
        self.since_hit = 0.0;
        lost
    }

    pub fn heal(&mut self, amount: f32) {
        if !self.is_dead() {
            self.hp = (self.hp + amount.max(0.0)).min(self.max);
        }
    }

    /// Advances time; with `regen` (survival) health comes back slowly after a while unhurt.
    pub fn tick(&mut self, dt: f32, regen: bool) {
        self.cooldown = (self.cooldown - dt).max(0.0);
        self.since_hit += dt;
        if regen && self.since_hit >= REGEN_DELAY && !self.is_dead() {
            self.heal(REGEN_RATE * dt);
        }
    }

    pub fn is_dead(&self) -> bool {
        self.hp <= 0.0
    }

    pub fn refill(&mut self) {
        *self = Self::default();
    }
}

/// Damage for a fall of `distance` blocks: one point per block beyond [`SAFE_FALL`].
pub fn fall_damage(distance: f32) -> f32 {
    (distance - SAFE_FALL).max(0.0)
}

/// Measures falls as the drop in height since the body last stood on something.
#[derive(Clone, Copy, Default, Debug)]
pub struct Fall {
    dist: f32,
    last: Option<f32>,
}

impl Fall {
    /// Call once per step after the body moved; returns the distance fallen on the step it lands.
    pub fn track(&mut self, body: &Body) -> Option<f32> {
        let counts = body.gravity_source.is_some()
            && body.station.is_none()
            && body.cabin.is_none()
            && !body.flying
            && !body.jetpack
            && body.swimming.is_none()
            && !body.climbing;
        self.step(body.pos.length(), counts, body.on_ground)
    }

    /// The engine-free core: `height` now, whether falls count, whether standing.
    pub fn step(&mut self, height: f32, counts: bool, on_ground: bool) -> Option<f32> {
        let last = self.last.replace(height);
        if !counts {
            self.dist = 0.0;
            return None;
        }
        let Some(last) = last else { return None };
        let d = last - height;
        if d.abs() > JUMP {
            self.dist = 0.0;
            return None;
        }
        // The drop from the highest point since leaving the ground (rises count against it).
        self.dist = (self.dist + d).max(0.0);
        if on_ground {
            let fell = std::mem::take(&mut self.dist);
            return (fell > 0.0).then_some(fell);
        }
        None
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn damage_and_death() {
        let mut h = Health::default();
        assert_eq!(h.damage(5.0, DamageKind::Mob), 5.0);
        assert_eq!(h.hp, 15.0);
        assert!(!h.is_dead());
        assert_eq!(h.damage(100.0, DamageKind::Command), 15.0);
        assert!(h.is_dead());
        h.heal(5.0);
        assert!(h.is_dead(), "no healing the dead");
    }

    #[test]
    fn cooldown_blocks_hits_but_not_falls() {
        let mut h = Health::default();
        h.damage(2.0, DamageKind::Mob);
        assert_eq!(h.damage(2.0, DamageKind::Mob), 0.0);
        assert_eq!(h.damage(3.0, DamageKind::Fall), 3.0);
        h.tick(INVULNERABLE + 0.01, false);
        assert_eq!(h.damage(2.0, DamageKind::Player), 2.0);
        assert_eq!(h.hp, 13.0);
    }

    #[test]
    fn regeneration_waits_and_needs_survival() {
        let mut h = Health::default();
        h.damage(10.0, DamageKind::Mob);
        h.tick(REGEN_DELAY - 1.0, true);
        assert_eq!(h.hp, 10.0);
        h.tick(2.0, false);
        assert_eq!(h.hp, 10.0, "creative does not regenerate");
        h.tick(2.0, true);
        assert!(h.hp > 10.0 && h.hp < MAX_HP);
        h.tick(1000.0, true);
        assert_eq!(h.hp, MAX_HP);
    }

    #[test]
    fn fall_damage_curve() {
        assert_eq!(fall_damage(2.0), 0.0);
        assert_eq!(fall_damage(3.0), 0.0);
        assert_eq!(fall_damage(10.0), 7.0);
    }

    #[test]
    fn fall_tracker_measures_the_drop() {
        let mut f = Fall::default();
        assert_eq!(f.step(100.0, true, true), None);
        assert_eq!(f.step(99.0, true, false), None);
        assert_eq!(f.step(95.0, true, false), None);
        assert_eq!(f.step(90.0, true, true), Some(10.0));
        assert_eq!(f.step(90.0, true, true), None, "standing is not a fall");
    }

    #[test]
    fn fall_tracker_subtracts_rises_and_ignores_water_and_jumps() {
        let mut f = Fall::default();
        f.step(100.0, true, true);
        f.step(101.0, true, false); // a jump up
        f.step(100.0, true, false);
        assert_eq!(
            f.step(100.0, true, true),
            Some(1.0),
            "a hop drops 1 block: harmless"
        );
        assert_eq!(fall_damage(1.0), 0.0);
        f.step(100.0, true, false);
        f.step(90.0, true, false);
        assert_eq!(
            f.step(88.0, false, false),
            None,
            "water or flying resets it"
        );
        assert_eq!(
            f.step(80.0, true, true),
            Some(8.0),
            "counts again once out of the water"
        );
        f.step(300.0, true, false);
        assert_eq!(f.step(0.0, true, true), None, "a teleport is not a fall");
    }
}
