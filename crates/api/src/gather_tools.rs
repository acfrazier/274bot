//! Host-owned selected 274/289 gather-tool identity, use, and wield facts.
//!
//! Generated game-data rows supply id/name/`wear_position: 3` only. Mining
//! `levelrequire` and Attack opheld2 live in selected content (identical on
//! both revisions for these objects). Axes have no woodcutting use gate.
//! Bronze wield is the tutorial path, not Attack-gated. Black axe 1361 is
//! in the native best-first woodcut list. Unknown names cannot wield.

/// One selected-world gather tool: identity plus use versus wield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatherTool {
    pub id: i32,
    pub name: &'static str,
    pub use_skill: Option<&'static str>,
    pub use_level: Option<i32>,
    pub wield_attack: Option<i32>,
}

const fn pick(
    id: i32,
    name: &'static str,
    use_level: i32,
    wield_attack: Option<i32>,
) -> GatherTool {
    GatherTool {
        id,
        name,
        use_skill: Some("mining"),
        use_level: Some(use_level),
        wield_attack,
    }
}

const fn axe(id: i32, name: &'static str, wield_attack: Option<i32>) -> GatherTool {
    GatherTool {
        id,
        name,
        use_skill: None,
        use_level: None,
        wield_attack,
    }
}

/// Best-first axes: rune → bronze, including native black. No WC use gate.
pub const AXES: &[GatherTool] = &[
    axe(1359, "Rune axe", Some(40)),
    axe(1357, "Adamant axe", Some(30)),
    axe(1355, "Mithril axe", Some(20)),
    axe(1361, "Black axe", Some(10)),
    axe(1353, "Steel axe", Some(5)),
    axe(1349, "Iron axe", Some(1)),
    axe(1351, "Bronze axe", None),
];

/// Best-first pickaxes: rune → bronze. Use is mining `levelrequire`.
pub const PICKAXES: &[GatherTool] = &[
    pick(1275, "Rune pickaxe", 41, Some(40)),
    pick(1271, "Adamant pickaxe", 31, Some(30)),
    pick(1273, "Mithril pickaxe", 21, Some(20)),
    pick(1269, "Steel pickaxe", 6, Some(5)),
    pick(1267, "Iron pickaxe", 0, Some(1)),
    pick(1265, "Bronze pickaxe", 0, None),
];

impl GatherTool {
    fn json(self) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "name": self.name,
            "use_skill": self.use_skill,
            "use_level": self.use_level,
            "wield_attack": self.wield_attack,
        })
    }
}

/// Posted onto `__rs2b0t_host.content.gather_tools`. Same rows on 274 and 289.
pub fn content_json_value() -> serde_json::Value {
    serde_json::json!({
        "axes": AXES.iter().copied().map(GatherTool::json).collect::<Vec<_>>(),
        "pickaxes": PICKAXES.iter().copied().map(GatherTool::json).collect::<Vec<_>>(),
    })
}

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
    use crate::game_data;
    use client::io::ClientRevision;

    fn names(list: &[GatherTool]) -> Vec<&str> {
        list.iter().map(|t| t.name).collect()
    }

    #[test]
    fn selected_274_289_ids_names_and_wearpos_agree() {
        for rev in [ClientRevision::R274, ClientRevision::R289] {
            let data = game_data::for_revision(rev).expect("generated game data");
            for tool in AXES.iter().chain(PICKAXES) {
                let item = data.item_by_id(tool.id).expect(tool.name);
                assert_eq!(item.name.as_deref(), Some(tool.name));
                assert_eq!(item.wear_position, 3, "{}", tool.name);
            }
        }
        assert_eq!(
            names(AXES),
            [
                "Rune axe",
                "Adamant axe",
                "Mithril axe",
                "Black axe",
                "Steel axe",
                "Iron axe",
                "Bronze axe",
            ]
        );
        assert_eq!(
            names(PICKAXES),
            [
                "Rune pickaxe",
                "Adamant pickaxe",
                "Mithril pickaxe",
                "Steel pickaxe",
                "Iron pickaxe",
                "Bronze pickaxe",
            ]
        );
        let steel = PICKAXES.iter().find(|t| t.id == 1269).unwrap();
        assert_eq!(steel.use_skill, Some("mining"));
        assert_eq!(steel.use_level, Some(6));
        assert_eq!(steel.wield_attack, Some(5));
        let steel_axe = AXES.iter().find(|t| t.id == 1353).unwrap();
        assert_eq!(steel_axe.use_skill, None);
        assert_eq!(steel_axe.use_level, None);
        assert_eq!(steel_axe.wield_attack, Some(5));
        let black = AXES.iter().find(|t| t.id == 1361).unwrap();
        assert_eq!(black.name, "Black axe");
        assert_eq!(black.wield_attack, Some(10));
        assert_eq!(
            PICKAXES.iter().find(|t| t.id == 1265).unwrap().use_level,
            Some(0)
        );
        assert_eq!(
            PICKAXES.iter().find(|t| t.id == 1267).unwrap().use_level,
            Some(0)
        );
        assert_eq!(
            PICKAXES.iter().find(|t| t.id == 1273).unwrap().use_level,
            Some(21)
        );
        assert_eq!(
            PICKAXES.iter().find(|t| t.id == 1271).unwrap().use_level,
            Some(31)
        );
        assert_eq!(
            PICKAXES.iter().find(|t| t.id == 1275).unwrap().use_level,
            Some(41)
        );
    }

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
