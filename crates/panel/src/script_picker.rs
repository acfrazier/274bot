//! Script Browse picker helpers: category order, rs2b0t catalog prompt, badges.

use std::path::Path;

use script::{ScriptKind, ScriptSource};

/// Browse uses a non-modal [`dear_imgui_rs::Ui::window`], not a popup modal.
pub const BROWSE_WINDOW_TITLE: &str = "Scripts";

/// Merge persisted category order with categories present on cards. Unknown
/// categories from cards append after the saved order.
pub fn resolve_category_order(saved: &[String], present: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for cat in saved {
        if present.iter().any(|p| p == cat) && !out.iter().any(|x| x == cat) {
            out.push(cat.clone());
        }
    }
    for cat in present {
        if !out.iter().any(|x| x == cat) {
            out.push(cat.clone());
        }
    }
    out
}

/// Move `from` in front of `onto`. No-op if either category is missing.
pub fn move_category(order: &mut Vec<String>, from: &str, onto: &str) {
    if from == onto {
        return;
    }
    let Some(i) = order.iter().position(|x| x == from) else {
        return;
    };
    order.remove(i);
    let at = order.iter().position(|x| x == onto).unwrap_or(order.len());
    order.insert(at, from.to_string());
}

pub fn kind_badge(kind: ScriptKind) -> &'static str {
    match kind {
        ScriptKind::Compat => "Compat",
        ScriptKind::NativeTick => "NativeTick",
        ScriptKind::Compiled => "Compiled",
    }
}

pub fn source_badge(source: ScriptSource) -> &'static str {
    match source {
        ScriptSource::Catalog => "Catalog",
        ScriptSource::File => "File",
        ScriptSource::Builtin => "Builtin",
    }
}

/// True when the operator has not set `$RS2B0T`, has no persisted root, and
/// has not deferred the first-run catalog import.
pub fn needs_rs2b0t_catalog_prompt(
    rs2b0t_root: Option<&Path>,
    import_deferred: bool,
) -> bool {
    rs2b0t_root.is_none() && !import_deferred
}

/// True when `root/src/bot/scripts/index.ts` exists.
pub fn rs2b0t_root_has_index(root: &Path) -> bool {
    script::registry_index_path(root).is_file()
}

#[cfg(test)]
mod tests {
    use super::{
        move_category, needs_rs2b0t_catalog_prompt, resolve_category_order,
        rs2b0t_root_has_index, BROWSE_WINDOW_TITLE,
    };
    use std::path::Path;

    #[test]
    fn browse_picker_uses_window_not_modal() {
        const APP: &str = include_str!("app.rs");
        assert!(
            APP.contains("ui.window(BROWSE_WINDOW_TITLE)"),
            "Browse must use ui.window(BROWSE_WINDOW_TITLE), not a modal popup"
        );
        assert!(
            !APP.contains("begin_modal_popup_config(BROWSE_WINDOW_TITLE)"),
            "Browse must not use begin_modal_popup_config with BROWSE_WINDOW_TITLE"
        );
        assert!(
            !APP.contains(&format!("begin_modal_popup_config(\"{BROWSE_WINDOW_TITLE}\")")),
            "Browse must not use begin_modal_popup_config with the Scripts title"
        );
    }

    #[test]
    fn category_order_merges_saved_and_appends_unknown() {
        let saved = vec!["Prayer".into(), "Combat".into()];
        let present = vec!["Combat".into(), "Skilling".into(), "Prayer".into()];
        assert_eq!(
            resolve_category_order(&saved, &present),
            vec!["Prayer", "Combat", "Skilling"]
        );
    }

    #[test]
    fn category_order_drag_reorder_moves_in_front() {
        let mut order = vec!["A".into(), "B".into(), "C".into()];
        move_category(&mut order, "C", "A");
        assert_eq!(order, vec!["C", "A", "B"]);
        move_category(&mut order, "C", "C");
        assert_eq!(order, vec!["C", "A", "B"]);
    }

    #[test]
    fn needs_catalog_prompt_when_no_root_and_not_deferred() {
        assert!(needs_rs2b0t_catalog_prompt(None, false));
        assert!(!needs_rs2b0t_catalog_prompt(Some(Path::new("/tmp/x")), false));
        assert!(!needs_rs2b0t_catalog_prompt(None, true));
    }

    #[test]
    fn rs2b0t_root_has_index_checks_catalog_file() {
        let dir = std::env::temp_dir().join(format!(
            "274bot-panel-index-check-{}",
            std::process::id()
        ));
        let root = dir.join("rs2b0t");
        let scripts = root.join("src/bot/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        assert!(!rs2b0t_root_has_index(&root));
        std::fs::write(scripts.join("index.ts"), "// empty").unwrap();
        assert!(rs2b0t_root_has_index(&root));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
