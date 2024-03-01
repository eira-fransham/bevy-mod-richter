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

use std::{mem, path::PathBuf, sync::Once};

use bevy::ecs::{
    system::{Res, ResMut},
    world::World,
};
use video_rs::Encoder;

use crate::{
    capture::cmd_screenshot,
    trace::{cmd_trace_begin, cmd_trace_end},
};

use richter::{
    client::{input::Input, render::GraphicsState},
    common::console::CmdRegistry,
};

use chrono::{Duration, TimeDelta, Utc};

fn cmd_startvideo(args: &[&str], world: &mut World) -> String {
    static VIDEO_INIT: Once = Once::new();

    let video_context: &mut Option<VideoState> = todo!();

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

    if let Some(VideoState::Recording(RecordingState { mut encoder, .. })) =
        mem::replace(video_context, Some(VideoState::Pending(path)))
    {
        if let Err(e) = encoder.finish() {
            return format!("Failed writing video: {}", e);
        }
    }

    String::new()
}

fn cmd_stopvideo(args: &[&str], world: &mut World) -> String {
    let video_context: &mut Option<VideoState> = todo!();

    if !args.is_empty() {
        return "Usage: endvideo".to_owned();
    }

    if let Some(VideoState::Recording(RecordingState { mut encoder, .. })) =
        mem::take(video_context)
    {
        if let Err(e) = encoder.finish() {
            return format!("Failed writing video: {}", e);
        }
    }

    String::new()
}

struct RecordingState {
    last_frame: TimeDelta,
    last_rendered_frame: TimeDelta,
    last_dt: TimeDelta,
    encoder: Encoder,
    size: (usize, usize),
}

enum VideoState {
    Pending(PathBuf),
    Recording(RecordingState),
}

pub fn setup(mut cmds: ResMut<CmdRegistry>, input: Res<Input>) {
    // set up screenshots
    cmds.insert("screenshot", cmd_screenshot).unwrap();

    // set up frame tracing
    // let trace = Rc::new(RefCell::new(None));
    cmds.insert("trace_begin", cmd_trace_begin).unwrap();
    cmds.insert("trace_end", cmd_trace_end).unwrap();

    cmds.insert("startvideo", cmd_startvideo).unwrap();
    cmds.insert("stopvideo", cmd_stopvideo).unwrap();

    input.register_cmds(&mut *cmds);
}

// impl Game {
//     pub fn new(world: &mut World) -> Result<Game, Error> {
//         Ok(Game {
//             // client,
//             // trace,
//             // screenshot_path,
//             // video_context,
//         })
//     }

// advance the simulation
pub fn frame(input: ResMut<Input>, frame_duration: Duration) {
    // let trace_frames: &mut Option<Vec<TraceFrame>> = todo!();

    // match client.frame(frame_duration, gfx_state) {
    //     Ok(()) => (),
    //     Err(e) => match e {
    //         Cvar(_)
    //         | UnrecognizedProtocol(_)
    //         | NoSuchClient(_)
    //         | NoSuchPlayer(_)
    //         | NoSuchEntity(_)
    //         | NullEntity
    //         | EntityExists(_)
    //         | InvalidViewEntity(_)
    //         | TooManyStaticEntities
    //         | NoSuchLightmapAnimation(_)
    //         | Model(_)
    //         | Network(_)
    //         | Sound(_)
    //         | Vfs(_) => {
    //             log::error!("{}", e);
    //             // TODO
    //             // self.client.disconnect();
    //         }

    //         _ => panic!("{}", e),
    //     },
    // };

    // if let Some(ref mut game_input) = input.game_input_mut() {
    //     // TODO
    //     self.client
    //         .handle_input(game_input, frame_duration)
    //         .unwrap();
    // }

    // if there's an active trace, record this frame
    // if let Some(ref mut trace_frames) = trace_frames {
    //     trace_frames.push(
    //         todo!(), // self.client
    //                  //     .trace(&[self.client.view_entity_id().unwrap()])
    //                  //     .unwrap(),
    //     );
    // }
}

//     pub fn render(
//         &mut self,
//         gfx_state: &GraphicsState,
//         color_attachment_view: &TextureView,
//         width: u32,
//         height: u32,
//         console: &Console,
//         menu: &Menu,
//     ) {
//         info!("Beginning render pass");
//         let device: &RenderDevice = todo!();
//         let queue: &RenderQueue = todo!();
//         let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
//             label: Some("Main render"),
//         });

//         // TODO
//         // render world, hud, console, menus
//         // self.client
//         //     .render(
//         //         gfx_state,
//         //         &mut encoder,
//         //         width,
//         //         height,
//         //         menu,
//         //         self.input.borrow().focus(),
//         //     )
//         //     .unwrap();

//         // TODO
//         // screenshot setup (TODO: This is pretty complex in order to correctly handle game-only vs final capture)
//         // let mut capture =
//         //     if self.screenshot_path.borrow().is_some() || self.video_context.borrow().is_some() {
//         //         let cap = Capture::new(gfx_state.device(), Extent2d { width, height });
//         //         cap.copy_from_texture(
//         //             &mut encoder,
//         //             wgpu::ImageCopyTexture {
//         //                 texture: gfx_state.deferred_pass_target().color_attachment(),
//         //                 mip_level: 0,
//         //                 origin: wgpu::Origin3d::ZERO,
//         //                 aspect: Default::default(),
//         //             },
//         //         );
//         //         Some(cap)
//         //     } else {
//         //         None
//         //     };

//         // blit to swap chain
//         {
//             let swap_chain_target = SwapChainTarget::with_swap_chain_view(color_attachment_view);
//             let blit_pass_builder = swap_chain_target.render_pass_builder();
//             let mut blit_pass = encoder.begin_render_pass(&blit_pass_builder.descriptor());
//             // gfx_state.blit_pipeline().blit(gfx_state, &mut blit_pass);
//         }

//         let command_buffer = encoder.finish();
//         {
//             queue.submit(vec![command_buffer]);
//             device.poll(wgpu::Maintain::Wait);
//         }

//         // write screenshot if requested and clear screenshot path
//         // self.screenshot_path.replace(None).map(|path| {
//         //     capture
//         //         .as_mut()
//         //         .unwrap()
//         //         .write_to_file(gfx_state.device(), path)
//         // });

//         // if let Some(state) = self.video_context.borrow_mut().take() {
//         //     // RGB texture
//         //     const PIXEL_SIZE: usize = 3;

//         //     capture.as_mut().unwrap().read_texture(gfx_state.device());

//         //     let mut state = match state {
//         //         VideoState::Pending(path) => RecordingState {
//         //             encoder: Encoder::new(
//         //                 &path.into(),
//         //                 EncoderSettings::for_h264_yuv420p(width as usize, height as usize, false),
//         //             )
//         //             .unwrap(),
//         //             last_dt: TimeDelta::try_milliseconds(1000 / 60).unwrap(),
//         //             last_frame: self.client.elapsed(),
//         //             last_rendered_frame: TimeDelta::zero(),
//         //             size: (width as usize, height as usize),
//         //         },
//         //         VideoState::Recording(state) => state,
//         //     };

//         //     let dt = self.client.elapsed() - state.last_frame;

//         //     let dt = if dt < TimeDelta::zero() {
//         //         state.last_dt
//         //     } else {
//         //         dt
//         //     };

//         //     state.last_frame = self.client.elapsed();
//         //     state.last_dt = dt;

//         //     let time = dt + state.last_rendered_frame;

//         //     state.last_rendered_frame = time;

//         //     let approx_time =
//         //         time.num_seconds() as f64 + (time.subsec_nanos() as f64 / 1_000_000_000f64);

//         //     let data = capture.as_ref().unwrap().data().unwrap();
//         //     fn to_srgb(v: u8) -> u8 {
//         //         ((v as f64 / u8::MAX as f64).sqrt() * (u8::MAX as f64)) as u8
//         //     }
//         //     let make_pixel: fn([u8; PIXEL_SIZE]) -> [u8; PIXEL_SIZE] =
//         //         match gfx_state.final_pass_target().format() {
//         //             TextureFormat::Bgra8Unorm => |pixel| {
//         //                 let pixel = pixel.map(to_srgb);

//         //                 [pixel[2], pixel[1], pixel[0]]
//         //             },
//         //             TextureFormat::Bgra8UnormSrgb => |pixel| [pixel[2], pixel[1], pixel[0]],
//         //             TextureFormat::Rgba8Unorm => |pixel| pixel.map(to_srgb),
//         //             TextureFormat::Rgba8UnormSrgb => |pixel| pixel,
//         //             _ => unimplemented!(),
//         //         };
//         //     let texture = match gfx_state.final_pass_target().format() {
//         //         TextureFormat::Bgra8Unorm
//         //         | TextureFormat::Bgra8UnormSrgb
//         //         | TextureFormat::Rgba8Unorm
//         //         | TextureFormat::Rgba8UnormSrgb => {
//         //             // TODO: This will lead to weird artifacts when resizing the window
//         //             let data = data
//         //                 .chunks_exact(4)
//         //                 .map(|pixel| {
//         //                     <[_; 3]>::into_iter(make_pixel([pixel[0], pixel[1], pixel[2]]))
//         //                 })
//         //                 .flatten()
//         //                 .chain(iter::repeat(0))
//         //                 .take(state.size.0 * state.size.1 * PIXEL_SIZE)
//         //                 .collect();
//         //             ndarray::Array3::from_shape_vec((state.size.1, state.size.0, PIXEL_SIZE), data)
//         //         }
//         //         _ => unimplemented!(),
//         //     }
//         //     .expect("TODO: Create texture failed");

//         //     // TODO: Handle failures gracefully
//         //     let _ = state
//         //         .encoder
//         //         .encode(&texture, &Time::from_secs_f64(approx_time));

//         //     *(*self.video_context).borrow_mut() = Some(VideoState::Recording(state));
//         // }
//     }
// }

// impl std::ops::Drop for Game {
//     fn drop(&mut self) {
//         let _ = (*self.cmds).borrow_mut().remove("trace_begin");
//         let _ = (*self.cmds).borrow_mut().remove("trace_end");
//         let _ = (*self.cmds).borrow_mut().remove("screenshot");
//         let _ = (*self.cmds).borrow_mut().remove("startvideo");
//         let _ = (*self.cmds).borrow_mut().remove("startvideogame");
//         let _ = (*self.cmds).borrow_mut().remove("endvideo");

//         if let Some(VideoState::Recording(RecordingState { mut encoder, .. })) =
//             self.video_context.replace(None)
//         {
//             if let Err(e) = encoder.finish() {
//                 debug!("Failed writing video: {}", e);
//             }
//         }
//     }
// }
