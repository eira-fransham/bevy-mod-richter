// Copyright © 2018 Cormac O'Brien
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

use std::{
    borrow::BorrowMut,
    cell::RefCell,
    iter,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex, Once},
};

use lazy_static::lazy_static;
use num::integer::Roots;
use video_rs::{Encoder, EncoderSettings, Locator, Time};
use wgpu::TextureFormat;

use crate::{
    capture::{cmd_screenshot, Capture},
    trace::{cmd_trace_begin, cmd_trace_end},
};

use richter::{
    client::{
        input::Input,
        menu::Menu,
        render::{
            Extent2d, GraphicsState, RenderTarget as _, RenderTargetResolve as _, SwapChainTarget,
        },
        trace::TraceFrame,
        Client, ClientError,
    },
    common::console::{CmdRegistry, Console, CvarRegistry},
};

use chrono::{Duration, TimeDelta, Utc};
use failure::Error;
use log::{debug, info};

#[derive(PartialEq, Eq, Default, Debug, Copy, Clone)]
enum ScreenshotTarget {
    #[default]
    Final,
    Game,
}

fn cmd_startvideo(
    target: ScreenshotTarget,
    video_context: Rc<RefCell<Option<VideoState>>>,
) -> impl Fn(&[&str]) -> String {
    static VIDEO_INIT: Once = Once::new();

    move |args| {
        let path = match args.len() {
            // TODO: make default path configurable
            0 => PathBuf::from(format!("richter-{}.mp4", Utc::now().format("%FT%H-%M-%S"))),
            1 => PathBuf::from(args[0]),
            _ => {
                return "Usage: startvideo [PATH]".to_owned();
            }
        };

        VIDEO_INIT.call_once(|| {
            video_rs::init().unwrap();
        });

        video_context.replace(Some(VideoState::Pending(path, target)));

        String::new()
    }
}

fn cmd_stopvideo(video_context: Rc<RefCell<Option<VideoState>>>) -> impl Fn(&[&str]) -> String {
    move |args| {
        if !args.is_empty() {
            return "Usage: endvideo".to_owned();
        }

        video_context.replace(None);

        String::new()
    }
}

struct RecordingState {
    last_frame: TimeDelta,
    last_rendered_frame: TimeDelta,
    last_dt: TimeDelta,
    encoder: Encoder,
    target: ScreenshotTarget,
    size: (usize, usize),
}

impl std::ops::Drop for RecordingState {
    fn drop(&mut self) {}
}

enum VideoState {
    Pending(PathBuf, ScreenshotTarget),
    Recording(RecordingState),
}

pub struct Game {
    cvars: Rc<RefCell<CvarRegistry>>,
    cmds: Rc<RefCell<CmdRegistry>>,
    input: Rc<RefCell<Input>>,
    pub client: Client,

    // if Some(v), trace is in progress
    trace: Rc<RefCell<Option<Vec<TraceFrame>>>>,

    // if Some(path), take a screenshot and save it to path
    // TODO: Move `ScreenshotTarget` to `capture` so that we can only screenshot the game if we want to
    screenshot_path: Rc<RefCell<Option<PathBuf>>>,

    video_context: Rc<RefCell<Option<VideoState>>>,
}

impl Game {
    pub fn new(
        cvars: Rc<RefCell<CvarRegistry>>,
        cmds: Rc<RefCell<CmdRegistry>>,
        input: Rc<RefCell<Input>>,
        client: Client,
    ) -> Result<Game, Error> {
        // set up input commands
        input.borrow().register_cmds(&mut (*cmds).borrow_mut());

        // set up screenshots
        let screenshot_path = Rc::new(RefCell::new(None));
        let video_context = Rc::new(RefCell::new(None));
        (*cmds)
            .borrow_mut()
            .insert("screenshot", cmd_screenshot(screenshot_path.clone()))
            .unwrap();

        // set up frame tracing
        let trace = Rc::new(RefCell::new(None));
        (*cmds)
            .borrow_mut()
            .insert("trace_begin", cmd_trace_begin(trace.clone()))
            .unwrap();
        (*cmds)
            .borrow_mut()
            .insert("trace_end", cmd_trace_end(cvars.clone(), trace.clone()))
            .unwrap();

        (*cmds)
            .borrow_mut()
            .insert(
                "startvideo",
                cmd_startvideo(Default::default(), video_context.clone()),
            )
            .unwrap();
        (*cmds)
            .borrow_mut()
            .insert(
                "startvideogame",
                cmd_startvideo(ScreenshotTarget::Game, video_context.clone()),
            )
            .unwrap();
        (*cmds)
            .borrow_mut()
            .insert("stopvideo", cmd_stopvideo(video_context.clone()))
            .unwrap();

        Ok(Game {
            cvars,
            cmds,
            input,
            client,
            trace,
            screenshot_path,
            video_context,
        })
    }

    // advance the simulation
    pub fn frame(&mut self, gfx_state: &GraphicsState, frame_duration: Duration) {
        use ClientError::*;

        match self.client.frame(frame_duration, gfx_state) {
            Ok(()) => (),
            Err(e) => match e {
                Cvar(_)
                | UnrecognizedProtocol(_)
                | NoSuchClient(_)
                | NoSuchPlayer(_)
                | NoSuchEntity(_)
                | NullEntity
                | EntityExists(_)
                | InvalidViewEntity(_)
                | TooManyStaticEntities
                | NoSuchLightmapAnimation(_)
                | Model(_)
                | Network(_)
                | Sound(_)
                | Vfs(_) => {
                    log::error!("{}", e);
                    self.client.disconnect();
                }

                _ => panic!("{}", e),
            },
        };

        if let Some(ref mut game_input) = (*self.input).borrow_mut().game_input_mut() {
            self.client
                .handle_input(game_input, frame_duration)
                .unwrap();
        }

        // if there's an active trace, record this frame
        if let Some(ref mut trace_frames) = *(*self.trace).borrow_mut() {
            trace_frames.push(
                self.client
                    .trace(&[self.client.view_entity_id().unwrap()])
                    .unwrap(),
            );
        }
    }

    pub fn render(
        &mut self,
        gfx_state: &GraphicsState,
        color_attachment_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        console: &Console,
        menu: &Menu,
    ) {
        info!("Beginning render pass");
        let mut encoder =
            gfx_state
                .device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Main render"),
                });

        // render world, hud, console, menus
        self.client
            .render(
                gfx_state,
                &mut encoder,
                width,
                height,
                menu,
                self.input.borrow().focus(),
                // SwapChainTarget::with_swap_chain_view(color_attachment_view),
                gfx_state.final_pass_target(),
            )
            .unwrap();

        // screenshot setup (TODO: This is pretty complex in order to correctly handle game-only vs final capture)
        let (mut capture, video_capture) = match (
            self.screenshot_path.borrow().as_ref(),
            self.video_context.borrow().as_ref(),
        ) {
            (_, Some(VideoState::Pending(_, ScreenshotTarget::Final)))
            | (
                _,
                Some(VideoState::Recording(RecordingState {
                    target: ScreenshotTarget::Final,
                    ..
                })),
            )
            | (Some(_), None) => {
                let cap = Capture::new(gfx_state.device(), Extent2d { width, height });
                cap.copy_from_texture(
                    &mut encoder,
                    wgpu::ImageCopyTexture {
                        texture: gfx_state.final_pass_target().resolve_attachment(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: Default::default(),
                    },
                );
                (Some(cap), None)
            }
            (None, Some(VideoState::Pending(_, ScreenshotTarget::Game)))
            | (
                None,
                Some(VideoState::Recording(RecordingState {
                    target: ScreenshotTarget::Game,
                    ..
                })),
            ) => {
                let cap = Capture::new(gfx_state.device(), Extent2d { width, height });
                cap.copy_from_texture(
                    &mut encoder,
                    wgpu::ImageCopyTexture {
                        texture: gfx_state.deferred_pass_target().color_attachment(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: Default::default(),
                    },
                );
                (None, Some(cap))
            }
            (Some(_), Some(VideoState::Pending(_, ScreenshotTarget::Game)))
            | (
                Some(_),
                Some(VideoState::Recording(RecordingState {
                    target: ScreenshotTarget::Game,
                    ..
                })),
            ) => {
                let final_cap = Capture::new(gfx_state.device(), Extent2d { width, height });
                final_cap.copy_from_texture(
                    &mut encoder,
                    wgpu::ImageCopyTexture {
                        texture: gfx_state.final_pass_target().resolve_attachment(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: Default::default(),
                    },
                );
                let game_cap = Capture::new(gfx_state.device(), Extent2d { width, height });
                game_cap.copy_from_texture(
                    &mut encoder,
                    wgpu::ImageCopyTexture {
                        texture: gfx_state.deferred_pass_target().color_attachment(),
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: Default::default(),
                    },
                );
                (Some(final_cap), Some(game_cap))
            }
            (None, None) => (None, None),
        };

        // blit to swap chain
        {
            let swap_chain_target = SwapChainTarget::with_swap_chain_view(color_attachment_view);
            let blit_pass_builder = swap_chain_target.render_pass_builder();
            let mut blit_pass = encoder.begin_render_pass(&blit_pass_builder.descriptor());
            gfx_state.blit_pipeline().blit(gfx_state, &mut blit_pass);
        }

        let command_buffer = encoder.finish();
        {
            gfx_state.queue().submit(vec![command_buffer]);
            gfx_state.device().poll(wgpu::Maintain::Wait);
        }

        // write screenshot if requested and clear screenshot path
        self.screenshot_path.replace(None).map(|path| {
            capture
                .as_mut()
                .unwrap()
                .write_to_file(gfx_state.device(), path)
        });

        if let Some(state) = self.video_context.borrow_mut().take() {
            // RGB texture
            const PIXEL_SIZE: usize = 3;

            let mut capture = video_capture.or(capture);

            capture.as_mut().unwrap().read_texture(gfx_state.device());

            let mut state = match state {
                VideoState::Pending(path, target) => RecordingState {
                    encoder: Encoder::new(
                        &path.into(),
                        EncoderSettings::for_h264_yuv420p(width as usize, height as usize, false),
                    )
                    .unwrap(),
                    last_dt: TimeDelta::try_milliseconds(1000 / 60).unwrap(),
                    last_frame: self.client.elapsed(),
                    last_rendered_frame: TimeDelta::zero(),
                    target,
                    size: (width as usize, height as usize),
                },
                VideoState::Recording(state) => state,
            };

            let dt = self.client.elapsed() - state.last_frame;

            let dt = if dt < TimeDelta::zero() {
                state.last_dt
            } else {
                dt
            };

            state.last_frame = self.client.elapsed();
            state.last_dt = dt;

            let time = dt + state.last_rendered_frame;

            state.last_rendered_frame = time;

            let approx_time =
                time.num_seconds() as f64 + (time.subsec_nanos() as f64 / 1_000_000_000f64);

            let data = capture.as_ref().unwrap().data().unwrap();
            let make_pixel: fn([u8; PIXEL_SIZE]) -> [u8; PIXEL_SIZE] =
                match gfx_state.final_pass_target().format() {
                    TextureFormat::Bgra8Unorm => |pixel| {
                        pixel.map(|v| ((v as f64 / u8::MAX as f64).sqrt() * (u8::MAX as f64)) as u8)
                    },
                    TextureFormat::Bgra8UnormSrgb => |pixel| pixel,
                    _ => unimplemented!(),
                };
            let texture = match gfx_state.final_pass_target().format() {
                TextureFormat::Bgra8Unorm | TextureFormat::Bgra8UnormSrgb => {
                    // TODO: This will lead to weird artifacts when resizing the window
                    let data = data
                        .chunks_exact(4)
                        .map(|pixel| {
                            <[_; 3]>::into_iter(make_pixel([pixel[0], pixel[1], pixel[2]]))
                        })
                        .flatten()
                        .chain(iter::repeat(0))
                        .take(state.size.0 * state.size.1 * PIXEL_SIZE)
                        .collect();
                    ndarray::Array3::from_shape_vec((state.size.1, state.size.0, PIXEL_SIZE), data)
                }
                _ => unimplemented!(),
            }
            .expect("TODO: Create texture failed");

            // TODO: Handle failures gracefully
            let _ = state
                .encoder
                .encode(&texture, &Time::from_secs_f64(approx_time));

            *(*self.video_context).borrow_mut() = Some(VideoState::Recording(state));
        }
    }
}

impl std::ops::Drop for Game {
    fn drop(&mut self) {
        let _ = (*self.cmds).borrow_mut().remove("trace_begin");
        let _ = (*self.cmds).borrow_mut().remove("trace_end");
        let _ = (*self.cmds).borrow_mut().remove("screenshot");
        let _ = (*self.cmds).borrow_mut().remove("startvideo");
        let _ = (*self.cmds).borrow_mut().remove("startvideogame");
        let _ = (*self.cmds).borrow_mut().remove("endvideo");

        if let Some(VideoState::Recording(RecordingState { encoder, .. })) =
            (*self.video_context).borrow_mut().as_mut()
        {
            if let Err(e) = encoder.finish() {
                debug!("Failed writing video: {}", e);
            }
        }
    }
}
