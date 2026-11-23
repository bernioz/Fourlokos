mod atmosphere;
mod chat;
mod commands;
mod demo;
mod elevator;
mod elevator_parts;
mod inventory;
mod lighting;
mod map;
mod menu;
mod player;
mod rocket;
mod save;
mod schematic_menu;
mod sky;
mod start_menu;
mod station;
mod textures;
mod travel;
mod viz;
mod warp;
mod world;

use bevy::prelude::*;

fn main() {
    if std::env::args().any(|a| a == "--export-textures") {
        match textures::export() {
            Ok(dir) => println!("textures written to {}", dir.display()),
            Err(e) => eprintln!("could not write textures: {e}"),
        }
        return;
    }
    let seed = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1337u64);
    // With the start menu up, the world waits for Start; else the seed's starts now. Demos keep
    // the default universe, the rest of the game the fixed solar system.
    let title = start_menu::wanted();
    let space = if std::env::var("ORBIT_DEMO").is_ok() { world::Space::new(seed) } else { world::Space::with(seed, start_menu::fixed_gen()) };
    let mut app = App::new();
    app.insert_resource(ClearColor(Color::BLACK))
        .insert_resource(space)
        .insert_resource(sky::SkySeed(seed))
        .insert_resource(if title { save::SaveDir(None) } else { save::SaveDir::for_seed(seed) })
        .insert_resource(world::WorldLive(!title))
        .add_plugins(
            DefaultPlugins
                // Opaque: a compositor may otherwise take the frame's alpha and show the desktop
                // through it. Demos play by themselves: they should not take the keyboard from whatever you are doing.
                .set(WindowPlugin { primary_window: Some(Window { title: "Fourlokos".into(), focused: std::env::var("ORBIT_DEMO").is_err(), composite_alpha_mode: bevy::window::CompositeAlphaMode::Opaque, ..default() }), ..default() })
                // Models live with the crate (assets/models), wherever the game is run from.
                .set(AssetPlugin { file_path: concat!(env!("CARGO_MANIFEST_DIR"), "/assets").into(), ..default() }),
        )
        .add_plugins((atmosphere::AtmospherePlugin, lighting::LightingPlugin, world::WorldPlugin, player::PlayerPlugin, sky::SkyPlugin, viz::VizPlugin, menu::MenuPlugin, textures::TexturesPlugin))
        .add_plugins((travel::TravelPlugin, warp::WarpPlugin, map::MapPlugin, demo::DemoPlugin, chat::ChatPlugin, station::StationsPlugin, save::SavePlugin, schematic_menu::SchematicMenuPlugin, elevator::ElevatorsPlugin))
        .add_plugins((rocket::RocketsPlugin, inventory::InventoryPlugin, start_menu::StartMenuPlugin));
    // Mesh buffers (slabs) are kept well under what GPUs allow in one buffer (often 256 MiB):
    // Bevy's 512 MiB default fails with "Out of Memory" once planets fill a slab.
    let slabs = bevy::render::slab_allocator::SlabAllocatorSettings { max_slab_size: 128 << 20, large_threshold: 64 << 20, ..default() };
    app.sub_app_mut(bevy::render::RenderApp)
        .insert_resource(bevy::render::mesh::allocator::MeshAllocatorSettings { slab_allocator_settings: slabs, ..default() });
    app.run();
}
