//! What the world remembers about its people, and putting them there: the one-time gifts and the
//! fallen Slime King (`orbit_core::dialogue::Flags`), and where Bodoque and Tulio stand, saved next
//! to the world as `npcs.bin` (b"FLKNPC01", flags, then each one's kind, planet and position).
//! Bodoque lives on the home planet (the meadow you start on), always near where you spawn (a
//! spot fixed by the world's seed); Tulio on its moon. They are put on the ground when their planet's
//! blocks are loaded, and floating names are drawn over them.

use crate::mobs::{Mobs, ground_from_above};
use crate::save::SaveDir;
use crate::sky::SkyCamera;
use crate::world::Space;
use bevy::prelude::*;
use orbit_core::dialogue::Flags;
use orbit_core::mob::{Kind, Mob};
use orbit_core::planet::{Planet, PlanetSpec};
use orbit_core::rng::mix;
use std::collections::HashMap;

const MAGIC: &[u8; 8] = b"FLKNPC01";
/// Farthest a name is drawn from, blocks.
const NAME_RANGE: f32 = 30.0;

pub struct StoryPlugin;

impl Plugin for StoryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Story>()
            .add_systems(PreUpdate, load.run_if(resource_changed::<SaveDir>))
            .add_systems(
                Update,
                (spawn_npcs, keep_spots, name_tags)
                    .chain()
                    .after(crate::mobs::run_mobs),
            )
            .add_systems(Update, autosave)
            .add_systems(Last, save_on_exit);
    }
}

#[derive(Resource, Default)]
pub struct Story {
    pub flags: Flags,
    /// Where each townsperson last stood: (planet, position in its frame), by `Kind::ALL` index.
    pub spots: HashMap<u8, (u64, Vec3)>,
    pub dirty: bool,
}

fn kind_index(k: Kind) -> u8 {
    Kind::ALL.iter().position(|&x| x == k).unwrap() as u8
}

pub fn encode(flags: Flags, spots: &HashMap<u8, (u64, Vec3)>) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend(flags.to_bytes());
    let mut keys: Vec<_> = spots.keys().copied().collect();
    keys.sort();
    out.push(keys.len() as u8);
    for k in keys {
        let (planet, p) = spots[&k];
        out.push(k);
        out.extend(planet.to_le_bytes());
        for c in p.to_array() {
            out.extend(c.to_le_bytes());
        }
    }
    out
}

pub fn decode(data: &[u8]) -> Option<(Flags, HashMap<u8, (u64, Vec3)>)> {
    let rest = data.strip_prefix(MAGIC)?;
    for flag_len in [3usize, 2] {
        let Some(flags) = Flags::from_bytes(rest.get(..flag_len)?) else {
            continue;
        };
        let Some(&count) = rest.get(flag_len) else {
            continue;
        };
        let n = count as usize;
        let mut at = flag_len + 1;
        let mut spots = HashMap::new();
        let mut valid = true;
        for _ in 0..n {
            let Some(b) = rest.get(at..at + 21) else {
                valid = false;
                break;
            };
            at += 21;
            let kind = b[0];
            if kind as usize >= Kind::ALL.len() {
                valid = false;
                break;
            }
            let Some(planet) = b[1..9].try_into().ok().map(u64::from_le_bytes) else {
                valid = false;
                break;
            };
            let f = |i: usize| f32::from_le_bytes(b[9 + 4 * i..13 + 4 * i].try_into().unwrap());
            let p = Vec3::new(f(0), f(1), f(2));
            if !p.is_finite() {
                valid = false;
                break;
            }
            spots.insert(kind, (planet, p));
        }
        if valid && at == rest.len() {
            return Some((flags, spots));
        }
    }
    None
}

fn load(dir: Res<SaveDir>, mut story: ResMut<Story>) {
    let Some(dir) = &dir.0 else { return };
    *story = Story::default();
    if let Ok(data) = std::fs::read(dir.join("npcs.bin")) {
        match decode(&data) {
            Some((flags, spots)) => {
                info!(
                    "story: loaded flags {flags:?} and {} townsfolk",
                    spots.len()
                );
                *story = Story {
                    flags,
                    spots,
                    dirty: false,
                };
            }
            None => warn!("save: could not read the townsfolk"),
        }
    }
}

fn write(dir: &SaveDir, story: &Story) {
    let Some(dir) = &dir.0 else { return };
    if story.spots.is_empty() && story.flags == Flags::default() && !dir.join("npcs.bin").exists() {
        return;
    }
    let path = dir.join("npcs.bin");
    let tmp = path.with_extension("tmp");
    let r = std::fs::create_dir_all(dir)
        .and_then(|_| std::fs::write(&tmp, encode(story.flags, &story.spots)))
        .and_then(|_| std::fs::rename(&tmp, &path));
    if let Err(e) = r {
        warn!("save: could not save the townsfolk: {e}");
    }
}

fn autosave(dir: Res<SaveDir>, mut story: ResMut<Story>, time: Res<Time>, mut since: Local<f32>) {
    *since += time.delta_secs();
    if *since >= 20.0 {
        *since = 0.0;
        if story.dirty {
            write(&dir, &story);
            story.dirty = false;
        }
    }
}

fn save_on_exit(mut exit: MessageReader<AppExit>, dir: Res<SaveDir>, story: Res<Story>) {
    if exit.read().count() > 0 {
        write(&dir, &story);
    }
}

/// Where townsfolk live: the planet each is on.
fn site(space: &Space, kind: Kind) -> Option<u64> {
    let sys = space.universe.system(IVec3::ZERO)?;
    let home = sys.planets.first()?;
    match kind {
        Kind::Bodoque => Some(home.id),
        Kind::Tulio => sys
            .planets
            .iter()
            .find(|m| m.host.is_some_and(|h| h.id == home.id))
            .map(|m| m.id),
        _ => None,
    }
}

/// A spot on the ground some blocks from `base` (a direction in the planet's frame), the same
/// every time for a seed: tries a ring of spots till one is dry and open.
pub fn site_spot(
    planet: &Planet,
    spec: &PlanetSpec,
    base: Vec3,
    seed: u64,
    kind: Kind,
) -> Option<Vec3> {
    let side = base.any_orthonormal_vector();
    for k in 0..24u64 {
        let h = mix(seed ^ mix(kind_index(kind) as u64 + 77) ^ k);
        let around = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
        let dist = 5.0 + (k / 6) as f32 * 2.5 + (h >> 20) as f32 % 2.0;
        let axis = Quat::from_axis_angle(base, around) * side;
        let dir = Quat::from_axis_angle(axis, dist / spec.radius) * base;
        if let Some(p) = ground_from_above(planet, dir, spec.ceiling() + 3.0, spec.radius * 0.4) {
            return Some(p);
        }
    }
    None
}

/// The direction (planet frame) a townsperson's spot is searched round: Bodoque by the spawn;
/// Tulio on the face of the moon that always looks at its planet (it is tidally locked), which a
/// ship coming from the planet meets first.
fn base_dir(space: &Space, spec: &PlanetSpec, kind: Kind) -> Vec3 {
    match kind {
        Kind::Tulio => spec
            .host
            .and_then(|h| space.planets.get(&h.id))
            .map_or(Vec3::Y, |h| {
                (spec.rotation.inverse() * (h.spec.center - spec.center).as_vec3())
                    .normalize_or(Vec3::Y)
            }),
        _ => orbit_core::universe::coast(spec),
    }
}

fn spawn_npcs(
    mut mobs: ResMut<Mobs>,
    space: Res<Space>,
    story: Res<Story>,
    time: Res<Time>,
    mut since: Local<f32>,
) {
    *since -= time.delta_secs();
    if *since > 0.0 {
        return;
    }
    *since = 0.5;
    for kind in [Kind::Bodoque, Kind::Tulio] {
        if mobs.list.iter().any(|s| s.mob.kind == kind) {
            continue;
        }
        let Some(id) = site(&space, kind) else {
            continue;
        };
        let Some(slot) = space.planets.get(&id) else {
            continue;
        };
        let Some(voxels) = slot.voxel.as_ref() else {
            continue;
        };
        let at = match story.spots.get(&kind_index(kind)) {
            Some(&(planet, p)) if planet == id => Some(p),
            _ => site_spot(
                &voxels.read().unwrap(),
                &slot.spec,
                base_dir(&space, &slot.spec, kind),
                space.universe.seed,
                kind,
            ),
        };
        let Some(at) = at else { continue };
        let mut mob = Mob::new(
            mix(0xB0D0_0000 ^ kind_index(kind) as u64 ^ space.universe.seed),
            kind,
            at,
            at.normalize_or(Vec3::Y),
        );
        mob.leash = if kind == Kind::Bodoque { 4.0 } else { 3.0 };
        mobs.add(id, mob);
    }
}

/// Where each townsperson stands is remembered (saved with the world).
fn keep_spots(mobs: Res<Mobs>, mut story: ResMut<Story>, time: Res<Time>, mut since: Local<f32>) {
    *since -= time.delta_secs();
    if *since > 0.0 {
        return;
    }
    *since = 2.0;
    for s in mobs.list.iter().filter(|s| s.mob.kind.is_npc()) {
        let k = kind_index(s.mob.kind);
        let moved = story
            .spots
            .get(&k)
            .is_none_or(|&(p, at)| p != s.planet || at.distance(s.mob.pos) > 0.3);
        if moved {
            story.spots.insert(k, (s.planet, s.mob.pos));
            story.dirty = true;
        }
    }
}

#[derive(Component)]
struct NameTag(u64);

/// A floating name over each townsperson and the Slime King (not the slimes).
fn name_tags(
    mut commands: Commands,
    mobs: Res<Mobs>,
    cam: Single<(&Camera, &GlobalTransform), With<SkyCamera>>,
    globals: Query<&GlobalTransform>,
    mut tags: Query<(Entity, &NameTag, &mut Node, &mut Visibility)>,
) {
    let (camera, cam_at) = *cam;
    let mut seen: Vec<u64> = Vec::new();
    for s in mobs.list.iter().filter(|s| s.mob.kind.is_npc()) {
        let Some(g) = s.model.and_then(|m| globals.get(m).ok()) else {
            continue;
        };
        let head = g.translation() + g.rotation() * Vec3::Y * (s.mob.size().1 + 0.35);
        let ok = head.distance(cam_at.translation()) < NAME_RANGE;
        let screen = camera.world_to_viewport(cam_at, head).ok().filter(|_| ok);
        seen.push(s.mob.id);
        match tags.iter_mut().find(|t| t.1.0 == s.mob.id) {
            Some((_, _, mut node, mut vis)) => match screen {
                Some(p) => {
                    node.left = px(p.x);
                    node.top = px(p.y);
                    *vis = Visibility::Inherited;
                }
                None => *vis = Visibility::Hidden,
            },
            None => {
                commands.spawn((
                    NameTag(s.mob.id),
                    Text::new(s.mob.kind.name()),
                    TextFont {
                        font_size: FontSize::Px(18.0),
                        ..default()
                    },
                    TextColor(Color::srgb(1.0, 0.95, 0.6)),
                    TextShadow::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        ..default()
                    },
                    UiTransform {
                        translation: Val2::percent(-50.0, -100.0),
                        ..default()
                    },
                    GlobalZIndex(4),
                ));
            }
        }
    }
    for (e, t, _, _) in &tags {
        if !seen.contains(&t.0) {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_townsfolk_save_and_load() {
        let mut spots = HashMap::new();
        spots.insert(2u8, (0xABCDu64, Vec3::new(1.5, -2.25, 100.0)));
        spots.insert(3u8, (7u64, Vec3::new(0.0, 80.0, 0.5)));
        let mut flags = Flags::default();
        flags.boss_defeated = true;
        let bytes = encode(flags, &spots);
        assert_eq!(decode(&bytes), Some((flags, spots.clone())));
        assert_eq!(
            decode(&encode(Flags::default(), &HashMap::new())),
            Some((Flags::default(), HashMap::new()))
        );
        let mut old = MAGIC.to_vec();
        old.extend([0, 1, 0]); // old two-byte flags, followed by an empty spot table
        assert_eq!(
            decode(&old),
            Some((
                Flags {
                    boss_defeated: true,
                    ..Flags::default()
                },
                HashMap::new()
            ))
        );
        // Bad magic, a cut file, trailing junk, an unknown kind.
        assert!(decode(b"nonsense").is_none());
        assert!(decode(&bytes[..bytes.len() - 3]).is_none());
        let mut junk = bytes.clone();
        junk.push(0);
        assert!(decode(&junk).is_none());
        let mut bad = bytes.clone();
        bad[11] = 9;
        assert!(decode(&bad).is_none());
    }
}
