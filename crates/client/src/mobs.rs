//! Criaturas: slimes, gallinas y `/summon`.
//! The state and the brains are `orbit_core::mob`; this drives them in their planet's frame, feeds
//! them the world, hits them (left click: a mob under the crosshair wins over the block behind
//! it), hurts the player on contact, drops what they leave, and draws them.
//!
//! The slime is the glTF in `assets/models/slime.gltf` (animations `idle`, `bounce`), its materials
//! swapped for `ModelMaterial` so it is lit like the blocks around it (as the elevator cabin is).
//! Mobs are children of their planet's root, in planet-local coordinates.
//! Wild slimes appear only in survival, only on meadow planets, at most `MAX_WILD` at a time.
//! They are not saved (neither is the Slime King).

use crate::chat::{ActionBar, Keyboard};
use crate::inventory::{Drops, Frame};
use crate::lighting::ModelMaterial;
use crate::map::GalaxyMap;
use crate::player::Player;
use crate::world::{Origin, Space};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy::world_serialization::WorldInstanceReady;
use orbit_core::health::DamageKind;
use orbit_core::inventory::GameMode;
use orbit_core::mob::{self, Env, Event, Hit, Kind, Mob, Spawner};
use orbit_core::planet::{Biome, Planet};
use orbit_core::player::REACH;

const SLIME_MODEL: &str = "models/slime.gltf";
const CHICKEN_MODEL: &str = "models/chicken.gltf";
const BODOQUE_MODEL: &str = "models/bodoque.gltf";
/// Blockbench pixel models use 16 units per metre; the chicken asset is deliberately half that
/// scale so its 24-pixel body is about 0.75 m tall and fits the 0.9 m mob hitbox.
const CHICKEN_MODEL_SCALE: f32 = 1.0 / 32.0;

pub struct MobsPlugin;

impl Plugin for MobsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Mobs>()
            .add_systems(Startup, (load_assets, spawn_boss_bar))
            .add_observer(slime_ready)
            .add_systems(
                Update,
                aim.after(crate::player::sync_camera)
                    .before(crate::player::interact),
            )
            .add_systems(
                Update,
                (
                    summon,
                    spawn_wild,
                    spawn_chickens,
                    run_mobs,
                    draw_mobs,
                    light_mobs,
                    animate_mobs,
                    boss_bar,
                )
                    .chain()
                    .after(crate::player::interact),
            );
    }
}

/// A mob asked for: where (None: ahead of the player, near for most, 8 blocks for the King).
pub struct Request {
    pub kind: Kind,
    pub at: Option<Vec3>,
}

/// One mob in the world and what is drawn for it.
pub struct Slot {
    pub mob: Mob,
    pub planet: u64,
    pub(crate) model: Option<Entity>,
    mats: Vec<(Handle<ModelMaterial>, Vec4)>,
    /// The animation player in its model, and its idle and bounce clips.
    anim: Option<(Entity, AnimationNodeIndex, AnimationNodeIndex)>,
    /// True while the colored-box fallback is waiting for the real GLTF.
    fallback: bool,
    last_ground: bool,
}

#[derive(Resource)]
pub struct Mobs {
    pub list: Vec<Slot>,
    /// `/summon` and the Slime Crown Core, waiting for the next frame.
    pub requests: Vec<Request>,
    /// The mob under the crosshair this frame (by id): the clicks are its.
    pub aimed: Option<u64>,
    /// ... and it is a townsperson (a right click talks to them).
    pub aimed_npc: bool,
    spawner: Spawner,
    chicken_timer: f32,
    next_id: u64,
    /// Ids of mobs that were killed (for the log and the demos).
    pub kills: u32,
}

impl Default for Mobs {
    fn default() -> Self {
        Self {
            list: Vec::new(),
            requests: Vec::new(),
            aimed: None,
            aimed_npc: false,
            spawner: Spawner::new(0xF04_10C0),
            chicken_timer: 8.0,
            next_id: 0,
            kills: 0,
        }
    }
}

impl Mobs {
    /// Whether a Slime King stands (alive) anywhere.
    pub fn king_alive(&self) -> bool {
        self.list
            .iter()
            .any(|s| s.mob.kind == Kind::SlimeKing && !s.mob.dead())
    }

    pub fn find(&self, id: u64) -> Option<&Slot> {
        self.list.iter().find(|s| s.mob.id == id)
    }

    fn new_id(&mut self) -> u64 {
        self.next_id += 1;
        orbit_core::rng::mix(0x30B_5EED ^ self.next_id)
    }

    /// Puts a mob on a planet's ground.
    pub fn add(&mut self, planet: u64, mob: Mob) {
        info!(
            "mobs: {} #{:x} appears on planet {planet:x} at {:?}",
            mob.kind.name(),
            mob.id & 0xFFFF,
            mob.pos
        );
        self.list.push(Slot {
            mob,
            planet,
            model: None,
            mats: Vec::new(),
            anim: None,
            fallback: false,
            last_ground: false,
        });
    }
}

#[derive(Resource)]
struct MobAssets {
    scene: Handle<WorldAsset>,
    gltf: Handle<Gltf>,
    chicken_scene: Handle<WorldAsset>,
    chicken_gltf: Handle<Gltf>,
    bodoque_scene: Handle<WorldAsset>,
    bodoque_gltf: Handle<Gltf>,
}

/// Marks a mob's model with the mob's id.
#[derive(Component)]
pub struct MobModel(pub u64);

fn load_assets(mut commands: Commands, assets: Res<AssetServer>) {
    commands.insert_resource(MobAssets {
        scene: assets.load(GltfAssetLabel::Scene(0).from_asset(SLIME_MODEL)),
        gltf: assets.load(SLIME_MODEL),
        chicken_scene: assets.load(GltfAssetLabel::Scene(0).from_asset(CHICKEN_MODEL)),
        chicken_gltf: assets.load(CHICKEN_MODEL),
        bodoque_scene: assets.load(GltfAssetLabel::Scene(0).from_asset(BODOQUE_MODEL)),
        bodoque_gltf: assets.load(BODOQUE_MODEL),
    });
}

fn npc_asset(assets: &MobAssets, kind: Kind) -> Option<(&Handle<WorldAsset>, &Handle<Gltf>)> {
    match kind {
        Kind::Chicken => Some((&assets.chicken_scene, &assets.chicken_gltf)),
        Kind::Bodoque => Some((&assets.bodoque_scene, &assets.bodoque_gltf)),
        _ => None,
    }
}

fn real_model_scale(kind: Kind) -> f32 {
    match kind {
        Kind::Chicken => CHICKEN_MODEL_SCALE,
        _ => 1.0,
    }
}

#[cfg(test)]
mod model_scale_tests {
    use super::*;

    #[test]
    fn chicken_pixel_extent_fits_hitbox() {
        assert!((24.0 * real_model_scale(Kind::Chicken) - 0.75).abs() < f32::EPSILON);
        assert!(24.0 * real_model_scale(Kind::Chicken) <= 0.9);
    }
}

/// The ground under `from` (planet-local): scans down along the local up from 8 blocks above to
/// 12 below; the first solid point, and the spot just over it. None: a wall or water or a hole.
pub fn ground_near(planet: &Planet, from: Vec3) -> Option<Vec3> {
    let up = from.normalize_or(Vec3::Y);
    let mut p = from + up * 8.0;
    if planet.holds_at(p) {
        return None;
    }
    for _ in 0..80 {
        let next = p - up * 0.25;
        if planet.holds_at(next) {
            let at = p - up * 0.1;
            return (!orbit_core::block::is_liquid(planet.block_at(at))
                && !orbit_core::block::is_liquid(planet.block_at(at - up * 0.3)))
            .then_some(at);
        }
        p = next;
    }
    None
}

/// Ground to put a mob on about `dist` blocks ahead of `pos` (looking along `forward`): that spot
/// if it is dry and open, else the nearest of a fan of others (turned aside, nearer, farther).
pub fn ground_ahead(planet: &Planet, pos: Vec3, forward: Vec3, dist: f32) -> Option<Vec3> {
    let up = pos.normalize_or(Vec3::Y);
    for (turn, scale) in [
        (0.0f32, 1.0f32),
        (0.4, 1.0),
        (-0.4, 1.0),
        (0.0, 0.75),
        (0.0, 1.25),
        (0.8, 1.0),
        (-0.8, 1.0),
        (1.4, 0.75),
        (-1.4, 0.75),
        (std::f32::consts::PI, 0.75),
    ] {
        let dir = Quat::from_axis_angle(up, turn) * forward;
        if let Some(at) = ground_near(planet, pos + dir * dist * scale) {
            return Some(at);
        }
    }
    None
}

/// The ground under a direction (planet frame), found by coming down from radius `top` to `bottom`:
/// the spot just over the first solid block, if it is dry.
pub fn ground_from_above(planet: &Planet, dir: Vec3, top: f32, bottom: f32) -> Option<Vec3> {
    let dir = dir.normalize_or(Vec3::Y);
    let mut r = top;
    while r > bottom {
        if planet.holds_at(dir * (r - 0.25)) {
            let at = dir * (r - 0.1);
            return (!orbit_core::block::is_liquid(planet.block_at(at))
                && !orbit_core::block::is_liquid(planet.block_at(at - dir * 0.3))
                && !planet.holds_at(at + dir * 1.0))
            .then_some(at);
        }
        r -= 0.25;
    }
    None
}

/// The player's feet if they are on this planet and able to be hurt or talked to.
fn player_on(player: &Player, planet: u64) -> Option<Vec3> {
    (player.body.gravity_source == Some(planet)
        && player.body.station.is_none()
        && player.body.cabin.is_none())
    .then_some(player.body.pos)
}

/// `/summon`: a mob a few blocks ahead of the player, on the ground.
fn summon(
    mut mobs: ResMut<Mobs>,
    player: Single<&Player>,
    space: Res<Space>,
    mut bar: ResMut<ActionBar>,
) {
    if mobs.requests.is_empty() {
        return;
    }
    let requests = std::mem::take(&mut mobs.requests);
    let Some(id) = player.body.gravity_source else {
        bar.show("Mobs can only be summoned on a planet's surface");
        return;
    };
    let Some(voxels) = space.planets.get(&id).and_then(|p| p.voxel.clone()) else {
        return;
    };
    let planet = voxels.read().unwrap();
    let body = &player.body;
    for Request { kind, at } in requests {
        if kind == Kind::SlimeKing && mobs.king_alive() {
            bar.show("El Rey Slime ya está aquí");
            continue;
        }
        let ahead = if kind == Kind::SlimeKing { 8.0 } else { 3.0 };
        let Some(at) = at.or_else(|| ground_ahead(&planet, body.pos, body.forward, ahead)) else {
            bar.show("No hay espacio para invocarlo aquí");
            continue;
        };
        let mut mob = Mob::new(mobs.new_id(), kind, at, at.normalize_or(Vec3::Y));
        mob.face = -body.forward; // looks back at the player
        mob.home = at;
        mobs.add(id, mob);
    }
}

/// Wild slimes: now and then one appears on the meadow near the player (survival only, capped).
fn spawn_wild(mut mobs: ResMut<Mobs>, player: Single<&Player>, space: Res<Space>, time: Res<Time>) {
    if player.inv.mode != GameMode::Survival {
        return;
    }
    let Some(id) = player.body.gravity_source else {
        return;
    };
    let Some(slot) = space.planets.get(&id) else {
        return;
    };
    if slot.spec.is_moon() || slot.spec.biome != Biome::Meadow || player_on(&player, id).is_none() {
        return;
    }
    let alive = mobs
        .list
        .iter()
        .filter(|s| s.planet == id && s.mob.kind == Kind::Slime)
        .count();
    let Some(at) = mobs.spawner.tick(time.delta_secs(), alive) else {
        return;
    };
    let Some(voxels) = slot.voxel.as_ref() else {
        return;
    };
    let body = &player.body;
    let dir = Quat::from_axis_angle(body.up, at.angle) * body.forward;
    if let Some(ground) = ground_near(&voxels.read().unwrap(), body.pos + dir * at.dist) {
        let mob = Mob::new(at.id, Kind::Slime, ground, ground.normalize_or(Vec3::Y));
        mobs.add(id, mob);
    }
}

/// Gallinas salvajes: aparecen en praderas y caminan como criaturas pasivas.
fn spawn_chickens(
    mut mobs: ResMut<Mobs>,
    player: Single<&Player>,
    space: Res<Space>,
    time: Res<Time>,
) {
    mobs.chicken_timer -= time.delta_secs();
    if player.inv.mode != GameMode::Survival || mobs.chicken_timer > 0.0 {
        return;
    }
    let Some(id) = player.body.gravity_source else {
        return;
    };
    let Some(slot) = space.planets.get(&id) else {
        return;
    };
    if slot.spec.is_moon() || slot.spec.biome != Biome::Meadow || player_on(&player, id).is_none() {
        return;
    }
    let alive = mobs
        .list
        .iter()
        .filter(|s| s.planet == id && s.mob.kind == Kind::Chicken)
        .count();
    mobs.chicken_timer = 12.0;
    if alive >= 8 {
        return;
    }
    let dir = Quat::from_axis_angle(player.body.up, mobs.chicken_timer) * player.body.forward;
    if let Some(at) = slot
        .voxel
        .as_ref()
        .and_then(|v| ground_near(&v.read().unwrap(), player.body.pos + dir * 10.0))
    {
        let mob = Mob::new(mobs.new_id(), Kind::Chicken, at, at.normalize_or(Vec3::Y));
        mobs.add(id, mob);
    }
}

/// What is under the crosshair; a left click hits it. Runs before the block clicks (`interact`),
/// which leave a click alone while a mob is aimed at.
#[allow(clippy::too_many_arguments)]
pub(crate) fn aim(
    mut mobs: ResMut<Mobs>,
    mut player: Single<&mut Player>,
    space: Res<Space>,
    mouse: Res<ButtonInput<MouseButton>>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    keyboard: Res<Keyboard>,
    map: Res<GalaxyMap>,
    rockets: Res<crate::rocket::Rockets>,
    elevators: Res<crate::elevator::Elevators>,
    mut drops: ResMut<Drops>,
    mut talk: ResMut<crate::dialogue::Talk>,
    mut story: ResMut<crate::story::Story>,
    mut bar: ResMut<ActionBar>,
) {
    mobs.aimed = None;
    mobs.aimed_npc = false;
    if keyboard.owner.is_some()
        || map.is_open()
        || rockets.is_piloting()
        || elevators.placing.is_some()
        || cursor.grab_mode == CursorGrabMode::None
    {
        return;
    }
    let Some(id) = player.body.gravity_source else {
        return;
    };
    let Some(voxels) = space.planets.get(&id).and_then(|p| p.voxel.as_ref()) else {
        return;
    };
    let (eye, look) = (player.body.eye(), player.body.look_dir());
    let block_t = voxels
        .read()
        .unwrap()
        .raycast(eye, look, REACH)
        .map_or(f32::MAX, |h| h.point.distance(eye));
    let Some((i, t)) = mobs
        .list
        .iter()
        .enumerate()
        .filter(|(_, s)| s.planet == id)
        .filter_map(|(i, s)| s.mob.ray(eye, look, REACH).map(|t| (i, t)))
        .min_by(|a, b| a.1.total_cmp(&b.1))
    else {
        return;
    };
    if t >= block_t {
        return;
    }
    let (aimed, kind) = (mobs.list[i].mob.id, mobs.list[i].mob.kind);
    mobs.aimed = Some(aimed);
    mobs.aimed_npc = kind.is_npc();
    // A right click on a townsperson talks to them.
    if mouse.just_pressed(MouseButton::Right) {
        if let Some(npc) = orbit_core::dialogue::Npc::of(kind) {
            talk.request = Some((npc, aimed));
        }
        return;
    }
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }
    let held = player.inv.held().map(|s| s.item);
    let (damage, sword) = (mob::player_damage(held), mob::is_sword(held));
    let down = -player.body.pos.normalize_or(Vec3::Y);
    let from = player.body.pos;
    let slot = &mut mobs.list[i];
    let result = slot.mob.hit(damage, from, down);
    if result == Hit::Ignored {
        return;
    }
    if player.inv.mode == GameMode::Survival {
        player.inv.wear_held();
    }
    info!(
        "mobs: hit {} #{:x} for {damage} (hp {:.0}/{:.0}) {result:?}",
        slot.mob.kind.name(),
        slot.mob.id & 0xFFFF,
        slot.mob.hp.max(0.0),
        slot.mob.kind.max_hp()
    );
    if result == Hit::Killed {
        let (at, up) = (
            slot.mob.pos + slot.mob.pos.normalize_or(Vec3::Y) * 0.4,
            slot.mob.pos.normalize_or(Vec3::Y),
        );
        let drops_all = slot.mob.drops_all(sword);
        let kind = slot.mob.kind;
        mobs.list.remove(i);
        mobs.kills += 1;
        if kind == Kind::SlimeKing {
            story.flags.boss_defeated = true;
            story.dirty = true;
            bar.show("You defeated the Slime King!");
            info!("mobs: the Slime King has fallen");
        }
        for stack in drops_all {
            info!(
                "mobs: {} dropped {} x {}",
                kind.name(),
                stack.count,
                orbit_core::items::name(stack.item)
            );
            // One ball per drop, so each pops out on its own.
            for _ in 0..stack.count {
                drops.pop(Frame::Planet(id), at, up, stack.item);
            }
        }
    }
}

/// Every mob thinks, falls, hops; touching hurts the player; the far or the lost go.
#[allow(clippy::too_many_arguments)]
pub(crate) fn run_mobs(
    mut mobs: ResMut<Mobs>,
    mut drops: ResMut<Drops>,
    player: Single<&Player>,
    space: Res<Space>,
    time: Res<Time>,
    mut vitals: ResMut<crate::health::Vitals>,
    mut bar: ResMut<ActionBar>,
) {
    let dt = time.delta_secs().min(0.05);
    let survival = player.inv.mode == GameMode::Survival;
    let mut gone = Vec::new();
    // What the Slime King asked for this step: small slimes round it.
    let mut summons: Vec<(u64, Vec3, u8)> = Vec::new();
    for (i, s) in mobs.list.iter_mut().enumerate() {
        let Some(voxels) = space.planets.get(&s.planet).and_then(|p| p.voxel.as_ref()) else {
            gone.push(i);
            continue;
        };
        let feet = player_on(&player, s.planet).filter(|_| !vitals.dead);
        if s.mob.kind == Kind::Slime && feet.is_some_and(|f| f.distance(s.mob.pos) > mob::DESPAWN) {
            gone.push(i);
            continue;
        }
        let planet = voxels.read().unwrap();
        let down = -s.mob.pos.normalize_or(Vec3::Y);
        let env = Env {
            down,
            solid: &|p| planet.holds_at(p),
            wet: &|p| orbit_core::block::is_liquid(planet.block_at(p)),
            player: feet,
        };
        for e in s.mob.step(&env, dt) {
            let name = s.mob.kind.name();
            match e {
                Event::Contact(d) if survival => {
                    let lost = vitals.hurt(d, DamageKind::Mob);
                    if lost > 0.0 {
                        info!(
                            "mobs: {name} touched the player: -{lost:.0} hp (now {:.0})",
                            vitals.health.hp
                        );
                    }
                }
                Event::Telegraph(a) => {
                    info!("mobs: {name} is about to {} (crouching, glowing)", a.name())
                }
                Event::Smash {
                    radius,
                    damage,
                    hits,
                    ..
                } => {
                    let lost = if hits && survival {
                        vitals.hurt(damage, DamageKind::Mob)
                    } else {
                        0.0
                    };
                    info!(
                        "mobs: {name} smashes the ground (radius {radius:.1}): player hit {hits}, -{lost:.0} hp (now {:.0})",
                        vitals.health.hp
                    );
                }
                Event::Summon(n) => {
                    info!("mobs: {name} summons {n} small slimes");
                    summons.push((s.planet, s.mob.pos, n));
                }
                Event::Enraged => {
                    info!("mobs: {name} enters phase 2 at {:.0} hp", s.mob.hp);
                    bar.show("¡El Rey Slime ruge!");
                }
                Event::LayEgg => {
                    let at = s.mob.pos + s.mob.pos.normalize_or(Vec3::Y) * 0.5;
                    drops.pop(
                        Frame::Planet(s.planet),
                        at,
                        at.normalize_or(Vec3::Y),
                        orbit_core::items::EGG,
                    );
                }
                _ => {}
            }
        }
    }
    for i in gone.into_iter().rev() {
        mobs.list.remove(i);
    }
    for (planet, at, n) in summons {
        // At most four small slimes at once.
        let have = mobs
            .list
            .iter()
            .filter(|s| s.mob.kind == Kind::Slime && s.mob.scale < 1.0)
            .count();
        for k in 0..(n as usize).min(4usize.saturating_sub(have)) {
            let up = at.normalize_or(Vec3::Y);
            let side =
                Quat::from_axis_angle(up, k as f32 * 2.1 + 0.7) * up.any_orthonormal_vector();
            let mut m = Mob::new(mobs.new_id(), Kind::Slime, at + side * 3.0 + up * 0.5, up);
            m.scale = orbit_core::boss::MINION_SCALE;
            m.hp = orbit_core::boss::MINION_HP;
            m.aggro = true;
            m.timer = 0.3;
            mobs.add(planet, m);
        }
    }
}

/// Models: one for each mob, a child of its planet's root, standing on the mob's feet.
#[allow(clippy::too_many_arguments)]
fn draw_mobs(
    mut commands: Commands,
    mut mobs: ResMut<Mobs>,
    space: Res<Space>,
    assets: Res<MobAssets>,
    mut tr: Query<&mut Transform>,
    models: Query<(Entity, &MobModel)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    server: Res<AssetServer>,
) {
    for (e, m) in &models {
        if !mobs.list.iter().any(|s| s.mob.id == m.0) {
            commands.entity(e).despawn();
        }
    }
    for s in &mut mobs.list {
        let Some(root) = space.planets.get(&s.planet).map(|p| p.root) else {
            continue;
        };
        if s.model.is_none() {
            if s.mob.kind.is_npc() || s.mob.kind == Kind::Chicken {
                let star = space
                    .planets
                    .get(&s.planet)
                    .map_or([1.0; 3], |p| p.star_color);
                let loaded = npc_asset(&assets, s.mob.kind).is_some_and(|(scene, _)| {
                    matches!(server.get_load_state(scene.id()), Some(bevy::asset::LoadState::Loaded))
                });
                if loaded {
                    let (scene, _) = npc_asset(&assets, s.mob.kind).unwrap();
                    s.model = Some(
                        commands
                            .spawn((
                                WorldAssetRoot(scene.clone()),
                                MobModel(s.mob.id),
                                Transform::default(),
                                Visibility::default(),
                                ChildOf(root),
                            ))
                            .id(),
                    );
                    s.fallback = false;
                } else if let Some(b) = crate::npc_models::build(
                    &mut commands,
                    &mut meshes,
                    &mut mats,
                    &server,
                    s.mob.kind,
                    root,
                    star,
                ) {
                    commands.entity(b.root).insert(MobModel(s.mob.id));
                    s.model = Some(b.root);
                    s.mats = b.mats;
                    s.fallback = true;
                }
            } else {
                s.model = Some(
                    commands
                        .spawn((
                            WorldAssetRoot(assets.scene.clone()),
                            MobModel(s.mob.id),
                            Transform::default(),
                            Visibility::default(),
                            ChildOf(root),
                        ))
                        .id(),
                );
                s.fallback = false;
            }
        } else if s.fallback && (s.mob.kind.is_npc() || s.mob.kind == Kind::Chicken) {
            let loaded = npc_asset(&assets, s.mob.kind).is_some_and(|(scene, _)| {
                matches!(server.get_load_state(scene.id()), Some(bevy::asset::LoadState::Loaded))
            });
            if loaded {
                commands.entity(s.model.unwrap()).despawn();
                s.model = None;
                s.mats.clear();
                s.anim = None;
                continue;
            }
        }
        let Some(model) = s.model else { continue };
        let up = s.mob.pos.normalize_or(Vec3::Y);
        let face = s.mob.face - up * s.mob.face.dot(up);
        if let Ok(mut t) = tr.get_mut(model) {
            let turn =
                Transform::default().looking_to(face.normalize_or(up.any_orthonormal_vector()), up);
            // The King squats before an attack.
            let squash = 1.0 - 0.3 * s.mob.boss.crouch;
            let model_scale = if s.fallback {
                1.0
            } else {
                real_model_scale(s.mob.kind)
            };
            *t = Transform {
                translation: s.mob.pos,
                rotation: turn.rotation,
                scale: Vec3::new(
                    s.mob.scale * model_scale * (2.0 - squash).min(1.15),
                    s.mob.scale * model_scale * squash,
                    s.mob.scale * model_scale * (2.0 - squash).min(1.15),
                ),
            };
        }
    }
}

/// A model is in: its materials swapped for lit ones, its animations hooked up.
#[allow(clippy::too_many_arguments)]
fn slime_ready(
    ev: On<WorldInstanceReady>,
    models: Query<&MobModel>,
    children: Query<&Children>,
    meshes: Query<&MeshMaterial3d<StandardMaterial>>,
    mut players: Query<&mut AnimationPlayer>,
    mut commands: Commands,
    standard: Res<Assets<StandardMaterial>>,
    mut mats: ResMut<Assets<ModelMaterial>>,
    space: Res<Space>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    gltfs: Res<Assets<Gltf>>,
    assets: Res<MobAssets>,
    mut mobs: ResMut<Mobs>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
) {
    let Ok(MobModel(id)) = models.get(ev.entity) else {
        return;
    };
    let Some(slot) = mobs.list.iter_mut().find(|s| s.mob.id == *id) else {
        return;
    };
    let star = space
        .planets
        .get(&slot.planet)
        .map_or([1.0; 3], |p| p.star_color);
    let mut copies: Vec<(AssetId<StandardMaterial>, Handle<ModelMaterial>)> = Vec::new();
    let mut player = None;
    slot.mats.clear();
    for e in children.iter_descendants(ev.entity) {
        if players.get(e).is_ok() && player.is_none() {
            player = Some(e);
        }
        let Ok(m) = meshes.get(e) else { continue };
        let handle = match copies.iter().find(|c| c.0 == m.0.id()) {
            Some(c) => c.1.clone(),
            None => {
                let Some(gltf) = standard.get(&m.0) else {
                    continue;
                };
                let mut look = ModelMaterial::new(star, gltf.base_color_texture.clone(), false);
                look.look.color = gltf.base_color.to_linear().to_vec4();
                let base = look.look.color;
                let h = mats.add(look);
                slot.mats.push((h.clone(), base));
                copies.push((m.0.id(), h.clone()));
                h
            }
        };
        commands
            .entity(e)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert(MeshMaterial3d(handle));
    }
    if slot.mob.kind == Kind::SlimeKing {
        // placeholder: replace with the real model (a big gold crown of boxes)
        let extra = crate::npc_models::king_crown(
            &mut commands,
            &mut mesh_assets,
            &mut mats,
            star,
            ev.entity,
        );
        slot.mats.extend(extra);
    }
    let Some(player) = player else { return };
    let Some((_, gltf_handle)) = npc_asset(&assets, slot.mob.kind) else {
        // Slime is the original model; NPCs and chickens use their own GLTF.
        let Some(gltf) = gltfs.get(&assets.gltf) else {
            return;
        };
        let (Some(idle), Some(bounce)) = (
            gltf.named_animations.get("animation.idle"),
            gltf.named_animations.get("animation.bounce"),
        ) else {
            return;
        };
        let (graph, idx) = AnimationGraph::from_clips([idle.clone(), bounce.clone()]);
        commands
            .entity(player)
            .insert(AnimationGraphHandle(graphs.add(graph)));
        if let Ok(mut p) = players.get_mut(player) {
            p.play(idx[0]).repeat();
        }
        slot.anim = Some((player, idx[0], idx[1]));
        slot.last_ground = true;
        return;
    };
    let Some(gltf) = gltfs.get(gltf_handle) else {
        return;
    };
    let Some(idle) = gltf
        .named_animations
        .get("animation.idle")
        .or_else(|| gltf.named_animations.get("idle"))
    else {
        return;
    };
    let Some(motion) = gltf
        .named_animations
        .get("animation.walk")
        .or_else(|| gltf.named_animations.get("animation.bounce"))
        .or_else(|| gltf.named_animations.get("walk"))
    else {
        return;
    };
    let (graph, idx) = AnimationGraph::from_clips([idle.clone(), motion.clone()]);
    commands
        .entity(player)
        .insert(AnimationGraphHandle(graphs.add(graph)));
    if let Ok(mut p) = players.get_mut(player) {
        p.play(idx[0]).repeat();
    }
    slot.anim = Some((player, idx[0], idx[1]));
    slot.last_ground = true;
}

/// Light on the models, as on the blocks around them; a mob just hit flashes red.
fn light_mobs(
    mobs: Res<Mobs>,
    space: Res<Space>,
    origin: Res<Origin>,
    mut mats: ResMut<Assets<ModelMaterial>>,
) {
    for s in &mobs.list {
        let Some(p) = space.planets.get(&s.planet) else {
            continue;
        };
        let star = origin.render(p.spec.star());
        let bodies: Vec<(Vec3, f32)> = space
            .eclipsers(s.planet)
            .into_iter()
            .map(|(c, r)| (origin.render(c), r))
            .collect();
        let flash = (s.mob.hit_cd / mob::HIT_COOLDOWN).clamp(0.0, 1.0);
        for (h, base) in &s.mats {
            if let Some(mut m) = mats.get_mut(h) {
                m.place(
                    origin.render(p.spec.center),
                    p.spec.radius,
                    star,
                    p.star_radius,
                    &bodies,
                );
                let k = 1.0 - 0.65 * flash;
                // The King shines gold (past white: it blooms) as an attack comes.
                let glow = s.mob.boss.glow;
                m.look.color = Vec4::new(
                    base.x * (1.0 + 2.5 * glow),
                    base.y * k * (1.0 + 2.0 * glow),
                    base.z * k * (1.0 + 0.2 * glow),
                    base.w,
                );
            }
        }
    }
}

/// Idle while on the ground; the bounce clip plays from each hop.
fn animate_mobs(mut mobs: ResMut<Mobs>, mut players: Query<&mut AnimationPlayer>) {
    for s in &mut mobs.list {
        let Some((e, idle, bounce)) = s.anim else {
            continue;
        };
        if s.mob.on_ground == s.last_ground {
            continue;
        }
        s.last_ground = s.mob.on_ground;
        let Ok(mut p) = players.get_mut(e) else {
            continue;
        };
        p.stop_all();
        if s.mob.on_ground {
            p.play(idle).repeat();
        } else {
            p.play(bounce);
        }
    }
}

/// How near the player has to be for the King's bar to show.
const BAR_RANGE: f32 = 40.0;
const BAR_W: f32 = 520.0;

#[derive(Component)]
struct BossBar;
#[derive(Component)]
struct BossFill;

/// The King's life bar, top and middle of the screen (hidden while there is no King near).
fn spawn_boss_bar(mut commands: Commands) {
    commands
        .spawn((
            BossBar,
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                top: px(14),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: px(4),
                ..default()
            },
            Visibility::Hidden,
            GlobalZIndex(8),
        ))
        .with_children(|r| {
            r.spawn((
                Text::new("Slime King"),
                TextFont {
                    font_size: FontSize::Px(22.0),
                    ..default()
                },
                TextColor(Color::WHITE),
                TextShadow::default(),
            ));
            r.spawn((
                Node {
                    width: px(BAR_W),
                    height: px(16),
                    border: UiRect::all(px(2)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.08, 0.02, 0.02, 0.85)),
                BorderColor::all(Color::srgb(0.1, 0.1, 0.1)),
            ))
            .with_child((
                BossFill,
                Node {
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.75, 0.15, 0.15)),
            ));
        });
}

fn boss_bar(
    mobs: Res<Mobs>,
    player: Single<&Player>,
    mut bar: Single<&mut Visibility, With<BossBar>>,
    mut fill: Single<(&mut Node, &mut BackgroundColor), With<BossFill>>,
) {
    let king = mobs
        .list
        .iter()
        .find(|s| s.mob.kind == Kind::SlimeKing && !s.mob.dead());
    let near = king.filter(|s| {
        player_on(&player, s.planet).is_some_and(|f| f.distance(s.mob.pos) < BAR_RANGE)
    });
    let want = if near.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if **bar != want {
        **bar = want;
    }
    if let Some(s) = near {
        fill.0.width = percent((s.mob.hp / s.mob.kind.max_hp() * 100.0).clamp(0.0, 100.0));
        fill.1.0 = if s.mob.boss.phase == 2 {
            Color::srgb(0.95, 0.45, 0.1)
        } else {
            Color::srgb(0.75, 0.15, 0.15)
        };
    }
}
