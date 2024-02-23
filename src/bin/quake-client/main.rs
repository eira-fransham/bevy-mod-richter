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
    process::{self, exit},
    rc::Rc,
};

use game::Game;

use chrono::Duration;
use common::net::ServerCmd;
use richter::{
    client::{
        self,
        demo::DemoServer,
        input::{Input, InputFocus},
        menu::Menu,
        render::{self, Extent2d, GraphicsState, UiRenderer, DIFFUSE_ATTACHMENT_FORMAT},
        Client,
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

struct ClientProgram<'a> {
    vfs: Rc<Vfs>,
    cvars: Rc<RefCell<CvarRegistry>>,
    cmds: Rc<RefCell<CmdRegistry>>,
    console: Rc<RefCell<Console>>,
    menu: Rc<RefCell<Menu>>,

    window: &'a Window,
    window_dimensions_changed: bool,

    surface: wgpu::Surface<'a>,
    gfx_state: RefCell<GraphicsState>,
    ui_renderer: Rc<UiRenderer>,

    game: Game,
    input: Rc<RefCell<Input>>,
}

impl<'a> ClientProgram<'a> {
    pub async fn new(
        window: &'a Window,
        base_dir: Option<PathBuf>,
        game: Option<&str>,
        trace: bool,
    ) -> Self {
        let vfs = Vfs::with_base_dir(base_dir.unwrap_or(common::default_base_dir()), game);

        let con_names = Rc::new(RefCell::new(Vec::new()));

        let cvars = Rc::new(RefCell::new(CvarRegistry::new(con_names.clone())));
        client::register_cvars(&cvars.borrow()).unwrap();
        render::register_cvars(&cvars.borrow());

        let cmds = Rc::new(RefCell::new(CmdRegistry::new(con_names)));
        // TODO: register commands as other subsystems come online

        let console = Rc::new(RefCell::new(Console::new(cmds.clone(), cvars.clone())));
        let menu = Rc::new(RefCell::new(menu::build_main_menu().unwrap()));

        let input = Rc::new(RefCell::new(Input::new(
            InputFocus::Console,
            console.clone(),
            menu.clone(),
        )));
        input.borrow_mut().bind_defaults();

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let surface = instance.create_surface(window).unwrap();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: Default::default(),
            })
            .await
            .unwrap();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::PUSH_CONSTANTS
                        | wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES
                        | wgpu::Features::TEXTURE_BINDING_ARRAY
                        | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
                    required_limits: wgpu::Limits {
                        max_sampled_textures_per_shader_stage: 128,
                        max_uniform_buffer_binding_size: 65536,
                        max_push_constant_size: 256,
                        ..Default::default()
                    },
                },
                if trace {
                    Some(Path::new("./trace/"))
                } else {
                    None
                },
            )
            .await
            .unwrap();
        let size: Extent2d = window.inner_size().into();
        let config = surface
            .get_default_config(&adapter, size.width, size.height)
            .unwrap_or(Self::surface_config(size.width, size.height));
        surface.configure(&device, &config);

        let default_swapchain_format = config
            .view_formats
            .first()
            .copied()
            .unwrap_or(wgpu::TextureFormat::Bgra8UnormSrgb);

        let vfs = Rc::new(vfs);

        // TODO: warn user if r_msaa_samples is invalid
        let mut sample_count = cvars.borrow().get_value("r_msaa_samples").unwrap_or(2.0) as u32;
        if !&[2, 4].contains(&sample_count) {
            sample_count = 2;
        }

        sample_count = 1;

        let gfx_state = GraphicsState::new(
            device,
            adapter,
            queue,
            default_swapchain_format,
            size,
            sample_count,
            vfs.clone(),
        )
        .unwrap();
        let ui_renderer = Rc::new(UiRenderer::new(&gfx_state, &menu.borrow()));

        // TODO: factor this out
        // implements "exec" command
        let exec_vfs = vfs.clone();

        {
            cmds.borrow_mut()
                .insert_or_replace("exec", move |args| {
                    match args.len() {
                        // exec (filename): execute a script file
                        1 => {
                            let mut script_file = match exec_vfs.open(args[0]) {
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

            // this will also execute config.cfg and autoexec.cfg (assuming an unmodified quake.rc)
            console.borrow().append_text("exec quake.rc\n");
        }

        let client = Client::new(
            vfs.clone(),
            cvars.clone(),
            cmds.clone(),
            console.clone(),
            input.clone(),
            &gfx_state,
            &menu.borrow(),
        );

        let game = Game::new(cvars.clone(), cmds.clone(), input.clone(), client).unwrap();

        ClientProgram {
            vfs,
            cvars,
            cmds,
            console,
            menu,
            window,
            window_dimensions_changed: false,
            surface,
            gfx_state: RefCell::new(gfx_state),
            ui_renderer,
            game,
            input,
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
        let winit::dpi::PhysicalSize { width, height } = self.window.inner_size();
        let config = self
            .surface
            .get_default_config(self.gfx_state.borrow().adapter(), width, height)
            .unwrap_or(Self::surface_config(width, height));
        self.surface
            .configure(self.gfx_state.borrow().device(), &config);
    }

    fn render(&mut self) {
        let swap_chain_output = self.surface.get_current_texture().unwrap();
        let winit::dpi::PhysicalSize { width, height } = self.window.inner_size();
        self.game.render(
            &self.gfx_state.borrow(),
            &swap_chain_output.texture.create_view(&Default::default()),
            width,
            height,
            &self.console.borrow(),
            &self.menu.borrow(),
        );

        swap_chain_output.present();
    }
}

impl Program for ClientProgram<'_> {
    fn handle_event<T>(
        &mut self,
        event: Event<T>,
        _target: &EventLoopWindowTarget<T>,
        _control_flow: &mut ControlFlow,
    ) -> Control {
        match event {
            Event::WindowEvent {
                event: WindowEvent::Resized(_),
                ..
            } => {
                self.window_dimensions_changed = true;
                Control::Continue
            }

            e => self.input.borrow_mut().handle_event(e).unwrap(),
        }
    }

    fn frame(&mut self, frame_duration: Duration) {
        // recreate swapchain if needed
        if self.window_dimensions_changed {
            self.window_dimensions_changed = false;
            self.recreate_swap_chain();
        }

        let size: Extent2d = self.window.inner_size().into();

        // TODO: warn user if r_msaa_samples is invalid
        let mut sample_count = self
            .cvars
            .borrow()
            .get_value("r_msaa_samples")
            .unwrap_or(2.0) as u32;
        if !&[2, 4].contains(&sample_count) {
            sample_count = 2;
        }
        sample_count = 1;

        // recreate attachments and rebuild pipelines if necessary
        self.gfx_state.borrow_mut().update(size, sample_count);
        self.game.frame(&self.gfx_state.borrow(), frame_duration);

        match self.input.borrow().focus() {
            InputFocus::Game => {
                if let Err(e) = self.window.set_cursor_grab(CursorGrabMode::Locked) {
                    // This can happen if the window is running in another
                    // workspace. It shouldn't be considered an error.
                    log::debug!("Couldn't grab cursor: {}", e);
                }

                self.window.set_cursor_visible(false);
            }

            _ => {
                if let Err(e) = self.window.set_cursor_grab(CursorGrabMode::None) {
                    log::debug!("Couldn't release cursor: {}", e);
                };
                self.window.set_cursor_visible(true);
            }
        }

        // run console commands
        self.console.borrow().execute();

        self.render();
    }

    fn shutdown(&mut self) {
        // TODO: do cleanup things here
        process::exit(0);
    }

    fn cvars(&self) -> Ref<CvarRegistry> {
        self.cvars.borrow()
    }

    fn cvars_mut(&self) -> RefMut<CvarRegistry> {
        self.cvars.borrow_mut()
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

fn main() {
    env_logger::init();
    let opt = Opt::from_args();

    let event_loop = EventLoop::new().unwrap();
    let window = {
        #[cfg(target_os = "windows")]
        {
            use winit::platform::windows::WindowBuilderExtWindows as _;
            winit::window::WindowBuilder::new()
                // disable file drag-and-drop so cpal and winit play nice
                .with_drag_and_drop(false)
                .with_title("Richter client")
                .with_inner_size(winit::dpi::PhysicalSize::<u32>::from((1366u32, 768)))
                .build(&event_loop)
                .unwrap()
        }

        #[cfg(not(target_os = "windows"))]
        {
            winit::window::WindowBuilder::new()
                .with_title("Richter client")
                .with_inner_size(winit::dpi::PhysicalSize::<u32>::from((1366u32, 768)))
                .build(&event_loop)
                .unwrap()
        }
    };

    let client_program = futures::executor::block_on(ClientProgram::new(
        &window,
        opt.base_dir,
        opt.game.as_deref(),
        opt.trace,
    ));

    // TODO: make dump_demo part of top-level binary and allow choosing file name
    if let Some(ref demo) = opt.dump_demo {
        let mut demfile = match client_program.vfs.open(demo) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error opening demofile: {}", e);
                std::process::exit(1);
            }
        };

        let mut demserv = match DemoServer::new(&mut demfile) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("error starting demo server: {}", e);
                std::process::exit(1);
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
                                std::process::exit(1);
                            }
                        }
                    }
                }
                None => break,
            }
        }

        std::process::exit(0);
    }

    let mut host = Host::new(client_program);

    if let Some(ref server) = opt.connect {
        host.program()
            .console
            .borrow()
            .append_text(format!("connect {}", server));
    } else if let Some(ref demo) = opt.demo {
        host.program()
            .console
            .borrow()
            .append_text(format!("playdemo {}", demo));
    } else if !opt.demos.is_empty() {
        host.program()
            .console
            .borrow()
            .append_text(format!("startdemos {}", opt.demos.join(" ")));
    }

    host.program().console.borrow().append_text(opt.commands);

    event_loop
        .run(move |event, target| {
            let mut control_flow = ControlFlow::Poll;
            match host.handle_event(event, target, &mut control_flow) {
                Control::Exit => target.exit(),
                Control::Continue => target.set_control_flow(control_flow),
            }
        })
        .unwrap();
}
