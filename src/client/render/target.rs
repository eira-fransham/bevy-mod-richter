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

use std::cell::RefCell;

use bevy::render::{
    extract_resource::ExtractResource as _,
    render_graph::{Node, RenderLabel, SlotInfo, SlotLabel, SlotType},
    render_resource::{Texture, TextureView},
    renderer::{RenderDevice, RenderQueue},
};
use bumpalo::Bump;

use crate::{
    client::{
        input::Input,
        menu::Menu,
        render::{
            DeferredRenderer, Extent2d, Fov, GraphicsState, PostProcessRenderer, UiRenderer,
            DEPTH_ATTACHMENT_FORMAT, DIFFUSE_ATTACHMENT_FORMAT, FINAL_ATTACHMENT_FORMAT,
            LIGHT_ATTACHMENT_FORMAT, NORMAL_ATTACHMENT_FORMAT,
        },
        ConnectionState, GameConnection, RenderConnectionKind, RenderResolution, RenderState,
        RenderStateRes,
    },
    common::console::{Console, CvarRegistry},
};

// TODO: collapse these into a single definition
/// Create a texture suitable for use as a color attachment.
///
/// The resulting texture will have the RENDER_ATTACHMENT flag as well as
/// any flags specified by `usage`.
pub fn create_color_attachment(
    device: &RenderDevice,
    size: Extent2d,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("color attachment"),
        size: size.into(),
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: DIFFUSE_ATTACHMENT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
        view_formats: Default::default(),
    })
}

/// Create a texture suitable for use as a normal attachment.
///
/// The resulting texture will have the RENDER_ATTACHMENT flag as well as
/// any flags specified by `usage`.
pub fn create_normal_attachment(
    device: &RenderDevice,
    size: Extent2d,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("normal attachment"),
        size: size.into(),
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: NORMAL_ATTACHMENT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
        view_formats: Default::default(),
    })
}

/// Create a texture suitable for use as a light attachment.
///
/// The resulting texture will have the RENDER_ATTACHMENT flag as well as
/// any flags specified by `usage`.
pub fn create_light_attachment(
    device: &RenderDevice,
    size: Extent2d,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("light attachment"),
        size: size.into(),
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: LIGHT_ATTACHMENT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
        view_formats: Default::default(),
    })
}

/// Create a texture suitable for use as a depth attachment.
///
/// The underlying texture will have the RENDER_ATTACHMENT flag as well as
/// any flags specified by `usage`.
pub fn create_depth_attachment(
    device: &RenderDevice,
    size: Extent2d,
    sample_count: u32,
    usage: wgpu::TextureUsages,
) -> Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth attachment"),
        size: size.into(),
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format: DEPTH_ATTACHMENT_FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | usage,
        view_formats: Default::default(),
    })
}

/// Intermediate object that can generate `RenderPassDescriptor`s.
pub struct RenderPassBuilder<'a> {
    color_attachments: Vec<Option<wgpu::RenderPassColorAttachment<'a>>>,
    depth_attachment: Option<wgpu::RenderPassDepthStencilAttachment<'a>>,
}

impl<'a> RenderPassBuilder<'a> {
    pub fn descriptor(&self) -> wgpu::RenderPassDescriptor {
        wgpu::RenderPassDescriptor {
            label: None,
            color_attachments: &self.color_attachments,
            depth_stencil_attachment: self.depth_attachment.clone(),
            timestamp_writes: Default::default(),
            occlusion_query_set: Default::default(),
        }
    }
}

/// A trait describing a render target.
///
/// A render target consists of a series of color attachments and an optional depth-stencil
/// attachment.
pub trait RenderTarget {
    fn render_pass_builder(&self) -> RenderPassBuilder<'_>;
}

impl<T> RenderTarget for &'_ T
where
    T: RenderTarget,
{
    fn render_pass_builder(&self) -> RenderPassBuilder<'_> {
        (**self).render_pass_builder()
    }
}

pub trait PreferredFormat {
    fn preferred_format(&self) -> wgpu::TextureFormat;
}

/// A trait describing a render target with a built-in resolve attachment.
pub trait RenderTargetResolve: RenderTarget {
    fn resolve_attachment(&self) -> &Texture;
    fn resolve_view(&self) -> &TextureView;
}

// TODO: use ArrayVec<TextureView> in concrete types so it can be passed
// as Cow::Borrowed in RenderPassDescriptor

/// Render target for the initial world pass.
pub struct InitialPassTarget {
    size: Extent2d,
    sample_count: u32,
    diffuse_attachment: Texture,
    diffuse_view: TextureView,
    normal_attachment: Texture,
    normal_view: TextureView,
    light_attachment: Texture,
    light_view: TextureView,
    depth_attachment: Texture,
    depth_view: TextureView,
}

impl InitialPassTarget {
    pub fn new(device: &RenderDevice, size: Extent2d, sample_count: u32) -> InitialPassTarget {
        let diffuse_attachment = create_color_attachment(
            device,
            size,
            sample_count,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let normal_attachment = create_normal_attachment(
            device,
            size,
            sample_count,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let light_attachment = create_light_attachment(
            device,
            size,
            sample_count,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let depth_attachment = create_depth_attachment(
            device,
            size,
            sample_count,
            wgpu::TextureUsages::TEXTURE_BINDING,
        );

        let diffuse_view = diffuse_attachment.create_view(&Default::default());
        let normal_view = normal_attachment.create_view(&Default::default());
        let light_view = light_attachment.create_view(&Default::default());
        let depth_view = depth_attachment.create_view(&Default::default());

        InitialPassTarget {
            size,
            sample_count,
            diffuse_attachment,
            diffuse_view,
            normal_attachment,
            normal_view,
            light_attachment,
            light_view,
            depth_attachment,
            depth_view,
        }
    }

    pub fn size(&self) -> Extent2d {
        self.size
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn diffuse_attachment(&self) -> &Texture {
        &self.diffuse_attachment
    }

    pub fn diffuse_view(&self) -> &TextureView {
        &self.diffuse_view
    }

    pub fn normal_attachment(&self) -> &Texture {
        &self.normal_attachment
    }

    pub fn normal_view(&self) -> &TextureView {
        &self.normal_view
    }

    pub fn light_attachment(&self) -> &Texture {
        &self.light_attachment
    }

    pub fn light_view(&self) -> &TextureView {
        &self.light_view
    }

    pub fn depth_attachment(&self) -> &Texture {
        &self.depth_attachment
    }

    pub fn depth_view(&self) -> &TextureView {
        &self.depth_view
    }
}

impl RenderTarget for InitialPassTarget {
    fn render_pass_builder<'a>(&'a self) -> RenderPassBuilder {
        RenderPassBuilder {
            color_attachments: vec![
                Some(wgpu::RenderPassColorAttachment {
                    view: self.diffuse_view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: self.normal_view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: self.light_view(),
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: self.depth_view(),
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
        }
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct InitPassLabel;

#[derive(Default)]
pub struct InitPass;

const INIT_DIFFUSE: &str = "init_diffuse";
const INIT_NORMAL: &str = "init_normal";
const INIT_LIGHT: &str = "init_light";
const INIT_DEPTH: &str = "init_depth";

pub enum InitPassOutput {
    Diffuse,
    Normal,
    Light,
    Depth,
}

impl From<InitPassOutput> for SlotLabel {
    fn from(value: InitPassOutput) -> Self {
        match value {
            InitPassOutput::Diffuse => INIT_DIFFUSE.into(),
            InitPassOutput::Normal => INIT_NORMAL.into(),
            InitPassOutput::Light => INIT_LIGHT.into(),
            InitPassOutput::Depth => INIT_DEPTH.into(),
        }
    }
}

impl Node for InitPass {
    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        world: &'w bevy::prelude::World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let RenderStateRes(conn) =
            RenderStateRes::extract_resource(world.resource::<GameConnection>());
        let queue = world.resource::<RenderQueue>();
        let gfx_state = world.resource::<GraphicsState>();
        let deferred_renderer = world.resource::<DeferredRenderer>();
        let postprocess_renderer = world.resource::<PostProcessRenderer>();
        let ui_renderer = world.resource::<UiRenderer>();
        let cvars = world.resource::<CvarRegistry>();
        let console = world.resource::<Console>();
        let &RenderResolution(width, height) = world.resource::<RenderResolution>();
        let menu = world.resource::<Menu>();
        let focus = world.resource::<Input>().focus();
        // let fov = world.resource::<Fov>();
        let fov = Fov::extract_resource(cvars).0;

        // TODO: Remove this
        thread_local! {
            static BUMP: RefCell<Bump> =Bump::new().into();
        }

        let encoder = render_context.command_encoder();

        BUMP.with_borrow_mut(|bump| bump.reset());
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
                    }

                    // if client is still signing on, draw the loading screen
                    ConnectionState::SignOn(_) => {
                        // TODO: loading screen
                    }
                }
            }
        });

        let target = gfx_state.initial_pass_target();

        graph.set_output(InitPassOutput::Diffuse, target.diffuse_view().clone())?;
        graph.set_output(InitPassOutput::Normal, target.normal_view().clone())?;
        graph.set_output(InitPassOutput::Light, target.light_view().clone())?;
        graph.set_output(InitPassOutput::Depth, target.depth_view().clone())?;

        Ok(())
    }

    fn output(&self) -> Vec<SlotInfo> {
        vec![
            SlotInfo {
                name: INIT_DIFFUSE.into(),
                slot_type: SlotType::TextureView,
            },
            SlotInfo {
                name: INIT_NORMAL.into(),
                slot_type: SlotType::TextureView,
            },
            SlotInfo {
                name: INIT_LIGHT.into(),
                slot_type: SlotType::TextureView,
            },
            SlotInfo {
                name: INIT_DEPTH.into(),
                slot_type: SlotType::TextureView,
            },
        ]
    }
}

pub struct DeferredPassTarget {
    size: Extent2d,
    sample_count: u32,
    color_attachment: Texture,
    color_view: TextureView,
}

impl DeferredPassTarget {
    pub const FORMAT: wgpu::TextureFormat = DIFFUSE_ATTACHMENT_FORMAT;

    pub fn format(&self) -> wgpu::TextureFormat {
        Self::FORMAT
    }

    pub fn new(device: &RenderDevice, size: Extent2d, sample_count: u32) -> DeferredPassTarget {
        let color_attachment = create_color_attachment(
            device,
            size,
            sample_count,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
        );
        let color_view = color_attachment.create_view(&Default::default());

        DeferredPassTarget {
            size,
            sample_count,
            color_attachment,
            color_view,
        }
    }

    pub fn size(&self) -> Extent2d {
        self.size
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    pub fn color_attachment(&self) -> &Texture {
        &self.color_attachment
    }

    pub fn color_view(&self) -> &TextureView {
        &self.color_view
    }
}

impl RenderTarget for DeferredPassTarget {
    fn render_pass_builder<'a>(&'a self) -> RenderPassBuilder {
        RenderPassBuilder {
            color_attachments: vec![Some(wgpu::RenderPassColorAttachment {
                view: self.color_view(),
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_attachment: None,
        }
    }
}

pub struct FinalPassTarget {
    size: Extent2d,
    sample_count: u32,
    resolve_attachment: Texture,
    resolve_view: TextureView,
}

impl FinalPassTarget {
    pub const FORMAT: wgpu::TextureFormat = FINAL_ATTACHMENT_FORMAT;

    pub fn format(&self) -> wgpu::TextureFormat {
        Self::FORMAT
    }

    pub fn new(device: &RenderDevice, size: Extent2d) -> FinalPassTarget {
        let sample_count = 1;

        // add COPY_SRC so we can copy to a buffer for capture and SAMPLED so we
        // can blit to the swap chain
        let resolve_attachment = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("resolve attachment"),
            size: size.into(),
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: Self::FORMAT,
            usage: wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: Default::default(),
        });
        let resolve_view = resolve_attachment.create_view(&Default::default());

        FinalPassTarget {
            size,
            sample_count,
            resolve_attachment,
            resolve_view,
        }
    }

    pub fn size(&self) -> Extent2d {
        self.size
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }
}

impl RenderTarget for FinalPassTarget {
    fn render_pass_builder<'a>(&'a self) -> RenderPassBuilder {
        RenderPassBuilder {
            color_attachments: vec![Some(wgpu::RenderPassColorAttachment {
                view: self.resolve_view(),
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_attachment: None,
        }
    }
}

impl RenderTargetResolve for FinalPassTarget {
    fn resolve_attachment(&self) -> &Texture {
        &self.resolve_attachment
    }

    fn resolve_view(&self) -> &TextureView {
        &self.resolve_view
    }
}

pub struct SwapChainTarget<'a> {
    swap_chain_view: &'a TextureView,
}

impl<'a> SwapChainTarget<'a> {
    pub fn with_swap_chain_view(swap_chain_view: &'a TextureView) -> SwapChainTarget<'a> {
        SwapChainTarget { swap_chain_view }
    }
}

impl<'a> RenderTarget for SwapChainTarget<'a> {
    fn render_pass_builder(&self) -> RenderPassBuilder {
        RenderPassBuilder {
            color_attachments: vec![Some(wgpu::RenderPassColorAttachment {
                view: self.swap_chain_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_attachment: None,
        }
    }
}
