//! Script Browse picker helpers: category order, rs2b0t catalog prompt, badges.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use script::{ScriptKind, ScriptSource};

/// Browse uses a non-modal [`dear_imgui_rs::Ui::window`], not a popup modal.
pub const BROWSE_WINDOW_TITLE: &str = "Scripts";

/// Font Awesome Free Solid PUA, merged into the default atlas the same way
/// the rail merges DejaVu. Drawn as `ui.text` / button prefixes — no PushFont.
pub const GLYPH_HOME: &str = "\u{f015}";
pub const GLYPH_DESKTOP: &str = "\u{f390}";
pub const GLYPH_DOCUMENTS: &str = "\u{f15c}";
pub const GLYPH_DOWNLOADS: &str = "\u{f019}";
pub const GLYPH_FOLDER: &str = "\u{f07b}";
pub const GLYPH_FILE: &str = "\u{f15b}";
pub const GLYPH_CHEVRON: &str = "\u{f054}";

/// Scripts window `FirstUseEver` size (same as WalkTo unconstrained).
pub const SCRIPTS_FIRST_W: f32 = 720.0;
pub const SCRIPTS_FIRST_H: f32 = 560.0;
/// Shared file dialog `FirstUseEver`.
pub const FILE_DIALOG_FIRST_W: f32 = 640.0;
pub const FILE_DIALOG_FIRST_H: f32 = 480.0;

pub const CARD_MIN_W: f32 = 220.0;
pub const CARD_GAP: f32 = 8.0;
/// Unselected card descriptions clip to this many wrapped lines.
pub const CARD_DESC_LINES: usize = 3;

pub const UNCATEGORIZED: &str = "Uncategorized";

/// Height of a card description: full when selected, else at most
/// [`CARD_DESC_LINES`] line-heights.
pub fn card_desc_height(line_h: f32, selected: bool, full_h: f32) -> f32 {
    let full = full_h.max(0.0);
    if selected {
        full
    } else {
        full.min(line_h.max(0.0) * CARD_DESC_LINES as f32)
    }
}

/// File dialog mode: Load a script file, or import a catalog folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogMode {
    File,
    Folder,
}

/// One place-button on the file-dialog sidebar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebarPlace {
    pub label: &'static str,
    pub glyph: &'static str,
    pub path: PathBuf,
}

/// One table row in the shared file dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialogRow {
    pub name: String,
    pub is_dir: bool,
    pub mtime: Option<SystemTime>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialogSort {
    Name,
    Date,
}

/// Category drawn on cards / tabs. Empty registry category is Uncategorized.
pub fn display_category(category: &str) -> &str {
    if category.is_empty() {
        UNCATEGORIZED
    } else {
        category
    }
}

/// Fluid card columns: at least one, then as many `min_w` tiles as fit.
pub fn card_columns(avail: f32, min_w: f32, gap: f32) -> usize {
    if avail <= 0.0 || min_w <= 0.0 {
        return 1;
    }
    let n = ((avail + gap) / (min_w + gap)).floor() as usize;
    n.max(1)
}

/// Stretch card width so `cols` tiles fill `avail` with `gap` between them.
pub fn card_width(avail: f32, cols: usize, gap: f32) -> f32 {
    let cols = cols.max(1) as f32;
    (avail - gap * (cols - 1.0)).max(1.0) / cols
}

/// Remaining width for a card title after reserving `badge_w` on the right.
pub fn title_clip_width(inner: f32, badge_w: f32, gap: f32) -> f32 {
    (inner - badge_w - gap).max(1.0)
}

/// True when `chip_w` would overflow `avail` given `used` on this line.
pub fn chip_wraps(used: f32, chip_w: f32, gap: f32, avail: f32) -> bool {
    used > 0.0 && used + gap + chip_w > avail
}

/// Extra Y pad so descenders (g, y, p, q, j) are not clipped. `SmallButton`
/// zeros FramePadding.y; category chips must not.
pub const CHIP_PAD_Y: f32 = 4.0;

/// Keep the current X pad; never let Y drop below [`CHIP_PAD_Y`].
pub fn chip_frame_padding(current: [f32; 2]) -> [f32; 2] {
    [current[0], current[1].max(CHIP_PAD_Y)]
}

/// Selected chips fill `#FFB000`; body text `#ddd` fails contrast, so
/// the label is black. Unselected keeps the chrome body color.
pub fn chip_text_color(selected: bool) -> [f32; 4] {
    if selected {
        [0.0, 0.0, 0.0, 1.0]
    } else {
        crate::theme::TEXT
    }
}

/// Card bodies are nested text / desc children; `is_item_clicked` on the
/// outer child only fires on leftover padding. Activate on mouse-up over
/// the painted rect (WalkTo's `is_mouse_hovering_rect`), not a drag-scroll.
pub fn card_rect_activated(hovering: bool, released: bool, dragging: bool) -> bool {
    hovering && released && !dragging
}

/// Status on the card currently at the front of the warmup queue.
pub fn card_transpile_label(
    front: Option<(ScriptSource, &str)>,
    source: ScriptSource,
    name: &str,
    done: usize,
    total: usize,
) -> Option<String> {
    let (front_source, front_name) = front?;
    if front_source != source || front_name != name {
        return None;
    }
    if total > 1 {
        Some(format!("transpiling… {}/{}", done + 1, total))
    } else {
        Some("transpiling…".into())
    }
}

/// Home / Desktop / Documents / Downloads under `home`, skipping missing paths.
pub fn sidebar_places(home: &Path) -> Vec<SidebarPlace> {
    let candidates = [
        ("Home", GLYPH_HOME, home.to_path_buf()),
        ("Desktop", GLYPH_DESKTOP, home.join("Desktop")),
        ("Documents", GLYPH_DOCUMENTS, home.join("Documents")),
        ("Downloads", GLYPH_DOWNLOADS, home.join("Downloads")),
    ];
    candidates
        .into_iter()
        .filter(|(_, _, path)| path.exists())
        .map(|(label, glyph, path)| SidebarPlace { label, glyph, path })
        .collect()
}

/// Clickable breadcrumb prefixes for `path` (root first).
pub fn breadcrumb_prefixes(path: &Path) -> Vec<(String, PathBuf)> {
    let mut acc = PathBuf::new();
    let mut out = Vec::new();
    for comp in path.components() {
        acc.push(comp.as_os_str());
        let label = match comp {
            std::path::Component::RootDir => "/".to_string(),
            std::path::Component::Prefix(p) => p.as_os_str().to_string_lossy().into_owned(),
            std::path::Component::Normal(s) => s.to_string_lossy().into_owned(),
            std::path::Component::CurDir | std::path::Component::ParentDir => continue,
        };
        out.push((label, acc.clone()));
    }
    out
}

pub fn name_matches_search(name: &str, query: &str) -> bool {
    let q = query.trim();
    q.is_empty() || name.to_lowercase().contains(&q.to_lowercase())
}

/// Directory listing for the shared dialog. File mode: dirs + `.ts`/`.js`.
/// Folder mode: dirs only. Dotfiles skipped.
pub fn dialog_rows(dir: &Path, mode: DialogMode, search: &str) -> Vec<DialogRow> {
    let mut rows = Vec::new();
    let Ok(read) = std::fs::read_dir(dir) else {
        return rows;
    };
    for entry in read.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if !name_matches_search(&name, search) {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            rows.push(DialogRow {
                name,
                is_dir: true,
                mtime: entry.metadata().ok().and_then(|m| m.modified().ok()),
            });
        } else if mode == DialogMode::File && is_load_script_file(&name) {
            rows.push(DialogRow {
                name,
                is_dir: false,
                mtime: entry.metadata().ok().and_then(|m| m.modified().ok()),
            });
        }
    }
    sort_dialog_rows(&mut rows, DialogSort::Name, false);
    rows
}

pub fn sort_dialog_rows(rows: &mut [DialogRow], col: DialogSort, desc: bool) {
    rows.sort_by(|a, b| {
        let ord = match col {
            DialogSort::Name => a
                .is_dir
                .cmp(&b.is_dir)
                .reverse()
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())),
            DialogSort::Date => a.mtime.cmp(&b.mtime).then_with(|| a.name.cmp(&b.name)),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

/// UTC `YYYY-MM-DD HH:MM` from mtime, empty if unknown.
pub fn format_mtime(t: Option<SystemTime>) -> String {
    let Some(t) = t else {
        return String::new();
    };
    let Ok(d) = t.duration_since(UNIX_EPOCH) else {
        return String::new();
    };
    let s = d.as_secs();
    let days = (s / 86400) as i32;
    let rem = s % 86400;
    let h = rem / 3600;
    let m = (rem % 3600) / 60;
    let (y, mo, day) = civil_from_days(days);
    format!("{y:04}-{mo:02}-{day:02} {h:02}:{m:02}")
}

/// Load date-column color. Odd `ROW_BG` stripes are the light alt fill;
/// disabled gray washes out there, so unselected dates are black.
/// The selected/highlighted row keeps dim text (its fill is already dark).
pub fn dialog_date_color(selected: bool, alt_row: bool) -> [f32; 4] {
    if selected || !alt_row {
        crate::theme::TEXT_DIM
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Howard Hinnant's civil_from_days (UTC).
fn civil_from_days(days: i32) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i32 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (y + if m <= 2 { 1 } else { 0 }, m, d)
}

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
        ScriptKind::Compat | ScriptKind::NativeTick => "JS",
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

/// Kind and source as the card paints them: `[JS] [Catalog]`.
pub fn card_kind_source(kind: ScriptKind, source: ScriptSource) -> String {
    format!("[{}] [{}]", kind_badge(kind), source_badge(source))
}

/// FirstUseEver origin: center `win` in the game pane (viewport minus the
/// right chrome strip). Keeps Scripts/Load floating over the blit so they
/// do not spawn as a Game tab.
pub fn overlay_first_pos(
    viewport_pos: [f32; 2],
    viewport_size: [f32; 2],
    right_strip: f32,
    win: [f32; 2],
) -> [f32; 2] {
    let game_w = (viewport_size[0] - right_strip).max(0.0);
    [
        viewport_pos[0] + ((game_w - win[0]) * 0.5).max(0.0),
        viewport_pos[1] + ((viewport_size[1] - win[1]) * 0.5).max(0.0),
    ]
}

/// Window-relative X so `n` buttons of `btn_w` sit centred in `avail`.
pub fn centered_row_x(avail: f32, n: usize, btn_w: f32, gap: f32) -> f32 {
    let n = n.max(1) as f32;
    let total = btn_w * n + gap * (n - 1.0);
    ((avail - total) * 0.5).max(0.0)
}

/// True when the operator has not set `$RS2B0T`, has no persisted root, and
/// has not deferred the first-run catalog import.
pub fn needs_rs2b0t_catalog_prompt(rs2b0t_root: Option<&Path>, import_deferred: bool) -> bool {
    rs2b0t_root.is_none() && !import_deferred
}

/// True when `root/src/bot/scripts/index.ts` exists.
pub fn rs2b0t_root_has_index(root: &Path) -> bool {
    script::registry_index_path(root).is_file()
}

/// Default Load browser directory: last dir from prefs, else the process
/// working directory (where the OS started the app), else `$HOME`.
pub fn default_load_browse_dir(last: Option<&Path>) -> PathBuf {
    last.filter(|p| p.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| std::env::current_dir().ok().filter(|p| p.is_dir()))
        .or_else(|| {
            let home = script::bot_home();
            home.exists().then_some(home)
        })
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn is_load_script_file(name: &str) -> bool {
    name.ends_with(".ts") || name.ends_with(".js")
}

/// One row in the out-of-tree Load file browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadBrowseEntry {
    Up,
    Subdir(String),
    File(String),
}

/// Directory listing for the Load picker (subdirs + `.ts`/`.js` files).
pub fn load_browse_entries(dir: &Path) -> Vec<LoadBrowseEntry> {
    let mut out = vec![LoadBrowseEntry::Up];
    let mut subdirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(read) = std::fs::read_dir(dir) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                subdirs.push(name);
            } else if is_load_script_file(&name) {
                files.push(name);
            }
        }
    }
    subdirs.sort();
    files.sort();
    for name in subdirs {
        out.push(LoadBrowseEntry::Subdir(name));
    }
    for name in files {
        out.push(LoadBrowseEntry::File(name));
    }
    out
}

/// Apply a Load browser row click (Up → parent, Subdir → descend, File → select).
pub fn apply_load_browse_select(
    dir: &mut PathBuf,
    sel: &mut usize,
    entry: &LoadBrowseEntry,
    index: usize,
) {
    match entry {
        LoadBrowseEntry::Up => {
            if let Some(parent) = dir.parent() {
                *dir = parent.to_path_buf();
                *sel = 0;
            }
        }
        LoadBrowseEntry::Subdir(name) => {
            dir.push(name);
            *sel = 0;
        }
        LoadBrowseEntry::File(_) => {
            *sel = index;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        apply_load_browse_select, breadcrumb_prefixes, card_columns, card_rect_activated,
        card_width, chip_frame_padding, chip_text_color, dialog_date_color, dialog_rows,
        display_category, format_mtime, move_category, name_matches_search,
        needs_rs2b0t_catalog_prompt, resolve_category_order, rs2b0t_root_has_index, sidebar_places,
        sort_dialog_rows, DialogMode, DialogRow, DialogSort, LoadBrowseEntry, BROWSE_WINDOW_TITLE,
        CARD_GAP, CARD_MIN_W, CHIP_PAD_Y, FILE_DIALOG_FIRST_H, FILE_DIALOG_FIRST_W, GLYPH_CHEVRON,
        GLYPH_DESKTOP, GLYPH_DOCUMENTS, GLYPH_DOWNLOADS, GLYPH_FILE, GLYPH_FOLDER, GLYPH_HOME,
        SCRIPTS_FIRST_H, SCRIPTS_FIRST_W, UNCATEGORIZED,
    };
    use std::path::{Path, PathBuf};
    use std::time::{Duration, UNIX_EPOCH};

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
            !APP.contains(&format!(
                "begin_modal_popup_config(\"{BROWSE_WINDOW_TITLE}\")"
            )),
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
        assert!(!needs_rs2b0t_catalog_prompt(
            Some(Path::new("/tmp/x")),
            false
        ));
        assert!(!needs_rs2b0t_catalog_prompt(None, true));
    }

    #[test]
    fn load_browse_up_navigates_to_parent() {
        let mut dir = PathBuf::from("/tmp/274bot-load-up-child");
        let mut sel = 2;
        apply_load_browse_select(&mut dir, &mut sel, &LoadBrowseEntry::Up, 0);
        assert_eq!(dir, PathBuf::from("/tmp"));
        assert_eq!(sel, 0);
    }

    #[test]
    fn load_window_has_no_free_text_path_field() {
        const APP: &str = include_str!("app.rs");
        assert!(
            !APP.contains("input_text(\"##load-path\""),
            "Load must not use a free-text path field"
        );
        assert!(
            APP.contains("script_load_dir"),
            "Load must browse directories and remember the last dir"
        );
    }

    #[test]
    fn rs2b0t_root_has_index_checks_catalog_file() {
        let dir =
            std::env::temp_dir().join(format!("274bot-panel-index-check-{}", std::process::id()));
        let root = dir.join("rs2b0t");
        let scripts = root.join("src/bot/scripts");
        std::fs::create_dir_all(&scripts).unwrap();
        assert!(!rs2b0t_root_has_index(&root));
        std::fs::write(scripts.join("index.ts"), "// empty").unwrap();
        assert!(rs2b0t_root_has_index(&root));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn file_dialog_glyphs_are_fa_pua() {
        assert_eq!(GLYPH_HOME, "\u{f015}");
        assert_eq!(GLYPH_DESKTOP, "\u{f390}");
        assert_eq!(GLYPH_DOCUMENTS, "\u{f15c}");
        assert_eq!(GLYPH_DOWNLOADS, "\u{f019}");
        assert_eq!(GLYPH_FOLDER, "\u{f07b}");
        assert_eq!(GLYPH_FILE, "\u{f15b}");
        assert_eq!(GLYPH_CHEVRON, "\u{f054}");
    }

    #[test]
    fn card_wrap_two_then_three_columns() {
        assert_eq!(card_columns(500.0, CARD_MIN_W, CARD_GAP), 2);
        assert_eq!(card_columns(720.0, CARD_MIN_W, CARD_GAP), 3);
        let w = card_width(720.0, 3, CARD_GAP);
        assert!((w * 3.0 + CARD_GAP * 2.0 - 720.0).abs() < 0.01);
        assert!(w >= CARD_MIN_W);
    }

    #[test]
    fn empty_category_is_uncategorized() {
        assert_eq!(display_category(""), UNCATEGORIZED);
        assert_eq!(display_category("Combat"), "Combat");
    }

    #[test]
    fn sidebar_skips_missing_dirs() {
        let home = std::env::temp_dir().join(format!("274bot-sidebar-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("Desktop")).unwrap();
        std::fs::create_dir_all(home.join("Downloads")).unwrap();
        let places = sidebar_places(&home);
        let labels: Vec<_> = places.iter().map(|p| p.label).collect();
        assert_eq!(labels, vec!["Home", "Desktop", "Downloads"]);
        assert!(!labels.contains(&"Documents"));
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn breadcrumbs_are_prefixes() {
        let crumbs = breadcrumb_prefixes(Path::new("/tmp/foo/bar"));
        assert_eq!(crumbs.first().map(|(l, _)| l.as_str()), Some("/"));
        assert_eq!(
            crumbs.last().map(|(l, p)| (l.as_str(), p.as_path())),
            Some(("bar", Path::new("/tmp/foo/bar")))
        );
        assert!(crumbs
            .iter()
            .any(|(l, p)| l == "tmp" && p == Path::new("/tmp")));
    }

    #[test]
    fn search_filters_case_insensitive() {
        assert!(name_matches_search("Alcher.ts", "alc"));
        assert!(name_matches_search("Alcher.ts", ""));
        assert!(!name_matches_search("Alcher.ts", "thiev"));
    }

    #[test]
    fn dialog_rows_file_vs_folder_and_search() {
        let dir = std::env::temp_dir().join(format!("274bot-dialog-rows-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("bot.ts"), "x").unwrap();
        std::fs::write(dir.join("readme.md"), "x").unwrap();
        let files = dialog_rows(&dir, DialogMode::File, "");
        let names: Vec<_> = files.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"src") && names.contains(&"bot.ts"));
        assert!(!names.iter().any(|n| *n == "readme.md"));
        let folders = dialog_rows(&dir, DialogMode::Folder, "");
        assert!(folders.iter().all(|r| r.is_dir));
        let filtered = dialog_rows(&dir, DialogMode::File, "BOT");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].name, "bot.ts");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sort_dialog_rows_name_keeps_dirs_first() {
        let mut rows = vec![
            DialogRow {
                name: "z.ts".into(),
                is_dir: false,
                mtime: None,
            },
            DialogRow {
                name: "b".into(),
                is_dir: true,
                mtime: None,
            },
            DialogRow {
                name: "a".into(),
                is_dir: true,
                mtime: None,
            },
        ];
        sort_dialog_rows(&mut rows, DialogSort::Name, false);
        assert_eq!(
            rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "z.ts"]
        );
    }

    #[test]
    fn format_mtime_epoch_is_utc() {
        assert_eq!(
            format_mtime(Some(UNIX_EPOCH + Duration::from_secs(0))),
            "1970-01-01 00:00"
        );
        assert_eq!(format_mtime(None), "");
    }

    #[test]
    fn load_date_column_is_black_on_alt_row_not_when_selected() {
        assert_eq!(dialog_date_color(false, true), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(dialog_date_color(true, true), crate::theme::TEXT_DIM);
        assert_eq!(dialog_date_color(true, false), crate::theme::TEXT_DIM);
        assert_eq!(dialog_date_color(false, false), crate::theme::TEXT_DIM);
        const APP: &str = include_str!("app.rs");
        let body = APP.split("fn file_dialog_body").nth(1).unwrap_or("");
        assert!(
            body.contains("dialog_date_color") && body.contains("text_colored"),
            "Load date cells must not stay text_disabled on the light ROW_BG stripe"
        );
        assert!(
            !body.contains("text_disabled(format_mtime"),
            "date column is contrast-colored, not globally disabled"
        );
    }

    #[test]
    fn scripts_window_first_size_is_walkto() {
        const APP: &str = include_str!("app.rs");
        assert!(
            APP.contains("SCRIPTS_FIRST_W") && APP.contains("SCRIPTS_FIRST_H"),
            "Scripts FirstUseEver must use SCRIPTS_FIRST_W/H (720×560)"
        );
        assert!(
            !APP.contains(".size([480.0, 520.0], Condition::FirstUseEver)"),
            "old 480×520 Scripts size must go"
        );
        assert_eq!((SCRIPTS_FIRST_W, SCRIPTS_FIRST_H), (720.0, 560.0));
        assert_eq!((FILE_DIALOG_FIRST_W, FILE_DIALOG_FIRST_H), (640.0, 480.0));
    }

    #[test]
    fn scripts_grid_does_not_list_compiled_ids() {
        const APP: &str = include_str!("app.rs");
        let body = APP.split("fn browse_window_body").nth(1).unwrap_or("");
        assert!(
            !body.contains("compiled_ids"),
            "Scripts grid is JsCard only; WalkTo stays host nav"
        );
        assert!(
            APP.contains("FILE_DIALOG_FIRST_W"),
            "shared file dialog must use FILE_DIALOG_FIRST_W/H"
        );
    }

    #[test]
    fn kind_badge_is_js_for_catalog_cards() {
        use script::{ScriptKind, ScriptSource};
        assert_eq!(super::kind_badge(ScriptKind::Compat), "JS");
        assert_eq!(super::kind_badge(ScriptKind::NativeTick), "JS");
        assert_eq!(super::source_badge(ScriptSource::Catalog), "Catalog");
        assert_eq!(
            super::card_kind_source(ScriptKind::Compat, ScriptSource::Catalog),
            "[JS] [Catalog]"
        );
    }

    #[test]
    fn overlay_first_pos_centers_in_game_pane() {
        let pos = super::overlay_first_pos([0.0, 0.0], [1095.0, 600.0], 330.0, [720.0, 560.0]);
        // Game pane is 765×600; 720×560 sits at ((765-720)/2, (600-560)/2).
        assert!((pos[0] - 22.5).abs() < 0.01);
        assert!((pos[1] - 20.0).abs() < 0.01);
    }

    #[test]
    fn centered_row_x_centers_two_buttons() {
        let x = super::centered_row_x(600.0, 2, 80.0, 6.0);
        assert!((x - 217.0).abs() < 0.01);
    }

    #[test]
    fn default_load_browse_dir_uses_process_cwd() {
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(super::default_load_browse_dir(None), cwd);
        let last = cwd.join("crates");
        if last.is_dir() {
            assert_eq!(super::default_load_browse_dir(Some(&last)), last);
        }
    }

    #[test]
    fn scripts_card_paints_kind_source_accent_title_wrapping_tags() {
        const APP: &str = include_str!("app.rs");
        let card = APP.split("fn browse_script_card").nth(1).unwrap_or("");
        assert!(
            card.contains("card_kind_source"),
            "cards must paint kind [JS] and source [Catalog]"
        );
        assert!(
            card.contains("text_colored(ACCENT") || card.contains("text_colored(ACCENT,"),
            "script title uses the same accent as the selected profile/script"
        );
        assert!(
            card.contains("TEXT_DIM") && card.contains("unloadable"),
            "unloadable cards dim the title; selection still works"
        );
        assert!(
            card.contains("text_wrapped") && card.contains("tags"),
            "tags wrap"
        );
        assert!(
            card.contains("CARD_DESC_LINES") || card.contains("card_desc_height"),
            "unselected descriptions clip to 3 lines"
        );
        assert!(
            !card.contains("text_wrapped(&card_kind_source")
                && !card.contains("text_wrapped(card_kind_source"),
            "kind/source must clip on one line, not wrap"
        );
        assert!(
            card.contains("title_clip_width"),
            "title clips so [JS] [Catalog] keeps its width"
        );
        assert!(
            card.contains("card_transpile_label") || card.contains("transpiling"),
            "first-click warmup paints transpiling… on the card"
        );
        assert!(
            card.contains("select_script_card"),
            "card click selects and queues that card only"
        );
    }

    #[test]
    fn title_clip_width_reserves_badge() {
        let w = super::title_clip_width(220.0, 90.0, 8.0);
        assert!((w - 122.0).abs() < 0.01);
        assert_eq!(super::title_clip_width(80.0, 90.0, 8.0), 1.0);
    }

    #[test]
    fn chip_wraps_when_the_next_label_would_overflow() {
        assert!(!super::chip_wraps(0.0, 80.0, 8.0, 200.0));
        assert!(!super::chip_wraps(100.0, 80.0, 8.0, 200.0));
        assert!(super::chip_wraps(120.0, 80.0, 8.0, 200.0));
    }

    #[test]
    fn unselected_description_clips_to_three_lines() {
        assert_eq!(super::CARD_DESC_LINES, 3);
        assert!((super::card_desc_height(16.0, false, 100.0) - 48.0).abs() < 0.01);
        assert!((super::card_desc_height(16.0, true, 100.0) - 100.0).abs() < 0.01);
        assert!((super::card_desc_height(16.0, false, 20.0) - 20.0).abs() < 0.01);
    }

    #[test]
    fn card_transpile_label_only_for_the_front_of_the_queue() {
        use script::ScriptSource;
        assert_eq!(
            super::card_transpile_label(None, ScriptSource::Catalog, "BoneBurier", 0, 1),
            None
        );
        assert_eq!(
            super::card_transpile_label(
                Some((ScriptSource::Catalog, "BoneBurier")),
                ScriptSource::Catalog,
                "BoneBurier",
                0,
                1
            ),
            Some("transpiling…".into())
        );
        assert_eq!(
            super::card_transpile_label(
                Some((ScriptSource::Catalog, "BoneBurier")),
                ScriptSource::Catalog,
                "BoneBurier",
                2,
                10
            ),
            Some("transpiling… 3/10".into())
        );
        assert_eq!(
            super::card_transpile_label(
                Some((ScriptSource::Catalog, "BoneBurier")),
                ScriptSource::Catalog,
                "ShopRunner",
                0,
                1
            ),
            None
        );
    }

    #[test]
    fn scripts_window_no_horizontal_scroll_or_tab_chevrons() {
        const APP: &str = include_str!("app.rs");
        let browse = APP.split("fn browse_window(").nth(1).unwrap_or("");
        let browse_fn = browse.split("fn persist_dialog_cwd").next().unwrap_or("");
        assert!(
            browse_fn.contains("NO_SCROLLBAR") && !browse_fn.contains("HORIZONTAL_SCROLLBAR"),
            "Scripts clips extra width instead of a horizontal bar"
        );
        let body = APP.split("fn browse_window_body").nth(1).unwrap_or("");
        let body_fn = body.split("fn overlay_right_strip").next().unwrap_or("");
        assert!(
            !body_fn.contains("tab_bar") && !body_fn.contains("FittingPolicy"),
            "categories are wrapping chips, not a shrinking imgui tab bar"
        );
        let chips = APP.split("fn script_category_chips").nth(1).unwrap_or("");
        let chips_fn = chips.split("fn category_chip_dnd").next().unwrap_or("");
        assert!(
            body_fn.contains("script_category_chips")
                && chips_fn.contains("chip_wraps")
                && chips_fn.contains("\"All\""),
            "All + category chips wrap onto the next line"
        );
        assert!(
            !chips_fn.contains("small_button"),
            "SmallButton zeros FramePadding.y and clips descenders (y, g)"
        );
        assert!(
            chips_fn.contains("chip_frame_padding")
                && chips_fn.contains("chip_text_color")
                && chips_fn.contains("FrameBorderSize")
                && chips_fn.contains("StyleColor::Border"),
            "chips are regular buttons: extra Y pad, black-on-amber when selected, amber outline"
        );
        let card = APP.split("fn browse_script_card").nth(1).unwrap_or("");
        let card_fn = card.split("fn browse_card_grid").next().unwrap_or("");
        assert!(
            card_fn.contains("card_rect_activated") && card_fn.contains("is_mouse_hovering_rect"),
            "card click is the painted rect, not leftover child padding"
        );
        assert!(
            !card_fn.contains("is_item_clicked"),
            "is_item_clicked on the card child misses title/desc/tag hits"
        );
        assert!(
            body_fn.contains("queue_transpile_all") && body_fn.contains("Transpile all"),
            "Scripts offers an explicit all-at-once warmup, not a click that burns the catalog"
        );
        let grid = APP.split("fn browse_card_grid").nth(1).unwrap_or("");
        assert!(
            grid.contains("same_line_with_spacing") && grid.contains("CARD_GAP"),
            "card rows must use CARD_GAP, not default item spacing"
        );
    }

    #[test]
    fn scripts_and_file_dialog_spawn_over_game_not_docked() {
        const APP: &str = include_str!("app.rs");
        let browse = APP.split("fn browse_window(").nth(1).unwrap_or("");
        let browse_fn = browse.split("fn persist_dialog_cwd").next().unwrap_or("");
        assert!(
            (browse_fn.contains("overlay_first_pos") || browse_fn.contains("overlay_spawn_pos"))
                && browse_fn.contains("FirstUseEver"),
            "Scripts FirstUseEver must sit over the game pane"
        );
        assert!(
            !browse_fn.contains("set_next_window_dock_id"),
            "Scripts must not auto-dock into the game node"
        );
        let dialog = APP.split("fn file_dialog_windows").nth(1).unwrap_or("");
        let dialog_fn = dialog.split("fn file_dialog_body").next().unwrap_or("");
        assert!(
            dialog_fn.contains("overlay_first_pos") || dialog_fn.contains("overlay_spawn_pos"),
            "Load/Import spawn over the game pane like Scripts"
        );
        assert!(
            dialog_fn.contains("centered_row_x") || APP.contains("centered_row_x"),
            "Load/Cancel are centered"
        );
    }

    #[test]
    fn general_and_nav_config_dock_into_panel() {
        const APP: &str = include_str!("app.rs");
        assert!(
            APP.contains("set_next_window_dock_id_with_cond")
                && APP.contains("Condition::FirstUseEver"),
            "General/Nav config FirstUseEver dock into the 274bot panel node"
        );
        let settings = APP.split("fn settings_window").nth(1).unwrap_or("");
        assert!(
            settings.contains("set_next_window_dock_id_with_cond"),
            "General config docks as a panel tab"
        );
        let nav = APP.split("fn nav_settings_window").nth(1).unwrap_or("");
        assert!(
            nav.contains("set_next_window_dock_id_with_cond"),
            "Nav config docks as a panel tab"
        );
    }

    #[test]
    fn ui_frame_pumps_transpile_queue() {
        const APP: &str = include_str!("app.rs");
        let frame = APP.split("fn ui_frame").nth(1).unwrap_or("");
        assert!(
            frame.contains("pump_script_transpile"),
            "one catalog file per frame — do not transpile the world on the click"
        );
    }

    #[test]
    fn selected_chip_text_is_black_on_amber() {
        assert_eq!(chip_text_color(true), [0.0, 0.0, 0.0, 1.0]);
        assert_eq!(chip_text_color(false), crate::theme::TEXT);
    }

    #[test]
    fn chip_frame_padding_keeps_descenders() {
        assert!(CHIP_PAD_Y >= 4.0);
        assert_eq!(chip_frame_padding([4.0, 0.0]), [4.0, CHIP_PAD_Y]);
        assert_eq!(chip_frame_padding([4.0, 8.0]), [4.0, 8.0]);
    }

    #[test]
    fn card_rect_activated_is_release_on_the_painted_rect() {
        assert!(card_rect_activated(true, true, false));
        assert!(
            !card_rect_activated(false, true, false),
            "miss when the cursor is not on the card"
        );
        assert!(
            !card_rect_activated(true, true, true),
            "a drag-scroll is not a select"
        );
        assert!(!card_rect_activated(true, false, false));
    }
}
