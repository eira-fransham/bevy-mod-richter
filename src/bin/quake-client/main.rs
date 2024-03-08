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

use std::{fs, net::SocketAddr, path::PathBuf, process::ExitCode};

use bevy::{
    audio::AudioPlugin,
    core_pipeline::{
        bloom::BloomSettings,
        prepass::{DepthPrepass, NormalPrepass},
        tonemapping::Tonemapping,
    },
    pbr::DefaultOpaqueRendererMethod,
    prelude::*,
    render::camera::Exposure,
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

const EXPOSURE_CURVE: &[[f32; 2]] = &[[-16.; 2], [0.; 2]];

fn get_default_adjust(tonemapping: Tonemapping) -> ExposureAdjust {
    match tonemapping {
        Tonemapping::AcesFitted => ExposureAdjust::new(1.4, 2.),
        Tonemapping::BlenderFilmic => ExposureAdjust::new(2.0, -1.0),
        Tonemapping::TonyMcMapface => ExposureAdjust::new(1.8, 0.5),
        Tonemapping::SomewhatBoringDisplayTransform => ExposureAdjust::new(2.0, 0.0),
        _ => ExposureAdjust::new(1.0, 0.0),
    }
}

#[derive(Copy, Clone, PartialEq, Debug, Component)]
struct ExposureAdjust {
    inv_scale: f32,
    offset: f32,
}

impl ExposureAdjust {
    fn set_scale(&mut self, scale: f32) {
        self.inv_scale = scale.recip();
    }

    fn set_offset(&mut self, offset: f32) {
        self.offset = offset;
    }

    fn new(scale: f32, offset: f32) -> Self {
        Self {
            inv_scale: scale.recip(),
            offset,
        }
    }
    fn combine(self, rhs: Self) -> Self {
        Self {
            inv_scale: self.inv_scale * rhs.inv_scale,
            offset: self.offset + rhs.offset,
        }
    }
}

impl Default for ExposureAdjust {
    fn default() -> Self {
        Self::new(1., 0.)
    }
}

fn adjust_exposure(
    mut cameras: Query<
        (&mut AutoExposure, Option<&Tonemapping>, &ExposureAdjust),
        Or<(Changed<ExposureAdjust>, Changed<Tonemapping>)>,
    >,
) {
    for (mut auto_exposure, tonemapping, adjust) in &mut cameras {
        let adjust = adjust.combine(get_default_adjust(
            tonemapping.copied().unwrap_or(Tonemapping::None),
        ));
        auto_exposure.compensation_curve = EXPOSURE_CURVE
            .iter()
            .map(|[from, to]| (*from, adjust.offset + *to * adjust.inv_scale).into())
            .collect();
    }
}

fn cmd_exposure(
    In(args): In<Box<[String]>>,
    mut adjusts: Query<&mut ExposureAdjust>,
) -> ExecResult {
    let (exposure, offset) = match &*args {
        [new_exposure] => (new_exposure, None),
        [new_exposure, new_offset] => (new_exposure, Some(new_offset)),
        _ => return "usage: r_exposure [EXPOSURE]".into(),
    };

    let exposure: f32 = match exposure.parse() {
        Ok(exposure) => exposure,
        Err(e) => return format!("couldn't parse exposure: {}", e).into(),
    };
    let offset: Option<f32> = match offset.map(|offset| offset.parse()) {
        Some(Ok(offset)) => Some(offset),
        None => None,
        Some(Err(e)) => return format!("couldn't parse exposure: {}", e).into(),
    };

    for mut adjust in &mut adjusts {
        adjust.set_scale(exposure);
        if let Some(offset) = offset {
            adjust.set_offset(offset);
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
                tonemapping: Tonemapping::TonyMcMapface,
                ..default()
            },
            AutoExposure {
                min: -16.0,
                max: 0.0,
                ..default()
            },
            BloomSettings::default(),
            DepthPrepass,
            NormalPrepass,
            ExposureAdjust::default(),
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
    .command(
        "r_exposure",
        cmd_exposure,
        "Adjust the exposure of the screen by a factor and an optional offset",
    )
    .command(
        "r_tonemapping",
        cmd_tonemapping ,
        "Set the tonemapping type - Tony McMapFace (TMMF), ACES, Blender Filmic, Somewhat Boring Display Transform (SBBT), or none",
    )
    .insert_resource(DefaultOpaqueRendererMethod::deferred())
    .add_systems(Startup, startup(opt))
    .add_systems(Update, adjust_exposure);

    fs::write(
        "debug-out.dot",
        bevy_mod_debugdump::render_graph_dot(&app, &Default::default()),
    )
    .unwrap();

    app.run();

    0.into()
}
