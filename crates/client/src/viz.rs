//! Debug/guide overlays toggled by keys: O shows orbits, G shows each planet's atmosphere
//! (the edge of its gravity) as a wireframe globe that turns with the planet.

use crate::world::{Origin, Space};
use bevy::prelude::*;
use std::f32::consts::{FRAC_PI_2, PI};

pub struct VizPlugin;

impl Plugin for VizPlugin {
    fn build(&self, app: &mut App) {
        // ORBIT_VIZ=1 starts with both overlays on.
        let on = std::env::var("ORBIT_VIZ").is_ok_and(|v| v == "1");
        app.insert_resource(Viz {
            orbits: on,
            atmospheres: on,
        })
        .add_systems(
            Update,
            (toggle, draw).chain().after(crate::player::sync_camera),
        );
    }
}

#[derive(Resource, Default)]
pub struct Viz {
    pub orbits: bool,
    pub atmospheres: bool,
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut viz: ResMut<Viz>) {
    if keys.just_pressed(KeyCode::KeyO) {
        viz.orbits = !viz.orbits;
    }
    if keys.just_pressed(KeyCode::KeyG) {
        viz.atmospheres = !viz.atmospheres;
    }
}

/// What a planet goes round: its star, or a moon's planet.
fn orbit_center(space: &Space, spec: &orbit_core::planet::PlanetSpec) -> bevy::math::DVec3 {
    match spec.host {
        Some(h) => space
            .planets
            .get(&h.id)
            .map_or(spec.star(), |p| p.spec.center),
        None => spec.orbit.star,
    }
}

fn draw(viz: Res<Viz>, space: Res<Space>, origin: Res<Origin>, mut gizmos: Gizmos) {
    for slot in space.planets.values() {
        let spec = &slot.spec;
        if viz.orbits {
            // Gizmo circles lie in their local XY plane; the orbit lies in the tilted XZ plane.
            let rot = spec.orbit.tilt * Quat::from_rotation_x(FRAC_PI_2);
            gizmos
                .circle(
                    Isometry3d::new(origin.render(orbit_center(&space, spec)), rot),
                    spec.orbit.radius,
                    Color::srgba(0.5, 0.75, 1.0, 0.45),
                )
                .resolution(256);
        }
        if viz.atmospheres {
            let r = spec.gravity_radius();
            let color = Color::srgba(0.45, 0.8, 1.0, 0.35);
            // Meridians through the spin axis...
            for k in 0..8 {
                let rot = spec.rotation * Quat::from_rotation_y(k as f32 * PI / 8.0);
                gizmos
                    .circle(Isometry3d::new(origin.render(spec.center), rot), r, color)
                    .resolution(64);
            }
            // ...and parallels around it.
            for k in 1..8 {
                let lat = -FRAC_PI_2 + k as f32 * PI / 8.0;
                let at = origin.render(spec.center) + spec.rotation * Vec3::Y * (r * lat.sin());
                let rot = spec.rotation * Quat::from_rotation_x(FRAC_PI_2);
                gizmos
                    .circle(Isometry3d::new(at, rot), r * lat.cos(), color)
                    .resolution(64);
            }
        }
    }
}
