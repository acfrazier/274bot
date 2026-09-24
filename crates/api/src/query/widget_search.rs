    //! Snapshot-level button lookups (the m8aq `WidgetSearch.ts`).

    use crate::snapshot::{GameSnapshot, WidgetView};

    /// One combat-style button paired with the nearest text label
    /// (the m8aq `combatStyleLabels` row).
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct CombatStyleLabel {
        pub mode: i32,
        pub label: String,
        pub component_id: i32,
    }

    /// The open roots' and side tabs' widgets under `root_component_id`.
    fn all_widgets(
        snapshot: &GameSnapshot,
        root_component_id: i32,
    ) -> impl Iterator<Item = &WidgetView> + '_ {
        snapshot
            .widgets()
            .iter()
            .chain(
                snapshot
                    .side_tabs()
                    .iter()
                    .flat_map(|tab| tab.widgets.iter()),
            )
            .filter(move |w| w.root_component_id == root_component_id)
    }

    /// The BUTTON_CLOSE component of the root, -1 when none is open.
    pub fn close_button_com_id(snapshot: &GameSnapshot, root_component_id: i32) -> i32 {
        all_widgets(snapshot, root_component_id)
            .find(|w| w.button_type == 3)
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The component whose button text equals `label` (trimmed,
    /// case-insensitive), -1 when none matches.
    pub fn button_by_text(snapshot: &GameSnapshot, root_component_id: i32, label: &str) -> i32 {
        let wanted = label.trim().to_ascii_lowercase();
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_text
                    .as_deref()
                    .is_some_and(|t| t.trim().to_ascii_lowercase() == wanted)
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The BUTTON_TARGET component whose target base equals `base`
    /// (trimmed, case-insensitive), -1 when none matches.
    pub fn target_button_by_base(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        base: &str,
    ) -> i32 {
        let wanted = base.trim().to_ascii_lowercase();
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_type == 2
                    && w.target_base
                        .as_deref()
                        .is_some_and(|t| t.trim().to_ascii_lowercase() == wanted)
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// The BUTTON_SELECT component bound to `varp` = `value`, -1 when
    /// none matches.
    pub fn select_button_by_varp(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        varp: i32,
        value: i32,
    ) -> i32 {
        all_widgets(snapshot, root_component_id)
            .find(|w| {
                w.button_type == 5
                    && w.varp_bindings
                        .iter()
                        .any(|b| b.varp == varp && b.value == Some(value))
            })
            .map(|w| w.component_id)
            .unwrap_or(-1)
    }

    /// True when `label` is one of the melee style names the isolate matches
    /// (`Accurate` / `Aggressive` / `Controlled` / `Defensive`), ignoring
    /// surrounding whitespace and parentheses. 274 combat IFs put that name
    /// on the same row as Punch/Kick; nearest-y alone would post the action.
    fn is_melee_style_label(label: &str) -> bool {
        let n = label
            .trim()
            .trim_matches(|c: char| c == '(' || c == ')')
            .trim()
            .to_ascii_lowercase();
        matches!(
            n.as_str(),
            "accurate" | "aggressive" | "controlled" | "defensive"
        )
    }

    fn nearest_text<'a>(texts: &[(&'a str, i32)], y: i32) -> Option<&'a str> {
        texts
            .iter()
            .min_by_key(|(_, ty)| (*ty - y).abs())
            .map(|(t, _)| *t)
    }

    /// The varp-select buttons of the root with their nearest text
    /// labels, sorted by mode (the m8aq default varp is 43). Prefers a
    /// melee style name already on the IF over a closer action name.
    pub fn combat_style_labels(
        snapshot: &GameSnapshot,
        root_component_id: i32,
        varp: i32,
    ) -> Vec<CombatStyleLabel> {
        let widgets: Vec<&WidgetView> = all_widgets(snapshot, root_component_id).collect();
        let buttons: Vec<(i32, i32, i32)> = widgets
            .iter()
            .filter(|w| w.button_type == 5 && w.varp_bindings.iter().any(|b| b.varp == varp))
            .map(|w| {
                let mode = w
                    .varp_bindings
                    .iter()
                    .find(|b| b.varp == varp)
                    .and_then(|b| b.value)
                    .unwrap_or(0);
                (w.component_id, mode, w.y)
            })
            .collect();
        let texts: Vec<(&str, i32)> = widgets
            .iter()
            .filter_map(|w| {
                w.text
                    .as_deref()
                    .filter(|t| !t.is_empty())
                    .map(|t| (t, w.y))
            })
            .collect();
        let style_texts: Vec<(&str, i32)> = texts
            .iter()
            .copied()
            .filter(|(t, _)| is_melee_style_label(t))
            .collect();
        let mut out: Vec<CombatStyleLabel> = buttons
            .into_iter()
            .map(|(component_id, mode, y)| {
                let label = nearest_text(&style_texts, y)
                    .or_else(|| nearest_text(&texts, y))
                    .unwrap_or_default()
                    .to_string();
                CombatStyleLabel {
                    mode,
                    label,
                    component_id,
                }
            })
            .collect();
        out.sort_by_key(|l| l.mode);
        out
    }