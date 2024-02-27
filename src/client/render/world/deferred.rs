use std::{mem::size_of, num::NonZeroU64};

use bevy::{
    ecs::{
        system::{Deferred, Resource},
        world::FromWorld,
    },
    render::{
        render_resource::{
            BindGroup, BindGroupLayout, BindGroupLayoutEntry, Buffer, RenderPipeline, TextureView,
        },
        renderer::{RenderDevice, RenderQueue},
    },
};
use cgmath::{Matrix4, SquareMatrix as _, Vector3, Zero as _};

use crate::{
    client::{
        entity::MAX_LIGHTS,
        render::{
            pipeline::Pipeline, ui::quad::QuadPipeline, GraphicsState, DIFFUSE_ATTACHMENT_FORMAT,
        },
    },
    common::util::any_as_bytes,
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
        sample_count: u32,
    ) -> DeferredPipeline {
        let (pipeline, bind_group_layouts) =
            DeferredPipeline::create(device, compiler, &[], sample_count, ());

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
        sample_count: u32,
    ) {
        let layout_refs = self.bind_group_layouts.iter();
        let pipeline = Self::recreate(device, compiler, layout_refs, sample_count, ());
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
    // light buffer
    wgpu::BindGroupLayoutEntry {
        binding: 4,
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
        binding: 5,
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
        binding: 6,
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

    type Args = ();

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

    fn color_target_states_with_args(_: Self::Args) -> Vec<Option<wgpu::ColorTargetState>> {
        vec![Some(wgpu::ColorTargetState {
            format: DIFFUSE_ATTACHMENT_FORMAT,
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

impl FromWorld for DeferredRenderer {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        let state = world.resource::<GraphicsState>();
        let device = world.resource::<RenderDevice>();

        DeferredRenderer::new(
            state,
            device,
            state.initial_pass_target.diffuse_view(),
            state.initial_pass_target.normal_view(),
            state.initial_pass_target.light_view(),
            state.initial_pass_target.depth_view(),
        )
    }
}

impl DeferredRenderer {
    fn create_bind_group(
        state: &GraphicsState,
        device: &RenderDevice,
        diffuse_buffer: &TextureView,
        normal_buffer: &TextureView,
        light_buffer: &TextureView,
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
                // light buffer
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(light_buffer),
                },
                // depth buffer
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(depth_buffer),
                },
                // uniform buffer
                wgpu::BindGroupEntry {
                    binding: 6,
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
        light_buffer: &TextureView,
        depth_buffer: &TextureView,
    ) -> DeferredRenderer {
        let bind_group = Self::create_bind_group(
            state,
            device,
            diffuse_buffer,
            normal_buffer,
            light_buffer,
            depth_buffer,
        );

        DeferredRenderer { bind_group }
    }

    pub fn rebuild(
        &mut self,
        state: &GraphicsState,
        device: &RenderDevice,
        diffuse_buffer: &TextureView,
        normal_buffer: &TextureView,
        light_buffer: &TextureView,
        depth_buffer: &TextureView,
    ) {
        self.bind_group = Self::create_bind_group(
            state,
            device,
            diffuse_buffer,
            normal_buffer,
            light_buffer,
            depth_buffer,
        );
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
