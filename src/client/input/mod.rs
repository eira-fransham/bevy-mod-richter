// Copyright © 2018 Cormac O'Brien
//
// Permission is hereby granted, free of charge, to any person obtaining a copy of this software
// and associated documentation files (the "Software"), to deal in the Software without
// restriction, including without limitation the rights to use, copy, modify, merge, publish,
// distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the
// Software is furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in all copies or
// substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING
// BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND
// NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM,
// DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

pub mod commands;
pub mod console;
pub mod game;
pub mod menu;

use bevy::{
    app::{Plugin, Update},
    ecs::system::Resource,
    render::extract_resource::ExtractResource,
};

use self::game::GameInput;

pub struct RichterInputPlugin;

impl Plugin for RichterInputPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.init_resource::<InputFocus>()
            .init_resource::<GameInput>()
            .add_systems(Update, (systems::handle_input,));

        commands::register_commands(app);
    }
}

#[derive(Default, Clone, Copy, Debug, PartialEq, Eq, Resource, ExtractResource)]
pub enum InputFocus {
    Game,
    #[default]
    Console,
    Menu,
}

pub mod systems {
    use bevy::{
        input::{keyboard::KeyboardInput, ButtonState},
        prelude::*,
        window::PrimaryWindow,
    };

    use crate::{client::menu::Menu, common::console::RunCmd};

    use super::{
        game::{AnyInput, Binding, BindingValidState, GameInput, Trigger},
        InputFocus,
    };

    pub fn handle_input(
        keyboard_events: EventReader<KeyboardInput>,
        focus: Res<InputFocus>,
        commands: Commands,
        windows: Query<&Window, With<PrimaryWindow>>,
        run_cmds: EventWriter<RunCmd<'static>>,
        input: Res<GameInput>,
        menu: Option<ResMut<Menu>>,
    ) {
        let Ok(window) = windows.get_single() else {
            return;
        };
        if !window.focused {
            return;
        }

        match *focus {
            InputFocus::Game => game_input(keyboard_events, run_cmds, input),
            InputFocus::Menu => {
                if let Some(menu) = menu {
                    menu_input(commands, keyboard_events, run_cmds, menu, input);
                }
            }
            InputFocus::Console => {
                // TODO: Implement console input
            }
        }
    }

    fn game_input(
        mut keyboard_events: EventReader<KeyboardInput>,
        mut run_cmds: EventWriter<RunCmd<'static>>,
        input: Res<GameInput>,
    ) {
        for (i, key) in keyboard_events.read().enumerate() {
            // TODO: Make this work better if we have arguments - currently we clone the arguments every time
            // TODO: Error handling
            if let Ok(Some(binding)) = input.binding(key.logical_key.clone()) {
                run_cmds.send_batch(binding.commands.iter().filter_map(|cmd| {
                    match (cmd.0.trigger, key.state) {
                        (Some(Trigger::Positive) | None, ButtonState::Pressed) => Some(cmd.clone()),
                        (Some(Trigger::Positive) | None, ButtonState::Released) => {
                            cmd.clone().invert()
                        }
                        (Some(Trigger::Negative), _) => unreachable!(
                            "Binding found to a negative edge! TODO: Do we want to support this?"
                        ),
                    }
                }));
            }
        }
    }

    fn menu_input(
        mut commands: Commands,
        mut keyboard_events: EventReader<KeyboardInput>,
        mut run_cmds: EventWriter<RunCmd<'static>>,
        mut menu: ResMut<Menu>,
        input: Res<GameInput>,
    ) {
        for (i, key) in keyboard_events.read().enumerate() {
            if let Ok(Some(Binding {
                valid: BindingValidState::Any,
                commands,
            })) = input.binding(key.logical_key.clone())
            {
                run_cmds.send_batch(commands.iter().filter_map(|cmd| {
                    match (cmd.0.trigger, key.state) {
                        (Some(Trigger::Positive) | None, ButtonState::Pressed) => Some(cmd.clone()),
                        (Some(Trigger::Positive) | None, ButtonState::Released) => {
                            cmd.clone().invert()
                        }
                        (Some(Trigger::Negative), _) => unreachable!(
                            "Binding found to a negative edge! TODO: Do we want to support this?"
                        ),
                    }
                }));
            }

            let KeyboardInput {
                logical_key: key,
                state: ButtonState::Pressed,
                ..
            } = key
            else {
                continue;
            };

            let input = AnyInput::from(key.clone());

            // TODO: Make this actually respect the `togglemenu` keybinding
            if input == AnyInput::ESCAPE {
                if menu.at_root() {
                    run_cmds.send("togglemenu".into());
                } else {
                    menu.back().expect("TODO: Handle menu failures");
                }
            } else if input == AnyInput::ENTER {
                if let Some(func) = menu.activate().expect("TODO: Handle menu failures") {
                    func(&mut commands);
                }
            } else if input == AnyInput::UPARROW {
                menu.prev().expect("TODO: Handle menu failures");
            } else if input == AnyInput::DOWNARROW {
                menu.next().expect("TODO: Handle menu failures");
            } else if input == AnyInput::LEFTARROW {
                if let Some(func) = menu.left().expect("TODO: Handle menu failures") {
                    func(&mut commands);
                }
            } else if input == AnyInput::RIGHTARROW {
                if let Some(func) = menu.right().expect("TODO: Handle menu failures") {
                    func(&mut commands);
                }
            }
        }
    }
}
