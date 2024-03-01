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
    str::FromStr as _,
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
use serde::{
    de::{value::StrDeserializer, MapAccess},
    Deserializer,
};
use serde_lexpr::Value;
use thiserror::Error;

pub struct RichterConsolePlugin;

impl Plugin for RichterConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleOutput>()
            .init_resource::<ConsoleInput>()
            .init_resource::<Registry>()
            .init_resource::<ConsoleAlertSettings>()
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

type CName = Cow<'static, str>;

#[derive(Error, Debug)]
pub enum ConsoleError {
    #[error("{0}")]
    CmdError(CName),
    #[error("Could not parse cvar: {name} = \"{value}\"")]
    CvarParseFailed { name: CName, value: Value },
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
        Self::CmdError(format!("{}", msg).into())
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
    Builtin(SystemId<Box<[String]>, ExecResult>),
    Alias(CName),
    Cvar(Cvar),
}

#[derive(Clone)]
pub struct CmdInfo {
    pub kind: CmdKind,
    pub help: CName,
}

#[derive(Clone)]
pub struct RunCmd(CName, Box<[String]>);

#[derive(Event, Clone)]
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
        S: IntoSystem<Box<[String]>, ExecResult, M> + 'static,
        I: Into<CName>;

    fn cvar<N, I, C>(&mut self, name: N, value: C, usage: I) -> &mut Self
    where
        N: Into<CName>,
        C: Into<Cvar>,
        I: Into<CName>;
}

impl RegisterCmdExt for App {
    fn command<N, I, S, M>(&mut self, name: N, run: S, usage: I) -> &mut Self
    where
        N: Into<CName>,
        S: IntoSystem<Box<[String]>, ExecResult, M> + 'static,
        I: Into<CName>,
    {
        let sys = self.world.register_system(run);
        self.world
            .resource_mut::<Registry>()
            .command(name, sys, usage);

        self
    }

    fn cvar<N, I, C>(&mut self, name: N, value: C, usage: I) -> &mut Self
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

#[derive(Default, Copy, Clone, PartialEq, Eq)]
pub enum OutputType {
    #[default]
    Console,
    Alert,
}

pub struct ExecResult {
    pub extra_commands: Box<dyn Iterator<Item = RunCmd>>,
    pub output: CName,
    pub output_ty: OutputType,
}

impl Default for ExecResult {
    fn default() -> Self {
        Self {
            extra_commands: Box::new(<[RunCmd; 0]>::into_iter([])),
            output: default(),
            output_ty: default(),
        }
    }
}

impl From<String> for ExecResult {
    fn from(value: String) -> Self {
        Self {
            output: value.into(),
            ..default()
        }
    }
}

impl From<&'static str> for ExecResult {
    fn from(value: &'static str) -> Self {
        Self {
            output: value.into(),
            ..default()
        }
    }
}

impl From<CName> for ExecResult {
    fn from(value: CName) -> Self {
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
    // TODO: Implement a compression pass (e.g. after a removal)
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
                help: help.into(),
            },
        );
    }

    fn insert<N: Into<CName>>(&mut self, name: N, value: CmdInfo) {
        let name = name.into();

        if self.layers.is_empty() {
            self.layers.push(default());
        } else if self.contains(&*name) {
            let new_layer = self.layers.last().cloned().unwrap_or_default();
            self.layers.push(new_layer);
        }

        self.layers.last_mut().unwrap().insert(name, value);
    }

    /// Registers a new command with the given name.
    ///
    /// Returns an error if a command with the specified name already exists.
    fn command<N, H>(&mut self, name: N, cmd: SystemId<Box<[String]>, ExecResult>, help: H)
    where
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
                // TODO: Remove clone
            } else if let im::hashmap::Entry::Occupied(entry) = layer.entry(name.clone()) {
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

    /// Get a command.
    ///
    /// Returns an error if no command with the specified name exists.
    pub fn get<S>(&self, name: S) -> Option<&CmdInfo>
    where
        S: AsRef<str>,
    {
        for layer in self.layers.iter().rev() {
            if let Some(info) = layer.get(name.as_ref()) {
                return Some(info);
            }
        }

        None
    }

    /// Get a command.
    ///
    /// Returns an error if no command with the specified name exists.
    pub fn get_mut<S>(&mut self, name: S) -> Option<&mut CmdInfo>
    where
        S: AsRef<str>,
    {
        for layer in self.layers.iter_mut().rev() {
            if let Some(info) = layer.get_mut(name.as_ref()) {
                return Some(info);
            }
        }

        None
    }

    fn contains<S>(&self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        for layer in self.layers.iter().rev() {
            if layer.contains_key(name.as_ref()) {
                return true;
            }
        }

        false
    }

    // We handle getting cvars differently, as they are needed internally
    fn get_cvar<S: AsRef<str>>(&self, name: S) -> Option<&Cvar> {
        for layer in self.layers.iter().rev() {
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

        struct LexprArrayDeserializer<T, V> {
            values: T,
            cur: Option<V>,
        }

        impl<'a, T>
            LexprArrayDeserializer<
                T,
                (
                    StrDeserializer<'a, ConsoleError>,
                    serde_lexpr::value::de::Deserializer<'a>,
                ),
            >
        where
            T: Iterator<
                Item = (
                    StrDeserializer<'a, ConsoleError>,
                    serde_lexpr::value::de::Deserializer<'a>,
                ),
            >,
        {
            fn new(mut values: T) -> Self {
                let cur = values.next();

                Self { values, cur }
            }
        }

        impl<'a, T> MapAccess<'a>
            for LexprArrayDeserializer<
                T,
                (
                    StrDeserializer<'a, ConsoleError>,
                    serde_lexpr::value::de::Deserializer<'a>,
                ),
            >
        where
            T: Iterator<
                Item = (
                    StrDeserializer<'a, ConsoleError>,
                    serde_lexpr::value::de::Deserializer<'a>,
                ),
            >,
        {
            type Error = ConsoleError;

            fn next_key_seed<K>(&mut self, seed: K) -> Result<Option<K::Value>, Self::Error>
            where
                K: serde::de::DeserializeSeed<'a>,
            {
                let out = match &mut self.cur {
                    Some((k, _)) => Ok(Some(seed.deserialize(*k)?)),
                    None => Ok(None),
                };

                out
            }

            fn next_value_seed<V>(&mut self, seed: V) -> Result<V::Value, Self::Error>
            where
                V: serde::de::DeserializeSeed<'a>,
            {
                match mem::replace(&mut self.cur, self.values.next()) {
                    Some((_, mut v)) => Ok(seed
                        .deserialize(&mut v)
                        .map_err(|_| ConsoleError::CvarParseInvalid)?),
                    None => Err(ConsoleError::CvarParseInvalid),
                }
            }
        }

        impl<'a> Deserializer<'a> for CvarDeserializer<'a> {
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
                let de = LexprArrayDeserializer::new(fields.into_iter().filter_map(|name| {
                    self.inner.get_cvar(name).map(|c| {
                        (
                            StrDeserializer::new(*name),
                            serde_lexpr::value::de::Deserializer::from_value(c.value()),
                        )
                    })
                }));

                visitor.visit_map(de)
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

        V::deserialize(CvarDeserializer { inner: self }).ok()
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
#[derive(Debug, Clone)]
pub struct Cvar {
    // Value of this variable
    pub value: Option<Value>,

    // If true, this variable should be archived in vars.rc
    pub archive: bool,

    // If true:
    // - If a server cvar, broadcast updates to clients
    // - If a client cvar, update userinfo
    pub notify: bool,

    // The default value of this variable
    pub default: Value,
}

impl Default for Cvar {
    fn default() -> Self {
        Self {
            value: default(),
            archive: default(),
            notify: default(),
            default: Value::Nil,
        }
    }
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
            default: Value::from_str(default.into().as_ref()).unwrap(),
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
        self.value.as_ref().unwrap_or(&self.default)
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
        if let Some(line) = self.hist.line_up().map(ToOwned::to_owned) {
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
            hist: default(),
            commands: default(),
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

pub struct Line {
    output_type: OutputType,
    text: CName,
}

#[derive(Resource, Default)]
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
        S: Into<CName>,
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

    pub fn set_center_print<S: Into<String>>(&mut self, print: S) {
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
    pub fn recent_lines(&self, since: i64) -> impl Iterator<Item = (i64, &Line)> + '_ {
        self.lines.range(since..).map(|(k, v)| (*k, v))
    }
}

#[derive(Component)]
struct AlertOutput {
    last_timestamp: Option<i64>,
}

impl Default for AlertOutput {
    fn default() -> Self {
        Self {
            last_timestamp: Some(0),
        }
    }
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

// TODO: Extract this so that it can be used elsewhere in the UI
mod console_text {
    use super::*;

    #[derive(Component, Debug)]
    pub struct AtlasText {
        pub text: String,
        pub image: UiImage,
        pub layout: Handle<TextureAtlasLayout>,
        pub glyph_size: (Val, Val),
    }

    pub mod systems {
        use super::*;

        pub fn update_atlas_text(
            mut commands: Commands,
            text: Query<(Entity, &AtlasText), Changed<AtlasText>>,
            asset_server: Res<AssetServer>,
        ) {
            for (ent, text) in text.iter() {
                let mut commands = commands.entity(ent);

                commands.clear_children();

                commands.with_children(|commands| {
                    for (line_id, line) in text.text.lines().enumerate() {
                        let glyph_y = text.glyph_size.1 * line_id as f32;

                        for (char_id, chr) in line.chars().enumerate() {
                            let glyph_x = text.glyph_size.0 * char_id as f32;

                            commands.spawn(
                                AtlasImageBundle {
                                    image: text.image.clone(),
                                    texture_atlas: TextureAtlas { layout: text.layout.clone(), index: chr as _ } ,
                                    style: Style {
                                    position_type: PositionType::Absolute,
                                        width: text.glyph_size.0,
                                        height: text.glyph_size.1,
                                        left: glyph_x,
                                        top: glyph_y,
                                        ..default()
                                    },
                                    ..default()
                                },
                            );
                        }
                    }
                });
            }
        }
    }
}

mod systems {
    use chrono::TimeDelta;

    use self::console_text::AtlasText;

    use super::*;

    pub mod startup {
        use bevy::render::render_asset::RenderAssetUsages;
        use wgpu::{Extent3d, TextureDimension};

        use crate::{
            client::render::{Palette, TextureData},
            common::{vfs::Vfs, wad::Wad},
        };

        use super::*;

        pub fn init_alert_output(mut commands: Commands, vfs: Res<Vfs>, assets: Res<AssetServer>) {
            commands.spawn(Camera2dBundle {
                camera: Camera {
                    order: 1,
                    ..default()
                },
                ..default()
            });

            let palette = Palette::load(&vfs, "gfx/palette.lmp");
            let gfx_wad = Wad::load(vfs.open("gfx.wad").unwrap()).unwrap();

            let conchars = gfx_wad.open_conchars().unwrap();

            // TODO: validate conchars dimensions

            let indices = conchars
                .indices()
                .iter()
                .map(|i| if *i == 0 { 0xFF } else { *i })
                .collect::<Vec<_>>();

            // reorder indices from atlas order to array order
            let mut array_order = Vec::new();
            for glyph_id in 0..GLYPH_COUNT {
                for glyph_r in 0..GLYPH_HEIGHT {
                    for glyph_c in 0..GLYPH_WIDTH {
                        let atlas_r = GLYPH_HEIGHT * (glyph_id / GLYPH_COLS) + glyph_r;
                        let atlas_c = GLYPH_WIDTH * (glyph_id % GLYPH_COLS) + glyph_c;
                        array_order.push(indices[atlas_r * GLYPH_TEXTURE_WIDTH + atlas_c]);
                    }
                }
            }
            let (diffuse_data, _) = palette.translate(&indices);
            let diffuse_data = TextureData::Diffuse(diffuse_data);

            // TODO: Deduplicate with glyph.rs
            const GLYPH_WIDTH: usize = 8;
            const GLYPH_HEIGHT: usize = 8;
            const GLYPH_COLS: usize = 16;
            const GLYPH_ROWS: usize = 8;
            const GLYPH_COUNT: usize = GLYPH_ROWS * GLYPH_COLS;
            const GLYPH_TEXTURE_WIDTH: usize = GLYPH_WIDTH * GLYPH_COLS;

            let layout = assets.add(TextureAtlasLayout::from_grid(
                Vec2::new(GLYPH_WIDTH as _, GLYPH_HEIGHT as _),
                GLYPH_COLS,
                GLYPH_COLS,
                None,
                None,
            ));

            let image = Image::new(
                Extent3d {
                    width: GLYPH_TEXTURE_WIDTH as _,
                    height: GLYPH_TEXTURE_WIDTH as _,
                    depth_or_array_layers: 1,
                },
                TextureDimension::D2,
                diffuse_data.data().to_owned(),
                diffuse_data.format(),
                RenderAssetUsages::RENDER_WORLD,
            );
            let image = assets.add(image);
            let image = image.into();

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
                    glyph_size: (Val::Percent(GLYPH_WIDTH as _), Val::Percent(GLYPH_HEIGHT as _)),
                },
                AlertOutput::default(),
            ));
        }
    }

    pub fn write_to_screen(
        settings: Res<ConsoleAlertSettings>,
        time: Res<Time<Virtual>>,
        console_out: Res<ConsoleOutput>,
        mut alert: Query<(&mut AtlasText, &mut AlertOutput)>,
    ) {
        // TODO
        return;
        for (mut text, alert_out) in alert.iter_mut() {
            let since = (TimeDelta::from_std(time.elapsed()).unwrap() - settings.timeout)
                .num_milliseconds();
            let mut lines = console_out
                .recent_lines(since)
                .filter(|(_, line)| line.output_type == OutputType::Alert)
                .map(|(ts, line)| (ts, &line.text))
                .take(settings.max_lines);

            let first = lines.next();
            let last_timestamp = first.map(|(ts, _)| ts);
            if last_timestamp == alert_out.last_timestamp {
                continue;
            }

            alert_out.last_timestamp;

            text.text.clear();

            let Some((_, first)) = first else {
                continue;
            };
            text.text.push_str(first.as_ref());

            for (_, line) in lines {
                text.text.push_str(&*line);
                text.text.push('\n');
            }
        }
    }

    pub fn execute_console(world: &mut World) {
        let time = world.resource::<Time<Real>>();
        let timestamp = TimeDelta::from_std(time.elapsed())
            .unwrap()
            .num_milliseconds();
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

        let mut commands = world
            .resource_mut::<Events<OutputCmd>>()
            .drain()
            .collect::<VecDeque<_>>();

        while let Some(cmd) = commands.pop_front() {
            let RunCmd(name, args) = match cmd {
                OutputCmd::EchoInput(input) => {
                    world
                        .resource_mut::<ConsoleOutput>()
                        .println(input.clone(), timestamp);
                    continue;
                }
                OutputCmd::Run(cmd) => cmd,
            };

            let mut name = CName::from(name.into_owned());
            loop {
                let (output, output_ty) = match world.resource_mut::<Registry>().get_mut(&*name) {
                    // TODO: Implement helptext
                    Some(CmdInfo { kind, help }) => {
                        match kind {
                            CmdKind::Cvar(cvar) => {
                                match args.split_first() {
                                    None => (
                                        Cow::from(format!("\"{}\" is \"{}\"", name, cvar.value())),
                                        OutputType::Console,
                                    ),
                                    Some((new_value, [])) => {
                                        cvar.value =
                                            Some(Value::from_str(new_value).unwrap_or_else(|_| {
                                                Value::String(new_value.clone().into())
                                            }));
                                        continue;
                                    }
                                    Some(_) => {
                                        // TODO: Collect into vector

                                        (
                                            Cow::from("Too many arguments, expected 1"),
                                            OutputType::Console,
                                        )
                                    }
                                }
                            }
                            CmdKind::Alias(alias) => {
                                name = alias.clone();
                                continue;
                            }
                            CmdKind::Builtin(cmd) => {
                                let args = args.clone();
                                let cmd = *cmd;

                                match world.run_system_with_input(cmd, args) {
                                    Err(_) => {
                                        error!("Command handler was registered in console but not in world");
                                        continue;
                                    }

                                    Ok(ExecResult {
                                        extra_commands,
                                        output,
                                        output_ty,
                                    }) => {
                                        // TODO: Merge this so that commands just return a string again
                                        commands.extend(extra_commands.map(OutputCmd::Run));

                                        (output, output_ty)
                                    }
                                }
                            }
                        }
                    }
                    None => (
                        Cow::from(format!("Unrecognized command \"{}\"", &*name)),
                        OutputType::Console,
                    ),
                };
                match output_ty {
                    OutputType::Console => world
                        .resource_mut::<ConsoleOutput>()
                        .println(output, timestamp),
                    OutputType::Alert => world
                        .resource_mut::<ConsoleOutput>()
                        .println_alert(output, timestamp),
                }
            }
        }
    }
}
