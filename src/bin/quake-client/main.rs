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

mod capture;
mod game;
mod menu;
mod trace;

use std::{io::Read, net::SocketAddr, path::PathBuf, process::ExitCode};

use bevy::{
    prelude::*,
    render::renderer::RenderDevice,
    window::{PresentMode, WindowTheme},
};
use chrono::Duration;
use richter::{
    client::{
        input::{Input, InputFocus},
        menu::Menu,
        render::{Extent2d, GraphicsState},
        sound::AudioOut,
        RichterPlugin,
    },
    common::{
        console::{CmdRegistry, Console, CvarRegistry, ExecResult},
        host::{Control, Program},
        vfs::Vfs,
    },
};
use rodio::OutputStream;
use structopt::StructOpt;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopWindowTarget},
    window::{CursorGrabMode, Window},
};

struct ClientProgram;

impl Program for ClientProgram {
    fn handle_event<T>(
        &mut self,
        event: Event<T>,
        _target: &EventLoopWindowTarget<T>,
        _control_flow: &mut ControlFlow,
    ) -> Control {
        let input: &mut Input = todo!();
        let menu: &mut Menu = todo!();
        let console: &mut Console = todo!();
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                // self.window_dimensions_changed = true;
                Control::Continue
            }

            e => input.handle_event(menu, console, e).unwrap(),
        }
    }

    fn frame(&mut self, frame_duration: Duration) {
        // recreate swapchain if needed
        // if self.window_dimensions_changed {
        //     self.window_dimensions_changed = false;
        //     self.recreate_swap_chain();
        // }

        let gfx_state: &mut GraphicsState = todo!();
        let input: &mut Input = todo!();
        let cvars: &CvarRegistry = todo!();
        let console: &mut Console = todo!();
        let window: &Window = todo!();
        let render_device: &RenderDevice = todo!();

        let size: Extent2d = window.inner_size().into();

        // TODO: warn user if r_msaa_samples is invalid
        let mut sample_count = cvars.get_value("r_msaa_samples").unwrap_or(2.0) as u32;
        if !&[2, 4].contains(&sample_count) {
            sample_count = 2;
        }
        sample_count = 1;

        // recreate attachments and rebuild pipelines if necessary
        gfx_state.update(render_device, size, sample_count);
        // self.game.frame(&*gfx_state, frame_duration);

        match input.focus() {
            InputFocus::Game => {
                if let Err(e) = window.set_cursor_grab(CursorGrabMode::Locked) {
                    // This can happen if the window is running in another
                    // workspace. It shouldn't be considered an error.
                    log::debug!("Couldn't grab cursor: {}", e);
                }

                window.set_cursor_visible(false);
            }

            _ => {
                if let Err(e) = window.set_cursor_grab(CursorGrabMode::None) {
                    log::debug!("Couldn't release cursor: {}", e);
                };
                window.set_cursor_visible(true);
            }
        }

        // run console commands
        console.execute(todo!());

        // TODO
        // self.render();
    }
}

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(long)]
    trace: bool,

    #[structopt(long)]
    connect: Option<SocketAddr>,

    #[structopt(long)]
    dump_demo: Option<String>,

    #[structopt(long)]
    demo: Option<String>,

    #[structopt(long)]
    demos: Vec<String>,

    #[structopt(short, long, default_value)]
    commands: String,

    #[structopt(long)]
    base_dir: Option<PathBuf>,

    #[structopt(long)]
    game: Option<String>,
}

fn startup(opt: Opt) -> impl FnMut(Commands, ResMut<Console>, ResMut<CmdRegistry>) {
    move |mut commands, mut console, mut cmds| {
        // camera
        commands.spawn((Camera3dBundle {
            transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0))
                .looking_at(Vec3::default(), Vec3::Y),
            ..default()
        },));

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

        console.append_text("exec quake.rc\n");

        if let Some(ref server) = opt.connect {
            console.append_text(format!("connect {}", server));
        } else if let Some(ref demo) = opt.demo {
            console.append_text(format!("playdemo {}", demo));
        } else if !opt.demos.is_empty() {
            console.append_text(format!("startdemos {}", opt.demos.join(" ")));
        }
    }
}

fn main() -> ExitCode {
    let opt = Opt::from_args();

    let (stream, handle) = OutputStream::try_default().unwrap();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(bevy::window::Window {
                title: "Richter client".into(),
                name: Some("Richter client".into()),
                resolution: (1366., 768.).into(),
                present_mode: PresentMode::AutoVsync,
                // Tells wasm not to override default event handling, like F5, Ctrl+R etc.
                prevent_default_event_handling: false,
                window_theme: Some(WindowTheme::Dark),
                enabled_buttons: bevy::window::EnabledButtons {
                    maximize: false,
                    ..Default::default()
                },
                // This will spawn an invisible window
                // The window will be made visible in the make_visible() system after 3 frames.
                // This is useful when you want to avoid the white window that shows up before the GPU is ready to render the app.
                // visible: false,
                ..Default::default()
            }),
            ..Default::default()
        }))
        .insert_resource(AudioOut(handle))
        .add_plugins(RichterPlugin {
            base_dir: opt.base_dir.clone(),
            game: opt.game.clone(),
            main_menu: menu::build_main_menu().expect("TODO: Error handling"),
        })
        .add_systems(Startup, startup(opt))
        .run();

    0.into()
}
