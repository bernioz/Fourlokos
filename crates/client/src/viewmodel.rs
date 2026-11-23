//! First-person arm: your right arm (a plain box until the character model arrives) with the
//! block you hold in its hand, drawn by a second camera on its own render layer so it never
//! sinks into walls. It bobs as you walk and swings when you click.

use crate::health::Vitals;
use crate::lighting::VoxelMaterial;
use crate::player::Player;
use crate::textures::Atlas;
use crate::world::{Space, gpu_mesh};
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use orbit_core::block::Block;

/// The render layer of the arm and what it holds.
const LAYER: usize = 2;
/// Seconds one swing of the arm takes.
const SWING: f32 = 0.3;

pub struct ViewmodelPlugin;

impl Plugin for ViewmodelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn)
            .add_systems(Update, update.after(crate::player::sync_camera));
    }
}

#[derive(Component)]
struct ViewCamera;
#[derive(Component)]
struct ArmRoot;
#[derive(Component)]
struct ArmPart(Color);
#[derive(Component)]
struct Held;

const SKIN: Color = Color::srgb(0.87, 0.66, 0.52);
const SLEEVE: Color = Color::srgb(0.2, 0.42, 0.78);

fn spawn(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let layers = RenderLayers::layer(LAYER);
    commands.spawn((
        ViewCamera,
        Camera3d::default(),
        // After the world, on top of it: only the depth is cleared.
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        Projection::Perspective(PerspectiveProjection {
            fov: 70f32.to_radians(),
            near: 0.02,
            far: 10.0,
            ..default()
        }),
        bevy::camera::Hdr,
        AmbientLight {
            color: Color::WHITE,
            brightness: 600.0,
            ..default()
        },
        Transform::default(),
        layers.clone(),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 7000.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::default().looking_to(Vec3::new(-0.4, -0.8, -0.5), Vec3::Y),
        layers.clone(),
    ));
    // The arm lies along -z, its shoulder at the origin and its hand at z = -0.6.
    let root = commands
        .spawn((
            ArmRoot,
            Transform::default(),
            Visibility::default(),
            layers.clone(),
        ))
        .id();
    for (color, len, mid) in [(SLEEVE, 0.3, -0.15), (SKIN, 0.3, -0.45)] {
        commands.spawn((
            ArmPart(color),
            Mesh3d(meshes.add(Cuboid::new(0.14, 0.14, len))),
            MeshMaterial3d(mats.add(StandardMaterial {
                base_color: color,
                perceptual_roughness: 1.0,
                ..default()
            })),
            Transform::from_xyz(0.0, 0.0, mid),
            layers.clone(),
            ChildOf(root),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn update(
    mut commands: Commands,
    player: Single<&Player>,
    vitals: Res<Vitals>,
    space: Res<Space>,
    (atlas, sprites): (Option<Res<Atlas>>, Option<Res<crate::textures::Sprites>>),
    time: Res<Time>,
    (mouse, cursor): (
        Res<ButtonInput<MouseButton>>,
        Single<&CursorOptions, With<PrimaryWindow>>,
    ),
    (rockets, riding): (Res<crate::rocket::Rockets>, Res<crate::elevator::Riding>),
    main: Single<&Camera, (With<crate::sky::SkyCamera>, Without<ViewCamera>)>,
    mut cam: Single<&mut Camera, With<ViewCamera>>,
    mut root: Single<(Entity, &mut Transform), With<ArmRoot>>,
    held: Query<Entity, With<Held>>,
    (mut meshes, mut mats, mut voxel_mats): (
        ResMut<Assets<Mesh>>,
        ResMut<Assets<StandardMaterial>>,
        ResMut<Assets<VoxelMaterial>>,
    ),
    parts: Query<(&ArmPart, &MeshMaterial3d<StandardMaterial>)>,
    mut state: Local<State>,
    (keyboard, menu): (
        Res<crate::chat::Keyboard>,
        Res<crate::start_menu::StartMenu>,
    ),
) {
    let dt = time.delta_secs();
    let b = &player.body;
    // (Not while talking: the arm would be drawn over the dialogue box.)
    let show = !menu.is_open()
        && main.is_active
        && !vitals.dead
        && !rockets.is_piloting()
        && riding.0.is_none()
        && keyboard.owner != Some(crate::chat::Owner::Dialogue);
    if cam.is_active != show {
        cam.is_active = show;
    }
    if !show {
        return;
    }

    // Swing: a click starts one, holding the button keeps swinging.
    // (`ORBIT_DEMO_SWING` swings all the time, for screenshots.)
    let clicking = (cursor.grab_mode == CursorGrabMode::Locked
        && (mouse.pressed(MouseButton::Left) || mouse.pressed(MouseButton::Right)))
        || std::env::var_os("ORBIT_DEMO_SWING").is_some();
    if state.swing >= SWING || state.swing < 0.0 {
        state.swing = if clicking { 0.0 } else { -1.0 };
    } else {
        state.swing += dt;
    }
    let s = if state.swing >= 0.0 {
        (state.swing / SWING).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let arc = (s * std::f32::consts::PI).sin();

    // Walk bob.
    let speed = if b.on_ground && !b.flying {
        b.vel.length().min(9.0)
    } else {
        0.0
    };
    state.phase += dt * (2.0 + speed * 1.1);
    state.walk += (speed / 9.0 - state.walk) * (1.0 - (-8.0 * dt).exp());
    let bob = Vec3::new(
        (state.phase * 0.5).cos() * 0.014,
        -(state.phase).sin().abs() * 0.02,
        0.0,
    ) * state.walk
        + Vec3::new(0.0, (time.elapsed_secs() * 1.3).sin() * 0.003, 0.0);

    let rest = Quat::from_euler(EulerRot::YXZ, 0.22, 0.12, 0.0);
    let swing = Quat::from_rotation_x(arc * 0.75) * Quat::from_rotation_y(-arc * 0.35);
    root.1.translation = Vec3::new(0.3, -0.27, -0.5) + bob + Vec3::new(-0.06, -0.04, -0.06) * arc;
    root.1.rotation = swing * rest;

    // What the hand holds and how bright it is where you stand.
    let item = player.inv.held().map(|s| s.item);
    let light = b
        .gravity_source
        .and_then(|id| space.planets.get(&id))
        .and_then(|p| p.voxel.as_ref())
        .and_then(|v| v.read().unwrap().light_at(b.pos + b.up * 1.0))
        .unwrap_or((15, 0));
    let shade = orbit_core::light::brightness(light.0.max(light.1) as f32).max(0.2);
    if (state.item, state.light) != (item, light) {
        state.item = item;
        state.light = light;
        for e in &held {
            commands.entity(e).despawn();
        }
        for (part, mat) in &parts {
            if let Some(mut m) = mats.get_mut(&mat.0) {
                m.base_color = part.0.mix(&Color::BLACK, 1.0 - shade);
            }
        }
        if let (Some(block), Some(atlas)) = (item, atlas) {
            spawn_held(
                &mut commands,
                root.0,
                block,
                light,
                &atlas,
                sprites.as_deref(),
                &mut meshes,
                &mut voxel_mats,
            );
        }
    }
}

#[derive(Default)]
struct State {
    swing: f32,
    phase: f32,
    walk: f32,
    item: Option<Block>,
    light: (u8, u8),
}

#[allow(clippy::too_many_arguments)]
fn spawn_held(
    commands: &mut Commands,
    arm: Entity,
    block: Block,
    light: (u8, u8),
    atlas: &Atlas,
    sprites: Option<&crate::textures::Sprites>,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<VoxelMaterial>,
) {
    let lit = [
        orbit_core::light::brightness(light.0 as f32),
        orbit_core::light::glow(light.1 as f32),
    ];
    // Tools, food and materials: their picture as a thin slab, held up and a little turned.
    if let Some(mask) = sprites
        .filter(|_| orbit_core::items::is_item(block))
        .and_then(|s| s.0.get((block - orbit_core::block::ITEM_BASE) as usize))
    {
        let (mesh, aabb) = gpu_mesh(
            meshes,
            orbit_core::mesh::sprite_mesh(orbit_core::items::tile(block), mask, 0.5, lit),
        );
        let mut m = VoxelMaterial::new([1.0, 0.95, 0.85], atlas.0.clone());
        m.place_flat(Vec3::ZERO, Vec3::new(0.3, 0.5, 0.8).normalize(), 1.0);
        commands.spawn((
            Held,
            Mesh3d(mesh),
            aabb,
            MeshMaterial3d(mats.add(m)),
            Transform::from_xyz(-0.02, -0.12, -0.55).with_rotation(Quat::from_euler(
                EulerRot::YXZ,
                0.5,
                -0.25,
                -0.1,
            )),
            RenderLayers::layer(LAYER),
            ChildOf(arm),
        ));
        return;
    }
    let (mesh, aabb) = gpu_mesh(meshes, orbit_core::mesh::item_mesh(block, lit));
    let mut m = VoxelMaterial::new([1.0, 0.95, 0.85], atlas.0.clone());
    m.place_flat(Vec3::ZERO, Vec3::new(0.45, 0.8, 0.4).normalize(), 1.0);
    commands.spawn((
        Held,
        Mesh3d(mesh),
        aabb,
        MeshMaterial3d(mats.add(m)),
        // Its bottom is its origin: centered on the hand, turned to show three faces.
        Transform::from_xyz(0.0, -0.045, -0.6)
            .with_rotation(Quat::from_euler(EulerRot::YXZ, 0.6, 0.35, 0.0))
            .with_scale(Vec3::splat(1.0)),
        RenderLayers::layer(LAYER),
        ChildOf(arm),
    ));
}
