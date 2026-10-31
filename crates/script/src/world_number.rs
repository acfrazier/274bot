//! Pure public-world selection for the narrow catalog client-config projection.
//!
//! The caller supplies `location.host` and the values from
//! `URLSearchParams.getAll("world")`. No navigation or world-hop action is
//! performed here.

/// The upstream `resolveWorldNumber` error for an invalid or repeated override.
pub const INVALID_WORLD: &str = "world must be 1, 2 or 3";

/// Resolve one explicit world override, or infer it from a browser host.
///
/// The host is the browser's canonical `location.host`, so a valid optional
/// port is stripped before matching the three public world names.
pub fn resolve_world_number(
    host: &str,
    explicit: Option<&str>,
    repeated: bool,
) -> Result<u8, &'static str> {
    if repeated {
        return Err(INVALID_WORLD);
    }
    if let Some(value) = explicit {
        return match value {
            "1" => Ok(1),
            "2" => Ok(2),
            "3" => Ok(3),
            _ => Err(INVALID_WORLD),
        };
    }

    let hostname = host.split_once(':').map_or(host, |(hostname, _)| hostname);
    Ok(if hostname.eq_ignore_ascii_case("w1.rs2b2t.com") {
        1
    } else if hostname.eq_ignore_ascii_case("w2.rs2b2t.com") {
        2
    } else if hostname.eq_ignore_ascii_case("w3.rs2b2t.com") {
        3
    } else {
        1
    })
}

#[cfg(test)]
mod tests {
    use super::{resolve_world_number, INVALID_WORLD};

    #[test]
    fn explicit_world_overrides_the_host() {
        assert_eq!(
            resolve_world_number("w2.rs2b2t.com", Some("1"), false),
            Ok(1)
        );
        assert_eq!(
            resolve_world_number("w1.rs2b2t.com", Some("3"), false),
            Ok(3)
        );
    }

    #[test]
    fn invalid_and_repeated_world_values_are_rejected() {
        assert_eq!(
            resolve_world_number("w2.rs2b2t.com", Some("4"), false),
            Err(INVALID_WORLD)
        );
        assert_eq!(
            resolve_world_number("w2.rs2b2t.com", Some("01"), false),
            Err(INVALID_WORLD)
        );
        assert_eq!(
            resolve_world_number("w2.rs2b2t.com", None, true),
            Err(INVALID_WORLD)
        );
    }

    #[test]
    fn recognized_hosts_ignore_case_and_valid_ports() {
        assert_eq!(
            resolve_world_number("w1.rs2b2t.com:443", None, false),
            Ok(1)
        );
        assert_eq!(
            resolve_world_number("W2.RS2B2T.COM:8080", None, false),
            Ok(2)
        );
        assert_eq!(resolve_world_number("w3.rs2b2t.com", None, false), Ok(3));
    }

    #[test]
    fn unrecognized_hosts_default_to_world_one() {
        assert_eq!(resolve_world_number("localhost:8081", None, false), Ok(1));
        assert_eq!(resolve_world_number("example.invalid", None, false), Ok(1));
    }
}
