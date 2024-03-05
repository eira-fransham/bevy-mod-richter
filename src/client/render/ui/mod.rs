pub mod console;
pub mod glyph;
pub mod hud;
pub mod layout;
pub mod menu;
pub mod quad;

use crate::{
    client::{
        input::InputFocus,
        menu::Menu,
        render::{
            ui::{
                console::ConsoleRenderer,
                glyph::{GlyphRenderer, GlyphRendererCommand},
                hud::{HudRenderer, HudState},
                menu::MenuRenderer,
                quad::{QuadRenderer, QuadRendererCommand},
            },
            Extent2d, GraphicsState,
        },
    },
    common::{
        console::{RenderConsoleInput, RenderConsoleOutput},
        vfs::Vfs,
    },
};

use bevy::{
    prelude::*,
    render::{
        render_graph::{RenderLabel, ViewNode},
        renderer::{RenderDevice, RenderQueue},
        view::ViewTarget,
    },
};
use cgmath::{Matrix4, Vector2};
use chrono::Duration;

use super::{world::WorldRenderer, RenderResolution, RenderState};

pub fn screen_space_vertex_translate(
    display_w: u32,
    display_h: u32,
    pos_x: i32,
    pos_y: i32,
) -> Vector2<f32> {
    // rescale from [0, DISPLAY_*] to [-1, 1] (NDC)
    Vector2::new(
        (pos_x * 2 - display_w as i32) as f32 / display_w as f32,
        (pos_y * 2 - display_h as i32) as f32 / display_h as f32,
    )
}

pub fn screen_space_vertex_scale(
    display_w: u32,
    display_h: u32,
    quad_w: u32,
    quad_h: u32,
) -> Vector2<f32> {
    Vector2::new(
        (quad_w * 2) as f32 / display_w as f32,
        (quad_h * 2) as f32 / display_h as f32,
    )
}

pub fn screen_space_vertex_transform(
    display_w: u32,
    display_h: u32,
    quad_w: u32,
    quad_h: u32,
    pos_x: i32,
    pos_y: i32,
) -> Matrix4<f32> {
    let Vector2 { x: ndc_x, y: ndc_y } =
        screen_space_vertex_translate(display_w, display_h, pos_x, pos_y);

    let Vector2 {
        x: scale_x,
        y: scale_y,
    } = screen_space_vertex_scale(display_w, display_h, quad_w, quad_h);

    Matrix4::from_translation([ndc_x, ndc_y, 0.0].into())
        * Matrix4::from_nonuniform_scale(scale_x, scale_y, 1.0)
}

pub enum UiOverlay<'a> {
    Menu(&'a Menu),
    Console(&'a RenderConsoleInput, &'a RenderConsoleOutput),
}

pub enum UiState<'a> {
    Title {
        overlay: UiOverlay<'a>,
    },
    InGame {
        hud: HudState<'a>,
        overlay: Option<UiOverlay<'a>>,
    },
}

#[derive(Resource)]
pub struct UiRenderer {
    console_renderer: ConsoleRenderer,
    menu_renderer: MenuRenderer,
    hud_renderer: HudRenderer,
    glyph_renderer: GlyphRenderer,
    quad_renderer: QuadRenderer,
}

impl UiRenderer {
    pub fn new(
        state: &GraphicsState,
        vfs: &Vfs,
        device: &RenderDevice,
        queue: &RenderQueue,
        menu: &Menu,
    ) -> UiRenderer {
        UiRenderer {
            console_renderer: ConsoleRenderer::new(state, vfs, device, queue),
            menu_renderer: MenuRenderer::new(state, vfs, device, queue, menu),
            hud_renderer: HudRenderer::new(state, vfs, device, queue),
            glyph_renderer: GlyphRenderer::new(state, device, queue),
            quad_renderer: QuadRenderer::new(state, device),
        }
    }

    pub fn render_pass<'this, 'a>(
        &'this self,
        state: &'this GraphicsState,
        queue: &'a RenderQueue,
        pass: &'a mut wgpu::RenderPass<'this>,
        target_size: Extent2d,
        time: Duration,
        ui_state: &'a UiState<'this>,
        quad_commands: &'a mut Vec<QuadRendererCommand<'this>>,
        glyph_commands: &'a mut Vec<GlyphRendererCommand>,
    ) {
        let (hud_state, overlay) = match ui_state {
            UiState::Title { overlay } => (None, Some(overlay)),
            UiState::InGame { hud, overlay } => (Some(hud), overlay.as_ref()),
        };

        if let Some(hstate) = hud_state {
            self.hud_renderer
                .generate_commands(hstate, time, quad_commands, glyph_commands);
        }

        if let Some(o) = overlay {
            match o {
                UiOverlay::Menu(menu) => {
                    self.menu_renderer
                        .generate_commands(menu, time, quad_commands, glyph_commands);
                }
                UiOverlay::Console(input, output) => {
                    // TODO: take in-game console proportion as cvar
                    let proportion = match hud_state {
                        Some(_) => 0.33,
                        None => 1.0,
                    };

                    self.console_renderer.generate_commands(
                        output,
                        input,
                        time,
                        quad_commands,
                        glyph_commands,
                        proportion,
                    );
                }
            }
        }

        self.quad_renderer
            .update_uniforms(state, queue, target_size, quad_commands);
        self.quad_renderer
            .record_draw(state, queue, pass, quad_commands);
        self.glyph_renderer
            .record_draw(state, queue, pass, target_size, glyph_commands);
    }
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, RenderLabel)]
pub struct UiPassLabel;

#[derive(Default)]
pub struct UiPass;

impl ViewNode for UiPass {
    type ViewQuery = (&'static ViewTarget, &'static Camera3d);

    fn run<'w>(
        &self,
        graph: &mut bevy::render::render_graph::RenderGraphContext,
        render_context: &mut bevy::render::renderer::RenderContext<'w>,
        (view_target, _): (&ViewTarget, &Camera3d),
        world: &'w World,
    ) -> Result<(), bevy::render::render_graph::NodeRunError> {
        let gfx_state = world.resource::<GraphicsState>();
        let ui_renderer = world.resource::<UiRenderer>();
        let renderer = world.get_resource::<WorldRenderer>();
        let conn = world.get_resource::<RenderState>();
        let queue = world.resource::<RenderQueue>();
        let device = world.resource::<RenderDevice>();
        let Some(&RenderResolution(width, height)) = world.get_resource::<RenderResolution>()
        else {
            return Ok(());
        };
        let console_out = world.get_resource::<RenderConsoleOutput>();
        let console_in = world.get_resource::<RenderConsoleInput>();
        let menu = world.get_resource::<Menu>();
        let focus = world.resource::<InputFocus>();

        // quad_commands must outlive final pass
        let mut quad_commands = Vec::new();
        let mut glyph_commands = Vec::new();

        let encoder = render_context.command_encoder();
        let diffuse_target = view_target.get_unsampled_color_attachment();

        // final render pass: postprocess the world and draw the UI
        {
            let mut final_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Ui pass"),
                color_attachments: &[Some(diffuse_target)],
                depth_stencil_attachment: None,
                ..default()
            });

            if let Some(RenderState {
                state: cl_state, ..
            }) = conn
            {
                let ui_state = match conn {
                    Some(RenderState {
                        state: cl_state, ..
                    }) => UiState::InGame {
                        hud: match cl_state.intermission() {
                            Some(kind) => HudState::Intermission {
                                kind,
                                completion_duration: cl_state.completion_time().unwrap()
                                    - cl_state.start_time(),
                                stats: cl_state.stats(),
                                console: console_out,
                            },

                            None => HudState::InGame {
                                items: cl_state.items(),
                                item_pickup_time: cl_state.item_pickup_times(),
                                stats: cl_state.stats(),
                                face_anim_time: cl_state.face_anim_time(),
                                console: console_out,
                            },
                        },

                        overlay: match (focus, console_in, console_out, menu) {
                            (InputFocus::Game, _, _, _) => None,
                            (InputFocus::Console, Some(input), Some(output), _) => {
                                Some(UiOverlay::Console(input, output))
                            }
                            (InputFocus::Menu, _, _, Some(menu)) => Some(UiOverlay::Menu(menu)),
                            _ => None,
                        },
                    },

                    None => UiState::Title {
                        overlay: match (focus, console_in, console_out, menu) {
                            (InputFocus::Console, Some(input), Some(output), _) => {
                                UiOverlay::Console(input, output)
                            }
                            (InputFocus::Menu, _, _, Some(menu)) => UiOverlay::Menu(menu),
                            (InputFocus::Game, _, _, _) => unreachable!(),
                            _ => return Ok(()),
                        },
                    },
                };

                let elapsed = conn.as_ref().map(|c| c.state.time).unwrap_or_default();
                ui_renderer.render_pass(
                    &*gfx_state,
                    queue,
                    &mut final_pass,
                    Extent2d { width, height },
                    // use client time when in game, renderer time otherwise
                    elapsed,
                    &ui_state,
                    &mut quad_commands,
                    &mut glyph_commands,
                );
            }

            Ok(())
        }
    }
}
