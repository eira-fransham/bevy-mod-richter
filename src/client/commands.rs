use std::collections::VecDeque;

use bevy::prelude::*;

use crate::{common::{console::{ExecResult, RegisterCmdExt as _, Registry}, net::SignOnStage, vfs::Vfs}, server::ClientState};

use super::{connect, demo::DemoServer, input::{Input, InputFocus}, sound::{MixerEvent, MusicSource}, Connection, ConnectionKind, ConnectionState, DemoQueue};

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
        |In(args), _: &mut World| {
            let msg = match args.len() {
                0 => "".to_owned(),
                _ => args.join(" "),
            };

            msg
        },
        "TODO: Documentation",
    );

    // TODO: Implement alias
    // app.command(
    //     "alias",
    //     move |In(args), world: &mut World| -> ExecResult {
    //         match args.len() {
    //             0 => {
    //                 let console = world.resource::<Registry>();

    //                 // TODO: We remove the console from the world, we should probably pass it to the
    //                 //       commands instead
    //                 let aliases = console.aliases();
    //                 let num_aliases = aliases.len();

    //                 aliases
    //                     .map(|(name, script)| format!("    {}: {}\n", name, script))
    //                     .chain(iter::once(format!("{} alias command(s)", num_aliases)))
    //                     .collect::<String>()
    //                     .into()
    //             }

    //             2 => {
    //                 let name = args[0].to_string();
    //                 let script = args[1].to_string();

    //                 ExecResult {
    //                     aliases: vec![(name, script)],
    //                     ..Default::default()
    //                 }
    //             }

    //             _ => String::new().into(),
    //         }
    //     },
    //     "TODO: Documentation",
    // );

    app.command(
        "find",
        move |In(args), cmds: Res<Registry>| {
            match args.len() {
                1 => {
                    // Take every item starting with the target.
                    let it = cmds
                        .names()
                        .skip_while(move |item| !item.starts_with(&args[0]))
                        .take_while(move |item| item.starts_with(&args[0]))
                        .collect::<Vec<_>>()
                        .join("\n");

                    it
                }

                _ => "usage: find <cvar or command>".into(),
            }
        },
        "TODO: Documentation",
    );
}

// implements the "toggleconsole" command
pub fn cmd_toggleconsole(In(_): In<&[&str]>, world: &mut World) -> String {
    let has_conn = world.contains_resource::<Connection>();
    let mut input = world.resource_mut::<Input>();
    let focus = input.focus();
    if has_conn {
        match focus {
            InputFocus::Game => input.set_focus(InputFocus::Console),
            InputFocus::Console => input.set_focus(InputFocus::Game),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    } else {
        match focus {
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Game => unreachable!(),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    }
    String::new()
}

// implements the "togglemenu" command
pub fn cmd_togglemenu(In(_): In<&[&str]>, world: &mut World) -> String {
    let has_conn = world.contains_resource::<Connection>();
    let mut input = world.resource_mut::<Input>();
    let focus = input.focus();
    if has_conn {
        match focus {
            InputFocus::Game => input.set_focus(InputFocus::Menu),
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Menu => input.set_focus(InputFocus::Game),
        }
    } else {
        match focus {
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Game => unreachable!(),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    }
    String::new()
}

// TODO: this will hang while connecting. ideally, input should be handled in a
// separate thread so the OS doesn't think the client has gone unresponsive.
pub fn cmd_connect(In(args): In<&[&str]>, world: &mut World) -> String {
    if args.len() < 1 {
        // TODO: print to console
        return "usage: connect <server_ip>:<server_port>".to_owned();
    }

    match connect(args[0]) {
        Ok((new_conn, new_state)) => {
            world.resource_mut::<Input>().set_focus(InputFocus::Game);
            world.insert_resource(new_conn);
            world.insert_resource(new_state);
            String::new()
        }
        Err(e) => format!("{}", e),
    }
}

pub fn cmd_reconnect(In(args): In<&[&str]>, world: &mut World) -> String {
    match world.get_resource_mut::<ConnectionState>() {
        Some(mut conn) => {
            // TODO: clear client state
            *conn = ConnectionState::SignOn(SignOnStage::Prespawn);
            world.resource_mut::<Input>().set_focus(InputFocus::Game);
            String::new()
        }
        // TODO: log message, e.g. "can't reconnect while disconnected"
        None => "not connected".to_string(),
    }
}

pub fn cmd_disconnect(In(_): In<&[&str]>, world: &mut World) -> String {
    if world.remove_resource::<Connection>().is_some() {
        world.resource_mut::<Input>().set_focus(InputFocus::Console);
        String::new()
    } else {
        "not connected".to_string()
    }
}

pub fn cmd_playdemo(In(args): In<&[&str]>, world: &mut World) -> String {
    if args.len() != 1 {
        return "usage: playdemo [DEMOFILE]".to_owned();
    }

    let demo = args[0];

    let (new_conn, new_state) = {
        let mut demo_file = match world.resource::<Vfs>().open(format!("{}.dem", demo)) {
            Ok(f) => f,
            Err(e) => {
                return format!("{}", e);
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
                return format!("{}", e);
            }
        }
    };

    world.resource_mut::<Input>().set_focus(InputFocus::Game);

    world.insert_resource(new_conn);
    *world.resource_mut::<ConnectionState>() = new_state;

    String::new()
}

pub fn cmd_startdemos(In(args): In<&[&str]>, world: &mut World) -> String {
    if args.len() == 0 {
        return "usage: startdemos [DEMOS]".to_owned();
    }

    let mut demo_queue = args
        .into_iter()
        .map(|s| s.to_string())
        .collect::<VecDeque<_>>();
    let (new_conn, new_state) = match demo_queue.pop_front() {
        Some(demo) => {
            let vfs = world.resource::<Vfs>();
            let mut demo_file = match vfs
                .open(format!("{}.dem", demo))
                .or_else(|_| vfs.open(format!("demos/{}.dem", demo)))
            {
                Ok(f) => f,
                Err(e) => {
                    // log the error, dump the demo queue and disconnect
                    return format!("{}", e);
                }
            };

            match DemoServer::new(&mut demo_file) {
                Ok(d) => (
                    Connection {
                        kind: ConnectionKind::Demo(d),
                        state: todo!(), // ClientState::new(),
                    },
                    ConnectionState::SignOn(SignOnStage::Prespawn),
                ),
                Err(e) => {
                    return format!("{}", e);
                }
            }
        }

        // if there are no more demos in the queue, disconnect
        None => return "usage: startdemos [DEMOS]".to_owned(),
    };

        world.insert_resource(DemoQueue(demo_queue));
    world.resource_mut::<Input>().set_focus(InputFocus::Game);

    world.insert_resource(new_conn);
    *world.resource_mut::<ConnectionState>() = new_state;

    String::new()
}

pub fn cmd_music(In(args): In<&[&str]>, world: &mut World) -> String {
    if args.len() != 1 {
        return "usage: music [TRACKNAME]".to_owned();
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
    String::new()
}

pub fn cmd_music_stop(In(_): In<&[&str]>, world: &mut World) -> String {
    world.send_event(MixerEvent::StopMusic);
    String::new()
}

pub fn cmd_music_pause(In(_): In<&[&str]>, world: &mut World) -> String {
    world.send_event(MixerEvent::PauseMusic);
    String::new()
}

pub fn cmd_music_resume(In(_): In<&[&str]>, world: &mut World) -> String {
    world.send_event(MixerEvent::StartMusic(None));
    String::new()
}
