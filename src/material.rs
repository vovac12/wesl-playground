//! The single material type used by every playground object.
//!
//! Bevy's `Material` trait picks shaders statically per type, so per-object shaders are handled
//! through specialization: the shader handles live in the material's bind group data (the
//! pipeline key), and `specialize` swaps them into the pipeline descriptor.

use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    reflect::TypePath,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::{ShaderDefVal, ShaderRef},
};

pub const DEFAULT_FRAGMENT: &str = "embedded://shadertoy/shaders/default_fragment.wesl";
const TOY_MODULE: &str = "embedded://shadertoy/shaders/toy.wesl";
const NOISE_MODULE: &str = "embedded://shadertoy/shaders/noise.wesl";

pub struct ToyMaterialPlugin;

impl Plugin for ToyMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/toy.wesl");
        embedded_asset!(app, "shaders/noise.wesl");
        embedded_asset!(app, "shaders/default_fragment.wesl");
        app.add_plugins(MaterialPlugin::<ToyMaterial>::default())
            .add_systems(Startup, load_library_shaders);
    }
}

/// Keeps the importable library modules loaded so user shaders can `import shadertoy::...`.
#[derive(Resource)]
#[expect(dead_code, reason = "handles are held only to keep the modules loaded")]
struct LibraryShaders(Vec<Handle<Shader>>);

fn load_library_shaders(mut commands: Commands, asset_server: Res<AssetServer>) {
    commands.insert_resource(LibraryShaders(vec![
        asset_server.load(TOY_MODULE),
        asset_server.load(NOISE_MODULE),
    ]));
}

#[derive(Clone, Default, Debug, ShaderType)]
pub struct ToyUniforms {
    pub color: Vec4,
    pub params0: Vec4,
    pub params1: Vec4,
    pub params2: Vec4,
    pub params3: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Clone, Debug)]
#[bind_group_data(ToyKey)]
pub struct ToyMaterial {
    #[uniform(0)]
    pub uniforms: ToyUniforms,
    #[texture(1)]
    #[sampler(2)]
    pub tex0: Option<Handle<Image>>,
    #[texture(3)]
    #[sampler(4)]
    pub tex1: Option<Handle<Image>>,
    #[texture(5)]
    #[sampler(6)]
    pub tex2: Option<Handle<Image>>,
    #[texture(7)]
    #[sampler(8)]
    pub tex3: Option<Handle<Image>>,
    pub alpha_mode: AlphaMode,
    pub double_sided: bool,
    pub vertex_shader: Option<Handle<Shader>>,
    pub fragment_shader: Option<Handle<Shader>>,
}

#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ToyKey {
    vertex_shader: Option<Handle<Shader>>,
    fragment_shader: Option<Handle<Shader>>,
    double_sided: bool,
}

impl From<&ToyMaterial> for ToyKey {
    fn from(material: &ToyMaterial) -> Self {
        Self {
            vertex_shader: material.vertex_shader.clone(),
            fragment_shader: material.fragment_shader.clone(),
            double_sided: material.double_sided,
        }
    }
}

impl Material for ToyMaterial {
    fn fragment_shader() -> ShaderRef {
        DEFAULT_FRAGMENT.into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        self.alpha_mode
    }

    // User shaders only target the main forward pass.
    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let is_prepass =
            descriptor.vertex.shader_defs.iter().any(
                |def| matches!(def, ShaderDefVal::Bool(name, true) if name == "PREPASS_PIPELINE"),
            );
        if is_prepass {
            return Ok(());
        }
        let key = key.bind_group_data;
        if let Some(vertex) = key.vertex_shader {
            descriptor.vertex.shader = vertex;
        }
        if let (Some(fragment), Some(state)) = (key.fragment_shader, descriptor.fragment.as_mut()) {
            state.shader = fragment;
        }
        if key.double_sided {
            descriptor.primitive.cull_mode = None;
        }
        Ok(())
    }
}

pub fn parse_alpha_mode(s: &str) -> Result<AlphaMode, String> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "" | "opaque" => AlphaMode::Opaque,
        "blend" => AlphaMode::Blend,
        "add" | "additive" => AlphaMode::Add,
        "multiply" => AlphaMode::Multiply,
        "premultiplied" => AlphaMode::Premultiplied,
        "mask" => AlphaMode::Mask(0.5),
        other => {
            return Err(format!(
                "unknown alpha_mode `{other}` (opaque|blend|add|multiply|premultiplied|mask)"
            ));
        }
    })
}
