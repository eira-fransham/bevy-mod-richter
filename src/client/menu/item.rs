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

use std::fmt::Debug;

use crate::{client::menu::Menu, common::host::Control};

use failure::{ensure, Error};

pub struct Action(pub Box<dyn Fn() -> Control + Send + Sync>);

impl From<Box<dyn Fn() -> Control + Send + Sync>> for Action {
    fn from(value: Box<dyn Fn() -> Control + Send + Sync>) -> Self {
        Self(value)
    }
}

impl<F> From<F> for Action
where
    F: Fn() + Send + Sync + 'static,
{
    fn from(value: F) -> Self {
        Self(Box::new(move || {
            value();
            Control::Continue
        }))
    }
}

pub enum Item {
    Submenu(Menu),
    Action(Action),
    Toggle(Toggle),
    Enum(Enum),
    Slider(Slider),
    TextField(TextField),
}

impl Clone for Item {
    fn clone(&self) -> Self {
        // TODO: We need to give items access to the world so that we don't need to make the actions `FnMut`
        todo!();
    }
}

impl Debug for Item {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        enum Item<'a> {
            Submenu(&'a Menu),
            Action,
            Toggle {
                state: bool,
            },
            Enum {
                selected: usize,
                items: Vec<&'a str>,
            },
            // TODO
        }

        todo!()
    }
}

pub struct Toggle {
    state: bool,
    on_toggle: Box<dyn Fn(bool) + Send + Sync>,
}

impl Toggle {
    pub fn new(init: bool, on_toggle: Box<dyn Fn(bool) + Send + Sync>) -> Toggle {
        let t = Toggle {
            state: init,
            on_toggle,
        };

        // initialize with default
        (t.on_toggle)(init);

        t
    }

    pub fn set_false(&mut self) {
        self.state = false;
        (self.on_toggle)(self.state);
    }

    pub fn set_true(&mut self) {
        self.state = true;
        (self.on_toggle)(self.state);
    }

    pub fn toggle(&mut self) {
        self.state = !self.state;
        (self.on_toggle)(self.state);
    }

    pub fn get(&self) -> bool {
        self.state
    }
}

// TODO: add wrapping configuration to enums
// e.g. resolution enum wraps, texture filtering does not
pub struct Enum {
    selected: usize,
    items: Vec<EnumItem>,
}

impl Enum {
    pub fn new(init: usize, items: Vec<EnumItem>) -> Result<Enum, Error> {
        ensure!(items.len() > 0, "Enum element must have at least one item");
        ensure!(init < items.len(), "Invalid initial item ID");

        let e = Enum {
            selected: init,
            items,
        };

        // initialize with the default choice
        (e.items[e.selected].on_select)();

        Ok(e)
    }

    pub fn selected_name(&self) -> &str {
        self.items[self.selected].name.as_str()
    }

    pub fn select_next(&mut self) {
        let selected = match self.selected + 1 {
            s if s >= self.items.len() => 0,
            s => s,
        };

        self.selected = selected;
        (self.items[selected].on_select)();
    }

    pub fn select_prev(&mut self) {
        let selected = match self.selected {
            0 => self.items.len() - 1,
            s => s - 1,
        };

        self.selected = selected;
        (self.items[selected].on_select)();
    }
}

pub struct EnumItem {
    name: String,
    on_select: Box<dyn Fn() + Send + Sync>,
}

impl EnumItem {
    pub fn new<S>(name: S, on_select: Box<dyn Fn() + Send + Sync>) -> Result<EnumItem, Error>
    where
        S: AsRef<str>,
    {
        Ok(EnumItem {
            name: name.as_ref().to_string(),
            on_select,
        })
    }
}

pub struct Slider {
    min: f32,
    _max: f32,
    increment: f32,
    steps: usize,

    selected: usize,
    on_select: Box<dyn Fn(f32) + Send + Sync>,
}

impl Slider {
    pub fn new(
        min: f32,
        max: f32,
        steps: usize,
        init: usize,
        on_select: Box<dyn Fn(f32) + Send + Sync>,
    ) -> Result<Slider, Error> {
        ensure!(steps > 1, "Slider must have at least 2 steps");
        ensure!(init < steps, "Invalid initial setting");
        ensure!(
            min < max,
            "Minimum setting must be less than maximum setting"
        );

        Ok(Slider {
            min,
            _max: max,
            increment: (max - min) / (steps - 1) as f32,
            steps,
            selected: init,
            on_select,
        })
    }

    pub fn increase(&mut self) {
        let old = self.selected;

        if old != self.steps - 1 {
            self.selected = old + 1;
        }

        (self.on_select)(self.min + self.selected as f32 * self.increment);
    }

    pub fn decrease(&mut self) {
        let old = self.selected;

        if old != 0 {
            self.selected = old - 1;
        }

        (self.on_select)(self.min + self.selected as f32 * self.increment);
    }

    pub fn position(&self) -> f32 {
        self.selected as f32 / self.steps as f32
    }
}

pub struct TextField {
    chars: String,
    max_len: Option<usize>,
    on_update: Box<dyn Fn(&str) + Send + Sync>,
    cursor: usize,
}

impl TextField {
    pub fn new<S>(
        default: Option<S>,
        max_len: Option<usize>,
        on_update: Box<dyn Fn(&str) + Send + Sync>,
    ) -> Result<TextField, Error>
    where
        S: AsRef<str>,
    {
        let chars = default.map(|s| s.as_ref().to_string()).unwrap_or_default();
        let cursor = chars.len();

        Ok(TextField {
            chars,
            max_len,
            on_update,
            cursor,
        })
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn text(&self) -> &str {
        &self.chars
    }

    pub fn len(&self) -> usize {
        self.chars.len()
    }

    pub fn set_cursor(&mut self, cursor: usize) -> Result<(), Error> {
        ensure!(cursor <= self.len(), "Index out of range");

        self.cursor = cursor;

        Ok(())
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.len();
    }

    pub fn cursor_right(&mut self) {
        let curs = self.cursor;
        if curs < self.len() {
            self.cursor = curs + 1;
        }
    }

    pub fn cursor_left(&mut self) {
        let curs = self.cursor;
        if curs > 1 {
            self.cursor = curs - 1;
        }
    }

    pub fn insert(&mut self, c: char) {
        if let Some(l) = self.max_len {
            if self.len() == l {
                return;
            }
        }

        self.chars.insert(self.cursor, c);
        (self.on_update)(&self.text());
    }

    pub fn backspace(&mut self) {
        if self.cursor > 1 {
            self.chars.remove(self.cursor - 1);
            (self.on_update)(&self.text());
        }
    }

    pub fn delete(&mut self) {
        if self.cursor < self.len() {
            self.chars.remove(self.cursor);
            (self.on_update)(&self.text());
        }
    }
}

// TODO: Fix tests
// #[cfg(test)]
// mod test {
//     use super::*;
//     use std::{cell::RefCell, rc::Rc};

//     #[test]
//     fn test_toggle() {
//         let s = Rc::new(RefCell::new("false".to_string()));

//         let s2 = s.clone();
//         let item = Toggle::new(
//             false,
//             Box::new(move |state| {
//                 s2.replace(format!("{}", state));
//             }),
//         );
//         item.toggle();

//         assert_eq!(*s.borrow(), "true");
//     }

//     #[test]
//     fn test_enum() {
//         let target = Rc::new(RefCell::new("null".to_string()));

//         let enum_items = (0..3i32)
//             .into_iter()
//             .map(|i: i32| {
//                 let target_handle = target.clone();
//                 EnumItem::new(
//                     format!("option_{}", i),
//                     Box::new(move || {
//                         target_handle.replace(format!("option_{}", i));
//                     }),
//                 )
//                 .unwrap()
//             })
//             .collect();

//         let e = Enum::new(0, enum_items).unwrap();
//         assert_eq!(*target.borrow(), "option_0");

//         // wrap under
//         e.select_prev();
//         assert_eq!(*target.borrow(), "option_2");

//         e.select_next();
//         e.select_next();
//         e.select_next();
//         assert_eq!(*target.borrow(), "option_2");

//         // wrap over
//         e.select_next();
//         assert_eq!(*target.borrow(), "option_0");
//     }

//     #[test]
//     fn test_slider() {
//         let f = Rc::new(Cell::new(0.0f32));

//         let f2 = f.clone();
//         let item = Slider::new(
//             0.0,
//             10.0,
//             11,
//             0,
//             Box::new(move |f| {
//                 f2.set(f);
//             }),
//         )
//         .unwrap();

//         // don't underflow
//         item.decrease();
//         assert_eq!(f.get(), 0.0);

//         for i in 0..10 {
//             item.increase();
//             assert_eq!(f.get(), i as f32 + 1.0);
//         }

//         // don't overflow
//         item.increase();
//         assert_eq!(f.get(), 10.0);
//     }

//     #[test]
//     fn test_textfield() {
//         let MAX_LEN = 10;
//         let s = Rc::new(RefCell::new("before".to_owned()));
//         let s2 = s.clone();

//         let mut tf = TextField::new(
//             Some("default"),
//             Some(MAX_LEN),
//             Box::new(move |x| {
//                 s2.replace(x.to_string());
//             }),
//         )
//         .unwrap();

//         tf.cursor_left();
//         tf.backspace();
//         tf.backspace();
//         tf.home();
//         tf.delete();
//         tf.delete();
//         tf.delete();
//         tf.cursor_right();
//         tf.insert('f');
//         tf.end();
//         tf.insert('e');
//         tf.insert('r');

//         assert_eq!(tf.text(), *s.borrow());

//         for _ in 0..2 * MAX_LEN {
//             tf.insert('x');
//         }

//         assert_eq!(tf.len(), MAX_LEN);
//     }
// }
