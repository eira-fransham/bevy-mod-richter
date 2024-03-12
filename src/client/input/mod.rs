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

use bevy::{
    ecs::system::Resource, input::keyboard::KeyboardInput, prelude::*,
    render::extract_resource::ExtractResource,
};

use self::{game::GameInput, systems::InputEventReader};

pub struct RichterInputPlugin;

impl Plugin for RichterInputPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.init_resource::<InputFocus>()
            .init_resource::<GameInput>()
            .init_resource::<InputEventReader<KeyboardInput>>()
            .add_systems(
                Update,
                (
                    systems::game_input
                        .run_if(resource_exists_and_equals::<InputFocus>(InputFocus::Game)),
                    systems::console_input.run_if(resource_exists_and_equals::<InputFocus>(
                        InputFocus::Console,
                    )),
                    systems::menu_input
                        .run_if(resource_exists_and_equals::<InputFocus>(InputFocus::Menu)),
                )
                    .run_if(systems::window_is_focused),
            );

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
        ecs::event::ManualEventReader,
        input::{keyboard::KeyboardInput, ButtonState},
        prelude::*,
        window::PrimaryWindow,
    };
    use chrono::TimeDelta;

    use crate::{
        client::menu::Menu,
        common::console::{to_terminal_key, ConsoleInput, ConsoleOutput, Registry, RunCmd},
    };

    use super::game::{AnyInput, Binding, BindingValidState, GameInput, Trigger};

    pub fn window_is_focused(windows: Query<&Window, With<PrimaryWindow>>) -> bool {
        let Ok(window) = windows.get_single() else {
            return false;
        };
        if !window.focused {
            return false;
        }

        true
    }

    #[derive(Resource)]
    pub struct InputEventReader<E: Event> {
        reader: ManualEventReader<E>,
    }

    impl<E: Event> Default for InputEventReader<E> {
        fn default() -> Self {
            Self { reader: default() }
        }
    }

    pub fn game_input(
        mut reader: ResMut<InputEventReader<KeyboardInput>>,
        keyboard_events: Res<Events<KeyboardInput>>,
        mut run_cmds: EventWriter<RunCmd<'static>>,
        input: Res<GameInput>,
    ) {
        for key in reader.reader.read(&keyboard_events) {
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

    pub fn console_input(
        mut reader: ResMut<InputEventReader<KeyboardInput>>,
        keyboard_events: Res<Events<KeyboardInput>>,
        button_state: Res<ButtonInput<KeyCode>>,
        mut run_cmds: EventWriter<RunCmd<'static>>,
        input: Res<GameInput>,
        mut console_in: ResMut<ConsoleInput>,
        mut console_out: ResMut<ConsoleOutput>,
        time: Res<Time<Virtual>>,
        registry: Res<Registry>,
    ) {
        // TODO: Use a thread_local vector instead of reallocating
        let mut keys = Vec::new();
        for key in reader.reader.read(&keyboard_events) {
            let KeyboardInput {
                logical_key, state, ..
            } = key;

            if let Ok(Some(Binding {
                commands,
                valid: BindingValidState::Any,
            })) = input.binding(logical_key.clone())
            {
                run_cmds.send_batch(commands.iter().filter_map(|cmd| {
                    match (cmd.0.trigger, state) {
                        (Some(Trigger::Positive) | None, ButtonState::Pressed) => Some(cmd.clone()),
                        (Some(Trigger::Positive) | None, ButtonState::Released) => {
                            cmd.clone().invert()
                        }
                        (Some(Trigger::Negative), _) => unreachable!(
                            "Binding found to a negative edge! TODO: Do we want to support this?"
                        ),
                    }
                }));
            } else {
                keys.push(key);
            }
        }

        let elapsed = TimeDelta::from_std(time.elapsed()).unwrap();

        for exec in console_in.update(
            keys.iter()
                .filter_map(
                    |KeyboardInput {
                         logical_key: key,
                         state,
                         ..
                     }| {
                        if *state == ButtonState::Pressed {
                            Some(to_terminal_key(key, &*button_state))
                        } else {
                            None
                        }
                    },
                )
                .flatten(),
            registry.all_names(),
        ) {
            match exec {
                Ok(cmd) => {
                    console_out.print(ConsoleInput::PROMPT, elapsed);
                    console_out.println(&cmd, elapsed);

                    let cmd = RunCmd::parse(&cmd);

                    match cmd {
                        Ok(cmd) => {
                            run_cmds.send(cmd.into_owned());
                        }
                        Err(e) => warn!("Console error: {}", e),
                    }
                }
                // TODO: Print these to console
                Err(e) => warn!("Console error: {}", e),
            }
        }
    }

    pub fn menu_input(
        mut reader: ResMut<InputEventReader<KeyboardInput>>,
        keyboard_events: Res<Events<KeyboardInput>>,
        mut commands: Commands,
        mut run_cmds: EventWriter<RunCmd<'static>>,
        mut menu: ResMut<Menu>,
        input: Res<GameInput>,
    ) {
        for key in reader.reader.read(&keyboard_events) {
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
                let func = menu.activate().expect("TODO: Handle menu failures");
                func(commands.reborrow());
            } else if input == AnyInput::UPARROW {
                menu.prev().expect("TODO: Handle menu failures");
            } else if input == AnyInput::DOWNARROW {
                menu.next().expect("TODO: Handle menu failures");
            } else if input == AnyInput::LEFTARROW {
                let func = menu.left().expect("TODO: Handle menu failures");
                func(commands.reborrow());
            } else if input == AnyInput::RIGHTARROW {
                let func = menu.right().expect("TODO: Handle menu failures");
                func(commands.reborrow());
            }
        }
    }
}
