//! Engine ports for the LIVE cells.
//!
//! A cell starts from the local Engine A ports and follows
//! `WORLD_GAME_PORT` / `WORLD_HTTP_PORT` when another lab engine is
//! selected; these are the names the Quester path cells already use. A cell
//! that owns a dedicated variable (`RELOG_GAME_PORT`, `GATHERER_GAME_PORT`,
//! `BOT_GAME_PORT`, `M3_PORT`) keeps reading that variable.

/// A `u16` port from `name`, or `default` when it is unset or invalid.
pub fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// The game port: `WORLD_GAME_PORT`, defaulting to Engine A's 44594.
pub fn game_port() -> u16 {
    env_port("WORLD_GAME_PORT", 44_594)
}

/// The asset (HTTP) port: `WORLD_HTTP_PORT`, defaulting to Engine A's 1080.
pub fn http_port() -> u16 {
    env_port("WORLD_HTTP_PORT", 1_080)
}
