use std::{cell::RefCell, mem::size_of, num::NonZeroU64};

use bevy::{
    core_pipeline::{core_3d::Camera3d, prepass::ViewPrepassTextures},
    ecs::system::Resource,
    prelude::default,
    render::{
        render_graph::{RenderLabel, ViewNode},
        render_resource::{
            BindGroup, BindGroupLayout, BindGroupLayoutEntry, Buffer, RenderPipeline, TextureView,
        },
        renderer::{RenderDevice, RenderQueue},
        texture::{CachedTexture, ColorAttachment},
        view::{PostProcessWrite, ViewTarget},
    },
};
use bumpalo::Bump;
use cgmath::{Deg, Matrix4, SquareMatrix as _, Vector3, Zero as _};

use crate::{
    client::{
        entity::MAX_LIGHTS,
        input::InputFocus,
        menu::Menu,
        render::{
            pipeline::Pipeline, ui::quad::QuadPipeline, GraphicsState, RenderConnectionKind,
            RenderResolution, RenderState, RenderVars, WorldRenderer,
        },
    },
    common::{
        console::{ConsoleInput, ConsoleOutput},
        util::any_as_bytes,
    },
};

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
    pub origin: Vector3<f32>,
    pub radius: f32,
}

#[repr(C, align(256))]
#[derive(Clone, Copy, Debug)]
pub struct DeferredUniforms {
    pub inv_projection: [[f32; 4]; 4],
    pub light_count: u32,
    pub _pad: [u32; 3],
    pub lights: [PointLight; MAX_LIGHTS],
}

pub struct DeferredPipeline {
    pipeline: RenderPipeline,
    bind_group_layouts: Vec<BindGroupLayout>,
    uniform_buffer: Buffer,
}

impl DeferredPipeline {
    pub fn new(
        device: &RenderDevice,
        compiler: &mut shaderc::Compiler,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> DeferredPipeline {
        let (pipeline, bind_group_layouts) =
            DeferredPipeline::create(device, compiler, &[], sample_count, format);

        let uniform_buffer = device.create_buffer_with_data(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: unsafe {
                any_as_bytes(&DeferredUniforms {
                    inv_projection: Matrix4::identity().into(),
                    light_count: 0,
                    _pad: [0; 3],
                    lights: [PointLight {
                        origin: Vector3::zero(),
                        radius: 0.0,
                    }; MAX_LIGHTS],
                })
            },
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        DeferredPipeline {
            pipeline,
            bind_group_layouts,
            uniform_buffer,
        }
    }

    pub fn rebuild(
        &mut self,
        device: &RenderDevice,
        compiler: &mut shaderc::Compiler,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        let layout_refs = self.bind_group_layouts.iter();
        let pipeline = Self::recreate(device, compiler, layout_refs, sample_count, format);
        self.pipeline = pipeline;
    }

    pub fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub fn bind_group_layouts(&self) -> &[BindGroupLayout] {
        &self.bind_group_layouts
    }

    pub fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }
}

const BIND_GROUP_LAYOUT_ENTRIES: &[wgpu::BindGroupLayoutEntry] = &[
    // sampler
    wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    },
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
        count: None,
    },
    // color buffer
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            multisampled: false,
        },
        count: None,
    },
    // normal buffer
    wgpu::BindGroupLayoutEntry {
        binding: 3,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            multisampled: false,
        },
        count: None,
    },
    // depth buffer
    wgpu::BindGroupLayoutEntry {
        binding: 4,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            multisampled: false,
        },
        count: None,
    },
    // uniform buffer
    wgpu::BindGroupLayoutEntry {
        binding: 5,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size_of::<DeferredUniforms>() as u64),
        },
        count: None,
    },
];

impl Pipeline for DeferredPipeline {
    type VertexPushConstants = ();
    type SharedPushConstants = ();
    type FragmentPushConstants = ();

    type Args = wgpu::TextureFormat;

    fn name() -> &'static str {
        "deferred"
    }

    fn bind_group_layout_descriptors() -> Vec<Vec<BindGroupLayoutEntry>> {
        vec![BIND_GROUP_LAYOUT_ENTRIES.to_owned()]
    }

    fn vertex_shader() -> &'static str {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shaders/deferred.vert"
        ))
    }

    fn fragment_shader() -> &'static str {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shaders/deferred.frag"
        ))
    }

    fn primitive_state() -> wgpu::PrimitiveState {
        QuadPipeline::primitive_state()
    }

    fn color_target_states_with_args(format: Self::Args) -> Vec<Option<wgpu::ColorTargetState>> {
        vec![Some(wgpu::ColorTargetState {
            format: format,
            blend: Some(wgpu::BlendState::REPLACE),
            write_mask: wgpu::ColorWrites::ALL,
        })]
    }

    fn depth_stencil_state() -> Option<wgpu::DepthStencilState> {
        None
    }

    fn vertex_buffer_layouts() -> Vec<wgpu::VertexBufferLayout<'static>> {
        QuadPipeline::vertex_buffer_layouts()
    }
}

#[derive(Resource)]
pub struct DeferredRenderer {
    bind_group: BindGroup,
}

impl DeferredRenderer {
    fn create_bind_group(
        state: &GraphicsState,
        device: &RenderDevice,
        diffuse_buffer: &TextureView,
        normal_buffer: &TextureView,
        depth_buffer: &TextureView,
    ) -> BindGroup {
        device.create_bind_group(
            Some("deferred bind group"),
            &state.deferred_pipeline().bind_group_layouts()[0],
            &[
                // sampler
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(state.diffuse_sampler()),
                },
                // sampler
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(state.nearest_sampler()),
                },
                // diffuse buffer
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(diffuse_buffer),
                },
                // normal buffer
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(normal_buffer),
                },
                // depth buffer
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(depth_buffer),
                },
                // uniform buffer
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: state.deferred_pipeline().uniform_buffer(),
                        offset: 0,
                        size: None,
                    }),
                },
            ],
        )
    }

    pub fn new(
        state: &GraphicsState,
        device: &RenderDevice,
        diffuse_buffer: &TextureView,
        normal_buffer: &TextureView,
        depth_buffer: &TextureView,
    ) -> DeferredRenderer {
        let bind_group =
            Self::create_bind_group(state, device, diffuse_buffer, normal_buffer, depth_buffer);

        DeferredRenderer { bind_group }
    }

    pub fn rebuild(
        &mut self,
        state: &GraphicsState,
        device: &RenderDevice,
        diffuse_buffer: &TextureView,
        normal_buffer: &TextureView,
        depth_buffer: &TextureView,
    ) {
        self.bind_group =
            Self::create_bind_group(state, device, diffuse_buffer, normal_buffer, depth_buffer);
    }

    pub fn update_uniform_buffers(
        &self,
        state: &GraphicsState,
        queue: &RenderQueue,
        uniforms: DeferredUniforms,
    ) {
        // update color shift
        queue.write_buffer(state.deferred_pipeline().uniform_buffer(), 0, unsafe {
            any_as_bytes(&uniforms)
        });
    }

    pub fn record_draw<'this, 'a>(
        &'this self,
        state: &'this GraphicsState,
        queue: &'a RenderQueue,
        pass: &'a mut wgpu::RenderPass<'this>,
        uniforms: DeferredUniforms,
    ) {
        self.update_uniform_buffers(state, queue, uniforms);
        pass.set_pipeline(state.deferred_pipeline().pipeline());
        pass.set_vertex_buffer(0, *state.quad_pipeline().vertex_buffer().slice(..));
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct DeferredPassLabel;

#[derive(Default)]
pub struct DeferredPass;

impl ViewNode for DeferredPass {
    type ViewQuery = (&'static ViewTarget, &'static ViewPrepassTextures, &'static Camera3d);

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (target, prepass, _): (&ViewTarget, &ViewPrepassTextures, &Camera3d),
        world: &'w bevy::prelude::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        thread_local! {
            static BUMP: RefCell<Bump> =Bump::new().into();
        }

        let gfx_state = world.resource::<GraphicsState>();
        let renderer = world.get_resource::<WorldRenderer>();
        let conn = world.get_resource::<RenderState>();
        let queue = world.resource::<RenderQueue>();
        let device = world.resource::<RenderDevice>();
        let console_out = world.get_resource::<ConsoleOutput>();
        let console_in = world.get_resource::<ConsoleInput>();
        let Some(&RenderResolution(width, height)) = world.get_resource::<RenderResolution>()
        else {
            return Ok(());
        };
        let menu = world.get_resource::<Menu>();
        let focus = world.resource::<InputFocus>();
        let render_vars = world.resource::<RenderVars>();

        let PostProcessWrite {
            source: diffuse_input,
            destination: diffuse_target,
        } = target.post_process_write();
        let ViewPrepassTextures {
            normal:
                Some(ColorAttachment {
                    texture:
                        CachedTexture {
                            default_view: normal_input,
                            ..
                        },
                    ..
                }),
            depth:
                Some(ColorAttachment {
                    texture:
                        CachedTexture {
                            default_view: depth_input,
                            ..
                        },
                    ..
                }),
            ..
        } = prepass
        else {
            return Ok(());
        };

        // TODO: Cache
        let deferred_renderer =
            DeferredRenderer::new(gfx_state, device, diffuse_input, normal_input, depth_input);

        let encoder = render_context.command_encoder();

        BUMP.with_borrow_mut(|bump| bump.reset());
        BUMP.with_borrow(|bump| {
            if let (
                Some(RenderState {
                    state: cl_state,
                    kind,
                }),
                Some(world),
            ) = (conn, renderer)
            {
                // if client is fully connected, draw world
                let camera = match kind {
                    RenderConnectionKind::Demo => {
                        cl_state.demo_camera(width as f32 / height as f32, Deg(render_vars.fov))
                    }
                    RenderConnectionKind::Server => {
                        cl_state.camera(width as f32 / height as f32, Deg(render_vars.fov))
                    }
                };

                let mut deferred_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Deferred pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: diffuse_target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    ..default()
                });

                let mut lights = [PointLight {
                    origin: Vector3::zero(),
                    radius: 0.0,
                }; MAX_LIGHTS];

                let mut light_count = 0;
                for (light_id, light) in cl_state.iter_lights().enumerate() {
                    light_count += 1;
                    let light_origin = light.origin();
                    let converted_origin =
                        Vector3::new(-light_origin.y, light_origin.z, -light_origin.x);
                    lights[light_id].origin =
                        (camera.view() * converted_origin.extend(1.0)).truncate();
                    lights[light_id].radius = light.radius(cl_state.time());
                }

                let uniforms = DeferredUniforms {
                    inv_projection: camera.inverse_projection().into(),
                    light_count,
                    _pad: [0; 3],
                    lights,
                };

                deferred_renderer.record_draw(gfx_state, queue, &mut deferred_pass, uniforms);
            }
        });

        Ok(())
    }
}
