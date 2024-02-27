use std::{mem::size_of, num::NonZeroU64};

use bevy::{
    ecs::{system::Resource, world::FromWorld},
    render::{
        render_resource::{BindGroup, BindGroupLayout, Buffer, RenderPipeline},
        renderer::{RenderDevice, RenderQueue},
    },
};
use wgpu::BindGroupLayoutEntry;

use crate::{
    client::render::{pipeline::Pipeline, ui::quad::QuadPipeline, GraphicsState},
    common::util::any_as_bytes,
};

#[repr(C, align(256))]
#[derive(Clone, Copy, Debug)]
pub struct PostProcessUniforms {
    pub color_shift: [f32; 4],
    pub brightness: f32,
    pub inv_gamma: f32,
}

const BRIGHTNESS: f32 = 2.5;
const GAMMA: f32 = 1.4;

pub struct PostProcessPipeline {
    pipeline: RenderPipeline,
    bind_group_layouts: Vec<BindGroupLayout>,
    swapchain_format: wgpu::TextureFormat,
    uniform_buffer: Buffer,
}

impl PostProcessPipeline {
    pub fn new(
        device: &RenderDevice,
        compiler: &mut shaderc::Compiler,
        swapchain_format: wgpu::TextureFormat,
        sample_count: u32,
    ) -> PostProcessPipeline {
        let (pipeline, bind_group_layouts) =
            PostProcessPipeline::create(device, compiler, &[], sample_count, swapchain_format);
        let uniform_buffer = device.create_buffer_with_data(&wgpu::util::BufferInitDescriptor {
            label: None,
            contents: unsafe {
                any_as_bytes(&PostProcessUniforms {
                    color_shift: [0.0; 4],
                    brightness: BRIGHTNESS,
                    inv_gamma: GAMMA.recip(),
                })
            },
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        PostProcessPipeline {
            pipeline,
            swapchain_format,
            bind_group_layouts,
            uniform_buffer,
        }
    }

    pub fn set_format(&mut self, format: wgpu::TextureFormat) {
        self.swapchain_format = format;
    }

    pub fn rebuild(
        &mut self,
        device: &RenderDevice,
        compiler: &mut shaderc::Compiler,
        sample_count: u32,
    ) {
        let layout_refs = self.bind_group_layouts.iter();
        let pipeline = Self::recreate(
            device,
            compiler,
            layout_refs,
            sample_count,
            self.swapchain_format,
        );
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
    // color buffer
    wgpu::BindGroupLayoutEntry {
        binding: 1,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            view_dimension: wgpu::TextureViewDimension::D2,
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            multisampled: false,
        },
        count: None,
    },
    // PostProcessUniforms
    wgpu::BindGroupLayoutEntry {
        binding: 2,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: NonZeroU64::new(size_of::<PostProcessUniforms>() as u64),
        },
        count: None,
    },
];

impl Pipeline for PostProcessPipeline {
    type VertexPushConstants = ();
    type SharedPushConstants = ();
    type FragmentPushConstants = ();

    type Args = wgpu::TextureFormat;

    fn name() -> &'static str {
        "postprocess"
    }

    fn bind_group_layout_descriptors() -> Vec<Vec<BindGroupLayoutEntry>> {
        vec![BIND_GROUP_LAYOUT_ENTRIES.to_owned()]
    }

    fn vertex_shader() -> &'static str {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shaders/postprocess.vert"
        ))
    }

    fn fragment_shader() -> &'static str {
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/shaders/postprocess.frag"
        ))
    }

    fn primitive_state() -> wgpu::PrimitiveState {
        QuadPipeline::primitive_state()
    }

    fn color_target_states_with_args(args: Self::Args) -> Vec<Option<wgpu::ColorTargetState>> {
        QuadPipeline::color_target_states_with_args(args)
    }

    fn depth_stencil_state() -> Option<wgpu::DepthStencilState> {
        None
    }

    fn vertex_buffer_layouts() -> Vec<wgpu::VertexBufferLayout<'static>> {
        QuadPipeline::vertex_buffer_layouts()
    }
}

#[derive(Resource)]
pub struct PostProcessRenderer {
    bind_group: BindGroup,
}

impl FromWorld for PostProcessRenderer {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        let state = world.resource::<GraphicsState>();
        let device = world.resource::<RenderDevice>();

        PostProcessRenderer::new(state, device, state.deferred_pass_target.color_view())
    }
}

impl PostProcessRenderer {
    pub fn create_bind_group(
        state: &GraphicsState,
        device: &RenderDevice,
        color_buffer: &wgpu::TextureView,
    ) -> BindGroup {
        device.create_bind_group(
            Some("postprocess bind group"),
            &state.postprocess_pipeline().bind_group_layouts()[0],
            &[
                // sampler
                wgpu::BindGroupEntry {
                    binding: 0,
                    // TODO: might need a dedicated sampler if downsampling
                    resource: wgpu::BindingResource::Sampler(state.diffuse_sampler()),
                },
                // color buffer
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(color_buffer),
                },
                // uniform buffer
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: state.postprocess_pipeline().uniform_buffer(),
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
        color_buffer: &wgpu::TextureView,
    ) -> PostProcessRenderer {
        let bind_group = Self::create_bind_group(state, device, color_buffer);

        PostProcessRenderer { bind_group }
    }

    pub fn rebuild(
        &mut self,
        state: &GraphicsState,
        device: &RenderDevice,
        color_buffer: &wgpu::TextureView,
    ) {
        self.bind_group = Self::create_bind_group(state, device, color_buffer);
    }

    pub fn update_uniform_buffers(
        &self,
        state: &GraphicsState,
        queue: &RenderQueue,
        color_shift: [f32; 4],
    ) {
        // update color shift
        queue.write_buffer(state.postprocess_pipeline().uniform_buffer(), 0, unsafe {
            any_as_bytes(&PostProcessUniforms {
                color_shift,
                brightness: BRIGHTNESS,
                inv_gamma: GAMMA.recip(),
            })
        });
    }

    pub fn record_draw<'this, 'a>(
        &'this self,
        state: &'this GraphicsState,
        queue: &'a RenderQueue,
        pass: &'a mut wgpu::RenderPass<'this>,
        color_shift: [f32; 4],
    ) {
        self.update_uniform_buffers(state, queue, color_shift);
        pass.set_pipeline(state.postprocess_pipeline().pipeline());
        pass.set_vertex_buffer(0, *state.quad_pipeline().vertex_buffer().slice(..));
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..6, 0..1);
    }
}
