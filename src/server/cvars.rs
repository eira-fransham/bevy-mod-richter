use bevy::app::App;

use crate::common::console::RegisterCmdExt;

pub fn register_cvars(app: &mut App) {
    app.cvar("sv_paused", "0", "1 if the server is paused, 0 otherwise")
        .cvar(
            "teamplay",
            "1",
            "0: deathmatch, 1: co-op (friendly fire disabled), 2: co-op (friendly fire enabled)",
        )
        .cvar("skill", "1", "0: easy, 1: normal, 2: hard, 3: nightmare")
        .cvar("sv_gravity", "800", "Gravity strength")
        .cvar("sv_maxvelocity", "2000", "Maximum velocity of entities");
}
