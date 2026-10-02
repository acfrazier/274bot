//! Host-owned selected 274/289 gather-tool identity, use, and wield facts.
//!
//! Generated game-data aliases supply selected item ids and display names.
//! Mining `levelrequire` and Attack opheld2 live in selected content
//! (identical on both revisions for these objects). Axes have no woodcutting
//! use gate. Bronze wield is the tutorial path, not Attack-gated. Black axe
//! 1361 is in the native best-first woodcut list. Unknown names cannot wield.
//!
//! These rows are posted facts; selection and wield policy live with their
//! script consumer in `script::gather_tools`.

/// One selected-world gather tool: item alias plus use versus wield.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatherTool {
    pub alias: &'static str,
    pub name: &'static str,
    pub use_skill: Option<&'static str>,
    pub use_level: Option<i32>,
    pub wield_attack: Option<i32>,
}

const fn pick(
    alias: &'static str,
    name: &'static str,
    use_level: i32,
    wield_attack: Option<i32>,
) -> GatherTool {
    GatherTool {
        alias,
        name,
        use_skill: Some("mining"),
        use_level: Some(use_level),
        wield_attack,
    }
}

const fn axe(alias: &'static str, name: &'static str, wield_attack: Option<i32>) -> GatherTool {
    GatherTool {
        alias,
        name,
        use_skill: None,
        use_level: None,
        wield_attack,
    }
}

/// Best-first axes: rune → bronze, including native black. No WC use gate.
pub const AXES: &[GatherTool] = &[
    axe("rune_axe", "Rune axe", Some(40)),
    axe("adamant_axe", "Adamant axe", Some(30)),
    axe("mithril_axe", "Mithril axe", Some(20)),
    axe("black_axe", "Black axe", Some(10)),
    axe("steel_axe", "Steel axe", Some(5)),
    axe("iron_axe", "Iron axe", Some(1)),
    axe("bronze_axe", "Bronze axe", None),
];

/// Best-first pickaxes: rune → bronze. Use is mining `levelrequire`.
pub const PICKAXES: &[GatherTool] = &[
    pick("rune_pickaxe", "Rune pickaxe", 41, Some(40)),
    pick("adamant_pickaxe", "Adamant pickaxe", 31, Some(30)),
    pick("mithril_pickaxe", "Mithril pickaxe", 21, Some(20)),
    pick("steel_pickaxe", "Steel pickaxe", 6, Some(5)),
    pick("iron_pickaxe", "Iron pickaxe", 0, Some(1)),
    pick("bronze_pickaxe", "Bronze pickaxe", 0, None),
];

impl GatherTool {
    fn json(self, data: &crate::game_data::SelectedGameData) -> Option<serde_json::Value> {
        let id = data.item_by_alias(self.alias)?.id;
        Some(serde_json::json!({
            "id": id,
            "name": self.name,
            "use_skill": self.use_skill,
            "use_level": self.use_level,
            "wield_attack": self.wield_attack,
        }))
    }
}

/// Posted onto `__rs2b0t_host.content.gather_tools` from selected item aliases.
pub fn content_json_value(data: Option<&crate::game_data::SelectedGameData>) -> serde_json::Value {
    let tools = |rows: &[GatherTool]| {
        data.map_or_else(Vec::new, |data| {
            rows.iter()
                .copied()
                .filter_map(|tool| tool.json(data))
                .collect()
        })
    };
    serde_json::json!({
        "axes": tools(AXES),
        "pickaxes": tools(PICKAXES),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game_data;
    use client::io::ClientRevision;

    #[test]
    fn posted_tool_ids_follow_selected_alias_rows() {
        for revision in [ClientRevision::R274, ClientRevision::R289] {
            let data = game_data::for_revision(revision).expect("generated game data");
            let posted = content_json_value(Some(data.as_ref()));
            for (kind, tools) in [("axes", AXES), ("pickaxes", PICKAXES)] {
                let rows = posted[kind].as_array().expect("tool rows");
                assert_eq!(rows.len(), tools.len(), "{revision:?} {kind}");
                for (row, tool) in rows.iter().zip(tools) {
                    let selected = data.item_by_alias(tool.alias).expect(tool.alias);
                    assert_eq!(row["id"], selected.id, "{revision:?} {}", tool.alias);
                    assert_eq!(row["name"], selected.name.as_deref().unwrap());
                }
            }
        }
    }
}
