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

use std::{fs, path::PathBuf, process::ExitCode};

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
    common::console::{ConsoleInput, RegisterCmdExt as _, RunCmd},
};
use serde_lexpr::Value;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct Opt {
    #[structopt(long)]
    base_dir: Option<PathBuf>,

    #[structopt(long)]
    game: Option<String>,

    commands: Vec<String>,
}

const EXPOSURE_CURVE: &[[f32; 2]] = &[[-16., -8.], [0., 0.]];

fn cmd_exposure(In(val): In<Value>, mut exposures: Query<&mut Exposure>) {
    let new_exposure = match val.as_str() {
        Some("indoor") => Exposure::INDOOR,
        Some("blender") => Exposure::BLENDER,
        Some("sunlight") => Exposure::SUNLIGHT,
        Some("overcast") => Exposure::OVERCAST,
        _ => match serde_lexpr::from_value(&val) {
            Ok(exposure) => Exposure { ev100: exposure },
            Err(_) => {
                // TODO: Error handling
                return;
            }
        },
    };

    for mut exposure in &mut exposures {
        *exposure = new_exposure;
    }
}

fn cmd_saturation(In(saturation): In<Value>, mut gradings: Query<&mut ColorGrading>) {
    let saturation: f32 = match serde_lexpr::from_value(&saturation) {
        Ok(saturation) => saturation,
        Err(_) => {
            // TODO: Error handling
            return;
        }
    };

    for mut grading in &mut gradings {
        grading.pre_saturation = saturation;
    }
}

fn cmd_gamma(In(gamma): In<Value>, mut gradings: Query<&mut ColorGrading>) {
    let gamma: f32 = match serde_lexpr::from_value(&gamma) {
        Ok(gamma) => gamma,
        Err(_) => {
            // TODO: Error handling
            return;
        }
    };

    for mut grading in &mut gradings {
        grading.gamma = 1. / gamma;
    }
}

fn cmd_autoexposure(
    In(autoexposure): In<Value>,
    mut commands: Commands,
    mut cameras: Query<(Entity, Option<&AutoExposure>), With<Camera3d>>,
) {
    let enabled: bool = match autoexposure.as_str() {
        Some("on") => true,
        Some("off") => false,
        _ => match serde_lexpr::from_value(&autoexposure) {
            Ok(autoexposure) => autoexposure,
            Err(_) => {
                // TODO: Error handling
                return;
            }
        },
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
}

fn cmd_tonemapping(In(new_tonemapping): In<Value>, mut tonemapping: Query<&mut Tonemapping>) {
    let new_tonemapping = match new_tonemapping.as_str() {
        Some("tmmf") => Tonemapping::TonyMcMapface,
        Some("aces") => Tonemapping::AcesFitted,
        Some("blender") => Tonemapping::BlenderFilmic,
        Some("sbdt") => Tonemapping::SomewhatBoringDisplayTransform,
        Some("none") => Tonemapping::None,
        _ => {
            // TODO: Error handling
            return;
        }
    };

    for mut tonemapping in &mut tonemapping {
        *tonemapping = new_tonemapping;
    }
}

fn startup(opt: Opt) -> impl FnMut(Commands, ResMut<ConsoleInput>, EventWriter<RunCmd<'static>>) {
    move |mut commands, mut input: ResMut<ConsoleInput>, mut console_cmds| {
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

        let mut commands = opt.commands.iter();
        let mut next = commands.next();
        while let Some(cur) = next {
            if let Some(rest) = cur.strip_prefix("+") {
                let mut cmd = rest.to_string();
                loop {
                    next = commands.next();

                    if let Some(arg) = next {
                        if arg.starts_with("+") {
                            break;
                        }

                        cmd.push(' ');
                        cmd.push_str(arg);
                    } else {
                        break;
                    }
                }

                let runcmd = match RunCmd::parse(&*cmd) {
                    Ok(cmd) => cmd.into_owned(),
                    Err(e) => {
                        warn!("Couldn't parse cmd {:?}: {}", cmd, e);
                        continue;
                    }
                };

                input.stuffcmds.push(runcmd);
            } else {
                warn!("Arg without command: {}", cur);
                next = commands.next();
            }
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
    .cvar_on_set(
        "r_exposure",
        "indoor",
        cmd_exposure,
        "Set the physically-based exposure of the screen: indoor, sunny, overcast, blender, or a specific ev100 value",
    )
    .cvar_on_set(
        "r_gamma",
        "1",
        cmd_gamma,
        "Adjust the gamma of the screen",
    )
    .cvar_on_set(
        "r_saturation",
        "1",
        cmd_saturation,
        "Adjust the color saturation of the screen",
    )
    .cvar_on_set(
        "r_tonemapping",
        "blender",
        cmd_tonemapping,
        "Set the tonemapping type - Tony McMapFace (TMMF), ACES, Blender Filmic, Somewhat Boring Display Transform (SBBT), or none",
    )
    .cvar_on_set(
        "r_autoexposure",
        "off",
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
