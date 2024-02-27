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

pub mod console;
pub mod game;
pub mod menu;

use crate::{
    client::menu::Menu,
    common::{
        console::{CmdRegistry, Console},
        host::Control,
    },
};

use bevy::ecs::system::Resource;
use failure::Error;
use winit::event::{Event, WindowEvent};

use self::game::{BindInput, BindTarget, GameInput};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Resource)]
pub enum InputFocus {
    Game,
    Console,
    Menu,
}

// TODO: Make this a component on player?
#[derive(Resource)]
pub struct Input {
    window_focused: bool,
    focus: InputFocus,

    game_input: GameInput,
}

impl Input {
    pub fn new(init_focus: InputFocus) -> Input {
        Input {
            window_focused: true,
            focus: init_focus,

            game_input: GameInput::new(),
        }
    }

    pub fn handle_event<T>(
        &mut self,
        menu: &mut Menu,
        console: &mut Console,
        event: Event<T>,
    ) -> Result<Control, Error> {
        match event {
            // we're polling for hardware events, so we have to check window focus ourselves
            Event::WindowEvent {
                event: WindowEvent::Focused(focused),
                ..
            } => self.window_focused = focused,

            _ => {
                if self.window_focused {
                    match self.focus {
                        InputFocus::Game => self.game_input.handle_event(console, event),
                        InputFocus::Console => self::console::handle_event(console, event)?,
                        InputFocus::Menu => return self::menu::handle_event(menu, console, event),
                    }
                }
            }
        }

        Ok(Control::Continue)
    }

    pub fn focus(&self) -> InputFocus {
        self.focus
    }

    pub fn set_focus(&mut self, new_focus: InputFocus) {
        self.focus = new_focus;
    }

    /// Bind a `BindInput` to a `BindTarget`.
    pub fn bind<I, T>(&mut self, input: I, target: T) -> Option<BindTarget>
    where
        I: Into<BindInput>,
        T: Into<BindTarget>,
    {
        self.game_input.bind(input, target)
    }

    pub fn bind_defaults(&mut self) {
        self.game_input.bind_defaults();
    }

    pub fn game_input(&self) -> Option<&GameInput> {
        if let InputFocus::Game = self.focus {
            Some(&self.game_input)
        } else {
            None
        }
    }

    pub fn game_input_mut(&mut self) -> Option<&mut GameInput> {
        if let InputFocus::Game = self.focus {
            Some(&mut self.game_input)
        } else {
            None
        }
    }

    pub fn register_cmds(&self, cmds: &mut CmdRegistry) {
        self.game_input.register_cmds(cmds);
    }
}
