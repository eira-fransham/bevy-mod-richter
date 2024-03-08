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

pub mod commands;
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

use self::{
    input::RichterInputPlugin,
    menu::{MenuBodyView, MenuBuilder, MenuView},
    render::{RenderResolution, RichterRenderPlugin},
    sound::{MixerEvent, RichterSoundPlugin},
};

use std::{collections::VecDeque, io::BufReader, net::ToSocketAddrs, path::PathBuf};

use crate::{
    client::{
        demo::{DemoServer, DemoServerError},
        entity::{particle::CreateParticle, ClientEntity, MAX_STATIC_ENTITIES},
        sound::{MusicPlayer, StartSound, StartStaticSound, StopSound},
        state::{ClientState, PlayerInfo},
        trace::{TraceEntity, TraceFrame},
        view::{IdleVars, KickVars, MouseVars, RollVars},
    },
    common::{
        self,
        console::{ConsoleError, ConsoleOutput, RichterConsolePlugin},
        engine,
        model::{Model, ModelError},
        net::{
            self,
            connect::{ConnectSocket, Request, Response, CONNECT_PROTOCOL_VERSION},
            BlockingMode, ClientCmd, ClientStat, EntityEffects, EntityState, GameType, NetError,
            PlayerColor, QSocket, ServerCmd, SignOnStage,
        },
        vfs::{Vfs, VfsError},
    },
};
use fxhash::FxHashMap;

use bevy::{
    asset::AssetServer,
    ecs::{
        event::EventWriter,
        system::{Res, ResMut, Resource},
    },
    prelude::*,
    render::extract_resource::ExtractResource,
    time::{Time, Virtual},
    window::PrimaryWindow,
};
use chrono::Duration;
use input::InputFocus;
use menu::Menu;
use num_derive::FromPrimitive;
use serde::Deserialize;
use sound::SoundError;
use thiserror::Error;
use view::BobVars;

// connections are tried 3 times, see
// https://github.com/id-Software/Quake/blob/master/WinQuake/net_dgrm.c#L1248
const MAX_CONNECT_ATTEMPTS: usize = 3;
const MAX_STATS: usize = 32;

const DEFAULT_SOUND_PACKET_VOLUME: u8 = 255;
const DEFAULT_SOUND_PACKET_ATTENUATION: f32 = 1.0;

const CONSOLE_DIVIDER: &str = "\
\n\n\
\x1D\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1E\
\x1E\x1E\x1E\x1E\x1E\x1E\x1E\x1F\
\n\n";

#[derive(Default)]
pub struct RichterPlugin<
    F = Box<dyn Fn(MenuBuilder) -> Result<Menu, failure::Error> + Send + Sync + 'static>,
> {
    pub base_dir: Option<PathBuf>,
    pub game: Option<String>,
    pub main_menu: F,
}

fn build_default(builder: MenuBuilder) -> Result<Menu, failure::Error> {
    Ok(builder.build(MenuView {
        draw_plaque: true,
        title_path: "gfx/ttl_main.lmp".into(),
        body: MenuBodyView::Predefined {
            path: "gfx/mainmenu.lmp".into(),
        },
    }))
}

impl RichterPlugin {
    pub fn new() -> Self {
        Self {
            base_dir: None,
            game: None,
            main_menu: Box::new(build_default),
        }
    }
}

#[derive(Clone, Resource, ExtractResource)]
pub struct RichterGameSettings {
    pub base_dir: PathBuf,
    pub game: Option<String>,
}

impl<F> Plugin for RichterPlugin<F>
where
    F: Fn(MenuBuilder) -> Result<Menu, failure::Error> + Clone + Send + Sync + 'static,
{
    fn build(&self, app: &mut bevy::prelude::App) {
        if let Ok(menu) = (self.main_menu)(MenuBuilder::new(&mut app.world)) {
            app.insert_resource(menu);
        }

        let app = app
            .insert_resource(RichterGameSettings {
                base_dir: self
                    .base_dir
                    .clone()
                    .unwrap_or_else(|| common::default_base_dir()),
                game: self.game.clone(),
            })
            .init_resource::<Vfs>()
            .init_resource::<MusicPlayer>()
            .init_resource::<DemoQueue>()
            .add_event::<Impulse>()
            // TODO: Use bevy's state system
            .insert_resource(ConnectionState::SignOn(SignOnStage::Not))
            .add_systems(
                Main,
                (
                    systems::set_resolution.run_if(any_with_component::<PrimaryWindow>),
                    systems::handle_input.pipe(|In(res)| {
                        // TODO: Error handling
                        if let Err(e) = res {
                            error!("Error handling input: {}", e);
                        }
                    }),
                    systems::frame.pipe(|In(res)| {
                        // TODO: Error handling
                        if let Err(e) = res {
                            error!("Error handling frame: {}", e);
                        }
                    }),
                    systems::update_camera.run_if(resource_exists::<Connection>),
                    state::systems::update_particles,
                ),
            )
            .add_plugins(RichterConsolePlugin)
            .add_plugins(RichterRenderPlugin)
            .add_plugins(RichterSoundPlugin)
            .add_plugins(RichterInputPlugin);

        cvars::register_cvars(app);
        commands::register_commands(app);
    }

    fn finish(&self, app: &mut bevy::prelude::App) {
        app.init_resource::<RenderResolution>();
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

#[derive(Deserialize, Copy, Clone, Debug)]
pub struct MoveVars {
    #[serde(rename(deserialize = "cl_anglespeedkey"))]
    cl_anglespeedkey: f32,
    #[serde(rename(deserialize = "cl_pitchspeed"))]
    cl_pitchspeed: f32,
    #[serde(rename(deserialize = "cl_yawspeed"))]
    cl_yawspeed: f32,
    #[serde(rename(deserialize = "cl_sidespeed"))]
    cl_sidespeed: f32,
    #[serde(rename(deserialize = "cl_upspeed"))]
    cl_upspeed: f32,
    #[serde(rename(deserialize = "cl_forwardspeed"))]
    cl_forwardspeed: f32,
    #[serde(rename(deserialize = "cl_backspeed"))]
    cl_backspeed: f32,
    #[serde(rename(deserialize = "cl_movespeedkey"))]
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

#[derive(Clone, Debug)]
pub struct ConnectedState {
    pub model_precache: im::Vector<Model>,
}

/// Indicates the state of an active connection.
#[derive(Resource, Debug, ExtractResource, Clone)]
pub enum ConnectionState {
    /// The client is in the sign-on process.
    SignOn(SignOnStage),

    /// The client is fully connected.
    Connected(ConnectedState),
}

/// Possible targets that a client can be connected to.
enum ConnectionKind {
    /// A regular Quake server.
    Server {
        /// The [`QSocket`](crate::net::QSocket) used to communicate with the server.
        qsock: QSocket,

        /// The client's packet composition buffer.
        compose: Vec<u8>,
    },

    /// A demo server.
    Demo(DemoServer),
}

/// A connection to a game server of some kind.
///
/// The exact nature of the connected server is specified by [`ConnectionKind`].
#[derive(Resource)]
pub struct Connection {
    state: ClientState,
    kind: ConnectionKind,
}

impl Connection {
    pub fn view_entity_id(&self) -> usize {
        self.state.view_entity_id()
    }

    pub fn trace<'a, I>(&self, entity_ids: I) -> Result<TraceFrame, ClientError>
    where
        I: IntoIterator<Item = &'a usize>,
    {
        let mut trace = TraceFrame {
            msg_times_ms: [
                self.state.msg_times[0].num_milliseconds(),
                self.state.msg_times[1].num_milliseconds(),
            ],
            time_ms: self.state.time.num_milliseconds(),
            lerp_factor: self.state.lerp_factor,
            entities: FxHashMap::default(),
        };

        for id in entity_ids.into_iter() {
            let ent = &self.state.entities[*id];

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

    fn handle_signon(
        &mut self,
        mut state: Mut<ConnectionState>,
        new_stage: SignOnStage,
    ) -> Result<(), ClientError> {
        use SignOnStage::*;

        let new_conn_state = match &*state {
            // TODO: validate stage transition
            ConnectionState::SignOn(_) => {
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
                    Done => ConnectionState::Connected(ConnectedState {
                        model_precache: self.state.models().clone(),
                    }),
                }
            }

            // ignore spurious sign-on messages
            ConnectionState::Connected { .. } => return Ok(()),
        };

        *state = new_conn_state;

        Ok(())
    }

    fn parse_server_msg(
        &mut self,
        mut commands: Commands,
        mut state: Mut<ConnectionState>,
        time: Time,
        vfs: &Vfs,
        asset_server: &AssetServer,
        mixer_events: &mut EventWriter<MixerEvent>,
        mut console_output: Mut<ConsoleOutput>,
        kick_vars: KickVars,
    ) -> Result<ConnectionStatus, ClientError> {
        use ConnectionStatus::*;

        let time = Duration::from_std(time.elapsed()).unwrap();

        let (msg, demo_view_angles, track_override) = match self.kind {
            ConnectionKind::Server { ref mut qsock, .. } => {
                let msg = qsock.recv_msg(match &*state {
                    // if we're in the game, don't block waiting for messages
                    ConnectionState::Connected(_) => BlockingMode::NonBlocking,

                    // otherwise, give the server some time to respond
                    // TODO: might make sense to make this a future or something
                    ConnectionState::SignOn(_) => {
                        BlockingMode::Timeout(Duration::try_seconds(5).unwrap())
                    }
                })?;

                (msg, None, None)
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
                ServerCmd::Bad => {} // panic!("Invalid command from server"),

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
                    console_output.set_center_print(text, self.state.time);
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
                    });
                }

                ServerCmd::FastUpdate(ent_update) => {
                    // first update signals the last sign-on stage
                    self.handle_signon(state.reborrow(), SignOnStage::Done)?;

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
                        255 => commands
                            .spawn(Transform::from_translation(
                                [origin.x, origin.y, origin.z].into(),
                            ))
                            .add(CreateParticle::Explosion),

                        // otherwise it's an impact
                        _ => commands
                            .spawn(Transform::from_translation(
                                [origin.x, origin.y, origin.z].into(),
                            ))
                            .add(CreateParticle::ProjectileImpact {
                                direction: [direction.x, direction.y, direction.z].into(),
                                color,
                                count: count.into(),
                            }),
                    };
                }

                ServerCmd::Print { text } => console_output.print_alert(text, time),

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

                    console_output.println_alert(CONSOLE_DIVIDER, time);
                    console_output.println_alert(message, time);
                    console_output.println_alert(CONSOLE_DIVIDER, time);

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
                    self.handle_signon(state.reborrow(), stage)?;
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
                    let id = self.state.static_entities.len();
                    self.state
                        .static_entities
                        .push_back(ClientEntity::from_baseline(
                            id,
                            EntityState {
                                origin,
                                angles,
                                model_id: model_id as usize,
                                frame_id: frame_id as usize,
                                colormap,
                                skin_id: skin_id as usize,
                                effects: EntityEffects::empty(),
                            },
                        ));
                }

                ServerCmd::SpawnStaticSound {
                    origin,
                    sound_id,
                    volume,
                    attenuation,
                } => {
                    if let Some(sound) = self.state.sounds.get(sound_id as usize) {
                        mixer_events.send(MixerEvent::StartStaticSound(StartStaticSound {
                            src: sound.clone(),
                            origin,
                            volume: volume as f32 / 255.0,
                            attenuation: attenuation as f32 / 64.0,
                        }));
                    }
                }

                ServerCmd::TempEntity { temp_entity } => {
                    self.state
                        .spawn_temp_entity(commands.reborrow(), mixer_events, &temp_entity)
                }

                ServerCmd::StuffText { text: _text } => {} // todo!("Reimplement console"), // console.append_text(text),

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
        mut state: Mut<ConnectionState>,
        mut commands: Commands,
        time: Time,
        vfs: &Vfs,
        asset_server: &AssetServer,
        mixer_events: &mut EventWriter<MixerEvent>,
        mut console: Mut<ConsoleOutput>,
        idle_vars: IdleVars,
        kick_vars: KickVars,
        roll_vars: RollVars,
        bob_vars: BobVars,
        cl_nolerp: bool,
    ) -> Result<ConnectionStatus, ClientError> {
        let frame_time = Duration::from_std(time.delta()).unwrap();
        debug!("frame time: {}ms", frame_time.num_milliseconds());

        // do this _before_ parsing server messages so that we know when to
        // request the next message from the demo server.
        self.state.advance_time(frame_time);
        match self.parse_server_msg(
            commands.reborrow(),
            state.reborrow(),
            time,
            vfs,
            asset_server,
            mixer_events,
            console.reborrow(),
            kick_vars,
        )? {
            ConnectionStatus::Maintain => {}
            // if Disconnect or NextDemo, delegate up the chain
            s => return Ok(s),
        };

        self.state.update_interp_ratio(cl_nolerp);

        // interpolate entity data and spawn particle effects, lights
        self.state.update_entities(commands)?;

        // update temp entities (lightning, etc.)
        self.state.update_temp_entities()?;

        // remove expired lights
        self.state.lights.update(self.state.time);

        if let ConnectionKind::Server {
            ref mut qsock,
            ref mut compose,
        } = self.kind
        {
            // respond to the server
            if qsock.can_send() && !compose.is_empty() {
                qsock.begin_send_msg(&compose)?;
                compose.clear();
            }
        }

        // these all require the player entity to have spawned
        // TODO: Need to improve this code - maybe split it out into surrounding function?
        if let ConnectionState::Connected(_) = &*state {
            // update view
            self.state.calc_final_view(
                idle_vars,
                kick_vars,
                roll_vars,
                if self.state.intermission().is_none() {
                    bob_vars
                } else {
                    default()
                },
            );

            // update camera color shifts for new position/effects
            self.state.update_color_shifts(frame_time)?;
        }

        Ok(ConnectionStatus::Maintain)
    }
}

#[derive(Resource, ExtractResource, Clone, Default)]
pub struct DemoQueue(pub VecDeque<String>);

fn connect<A>(server_addrs: A) -> Result<(Connection, ConnectionState), ClientError>
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
        match con_sock.recv_response(Some(Duration::try_milliseconds(2500).unwrap())) {
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

    Ok((
        Connection {
            state: ClientState::new(),
            kind: ConnectionKind::Server {
                qsock,
                compose: Vec::new(),
            },
        },
        ConnectionState::SignOn(SignOnStage::Prespawn),
    ))
}

#[derive(Event)]
pub struct Impulse(pub u8);

mod systems {
    use serde::Deserialize;

    use self::{
        common::{console::Registry, math::Angles},
        entity::particle::Particle,
    };

    use super::*;

    pub fn handle_input(
        // mut console: ResMut<Console>,
        registry: ResMut<Registry>,
        mut conn: Option<ResMut<Connection>>,
        frame_time: Res<Time<Virtual>>,
        mut impulses: EventReader<Impulse>,
    ) -> Result<(), ClientError> {
        // TODO: Error handling
        let move_vars: MoveVars = registry.read_cvars().unwrap();
        let mouse_vars: MouseVars = registry.read_cvars().unwrap();

        // TODO: Unclear fromm the bevy documentation if this drops all other events for the frame,
        //       but in this case it's almost certainly fine
        let impulse = impulses.read().next().map(|i| i.0);

        match conn.as_deref_mut() {
            Some(Connection {
                ref mut state,
                kind: ConnectionKind::Server { ref mut qsock, .. },
                ..
            }) => {
                let move_cmd = state.handle_input(
                    &*registry,
                    Duration::from_std(frame_time.delta()).unwrap(),
                    move_vars,
                    mouse_vars,
                    impulse,
                );
                let mut msg = Vec::new();
                move_cmd.serialize(&mut msg)?;
                qsock.send_msg_unreliable(&msg)?;

                // TODO: Refresh input (e.g. mouse movement)
            }

            _ => (),
        }

        Ok(())
    }

    #[derive(Deserialize)]
    struct NetworkVars {
        #[serde(rename(deserialize = "cl_nolerp"))]
        disable_lerp: f32,
        #[serde(rename(deserialize = "sv_gravity"))]
        gravity: f32,
    }

    pub fn frame(
        mut commands: Commands,
        cvars: Res<Registry>,
        vfs: Res<Vfs>,
        time: Res<Time<Virtual>>,
        asset_server: Res<AssetServer>,
        mut mixer_events: EventWriter<MixerEvent>,
        mut console: ResMut<ConsoleOutput>,
        mut demo_queue: ResMut<DemoQueue>,
        mut focus: ResMut<InputFocus>,
        mut conn: Option<ResMut<Connection>>,
        mut conn_state: ResMut<ConnectionState>,
    ) -> Result<(), ClientError> {
        let NetworkVars { disable_lerp, .. } = cvars
            .read_cvars()
            .ok_or(ClientError::Cvar(ConsoleError::CvarParseInvalid))?;
        let idle_vars: IdleVars = cvars
            .read_cvars()
            .ok_or(ClientError::Cvar(ConsoleError::CvarParseInvalid))?;
        let kick_vars: KickVars = cvars
            .read_cvars()
            .ok_or(ClientError::Cvar(ConsoleError::CvarParseInvalid))?;
        let roll_vars: RollVars = cvars
            .read_cvars()
            .ok_or(ClientError::Cvar(ConsoleError::CvarParseInvalid))?;
        let bob_vars: BobVars = cvars
            .read_cvars()
            .ok_or(ClientError::Cvar(ConsoleError::CvarParseInvalid))?;

        let status = match conn.as_deref_mut() {
            Some(ref mut conn) => conn.frame(
                conn_state.reborrow(),
                commands.reborrow(),
                time.as_generic(),
                &*vfs,
                &*asset_server,
                &mut mixer_events,
                console.reborrow(),
                idle_vars,
                kick_vars,
                roll_vars,
                bob_vars,
                disable_lerp != 0.,
            )?,
            None => ConnectionStatus::Disconnect,
        };

        use ConnectionStatus::*;
        match status {
            Maintain => (),
            _ => {
                let time = Duration::from_std(time.elapsed()).unwrap();
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
                                        console.println(format!("{}", e), time);
                                        demo_queue.0.clear();
                                        None
                                    }
                                };

                                demo_file.as_mut().and_then(|df| match DemoServer::new(df) {
                                    Ok(d) => Some(Connection {
                                        kind: ConnectionKind::Demo(d),
                                        state: ClientState::new(),
                                    }),
                                    Err(e) => {
                                        console.println(format!("{}", e), time);
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
                    Some(_) => *focus = InputFocus::Game,

                    // don't allow game focus when disconnected
                    None => *focus = InputFocus::Console,
                }

                match (conn, new_conn) {
                    (Some(mut conn), Some(new_conn)) => {
                        *conn = new_conn;
                        *conn_state = ConnectionState::SignOn(SignOnStage::Prespawn);
                    }
                    (None, Some(new_conn)) => {
                        commands.insert_resource(new_conn);
                        *conn_state = ConnectionState::SignOn(SignOnStage::Prespawn);
                    }
                    (Some(_), None) => {
                        commands.remove_resource::<Connection>();
                        *conn_state = ConnectionState::SignOn(SignOnStage::Not);
                    }
                    (None, None) => {}
                }
            }
        }

        Ok(())
    }

    pub fn set_resolution(
        window: Query<&Window, With<PrimaryWindow>>,
        mut target_res: ResMut<RenderResolution>,
    ) {
        let res = &window.single().resolution;
        let res = RenderResolution(res.width() as _, res.height() as _);
        if *target_res != res {
            *target_res = res;
        }
    }

    pub fn update_camera(
        conn: Res<Connection>,
        registry: Res<Registry>,
        mut cameras: Query<(&mut Transform, &mut PerspectiveProjection), With<Camera3d>>,
    ) {
        let Ok(fov) = registry.read_cvar::<f32>("fov") else {
            return;
        };

        let origin = conn.state.view.final_origin();
        // if client is fully connected, draw world
        let angles = match conn.kind {
            ConnectionKind::Demo(..) => {
                conn.state
                    .entities
                    .get(conn.state.view.entity_id())
                    .map(|e| Angles {
                        pitch: e.angles.x,
                        roll: e.angles.z,
                        yaw: e.angles.y,
                    })
            }
            _ => None,
        }
        .unwrap_or(conn.state.view.final_angles());

        let rotation = Quat::from_euler(EulerRot::XZY, angles.pitch.0, angles.roll.0, angles.yaw.0);

        for (mut transform, mut perspective) in &mut cameras {
            transform.rotation = rotation;
            transform.translation = Vec3 {
                x: origin.x,
                y: origin.y,
                z: origin.z,
            };
            perspective.fov = fov;
        }
    }

    pub fn update_particles(
        mut particles: Query<(&mut Transform, &mut Particle)>,
        time: Res<Time<Virtual>>,
        cvars: Res<Registry>,
    ) {
        for (transform, mut p) in &mut particles {
            p.update(
                transform,
                Duration::from_std(time.elapsed()).unwrap(),
                Duration::from_std(time.delta()).unwrap(),
                cvars.read_cvar::<f32>("sv_gravity").unwrap_or(800.),
            );
        }
    }
}
