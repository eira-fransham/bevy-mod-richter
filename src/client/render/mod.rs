// Copyright © 2020 Cormac O'Brien.
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
// SOFTWARE.

/// Rendering functionality.
///
/// # Pipeline stages
///
/// The current rendering implementation consists of the following stages:
/// - Initial geometry pass
///   - Inputs:
///     - `AliasPipeline`
///     - `BrushPipeline`
///     - `SpritePipeline`
///   - Output: `InitialPassTarget`
/// - Deferred lighting pass
///   - Inputs:
///     - `DeferredPipeline`
///     - `QuadPipeline`
///     - `GlyphPipeline`
///   - Output: `DeferredPassTarget`
/// - Final pass
///   - Inputs:
///     - `PostProcessPipeline`
///   - Output: `FinalPassTarget`
/// - Blit to swap chain
///   - Inputs:
///     - `BlitPipeline`
///   - Output: `SwapChainTarget`
mod atlas;
mod blit;
mod cvars;
mod error;
mod palette;
mod pipeline;
mod target;
mod ui;
mod uniform;
mod warp;
mod world;

use bevy::{
    app::Plugin,
    core_pipeline::core_3d::graph::{Core3d, Node3d},
    ecs::{
        event::EventReader,
        system::{In, Res, ResMut, Resource},
        world::FromWorld,
    },
    prelude::*,
    render::{
        extract_resource::{ExtractResource as _, ExtractResourcePlugin},
        render_graph::{RenderGraphApp as _, RenderLabel, ViewNode, ViewNodeRunner},
        render_resource::{BindGroup, BindGroupLayout, Buffer, Sampler, Texture, TextureView},
        renderer::{RenderDevice, RenderQueue},
        view::ViewTarget,
        ExtractSchedule, MainWorld, Render, RenderApp,
    },
};
pub use cvars::register_cvars;
pub use error::{RenderError, RenderErrorKind};
pub use palette::Palette;
use parking_lot::RwLock;
pub use pipeline::Pipeline;
pub use postprocess::PostProcessRenderer;
pub use target::{PreferredFormat, RenderTarget, RenderTargetResolve, SwapChainTarget};
pub use ui::{hud::HudState, UiOverlay, UiRenderer, UiState};
pub use world::{
    deferred::{DeferredRenderer, DeferredUniforms, PointLight},
    Camera, WorldRenderer,
};

use std::{
    borrow::Cow,
    cell::RefCell,
    io::Read as _,
    mem::size_of,
    num::NonZeroU64,
    ops::{Deref, DerefMut},
};

use crate::{
    client::{
        entity::MAX_LIGHTS,
        input::InputFocus,
        menu::Menu,
        render::{
            target::{DeferredPassTarget, FinalPassTarget, InitialPassTarget},
            ui::{glyph::GlyphPipeline, quad::QuadPipeline},
            uniform::DynamicUniformBuffer,
            world::{
                alias::AliasPipeline,
                brush::BrushPipeline,
                deferred::DeferredPipeline,
                particle::ParticlePipeline,
                postprocess::{self, PostProcessPipeline},
                sprite::SpritePipeline,
                EntityUniforms,
            },
        },
        RenderConnectionKind,
    },
    common::{
        console::{CmdRegistry, Console, CvarRegistry, ExecResult},
        vfs::Vfs,
        wad::Wad,
    },
};

use self::blit::BlitPipeline;

use super::{
    extract_resolution, init_client,
    input::Input,
    sound::{MixerEvent, MusicPlayer},
    ConnectionState, DemoQueue, GameConnection, RenderResolution, RenderState, RenderStateRes,
    TimeHack,
};
use bumpalo::Bump;
use cgmath::{Deg, Vector3, Zero};
use chrono::{DateTime, Duration, Utc};
use failure::Error;

pub struct RichterRenderPlugin;

fn send_mixer_events_to_main_thread_hack(
    mut main_world: ResMut<MainWorld>,
    mut events: EventReader<MixerEvent>,
) {
    for e in events.read() {
        main_world.send_event(e.clone());
    }
}

impl Plugin for RichterRenderPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app //.add_systems(Startup, register_cvars.pipe(cvar_error_handler))
            .init_resource::<RenderResolution>()
            .add_plugins((
                // ExtractResourcePlugin::<Vfs>::default(),
                // ExtractResourcePlugin::<CvarRegistry>::default(),
                // ExtractResourcePlugin::<Console>::default(),
                // ExtractResourcePlugin::<Menu>::default(),
                // ExtractResourcePlugin::<RenderStateRes>::default(),
                // ExtractResourcePlugin::<InputFocus>::default(),
                // ExtractResourcePlugin::<Fov>::default(),
                // TODO: This is only so we can run the per-frame update in the render thread, which we should not do
                ExtractResourcePlugin::<TimeHack>::default(),
                // ExtractResourcePlugin::<DemoQueue>::default(),
                // ExtractResourcePlugin::<GameConnection>::default(),
                // ExtractResourcePlugin::<CmdRegistry>::default(),
                // ExtractResourcePlugin::<AudioOut>::default(),
            ))
            .add_systems(ExtractSchedule, extract_resolution);

        let vfs = app.world.resource::<Vfs>().clone();
        let mut cvars = app.world.resource_mut::<CvarRegistry>();
        super::register_cvars(&mut *cvars).unwrap();
        register_cvars(&mut *cvars).unwrap();
        let cvars = cvars.clone();

        let mut console = app.world.resource_mut::<Console>();
        console.append_text("exec quake.rc\n");
        let console = console.clone();
        let menu = app.world.resource::<Menu>().clone();
        let res = *app.world.resource::<RenderResolution>();
        // TODO: This is only so we can run the per-frame update in the render thread, which we should not do
        let mut cmds = app.world.resource_mut::<CmdRegistry>();
        init_client(&mut *cmds);
        cmds.insert_or_replace("exec", move |args, world| {
            let vfs = world.resource::<Vfs>();
            match args.len() {
                // exec (filename): execute a script file
                1 => {
                    let mut script_file = match vfs.open(args[0]) {
                        Ok(s) => s,
                        Err(e) => {
                            return ExecResult {
                                extra_commands: String::new(),
                                output: format!("Couldn't exec {}: {:?}", args[0], e),
                            };
                        }
                    };

                    let mut script = String::new();
                    script_file.read_to_string(&mut script).unwrap();

                    ExecResult {
                        extra_commands: script,
                        output: String::new(),
                    }
                }

                _ => ExecResult {
                    extra_commands: String::new(),
                    output: format!("exec (filename): execute a script file"),
                },
            }
        })
        .unwrap();
        let cmd = cmds.clone();
        // let conn = app.world.resource::<GameConnection>().clone();
        let demo_queue = app.world.resource::<DemoQueue>().clone();
        let input = app.world.resource::<Input>().clone();

        let Ok(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        // TODO: Is there a cleaner way to do this?
        render_app.insert_resource(vfs);
        render_app.insert_resource(cvars);
        render_app.insert_resource(console);
        render_app.insert_resource(res);
        render_app.insert_resource(menu);

        // TODO: This is only so we can run the per-frame update in the render thread, which we should not do
        render_app.insert_resource(cmd);
        // render_app.insert_resource(conn);
        render_app.init_resource::<GameConnection>();
        render_app.insert_resource(demo_queue);
        render_app.insert_resource(input);

        render_app
            // .init_resource::<RenderStateRes>()
            .add_render_graph_node::<ViewNodeRunner<ClientRenderer>>(
                // Specify the label of the graph, in this case we want the graph for 3d
                Core3d,
                // It also needs the label of the node
                ClientRenderLabel,
            )
            .add_render_graph_edges(
                Core3d,
                // Specify the node ordering.
                // This will automatically create all required node edges to enforce the given ordering.
                (
                    Node3d::MainOpaquePass,
                    ClientRenderLabel,
                    Node3d::EndMainPass,
                ),
            );

        // TODO: This is only so we can run the per-frame update in the render thread, which we should not do
        render_app
            .init_resource::<TimeHack>()
            .init_resource::<MusicPlayer>()
            // .add_systems(
            //     Render,
            //     (
            //         super::frame.pipe(|In(res)| {
            //             // TODO: Error handling
            //             if let Err(e) = res {
            //                 warn!("{}", e);
            //             }
            //         }),
            //         super::handle_input.pipe(|In(res)| {
            //             // TODO: Error handling
            //             if let Err(e) = res {
            //                 warn!("{}", e);
            //             }
            //         }),
            //         super::run_console,
            //     ),
            // )
            .add_systems(
                Render,
                super::frame.pipe(|In(res)| {
                    // TODO: Error handling
                    if let Err(e) = res {
                        warn!("{}", e);
                    }
                }),
            )
            .add_systems(
                Render,
                super::handle_input.pipe(|In(res)| {
                    // TODO: Error handling
                    if let Err(e) = res {
                        warn!("{}", e);
                    }
                }),
            )
            .add_systems(Render, super::run_console)
            .add_event::<MixerEvent>()
            .add_systems(ExtractSchedule, send_mixer_events_to_main_thread_hack);
    }

    fn finish(&self, app: &mut bevy::prelude::App) {
        let Ok(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };

        render_app
            .init_resource::<GraphicsState>()
            .init_resource::<PostProcessRenderer>()
            .init_resource::<DeferredRenderer>()
            .init_resource::<UiRenderer>();
    }
}

const DEPTH_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
pub const DIFFUSE_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
pub const FINAL_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const NORMAL_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const LIGHT_ATTACHMENT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

const DIFFUSE_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const FULLBRIGHT_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const LIGHTMAP_TEXTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Create a `wgpu::TextureDescriptor` appropriate for the provided texture data.
pub fn texture_descriptor<'a>(
    label: Option<&'a str>,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
) -> wgpu::TextureDescriptor {
    wgpu::TextureDescriptor {
        label,
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: Default::default(),
    }
}

pub fn create_texture<'a>(
    device: &RenderDevice,
    queue: &RenderQueue,
    label: Option<&'a str>,
    width: u32,
    height: u32,
    data: &TextureData,
) -> Texture {
    trace!(
        "Creating texture ({:?}: {}x{})",
        data.format(),
        width,
        height
    );

    // It looks like sometimes quake includes textures with at least one zero aspect?
    let texture = device.create_texture(&texture_descriptor(
        label,
        width.max(1),
        height.max(1),
        data.format(),
    ));
    queue.write_texture(
        wgpu::ImageCopyTexture {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: Default::default(),
        },
        data.data(),
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(width * data.stride()),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    texture
}

pub struct DiffuseData<'a> {
    pub rgba: Cow<'a, [u8]>,
}

pub struct FullbrightData<'a> {
    pub fullbright: Cow<'a, [u8]>,
}

pub struct LightmapData<'a> {
    pub lightmap: Cow<'a, [u8]>,
}

pub enum TextureData<'a> {
    Diffuse(DiffuseData<'a>),
    Fullbright(FullbrightData<'a>),
    Lightmap(LightmapData<'a>),
}

impl<'a> TextureData<'a> {
    pub fn format(&self) -> wgpu::TextureFormat {
        match self {
            TextureData::Diffuse(_) => DIFFUSE_TEXTURE_FORMAT,
            TextureData::Fullbright(_) => FULLBRIGHT_TEXTURE_FORMAT,
            TextureData::Lightmap(_) => LIGHTMAP_TEXTURE_FORMAT,
        }
    }

    pub fn data(&self) -> &[u8] {
        match self {
            TextureData::Diffuse(d) => &d.rgba,
            TextureData::Fullbright(d) => &d.fullbright,
            TextureData::Lightmap(d) => &d.lightmap,
        }
    }

    pub fn stride(&self) -> u32 {
        use std::mem;
        use wgpu::TextureFormat::*;

        (match self.format() {
            Rg8Unorm | Rg8Snorm | Rg8Uint | Rg8Sint => mem::size_of::<[u8; 2]>(),
            R8Unorm | R8Snorm | R8Uint | R8Sint => mem::size_of::<u8>(),
            Bgra8Unorm | Bgra8UnormSrgb | Rgba8Unorm | Rgba8UnormSrgb => mem::size_of::<[u8; 4]>(),
            R16Uint | R16Sint | R16Unorm | R16Snorm | R16Float => mem::size_of::<u16>(),
            Rg16Uint | Rg16Sint | Rg16Unorm | Rg16Snorm | Rg16Float => mem::size_of::<[u16; 2]>(),
            Rgba16Uint | Rgba16Sint | Rgba16Unorm | Rgba16Snorm | Rgba16Float => {
                mem::size_of::<[u16; 4]>()
            }
            _ => todo!(),
        }) as u32
    }

    pub fn size(&self) -> wgpu::BufferAddress {
        self.data().len() as wgpu::BufferAddress
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Extent2d {
    pub width: u32,
    pub height: u32,
}

impl std::convert::Into<wgpu::Extent3d> for Extent2d {
    fn into(self) -> wgpu::Extent3d {
        wgpu::Extent3d {
            width: self.width,
            height: self.height,
            depth_or_array_layers: 1,
        }
    }
}

impl std::convert::From<winit::dpi::PhysicalSize<u32>> for Extent2d {
    fn from(other: winit::dpi::PhysicalSize<u32>) -> Extent2d {
        let winit::dpi::PhysicalSize { width, height } = other;
        Extent2d { width, height }
    }
}

#[derive(Resource)]
pub struct GraphicsState {
    initial_pass_target: InitialPassTarget,
    deferred_pass_target: DeferredPassTarget,
    final_pass_target: FinalPassTarget,

    world_bind_group_layouts: Vec<BindGroupLayout>,
    world_bind_groups: Vec<BindGroup>,

    frame_uniform_buffer: Buffer,

    // TODO: This probably doesn't need to be a rwlock
    entity_uniform_buffer: RwLock<DynamicUniformBuffer<EntityUniforms>>,

    diffuse_sampler: Sampler,
    nearest_sampler: Sampler,
    lightmap_sampler: Sampler,

    sample_count: u32,

    alias_pipeline: AliasPipeline,
    brush_pipeline: BrushPipeline,
    sprite_pipeline: SpritePipeline,
    deferred_pipeline: DeferredPipeline,
    particle_pipeline: ParticlePipeline,
    postprocess_pipeline: PostProcessPipeline,
    glyph_pipeline: GlyphPipeline,
    quad_pipeline: QuadPipeline,
    blit_pipeline: BlitPipeline,

    default_lightmap: Texture,
    default_lightmap_view: TextureView,

    palette: Palette,
    gfx_wad: Wad,
}

impl FromWorld for GraphicsState {
    fn from_world(world: &mut bevy::prelude::World) -> Self {
        let render_device = world.resource::<RenderDevice>();
        let render_queue = world.resource::<RenderQueue>();
        let render_resolution = world.resource::<RenderResolution>();
        let cvars = world.resource::<CvarRegistry>();
        let mut sample_count = cvars.get_value("r_msaa_samples").unwrap_or(2.0) as u32;
        if !&[2, 4].contains(&sample_count) {
            sample_count = 2;
        }
        // TODO: Reimplement MSAA
        sample_count = 1;

        let vfs = world.resource::<Vfs>();

        Self::new(
            render_device,
            render_queue,
            Extent2d {
                width: render_resolution.0,
                height: render_resolution.1,
            },
            sample_count,
            vfs,
        )
        .unwrap()
    }
}

thread_local! {
    static COMPILER: RefCell<shaderc::Compiler> = shaderc::Compiler::new().unwrap().into();
}

impl GraphicsState {
    pub fn new(
        device: &RenderDevice,
        queue: &RenderQueue,
        size: Extent2d,
        sample_count: u32,
        vfs: &Vfs,
    ) -> Result<GraphicsState, Error> {
        let palette = Palette::load(&vfs, "gfx/palette.lmp");
        let gfx_wad = Wad::load(vfs.open("gfx.wad")?).unwrap();

        let initial_pass_target = InitialPassTarget::new(device, size, sample_count);
        let deferred_pass_target = DeferredPassTarget::new(device, size, sample_count);
        let final_pass_target = FinalPassTarget::new(device, size);

        let frame_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniform buffer"),
            size: size_of::<world::FrameUniforms>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entity_uniform_buffer = DynamicUniformBuffer::new(device);

        let diffuse_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            // TODO: these are the OpenGL defaults; see if there's a better choice for us
            lod_max_clamp: 1000.0,
            compare: None,
            ..Default::default()
        });

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            // TODO: these are the OpenGL defaults; see if there's a better choice for us
            lod_max_clamp: 1000.0,
            compare: None,
            ..Default::default()
        });

        let lightmap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: None,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            // TODO: these are the OpenGL defaults; see if there's a better choice for us
            lod_max_clamp: 1000.0,
            compare: None,
            ..Default::default()
        });

        let world_bind_group_layouts: Vec<BindGroupLayout> = world::BIND_GROUP_LAYOUT_DESCRIPTORS
            .iter()
            .map(|desc| device.create_bind_group_layout(None, desc))
            .collect();
        let world_bind_groups = vec![
            device.create_bind_group(
                Some("per-frame bind group"),
                &world_bind_group_layouts[world::BindGroupLayoutId::PerFrame as usize],
                &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &frame_uniform_buffer,
                        offset: 0,
                        size: None,
                    }),
                }],
            ),
            device.create_bind_group(
                Some("brush per-entity bind group"),
                &world_bind_group_layouts[world::BindGroupLayoutId::PerEntity as usize],
                &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &entity_uniform_buffer.buffer(),
                            offset: 0,
                            size: Some(
                                NonZeroU64::new(size_of::<EntityUniforms>() as u64).unwrap(),
                            ),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&diffuse_sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&lightmap_sampler),
                    },
                ],
            ),
        ];

        let (
            alias_pipeline,
            brush_pipeline,
            sprite_pipeline,
            deferred_pipeline,
            particle_pipeline,
            quad_pipeline,
            glyph_pipeline,
            postprocess_pipeline,
            blit_pipeline,
        ) = COMPILER.with_borrow_mut(|compiler| {
            let alias_pipeline =
                AliasPipeline::new(device, compiler, &world_bind_group_layouts, sample_count);
            let brush_pipeline =
                BrushPipeline::new(device, compiler, &world_bind_group_layouts, sample_count);
            let sprite_pipeline =
                SpritePipeline::new(device, compiler, &world_bind_group_layouts, sample_count);
            let deferred_pipeline = DeferredPipeline::new(device, compiler, sample_count);
            let particle_pipeline =
                ParticlePipeline::new(device, &queue, compiler, sample_count, &palette);
            let quad_pipeline =
                QuadPipeline::new(device, compiler, DIFFUSE_ATTACHMENT_FORMAT, sample_count);
            let glyph_pipeline =
                GlyphPipeline::new(device, compiler, DIFFUSE_ATTACHMENT_FORMAT, sample_count);

            let postprocess_pipeline = PostProcessPipeline::new(
                device,
                compiler,
                final_pass_target.format(),
                final_pass_target.sample_count(),
            );

            let blit_pipeline = BlitPipeline::new(
                device,
                compiler,
                final_pass_target.resolve_view(),
                final_pass_target.format(),
            );

            (
                alias_pipeline,
                brush_pipeline,
                sprite_pipeline,
                deferred_pipeline,
                particle_pipeline,
                quad_pipeline,
                glyph_pipeline,
                postprocess_pipeline,
                blit_pipeline,
            )
        });

        let default_lightmap = create_texture(
            device,
            queue,
            None,
            1,
            1,
            &TextureData::Lightmap(LightmapData {
                lightmap: (&[0xFF][..]).into(),
            }),
        );
        let default_lightmap_view = default_lightmap.create_view(&Default::default());

        Ok(GraphicsState {
            initial_pass_target,
            deferred_pass_target,
            final_pass_target,
            frame_uniform_buffer,
            entity_uniform_buffer: entity_uniform_buffer.into(),

            world_bind_group_layouts,
            world_bind_groups,

            sample_count,

            alias_pipeline,
            brush_pipeline,
            sprite_pipeline,
            deferred_pipeline,
            particle_pipeline,
            postprocess_pipeline,
            glyph_pipeline,
            quad_pipeline,
            blit_pipeline,

            diffuse_sampler,
            nearest_sampler,
            lightmap_sampler,

            default_lightmap,
            default_lightmap_view,
            palette,
            gfx_wad,
        })
    }

    pub fn create_texture<'a>(
        &self,
        device: &RenderDevice,
        queue: &RenderQueue,
        label: Option<&'a str>,
        width: u32,
        height: u32,
        data: &TextureData,
    ) -> Texture {
        create_texture(device, queue, label, width, height, data)
    }

    /// Update graphics state with the new framebuffer size and sample count.
    ///
    /// If the framebuffer size has changed, this recreates all render targets with the new size.
    ///
    /// If the framebuffer sample count has changed, this recreates all render targets with the
    /// new sample count and rebuilds the render pipelines to output that number of samples.
    pub fn update(&mut self, device: &RenderDevice, size: Extent2d, sample_count: u32) {
        if self.sample_count != sample_count {
            self.sample_count = sample_count;
            self.recreate_pipelines(device, sample_count);
        }

        if self.initial_pass_target.size() != size
            || self.initial_pass_target.sample_count() != sample_count
        {
            self.initial_pass_target = InitialPassTarget::new(device, size, sample_count);
        }

        if self.deferred_pass_target.size() != size
            || self.deferred_pass_target.sample_count() != sample_count
        {
            self.deferred_pass_target = DeferredPassTarget::new(device, size, sample_count);
        }

        if self.final_pass_target.size() != size
            || self.final_pass_target.sample_count() != sample_count
        {
            self.final_pass_target = FinalPassTarget::new(device, size);

            // TODO: How do we do the final pass?
            // COMPILER.with_borrow_mut(|compiler| {
            //     self.blit_pipeline
            //         .rebuild(device, compiler, self.final_pass_target.resolve_view());
            // });
        }
    }

    /// Rebuild all render pipelines using the new sample count.
    ///
    /// This must be called when the sample count of the render target(s) changes or the program
    /// will panic.
    fn recreate_pipelines(&mut self, device: &RenderDevice, sample_count: u32) {
        COMPILER.with_borrow_mut(|compiler| {
            self.alias_pipeline.rebuild(
                device,
                compiler,
                &self.world_bind_group_layouts,
                sample_count,
            );
            self.brush_pipeline.rebuild(
                device,
                compiler,
                &self.world_bind_group_layouts,
                sample_count,
            );
            self.sprite_pipeline.rebuild(
                device,
                compiler,
                &self.world_bind_group_layouts,
                sample_count,
            );
            self.deferred_pipeline
                .rebuild(device, compiler, sample_count);
            self.postprocess_pipeline
                .rebuild(device, compiler, sample_count);
            self.glyph_pipeline.rebuild(device, compiler, sample_count);
            self.quad_pipeline.rebuild(device, compiler, sample_count);
        });
    }

    pub fn initial_pass_target(&self) -> &InitialPassTarget {
        &self.initial_pass_target
    }

    pub fn deferred_pass_target(&self) -> &DeferredPassTarget {
        &self.deferred_pass_target
    }

    pub fn final_pass_target(&self) -> &FinalPassTarget {
        &self.final_pass_target
    }

    pub fn frame_uniform_buffer(&self) -> &Buffer {
        &self.frame_uniform_buffer
    }

    pub fn entity_uniform_buffer(
        &self,
    ) -> impl Deref<Target = DynamicUniformBuffer<EntityUniforms>> + '_ {
        self.entity_uniform_buffer.read()
    }

    pub fn entity_uniform_buffer_mut(
        &self,
    ) -> impl DerefMut<Target = DynamicUniformBuffer<EntityUniforms>> + '_ {
        self.entity_uniform_buffer.write()
    }

    pub fn diffuse_sampler(&self) -> &Sampler {
        &self.diffuse_sampler
    }

    pub fn nearest_sampler(&self) -> &Sampler {
        &self.nearest_sampler
    }

    pub fn default_lightmap(&self) -> &Texture {
        &self.default_lightmap
    }

    pub fn default_lightmap_view(&self) -> &TextureView {
        &self.default_lightmap_view
    }

    pub fn lightmap_sampler(&self) -> &Sampler {
        &self.lightmap_sampler
    }

    pub fn world_bind_group_layouts(&self) -> &[BindGroupLayout] {
        &self.world_bind_group_layouts
    }

    pub fn world_bind_groups(&self) -> &[BindGroup] {
        &self.world_bind_groups
    }

    // pipelines

    pub fn alias_pipeline(&self) -> &AliasPipeline {
        &self.alias_pipeline
    }

    pub fn brush_pipeline(&self) -> &BrushPipeline {
        &self.brush_pipeline
    }

    pub fn sprite_pipeline(&self) -> &SpritePipeline {
        &self.sprite_pipeline
    }

    pub fn deferred_pipeline(&self) -> &DeferredPipeline {
        &self.deferred_pipeline
    }

    pub fn particle_pipeline(&self) -> &ParticlePipeline {
        &self.particle_pipeline
    }

    pub fn postprocess_pipeline(&self) -> &PostProcessPipeline {
        &self.postprocess_pipeline
    }

    pub fn glyph_pipeline(&self) -> &GlyphPipeline {
        &self.glyph_pipeline
    }

    pub fn quad_pipeline(&self) -> &QuadPipeline {
        &self.quad_pipeline
    }

    pub fn quad_pipeline_mut(&mut self) -> &mut QuadPipeline {
        &mut self.quad_pipeline
    }

    pub fn blit_pipeline(&self) -> &BlitPipeline {
        &self.blit_pipeline
    }

    pub fn palette(&self) -> &Palette {
        &self.palette
    }

    pub fn gfx_wad(&self) -> &Wad {
        &self.gfx_wad
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
struct ClientRenderLabel;

pub struct ClientRenderer {
    start_time: DateTime<Utc>,
}

impl Default for ClientRenderer {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Resource)]
pub struct Fov(pub Deg<f32>);

impl ViewNode for ClientRenderer {
    type ViewQuery = &'static ViewTarget;

    fn run<'w>(
        &self,
        _graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        view_target: &ViewTarget,
        world: &'w bevy::prelude::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let render_state = RenderStateRes::extract_resource(world.resource::<GameConnection>());
        let queue = world.resource::<RenderQueue>();
        let gfx_state = world.resource::<GraphicsState>();
        let deferred_renderer = world.resource::<DeferredRenderer>();
        let postprocess_renderer = world.resource::<PostProcessRenderer>();
        let ui_renderer = world.resource::<UiRenderer>();
        let cvars = world.resource::<CvarRegistry>();
        let console = world.resource::<Console>();
        let resolution = world.resource::<RenderResolution>();
        let menu = world.resource::<Menu>();
        let focus = world.resource::<Input>().focus();
        // let fov = world.resource::<Fov>();
        let fov = Fov::extract_resource(cvars);

        self.render(
            gfx_state,
            render_context.command_encoder(),
            &*queue,
            deferred_renderer,
            postprocess_renderer,
            ui_renderer,
            render_state.0.as_ref(),
            resolution.0,
            resolution.1,
            fov.0,
            cvars,
            console,
            menu,
            focus,
        );

        let attachment = view_target.main_texture_view();

        {
            let swap_chain_target = SwapChainTarget::with_swap_chain_view(attachment);
            let blit_pass_builder = swap_chain_target.render_pass_builder();
            let mut blit_pass = render_context
                .command_encoder()
                .begin_render_pass(&blit_pass_builder.descriptor());
            gfx_state.blit_pipeline().blit(gfx_state, &mut blit_pass);
        }

        Ok(())
    }
}

pub fn update_renderers(
    gfx_state: Res<GraphicsState>,
    device: Res<RenderDevice>,
    mut deferred_renderer: ResMut<DeferredRenderer>,
    mut postprocess_renderer: ResMut<PostProcessRenderer>,
) {
    deferred_renderer.rebuild(
        &*gfx_state,
        &*device,
        gfx_state.initial_pass_target().diffuse_view(),
        gfx_state.initial_pass_target().normal_view(),
        gfx_state.initial_pass_target().light_view(),
        gfx_state.initial_pass_target().depth_view(),
    );
    postprocess_renderer.rebuild(
        &*gfx_state,
        &*device,
        gfx_state.deferred_pass_target().color_view(),
    );
}

impl ClientRenderer {
    pub fn new() -> ClientRenderer {
        ClientRenderer {
            start_time: Utc::now(),
        }
    }

    pub fn elapsed(&self, time: Option<Duration>) -> Duration {
        match time {
            Some(time) => time,
            None => Utc::now().signed_duration_since(self.start_time),
        }
    }

    pub fn render(
        &self,
        gfx_state: &GraphicsState,
        encoder: &mut wgpu::CommandEncoder,
        queue: &RenderQueue,
        deferred_renderer: &DeferredRenderer,
        postprocess_renderer: &PostProcessRenderer,
        ui_renderer: &UiRenderer,
        conn: Option<&RenderState>,
        width: u32,
        height: u32,
        fov: Deg<f32>,
        cvars: &CvarRegistry,
        console: &Console,
        menu: &Menu,
        focus: InputFocus,
    ) {
        thread_local! {
            static BUMP: RefCell<Bump> =Bump::new().into();
        }

        BUMP.with_borrow(|bump| {
            if let Some(RenderState {
                state: ref cl_state,
                conn_state,
                kind,
            }) = conn
            {
                match conn_state {
                    ConnectionState::Connected(ref world) => {
                        // if client is fully connected, draw world
                        let camera = match kind {
                            RenderConnectionKind::Demo => {
                                cl_state.demo_camera(width as f32 / height as f32, fov)
                            }
                            RenderConnectionKind::Server => {
                                cl_state.camera(width as f32 / height as f32, fov)
                            }
                        };

                        // initial render pass
                        {
                            let init_pass_builder =
                                gfx_state.initial_pass_target().render_pass_builder();

                            world.update_uniform_buffers(
                                gfx_state,
                                queue,
                                &camera,
                                cl_state.time(),
                                cl_state.iter_visible_entities(),
                                cl_state.lightstyle_values().unwrap().as_slice(),
                                cvars,
                            );

                            let mut init_pass =
                                encoder.begin_render_pass(&init_pass_builder.descriptor());

                            world.render_pass(
                                gfx_state,
                                &mut init_pass,
                                bump,
                                &camera,
                                cl_state.time(),
                                cl_state.iter_visible_entities(),
                                cl_state.iter_particles(),
                                cl_state.viewmodel_id(),
                            );
                        }

                        // quad_commands must outlive final pass
                        let mut quad_commands = Vec::new();
                        let mut glyph_commands = Vec::new();

                        // deferred lighting pass
                        {
                            let deferred_pass_builder =
                                gfx_state.deferred_pass_target().render_pass_builder();
                            let mut deferred_pass =
                                encoder.begin_render_pass(&deferred_pass_builder.descriptor());

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

                            deferred_renderer.record_draw(
                                gfx_state,
                                queue,
                                &mut deferred_pass,
                                uniforms,
                            );

                            let ui_state = match conn {
                                Some(RenderState {
                                    state: cl_state, ..
                                }) => UiState::InGame {
                                    hud: match cl_state.intermission() {
                                        Some(kind) => HudState::Intermission {
                                            kind,
                                            completion_duration: cl_state
                                                .completion_time()
                                                .unwrap()
                                                - cl_state.start_time(),
                                            stats: cl_state.stats(),
                                            console,
                                        },

                                        None => HudState::InGame {
                                            items: cl_state.items(),
                                            item_pickup_time: cl_state.item_pickup_times(),
                                            stats: cl_state.stats(),
                                            face_anim_time: cl_state.face_anim_time(),
                                            console,
                                        },
                                    },

                                    overlay: match focus {
                                        InputFocus::Game => None,
                                        InputFocus::Console => Some(UiOverlay::Console(console)),
                                        InputFocus::Menu => Some(UiOverlay::Menu(menu)),
                                    },
                                },

                                None => UiState::Title {
                                    overlay: match focus {
                                        InputFocus::Console => UiOverlay::Console(console),
                                        InputFocus::Menu => UiOverlay::Menu(menu),
                                        InputFocus::Game => unreachable!(),
                                    },
                                },
                            };

                            let elapsed = self.elapsed(conn.map(|c| c.state.time));

                            ui_renderer.render_pass(
                                &gfx_state,
                                queue,
                                &mut deferred_pass,
                                Extent2d { width, height },
                                // use client time when in game, renderer time otherwise
                                elapsed,
                                &ui_state,
                                &mut quad_commands,
                                &mut glyph_commands,
                            );
                        }
                    }

                    // if client is still signing on, draw the loading screen
                    ConnectionState::SignOn(_) => {
                        // TODO: loading screen
                    }
                }
            }

            // final render pass: postprocess the world and draw the UI
            {
                let final_pass_builder = gfx_state.final_pass_target().render_pass_builder();
                let mut final_pass = encoder.begin_render_pass(&final_pass_builder.descriptor());

                if let Some(RenderState {
                    state: cl_state,
                    conn_state,
                    ..
                }) = conn
                {
                    // only postprocess if client is in the game
                    if let ConnectionState::Connected(_) = conn_state {
                        // self.postprocess_renderer
                        //     .rebuild(gfx_state, gfx_state.deferred_pass_target().color_view());
                        postprocess_renderer.record_draw(
                            gfx_state,
                            queue,
                            &mut final_pass,
                            cl_state.color_shift(),
                        );
                    }
                }
            }
        });
    }
}
