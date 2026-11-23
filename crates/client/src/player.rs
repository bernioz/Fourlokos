//! Local player: input → orbit_core physics, first-person camera, block breaking/placing, HUD.

use crate::chat::Keyboard;
use crate::map::GalaxyMap;
use crate::menu::PlanetMenu;
use crate::station::{StationMenu, Stations};
use crate::travel::{Pilot, Request, Travel, relative};
use crate::world::{Origin, Space, located, remesh};
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block;
use orbit_core::inventory::{GameMode, Hand, Inventory};
use orbit_core::player::{Body, Input, REACH, collides};
use orbit_core::shape::{self, Against, Around};
use orbit_core::travel::{elements, predict};

const SENSITIVITY: f32 = 0.0025;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, (spawn_player, spawn_hud))
            .add_systems(
                Update,
                (
                    grab_cursor,
                    move_player,
                    sync_camera,
                    interact,
                    update_hud,
                    tint_view,
                    draw_trajectory,
                )
                    .chain()
                    .after(crate::world::MovePlanets),
            );
    }
}

#[derive(Component)]
pub struct Player {
    pub body: Body,
    pub inv: Inventory,
}

#[derive(Component)]
pub struct HudText;

/// A wash over the whole view while your eyes are under water (blue) or in lava (orange).
#[derive(Component)]
struct Submerged;

fn spawn_player(mut commands: Commands, space: Res<Space>) {
    let (mut home, mut at) = space.universe.spawn();
    // Debug: ORBIT_SECTOR=x,y,z starts on the first planet of the first system from that sector
    // on (along +x): for trying the far reaches of the universe.
    let far: Option<Vec<i32>> = std::env::var("ORBIT_SECTOR")
        .ok()
        .map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect());
    if let Some(&[x, y, z]) = far.as_deref() {
        let sys = (0..)
            .find_map(|i| space.universe.system(IVec3::new(x + i, y, z)))
            .unwrap();
        home = sys.planets[0].clone();
        at = home.to_world(Vec3::Y * (home.ceiling() - 4.0));
    }
    // With ORBIT_TIME the home planet has moved on by the first frame: spawn over where it will be.
    if let Some(t) = std::env::var("ORBIT_TIME")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
    {
        let local = home.to_local(at);
        home.advance(t);
        at = home.to_world(local);
    }
    let mut body = Body::new(at, (at - home.center).as_vec3());
    // Debug: ORBIT_VIEW=<planet index> starts on that planet's night side, sun behind it.
    if let Some(i) = std::env::var("ORBIT_VIEW")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        let sys = space.universe.system(IVec3::ZERO).unwrap();
        if let Some(p) = sys.planets.get(i) {
            let away = (p.center - sys.center).normalize().as_vec3();
            body = Body::new(
                p.center + (away * (p.gravity_radius() + 300.0)).as_dvec3(),
                away.any_orthonormal_vector(),
            );
            body.forward = -away; // facing the planet, its star behind it
        }
    }
    commands.spawn(Player {
        body,
        inv: Inventory::default(),
    });
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection {
            fov: 75f32.to_radians(),
            near: 0.05,
            far: 1_000_000.0,
            ..default()
        }),
        crate::sky::SkyCamera,
        // The HUD and menus draw here; the galaxy map takes this over while it is open.
        IsDefaultUiCamera,
        bevy::camera::Hdr,
        bevy::post_process::bloom::Bloom::NATURAL,
        Transform::default(),
    ));
}

fn spawn_hud(mut commands: Commands) {
    commands.spawn((
        Submerged,
        Node {
            position_type: PositionType::Absolute,
            left: px(0),
            top: px(0),
            width: percent(100),
            height: percent(100),
            ..default()
        },
        BackgroundColor(Color::NONE),
        bevy::ui::FocusPolicy::Pass,
        GlobalZIndex(-1),
    ));
    commands.spawn((
        Text::new("+"),
        TextFont {
            font_size: FontSize::Px(22.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            left: percent(50),
            top: percent(50),
            margin: UiRect::new(px(-6), px(0), px(-14), px(0)),
            ..default()
        },
    ));
    commands.spawn((
        HudText,
        Text::default(),
        TextFont {
            font_size: FontSize::Px(15.0),
            ..default()
        },
        Node {
            position_type: PositionType::Absolute,
            left: px(10),
            top: px(8),
            ..default()
        },
    ));
}

fn grab_cursor(
    mut cursor: Single<&mut CursorOptions, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    menu: Res<PlanetMenu>,
    map: Res<GalaxyMap>,
    keyboard: Res<Keyboard>,
    station_menu: Res<StationMenu>,
) {
    // With a menu or the chat open, clicks belong to it.
    let ui = menu.is_open() || map.is_open() || keyboard.owner.is_some() || station_menu.is_open();
    if mouse.just_pressed(MouseButton::Left) && cursor.grab_mode == CursorGrabMode::None && !ui {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn move_player(
    mut player: Single<&mut Player>,
    space: Res<Space>,
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    menu: Res<PlanetMenu>,
    map: Res<GalaxyMap>,
    pilot: Res<Pilot>,
    mut travel: ResMut<Travel>,
    stations: Res<Stations>,
    clock: Res<crate::world::Clock>,
    keyboard: Res<Keyboard>,
    riding: Res<crate::elevator::Riding>,
    rockets: Res<crate::rocket::Rockets>,
    settings: Res<crate::settings::Settings>,
) {
    // Flying is for creative; in survival you come down (the jetpack stays: it is your ship).
    if player.inv.mode == GameMode::Survival {
        player.body.flying = false;
    }
    let creative = player.inv.mode == GameMode::Creative;
    let body = &mut player.body;
    // At a rocket's helm the body is the rocket's (see `rocket`).
    let piloting = rockets.is_piloting();
    let ui = menu.is_open() || map.is_open() || keyboard.owner.is_some();
    if cursor.grab_mode != CursorGrabMode::None && !pilot.frozen {
        body.turn(
            motion.delta.x * SENSITIVITY * settings.sensitivity,
            motion.delta.y * SENSITIVITY * settings.sensitivity,
        );
    }
    if !ui {
        if keys.just_pressed(KeyCode::KeyH) && !piloting && creative {
            body.flying = !body.flying;
            body.jetpack = false;
        }
        if keys.just_pressed(KeyCode::KeyJ) && !piloting {
            body.jetpack = !body.jetpack;
            body.flying = false;
        }
        if keys.just_pressed(KeyCode::KeyL) {
            travel.requests.push(Request::Land);
        }
        if keys.just_pressed(KeyCode::KeyK) {
            travel.requests.push(Request::Launch);
        }
    }
    let axis = |pos, neg| keys.pressed(pos) as i32 as f32 - keys.pressed(neg) as i32 as f32;
    let mut input = Input {
        movement: Vec2::new(
            axis(KeyCode::KeyD, KeyCode::KeyA),
            axis(KeyCode::KeyW, KeyCode::KeyS),
        ),
        jump: keys.pressed(KeyCode::Space),
        descend: keys.pressed(KeyCode::ShiftLeft),
        sprint: keys.pressed(KeyCode::ControlLeft),
        brake: keys.pressed(KeyCode::KeyX),
        thrust: None,
    };
    if ui || pilot.hands_off {
        input = Input::default(); // keys are for the menu or the autopilot; gravity still pulls
    }
    input.thrust = pilot.thrust;
    // In an elevator's cabin: it carries you. If the elevator is gone, you let go, as you were.
    if body.cabin.is_some() {
        match riding.0.as_ref() {
            Some(cabin) => body.ride(cabin, time.delta_secs().min(0.1)),
            None => {
                let (at, vel, up, fwd) = (
                    body.world_point(body.pos),
                    body.world_vel(),
                    body.world_dir(body.up),
                    body.world_dir(body.forward),
                );
                let pitch = body.pitch;
                body.place(at, vel, up, fwd, pitch);
            }
        }
        return;
    }
    if piloting {
        // A rocket hovers on its engines when flown by hand, and sits still on the ground; the
        // autopilot flies it against gravity itself. It never lands on stations (or on itself).
        body.flying = false;
        body.jetpack = travel.job.is_none();
        if body.on_ground && !input.jump {
            input.movement = Vec2::ZERO;
        }
    }
    // On a planet the body lives in its frame and moves with it; only the frame needs updating.
    let voxels = space.voxels();
    let planets = located(&voxels);
    let wells = space.wells();
    let placed = stations.placed(&space, clock.0);
    let on = if piloting {
        vec![]
    } else {
        crate::station::located(&placed)
    };
    body.sync_frame(&planets, &wells, &on);
    if pilot.frozen {
        return;
    }
    if space.waiting_at(body.world_point(body.pos)) {
        body.vel = Vec3::ZERO; // hold still until the planet under us is generated
        return;
    }
    // Fixed-size substeps keep collisions stable at low frame rates.
    let mut dt = time.delta_secs().min(0.1);
    while dt > 0.0 {
        let h = dt.min(1.0 / 60.0);
        body.step_with(input, &planets, &wells, &on, h);
        dt -= h;
    }
}

/// Sets the frame's floating origin (see `Origin`) and puts the camera at the eye, in render space.
pub(crate) fn sync_camera(
    player: Single<&Player>,
    mut origin: ResMut<Origin>,
    mut cam: Single<&mut Transform, With<crate::sky::SkyCamera>>,
    mut rockets: ResMut<crate::rocket::Rockets>,
    stations: Res<Stations>,
    elevators: Res<crate::elevator::Elevators>,
    riding: Res<crate::elevator::Riding>,
) {
    let b = &player.body;
    // Riding an elevator, the outside view: from out there, looking at the cabin.
    if let Some((eye, up, mid)) =
        crate::elevator::outside_eye(&elevators, &riding, b.world_dir(b.look_dir()))
    {
        let at = if b.planet().is_some() {
            b.frame_center()
        } else {
            eye
        };
        *origin = Origin { at, eye };
        **cam = Transform::from_translation(origin.render(eye))
            .looking_to((mid - eye).as_vec3().normalize_or(Vec3::Z), up);
        return;
    }
    // At a rocket's helm: from your eyes inside it, or from behind it, looking where it looks.
    if let Some(pil) = rockets.piloting.as_ref() {
        let flight = stations
            .by_id(&pil.id)
            .and_then(|s| s.station.read().unwrap().flight);
        if let Some(f) = flight {
            let look = b.world_dir(b.look_dir());
            // The camera stays level with the ground (or your up in space) while the rocket tips.
            let up = b.world_dir(b.up);
            let (eye, dir, cam_up, reach) = if rockets.cockpit {
                let p = &pil.person;
                (
                    p.world_point(p.eye()),
                    p.world_dir(p.look_dir()),
                    p.world_dir(p.up),
                    0.0,
                )
            } else {
                // Well back and a little over it, aiming above it: the rocket sits low in the
                // view and leaves most of it for where it is going.
                let mid = f.center + (f.rotation * pil.middle).as_dvec3();
                let back = (pil.size * 5.0 + 14.0) * rockets.zoom;
                let eye = mid - (look * back).as_dvec3()
                    + (up * (pil.size * 0.8 + 3.0) * rockets.zoom.sqrt()).as_dvec3();
                let aim = mid + (up * pil.size * 0.9).as_dvec3();
                (
                    eye,
                    (aim - eye).as_vec3().normalize_or(look),
                    up,
                    (mid - eye).length() as f32 + pil.size * 1.8,
                )
            };
            let at = if b.planet().is_some() {
                b.frame_center()
            } else {
                eye
            };
            *origin = Origin { at, eye };
            **cam = Transform::from_translation(origin.render(eye)).looking_to(dir, cam_up);
            rockets.reach = reach;
            return;
        }
    }
    let eye = b.world_point(b.eye());
    // On or around a planet: its center, so the planet draws at exactly zero. In space: the eye.
    let at = if b.planet().is_some() {
        b.frame_center()
    } else {
        eye
    };
    *origin = Origin { at, eye };
    **cam = Transform::from_translation(b.eye_from(at))
        .looking_to(b.world_dir(b.look_dir()), b.world_dir(b.up));
}

/// What a click can touch besides the world, bundled (a system takes only so many parameters).
#[derive(bevy::ecs::system::SystemParam)]
pub(crate) struct Acts<'w> {
    pub drops: ResMut<'w, crate::inventory::Drops>,
    pub mining: ResMut<'w, crate::mining::Mining>,
    pub screen: ResMut<'w, crate::inventory::Screen>,
    pub vitals: ResMut<'w, crate::health::Vitals>,
    pub mobs: ResMut<'w, crate::mobs::Mobs>,
}

impl Acts<'_> {
    /// A right click eats: the first hand holding food, if hurt (see `Inventory::eat`).
    pub fn eat(&mut self, player: &mut Player, bar: &mut crate::chat::ActionBar) -> bool {
        if self.vitals.dead {
            return false;
        }
        for hand in Hand::ORDER {
            if player.inv.eat(hand, &mut self.vitals.health) {
                bar.show(&format!(
                    "Ate: {:.0}/{:.0} HP",
                    self.vitals.health.hp, self.vitals.health.max
                ));
                return true;
            }
        }
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn interact(
    mut player: Single<&mut Player>,
    mut space: ResMut<Space>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Single<&CursorOptions, With<PrimaryWindow>>,
    elevators: Res<crate::elevator::Elevators>,
    origin: Res<Origin>,
    keyboard: Res<Keyboard>,
    map: Res<GalaxyMap>,
    mut gizmos: Gizmos,
    mut net: ResMut<crate::netplay::NetRole>,
    mut netplay: ResMut<crate::netplay::NetPlay>,
    mut rockets: ResMut<crate::rocket::Rockets>,
    mut bar: ResMut<crate::chat::ActionBar>,
    mut acts: Acts,
) {
    if keyboard.owner.is_some() || map.is_open() {
        return; // the wheel scrolls the chat, or zooms the map
    }
    let n = orbit_core::inventory::HOTBAR;
    for (i, key) in [
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ]
    .into_iter()
    .enumerate()
    {
        if keys.just_pressed(key) {
            player.inv.selected = i;
        }
    }
    // F: the left hand and the one in hand trade what they hold.
    if keys.just_pressed(KeyCode::KeyF)
        && cursor.grab_mode != CursorGrabMode::None
        && !rockets.is_piloting()
    {
        player.inv.swap_hands();
    }
    // At a rocket's helm, in the chase view, the wheel moves the camera (see `rocket`).
    // Riding an elevator, in the outside view, it zooms (see `elevator`).
    let zooming = (rockets.is_piloting() && !rockets.cockpit)
        || (player.body.cabin.is_some() && elevators.outside);
    if scroll.delta.y != 0.0 && !zooming {
        player.inv.selected = (player.inv.selected as isize - scroll.delta.y.signum() as isize)
            .rem_euclid(n as isize) as usize;
    }

    // On a station, `station::interact_station` does this; at a helm, the rocket's body is yours;
    // while an elevator's hologram is out, the clicks are for it (`elevator::run_placing`).
    if rockets.is_piloting() || elevators.placing.is_some() {
        return;
    }
    let Some(id) = player.body.gravity_source else {
        return;
    };
    let Some(slot) = space.planets.get_mut(&id) else {
        return;
    };
    let Some(voxels) = slot.voxel.clone() else {
        return;
    };
    let spec = slot.spec.clone(); // where the planet is now
    // The body is in this planet's frame: eye and look are already planet-local.
    let (eye, look) = (player.body.eye(), player.body.look_dir());
    let planet = voxels.read().unwrap();
    let Some(hit) = planet.raycast(eye, look, REACH) else {
        if mouse.just_pressed(MouseButton::Right) && cursor.grab_mode != CursorGrabMode::None {
            acts.eat(&mut player, &mut bar);
        }
        return;
    };

    // Outline what is looked at: its own shape (a door's board, a torch, a flower), each box
    // grown a hair about its middle so it sits on the block, not inside it. In render space,
    // from the planet's local coordinates: exact (see `Origin`).
    let g = &planet.grid;
    let base = origin.render(spec.center);
    let shape_boxes = shape::outline(planet.get(hit.cell), shape::joined(&*planet, hit.cell));
    for x in shape_boxes.iter() {
        let mid = [
            (x[0] + x[3]) * 0.5,
            (x[1] + x[4]) * 0.5,
            (x[2] + x[5]) * 0.5,
        ];
        let grow = |a: f32, m: f32| m + (a - m) * 1.004 + (a - m).signum() * 0.002;
        let c = |u: f32, v: f32, w: f32| {
            base + spec.rotation
                * g.point(hit.cell, grow(u, mid[0]), grow(v, mid[1]), grow(w, mid[2]))
        };
        box_edges(&mut gizmos, |i, j, k| {
            c([x[0], x[3]][i], [x[1], x[4]][j], [x[2], x[5]][k])
        });
    }

    if cursor.grab_mode == CursorGrabMode::None {
        return;
    }
    let target = planet.get(hit.cell);
    if mouse.just_pressed(MouseButton::Middle) {
        player.inv.pick(target);
        return;
    }
    drop(planet);
    let survival = player.inv.mode == GameMode::Survival;
    let mut planet = voxels.write().unwrap();
    // Cosechar cultivos maduros con clic derecho los deja en la etapa inicial para que rebrote.
    if mouse.just_pressed(MouseButton::Right) && orbit_core::crop::mature(target) {
        let crop = orbit_core::crop::crop(target).unwrap();
        let first = crop.stages[0];
        let at = planet.grid.center(hit.cell);
        let dirty = planet.set(hit.cell, first);
        drop(planet);
        if survival {
            acts.drops.pop(
                crate::inventory::Frame::Planet(id),
                at,
                at.normalize_or(Vec3::Y),
                crop.harvest,
            );
        }
        bar.show(&format!(
            "Cosechaste {}",
            orbit_core::items::name(crop.harvest)
        ));
        slot.edited = true;
        crate::world::remesh(slot, &dirty);
        return;
    }
    // Survival: hold the click until the block gives (see `mining`); creative: one click.
    let tool = player.inv.held().map(|s| s.item);
    // A mob under the crosshair has the left click (`mobs::aim`).
    let digs = block::breakable(target)
        && acts.mobs.aimed.is_none()
        && if survival {
            mouse.pressed(MouseButton::Left)
                && acts.mining.advance(
                    (crate::inventory::Frame::Planet(id), [hit.cell as i64, 0, 0]),
                    orbit_core::mining::mining_time(target, tool),
                )
        } else {
            mouse.just_pressed(MouseButton::Left)
        };
    let (mut dirty, changed, was) = if digs {
        let (keys, gone) = planet.break_block(hit.cell);
        if survival {
            acts.drops.broke_on_planet(id, &planet.grid, &gone, tool);
            player.inv.wear_held();
        }
        (keys, hit.cell, target)
    } else if mouse.just_pressed(MouseButton::Right) && !acts.mobs.aimed_npc {
        // A helm: take it (the rocket is put together), unless crouching to build on it.
        let crouching = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
        if target == block::HELM && !crouching {
            rockets.requests.push(crate::rocket::Request::Assemble {
                planet: id,
                cell: hit.cell,
            });
            return;
        }
        // A crafting table opens its screen.
        if target == block::CRAFTING_TABLE && !crouching {
            acts.screen.request = Some(crate::inventory::Kind::Table);
            return;
        }
        // A furnace opens its screen.
        if target == block::FURNACE && !crouching {
            acts.screen
                .request_furnace((crate::inventory::Frame::Planet(id), [hit.cell as i64, 0, 0]));
            return;
        }
        // A Rocket Kit uses the same pad search and construction as `/rocket build`.
        // The rocket system consumes it only after the construction succeeds.
        if let Some(hand) = Hand::ORDER.into_iter().find(|&h| {
            player
                .inv
                .in_hand(h)
                .is_some_and(|s| s.item == orbit_core::items::ROCKET_KIT)
        }) {
            rockets
                .requests
                .push(crate::rocket::Request::BuildKit(hand));
            return;
        }
        // A door or trapdoor opens or shuts.
        if block::opens(target) && !crouching {
            let mut keys = Vec::new();
            for (c, b) in shape::toggle(&*planet, hit.cell) {
                keys.extend(planet.set(c, b));
            }
            drop(planet);
            remesh(slot, &keys);
            return;
        }
        // A Slime Crown Core, right clicked on a block, is spent to call the Slime King 8 blocks off.
        if let Some(hand) = Hand::ORDER.into_iter().find(|&h| {
            player
                .inv
                .in_hand(h)
                .is_some_and(|s| s.item == orbit_core::items::SLIME_CROWN_CORE)
        }) {
            if acts.mobs.king_alive() {
                bar.show("The Slime King is already here");
            } else if let Some(at) =
                crate::mobs::ground_ahead(&planet, player.body.pos, player.body.forward, 8.0)
            {
                player.inv.use_hand(hand);
                info!("mobs: a Slime Crown Core is spent: the Slime King is called at {at:?}");
                acts.mobs.requests.push(crate::mobs::Request {
                    kind: orbit_core::mob::Kind::SlimeKing,
                    at: Some(at),
                });
                bar.show("The ground trembles...");
            } else {
                info!("mobs: no ground found 8 blocks ahead to summon the Slime King");
                bar.show("No room to summon here");
            }
            return;
        }
        // Food in hand is eaten (unless there is a block to open above); else it goes on to place.
        if acts.eat(&mut player, &mut bar) {
            return;
        }
        let Some(at) = hit.before else { return };
        let against = against_planet(&planet, at, hit.cell);
        let face_dirs = block::Dir::ALL.map(|d| {
            planet.side(at, d).map_or(Vec3::ZERO, |nb| {
                (g_center(&planet, nb) - g_center(&planet, at)).normalize()
            })
        });
        let look_dir = block::Dir::ALL
            .into_iter()
            .max_by(|a, b| {
                face_dirs[a.0 as usize]
                    .dot(look)
                    .total_cmp(&face_dirs[b.0 as usize].dot(look))
            })
            .unwrap();
        let (_, _, _, kh) = planet.grid.coords(hit.cell);
        let high = hit.point.length() - planet.grid.radius(kh) > 0.5;
        // The main hand first; if it cannot place what it holds (or holds nothing), the left one.
        let mut placed = None;
        for hand in Hand::ORDER {
            let Some(held) = player.inv.in_hand(hand) else {
                continue;
            };
            let Some(puts) = shape::place(&*planet, at, held.item, against, look_dir, high) else {
                continue;
            };
            let before: Vec<_> = puts.iter().map(|&(c, _)| (c, planet.get(c))).collect();
            let mut keys = Vec::new();
            for &(c, b) in &puts {
                keys.extend(planet.set(c, b));
            }
            if collides(&planet, player.body.pos) {
                for (c, b) in before {
                    planet.set(c, b); // would trap the player
                }
                continue;
            }
            placed = Some((hand, keys));
            break;
        }
        let Some((hand, keys)) = placed else { return };
        player.inv.use_hand(hand);
        (keys, at, block::AIR)
    } else {
        return;
    };
    // Landing pads light up when they make a square, and go dark when they no longer do.
    let (more, shape) = orbit_core::pad::refresh_planet(&mut planet, changed);
    for k in more {
        if !dirty.contains(&k) {
            dirty.push(k);
        }
    }
    if let Some(shape) = shape.filter(|_| block::is_pad(was) || block::is_pad(planet.get(changed)))
    {
        bar.show(&shape.message());
    }
    let net_block = planet.get(changed) as u16;
    drop(planet);
    slot.edited = true;
    crate::netplay::record_local_edit(
        &mut *netplay,
        &mut *net,
        crate::net::Msg::BlockEdit {
            planet: id as u32,
            pos: [changed as i32, 0, 0],
            block: net_block,
        },
    );
    remesh(slot, &dirty);
}

/// The 12 edges of a box given its corners `at(i, j, k)` (each 0 or 1), in black.
pub(crate) fn box_edges(gizmos: &mut Gizmos, at: impl Fn(usize, usize, usize) -> Vec3) {
    for (a, b) in [
        ((0, 0), (1, 0)),
        ((1, 0), (1, 1)),
        ((1, 1), (0, 1)),
        ((0, 1), (0, 0)),
    ] {
        for k in 0..2 {
            gizmos.line(at(a.0, a.1, k), at(b.0, b.1, k), Color::BLACK);
        }
        gizmos.line(at(a.0, a.1, 0), at(a.0, a.1, 1), Color::BLACK);
    }
}

fn g_center(planet: &orbit_core::planet::Planet, c: usize) -> Vec3 {
    planet.grid.center(c)
}

/// Where the block clicked (`hit`) lies from the cell a block goes in (`at`).
fn against_planet(planet: &orbit_core::planet::Planet, at: usize, hit: usize) -> Against {
    if planet.below(at) == Some(hit) {
        return Against::Below;
    }
    if planet.above(at) == Some(hit) {
        return Against::Above;
    }
    match block::Dir::ALL
        .into_iter()
        .find(|&d| planet.side(at, d) == Some(hit))
    {
        Some(d) => Against::Side(d),
        None => Against::Below,
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn update_hud(
    player: Single<&Player>,
    space: Res<Space>,
    stations: Res<Stations>,
    travel: Res<Travel>,
    map: Res<GalaxyMap>,
    mut text: Single<&mut Text, With<HudText>>,
    time: Res<Time>,
    elevators: Res<crate::elevator::Elevators>,
    rockets: Res<crate::rocket::Rockets>,
    menu: Res<crate::start_menu::StartMenu>,
) {
    if map.is_open() || menu.is_open() {
        text.0.clear();
        return;
    }
    let b = &player.body;
    let slot = b.planet().and_then(|id| space.planets.get(&id));
    let on = b
        .station
        .as_ref()
        .and_then(|id| stations.by_id(id))
        .map(|s| {
            let s = s.station.read().unwrap();
            format!(
                "{} {} ({})  ",
                if s.is_rocket() { "rocket" } else { "station" },
                s.name,
                s.id
            )
        });
    let place = match slot {
        Some(p) => {
            // Sun elevation over the local horizon, degrees.
            let up = b.point_from(b.pos, p.spec.center);
            let sun = (p.spec.star() - p.spec.center).as_vec3().angle_between(up);
            let alt = up.length() - p.spec.radius;
            format!(
                "{}{} ({:?}, r {})  alt {:.0}  sun {:.0} deg",
                on.unwrap_or_default(),
                p.spec.name,
                p.spec.biome,
                p.spec.radius as i32,
                alt,
                90.0 - sun.to_degrees()
            )
        }
        None => "open space".into(),
    };
    let flying = rockets
        .piloting
        .as_ref()
        .map(|p| format!("rocket {}  ", p.name));
    let mode = if b.cabin.is_some() {
        "elevator"
    } else if travel.job.is_some() {
        "autopilot"
    } else if flying.is_some() {
        "at the helm"
    } else if b.station.is_some() && b.flying {
        "flying (station)"
    } else if b.station.is_some() && b.jetpack {
        "jetpack (station)"
    } else if b.station.is_some() {
        "walking (station)"
    } else if b.orbiting.is_some() {
        "orbit (thrusters)"
    } else if b.gravity_source.is_none() {
        "deep space (thrusters)"
    } else if b.flying {
        "flying"
    } else if b.jetpack {
        "jetpack"
    } else {
        "walking"
    };
    let mut orbit = String::new();
    if let (Some(p), Some(_)) = (slot, b.orbiting) {
        let (pos, vel) = relative(b, p.spec.center);
        let e = elements(pos, vel, p.spec.gravity_radius());
        let alt = |r: f32| {
            if r.is_finite() {
                format!("{:.0}", r - p.spec.radius)
            } else {
                "escape".into()
            }
        };
        orbit = format!(
            "\nperiapsis {}  apoapsis {}  ecc {:.2}",
            alt(e.periapsis),
            alt(e.apoapsis),
            e.eccentricity
        );
        if e.period.is_finite() {
            orbit += &format!("  period {:.0} s", e.period);
        }
    }
    let mut note = if travel.message.is_empty() {
        String::new()
    } else {
        format!("\n> {}", travel.message)
    };
    if !elevators.hud.is_empty() {
        note += &format!("\n> {}", elevators.hud);
    }
    if let Some(name) = flying {
        text.0 = format!(
            "{name}{place}\n{mode}  {:.1} m/s  {:.0} fps{orbit}{note}\nWASD/Space/Shift thrust  Ctrl boost  mouse camera  wheel zoom  X brake  V {} view\nK launch to a free orbit  L land (landing pad, station pad or flat 3x3)  M galaxy map (warp/fly)  R leave the helm",
            b.world_vel().length(),
            1.0 / time.delta_secs().max(1e-4),
            if rockets.cockpit { "chase" } else { "cockpit" },
        );
        return;
    }
    text.0 = format!(
        "{place}\n{mode}  {:.1} m/s  {:.0} fps  {}{orbit}{note}\nWASD move  Space/Shift up/down  Ctrl sprint/boost  X brake  H fly  J jetpack  F swap hands  LMB break  RMB place/open  E inventory  Q drop\nM galaxy map  K launch to orbit  L land  O orbits  G atmospheres  P customize planet\nT chat  / commands (/help, /station place, /rocket build, /elevator)",
        b.vel.length(),
        1.0 / time.delta_secs().max(1e-4),
        player.inv.mode.name().to_lowercase(),
    );
}

fn tint_view(
    player: Single<&Player>,
    rockets: Res<crate::rocket::Rockets>,
    mut wash: Single<&mut BackgroundColor, With<Submerged>>,
    mut fog: ResMut<crate::lighting::Submerged>,
) {
    let eyes = player.body.eyes_in.filter(|_| !rockets.is_piloting());
    fog.set_if_neq(crate::lighting::Submerged(eyes));
    let c = match eyes {
        Some(block::Liquid::Water) => Color::srgba(0.04, 0.18, 0.42, 0.22),
        Some(block::Liquid::Lava) => Color::srgba(0.95, 0.32, 0.04, 0.82),
        None => Color::NONE,
    };
    if wash.0 != c {
        wash.0 = c;
    }
}

/// In orbit, the path you will coast along (cyan), so you can see what a burn does.
fn draw_trajectory(
    player: Single<&Player>,
    space: Res<Space>,
    origin: Res<Origin>,
    mut gizmos: Gizmos,
) {
    let b = &player.body;
    let Some(p) = b.orbiting.and_then(|id| space.planets.get(&id)) else {
        return;
    };
    let (pos, vel) = relative(b, p.spec.center);
    let path = predict(pos, vel, p.spec.pull(), 0.25, 720);
    let c = origin.render(p.spec.center);
    gizmos.linestrip(
        path.into_iter().map(|q| c + q),
        Color::srgba(0.35, 0.9, 1.0, 0.35),
    );
}
