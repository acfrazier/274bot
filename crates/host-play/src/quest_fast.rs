//! Safety guard for world-wide tick-speed changes.

/// Engine Q's dedicated loopback game endpoint. `::speed` changes the whole
/// world, so other 289 endpoints are never eligible for fast-path control.
pub const ENGINE_Q_HOST: &str = "127.0.0.1";
pub const ENGINE_Q_GAME_PORT: u16 = 44694;

/// Refuse a world tick-speed command unless the bound endpoint is Engine Q.
pub fn validate_tick_speed_target(host: &str, port: u16) -> Result<(), String> {
    if host == ENGINE_Q_HOST && port == ENGINE_Q_GAME_PORT {
        return Ok(());
    }
    Err(format!(
        "world tick-speed changes are allowed only on Engine Q at {ENGINE_Q_HOST}:{ENGINE_Q_GAME_PORT}; refusing {host}:{port}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_speed_guard_accepts_only_engine_q() {
        assert!(validate_tick_speed_target(ENGINE_Q_HOST, ENGINE_Q_GAME_PORT).is_ok());
        let engine_a = validate_tick_speed_target("127.0.0.1", 44594).unwrap_err();
        assert!(engine_a.contains("refusing 127.0.0.1:44594"));
        let builder = validate_tick_speed_target("127.0.0.1", 45594).unwrap_err();
        assert!(builder.contains("refusing 127.0.0.1:45594"));
        assert!(validate_tick_speed_target("localhost", ENGINE_Q_GAME_PORT).is_err());
    }
}
