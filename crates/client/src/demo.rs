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
use bevy::prelude::*;
use bevy::input::keyboard::{Key, KeyboardInput, NativeKeyCode};
use bevy::input::ButtonState;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::window::PrimaryWindow;

pub struct DemoPlugin;

impl Plugin for DemoPlugin {
    fn build(&self, app: &mut App) {
        let Ok(mode) = std::env::var("ORBIT_DEMO") else { return };
        let map = mode == "map";
        let look = mode == "orbit";
        let water = mode == "water";
        let blocks = mode == "blocks";
        let stay = mode == "stay" || water || blocks;
        let elevator = mode == "elevator";
        let station = mode == "station" || elevator;
        let rocket = mode == "rocket";
        let mode = if mode == "fly" { Mode::Fly } else { Mode::Warp };
        app.insert_resource(Demo { mode, map, look, station, elevator, rocket, typing: Vec::new(), since: 0.0, step: if stay { 100 } else { 0 }, wait: 0.0, shot: 0.0, n: 0, jitter: (0.0, 0.0), frames: 0 }).add_systems(Update, run.before(crate::map::toggle).before(crate::elevator::run_elevators).before(crate::elevator::run_placing));
        if water {
            app.add_systems(Update, water_demo.after(crate::player::move_player).before(crate::player::sync_camera));
        }
        if blocks {
            app.add_systems(Update, blocks_demo.after(crate::player::move_player).before(crate::player::sync_camera).before(crate::inventory::open_screen));
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
    let Some(slot) = b.gravity_source.and_then(|id| space.planets.get_mut(&id)) else { return };
    let Some(v) = slot.voxel.clone() else { return };
    let mut g = v.write().unwrap();
    let up = b.pos.normalize_or(Vec3::Y);
    let ahead = b.forward.reject_from(up).normalize_or(up.any_orthonormal_vector());
    let Some(lay) = orbit_core::rocket::Lay::at(&g, b.pos + ahead * 9.0, ahead) else { return };
    let Some(k) = orbit_core::rocket::ground(&g, lay.face, lay.i0, lay.j0) else { return };
    let lay = orbit_core::rocket::Lay { k0: k, ..lay };
    let mut dirty = Vec::new();
    for x in -4..=4 {
        for z in -4..=4 {
            for y in 0..14 {
                if let Some(c) = lay.cell(&g, IVec3::new(x, y, z)) {
                    dirty.extend(g.set(c, if y == 0 { orbit_core::block::PAD } else { orbit_core::block::AIR }));
                }
            }
        }
    }
    let Some(c) = lay.cell(&g, IVec3::ZERO) else { return };
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
    std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d)
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
    (mut keys, mut mouse): (ResMut<ButtonInput<KeyCode>>, ResMut<ButtonInput<MouseButton>>),
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
            Key::Character(c) => (KeyCode::Unidentified(NativeKeyCode::Unidentified), Some(c.clone())),
            _ => (KeyCode::Unidentified(NativeKeyCode::Unidentified), None),
        };
        typed.write(KeyboardInput { key_code, logical_key: k, state: ButtonState::Pressed, text, repeat: false, window: *window });
        return;
    }
    if let Ok(dir) = std::env::var("ORBIT_SHOTS") {
        demo.shot -= dt;
        if demo.shot <= 0.0 {
            demo.shot = env_f32("ORBIT_SHOT_EVERY", 2.0);
            let path = format!("{dir}/shot_{:04}_{:.0}s.png", demo.n, t);
            demo.n += 1;
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
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
            demo.step = if demo.rocket { 40 } else if demo.elevator { 60 } else { 1 };
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
            let Some(p) = b.gravity_source.and_then(|id| space.planets.get(&id)) else { return };
            let Some(v) = p.voxel.as_ref() else { return };
            let g = v.read().unwrap();
            let mut helm = None;
            for x in -32..=32 {
                for y in -32..=32 {
                    for z in -32..=32 {
                        if let Some(c) = g.grid.cell_at(b.pos + Vec3::new(x as f32, y as f32, z as f32)) {
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
            if let (Some(cell), true) = (helm, std::env::var("ORBIT_DEMO_ROCKET_ROOF").is_ok_and(|x| x == "1")) {
                let mut g = v.write().unwrap();
                let (f, i, j, k) = g.grid.coords(cell);
                if let Some(over) = g.grid.cell_on_face(f, i as i64, j as i64, k as i64 + 10) {
                    g.set(over, orbit_core::block::STONE);
                }
                demo.step = 99;
                demo.wait = 0.0;
                rockets.requests.push(crate::rocket::Request::Assemble { planet: p.spec.id, cell });
                return;
            }
            if let Some(cell) = helm {
                rockets.requests.push(crate::rocket::Request::Assemble { planet: p.spec.id, cell });
            }
            demo.step = 42;
            demo.wait = 4.0;
        }
        42 => {
            info!("demo: at the helm: {} ({})", rockets.is_piloting(), travel.message);
            travel.requests.push(Request::Launch);
            demo.step = 43;
            demo.wait = 2.0;
        }
        43 if travel.job.is_none() && b.orbiting.is_some() => {
            let alt = b.planet().and_then(|id| space.planets.get(&id)).map_or(0.0, |p| b.point_from(b.pos, p.spec.center).length() - p.spec.radius);
            info!("demo: rocket in orbit at {t:.0}s, {alt:.0} blocks up ({})", travel.message);
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
            info!("demo: rocket on the station at {t:.0}s: on {:?} ({})", b.station, travel.message);
            demo.step = 53;
            demo.wait = 4.0;
        }
        52 => info!("demo: docking: {:?} ({})", travel.job.as_ref().map(|_| "on"), travel.message),
        53 => {
            let helm = b.station.as_ref().and_then(|id| stations.by_id(id)).and_then(|s| {
                let s = s.station.read().unwrap();
                s.cells().find(|&(_, k)| k == orbit_core::block::HELM).map(|(p, _)| (s.id.clone(), p))
            });
            if let Some((station, cell)) = helm {
                rockets.requests.push(crate::rocket::Request::AssembleOnStation { station, cell });
            }
            demo.step = 54;
            demo.wait = 2.0;
        }
        54 => {
            info!("demo: off the station again: piloting {} ({})", rockets.is_piloting(), travel.message);
            demo.step = 99;
        }
        44 => {
            rockets.cockpit = false;
            rockets.requests.push(crate::rocket::Request::Release);
            demo.step = 45;
            demo.wait = 7.0;
        }
        45 => {
            let parked: Vec<String> = stations.list.iter().map(|s| s.station.read().unwrap()).filter(|s| s.is_rocket()).map(|s| format!("{} orbit {:?}", s.name, s.orbit.map(|o| o.radius()))).collect();
            info!("demo: parked? piloting {}, on {:?}, ground {}, rockets {parked:?} ({})", rockets.is_piloting(), b.station, b.on_ground, travel.message);
            let id = stations.list.iter().map(|s| s.station.read().unwrap()).find(|s| s.is_rocket()).map(|s| s.id.clone());
            if let Some(id) = id {
                rockets.requests.push(crate::rocket::Request::Take(id));
            }
            demo.step = 46;
            demo.wait = 3.0;
        }
        46 => {
            info!("demo: took the helm again: {} ({})", rockets.is_piloting(), travel.message);
            travel.requests.push(Request::Land);
            demo.step = 47;
            demo.wait = 2.0;
        }
        47 if travel.job.is_none() && !rockets.is_piloting() => {
            info!("demo: landed and taken apart at {t:.0}s: on {:?}, ground {} ({})", b.gravity_source, b.on_ground, travel.message);
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
            let Some(p) = space.planets.get(&id) else { return };
            let Some(v) = p.voxel.as_ref() else { return };
            let axis = p.spec.spin.axis;
            let eq = b.pos.reject_from(axis).normalize();
            let ground = {
                let planet = v.read().unwrap();
                orbit_core::elevator::surface_layer(&planet, eq).map(|k| planet.grid.radius(k + 1)).unwrap_or(p.spec.radius)
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
            let lat = b.gravity_source.and_then(|id| space.planets.get(&id)).map_or(0.0, |p| b.pos.normalize().dot(p.spec.spin.axis).asin() * p.spec.radius);
            info!("demo: hologram: {} (feet {lat:.1} blocks off the equator, placing {})", elevators.hud, elevators.placing.is_some());
            let body = &mut player.body;
            body.pitch = -0.35;
            mouse.press(MouseButton::Right);
            demo.step = 63;
            demo.wait = 1.0;
        }
        63 => {
            mouse.release(MouseButton::Right);
            info!("demo: elevators {}, stations {}", elevators.list.len(), stations.list.len());
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
            let Some(slot) = elevators.list.last() else { return };
            let Some(door) = crate::elevator::ground_door(slot) else { return };
            let Some(mid) = crate::elevator::ground_middle(slot) else { return };
            player.body.pos = mid + (door - mid) * 0.3;
            demo.since = t;
            demo.step = 65;
            demo.wait = 0.5;
        }
        65 => {
            let Some(slot) = elevators.list.last() else { return };
            let Some(mid) = crate::elevator::ground_middle(slot) else { return };
            info!("demo: put inside the cabin, now {:.2} from its middle (pushed out past {:.2})", (b.pos - mid).reject_from(b.up).length(), orbit_core::elevator::CABIN_HALF_X);
            // To the door, facing the cabin, and in.
            let Some(door) = crate::elevator::ground_door(slot) else { return };
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
                elevators.outside = std::env::var("ORBIT_DEMO_ELEVATOR_OUTSIDE").is_ok_and(|v| v == "1");
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
                info!("demo: up after {:.0} s: on station {:?}, on ground {}", t - demo.since, player.body.station, player.body.on_ground);
                demo.step = 68;
                demo.wait = 2.0;
            }
        }
        68 => {
            // Walk back into the docked cabin from its door: pushed back out again.
            let Some(slot) = elevators.list.last() else { return };
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
            let Some(slot) = elevators.list.last() else { return };
            let mid = slot.e.dock() + Vec3::Y * orbit_core::elevator::CABIN_FLOOR;
            info!("demo: on the deck, put in the cabin: now {:.2} from its middle", (b.pos - mid).with_y(0.0).length());
            // Look round the terminal from its deck.
            player.body.forward = -player.body.forward;
            player.body.pos = Vec3::new(0.5, 1.0, -5.5);
            demo.step = 99;
        }
        21 if b.station.is_some() && b.on_ground => {
            info!("demo: standing on station {:?} at {:?} after {:.1}s", b.station, b.pos, t - demo.since);
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
            info!("demo: did not reach the deck by thrusters (orbiting {:?}); /station tp instead", b.orbiting);
            keys.release(KeyCode::KeyW);
            let id = stations.ids(Some(true)).first().cloned().unwrap_or_default();
            type_keys(&mut demo, &format!("/station tp {id}\n"));
            demo.since = t + 30.0; // no second try
        }
        22 => {
            // Walk off the deck's edge.
            keys.press(KeyCode::KeyW);
            if b.station.is_none() {
                keys.release(KeyCode::KeyW);
                info!("demo: off the station's edge, now orbiting {:?} at {:.1} m/s", b.orbiting, b.world_vel().length());
                demo.step = 23;
                demo.since = t;
            }
        }
        23 if t - demo.since > 4.0 => {
            let here = b.world_point(b.pos);
            let placed = stations.placed(&space, clock.0);
            let d = placed.first().map_or(f64::NAN, |(_, f)| f.center.distance(here));
            info!("demo: 4 s after walking off, {d:.1} blocks from the station (orbiting {:?})", b.orbiting);
            let id = placed.first().map(|(s, _)| s.id.clone()).unwrap_or_default();
            drop(placed);
            type_keys(&mut demo, &format!("/station tp {id}\n"));
            demo.step = 24;
            demo.wait = 3.0;
        }
        24 => {
            info!("demo: after /station tp: on station {:?}, on ground {}", b.station, b.on_ground);
            type_keys(&mut demo, "/time set noon\n/station list\n/statoin\n/station fly\n");
            demo.step = 30;
            demo.wait = 4.0;
        }
        30 => {
            info!("demo: 4 s after /time set noon: on station {:?}, on ground {}, feet {:?}", b.station, b.on_ground, b.pos);
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
            info!("demo: 3 s after Launch from the map: map open {}, cameras active {:?}, autopilot {:?}, {:.1} m/s", map.is_open(), cams, travel.job, b.world_vel().length());
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
                let mine = b.planet().and_then(|id| space.planets.get(&id)).map(|p| orbit_core::universe::Universe::sector_of(p.spec.star()));
                all.retain(|s| Some(s.sector) != mine);
                all.sort_by(|a, b| a.center.distance(here).total_cmp(&b.center.distance(here)));
                all.swap_remove(0)
            } else {
                space.universe.system(IVec3::ZERO).unwrap()
            };
            // ORBIT_DEMO_MOON=1: the home planet's moon.
            let moon = std::env::var("ORBIT_DEMO_MOON").is_ok_and(|v| v == "1");
            let first = sys.planets[0].id;
            let p = match sys.planets.iter().find(|m| moon && m.host.is_some_and(|h| h.id == first)) {
                Some(m) => m,
                None => &sys.planets[if far { 0 } else { 1 }],
            };
            info!("demo: in orbit at {t:.0}s, going to {} in {} ({:.0} km)", p.name, sys.name, p.center.distance(b.world_point(b.pos)) / 1000.0);
            travel.requests.push(Request::Go(Target { sector: sys.sector, planet: p.id }, demo.mode));
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
                let old = p.spec.rotation.inverse() * (b.world_point(local) - p.spec.center).as_vec3();
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
                    info!("demo: {:.0} km out, camera vs ground over 300 frames: world coords (f64) off by up to {:.4} m, drawn off by up to {:.6} m", b.frame_center().length() / 1000.0, demo.jitter.0, demo.jitter.1);
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
    let Some(slot) = b.gravity_source.and_then(|id| space.planets.get_mut(&id)) else { return };
    let Some(v) = slot.voxel.clone() else { return };
    let spec = slot.spec.clone();
    let (step, since) = (state.0, state.1);
    let at = |d: Vec3, up: f32, ground: i64| d * (spec.radius + ground as f32 + up);
    match step {
        // Find the sea from where we stand, and hover on its shore looking out over it.
        0 if b.on_ground || b.swimming.is_some() => {
            let terrain = Terrain::new(&spec);
            let here = b.pos.normalize();
            let (t1, t2) = (here.any_orthonormal_vector(), here.cross(here.any_orthonormal_vector()));
            let found = (1..200).find_map(|r| {
                (0..32).find_map(|k| {
                    let a = k as f32 / 32.0 * std::f32::consts::TAU;
                    let d = (here + (t1 * a.cos() + t2 * a.sin()) * (r as f32 / spec.radius)).normalize();
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
                let Some(mut c) = g.grid.cell_at(p) else { continue };
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
    let Some(slot) = space.planets.get_mut(&id) else { return };
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
            let dir_vec = |g: &orbit_core::planet::Planet, d: Dir| g.side(origin, d).map_or(Vec3::ZERO, |n| (g.grid.center(n) - g.grid.center(origin)).normalize());
            let ahead = Dir::ALL.into_iter().max_by(|a, b| dir_vec(&g, *a).dot(fwd).total_cmp(&dir_vec(&g, *b).dot(fwd))).unwrap();
            let right = if dir_vec(&g, ahead.turned()).dot(side) > 0.0 { ahead.turned() } else { ahead.turned().opposite() };
            // The cell `f` ahead, `x` to the right, `h` up from the feet, stepping cell to cell.
            let cell = |g: &orbit_core::planet::Planet, f: f32, x: f32, h: f32| {
                let mut c = origin;
                let step = |c: usize, d: Dir, n: i32| (0..n.abs()).fold(c, |c, _| g.side(c, if n < 0 { d.opposite() } else { d }).unwrap_or(c));
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
                    { let c = cell(&g, f as f32, x as f32, -1.0); dirty.extend(g.set(c, block::GRASS)); }
                    for h in 0..5 {
                        { let c = cell(&g, f as f32, x as f32, h as f32); dirty.extend(g.set(c, block::AIR)); }
                    }
                }
            }
            fn put(g: &mut orbit_core::planet::Planet, dirty: &mut Vec<orbit_core::planet::ChunkKey>, c: usize, item: block::Block, against: Against, look: Dir) {
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
            let toward = |g: &orbit_core::planet::Planet, a: usize, b: usize| Dir::ALL.into_iter().find(|&d| g.side(a, d) == Some(b)).unwrap_or(Dir(0));
            let away = {
                let (a, c) = (cell(&g, 5.0, 0.0, 0.0), cell(&g, 4.0, 0.0, 0.0));
                toward(&g, a, c) // from the wall back toward the player
            };
            // A wall of stone with a doorway, a door in it; torches on it either side.
            for x in -2..=2 {
                for h in 0..3 {
                    if x != 0 || h == 2 {
                        { let c = cell(&g, 6.0, x as f32, h as f32); let ag = Against::Below; put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite()); }
                    }
                }
            }
            { let c = cell(&g, 6.0, 0.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::DOOR, ag, away.opposite()); }
            for x in [-1.0, 1.0] {
                let (wall, at) = (cell(&g, 6.0, x, 1.0), cell(&g, 5.0, x, 1.0));
                let d = toward(&g, at, wall);
                { let c = cell(&g, 5.0, x, 1.0); let ag = Against::Side(d); put(&mut g, &mut dirty, c, block::TORCH, ag, away.opposite()); }
            }
            // An open door beside it, on its own.
            { let c = cell(&g, 4.0, -5.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::DOOR, ag, away.opposite()); }
            let c = cell(&g, 4.0, -5.0, 0.0);
            for (c, b) in shape::toggle(&*g, c) {
                dirty.extend(g.set(c, b));
            }
            // A pillar with a ladder up its face.
            for h in 0..4 {
                { let c = cell(&g, 4.0, -7.0, h as f32); let ag = Against::Below; put(&mut g, &mut dirty, c, block::COBBLESTONE, ag, away.opposite()); }
                let (wall, at) = (cell(&g, 4.0, -7.0, h as f32), cell(&g, 3.0, -7.0, h as f32));
                { let c = cell(&g, 3.0, -7.0, h as f32); let ag = Against::Side(toward(&g, at, wall)); put(&mut g, &mut dirty, c, block::LADDER, ag, away.opposite()); }
            }
            // Fences joined in a row and round a corner, to a stone block.
            for x in 3..=6 {
                { let c = cell(&g, 5.0, x as f32, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::FENCE, ag, away.opposite()); }
            }
            { let c = cell(&g, 4.0, 6.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::FENCE, ag, away.opposite()); }
            { let c = cell(&g, 3.0, 6.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite()); }
            { let c = cell(&g, 2.0, 6.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::TORCH, ag, away.opposite()); }
            // Panes between glass and stone, two high.
            for h in 0..2 {
                { let c = cell(&g, 7.0, 3.0, h as f32); let ag = Against::Below; put(&mut g, &mut dirty, c, block::STONE, ag, away.opposite()); }
                for x in 4..=6 {
                    { let c = cell(&g, 7.0, x as f32, h as f32); let ag = Against::Below; put(&mut g, &mut dirty, c, block::GLASS_PANE, ag, away.opposite()); }
                }
                { let c = cell(&g, 7.0, 7.0, h as f32); let ag = Against::Below; put(&mut g, &mut dirty, c, block::GLASS, ag, away.opposite()); }
            }
            // Trapdoors: one shut on the ground, one open.
            { let c = cell(&g, 3.0, -2.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::TRAPDOOR, ag, away.opposite()); }
            { let c = cell(&g, 3.0, 2.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::TRAPDOOR, ag, away.opposite()); }
            let c = cell(&g, 3.0, 2.0, 0.0);
            for (c, b) in shape::toggle(&*g, c) {
                dirty.extend(g.set(c, b));
            }
            // Flowers on the grass; one on stone would not go.
            { let c = cell(&g, 2.0, -3.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::POPPY, ag, away.opposite()); }
            { let c = cell(&g, 2.0, -4.0, 0.0); let ag = Against::Below; put(&mut g, &mut dirty, c, block::DANDELION, ag, away.opposite()); }
            // Bare dirt by the grass, for it to grow over.
            for x in -1..=1 {
                { let c = cell(&g, 1.0, x as f32, -1.0); dirty.extend(g.set(c, block::DIRT)); }
            }
            // A few items lying about.
            for (k, item) in [block::COBBLESTONE, block::DIRT, block::TORCH, block::DOOR, block::POPPY].into_iter().enumerate() {
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
