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
#[path = "script_picker_tests.rs"]
mod tests;
