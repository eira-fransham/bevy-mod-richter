use std::str::FromStr as _;

use crate::common::console::{ExecResult, RegisterCmdExt};

use bevy::prelude::*;
use fxhash::FxHashMap;
use strum::IntoEnumIterator as _;

use super::game::{Action, BindInput, BindTarget, GameInput};

pub fn register_commands(app: &mut App) {
    let states = [("+", true), ("-", false)];
    for action in Action::iter() {
        for (state_str, state_bool) in states.iter().cloned() {
            let cmd_name = format!("{}{}", state_str, action.to_string());
            // TODO: Document bindings
            app.command(
                cmd_name,
                move |_: In<Box<[String]>>, mut game_input: ResMut<GameInput>| -> ExecResult {
                    game_input.action_states[action as usize] = state_bool;
                    default()
                },
                "",
            );
        }
    }

    // "bind"
    app.command("bind", cmd_bind, "attach a command to a key");

    // "unbindall"
    app.command("unbindall", cmd_unbindall, "delete all keybindings");

    // "impulse"
    // TODO: Add "extended help" for cases like this
    app.command(
        "impulse",
        move |In(args): In<Box<[String]>>, world: &mut World| -> ExecResult {
            println!("args: {}", args.len());
            match args.len() {
                1 => match u8::from_str(&args[0]) {
                    Ok(i) => {
                        let mut game_input = world.resource_mut::<GameInput>();
                        game_input.impulse = i;
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
        1 => match BindInput::from_str(&args[0]) {
            Ok(i) => match game_input.bindings.get(&i) {
                Some(t) => format!("\"{}\" = \"{}\"", i.to_string(), t.to_string()).into(),
                None => format!("\"{}\" is not bound", i.to_string()).into(),
            },

            Err(_) => format!("\"{}\" isn't a valid key", args[0]).into(),
        },

        // bind (key) [command]
        2 => match BindInput::from_str(&args[0]) {
            Ok(input) => match BindTarget::from_str(&args[1]) {
                Ok(target) => {
                    game_input.bindings.insert(input.clone(), target);
                    debug!("Bound {:?} to {:?}", input, args[1]);
                    default()
                }
                Err(_) => format!("\"{}\" isn't a valid bind target", args[1]).into(),
            },

            Err(_) => format!("\"{}\" isn't a valid key", args[0]).into(),
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
