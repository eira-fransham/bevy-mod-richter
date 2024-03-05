use crate::{
    client::Impulse,
    common::console::{ExecResult, RegisterCmdExt},
};
use std::str::FromStr as _;

use bevy::prelude::*;
use fxhash::FxHashMap;

use super::game::GameInput;

pub fn register_commands(app: &mut App) {
    // "bind"
    app.command("bind", cmd_bind, "attach a command to a key");

    // "unbindall"
    app.command("unbindall", cmd_unbindall, "delete all keybindings");

    // "impulse"
    // TODO: Add "extended help" for cases like this
    app.command(
        "impulse",
        move |In(args): In<Box<[String]>>, mut impulse: EventWriter<Impulse>| -> ExecResult {
            match args.len() {
                1 => match u8::from_str(&args[0]) {
                    Ok(i) => {
                        impulse.send(Impulse(i));
                        default()
                    }
                    Err(_) => "Impulse must be a number between 0 and 255".into(),
                },

                _ => "usage: impulse [number]".into(),
            }
        },
        "apply various effects depending on number",
    );
}

fn cmd_bind(In(args): In<Box<[String]>>, mut game_input: ResMut<GameInput>) -> ExecResult {
    match args.len() {
        // bind (key)
        // queries what (key) is bound to, if anything
        1 => match args.get(0) {
            Some(i) => match game_input.binding(&**i) {
                Ok(Some(t)) => format!("\"{}\" = \"{}\"", i.to_string(), t.to_string()).into(),
                _ => format!("\"{}\" is not bound", i).into(),
            },

            None => format!("\"{}\" isn't a valid key", args[0]).into(),
        },

        // bind (key) [command]
        2 => match &args.get(0) {
            Some(input) => match args.get(1) {
                Some(target) => {
                    game_input
                        .bind(&***input, target.clone())
                        .expect("TODO: Handle binding failures (e.g. invalid key)");
                    debug!("Bound {:?} to {:?}", input, args[1]);
                    default()
                }
                None => format!("\"{}\" isn't a valid bind target", args[1]).into(),
            },
            None => format!("\"{}\" isn't a valid key", args[0]).into(),
        },

        _ => "bind [key] (command): attach a command to a key".into(),
    }
}

fn cmd_unbindall(In(args): In<Box<[String]>>, mut game_input: ResMut<GameInput>) -> ExecResult {
    match args.len() {
        0 => {
            game_input.bindings = FxHashMap::default();
            default()
        }
        _ => "unbindall: delete all keybindings".into(),
    }
}
