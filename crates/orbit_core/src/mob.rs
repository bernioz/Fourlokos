//! Creatures and townsfolk, no engine: a `Mob` is a point-footed body in its frame (planet-local
//! coordinates, down toward the planet's middle) with hit points and a little brain. The client
//! feeds `step` what the world looks like (`Env`) and reads back `Event`s; everything random comes
//! from the mob's own `Rng`, so a mob's life is the same on every machine (and testable).
//!
//! - Slime: passive (hops about at random) until hit; then hostile for good: hops after the player
//!   and hurts on contact. Drops slimeballs.
//! - Bodoque and Tulio: townsfolk. Invulnerable, harmless; they turn to face a player who is near
//!   and otherwise stroll slowly inside a small radius of their home.
//! - The Slime King (`boss`): a big slime with attacks and two phases.

use crate::block::Block;
use crate::inventory::Stack;
use crate::items;
use crate::rng::{Rng, mix};
use glam::Vec3;

pub const SLIME_HP: f32 = 8.0;
pub const CHICKEN_HP: f32 = 4.0;
pub const EGG_INTERVAL_SECS: f32 = 120.0;
/// Backwards-compatible short name for the laying interval.
pub const EGG_INTERVAL: f32 = EGG_INTERVAL_SECS;
/// Seconds a mob ignores further hits after one.
pub const HIT_COOLDOWN: f32 = 0.5;
/// Seconds between contact hits of one mob.
pub const CONTACT_COOLDOWN: f32 = 1.0;
/// Punches and pickaxes do this much; swords what `items::damage` says.
pub const FIST_DAMAGE: f32 = 1.0;
const GRAVITY: f32 = 28.0;
/// How far a townsperson notices a player and turns to face them.
pub const NOTICE: f32 = 7.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Slime,
    SlimeKing,
    Bodoque,
    Tulio,
    Chicken,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::Slime,
        Kind::SlimeKing,
        Kind::Bodoque,
        Kind::Tulio,
        Kind::Chicken,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Slime => "Slime",
            Kind::SlimeKing => "Slime King",
            Kind::Bodoque => "Bodoque",
            Kind::Tulio => "Tulio",
            Kind::Chicken => "Gallina",
        }
    }

    /// What `/summon` calls it.
    pub fn id(self) -> &'static str {
        match self {
            Kind::Slime => "slime",
            Kind::SlimeKing => "slime_king",
            Kind::Bodoque => "bodoque",
            Kind::Tulio => "tulio",
            Kind::Chicken => "gallina",
        }
    }

    pub fn from_id(s: &str) -> Option<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.id() == s || (s == "chicken" && *k == Kind::Chicken))
    }

    pub fn max_hp(self) -> f32 {
        match self {
            Kind::Slime => SLIME_HP,
            Kind::SlimeKing => crate::boss::KING_HP,
            Kind::Chicken => CHICKEN_HP,
            _ => 1.0,
        }
    }

    pub fn is_npc(self) -> bool {
        matches!(self, Kind::Bodoque | Kind::Tulio)
    }

    pub fn invulnerable(self) -> bool {
        self.is_npc()
    }

    /// Hostile from the start (never needs a hit to wake).
    pub fn always_hostile(self) -> bool {
        self == Kind::SlimeKing
    }

    /// Half width and height of its box, at scale 1.
    pub fn size(self) -> (f32, f32) {
        match self {
            Kind::Slime => (0.375, 0.9),
            Kind::SlimeKing => (0.375, 0.9), // times its scale
            Kind::Bodoque | Kind::Tulio => (0.3, 1.8),
            Kind::Chicken => (0.4, 0.9),
        }
    }

    /// Hit points of damage a touch does.
    pub fn contact_damage(self) -> f32 {
        match self {
            Kind::Slime => 2.0,
            Kind::SlimeKing => crate::boss::KING_DAMAGE,
            _ => 0.0,
        }
    }
}

/// What a mob needs to know about the world this step.
pub struct Env<'a> {
    /// Unit vector the way things fall.
    pub down: Vec3,
    /// Whether a point (in the mob's frame) is inside something solid.
    pub solid: &'a dyn Fn(Vec3) -> bool,
    /// Whether a point is in water or lava (mobs float, and strolling ones keep out of it).
    pub wet: &'a dyn Fn(Vec3) -> bool,
    /// The player's feet, if one is around.
    pub player: Option<Vec3>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Event {
    /// Touched the player: this much damage.
    Contact(f32),
    /// The King crouched, glowed: this attack is coming.
    Telegraph(crate::boss::Attack),
    /// The King hit the ground: damage within this radius of `at`.
    Smash {
        at: Vec3,
        radius: f32,
        damage: f32,
        hits: bool,
    },
    /// The King asks for this many small slimes round it.
    Summon(u8),
    /// A chicken left an egg at its feet.
    LayEgg,
    /// The King crossed into its second phase.
    Enraged,
}

/// What a hit did.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Hit {
    /// Nothing: in its hit cooldown, or it cannot be hurt.
    Ignored,
    Hurt,
    Killed,
}

#[derive(Clone, Debug)]
pub struct Mob {
    pub id: u64,
    pub kind: Kind,
    /// Its feet, in its frame.
    pub pos: Vec3,
    pub vel: Vec3,
    /// Unit, along the ground: where it looks.
    pub face: Vec3,
    pub hp: f32,
    /// Size over the model's own (small slimes the King calls are under 1, the King 3).
    pub scale: f32,
    pub aggro: bool,
    pub on_ground: bool,
    /// Where townsfolk live and how far from it they stroll.
    pub home: Vec3,
    pub leash: f32,
    pub hit_cd: f32,
    pub attack_cd: f32,
    /// Seconds to the next hop (slime) or the next decision (townsperson).
    pub timer: f32,
    /// Seconds until the next egg. Only used by chickens.
    pub egg_timer: f32,
    /// A townsperson's destination, if it is walking.
    pub goal: Option<Vec3>,
    /// The way the current hop goes.
    pub heading: Vec3,
    pub boss: crate::boss::State,
    pub(crate) rng: Rng,
}

impl Mob {
    pub fn new(id: u64, kind: Kind, pos: Vec3, up: Vec3) -> Self {
        let mut rng = Rng::new(mix(id ^ 0x30B5_1111));
        let t = up.any_orthonormal_vector();
        let face = (t * rng.range(-1.0, 1.0) + up.cross(t) * rng.range(-1.0, 1.0)).normalize_or(t);
        let scale = if kind == Kind::SlimeKing {
            crate::boss::SCALE
        } else {
            1.0
        };
        Self {
            id,
            kind,
            pos,
            vel: Vec3::ZERO,
            face,
            hp: kind.max_hp(),
            scale,
            aggro: kind.always_hostile(),
            on_ground: false,
            home: pos,
            leash: 4.0,
            hit_cd: 0.0,
            attack_cd: 0.0,
            timer: rng.range(0.3, 1.5),
            egg_timer: EGG_INTERVAL_SECS + rng.range(-12.0, 12.0),
            goal: None,
            heading: face,
            boss: Default::default(),
            rng,
        }
    }

    pub fn up(&self) -> Vec3 {
        self.pos.normalize_or(Vec3::Y)
    }

    /// Half width and height of its box, scaled.
    pub fn size(&self) -> (f32, f32) {
        let (w, h) = self.kind.size();
        (w * self.scale, h * self.scale)
    }

    pub fn dead(&self) -> bool {
        self.hp <= 0.0
    }

    pub fn contact_damage(&self) -> f32 {
        self.kind.contact_damage()
    }

    /// A hit of `damage` from `from` (the attacker's position, for the knockback); `sword`: with a sword.
    pub fn hit(&mut self, damage: f32, from: Vec3, down: Vec3) -> Hit {
        if self.kind.invulnerable() || self.hit_cd > 0.0 || self.dead() {
            return Hit::Ignored;
        }
        self.hp -= damage;
        self.hit_cd = HIT_COOLDOWN;
        self.aggro = true;
        // Knocked back, away from the attacker and a little up (the King is too heavy for much).
        let up = -down;
        let away = (self.pos - from) - up * (self.pos - from).dot(up);
        let kb = away.normalize_or(self.face) * 5.0 + up * 4.0;
        self.vel += kb / self.scale.max(1.0);
        self.on_ground = false;
        if self.dead() { Hit::Killed } else { Hit::Hurt }
    }

    /// Where a ray from `eye` along `dir` (unit) first meets its box (a box standing on its feet, turned to
    /// its face), within `reach`: the distance along the ray.
    pub fn ray(&self, eye: Vec3, dir: Vec3, reach: f32) -> Option<f32> {
        let up = self.up();
        let (w, h) = self.size();
        let back =
            (-(self.face - up * self.face.dot(up))).normalize_or(up.any_orthonormal_vector());
        let right = up.cross(back);
        let to_local = |v: Vec3| Vec3::new(v.dot(right), v.dot(up), v.dot(back));
        let (o, d) = (to_local(eye - self.pos), to_local(dir));
        let (lo, hi) = (Vec3::new(-w, 0.0, -w), Vec3::new(w, h, w));
        let (mut t0, mut t1) = (0.0f32, reach);
        for i in 0..3 {
            if d[i].abs() < 1e-6 {
                if o[i] < lo[i] || o[i] > hi[i] {
                    return None;
                }
            } else {
                let (a, b) = ((lo[i] - o[i]) / d[i], (hi[i] - o[i]) / d[i]);
                t0 = t0.max(a.min(b));
                t1 = t1.min(a.max(b));
                if t0 > t1 {
                    return None;
                }
            }
        }
        Some(t0)
    }

    /// What it leaves when killed (by a sword or not): fixed by its id, so the same everywhere.
    pub fn drops(&self, sword: bool) -> Option<Stack> {
        let mut r = Rng::new(mix(self.id ^ 0x5117_E8A1));
        let n = match (self.kind, sword) {
            (Kind::Slime, false) => r.int(0, 2),
            (Kind::Slime, true) => r.int(1, 3),
            (Kind::SlimeKing, _) => crate::boss::drop_count(&mut r),
            (Kind::Chicken, _) => r.int(1, 2),
            _ => 0,
        };
        // Small slimes the King calls drop less.
        let n = if self.scale < 1.0 { n.min(1) } else { n };
        let item = if self.kind == Kind::Chicken {
            items::RAW_CHICKEN
        } else {
            items::SLIMEBALL
        };
        (n > 0).then(|| Stack::new(item, n as u8))
    }

    /// All chicken drops, including its optional feather.
    pub fn drops_all(&self, sword: bool) -> Vec<Stack> {
        let mut out = self.drops(sword).into_iter().collect::<Vec<_>>();
        if self.kind == Kind::Chicken {
            let mut r = Rng::new(mix(self.id ^ 0xFEA7_1234));
            let n = r.int(0, 2);
            if n > 0 {
                out.push(Stack::new(items::FEATHER, n as u8));
            }
        }
        out
    }

    /// One step of `dt` seconds.
    pub fn step(&mut self, env: &Env, dt: f32) -> Vec<Event> {
        let mut ev = Vec::new();
        self.hit_cd = (self.hit_cd - dt).max(0.0);
        self.attack_cd = (self.attack_cd - dt).max(0.0);
        let up = -env.down;
        // Where the player is, along the ground and straight up from here.
        let to_player = env.player.map(|p| p - self.pos);
        let flat = |v: Vec3| v - up * v.dot(up);
        match self.kind {
            Kind::Slime => self.slime_ai(env, dt, to_player, &mut ev),
            Kind::SlimeKing => crate::boss::ai(self, env, dt, to_player, &mut ev),
            Kind::Bodoque | Kind::Tulio | Kind::Chicken => self.townsperson_ai(env, dt, to_player),
        }
        if self.kind == Kind::Chicken {
            self.egg_timer -= dt;
            if self.egg_timer <= 0.0 && !self.dead() {
                ev.push(Event::LayEgg);
                self.egg_timer = EGG_INTERVAL_SECS + self.rng.range(-12.0, 12.0);
            }
        }
        // Touching the player hurts, if awake and not on cooldown.
        if let Some(d) = to_player.filter(|_| {
            self.aggro && !self.dead() && self.attack_cd <= 0.0 && self.touches_damage()
        }) {
            let (w, h) = self.size();
            let along = d.dot(up);
            // The player's body (feet at `along`, as tall as `HEIGHT`) overlaps its box, standing in line.
            if flat(d).length() < w + crate::player::HALF_WIDTH + 0.15
                && along > -crate::player::HEIGHT
                && along < h
            {
                self.attack_cd = CONTACT_COOLDOWN;
                ev.push(Event::Contact(self.kind.contact_damage()));
            }
        }
        self.physics(env, dt);
        ev
    }

    /// Whether touching hurts right now (the King's crouch does not).
    fn touches_damage(&self) -> bool {
        self.kind.contact_damage() > 0.0 && self.boss.warning().is_none()
    }

    pub(crate) fn slime_ai(
        &mut self,
        env: &Env,
        dt: f32,
        to_player: Option<Vec3>,
        _ev: &mut Vec<Event>,
    ) {
        let up = -env.down;
        // Hops from the ground, or from the water it floats in.
        let afloat = (env.wet)(self.pos + up * (self.size().1 * 0.3));
        if !self.on_ground && !afloat {
            return;
        }
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        let toward = to_player
            .map(|d| d - up * d.dot(up))
            .filter(|d| self.aggro && d.length_squared() > 1e-4);
        // The King hops faster in its second phase.
        let k = if self.kind == Kind::SlimeKing {
            if self.boss.phase == 2 { 1.5 } else { 1.15 }
        } else {
            1.0
        };
        let (dir, fwd, rest) = match toward {
            // Hunting: straight at the player, quick hops.
            Some(d) => (d.normalize(), 3.6 * k, self.rng.range(0.45, 0.9) / k),
            // Strolling: wanders, mostly the way it was going.
            None => {
                let t = up.cross(self.heading).normalize_or(self.heading);
                let mut dir = self.heading;
                // A few tries for a heading whose landing is dry; else it stays put this time.
                for _ in 0..4 {
                    let turn = self.rng.range(-1.0, 1.0);
                    dir = (self.heading * (1.0 - turn.abs()) + t * turn).normalize_or(self.heading);
                    if afloat || !(env.wet)(self.pos + dir * 1.3 + up * 0.1) {
                        break;
                    }
                    dir = -dir;
                    if !(env.wet)(self.pos + dir * 1.3 + up * 0.1) {
                        break;
                    }
                }
                let rest = self.rng.range(1.2, 3.0);
                if !afloat && (env.wet)(self.pos + dir * 1.3 + up * 0.1) {
                    self.timer = rest;
                    return;
                }
                (dir, 2.0, rest)
            }
        };
        self.heading = dir;
        self.face = dir;
        self.vel += up * 8.5 + dir * fwd;
        self.on_ground = false;
        self.timer = rest;
    }

    fn townsperson_ai(&mut self, env: &Env, dt: f32, to_player: Option<Vec3>) {
        let up = -env.down;
        let flat = |v: Vec3| v - up * v.dot(up);
        self.aggro = false;
        // A player near: stands still, looking at them.
        if let Some(d) = to_player
            .map(flat)
            .filter(|d| d.length() < NOTICE && d.length_squared() > 1e-4)
        {
            self.face = d.normalize();
            self.goal = None;
            self.timer = self.timer.max(1.5);
            return;
        }
        self.timer -= dt;
        if self.goal.is_none() && self.timer <= 0.0 {
            // A spot inside the leash round home, around a third of the time; else a rest.
            if self.rng.f32() < 0.6 {
                let t = up.any_orthonormal_vector();
                let u = t.cross(up);
                let a = self.rng.range(0.0, std::f32::consts::TAU);
                let r = self.rng.range(0.4, 1.0) * self.leash;
                self.goal = Some(self.home + (t * a.cos() + u * a.sin()) * r);
            }
            self.timer = self.rng.range(2.0, 5.0);
        }
        if let Some(g) = self.goal {
            let d = flat(g - self.pos);
            if d.length() < 0.3 {
                self.goal = None;
            } else {
                self.face = d.normalize();
            }
        }
    }

    /// Falls, walks (townsfolk), hops (slimes) and collides, one step.
    fn physics(&mut self, env: &Env, dt: f32) {
        let down = env.down;
        let up = -down;
        let (w, h) = self.size();
        // Shoved into something solid (a block went where it stands): works its way out, upward.
        if (env.solid)(self.pos + up * 0.05) {
            self.pos += up * 3.0 * dt;
            self.vel = Vec3::ZERO;
            return;
        }
        self.vel += down * GRAVITY * dt;
        // In water it floats up, slowed by it.
        if (env.wet)(self.pos + up * (h * 0.3)) {
            self.vel -= down * GRAVITY * 1.2 * dt;
            self.vel *= (-3.0 * dt).exp();
        }
        // Townsfolk on the ground walk toward their goal: a steady slow stride.
        if (self.kind.is_npc() || self.kind == Kind::Chicken) && self.on_ground {
            let walk = self
                .goal
                .map(|g| g - self.pos)
                .map(|d| d - up * d.dot(up))
                .filter(|d| d.length_squared() > 1e-4)
                .map_or(Vec3::ZERO, |d| d.normalize() * NPC_SPEED);
            let vr = self.vel.dot(down);
            self.vel = walk + down * vr;
        } else if self.on_ground && !self.boss.charging {
            // Landed hops do not slide.
            let vr = self.vel.dot(down);
            let vt = self.vel - down * vr;
            self.vel = vt * (-9.0 * dt).exp() + down * vr;
        }
        let vr = self.vel.dot(down);
        let vt = self.vel - down * vr;
        // Down.
        let next = self.pos + down * vr * dt;
        let was_ground = self.on_ground;
        self.on_ground = false;
        if (env.solid)(next) {
            if vr > 0.0 {
                self.on_ground = true;
            }
            self.vel = vt;
        } else {
            self.pos = next;
        }
        // Sideways: the leading edge at knee and chest must be clear; a townsperson steps up one block.
        let step = vt * dt;
        if step.length_squared() > 0.0 {
            let lead = step.normalize() * w;
            let clear = |dy: f32| !(env.solid)(self.pos + step + lead + up * dy);
            if clear(0.15) && clear(h * 0.5) && clear(h * 0.95) {
                self.pos += step;
            } else if (self.kind.is_npc() || self.kind == Kind::Chicken)
                && was_ground
                && self.on_ground
                && clear(1.15)
                && clear(h * 0.95 + 1.0)
            {
                self.vel += up * 9.0; // a hop up the step
                self.on_ground = false;
            } else if self.kind.is_npc() || self.kind == Kind::Chicken {
                self.vel -= vt;
                self.goal = None;
            }
            // (A hopping slime keeps its speed: it clears a ledge once it has risen above it.)
        }
    }
}

/// Townspeople's strolling speed, blocks per second ("very slow").
pub const NPC_SPEED: f32 = 0.7;

/// Damage the player does with `held` in hand: a sword's, else a fist's.
pub fn player_damage(held: Option<Block>) -> f32 {
    match held.map(items::damage) {
        Some(d) if d > 0 => d as f32,
        _ => FIST_DAMAGE,
    }
}

/// Whether the held item is a sword (it makes slimes drop more).
pub fn is_sword(held: Option<Block>) -> bool {
    held.is_some_and(|h| items::damage(h) > 0)
}

/// When wild slimes appear: a timer and a cap, all from one `Rng`.
#[derive(Clone, Debug)]
pub struct Spawner {
    rng: Rng,
    timer: f32,
    next_id: u64,
}

/// Where a new slime goes: an angle round the player and a distance, both along the ground.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SpawnAt {
    pub angle: f32,
    pub dist: f32,
    pub id: u64,
}

/// Most wild slimes at once, and how far from the player they may appear (and live).
pub const MAX_WILD: usize = 6;
pub const SPAWN_NEAR: f32 = 14.0;
pub const SPAWN_FAR: f32 = 32.0;
pub const DESPAWN: f32 = 64.0;

impl Spawner {
    pub fn new(seed: u64) -> Self {
        let mut rng = Rng::new(mix(seed ^ 0x5B_A77E));
        let timer = rng.range(2.0, 6.0);
        Self {
            rng,
            timer,
            next_id: mix(seed) & 0x0000_FFFF_FFFF_FFFF,
        }
    }

    /// Time passes with `alive` wild slimes about; maybe one should appear.
    pub fn tick(&mut self, dt: f32, alive: usize) -> Option<SpawnAt> {
        self.timer -= dt;
        if self.timer > 0.0 {
            return None;
        }
        self.timer = self.rng.range(6.0, 16.0);
        if alive >= MAX_WILD {
            return None;
        }
        self.next_id += 1;
        Some(SpawnAt {
            angle: self.rng.range(0.0, std::f32::consts::TAU),
            dist: self.rng.range(SPAWN_NEAR, SPAWN_FAR),
            id: mix(self.next_id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn flat_world(p: Vec3) -> bool {
        p.y < 0.0
    }

    fn env<'a>(player: Option<Vec3>, solid: &'a dyn Fn(Vec3) -> bool) -> Env<'a> {
        Env {
            down: Vec3::NEG_Y,
            solid,
            wet: &|_| false,
            player,
        }
    }

    /// A planet-ish frame: a flat floor at y = 0 and "up" is +y, so `up()` (from the position) is not
    /// used by the tests; mobs standing on the floor at the origin are given a position high on +y.
    fn slime(id: u64, at: Vec3) -> Mob {
        // pos.normalize() is +y when x = z = 0, which is the floor's up.
        Mob::new(id, Kind::Slime, at, Vec3::Y)
    }

    fn run(m: &mut Mob, player: Option<Vec3>, secs: f32) -> Vec<Event> {
        let mut all = Vec::new();
        for _ in 0..(secs / DT) as usize {
            let e = env(player, &flat_world);
            all.extend(m.step(&e, DT));
        }
        all
    }

    #[test]
    fn a_passive_slime_wanders_and_never_attacks() {
        let mut m = slime(1, Vec3::new(0.0, 0.5, 0.0));
        // The player stands right next to it the whole time.
        let player = Some(Vec3::new(0.3, 0.0, 0.3));
        let start = m.pos;
        let mut hopped = false;
        let mut contacts = 0;
        for _ in 0..(30.0 / DT) as usize {
            let e = env(player, &flat_world);
            contacts += m
                .step(&e, DT)
                .iter()
                .filter(|e| matches!(e, Event::Contact(_)))
                .count();
            hopped |= !m.on_ground;
            // Keep it from walking off for the test: put it back near the player.
            if Vec3::new(m.pos.x, 0.0, m.pos.z).length() > 3.0 {
                m.pos = Vec3::new(0.0, m.pos.y, 0.0);
            }
        }
        assert!(hopped && m.pos != start, "it moves about");
        assert_eq!(contacts, 0);
        assert!(!m.aggro);
        // It rests on the floor, not in it.
        assert!(m.pos.y >= -0.01 && m.pos.y < 3.0);
    }

    #[test]
    fn chicken_lays_an_egg_after_about_two_minutes_and_has_food_drops() {
        let mut chicken = Mob::new(99, Kind::Chicken, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        let mut laid = false;
        for _ in 0..(EGG_INTERVAL as usize + 20) {
            laid |= run(&mut chicken, None, 1.0)
                .iter()
                .any(|e| matches!(e, Event::LayEgg));
            if laid {
                break;
            }
        }
        assert!(laid);
        let drops = chicken.drops_all(false);
        assert!(
            drops
                .iter()
                .any(|s| s.item == items::RAW_CHICKEN && (1..=2).contains(&s.count))
        );
        assert!(
            drops
                .iter()
                .filter(|s| s.item == items::FEATHER)
                .all(|s| (1..=2).contains(&s.count))
        );
    }

    #[test]
    fn chicken_is_passive_even_when_a_player_is_nearby() {
        let mut chicken = Mob::new(100, Kind::Chicken, Vec3::new(0.0, 0.5, 0.0), Vec3::Y);
        let events = run(&mut chicken, Some(Vec3::new(0.3, 0.0, 0.3)), 20.0);
        assert!(events.iter().all(|e| !matches!(e, Event::Contact(_))));
        assert_eq!(chicken.contact_damage(), 0.0);
        assert!(!chicken.aggro);
    }

    #[test]
    fn a_hit_slime_hunts_and_hurts_with_a_cooldown() {
        let mut m = slime(2, Vec3::new(0.0, 0.0, 0.0));
        run(&mut m, None, 1.0);
        let player = Vec3::new(6.0, 0.0, 0.0);
        assert_eq!(m.hit(1.0, player, Vec3::NEG_Y), Hit::Hurt);
        assert!(m.aggro && m.hp == SLIME_HP - 1.0);
        // It crosses 6 blocks to the player and touches them, and keeps doing so every second.
        let events = run(&mut m, Some(player), 12.0);
        let contacts = events
            .iter()
            .filter(|e| matches!(e, Event::Contact(d) if *d == 2.0))
            .count();
        assert!(contacts >= 2, "got {contacts} contacts");
        assert!(contacts <= 12, "cooldown holds ({contacts})");
        assert!(m.pos.distance(player) < 2.0);
    }

    #[test]
    fn contact_waits_out_its_cooldown() {
        let mut m = slime(3, Vec3::new(0.0, 0.0, 0.0));
        m.aggro = true;
        m.timer = 100.0; // stays put
        let near = Some(Vec3::new(0.4, 0.0, 0.0));
        let ev = run(&mut m, near, 0.5);
        assert_eq!(
            ev.iter().filter(|e| matches!(e, Event::Contact(_))).count(),
            1
        );
        assert_eq!(run(&mut m, near, 0.4).len(), 0, "still cooling down");
        assert_eq!(
            run(&mut m, near, 0.3)
                .iter()
                .filter(|e| matches!(e, Event::Contact(_)))
                .count(),
            1
        );
    }

    #[test]
    fn hits_have_a_cooldown_and_knock_back() {
        let mut m = slime(4, Vec3::new(0.0, 0.0, 0.0));
        run(&mut m, None, 1.0);
        let from = Vec3::new(-2.0, 0.0, 0.0);
        assert_eq!(m.hit(4.0, from, Vec3::NEG_Y), Hit::Hurt);
        assert!(
            m.vel.x > 1.0 && m.vel.y > 1.0,
            "knocked away from the attacker: {:?}",
            m.vel
        );
        assert_eq!(
            m.hit(4.0, from, Vec3::NEG_Y),
            Hit::Ignored,
            "still inside the hit cooldown"
        );
        run(&mut m, None, 0.6);
        assert_eq!(m.hit(4.0, from, Vec3::NEG_Y), Hit::Killed);
        assert!(m.dead());
    }

    #[test]
    fn player_damage_follows_the_sword() {
        assert_eq!(player_damage(None), 1.0);
        assert_eq!(player_damage(Some(items::IRON_PICKAXE)), 1.0);
        assert_eq!(player_damage(Some(crate::block::DIRT)), 1.0);
        let d: Vec<f32> = [
            items::WOODEN_SWORD,
            items::STONE_SWORD,
            items::COPPER_SWORD,
            items::IRON_SWORD,
        ]
        .iter()
        .map(|&s| player_damage(Some(s)))
        .collect();
        assert_eq!(d, [4.0, 5.0, 6.0, 7.0]);
        assert!(
            is_sword(Some(items::STONE_SWORD)) && !is_sword(Some(items::COAL)) && !is_sword(None)
        );
        // Two iron swings kill a slime (8 hp); eight fists do.
        let mut m = slime(5, Vec3::ZERO);
        assert_eq!(
            m.hit(player_damage(Some(items::IRON_SWORD)), Vec3::X, Vec3::NEG_Y),
            Hit::Hurt
        );
        m.hit_cd = 0.0;
        assert_eq!(
            m.hit(player_damage(Some(items::IRON_SWORD)), Vec3::X, Vec3::NEG_Y),
            Hit::Killed
        );
    }

    #[test]
    fn drops_are_deterministic_and_swords_give_more() {
        for id in 0..200u64 {
            let m = slime(id, Vec3::ZERO);
            assert_eq!(m.drops(false), m.drops(false));
            assert_eq!(m.drops(true), m.drops(true));
            let by_hand = m.drops(false).map_or(0, |s| s.count);
            let by_sword = m.drops(true).map_or(0, |s| s.count);
            assert!(
                by_hand <= 2 && (1..=3).contains(&by_sword),
                "{id}: {by_hand}/{by_sword}"
            );
            if let Some(s) = m.drops(true) {
                assert_eq!(s.item, items::SLIMEBALL);
            }
        }
        // Over many ids every count turns up.
        let counts = |sword: bool| {
            (0..400u64)
                .map(|i| slime(i, Vec3::ZERO).drops(sword).map_or(0, |s| s.count))
                .collect::<std::collections::BTreeSet<_>>()
        };
        assert_eq!(counts(false).into_iter().collect::<Vec<_>>(), [0, 1, 2]);
        assert_eq!(counts(true).into_iter().collect::<Vec<_>>(), [1, 2, 3]);
        // Townsfolk drop nothing.
        assert!(
            Mob::new(1, Kind::Bodoque, Vec3::Y, Vec3::Y)
                .drops(true)
                .is_none()
        );
    }

    #[test]
    fn townsfolk_cannot_be_hurt_and_face_a_near_player() {
        let mut b = Mob::new(9, Kind::Bodoque, Vec3::new(0.0, 0.2, 0.0), Vec3::Y);
        assert_eq!(b.hit(100.0, Vec3::X, Vec3::NEG_Y), Hit::Ignored);
        assert_eq!(b.hp, 1.0);
        let player = Some(Vec3::new(0.0, 0.0, 4.0));
        run(&mut b, player, 1.0);
        assert!(b.face.dot(Vec3::Z) > 0.99, "faces the player: {:?}", b.face);
        let at = b.pos;
        let ev = run(&mut b, player, 10.0);
        assert!(ev.is_empty());
        assert!(b.pos.distance(at) < 0.1, "stands still while watched");
    }

    #[test]
    fn townsfolk_stroll_slowly_inside_their_leash() {
        let mut b = Mob::new(10, Kind::Tulio, Vec3::new(0.0, 0.2, 0.0), Vec3::Y);
        b.leash = 3.0;
        let mut far = 0.0f32;
        let mut moved = 0.0f32;
        let mut last = b.pos;
        for _ in 0..(120.0 / DT) as usize {
            b.step(&env(None, &flat_world), DT);
            far = far.max((b.pos - b.home).length());
            moved += b.pos.distance(last);
            last = b.pos;
        }
        assert!(moved > 2.0, "walks about ({moved})");
        assert!(far < 3.0 + 0.8, "stays near home: {far}");
        // Slow: never faster than the stride.
        let before = b.pos;
        b.step(&env(None, &flat_world), DT);
        assert!(b.pos.distance(before) <= NPC_SPEED * DT * 1.5 + 0.1);
    }

    #[test]
    fn a_strolling_slime_keeps_out_of_water_and_floats_if_dropped_in() {
        // A pond: water for x > 2 (down to the floor).
        let wet = |p: Vec3| p.x > 2.0 && p.y < 1.0;
        let mut m = slime(11, Vec3::new(0.0, 0.0, 0.0));
        m.heading = Vec3::X;
        let mut max_x = f32::MIN;
        for _ in 0..(40.0 / DT) as usize {
            let e = Env {
                down: Vec3::NEG_Y,
                solid: &flat_world,
                wet: &wet,
                player: None,
            };
            m.step(&e, DT);
            max_x = max_x.max(m.pos.x);
            // Keep it in the test area.
            if m.pos.x < -6.0 || m.pos.z.abs() > 6.0 {
                m.pos = Vec3::new(0.0, m.pos.y, 0.0);
            }
        }
        assert!(max_x < 3.2, "went into the pond: {max_x}");
        // Dropped in, it floats up rather than sinking to the floor.
        let mut m = slime(12, Vec3::new(5.0, 0.4, 0.0));
        m.timer = 1000.0;
        for _ in 0..(3.0 / DT) as usize {
            let e = Env {
                down: Vec3::NEG_Y,
                solid: &flat_world,
                wet: &wet,
                player: None,
            };
            m.step(&e, DT);
        }
        assert!(m.pos.y > 0.5, "floats: {}", m.pos.y);
    }

    #[test]
    fn rays_meet_the_box() {
        let m = slime(1, Vec3::new(0.0, 0.0, 0.0));
        // Straight at it from 3 blocks away at chest height: hits the near face (0.375 from the middle).
        let t = m.ray(Vec3::new(-3.0, 0.5, 0.0), Vec3::X, 5.0).unwrap();
        assert!((t - 2.625).abs() < 0.01 || (t - 2.625).abs() < 0.2, "{t}");
        assert!(
            m.ray(Vec3::new(-3.0, 0.5, 0.0), Vec3::X, 2.0).is_none(),
            "out of reach"
        );
        assert!(
            m.ray(Vec3::new(-3.0, 2.0, 0.0), Vec3::X, 5.0).is_none(),
            "over its head"
        );
        assert!(
            m.ray(Vec3::new(-3.0, 0.5, 2.0), Vec3::X, 5.0).is_none(),
            "beside it"
        );
        assert!(
            m.ray(Vec3::new(-3.0, 0.5, 0.0), -Vec3::X, 5.0).is_none(),
            "looking away"
        );
        assert!(
            m.ray(Vec3::new(0.0, 0.5, 0.0), Vec3::X, 5.0).is_some(),
            "from inside"
        );
    }

    #[test]
    fn spawner_is_deterministic_and_capped() {
        let run = |seed| {
            let mut s = Spawner::new(seed);
            (0..2000).filter_map(|_| s.tick(0.1, 0)).collect::<Vec<_>>()
        };
        assert_eq!(run(7), run(7));
        assert_ne!(run(7), run(8));
        let a = run(7);
        assert!(a.len() > 10 && a.iter().all(|s| (SPAWN_NEAR..SPAWN_FAR).contains(&s.dist)));
        let mut s = Spawner::new(7);
        assert!(
            (0..2000).all(|_| s.tick(0.1, MAX_WILD).is_none()),
            "no spawns at the cap"
        );
    }

    #[test]
    fn kinds_have_names() {
        for k in Kind::ALL {
            assert_eq!(Kind::from_id(k.id()), Some(k));
        }
        assert_eq!(Kind::from_id("zombie"), None);
        assert_eq!(Kind::from_id("chicken"), Some(Kind::Chicken));
        assert!(
            Kind::Tulio.invulnerable()
                && !Kind::Slime.invulnerable()
                && Kind::SlimeKing.always_hostile()
        );
    }
}
