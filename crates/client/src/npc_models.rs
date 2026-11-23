//! placeholder: replace with the real model
//!
//! Bodoque and Tulio drawn as figures of colored boxes (the real models are not made yet). Each
//! figure stands on its feet at the origin, facing -Z (as the slime's glTF does), and is lit like
//! every other model (`ModelMaterial`). Tulio's face is the reference picture
//! (`assets/models/tulio_face.png`, cut from `art/tulio/closeup-head-front.png`) on a flat quad in
//! front of his head. To use a real model, load it where `mobs::draw_mobs` calls `build` and
//! delete this file.

use crate::lighting::ModelMaterial;
use bevy::image::{ImageLoaderSettings, ImageSampler};
use bevy::prelude::*;
use orbit_core::mob::Kind;

const TULIO_FACE: &str = "models/tulio_face.png";

/// One box: its middle (from the feet), its size, its color.
struct Part(Vec3, Vec3, [f32; 3]);

const fn part(cx: f32, cy: f32, cz: f32, sx: f32, sy: f32, sz: f32, c: [f32; 3]) -> Part {
    Part(Vec3::new(cx, cy, cz), Vec3::new(sx, sy, sz), c)
}

const RED: [f32; 3] = [0.78, 0.12, 0.10];
const DARK_RED: [f32; 3] = [0.55, 0.07, 0.07];
const PINK: [f32; 3] = [0.95, 0.55, 0.60];
const WHITE: [f32; 3] = [0.95, 0.95, 0.95];
const BLACK: [f32; 3] = [0.05, 0.05, 0.06];
const SUIT: [f32; 3] = [0.55, 0.40, 0.30];
const SKIN: [f32; 3] = [0.72, 0.68, 0.60];
const TIE: [f32; 3] = [0.90, 0.48, 0.20];
const SHOE: [f32; 3] = [0.32, 0.26, 0.22];
const CHICKEN: [f32; 3] = [0.95, 0.95, 0.92];
const COMB: [f32; 3] = [0.85, 0.08, 0.06];
const BEAK: [f32; 3] = [0.95, 0.60, 0.08];

/// Bodoque: a short red rabbit.
fn bodoque() -> Vec<Part> {
    vec![
        part(-0.14, 0.2, 0.0, 0.2, 0.4, 0.26, DARK_RED),
        part(0.14, 0.2, 0.0, 0.2, 0.4, 0.26, DARK_RED),
        part(0.0, 0.7, 0.0, 0.58, 0.6, 0.38, RED),
        part(-0.38, 0.72, 0.0, 0.16, 0.46, 0.16, RED),
        part(0.38, 0.72, 0.0, 0.16, 0.46, 0.16, RED),
        part(0.0, 1.2, 0.0, 0.5, 0.44, 0.44, RED),
        part(-0.14, 1.66, 0.0, 0.13, 0.5, 0.1, RED),
        part(0.14, 1.66, 0.0, 0.13, 0.5, 0.1, RED),
        part(-0.14, 1.66, -0.05, 0.07, 0.4, 0.02, PINK),
        part(0.14, 1.66, -0.05, 0.07, 0.4, 0.02, PINK),
        part(-0.12, 1.26, -0.23, 0.1, 0.1, 0.03, WHITE),
        part(0.12, 1.26, -0.23, 0.1, 0.1, 0.03, WHITE),
        part(-0.12, 1.26, -0.25, 0.04, 0.04, 0.02, BLACK),
        part(0.12, 1.26, -0.25, 0.04, 0.04, 0.02, BLACK),
        part(0.0, 1.14, -0.24, 0.1, 0.07, 0.04, PINK),
    ]
}

/// Tulio: a news anchor in a brown suit and an orange tie, a puppet's head.
fn tulio() -> Vec<Part> {
    vec![
        part(-0.14, 0.4, 0.0, 0.2, 0.8, 0.24, SUIT),
        part(0.14, 0.4, 0.0, 0.2, 0.8, 0.24, SUIT),
        part(-0.14, 0.05, -0.04, 0.22, 0.1, 0.32, SHOE),
        part(0.14, 0.05, -0.04, 0.22, 0.1, 0.32, SHOE),
        part(0.0, 1.15, 0.0, 0.62, 0.7, 0.32, SUIT),
        part(0.0, 1.1, -0.17, 0.1, 0.42, 0.02, TIE),
        part(-0.4, 1.05, 0.0, 0.17, 0.62, 0.2, SUIT),
        part(0.4, 1.05, 0.0, 0.17, 0.62, 0.2, SUIT),
        part(-0.4, 0.68, 0.0, 0.16, 0.14, 0.18, SKIN),
        part(0.4, 0.68, 0.0, 0.16, 0.14, 0.18, SKIN),
        part(0.0, 1.55, 0.0, 0.2, 0.18, 0.2, SKIN),
        part(0.0, 1.76, 0.0, 0.46, 0.38, 0.4, SKIN),
        part(-0.3, 1.78, 0.0, 0.14, 0.26, 0.2, SKIN),
        part(0.3, 1.78, 0.0, 0.14, 0.26, 0.2, SKIN),
    ]
}

/// Gallina placeholder: cajas blancas con cresta y pico rojos/amarillos.
fn chicken() -> Vec<Part> {
    vec![
        part(0.0, 0.32, 0.0, 0.62, 0.58, 0.48, CHICKEN),
        part(0.0, 0.78, -0.03, 0.48, 0.42, 0.42, CHICKEN),
        part(0.0, 0.88, -0.27, 0.18, 0.12, 0.12, BEAK),
        part(0.0, 1.03, -0.02, 0.16, 0.18, 0.08, COMB),
        part(-0.18, 0.06, 0.0, 0.10, 0.36, 0.10, BEAK),
        part(0.18, 0.06, 0.0, 0.10, 0.36, 0.10, BEAK),
    ]
}

/// What the figure is made of, and its height.
fn parts(kind: Kind) -> Option<Vec<Part>> {
    match kind {
        Kind::Bodoque => Some(bodoque()),
        Kind::Tulio => Some(tulio()),
        Kind::Chicken => Some(chicken()),
        _ => None,
    }
}

pub struct Built {
    pub root: Entity,
    pub mats: Vec<(Handle<ModelMaterial>, Vec4)>,
}

/// Spawns the figure as a child of `parent` (a planet's root); None for a kind with no figure.
pub fn build(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ModelMaterial>,
    assets: &AssetServer,
    kind: Kind,
    parent: Entity,
    star: [f32; 3],
) -> Option<Built> {
    let parts = parts(kind)?;
    let mut by_color: Vec<([f32; 3], Handle<ModelMaterial>)> = Vec::new();
    let mut mats = Vec::new();
    let root = commands
        .spawn((Transform::default(), Visibility::default(), ChildOf(parent)))
        .id();
    let mut look = |c: [f32; 3],
                    materials: &mut Assets<ModelMaterial>,
                    mats: &mut Vec<(Handle<ModelMaterial>, Vec4)>|
     -> Handle<ModelMaterial> {
        if let Some((_, h)) = by_color.iter().find(|(k, _)| *k == c) {
            return h.clone();
        }
        let mut m = ModelMaterial::new(star, None, false);
        m.look.color = Vec4::new(c[0], c[1], c[2], 1.0);
        let base = m.look.color;
        let h = materials.add(m);
        mats.push((h.clone(), base));
        by_color.push((c, h.clone()));
        h
    };
    for Part(at, size, color) in parts {
        let h = look(color, materials, &mut mats);
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
            MeshMaterial3d(h),
            Transform::from_translation(at),
            ChildOf(root),
        ));
    }
    if kind == Kind::Tulio {
        // The reference face, on a quad just in front of the head (-Z), cut out where it is clear.
        let tex: Handle<Image> = assets
            .load_builder()
            .with_settings(|s: &mut ImageLoaderSettings| s.sampler = ImageSampler::nearest())
            .load(TULIO_FACE);
        let mut m = ModelMaterial::new(star, Some(tex), false);
        m.look.color = Vec4::ONE;
        let h = materials.add(m);
        mats.push((h.clone(), Vec4::ONE));
        // 590 x 278 pixels of picture: the head is 0.46 wide there, ears and all 0.74.
        let quad = meshes.add(Rectangle::new(0.74, 0.74 * 278.0 / 590.0));
        commands.spawn((
            Mesh3d(quad),
            MeshMaterial3d(h),
            Transform::from_xyz(0.0, 1.78, -0.215)
                .with_rotation(Quat::from_rotation_y(std::f32::consts::PI)),
            ChildOf(root),
        ));
    }
    Some(Built { root, mats })
}

/// The Slime King's big gold crown, boxes over the (three times bigger) slime's own small one.
/// placeholder: replace with the real model. In the slime model's own units (the model is scaled).
pub fn king_crown(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<ModelMaterial>,
    star: [f32; 3],
    parent: Entity,
) -> Vec<(Handle<ModelMaterial>, Vec4)> {
    const GOLD: [f32; 3] = [1.0, 0.78, 0.15];
    const GEM: [f32; 3] = [0.9, 0.1, 0.25];
    let boxes = [
        // The band, then a prong at each corner and a taller one in the middle, and a gem.
        (Vec3::new(0.0, 0.9, 0.0), Vec3::new(0.82, 0.12, 0.82), GOLD),
        (
            Vec3::new(-0.34, 1.02, -0.34),
            Vec3::new(0.14, 0.16, 0.14),
            GOLD,
        ),
        (
            Vec3::new(0.34, 1.02, -0.34),
            Vec3::new(0.14, 0.16, 0.14),
            GOLD,
        ),
        (
            Vec3::new(-0.34, 1.02, 0.34),
            Vec3::new(0.14, 0.16, 0.14),
            GOLD,
        ),
        (
            Vec3::new(0.34, 1.02, 0.34),
            Vec3::new(0.14, 0.16, 0.14),
            GOLD,
        ),
        (Vec3::new(0.0, 1.05, 0.0), Vec3::new(0.16, 0.22, 0.16), GOLD),
        (
            Vec3::new(0.0, 0.92, -0.42),
            Vec3::new(0.14, 0.14, 0.04),
            GEM,
        ),
    ];
    let mut mats = Vec::new();
    for (at, size, c) in boxes {
        let mut m = ModelMaterial::new(star, None, false);
        m.look.color = Vec4::new(c[0], c[1], c[2], 1.0);
        let base = m.look.color;
        let h = materials.add(m);
        mats.push((h.clone(), base));
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(size.x, size.y, size.z))),
            MeshMaterial3d(h),
            Transform::from_translation(at),
            ChildOf(parent),
        ));
    }
    mats
}
