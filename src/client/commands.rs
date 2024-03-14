use std::{collections::VecDeque, io::Read as _};

use beef::Cow;
use bevy::prelude::*;

use crate::common::{
    console::{AliasInfo, ExecResult, RegisterCmdExt as _, Registry, RunCmd},
    net::{ColorShift, QSocket, SignOnStage},
    vfs::Vfs,
};

use super::{
    connect,
    demo::DemoServer,
    input::InputFocus,
    sound::{MixerEvent, MusicSource},
    state::ClientState,
    ColorShiftCode, Connection, ConnectionKind, ConnectionState, DemoQueue,
};

pub fn register_commands(app: &mut App) {
    // set up overlay/ui toggles
    app.command("toggleconsole", cmd_toggleconsole, "TODO: Documentation");
    app.command("togglemenu", cmd_togglemenu, "TODO: Documentation");

    // set up connection console commands
    app.command("connect", cmd_connect, "TODO: Documentation");
    app.command("reconnect", cmd_reconnect, "TODO: Documentation");
    app.command("disconnect", cmd_disconnect, "TODO: Documentation");

    // set up demo playback
    app.command("playdemo", cmd_playdemo, "TODO: Documentation");

    app.command("startdemos", cmd_startdemos, "TODO: Documentation");

    app.command("music", cmd_music, "TODO: Documentation");
    app.command("music_stop", cmd_music_stop, "TODO: Documentation");
    app.command("music_pause", cmd_music_pause, "TODO: Documentation");
    app.command("music_resume", cmd_music_resume, "TODO: Documentation");

    app.command(
        "echo",
        |In(args): In<Box<[String]>>| -> ExecResult {
            let msg = match args.len() {
                0 => Cow::from(""),
                _ => args.join(" ").into(),
            };

            msg.into()
        },
        "TODO: Documentation",
    );

    // TODO: Implement alias
    app.command(
        "alias",
        move |In(args): In<Box<[String]>>, mut registry: ResMut<Registry>| -> ExecResult {
            match &*args {
                [] => {
                    let aliases = registry.aliases();

                    // TODO: There's probably a better way to do this
                    let mut count = 0;
                    let alias_text = aliases.flat_map(
                        |AliasInfo {
                             name,
                             target,
                             help: _,
                         }| {
                            count += 1;
                            ["    ", name, ": ", target, "\n"]
                        },
                    );
                    let mut out = String::new();
                    for text in alias_text {
                        out.push_str(text);
                    }
                    out.push_str(&count.to_string());
                    out.push_str("alias command(s)");

                    out.into()
                }

                [from, to, ..] => {
                    registry.alias(from.clone(), to.clone());

                    default()
                }

                _ => String::new().into(),
            }
        },
        "TODO: Documentation",
    );

    app.command(
        "find",
        move |In(args): In<Box<[String]>>, cmds: Res<Registry>| -> ExecResult {
            match args.len() {
                1 => {
                    // Take every item starting with the target.
                    let it = cmds
                        .all_names()
                        .skip_while(|item| !item.starts_with(&args[0]))
                        .take_while(|item| item.starts_with(&args[0]))
                        .collect::<Vec<_>>()
                        .join("\n");

                    it.into()
                }

                _ => "usage: find <cvar or command>".into(),
            }
        },
        "TODO: Documentation",
    );

    app.command(
        "exec",
        move |In(args): In<Box<[String]>>, vfs: Res<Vfs>| {
            match args.len() {
                // exec (filename): execute a script file
                1 => {
                    let mut script_file = match vfs.open(&args[0]) {
                        Ok(s) => s,
                        Err(e) => {
                            return ExecResult {
                                output: format!("Couldn't exec {}: {:?}", args[0], e).into(),
                                ..default()
                            };
                        }
                    };

                    let mut script = String::new();
                    // TODO: Error handling
                    script_file.read_to_string(&mut script).unwrap();

                    let script = match RunCmd::parse_many(&*script) {
                        Ok(commands) => commands,
                        Err(e) => {
                            return ExecResult {
                                output: format!("Couldn't exec {}: {:?}", args[0], e).into(),
                                ..default()
                            }
                        }
                    };

                    let extra_commands = Box::new(
                        script
                            .into_iter()
                            .map(RunCmd::into_owned)
                            .collect::<Vec<_>>()
                            .into_iter(),
                    );

                    ExecResult {
                        extra_commands,
                        ..default()
                    }
                }

                _ => ExecResult {
                    output: format!("exec (filename): execute a script file").into(),
                    ..default()
                },
            }
        },
        "Execute commands from a script file",
    );

    app.command(
        "bf",
        move |In(_): In<Box<[String]>>, conn: Option<ResMut<Connection>>| -> ExecResult {
            if let Some(mut conn) = conn {
                conn.state.color_shifts[ColorShiftCode::Bonus as usize] = ColorShift {
                    dest_color: [215, 186, 69],
                    percent: 50,
                };
            }
            default()
        },
        "Set extra color shifts",
    );
}

// implements the "toggleconsole" command
pub fn cmd_toggleconsole(
    In(_): In<Box<[String]>>,
    conn: Option<Res<Connection>>,
    mut focus: ResMut<InputFocus>,
) -> ExecResult {
    if conn.is_some() {
        match &*focus {
            InputFocus::Menu | InputFocus::Game => *focus = InputFocus::Console,
            InputFocus::Console => *focus = InputFocus::Game,
        }
    } else {
        match &*focus {
            InputFocus::Console => *focus = InputFocus::Menu,
            InputFocus::Menu => *focus = InputFocus::Console,
            InputFocus::Game => unreachable!("Game focus is invalid when we are disconnected"),
        }
    }

    default()
}

// implements the "togglemenu" command
pub fn cmd_togglemenu(
    In(_): In<Box<[String]>>,
    conn: Option<Res<Connection>>,
    mut focus: ResMut<InputFocus>,
) -> ExecResult {
    if conn.is_some() {
        match &*focus {
            InputFocus::Game => *focus = InputFocus::Menu,
            InputFocus::Console => *focus = InputFocus::Menu,
            InputFocus::Menu => *focus = InputFocus::Game,
        }
    } else {
        match &*focus {
            InputFocus::Console => *focus = InputFocus::Menu,
            InputFocus::Menu => *focus = InputFocus::Console,
            InputFocus::Game => unreachable!("Game focus is invalid when we are disconnected"),
        }
    }
    default()
}

// TODO: this will hang while connecting. ideally, input should be handled in a
// separate thread so the OS doesn't think the client has gone unresponsive.
pub fn cmd_connect(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    mut focus: ResMut<InputFocus>,
) -> ExecResult {
    if args.len() < 1 {
        // TODO: print to console
        return "usage: connect <server_ip>:<server_port>".into();
    }

    match connect(&*args[0]) {
        Ok((new_conn, new_state)) => {
            *focus = InputFocus::Game;
            commands.insert_resource(new_conn);
            commands.insert_resource(Connection::new_server());
            commands.insert_resource(new_state);
            default()
        }
        Err(e) => format!("{}", e).into(),
    }
}

pub fn cmd_reconnect(
    In(_): In<Box<[String]>>,
    conn: Option<Res<Connection>>,
    mut conn_state: ResMut<ConnectionState>,
    mut focus: ResMut<InputFocus>,
) -> ExecResult {
    if conn.is_some() {
        // TODO: clear client state
        *conn_state = ConnectionState::SignOn(SignOnStage::Prespawn);
        *focus = InputFocus::Game;
        default()
    } else {
        // TODO: log message, e.g. "can't reconnect while disconnected"
        "not connected".into()
    }
}

pub fn cmd_disconnect(
    In(_): In<Box<[String]>>,
    mut commands: Commands,
    conn: Option<Res<Connection>>,
    mut focus: ResMut<InputFocus>,
) -> ExecResult {
    if conn.is_some() {
        commands.remove_resource::<Connection>();
        commands.remove_resource::<QSocket>();
        *focus = InputFocus::Console;
        default()
    } else {
        "not connected".into()
    }
}

pub fn cmd_playdemo(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    vfs: Res<Vfs>,
    mut focus: ResMut<InputFocus>,
    mut conn_state: ResMut<ConnectionState>,
) -> ExecResult {
    if args.len() != 1 {
        return "usage: playdemo [DEMOFILE]".into();
    }

    let demo = &args[0];

    let (new_conn, new_state) = {
        let mut demo_file = match vfs.open(format!("{}.dem", demo)) {
            Ok(f) => f,
            Err(e) => {
                return format!("{}", e).into();
            }
        };

        match DemoServer::new(&mut demo_file) {
            Ok(d) => (
                Connection {
                    kind: ConnectionKind::Demo(d),
                    state: ClientState::new(),
                },
                ConnectionState::SignOn(SignOnStage::Prespawn),
            ),
            Err(e) => {
                return format!("{}", e).into();
            }
        }
    };

    *focus = InputFocus::Game;

    commands.insert_resource(new_conn);
    *conn_state = new_state;

    default()
}

pub fn cmd_startdemos(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    vfs: Res<Vfs>,
    mut focus: ResMut<InputFocus>,
    mut conn_state: ResMut<ConnectionState>,
) -> ExecResult {
    if args.len() == 0 {
        return "usage: startdemos [DEMOS]".into();
    }

    let mut demo_queue = args
        .into_iter()
        .map(|s| s.to_string())
        .collect::<VecDeque<_>>();
    let (new_conn, new_state) = match demo_queue.pop_front() {
        Some(demo) => {
            let mut demo_file = match vfs
                .open(format!("{}.dem", demo))
                .or_else(|_| vfs.open(format!("demos/{}.dem", demo)))
            {
                Ok(f) => f,
                Err(e) => {
                    // log the error, dump the demo queue and disconnect
                    return format!("{}", e).into();
                }
            };

            match DemoServer::new(&mut demo_file) {
                Ok(d) => (
                    Connection {
                        kind: ConnectionKind::Demo(d),
                        state: ClientState::new(),
                    },
                    ConnectionState::SignOn(SignOnStage::Prespawn),
                ),
                Err(e) => {
                    return format!("{}", e).into();
                }
            }
        }

        // if there are no more demos in the queue, disconnect
        None => return "usage: startdemos [DEMOS]".into(),
    };

    commands.insert_resource(DemoQueue(demo_queue));
    *focus = InputFocus::Game;

    commands.insert_resource(new_conn);
    *conn_state = new_state;

    default()
}

pub fn cmd_music(In(args): In<Box<[String]>>, world: &mut World) -> ExecResult {
    if args.len() != 1 {
        return "usage: music [TRACKNAME]".into();
    }

    world.send_event(MixerEvent::StartMusic(Some(MusicSource::Named(
        args[0].to_owned(),
    ))));
    // TODO: Handle failure correctly
    // match res {
    //     Ok(()) => String::new(),
    //     Err(e) => {
    //         music_player.stop(commands);
    //         format!("{}", e)
    //     }
    // }
    default()
}

pub fn cmd_music_stop(In(_): In<Box<[String]>>, world: &mut World) -> ExecResult {
    world.send_event(MixerEvent::StopMusic);
    default()
}

pub fn cmd_music_pause(In(_): In<Box<[String]>>, world: &mut World) -> ExecResult {
    world.send_event(MixerEvent::PauseMusic);
    default()
}

pub fn cmd_music_resume(In(_): In<Box<[String]>>, world: &mut World) -> ExecResult {
    world.send_event(MixerEvent::StartMusic(None));
    default()
}
