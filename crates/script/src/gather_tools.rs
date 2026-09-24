//! Native gather-tool selection over the `api::gather_tools` rows.
//!
//! Pickaxe use is gated by the mining `levelrequire` row; axes have no
//! woodcutting use gate. Bronze is the tutorial wield path with no Attack
//! gate and unknown names cannot wield. The identity/use/wield rows stay in
//! `api::gather_tools` as selected-revision facts — selection policy lives
//! here, not in the JS shim.
//!
//! The rs2b0t `Tools` exports that take caller callbacks (`bestAxe`,
//! `bestPickaxe`, …) are one native call each (`load::tools_v8`): that helper
//! calls [`best_tool`] with the script's own `available` callback and level
//! conversion, so candidate order, the use gate and the first-hit exit are
//! decided here while each callback runs exactly where the frozen
//! `bestFromTiers` loop would reach it.

use api::gather_tools::{GatherTool, AXES, PICKAXES};
use std::convert::Infallible;

/// Which best-first table a selection walks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Axe,
    Pickaxe,
}

impl ToolKind {
    /// The candidates, best first.
    pub fn candidates(self) -> &'static [GatherTool] {
        match self {
            ToolKind::Axe => AXES,
            ToolKind::Pickaxe => PICKAXES,
        }
    }
}

/// The skill level a candidate's own use gate requires: mining pickaxes only.
fn use_gate(tool: &GatherTool) -> Option<i32> {
    match (tool.use_skill, tool.use_level) {
        (Some("mining"), Some(need)) => Some(need),
        _ => None,
    }
}

/// First best-first candidate whose use gate `meets` and that `available`
/// accepts. `meets(cx, need)` is asked only at a gated candidate, and
/// `available` only for a candidate that passed its gate; the first accepted
/// candidate ends the walk. `cx` is the caller's context (the V8 scope for a
/// script callback), threaded through both answers; an `Err` from either ends
/// the walk and is returned unchanged.
pub fn best_tool<C: ?Sized, E>(
    cx: &mut C,
    kind: ToolKind,
    mut meets: impl FnMut(&mut C, i32) -> Result<bool, E>,
    mut available: impl FnMut(&mut C, &'static str) -> Result<bool, E>,
) -> Result<Option<&'static str>, E> {
    for tool in kind.candidates() {
        if let Some(need) = use_gate(tool) {
            if !meets(cx, need)? {
                continue;
            }
        }
        if available(cx, tool.name)? {
            return Ok(Some(tool.name));
        }
    }
    Ok(None)
}

fn infallible<T>(result: Result<T, Infallible>) -> T {
    match result {
        Ok(value) => value,
        Err(never) => match never {},
    }
}

/// First best-first pickaxe whose mining use level is met and `available`.
pub fn best_pickaxe(level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    infallible(best_tool(
        &mut (),
        ToolKind::Pickaxe,
        |_, need| Ok(level >= need),
        |_, name| Ok(available(name)),
    ))
}

/// First best-first axe that `available` accepts. `level` is unused (no WC gate).
pub fn best_axe(level: i32, mut available: impl FnMut(&str) -> bool) -> Option<&'static str> {
    infallible(best_tool(
        &mut (),
        ToolKind::Axe,
        |_, need| Ok(level >= need),
        |_, name| Ok(available(name)),
    ))
}

/// The Attack gate for an exact tool name: `None` for an unknown name,
/// `Some(None)` for a tool with no wield gate (bronze), else the level.
pub fn wield_gate(name: &str) -> Option<Option<i32>> {
    let name = name.trim();
    AXES.iter()
        .chain(PICKAXES)
        .find(|tool| tool.name == name)
        .map(|tool| tool.wield_attack)
}

/// Unknown names are unavailable. Bronze tutorial path has no Attack gate.
pub fn can_wield_tool(name: &str, attack: i32) -> bool {
    match wield_gate(name) {
        None => false,
        Some(None) => true,
        Some(Some(need)) => attack >= need,
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
