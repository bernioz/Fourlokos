//! Planet lighting: a material for voxel chunks and far meshes that lights them from their own
//! star, darkens each planet's night side and uses the sky and torch light baked into the meshes;
//! and one for models (the elevator's cabin, ribbon, base) lit the same way, so they sit in the
//! world as blocks do instead of going black on their shaded side; and one for water (see-through,
//! moving, mirroring the sky).

use bevy::asset::embedded_asset;
use bevy::mesh::MeshVertexBufferLayoutRef;
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey, MaterialPlugin};
use bevy::render::render_resource::{RenderPipelineDescriptor, SpecializedMeshPipelineError};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use orbit_core::block::tile;

/// Sunlight at a planet, lux.
const SUN_LUX: f32 = 9000.0;
/// Ambient sky light by day and at night (cd/m², like `GlobalAmbientLight::brightness`).
const DAY_AMBIENT: Vec3 = Vec3::new(0.6 * 450.0, 0.65 * 450.0, 0.9 * 450.0);
const NIGHT_AMBIENT: Vec3 = Vec3::new(0.3 * 25.0, 0.38 * 25.0, 0.6 * 25.0);

pub struct LightingPlugin;

impl Plugin for LightingPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "voxel.wesl");
        embedded_asset!(app, "liquid.wesl");
        embedded_asset!(app, "model.wesl");
        app.init_resource::<Submerged>();
        app.add_plugins((
            MaterialPlugin::<VoxelMaterial>::default(),
            MaterialPlugin::<LiquidMaterial>::default(),
            MaterialPlugin::<ModelMaterial>::default(),
        ));
    }
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct VoxelMaterial {
    #[uniform(0)]
    pub light: VoxelLight,
    /// The block texture atlas (textures.rs).
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

#[derive(ShaderType, Clone, Copy, Debug, Default)]
pub struct VoxelLight {
    sun_dir: Vec4,
    sun: Vec4,
    day_ambient: Vec4,
    night_ambient: Vec4,
    center: Vec4,
    /// xyz: world point, w: radius; fragments closer than that are cut (0 = off).
    hole: Vec4,
    /// xyz: the star (render space), w: its radius; 0: no eclipses.
    star: Vec4,
    /// Bodies that can pass in front of the star: xyz center (render space), w radius (0: none).
    occ0: Vec4,
    occ1: Vec4,
    /// x, y: the atlas's tiles per row and rows (to find a point's place in its tile).
    atlas: Vec4,
    /// Fog while the eyes are under water or in lava: rgb its color (times the ambient light),
    /// w how thick (0: none).
    fog: Vec4,
}

/// The liquid the camera is in, if any: everything is seen through it (fog).
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct Submerged(pub Option<orbit_core::block::Liquid>);

impl VoxelMaterial {
    /// A planet lit by a star of the given color.
    pub fn new(star_color: [f32; 3], atlas: Handle<Image>) -> Self {
        // Mostly white, tinted toward the star's color.
        let tint = Vec3::ONE.lerp(Vec3::from_array(star_color), 0.35);
        let atlas_size = Vec4::new(tile::COLUMNS as f32, tile::ROWS as f32, 0.0, 0.0);
        let light = VoxelLight { sun: (tint * SUN_LUX).extend(0.0), day_ambient: DAY_AMBIENT.extend(0.0), night_ambient: NIGHT_AMBIENT.extend(0.0), atlas: atlas_size, ..default() };
        Self { light, atlas }
    }

    /// Follows the planet (center) and its star.
    pub fn place(&mut self, center: Vec3, star: Vec3) {
        self.light.center = center.extend(0.0);
        self.light.sun_dir = (star - center).normalize_or(Vec3::Y).extend(0.0);
    }

    /// Lights something small and flat (a station) from a star in direction `sun_dir`, with the
    /// sun `height` over its horizon (`orbit_core::station::sun_height`: 1 in full sun, -1 deep
    /// in a planet's shadow). Its center is put far away so the shader's "up" makes that angle
    /// with the sun, whatever it is at.
    pub fn place_flat(&mut self, at: Vec3, sun_dir: Vec3, height: f32) {
        let l = sun_dir.normalize_or(Vec3::Y);
        let h = height.clamp(-1.0, 1.0);
        let up = l * h + l.any_orthonormal_vector() * (1.0 - h * h).sqrt();
        self.light.center = (at - up * 20_000.0).extend(0.0);
        self.light.sun_dir = l.extend(0.0);
    }

    /// Eclipses: the star (render space) and its radius, and up to two bodies (render-space
    /// center, radius) that can pass in front of it.
    pub fn eclipses(&mut self, star: Vec3, star_radius: f32, bodies: &[(Vec3, f32)]) {
        self.light.star = star.extend(star_radius);
        let mut it = bodies.iter().map(|&(c, r)| c.extend(r));
        self.light.occ0 = it.next().unwrap_or(Vec4::ZERO);
        self.light.occ1 = it.next().unwrap_or(Vec4::ZERO);
    }

    /// Fogs the view for eyes in a liquid.
    pub fn submerge(&mut self, s: Submerged) {
        use orbit_core::block::Liquid;
        self.light.fog = match s.0 {
            Some(Liquid::Water) => Vec4::new(0.05, 0.2, 0.42, 0.11),
            Some(Liquid::Lava) => Vec4::new(6.0, 1.6, 0.2, 1.2),
            None => Vec4::ZERO,
        };
    }

    /// Cuts everything within `radius` of `at` (a far mesh where voxel chunks stand); 0 cuts nothing.
    pub fn cut(&mut self, at: Vec3, radius: f32) {
        self.light.hole = at.extend(radius);
    }
}

impl Material for VoxelMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fourlokos/voxel.wesl".into()
    }
}

/// Water: see-through, waves that move, the sky and the sun on it. Lit like `VoxelMaterial` (it
/// copies one's light with `follow`); drawn from both sides, so the surface shows from below.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct LiquidMaterial {
    #[uniform(0)]
    pub light: VoxelLight,
    #[texture(1)]
    #[sampler(2)]
    pub atlas: Handle<Image>,
}

impl LiquidMaterial {
    pub fn new(of: &VoxelMaterial) -> Self {
        Self { light: of.light, atlas: of.atlas.clone() }
    }

    /// Lit the same as `m`.
    pub fn follow(&mut self, m: &VoxelMaterial) {
        self.light = m.light;
    }
}

impl Material for LiquidMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fourlokos/liquid.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }

    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// A model (glTF or built here) lit like the blocks around it: by its star, dark in its planet's
/// shadow, with the same sky light as the ground by day and starlight by night. Its texture
/// (none: white) times `color`; both sides of each face drawn.
#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
pub struct ModelMaterial {
    #[uniform(0)]
    pub light: VoxelLight,
    #[uniform(1)]
    pub look: ModelLook,
    #[texture(2)]
    #[sampler(3)]
    pub texture: Option<Handle<Image>>,
    /// Blend for glass; otherwise opaque, with see-through texels cut out.
    pub see_through: bool,
}

#[derive(ShaderType, Clone, Copy, Debug)]
pub struct ModelLook {
    pub color: Vec4,
    /// x: the planet's radius (where its shadow starts), y: glass (1), a hologram (2) or neither, zw: how many times
    /// the texture repeats along u and v.
    pub planet: Vec4,
    /// A tile of the block atlas to repeat over the faces (xy: its corner, zw: its size in uv);
    /// zero: the texture as it is.
    pub tile: Vec4,
}

impl Default for ModelLook {
    fn default() -> Self {
        Self { color: Vec4::ONE, planet: Vec4::new(1.0, 0.0, 1.0, 1.0), tile: Vec4::ZERO }
    }
}

impl ModelMaterial {
    pub fn new(star_color: [f32; 3], texture: Option<Handle<Image>>, see_through: bool) -> Self {
        let VoxelMaterial { light, .. } = VoxelMaterial::new(star_color, Handle::default());
        let mut look = ModelLook::default();
        look.planet.y = if see_through { 1.0 } else { 0.0 };
        Self { light, look, texture, see_through }
    }

    /// A tile of the block atlas (`texture` must be the atlas), once per block over the faces.
    pub fn atlas_tile(star_color: [f32; 3], atlas: Handle<Image>, t: u16) -> Self {
        let mut m = Self::new(star_color, Some(atlas), false);
        let [u0, v0] = orbit_core::block::tile::uv(t, 0.0, 0.0);
        let [u1, v1] = orbit_core::block::tile::uv(t, 1.0, 1.0);
        m.look.tile = Vec4::new(u0, v0, u1 - u0, v1 - v0);
        m
    }

    /// Follows its planet (render-space center and radius), the planet's star and what can
    /// eclipse it, as `VoxelMaterial::place` and `eclipses` do.
    pub fn place(&mut self, center: Vec3, radius: f32, star: Vec3, star_radius: f32, bodies: &[(Vec3, f32)]) {
        self.light.center = center.extend(0.0);
        self.light.sun_dir = (star - center).normalize_or(Vec3::Y).extend(0.0);
        self.look.planet.x = radius;
        self.light.star = star.extend(star_radius);
        let mut it = bodies.iter().map(|&(c, r)| c.extend(r));
        self.light.occ0 = it.next().unwrap_or(Vec4::ZERO);
        self.light.occ1 = it.next().unwrap_or(Vec4::ZERO);
    }
}

impl Material for ModelMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://fourlokos/model.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        if self.see_through { AlphaMode::Blend } else { AlphaMode::Opaque }
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(_: &MaterialPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: MaterialPipelineKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}
