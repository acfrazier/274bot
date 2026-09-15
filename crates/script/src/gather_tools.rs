//! Native gather-tool selection over the posted `api::gather_tools` rows.
//!
//! Pickaxe use is gated by the mining `levelrequire` row; axes have no
//! woodcutting use gate. Bronze is the tutorial wield path with no Attack
//! gate and unknown names cannot wield. The identity/use/wield rows stay in
//! `api::gather_tools` as posted facts — selection policy lives here, not in
//! the JS shim.

use api::gather_tools::{GatherTool, AXES, PICKAXES};

fn meets_use(tool: &GatherTool, level: i32) -> bool {
    match (tool.use_skill, tool.use_level) {
        (Some("mining"), Some(need)) => level >= need,
        _ => true,
    }
}

/// First best-first pickaxe whose mining use level is met and `available`.
pub fn best_pickaxe(level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    PICKAXES
        .iter()
        .find_map(|tool| (meets_use(tool, level) && available(tool.name)).then_some(tool.name))
}

/// First best-first axe that `available` accepts. `level` is unused (no WC gate).
pub fn best_axe(_level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    AXES.iter()
        .find_map(|tool| available(tool.name).then_some(tool.name))
}

fn named(name: &str) -> Option<&'static GatherTool> {
    let name = name.trim();
    AXES.iter().chain(PICKAXES).find(|tool| tool.name == name)
}

/// Unknown names are unavailable. Bronze tutorial path has no Attack gate.
pub fn can_wield_tool(name: &str, attack: i32) -> bool {
    match named(name) {
        None => false,
        Some(tool) => match tool.wield_attack {
            None => true,
            Some(need) => attack >= need,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn best_pickaxe_uses_mining_level_and_available_only() {
        let steel = |name: &str| name == "Steel pickaxe";
        assert_eq!(best_pickaxe(30, steel), Some("Steel pickaxe"));
        assert_eq!(best_pickaxe(5, steel), None);
        assert_eq!(
            best_pickaxe(1, |name| name == "Bronze pickaxe"
                || name == "Steel pickaxe"),
            Some("Bronze pickaxe")
        );
        assert_eq!(best_pickaxe(30, |_| false), None);
        let mut skipped = Vec::new();
        assert_eq!(
            best_pickaxe(30, |name| {
                skipped.push(name.to_string());
                false
            }),
            None
        );
        assert_eq!(
            skipped,
            [
                "Mithril pickaxe",
                "Steel pickaxe",
                "Iron pickaxe",
                "Bronze pickaxe",
            ]
        );
        let mut calls = Vec::new();
        let hit = best_pickaxe(99, |name| {
            calls.push(name.to_string());
            name == "Steel pickaxe"
        });
        assert_eq!(hit, Some("Steel pickaxe"));
        assert_eq!(
            calls,
            [
                "Rune pickaxe",
                "Adamant pickaxe",
                "Mithril pickaxe",
                "Steel pickaxe",
            ]
        );
    }

    #[test]
    fn best_axe_has_no_woodcutting_gate_and_keeps_black() {
        assert_eq!(
            best_axe(1, |name| name == "Rune axe" || name == "Steel axe"),
            Some("Rune axe")
        );
        let mut calls = Vec::new();
        let hit = best_axe(1, |name| {
            calls.push(name.to_string());
            name == "Steel axe"
        });
        assert_eq!(hit, Some("Steel axe"));
        assert_eq!(
            calls,
            [
                "Rune axe",
                "Adamant axe",
                "Mithril axe",
                "Black axe",
                "Steel axe",
            ]
        );
        assert_eq!(best_axe(99, |name| name == "Bank-only mithril"), None);
    }

    #[test]
    fn can_wield_is_attack_gate_and_unknown_is_unavailable() {
        assert!(!can_wield_tool("Steel pickaxe", 1));
        assert!(can_wield_tool("Steel pickaxe", 5));
        assert!(!can_wield_tool("Steel axe", 1));
        assert!(can_wield_tool("Steel axe", 5));
        assert!(can_wield_tool("Bronze pickaxe", 0));
        assert!(can_wield_tool("Bronze axe", 0));
        assert!(can_wield_tool("Black axe", 10));
        assert!(!can_wield_tool("Black axe", 9));
        assert!(!can_wield_tool("Dragon pickaxe", 99));
        assert!(!can_wield_tool("", 99));
        assert!(!can_wield_tool("steel pickaxe", 99));
    }
}
