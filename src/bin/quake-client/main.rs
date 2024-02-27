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

use std::{
    cell::{Ref, RefCell, RefMut},
    fs::File,
    io::{Cursor, Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
    process::ExitCode,
    rc::Rc,
};

use game::Game;

use bevy::{
    prelude::*,
    render::renderer::{RenderAdapterInfo, RenderDevice, RenderQueue},
    window::{PresentMode, WindowTheme},
};
use chrono::Duration;
use common::net::ServerCmd;
use richter::{
    client::{
        self,
        demo::DemoServer,
        input::{Input, InputFocus},
        menu::Menu,
        render::{self, Extent2d, GraphicsState, UiRenderer, DIFFUSE_ATTACHMENT_FORMAT},
    },
    common::{
        self,
        console::{CmdRegistry, Console, CvarRegistry, ExecResult},
        host::{Control, Host, Program},
        vfs::Vfs,
    },
};
use structopt::StructOpt;
use wgpu::CompositeAlphaMode;
use winit::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget},
    window::{CursorGrabMode, Window},
};

#[derive(Resource)]
struct ClientProgram {
    // vfs: Rc<Vfs>,
    // cvars: Rc<RefCell<CvarRegistry>>,
    // cmds: Rc<RefCell<CmdRegistry>>,
    // console: Rc<RefCell<Console>>,
    // menu: Rc<RefCell<Menu>>,
    window_dimensions_changed: bool,

    game: Game,
}

pub fn init(commands: Commands) {
    let vfs = Vfs::with_base_dir(todo!(), todo!()); //base_dir.unwrap_or(common::default_base_dir()), game);

    let con_names = Vec::new();

    let mut cvars = CvarRegistry::new(con_names.clone());
    client::register_cvars(&mut cvars).unwrap();
    render::register_cvars(&mut cvars);
}

impl ClientProgram {
    pub async fn new(
        world: &mut World,
        // commands: Commands,
        base_dir: Option<PathBuf>,
        game: Option<&str>,
        trace: bool,
    ) -> Self {
        let vfs = Vfs::with_base_dir(base_dir.unwrap_or(common::default_base_dir()), game);

        let con_names = Vec::new();

        let mut cvars = CvarRegistry::new(con_names.clone());
        client::register_cvars(&mut cvars).unwrap();
        render::register_cvars(&mut cvars);

        let cmds = CmdRegistry::new(con_names.into_iter());
        // TODO: register commands as other subsystems come online

        let menu = menu::build_main_menu().unwrap();

        let mut input = Input::new(InputFocus::Console);
        input.bind_defaults();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let vfs = Rc::new(vfs);

        // TODO: warn user if r_msaa_samples is invalid
        let mut sample_count = cvars.get_value("r_msaa_samples").unwrap_or(2.0) as u32;
        if !&[2, 4].contains(&sample_count) {
            sample_count = 2;
        }

        sample_count = 1;

        let gfx_state = GraphicsState::new(
            todo!(),
            todo!(),
            todo!(),
            todo!(),
            todo!(),
            sample_count,
            &*vfs,
        )
        .unwrap();

        // TODO: factor this out
        // implements "exec" command
        let exec_vfs = vfs.clone();

        {
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

            let console: &mut Console = todo!();
            // this will also execute config.cfg and autoexec.cfg (assuming an unmodified quake.rc)
            console.append_text("exec quake.rc\n");
        }

        let game = Game::new(world).unwrap();

        ClientProgram {
            window_dimensions_changed: false,
            game,
        }
    }

    fn surface_config(width: u32, height: u32) -> wgpu::SurfaceConfiguration {
        wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: DIFFUSE_ATTACHMENT_FORMAT,
            width,
            height,
            present_mode: wgpu::PresentMode::Immediate,
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: Default::default(),
            desired_maximum_frame_latency: Default::default(),
        }
    }

    /// Builds a new swap chain with the specified present mode and the window's current dimensions.
    fn recreate_swap_chain(&self) {
        // TODO: This should be handled by bevy
        todo!()
    }
}

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
                self.window_dimensions_changed = true;
                Control::Continue
            }

            e => input.handle_event(menu, console, e).unwrap(),
        }
    }

    fn frame(&mut self, frame_duration: Duration) {
        // recreate swapchain if needed
        if self.window_dimensions_changed {
            self.window_dimensions_changed = false;
            self.recreate_swap_chain();
        }

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
        self.game.frame(&*gfx_state, frame_duration);

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

fn main() -> ExitCode {
    env_logger::init();
    let opt = Opt::from_args();

    // fn setup(base_dir: Option<PathBuf>, game: Option<PathBuf>, trace: bool) -> impl System {
    //     move |commands: Commands| {
    //         let vfs = Vfs::with_base_dir(base_dir.unwrap_or(common::default_base_dir()), game);
    //         commands.insert_resource(vfs);
    //     }
    // }

    let app = App::new().add_plugins(DefaultPlugins.set(WindowPlugin {
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
            visible: false,
            ..Default::default()
        }),
        ..Default::default()
    }));
    // .run();

    let world: &mut World = todo!();

    let client_program = futures::executor::block_on(ClientProgram::new(
        world,
        opt.base_dir,
        opt.game.as_deref(),
        opt.trace,
    ));

    // TODO: make dump_demo part of top-level binary and allow choosing file name
    if let Some(ref demo) = opt.dump_demo {
        let vfs: &Vfs = todo!();
        let mut demfile = match vfs.open(demo) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error opening demofile: {}", e);
                return 1.into();
            }
        };

        let mut demserv = match DemoServer::new(&mut demfile) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error starting demo server: {}", e);
                return 1.into();
            }
        };

        let mut outfile = File::create("demodump.txt").unwrap();
        loop {
            match demserv.next() {
                Some(msg) => {
                    let mut curs = Cursor::new(msg.message());
                    loop {
                        match ServerCmd::deserialize(&mut curs) {
                            Ok(Some(cmd)) => write!(&mut outfile, "{:#?}\n", cmd).unwrap(),
                            Ok(None) => break,
                            Err(e) => {
                                eprintln!("error processing demo: {}", e);
                                return 1.into();
                            }
                        }
                    }
                }
                None => break,
            }
        }

        return 0.into();
    }

    let mut host: Host<ClientProgram> = todo!();
    let console: &mut Console = todo!();

    if let Some(ref server) = opt.connect {
        console.append_text(format!("connect {}", server));
    } else if let Some(ref demo) = opt.demo {
        console.append_text(format!("playdemo {}", demo));
    } else if !opt.demos.is_empty() {
        console.append_text(format!("startdemos {}", opt.demos.join(" ")));
    }

    // TODO
    // host.program().console.borrow().append_text(opt.commands);

    // event_loop
    //     .run(move |event, target| {
    //         let mut control_flow = ControlFlow::Poll;
    //         match host.handle_event(event, target, &mut control_flow) {
    //             Control::Exit => target.exit(),
    //             Control::Continue => target.set_control_flow(control_flow),
    //         }
    //     })
    //     .unwrap();

    0.into()
}
