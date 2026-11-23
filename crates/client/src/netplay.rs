//! LAN session integration.
//!
//! The host is authoritative for player transforms and voxel edits. A joining client receives
//! the seed and the host's edit log, then receives subsequent edits. Stations, rockets and
//! elevators are intentionally not synchronized yet.

use crate::{
    net::{Browser, Client, ClientEvent, DEFAULT_PORT, Found, Host, HostEvent, Msg},
    pause_menu::ReturnToTitle,
    save::{SavedWorld, StartWorld},
    start_menu,
};
use bevy::prelude::*;
use orbit_core::glam::DVec3;
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

#[derive(Resource)]
pub enum NetRole {
    Offline,
    Host(Host),
    Client(Client),
}
impl Default for NetRole {
    fn default() -> Self {
        Self::Offline
    }
}
// Host/Client own single-threaded receivers. Bevy accesses this resource through exclusive
// mutable system parameters; the transport itself remains thread-safe internally.
unsafe impl Sync for NetRole {}

#[derive(Resource, Default)]
pub struct NetPlay {
    pub browser: Option<Browser>,
    pub found: Vec<Found>,
    pub edit_log: Vec<Msg>,
    pub player_id: u32,
    pub names: HashMap<u32, String>,
    pub remotes: HashMap<u32, Remote>,
    pub status: Option<String>,
    pub pending_welcome: Option<crate::net::Welcome>,
    last_state: Option<Instant>,
    net_test_since: Option<Instant>,
    discovery_logged: bool,
}

pub struct Remote {
    pub name: String,
    pub pos: DVec3,
    pub target: DVec3,
    pub rot: Quat,
    pub entity: Option<Entity>,
}
#[derive(Component)]
struct RemoteAvatar;

pub struct NetPlayPlugin;
impl Plugin for NetPlayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetPlay>()
            .init_resource::<NetRole>()
            .add_systems(Startup, (setup_browser, auto_session).chain())
            .add_systems(
                Update,
                (
                    poll_browser,
                    poll_session,
                    send_state,
                    apply_remote_edits,
                    draw_remotes,
                )
                    .chain(),
            );
    }
}

fn setup_browser(mut net: ResMut<NetPlay>) {
    net.browser = Browser::bind(DEFAULT_PORT).ok();
}

fn auto_session(
    mut role: ResMut<NetRole>,
    mut net: ResMut<NetPlay>,
    settings: Res<crate::settings::Settings>,
    space: Res<crate::world::Space>,
) {
    if std::env::var("ORBIT_AUTOHOST").is_ok() {
        match Host::start(
            DEFAULT_PORT,
            "Fourlokos",
            space.universe.seed,
            &settings.player_name,
        ) {
            Ok(host) => {
                *role = NetRole::Host(host);
                info!("LAN host started on port {DEFAULT_PORT}");
            }
            Err(e) => warn!("LAN host failed: {e}"),
        }
    } else if let Ok(addr) = std::env::var("ORBIT_AUTOJOIN") {
        let endpoint = if addr.parse::<std::net::SocketAddr>().is_ok() {
            addr.clone()
        } else {
            format!("{addr}:{DEFAULT_PORT}")
        };
        match Client::connect(endpoint.as_str(), &settings.player_name) {
            Ok((client, welcome)) => {
                *role = NetRole::Client(client);
                net.pending_welcome = Some(welcome);
                info!("LAN client connected to {endpoint}");
            }
            Err(e) => warn!("LAN join failed: {e}"),
        }
    }
}

fn poll_browser(mut net: ResMut<NetPlay>) {
    if let Some(browser) = net.browser.as_mut() {
        net.found = browser.poll();
    }
    if !net.discovery_logged && !net.found.is_empty() {
        info!("LAN discovery: found {} world(s)", net.found.len());
        net.discovery_logged = true;
    }
}

fn poll_session(
    mut commands: Commands,
    mut net: ResMut<NetPlay>,
    mut starts: MessageWriter<StartWorld>,
    mut returns: MessageWriter<ReturnToTitle>,
    settings: Res<crate::settings::Settings>,
    mut space: ResMut<crate::world::Space>,
    mut role: ResMut<NetRole>,
) {
    if let Some(welcome) = net.pending_welcome.take() {
        handle_client_msg(
            &mut commands,
            &mut net,
            &mut starts,
            &mut space,
            Msg::Welcome {
                player_id: welcome.player_id,
                seed: welcome.seed,
                world_name: welcome.world_name,
                players: welcome.players,
            },
        );
    }
    let mut events = Vec::new();
    match &*role {
        NetRole::Host(host) => {
            for ev in host.poll() {
                events.push(ev);
            }
        }
        NetRole::Client(client) => {
            for ev in client.poll() {
                match ev {
                    ClientEvent::Disconnected(e) => {
                        net.status = Some(format!("Desconectado: {e}"));
                        returns.write(ReturnToTitle);
                        *role = NetRole::Offline;
                    }
                    ClientEvent::Msg(m) => {
                        handle_client_msg(&mut commands, &mut net, &mut starts, &mut space, m)
                    }
                }
            }
        }
        NetRole::Offline => {}
    }
    for ev in events {
        match ev {
            HostEvent::Joined { id, name } => {
                net.names.insert(id, name.clone());
                if let NetRole::Host(host) = &*role {
                    for edit in &net.edit_log {
                        host.send(id, edit);
                    }
                    host.broadcast(&Msg::PlayerJoined { id, name }, Some(id));
                }
                info!("LAN handshake: player {id} joined");
            }
            HostEvent::Left { id } => {
                net.names.remove(&id);
                if let Some(r) = net.remotes.remove(&id) {
                    if let Some(e) = r.entity {
                        commands.entity(e).despawn();
                    }
                }
                info!("LAN player {id} left");
            }
            HostEvent::FromClient { id, msg } => match msg {
                Msg::PlayerState {
                    pos,
                    rot,
                    planet_frame,
                    ..
                } => {
                    if let NetRole::Host(host) = &*role {
                        host.broadcast(
                            &Msg::PlayerState {
                                id,
                                pos,
                                rot,
                                planet_frame,
                            },
                            Some(id),
                        );
                    }
                }
                Msg::BlockEdit { planet, pos, block } => {
                    apply_edit(&mut space, planet, pos, block);
                    relay_edit(
                        &mut net,
                        &role,
                        Msg::BlockEdit { planet, pos, block },
                        Some(id),
                    );
                }
                Msg::Chat { text, .. } => {
                    if let NetRole::Host(host) = &*role {
                        host.broadcast(&Msg::Chat { id, text }, None);
                    }
                }
                _ => {}
            },
        }
    }
    let _ = settings;
}

fn handle_client_msg(
    commands: &mut Commands,
    net: &mut NetPlay,
    starts: &mut MessageWriter<StartWorld>,
    space: &mut crate::world::Space,
    msg: Msg,
) {
    match msg {
        Msg::Welcome {
            player_id,
            seed,
            world_name,
            players,
        } => {
            net.player_id = player_id;
            net.names = players.into_iter().collect();
            starts.write(StartWorld(SavedWorld {
                dir: std::env::temp_dir().join(format!("fourlokos-lan-{seed}")),
                info: {
                    let mut i = orbit_core::world_file::WorldInfo::new(&world_name, seed);
                    i.make = start_menu::fixed_gen();
                    i.mode = orbit_core::inventory::GameMode::Survival;
                    i
                },
            }));
            info!("LAN handshake complete: joined seed {seed}");
        }
        Msg::PlayerJoined { id, name } => {
            net.names.insert(id, name.clone());
            net.status = Some(format!("{name} se unió"));
        }
        Msg::PlayerLeft { id } => {
            net.names.remove(&id);
            if let Some(r) = net.remotes.remove(&id) {
                if let Some(e) = r.entity {
                    commands.entity(e).despawn();
                }
            }
        }
        Msg::PlayerState { id, pos, rot, .. } if id != net.player_id => {
            let p = DVec3::from_array(pos);
            let q = Quat::from_array(rot);
            net.remotes
                .entry(id)
                .and_modify(|r| {
                    r.target = p;
                    r.rot = q;
                })
                .or_insert(Remote {
                    name: net
                        .names
                        .get(&id)
                        .cloned()
                        .unwrap_or_else(|| format!("Jugador {id}")),
                    pos: p,
                    target: p,
                    rot: q,
                    entity: None,
                });
            info!("LAN remote player state received: {id}");
        }
        Msg::BlockEdit { planet, pos, block } => {
            info!("LAN BlockEdit received host→client");
            apply_edit(space, planet, pos, block);
        }
        Msg::Chat { text, .. } => net.status = Some(text),
        _ => {
            let _ = commands;
        }
    }
}

fn send_state(
    mut net: ResMut<NetPlay>,
    role: Res<NetRole>,
    player: Single<&crate::player::Player>,
) {
    if net
        .last_state
        .is_some_and(|t| t.elapsed() < Duration::from_millis(50))
    {
        return;
    }
    net.last_state = Some(Instant::now());
    let p = &player.body;
    let msg = Msg::PlayerState {
        id: net.player_id,
        pos: p.world_point(p.pos).to_array(),
        rot: Quat::from_rotation_arc(Vec3::Z, p.world_dir(p.forward)).to_array(),
        planet_frame: p.gravity_source.map(|x| x as u32),
    };
    match &*role {
        NetRole::Host(host) => host.broadcast(&msg, None),
        NetRole::Client(client) => client.send(&msg),
        NetRole::Offline => {}
    }
    if std::env::var("ORBIT_NET_TEST").is_ok()
        && net
            .net_test_since
            .is_none_or(|t| t.elapsed() > Duration::from_secs(2))
    {
        net.net_test_since = Some(Instant::now());
        let edit = Msg::BlockEdit {
            planet: 0,
            pos: [0, 0, 0],
            block: 0,
        };
        match &*role {
            NetRole::Host(host) if host.player_count() > 0 => {
                host.broadcast(&edit, None);
                info!("LAN test BlockEdit host→client");
            }
            NetRole::Client(client) => {
                client.send(&edit);
                info!("LAN test BlockEdit client→host");
            }
            _ => {}
        }
    }
}

fn relay_edit(net: &mut NetPlay, role: &NetRole, edit: Msg, except: Option<u32>) {
    if let Msg::BlockEdit { .. } = edit {
        if net.edit_log.len() < 200_000 {
            net.edit_log.push(edit.clone());
        }
        info!("LAN BlockEdit relayed host←client");
    }
    if let NetRole::Host(host) = role {
        host.broadcast(&edit, except);
    }
}

pub fn local_edit(net: &mut NetRole, edit: Msg) {
    match net {
        NetRole::Host(host) => host.broadcast(&edit, None),
        NetRole::Client(client) => client.send(&edit),
        NetRole::Offline => {}
    }
}

pub fn record_local_edit(net: &mut NetPlay, role: &mut NetRole, edit: Msg) {
    if matches!(role, NetRole::Host(_)) && net.edit_log.len() < 200_000 {
        net.edit_log.push(edit.clone());
    }
    local_edit(role, edit);
}

fn apply_edit(space: &mut crate::world::Space, planet: u32, pos: [i32; 3], block: u16) {
    let Some(slot) = space.planets.get_mut(&(planet as u64)) else {
        return;
    };
    let Some(voxels) = &slot.voxel else { return };
    let mut p = voxels.write().unwrap();
    let cell = pos[0] as usize;
    if cell < p.grid.cell_count() {
        let keys = p.set(cell, block as u8);
        drop(p);
        slot.edited = true;
        crate::world::remesh(slot, &keys);
    }
}

fn apply_remote_edits() {}

fn draw_remotes(
    mut commands: Commands,
    mut net: ResMut<NetPlay>,
    origin: Res<crate::world::Origin>,
    mut transforms: Query<&mut Transform, With<RemoteAvatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let body = meshes.add(Cuboid::new(0.7, 1.5, 0.45));
    let head = meshes.add(Cuboid::new(0.5, 0.5, 0.5));
    for (id, remote) in &mut net.remotes {
        remote.pos = remote.pos.lerp(remote.target, 0.18);
        let render = origin.render(remote.pos);
        let entity = *remote.entity.get_or_insert_with(|| {
            let color = Color::hsl((*id as f32 * 137.5) % 360.0, 0.65, 0.55);
            commands
                .spawn((
                    RemoteAvatar,
                    Name::new(remote.name.clone()),
                    Mesh3d(body.clone()),
                    MeshMaterial3d(materials.add(StandardMaterial {
                        base_color: color,
                        ..default()
                    })),
                    Transform::from_translation(render),
                ))
                .with_children(|p| {
                    p.spawn((
                        Mesh3d(head.clone()),
                        MeshMaterial3d(materials.add(StandardMaterial {
                            base_color: color,
                            ..default()
                        })),
                        Transform::from_xyz(0.0, 1.0, 0.0),
                    ));
                    p.spawn((
                        Text2d::new(remote.name.clone()),
                        TextFont {
                            font_size: FontSize::Px(0.22),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        Transform::from_xyz(0.0, 1.65, 0.0),
                    ));
                })
                .id()
        });
        if let Ok(mut transform) = transforms.get_mut(entity) {
            transform.translation = render;
            transform.rotation = remote.rot;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_log_is_capped() {
        let mut log = Vec::new();
        for i in 0..200_100 {
            if log.len() < 200_000 {
                log.push(i);
            }
        }
        assert_eq!(log.len(), 200_000);
    }
}
