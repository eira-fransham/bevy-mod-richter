// Copyright © 2020 Cormac O'Brien
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

mod cvars;
pub mod demo;
pub mod entity;
pub mod input;
pub mod menu;
pub mod render;
pub mod sound;
pub mod state;
pub mod trace;
pub mod view;

pub use self::cvars::register_cvars;
use self::{
    render::{Fov, RichterRenderPlugin},
    sound::{MixerEvent, MusicSource, RichterSoundPlugin},
};

use std::{
    collections::{HashMap, VecDeque},
    io::BufReader,
    net::ToSocketAddrs,
    ops::DerefMut,
    path::PathBuf,
    sync::Arc,
};

use crate::{
    client::{
        demo::{DemoServer, DemoServerError},
        entity::{ClientEntity, MAX_STATIC_ENTITIES},
        input::Input,
        sound::{MusicPlayer, StartSound, StartStaticSound, StopSound},
        state::{ClientState, PlayerInfo},
        trace::{TraceEntity, TraceFrame},
        view::{IdleVars, KickVars, MouseVars, RollVars},
    },
    common::{
        self,
        console::{CmdRegistry, Console, ConsoleError, CvarRegistry},
        engine,
        model::ModelError,
        net::{
            self,
            connect::{ConnectSocket, Request, Response, CONNECT_PROTOCOL_VERSION},
            ClientCmd, ClientStat, ColorShift, EntityEffects, EntityState, GameType, NetError,
            PlayerColor, ServerCmd, SignOnStage,
        },
        vfs::{Vfs, VfsError},
    },
};

use bevy::{
    app::Plugin,
    asset::AssetServer,
    ecs::{
        event::EventWriter,
        query::With,
        system::{In, Query, Res, ResMut, Resource},
        world::{FromWorld, World},
    },
    prelude::*,
    render::{
        extract_resource::ExtractResource,
        renderer::{RenderDevice, RenderQueue},
        Extract,
    },
    time::{Time, Virtual},
    window::{PrimaryWindow, Window},
};
use cgmath::Deg;
use chrono::Duration;
use input::InputFocus;
use menu::Menu;
use num_derive::FromPrimitive;
use render::{GraphicsState, WorldRenderer};
use sound::SoundError;
use thiserror::Error;
use view::BobVars;

// connections are tried 3 times, see
// https://github.com/id-Software/Quake/blob/master/WinQuake/net_dgrm.c#L1248
const MAX_CONNECT_ATTEMPTS: usize = 3;
const MAX_STATS: usize = 32;

const DEFAULT_SOUND_PACKET_VOLUME: u8 = 255;
const DEFAULT_SOUND_PACKET_ATTENUATION: f32 = 1.0;

const CONSOLE_DIVIDER: &'static str = "\
\n\n\
\x1D\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1F\
\n\n";

#[derive(Default)]
pub struct RichterPlugin {
    pub base_dir: Option<PathBuf>,
    pub game: Option<String>,
    pub main_menu: Menu,
}

#[derive(Clone, Resource, ExtractResource)]
pub struct RichterGameSettings {
    pub base_dir: PathBuf,
    pub game: Option<String>,
}

fn cvar_error_handler(In(result): In<Result<(), ConsoleError>>) {
    if let Err(err) = result {
        println!("encountered an error {:?}", err);
    }
}

impl Plugin for RichterPlugin {
    fn build(&self, app: &mut bevy::prelude::App) {
        app.insert_resource(RichterGameSettings {
            base_dir: self
                .base_dir
                .clone()
                .unwrap_or_else(|| common::default_base_dir()),
            game: self.game.clone(),
        })
        .insert_resource(self.main_menu.clone())
        .init_resource::<Vfs>()
        .init_resource::<CmdRegistry>()
        .init_resource::<CvarRegistry>()
        .init_resource::<Console>()
        .init_resource::<Input>()
        .init_resource::<MusicPlayer>()
        .init_resource::<DemoQueue>()
        // .init_resource::<GameConnection>()
        // .add_systems(
        //     Startup,
        //     (register_cvars.pipe(cvar_error_handler), init_client),
        // )
        // .add_systems(Main, run_console)
        // .add_systems(
        //     Main,
        //     (
        //         handle_input.pipe(|In(res)| {
        //             // TODO: Error handling
        //             let _ = res;
        //         }),
        //         frame.pipe(|In(res)| {
        //             // TODO: Error handling
        //             let _ = res;
        //         }),
        //     ),
        // )
        .add_plugins(RichterRenderPlugin)
        .add_plugins(RichterSoundPlugin);
    }
}

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Connection rejected: {0}")]
    ConnectionRejected(String),
    #[error("Couldn't read cvar value: {0}")]
    Cvar(ConsoleError),
    #[error("Server sent an invalid port number ({0})")]
    InvalidConnectPort(i32),
    #[error("Server sent an invalid connect response")]
    InvalidConnectResponse,
    #[error("Invalid server address")]
    InvalidServerAddress,
    #[error("No response from server")]
    NoResponse,
    #[error("Unrecognized protocol: {0}")]
    UnrecognizedProtocol(i32),
    #[error("Client is not connected")]
    NotConnected,
    #[error("Client has already signed on")]
    AlreadySignedOn,
    #[error("No client with ID {0}")]
    NoSuchClient(usize),
    #[error("No player with ID {0}")]
    NoSuchPlayer(usize),
    #[error("No entity with ID {0}")]
    NoSuchEntity(usize),
    #[error("Null entity access")]
    NullEntity,
    #[error("Entity already exists: {0}")]
    EntityExists(usize),
    #[error("Invalid view entity: {0}")]
    InvalidViewEntity(usize),
    #[error("Too many static entities")]
    TooManyStaticEntities,
    #[error("No such lightmap animation: {0}")]
    NoSuchLightmapAnimation(usize),
    // TODO: wrap PlayError
    #[error("Failed to open audio output stream")]
    OutputStream,
    #[error("Demo server error: {0}")]
    DemoServer(#[from] DemoServerError),
    #[error("Model error: {0}")]
    Model(#[from] ModelError),
    #[error("Network error: {0}")]
    Network(#[from] NetError),
    #[error("Failed to load sound: {0}")]
    Sound(#[from] SoundError),
    #[error("Virtual filesystem error: {0}")]
    Vfs(#[from] VfsError),
}

impl From<ConsoleError> for ClientError {
    fn from(value: ConsoleError) -> Self {
        Self::Cvar(value)
    }
}

pub struct MoveVars {
    cl_anglespeedkey: f32,
    cl_pitchspeed: f32,
    cl_yawspeed: f32,
    cl_sidespeed: f32,
    cl_upspeed: f32,
    cl_forwardspeed: f32,
    cl_backspeed: f32,
    cl_movespeedkey: f32,
}

#[derive(Debug, FromPrimitive)]
enum ColorShiftCode {
    Contents = 0,
    Damage = 1,
    Bonus = 2,
    Powerup = 3,
}

struct ServerInfo {
    _max_clients: u8,
    _game_type: GameType,
}

#[derive(Clone, Debug)]
pub enum IntermissionKind {
    Intermission,
    Finale { text: String },
    Cutscene { text: String },
}

/// Indicates to the client what should be done with the current connection.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum ConnectionStatus {
    /// Maintain the connection.
    Maintain,

    /// Disconnect from the server or demo server.
    Disconnect,

    /// Play the next demo in the demo queue.
    NextDemo,
}

/// Indicates the state of an active connection.
#[derive(Clone)]
enum ConnectionState {
    /// The client is in the sign-on process.
    SignOn(SignOnStage),

    /// The client is fully connected.
    Connected(Arc<WorldRenderer>),
}

/// Possible targets that a client can be connected to.
#[derive(Clone)]
enum ConnectionKind {
    /// A regular Quake server.
    Server {
        /// The [`QSocket`](crate::net::QSocket) used to communicate with the server.
        qsock: (), // QSocket,

        /// The client's packet composition buffer.
        compose: Vec<u8>,
    },

    /// A demo server.
    Demo(DemoServer),
}

/// A connection to a game server of some kind.
///
/// The exact nature of the connected server is specified by [`ConnectionKind`].
#[derive(Clone)]
pub struct Connection {
    state: ClientState,
    conn_state: ConnectionState,
    kind: ConnectionKind,
}

#[derive(Resource, Clone, ExtractResource, Default)]
pub struct GameConnection(pub Option<Connection>);

impl GameConnection {
    pub fn view_entity_id(&self) -> Option<usize> {
        match &self.0 {
            Some(Connection { state, .. }) => Some(state.view_entity_id()),
            None => None,
        }
    }

    pub fn trace<'a, I>(&self, entity_ids: I) -> Result<TraceFrame, ClientError>
    where
        I: IntoIterator<Item = &'a usize>,
    {
        match &self.0 {
            Some(Connection { state, .. }) => {
                let mut trace = TraceFrame {
                    msg_times_ms: [
                        state.msg_times[0].num_milliseconds(),
                        state.msg_times[1].num_milliseconds(),
                    ],
                    time_ms: state.time.num_milliseconds(),
                    lerp_factor: state.lerp_factor,
                    entities: HashMap::new(),
                };

                for id in entity_ids.into_iter() {
                    let ent = &state.entities[*id];

                    let msg_origins = [ent.msg_origins[0].into(), ent.msg_origins[1].into()];
                    let msg_angles_deg = [
                        [
                            ent.msg_angles[0][0].0,
                            ent.msg_angles[0][1].0,
                            ent.msg_angles[0][2].0,
                        ],
                        [
                            ent.msg_angles[1][0].0,
                            ent.msg_angles[1][1].0,
                            ent.msg_angles[1][2].0,
                        ],
                    ];

                    trace.entities.insert(
                        *id as u32,
                        TraceEntity {
                            msg_origins,
                            msg_angles_deg,
                            origin: ent.origin.into(),
                        },
                    );
                }

                Ok(trace)
            }

            None => Err(ClientError::NotConnected),
        }
    }
}

impl Connection {
    fn handle_signon(
        &mut self,
        new_stage: SignOnStage,
        gfx_state: &mut GraphicsState,
        device: &RenderDevice,
        queue: &RenderQueue,
    ) -> Result<(), ClientError> {
        use SignOnStage::*;

        let new_conn_state = match self.conn_state {
            // TODO: validate stage transition
            ConnectionState::SignOn(ref mut _stage) => {
                if let ConnectionKind::Server {
                    ref mut compose, ..
                } = self.kind
                {
                    match new_stage {
                        Not => (), // TODO this is an error (invalid value)
                        Prespawn => {
                            ClientCmd::StringCmd {
                                cmd: String::from("prespawn"),
                            }
                            .serialize(compose)?;
                        }
                        ClientInfo => {
                            // TODO: fill in client info here
                            ClientCmd::StringCmd {
                                cmd: format!("name \"{}\"\n", "UNNAMED"),
                            }
                            .serialize(compose)?;
                            ClientCmd::StringCmd {
                                cmd: format!("color {} {}", 0, 0),
                            }
                            .serialize(compose)?;
                            // TODO: need default spawn parameters?
                            ClientCmd::StringCmd {
                                cmd: format!("spawn {}", ""),
                            }
                            .serialize(compose)?;
                        }
                        SignOnStage::Begin => {
                            ClientCmd::StringCmd {
                                cmd: String::from("begin"),
                            }
                            .serialize(compose)?;
                        }
                        SignOnStage::Done => {
                            debug!("SignOn complete");
                            // TODO: end load screen
                            self.state.start_time = self.state.time;
                        }
                    }
                }

                match new_stage {
                    // TODO proper error
                    Not => panic!("SignOnStage::Not in handle_signon"),
                    // still signing on, advance to the new stage
                    Prespawn | ClientInfo | Begin => ConnectionState::SignOn(new_stage),

                    // finished signing on, build world renderer
                    Done => ConnectionState::Connected(
                        WorldRenderer::new(gfx_state, device, queue, self.state.models(), 1).into(),
                    ),
                }
            }

            // ignore spurious sign-on messages
            ConnectionState::Connected { .. } => return Ok(()),
        };

        self.conn_state = new_conn_state;

        Ok(())
    }

    fn parse_server_msg(
        &mut self,
        vfs: &Vfs,
        asset_server: &AssetServer,
        gfx_state: &mut GraphicsState,
        device: &RenderDevice,
        queue: &RenderQueue,
        cmds: &mut CmdRegistry,
        console: &mut Console,
        mixer_events: &mut EventWriter<MixerEvent>,
        kick_vars: KickVars,
    ) -> Result<ConnectionStatus, ClientError> {
        use ConnectionStatus::*;

        let (msg, demo_view_angles, track_override) = match self.kind {
            ConnectionKind::Server { ref mut qsock, .. } => {
                // let msg = qsock.recv_msg(match self.conn_state {
                //     // if we're in the game, don't block waiting for messages
                //     ConnectionState::Connected(_) => BlockingMode::NonBlocking,

                //     // otherwise, give the server some time to respond
                //     // TODO: might make sense to make this a future or something
                //     ConnectionState::SignOn(_) => BlockingMode::Timeout(Duration::seconds(5)),
                // })?;

                // (msg, None, None)
                todo!()
            }

            ConnectionKind::Demo(ref mut demo_srv) => {
                // only get the next update once we've made it all the way to
                // the previous one
                if self.state.time >= self.state.msg_times[0] {
                    let msg_view = match demo_srv.next() {
                        Some(v) => v,
                        None => {
                            // if there are no commands left in the demo, play
                            // the next demo if there is one
                            return Ok(NextDemo);
                        }
                    };

                    let mut view_angles = msg_view.view_angles();
                    // invert entity angles to get the camera direction right.
                    // yaw is already inverted.
                    view_angles.z = -view_angles.z;

                    // TODO: we shouldn't have to copy the message here
                    (
                        msg_view.message().to_owned(),
                        Some(view_angles),
                        demo_srv.track_override(),
                    )
                } else {
                    (Vec::new(), None, demo_srv.track_override())
                }
            }
        };

        // no data available at this time
        if msg.is_empty() {
            return Ok(Maintain);
        }

        let mut reader = BufReader::new(msg.as_slice());

        while let Some(cmd) = ServerCmd::deserialize(&mut reader)? {
            match cmd {
                // TODO: have an error for this instead of panicking
                // once all other commands have placeholder handlers, just error
                // in the wildcard branch
                ServerCmd::Bad => panic!("Invalid command from server"),

                ServerCmd::NoOp => (),

                ServerCmd::CdTrack { track, .. } => {
                    mixer_events.send(MixerEvent::StartMusic(Some(sound::MusicSource::TrackId(
                        match track_override {
                            Some(t) => t as usize,
                            None => track as usize,
                        },
                    ))));
                }

                ServerCmd::CenterPrint { text } => {
                    // TODO: print to center of screen
                    warn!("Center print not yet implemented!");
                    println!("{}", text);
                }

                ServerCmd::PlayerData(player_data) => self.state.update_player(player_data),

                ServerCmd::Cutscene { text } => {
                    self.state.intermission = Some(IntermissionKind::Cutscene { text });
                    self.state.completion_time = Some(self.state.time);
                }

                ServerCmd::Damage {
                    armor,
                    blood,
                    source,
                } => self.state.handle_damage(armor, blood, source, kick_vars),

                ServerCmd::Disconnect => {
                    return Ok(match self.kind {
                        ConnectionKind::Demo(_) => NextDemo,
                        ConnectionKind::Server { .. } => Disconnect,
                    })
                }

                ServerCmd::FastUpdate(ent_update) => {
                    // first update signals the last sign-on stage
                    self.handle_signon(SignOnStage::Done, gfx_state, device, queue)?;

                    let ent_id = ent_update.ent_id as usize;
                    self.state.update_entity(ent_id, ent_update)?;

                    // patch view angles in demos
                    if let Some(angles) = demo_view_angles {
                        if ent_id == self.state.view_entity_id() {
                            self.state.update_view_angles(angles);
                        }
                    }
                }

                ServerCmd::Finale { text } => {
                    self.state.intermission = Some(IntermissionKind::Finale { text });
                    self.state.completion_time = Some(self.state.time);
                }

                ServerCmd::FoundSecret => self.state.stats[ClientStat::FoundSecrets as usize] += 1,
                ServerCmd::Intermission => {
                    self.state.intermission = Some(IntermissionKind::Intermission);
                    self.state.completion_time = Some(self.state.time);
                }
                ServerCmd::KilledMonster => {
                    self.state.stats[ClientStat::KilledMonsters as usize] += 1
                }

                ServerCmd::LightStyle { id, value } => {
                    trace!("Inserting light style {} with value {}", id, &value);
                    let _ = self.state.light_styles.insert(id, value);
                }

                ServerCmd::Particle {
                    origin,
                    direction,
                    count,
                    color,
                } => {
                    match count {
                        // if count is 255, this is an explosion
                        255 => Arc::make_mut(&mut self.state.particles)
                            .create_explosion(self.state.time, origin),

                        // otherwise it's an impact
                        _ => Arc::make_mut(&mut self.state.particles).create_projectile_impact(
                            self.state.time,
                            origin,
                            direction,
                            color,
                            count as usize,
                        ),
                    }
                }

                ServerCmd::Print { text } => console.print_alert(&text),

                ServerCmd::ServerInfo {
                    protocol_version,
                    max_clients,
                    game_type,
                    message,
                    model_precache,
                    sound_precache,
                } => {
                    // check protocol version
                    if protocol_version != net::PROTOCOL_VERSION as i32 {
                        Err(ClientError::UnrecognizedProtocol(protocol_version))?;
                    }

                    console.println(CONSOLE_DIVIDER);
                    console.println(message);
                    console.println(CONSOLE_DIVIDER);

                    let _server_info = ServerInfo {
                        _max_clients: max_clients,
                        _game_type: game_type,
                    };

                    self.state = ClientState::from_server_info(
                        vfs,
                        asset_server,
                        max_clients,
                        model_precache,
                        sound_precache,
                    )?;

                    cmds.insert_or_replace("bf", move |_, world: &mut World| {
                        if let Some(Connection { state, .. }) =
                            &mut world.resource_mut::<GameConnection>().0
                        {
                            state.color_shifts[ColorShiftCode::Bonus as usize] = ColorShift {
                                dest_color: [215, 186, 69],
                                percent: 50,
                            };
                        }
                        String::new()
                    })
                    .unwrap();
                }

                ServerCmd::SetAngle { angles } => self.state.set_view_angles(angles),

                ServerCmd::SetView { ent_id } => {
                    if ent_id == 0 {
                        // TODO: Why do we occasionally see this in demos?
                    } else if ent_id <= 0 {
                        Err(ClientError::InvalidViewEntity(ent_id as usize))?;
                    } else {
                        self.state.set_view_entity(ent_id as usize)?;
                    }
                }

                ServerCmd::SignOnStage { stage } => {
                    self.handle_signon(stage, gfx_state, device, queue)?
                }

                ServerCmd::Sound {
                    volume,
                    attenuation,
                    entity_id,
                    channel,
                    sound_id,
                    position,
                } => {
                    trace!(
                        "starting sound with id {} on entity {} channel {}",
                        sound_id,
                        entity_id,
                        channel
                    );

                    if entity_id as usize >= self.state.entities.len() {
                        warn!(
                            "server tried to start sound on nonexistent entity {}",
                            entity_id
                        );
                        break;
                    }

                    let volume = volume.unwrap_or(DEFAULT_SOUND_PACKET_VOLUME);
                    let attenuation = attenuation.unwrap_or(DEFAULT_SOUND_PACKET_ATTENUATION);
                    // TODO: apply volume, attenuation, spatialization
                    mixer_events.send(MixerEvent::StartSound(StartSound {
                        src: self.state.sounds[sound_id as usize].clone(),
                        ent_id: Some(entity_id as usize),
                        ent_channel: channel,
                        volume: volume as f32 / 255.0,
                        attenuation,
                        origin: position.into(),
                    }));
                }

                ServerCmd::SpawnBaseline {
                    ent_id,
                    model_id,
                    frame_id,
                    colormap,
                    skin_id,
                    origin,
                    angles,
                } => {
                    self.state.spawn_entities(
                        ent_id as usize,
                        EntityState {
                            model_id: model_id as usize,
                            frame_id: frame_id as usize,
                            colormap,
                            skin_id: skin_id as usize,
                            origin,
                            angles,
                            effects: EntityEffects::empty(),
                        },
                    )?;
                }

                ServerCmd::SpawnStatic {
                    model_id,
                    frame_id,
                    colormap,
                    skin_id,
                    origin,
                    angles,
                } => {
                    if self.state.static_entities.len() >= MAX_STATIC_ENTITIES {
                        Err(ClientError::TooManyStaticEntities)?;
                    }
                    self.state
                        .static_entities
                        .push_back(ClientEntity::from_baseline(EntityState {
                            origin,
                            angles,
                            model_id: model_id as usize,
                            frame_id: frame_id as usize,
                            colormap,
                            skin_id: skin_id as usize,
                            effects: EntityEffects::empty(),
                        }));
                }

                ServerCmd::SpawnStaticSound {
                    origin,
                    sound_id,
                    volume,
                    attenuation,
                } => {
                    mixer_events.send(MixerEvent::StartStaticSound(StartStaticSound {
                        src: self.state.sounds[sound_id as usize].clone(),
                        origin,
                        volume: volume as f32 / 255.0,
                        attenuation: attenuation as f32 / 64.0,
                    }));
                }

                ServerCmd::TempEntity { temp_entity } => {
                    self.state.spawn_temp_entity(mixer_events, &temp_entity)
                }

                ServerCmd::StuffText { text } => console.append_text(text),

                ServerCmd::Time { time } => {
                    self.state.msg_times[1] = self.state.msg_times[0];
                    self.state.msg_times[0] = engine::duration_from_f32(time);
                }

                ServerCmd::UpdateColors {
                    player_id,
                    new_colors,
                } => {
                    let player_id = player_id as usize;
                    self.state.check_player_id(player_id)?;

                    match self.state.player_info[player_id] {
                        Some(ref mut info) => {
                            trace!(
                                "Player {} (ID {}) colors: {:?} -> {:?}",
                                info.name,
                                player_id,
                                info.colors,
                                new_colors,
                            );
                            info.colors = new_colors;
                        }

                        None => {
                            error!(
                                "Attempted to set colors on nonexistent player with ID {}",
                                player_id
                            );
                        }
                    }
                }

                ServerCmd::UpdateFrags {
                    player_id,
                    new_frags,
                } => {
                    let player_id = player_id as usize;
                    self.state.check_player_id(player_id)?;

                    match self.state.player_info[player_id] {
                        Some(ref mut info) => {
                            trace!(
                                "Player {} (ID {}) frags: {} -> {}",
                                &info.name,
                                player_id,
                                info.frags,
                                new_frags
                            );
                            info.frags = new_frags as i32;
                        }
                        None => {
                            error!(
                                "Attempted to set frags on nonexistent player with ID {}",
                                player_id
                            );
                        }
                    }
                }

                ServerCmd::UpdateName {
                    player_id,
                    new_name,
                } => {
                    let player_id = player_id as usize;
                    self.state.check_player_id(player_id)?;

                    if let Some(ref mut info) = self.state.player_info[player_id] {
                        // if this player is already connected, it's a name change
                        debug!("Player {} has changed name to {}", &info.name, &new_name);
                        info.name = new_name.into();
                    } else {
                        // if this player is not connected, it's a join
                        debug!("Player {} with ID {} has joined", &new_name, player_id);
                        self.state.player_info[player_id] = Some(PlayerInfo {
                            name: new_name.into(),
                            colors: PlayerColor::new(0, 0),
                            frags: 0,
                        });
                    }
                }

                ServerCmd::UpdateStat { stat, value } => {
                    debug!(
                        "{:?}: {} -> {}",
                        stat, self.state.stats[stat as usize], value
                    );
                    self.state.stats[stat as usize] = value;
                }

                ServerCmd::Version { version } => {
                    if version != net::PROTOCOL_VERSION as i32 {
                        // TODO: handle with an error
                        error!(
                            "Incompatible server version: server's is {}, client's is {}",
                            version,
                            net::PROTOCOL_VERSION,
                        );
                        panic!("bad version number");
                    }
                }

                ServerCmd::SetPause { .. } => {}

                ServerCmd::StopSound { entity_id, channel } => {
                    mixer_events.send(MixerEvent::StopSound(StopSound {
                        ent_id: Some(entity_id as _),
                        ent_channel: channel,
                    }));
                }
                ServerCmd::SellScreen => todo!(),
            }
        }

        Ok(Maintain)
    }

    fn frame(
        &mut self,
        frame_time: Duration,
        vfs: &Vfs,
        asset_server: &AssetServer,
        gfx_state: &mut GraphicsState,
        device: &RenderDevice,
        queue: &RenderQueue,
        cmds: &mut CmdRegistry,
        console: &mut Console,
        mixer_events: &mut EventWriter<MixerEvent>,
        idle_vars: IdleVars,
        kick_vars: KickVars,
        roll_vars: RollVars,
        bob_vars: BobVars,
        cl_nolerp: f32,
        sv_gravity: f32,
    ) -> Result<ConnectionStatus, ClientError> {
        debug!("frame time: {}ms", frame_time.num_milliseconds());

        // do this _before_ parsing server messages so that we know when to
        // request the next message from the demo server.
        self.state.advance_time(frame_time);
        match self.parse_server_msg(
            vfs,
            asset_server,
            gfx_state,
            device,
            queue,
            cmds,
            console,
            mixer_events,
            kick_vars,
        )? {
            ConnectionStatus::Maintain => (),
            // if Disconnect or NextDemo, delegate up the chain
            s => return Ok(s),
        };

        self.state.update_interp_ratio(cl_nolerp);

        // interpolate entity data and spawn particle effects, lights
        self.state.update_entities()?;

        // update temp entities (lightning, etc.)
        self.state.update_temp_entities()?;

        // remove expired lights
        Arc::make_mut(&mut self.state.lights).update(self.state.time);

        // apply particle physics and remove expired particles
        Arc::make_mut(&mut self.state.particles).update(self.state.time, frame_time, sv_gravity);

        if let ConnectionKind::Server {
            ref mut qsock,
            ref mut compose,
        } = self.kind
        {
            // respond to the server
            // if qsock.can_send() && !compose.is_empty() {
            //     qsock.begin_send_msg(&compose)?;
            //     compose.clear();
            // }
            todo!()
        }

        // these all require the player entity to have spawned
        if let ConnectionState::Connected(_) = self.conn_state {
            // update view
            self.state
                .calc_final_view(idle_vars, kick_vars, roll_vars, bob_vars);

            // update camera color shifts for new position/effects
            self.state.update_color_shifts(frame_time)?;
        }

        Ok(ConnectionStatus::Maintain)
    }
}

#[derive(Resource, ExtractResource, Clone, Default)]
pub struct DemoQueue(pub VecDeque<String>);

pub fn init_client<C: DerefMut<Target = CmdRegistry>>(mut cmds: C) {
    // TODO
    // commands.init_resource();

    // set up overlay/ui toggles
    cmds.insert_or_replace("toggleconsole", cmd_toggleconsole)
        .unwrap();
    cmds.insert_or_replace("togglemenu", cmd_togglemenu)
        .unwrap();

    // set up connection console commands
    cmds.insert_or_replace("connect", cmd_connect).unwrap();
    cmds.insert_or_replace("reconnect", cmd_reconnect).unwrap();
    cmds.insert_or_replace("disconnect", cmd_disconnect)
        .unwrap();

    // set up demo playback
    cmds.insert_or_replace("playdemo", cmd_playdemo).unwrap();

    cmds.insert_or_replace("startdemos", cmd_startdemos)
        .unwrap();

    cmds.insert_or_replace("music", cmd_music).unwrap();
    cmds.insert_or_replace("music_stop", cmd_music_stop)
        .unwrap();
    cmds.insert_or_replace("music_pause", cmd_music_pause)
        .unwrap();
    cmds.insert_or_replace("music_resume", cmd_music_resume)
        .unwrap();
}

#[derive(Resource, Default)]
pub struct TimeHack(pub Time<Virtual>);

impl ExtractResource for TimeHack {
    type Source = Time<Virtual>;
    fn extract_resource(source: &Self::Source) -> Self {
        Self(*source)
    }
}

pub fn frame(
    frame_time: Res<TimeHack>,
    mut gfx_state: ResMut<GraphicsState>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    cvars: Res<CvarRegistry>,
    vfs: Res<Vfs>,
    asset_server: Res<AssetServer>,
    mut cmds: ResMut<CmdRegistry>,
    mut console: ResMut<Console>,
    mut mixer_events: EventWriter<MixerEvent>,
    mut demo_queue: ResMut<DemoQueue>,
    mut input: ResMut<Input>,
    mut conn: ResMut<GameConnection>,
) -> Result<(), ClientError> {
    let frame_time = frame_time.0;
    let conn = &mut conn.0;
    let cl_nolerp = cvars.get_value("cl_nolerp")?;
    let sv_gravity = cvars.get_value("sv_gravity")?;
    let idle_vars = cvars.idle_vars()?;
    let kick_vars = cvars.kick_vars()?;
    let roll_vars = cvars.roll_vars()?;
    let bob_vars = cvars.bob_vars()?;

    let status = match *conn {
        Some(ref mut conn) => conn.frame(
            Duration::from_std(frame_time.delta()).unwrap(),
            &*vfs,
            &*asset_server,
            &mut *gfx_state,
            &*device,
            &*queue,
            &mut *cmds,
            &mut *console,
            &mut mixer_events,
            idle_vars,
            kick_vars,
            roll_vars,
            bob_vars,
            cl_nolerp,
            sv_gravity,
        )?,
        None => ConnectionStatus::Disconnect,
    };

    use ConnectionStatus::*;
    match status {
        Maintain => (),
        _ => {
            let new_conn = match status {
                // if client is already disconnected, this is a no-op
                Disconnect => None,

                // get the next demo from the queue
                NextDemo => {
                    // Prevent the demo queue borrow from lasting too long
                    let next = {
                        let next = demo_queue.0.pop_front();

                        next
                    };
                    match next {
                        Some(demo) => {
                            // TODO: Extract this to a separate function so we don't duplicate the logic to find the demos in different places
                            let mut demo_file = match vfs
                                .open(format!("{}.dem", demo))
                                .or_else(|_| vfs.open(format!("demos/{}.dem", demo)))
                            {
                                Ok(f) => Some(f),
                                Err(e) => {
                                    // log the error, dump the demo queue and disconnect
                                    console.println(format!("{}", e));
                                    demo_queue.0.clear();
                                    None
                                }
                            };

                            demo_file.as_mut().and_then(|df| match DemoServer::new(df) {
                                Ok(d) => Some(Connection {
                                    kind: ConnectionKind::Demo(d),
                                    state: ClientState::new(),
                                    conn_state: ConnectionState::SignOn(SignOnStage::Prespawn),
                                }),
                                Err(e) => {
                                    console.println(format!("{}", e));
                                    demo_queue.0.clear();
                                    None
                                }
                            })
                        }

                        // if there are no more demos in the queue, disconnect
                        None => None,
                    }
                }

                // covered in first match
                Maintain => unreachable!(),
            };

            match new_conn {
                Some(_) => input.set_focus(InputFocus::Game),

                // don't allow game focus when disconnected
                None => input.set_focus(InputFocus::Console),
            }

            *conn = new_conn;
        }
    }

    Ok(())
}

pub fn run_console(world: &mut World) {
    let mut console = world.resource::<Console>().clone();
    console.execute(world);
    *world.resource_mut::<Console>() = console;
}

#[derive(Resource, Clone, Copy)]
pub struct RenderResolution(pub u32, pub u32);

impl FromWorld for RenderResolution {
    fn from_world(world: &mut World) -> Self {
        let res = &world
            .query_filtered::<&Window, With<PrimaryWindow>>()
            .single(world)
            .resolution;

        RenderResolution(res.width() as _, res.height() as _)
    }
}

pub enum RenderConnectionKind {
    Server,
    Demo,
}

pub struct RenderState {
    state: ClientState,
    conn_state: ConnectionState,
    kind: RenderConnectionKind,
}

#[derive(Resource, Default)]
pub struct RenderStateRes(Option<RenderState>);

impl ExtractResource for RenderStateRes {
    type Source = GameConnection;

    fn extract_resource(source: &Self::Source) -> Self {
        Self(source.0.as_ref().map(
            |Connection {
                 state,
                 conn_state,
                 kind,
             }| {
                RenderState {
                    state: state.clone(),
                    conn_state: conn_state.clone(),
                    kind: match kind {
                        ConnectionKind::Server { .. } => RenderConnectionKind::Server,
                        ConnectionKind::Demo(_) => RenderConnectionKind::Demo,
                    },
                }
            },
        ))
    }
}

impl ExtractResource for InputFocus {
    type Source = Input;

    fn extract_resource(source: &Self::Source) -> Self {
        source.focus()
    }
}

impl ExtractResource for Fov {
    type Source = CvarRegistry;

    fn extract_resource(source: &Self::Source) -> Self {
        Self(Deg(source.get_value("fov").unwrap()))
    }
}

pub fn extract_resolution(
    mut render_resolution: ResMut<RenderResolution>,
    extracted_vars: Extract<Query<&Window, With<PrimaryWindow>>>,
) {
    let window = &*extracted_vars;

    let res = &window.single().resolution;
    *render_resolution = RenderResolution(res.width() as _, res.height() as _);
}

pub fn handle_input(
    cvars: Res<CvarRegistry>,
    mut console: ResMut<Console>,
    mut conn: ResMut<GameConnection>,
    mut input: ResMut<Input>,
    frame_time: Res<TimeHack>,
) -> Result<(), ClientError> {
    let frame_time = frame_time.0;
    let move_vars = cvars.move_vars()?;
    let mouse_vars = cvars.mouse_vars()?;

    match conn.0 {
        Some(Connection {
            ref mut state,
            kind: ConnectionKind::Server { ref mut qsock, .. },
            ..
        }) => {
            let move_cmd = state.handle_input(
                input.game_input_mut().unwrap(),
                Duration::from_std(frame_time.delta()).unwrap(),
                move_vars,
                mouse_vars,
            );
            // TODO: arrayvec here
            let mut msg = Vec::new();
            move_cmd.serialize(&mut msg)?;
            // qsock.send_msg_unreliable(&msg)?;

            // clear mouse and impulse
            input.game_input_mut().unwrap().refresh(&mut *console);
        }

        _ => (),
    }

    Ok(())
}

trait CvarsExt {
    fn move_vars(&self) -> Result<MoveVars, ClientError>;
    fn idle_vars(&self) -> Result<IdleVars, ClientError>;
    fn kick_vars(&self) -> Result<KickVars, ClientError>;
    fn mouse_vars(&self) -> Result<MouseVars, ClientError>;
    fn roll_vars(&self) -> Result<RollVars, ClientError>;
    fn bob_vars(&self) -> Result<BobVars, ClientError>;
}

impl CvarsExt for CvarRegistry {
    fn move_vars(&self) -> Result<MoveVars, ClientError> {
        Ok(MoveVars {
            cl_anglespeedkey: self.get_value("cl_anglespeedkey")?,
            cl_pitchspeed: self.get_value("cl_pitchspeed")?,
            cl_yawspeed: self.get_value("cl_yawspeed")?,
            cl_sidespeed: self.get_value("cl_sidespeed")?,
            cl_upspeed: self.get_value("cl_upspeed")?,
            cl_forwardspeed: self.get_value("cl_forwardspeed")?,
            cl_backspeed: self.get_value("cl_backspeed")?,
            cl_movespeedkey: self.get_value("cl_movespeedkey")?,
        })
    }

    fn idle_vars(&self) -> Result<IdleVars, ClientError> {
        Ok(IdleVars {
            v_idlescale: self.get_value("v_idlescale")?,
            v_ipitch_cycle: self.get_value("v_ipitch_cycle")?,
            v_ipitch_level: self.get_value("v_ipitch_level")?,
            v_iroll_cycle: self.get_value("v_iroll_cycle")?,
            v_iroll_level: self.get_value("v_iroll_level")?,
            v_iyaw_cycle: self.get_value("v_iyaw_cycle")?,
            v_iyaw_level: self.get_value("v_iyaw_level")?,
        })
    }

    fn kick_vars(&self) -> Result<KickVars, ClientError> {
        Ok(KickVars {
            v_kickpitch: self.get_value("v_kickpitch")?,
            v_kickroll: self.get_value("v_kickroll")?,
            v_kicktime: self.get_value("v_kicktime")?,
        })
    }

    fn mouse_vars(&self) -> Result<MouseVars, ClientError> {
        Ok(MouseVars {
            m_pitch: self.get_value("m_pitch")?,
            m_yaw: self.get_value("m_yaw")?,
            sensitivity: self.get_value("sensitivity")?,
        })
    }

    fn roll_vars(&self) -> Result<RollVars, ClientError> {
        Ok(RollVars {
            cl_rollangle: self.get_value("cl_rollangle")?,
            cl_rollspeed: self.get_value("cl_rollspeed")?,
        })
    }

    fn bob_vars(&self) -> Result<BobVars, ClientError> {
        Ok(BobVars {
            cl_bob: self.get_value("cl_bob")?,
            cl_bobcycle: self.get_value("cl_bobcycle")?,
            cl_bobup: self.get_value("cl_bobup")?,
        })
    }
}

// implements the "toggleconsole" command
fn cmd_toggleconsole(_: &[&str], world: &mut World) -> String {
    let has_conn = world.resource::<GameConnection>().0.is_some();
    let mut input = world.resource_mut::<Input>();
    let focus = input.focus();
    if has_conn {
        match focus {
            InputFocus::Game => input.set_focus(InputFocus::Console),
            InputFocus::Console => input.set_focus(InputFocus::Game),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    } else {
        match focus {
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Game => unreachable!(),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    }
    String::new()
}

// implements the "togglemenu" command
fn cmd_togglemenu(_: &[&str], world: &mut World) -> String {
    let has_conn = world.resource::<GameConnection>().0.is_some();
    let mut input = world.resource_mut::<Input>();
    let focus = input.focus();
    if has_conn {
        match focus {
            InputFocus::Game => input.set_focus(InputFocus::Menu),
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Menu => input.set_focus(InputFocus::Game),
        }
    } else {
        match focus {
            InputFocus::Console => input.set_focus(InputFocus::Menu),
            InputFocus::Game => unreachable!(),
            InputFocus::Menu => input.set_focus(InputFocus::Console),
        }
    }
    String::new()
}

fn connect<A>(server_addrs: A) -> Result<Connection, ClientError>
where
    A: ToSocketAddrs,
{
    let mut con_sock = ConnectSocket::bind("0.0.0.0:0")?;
    let server_addr = match server_addrs.to_socket_addrs() {
        Ok(ref mut a) => a.next().ok_or(ClientError::InvalidServerAddress),
        Err(_) => Err(ClientError::InvalidServerAddress),
    }?;

    let mut response = None;

    for attempt in 0..MAX_CONNECT_ATTEMPTS {
        println!(
            "Connecting...(attempt {} of {})",
            attempt + 1,
            MAX_CONNECT_ATTEMPTS
        );
        con_sock.send_request(
            Request::connect(net::GAME_NAME, CONNECT_PROTOCOL_VERSION),
            server_addr,
        )?;

        // TODO: get rid of magic constant (2.5 seconds wait time for response)
        match con_sock.recv_response(Some(Duration::milliseconds(2500))) {
            Err(err) => {
                match err {
                    // if the message is invalid, log it but don't quit
                    // TODO: this should probably disconnect
                    NetError::InvalidData(msg) => error!("{}", msg),

                    // other errors are fatal
                    e => return Err(e.into()),
                }
            }

            Ok(opt) => {
                if let Some((resp, remote)) = opt {
                    // if this response came from the right server, we're done
                    if remote == server_addr {
                        response = Some(resp);
                        break;
                    }
                }
            }
        }
    }

    let port = match response.ok_or(ClientError::NoResponse)? {
        Response::Accept(accept) => {
            // validate port number
            if accept.port < 0 || accept.port >= std::u16::MAX as i32 {
                Err(ClientError::InvalidConnectPort(accept.port))?;
            }

            debug!("Connection accepted on port {}", accept.port);
            accept.port as u16
        }

        // our request was rejected.
        Response::Reject(reject) => Err(ClientError::ConnectionRejected(reject.message))?,

        // the server sent back a response that doesn't make sense here (i.e. something other
        // than an Accept or Reject).
        _ => Err(ClientError::InvalidConnectResponse)?,
    };

    let mut new_addr = server_addr;
    new_addr.set_port(port);

    // we're done with the connection socket, so turn it into a QSocket with the new address
    let qsock = con_sock.into_qsocket(new_addr);

    Ok(Connection {
        state: ClientState::new(),
        kind: ConnectionKind::Server {
            qsock: todo!(),
            compose: Vec::new(),
        },
        conn_state: ConnectionState::SignOn(SignOnStage::Prespawn),
    })
}

// TODO: when an audio device goes down, every command with an
// OutputStreamHandle needs to be reconstructed so it doesn't pass out
// references to a dead output stream

// TODO: this will hang while connecting. ideally, input should be handled in a
// separate thread so the OS doesn't think the client has gone unresponsive.
fn cmd_connect(args: &[&str], world: &mut World) -> String {
    let world = world.cell();
    let mut conn = world.resource_mut::<GameConnection>();
    let mut input = world.resource_mut::<Input>();

    if args.len() < 1 {
        // TODO: print to console
        return "usage: connect <server_ip>:<server_port>".to_owned();
    }

    match connect(args[0]) {
        Ok(new_conn) => {
            conn.0 = Some(new_conn);
            input.set_focus(InputFocus::Game);
            String::new()
        }
        Err(e) => format!("{}", e),
    }
}

fn cmd_reconnect(args: &[&str], world: &mut World) -> String {
    let world = world.cell();
    let mut conn = world.resource_mut::<GameConnection>();
    let mut input = world.resource_mut::<Input>();

    match &mut conn.0 {
        Some(conn) => {
            // TODO: clear client state
            conn.conn_state = ConnectionState::SignOn(SignOnStage::Prespawn);
            input.set_focus(InputFocus::Game);
            String::new()
        }
        // TODO: log message, e.g. "can't reconnect while disconnected"
        None => "not connected".to_string(),
    }
}

fn cmd_disconnect(_: &[&str], world: &mut World) -> String {
    let world = world.cell();
    let mut conn = world.resource_mut::<GameConnection>();
    let mut input = world.resource_mut::<Input>();
    let connected = conn.0.is_some();
    if connected {
        conn.0 = None;
        input.set_focus(InputFocus::Console);
        String::new()
    } else {
        "not connected".to_string()
    }
}

fn cmd_playdemo(args: &[&str], world: &mut World) -> String {
    let world = world.cell();
    let mut conn = world.resource_mut::<GameConnection>();
    let vfs = world.resource::<Vfs>();
    let mut input = world.resource_mut::<Input>();

    if args.len() != 1 {
        return "usage: playdemo [DEMOFILE]".to_owned();
    }

    let demo = args[0];

    let new_conn = {
        let mut demo_file = match vfs.open(format!("{}.dem", demo)) {
            Ok(f) => f,
            Err(e) => {
                return format!("{}", e);
            }
        };

        match DemoServer::new(&mut demo_file) {
            Ok(d) => Connection {
                kind: ConnectionKind::Demo(d),
                state: ClientState::new(),
                conn_state: ConnectionState::SignOn(SignOnStage::Prespawn),
            },
            Err(e) => {
                return format!("{}", e);
            }
        }
    };

    input.set_focus(InputFocus::Game);

    conn.0 = Some(new_conn);

    String::new()
}

fn cmd_startdemos(args: &[&str], world: &mut World) -> String {
    if args.len() == 0 {
        return "usage: startdemos [DEMOS]".to_owned();
    }

    let world = world.cell();

    let mut demo_queue = world.resource_mut::<DemoQueue>();
    let mut conn = world.resource_mut::<GameConnection>();
    let mut input = world.resource_mut::<Input>();
    let vfs = world.resource::<Vfs>();
    demo_queue.0 = args.into_iter().map(|s| s.to_string()).collect();

    let new_conn = match demo_queue.0.pop_front() {
        Some(demo) => {
            let mut demo_file = match vfs
                .open(format!("{}.dem", demo))
                .or_else(|_| vfs.open(format!("demos/{}.dem", demo)))
            {
                Ok(f) => f,
                Err(e) => {
                    // log the error, dump the demo queue and disconnect
                    demo_queue.0.clear();
                    return format!("{}", e);
                }
            };

            match DemoServer::new(&mut demo_file) {
                Ok(d) => Connection {
                    kind: ConnectionKind::Demo(d),
                    state: ClientState::new(),
                    conn_state: ConnectionState::SignOn(SignOnStage::Prespawn),
                },
                Err(e) => {
                    demo_queue.0.clear();
                    return format!("{}", e);
                }
            }
        }

        // if there are no more demos in the queue, disconnect
        None => return "usage: startdemos [DEMOS]".to_owned(),
    };

    input.set_focus(InputFocus::Game);

    conn.0 = Some(new_conn);

    String::new()
}

fn cmd_music(args: &[&str], world: &mut World) -> String {
    if args.len() != 1 {
        return "usage: music [TRACKNAME]".to_owned();
    }

    world.send_event(MixerEvent::StartMusic(Some(MusicSource::Named(
        args[0].to_owned(),
    ))));
    // TODO: Handle failure correctly
    // match res {
    //     Ok(()) => String::new(),
    //     Err(e) => {
    //         music_player.stop(commands);
    //         format!("{}", e)
    //     }
    // }
    String::new()
}

fn cmd_music_stop(_: &[&str], world: &mut World) -> String {
    world.send_event(MixerEvent::StopMusic);
    String::new()
}

fn cmd_music_pause(_: &[&str], world: &mut World) -> String {
    world.send_event(MixerEvent::PauseMusic);
    String::new()
}

fn cmd_music_resume(_: &[&str], world: &mut World) -> String {
    world.send_event(MixerEvent::StartMusic(None));
    String::new()
}
