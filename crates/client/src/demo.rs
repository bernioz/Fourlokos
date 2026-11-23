//! Debug: `ORBIT_DEMO=warp` (or `fly`; `map` only opens the galaxy map once in orbit, `orbit` stays
//! there looking down, `stay` does nothing but the screenshots) plays a trip by itself: launch to orbit from the spawn,
//! then travel to the home system's second planet and land there. With `ORBIT_SHOTS=<dir>` it
//! saves a screenshot every `ORBIT_SHOT_EVERY` seconds (default 2) there, and quits after
//! `ORBIT_DEMO_SECS` (default 150). `ORBIT_DEMO_MOON=1` goes to the home planet's moon instead. `ORBIT_DEMO=station` types `/station place` in the chat once
//! in orbit, lands on the station, walks off its edge into orbit and comes back with `/station tp`.
//! `ORBIT_DEMO=elevator` goes to the equator, brings out an elevator's hologram (`/elevator
//! build`), builds it with a right click, walks into the cabin (to see it push back), rides up to
//! its new station (`ORBIT_DEMO_ELEVATOR_OUTSIDE=1`: in the outside view) and tries the cabin
//! there too.
//! `ORBIT_DEMO=rocket` builds the starter rocket (`/rocket build`), takes its helm, launches it
//! into a free lane, parks it, takes the helm again and lands it on a pad. With
//! `ORBIT_DEMO_ROCKET_WARP=1` it warps the rocket to the second planet from orbit instead, in the
//! chase view, and lands it there. `ORBIT_DEMO_ROCKET_ROOF=1` puts a block over it first and only
//! tries to take its helm, to see the block in the way marked. `ORBIT_DEMO_ROCKET_PAD=1` lays a
//! 9 × 9 landing pad first (the rocket goes up on it), and in orbit puts a station up with a 7 × 7
//! pad, lands the rocket on it and takes it up from there again. `ORBIT_DEMO=water` hovers by the
//! sea looking out over it, pours lava and then water on the shore (obsidian and cobblestone),
//! and dives under the sea. `ORBIT_DEMO=blocks` builds a door in a wall, fences, panes, torches
//! on walls, a ladder and trapdoors before you, drops a few items, then opens the inventory
//! (`ORBIT_DEMO_SURVIVAL=1`: in survival).

use crate::player::Player;
use crate::travel::{Mode, Request, Target, Travel};
use bevy::input::ButtonState;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKeyCode};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::PrimaryWindow;
use orbit_core::shape::Around;

pub struct DemoPlugin;

impl Plugin for DemoPlugin {
    fn build(&self, app: &mut App) {
        let Ok(mode) = std::env::var("ORBIT_DEMO") else {
            return;
        };
        let map = mode == "map";
        let look = mode == "orbit";
        let water = mode == "water";
        let blocks = mode == "blocks";
        let health = mode == "health" || mode == "menu";
        let hands = mode == "hands";
        let mining = mode == "mining";
        let items = mode == "items";
        let craft = mode == "craft";
        let furnace = mode == "furnace";
        let food = mode == "food";
        let held = mode == "held";
        let mobs = mode == "mobs";
        let npc = mode == "npc";
        let boss = mode == "boss";
        let moon = mode == "moon";
        let stay = mode == "stay"
            || water
            || blocks
            || health
            || hands
            || mining
            || items
            || craft
            || furnace
            || food
            || held
            || mobs
            || npc
            || boss;
        let elevator = mode == "elevator";
        let station = mode == "station" || elevator;
        let rocket = mode == "rocket";
        let mode = if mode == "fly" { Mode::Fly } else { Mode::Warp };
        app.insert_resource(Demo {
            mode,
            map,
            look,
            station,
            elevator,
            rocket,
            typing: Vec::new(),
            since: 0.0,
            step: if stay { 100 } else { 0 },
            wait: 0.0,
            shot: 0.0,
            n: 0,
            jitter: (0.0, 0.0),
            frames: 0,
        })
        .add_systems(
            Update,
            run.before(crate::map::toggle)
                .before(crate::elevator::run_elevators)
                .before(crate::elevator::run_placing),
        );
        if water {
            app.add_systems(
                Update,
                water_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            );
        }
        if hands {
            app.add_systems(
                Update,
                hands_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            );
        }
        if held {
            app.add_systems(
                Update,
                held_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            );
        }
        if food {
            app.add_systems(
                Update,
                food_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if mobs {
            app.add_systems(
                Update,
                mobs_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if boss {
            app.add_systems(
                Update,
                boss_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if moon {
            app.add_systems(
                Update,
                moon_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if npc {
            app.add_systems(
                Update,
                npc_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if furnace {
            app.add_systems(
                Update,
                furnace_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::inventory::open_screen),
            );
        }
        if craft {
            app.add_systems(
                Update,
                craft_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::inventory::open_screen),
            );
        }
        if items {
            app.add_systems(
                Update,
                items_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::inventory::open_screen),
            );
        }
        if mining {
            app.add_systems(
                Update,
                mining_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::player::interact),
            );
        }
        if health {
            app.add_systems(
                Update,
                health_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            );
        }
        if blocks {
            app.add_systems(
                Update,
                blocks_demo
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera)
                    .before(crate::inventory::open_screen),
            );
        }
    }
}

#[derive(Resource)]
struct Demo {
    mode: Mode,
    /// Only open the galaxy map once in orbit.
    map: bool,
    /// Only stay in orbit looking down at the planet.
    look: bool,
    step: u32,
    wait: f32,
    shot: f32,
    n: u32,
    jitter: (f32, f32),
    frames: u32,
    /// Play the station trip.
    station: bool,
    /// Play the elevator ride instead (it starts as the station trip does).
    elevator: bool,
    /// Play the rocket trip.
    rocket: bool,
    /// Keys still to type, one a frame.
    typing: Vec<Key>,
    /// When the current station step began.
    since: f32,
}

/// Types a line into the game, one key a frame, as a player would; `\t` is Tab, `\n` Enter.
fn type_keys(demo: &mut Demo, s: &str) {
    for c in s.chars() {
        demo.typing.push(match c {
            '\t' => Key::Tab,
            '\n' => Key::Enter,
            '\x1b' => Key::Escape,
            c => Key::Character(c.to_string().into()),
        });
    }
}

fn pad_demo() -> bool {
    std::env::var("ORBIT_DEMO_ROCKET_PAD").is_ok_and(|v| v == "1")
}

/// A 9 × 9 landing pad on the ground ahead, where `/rocket build` will look, with the air over it
/// cleared.
fn lay_pad(b: &orbit_core::player::Body, space: &mut crate::world::Space) {
    let Some(slot) = b.gravity_source.and_then(|id| space.planets.get_mut(&id)) else {
        return;
    };
    let Some(v) = slot.voxel.clone() else { return };
    let mut g = v.write().unwrap();
    let up = b.pos.normalize_or(Vec3::Y);
    let ahead = b
        .forward
        .reject_from(up)
        .normalize_or(up.any_orthonormal_vector());
    let Some(lay) = orbit_core::rocket::Lay::at(&g, b.pos + ahead * 9.0, ahead) else {
        return;
    };
    let Some(k) = orbit_core::rocket::ground(&g, lay.face, lay.i0, lay.j0) else {
        return;
    };
    let lay = orbit_core::rocket::Lay { k0: k, ..lay };
    let mut dirty = Vec::new();
    for x in -4..=4 {
        for z in -4..=4 {
            for y in 0..14 {
                if let Some(c) = lay.cell(&g, IVec3::new(x, y, z)) {
                    dirty.extend(g.set(
                        c,
                        if y == 0 {
                            orbit_core::block::PAD
                        } else {
                            orbit_core::block::AIR
                        },
                    ));
                }
            }
        }
    }
    let Some(c) = lay.cell(&g, IVec3::ZERO) else {
        return;
    };
    let (more, shape) = orbit_core::pad::refresh_planet(&mut g, c);
    dirty.extend(more);
    info!("demo: landing pad laid: {shape:?}");
    drop(g);
    slot.edited = true;
    dirty.sort_by_key(|k| format!("{k:?}"));
    dirty.dedup();
    crate::world::remesh(slot, &dirty);
}

fn env_f32(k: &str, d: f32) -> f32 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(d)
}

#[allow(clippy::too_many_arguments)]
fn run(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut travel: ResMut<Travel>,
    mut player: Single<&mut Player>,
    time: Res<Time>,
    mut space: ResMut<crate::world::Space>,
    mut exit: MessageWriter<AppExit>,
    (mut keys, mut mouse): (
        ResMut<ButtonInput<KeyCode>>,
        ResMut<ButtonInput<MouseButton>>,
    ),
    mut typed: MessageWriter<KeyboardInput>,
    window: Single<Entity, With<PrimaryWindow>>,
    mut stations: ResMut<crate::station::Stations>,
    clock: Res<crate::world::Clock>,
    mut map: ResMut<crate::map::GalaxyMap>,
    cameras: Query<&Camera>,
    mut elevators: ResMut<crate::elevator::Elevators>,
    mut rockets: ResMut<crate::rocket::Rockets>,
) {
    let dt = time.delta_secs();
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() {
        let k = demo.typing.remove(0);
        let (key_code, text) = match &k {
            Key::Tab => (KeyCode::Tab, None),
            Key::Enter => (KeyCode::Enter, None),
            Key::Escape => (KeyCode::Escape, None),
            Key::Character(c) => (
                KeyCode::Unidentified(NativeKeyCode::Unidentified),
                Some(c.clone()),
            ),
            _ => (KeyCode::Unidentified(NativeKeyCode::Unidentified), None),
        };
        typed.write(KeyboardInput {
            key_code,
            logical_key: k,
            state: ButtonState::Pressed,
            text,
            repeat: false,
            window: *window,
        });
        return;
    }
    if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
        demo.shot -= dt;
        if demo.shot <= 0.0 {
            demo.shot = env_f32("ORBIT_SHOT_EVERY", 2.0);
            let path = format!("{dir}/shot_{:04}_{:.0}s.png", demo.n, t);
            demo.n += 1;
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(path));
        }
    }
    if t > env_f32("ORBIT_DEMO_SECS", 150.0) {
        exit.write(AppExit::Success);
    }
    if demo.step == 7 {
        let b = &mut player.body;
        if let Some(p) = b.orbiting.and_then(|id| space.planets.get(&id)) {
            let to = -b.point_from(b.eye(), p.spec.center);
            b.look_toward(to, 4.0, dt);
        }
        return;
    }
    let b = &player.body;
    demo.wait -= dt;
    if demo.wait > 0.0 {
        return;
    }
    match demo.step {
        // Wait to stand on the home planet.
        0 if b.on_ground => {
            demo.wait = 2.0;
            demo.step = if demo.rocket {
                40
            } else if demo.elevator {
                60
            } else {
                1
            };
        }
        40 => {
            if pad_demo() {
                lay_pad(b, &mut space);
            }
            type_keys(&mut demo, "/rocket build Kestrel\n");
            demo.step = 41;
            demo.wait = 4.0;
        }
        41 => {
            // Find its helm near by and take it, as a right click would.
            let Some(p) = b.gravity_source.and_then(|id| space.planets.get(&id)) else {
                return;
            };
            let Some(v) = p.voxel.as_ref() else { return };
            let g = v.read().unwrap();
            let mut helm = None;
            for x in -32..=32 {
                for y in -32..=32 {
                    for z in -32..=32 {
                        if let Some(c) = g
                            .grid
                            .cell_at(b.pos + Vec3::new(x as f32, y as f32, z as f32))
                        {
                            if g.get(c) == orbit_core::block::HELM {
                                helm = Some(c);
                            }
                        }
                    }
                }
            }
            info!("demo: rocket built ({}); helm at {helm:?}", travel.message);
            drop(g);
            // ORBIT_DEMO_ROCKET_ROOF=1: a block over it first, to see it marked.
            if let (Some(cell), true) = (
                helm,
                std::env::var("ORBIT_DEMO_ROCKET_ROOF").is_ok_and(|x| x == "1"),
            ) {
                let mut g = v.write().unwrap();
                let (f, i, j, k) = g.grid.coords(cell);
                if let Some(over) = g.grid.cell_on_face(f, i as i64, j as i64, k as i64 + 10) {
                    g.set(over, orbit_core::block::STONE);
                }
                demo.step = 99;
                demo.wait = 0.0;
                rockets.requests.push(crate::rocket::Request::Assemble {
                    planet: p.spec.id,
                    cell,
                });
                return;
            }
            if let Some(cell) = helm {
                rockets.requests.push(crate::rocket::Request::Assemble {
                    planet: p.spec.id,
                    cell,
                });
            }
            demo.step = 42;
            demo.wait = 4.0;
        }
        42 => {
            info!(
                "demo: at the helm: {} ({})",
                rockets.is_piloting(),
                travel.message
            );
            travel.requests.push(Request::Launch);
            demo.step = 43;
            demo.wait = 2.0;
        }
        43 if travel.job.is_none() && b.orbiting.is_some() => {
            let alt = b
                .planet()
                .and_then(|id| space.planets.get(&id))
                .map_or(0.0, |p| {
                    b.point_from(b.pos, p.spec.center).length() - p.spec.radius
                });
            info!(
                "demo: rocket in orbit at {t:.0}s, {alt:.0} blocks up ({})",
                travel.message
            );
            if std::env::var("ORBIT_DEMO_ROCKET_WARP").is_ok_and(|v| v == "1") {
                demo.step = 2; // the warp trip, flown in the rocket
                return;
            }
            rockets.cockpit = true;
            demo.step = if pad_demo() { 50 } else { 44 };
            demo.wait = 4.0;
        }
        50 => {
            rockets.cockpit = false;
            match stations.deploy(None, Some("Dock"), b, &space, clock.0) {
                Ok(i) => {
                    let mut s = stations.list[i].station.write().unwrap();
                    for x in -3..=3 {
                        for z in -3..=3 {
                            s.set(IVec3::new(x, 1, z), orbit_core::block::PAD);
                        }
                    }
                    let (_, shape) = orbit_core::pad::refresh_station(&mut s, IVec3::new(0, 1, 0));
                    info!("demo: station {} up with a pad: {shape:?}", s.id);
                    drop(s);
                    stations.mark_dirty(i);
                }
                Err(r) => info!("demo: no station: {}", r.message()),
            }
            demo.step = 51;
            demo.wait = 3.0;
        }
        51 => {
            travel.requests.push(Request::Land);
            demo.step = 52;
            demo.wait = 2.0;
        }
        52 if !rockets.is_piloting() => {
            info!(
                "demo: rocket on the station at {t:.0}s: on {:?} ({})",
                b.station, travel.message
            );
            demo.step = 53;
            demo.wait = 4.0;
        }
        52 => info!(
            "demo: docking: {:?} ({})",
            travel.job.as_ref().map(|_| "on"),
            travel.message
        ),
        53 => {
            let helm = b
                .station
                .as_ref()
                .and_then(|id| stations.by_id(id))
                .and_then(|s| {
                    let s = s.station.read().unwrap();
                    s.cells()
                        .find(|&(_, k)| k == orbit_core::block::HELM)
                        .map(|(p, _)| (s.id.clone(), p))
                });
            if let Some((station, cell)) = helm {
                rockets
                    .requests
                    .push(crate::rocket::Request::AssembleOnStation { station, cell });
            }
            demo.step = 54;
            demo.wait = 2.0;
        }
        54 => {
            info!(
                "demo: off the station again: piloting {} ({})",
                rockets.is_piloting(),
                travel.message
            );
            demo.step = 99;
        }
        44 => {
            rockets.cockpit = false;
            rockets.requests.push(crate::rocket::Request::Release);
            demo.step = 45;
            demo.wait = 7.0;
        }
        45 => {
            let parked: Vec<String> = stations
                .list
                .iter()
                .map(|s| s.station.read().unwrap())
                .filter(|s| s.is_rocket())
                .map(|s| format!("{} orbit {:?}", s.name, s.orbit.map(|o| o.radius())))
                .collect();
            info!(
                "demo: parked? piloting {}, on {:?}, ground {}, rockets {parked:?} ({})",
                rockets.is_piloting(),
                b.station,
                b.on_ground,
                travel.message
            );
            let id = stations
                .list
                .iter()
                .map(|s| s.station.read().unwrap())
                .find(|s| s.is_rocket())
                .map(|s| s.id.clone());
            if let Some(id) = id {
                rockets.requests.push(crate::rocket::Request::Take(id));
            }
            demo.step = 46;
            demo.wait = 3.0;
        }
        46 => {
            info!(
                "demo: took the helm again: {} ({})",
                rockets.is_piloting(),
                travel.message
            );
            travel.requests.push(Request::Land);
            demo.step = 47;
            demo.wait = 2.0;
        }
        47 if travel.job.is_none() && !rockets.is_piloting() => {
            info!(
                "demo: landed and taken apart at {t:.0}s: on {:?}, ground {} ({})",
                b.gravity_source, b.on_ground, travel.message
            );
            demo.step = 48;
            demo.wait = 3.0;
        }
        48 => {
            info!("demo: rocket trip done");
            demo.step = 99;
        }
        1 => {
            travel.requests.push(Request::Launch);
            demo.step = 2;
            demo.wait = 1.0;
        }
        2 if demo.look && travel.job.is_none() && b.orbiting.is_some() => demo.step = 7,
        2 if demo.station && travel.job.is_none() && b.orbiting.is_some() => {
            // Open the chat and type the command with a Tab in it, as a player would.
            type_keys(&mut demo, "/sta");
            demo.step = 20;
            demo.wait = 1.0;
        }
        20 => {
            type_keys(&mut demo, "\t place\n");
            demo.step = 21;
            demo.since = t;
        }
        60 => {
            // Over to the equator, facing along it, looking at the ground a few blocks ahead.
            let Some(id) = b.gravity_source else { return };
            let Some(p) = space.planets.get(&id) else {
                return;
            };
            let Some(v) = p.voxel.as_ref() else { return };
            let axis = p.spec.spin.axis;
            let eq = b.pos.reject_from(axis).normalize();
            let ground = {
                let planet = v.read().unwrap();
                orbit_core::elevator::surface_layer(&planet, eq)
                    .map(|k| planet.grid.radius(k + 1))
                    .unwrap_or(p.spec.radius)
            };
            let (center, rotation, drift) = (p.spec.center, p.spec.rotation, p.spec.velocity);
            let body = &mut player.body;
            body.stand_on(id, center, rotation, drift, eq * (ground + 12.0));
            body.forward = axis.cross(eq).normalize();
            body.pitch = -0.35;
            type_keys(&mut demo, "/time set noon\n");
            demo.step = 61;
            demo.wait = 4.0;
        }
        61 if b.on_ground => {
            type_keys(&mut demo, "/elevator build\n");
            demo.step = 62;
            demo.wait = 4.0;
        }
        62 => {
            let lat = b
                .gravity_source
                .and_then(|id| space.planets.get(&id))
                .map_or(0.0, |p| {
                    b.pos.normalize().dot(p.spec.spin.axis).asin() * p.spec.radius
                });
            info!(
                "demo: hologram: {} (feet {lat:.1} blocks off the equator, placing {})",
                elevators.hud,
                elevators.placing.is_some()
            );
            let body = &mut player.body;
            body.pitch = -0.35;
            mouse.press(MouseButton::Right);
            demo.step = 63;
            demo.wait = 1.0;
        }
        63 => {
            mouse.release(MouseButton::Right);
            info!(
                "demo: elevators {}, stations {}",
                elevators.list.len(),
                stations.list.len()
            );
            if elevators.list.is_empty() {
                demo.step = 99;
                return;
            }
            // Step back to see it.
            let body = &mut player.body;
            body.pitch = -0.1;
            body.pos -= body.forward * 3.0;
            demo.step = 64;
            demo.wait = 5.0;
        }
        64 => {
            // Walk into the cabin: it should push back.
            let Some(slot) = elevators.list.last() else {
                return;
            };
            let Some(door) = crate::elevator::ground_door(slot) else {
                return;
            };
            let Some(mid) = crate::elevator::ground_middle(slot) else {
                return;
            };
            player.body.pos = mid + (door - mid) * 0.3;
            demo.since = t;
            demo.step = 65;
            demo.wait = 0.5;
        }
        65 => {
            let Some(slot) = elevators.list.last() else {
                return;
            };
            let Some(mid) = crate::elevator::ground_middle(slot) else {
                return;
            };
            info!(
                "demo: put inside the cabin, now {:.2} from its middle (pushed out past {:.2})",
                (b.pos - mid).reject_from(b.up).length(),
                orbit_core::elevator::CABIN_HALF_X
            );
            // To the door, facing the cabin, and in.
            let Some(door) = crate::elevator::ground_door(slot) else {
                return;
            };
            let body = &mut player.body;
            body.pos = door;
            body.forward = (mid - door).reject_from(body.up).normalize();
            keys.press(KeyCode::KeyE);
            demo.step = 66;
            demo.since = t;
        }
        66 => {
            keys.release(KeyCode::KeyE);
            if b.cabin.is_some() {
                info!("demo: riding up: {}", elevators.hud);
                // ORBIT_DEMO_ELEVATOR_OUTSIDE=1: ride in the outside view, the camera going round.
                elevators.outside =
                    std::env::var("ORBIT_DEMO_ELEVATOR_OUTSIDE").is_ok_and(|v| v == "1");
                demo.step = 67;
                demo.since = t;
            } else if t - demo.since > 3.0 {
                info!("demo: did not get in: {}", elevators.hud);
                demo.step = 99;
            }
        }
        67 => {
            if elevators.outside {
                let body = &mut player.body;
                body.forward = Quat::from_axis_angle(body.up, 0.25 * dt) * body.forward;
            }
            if player.body.cabin.is_none() {
                info!(
                    "demo: up after {:.0} s: on station {:?}, on ground {}",
                    t - demo.since,
                    player.body.station,
                    player.body.on_ground
                );
                demo.step = 68;
                demo.wait = 2.0;
            }
        }
        68 => {
            // Walk back into the docked cabin from its door: pushed back out again.
            let Some(slot) = elevators.list.last() else {
                return;
            };
            let back = -orbit_core::elevator::front_dir(slot.e.front);
            let mid = slot.e.dock() + Vec3::Y * orbit_core::elevator::CABIN_FLOOR;
            let body = &mut player.body;
            body.forward = back.cross(Vec3::Y).normalize();
            body.pitch = -0.1;
            body.pos = mid + Vec3::Y.cross(back) * 1.0;
            demo.step = 69;
            demo.wait = 0.5;
        }
        69 => {
            let Some(slot) = elevators.list.last() else {
                return;
            };
            let mid = slot.e.dock() + Vec3::Y * orbit_core::elevator::CABIN_FLOOR;
            info!(
                "demo: on the deck, put in the cabin: now {:.2} from its middle",
                (b.pos - mid).with_y(0.0).length()
            );
            // Look round the terminal from its deck.
            player.body.forward = -player.body.forward;
            player.body.pos = Vec3::new(0.5, 1.0, -5.5);
            demo.step = 99;
        }
        21 if b.station.is_some() && b.on_ground => {
            info!(
                "demo: standing on station {:?} at {:?} after {:.1}s",
                b.station,
                b.pos,
                t - demo.since
            );
            type_keys(&mut demo, "/station info\n");
            demo.step = 22;
            demo.wait = 3.0;
        }
        21 if b.station.is_none() && t - demo.since < 8.0 => {
            // Fly onto the deck: aim over the core and thrust.
            let placed = stations.placed(&space, clock.0);
            if let Some((_, f)) = placed.first() {
                let target = f.center + (f.rotation * Vec3::new(0.5, 3.0, 0.5)).as_dvec3();
                let to = (target - b.world_point(b.eye())).as_vec3();
                let body = &mut player.body;
                body.look_toward(to, 6.0, dt);
                keys.press(KeyCode::KeyW);
            }
        }
        21 if b.station.is_some() && !b.on_ground => keys.release(KeyCode::KeyW),
        21 if t - demo.since >= 8.0 && b.station.is_none() => {
            info!(
                "demo: did not reach the deck by thrusters (orbiting {:?}); /station tp instead",
                b.orbiting
            );
            keys.release(KeyCode::KeyW);
            let id = stations
                .ids(Some(true))
                .first()
                .cloned()
                .unwrap_or_default();
            type_keys(&mut demo, &format!("/station tp {id}\n"));
            demo.since = t + 30.0; // no second try
        }
        22 => {
            // Walk off the deck's edge.
            keys.press(KeyCode::KeyW);
            if b.station.is_none() {
                keys.release(KeyCode::KeyW);
                info!(
                    "demo: off the station's edge, now orbiting {:?} at {:.1} m/s",
                    b.orbiting,
                    b.world_vel().length()
                );
                demo.step = 23;
                demo.since = t;
            }
        }
        23 if t - demo.since > 4.0 => {
            let here = b.world_point(b.pos);
            let placed = stations.placed(&space, clock.0);
            let d = placed
                .first()
                .map_or(f64::NAN, |(_, f)| f.center.distance(here));
            info!(
                "demo: 4 s after walking off, {d:.1} blocks from the station (orbiting {:?})",
                b.orbiting
            );
            let id = placed
                .first()
                .map(|(s, _)| s.id.clone())
                .unwrap_or_default();
            drop(placed);
            type_keys(&mut demo, &format!("/station tp {id}\n"));
            demo.step = 24;
            demo.wait = 3.0;
        }
        24 => {
            info!(
                "demo: after /station tp: on station {:?}, on ground {}",
                b.station, b.on_ground
            );
            type_keys(
                &mut demo,
                "/time set noon\n/station list\n/statoin\n/station fly\n",
            );
            demo.step = 30;
            demo.wait = 4.0;
        }
        30 => {
            info!(
                "demo: 4 s after /time set noon: on station {:?}, on ground {}, feet {:?}",
                b.station, b.on_ground, b.pos
            );
            // Look down at the deck and build on it: a block placed by hand.
            player.body.pitch = -0.9;
            demo.step = 25;
            demo.wait = 2.0;
        }
        25 => {
            // Leave the chat open on a half-typed command, for a screenshot of its suggestions.
            type_keys(&mut demo, "/station ");
            demo.step = 26;
            demo.wait = 2.0;
        }
        26 => {
            // Esc hides the suggestions, Esc again closes the chat.
            type_keys(&mut demo, "\x1b\x1b");
            demo.step = 27;
            demo.wait = 1.0;
        }
        27 => {
            // The map opens on this system: the station goes round its planet in it.
            keys.press(KeyCode::KeyM);
            demo.step = 28;
            demo.wait = 7.0;
        }
        28 => {
            map.follow_station(0);
            demo.step = 31;
            demo.wait = 6.0;
        }
        31 => {
            keys.press(KeyCode::Backspace);
            demo.step = 29;
            demo.wait = 5.0;
        }
        29 => {
            info!("demo: map shown; pressing its Launch button");
            map.press_launch();
            demo.step = 32;
            demo.wait = 3.0;
        }
        32 => {
            let cams: Vec<bool> = cameras.iter().map(|c| c.is_active).collect();
            info!(
                "demo: 3 s after Launch from the map: map open {}, cameras active {:?}, autopilot {:?}, {:.1} m/s",
                map.is_open(),
                cams,
                travel.job,
                b.world_vel().length()
            );
            demo.step = 99;
        }
        2 if demo.map && travel.job.is_none() && b.orbiting.is_some() => {
            keys.press(KeyCode::KeyM);
            demo.step = 6;
        }
        2 if travel.job.is_none() && b.orbiting.is_some() => {
            // ORBIT_DEMO_FAR=1: the first planet of the nearest other star system instead.
            let far = std::env::var("ORBIT_DEMO_FAR").is_ok_and(|v| v == "1");
            let sys = if far {
                let here = b.world_point(b.pos);
                let mut all = space.universe.systems_near(here, 1);
                let mine = b
                    .planet()
                    .and_then(|id| space.planets.get(&id))
                    .map(|p| orbit_core::universe::Universe::sector_of(p.spec.star()));
                all.retain(|s| Some(s.sector) != mine);
                all.sort_by(|a, b| a.center.distance(here).total_cmp(&b.center.distance(here)));
                all.swap_remove(0)
            } else {
                space.universe.system(IVec3::ZERO).unwrap()
            };
            // ORBIT_DEMO_MOON=1: the home planet's moon.
            let moon = std::env::var("ORBIT_DEMO_MOON").is_ok_and(|v| v == "1")
                || std::env::var("ORBIT_DEMO").is_ok_and(|v| v == "moon");
            let first = sys.planets[0].id;
            let p = match sys
                .planets
                .iter()
                .find(|m| moon && m.host.is_some_and(|h| h.id == first))
            {
                Some(m) => m,
                None => &sys.planets[if far { 0 } else { 1 }],
            };
            info!(
                "demo: in orbit at {t:.0}s, going to {} in {} ({:.0} km)",
                p.name,
                sys.name,
                p.center.distance(b.world_point(b.pos)) / 1000.0
            );
            travel.requests.push(Request::Go(
                Target {
                    sector: sys.sector,
                    planet: p.id,
                },
                demo.mode,
            ));
            demo.step = 3;
            demo.wait = 1.0;
        }
        3 if travel.job.is_none() => {
            info!("demo: arrived at {t:.0}s ({})", travel.message);
            demo.wait = 4.0;
            demo.step = 4;
        }
        4 => {
            travel.requests.push(Request::Land);
            demo.step = 5;
            demo.wait = 1.0;
        }
        5 if travel.job.is_none() => {
            info!("demo: landed at {t:.0}s ({})", travel.message);
            demo.step = 6;
            demo.wait = 2.0;
        }
        // Standing still: how far the camera wanders against the ground from frame to frame,
        // through world coordinates (f64) and the way it is drawn (floating origin).
        6 | 8 if b.on_ground => {
            if let Some(p) = b.gravity_source.and_then(|id| space.planets.get(&id)) {
                let local = b.eye();
                let old =
                    p.spec.rotation.inverse() * (b.world_point(local) - p.spec.center).as_vec3();
                let new = p.spec.rotation.inverse() * (b.eye_from(p.spec.center));
                demo.jitter.0 = demo.jitter.0.max(old.distance(local));
                demo.jitter.1 = demo.jitter.1.max(new.distance(local));
                demo.frames += 1;
                demo.step = 8;
                if demo.frames == 300 {
                    if let Some(v) = p.voxel.as_ref() {
                        let g = &v.read().unwrap().grid;
                        let local = b.pos.as_dvec3();
                        let f = g.face_of(local);
                        let (x, y) = g.face_coords(f, local);
                        let edge = x.min(y).min(g.n as f64 - x).min(g.n as f64 - y);
                        info!("demo: standing on cube face {f}, {edge:.1} blocks from its edge");
                    }
                    info!(
                        "demo: {:.0} km out, camera vs ground over 300 frames: world coords (f64) off by up to {:.4} m, drawn off by up to {:.6} m",
                        b.frame_center().length() / 1000.0,
                        demo.jitter.0,
                        demo.jitter.1
                    );
                    demo.step = 9;
                }
            }
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=water`: (elapsed seconds when each part started, the place by the sea: a dry
/// direction and the wet one beyond it).
fn water_demo(
    mut player: Single<&mut Player>,
    mut space: ResMut<crate::world::Space>,
    time: Res<Time>,
    mut state: Local<(u32, f32, Vec3, Vec3)>,
) {
    use orbit_core::block;
    use orbit_core::planet::Terrain;
    let t = time.elapsed_secs();
    let b = &mut player.body;
    let Some(slot) = b.gravity_source.and_then(|id| space.planets.get_mut(&id)) else {
        return;
    };
    let Some(v) = slot.voxel.clone() else { return };
    let spec = slot.spec.clone();
    let (step, since) = (state.0, state.1);
    let at = |d: Vec3, up: f32, ground: i64| d * (spec.radius + ground as f32 + up);
    match step {
        // Find the sea from where we stand, and hover on its shore looking out over it.
        0 if b.on_ground || b.swimming.is_some() => {
            let terrain = Terrain::new(&spec);
            let here = b.pos.normalize();
            let (t1, t2) = (
                here.any_orthonormal_vector(),
                here.cross(here.any_orthonormal_vector()),
            );
            let found = (1..200).find_map(|r| {
                (0..32).find_map(|k| {
                    let a = k as f32 / 32.0 * std::f32::consts::TAU;
                    let d = (here + (t1 * a.cos() + t2 * a.sin()) * (r as f32 / spec.radius))
                        .normalize();
                    (terrain.column(d).height <= -5).then_some(d)
                })
            });
            let Some(sea) = found else { return };
            let dir = (sea - here).normalize();
            // Back up the way we came to dry land, a few blocks from the water.
            let mut shore = sea;
            for _ in 0..60 {
                shore = (shore - dir * (1.0 / spec.radius)).normalize();
                if !terrain.column(shore).is_wet() {
                    break;
                }
            }
            let shore = (shore - dir * (4.0 / spec.radius)).normalize();
            // Out to sea, where it is deep.
            let mut wet = sea;
            for n in 0..60 {
                wet = (sea + dir * (n as f32 / spec.radius)).normalize();
                if terrain.column(wet).height <= -7 {
                    break;
                }
            }
            *state = (1, t, shore, wet);
            b.flying = true;
            b.pos = at(shore, 4.5, terrain.column(shore).ground().max(0));
            b.vel = Vec3::ZERO;
            b.forward = dir.reject_from(b.pos.normalize()).normalize();
            b.pitch = -0.25;
        }
        // Lava and water poured side by side on the shore below.
        1 if t > since + 6.0 => {
            let mut g = v.write().unwrap();
            let up = b.pos.normalize();
            let side = b.forward.cross(up);
            let mut dirty = Vec::new();
            for (off, liquid) in [(-2.0, block::LAVA), (2.5, block::WATER)] {
                let p = b.pos + b.forward * 4.0 + side * off;
                // The first open cell over the ground there.
                let Some(mut c) = g.grid.cell_at(p) else {
                    continue;
                };
                while let Some(below) = g.grid.neighbor(c, orbit_core::sphere::Side::Bottom) {
                    if block::is_solid(g.get(below)) || block::is_liquid(g.get(below)) {
                        break;
                    }
                    c = below;
                }
                dirty.extend(g.set(c, liquid));
            }
            drop(g);
            slot.edited = true;
            crate::world::remesh(slot, &dirty);
            state.0 = 2;
            state.1 = t;
        }
        // Under the sea, looking out along the bottom.
        2 if t > since + 22.0 => {
            let wet = state.3;
            let floor = Terrain::new(&spec).column(wet).height;
            b.pos = at(wet, 1.5, floor);
            b.forward = (wet - state.2).normalize().reject_from(wet).normalize();
            b.pitch = 0.25;
            state.0 = 3;
            state.1 = t;
        }
        // Back up, looking down at the shore from the air.
        3 if t > since + 8.0 => {
            let shore = state.2;
            b.pos = at(shore, 22.0, 0);
            b.pitch = -0.6;
            state.0 = 4;
        }
        _ => {}
    }
    if step >= 1 {
        b.vel = Vec3::ZERO;
    }
}

/// `ORBIT_DEMO=blocks`: a showcase of the blocks with shapes, built on the ground before you.
fn blocks_demo(
    mut player: Single<&mut Player>,
    mut space: ResMut<crate::world::Space>,
    mut drops: ResMut<crate::inventory::Drops>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::block::{self, Dir};
    use orbit_core::shape::{self, Against, Around};
    let t = time.elapsed_secs();
    let p = &mut **player;
    let b = &mut p.body;
    let Some(id) = b.gravity_source else { return };
    let Some(slot) = space.planets.get_mut(&id) else {
        return;
    };
    let Some(v) = slot.voxel.clone() else { return };
    match state.0 {
        0 if b.on_ground && t > 4.0 => {
            if std::env::var("ORBIT_DEMO_SURVIVAL").is_ok_and(|v| v == "1") {
                p.inv.mode = orbit_core::inventory::GameMode::Survival;
            }
            let mut g = v.write().unwrap();
            let up = b.pos.normalize();
            let fwd = b.forward.reject_from(up).normalize();
            let side = fwd.cross(up);
            // On a platform up in the air, clear of whatever is round the spawn.
            b.pos += up * 14.0;
            b.vel = Vec3::ZERO;
            let feet = b.pos + up * 0.5;
            // Directions on the grid from the cell under the feet: ahead and to the right.
            let origin = g.grid.cell_at(feet).unwrap();
            let dir_vec = |g: &orbit_core::planet::Planet, d: Dir| {
                g.side(origin, d).map_or(Vec3::ZERO, |n| {
                    (g.grid.center(n) - g.grid.center(origin)).normalize()
                })
            };
            let ahead = Dir::ALL
                .into_iter()
                .max_by(|a, b| {
                    dir_vec(&g, *a)
                        .dot(fwd)
                        .total_cmp(&dir_vec(&g, *b).dot(fwd))
                })
                .unwrap();
            let right = if dir_vec(&g, ahead.turned()).dot(side) > 0.0 {
                ahead.turned()
            } else {
                ahead.turned().opposite()
            };
            // The cell `f` ahead, `x` to the right, `h` up from the feet, stepping cell to cell.
            let cell = |g: &orbit_core::planet::Planet, f: f32, x: f32, h: f32| {
                let mut c = origin;
                let step = |c: usize, d: Dir, n: i32| {
                    (0..n.abs()).fold(c, |c, _| {
                        g.side(c, if n < 0 { d.opposite() } else { d }).unwrap_or(c)
                    })
                };
                c = step(c, ahead, f as i32);
                c = step(c, right, x as i32);
                for _ in 0..(h as i32).abs() {
                    c = if h < 0.0 { g.below(c) } else { g.above(c) }.unwrap_or(c);
                }
                c
            };
            let mut dirty = Vec::new();
            for f in -2..=9 {
                for x in -8..=9 {
                    {
                        let c = cell(&g, f as f32, x as f32, -1.0);
                        dirty.extend(g.set(c, block::GRASS));
                    }
                    for h in 0..5 {
                        {
                            let c = cell(&g, f as f32, x as f32, h as f32);
                            dirty.extend(g.set(c, block::AIR));
                        }
                    }
                }
            }
            fn put(
                g: &mut orbit_core::planet::Planet,
                dirty: &mut Vec<orbit_core::planet::ChunkKey>,
                c: usize,
                item: block::Block,
                against: Against,
                look: Dir,
            ) {
                match shape::place(&*g, c, item, against, look, false) {
                    Some(puts) => {
                        for (c, b) in puts {
                            dirty.extend(g.set(c, b));
                        }
                    }
                    None => info!("demo: {} would not go there", block::name(item)),
                }
            }
            // The Dir from cell a to cell b, if they are side by side.
            let toward = |g: &orbit_core::planet::Planet, a: usize, b: usize| {
                Dir::ALL
                    .into_iter()
                    .find(|&d| g.side(a, d) == Some(b))
                    .unwrap_or(Dir(0))
            };
            let away = {
                let (a, c) = (cell(&g, 5.0, 0.0, 0.0), cell(&g, 4.0, 0.0, 0.0));
                toward(&g, a, c) // from the wall back toward the player
            };
            // A wall of stone with a doorway, a door in it; torches on it either side.
            for x in -2..=2 {
                for h in 0..3 {
                    if x != 0 || h == 2 {
                        {
                            let c = cell(&g, 6.0, x as f32, h as f32);
                            let ag = Against::Below;
                            put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite());
                        }
                    }
                }
            }
            {
                let c = cell(&g, 6.0, 0.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::DOOR, ag, away.opposite());
            }
            for x in [-1.0, 1.0] {
                let (wall, at) = (cell(&g, 6.0, x, 1.0), cell(&g, 5.0, x, 1.0));
                let d = toward(&g, at, wall);
                {
                    let c = cell(&g, 5.0, x, 1.0);
                    let ag = Against::Side(d);
                    put(&mut g, &mut dirty, c, block::TORCH, ag, away.opposite());
                }
            }
            // An open door beside it, on its own.
            {
                let c = cell(&g, 4.0, -5.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::DOOR, ag, away.opposite());
            }
            let c = cell(&g, 4.0, -5.0, 0.0);
            for (c, b) in shape::toggle(&*g, c) {
                dirty.extend(g.set(c, b));
            }
            // A pillar with a ladder up its face.
            for h in 0..4 {
                {
                    let c = cell(&g, 4.0, -7.0, h as f32);
                    let ag = Against::Below;
                    put(
                        &mut g,
                        &mut dirty,
                        c,
                        block::COBBLESTONE,
                        ag,
                        away.opposite(),
                    );
                }
                let (wall, at) = (cell(&g, 4.0, -7.0, h as f32), cell(&g, 3.0, -7.0, h as f32));
                {
                    let c = cell(&g, 3.0, -7.0, h as f32);
                    let ag = Against::Side(toward(&g, at, wall));
                    put(&mut g, &mut dirty, c, block::LADDER, ag, away.opposite());
                }
            }
            // Fences joined in a row and round a corner, to a stone block.
            for x in 3..=6 {
                {
                    let c = cell(&g, 5.0, x as f32, 0.0);
                    let ag = Against::Below;
                    put(&mut g, &mut dirty, c, block::FENCE, ag, away.opposite());
                }
            }
            {
                let c = cell(&g, 4.0, 6.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::FENCE, ag, away.opposite());
            }
            {
                let c = cell(&g, 3.0, 6.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite());
            }
            {
                let c = cell(&g, 2.0, 6.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::TORCH, ag, away.opposite());
            }
            // Panes between glass and stone, two high.
            for h in 0..2 {
                {
                    let c = cell(&g, 7.0, 3.0, h as f32);
                    let ag = Against::Below;
                    put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite());
                }
                for x in 4..=6 {
                    {
                        let c = cell(&g, 7.0, x as f32, h as f32);
                        let ag = Against::Below;
                        put(
                            &mut g,
                            &mut dirty,
                            c,
                            block::GLASS_PANE,
                            ag,
                            away.opposite(),
                        );
                    }
                }
                {
                    let c = cell(&g, 7.0, 7.0, h as f32);
                    let ag = Against::Below;
                    put(&mut g, &mut dirty, c, block::GLASS, ag, away.opposite());
                }
            }
            // Trapdoors: one shut on the ground, one open.
            {
                let c = cell(&g, 3.0, -2.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::TRAPDOOR, ag, away.opposite());
            }
            {
                let c = cell(&g, 3.0, 2.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::TRAPDOOR, ag, away.opposite());
            }
            let c = cell(&g, 3.0, 2.0, 0.0);
            for (c, b) in shape::toggle(&*g, c) {
                dirty.extend(g.set(c, b));
            }
            // Flowers on the grass; one on stone would not go.
            {
                let c = cell(&g, 2.0, -3.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::POPPY, ag, away.opposite());
            }
            {
                let c = cell(&g, 2.0, -4.0, 0.0);
                let ag = Against::Below;
                put(&mut g, &mut dirty, c, block::DANDELION, ag, away.opposite());
            }
            // Bare dirt by the grass, for it to grow over.
            for x in -1..=1 {
                {
                    let c = cell(&g, 1.0, x as f32, -1.0);
                    dirty.extend(g.set(c, block::DIRT));
                }
            }
            // A few items lying about.
            for (k, item) in [
                block::COBBLESTONE,
                block::DIRT,
                block::TORCH,
                block::DOOR,
                block::POPPY,
            ]
            .into_iter()
            .enumerate()
            {
                let c = g.grid.center(cell(&g, 2.0, k as f32 - 1.0, 0.0));
                drops.pop(crate::inventory::Frame::Planet(id), c, up, item);
            }
            drop(g);
            dirty.sort_by_key(|k| format!("{k:?}"));
            dirty.dedup();
            crate::world::remesh(slot, &dirty);
            b.pitch = -0.1;
            *state = (1, t);
        }
        // Then the inventory.
        // A look down at what is before you.
        1 if t > state.1 + 5.0 && b.pitch > -0.2 => b.pitch = -0.35,
        1 if t > state.1 + 9.0 => {
            keys.press(KeyCode::KeyE);
            *state = (2, t);
        }
        2 => {
            keys.release(KeyCode::KeyE);
            *state = (3, t);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=health` (with `ORBIT_SHOTS`): survival, a command hit, a fall, lava, death and
/// the respawn, a screenshot after each.
fn health_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    mut space: ResMut<crate::world::Space>,
    vitals: Res<crate::health::Vitals>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let b = &mut player.body;
    let hp = vitals.health.hp;
    match state.0 {
        0 if b.on_ground && t > 4.0 => {
            // From the start menu the game is already in survival.
            if std::env::var("ORBIT_DEMO").is_ok_and(|v| v != "menu") {
                type_keys(&mut demo, "/gamemode survival\n");
            }
            *state = (1, t + 1.0);
        }
        1 => {
            info!("health demo: survival, hp {hp}");
            shot(&mut commands, "1_survival");
            type_keys(&mut demo, "/damage 7\n");
            *state = (2, t + 0.3);
        }
        2 => {
            info!("health demo: after /damage 7, hp {hp}");
            shot(&mut commands, "2_damage");
            *state = (3, t + 1.5);
        }
        3 => {
            let up = b.pos.normalize();
            b.pos += up * 8.0;
            b.vel = Vec3::ZERO;
            info!("health demo: dropped from 8 blocks, hp before {hp}");
            *state = (4, t + 2.5);
        }
        4 => {
            info!("health demo: after the fall, hp {hp} (expected 20-7=13 minus 5 = 8)");
            shot(&mut commands, "3_fall");
            type_keys(&mut demo, "/heal\n");
            *state = (5, t + 1.0);
        }
        5 | 6 | 61 | 7 => {
            if state.0 == 5 {
                info!("health demo: after /heal, hp {hp}");
                state.0 = 6;
                state.1 = t;
            }
            // Keep lava over the feet (it would flow away).
            let up = b.pos.normalize();
            if let Some(slot) = b.gravity_source.and_then(|id| space.planets.get_mut(&id)) {
                if let Some(v) = slot.voxel.clone() {
                    let mut g = v.write().unwrap();
                    let mut dirty = Vec::new();
                    let mut c = g.grid.cell_at(b.pos + up * 0.3).unwrap();
                    for _ in 0..3 {
                        dirty.extend(g.set(c, orbit_core::block::LAVA));
                        c = g.above(c).unwrap_or(c);
                    }
                    drop(g);
                    slot.edited = true;
                    crate::world::remesh(slot, &dirty);
                }
            }
            if state.0 == 6 {
                state.0 = 61;
                state.1 = t + 0.8;
                return;
            }
            if state.0 == 61 {
                info!("health demo: in lava, hp {hp}, swimming {:?}", b.swimming);
                shot(&mut commands, "4_lava");
                state.0 = 7;
            }
            if vitals.dead {
                info!("health demo: dead, hp {hp}");
                shot(&mut commands, "5_dead");
                *state = (8, t + 1.0);
            }
        }
        8 => {
            type_keys(&mut demo, "\n");
            *state = (9, t + 3.0);
        }
        9 => {
            info!("health demo: after respawn dead {} hp {hp}", vitals.dead);
            shot(&mut commands, "6_respawned");
            *state = (10, t + 1.0);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=hands` (with `ORBIT_SHOTS`): stone in hand, a torch in the left hand; F swaps
/// them; then with the main hand empty a right click places the torch from the left one.
fn hands_demo(
    mut commands: Commands,
    mut player: Single<&mut Player>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::inventory::{GameMode, Stack};
    let t = time.elapsed_secs();
    if t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let show = |p: &Player| {
        format!(
            "main {:?} off {:?}",
            p.inv.held().map(|s| (s.item, s.count)),
            p.inv.offhand.map(|s| (s.item, s.count))
        )
    };
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; orbit_core::inventory::SLOTS];
            p.inv.slots[0] = Some(Stack::new(orbit_core::block::STONE, 5));
            p.inv.offhand = Some(Stack::new(orbit_core::block::TORCH, 5));
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.7;
            info!("hands demo: start: {}", show(p));
            *state = (1, t + 1.5);
        }
        1 => {
            shot(&mut commands, "1_start");
            keys.press(KeyCode::KeyF);
            *state = (2, t + 0.2);
        }
        2 => {
            keys.release(KeyCode::KeyF);
            info!("hands demo: after F: {}", show(p));
            *state = (3, t + 1.0);
        }
        3 => {
            shot(&mut commands, "2_swapped");
            keys.press(KeyCode::KeyF);
            *state = (4, t + 0.2);
        }
        4 => {
            keys.release(KeyCode::KeyF);
            info!("hands demo: after F again: {}", show(p));
            // Main hand empty: the right click falls to the left hand.
            p.inv.slots[0] = None;
            *state = (5, t + 0.5);
        }
        5 => {
            mouse.press(MouseButton::Right);
            *state = (6, t + 0.2);
        }
        6 => {
            mouse.release(MouseButton::Right);
            info!(
                "hands demo: after right click with an empty main hand: {}",
                show(p)
            );
            *state = (7, t + 1.0);
        }
        7 => {
            shot(&mut commands, "3_placed");
            // Main hand holds stone and can place it: the main hand wins.
            p.inv.slots[0] = Some(Stack::new(orbit_core::block::STONE, 5));
            *state = (8, t + 0.3);
        }
        8 => {
            p.body.pitch = -0.5;
            mouse.press(MouseButton::Right);
            *state = (9, t + 0.2);
        }
        9 => {
            mouse.release(MouseButton::Right);
            info!("hands demo: right click with stone in both: {}", show(p));
            *state = (10, t + 1.0);
        }
        10 => {
            shot(&mut commands, "4_main_wins");
            *state = (11, t);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=mining` (with `ORBIT_SHOTS`): in creative a click breaks stone at once; in
/// survival bare hands do not get through stone in 3 s, a wooden pickaxe does and it wears,
/// a stone pickaxe breaks iron ore but leaves nothing, a copper pickaxe leaves the ore.
#[allow(clippy::too_many_arguments)]
/// Where a demo looks to face a point from its eye: (forward, pitch).
fn face(b: &orbit_core::player::Body, at: Vec3) -> (Vec3, f32) {
    let d = at - b.eye();
    let flat = d - b.up * d.dot(b.up);
    (
        flat.normalize_or(b.forward),
        (d.dot(b.up) / d.length().max(1e-3)).clamp(-1.0, 1.0).asin(),
    )
}

/// `ORBIT_DEMO=mobs`: in survival with an iron sword, `/summon slime`: it wanders without attacking;
/// a hit with the sword (7 of its 8 hp) wakes it, it hunts the player and hurts on contact; a
/// second hit kills it and it drops slimeballs, which the player picks up.
#[allow(clippy::too_many_arguments)]
fn mobs_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    vitals: Res<crate::health::Vitals>,
    mobs: Res<crate::mobs::Mobs>,
    drops: Res<crate::inventory::Drops>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32, u64, f32)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS, Stack};
    use orbit_core::items;
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let balls = |p: &Player| {
        p.inv
            .slots
            .iter()
            .flatten()
            .filter(|s| s.item == items::SLIMEBALL)
            .map(|s| s.count as u32)
            .sum::<u32>()
    };
    // The slime we follow, if it is still there.
    let slime = mobs.find(state.2).map(|s| s.mob.clone());
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            p.inv.slots[0] = Some(Stack::new(items::IRON_SWORD, 1));
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.25;
            type_keys(&mut demo, "/summon slime\n");
            *state = (1, t + 1.0, 0, 0.0);
        }
        1 => {
            // The nearest slime is the one summoned.
            let Some(s) = mobs
                .list
                .iter()
                .filter(|s| s.mob.kind == orbit_core::mob::Kind::Slime)
                .min_by(|a, b| {
                    a.mob
                        .pos
                        .distance(p.body.pos)
                        .total_cmp(&b.mob.pos.distance(p.body.pos))
                })
            else {
                info!("mobs demo: no slime appeared");
                state.1 = t + 0.5;
                return;
            };
            state.2 = s.mob.id;
            state.3 = vitals.health.hp;
            info!(
                "mobs demo: slime #{:x} at {:.1} blocks, hp {}, aggro {}, player hp {}",
                s.mob.id & 0xFFFF,
                s.mob.pos.distance(p.body.pos),
                s.mob.hp,
                s.mob.aggro,
                vitals.health.hp
            );
            (p.body.forward, p.body.pitch) =
                face(&p.body, s.mob.pos + s.mob.pos.normalize() * 0.45);
            *state = (2, t + 0.6, s.mob.id, vitals.health.hp);
        }
        2 => {
            // Watch it wander for 6 seconds, the player right there, unhurt.
            let Some(s) = slime else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            *state = (12, t + 0.25, state.2, state.3);
        }
        12 => {
            let Some(s) = slime else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            shot(&mut commands, "1_passive_wander");
            info!(
                "mobs demo: wandering: slime {:.1} blocks away, aggro {}, player hp {} (was {})",
                s.pos.distance(p.body.pos),
                s.aggro,
                vitals.health.hp,
                state.3
            );
            *state = (3, t + 3.0, state.2, state.3);
        }
        3 => {
            let Some(s) = slime else { return };
            info!(
                "mobs demo: still passive: slime {:.1} blocks away, aggro {}, player hp {} (expected unchanged)",
                s.pos.distance(p.body.pos),
                s.aggro,
                vitals.health.hp
            );
            // Close in if it wandered off, then face it.
            if s.pos.distance(p.body.pos) > 3.5 {
                let dir = (s.pos - p.body.pos).normalize();
                p.body.pos = s.pos - dir * 2.0 + p.body.up * 0.5;
                p.body.vel = Vec3::ZERO;
            }
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            *state = (4, t + 0.3, state.2, state.3);
        }
        4 => {
            // Swing: aim again, press the left button.
            let Some(s) = slime else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            if mobs.aimed == Some(s.id) {
                info!(
                    "mobs demo: swinging the iron sword at the slime (hp {})",
                    s.hp
                );
                mouse.press(MouseButton::Left);
                *state = (5, t + 0.1, state.2, state.3);
            } else if s.pos.distance(p.body.pos) > 3.0 {
                let dir = (s.pos - p.body.pos).normalize();
                p.body.pos = s.pos - dir * 2.0 + p.body.up * 0.5;
                p.body.vel = Vec3::ZERO;
            }
        }
        5 => {
            mouse.release(MouseButton::Left);
            let hp = slime.as_ref().map(|s| (s.hp, s.aggro));
            info!("mobs demo: after the first hit: slime (hp, aggro) = {hp:?} (expected 1, true)");
            shot(&mut commands, "2_hit");
            *state = (6, t + 0.2, state.2, state.3);
        }
        6 => {
            // It hunts: let it come, face it, count the damage taken.
            let Some(s) = slime else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            info!(
                "mobs demo: hunting: slime {:.1} blocks away, player hp {}",
                s.pos.distance(p.body.pos),
                vitals.health.hp
            );
            if vitals.health.hp < state.3 {
                info!(
                    "mobs demo: the slime hurt the player: {} -> {} hp",
                    state.3, vitals.health.hp
                );
                shot(&mut commands, "3_hurt_by_contact");
                *state = (7, t + 0.6, state.2, state.3);
            } else {
                state.1 = t + 0.5;
            }
        }
        7 => {
            // Finish it (after its hit cooldown).
            let Some(s) = slime else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, s.pos + s.pos.normalize() * 0.45);
            if mobs.aimed == Some(s.id) {
                info!("mobs demo: second swing (slime hp {})", s.hp);
                mouse.press(MouseButton::Left);
                *state = (8, t + 0.1, state.2, state.3);
            }
        }
        8 => {
            mouse.release(MouseButton::Left);
            shot(&mut commands, "4_killed");
            info!(
                "mobs demo: slime gone: {}; slimeballs on the ground: {}",
                mobs.find(state.2).is_none(),
                drops
                    .list
                    .iter()
                    .filter(|d| d.d.stack.item == items::SLIMEBALL)
                    .count()
            );
            // Walk over to the nearest one.
            if let Some(d) = drops
                .list
                .iter()
                .find(|d| d.d.stack.item == items::SLIMEBALL)
            {
                p.body.pos = d.d.pos + p.body.up * 0.2;
                p.body.vel = Vec3::ZERO;
            }
            *state = (9, t + 2.5, state.2, state.3);
        }
        9 => {
            info!(
                "mobs demo: slimeballs in the inventory: {} (kills {})",
                balls(p),
                mobs.kills
            );
            shot(&mut commands, "5_picked_up");
            *state = (10, t + 100.0, state.2, state.3);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=npc`: Bodoque stands near the spawn. The player walks up to him, right clicks, reads the
/// first lines, asks how to get started, takes the Stone Pickaxe he offers, closes with Esc; talks
/// again and is told there is no second one.
#[allow(clippy::too_many_arguments)]
fn npc_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    mobs: Res<crate::mobs::Mobs>,
    talk: Res<crate::dialogue::Talk>,
    story: Res<crate::story::Story>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32, f32)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS};
    use orbit_core::items;
    use orbit_core::mob::Kind;
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let bodoque = mobs
        .list
        .iter()
        .find(|s| s.mob.kind == Kind::Bodoque)
        .map(|s| s.mob.clone());
    let inv = |p: &Player| {
        p.inv
            .slots
            .iter()
            .flatten()
            .map(|s| format!("{} x{}", items::name(s.item), s.count))
            .collect::<Vec<_>>()
    };
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            *state = (1, t + 0.5, t);
        }
        1 => {
            let Some(b) = bodoque else {
                info!("npc demo: waiting for Bodoque to appear");
                state.1 = t + 0.5;
                return;
            };
            info!(
                "npc demo: Bodoque is at {:.1} blocks from the player (spawn), hp {}, invulnerable {}",
                b.pos.distance(p.body.pos),
                b.hp,
                b.kind.invulnerable()
            );
            *state = (2, t + 0.1, t);
        }
        2 => {
            // Walk up to him, looking at him; if the ground is in the way, step over.
            let Some(b) = bodoque else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.4);
            let d = b.pos.distance(p.body.pos);
            if d > 3.2 {
                keys.press(KeyCode::KeyW);
                if t - state.2 > 6.0 {
                    let dir = (b.pos - p.body.pos).normalize();
                    p.body.pos = b.pos - dir * 3.0 + p.body.up * 0.5;
                    p.body.vel = Vec3::ZERO;
                    state.2 = t;
                    info!("npc demo: ground in the way: stepped up to him");
                }
            } else {
                keys.release(KeyCode::KeyW);
                info!(
                    "npc demo: arrived, {d:.1} blocks from Bodoque, aimed at him: {}",
                    mobs.aimed == Some(b.id)
                );
                *state = (3, t + 0.6, t);
            }
        }
        3 => {
            let Some(b) = bodoque else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.4);
            shot(&mut commands, "1_bodoque");
            if mobs.aimed == Some(b.id) {
                mouse.press(MouseButton::Right);
                *state = (4, t + 0.15, t);
            } else {
                state.1 = t + 0.2;
            }
        }
        4 => {
            mouse.release(MouseButton::Right);
            info!(
                "npc demo: dialogue open: {}, node {:?}",
                talk.is_open(),
                talk.node()
            );
            *state = (5, t + 0.9, t);
        }
        5 => {
            shot(&mut commands, "2_typing");
            *state = (6, t + 2.6, t);
        }
        6 => {
            shot(&mut commands, "3_choices");
            type_keys(&mut demo, "1");
            *state = (7, t + 3.2, t);
        }
        7 => {
            info!("npc demo: after choosing 1: node {:?}", talk.node());
            shot(&mut commands, "4_how_to_start");
            // The Stone Pickaxe is reply 1 of this node.
            type_keys(&mut demo, "1");
            *state = (8, t + 0.6, t);
        }
        8 => {
            info!(
                "npc demo: inventory after the gift: {:?}; flag gave={}; back at node {:?}",
                inv(p),
                story.flags.gave(orbit_core::dialogue::Npc::Bodoque),
                talk.node()
            );
            shot(&mut commands, "5_gift_received");
            *state = (9, t + 0.5, t);
        }
        9 => {
            type_keys(&mut demo, "\x1b");
            *state = (10, t + 0.6, t);
        }
        10 => {
            info!(
                "npc demo: closed with Esc: dialogue open {}",
                talk.is_open()
            );
            shot(&mut commands, "6_closed");
            // Talk again: the second time there is no second pickaxe.
            let Some(b) = bodoque else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.4);
            *state = (11, t + 0.4, t);
        }
        11 => {
            let Some(b) = bodoque else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.4);
            mouse.press(MouseButton::Right);
            *state = (12, t + 0.15, t);
        }
        12 => {
            mouse.release(MouseButton::Right);
            *state = (13, t + 2.8, t);
        }
        13 => {
            type_keys(&mut demo, "1");
            *state = (14, t + 3.0, t);
        }
        14 => {
            type_keys(&mut demo, "1");
            *state = (15, t + 0.5, t);
        }
        15 => {
            let n = p
                .inv
                .slots
                .iter()
                .flatten()
                .filter(|s| s.item == items::STONE_PICKAXE)
                .count();
            info!(
                "npc demo: asked again: node {:?} (expected 4, 'I already gave you'); pickaxes in the inventory: {n} (expected 1)",
                talk.node()
            );
            shot(&mut commands, "7_no_second_gift");
            type_keys(&mut demo, "\x1b");
            *state = (16, t + 100.0, t);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=moon`: the warp trip to the home planet's moon (see `run`: launch, travel, land), then,
/// on its ground, Tulio: the player walks up to him, talks (advice on the King, the gift of three
/// slimeballs) and opens the galaxy map to show the moon's name.
#[allow(clippy::too_many_arguments)]
fn moon_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    space: Res<crate::world::Space>,
    mobs: Res<crate::mobs::Mobs>,
    talk: Res<crate::dialogue::Talk>,
    story: Res<crate::story::Story>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32, f32)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS};
    use orbit_core::items;
    use orbit_core::mob::Kind;
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let on_moon = p
        .body
        .gravity_source
        .and_then(|id| space.planets.get(&id))
        .filter(|s| s.spec.is_moon());
    let tulio = mobs
        .list
        .iter()
        .find(|s| s.mob.kind == Kind::Tulio)
        .map(|s| s.mob.clone());
    match state.0 {
        0 => {
            let Some(m) = on_moon.filter(|_| p.body.on_ground) else {
                return;
            };
            info!(
                "moon demo: standing on {} (moon {}, r {:.0}) after the trip, at {t:.0}s",
                m.spec.name,
                m.spec.is_moon(),
                m.spec.radius
            );
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            *state = (1, t + 0.5, t);
        }
        1 => {
            let Some(b) = tulio else {
                info!("moon demo: waiting for Tulio to appear");
                state.1 = t + 0.5;
                return;
            };
            info!(
                "moon demo: Tulio is {:.1} blocks away, invulnerable {}",
                b.pos.distance(p.body.pos),
                b.kind.invulnerable()
            );
            *state = (2, t + 0.1, t);
        }
        2 => {
            let Some(b) = tulio else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.6);
            let d = b.pos.distance(p.body.pos);
            if d > 3.4 {
                keys.press(KeyCode::KeyW);
                if t - state.2 > 6.0 {
                    let dir = (b.pos - p.body.pos).normalize();
                    p.body.pos = b.pos - dir * 3.0 + p.body.up * 0.5;
                    p.body.vel = Vec3::ZERO;
                    state.2 = t;
                    info!("moon demo: ground in the way: stepped up to him");
                }
            } else {
                keys.release(KeyCode::KeyW);
                info!("moon demo: arrived, {d:.1} blocks from Tulio");
                *state = (3, t + 0.6, t);
            }
        }
        3 => {
            let Some(b) = tulio else { return };
            (p.body.forward, p.body.pitch) = face(&p.body, b.pos + b.up() * 1.6);
            shot(&mut commands, "1_tulio_on_the_moon");
            if mobs.aimed == Some(b.id) {
                mouse.press(MouseButton::Right);
                *state = (4, t + 0.15, t);
            } else {
                state.1 = t + 0.2;
            }
        }
        4 => {
            mouse.release(MouseButton::Right);
            info!(
                "moon demo: dialogue open: {}, node {:?}",
                talk.is_open(),
                talk.node()
            );
            *state = (5, t + 3.0, t);
        }
        5 => {
            shot(&mut commands, "2_tulio_greeting");
            type_keys(&mut demo, "2"); // Any advice?
            *state = (6, t + 3.5, t);
        }
        6 => {
            info!(
                "moon demo: node {:?} (the tip on the Slime Crown Core)",
                talk.node()
            );
            shot(&mut commands, "3_tulio_advice");
            type_keys(&mut demo, "1"); // the gift
            *state = (7, t + 0.8, t);
        }
        7 => {
            let balls: u32 = p
                .inv
                .slots
                .iter()
                .flatten()
                .filter(|s| s.item == items::SLIMEBALL)
                .map(|s| s.count as u32)
                .sum();
            info!(
                "moon demo: slimeballs after the gift: {balls} (expected 3); flag gave={}",
                story.flags.gave(orbit_core::dialogue::Npc::Tulio)
            );
            shot(&mut commands, "4_tulio_gift");
            type_keys(&mut demo, "\x1b");
            *state = (8, t + 0.6, t);
        }
        8 => {
            info!("moon demo: closed: dialogue open {}", talk.is_open());
            keys.press(KeyCode::KeyM);
            *state = (9, t + 0.2, t);
        }
        9 => {
            keys.release(KeyCode::KeyM);
            *state = (10, t + 2.5, t);
        }
        10 => {
            shot(&mut commands, "5_galaxy_map");
            keys.press(KeyCode::KeyM);
            *state = (11, t + 0.2, t);
        }
        11 => {
            keys.release(KeyCode::KeyM);
            *state = (12, t + 100.0, t);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=boss`: survival, a Slime Crown Core and an iron sword. A right click on the ground spends the
/// core and the Slime King rises 8 blocks off; a second core is refused while it lives. It hunts
/// and attacks (each attack and each hurt is logged by `mobs`). The player hits it with the sword;
/// to shorten the fight the demo sets its life to just over half, and the next hit takes it into
/// phase 2 (it summons); then to a few points: the next hit kills it (slimeballs, message, bar gone).
#[allow(clippy::too_many_arguments)]
fn boss_demo(
    mut commands: Commands,
    mut player: Single<&mut Player>,
    mut mobs: ResMut<crate::mobs::Mobs>,
    mut vitals: ResMut<crate::health::Vitals>,
    story: Res<crate::story::Story>,
    drops: Res<crate::inventory::Drops>,
    bar: Res<crate::chat::ActionBar>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32, f32, u32, bool, bool)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS, Stack};
    use orbit_core::items;
    use orbit_core::mob::Kind;
    let t = time.elapsed_secs();
    if t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let king = mobs
        .list
        .iter()
        .find(|s| s.mob.kind == Kind::SlimeKing)
        .map(|s| s.mob.clone());
    let cores = |p: &Player| {
        p.inv
            .slots
            .iter()
            .flatten()
            .filter(|s| s.item == items::SLIME_CROWN_CORE)
            .map(|s| s.count as u32)
            .sum::<u32>()
    };
    // Keeps the player alive: it is the King's attacks that are on show, not the death screen.
    if vitals.health.hp < 8.0 && state.0 >= 4 {
        vitals.health.hp = vitals.health.max;
        info!("boss demo: (the demo refills the player's hp)");
    }
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            p.inv.slots[0] = Some(Stack::new(items::SLIME_CROWN_CORE, 1));
            p.inv.slots[1] = Some(Stack::new(items::IRON_SWORD, 1));
            p.inv.slots[2] = Some(Stack::new(items::SLIME_CROWN_CORE, 1));
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.6;
            *state = (1, t + 1.0, t, 0, false, false);
        }
        1 => {
            info!(
                "boss demo: Slime Crown Cores in the inventory: {}; right click on the ground",
                cores(p)
            );
            mouse.press(MouseButton::Right);
            state.0 = 2;
            state.1 = t + 0.15;
        }
        2 => {
            mouse.release(MouseButton::Right);
            state.0 = 3;
            state.1 = t + 1.2;
        }
        3 => {
            let Some(k) = king else {
                // Not summoned (no open ground ahead where the spawn happens to be): turn and try again.
                if cores(p) >= 2 {
                    info!("boss demo: no room ahead, turning and trying again");
                    p.body.forward = Quat::from_axis_angle(p.body.up, 0.9) * p.body.forward;
                    state.0 = 1;
                }
                state.1 = t + 0.5;
                return;
            };
            info!(
                "boss demo: the Slime King rose {:.1} blocks away: hp {}, scale {}, phase {}; cores left {} (expected 1)",
                k.pos.distance(p.body.pos),
                k.hp,
                k.scale,
                k.boss.phase,
                cores(p)
            );
            // A second core is refused while the King lives.
            p.inv.selected = 2;
            (p.body.forward, p.body.pitch) = (p.body.forward, -0.6);
            mouse.press(MouseButton::Right);
            state.0 = 31;
            state.1 = t + 0.15;
        }
        31 => {
            mouse.release(MouseButton::Right);
            info!(
                "boss demo: second core tried: cores {} (expected 1, kept); message {:?}",
                cores(p),
                bar.text()
            );
            p.inv.selected = 1; // the iron sword
            state.0 = 4;
            state.1 = t + 0.3;
            state.2 = t;
        }
        4 | 5 | 6 => {
            let Some(k) = king else {
                if state.0 == 6 {
                    info!(
                        "boss demo: the King is gone; flag boss_defeated = {}; King alive {}; slimeballs lying about: {}",
                        story.flags.boss_defeated,
                        mobs.king_alive(),
                        drops
                            .list
                            .iter()
                            .filter(|d| d.d.stack.item == items::SLIMEBALL)
                            .count()
                    );
                    shot(&mut commands, "6_victory");
                    state.0 = 7;
                    state.1 = t + 100.0;
                }
                return;
            };
            (p.body.forward, p.body.pitch) = face(&p.body, k.pos + k.up() * 1.5);
            // Photographs: the bar once, then a telegraphed attack, then the second phase.
            if !state.4 && t - state.2 > 1.0 {
                state.4 = true;
                shot(&mut commands, "2_bar_and_king");
            }
            if !state.5 && k.boss.warning().is_some() && k.boss.glow > 0.7 {
                state.5 = true;
                info!(
                    "boss demo: telegraphed attack {:?}: glow {:.2}, crouch {:.2}",
                    k.boss.attack, k.boss.glow, k.boss.crouch
                );
                shot(&mut commands, "3_telegraph");
            }
            // Too far to reach: step up to it.
            if t - state.2 > 7.0 && mobs.aimed != Some(k.id) {
                let dir = (k.pos - p.body.pos).normalize();
                p.body.pos = k.pos - dir * 4.5 + p.body.up * 0.5;
                p.body.vel = Vec3::ZERO;
                state.2 = t;
                info!("boss demo: stepped up to the King");
            }
            if mobs.aimed == Some(k.id) && k.hit_cd <= 0.0 {
                mouse.press(MouseButton::Left);
                state.3 += 1;
                info!(
                    "boss demo: swing {} (King hp {:.0}, phase {})",
                    state.3, k.hp, k.boss.phase
                );
                state.1 = t + 0.1;
                state.0 = state.0 + 10;
                return;
            }
            // The fight moves on by what the King's life is.
            if state.0 == 4 && state.3 >= 3 && k.hp > 101.0 {
                mobs.list
                    .iter_mut()
                    .find(|s| s.mob.id == k.id)
                    .unwrap()
                    .mob
                    .hp = 101.0;
                info!(
                    "boss demo: (life set to 101 to shorten the fight: the next hit crosses half)"
                );
            }
            if state.0 == 5 && k.boss.phase == 2 && t - state.2 > 14.0 {
                mobs.list
                    .iter_mut()
                    .find(|s| s.mob.id == k.id)
                    .unwrap()
                    .mob
                    .hp = 6.0;
                info!("boss demo: (life set to 6: the next hit kills it)");
                state.0 = 6;
            }
            state.1 = t + 0.25;
        }
        14 | 15 | 16 => {
            mouse.release(MouseButton::Left);
            let k = mobs
                .list
                .iter()
                .find(|s| s.mob.kind == Kind::SlimeKing)
                .map(|s| s.mob.clone());
            let was = state.0 - 10;
            match k {
                Some(k) if k.boss.phase == 2 && was == 4 => {
                    info!("boss demo: PHASE 2 reached (hp {:.0})", k.hp);
                    shot(&mut commands, "4_phase_two");
                    state.0 = 5;
                    state.2 = t;
                    state.1 = t + 0.4;
                }
                Some(_) => {
                    state.0 = was;
                    state.1 = t + 0.5;
                }
                None => {
                    state.0 = 6;
                    state.1 = t + 0.5;
                }
            }
        }
        _ => {}
    }
}

fn mining_demo(
    mut commands: Commands,
    mut player: Single<&mut Player>,
    mut space: ResMut<crate::world::Space>,
    mut drops: ResMut<crate::inventory::Drops>,
    mining: Res<crate::mining::Mining>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32, f32, usize)>,
    mut fixed: Local<Option<usize>>,
) {
    use orbit_core::block;
    use orbit_core::inventory::{GameMode, Stack};
    use orbit_core::items;
    let t = time.elapsed_secs();
    if t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let Some(id) = p.body.gravity_source else {
        return;
    };
    let Some(slot) = space.planets.get_mut(&id) else {
        return;
    };
    let Some(v) = slot.voxel.clone() else { return };
    // Puts `b` in the cell first looked at (found once, kept: it is the first one the ray meets
    // while it holds a block) and returns it.
    let mut aim = |slot: &mut crate::world::PlanetSlot, p: &Player, b: block::Block| {
        let cell = match *fixed {
            Some(c) => c,
            None => {
                let Some(hit) = v.read().unwrap().raycast(
                    p.body.eye(),
                    p.body.look_dir(),
                    orbit_core::player::REACH,
                ) else {
                    info!("mining demo: nothing in reach");
                    return None;
                };
                *fixed = Some(hit.cell);
                hit.cell
            }
        };
        let keys = v.write().unwrap().set(cell, b);
        crate::world::remesh(slot, &keys);
        Some(cell)
    };
    const TRIALS: [(block::Block, block::Block); 3] = [
        (items::WOODEN_PICKAXE, block::STONE),
        (items::STONE_PICKAXE, block::IRON_ORE),
        (items::COPPER_PICKAXE, block::IRON_ORE),
    ];
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Creative;
            p.inv.slots = [None; orbit_core::inventory::SLOTS];
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.7;
            *state = (1, t + 1.0, 0.0, 0);
        }
        1 => {
            state.3 = aim(slot, p, block::STONE).unwrap_or(0);
            info!("mining demo: creative, stone placed, clicking");
            mouse.press(MouseButton::Left);
            *state = (2, t + 0.3, 0.0, state.3);
        }
        2 => {
            mouse.release(MouseButton::Left);
            let b = v.read().unwrap().get(state.3);
            info!(
                "mining demo: creative click: stone is now {} ({})",
                block::name(b),
                if b == block::AIR {
                    "instant: OK"
                } else {
                    "FAIL"
                }
            );
            p.inv.mode = GameMode::Survival;
            *state = (3, t + 0.5, 0.0, 0);
        }
        3 => {
            state.3 = aim(slot, p, block::STONE).unwrap_or(0);
            mouse.press(MouseButton::Left);
            info!(
                "mining demo: survival, bare hands on stone (mining time {:.1} s)",
                orbit_core::mining::mining_time(block::STONE, None)
            );
            *state = (4, t + 2.0, t, state.3);
        }
        4 => {
            info!(
                "mining demo: bare hands after {:.1} s: progress {:.0}%",
                t - state.2,
                mining.progress() * 100.0
            );
            shot(&mut commands, "1_bare_hands_bar");
            *state = (5, t + 1.0, state.2, state.3);
        }
        5 => {
            mouse.release(MouseButton::Left);
            let b = v.read().unwrap().get(state.3);
            info!(
                "mining demo: bare hands after {:.1} s: stone is {} ({}), drops {}",
                t - state.2,
                block::name(b),
                if b == block::STONE {
                    "not broken: OK"
                } else {
                    "FAIL"
                },
                drops.list.len()
            );
            *state = (10, t + 0.5, 0.0, 0);
        }
        // Trial n: put the tool in hand, the block before it, hold the click until it goes.
        n @ 10..=19 if ((n - 10) as usize) < TRIALS.len() => {
            let (tool, target) = TRIALS[(n - 10) as usize];
            p.inv.slots[0] = Some(Stack::new(tool, 1));
            drops.list.clear();
            let cell = aim(slot, p, target).unwrap_or(0);
            mouse.press(MouseButton::Left);
            info!(
                "mining demo: {} on {} (expected {:.1} s, drops {:?})",
                items::name(tool),
                block::name(target),
                orbit_core::mining::mining_time(target, Some(tool)),
                orbit_core::mining::drops(target, Some(tool), 0).map(block::name)
            );
            *state = (n + 100, t + 0.1, t, cell);
        }
        n @ 110..=119 => {
            let (tool, target) = TRIALS[(n - 110) as usize];
            let b = v.read().unwrap().get(state.3);
            if b != target || t - state.2 > 16.0 {
                mouse.release(MouseButton::Left);
                let held = p.inv.slots[0].map(|s| (items::name(s.item), s.wear));
                let left: Vec<_> = drops
                    .list
                    .iter()
                    .map(|d| block::name(d.d.stack.item))
                    .collect();
                info!(
                    "mining demo: {} on {}: {} after {:.1} s; tool {:?}; drops {:?}",
                    items::name(tool),
                    block::name(target),
                    if b != target { "broken" } else { "NOT broken" },
                    t - state.2,
                    held,
                    left
                );
                shot(
                    &mut commands,
                    &format!("{}_{}", n - 108, items::name(tool).replace(' ', "_")),
                );
                *state = (n - 99, t + 0.7, 0.0, 0);
            }
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=items` (with `ORBIT_SHOTS`): `/give`s every new item, shows the hotbar with a worn
/// pickaxe, a crafting table and a furnace standing in the world, items on the ground, then the
/// inventory screen.
fn items_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    mut space: ResMut<crate::world::Space>,
    mut drops: ResMut<crate::inventory::Drops>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    screen: Res<crate::inventory::Screen>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::{block, items};
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            type_keys(&mut demo, "/gamemode survival\n/clear\n");
            for g in [
                "stick 5",
                "coal 12",
                "copper_ingot 3",
                "iron_ingot 3",
                "apple 4",
                "iron_pickaxe",
                "stone_sword",
                "crafting_table",
                "furnace",
            ] {
                type_keys(&mut demo, &format!("/give {g}\n"));
            }
            *state = (1, t + 0.5);
        }
        1 if p.inv.slots[8].is_some() => {
            let hot: Vec<_> = p.inv.slots[..9]
                .iter()
                .map(|s| s.map(|s| (block::name(s.item), s.count)))
                .collect();
            info!("items demo: after the /gives: {hot:?}");
            p.inv.slots[5].as_mut().unwrap().wear = 100;
            p.inv.slots[6].as_mut().unwrap().wear = 125;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.6;
            *state = (11, t + 0.5);
        }
        11 => {
            let Some(id) = p.body.gravity_source else {
                return;
            };
            let Some(slot) = space.planets.get_mut(&id) else {
                return;
            };
            let Some(v) = slot.voxel.clone() else { return };
            let hit = v.read().unwrap().raycast(
                p.body.eye(),
                p.body.look_dir(),
                orbit_core::player::REACH,
            );
            let Some(hit) = hit else { return };
            let mut g = v.write().unwrap();
            let a = g.above(hit.cell).unwrap_or(hit.cell);
            let b = g.side(a, block::Dir(0)).unwrap_or(a);
            let c = g.above(a).unwrap_or(a);
            let mut dirty = Vec::new();
            dirty.extend(g.set(a, block::CRAFTING_TABLE));
            dirty.extend(g.set(b, block::FURNACE));
            let up = p.body.pos.normalize();
            let at = g.grid.center(c);
            drop(g);
            crate::world::remesh(slot, &dirty);
            for (i, item) in [
                items::IRON_PICKAXE,
                items::APPLE,
                items::COAL,
                items::STONE_SWORD,
            ]
            .into_iter()
            .enumerate()
            {
                drops.pop(
                    crate::inventory::Frame::Planet(id),
                    at + up * 0.35 * i as f32,
                    up,
                    item,
                );
            }
            info!("items demo: table at {a}, furnace at {b}");
            *state = (2, t + 2.5);
        }
        2 => {
            // A stray E typed into the chat may have opened the screen: Escape closes it.
            if screen.is_open() {
                type_keys(&mut demo, "\x1b");
                *state = (2, t + 0.5);
            } else {
                shot(&mut commands, "1_hotbar_and_world");
                *state = (12, t + 1.0);
            }
        }
        12 => {
            keys.press(KeyCode::KeyE);
            *state = (13, t + 0.3);
        }
        13 => {
            keys.release(KeyCode::KeyE);
            *state = (4, t + 0.8);
        }
        4 => {
            shot(&mut commands, "2_inventory");
            *state = (5, t + 1.0);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=craft` (with `ORBIT_SHOTS`): logs into planks, sticks and a crafting table on the
/// inventory screen's 2 × 2 grid; a wooden pickaxe is not made there; the table is placed and
/// opened with a right click; the pickaxe is made on its 3 × 3 grid. Ingredients are counted.
#[allow(clippy::too_many_arguments)]
fn craft_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    screen: Res<crate::inventory::Screen>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::inventory::{CRAFT, GameMode, RESULT, SLOTS, Stack};
    use orbit_core::{block, items};
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let count = |p: &Player, item: block::Block| -> u32 {
        p.inv
            .slots
            .iter()
            .chain(p.inv.craft.iter())
            .chain([&p.inv.cursor])
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count as u32)
            .sum()
    };
    let slot_of = |p: &Player, item: block::Block| {
        (0..SLOTS)
            .find(|&i| p.inv.slots[i].is_some_and(|s| s.item == item))
            .unwrap_or(0)
    };
    let report = |p: &Player, what: &str| {
        info!(
            "craft demo: {what}: logs {} planks {} sticks {} tables {} wooden pickaxes {}",
            count(p, block::WOOD),
            count(p, block::PLANKS),
            count(p, items::STICK),
            count(p, block::CRAFTING_TABLE),
            count(p, items::WOODEN_PICKAXE)
        );
    };
    // One of what is in bag slot `from` onto grid cell `cell`, with the clicks a player makes.
    let put_one = |p: &mut Player, from: usize, cell: usize| {
        p.inv.click(from);
        p.inv.right_click(CRAFT + cell);
        p.inv.click(from);
    };
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            p.inv.add(block::WOOD, 3);
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.7;
            report(p, "start");
            keys.press(KeyCode::KeyE);
            *state = (1, t + 0.3);
        }
        1 => {
            keys.release(KeyCode::KeyE);
            info!(
                "craft demo: inventory screen open: {} ({:?}, grid {}x{})",
                screen.is_open(),
                screen.kind(),
                p.inv.craft_size,
                p.inv.craft_size
            );
            // All three logs onto cell 0 of the 2 x 2 grid: planks come out, a log at a time.
            p.inv.click(0);
            p.inv.click(CRAFT);
            info!(
                "craft demo: 3 logs on the grid, result {:?}",
                p.inv.at(RESULT).map(|s| (block::name(s.item), s.count))
            );
            *state = (2, t + 0.6);
        }
        2 => {
            shot(&mut commands, "1_inventory_2x2_planks");
            p.inv.quick_move(RESULT);
            report(p, "after shift-clicking the result (planks)");
            // Two planks one over the other: sticks.
            let pl = slot_of(p, block::PLANKS);
            put_one(p, pl, 0);
            put_one(p, pl, 3);
            info!(
                "craft demo: planks over planks, result {:?}",
                p.inv.at(RESULT).map(|s| (block::name(s.item), s.count))
            );
            p.inv.quick_move(RESULT);
            report(p, "after sticks");
            // Four planks: the table.
            let pl = slot_of(p, block::PLANKS);
            for cell in [0, 1, 3, 4] {
                put_one(p, pl, cell);
            }
            info!(
                "craft demo: 2x2 of planks, result {:?}",
                p.inv.at(RESULT).map(|s| (block::name(s.item), s.count))
            );
            p.inv.quick_move(RESULT);
            report(p, "after the table");
            // The pickaxe laid out on the 3 x 3 cells, the 2 x 2 grid sees only a corner of it.
            let pl = slot_of(p, block::PLANKS);
            let st = slot_of(p, items::STICK);
            for cell in [0, 1, 2] {
                put_one(p, pl, cell);
            }
            put_one(p, st, 4);
            put_one(p, st, 7);
            info!(
                "craft demo: pickaxe laid out but no table (grid {0}x{0}): result {1:?}",
                p.inv.craft_size,
                p.inv.at(RESULT).map(|s| block::name(s.item))
            );
            *state = (3, t + 0.6);
        }
        3 => {
            shot(&mut commands, "2_no_pickaxe_without_table");
            type_keys(&mut demo, "\x1b");
            *state = (4, t + 0.5);
        }
        4 => {
            report(p, "screen closed, grid returned");
            // Place the table: it is the item with the highest slot; select it.
            p.inv.selected = (0..9)
                .find(|&i| p.inv.slots[i].is_some_and(|s| s.item == block::CRAFTING_TABLE))
                .unwrap_or(0);
            *state = (5, t + 0.4);
        }
        5 => {
            mouse.press(MouseButton::Right);
            *state = (6, t + 0.2);
        }
        6 => {
            mouse.release(MouseButton::Right);
            report(p, "after right clicking with the table in hand (placed)");
            *state = (7, t + 0.8);
        }
        7 => {
            // Right click again: now on the table itself.
            mouse.press(MouseButton::Right);
            *state = (8, t + 0.2);
        }
        8 => {
            mouse.release(MouseButton::Right);
            *state = (9, t + 0.5);
        }
        9 => {
            info!(
                "craft demo: right click on the table: screen open {}, kind {:?}, grid {}x{}",
                screen.is_open(),
                screen.kind(),
                p.inv.craft_size,
                p.inv.craft_size
            );
            let pl = slot_of(p, block::PLANKS);
            let st = slot_of(p, items::STICK);
            for cell in [0, 1, 2] {
                put_one(p, pl, cell);
            }
            put_one(p, st, 4);
            put_one(p, st, 7);
            info!(
                "craft demo: pickaxe laid out on the table (grid {0}x{0}): result {1:?}",
                p.inv.craft_size,
                p.inv.at(RESULT).map(|s| block::name(s.item))
            );
            *state = (10, t + 0.6);
        }
        10 => {
            shot(&mut commands, "3_table_3x3_pickaxe");
            *state = (11, t + 0.8);
        }
        11 => {
            p.inv.quick_move(RESULT);
            report(p, "after taking the pickaxe");
            *state = (12, t + 0.6);
        }
        12 => {
            shot(&mut commands, "4_table_after");
            let _ = Stack::new(block::DIRT, 1);
            *state = (12, t + 1.0);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=furnace` (with `ORBIT_SHOTS`, and `ORBIT_SMELT_SPEED=8` to hurry it): places a
/// furnace, opens it with a right click, puts iron ore and coal in, watches ingots come out and
/// the coal go down, takes them, then breaks the furnace with a pickaxe: what it held falls out.
#[allow(clippy::too_many_arguments)]
fn furnace_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    mut furnaces: ResMut<crate::furnace::Furnaces>,
    drops: Res<crate::inventory::Drops>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    screen: Res<crate::inventory::Screen>,
    time: Res<Time>,
    mut state: Local<(u32, f32, f32)>,
) {
    use orbit_core::furnace::Slot;
    use orbit_core::inventory::{GameMode, SLOTS};
    use orbit_core::{block, items};
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let slot_of = |p: &Player, item: block::Block| {
        (0..SLOTS).find(|&i| p.inv.slots[i].is_some_and(|s| s.item == item))
    };
    let count = |p: &Player, item: block::Block| -> u32 {
        p.inv
            .slots
            .iter()
            .chain([&p.inv.cursor])
            .flatten()
            .filter(|s| s.item == item)
            .map(|s| s.count as u32)
            .sum()
    };
    let show = |f: &orbit_core::furnace::Furnace| {
        format!(
            "input {:?} fuel {:?} output {:?} progress {:.1}s burn_left {:.0}s",
            f.input.map(|s| (block::name(s.item), s.count)),
            f.fuel.map(|s| (block::name(s.item), s.count)),
            f.output.map(|s| (block::name(s.item), s.count)),
            f.progress,
            f.burn_left
        )
    };
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            p.inv.add(block::FURNACE, 1);
            p.inv.add(block::IRON_ORE, 4);
            p.inv.add(items::COAL, 2);
            p.inv.add(items::STONE_PICKAXE, 1);
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.7;
            *state = (1, t + 0.5, 0.0);
        }
        1 => {
            mouse.press(MouseButton::Right);
            *state = (2, t + 0.2, 0.0);
        }
        2 => {
            mouse.release(MouseButton::Right);
            info!(
                "furnace demo: furnace placed, furnaces in hand: {}",
                count(p, 0)
                    + p.inv
                        .slots
                        .iter()
                        .flatten()
                        .filter(|s| s.item == block::FURNACE)
                        .count() as u32
            );
            *state = (3, t + 0.8, 0.0);
        }
        3 => {
            mouse.press(MouseButton::Right);
            *state = (4, t + 0.2, 0.0);
        }
        4 => {
            mouse.release(MouseButton::Right);
            *state = (5, t + 0.5, 0.0);
        }
        5 => {
            info!(
                "furnace demo: right click on the furnace: screen open {}, kind {:?}",
                screen.is_open(),
                screen.kind()
            );
            let Some(key) = screen.furnace.clone() else {
                return;
            };
            // Iron ore into the input, coal into the fuel, with the clicks of the screen.
            let ore = slot_of(p, block::IRON_ORE).unwrap_or(0);
            p.inv.click(ore);
            furnaces
                .map
                .get_mut(&key)
                .unwrap()
                .click(Slot::Input, &mut p.inv.cursor);
            let coal = slot_of(p, items::COAL).unwrap_or(0);
            p.inv.click(coal);
            furnaces
                .map
                .get_mut(&key)
                .unwrap()
                .click(Slot::Fuel, &mut p.inv.cursor);
            info!("furnace demo: loaded: {}", show(&furnaces.map[&key]));
            *state = (6, t + 0.5, t);
        }
        6 => {
            let key = screen.furnace.clone().unwrap();
            let f = &furnaces.map[&key];
            info!("furnace demo: after {:.1} s: {}", t - state.2, show(f));
            if state.0 == 6 && f.output.is_some_and(|o| o.count >= 1) && f.progress > 0.2 {
                shot(&mut commands, "1_furnace_working");
                *state = (7, t + 0.8, state.2);
            } else {
                state.1 = t + 0.5;
            }
        }
        7 => {
            let key = screen.furnace.clone().unwrap();
            let f = &furnaces.map[&key];
            info!("furnace demo: after {:.1} s: {}", t - state.2, show(f));
            if f.output.is_some_and(|o| o.count >= 3) {
                *state = (8, t + 0.3, state.2);
            } else {
                state.1 = t + 0.7;
            }
        }
        8 => {
            let key = screen.furnace.clone().unwrap();
            // Take the ingots with the output slot's click, put them in the bag.
            let f = furnaces.map.get_mut(&key).unwrap();
            f.click(Slot::Output, &mut p.inv.cursor);
            let n = p.inv.cursor.map_or(0, |s| s.count);
            if let Some(c) = p.inv.cursor.take() {
                p.inv.add_stack(c);
            }
            info!(
                "furnace demo: took {n} ingots: bag has {} iron ingots; {}",
                count(p, items::IRON_INGOT),
                show(&furnaces.map[&key])
            );
            shot(&mut commands, "2_furnace_after_taking");
            type_keys(&mut demo, "\x1b");
            *state = (9, t + 0.8, 0.0);
        }
        9 => {
            // Break it with the pickaxe: hold the left click until the furnace is gone.
            p.inv.selected = slot_of(p, items::STONE_PICKAXE).unwrap_or(0).min(8);
            mouse.press(MouseButton::Left);
            *state = (10, t + 0.5, t);
        }
        10 => {
            if furnaces.map.is_empty() || t - state.2 > 10.0 {
                mouse.release(MouseButton::Left);
                let on_ground: Vec<_> = drops
                    .list
                    .iter()
                    .map(|d| (block::name(d.d.stack.item), d.d.stack.count))
                    .collect();
                info!(
                    "furnace demo: furnace broken after {:.1} s, furnaces left {}, on the ground {on_ground:?}",
                    t - state.2,
                    furnaces.map.len()
                );
                *state = (11, t + 1.5, 0.0);
            }
        }
        11 => {
            shot(&mut commands, "3_furnace_broken");
            *state = (12, t + 1.0, 0.0);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=food` (with `ORBIT_SHOTS`): apples in the main hand, torches in the left. At full
/// health a right click eats nothing and the left hand places a torch; after `/damage 8` each
/// right click eats an apple (4 hp) and the torches are left alone.
#[allow(clippy::too_many_arguments)]
fn food_demo(
    mut commands: Commands,
    mut demo: ResMut<Demo>,
    mut player: Single<&mut Player>,
    vitals: Res<crate::health::Vitals>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS, Stack};
    use orbit_core::{block, items};
    let t = time.elapsed_secs();
    if !demo.typing.is_empty() || t < state.1 {
        return;
    }
    let shot = |commands: &mut Commands, name: &str| {
        if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
            commands
                .spawn(Screenshot::primary_window())
                .observe(save_to_disk(format!("{dir}/{name}.png")));
        }
    };
    let p = &mut **player;
    let show = |p: &Player, hp: f32, what: &str| {
        info!(
            "food demo: {what}: hp {hp:.0}, apples {}, left hand {:?}",
            p.inv
                .held()
                .map_or(0, |s| if s.item == items::APPLE { s.count } else { 0 }),
            p.inv.offhand.map(|s| (block::name(s.item), s.count))
        );
    };
    let hp = vitals.health.hp;
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            p.inv.slots[0] = Some(Stack::new(items::APPLE, 3));
            p.inv.offhand = Some(Stack::new(block::TORCH, 5));
            p.inv.selected = 0;
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.7;
            *state = (1, t + 0.8);
        }
        1 => {
            show(p, hp, "start (full health)");
            mouse.press(MouseButton::Right);
            *state = (2, t + 0.2);
        }
        2 => {
            mouse.release(MouseButton::Right);
            *state = (3, t + 0.4);
        }
        3 => {
            show(
                p,
                hp,
                "right click at full health: apple kept, left hand placed a torch",
            );
            type_keys(&mut demo, "/damage 8\n");
            *state = (4, t + 0.5);
        }
        4 => {
            show(p, hp, "after /damage 8");
            shot(&mut commands, "1_hurt");
            *state = (5, t + 0.6);
        }
        5 => {
            mouse.press(MouseButton::Right);
            *state = (6, t + 0.2);
        }
        6 => {
            mouse.release(MouseButton::Right);
            *state = (7, t + 0.4);
        }
        7 => {
            show(
                p,
                hp,
                "after eating one apple (expected hp +4, apples -1, torches unchanged)",
            );
            shot(&mut commands, "2_ate");
            *state = (8, t + 0.6);
        }
        8 => {
            mouse.press(MouseButton::Right);
            *state = (9, t + 0.2);
        }
        9 => {
            mouse.release(MouseButton::Right);
            *state = (10, t + 0.4);
        }
        10 => {
            show(p, hp, "after eating a second apple");
            mouse.press(MouseButton::Right);
            *state = (11, t + 0.2);
        }
        11 => {
            mouse.release(MouseButton::Right);
            *state = (12, t + 0.4);
        }
        12 => {
            show(
                p,
                hp,
                "right click at full health again: apple kept, torch placed",
            );
            *state = (13, t + 1.0);
        }
        _ => {}
    }
}

/// `ORBIT_DEMO=held` (with `ORBIT_SHOTS`): a screenshot of the hand with each kind of item in it.
fn held_demo(
    mut commands: Commands,
    mut player: Single<&mut Player>,
    mut cursor: Single<&mut bevy::window::CursorOptions, With<PrimaryWindow>>,
    time: Res<Time>,
    mut state: Local<(u32, f32)>,
) {
    use orbit_core::inventory::{GameMode, SLOTS, Stack};
    use orbit_core::{block, items};
    let t = time.elapsed_secs();
    if t < state.1 {
        return;
    }
    let p = &mut **player;
    let list = [
        items::IRON_PICKAXE,
        items::STONE_SWORD,
        items::APPLE,
        items::COAL,
        items::COPPER_INGOT,
        items::STICK,
        block::FURNACE,
    ];
    match state.0 {
        0 if p.body.on_ground && t > 4.0 => {
            p.inv.mode = GameMode::Survival;
            p.inv.slots = [None; SLOTS];
            for (i, &it) in list.iter().enumerate() {
                p.inv.slots[i] = Some(Stack::new(it, 1));
            }
            cursor.grab_mode = bevy::window::CursorGrabMode::Locked;
            p.body.pitch = -0.2;
            *state = (1, t + 1.0);
        }
        n @ 1..=7 => {
            p.inv.selected = (n - 1) as usize;
            *state = (n + 10, t + 0.6);
        }
        n @ 11..=17 => {
            if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_to_disk(format!(
                        "{dir}/held_{}.png",
                        block::name(list[(n - 11) as usize]).replace(' ', "_")
                    )));
            }
            *state = (n - 9, t + 0.3);
        }
        _ => {}
    }
}
