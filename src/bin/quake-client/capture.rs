use crossbeam_channel::{Receiver, Sender};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use bevy::{prelude::*, render::view::screenshot::ScreenshotManager, window::PrimaryWindow};
use chrono::Utc;
use image::RgbImage;
use seismon::common::console::{ExecResult, RegisterCmdExt as _};

pub struct CapturePlugin;

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                systems::video_frame.run_if(resource_exists::<VideoCtx>),
                systems::recv_frame.run_if(resource_exists::<VideoCtxRecv>),
            ),
        )
        .command(
            "screenshot",
            cmd_screenshot,
            "Take a screenshot of the primary window",
        )
        .command(
            "startvideo",
            cmd_startvideo,
            "Start recording a video of the screen",
        )
        .command(
            "stopvideo",
            cmd_stopvideo,
            "Stop recording a video of the screen",
        );
    }
}

/// Implements the "screenshot" command.
///
/// This function returns a boxed closure which sets the `screenshot_path`
/// argument to `Some` when called.
fn cmd_screenshot(
    In(args): In<Box<[String]>>,
    window: Query<Entity, With<PrimaryWindow>>,
    mut screenshot_manager: ResMut<ScreenshotManager>,
) -> ExecResult {
    let Ok(window) = window.get_single() else {
        return "Can't find primary window".to_owned().into();
    };

    let path = match &*args {
        // TODO: make default path configurable
        [] => PathBuf::from(format!("richter-{}.png", Utc::now().format("%FT%H-%M-%S"))),
        [path] => PathBuf::from(path),
        _ => {
            return "Usage: screenshot [PATH]".to_owned().into();
        }
    };

    match screenshot_manager.save_screenshot_to_disk(window, path) {
        Ok(()) => default(),
        Err(e) => format!("Couldn't take screenshot: {}", e).into(),
    }
}

/// Implements the "screenshot" command.
///
/// This function returns a boxed closure which sets the `screenshot_path`
/// argument to `Some` when called.
fn cmd_startvideo(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    window: Query<&Window, With<PrimaryWindow>>,
    ctx: Option<Res<VideoCtx>>,
) -> ExecResult {
    fn round_to_nearest(x: u32, to: u32) -> u32 {
        let y = to - 1;
        (x + y) & !y
    }

    const LONGEST_SIDE: usize = 800;
    const FPS: f64 = 30.;

    if ctx.is_some() {
        return "Already recording video".into();
    }

    let (mut path, longest_side) = match &*args {
        // TODO: make default path configurable
        [] => (
            PathBuf::from(format!("richter-{}.mp4", Utc::now().format("%FT%H-%M-%S"))),
            LONGEST_SIDE,
        ),
        [path] => (PathBuf::from(path), LONGEST_SIDE),
        [path, resolution] => (
            PathBuf::from(path),
            resolution.parse::<usize>().unwrap_or(LONGEST_SIDE),
        ),
        _ => {
            return "Usage: startvideo [PATH] [RESOLUTION]".to_owned().into();
        }
    };
    path.set_extension("mp4");

    let aspect_ratio = window
        .get_single()
        .map(|w| w.width() / w.height())
        .unwrap_or(4. / 3.);
    let (w, h) = if aspect_ratio > 1. {
        (longest_side as f32, longest_side as f32 / aspect_ratio)
    } else {
        (longest_side as f32 * aspect_ratio, longest_side as f32)
    };
    let (w, h) = (
        round_to_nearest(w as u32, 10),
        round_to_nearest(h as u32, 10),
    );

    let out = format!("Recording a video ({}x{}) to {}", w, h, path.display());

    let (sender, receiver) = crossbeam_channel::unbounded::<VideoFrame>();
    let frame_time = Duration::from_secs_f64(FPS.recip());

    let encoder = video_rs::Encoder::new(
        &path.into(),
        video_rs::EncoderSettings::for_h264_yuv420p(w as _, h as _, true),
    )
    .unwrap();

    commands.insert_resource(VideoCtx {
        send_frame: sender,
        size: (w, h),
        frame_time,
        last_time: None,
        cur_frame: 0,
        closed: Arc::new(false.into()),
    });

    commands.insert_resource(VideoCtxRecv {
        recv_frame: Some(receiver),
        frame_buf: default(),
        encoder,
        frame_time: video_rs::Time::from_nth_of_a_second(FPS as _),
        cur_frame: 0,
    });

    out.into()
}

fn cmd_stopvideo(
    In(_): In<Box<[String]>>,
    mut commands: Commands,
    ctx: Option<Res<VideoCtx>>,
) -> ExecResult {
    if ctx.is_some() {
        commands.remove_resource::<VideoCtx>();
        default()
    } else {
        "Error: no video recording in progress".into()
    }
}

struct VideoFrame {
    image: RgbImage,
    frame_id: usize,
}

#[derive(Resource)]
struct VideoCtx {
    send_frame: Sender<VideoFrame>,
    size: (u32, u32),
    last_time: Option<Duration>,
    frame_time: Duration,
    cur_frame: usize,
    closed: Arc<AtomicBool>,
}

#[derive(Resource)]
struct VideoCtxRecv {
    recv_frame: Option<Receiver<VideoFrame>>,
    frame_buf: BTreeMap<usize, RgbImage>,
    cur_frame: usize,
    frame_time: video_rs::Time,
    encoder: video_rs::Encoder,
}

mod systems {
    use crossbeam_channel::TryRecvError;
    use std::sync::atomic::Ordering;

    use image::imageops::FilterType;

    use super::*;

    pub fn video_frame(
        mut commands: Commands,
        mut screenshot: ResMut<ScreenshotManager>,
        window: Query<Entity, With<PrimaryWindow>>,
        time: Res<Time>,
        mut ctx: ResMut<VideoCtx>,
    ) {
        let Ok(window) = window.get_single() else {
            commands.remove_resource::<VideoCtx>();
            return;
        };

        if ctx.closed.load(Ordering::SeqCst) {
            commands.remove_resource::<VideoCtx>();
            return;
        }

        if ctx
            .last_time
            .map(|t| time.elapsed() >= (t + ctx.frame_time))
            .unwrap_or(true)
        {
            let sender = ctx.send_frame.clone();
            let frame_id = ctx.cur_frame;
            let size = ctx.size;
            let closed = ctx.closed.clone();

            ctx.last_time = Some(time.elapsed());

            if let Ok(_) = screenshot.take_screenshot(window, move |image| {
                let image = image
                    .try_into_dynamic()
                    .unwrap()
                    .resize(size.0, size.1, FilterType::Nearest)
                    .into_rgb8();

                if let Err(_) = sender.send(VideoFrame { image, frame_id }) {
                    closed.store(true, Ordering::SeqCst);
                }
            }) {
                ctx.cur_frame += 1;
            }
        }

        // Handle new frames
    }

    pub fn recv_frame(mut ctx: ResMut<VideoCtxRecv>, mut commands: Commands) {
        loop {
            let frame = match (ctx.frame_buf.first_key_value(), &ctx.recv_frame) {
                (Some((frame, _)), _) if *frame == ctx.cur_frame => {
                    let (_, frame) = ctx.frame_buf.pop_first().unwrap();
                    frame
                }
                (Some(_), None) => {
                    let (_, frame) = ctx.frame_buf.pop_first().unwrap();
                    frame
                }
                (_, Some(recv)) => {
                    match recv.try_recv() {
                        Ok(next) => {
                            ctx.frame_buf.insert(next.frame_id, next.image);
                        }
                        Err(TryRecvError::Empty) => break,
                        Err(TryRecvError::Disconnected) => ctx.recv_frame = None,
                    }

                    continue;
                }
                (None, None) => {
                    commands.remove_resource::<VideoCtxRecv>();
                    break;
                }
            };

            let frame = frame.into_flat_samples();
            let frame_array = ndarray::Array3::<u8>::from_shape_vec(
                (
                    frame.layout.height as usize,
                    frame.layout.width as usize,
                    frame.layout.channels as usize,
                ),
                frame.samples,
            )
            .unwrap();
            let time = video_rs::Time::new(
                Some(ctx.cur_frame as _),
                ctx.frame_time.clone().into_parts().1,
            );
            ctx.encoder.encode(&frame_array, &time).unwrap();
            ctx.cur_frame += 1;
        }
    }
}
