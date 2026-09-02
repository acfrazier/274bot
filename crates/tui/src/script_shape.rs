//! Script pane (spec `2026-09-01-headless-tui-design.md`): the
//! Browse/Start/Pause/Stop/Load widgets, the Browse picker over registry
//! cards (description, category, tags, kind, source), and the Load path
//! input. The pane is a plain widget over owned view data — it never calls
//! [`SlotScript`]; clicks map to [`crate::app::AppAction`]s that `tui-play`
//! dispatches onto `Play::script_start_load` / pause / stop.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

use script::{JsCard, RunState, ScriptKind, ScriptSel, ScriptSource, SlotScript};

/// The script button labels, left to right (Pause slot is dynamic — see
/// [`pause_button_label`]).
pub const SCRIPT_BUTTONS: [&str; 5] = ["Browse", "Start", "Pause", "Stop", "Load"];

/// Pause/Resume label for the third button (matches panel `script_section`).
pub fn pause_button_label(state: RunState) -> &'static str {
    if state == RunState::Paused {
        "Resume"
    } else {
        "Pause"
    }
}

/// The lifecycle text for a [`RunState`].
pub fn run_state_text(state: RunState) -> &'static str {
    match state {
        RunState::Idle => "idle",
        RunState::Running => "running",
        RunState::Paused => "paused",
        RunState::Stopping => "stopping",
        RunState::Error => "error",
    }
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

/// Display category for a card (empty registry category → Uncategorized).
pub fn card_category(card: &BrowseCard) -> String {
    if card.category.is_empty() {
        "Uncategorized".into()
    } else {
        card.category.clone()
    }
}

/// Lightweight Browse row (no origin/js bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowseCard {
    pub name: String,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub kind: ScriptKind,
    pub source: ScriptSource,
}

impl From<&JsCard> for BrowseCard {
    fn from(card: &JsCard) -> Self {
        Self {
            name: card.name.clone(),
            description: card.description.clone(),
            category: card.category.clone(),
            tags: card.tags.clone(),
            kind: card.kind,
            source: card.source,
        }
    }
}

/// Categories present on `cards`, first-seen order.
pub fn categories_present(cards: &[BrowseCard]) -> Vec<String> {
    let mut out = Vec::new();
    for card in cards {
        let cat = card_category(card);
        if !out.iter().any(|c| c == &cat) {
            out.push(cat);
        }
    }
    out
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

/// One logical line in the Browse list (maps to one terminal row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseLine {
    ImportCatalog,
    Category(String),
    Card(usize),
    Empty,
}

/// Build grouped Browse lines: optional Import row, category headers, cards.
pub fn browse_lines(
    cards: &[BrowseCard],
    category_order: &[String],
    catalog_deferred: bool,
) -> Vec<BrowseLine> {
    let mut lines = Vec::new();
    if catalog_deferred {
        lines.push(BrowseLine::ImportCatalog);
    }
    if cards.is_empty() {
        lines.push(BrowseLine::Empty);
        return lines;
    }
    let present = categories_present(cards);
    let order = resolve_category_order(category_order, &present);
    for cat in order {
        let mut any = false;
        for (idx, card) in cards.iter().enumerate() {
            if card_category(card) != cat {
                continue;
            }
            if !any {
                lines.push(BrowseLine::Category(cat.clone()));
                any = true;
            }
            lines.push(BrowseLine::Card(idx));
        }
    }
    lines
}

/// Paint rows for one card (name+badges, optional description, category, tags).
pub fn card_detail_lines(card: &BrowseCard, selected: bool) -> Vec<Line<'static>> {
    let mark = if selected { "> " } else { "  " };
    let mut out = vec![Line::from(format!(
        "{mark}{}  [{}] [{}]",
        card.name,
        kind_badge(card.kind),
        source_badge(card.source)
    ))];
    if !card.description.is_empty() {
        out.push(Line::from(format!("    {}", card.description)));
    }
    out.push(Line::from(format!(
        "    category: {}",
        card_category(card)
    )));
    if !card.tags.is_empty() {
        out.push(Line::from(format!("    tags: {}", card.tags.join(", "))));
    }
    out
}

/// Total terminal rows the Browse section needs (each card is multi-line).
pub fn browse_section_height(lines: &[BrowseLine], cards: &[BrowseCard]) -> u16 {
    let mut h = 0u16;
    for line in lines {
        h += match line {
            BrowseLine::ImportCatalog | BrowseLine::Category(_) | BrowseLine::Empty => 1,
            BrowseLine::Card(idx) => card_detail_lines(&cards[*idx], false).len() as u16,
        };
    }
    h
}

/// Map a pane-local row (after state + buttons) to a card index, if any.
pub fn card_index_at_row(
    lines: &[BrowseLine],
    cards: &[BrowseCard],
    list_row: u16,
) -> Option<usize> {
    let mut y = 0u16;
    for line in lines {
        let h = match line {
            BrowseLine::ImportCatalog | BrowseLine::Category(_) | BrowseLine::Empty => 1,
            BrowseLine::Card(idx) => {
                let card_h = card_detail_lines(&cards[*idx], false).len() as u16;
                if list_row >= y && list_row < y + card_h {
                    return Some(*idx);
                }
                card_h
            }
        };
        y += h;
    }
    None
}

/// True when `root/src/bot/scripts/index.ts` exists.
pub fn rs2b0t_root_has_index(root: &std::path::Path) -> bool {
    script::registry_index_path(root).is_file()
}

/// One script-pane click result. The app maps buttons to [`AppAction`]s
/// and `Pick`s to the Browse selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptClick {
    /// A button row hit: one of [`SCRIPT_BUTTONS`].
    Button(&'static str),
    /// A Browse picker row hit: index into `cards`.
    Pick(usize),
    /// Import catalog… row while deferred.
    ImportCatalog,
    /// `[Params]` row when the selected card has a settings schema.
    Params,
    /// A miss.
    None,
}

/// The script pane. `slot` is a read-only handle the TUI never calls —
/// it exists so a test can prove clicking a button does not mutate a
/// [`SlotScript`]; the app only emits actions the binary dispatches.
pub struct ScriptPane<'a> {
    pub state: RunState,
    pub sel: Option<&'a ScriptSel>,
    pub cards: &'a [BrowseCard],
    pub category_order: &'a [String],
    pub catalog_deferred: bool,
    pub browse_open: bool,
    pub load_open: bool,
    pub load_path: &'a str,
    pub params_available: bool,
    pub slot: Option<&'a SlotScript>,
}

impl<'a> ScriptPane<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state: RunState,
        sel: Option<&'a ScriptSel>,
        cards: &'a [BrowseCard],
        category_order: &'a [String],
        catalog_deferred: bool,
        browse_open: bool,
        load_open: bool,
        load_path: &'a str,
        params_available: bool,
        slot: Option<&'a SlotScript>,
    ) -> Self {
        Self {
            state,
            sel,
            cards,
            category_order,
            catalog_deferred,
            browse_open,
            load_open,
            load_path,
            params_available,
            slot,
        }
    }

    fn browse_lines(&self) -> Vec<BrowseLine> {
        browse_lines(self.cards, self.category_order, self.catalog_deferred)
    }

    fn card_selected(&self, card: &BrowseCard) -> bool {
        matches!(
            self.sel,
            Some(ScriptSel::Loaded(source, name))
                if card.source == *source && card.name == *name
        )
    }

    /// The click inside the pane. Buttons live on the second inner line;
    /// Browse picker rows follow the state + buttons lines when the
    /// picker is open.
    pub fn on_click(&self, area: Rect, col: u16, row: u16) -> ScriptClick {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        if row == inner.y + 1 {
            let mut cursor = inner.x;
            for slot in SCRIPT_BUTTONS {
                let label = if slot == "Pause" {
                    pause_button_label(self.state)
                } else {
                    slot
                };
                if col >= cursor && col < cursor + label.len() as u16 + 3 {
                    return ScriptClick::Button(label);
                }
                cursor += label.len() as u16 + 3;
            }
        }
        if self.params_available && !self.browse_open && !self.load_open && row == inner.y + 2 {
            if col >= inner.x && col < inner.x + 8 {
                return ScriptClick::Params;
            }
        }
        if self.browse_open {
            let list_top = inner.y + 2;
            if row >= list_top {
                let list_row = row - list_top;
                let lines = self.browse_lines();
                let mut y = 0u16;
                for line in &lines {
                    match line {
                        BrowseLine::ImportCatalog => {
                            if list_row == y {
                                return ScriptClick::ImportCatalog;
                            }
                            y += 1;
                        }
                        BrowseLine::Category(_) | BrowseLine::Empty => {
                            y += 1;
                        }
                        BrowseLine::Card(idx) => {
                            let h = card_detail_lines(&self.cards[*idx], false).len() as u16;
                            if list_row >= y && list_row < y + h {
                                return ScriptClick::Pick(*idx);
                            }
                            y += h;
                        }
                    }
                }
            }
        }
        ScriptClick::None
    }
}

impl Widget for ScriptPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let title = if self.load_open {
            "script — load"
        } else if self.browse_open {
            "script — browse"
        } else {
            "script"
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        let state = run_state_text(self.state);
        let sel = self
            .sel
            .map(|s| s.label())
            .unwrap_or_else(|| "—".into());
        let buttons = SCRIPT_BUTTONS
            .iter()
            .map(|b| {
                let label = if *b == "Pause" {
                    pause_button_label(self.state)
                } else {
                    b
                };
                format!("[{label}]")
            })
            .collect::<Vec<_>>()
            .join(" ");
        let mut lines = vec![
            Line::from(format!("script: {state}   sel: {sel}")),
            Line::from(buttons),
        ];
        if self.params_available && !self.browse_open && !self.load_open {
            lines.push(Line::from("[Params]"));
        }
        if self.load_open {
            lines.push(Line::from("load: browse for .ts/.js file"));
        }
        if self.browse_open {
            for line in self.browse_lines() {
                match line {
                    BrowseLine::ImportCatalog => {
                        lines.push(Line::from("[Import catalog…]"));
                    }
                    BrowseLine::Empty => {
                        lines.push(Line::from(
                            "browse: (no cards — Load a JS file or import catalog)",
                        ));
                    }
                    BrowseLine::Category(cat) => {
                        lines.push(Line::from(format!("— {cat} —")));
                    }
                    BrowseLine::Card(idx) => {
                        let selected = self
                            .cards
                            .get(idx)
                            .is_some_and(|c| self.card_selected(c));
                        if let Some(card) = self.cards.get(idx) {
                            lines.extend(card_detail_lines(card, selected));
                        }
                    }
                }
            }
        }
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .render(inner, buf);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use script::{RunState, ScriptKind, ScriptSel, ScriptSource, SlotScript};

    use super::{
        browse_lines, card_category, categories_present, resolve_category_order, run_state_text,
        BrowseCard, BrowseLine, ScriptClick, ScriptPane, SCRIPT_BUTTONS,
    };

    fn sample_file_card() -> BrowseCard {
        BrowseCard {
            name: "MineRobber".into(),
            description: "Steals from mines".into(),
            category: "Skilling".into(),
            tags: vec!["mining".into()],
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
        }
    }

    fn render(pane: ScriptPane<'_>, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(pane, frame.area()))
            .unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn the_script_widgets_render() {
        let text = render(
            ScriptPane::new(
                script::RunState::Idle,
                None,
                &[],
                &[],
                false,
                false,
                false,
                "",
                false,
                None,
            ),
            60,
            4,
        );
        for label in SCRIPT_BUTTONS {
            assert!(
                text.contains(&format!("[{label}]")),
                "button {label} paints: {text:?}"
            );
        }
        assert!(text.contains("script: idle"), "state line: {text:?}");
    }

    #[test]
    fn clicking_start_does_not_change_a_dummy_slot_script() {
        let slot = SlotScript::new();
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(
            slot.state(),
            None,
            &[],
            &[],
            false,
            false,
            false,
            "",
            false,
            Some(&slot),
        );
        let start = pane.on_click(area, 10, 2);
        assert_eq!(
            start,
            ScriptClick::Button("Start"),
            "Start is clickable (the app maps it to ScriptStart)"
        );
        assert_eq!(slot.state(), script::RunState::Idle, "state unchanged");
        assert!(!slot.want_run, "want_run unchanged");
        assert_eq!(run_state_text(slot.state()), "idle");
    }

    #[test]
    fn click_between_buttons_misses() {
        let pane = ScriptPane::new(
            script::RunState::Idle,
            None,
            &[],
            &[],
            false,
            false,
            false,
            "",
            false,
            None,
        );
        let area = Rect::new(0, 0, 60, 4);
        assert_eq!(pane.on_click(area, 9, 3), ScriptClick::None);
    }

    #[test]
    fn browse_card_paints_description_kind_and_source() {
        let cards = vec![sample_file_card()];
        let text = render(
            ScriptPane::new(
                RunState::Idle,
                None,
                &cards,
                &[],
                false,
                true,
                false,
                "",
                false,
                None,
            ),
            80,
            12,
        );
        assert!(text.contains("MineRobber"), "name: {text:?}");
        assert!(text.contains("Steals from mines"), "description: {text:?}");
        assert!(text.contains("[Compat]"), "kind badge: {text:?}");
        assert!(text.contains("[File]"), "source badge: {text:?}");
        assert!(text.contains("category: Skilling"), "category: {text:?}");
        assert!(text.contains("tags: mining"), "tags: {text:?}");
    }

    #[test]
    fn category_order_changes_list_grouping() {
        let cards = vec![
            BrowseCard {
                name: "A".into(),
                description: String::new(),
                category: "Combat".into(),
                tags: Vec::new(),
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
            },
            BrowseCard {
                name: "B".into(),
                description: String::new(),
                category: "Prayer".into(),
                tags: Vec::new(),
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
            },
        ];
        let prayer_first = browse_lines(&cards, &["Prayer".into(), "Combat".into()], false);
        let combat_first = browse_lines(&cards, &["Combat".into(), "Prayer".into()], false);
        let idx = |lines: &[BrowseLine]| {
            lines
                .iter()
                .position(|l| matches!(l, BrowseLine::Category(c) if c == "Prayer"))
                .unwrap()
        };
        assert!(
            idx(&prayer_first) < idx(&combat_first),
            "Prayer header moves with category order"
        );
    }

    #[test]
    fn resolve_category_order_appends_unknown() {
        let saved = vec!["Prayer".into()];
        let present = vec!["Combat".into(), "Prayer".into()];
        assert_eq!(
            resolve_category_order(&saved, &present),
            vec!["Prayer", "Combat"]
        );
        assert_eq!(
            categories_present(&[BrowseCard {
                name: "x".into(),
                description: String::new(),
                category: String::new(),
                tags: Vec::new(),
                kind: ScriptKind::Compat,
                source: ScriptSource::File,
            }]),
            vec!["Uncategorized"]
        );
        assert_eq!(card_category(&BrowseCard {
            name: "x".into(),
            description: String::new(),
            category: String::new(),
            tags: Vec::new(),
            kind: ScriptKind::Compat,
            source: ScriptSource::File,
        }), "Uncategorized");
    }

    #[test]
    fn browse_open_picks_a_card_row() {
        let cards = vec![
            BrowseCard {
                name: "BoneBurier".into(),
                description: String::new(),
                category: "Prayer".into(),
                tags: Vec::new(),
                kind: ScriptKind::Compat,
                source: ScriptSource::Catalog,
            },
            sample_file_card(),
        ];
        let pane = ScriptPane::new(
            RunState::Idle,
            None,
            &cards,
            &[],
            false,
            true,
            false,
            "",
            false,
            None,
        );
        let area = Rect::new(0, 0, 80, 16);
        // Category header occupies the first browse row; card name is next.
        assert_eq!(
            pane.on_click(area, 2, 4),
            ScriptClick::Pick(0),
            "first card row picks card 0"
        );
        let text = render(
            ScriptPane::new(
                RunState::Idle,
                Some(&ScriptSel::Loaded(
                    ScriptSource::Catalog,
                    "BoneBurier".into(),
                )),
                &cards,
                &[],
                false,
                true,
                false,
                "",
                false,
                None,
            ),
            80,
            16,
        );
        assert!(text.contains('>'), "selected card is marked: {text:?}");
    }

    #[test]
    fn deferred_browse_shows_import_catalog() {
        let text = render(
            ScriptPane::new(
                RunState::Idle,
                None,
                &[],
                &[],
                true,
                true,
                false,
                "",
                false,
                None,
            ),
            80,
            8,
        );
        assert!(
            text.contains("Import catalog"),
            "deferred browse shows import affordance: {text:?}"
        );
    }

    #[test]
    fn paused_state_shows_resume_not_pause() {
        let text = render(
            ScriptPane::new(
                RunState::Paused,
                None,
                &[],
                &[],
                false,
                false,
                false,
                "",
                false,
                None,
            ),
            60,
            4,
        );
        assert!(text.contains("[Resume]"), "paused paints Resume: {text:?}");
        assert!(!text.contains("[Pause]"), "paused must not paint Pause: {text:?}");
    }

    #[test]
    fn running_state_shows_pause_not_resume() {
        let text = render(
            ScriptPane::new(
                RunState::Running,
                None,
                &[],
                &[],
                false,
                false,
                false,
                "",
                false,
                None,
            ),
            60,
            4,
        );
        assert!(text.contains("[Pause]"), "running paints Pause: {text:?}");
        assert!(
            !text.contains("[Resume]"),
            "running must not paint Resume: {text:?}"
        );
    }

    #[test]
    fn clicking_resume_when_paused_returns_resume_button() {
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(
            RunState::Paused,
            None,
            &[],
            &[],
            false,
            false,
            false,
            "",
            false,
            None,
        );
        assert_eq!(
            pane.on_click(area, 18, 2),
            ScriptClick::Button("Resume"),
            "Resume is clickable when paused"
        );
    }
}
