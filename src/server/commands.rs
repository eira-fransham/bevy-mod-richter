use bevy::prelude::*;
use failure::Error;

use crate::{
    client::{input::InputFocus, Connection, ConnectionState},
    common::{
        console::{ExecResult, RegisterCmdExt},
        net::SignOnStage,
    },
};
use failure::bail;

use super::*;

pub fn register_commands(app: &mut App) {
    app.command(
        "map",
        cmd_map.map(|res| -> ExecResult {
            if let Err(e) = res {
                format!("{}", e).into()
            } else {
                default()
            }
        }),
        "Load and start a new map",
    );
}

fn cmd_map(
    In(args): In<Box<[String]>>,
    mut commands: Commands,
    session: Option<ResMut<Session>>,
    mut focus: ResMut<InputFocus>,
    vfs: Res<Vfs>,
    mut registry: ResMut<Registry>,
) -> Result<(), Error> {
    let map = match &*args {
        [map_name] => map_name,
        _ => bail!("usage: map [MAP_NAME]"),
    };
    let bsp = vfs.open(format!("maps/{}.bsp", map))?;
    let (models, entmap) = crate::common::bsp::load(bsp)?;
    let progs = vfs.open("progs.dat")?;
    let progs = crate::server::progs::load(progs)?;

    if let Some(mut session) = session {
        session.state = SessionState::Loading;
        session.level = LevelState::new(progs, models, entmap, registry.reborrow(), &*vfs);
    } else {
        // TODO: Make `max_clients` a cvar
        commands.insert_resource(Session::new(
            8,
            registry.reborrow(),
            &*vfs,
            progs,
            models,
            entmap,
        ));
    }

    // TODO: This should not be handled here, server and client should be decoupled
    commands.insert_resource(Connection::new_server());
    commands.insert_resource(ConnectionState::SignOn(SignOnStage::Prespawn));
    *focus = InputFocus::Game;

    Ok(())
}
