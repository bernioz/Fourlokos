//! Rockets in the game (the blocks and rules are `orbit_core::rocket`): putting them together on
//! the ground, flying them, parking them in orbit and landing them again.
//!
//! - **Build** one with blocks (Hull, Glass, Engine, Helm, Chest and any others), or put up the
//!   starter with `/rocket build [<name>]`. Right click its **Helm** (Shift + right click places a
//!   block on it instead): the blocks you placed that touch it become a rocket, if it stands on a
//!   flat 3 × 3 pad under open sky, with an engine.
//! - **Fly**: while you hold the helm your body stays at it and the rocket flies in its place, so
//!   everything a player can do in flight the rocket does: WASD / Space / Shift thrust (Ctrl
//!   boosts; on a planet it hovers, its nose turning the way it goes), the mouse turns the camera
//!   round it (the wheel brings it in or out), X brakes, K launches it into a free
//!   station lane (so it never meets a station), the map (M) warps or flies it to another planet,
//!   L lands it on the nearest flat 3 × 3. V switches between the chase camera and the cockpit.
//! - **R** lets go of the helm: in orbit the rocket parks in a free lane and goes round like a
//!   station (walk about in it; right click the helm to fly on); on the ground it comes apart into
//!   ordinary blocks again.
//!
//! Rockets in orbit are saved with the stations; one being flown when the game closes is saved
//! packed (`/station restore <id>` brings it back up in front of you).

use crate::chat::{ActionBar, Chat, Keyboard};
use crate::map::GalaxyMap;
use crate::player::Player;
use crate::station::{StationMenu, Stations};
use crate::travel::{Pilot, Travel, relative};
use crate::world::{Clock, Origin, Space, remesh};
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use orbit_core::block;
use orbit_core::planet::ChunkKey;
use orbit_core::player::{Body, Located, StationAt, collides};
use orbit_core::rng::mix;
use orbit_core::rocket::{self, Lay};
use orbit_core::sphere::CellId;
use orbit_core::station::{Flight, Glide, Station, StationOrbit};
use orbit_core::travel::{PILOT_ACCEL, in_orbit};

pub struct RocketsPlugin;

impl Plugin for RocketsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Rockets>()
            .add_systems(Startup, setup_flames)
            .add_systems(
                Update,
                (rocket_keys, run_requests)
                    .chain()
                    .after(crate::world::MovePlanets)
                    .before(crate::player::move_player),
            )
            .add_systems(
                Update,
                follow_rocket
                    .after(crate::player::move_player)
                    .before(crate::player::sync_camera),
            )
            .add_systems(
                Update,
                (draw_flames, draw_marks).after(crate::player::sync_camera),
            );
    }
}

pub enum Request {
    /// Gather the rocket whose helm is in this planet cell, and fly it.
    Assemble { planet: u64, cell: CellId },
    /// Take the helm of a rocket parked in orbit (you are on it).
    Take(String),
    /// Let go of the helm: park in orbit, or come apart on the ground.
    Release,
    /// Put the starter rocket up in front of you.
    Build(Option<String>),
    /// Build the starter rocket from a Rocket Kit; the caller consumes it only on success.
    BuildKit(orbit_core::inventory::Hand),
    /// Gather the rocket whose helm is at this block of a station (standing on its pad), and fly it.
    AssembleOnStation { station: String, cell: IVec3 },
}

/// The rocket being flown, and requests for the next frame.
#[derive(Resource)]
pub struct Rockets {
    pub requests: Vec<Request>,
    pub piloting: Option<Piloting>,
    /// Cockpit camera (from your eyes) instead of the chase camera behind the rocket.
    pub cockpit: bool,
    /// How far back the chase camera sits, times its usual distance (the mouse wheel).
    pub zoom: f32,
    /// The blocks to blame when a rocket could not go up, outlined in the world.
    marks: Option<Marks>,
    /// How far from the camera the rocket's far end is (0 in the cockpit): the warp tunnel is
    /// drawn round all of it.
    pub reach: f32,
    flames: Vec<Entity>,
    /// Names of the rockets standing on the ground, by their helm's cell: a rocket keeps its
    /// name when it lands and goes up again.
    names: std::collections::HashMap<(u64, CellId), String>,
    /// The same for rockets standing on stations' pads: by station and helm block.
    station_names: std::collections::HashMap<(String, IVec3), String>,
}

impl Default for Rockets {
    fn default() -> Self {
        Self {
            requests: Vec::new(),
            piloting: None,
            cockpit: false,
            zoom: 1.0,
            reach: 0.0,
            marks: None,
            flames: Vec::new(),
            names: Default::default(),
            station_names: Default::default(),
        }
    }
}

impl Rockets {
    pub fn is_piloting(&self) -> bool {
        self.piloting.is_some()
    }
}

/// How long the blocks to blame stay marked, seconds.
const MARK_SECS: f32 = 15.0;

struct Marks {
    planet: u64,
    cells: Vec<CellId>,
    left: f32,
}

pub struct Piloting {
    pub id: String,
    pub name: String,
    /// The player themself, at the helm in the rocket's frame, while their `Player::body` is the
    /// rocket's (the rocket flies as a player would).
    pub person: Body,
    /// The planet it was last near: its star lights it out in space.
    pub planet: u64,
    /// 0 (off) to 1 (full), smoothed: how big the flames are.
    pub throttle: f32,
    /// Bottoms of the engines with nothing under them, rocket coordinates.
    nozzles: Vec<Vec3>,
    /// Its middle (rocket coordinates) and half its longest side, for the chase camera.
    pub middle: Vec3,
    pub size: f32,
    /// Where its nose points and where its front faces (world), turning smoothly toward the
    /// way it goes; the mouse only turns the camera round it. With the frame's turn and planet
    /// they were measured in, to carry them round with a spinning planet.
    aim: Option<(Vec3, Vec3, Quat, Option<u64>)>,
}

impl Piloting {
    fn new(s: &Station, person: Body, planet: u64) -> Self {
        let nozzles = s
            .cells()
            .filter(|&(p, b)| b == block::ENGINE && !s.solid(p - IVec3::Y))
            .map(|(p, _)| p.as_vec3() + Vec3::new(0.5, 0.0, 0.5))
            .collect();
        let (lo, hi) = (
            s.bounds.min.as_vec3(),
            (s.bounds.max + IVec3::ONE).as_vec3(),
        );
        Self {
            id: s.id.clone(),
            name: s.name.clone(),
            person,
            planet,
            throttle: 0.0,
            nozzles,
            middle: (lo + hi) * 0.5,
            size: (hi - lo).max_element() * 0.5,
            aim: None,
        }
    }
}

/// Where you stand at a rocket's helm: just behind it (or beside it, or on it, if that is
/// blocked), facing it. Rocket coordinates: feet, and the way to face.
fn seat(s: &Station) -> (Vec3, Vec3) {
    let free = |p: IVec3| !s.solid(p) && !s.solid(p + IVec3::Y);
    for d in [IVec3::NEG_Z, IVec3::X, IVec3::NEG_X, IVec3::Z] {
        let at = s.core + d;
        if free(at) {
            return (at.as_vec3() + Vec3::new(0.5, 0.0, 0.5), -d.as_vec3());
        }
    }
    (
        (s.core + IVec3::Y).as_vec3() + Vec3::new(0.5, 0.0, 0.5),
        Vec3::Z,
    )
}

/// A rocket id no station or rocket has.
fn new_id(stations: &Stations, t: f64) -> String {
    (0u64..)
        .map(|n| Station::new_id(mix(t.to_bits() ^ n.wrapping_mul(0x9E37) ^ 0x50C7)))
        .find(|id| stations.find(id).is_none())
        .unwrap()
}

fn rocket_keys(
    keys: Res<ButtonInput<KeyCode>>,
    mut rockets: ResMut<Rockets>,
    keyboard: Res<Keyboard>,
    map: Res<GalaxyMap>,
    menu: Res<StationMenu>,
    mut bar: ResMut<ActionBar>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
) {
    if keyboard.owner.is_some() || map.is_open() || menu.is_open() || !rockets.is_piloting() {
        return;
    }
    // The wheel brings the chase camera in or takes it out (it picks blocks in the cockpit).
    if !rockets.cockpit && scroll.delta.y != 0.0 {
        rockets.zoom = (rockets.zoom * 0.88f32.powf(scroll.delta.y.signum())).clamp(0.35, 3.0);
    }
    if keys.just_pressed(KeyCode::KeyR) {
        rockets.requests.push(Request::Release);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        rockets.cockpit = !rockets.cockpit;
        bar.show(if rockets.cockpit {
            "Cockpit view"
        } else {
            "Chase view"
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn run_requests(
    mut commands: Commands,
    mut rockets: ResMut<Rockets>,
    mut player: Single<&mut Player>,
    mut stations: ResMut<Stations>,
    mut space: ResMut<Space>,
    clock: Res<Clock>,
    mut bar: ResMut<ActionBar>,
    mut chat: ResMut<Chat>,
    mut travel: ResMut<Travel>,
) {
    // A landing the autopilot finished takes the rocket apart.
    if std::mem::take(&mut travel.landed) && rockets.is_piloting() {
        rockets.requests.push(Request::Release);
    }
    // One that came down on a station's pad becomes blocks of the station.
    if let Some((station, berth)) = travel.docked.take() {
        let said = onto_station(
            &mut commands,
            &mut rockets,
            &mut player,
            &mut stations,
            &space,
            &station,
            berth,
            clock.0,
        );
        match said {
            Ok(m) => {
                travel.message = m.clone();
                bar.show(&m);
            }
            Err(e) => {
                travel.message = e.clone();
                bar.show(&e);
            }
        }
    }
    let t = clock.0;
    for req in std::mem::take(&mut rockets.requests) {
        let said = match req {
            Request::Assemble { planet, cell } => assemble(
                &mut rockets,
                &mut player,
                &mut stations,
                &mut space,
                planet,
                cell,
                t,
            ),
            Request::AssembleOnStation { station, cell } => assemble_on_station(
                &mut rockets,
                &mut player,
                &mut stations,
                &space,
                &station,
                cell,
                t,
            ),
            Request::Take(id) => take(&mut rockets, &mut player, &mut stations, &space, &id, t),
            Request::Release => release(
                &mut commands,
                &mut rockets,
                &mut player,
                &mut stations,
                &mut space,
                t,
            ),
            Request::Build(name) => match build(
                &mut rockets,
                &player,
                &stations,
                &mut space,
                name.as_deref(),
                t,
            ) {
                Ok(m) => {
                    chat.info(&m);
                    Ok(m)
                }
                Err(e) => {
                    chat.error(&e);
                    Err(e)
                }
            },
            Request::BuildKit(hand) => {
                match build(&mut rockets, &player, &stations, &mut space, None, t) {
                    Ok(m) => {
                        player.inv.use_hand(hand);
                        chat.info(&m);
                        Ok(m)
                    }
                    Err(e) => {
                        chat.error(&e);
                        Err(e)
                    }
                }
            }
        };
        match said {
            Ok(m) => {
                travel.message = m.clone();
                bar.show(&m);
            }
            Err(e) => {
                info!("rocket: {e}");
                bar.show(&e);
            }
        }
    }
}

/// Puts blocks into a planet; returns the chunks to mesh again.
fn put_blocks(
    planet: &mut orbit_core::planet::Planet,
    cells: impl IntoIterator<Item = (CellId, block::Block)>,
) -> Vec<ChunkKey> {
    let mut dirty = Vec::new();
    for (c, b) in cells {
        for k in planet.set(c, b) {
            if !dirty.contains(&k) {
                dirty.push(k);
            }
        }
    }
    dirty
}

fn assemble(
    rockets: &mut Rockets,
    player: &mut Player,
    stations: &mut Stations,
    space: &mut Space,
    planet: u64,
    cell: CellId,
    t: f64,
) -> Result<String, String> {
    if rockets.is_piloting() {
        return Err("Ya estás frente a un timón".into());
    }
    if player.body.gravity_source != Some(planet) {
        return Err("Acércate al cohete para tomar su timón".into());
    }
    let id = new_id(stations, t);
    let slot = space.planets.get_mut(&planet).ok_or("Lost the planet")?;
    let voxels = slot.voxel.clone().ok_or("El terreno todavía no está listo")?;
    let spec = slot.spec.clone();
    let ground = voxels.read().unwrap();
    if ground.get(cell) != block::HELM {
        return Err("Eso no es un timón".into());
    }
    let name = rockets
        .names
        .get(&(planet, cell))
        .cloned()
        .unwrap_or_default();
    let a = match rocket::assemble(&ground, cell, player.body.forward, id, &name) {
        Ok(a) => a,
        Err(f) => {
            // Show where: the blocks in the way, or the holes in the pad, outlined for a while.
            rockets.marks = (!f.cells.is_empty()).then(|| Marks {
                planet,
                cells: f.cells.clone(),
                left: MARK_SECS,
            });
            return Err(f.message());
        }
    };
    rockets.marks = None;
    rockets.names.remove(&(planet, cell));
    let turn_local = Quat::from_mat3(&a.lay.turn(&ground)).normalize();
    let base_local = a.lay.base(&ground);
    let rotation = (spec.rotation * turn_local).normalize();
    let center = rocket::origin_from_base(
        spec.center + (spec.rotation * base_local).as_dvec3(),
        rotation,
    );
    let mut r = a.rocket;
    r.flight = Some(Flight {
        center,
        rotation,
        planet,
    });
    // The rocket flies as a player would: a body standing on the pad where its base is.
    let here = Located {
        planet: &ground,
        center: spec.center,
        rotation: spec.rotation,
    };
    let mut vehicle = Body::on_planet(
        &here,
        base_local,
        base_local.normalize(),
        turn_local * Vec3::Z,
    );
    vehicle.jetpack = true;
    vehicle.on_ground = true;
    let at = StationAt {
        station: &r,
        center,
        rotation,
        planet,
        planet_center: spec.center,
        spin: Vec3::ZERO,
    };
    let (feet, face) = seat(&r);
    let mut person = player.body.clone();
    person.board(&at, feet, face);
    let (n, engines) = rocket::summary(&r);
    let piloting = Piloting::new(&r, person, planet);
    drop(ground);
    let dirty = put_blocks(
        &mut voxels.write().unwrap(),
        a.cells.iter().map(|&c| (c, block::AIR)),
    );
    slot.edited = true;
    remesh(slot, &dirty);
    let name = r.name.clone();
    stations.push(r);
    player.body = vehicle;
    rockets.piloting = Some(piloting);
    Ok(format!(
        "{name} está listo: {n} bloques, {engines} motor{}. K despega, L aterriza, M abre el mapa, R deja el timón, V cambia la cámara",
        if engines == 1 { "" } else { "s" }
    ))
}

/// Puts together the rocket whose helm is at `cell` on station `station` and takes its helm: it
/// floats off the station's pad, going round with it, until you fly it away.
#[allow(clippy::too_many_arguments)]
fn assemble_on_station(
    rockets: &mut Rockets,
    player: &mut Player,
    stations: &mut Stations,
    space: &Space,
    station: &str,
    cell: IVec3,
    t: f64,
) -> Result<String, String> {
    if rockets.is_piloting() {
        return Err("Ya estás frente a un timón".into());
    }
    if player.body.station.as_deref() != Some(station) {
        return Err("Acércate al cohete para tomar su timón".into());
    }
    let id = new_id(stations, t);
    let i = stations.find(station).ok_or("Se perdió la estación")?;
    let slot = &stations.list[i];
    let mut st = slot.station.write().unwrap();
    if st.get(cell) != block::HELM {
        return Err("Eso no es un timón".into());
    }
    let f = Stations::frame(&st, space, t).ok_or("La estación no está en órbita aquí")?;
    let well = space
        .wells()
        .into_iter()
        .find(|w| w.id == f.planet)
        .ok_or("Lost the planet")?;
    let key = (station.to_string(), cell);
    let name = rockets.station_names.get(&key).cloned().unwrap_or_default();
    // The body is in the station's frame: its forward is in station coordinates.
    let a = rocket::assemble_on_station(&st, cell, player.body.forward, id, &name)
        .map_err(|p| p.message().to_string())?;
    for &c in &a.cells {
        st.set(c, block::AIR);
    }
    rockets.station_names.remove(&key);
    let rotation = (f.rotation * a.berth.rotation()).normalize();
    let center = f.center + (f.rotation * a.berth.point(Vec3::ZERO)).as_dvec3();
    let mut r = a.rocket;
    r.flight = Some(Flight {
        center,
        rotation,
        planet: f.planet,
    });
    // It goes round with the station: that is how fast it is moving when you take over.
    let base = f.center + (f.rotation * a.berth.point(Vec3::new(0.5, 0.0, 0.5))).as_dvec3();
    let vel = f.spin.cross((base - f.planet_center).as_vec3());
    let mut vehicle = Body::in_orbit_of(
        &well,
        (base - well.center).as_vec3(),
        vel,
        f.rotation * Vec3::Y,
        rotation * Vec3::Z,
    );
    vehicle.jetpack = true;
    let at = StationAt {
        station: &r,
        center,
        rotation,
        planet: f.planet,
        planet_center: f.planet_center,
        spin: Vec3::ZERO,
    };
    let (feet, face) = seat(&r);
    let mut person = player.body.clone();
    person.board(&at, feet, face);
    let (n, engines) = rocket::summary(&r);
    let piloting = Piloting::new(&r, person, f.planet);
    let sname = st.name.clone();
    drop(st);
    stations.mark_dirty(i);
    let name = r.name.clone();
    stations.push(r);
    player.body = vehicle;
    rockets.piloting = Some(piloting);
    Ok(format!(
        "{name} salió de la plataforma de {sname}: {n} bloques, {engines} motor{}. Espacio para subir, L para volver a una plataforma, M mapa, R estacionar",
        if engines == 1 { "" } else { "s" }
    ))
}

/// The rocket being flown, come down on station `station`'s pad at `berth`: it becomes blocks of
/// the station, and you stand where you were in it.
#[allow(clippy::too_many_arguments)]
fn onto_station(
    commands: &mut Commands,
    rockets: &mut Rockets,
    player: &mut Player,
    stations: &mut Stations,
    space: &Space,
    station: &str,
    berth: rocket::Berth,
    t: f64,
) -> Result<String, String> {
    let Some(pil) = rockets.piloting.as_ref() else {
        return Err("No estás pilotando un cohete".into());
    };
    let ri = stations.find(&pil.id).ok_or("El cohete ya no existe")?;
    let si = stations.find(station).ok_or("Se perdió la estación")?;
    let (cells, helm, name) = {
        let r = stations.list[ri].station.read().unwrap();
        let st = stations.list[si].station.read().unwrap();
        (
            rocket::berth_cells(&st, &r, &berth).map_err(|p| p.message().to_string())?,
            berth.cell(r.core),
            r.name.clone(),
        )
    };
    let sname = {
        let mut st = stations.list[si].station.write().unwrap();
        for (p, b) in cells {
            st.set(p, b);
        }
        let f = Stations::frame(&st, space, t).ok_or("La estación no está en órbita aquí")?;
        // You stay where you stood in it, now on the station.
        let feet = berth.point(pil.person.pos);
        let face = berth.rotation() * pil.person.forward;
        let mut body = pil.person.clone();
        body.board(&f.at(&st), feet, face);
        body.pitch = pil.person.pitch;
        player.body = body;
        st.name.clone()
    };
    stations.mark_dirty(si);
    if let Some(e) = stations.remove(ri) {
        commands.entity(e).despawn();
    }
    rockets
        .station_names
        .insert((station.to_string(), helm), name.clone());
    rockets.piloting = None;
    Ok(format!(
        "{name} aterrizó en {sname}: quedó como bloques sobre la plataforma. Haz clic derecho en su timón para volver a volar"
    ))
}

fn take(
    rockets: &mut Rockets,
    player: &mut Player,
    stations: &mut Stations,
    space: &Space,
    id: &str,
    t: f64,
) -> Result<String, String> {
    if rockets.is_piloting() {
        return Err("Ya estás frente a un timón".into());
    }
    let i = stations.find(id).ok_or("No existe ese cohete")?;
    let slot = &stations.list[i];
    let mut s = slot.station.write().unwrap();
    let f = Stations::frame(&s, space, t).ok_or("El cohete no está en órbita aquí")?;
    let well = space
        .wells()
        .into_iter()
        .find(|w| w.id == f.planet)
        .ok_or("Lost the planet")?;
    let base = f.center + (f.rotation * Vec3::new(0.5, 0.0, 0.5)).as_dvec3();
    // It goes round with its orbit: that is how fast it is moving when you take over.
    let vel = f.spin.cross((base - f.planet_center).as_vec3());
    let mut vehicle = Body::in_orbit_of(
        &well,
        (base - well.center).as_vec3(),
        vel,
        f.rotation * Vec3::Y,
        f.rotation * Vec3::Z,
    );
    vehicle.jetpack = true;
    let (feet, face) = seat(&s);
    let mut person = player.body.clone();
    person.board(&f.at(&s), feet, face);
    s.orbit = None;
    s.glide = None;
    s.flight = Some(Flight {
        center: f.center,
        rotation: f.rotation,
        planet: f.planet,
    });
    let name = s.name.clone();
    rockets.piloting = Some(Piloting::new(&s, person, f.planet));
    drop(s);
    stations.mark_dirty(i);
    player.body = vehicle;
    Ok(format!(
        "Al timón de {name}. M mapa para viajar, L aterrizar, R estacionar, V cámara"
    ))
}

fn release(
    commands: &mut Commands,
    rockets: &mut Rockets,
    player: &mut Player,
    stations: &mut Stations,
    space: &mut Space,
    t: f64,
) -> Result<String, String> {
    let Some(pil) = rockets.piloting.as_ref() else {
        return Err("No estás pilotando un cohete".into());
    };
    let Some(i) = stations.find(&pil.id) else {
        player.body = pil.person.clone();
        rockets.piloting = None;
        return Err("El cohete ya no existe".into());
    };
    let v = &player.body;
    if let Some(pid) = v.gravity_source {
        if !v.on_ground {
            return Err("Aterriza primero (L): el cohete solo se desmonta en el suelo".into());
        }
        let slot = space.planets.get_mut(&pid).ok_or("Lost the planet")?;
        let voxels = slot.voxel.clone().ok_or("El terreno todavía no está listo")?;
        let spec = slot.spec.clone();
        let mut ground = voxels.write().unwrap();
        let up = v.pos.normalize_or(Vec3::Y);
        // It stands the way it faces (the camera may look elsewhere), planet-local.
        let facing = stations.list[i]
            .station
            .read()
            .unwrap()
            .flight
            .map_or(v.forward, |f| {
                v.frame_rotation().inverse() * (f.rotation * Vec3::Z)
            });
        let lay = Lay::at(&ground, v.pos + up * 0.5, facing).ok_or("No hay suelo allí")?;
        let (cells, helm) = {
            let s = stations.list[i].station.read().unwrap();
            (
                rocket::landing(&ground, &s, &lay).map_err(|p| p.message().to_string())?,
                lay.cell(&ground, s.core),
            )
        };
        let dirty = put_blocks(&mut ground, cells);
        // You stay where you stood in it, now on the planet.
        let feet = lay.point(&ground, pil.person.pos);
        let face = lay.turn(&ground) * pil.person.forward;
        let here = Located {
            planet: &ground,
            center: spec.center,
            rotation: spec.rotation,
        };
        let mut body = Body::on_planet(&here, feet, feet.normalize(), face);
        body.pitch = pil.person.pitch;
        drop(ground);
        slot.edited = true;
        remesh(slot, &dirty);
        player.body = body;
        if let Some(e) = stations.remove(i) {
            commands.entity(e).despawn();
        }
        let name = pil.name.clone();
        if let Some(c) = helm {
            rockets.names.insert((pid, c), name.clone());
        }
        rockets.piloting = None;
        return Ok(format!(
            "{name} aterrizó: quedó como bloques sobre el suelo. Haz clic derecho en su timón para volver a volar"
        ));
    }
    let Some(pid) = v.orbiting else {
        return Err("No puedes dejarlo aquí: entra en órbita (K) o aterriza (L) primero".into());
    };
    let p = space.planets.get(&pid).ok_or("Lost the planet")?;
    let (pos, vel) = relative(v, p.spec.center);
    if !in_orbit(pos, vel, p.spec.pull()) {
        return Err("Todavía no estás en una órbita estable: K para circularizar o L para aterrizar".into());
    }
    let f = stations.list[i]
        .station
        .read()
        .unwrap()
        .flight
        .ok_or("El cohete no está volando")?;
    let rel = f.center - p.spec.center;
    let want = StationOrbit::through(pid, rel.as_vec3(), f.rotation, vel, p.spec.pull(), t);
    let radius = p.spec.radius;
    let o = stations
        .park(i, &want, space)
        .map_err(|r| r.message().to_string())?;
    {
        let mut s = stations.list[i].station.write().unwrap();
        s.flight = None;
        // Eases into its place, unless that is far along the lane (then it is just there).
        s.glide = (o.offset(t).distance(rel) < 3000.0).then_some(Glide {
            from: rel,
            turn: f.rotation,
            t0: t,
        });
    }
    player.body = pil.person.clone();
    let name = pil.name.clone();
    rockets.piloting = None;
    Ok(format!(
        "{name} está estacionado en un carril libre, a {:.0} bloques de altura: camina y haz clic derecho en su timón para volar",
        o.radius() - radius
    ))
}

/// Puts the starter rocket on the nearest pad a few blocks in front of the player.
fn build(
    rockets: &mut Rockets,
    player: &Player,
    stations: &Stations,
    space: &mut Space,
    name: Option<&str>,
    t: f64,
) -> Result<String, String> {
    let b = &player.body;
    let pid = b
        .gravity_source
        .ok_or("Debes estar en un planeta para construir un cohete")?;
    let slot = space.planets.get_mut(&pid).ok_or("Se perdió el planeta")?;
    let voxels = slot
        .voxel
        .clone()
        .ok_or("El terreno todavía no está listo")?;
    let mut ground = voxels.write().unwrap();
    let up = b.pos.normalize_or(Vec3::Y);
    let ahead = b
        .forward
        .reject_from(up)
        .normalize_or(up.any_orthonormal_vector());
    let kit = rocket::starter(new_id(stations, t), name.unwrap_or(""));
    let lay = rocket::find_pad(&ground, &kit, b.pos + ahead * 9.0, ahead).ok_or(
        "No hay una plataforma plana de 3x3 con cielo despejado: prueba en un terreno más llano",
    )?;
    let cells = rocket::landing(&ground, &kit, &lay).map_err(|p| p.message().to_string())?;
    let undo: Vec<(CellId, block::Block)> =
        cells.iter().map(|&(c, _)| (c, ground.get(c))).collect();
    let dirty = put_blocks(&mut ground, cells);
    if collides(&ground, b.pos) {
        put_blocks(&mut ground, undo);
        return Err("Estás donde iría el cohete: retrocede un poco".into());
    }
    if let Some(c) = lay.cell(&ground, kit.core) {
        rockets.names.insert((pid, c), kit.name.clone());
    }
    drop(ground);
    slot.edited = true;
    remesh(slot, &dirty);
    let (n, _) = rocket::summary(&kit);
    Ok(format!(
        "Construido {} ({n} bloques) en la plataforma de adelante: entra por las escaleras y haz clic derecho en su timón",
        kit.name
    ))
}

/// Keeps the rocket where the body flying it is, and you at its helm, looking where it looks.
#[allow(clippy::too_many_arguments)]
fn follow_rocket(
    mut rockets: ResMut<Rockets>,
    mut player: Single<&mut Player>,
    stations: Res<Stations>,
    space: Res<Space>,
    pilot: Res<Pilot>,
    keys: Res<ButtonInput<KeyCode>>,
    keyboard: Res<Keyboard>,
    time: Res<Time>,
    fx: Res<crate::travel::WarpFx>,
    travel: Res<Travel>,
) {
    let Some(pil) = rockets.piloting.as_mut() else {
        return;
    };
    let Some(slot) = stations.by_id(&pil.id) else {
        player.body = pil.person.clone();
        rockets.piloting = None;
        return;
    };
    let v = &player.body;
    if let Some(p) = v.planet() {
        pil.planet = p;
    }
    // The nose points the way it goes (in a warp, the way of the jump), turning smoothly.
    let up = v.world_dir(v.up);
    let frame = v.frame_rotation();
    let (mut before, mut facing) = match pil.aim {
        Some((n, z, _, _)) => (n, z),
        // As it stands now (just assembled, or taken in orbit).
        None => slot
            .station
            .read()
            .unwrap()
            .flight
            .map_or((up, v.world_dir(v.forward)), |f| {
                (f.rotation * Vec3::Y, f.rotation * Vec3::Z)
            }),
    };
    // On a planet its frame turns with the ground: the rocket turns with it.
    if let Some((_, _, was, Some(p))) = pil.aim {
        if v.gravity_source == Some(p) {
            let spin = frame * was.inverse();
            (before, facing) = (spin * before, spin * facing);
        }
    }
    let want = if let Some((nose, _)) = travel.align {
        nose // coming down on a station's pad: stood up square to it
    } else if fx.phase.is_some() {
        v.world_dir(v.look_dir())
    } else {
        rocket::nose_toward(
            up,
            v.world_dir(v.vel),
            v.gravity_source.is_some(),
            v.on_ground,
            before,
        )
    };
    let k = 1.0 - (-2.5 * time.delta_secs()).exp();
    let step = Quat::IDENTITY.slerp(Quat::from_rotation_arc(before, want), k);
    let nose = (step * before).normalize_or(up);
    let mut front = step * facing;
    if let Some((_, f)) = travel.align {
        front = front.lerp(f.reject_from(nose), k).normalize_or(front);
    }
    let rotation = rocket::turn_from(nose, front);
    pil.aim = Some((nose, rotation * Vec3::Z, frame, v.gravity_source));
    let center = rocket::origin_about_middle(v.world_point(v.pos), up, pil.middle.y, rotation);
    slot.station.write().unwrap().flight = Some(Flight {
        center,
        rotation,
        planet: pil.planet,
    });
    let s = slot.station.read().unwrap();
    let planet_center = space
        .planets
        .get(&pil.planet)
        .map_or(center, |p| p.spec.center);
    let at = StationAt {
        station: &s,
        center,
        rotation,
        planet: pil.planet,
        planet_center,
        spin: Vec3::ZERO,
    };
    pil.person.sync_frame(&[], &[], &[at]);
    let look = rotation.inverse() * v.world_dir(v.look_dir());
    pil.person.up = Vec3::Y;
    if let Some(flat) = look.reject_from(Vec3::Y).try_normalize() {
        pil.person.forward = flat;
    }
    pil.person.pitch = look.y.clamp(-1.0, 1.0).asin();

    let typing = keyboard.owner.is_some();
    let pushing = !typing
        && [
            KeyCode::KeyW,
            KeyCode::KeyA,
            KeyCode::KeyS,
            KeyCode::KeyD,
            KeyCode::Space,
            KeyCode::ShiftLeft,
        ]
        .iter()
        .any(|k| keys.pressed(*k));
    let boost = keys.pressed(KeyCode::ControlLeft);
    let want = if pilot.frozen {
        0.0
    } else if let Some(a) = pilot.thrust {
        (a.length() / PILOT_ACCEL).clamp(0.3, 1.0)
    } else if pushing {
        if boost { 1.0 } else { 0.65 }
    } else if v.gravity_source.is_some() && !v.on_ground {
        0.35 // hovering
    } else {
        0.0
    };
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    pil.throttle += (want - pil.throttle) * k;
}

#[derive(Resource)]
struct FlameAssets {
    mesh: Handle<Mesh>,
    mat: Handle<StandardMaterial>,
}

fn setup_flames(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cone {
        radius: 0.42,
        height: 1.0,
    });
    let mat = mats.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::new(6.0, 2.4, 0.6, 1.0)),
        unlit: true,
        alpha_mode: AlphaMode::Add,
        cull_mode: None,
        ..default()
    });
    commands.insert_resource(FlameAssets { mesh, mat });
}

/// A flame under each engine, as long as the throttle, flickering.
fn draw_flames(
    mut commands: Commands,
    mut rockets: ResMut<Rockets>,
    stations: Res<Stations>,
    origin: Res<Origin>,
    time: Res<Time>,
    assets: Res<FlameAssets>,
    mut q: Query<(&mut Transform, &mut Visibility)>,
) {
    let want = rockets.piloting.as_ref().map_or(0, |p| p.nozzles.len());
    while rockets.flames.len() > want {
        let e = rockets.flames.pop().unwrap();
        commands.entity(e).despawn();
    }
    while rockets.flames.len() < want {
        let e = commands
            .spawn((
                Mesh3d(assets.mesh.clone()),
                MeshMaterial3d(assets.mat.clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
            ))
            .id();
        rockets.flames.push(e);
    }
    let Some(pil) = rockets.piloting.as_ref() else {
        return;
    };
    let Some(f) = stations
        .by_id(&pil.id)
        .and_then(|s| s.station.read().unwrap().flight)
    else {
        return;
    };
    let down = f.rotation * Vec3::NEG_Y;
    let now = time.elapsed_secs();
    for (n, (&e, &nozzle)) in rockets.flames.iter().zip(&pil.nozzles).enumerate() {
        let Ok((mut tr, mut vis)) = q.get_mut(e) else {
            continue;
        };
        let flicker =
            0.85 + 0.15 * (now * 37.0 + n as f32 * 1.7).sin() * (now * 23.0 + n as f32).cos();
        let len = (0.5 + 3.2 * pil.throttle) * flicker;
        let width = 0.75 + 0.35 * pil.throttle;
        let at = origin.render(f.center + (f.rotation * nozzle).as_dvec3());
        // The cone's tip is its +y: turned over, it points down out of the nozzle.
        *tr = Transform {
            translation: at + down * (len * 0.5),
            rotation: f.rotation * Quat::from_rotation_x(std::f32::consts::PI),
            scale: Vec3::new(width, len, width),
        };
        *vis = if pil.throttle > 0.04 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Outlines the blocks to blame for a rocket that could not go up, in red, blinking, and draws a
/// line up to them from the helm side so ones high up (a branch, an overhang) are found too.
fn draw_marks(
    mut rockets: ResMut<Rockets>,
    space: Res<Space>,
    origin: Res<Origin>,
    time: Res<Time>,
    player: Single<&Player>,
    mut gizmos: Gizmos,
) {
    let Some(m) = rockets.marks.as_mut() else {
        return;
    };
    m.left -= time.delta_secs();
    if m.left <= 0.0 {
        rockets.marks = None;
        return;
    }
    let Some(slot) = space.planets.get(&m.planet) else {
        return;
    };
    let Some(voxels) = slot.voxel.as_ref() else {
        return;
    };
    let planet = voxels.read().unwrap();
    let g = &planet.grid;
    let spec = &slot.spec;
    let base = origin.render(spec.center);
    let blink = 0.55 + 0.45 * (time.elapsed_secs() * 6.0).sin().abs();
    let color = Color::srgba(1.0, 0.15, 0.1, blink);
    for &cell in &m.cells {
        let mid = g.center(cell);
        let c =
            |di, dj, dk| base + spec.rotation * (mid + (g.corner(cell, di, dj, dk) - mid) * 1.06);
        for (a, b) in [
            ((0, 0), (1, 0)),
            ((1, 0), (1, 1)),
            ((1, 1), (0, 1)),
            ((0, 1), (0, 0)),
        ] {
            for k in 0..2 {
                gizmos.line(c(a.0, a.1, k), c(b.0, b.1, k), color);
            }
            gizmos.line(c(a.0, a.1, 0), c(a.0, a.1, 1), color);
        }
    }
    // A line from your eye to the nearest one, so it can be found wherever it is.
    let b = &player.body;
    if let Some(&first) = m.cells.first() {
        let eye = origin.render(b.world_point(b.eye()));
        let to = base + spec.rotation * g.center(first);
        gizmos.line(
            eye + (to - eye).normalize_or_zero() * 1.5,
            to,
            Color::srgba(1.0, 0.3, 0.2, 0.6 * blink),
        );
    }
}
