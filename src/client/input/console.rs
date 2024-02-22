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

use std::{cell::RefCell, rc::Rc};

use crate::common::console::Console;

use failure::Error;
use winit::{
    event::{ElementState, Event, KeyEvent, WindowEvent},
    keyboard::{Key, NamedKey},
};

pub struct ConsoleInput {
    console: Rc<RefCell<Console>>,
}

impl ConsoleInput {
    pub fn new(console: Rc<RefCell<Console>>) -> ConsoleInput {
        ConsoleInput { console }
    }

    pub fn handle_event<T>(&self, event: Event<T>) -> Result<(), Error> {
        match event {
            Event::WindowEvent { event, .. } => match event {
                WindowEvent::KeyboardInput {
                    event:
                        KeyEvent {
                            logical_key: key,
                            state: ElementState::Pressed,
                            ..
                        },
                    ..
                } => match key.as_ref() {
                    Key::Named(NamedKey::ArrowUp) => self.console.borrow_mut().history_up(),
                    Key::Named(NamedKey::ArrowDown) => self.console.borrow_mut().history_down(),
                    Key::Named(NamedKey::ArrowLeft) => self.console.borrow_mut().cursor_left(),
                    Key::Named(NamedKey::ArrowRight) => self.console.borrow_mut().cursor_right(),
                    Key::Character("`") => self.console.borrow_mut().stuff_text("toggleconsole\n"),
                    Key::Character(c) => {
                        for c in c.chars() {
                            self.console.borrow_mut().send_char(c);
                        }
                    }
                    _ => (),
                },

                _ => (),
            },

            _ => (),
        }

        Ok(())
    }
}
