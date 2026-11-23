//! The Slime King: a big slime (the slime's model at three times its size), always hostile, with
//! 200 hit points and two phases. It hops after the player; now and then it stops for an attack
//! that is announced first (it crouches and glows, `State::crouch` and `glow`):
//!
//! - Smash: leaps at where the player stands and lands with a shock that hurts everyone close
//!   to the ground within its radius.
//! - Charge: a straight, fast run the way the player was when it began.
//! - Summon (phase 2 only): calls two or three small slimes.
//!
//! Under half its life (phase 2) it is faster, waits less between attacks, shocks wider and
//! summons. It hurts by touch too (not while crouching). All of it is deterministic per id.

use crate::mob::{Env, Event, Mob};
use crate::rng::Rng;
use glam::Vec3;

pub const KING_HP: f32 = 200.0;
/// Damage of its touch, and of the Smash's shock.
pub const KING_DAMAGE: f32 = 4.0;
pub const SMASH_DAMAGE: f32 = 6.0;
pub const SMASH_RADIUS: f32 = 5.0;
pub const SCALE: f32 = 3.0;
/// It does not attack a player farther than this.
pub const ATTACK_RANGE: f32 = 32.0;
pub const CHARGE_SPEED: f32 = 12.0;
pub const CHARGE_TIME: f32 = 1.1;
/// Fraction of its life under which it is in its second phase.
pub const PHASE_TWO: f32 = 0.5;
/// The small slimes it calls: size and hit points.
pub const MINION_SCALE: f32 = 0.6;
pub const MINION_HP: f32 = 4.0;
const LEAP_SPEED: f32 = 13.0;
const GRAVITY: f32 = 28.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Attack {
    Smash,
    Charge,
    Summon,
}

impl Attack {
    pub fn name(self) -> &'static str {
        match self {
            Attack::Smash => "smash",
            Attack::Charge => "charge",
            Attack::Summon => "summon",
        }
    }

    /// Seconds of crouching and glowing before it (phase 1).
    pub fn warning(self) -> f32 {
        match self {
            Attack::Smash => 1.0,
            Attack::Charge => 0.9,
            Attack::Summon => 1.2,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    /// Crouching, glowing: the attack is coming.
    Warning,
    Doing,
    /// Catching its breath after it.
    Recover,
}

#[derive(Clone, Copy, Debug)]
pub struct State {
    /// 1, or 2 under half its life.
    pub phase: u8,
    pub attack: Option<(Attack, Stage)>,
    /// Seconds left in the current stage.
    pub timer: f32,
    /// Seconds before it may start another attack.
    pub cooldown: f32,
    /// The way a Smash or a Charge goes (unit, along the ground).
    pub dir: Vec3,
    /// Running flat out (no friction on the ground).
    pub charging: bool,
    /// Left the ground in its leap (so landing is the shock).
    pub airborne: bool,
    /// 0..1: how much it shines (a hint of the coming attack) and how low it squats.
    pub glow: f32,
    pub crouch: f32,
}

impl Default for State {
    fn default() -> Self {
        Self {
            phase: 1,
            attack: None,
            timer: 0.0,
            cooldown: 2.0,
            dir: Vec3::X,
            charging: false,
            airborne: false,
            glow: 0.0,
            crouch: 0.0,
        }
    }
}

impl State {
    pub fn warning(&self) -> Option<Attack> {
        match self.attack {
            Some((a, Stage::Warning)) => Some(a),
            _ => None,
        }
    }
}

/// How many slimeballs it drops when it falls.
pub fn drop_count(r: &mut Rng) -> i64 {
    r.int(6, 10)
}

/// How many small slimes a Summon calls.
pub fn summon_count(r: &mut Rng) -> u8 {
    r.int(2, 3) as u8
}

fn ease(x: &mut f32, to: f32, rate: f32, dt: f32) {
    let step = rate * dt;
    *x = if *x < to {
        (*x + step).min(to)
    } else {
        (*x - step).max(to)
    };
}

pub(crate) fn ai(m: &mut Mob, env: &Env, dt: f32, to_player: Option<Vec3>, ev: &mut Vec<Event>) {
    let up = -env.down;
    let flat = |v: Vec3| v - up * v.dot(up);
    m.aggro = true;
    if m.boss.phase == 1 && m.hp <= KING_HP * PHASE_TWO && !m.dead() {
        m.boss.phase = 2;
        m.boss.cooldown = m.boss.cooldown.min(1.0);
        ev.push(Event::Enraged);
    }
    let fast = if m.boss.phase == 2 { 1.4 } else { 1.0 };
    m.boss.charging = false;
    let to = to_player.map(flat).filter(|d| d.length_squared() > 1e-4);
    let vr = m.vel.dot(env.down);
    let stand = |m: &mut Mob| m.vel = env.down * vr;
    match m.boss.attack {
        None => {
            ease(&mut m.boss.glow, 0.0, 3.0, dt);
            ease(&mut m.boss.crouch, 0.0, 4.0, dt);
            m.slime_ai(env, dt, to_player, ev);
            m.boss.cooldown -= dt;
            if m.boss.cooldown <= 0.0 && m.on_ground {
                if let Some(d) = to.filter(|d| d.length() < ATTACK_RANGE) {
                    let mut options = vec![Attack::Smash];
                    if d.length() > 5.0 {
                        options.push(Attack::Charge);
                    }
                    if m.boss.phase == 2 {
                        options.push(Attack::Summon);
                    }
                    let a = options[m.rng.int(0, options.len() as i64 - 1) as usize];
                    m.boss.attack = Some((a, Stage::Warning));
                    m.boss.timer = a.warning() / fast;
                    m.boss.dir = d.normalize();
                    stand(m);
                    ev.push(Event::Telegraph(a));
                }
            }
        }
        Some((a, Stage::Warning)) => {
            ease(&mut m.boss.glow, 1.0, 1.6 * fast, dt);
            ease(&mut m.boss.crouch, 1.0, 2.5 * fast, dt);
            stand(m);
            // Turns with the player until the last moment (a charge keeps the way it was facing).
            if let Some(d) = to {
                m.boss.dir = d.normalize();
                m.face = m.boss.dir;
            }
            m.boss.timer -= dt;
            if m.boss.timer <= 0.0 {
                match a {
                    Attack::Smash => {
                        // Lands where the player stands now: its flight takes `2 v / g` seconds.
                        let dist = to.map_or(4.0, |d| d.length()).clamp(2.0, 18.0);
                        let air = 2.0 * LEAP_SPEED / GRAVITY;
                        m.vel = up * LEAP_SPEED + m.boss.dir * (dist / air);
                        m.on_ground = false;
                        m.boss.airborne = true;
                        m.boss.attack = Some((a, Stage::Doing));
                    }
                    Attack::Charge => {
                        m.boss.timer = CHARGE_TIME;
                        m.boss.attack = Some((a, Stage::Doing));
                    }
                    Attack::Summon => {
                        ev.push(Event::Summon(summon_count(&mut m.rng)));
                        m.boss.timer = 1.2 / fast;
                        m.boss.attack = Some((a, Stage::Recover));
                    }
                }
            }
        }
        Some((Attack::Smash, Stage::Doing)) => {
            ease(&mut m.boss.crouch, 0.0, 6.0, dt);
            ease(&mut m.boss.glow, 0.3, 2.0, dt);
            if m.on_ground && m.boss.airborne {
                m.boss.airborne = false;
                let radius = SMASH_RADIUS * if m.boss.phase == 2 { 1.25 } else { 1.0 };
                let hits = to_player.is_some_and(|d| {
                    flat(d).length() < radius
                        && d.dot(up) > -crate::player::HEIGHT
                        && d.dot(up) < 2.2
                });
                ev.push(Event::Smash {
                    at: m.pos,
                    radius,
                    damage: SMASH_DAMAGE,
                    hits,
                });
                m.boss.attack = Some((Attack::Smash, Stage::Recover));
                m.boss.timer = 1.0 / fast;
            }
        }
        Some((Attack::Charge, Stage::Doing)) => {
            ease(&mut m.boss.crouch, 0.0, 6.0, dt);
            ease(&mut m.boss.glow, 0.6, 2.0, dt);
            m.boss.charging = true;
            m.face = m.boss.dir;
            if m.on_ground {
                m.vel = m.boss.dir * CHARGE_SPEED * fast + env.down * vr;
            }
            m.boss.timer -= dt;
            if m.boss.timer <= 0.0 {
                m.boss.attack = Some((Attack::Charge, Stage::Recover));
                m.boss.timer = 1.0 / fast;
            }
        }
        Some((_, Stage::Doing)) => m.boss.attack = None,
        Some((_, Stage::Recover)) => {
            ease(&mut m.boss.glow, 0.0, 2.0, dt);
            ease(&mut m.boss.crouch, 0.0, 4.0, dt);
            m.boss.timer -= dt;
            if m.boss.timer <= 0.0 {
                m.boss.attack = None;
                m.boss.cooldown = m.rng.range(2.0, 3.5) / fast;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mob::{Event, Kind};

    const DT: f32 = 1.0 / 60.0;

    fn floor(p: Vec3) -> bool {
        p.y < 0.0
    }

    fn king() -> Mob {
        Mob::new(7, Kind::SlimeKing, Vec3::new(0.0, 0.1, 0.0), Vec3::Y)
    }

    /// Runs `secs` with the player at `at`; the events with the time they came at.
    fn run(m: &mut Mob, at: Option<Vec3>, secs: f32) -> Vec<(f32, Event)> {
        let mut out = Vec::new();
        for i in 0..(secs / DT) as usize {
            let env = Env {
                down: Vec3::NEG_Y,
                solid: &floor,
                wet: &|_| false,
                player: at,
            };
            for e in m.step(&env, DT) {
                out.push((i as f32 * DT, e));
            }
        }
        out
    }

    #[test]
    fn the_king_is_big_hostile_and_tough() {
        let m = king();
        assert_eq!((m.hp, m.scale, m.aggro), (200.0, 3.0, true));
        assert_eq!(m.kind.contact_damage(), 4.0);
        assert!(m.size().1 > 2.5, "three times the slime");
        assert_eq!(m.boss.phase, 1);
    }

    #[test]
    fn every_attack_is_announced_before_it_lands() {
        let mut m = king();
        let player = Some(Vec3::new(9.0, 0.0, 0.0));
        let ev = run(&mut m, player, 40.0);
        let first_warning = ev
            .iter()
            .find(|(_, e)| matches!(e, Event::Telegraph(_)))
            .expect("an attack began");
        // Whatever the first attack was, nothing of it (a shock) happens before its warning time.
        if let Event::Telegraph(a) = first_warning.1 {
            assert_ne!(a, Attack::Summon, "no summoning in the first phase");
        }
        let smash = ev.iter().find(|(_, e)| matches!(e, Event::Smash { .. }));
        if let Some((t, _)) = smash {
            let warned = ev
                .iter()
                .filter(|(w, e)| w < t && matches!(e, Event::Telegraph(Attack::Smash)))
                .last()
                .expect("a warning came first");
            assert!(t - warned.0 >= 0.9, "warned {:.2}s before", t - warned.0);
        }
        // Over 40 seconds there are several attacks, never a summon, and every Telegraph came with
        // its crouch (glow rose while it waited).
        assert!(
            ev.iter()
                .filter(|(_, e)| matches!(e, Event::Telegraph(_)))
                .count()
                >= 3
        );
        assert!(
            !ev.iter()
                .any(|(_, e)| matches!(e, Event::Summon(_) | Event::Enraged))
        );
    }

    #[test]
    fn it_crouches_and_glows_while_it_warns() {
        let mut m = king();
        let player = Some(Vec3::new(9.0, 0.0, 0.0));
        let mut seen = false;
        for _ in 0..(40.0 / DT) as usize {
            let env = Env {
                down: Vec3::NEG_Y,
                solid: &floor,
                wet: &|_| false,
                player,
            };
            m.step(&env, DT);
            if let Some((_, crate::boss::Stage::Warning)) = m.boss.attack {
                if m.boss.timer < 0.1 {
                    assert!(
                        m.boss.glow > 0.5 && m.boss.crouch > 0.5,
                        "glow {} crouch {}",
                        m.boss.glow,
                        m.boss.crouch
                    );
                    seen = true;
                }
            }
        }
        assert!(seen);
    }

    #[test]
    fn the_smash_shocks_a_player_standing_where_it_lands_but_not_one_far_off() {
        // The player stands still at 8 blocks: its leap lands on them.
        let mut hit = false;
        let mut m = king();
        for e in run(&mut m, Some(Vec3::new(8.0, 0.0, 0.0)), 60.0) {
            if let Event::Smash {
                damage,
                hits,
                radius,
                ..
            } = e.1
            {
                assert_eq!((damage, radius), (6.0, 5.0));
                hit |= hits;
            }
        }
        assert!(hit, "a shock reached a player standing in its way");
        // A player 25 blocks off is out of the shock's reach (and the King may not even try).
        let mut m = king();
        for e in run(&mut m, Some(Vec3::new(25.0, 0.0, 0.0)), 40.0) {
            if let Event::Smash { hits, .. } = e.1 {
                assert!(!hits || m.pos.x > 18.0);
            }
        }
        // Up in the air (jumping) over the shock: not hurt.
        let mut m = king();
        let jumping = run(&mut m, Some(Vec3::new(8.0, 3.0, 0.0)), 40.0);
        assert!(
            jumping
                .iter()
                .all(|(_, e)| !matches!(e, Event::Smash { hits: true, .. }))
        );
    }

    #[test]
    fn a_charge_runs_fast_and_straight() {
        let mut m = king();
        let player = Some(Vec3::new(14.0, 0.0, 0.0));
        let mut best = 0.0f32;
        let mut start = None;
        for _ in 0..(120.0 / DT) as usize {
            let env = Env {
                down: Vec3::NEG_Y,
                solid: &floor,
                wet: &|_| false,
                player,
            };
            m.step(&env, DT);
            match (m.boss.attack, start) {
                (Some((Attack::Charge, Stage::Doing)), None) => start = Some(m.pos),
                (Some((Attack::Charge, Stage::Doing)), Some(s)) => {
                    best = best.max(m.pos.distance(s));
                    assert!(m.pos.z.abs() < 0.5 + s.z.abs(), "straight");
                }
                (Some((Attack::Charge, Stage::Recover)), Some(_)) => start = None,
                _ => {}
            }
            // Keep the arena in bounds, back at the start, so the test sees many charges.
            if m.pos.x > 12.0 {
                m.pos.x = 0.0;
                start = None;
            }
        }
        assert!(best > 3.0, "charged {best} blocks");
    }

    #[test]
    fn under_half_life_it_changes_phase_and_summons() {
        let mut m = king();
        let player = Some(Vec3::new(9.0, 0.0, 0.0));
        m.hp = 101.0;
        assert!(
            run(&mut m, player, 1.0)
                .iter()
                .all(|(_, e)| !matches!(e, Event::Enraged))
        );
        m.hp = 99.0;
        let ev = run(&mut m, player, 60.0);
        assert_eq!(
            ev.iter()
                .filter(|(_, e)| matches!(e, Event::Enraged))
                .count(),
            1,
            "once"
        );
        assert_eq!(m.boss.phase, 2);
        let summons: Vec<u8> = ev
            .iter()
            .filter_map(|(_, e)| {
                if let Event::Summon(n) = e {
                    Some(*n)
                } else {
                    None
                }
            })
            .collect();
        assert!(
            !summons.is_empty() && summons.iter().all(|n| (2..=3).contains(n)),
            "{summons:?}"
        );
        // Phase 2 attacks more often than phase 1 did.
        let count = |m: &mut Mob, secs| {
            run(m, player, secs)
                .iter()
                .filter(|(_, e)| matches!(e, Event::Telegraph(_)))
                .count()
        };
        let (mut a, mut b) = (king(), king());
        b.hp = 50.0;
        assert!(count(&mut b, 60.0) > count(&mut a, 60.0));
    }

    #[test]
    fn it_ignores_a_player_out_of_range_and_is_deterministic() {
        let mut m = king();
        assert!(
            run(&mut m, Some(Vec3::new(500.0, 0.0, 0.0)), 30.0)
                .iter()
                .all(|(_, e)| !matches!(e, Event::Telegraph(_)))
        );
        assert!(run(&mut king(), None, 10.0).is_empty());
        let (mut a, mut b) = (king(), king());
        let p = Some(Vec3::new(9.0, 0.0, 0.0));
        assert_eq!(run(&mut a, p, 30.0), run(&mut b, p, 30.0));
        assert_eq!(a.pos, b.pos);
    }

    #[test]
    fn the_king_drops_several_slimeballs_and_touching_hurts_unless_it_crouches() {
        let m = king();
        let n = m.drops(true).map_or(0, |s| s.count);
        assert!((6..=10).contains(&n), "{n}");
        // Close to the player it hurts by touch (4), but not while crouching for an attack.
        let mut m = king();
        m.boss.cooldown = 1e9;
        let touch = Some(Vec3::new(1.0, 0.0, 0.0));
        let ev = run(&mut m, touch, 1.2);
        assert!(
            ev.iter()
                .any(|(_, e)| matches!(e, Event::Contact(d) if *d == 4.0))
        );
        let mut m = king();
        m.boss.attack = Some((Attack::Smash, Stage::Warning));
        m.boss.timer = 5.0;
        assert!(
            run(&mut m, touch, 2.0)
                .iter()
                .all(|(_, e)| !matches!(e, Event::Contact(_)))
        );
    }
}
