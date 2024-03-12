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

#![recursion_limit = "256"]

mod capture;
mod menu;

use std::{borrow::Cow, fs, net::SocketAddr, path::PathBuf, process::ExitCode};

use bevy::{
    audio::AudioPlugin,
    core_pipeline::{
        bloom::BloomSettings,
        prepass::{DepthPrepass, NormalPrepass},
        tonemapping::Tonemapping,
    },
    pbr::DefaultOpaqueRendererMethod,
    prelude::*,
    render::{camera::Exposure, view::ColorGrading},
    window::{PresentMode, WindowTheme},
};
use bevy_mod_auto_exposure::{AutoExposure, AutoExposurePlugin};
use capture::CapturePlugin;
use richter::{
    client::RichterPlugin,
    common::console::{ExecResult, RegisterCmdExt as _, RunCmd},
};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(long)]
    connect: Option<SocketAddr>,

    #[structopt(long)]
    demo: Option<String>,

    #[structopt(long)]
    demos: Vec<String>,

    #[structopt(short, long)]
    commands: Option<String>,

    #[structopt(long)]
    base_dir: Option<PathBuf>,

    #[structopt(long)]
    tonemapping: Option<String>,

    #[structopt(long)]
    game: Option<String>,
}

const EXPOSURE_CURVE: &[[f32; 2]] = &[[-16., -8.], [0., 0.]];

fn cmd_exposure(In(args): In<Box<[String]>>, mut exposures: Query<&mut Exposure>) -> ExecResult {
    let exposure = match &*args {
        [] => {
            let exposures = exposures
                .iter()
                .map(|exposure| -> Cow<str> {
                    match exposure.ev100 {
                        Exposure::EV100_INDOOR => "indoor ".into(),
                        Exposure::EV100_BLENDER => "blender ".into(),
                        Exposure::EV100_SUNLIGHT => "sunlight ".into(),
                        Exposure::EV100_OVERCAST => "overcast ".into(),
                        _ => format!("{} ", exposure.ev100).into(),
                    }
                })
                .collect::<String>();
            return format!("exposure: {}", exposures).into();
        }
        [new_exposure] => new_exposure,
        _ => return "usage: r_exposure [indoor|blender|sunlight|overcast|EV100]".into(),
    };

    let new_exposure = match &**exposure {
        "indoor" => Exposure::INDOOR,
        "blender" => Exposure::BLENDER,
        "sunlight" => Exposure::SUNLIGHT,
        "overcast" => Exposure::OVERCAST,
        _ => match exposure.parse() {
            Ok(exposure) => Exposure { ev100: exposure },
            Err(e) => return format!("couldn't parse exposure: {}", e).into(),
        },
    };

    for mut exposure in &mut exposures {
        *exposure = new_exposure;
    }

    default()
}

fn cmd_saturation(
    In(args): In<Box<[String]>>,
    mut gradings: Query<&mut ColorGrading>,
) -> ExecResult {
    let saturation = match &*args {
        [] => {
            let saturations = gradings
                .iter()
                .map(|g| format!("{} ", g.pre_saturation))
                .collect::<String>();
            return format!("saturation: {}", saturations).into();
        }
        [new_exposure] => new_exposure,
        _ => return "usage: r_saturation [SATURATION]".into(),
    };

    let saturation: f32 = match saturation.parse() {
        Ok(saturation) => saturation,
        Err(e) => return format!("couldn't parse saturation: {}", e).into(),
    };

    for mut grading in &mut gradings {
        grading.pre_saturation = saturation;
    }

    default()
}

fn cmd_gamma(In(args): In<Box<[String]>>, mut gradings: Query<&mut ColorGrading>) -> ExecResult {
    let gamma = match &*args {
        [] => {
            let gammas = gradings
                .iter()
                .map(|g| format!("{} ", g.gamma))
                .collect::<String>();
            return format!("gamma: {}", gammas).into();
        }
        [new_exposure] => new_exposure,
        _ => return "usage: r_gamma [SATURATION]".into(),
    };

    let gamma: f32 = match gamma.parse() {
        Ok(gamma) => gamma,
        Err(e) => return format!("couldn't parse gamma: {}", e).into(),
    };

    for mut grading in &mut gradings {
        grading.gamma = gamma;
    }

    default()
}

fn cmd_autoexposure(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    mut cameras: Query<(Entity, Option<&AutoExposure>), With<Camera3d>>,
) -> ExecResult {
    let enabled = match &*args {
        [] => {
            let enabled = cameras
                .iter()
                .map(|(_, val)| format!("{} ", if val.is_some() { "on" } else { "off" }))
                .collect::<String>();
            return format!("autoexposure: {}", enabled).into();
        }
        [val] => match &**val {
            "on" => true,
            "off" => false,
            _ => return "usage: r_autoexposure [on|off]".into(),
        },
        _ => return "usage: r_autoexposure [on|off]".into(),
    };

    for (e, autoexposure) in &mut cameras {
        match (autoexposure.is_some(), enabled) {
            (true, false) => {
                commands.entity(e).remove::<AutoExposure>();
            }
            (false, true) => {
                commands.entity(e).insert(AutoExposure {
                    compensation_curve: EXPOSURE_CURVE
                        .iter()
                        .copied()
                        .map(|vals| vals.into())
                        .collect(),
                    ..default()
                });
            }
            _ => {}
        }
    }

    default()
}

fn cmd_tonemapping(
    In(args): In<Box<[String]>>,
    mut tonemapping: Query<&mut Tonemapping>,
) -> ExecResult {
    let new_tonemapping = match args.split_first().map(|(s, rest)| (&**s, rest)) {
        Some(("tmmf", [])) => Tonemapping::TonyMcMapface,
        Some(("aces", [])) => Tonemapping::AcesFitted,
        Some(("blender", [])) => Tonemapping::BlenderFilmic,
        Some(("sbdt", [])) => Tonemapping::SomewhatBoringDisplayTransform,
        Some(("none", [])) => Tonemapping::None,
        _ => return "usage: r_tonemapping [tmmf|aces|blender|sbdt|none]".into(),
    };

    for mut tonemapping in &mut tonemapping {
        *tonemapping = new_tonemapping;
    }

    default()
}

fn startup(opt: Opt) -> impl FnMut(Commands, EventWriter<RunCmd<'static>>) {
    move |mut commands, mut console_cmds| {
        // main game camera
        commands.spawn((
            Camera3dBundle {
                transform: Transform::from_translation(Vec3::new(0.0, 0.0, 5.0))
                    .looking_at(Vec3::default(), Vec3::Y),
                camera: Camera {
                    hdr: true,
                    ..default()
                },
                exposure: Exposure::INDOOR,
                // In addition to the in-camera exposure, we add a post exposure grading
                // in order to adjust the brightness on the UI elements.
                color_grading: ColorGrading {
                    exposure: 2.,
                    ..default()
                },
                tonemapping: Tonemapping::TonyMcMapface,
                ..default()
            },
            BloomSettings::default(),
            DepthPrepass,
            NormalPrepass,
        ));

        console_cmds.send(RunCmd::parse("exec quake.rc").unwrap());

        if let Some(server) = opt.connect {
            console_cmds.send(format!("connect {}", server).parse().unwrap());
        } else if let Some(demo) = &opt.demo {
            console_cmds.send(format!("playdemo {}", demo).parse().unwrap());
        } else if !opt.demos.is_empty() {
            console_cmds.send(
                format!("startdemos {}", opt.demos.join(" "))
                    .parse()
                    .unwrap(),
            );
        }

        if let Some(tonemapping) = &opt.tonemapping {
            console_cmds.send(format!("r_tonemapping {}", tonemapping).parse().unwrap());
        }

        if let Some(extra_commands) = &opt.commands {
            console_cmds.send_batch(
                RunCmd::parse_many(extra_commands)
                    .unwrap()
                    .into_iter()
                    .map(RunCmd::into_owned),
            );
        }
    }
}

fn main() -> ExitCode {
    let opt = Opt::from_args();

    let mut app = App::new();
    let default_plugins = DefaultPlugins
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
        .set(ImagePlugin::default_nearest());

    let default_plugins = default_plugins
        .disable::<AudioPlugin>()
        .add(bevy_mod_dynamicaudio::AudioPlugin::default());

    app.add_plugins(
        default_plugins
    )
    .insert_resource(Msaa::Off)
    .add_plugins(AutoExposurePlugin)
    .add_plugins(RichterPlugin {
        base_dir: opt.base_dir.clone(),
        game: opt.game.clone(),
        main_menu: menu::build_main_menu,
    })
    .add_plugins(CapturePlugin)
    // TODO: Make these into cvars - should we allow cvars to access arbitrary parts of the world on get/set?
    .command(
        "r_exposure",
        cmd_exposure,
        "Adjust the exposure of the screen by a factor and an optional offset",
    )
    .command(
        "r_gamma",
        cmd_gamma,
        "Adjust the exposure of the screen by a factor and an optional offset",
    )
    .command(
        "r_saturation",
        cmd_saturation,
        "Adjust the color saturation of the screen (applied before tonemapping)",
    )
    .command(
        "r_tonemapping",
        cmd_tonemapping,
        "Set the tonemapping type - Tony McMapFace (TMMF), ACES, Blender Filmic, Somewhat Boring Display Transform (SBBT), or none",
    )
    .command(
        "r_autoexposure",
        cmd_autoexposure,
        "Enable/disable automatic exposure compensation",
    )
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
