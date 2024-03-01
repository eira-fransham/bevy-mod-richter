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

use std::{
    collections::{BTreeMap, VecDeque},
    mem,
};

use beef::Cow;
use bevy::{
    ecs::{
        system::{Resource, SystemId},
        world::World,
    },
    prelude::*,
};
use chrono::{Duration, Utc};
use fxhash::FxBuildHasher;
use serde::{de::value::MapDeserializer, Deserializer};
use serde_lexpr::Value;
use thiserror::Error;

type CName = Cow<'static, str>;

#[derive(Error, Debug)]
pub enum ConsoleError {
    #[error("{0}")]
    CmdError(CName),
    #[error("Could not parse cvar: {name} = \"{value}\"")]
    CvarParseFailed { name: CName, value: CName },
    #[error("Could not parse cvar")]
    CvarParseInvalid,
    #[error("No such command: {0}")]
    NoSuchCommand(CName),
    #[error("No such cvar: {0}")]
    NoSuchCvar(CName),
}

impl serde::de::Error for ConsoleError {
    // Required method
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        Self::CmdError(format!("{}", msg))
    }

    // Provided methods
    fn invalid_type(unexp: serde::de::Unexpected<'_>, exp: &dyn serde::de::Expected) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn invalid_value(unexp: serde::de::Unexpected<'_>, exp: &dyn serde::de::Expected) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn invalid_length(len: usize, exp: &dyn serde::de::Expected) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn unknown_variant(variant: &str, expected: &'static [&'static str]) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn unknown_field(field: &str, expected: &'static [&'static str]) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn missing_field(field: &'static str) -> Self {
        ConsoleError::CvarParseInvalid
    }
    fn duplicate_field(field: &'static str) -> Self {
        ConsoleError::CvarParseInvalid
    }
}

pub fn cvar_error_handler(In(result): In<Result<(), ConsoleError>>) {
    if let Err(err) = result {
        warn!("encountered an error {:?}", err);
    }
}

// TODO: Add more-complex scripting language
#[derive(Clone)]
pub enum CmdKind {
    Builtin(SystemId<Box<[String]>, Option<ExecResult>>),
    Alias(CName),
    Cvar(Cvar),
}

#[derive(Clone)]
struct CmdInfo {
    kind: CmdKind,
    help: CName,
}

struct RunCmd(CName, Box<[CName]>);

#[derive(Event)]
enum OutputCmd {
    EchoInput(CName),
    Run(RunCmd),
}

#[derive(Event)]
pub struct Print {
    text: CName,
    output_ty: OutputType,
}

pub trait RegisterCmdExt {
    fn command<N, I, S, M>(&mut self, name: N, run: S, usage: I) -> &mut Self
    where
        N: Into<CName>,
        S: IntoSystem<Box<[String]>, Option<ExecResult>, M> + 'static,
        I: Into<CName>;

    fn cvar<N, I, C, M>(&mut self, name: N, value: C, usage: I) -> &mut Self
    where
        N: Into<CName>,
        C: Into<Cvar>,
        I: Into<CName>;
}

impl RegisterCmdExt for App {
    fn command<N, I, S, M>(&mut self, name: N, run: S, usage: I) -> &mut Self
    where
        N: Into<CName>,
        S: IntoSystem<Box<[String]>, Option<ExecResult>, M> + 'static,
        I: Into<CName>,
    {
        let sys = self.world.register_system(run);
        self.world
            .resource_mut::<Registry>()
            .command(name, sys, usage);

        self
    }

    fn cvar<N, I, C, M>(&mut self, name: N, value: C, usage: I) -> &mut Self
    where
        N: Into<CName>,
        C: Into<Cvar>,
        I: Into<CName>,
    {
        self.world
            .resource_mut::<Registry>()
            .cvar(name, value, usage);

        self
    }
}

pub trait CmdExt {
    fn println<T: Into<CName>>(&self, text: T);
    fn println_alert<T: Into<CName>>(&self, text: T);
}

#[derive(Default)]
pub enum OutputType {
    #[default]
    Console,
    Alert,
}

#[derive(Default)]
pub struct ExecResult {
    pub extra_commands: Box<dyn Iterator<Item = RunCmd>>,
    pub output: String,
    pub output_ty: OutputType,
}

impl From<String> for ExecResult {
    fn from(value: String) -> Self {
        Self {
            output: value,
            ..default()
        }
    }
}

/// Stores console commands.
#[derive(Resource, Default, Clone)]
pub struct Registry {
    // We store a history so that we can remove functions and see the previously-defined ones
    layers: Vec<im::HashMap<CName, CmdInfo, FxBuildHasher>>,
    names: im::OrdSet<CName>,
}

impl Registry {
    fn new() -> Registry {
        Self::default()
    }

    fn alias<S, C>(&mut self, name: S, command: C)
    where
        S: Into<CName>,
        C: Into<CName>,
    {
        self.insert(
            name.into(),
            CmdInfo {
                kind: CmdKind::Alias(command.into()),
                // TODO: Implement help text for aliases?
                help: "".into(),
            },
        );
    }

    fn cvar<S, C, H>(&mut self, name: S, cvar: C, help: H)
    where
        S: Into<CName>,
        C: Into<Cvar>,
        H: Into<CName>,
    {
        self.insert(
            name.into(),
            CmdInfo {
                kind: CmdKind::Cvar(cvar.into()),
                help: todo!(),
            },
        );
    }

    fn insert<N: Into<CName>>(&mut self, name: N, value: CmdInfo) {
        let name = name.into();

        if self.contains(&*name) {
            let new_layer = self.layers.last().cloned().unwrap_or_default();
            self.layers.push(new_layer);
        }

        self.layers.last_mut().unwrap().insert(name, value);
    }

    /// Registers a new command with the given name.
    ///
    /// Returns an error if a command with the specified name already exists.
    fn command<N, C, H>(
        &mut self,
        name: N,
        cmd: SystemId<Box<[String]>, Option<ExecResult>>,
        help: H,
    ) where
        N: Into<CName>,
        H: Into<CName>,
    {
        self.insert(
            name.into(),
            CmdInfo {
                kind: CmdKind::Builtin(cmd),
                help: help.into(),
            },
        );
    }

    /// Removes the command with the given name.
    ///
    /// Returns an error if there was no command with that name.
    // TODO: If we remove a builtin we should also remove the corresponding system from the world
    fn remove<S>(&mut self, name: S) -> Result<(), ConsoleError>
    where
        S: Into<CName>,
    {
        let name = name.into();
        let mut found_command = false;
        let mut remove_from_names = true;
        for layer in self.layers.iter_mut().rev() {
            if found_command {
                if layer.contains_key(&*name) {
                    remove_from_names = false;
                    break;
                }
            } else if layer.remove(&*name).is_some() {
                found_command = true;
            }
        }

        if remove_from_names {
            self.names.remove(&*name);
        }

        if found_command {
            Ok(())
        } else {
            Err(ConsoleError::NoSuchCommand(name))
        }
    }

    /// Removes the command with the given name.
    ///
    /// Returns an error if there was no command with that name.
    fn remove_alias<S>(&mut self, name: S) -> Result<(), ConsoleError>
    where
        S: Into<CName>,
    {
        let name = name.into();
        let mut found_command = false;
        let mut remove_from_names = true;
        for layer in self.layers.iter_mut().rev() {
            if found_command {
                if layer.contains_key(&*name) {
                    remove_from_names = false;
                    break;
                }
            } else if let im::hashmap::Entry::Occupied(entry) = layer.entry(name) {
                if let CmdKind::Alias(_) = entry.get().kind {
                    entry.remove();
                    found_command = true;
                }
            }
        }

        if remove_from_names {
            self.names.remove(&*name);
        }

        if found_command {
            Ok(())
        } else {
            Err(ConsoleError::NoSuchCommand(name))
        }
    }

    /// Executes a command.
    ///
    /// Returns an error if no command with the specified name exists.
    fn get<S>(&self, name: S) -> Option<CmdInfo>
    where
        S: AsRef<str>,
    {
        for layer in self.layers.iter_mut().rev() {
            if let Some(info) = layer.get(name.as_ref()) {
                return Some(info);
            }
        }

        None
    }

    fn contains<S>(&self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        for layer in self.layers.iter_mut().rev() {
            if layer.contains_key(name.as_ref()) {
                return true;
            }
        }

        false
    }

    // We handle getting cvars differently, as they are needed internally
    fn get_cvar<S: AsRef<str>>(&self, name: S) -> Option<&Cvar> {
        for layer in self.layers.iter_mut().rev() {
            if let Some(info) = layer.get(name.as_ref()) {
                if let CmdKind::Cvar(cvar) = &info.kind {
                    return Some(cvar);
                }
            }
        }

        None
    }

    // We handle getting cvars differently, as they are needed internally
    fn get_cvar_mut<S: AsRef<str>>(&mut self, name: S) -> Option<&mut Cvar> {
        for layer in self.layers.iter_mut().rev() {
            if let Some(info) = layer.get_mut(name.as_ref()) {
                if let CmdKind::Cvar(cvar) = &mut info.kind {
                    return Some(cvar);
                }
            }
        }

        None
    }

    /// Deserialize a single value from cvars
    pub fn read_cvar<'a, V: serde::Deserialize<'a>>(
        &'a self,
        name: impl AsRef<str>,
    ) -> Result<V, ConsoleError> {
        let name = name.as_ref();
        let cvar = self
            .get_cvar(name)
            .ok_or_else(|| ConsoleError::NoSuchCvar(name.to_owned().into()))?;
        serde_lexpr::from_value::<V>(cvar.value()).map_err(|_| ConsoleError::CvarParseFailed {
            name: name.to_owned().into(),
            value: cvar.value().clone(),
        })
    }

    /// Deserialize a struct or similar from cvars
    pub fn read_cvars<'a, V: serde::Deserialize<'a>>(&'a self) -> Option<V> {
        struct CvarDeserializer<'a> {
            inner: &'a Registry,
        }

        impl<'a> Deserializer<'a> for &'a CvarDeserializer<'a> {
            type Error = ConsoleError;

            fn deserialize_struct<V>(
                self,
                name: &'static str,
                fields: &'static [&'static str],
                visitor: V,
            ) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                let deserializer = MapDeserializer::new(
                    fields
                        .into_iter()
                        .filter_map(|name| {
                            self.inner.get_cvar(name).map(|c| (*name, serde_lexpr::value::de::Deserializer::from_value(c.value())))
                        }));

                visitor.visit_map(deserializer)
            }

            fn deserialize_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_bool<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_i8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_i16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_i32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_i64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_u8<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_u16<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_u32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_u64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_f32<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_f64<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_char<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_str<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_string<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_bytes<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_byte_buf<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_option<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_unit<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_unit_struct<V>(
                self,
                name: &'static str,
                visitor: V,
            ) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_newtype_struct<V>(
                self,
                name: &'static str,
                visitor: V,
            ) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_seq<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_tuple<V>(self, len: usize, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_tuple_struct<V>(
                self,
                name: &'static str,
                len: usize,
                visitor: V,
            ) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_map<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_enum<V>(
                self,
                name: &'static str,
                variants: &'static [&'static str],
                visitor: V,
            ) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_identifier<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }

            fn deserialize_ignored_any<V>(self, visitor: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::Visitor<'a>,
            {
                Err(ConsoleError::CvarParseInvalid)
            }
        }

        V::deserialize(CvarDeserializer { inner: self })
    }

    pub fn cmd_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.all_names().filter_map(move |name| {
            self.get(name).and_then(|CmdInfo { kind, .. }| match kind {
                CmdKind::Builtin(_) => Some(name),
                _ => None,
            })
        })
    }

    pub fn alias_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.all_names().filter_map(move |name| {
            self.get(name).and_then(|CmdInfo { kind, .. }| match kind {
                CmdKind::Alias(_) => Some(name),
                _ => None,
            })
        })
    }

    pub fn cvar_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.all_names().filter_map(move |name| {
            self.get(name).and_then(|CmdInfo { kind, .. }| match kind {
                CmdKind::Cvar(_) => Some(name),
                _ => None,
            })
        })
    }

    pub fn all_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.names.iter().map(AsRef::as_ref)
    }
}

/// A configuration variable.
///
/// Cvars are the primary method of configuring the game.
#[derive(Default, Debug, Clone)]
pub struct Cvar {
    // Value of this variable
    val: Option<Value>,

    // If true, this variable should be archived in vars.rc
    archive: bool,

    // If true:
    // - If a server cvar, broadcast updates to clients
    // - If a client cvar, update userinfo
    notify: bool,

    // The default value of this variable
    default: Value,
}

impl From<&'static str> for Cvar {
    fn from(value: &'static str) -> Self {
        Self::new(value)
    }
}

impl Cvar {
    pub fn new<D: Into<CName>>(default: D) -> Self {
        Self {
            // TODO: Error handling
            default: Value::from_str(default.into()).unwrap(),
            ..Default::default()
        }
    }

    pub fn archive(mut self) -> Self {
        self.archive = true;

        self
    }

    pub fn notify(mut self) -> Self {
        self.notify = true;

        self
    }

    fn value(&self) -> &Value {
        self.val.as_ref().unwrap_or(&self.default)
    }
}

/// The line of text currently being edited in the console.
#[derive(Default, Resource)]
pub struct ConsoleInput {
    text: String,
    curs: usize,

    hist: History,
    commands: Vec<RunCmd>,
}

impl ConsoleInput {
    pub fn send_char(&mut self, c: char) {
        // TODO: Reimplement console
        // match c {
        //     // ignore grave and escape keys
        //     '`' | '\x1b' => (),

        //     '\r' => {
        //         // cap with a newline and push to the execution buffer
        //         self.buffer.push_str(self.input.get_text());
        //         self.buffer.push_str("\n");

        //         // add the current input to the history
        //         self.hist.add_line(self.input.get_text().to_owned());

        //         // echo the input to console output
        //         let input_echo = format!("]{}", self.input.get_text());
        //         // TODO: Send output
        //         self.output.push(input_echo, None);

        //         // clear the input line
        //         // TODO: Send output
        //         self.text.clear();
        //     }

        //     '\x08' => self.input.backspace(),
        //     '\x7f' => self.input.delete(),

        //     '\t' => warn!("Tab completion not implemented"), // TODO: tab completion

        //     // TODO: we should probably restrict what characters are allowed
        //     c => self.input.insert(c),
        // }
    }

    pub fn cursor(&self) -> usize {
        self.curs
    }

    pub fn history_up(&mut self) {
        if let Some(line) = self.hist.line_up().map(ToOwned::to_owned)  {
            self.set_text(line);
        }
    }

    pub fn history_down(&mut self) {
        if let Some(line) = self.hist.line_down().map(ToOwned::to_owned) {
            self.set_text(line);
        }
    }

    pub fn get(&self) -> &str {
        &*self.text
    }

    pub fn append_text<S>(&mut self, text: S)
    where
        S: AsRef<str>,
    {
        debug!("stuff_text:\n{:?}", text.as_ref());
        self.text.push_str(text.as_ref());
        // TODO: Implement this correctly
        self.text.push_str("\n");
    }

    /// Constructs a new `ConsoleInput`.
    ///
    /// Initializes the text content to be empty and places the cursor at position 0.
    pub fn new() -> ConsoleInput {
        ConsoleInput {
            text: Default::default(),
            curs: 0,
            hist: todo!(),
            commands: todo!(),
        }
    }

    /// Returns the current content of the `ConsoleInput`.
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// Sets the content of the `ConsoleInput` to `Text`.
    ///
    /// This also moves the cursor to the end of the line.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.curs = self.text.len();
    }

    /// Inserts the specified character at the position of the cursor.
    ///
    /// The cursor is moved one character to the right.
    pub fn insert(&mut self, c: char) {
        self.text.insert(self.curs, c);
        self.cursor_right();
    }

    /// Moves the cursor to the right.
    ///
    /// If the cursor is at the end of the current text, no change is made.
    pub fn cursor_right(&mut self) {
        if self.curs < self.text.len() {
            self.curs += 1;
        }
        // TODO: ceil_char_boundary is unstable
        while !self.text.is_char_boundary(self.curs) && self.curs < self.text.len() {
            self.curs += 1;
        }
    }

    /// Moves the cursor to the left.
    ///
    /// If the cursor is at the beginning of the current text, no change is made.
    pub fn cursor_left(&mut self) {
        if self.curs > 0 {
            self.curs -= 1;
        }
        // TODO: floor_char_boundary is unstable
        while !self.text.is_char_boundary(self.curs) && self.curs < self.text.len() {
            self.curs += 1;
        }
    }

    /// Deletes the character to the right of the cursor.
    ///
    /// If the cursor is at the end of the current text, no character is deleted.
    pub fn delete(&mut self) {
        if self.curs < self.text.len() {
            let rest = self.text.split_off(self.curs);
            self.text.push_str(rest.as_str());
        }
    }

    /// Deletes the character to the left of the cursor.
    ///
    /// If the cursor is at the beginning of the current text, no character is deleted.
    pub fn backspace(&mut self) {
        if self.curs > 0 {
            self.cursor_left();
            self.delete();
        }
    }

    /// Clears the contents of the `ConsoleInput`.
    ///
    /// Also moves the cursor to position 0.
    pub fn clear(&mut self) {
        self.text.clear();
        self.curs = 0;
    }
}

#[derive(Default)]
pub struct History {
    lines: Vec<String>,
    curs: usize,
}

impl History {
    pub fn new() -> History {
        History {
            lines: Default::default(),
            curs: 0,
        }
    }

    pub fn add_line(&mut self, line: String) {
        self.lines.push(line);
        self.curs = 0;
    }

    // TODO: handle case where history is empty
    pub fn line_up(&mut self) -> Option<&str> {
        if self.lines.len() == 0 || self.curs >= self.lines.len() {
            None
        } else {
            self.curs += 1;
            Some(&self.lines[self.curs - 1])
        }
    }

    pub fn line_down(&mut self) -> Option<&str> {
        if self.curs > 0 {
            self.curs -= 1;
        }

        if self.curs > 0 {
            Some(&self.lines[self.curs - 1])
        } else {
            Some("")
        }
    }
}

struct Line {
    output_type: OutputType,
    text: CName,
}

#[derive(Resource)]
pub struct ConsoleOutput {
    // A ring buffer of lines of text. Each line has an optional timestamp used
    // to determine whether it should be displayed on screen. If the timestamp
    // is `None`, the message will not be displayed.
    //
    // The timestamp is specified in seconds since the Unix epoch (so it is
    // decoupled from client/server time).
    lines: BTreeMap<i64, Line>,
    center_print: (String, i64),
}

impl ConsoleOutput {
    pub fn println<S: Into<Cow<'static, str>>>(&mut self, s: S, timestamp: i64) {
        self.push_line(s, timestamp, OutputType::Console);
    }

    pub fn println_alert<S>(&mut self, s: S, timestamp: i64)
    where
        S: AsRef<str>,
    {
        self.push_line(s, timestamp, OutputType::Console);
    }

    pub fn new() -> ConsoleOutput {
        ConsoleOutput::default()
    }

    fn push_line<S: Into<Cow<'static, str>>>(&mut self, chars: S, timestamp: i64, ty: OutputType) {
        // TODO: set maximum capacity and pop_back when we reach it
        self.lines.insert(
            timestamp,
            Line {
                text: chars.into(),
                output_type: ty,
            },
        );
    }

    pub fn set_center_print<S: Into<imstr::ImString>>(&mut self, print: S) {
        self.center_print = (print.into(), Utc::now().timestamp());
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> + '_ {
        self.lines.iter().map(|(_, line)| line.text.as_ref())
    }

    pub fn center_print(&self, max_time: Duration) -> Option<&str> {
        if self.center_print.1 > (Utc::now() - max_time).timestamp() {
            Some(&*self.center_print.0)
        } else {
            None
        }
    }

    /// Return an iterator over lines that have been printed in the last
    /// `interval` of time.
    ///
    /// The iterator yields the oldest results first.
    ///
    /// `max_candidates` specifies the maximum number of lines to consider,
    /// while `max_results` specifies the maximum number of lines that should
    /// be returned.
    pub fn recent_lines(&self, since: i64) -> impl Iterator<Item = &Line> + '_ {
        self.lines.range(since..).map(|(_, line)| line)
    }
}

#[derive(Component)]
struct AlertOutput {
    since: i64,
}

#[derive(Resource)]
pub struct ConsoleAlertSettings {
    timeout: Duration,
    max_lines: usize,
}

impl Default for ConsoleAlertSettings {
    fn default() -> Self {
        Self {
            timeout: Duration::seconds(3),
            max_lines: 10,
        }
    }
}

pub struct RichterConsolePlugin;

impl Plugin for RichterConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleOutput>()
            .init_resource::<ConsoleInput>()
            .add_systems(Startup, systems::startup::init_alert_output)
            .add_systems(
                PostUpdate,
                (
                    systems::write_to_screen,
                    console_text::systems::update_atlas_text,
                ),
            );
    }
}

// TODO: Extract this so that it can be used elsewhere in the UI
mod console_text {
    use super::*;

    #[derive(Component)]
    pub struct AtlasText {
        pub text: String,
        pub image: Handle<Image>,
        pub layout: Handle<TextureAtlasLayout>,
        pub glyph_size: (Val, Val),
    }

    pub mod systems {
        use super::*;

        pub fn update_atlas_text(
            commands: Commands,
            text: Query<(Entity, &AtlasText), Changed<AtlasText>>,
        ) {
            for (ent, text) in text {
                let mut commands = commands.entity(ent);

                commands.clear_children();

                for (line_id, line) in text.0.lines().enumerate() {
                    let glyph_y = text.glyph_size.1 * line_id as f32;

                    for (char_id, chr) in line.chars().enumerate() {
                        let glyph_x = text.glyph_size.0 * char_id as f32;

                        commands.spawn(AtlasImageBundle {
                            style: Style {
                                width: text.glyph_size.0,
                                height: text.glyph_size.1,
                                left: glyph_x,
                                top: glyph_y,
                                ..default()
                            },
                            ..default()
                        });
                    }
                }
            }
        }
    }
}

mod systems {
    use bevy::ecs::event::ManualEventReader;

    use self::console_text::AtlasText;

    use super::*;

    pub mod startup {
        use super::*;

        pub fn init_alert_output(mut commands: Commands, assets: AssetServer) {
            // TODO: Deduplicate with glyph.rs
            const GLYPH_WIDTH: usize = 8;
            const GLYPH_HEIGHT: usize = 8;
            const GLYPH_COLS: usize = 16;
            const GLYPH_ROWS: usize = 8;
            const GLYPH_COUNT: usize = GLYPH_ROWS * GLYPH_COLS;
            const GLYPH_TEXTURE_WIDTH: usize = GLYPH_WIDTH * GLYPH_COLS;

            let layout = assets.add(TextureAtlasLayout::from_grid(
                Vec2::new(GLYPH_WIDTH, GLYPH_HEIGHT),
                GLYPH_COLS,
                GLYPH_ROWS,
                None,
                None,
            ));
            let image = assets
                .load_with_settings("gfx/conchars.lmp", |s: &mut Option<(u32, u32)>| {
                    *s = Some((128, 128))
                });
            commands.spawn((
                NodeBundle {
                    style: Style {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ..default()
                },
                AtlasText {
                    text: format!("Test!"),
                    image,
                    layout,
                    glyph_size: (Val::Px(GLYPH_WIDTH as _), Val::Px(GLYPH_HEIGHT as _)),
                },
            ));
        }
    }

    pub fn write_to_screen(
        settings: Res<ConsoleAlertSettings>,
        time: Res<Time<Virtual>>,
        console_out: Res<ConsoleOutput>,
        mut alert: Query<(&mut AtlasText, &mut AlertOutput), Changed<AtlasText>>,
    ) {
        for text in alert.iter_mut() {
            let since = time.timestamp() - settings.timeout.timestamp();
            let mut lines = console_out
                .recent_lines(since)
                .filter(|line| line.output_type == OutputType::Alert)
                .take(settings.max_lines)
                .map(|l| l.text);
            let Some(first) = lines.next() else {
                continue;
            };
            text.0.clear();
            text.0.push_str(first);

            for line in lines {
                text.0.push_str(&**line);
                text.0.push('\n');
            }
        }
    }

    pub fn execute_console(reader: Local<ManualEventReader<OutputCmd>>, world: &mut World) {
        let time = world.resource::<Time<Real>>();
        let timestamp = time.timestamp();
        // fn parse_commands<'a, 'b, O, F: FnMut(Vec<&'b str>) -> O + 'a>(
        //     input: &'b str,
        //     mut func: F,
        // ) -> nom::IResult<&'b str, VecDeque<O>> {
        //     let (rest, commands) = parse::commands(input)?;
        //     let out = commands
        //         .into_iter()
        //         .map(|cmd| cmd.into_iter().map(&mut func).collect::<Vec<_>>())
        //         .collect();

        //     Ok((rest, out))
        // }

        // let Ok((_, mut commands)) = parse_commands(&text[..], |c| {
        //     c.split_first().map(|(name, args)| {
        //         RunCmd(
        //             Cow::Borrowed(c),
        //             args.into_iter().map(Cow::borrowed).collect::<Vec<_>>(),
        //         )
        //     })
        // }) else {
        //     // TODO: Stream lines so we don't fail the whole execution if one line fails
        //     error!("Couldn't parse commands");
        // };

        let run_events = mem::take(&mut *world.resource_mut::<Events<OutputCmd>>());
        let mut events_iter = reader.read(&run_events);
        let mut commands = VecDeque::new();

        while let Some(cmd) = commands.pop_front().or_else(|| events_iter.next()) {
            let RunCmd(name, args) = match cmd {
                OutputCmd::EchoInput(input) => {
                    world
                        .resource_mut::<ConsoleOutput>()
                        .println(input, timestamp);
                    continue;
                }
                OutputCmd::Run(cmd) => cmd,
            };

            let mut name = CName::from(name.as_ref());
            loop {
                match world.resource::<Registry>().get_mut(name) {
                    // TODO: Implement helptext
                    Some(CmdInfo { kind, help }) => {
                        match kind {
                            CmdKind::Cvar(cvar) => {
                                match args.split_first() {
                                    None => world.resource_mut::<ConsoleOutput>().println(
                                        format!("\"{}\" is \"{}\"", name, cvar.value()),
                                        timestamp,
                                    ),
                                    Some((new_value, [])) => {
                                        *cvar.value = Value::from_str(new_value)
                                            .unwrap_or_else(|| Value::String(new_value.into()))
                                    }
                                    Some(_) => {
                                        // TODO: Collect into vector
                                        world
                                            .resource_mut::<ConsoleOutput>()
                                            .println("Too many arguments, expected 1", timestamp);
                                    }
                                }
                            }
                            CmdKind::Alias(alias) => {
                                name = alias;
                                continue;
                            }
                            CmdKind::Builtin(cmd) => {
                                match world.run_system_with_input(cmd, args.into()) {
                                    Err(_) => {
                                        error!("Command handler was registered in console but not in world");
                                    }

                                    Ok(Some(ExecResult {
                                        extra_commands,
                                        output,
                                        output_ty,
                                    })) => {
                                        // TODO: Merge this so that commands just return a string again
                                        commands.extend(extra_commands);

                                        match output_ty {
                                            OutputType::Normal => world
                                                .resource_mut::<ConsoleOutput>()
                                                .println(output, timestamp),
                                            OutputType::Alert => world
                                                .resource_mut::<ConsoleOutput>()
                                                .println_alert(output, timestamp),
                                        }
                                    }

                                    Ok(None) => {
                                        world
                                            .resource_mut::<ConsoleOutput>()
                                            .println(format!("usage: {}", help), timestamp);
                                    }
                                }
                            }
                        }
                    }
                    None => {
                        world
                            .resource_mut::<ConsoleOutput>()
                            .println(format!("Unrecognized command \"{}\"", timestamp), timestamp);
                    }
                }
            }
        }
    }
}
