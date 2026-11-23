use orbit_core::{mesh::planet_meshes, planet::Planet, universe::Universe};
fn main() {
    let u = Universe::new(1337);
    let sys = u.system(Default::default()).unwrap();
    println!("star r {:.0} at {:?}", sys.star_radius, sys.center);
    for p in sys.planets {
        let t = std::time::Instant::now();
        let pl = Planet::generate(p.clone());
        let tg = t.elapsed();
        let tris: usize = planet_meshes(&pl)
            .iter()
            .map(|(_, m)| m.indices.len() / 3)
            .sum();
        println!(
            "{} r{:.0} {:?} dist {:.0} gen {:?} total {:?} tris {}",
            p.name,
            p.radius,
            p.biome,
            p.center.length(),
            tg,
            t.elapsed(),
            tris
        );
    }
}
