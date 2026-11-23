//! Props glTF para cultivos. La malla voxel original se conserva como fallback y como superficie
//! de selección; estos props sólo mejoran la representación de las celdas cargadas.

use crate::world::Space;
use bevy::prelude::*;
use orbit_core::block;
use orbit_core::planet::ChunkKey;
use orbit_core::sphere::CellId;
use std::collections::HashSet;

const SUNFLOWER: &str = "models/sunflower.gltf";
const TOMATO: &str = "models/tomato_bush.gltf";
const AVOCADO: &str = "models/avocado_tree.gltf";
/// Blockbench plants use pixels, while the generated farm models are already in metres.
const BLOCKBENCH_PIXEL: f32 = 1.0 / 16.0;

pub struct CropModelsPlugin;

impl Plugin for CropModelsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load_assets)
            .add_systems(Update, sync_crop_models.after(crate::world::MovePlanets));
    }
}

#[derive(Resource)]
struct CropAssets {
    sunflower: Handle<WorldAsset>,
    tomato: Handle<WorldAsset>,
    avocado: Handle<WorldAsset>,
}

#[derive(Component)]
struct CropModel;

fn load_assets(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CropAssets {
        sunflower: server.load(GltfAssetLabel::Scene(0).from_asset(SUNFLOWER)),
        tomato: server.load(GltfAssetLabel::Scene(0).from_asset(TOMATO)),
        avocado: server.load(GltfAssetLabel::Scene(0).from_asset(AVOCADO)),
    });
}

fn model_for(b: block::Block) -> Option<(&'static str, usize)> {
    let stage = orbit_core::crop::stage(b)?;
    if (block::SUNFLOWER_0..=block::SUNFLOWER_3).contains(&b) {
        Some((SUNFLOWER, stage))
    } else if (block::TOMATO_0..=block::TOMATO_3).contains(&b) {
        Some((TOMATO, stage))
    } else if (block::PALTO_0..=block::PALTO_1).contains(&b) {
        Some((AVOCADO, stage))
    } else {
        None
    }
}

fn scene_for<'a>(assets: &'a CropAssets, path: &str) -> &'a Handle<WorldAsset> {
    match path {
        SUNFLOWER => &assets.sunflower,
        TOMATO => &assets.tomato,
        _ => &assets.avocado,
    }
}

fn stage_scale(path: &str, b: block::Block, stage: usize) -> f32 {
    let max = orbit_core::crop::crop(b).map_or(0, |c| c.stages.len().saturating_sub(1));
    // Keep the mature asset at its authored size (roughly one crop cell), while making the
    // earlier voxel stages visibly smaller. Sunflower is the only pixel-unit asset here.
    let growth = if max == 0 {
        1.0
    } else {
        0.35 + 0.65 * stage as f32 / max as f32
    };
    let authored_scale = if path == SUNFLOWER { BLOCKBENCH_PIXEL } else { 1.0 };
    authored_scale * growth
}

/// Chunks revisados por frame: el escaneo completo de celdas era el origen del lag.
const CHUNKS_PER_FRAME: usize = 3;

#[derive(Default)]
struct SyncState {
    /// Entidad de cada cultivo con modelo, por (planeta, celda).
    models: std::collections::HashMap<(u64, CellId), (Entity, block::Block)>,
    /// Chunk al que pertenece cada modelo.
    chunk_of: std::collections::HashMap<(u64, CellId), ChunkKey>,
    cursor: usize,
}

fn sync_crop_models(
    mut commands: Commands,
    space: Res<Space>,
    assets: Option<Res<CropAssets>>,
    mut state: Local<SyncState>,
) {
    let Some(assets) = assets else { return };
    let state = &mut *state;

    for slot in space.planets.values() {
        let Some(voxel) = &slot.voxel else { continue };
        let Ok(planet) = voxel.try_read() else { continue };
        let id = slot.spec.id;
        let keys: Vec<ChunkKey> = slot.loaded_chunks().collect();
        let loaded: HashSet<ChunkKey> = keys.iter().copied().collect();

        // Modelos de chunks descargados.
        let gone: Vec<(u64, CellId)> = state
            .chunk_of
            .iter()
            .filter(|((p, _), k)| *p == id && !loaded.contains(k))
            .map(|(c, _)| *c)
            .collect();
        for c in gone {
            if let Some((e, _)) = state.models.remove(&c) {
                commands.entity(e).despawn();
            }
            state.chunk_of.remove(&c);
        }
        if keys.is_empty() {
            continue;
        }

        for n in 0..CHUNKS_PER_FRAME.min(keys.len()) {
            let key = keys[(state.cursor + n) % keys.len()];
            let mut wanted = HashSet::new();
            for cell in planet.chunk_cells(key) {
                let b = planet.get(cell);
                let Some((path, stage)) = model_for(b) else { continue };
                wanted.insert(cell);
                if state.models.get(&(id, cell)).is_some_and(|(_, old)| *old == b) {
                    continue;
                }
                let up = planet.grid.center(cell).normalize_or(Vec3::Y);
                let transform = Transform {
                    translation: planet.grid.point(cell, 0.5, 0.5, 0.0),
                    rotation: Quat::from_rotation_arc(Vec3::Y, up),
                    scale: Vec3::splat(stage_scale(path, b, stage)),
                };
                let entity = commands
                    .spawn((
                        WorldAssetRoot(scene_for(&assets, path).clone()),
                        CropModel,
                        transform,
                        Visibility::default(),
                        ChildOf(slot.root),
                    ))
                    .id();
                if let Some((old, _)) = state.models.insert((id, cell), (entity, b)) {
                    commands.entity(old).despawn();
                }
                state.chunk_of.insert((id, cell), key);
            }
            // Cultivos cosechados o rotos en este chunk.
            let stale: Vec<(u64, CellId)> = state
                .chunk_of
                .iter()
                .filter(|((p, c), k)| *p == id && **k == key && !wanted.contains(c))
                .map(|(c, _)| *c)
                .collect();
            for c in stale {
                if let Some((e, _)) = state.models.remove(&c) {
                    commands.entity(e).despawn();
                }
                state.chunk_of.remove(&c);
            }
        }
        state.cursor = (state.cursor + CHUNKS_PER_FRAME) % keys.len().max(1);
    }
}
