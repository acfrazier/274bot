//! Sidecar rail chrome: window name and the fixed rail width. Task 11
//! paints the member tiles; this task only opens the stub window.

/// Width of the MultiBox sidecar rail (rs2b0t's 264px strip).
pub const RAIL_W: f32 = 264.0;
/// Stable ImGui window name for the rail.
pub const RAIL_WINDOW: &str = "274bot-rail";

#[cfg(test)]
mod tests {
    use super::{RAIL_W, RAIL_WINDOW};

    #[test]
    fn rail_constants_match_the_plan() {
        assert_eq!(RAIL_W, 264.0);
        assert_eq!(RAIL_WINDOW, "274bot-rail");
    }
}
