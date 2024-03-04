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

// TODO: These should be removed, but we're still in the refactoring process
#![cfg_attr(debug_assertions, allow(dead_code))]
#![cfg_attr(debug_assertions, allow(unreachable_code))]
#![cfg_attr(debug_assertions, allow(dead_code))]
#![cfg_attr(debug_assertions, allow(unused_variables))]
#![cfg_attr(debug_assertions, allow(unused_assignments))]
#![recursion_limit = "256"]

mod capture;
mod game;
mod menu;
mod trace;

use std::{fs, net::SocketAddr, path::PathBuf, process::ExitCode};

use bevy::{
    core_pipeline::{
        prepass::{DepthPrepass, NormalPrepass},
        tonemapping::Tonemapping,
    },
    pbr::DefaultOpaqueRendererMethod,
    prelude::*,
    render::camera::Exposure,
    window::{PresentMode, WindowTheme},
};
use bevy_mod_auto_exposure::{AutoExposure, AutoExposurePlugin};
use richter::{client::RichterPlugin, common::console::RunCmd};
use structopt::StructOpt;

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

fn startup(opt: Opt) -> impl FnMut(Commands, EventWriter<RunCmd<'static>>) {
    move |mut commands, mut console_cmds| {
        // camera
        commands.spawn((
            Camera3dBundle {
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0))
                    .looking_at(Vec3::default(), Vec3::Y),
                camera: Camera {
                    hdr: true,
                    ..default()
                },
                exposure: Exposure::INDOOR,
                tonemapping: Tonemapping::BlenderFilmic,
                ..default()
            },
            DepthPrepass,
            NormalPrepass,
            AutoExposure {
                min: -16.0,
                max: 16.0,
                compensation_curve: vec![(-16.0, -8.0).into(), (0.0, -2.0).into()],
                ..default()
            },
        ));

        console_cmds.send(RunCmd::parse("exec quake.rc").unwrap());

        if let Some(ref server) = opt.connect {
            console_cmds.send(format!("connect {}", server).parse().unwrap());
        } else if let Some(ref demo) = opt.demo {
            console_cmds.send(format!("playdemo {}", demo).parse().unwrap());
        } else if !opt.demos.is_empty() {
            console_cmds.send(
                format!("startdemos {}", opt.demos.join(" "))
                    .parse()
                    .unwrap(),
            );
        }
    }
}

fn main() -> ExitCode {
    let opt = Opt::from_args();

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
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
            })
            .set(ImagePlugin::default_nearest()),
    )
    .insert_resource(Msaa::Off)
    .add_plugins(AutoExposurePlugin)
    .add_plugins(RichterPlugin {
        base_dir: opt.base_dir.clone(),
        game: opt.game.clone(),
        main_menu: menu::build_main_menu,
    })
    .insert_resource(DefaultOpaqueRendererMethod::deferred())
    .add_systems(Startup, startup(opt));

    fs::write(
        "debug-out.dot",
        bevy_mod_debugdump::render_graph_dot(&app, &Default::default()),
    )
    .unwrap();

    app.run();

    0.into()
}
