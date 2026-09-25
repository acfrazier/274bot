use super::{
    apply_load_browse_select, breadcrumb_prefixes, card_columns, card_rect_activated, card_width,
    chip_frame_padding, chip_text_color, dialog_date_color, dialog_rows, display_category,
    format_mtime, move_category, name_matches_search, needs_rs2b0t_catalog_prompt,
    resolve_category_order, rs2b0t_root_has_index, sidebar_places, sort_dialog_rows, DialogMode,
    DialogRow, DialogSort, LoadBrowseEntry, CARD_GAP, CARD_MIN_W, CHIP_PAD_Y, GLYPH_CHEVRON,
    GLYPH_DESKTOP, GLYPH_DOCUMENTS, GLYPH_DOWNLOADS, GLYPH_FILE, GLYPH_FOLDER, GLYPH_HOME,
    UNCATEGORIZED,
};
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

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
fn rs2b0t_root_has_index_checks_catalog_file() {
    let dir = std::env::temp_dir().join(format!("274bot-panel-index-check-{}", std::process::id()));
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
    assert!(!names.contains(&"readme.md"));
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
fn selected_chip_text_is_black_on_amber() {
    assert_eq!(chip_text_color(true), [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(chip_text_color(false), crate::theme::TEXT);
}

#[test]
fn chip_frame_padding_keeps_descenders() {
    const {
        assert!(CHIP_PAD_Y >= 4.0);
    }
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
