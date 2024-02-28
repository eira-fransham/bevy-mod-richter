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

use std::{borrow::Cow, fmt::Write, mem, sync::Arc};

use crate::common::parse;

use bevy::{
    ecs::{
        system::{Commands, Resource},
        world::{FromWorld, World},
    },
    render::extract_resource::ExtractResource,
};
use chrono::{Duration, Utc};
use imstr::ImString;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConsoleError {
    #[error("{0}")]
    CmdError(String),
    #[error("Could not parse cvar as a number: {name} = \"{value}\"")]
    CvarParseFailed { name: String, value: String },
    #[error("A command named \"{0}\" already exists")]
    DuplicateCommand(String),
    #[error("A cvar named \"{0}\" already exists")]
    DuplicateCvar(String),
    #[error("No such command: {0}")]
    NoSuchCommand(String),
    #[error("No such cvar: {0}")]
    NoSuchCvar(String),
}

type Cmd = Arc<dyn Fn(&[&str], &mut World) -> ExecResult + Send + Sync>;

pub struct CommandArgs<'a, 'w, 's> {
    pub world: &'a mut World,
    pub commands: &'a mut Commands<'w, 's>,
}

impl<'a, 'w, 's> CommandArgs<'a, 'w, 's> {
    pub fn new(world: &'a mut World, commands: &'a mut Commands<'w, 's>) -> Self {
        Self { world, commands }
    }
}

fn insert_name<S>(names: &mut im::Vector<String>, name: S) -> Result<usize, usize>
where
    S: AsRef<str>,
{
    let name = name.as_ref();
    match names.binary_search_by(|item| item.as_str().cmp(name)) {
        Ok(i) => Err(i),
        Err(i) => {
            names.insert(i, name.to_owned());
            Ok(i)
        }
    }
}

/// Stores console commands.
#[derive(Resource, ExtractResource, Clone, Default)]
pub struct CmdRegistry {
    cmds: im::HashMap<String, Cmd>,
    names: im::Vector<String>,
}

pub struct ExecResult {
    pub extra_commands: String,
    pub output: String,
}

impl From<String> for ExecResult {
    fn from(value: String) -> Self {
        Self {
            extra_commands: String::new(),
            output: value,
        }
    }
}

impl CmdRegistry {
    pub fn new() -> CmdRegistry {
        Self::default()
    }

    /// Registers a new command with the given name.
    ///
    /// Returns an error if a command with the specified name already exists.
    pub fn insert<S, C, CO>(&mut self, name: S, cmd: C) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
        C: Fn(&[&str], &mut World) -> CO + Send + Sync + 'static,
        CO: Into<ExecResult>,
    {
        let name = name.as_ref();

        match self.cmds.get(name) {
            Some(_) => Err(ConsoleError::DuplicateCommand(name.to_owned()))?,
            None => {
                if insert_name(&mut self.names, name).is_err() {
                    return Err(ConsoleError::DuplicateCvar(name.into()));
                }

                self.cmds.insert(
                    name.to_owned(),
                    Arc::new(move |args, input| cmd(args, input).into()),
                );
            }
        }

        Ok(())
    }

    /// Registers a new command with the given name, or replaces one if the name is in use.
    pub fn insert_or_replace<S, C, CO>(&mut self, name: S, cmd: C) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
        C: Fn(&[&str], &mut World) -> CO + Send + Sync + 'static,
        CO: Into<ExecResult>,
    {
        let name = name.as_ref();

        // If the name isn't registered as a command and it exists in the name
        // table, it's a cvar.
        if !self.cmds.contains_key(name) && insert_name(&mut self.names, name).is_err() {
            return Err(ConsoleError::DuplicateCvar(name.into()));
        }

        self.cmds.insert(
            name.into(),
            Arc::new(move |args, input| cmd(args, input).into()),
        );

        Ok(())
    }

    /// Removes the command with the given name.
    ///
    /// Returns an error if there was no command with that name.
    pub fn remove<S>(&mut self, name: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        if self.cmds.remove(name.as_ref()).is_none() {
            return Err(ConsoleError::NoSuchCommand(name.as_ref().to_string()))?;
        }

        match self
            .names
            .binary_search_by(|item| item.as_str().cmp(name.as_ref()))
        {
            Ok(i) => drop(self.names.remove(i)),
            Err(_) => unreachable!("name in map but not in list: {}", name.as_ref()),
        }

        Ok(())
    }

    /// Executes a command.
    ///
    /// Returns an error if no command with the specified name exists.
    pub fn exec<'this, 'args, S>(
        &'this self,
        name: S,
        args: &'args [&'args str],
    ) -> Result<impl Fn(&mut World) -> ExecResult + 'args, ConsoleError>
    where
        S: AsRef<str>,
    {
        let cmd = self
            .cmds
            .get(name.as_ref())
            .ok_or(ConsoleError::NoSuchCommand(name.as_ref().to_string()))?
            .clone();

        Ok(move |world: &mut World| cmd(args, world))
    }

    pub fn contains<S>(&self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        self.cmds.contains_key(name.as_ref())
    }

    pub fn names(&self) -> impl Iterator<Item = &str> + '_ {
        self.names.iter().map(AsRef::as_ref)
    }
}

/// A configuration variable.
///
/// Cvars are the primary method of configuring the game.
#[derive(Debug, Clone)]
struct Cvar {
    // Value of this variable
    val: String,

    // If true, this variable should be archived in vars.rc
    archive: bool,

    // If true:
    // - If a server cvar, broadcast updates to clients
    // - If a client cvar, update userinfo
    notify: bool,

    // The default value of this variable
    default: String,
}

#[derive(Default, Debug, Resource, ExtractResource, Clone)]
pub struct CvarRegistry {
    cvars: im::HashMap<String, Cvar>,
    names: im::Vector<String>,
}

impl CvarRegistry {
    /// Construct a new empty `CvarRegistry`.
    pub fn new() -> CvarRegistry {
        Self {
            cvars: Default::default(),
            names: Default::default(),
        }
    }

    fn register_impl<S>(
        &mut self,
        name: S,
        default: S,
        archive: bool,
        notify: bool,
    ) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        let default = default.as_ref();

        let cvars = &mut self.cvars;
        match cvars.get(name) {
            Some(_) => Err(ConsoleError::DuplicateCvar(name.into()))?,
            None => {
                if insert_name(&mut self.names, name).is_err() {
                    return Err(ConsoleError::DuplicateCommand(name.into()));
                }

                cvars.insert(
                    name.to_owned(),
                    Cvar {
                        val: default.to_owned(),
                        archive,
                        notify,
                        default: default.to_owned(),
                    },
                );
            }
        }

        Ok(())
    }

    /// Register a new `Cvar` with the given name.
    pub fn register<S>(&mut self, name: S, default: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        self.register_impl(name, default, false, false)
    }

    /// Register a new archived `Cvar` with the given name.
    ///
    /// The value of this `Cvar` should be written to `vars.rc` whenever the game is closed or
    /// `host_writeconfig` is issued.
    pub fn register_archive<S>(&mut self, name: S, default: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        self.register_impl(name, default, true, false)
    }

    /// Register a new notify `Cvar` with the given name.
    ///
    /// When this `Cvar` is set:
    /// - If the host is a server, broadcast that the variable has been changed to all clients.
    /// - If the host is a client, update the clientinfo string.
    pub fn register_notify<S>(&mut self, name: S, default: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        self.register_impl(name, default, false, true)
    }

    /// Register a new notify + archived `Cvar` with the given name.
    ///
    /// The value of this `Cvar` should be written to `vars.rc` whenever the game is closed or
    /// `host_writeconfig` is issued.
    ///
    /// Additionally, when this `Cvar` is set:
    /// - If the host is a server, broadcast that the variable has been changed to all clients.
    /// - If the host is a client, update the clientinfo string.
    pub fn register_archive_notify<S>(&mut self, name: S, default: S) -> Result<(), ConsoleError>
    where
        S: AsRef<str>,
    {
        self.register_impl(name, default, true, true)
    }

    pub fn get<S>(&self, name: S) -> Result<String, ConsoleError>
    where
        S: AsRef<str>,
    {
        Ok(self
            .cvars
            .get(name.as_ref())
            .ok_or(ConsoleError::NoSuchCvar(name.as_ref().to_owned()))?
            .val
            .clone())
    }

    pub fn get_value<S>(&self, name: S) -> Result<f32, ConsoleError>
    where
        S: AsRef<str>,
    {
        let name = name.as_ref();
        let cvar = self
            .cvars
            .get(name)
            .ok_or(ConsoleError::NoSuchCvar(name.to_owned()))?;

        // try parsing as f32
        let val_string = cvar.val.clone();
        let val = match val_string.parse::<f32>() {
            Ok(v) => Ok(v),
            // if parse fails, reset to default value and try again
            Err(_) => cvar.default.parse::<f32>(),
        }
        .or(Err(ConsoleError::CvarParseFailed {
            name: name.to_owned(),
            value: val_string.clone(),
        }))?;

        Ok(val)
    }

    pub fn set<N, V>(&mut self, name: N, value: V) -> Result<(), ConsoleError>
    where
        N: AsRef<str>,
        V: Into<String>,
    {
        let (name, value) = (name.as_ref(), value.into());
        trace!("cvar assignment: {} {}", name, value);
        let cvars = &mut self.cvars;
        let cvar = cvars
            .get_mut(name)
            .ok_or_else(|| ConsoleError::NoSuchCvar(name.to_owned()))?;
        cvar.val = value;

        if cvar.notify {
            // TODO: update userinfo/serverinfo
            unimplemented!();
        }

        Ok(())
    }

    pub fn contains<S>(&self, name: S) -> bool
    where
        S: AsRef<str>,
    {
        self.cvars.contains_key(name.as_ref())
    }
}

/// The line of text currently being edited in the console.
#[derive(Clone)]
pub struct ConsoleInput {
    text: imstr::ImString,
    curs: usize,
}

impl ConsoleInput {
    /// Constructs a new `ConsoleInput`.
    ///
    /// Initializes the text content to be empty and places the cursor at position 0.
    pub fn new() -> ConsoleInput {
        ConsoleInput {
            text: Default::default(),
            curs: 0,
        }
    }

    /// Returns the current content of the `ConsoleInput`.
    pub fn get_text(&self) -> &str {
        &self.text
    }

    /// Sets the content of the `ConsoleInput` to `Text`.
    ///
    /// This also moves the cursor to the end of the line.
    pub fn set_text(&mut self, text: impl Into<ImString>) {
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

#[derive(Clone)]
pub struct History {
    lines: im::Vector<String>,
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
        self.lines.push_front(line);
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

#[derive(Clone)]
pub struct ConsoleOutput {
    // A ring buffer of lines of text. Each line has an optional timestamp used
    // to determine whether it should be displayed on screen. If the timestamp
    // is `None`, the message will not be displayed.
    //
    // The timestamp is specified in seconds since the Unix epoch (so it is
    // decoupled from client/server time).
    lines: im::Vector<(String, Option<i64>)>,
}

impl ConsoleOutput {
    pub fn new() -> ConsoleOutput {
        ConsoleOutput {
            lines: Default::default(),
        }
    }

    fn push(&mut self, chars: String, timestamp: Option<i64>) {
        self.lines.push_front((chars, timestamp))
        // TODO: set maximum capacity and pop_back when we reach it
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> + '_ {
        self.lines.iter().map(|(v, _)| v.as_ref())
    }

    /// Return an iterator over lines that have been printed in the last
    /// `interval` of time.
    ///
    /// The iterator yields the oldest results first.
    ///
    /// `max_candidates` specifies the maximum number of lines to consider,
    /// while `max_results` specifies the maximum number of lines that should
    /// be returned.
    pub fn recent_lines(
        &self,
        interval: Duration,
        max_candidates: usize,
        max_results: usize,
    ) -> impl Iterator<Item = &str> + '_ {
        let timestamp = (Utc::now() - interval).timestamp();
        self.lines
            .iter()
            // search only the most recent `max_candidates` lines
            .take(max_candidates)
            // yield oldest to newest
            .rev()
            // eliminate non-timestamped lines and lines older than `timestamp`
            .filter_map(move |(l, t)| if (*t)? > timestamp { Some(l) } else { None })
            // return at most `max_results` lines
            .take(max_results)
            .map(AsRef::as_ref)
    }
}

#[derive(Resource, ExtractResource, Clone)]
pub struct Console {
    aliases: im::HashMap<String, String>,

    input: ConsoleInput,
    hist: History,
    buffer: imstr::ImString,

    out_buffer: imstr::ImString,
    output: ConsoleOutput,
}

impl FromWorld for Console {
    fn from_world(world: &mut World) -> Console {
        let output = ConsoleOutput::new();
        let mut cmds = world.resource_mut::<CmdRegistry>();
        cmds.insert("echo", |args, _| {
            let msg = match args.len() {
                0 => "".to_owned(),
                _ => args.join(" "),
            };

            msg
        })
        .unwrap();

        cmds.insert("alias", move |args, world| {
            let mut console = world.resource_mut::<Console>();

            match args.len() {
                0 => {
                    for (name, script) in console.aliases.iter() {
                        return format!("    {}: {}", name, script);
                    }
                    return format!("{} alias command(s)", console.aliases.len());
                }

                2 => {
                    let name = args[0].to_string();
                    let script = args[1].to_string();
                    console.alias(name, script);
                }

                _ => (),
            }

            String::new()
        })
        .unwrap();

        cmds.insert("find", move |args, world| {
            match args.len() {
                1 => {
                    let cmds = world.resource::<CmdRegistry>();
                    // Take every item starting with the target.
                    let it = cmds
                        .names()
                        .skip_while(move |item| !item.starts_with(&args[0]))
                        .take_while(move |item| item.starts_with(&args[0]));

                    let mut output = String::new();
                    for name in it {
                        write!(&mut output, "{}\n", name).unwrap();
                    }

                    output
                }

                _ => "usage: find <cvar or command>".into(),
            }
        })
        .unwrap();

        Console {
            aliases: im::HashMap::new(),
            input: ConsoleInput::new(),
            hist: History::new(),
            buffer: Default::default(),
            out_buffer: Default::default(),
            output,
        }
    }
}

impl Console {
    pub fn alias(&mut self, name: String, script: String) {
        self.aliases.insert(name, script);
    }

    // The timestamp is applied to any line flushed during this call.
    fn print_impl<S>(&mut self, s: S, timestamp: Option<i64>)
    where
        S: AsRef<str>,
    {
        let mut it = s.as_ref().lines();

        if let Some(val) = it.next() {
            self.out_buffer.push_str(val);
        }

        while let Some(c) = it.next() {
            self.output
                .push(mem::take(&mut self.out_buffer).into(), timestamp);
            self.out_buffer.push_str(c);
        }
    }

    pub fn print<S>(&mut self, s: S)
    where
        S: AsRef<str>,
    {
        self.print_impl(s, None);
    }

    pub fn print_alert<S>(&mut self, s: S)
    where
        S: AsRef<str>,
    {
        self.print_impl(s, Some(Utc::now().timestamp()));
    }

    pub fn println<S>(&mut self, s: S)
    where
        S: AsRef<str>,
    {
        self.print_impl(s, None);
        self.print_impl("\n", None);
    }

    pub fn println_alert<S>(&mut self, s: S)
    where
        S: AsRef<str>,
    {
        let ts = Some(Utc::now().timestamp());
        self.print_impl(s, ts);
        self.print_impl("\n", ts);
    }

    pub fn send_char(&mut self, c: char) {
        match c {
            // ignore grave and escape keys
            '`' | '\x1b' => (),

            '\r' => {
                // cap with a newline and push to the execution buffer
                self.buffer.push_str(self.input.get_text());
                self.buffer.push_str("\n");

                // add the current input to the history
                self.hist.add_line(self.input.get_text().to_owned());

                // echo the input to console output
                let input_echo = format!("]{}", self.input.get_text());
                self.output.push(input_echo, None);

                // clear the input line
                self.input.clear();
            }

            '\x08' => self.input.backspace(),
            '\x7f' => self.input.delete(),

            '\t' => warn!("Tab completion not implemented"), // TODO: tab completion

            // TODO: we should probably restrict what characters are allowed
            c => self.input.insert(c),
        }
    }

    pub fn cursor(&self) -> usize {
        self.input.curs
    }

    pub fn cursor_right(&mut self) {
        self.input.cursor_right()
    }

    pub fn cursor_left(&mut self) {
        self.input.cursor_left()
    }

    pub fn history_up(&mut self) {
        if let Some(line) = self.hist.line_up() {
            self.input.set_text(line.to_owned());
        }
    }

    pub fn history_down(&mut self) {
        if let Some(line) = self.hist.line_down() {
            self.input.set_text(line.to_owned());
        }
    }

    /// Interprets the contents of the execution buffer.
    pub fn execute(&mut self, world: &mut World) {
        let text = mem::take(&mut self.buffer);
        fn parse_commands<'a, 'b, F: FnMut(&'b str) -> Cow<'a, str> + 'a>(
            input: &'b str,
            mut func: F,
        ) -> nom::IResult<&'b str, Vec<Vec<Cow<'a, str>>>> {
            let (rest, commands) = parse::commands(input)?;
            let out = commands
                .into_iter()
                .map(|cmd| cmd.into_iter().map(&mut func).collect::<Vec<_>>())
                .collect::<Vec<_>>();

            Ok((rest, out))
        }

        let (_, mut commands) = parse_commands(&text[..], |c| Cow::Borrowed(c)).unwrap();

        commands.reverse();

        while let Some(args) = commands.pop() {
            let tail_args: Vec<&str>;

            let func = {
                let world_cell = world.cell();
                let cmds = world_cell.resource::<CmdRegistry>();
                let mut cvars = world_cell.resource_mut::<CvarRegistry>();

                if let Some(arg_0) = args.get(0) {
                    let maybe_alias = self.aliases.get(arg_0.as_ref()).map(|a| a.to_owned());
                    match maybe_alias {
                        Some(a) => {
                            let (_, extra_commands) =
                                parse_commands(&*a, |s| Cow::from(s.to_string())).unwrap();
                            commands.extend(extra_commands.into_iter().rev());
                            continue;
                        }

                        None => {
                            tail_args = args.iter().map(|s| s.as_ref()).skip(1).collect();

                            if cmds.contains(arg_0) {
                                match cmds.exec(arg_0, &tail_args) {
                                    Ok(func) => func,
                                    Err(e) => {
                                        self.println(format!("{}", e));
                                        continue;
                                    }
                                }
                            } else if cvars.contains(arg_0) {
                                // TODO error handling on cvar set
                                match args.get(1) {
                                    Some(arg_1) => cvars.set(arg_0, arg_1.clone()).unwrap(),
                                    None => {
                                        let msg = format!(
                                            "\"{}\" is \"{}\"",
                                            arg_0,
                                            cvars.get(arg_0).unwrap()
                                        );
                                        self.println(msg);
                                    }
                                }

                                continue;
                            } else {
                                // TODO: try sending to server first
                                self.println(format!("Unrecognized command \"{}\"", arg_0));
                                continue;
                            }
                        }
                    }
                } else {
                    continue;
                }
            };

            let ExecResult {
                extra_commands,
                output,
            } = func(world);
            if !extra_commands.is_empty() {
                let (_, extra_commands) =
                    parse_commands(&*extra_commands, |s| Cow::from(s.to_string())).unwrap();
                commands.extend(extra_commands.into_iter().rev());
            }

            if !output.is_empty() {
                self.println(output)
            }
        }
    }

    pub fn get_string(&self) -> &str {
        self.input.get_text()
    }

    pub fn append_text<S>(&mut self, text: S)
    where
        S: AsRef<str>,
    {
        debug!("stuff_text:\n{:?}", text.as_ref());
        self.buffer.push_str(text.as_ref());
        self.buffer.push_str("\n");
    }

    pub fn output(&self) -> &ConsoleOutput {
        &self.output
    }
}
