//! Sidecar rail chrome: window geometry, the tile size, and the traffic
//! light that colours each member's cap dot.

/// Width of the MultiBox sidecar rail (rs2b0t's 264px strip).
pub const RAIL_W: f32 = 264.0;
/// Cap-body tile draw size inside the rail (rs2b0t ~236×155).
pub const TILE_W: f32 = 236.0;
pub const TILE_H: f32 = 155.0;

/// Cap dot state: error red wins, then ingame amber, then FIFO-queue warn,
/// else grey (title screen, nothing pending).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Light {
    Amber,
    Red,
    Warn,
    Grey,
}

impl Light {
    /// The cap dot's fill color (amber CRT palette).
    pub fn rgb(&self) -> [f32; 4] {
        match self {
            Light::Amber => crate::theme::ACCENT,
            Light::Red => crate::theme::ERROR,
            Light::Warn => crate::theme::WARN,
            Light::Grey => crate::theme::TEXT_DIM,
        }
    }
}

/// Map a slot's status to its tile's traffic light.
pub fn traffic_light(ingame: bool, error: bool, queued: bool) -> Light {
    if error {
        Light::Red
    } else if ingame {
        Light::Amber
    } else if queued {
        Light::Warn
    } else {
        Light::Grey
    }
}

#[cfg(test)]
mod tests {
    use super::{traffic_light, Light, RAIL_W, TILE_H, TILE_W};

    #[test]
    fn rail_constants_match_the_plan() {
        assert_eq!(RAIL_W, 264.0);
        assert_eq!(TILE_W, 236.0);
        assert_eq!(TILE_H, 155.0);
        assert_eq!(crate::theme::RAIL_WINDOW, "274bot-rail");
    }

    #[test]
    fn traffic_light_maps_status_to_cap_color() {
        assert_eq!(traffic_light(true, false, false), Light::Amber);
        assert_eq!(traffic_light(false, true, false), Light::Red);
        assert_eq!(traffic_light(false, false, true), Light::Warn);
        assert_eq!(traffic_light(false, false, false), Light::Grey);
        // Precedence: error red wins, then ingame amber, then queue warn.
        assert_eq!(traffic_light(true, true, true), Light::Red);
        assert_eq!(traffic_light(true, false, true), Light::Amber);
        assert_eq!(traffic_light(false, true, true), Light::Red);
    }
}
