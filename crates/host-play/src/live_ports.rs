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

fn parse_required_port(name: &str, value: Option<&str>) -> Result<u16, String> {
    let value = value.ok_or_else(|| format!("{name} is required"))?;
    value
        .parse()
        .map_err(|_| format!("{name} must be a valid u16 port, got {value:?}"))
}

/// A configured `u16` port from `name`; missing or invalid values are errors.
pub fn required_port(name: &str) -> Result<u16, String> {
    let value = std::env::var(name).ok();
    parse_required_port(name, value.as_deref())
}

/// The game port: `WORLD_GAME_PORT`, defaulting to Engine A's 44594.
pub fn game_port() -> u16 {
    env_port("WORLD_GAME_PORT", 44_594)
}

/// The asset (HTTP) port: `WORLD_HTTP_PORT`, defaulting to Engine A's 1080.
pub fn http_port() -> u16 {
    env_port("WORLD_HTTP_PORT", 1_080)
}
#[cfg(test)]
mod tests {
    use super::parse_required_port;

    #[test]
    fn required_port_reports_a_missing_variable() {
        assert_eq!(
            parse_required_port("WORLD_GAME_PORT", None).unwrap_err(),
            "WORLD_GAME_PORT is required"
        );
    }

    #[test]
    fn required_port_parses_a_configured_value() {
        assert_eq!(
            parse_required_port("WORLD_HTTP_PORT", Some("3080")).unwrap(),
            3080
        );
    }

    #[test]
    fn required_port_rejects_an_invalid_value() {
        assert!(parse_required_port("WORLD_GAME_PORT", Some("not-a-port"))
            .unwrap_err()
            .contains("WORLD_GAME_PORT"));
    }
}
