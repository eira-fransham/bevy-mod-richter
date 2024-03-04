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
    collections::{hash_map::Entry, BTreeMap, BTreeSet},
    fmt, io, mem,
    str::FromStr,
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
use fxhash::FxHashMap;
use liner::{
    BasicCompleter, Editor, EditorContext, Emacs, Key, KeyBindings, KeyMap as _, Prompt, Tty,
};
use serde::{
    de::{value::StrDeserializer, MapAccess},
    Deserializer,
};
use serde_lexpr::Value;
use thiserror::Error;

use crate::client::input::game::Trigger;

use super::parse;

pub struct RichterConsolePlugin;

impl Plugin for RichterConsolePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ConsoleOutput>()
            .init_resource::<ConsoleInput>()
            .init_resource::<Registry>()
            .init_resource::<ConsoleAlertSettings>()
            .add_event::<RunCmd<'static>>()
            .add_systems(Startup, systems::startup::init_alert_output)
            .add_systems(
                PostUpdate,
                (
                    systems::write_to_screen,
                    systems::execute_console,
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
    #[error("No such alias: {0}")]
    NoSuchAlias(CName),
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
    Action {
        system: Option<SystemId<(Trigger, Box<[String]>), ()>>,
        state: Trigger,
        // TODO: Mark when the last state update was, so we know how long a key has been pressed
    },
    // TODO: Allow `Alias` to invoke an arbitrary sequence of commands
    Alias(CName),
    Cvar(Cvar),
}

#[derive(Clone)]
pub struct CommandImpl {
    pub kind: CmdKind,
    pub help: CName,
}

pub struct AliasInfo<'a> {
    pub name: &'a str,
    pub target: &'a str,
    pub help: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmdName<'a> {
    pub trigger: Option<Trigger>,
    pub name: Cow<'a, str>,
}

impl CmdName<'_> {
    pub fn into_owned(self) -> CmdName<'static> {
        let CmdName { trigger, name } = self;

        CmdName {
            name: name.into_owned().into(),
            trigger,
        }
    }
}

impl FromStr for CmdName<'static> {
    type Err = nom::Err<nom::error::Error<String>>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match parse::command_name(s) {
            Ok(("", val)) => Ok(val.into_owned().into()),
            Ok((rest, _)) => Err(nom::Err::Failure(nom::error::Error::new(
                rest.to_owned(),
                nom::error::ErrorKind::Verify,
            ))),
            Err(e) => Err(e.to_owned()),
        }
    }
}

impl From<&'static str> for CmdName<'static> {
    fn from(s: &'static str) -> Self {
        Self {
            trigger: None,
            name: s.into(),
        }
    }
}

impl From<String> for CmdName<'static> {
    fn from(s: String) -> Self {
        Self {
            trigger: None,
            name: s.into(),
        }
    }
}

impl std::fmt::Display for CmdName<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(trigger) = &self.trigger {
            write!(f, "{}{}", trigger, self.name)
        } else {
            write!(f, "{}", self.name)
        }
    }
}

#[derive(Event, PartialEq, Eq, Clone, Debug)]
pub struct RunCmd<'a>(pub CmdName<'a>, pub Box<[String]>);

impl<'a> RunCmd<'a> {
    pub fn into_owned(self) -> RunCmd<'static> {
        let RunCmd(name, args) = self;
        RunCmd(name.into_owned(), args)
    }

    pub fn parse(s: &'a str) -> Result<Self, <RunCmd<'static> as FromStr>::Err> {
        match parse::command(s) {
            Ok(("", val)) => Ok(val),
            Ok((rest, _)) => Err(nom::Err::Failure(nom::error::Error::new(
                rest.to_owned(),
                nom::error::ErrorKind::Verify,
            ))),
            Err(e) => Err(e.to_owned()),
        }
    }

    pub fn parse_many(s: &'a str) -> Result<Vec<Self>, nom::Err<nom::error::Error<&str>>> {
        parse::commands(s).map(|(_, cmds)| cmds)
    }

    pub fn invert(self) -> Option<Self> {
        self.0.trigger.map(|t| {
            RunCmd(
                CmdName {
                    trigger: Some(!t),
                    name: self.0.name,
                },
                self.1,
            )
        })
    }
}

impl std::fmt::Display for RunCmd<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", &self.0)?;

        for arg in self.1.iter() {
            // TODO: This doesn't work if the value is a string that requires quotes - use `lexpr::Value`?
            write!(f, " {:?}", arg)?;
        }

        Ok(())
    }
}

impl FromStr for RunCmd<'static> {
    type Err = nom::Err<nom::error::Error<String>>;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        RunCmd::parse(s).map(RunCmd::into_owned)
    }
}

impl From<&'static str> for RunCmd<'static> {
    fn from(s: &'static str) -> Self {
        Self(s.into(), default())
    }
}

impl From<String> for RunCmd<'static> {
    fn from(s: String) -> Self {
        Self(s.into(), default())
    }
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

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq)]
pub enum OutputType {
    #[default]
    Console,
    Alert,
}

pub struct ExecResult {
    pub extra_commands: Box<dyn Iterator<Item = RunCmd<'static>>>,
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
    commands: FxHashMap<CName, (CommandImpl, Vec<CommandImpl>)>,
    names: BTreeSet<CName>,
}

impl Registry {
    fn new() -> Registry {
        Self::default()
    }

    pub fn alias<S, C>(&mut self, name: S, command: C)
    where
        S: Into<CName>,
        C: Into<CName>,
    {
        self.insert(
            name.into(),
            CommandImpl {
                kind: CmdKind::Alias(command.into()),
                // TODO: Implement help text for aliases?
                help: "".into(),
            },
        );
    }

    pub fn aliases(&self) -> impl Iterator<Item = AliasInfo<'_>> + '_ {
        self.all_names().filter_map(move |name| {
            let cmd = self.get(name).expect("Name in `names` but not in map");

            match &cmd.kind {
                CmdKind::Alias(target) => Some(AliasInfo {
                    name,
                    target: &**target,
                    help: &*cmd.help,
                }),
                _ => None,
            }
        })
    }

    fn cvar<S, C, H>(&mut self, name: S, cvar: C, help: H)
    where
        S: Into<CName>,
        C: Into<Cvar>,
        H: Into<CName>,
    {
        self.insert(
            name.into(),
            CommandImpl {
                kind: CmdKind::Cvar(cvar.into()),
                help: help.into(),
            },
        );
    }

    fn insert<N: Into<CName>>(&mut self, name: N, value: CommandImpl) {
        let name = name.into();

        match self.commands.entry(name) {
            Entry::Occupied(mut commands) => commands.get_mut().1.push(value),
            Entry::Vacant(entry) => {
                entry.insert((value, vec![]));
            }
        }
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
            CommandImpl {
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
        S: AsRef<str>,
    {
        let name = name.as_ref();
        // TODO: Use `HashMap::extract_if` when stabilised
        match self.commands.get_mut(name) {
            Some((cmd, overlays)) => {
                if overlays.pop().is_none() {
                    self.commands.remove(name);
                }

                Ok(())
            }
            None => Err(ConsoleError::NoSuchCommand(name.to_owned().into())),
        }
    }

    /// Removes the command with the given name.
    ///
    /// Returns an error if there was no command with that name.
    fn remove_alias<S>(&mut self, name: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        // TODO: Use `HashMap::extract_if` when stabilised
        match self.commands.get_mut(name) {
            Some((cmd, overlays)) => {
                let CommandImpl {
                    kind: CmdKind::Alias(_),
                    ..
                } = overlays.last().unwrap_or(cmd)
                else {
                    return Err(ConsoleError::NoSuchAlias(name.to_owned().into()));
                };
                if overlays.pop().is_none() {
                    self.commands.remove(name);
                }

                Ok(())
            }
            None => Err(ConsoleError::NoSuchAlias(name.to_owned().into())),
        }
    }

    /// Get a command.
    ///
    /// Returns an error if no command with the specified name exists.
    pub fn get<S>(&self, name: S) -> Option<&CommandImpl>
    where
        S: AsRef<str>,
    {
        self.commands
            .get(name.as_ref())
            .map(|(first, rest)| rest.last().unwrap_or(first))
    }

    /// Get a command.
    ///
    /// Returns an error if no command with the specified name exists.
    pub fn get_mut<S>(&mut self, name: S) -> Option<&mut CommandImpl>
    where
        S: AsRef<str>,
    {
        self.commands
            .get_mut(name.as_ref())
            .map(|(first, rest)| rest.last_mut().unwrap_or(first))
    }

    fn contains<S>(&self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        self.commands.contains_key(name.as_ref())
    }

    fn get_cvar<S: AsRef<str>>(&self, name: S) -> Option<&Cvar> {
        self.get(name).and_then(|info| match &info.kind {
            CmdKind::Cvar(cvar) => Some(cvar),
            _ => None,
        })
    }

    fn get_cvar_mut<S: AsRef<str>>(&mut self, name: S) -> Option<&mut Cvar> {
        self.get_mut(name).and_then(|info| match &mut info.kind {
            CmdKind::Cvar(cvar) => Some(cvar),
            _ => None,
        })
    }

    pub fn is_pressed<S: AsRef<str>>(&self, name: S) -> bool {
        self.get(name).and_then(|info| match &info.kind {
            CmdKind::Action { state, .. } => Some(*state),
            _ => None,
        }) == Some(Trigger::Positive)
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
            self.get(name)
                .and_then(|CommandImpl { kind, .. }| match kind {
                    CmdKind::Builtin(_) => Some(name),
                    _ => None,
                })
        })
    }

    pub fn alias_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.all_names().filter_map(move |name| {
            self.get(name)
                .and_then(|CommandImpl { kind, .. }| match kind {
                    CmdKind::Alias(_) => Some(name),
                    _ => None,
                })
        })
    }

    pub fn cvar_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.all_names().filter_map(move |name| {
            self.get(name)
                .and_then(|CommandImpl { kind, .. }| match kind {
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
#[derive(Default)]
struct ConsoleInputContext {
    input_buf: String,
    history: liner::History,
    key_bindings: KeyBindings,
    commands: Vec<RunCmd<'static>>,
    terminal: ConsoleInputTerminal,
    cmd_buf: String,
}

#[derive(Default)]
struct ConsoleInputTerminal {
    stdout: Vec<u8>,
}

impl io::Write for ConsoleInputTerminal {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.stdout.write(buf)
    }

    fn write_vectored(&mut self, bufs: &[io::IoSlice<'_>]) -> io::Result<usize> {
        self.stdout.write_vectored(bufs)
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        self.stdout.write_all(buf)
    }

    fn write_fmt(&mut self, fmt: fmt::Arguments<'_>) -> io::Result<()> {
        self.stdout.write_fmt(fmt)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }
}

impl Tty for ConsoleInputTerminal {
    fn next_key(&mut self) -> Option<std::io::Result<liner::Key>> {
        unreachable!("TODO: Remove `next_key` from `liner::Tty`")
    }

    fn width(&self) -> std::io::Result<usize> {
        Ok(80) // TODO: Make this actually read the console width
    }
}

impl fmt::Write for ConsoleInputContext {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.cmd_buf.push_str(s);

        Ok(())
    }
}

impl EditorContext for ConsoleInputContext {
    type Terminal = ConsoleInputTerminal;
    type WordDividerIter = <liner::Context as EditorContext>::WordDividerIter;

    fn history(&self) -> &liner::History {
        &self.history
    }

    fn history_mut(&mut self) -> &mut liner::History {
        &mut self.history
    }

    fn word_divider(&self, buf: &liner::Buffer) -> Self::WordDividerIter {
        liner::get_buffer_words(buf).into_iter()
    }

    fn terminal(&self) -> &Self::Terminal {
        &self.terminal
    }

    fn terminal_mut(&mut self) -> &mut Self::Terminal {
        &mut self.terminal
    }

    fn key_bindings(&self) -> liner::KeyBindings {
        self.key_bindings
    }
}

#[derive(Resource)]
pub struct ConsoleInput {
    editor: Editor<ConsoleInputContext>,
    keymap: Emacs,
}

impl Default for ConsoleInput {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

impl ConsoleInput {
    /// Constructs a new `ConsoleInput`.
    ///
    /// Initializes the text content to be empty and places the cursor at position 0.
    pub fn new() -> io::Result<ConsoleInput> {
        let mut keymap = Emacs::new();

        let mut editor = Editor::new(
            Prompt::from("] ".to_owned()),
            None,
            ConsoleInputContext::default(),
        )
        .unwrap();
        // TODO: Error handling
        keymap.init(&mut editor)?;

        Ok(ConsoleInput { editor, keymap })
    }

    /// Send characters to the inner editor
    pub fn update<I: Iterator<Item = Key>>(&mut self, keys: I) -> io::Result<Option<String>> {
        let mut keymap = Emacs::new();

        for key in keys {
            // TODO: Completion
            if keymap.handle_key(
                key,
                &mut self.editor,
                &mut BasicCompleter::new(Vec::<String>::new()),
            )? {
                return Ok(Some(self.editor.take_exec_buffer()));
            }
        }

        return Ok(None);
    }

    /// Returns the text currently being edited
    pub fn get_text(&self) -> impl Iterator<Item = char> + '_ {
        self.editor.current_buffer().chars().copied()
    }
}

pub struct ConsoleText {
    pub output_type: OutputType,
    pub text: CName,
}

#[derive(Resource, Default)]
pub struct ConsoleOutput {
    // A ring buffer of lines of text. Each line has an optional timestamp used
    // to determine whether it should be displayed on screen. If the timestamp
    // is `None`, the message will not be displayed.
    //
    // The timestamp is specified in seconds since the Unix epoch (so it is
    // decoupled from client/server time).
    text_chunks: BTreeMap<(i64, u16), ConsoleText>,
    generation: u16,
    center_print: (String, i64),
    buffer_ty: OutputType,
    buffer: String,
    last_timestamp: i64,
}

impl ConsoleOutput {
    pub fn print<S: AsRef<str>>(&mut self, s: S, timestamp: Duration) {
        self.push(s, timestamp.num_milliseconds(), OutputType::Console);
    }

    pub fn print_alert<S: AsRef<str>>(&mut self, s: S, timestamp: Duration) {
        self.push(s, timestamp.num_milliseconds(), OutputType::Alert);
    }

    pub fn println<S: AsRef<str>>(&mut self, s: S, timestamp: Duration) {
        self.push_line(s, timestamp.num_milliseconds(), OutputType::Console);
    }

    pub fn println_alert<S: AsRef<str>>(&mut self, s: S, timestamp: Duration) {
        self.push_line(s, timestamp.num_milliseconds(), OutputType::Alert);
    }

    pub fn new() -> ConsoleOutput {
        ConsoleOutput::default()
    }

    fn push<S: AsRef<str>>(&mut self, chars: S, timestamp: i64, ty: OutputType) {
        let chars = chars.as_ref();

        if chars.is_empty() {
            return;
        }

        self.last_timestamp = timestamp;

        // TODO: set maximum capacity and pop_back when we reach it
        let generation = self.generation;
        if ty != self.buffer_ty {
            self.flush();
        }

        self.buffer_ty = ty;
        self.buffer.push_str(chars);

        self.try_flush();
    }

    fn try_flush(&mut self) {
        if let Some(last_newline) = self.buffer.rfind('\n') {
            let (to_flush, rest) = self.buffer.split_at(last_newline);
            let new_buf = rest
                .char_indices()
                .nth(1)
                .and_then(|(i, _)| rest.get(i..))
                .unwrap_or_default()
                .to_owned();
            self.buffer.truncate(to_flush.len());
            self.generation = self.generation.wrapping_add(1);
            self.text_chunks.insert(
                (self.last_timestamp, self.generation),
                ConsoleText {
                    text: mem::replace(&mut self.buffer, new_buf).into(),
                    output_type: self.buffer_ty,
                },
            );
            self.generation = self.generation.wrapping_add(1);
            self.text_chunks.insert(
                (self.last_timestamp, self.generation.wrapping_add(1)),
                ConsoleText {
                    text: "\n".into(),
                    output_type: self.buffer_ty,
                },
            );
        }
    }

    fn flush(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.text_chunks.insert(
            (self.last_timestamp, self.generation),
            ConsoleText {
                text: mem::take(&mut self.buffer).into(),
                output_type: self.buffer_ty,
            },
        );
    }

    fn push_line<S: AsRef<str>>(&mut self, chars: S, timestamp: i64, ty: OutputType) {
        self.push(chars, timestamp, ty);
        self.push("\n", timestamp, ty);
    }

    pub fn set_center_print<S: Into<String>>(&mut self, print: S) {
        self.center_print = (print.into(), Utc::now().timestamp());
    }

    pub fn text(&self) -> impl Iterator<Item = (i64, &ConsoleText)> + '_ {
        self.text_chunks.iter().map(|((k, _), v)| (*k, v))
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
    pub fn recent(&self, since: i64) -> impl Iterator<Item = (i64, &ConsoleText)> + '_ {
        self.text_chunks
            .range((since, 0)..)
            .map(|((k, _), v)| (*k, v))
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
            for (entity, text) in text.iter() {
                commands.add(DespawnChildrenRecursive { entity });

                let mut commands = commands.entity(entity);

                commands.with_children(|commands| {
                    for (line_id, line) in text.text.lines().enumerate() {
                        let glyph_y = text.glyph_size.1 * line_id as f32;

                        for (char_id, chr) in line.chars().enumerate() {
                            let glyph_x = text.glyph_size.0 * char_id as f32;

                            commands.spawn(AtlasImageBundle {
                                image: text.image.clone(),
                                texture_atlas: TextureAtlas {
                                    layout: text.layout.clone(),
                                    index: chr as _,
                                },
                                style: Style {
                                    position_type: PositionType::Absolute,
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
                });
            }
        }
    }
}

mod systems {
    use std::collections::VecDeque;

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
            let palette = Palette::load(&vfs, "gfx/palette.lmp");
            let gfx_wad = Wad::load(vfs.open("gfx.wad").unwrap()).unwrap();

            let conchars = gfx_wad.open_conchars().unwrap();

            // TODO: validate conchars dimensions

            let indices = conchars
                .indices()
                .iter()
                .map(|i| if *i == 0 { 0xFF } else { *i })
                .collect::<Vec<_>>();

            let (diffuse_data, _) = palette.translate(&indices);
            let diffuse_data = TextureData::Diffuse(diffuse_data);

            // TODO: Deduplicate with glyph.rs
            const GLYPH_WIDTH: usize = 8;
            const GLYPH_HEIGHT: usize = 8;
            const GLYPH_COLS: usize = 16;
            const GLYPH_ROWS: usize = 8;
            const GLYPH_COUNT: usize = GLYPH_ROWS * GLYPH_COLS;
            const GLYPH_TEXTURE_WIDTH: usize = GLYPH_WIDTH * GLYPH_COLS;
            const SCALE: f32 = 3.;

            let layout = assets.add(TextureAtlasLayout::from_grid(
                Vec2::new(GLYPH_WIDTH as _, GLYPH_HEIGHT as _),
                GLYPH_COLS,
                GLYPH_COLS,
                None,
                None,
            ));

            let image = assets
                .add(Image::new(
                    Extent3d {
                        width: GLYPH_TEXTURE_WIDTH as _,
                        height: GLYPH_TEXTURE_WIDTH as _,
                        depth_or_array_layers: 1,
                    },
                    TextureDimension::D2,
                    diffuse_data.data().to_owned(),
                    diffuse_data.format(),
                    RenderAssetUsages::RENDER_WORLD,
                ))
                .into();

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
                    text: "".into(),
                    image,
                    layout,
                    glyph_size: (
                        Val::Px(GLYPH_WIDTH as _) * SCALE,
                        Val::Px(GLYPH_HEIGHT as _) * SCALE,
                    ),
                },
                AlertOutput::default(),
            ));
        }
    }

    pub fn read_console(console_in: ResMut<ConsoleInput>, run_cmds: EventWriter<RunCmd<'static>>) {}

    pub fn write_to_screen(
        settings: Res<ConsoleAlertSettings>,
        time: Res<Time<Virtual>>,
        console_out: Res<ConsoleOutput>,
        mut alert: Query<(&mut AtlasText, &mut AlertOutput)>,
    ) {
        // TODO
        for (mut text, mut alert_out) in alert.iter_mut() {
            let since = (TimeDelta::from_std(time.elapsed()).unwrap() - settings.timeout)
                .num_milliseconds();
            let mut lines = console_out
                .recent(since)
                .filter(|(_, line)| line.output_type == OutputType::Alert)
                .map(|(ts, line)| (ts, &line.text))
                .take(settings.max_lines);

            let first = lines.next();
            let last_timestamp = first.map(|(ts, _)| ts);

            if last_timestamp == alert_out.last_timestamp {
                continue;
            }

            alert_out.last_timestamp = last_timestamp;

            text.text.clear();

            let Some((_, first)) = first else {
                continue;
            };
            text.text.push_str(first.as_ref());

            for (_, line) in lines {
                text.text.push_str(&*line);
            }
        }
    }

    pub fn execute_console(world: &mut World) {
        let time = world.resource::<Time<Real>>();
        let timestamp = TimeDelta::from_std(time.elapsed()).unwrap();

        let mut commands = world
            .resource_mut::<Events<RunCmd>>()
            .drain()
            .collect::<VecDeque<_>>();

        while let Some(RunCmd(CmdName { name, trigger }, args)) = commands.pop_front() {
            let mut name = Cow::from(name);
            loop {
                let (output, output_ty) = match world.resource_mut::<Registry>().get_mut(&*name) {
                    // TODO: Implement helptext
                    Some(CommandImpl { kind, help }) => {
                        match (trigger, kind) {
                            (None, CmdKind::Cvar(cvar)) => {
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
                                        break;
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
                            (Some(_), CmdKind::Cvar(_)) => (
                                Cow::from(format!("{} is a cvar", name)),
                                OutputType::Console,
                            ),
                            // Currently this allows action aliases - do we want that?
                            (_, CmdKind::Alias(alias)) => {
                                name = alias.clone();
                                continue;
                            }
                            (None, CmdKind::Builtin(cmd)) => {
                                let args = args.clone();
                                let cmd = *cmd;

                                match world.run_system_with_input(cmd, args) {
                                    Err(_) => {
                                        error!("Command handler was registered in console but not in world");
                                        break;
                                    }

                                    Ok(ExecResult {
                                        extra_commands,
                                        output,
                                        output_ty,
                                    }) => {
                                        commands.extend(extra_commands);

                                        (output, output_ty)
                                    }
                                }
                            }
                            (Some(_), CmdKind::Builtin(cmd)) => (
                                Cow::from(format!(
                                    "{} is a command, and cannot be invoked with +/-",
                                    name
                                )),
                                OutputType::Console,
                            ),
                            (Some(trigger), CmdKind::Action { system, state }) => {
                                if *state == trigger {
                                    break;
                                }

                                let args = args.clone();
                                *state = trigger;

                                let Some(cmd) = system else {
                                    // No invocation handler, just mark the pressed/released state
                                    break;
                                };

                                let cmd = *cmd;

                                match world.run_system_with_input(cmd, (trigger, args)) {
                                    Err(_) => {
                                        error!("Command handler was registered in console but not in world");
                                        break;
                                    }

                                    Ok(()) => break,
                                }
                            }
                            (None, CmdKind::Action { .. }) => (
                                Cow::from(format!(
                                    "{} is an action, and must be invoked with +/-",
                                    name
                                )),
                                OutputType::Console,
                            ),
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
                        .print(output, timestamp),
                    OutputType::Alert => world
                        .resource_mut::<ConsoleOutput>()
                        .print_alert(output, timestamp),
                }

                break;
            }
        }
    }
}
