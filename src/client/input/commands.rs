use crate::common::console::RegisterCmdExt;

use bevy::prelude::*;
use fxhash::FxHashMap;

use super::game::{Action, BindInput, BindTarget, GameInput};

pub fn register_commands(app: &mut App) {
    let states = [("+", true), ("-", false)];
    for action in Action::iter() {
        for (state_str, state_bool) in states.iter().cloned() {
            let cmd_name = format!("{}{}", state_str, action.to_string());
            // TODO: Document bindings
            app.command(
                &cmd_name,
                move |_, world| {
                    let mut game_input = world.resource_mut::<GameInput>();
                    game_input.action_states[action as usize] = state_bool;
                    String::new()
                },
                "",
            )
            .unwrap();
        }
    }

    // "bind"
    app.command(
        "bind",
        move |args, world| {
            let mut game_input = world.resource_mut::<GameInput>();
            match args.len() {
                // bind (key)
                // queries what (key) is bound to, if anything
                1 => match BindInput::from_str(args[0]) {
                    Ok(i) => match game_input.bindings.get(&i) {
                        Some(t) => format!("\"{}\" = \"{}\"", i.to_string(), t.to_string()),
                        None => format!("\"{}\" is not bound", i.to_string()),
                    },

                    Err(_) => format!("\"{}\" isn't a valid key", args[0]),
                },

                // bind (key) [command]
                2 => match BindInput::from_str(args[0]) {
                    Ok(input) => match BindTarget::from_str(args[1]) {
                        Ok(target) => {
                            game_input.bindings.insert(input.clone(), target);
                            debug!("Bound {:?} to {:?}", input, args[1]);
                            String::new()
                        }
                        Err(_) => {
                            format!("\"{}\" isn't a valid bind target", args[1])
                        }
                    },

                    Err(_) => format!("\"{}\" isn't a valid key", args[0]),
                },

                _ => "bind [key] (command): attach a command to a key".to_owned(),
            }
        },
        "attach a command to a key",
    )
    .unwrap();

    // "unbindall"
    app.command(
        "unbindall",
        move |args, world| match args.len() {
            0 => {
                let mut game_input = world.resource_mut::<GameInput>();
                game_input.bindings = FxHashMap::default();
                String::new()
            }
            _ => "unbindall: delete all keybindings".to_owned(),
        },
        "delete all keybindings",
    )
    .unwrap();

    // "impulse"
    // TODO: Add "extended help" for cases like this
    app.command(
        "impulse",
        move |args, world| {
            println!("args: {}", args.len());
            match args.len() {
                1 => match u8::from_str(args[0]) {
                    Ok(i) => {
                        let mut game_input = world.resource_mut::<GameInput>();
                        game_input.impulse = i;
                        String::new()
                    }
                    Err(_) => "Impulse must be a number between 0 and 255".to_owned(),
                },

                _ => "usage: impulse [number]".to_owned(),
            }
        },
        "apply various effects depending on number",
    )
    .unwrap();
}
