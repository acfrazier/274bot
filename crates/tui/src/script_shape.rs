//! Script pane for native and JavaScript script controls. The shared widget
//! owns only view data; clicks map to app actions dispatched by `tui-play`.

use std::path::PathBuf;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
#[cfg(feature = "js")]
use ratatui::style::{Color, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Widget, Wrap};

#[cfg(feature = "js")]
use frontend_core::scripts::CardStatus;
use frontend_core::scripts::{
    CardRuntime, CardSource, CompiledCardId, ScriptChoice, ScriptRunState, StatusView,
};
use frontend_core::views::{script_status_label, script_status_reason, script_status_rows};

use crate::text::{cells, wrap_cells};

/// The script button labels, left to right (Pause slot is dynamic — see
/// [`pause_button_label`]).
#[cfg(any(feature = "js", feature = "luau"))]
pub const SCRIPT_BUTTONS: [&str; 5] = ["Browse", "Start", "Pause", "Stop", "Load"];
#[cfg(not(any(feature = "js", feature = "luau")))]
pub const SCRIPT_BUTTONS: [&str; 4] = ["Browse", "Start", "Pause", "Stop"];

/// The bulk row under the buttons (after `[Params]` when a schema exists).
#[cfg(feature = "js")]
pub const SCRIPT_BULK_BUTTONS: [&str; 3] = ["Reload", "Start fleet", "Stop fleet"];
#[cfg(not(feature = "js"))]
/// Native fleet actions under the script buttons.
pub const SCRIPT_BULK_BUTTONS: [&str; 2] = ["Start fleet", "Stop fleet"];

/// Every script command's key (shown in its button as `[Label k]`): the
/// same commands the buttons emit, reachable when the pane is clipped.
/// Pause also toggles Resume.
#[cfg(feature = "js")]
pub const SCRIPT_KEYS: [(&str, char); 11] = [
    ("Browse", 'b'),
    ("Start", 't'),
    ("Pause", 'P'),
    ("Stop", 'e'),
    ("Load", 'f'),
    ("Params", 'v'),
    ("Reload", 'R'),
    ("Cancel", 'C'),
    ("Start fleet", 'T'),
    ("Stop fleet", 'E'),
    ("Failures", 'F'),
];
#[cfg(all(feature = "luau", not(feature = "js")))]
pub const SCRIPT_KEYS: [(&str, char); 8] = [
    ("Browse", 'b'),
    ("Start", 't'),
    ("Pause", 'P'),
    ("Stop", 'e'),
    ("Load", 'f'),
    ("Params", 'v'),
    ("Start fleet", 'T'),
    ("Stop fleet", 'E'),
];
#[cfg(not(any(feature = "js", feature = "luau")))]
pub const SCRIPT_KEYS: [(&str, char); 7] = [
    ("Browse", 'b'),
    ("Start", 't'),
    ("Pause", 'P'),
    ("Stop", 'e'),
    ("Params", 'v'),
    ("Start fleet", 'T'),
    ("Stop fleet", 'E'),
];

/// The key associated with a script-pane button label.
pub fn script_key(label: &str) -> char {
    let command = match label {
        "Resume" => "Pause",
        "Confirm" => "Reload",
        other if other.starts_with("Failed ") => "Failures",
        other => other,
    };
    SCRIPT_KEYS
        .iter()
        .find(|(name, _)| *name == command)
        .map_or(' ', |(_, key)| *key)
}

/// The script command a key names, if any.
pub fn script_command_for_key(key: char) -> Option<&'static str> {
    SCRIPT_KEYS
        .iter()
        .find(|(_, k)| *k == key)
        .map(|(name, _)| *name)
}

/// Columns a `[Label k]` button takes, without the gap after it.
fn button_width(label: &str) -> u16 {
    label.len() as u16 + 4
}

/// The load-failure button's label: `Failed 7`.
pub fn failures_label(count: usize) -> String {
    format!("Failed {count}")
}

fn push_button(row: &mut String, label: &str) {
    if !row.is_empty() {
        row.push(' ');
    }
    row.push('[');
    row.push_str(label);
    row.push(' ');
    row.push(script_key(label));
    row.push(']');
}

/// Pause/Resume label for the third button (matches panel `script_section`).
pub fn pause_button_label(state: ScriptRunState) -> &'static str {
    if state == ScriptRunState::Paused {
        "Resume"
    } else {
        "Pause"
    }
}

pub fn kind_badge(kind: CardRuntime) -> &'static str {
    match kind {
        CardRuntime::Compat => "Compat",
        CardRuntime::NativeTick => "NativeTick",
        CardRuntime::Compiled => "Compiled",
        CardRuntime::Luau => "Luau",
    }
}

pub fn source_badge(source: CardSource) -> &'static str {
    match source {
        CardSource::Catalog => "Catalog",
        CardSource::File => "File",
        CardSource::Builtin => "Builtin",
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

/// What a File or Catalog card has and a compiled card does not: the facts
/// frontend-core lists for it. Boxed, so a compiled row stays small.
#[derive(Debug, PartialEq, Eq)]
pub struct CardOrigin {
    /// The card's identity token: a File card's canonical path, a Catalog
    /// card's name.
    pub identity_id: String,
    /// The source file.
    pub path: Option<PathBuf>,
    /// `File: /path` or `Catalog: /path`.
    pub provenance: String,
    /// The selected-card detail rows, as frontend-core lists them.
    pub details: Vec<(&'static str, String)>,
}

/// Lightweight Browse row (no origin/js bytes). It carries the card's
/// identity token and provenance from frontend-core; selection, the selected
/// mark and Params key on [`Self::identity`], never on the display name.
#[derive(Debug, PartialEq, Eq)]
pub struct BrowseCard {
    /// The name the picker shows (a manifest card's manifest name).
    pub name: String,
    /// The identity, path, provenance and detail rows of a File or Catalog
    /// card; `None` for a compiled card, whose identity is its name.
    pub origin: Option<Box<CardOrigin>>,
    /// Another card shows the same name.
    pub shared_name: bool,
    pub description: String,
    pub category: String,
    pub tags: Vec<String>,
    pub kind: CardRuntime,
    pub source: CardSource,
    /// Whether the card matches the Browse filter (frontend-core decides).
    pub filter_match: bool,
    #[cfg(feature = "js")]
    pub status: Option<CardStatus>,
}

impl BrowseCard {
    /// The row for `card`; `shared_name` says another card shows its name.
    /// A compiled card carries no origin (its identity is its name), so a
    /// frame's refresh of the rows costs no more for it than the row's own
    /// text.
    pub fn from_core(card: frontend_core::scripts::BrowseCard<'_>, shared_name: bool) -> Self {
        let origin = card.compiled_id().is_none().then(|| {
            Box::new(CardOrigin {
                identity_id: card.identity_id(),
                path: card.path().map(std::path::Path::to_path_buf),
                provenance: card.provenance(),
                details: card
                    .details()
                    .rows()
                    .map(|(label, value)| (label, value.to_owned()))
                    .collect(),
            })
        });
        Self {
            name: card.display_name().to_owned(),
            origin,
            shared_name,
            description: card.description().to_owned(),
            category: card.category().to_owned(),
            tags: card.tags().map_or_else(Vec::new, ToOwned::to_owned),
            kind: card.kind(),
            source: card.source(),
            #[cfg(feature = "js")]
            status: card.status(),
            filter_match: true,
        }
    }

    /// The card's identity token: its origin's `identity_id`, or its name
    /// for a compiled card (whose id is its name).
    pub fn identity(&self) -> &str {
        self.origin
            .as_ref()
            .map_or(self.name.as_str(), |origin| origin.identity_id.as_str())
    }

    /// The source file of a File or Catalog card.
    pub fn path(&self) -> Option<&std::path::Path> {
        self.origin.as_ref()?.path.as_deref()
    }

    /// `File: /path`, `Catalog: /path`; empty for a compiled card.
    pub fn provenance(&self) -> &str {
        self.origin
            .as_ref()
            .map_or("", |origin| origin.provenance.as_str())
    }

    /// The selected-card detail rows; none for a compiled card.
    pub fn details(&self) -> &[(&'static str, String)] {
        self.origin
            .as_ref()
            .map_or(&[], |origin| origin.details.as_slice())
    }
}

#[cfg(test)]
impl BrowseCard {
    /// A row for a test: the identity token follows the name (a File row's
    /// token is its bare name), with no path, status or details.
    pub fn for_test(
        name: &str,
        description: &str,
        category: &str,
        tags: &[&str],
        kind: CardRuntime,
        source: CardSource,
    ) -> Self {
        Self {
            name: name.into(),
            origin: (kind != CardRuntime::Compiled).then(|| {
                Box::new(CardOrigin {
                    identity_id: name.into(),
                    path: None,
                    provenance: format!("{}: {name}", source_badge(source)),
                    details: Vec::new(),
                })
            }),
            shared_name: false,
            description: description.into(),
            category: category.into(),
            tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
            kind,
            source,
            filter_match: true,
            #[cfg(feature = "js")]
            status: None,
        }
    }
}

/// The selection represented by a Browse card: its identity token, never its
/// display name.
pub fn card_selection(card: &BrowseCard) -> Option<ScriptChoice> {
    if card.kind == CardRuntime::Compiled {
        CompiledCardId::lookup(card.identity()).map(ScriptChoice::compiled)
    } else if card.kind == CardRuntime::Luau {
        Some(ScriptChoice::luau(card.identity()))
    } else {
        Some(ScriptChoice::loaded(card.source, card.identity()))
    }
}

impl<'a> From<frontend_core::scripts::BrowseCard<'a>> for BrowseCard {
    fn from(card: frontend_core::scripts::BrowseCard<'a>) -> Self {
        Self::from_core(card, false)
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

/// One logical line in the Browse list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BrowseLine {
    #[cfg(feature = "js")]
    ImportCatalog,
    Category(String),
    Card(usize),
    Empty,
    /// The Browse filter matches no card.
    NoMatch,
}

/// Build grouped Browse lines.
pub fn browse_lines(
    cards: &[BrowseCard],
    category_order: &[String],
    #[cfg(feature = "js")] catalog_deferred: bool,
) -> Vec<BrowseLine> {
    let mut lines = Vec::new();
    #[cfg(feature = "js")]
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

/// `lines` limited to the cards that match the Browse filter
/// ([`BrowseCard::filter_match`]), without the heading of a category left
/// empty. A filter that matches no card leaves [`BrowseLine::NoMatch`].
pub fn retain_matching(lines: Vec<BrowseLine>, cards: &[BrowseCard]) -> Vec<BrowseLine> {
    let mut out = Vec::with_capacity(lines.len());
    let mut heading = None;
    let mut shown = 0usize;
    for line in lines {
        match line {
            BrowseLine::Category(category) => heading = Some(BrowseLine::Category(category)),
            BrowseLine::Card(idx) if cards.get(idx).is_some_and(|card| card.filter_match) => {
                out.extend(heading.take());
                out.push(BrowseLine::Card(idx));
                shown += 1;
            }
            BrowseLine::Card(_) => {}
            other => out.push(other),
        }
    }
    if shown == 0 && !cards.is_empty() {
        out.push(BrowseLine::NoMatch);
    }
    out
}

/// Logical rows for one card: its title line, then the description,
/// category, tags and status. A card that shares its display name with
/// another shows its provenance, so the pair can be told apart; the
/// selected card shows every detail row (source, path, version, author, API
/// family, declared capabilities). Rows are not wrapped here; see
/// [`card_rows`].
pub fn card_detail_lines(card: &BrowseCard, selected: bool) -> Vec<Line<'static>> {
    let mark = if selected { "> " } else { "  " };
    #[cfg(feature = "js")]
    let style = if card.status.as_ref().is_some_and(CardStatus::blocks_start) {
        Style::default()
            .fg(Color::DarkGray)
            .add_modifier(Modifier::DIM)
    } else {
        Style::default()
    };
    #[cfg(not(feature = "js"))]
    let style = Style::default();
    let mut out = vec![Line::from(Span::styled(
        format!(
            "{mark}{}  [{}] [{}]",
            card.name,
            kind_badge(card.kind),
            source_badge(card.source)
        ),
        style,
    ))];
    if !card.description.is_empty() {
        out.push(Line::from(Span::styled(
            format!("    {}", card.description),
            style,
        )));
    }
    out.push(Line::from(Span::styled(
        format!("    category: {}", card_category(card)),
        style,
    )));
    if !card.tags.is_empty() {
        out.push(Line::from(Span::styled(
            format!("    tags: {}", card.tags.join(", ")),
            style,
        )));
    }
    #[cfg(feature = "js")]
    if let Some(status) = &card.status {
        let color = if status.is_error() {
            Color::Red
        } else {
            Color::Yellow
        };
        out.push(Line::from(Span::styled(
            format!("    {}", status.line()),
            Style::default().fg(color),
        )));
    }
    if selected {
        for (label, value) in card.details() {
            out.push(Line::from(Span::styled(
                format!("    {label}: {value}"),
                style,
            )));
        }
    } else if card.shared_name && !card.provenance().is_empty() {
        out.push(Line::from(Span::styled(
            format!("    {}", card.provenance()),
            style,
        )));
    }
    out
}

/// [`card_detail_lines`] wrapped to `width` cells, so every row is at most
/// as wide as the pane and the row count is exact: the paint, the click
/// map and the scroll bounds agree. A long description, tag list or path
/// wraps under a four-cell indent; nothing is cut.
pub fn card_rows(card: &BrowseCard, selected: bool, width: u16) -> Vec<Line<'static>> {
    let width = usize::from(width);
    let mut rows = Vec::new();
    for line in card_detail_lines(card, selected) {
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        if cells(&text) <= width {
            rows.push(line);
            continue;
        }
        let style = line
            .spans
            .first()
            .map(|span| span.style)
            .unwrap_or_default();
        let marked = text.starts_with("> ");
        let body = text.strip_prefix("> ").unwrap_or(&text).trim_start();
        let indent = " ".repeat(text.len() - text.trim_start().len());
        let first = if marked { "> " } else { indent.as_str() };
        rows.extend(
            wrap_cells(body, first, "    ", width)
                .into_iter()
                .map(|row| Line::from(Span::styled(row, style))),
        );
    }
    rows
}

/// True when `root/src/bot/scripts/index.ts` exists.
#[cfg(feature = "js")]
pub fn rs2b0t_root_has_index(root: &std::path::Path) -> bool {
    frontend_core::scripts::paths::registry_index_path(root).is_file()
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
    #[cfg(feature = "js")]
    ImportCatalog,
    /// `[Params]` row when the selected card has a settings schema.
    Params,
    /// `[Failed N]`: open the load-failure list.
    Failures,
    /// The cut park reason row: show the whole reason.
    Reason,
    /// A miss.
    None,
}

/// The script pane; the app only emits actions the binary dispatches.
pub struct ScriptPane<'a> {
    pub state: ScriptRunState,
    pub sel: Option<&'a ScriptChoice>,
    pub cards: &'a [BrowseCard],
    pub category_order: &'a [String],
    #[cfg(feature = "js")]
    pub catalog_deferred: bool,
    pub browse_open: bool,
    #[cfg(any(feature = "js", feature = "luau"))]
    pub load_open: bool,
    pub params_available: bool,
    #[cfg(feature = "js")]
    pub reload_confirm: bool,
    pub native_status: Option<StatusView<'a>>,
    /// How many script cards failed to load: `[Failed N F]` opens the list
    /// while it is more than zero.
    pub failure_count: usize,
    pub walk_risk: Option<&'a str>,
    pub start_zones: Option<&'a frontend_core::scripts::QuesterStartZones>,
    pub start_zones_error: Option<&'a str>,
    pub start_zone_scroll: u16,
    /// The Browse filter text, shown in the title (frontend-core matched the
    /// cards).
    pub browse_filter: &'a str,
    /// The filter is being typed.
    pub browse_filter_editing: bool,
}

pub(crate) fn wrapped_line_count(lines: &[Line<'_>], width: u16) -> usize {
    let width = usize::from(width.max(1));
    lines
        .iter()
        .map(|line| {
            let mut rows = 0;
            let mut used = 0;
            for span in &line.spans {
                for word in span.content.split_whitespace() {
                    let word_width = Span::raw(word).width().max(1);
                    if used == 0 {
                        rows += word_width.div_ceil(width);
                        used = word_width % width;
                    } else if word_width <= width && used + 1 + word_width <= width {
                        used += 1 + word_width;
                    } else {
                        rows += word_width.div_ceil(width);
                        used = word_width % width;
                    }
                }
            }
            rows.max(1)
        })
        .sum()
}
impl<'a> ScriptPane<'a> {
    pub fn new(
        state: ScriptRunState,
        sel: Option<&'a ScriptChoice>,
        cards: &'a [BrowseCard],
        category_order: &'a [String],
        browse_open: bool,
        params_available: bool,
    ) -> Self {
        Self {
            state,
            sel,
            cards,
            category_order,
            #[cfg(feature = "js")]
            catalog_deferred: false,
            browse_open,
            #[cfg(any(feature = "js", feature = "luau"))]
            load_open: false,
            params_available,
            #[cfg(feature = "js")]
            reload_confirm: false,
            native_status: None,
            failure_count: 0,
            walk_risk: None,
            start_zones: None,
            start_zones_error: None,
            start_zone_scroll: 0,
            browse_filter: "",
            browse_filter_editing: false,
        }
    }

    #[cfg(feature = "js")]
    pub fn with_js_controls(
        mut self,
        catalog_deferred: bool,
        load_open: bool,
        reload_confirm: bool,
    ) -> Self {
        self.catalog_deferred = catalog_deferred;
        self.load_open = load_open;
        self.reload_confirm = reload_confirm;
        self
    }

    #[cfg(any(feature = "js", feature = "luau"))]
    pub fn with_load_open(mut self, load_open: bool) -> Self {
        self.load_open = load_open;
        self
    }

    pub fn with_native_status(mut self, status: Option<StatusView<'a>>) -> Self {
        self.native_status = status;
        self
    }

    pub fn with_load_failures(mut self, count: usize) -> Self {
        self.failure_count = count;
        self
    }

    pub fn with_walk_risk(mut self, reason: Option<&'a str>) -> Self {
        self.walk_risk = reason;
        self
    }

    /// Show the Browse filter text in the title; `editing` shows the caret.
    pub fn with_browse_filter(mut self, filter: &'a str, editing: bool) -> Self {
        self.browse_filter = filter;
        self.browse_filter_editing = editing;
        self
    }

    pub fn with_start_zone_view(
        mut self,
        view: Option<Result<&'a frontend_core::scripts::QuesterStartZones, &'a str>>,
        scroll: u16,
    ) -> Self {
        self.start_zones = None;
        self.start_zones_error = None;
        match view {
            Some(Ok(zones)) => self.start_zones = Some(zones),
            Some(Err(reason)) => self.start_zones_error = Some(reason),
            None => {}
        }
        self.start_zone_scroll = scroll;
        self
    }

    fn start_zone_disclosure_shown(&self) -> bool {
        #[cfg(any(feature = "js", feature = "luau"))]
        let load_open = self.load_open;
        #[cfg(not(any(feature = "js", feature = "luau")))]
        let load_open = false;
        !self.browse_open
            && !load_open
            && matches!(self.state, ScriptRunState::Idle | ScriptRunState::Error)
            && self.sel.and_then(ScriptChoice::compiled_id) == Some(CompiledCardId::QUESTER)
            && (self.start_zones.is_some() || self.start_zones_error.is_some())
    }

    /// The main row's buttons, left to right (Pause reads Resume while
    /// paused).
    fn main_buttons(&self) -> impl Iterator<Item = &'static str> {
        let state = self.state;
        SCRIPT_BUTTONS.iter().map(move |b| {
            if *b == "Pause" {
                pause_button_label(state)
            } else {
                b
            }
        })
    }

    /// The bulk row's buttons, left to right.
    fn bulk_buttons(&self) -> impl Iterator<Item = &'static str> {
        #[cfg(feature = "js")]
        {
            let reload = if self.reload_confirm {
                "Confirm"
            } else {
                "Reload"
            };
            let cancel = self.reload_confirm.then_some("Cancel");
            std::iter::once(reload)
                .chain(cancel)
                .chain(SCRIPT_BULK_BUTTONS[1..].iter().copied())
        }
        #[cfg(not(feature = "js"))]
        {
            SCRIPT_BULK_BUTTONS.iter().copied()
        }
    }

    /// Whether the bulk row (and `[Params]`) is shown under the active view.
    fn bulk_row_shown(&self) -> bool {
        #[cfg(any(feature = "js", feature = "luau"))]
        {
            !self.browse_open && !self.load_open
        }
        #[cfg(not(any(feature = "js", feature = "luau")))]
        {
            !self.browse_open
        }
    }

    fn browse_lines(&self) -> Vec<BrowseLine> {
        #[cfg(feature = "js")]
        let lines = browse_lines(self.cards, self.category_order, self.catalog_deferred);
        #[cfg(not(feature = "js"))]
        let lines = browse_lines(self.cards, self.category_order);
        retain_matching(lines, self.cards)
    }

    /// Whether `card` is the selected card: the same identity token, not the
    /// same display name, so two cards that share a name are never both
    /// marked.
    fn card_selected(&self, card: &BrowseCard) -> bool {
        self.sel
            .is_some_and(|selection| card_selection(card).as_ref() == Some(selection))
    }

    /// What the state line names as the selection: the shown name of the
    /// selected card, else the selection's own label.
    fn selection_label(&self) -> &str {
        let Some(selection) = self.sel else {
            return "—";
        };
        self.cards
            .iter()
            .find(|card| self.card_selected(card))
            .map_or_else(|| selection.label(), |card| card.name.as_str())
    }

    /// Where the park reason's row sits in `area` when it is cut (a click
    /// there opens the whole text), `None` when the pane shows it whole or
    /// shows no reason.
    pub fn reason_hit(&self, area: Rect) -> Option<Rect> {
        if !self.bulk_row_shown() {
            return None;
        }
        let reason = script_status_reason(self.native_status?)?;
        if !reason.is_cut() {
            return None;
        }
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let lines = self.prefix_lines(inner.width);
        // Everything before the reason line: state, buttons, bulk row.
        let before = wrapped_line_count(&lines[..3], inner.width);
        let y = inner.y.checked_add(u16::try_from(before).ok()?)?;
        (y < inner.bottom()).then(|| Rect::new(inner.x, y, inner.width, 1))
    }

    /// The click inside the pane. Buttons live on the second inner line;
    /// Browse picker rows follow the state + buttons lines when the
    /// picker is open.
    pub fn on_click(&self, area: Rect, col: u16, row: u16) -> ScriptClick {
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let hit = |labels: &mut dyn Iterator<Item = &'static str>| {
            let mut cursor = inner.x;
            for label in labels {
                let width = button_width(label);
                if col >= cursor && col < cursor + width {
                    return Some(label);
                }
                cursor += width + 1;
            }
            None
        };
        if row == inner.y + 1 {
            if let Some(label) = hit(&mut self.main_buttons()) {
                return ScriptClick::Button(label);
            }
        }
        if self.bulk_row_shown() && row == inner.y + 2 && row < inner.bottom() {
            let params = self.params_available.then_some("Params");
            match hit(&mut params.into_iter().chain(self.bulk_buttons())) {
                Some("Params") => return ScriptClick::Params,
                Some(label) => return ScriptClick::Button(label),
                None => {}
            }
            if self.failure_count > 0 {
                let before = params
                    .into_iter()
                    .chain(self.bulk_buttons())
                    .map(|label| button_width(label) + 1)
                    .sum::<u16>();
                let start = inner.x + before;
                if col >= start && col < start + button_width(&failures_label(self.failure_count)) {
                    return ScriptClick::Failures;
                }
            }
        }
        if self.reason_hit(area).is_some_and(|rect| {
            col >= rect.x && col < rect.right() && row >= rect.y && row < rect.bottom()
        }) {
            return ScriptClick::Reason;
        }
        if self.browse_open {
            let list_top = inner.y + 2;
            if row >= list_top {
                let list_row = row - list_top;
                let lines = self.browse_lines();
                let mut y = 0u16;
                for line in &lines {
                    match line {
                        #[cfg(feature = "js")]
                        BrowseLine::ImportCatalog => {
                            if list_row == y {
                                return ScriptClick::ImportCatalog;
                            }
                            y += 1;
                        }
                        BrowseLine::Category(_) | BrowseLine::Empty | BrowseLine::NoMatch => {
                            y += 1;
                        }
                        BrowseLine::Card(idx) => {
                            let card = &self.cards[*idx];
                            let h =
                                card_rows(card, self.card_selected(card), inner.width).len() as u16;
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
    /// The lines above the picker or disclosure, for an inner width of
    /// `width` cells (the park reason is cut to one row of it).
    fn prefix_lines(&self, width: u16) -> Vec<Line<'a>> {
        let state = script_status_label(self.state, self.native_status);
        let selection = self.selection_label();
        let mut buttons = String::new();
        for label in self.main_buttons() {
            push_button(&mut buttons, label);
        }
        let mut lines = vec![
            Line::from(format!("script: {state}   sel: {selection}")),
            Line::from(buttons),
        ];
        if self.bulk_row_shown() {
            let mut row = String::new();
            if self.params_available {
                push_button(&mut row, "Params");
            }
            for label in self.bulk_buttons() {
                push_button(&mut row, label);
            }
            if self.failure_count > 0 {
                push_button(&mut row, &failures_label(self.failure_count));
            }
            lines.push(Line::from(row));
            if let Some(status) = self.native_status {
                if let Some(reason) = script_status_reason(status) {
                    // One row: at most `SUMMARY_MAX` characters, then the
                    // cells left. The whole text is one click away.
                    lines.push(Line::from(
                        crate::text::ellipsize(&reason.as_str(), usize::from(width)).into_owned(),
                    ));
                }
                for (label, value) in script_status_rows(status) {
                    lines.push(Line::from(format!("{label}: {value}")));
                }
            }
        }
        if let Some(reason) = self.walk_risk {
            lines.push(Line::from(format!("Walk risk: {reason}")));
        }
        #[cfg(any(feature = "js", feature = "luau"))]
        if self.load_open {
            lines.push(Line::from("load: browse for script files"));
        }
        if self.start_zone_disclosure_shown() {
            lines.push(Line::from(frontend_core::scripts::START_ZONE_SCOPE_TITLE));
            match self.start_zones {
                Some(zones) => {
                    if let Some(warning) = zones.unqualified_warning() {
                        lines.push(Line::styled(
                            warning,
                            Style::default().add_modifier(ratatui::style::Modifier::BOLD),
                        ));
                    }
                    lines.push(Line::from(zones.notice()));
                    if !zones.is_empty() {
                        lines.push(Line::from(
                            frontend_core::scripts::START_ZONE_TUI_SCROLL_HINT,
                        ));
                    }
                }
                None => {
                    lines.push(Line::from(
                        frontend_core::scripts::START_ZONE_UNAVAILABLE_NOTICE,
                    ));
                    lines.push(Line::from(
                        frontend_core::scripts::START_ZONE_TUI_SCROLL_HINT,
                    ));
                }
            }
        }
        lines
    }

    fn start_zone_lines(&self) -> Vec<Line<'a>> {
        if let Some(zones) = self.start_zones {
            zones
                .scopes
                .iter()
                .flat_map(|scope| {
                    std::iter::once(Line::from(scope.display_label.as_str())).chain(
                        scope.zones.iter().flat_map(|zone| {
                            [
                                Line::from(zone.display_label.as_str()),
                                Line::from("Guide reason:"),
                                Line::from(zone.reason.as_str()),
                            ]
                        }),
                    )
                })
                .collect()
        } else {
            self.start_zones_error
                .map(|reason| vec![Line::from(reason)])
                .unwrap_or_default()
        }
    }

    fn content_lines(&self, width: u16) -> Vec<Line<'a>> {
        let mut lines = self.prefix_lines(width);
        if self.start_zone_disclosure_shown() {
            lines.extend(self.start_zone_lines());
        } else if self.browse_open {
            for line in self.browse_lines() {
                match line {
                    #[cfg(feature = "js")]
                    BrowseLine::ImportCatalog => {
                        lines.push(Line::from("[Import catalog…]"));
                    }
                    #[cfg(feature = "js")]
                    BrowseLine::Empty => {
                        lines.push(Line::from(
                            "browse: (no cards — Load a JS file or import catalog)",
                        ));
                    }
                    #[cfg(not(feature = "js"))]
                    BrowseLine::Empty => {
                        lines.push(Line::from("browse: (no cards)"));
                    }
                    BrowseLine::NoMatch => {
                        lines.push(Line::from("browse: (no card matches the filter)"));
                    }
                    BrowseLine::Category(cat) => {
                        lines.push(Line::from(format!("— {cat} —")));
                    }
                    BrowseLine::Card(idx) => {
                        let selected = self.cards.get(idx).is_some_and(|c| self.card_selected(c));
                        if let Some(card) = self.cards.get(idx) {
                            lines.extend(card_rows(card, selected, width));
                        }
                    }
                }
            }
        }
        lines
    }

    /// Return the maximum offset and PageUp/PageDown step in rendered rows.
    pub fn vertical_scroll_limits(&self, area: Rect) -> (u16, u16) {
        if !self.start_zone_disclosure_shown() {
            return (0, 1);
        }
        let inner = Block::default().borders(Borders::ALL).inner(area);
        let prefix_count = wrapped_line_count(&self.prefix_lines(inner.width), inner.width)
            .min(usize::from(inner.height));
        let available = usize::from(inner.height).saturating_sub(prefix_count);
        let content_count = wrapped_line_count(&self.start_zone_lines(), inner.width);
        let max_scroll = content_count
            .saturating_sub(available)
            .min(usize::from(u16::MAX)) as u16;
        let page_step = available.saturating_sub(1).max(1) as u16;
        (max_scroll, page_step)
    }

    pub fn max_vertical_scroll(&self, area: Rect) -> u16 {
        self.vertical_scroll_limits(area).0
    }
}

impl Widget for ScriptPane<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        #[cfg(any(feature = "js", feature = "luau"))]
        let load_open = self.load_open;
        #[cfg(not(any(feature = "js", feature = "luau")))]
        let load_open = false;
        let title = if load_open {
            "script — load".to_owned()
        } else if self.browse_open && self.browse_filter_editing {
            format!("script — browse /{}_", self.browse_filter)
        } else if self.browse_open && !self.browse_filter.is_empty() {
            format!("script — browse /{}", self.browse_filter)
        } else if self.browse_open {
            "script — browse".to_owned()
        } else {
            "script".to_owned()
        };
        let block = Block::default().borders(Borders::ALL).title(title);
        let inner = block.inner(area);
        block.render(area, buf);
        if self.start_zone_disclosure_shown() {
            let prefix_lines = self.prefix_lines(inner.width);
            let prefix_height = wrapped_line_count(&prefix_lines, inner.width)
                .min(usize::from(inner.height)) as u16;
            let prefix_area = Rect::new(inner.x, inner.y, inner.width, prefix_height);
            Paragraph::new(prefix_lines)
                .wrap(Wrap { trim: false })
                .render(prefix_area, buf);
            let list_area = Rect::new(
                inner.x,
                inner.y.saturating_add(prefix_height),
                inner.width,
                inner.height.saturating_sub(prefix_height),
            );
            Paragraph::new(self.start_zone_lines())
                .wrap(Wrap { trim: false })
                .scroll((
                    self.start_zone_scroll.min(self.max_vertical_scroll(area)),
                    0,
                ))
                .render(list_area, buf);
        } else {
            Paragraph::new(self.content_lines(inner.width))
                .wrap(Wrap { trim: false })
                .render(inner, buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use ratatui::Terminal;

    use frontend_core::scripts::{
        CardRuntime, CardSource, CompiledCardId, ScriptChoice, ScriptRunState,
    };
    use frontend_core::test_support::scripts::NativeStatus;

    #[cfg(feature = "js")]
    use super::card_detail_lines;
    use super::{
        browse_lines, card_category, card_selection, categories_present, resolve_category_order,
        BrowseCard, BrowseLine, CardOrigin, ScriptClick, ScriptPane, SCRIPT_BUTTONS,
    };
    use crate::text::cells;

    fn sample_file_card() -> BrowseCard {
        BrowseCard::for_test(
            "MineRobber",
            "Steals from mines",
            "Skilling",
            &["mining"],
            CardRuntime::Compat,
            CardSource::File,
        )
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

    fn browse_lines_for_test(
        cards: &[BrowseCard],
        category_order: &[String],
        catalog_deferred: bool,
    ) -> Vec<BrowseLine> {
        #[cfg(feature = "js")]
        {
            browse_lines(cards, category_order, catalog_deferred)
        }
        #[cfg(not(feature = "js"))]
        {
            let _ = catalog_deferred;
            browse_lines(cards, category_order)
        }
    }

    #[test]
    fn the_script_widgets_render() {
        let text = render(
            ScriptPane::new(ScriptRunState::Idle, None, &[], &[], false, false),
            60,
            4,
        );
        for label in SCRIPT_BUTTONS {
            assert!(
                text.contains(&format!("[{label} {}]", super::script_key(label))),
                "button {label} paints: {text:?}"
            );
        }
        assert!(text.contains("script: idle"), "state line: {text:?}");
    }

    #[test]
    fn clicking_start_maps_to_the_start_button() {
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(ScriptRunState::Idle, None, &[], &[], false, false);
        let start = pane.on_click(area, 14, 2);
        assert_eq!(
            start,
            ScriptClick::Button("Start"),
            "Start is clickable (the app maps it to ScriptStart)"
        );
    }

    /// The bulk row under the buttons: Reload / Start fleet / Stop fleet after
    /// `[Params]`; a pending reload warning turns Reload into Confirm and
    /// adds Cancel, shifting the rest.
    #[cfg(feature = "js")]
    #[test]
    fn bulk_row_clicks_follow_the_painted_labels() {
        let pane = |confirm: bool| {
            ScriptPane::new(ScriptRunState::Running, None, &[], &[], false, true)
                .with_js_controls(false, false, confirm)
        };
        let area = Rect::new(0, 0, 80, 6);
        let row = 3;
        let painted = render(pane(false), 80, 6);
        assert!(
            painted.contains("[Params v] [Reload R] [Start fleet T] [Stop fleet E]"),
            "{painted}"
        );
        assert_eq!(pane(false).on_click(area, 1, row), ScriptClick::Params);
        assert_eq!(pane(false).on_click(area, 11, row), ScriptClick::None);
        assert_eq!(
            pane(false).on_click(area, 12, row),
            ScriptClick::Button("Reload")
        );
        assert_eq!(
            pane(false).on_click(area, 23, row),
            ScriptClick::Button("Start fleet")
        );
        assert_eq!(
            pane(false).on_click(area, 48, row),
            ScriptClick::Button("Stop fleet")
        );
        let painted = render(pane(true), 80, 6);
        assert!(
            painted.contains("[Params v] [Confirm R] [Cancel C] [Start fleet T] [Stop fleet E]"),
            "{painted}"
        );
        assert_eq!(
            pane(true).on_click(area, 22, row),
            ScriptClick::Button("Confirm")
        );
        assert_eq!(
            pane(true).on_click(area, 24, row),
            ScriptClick::Button("Cancel")
        );
        assert_eq!(
            pane(true).on_click(area, 35, row),
            ScriptClick::Button("Start fleet")
        );
    }

    #[test]
    fn click_between_buttons_misses() {
        let pane = ScriptPane::new(ScriptRunState::Idle, None, &[], &[], false, false);
        let area = Rect::new(0, 0, 60, 4);
        assert_eq!(pane.on_click(area, 9, 3), ScriptClick::None);
    }

    #[test]
    fn browse_card_paints_description_kind_and_source() {
        let cards = vec![sample_file_card()];
        let text = render(
            ScriptPane::new(ScriptRunState::Idle, None, &cards, &[], true, false),
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

    #[cfg(feature = "js")]
    #[test]
    fn catalog_status_preserves_import_error_at_both_terminal_sizes() {
        use frontend_core::scripts::Scripts;
        use ratatui::style::{Color, Modifier};
        use std::fs;

        let root =
            std::env::temp_dir().join(format!("274bot-tui-catalog-card-{}", std::process::id()));
        let scripts = root.join("src/bot/scripts");
        fs::create_dir_all(scripts.join("Ghost")).unwrap();
        fs::write(
            scripts.join("index.ts"),
            "import { ScriptRegistry } from '../runtime/ScriptRegistry.js';\n\
             import Ghost from './Ghost/Ghost.js';\n\
             import Woodcutter from './Woodcutter.js';\n\
             ScriptRegistry.register({ name: 'Ghost', description: '', category: 'Test', create: () => new Ghost() });\n\
             ScriptRegistry.register({ name: 'Woodcutter', description: '', category: 'Test', create: () => new Woodcutter() });\n",
        )
        .unwrap();
        fs::write(
            scripts.join("Ghost/Ghost.ts"),
            "import Missing from '../../event/webwalk/Missing.js';\n\
             export default class Ghost extends LoopingBot { loop() {} }\n",
        )
        .unwrap();
        fs::write(
            scripts.join("Woodcutter.ts"),
            "export default class Woodcutter extends LoopingBot { loop() {} }\n",
        )
        .unwrap();
        let core = frontend_core::session::OperatorSession::<()>::open(
            frontend_core::launch::InstancePermit::skip(),
        );
        let mut model = Scripts::open_default(&core).0;
        model.use_library_at(root.join("cards.json"), root.join("cache"));
        model
            .fill_catalog_blocking(&root, &root.join("rs2b0t-path"))
            .unwrap();
        let cards: Vec<_> = model
            .browse_cards()
            .filter(|card| card.source() == CardSource::Catalog)
            .map(BrowseCard::from)
            .collect();
        fs::remove_dir_all(root).unwrap();

        let ghost = cards.iter().find(|card| card.name == "Ghost").unwrap();
        let woodcutter = cards.iter().find(|card| card.name == "Woodcutter").unwrap();
        let ghost_lines = card_detail_lines(ghost, false);
        let woodcutter_lines = card_detail_lines(woodcutter, false);
        assert!(ghost_lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::DIM));
        assert_eq!(
            ghost_lines.last().unwrap().spans[0].style.fg,
            Some(Color::Red)
        );
        assert!(!woodcutter_lines[0].spans[0]
            .style
            .add_modifier
            .contains(Modifier::DIM));
        assert_eq!(
            woodcutter_lines.last().unwrap().spans[0].content.trim(),
            "unverified on 274bot"
        );

        let sel = ScriptChoice::loaded(CardSource::Catalog, "Ghost");
        for (w, h) in [(120, 40), (80, 24)] {
            let pane = ScriptPane::new(ScriptRunState::Idle, Some(&sel), &cards, &[], true, false);
            let text = render(pane, w, h);
            assert_eq!(
                text.matches("unverified on 274bot").count(),
                2,
                "{w}x{h}: {text:?}"
            );
            assert!(
                text.contains(
                    "unverified on 274bot — failed import: ../../event/webwalk/Missing.js"
                ),
                "{w}x{h}: {text:?}"
            );
        }
    }

    #[test]
    fn category_order_changes_list_grouping() {
        let cards = vec![
            BrowseCard::for_test(
                "A",
                "",
                "Combat",
                &[],
                CardRuntime::Compat,
                CardSource::Catalog,
            ),
            BrowseCard::for_test(
                "B",
                "",
                "Prayer",
                &[],
                CardRuntime::Compat,
                CardSource::Catalog,
            ),
        ];
        let prayer_first =
            browse_lines_for_test(&cards, &["Prayer".into(), "Combat".into()], false);
        let combat_first =
            browse_lines_for_test(&cards, &["Combat".into(), "Prayer".into()], false);
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
            categories_present(&[BrowseCard::for_test(
                "x",
                "",
                "",
                &[],
                CardRuntime::Compat,
                CardSource::File,
            )]),
            vec!["Uncategorized"]
        );
        assert_eq!(
            card_category(&BrowseCard::for_test(
                "x",
                "",
                "",
                &[],
                CardRuntime::Compat,
                CardSource::File,
            )),
            "Uncategorized"
        );
    }

    #[test]
    fn browse_open_picks_a_card_row() {
        let cards = vec![
            BrowseCard::for_test(
                "BoneBurier",
                "",
                "Prayer",
                &[],
                CardRuntime::Compat,
                CardSource::Catalog,
            ),
            sample_file_card(),
            BrowseCard::for_test(
                "Gatherer",
                "",
                "Skilling",
                &[],
                CardRuntime::Compiled,
                CardSource::Builtin,
            ),
        ];
        let pane = ScriptPane::new(ScriptRunState::Idle, None, &cards, &[], true, false);
        let area = Rect::new(0, 0, 80, 16);
        // Category header occupies the first browse row; card name is next.
        assert_eq!(
            pane.on_click(area, 2, 4),
            ScriptClick::Pick(0),
            "first card row picks card 0"
        );
        let text = render(
            ScriptPane::new(
                ScriptRunState::Idle,
                Some(&ScriptChoice::loaded(CardSource::Catalog, "BoneBurier")),
                &cards,
                &[],
                true,
                false,
            ),
            80,
            16,
        );
        assert!(text.contains('>'), "selected card is marked: {text:?}");
        let selection = ScriptChoice::compiled(
            CompiledCardId::lookup("Gatherer").expect("compiled Gatherer card"),
        );
        let text = render(
            ScriptPane::new(
                ScriptRunState::Idle,
                Some(&selection),
                &cards,
                &[],
                true,
                false,
            ),
            80,
            16,
        );
        assert!(
            text.contains("> Gatherer"),
            "the compiled selection is marked in both builds: {text:?}"
        );
    }

    #[cfg(feature = "js")]
    #[test]
    fn deferred_browse_shows_import_catalog() {
        let text = render(
            ScriptPane::new(ScriptRunState::Idle, None, &[], &[], true, false)
                .with_js_controls(true, false, false),
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
            ScriptPane::new(ScriptRunState::Paused, None, &[], &[], false, false),
            60,
            4,
        );
        assert!(
            text.contains("[Resume P]"),
            "paused paints Resume: {text:?}"
        );
        assert!(
            !text.contains("[Pause P]"),
            "paused must not paint Pause: {text:?}"
        );
    }

    #[test]
    fn running_state_shows_pause_not_resume() {
        let text = render(
            ScriptPane::new(ScriptRunState::Running, None, &[], &[], false, false),
            60,
            4,
        );
        assert!(text.contains("[Pause P]"), "running paints Pause: {text:?}");
        assert!(
            !text.contains("[Resume P]"),
            "running must not paint Resume: {text:?}"
        );
    }

    #[test]
    fn clicking_resume_when_paused_returns_resume_button() {
        let area = Rect::new(0, 0, 60, 4);
        let pane = ScriptPane::new(ScriptRunState::Paused, None, &[], &[], false, false);
        assert_eq!(
            pane.on_click(area, 23, 2),
            ScriptClick::Button("Resume"),
            "Resume is clickable when paused"
        );
    }
    #[test]
    fn native_block_is_visible_without_a_quester_window() {
        let status = NativeStatus::blocked(
            CompiledCardId::QUESTER,
            "queue-blocked",
            "quest list unavailable; log in normally, then start the Quester again",
        );
        let text = render(
            ScriptPane::new(ScriptRunState::Running, None, &[], &[], false, false)
                .with_native_status(Some(status.view())),
            90,
            10,
        );
        assert!(text.contains("script: blocked"), "{text:?}");
        assert!(text.contains("quest list unavailable"), "{text:?}");
        assert!(text.contains("start the Quester again"), "{text:?}");
        assert!(!text.contains("script: running"), "{text:?}");
    }

    #[test]
    fn retained_walk_assessment_has_a_walk_risk_row_without_native_status() {
        let text = render(
            ScriptPane::new(ScriptRunState::Idle, None, &[], &[], false, false)
                .with_walk_risk(Some("Route assessment unknown: Poison")),
            90,
            10,
        );
        assert!(
            text.contains("Walk risk: Route assessment unknown: Poison"),
            "{text:?}"
        );
    }

    #[test]
    fn folder_path_marking_is_visible_at_common_sizes() {
        let live = std::env::var_os("QUESTER_PATHS_LIVE_STATUS").map(|path| {
            serde_json::from_slice::<serde_json::Value>(&std::fs::read(path).unwrap()).unwrap()
        });
        let fields = live
            .as_ref()
            .map(|receipt| &receipt["final_status"]["fields"]);
        let source = fields
            .map(|fields| fields["path_source"].as_str().unwrap())
            .unwrap_or("folder");
        let comment = fields
            .map(|fields| fields["step_comment"].as_str().unwrap())
            .unwrap_or("Folder Path live-edit witness: accept Cook's request");
        let mut status = NativeStatus::working(CompiledCardId::QUESTER)
            .with_text_field("path_source", "Path source", source)
            .with_text_field("step_comment", "Step comment", comment);
        for (width, height) in [(80, 24), (120, 40)] {
            let text = render(
                ScriptPane::new(ScriptRunState::Running, None, &[], &[], false, false)
                    .with_native_status(Some(status.view())),
                width,
                height,
            );
            assert!(text.contains("Path source: folder"), "{text:?}");
            assert!(
                text.contains(&format!("Step comment: {comment}")),
                "{text:?}"
            );
            if let Some(directory) = std::env::var_os("QUESTER_PATHS_TUI_EVIDENCE") {
                let directory = std::path::PathBuf::from(directory);
                std::fs::create_dir_all(&directory).unwrap();
                let mut rows = String::with_capacity(text.len() + usize::from(height));
                for (index, character) in text.chars().enumerate() {
                    if index > 0 && index % usize::from(width) == 0 {
                        rows.push('\n');
                    }
                    rows.push(character);
                }
                std::fs::write(
                    directory.join(format!("path-source-{width}x{height}.txt")),
                    rows,
                )
                .unwrap();
            }
        }
        status.keep_fields(&["step_comment"]);
        assert!(
            frontend_core::views::script_status_rows(status.view())
                .next()
                .is_none(),
            "a bundled-only status retains the existing short rows"
        );
    }

    /// Two File rows that show one name; only their identity token, path and
    /// provenance differ.
    fn same_name_pair() -> Vec<BrowseCard> {
        ["one", "two"]
            .into_iter()
            .map(|folder| {
                let path = format!("/work/{folder}/a/very/long/folder/name/for/a/script/miner.ts");
                let mut card = BrowseCard::for_test(
                    "Miner",
                    "Digs ore",
                    "Skilling",
                    &["mining"],
                    CardRuntime::NativeTick,
                    CardSource::File,
                );
                card.shared_name = true;
                card.origin = Some(Box::new(CardOrigin {
                    identity_id: path.clone(),
                    path: Some(path.clone().into()),
                    provenance: format!("File: {path}"),
                    details: vec![
                        ("Source", "File".to_owned()),
                        ("Path", path),
                        ("Version", "1".to_owned()),
                        ("Author", "Alice".to_owned()),
                        ("API", "v2 (native)".to_owned()),
                        ("Declares", "no host capabilities".to_owned()),
                    ],
                }));
                card
            })
            .collect()
    }

    /// The pane's inner rows, one string each: the buffer `render` returns is
    /// cut into rows, and the border cells are dropped.
    fn inner_rows(text: &str, width: usize) -> Vec<String> {
        let rows: Vec<String> = text
            .chars()
            .collect::<Vec<_>>()
            .chunks(width)
            .map(|row| row.iter().collect())
            .collect();
        rows[1..rows.len() - 1]
            .iter()
            .map(|row| row.chars().skip(1).take(width - 2).collect())
            .collect()
    }

    #[test]
    fn equal_names_are_selected_marked_and_told_apart_by_identity_at_both_widths() {
        let cards = same_name_pair();
        let first = card_selection(&cards[0]).unwrap();
        let second = card_selection(&cards[1]).unwrap();
        assert_ne!(first, second, "one name, two identities");
        assert_eq!(
            second.loaded_card(),
            Some((CardSource::File, cards[1].identity()))
        );
        for (width, height) in [(120u16, 40u16), (80, 40)] {
            let area = Rect::new(0, 0, width, height);
            let pane = ScriptPane::new(
                ScriptRunState::Idle,
                Some(&second),
                &cards,
                &[],
                true,
                false,
            );
            let painted = inner_rows(&render(pane, width, height), usize::from(width));
            let marked: Vec<&String> = painted
                .iter()
                .filter(|row| row.contains("> Miner"))
                .collect();
            assert_eq!(marked.len(), 1, "{width}: one selected row: {painted:#?}");
            assert!(
                painted.iter().all(|row| cells(row) <= usize::from(width)),
                "{width}: no row is wider than the pane"
            );
            let squeezed: String = painted
                .iter()
                .flat_map(|row| row.split_whitespace())
                .collect();
            // The unselected card shows its provenance, the selected one its
            // rows; neither path is clipped.
            assert!(
                squeezed.contains("File:/work/one/a/very/long/folder/name/for/a/script/miner.ts"),
                "{width}: {painted:#?}"
            );
            for needle in [
                "Path:/work/two/a/very/long/folder/name/for/a/script/miner.ts",
                "Version:1",
                "Author:Alice",
                "API:v2(native)",
                "Declares:nohostcapabilities",
            ] {
                assert!(squeezed.contains(needle), "{width}: {needle}: {painted:#?}");
            }
            // Click mapping follows the wrapped rows: the first row under the
            // first card's title picks card 0, the second card's title picks
            // card 1.
            let click = ScriptPane::new(
                ScriptRunState::Idle,
                Some(&second),
                &cards,
                &[],
                true,
                false,
            );
            let title_rows: Vec<u16> = painted
                .iter()
                .enumerate()
                .filter(|(_, row)| row.contains("Miner  [NativeTick]"))
                .map(|(y, _)| y as u16)
                .collect();
            assert_eq!(title_rows.len(), 2, "{painted:#?}");
            assert_eq!(
                click.on_click(area, 4, title_rows[0] + 1),
                ScriptClick::Pick(0)
            );
            assert_eq!(
                click.on_click(area, 4, title_rows[1] + 1),
                ScriptClick::Pick(1)
            );
        }
    }

    #[test]
    fn the_browse_filter_hides_unmatched_cards_and_their_empty_categories() {
        let mut cards = vec![
            BrowseCard::for_test(
                "A",
                "",
                "Combat",
                &[],
                CardRuntime::Compat,
                CardSource::Catalog,
            ),
            BrowseCard::for_test(
                "B",
                "",
                "Prayer",
                &[],
                CardRuntime::Compat,
                CardSource::Catalog,
            ),
        ];
        cards[0].filter_match = false;
        let order: Vec<String> = Vec::new();
        let lines = super::retain_matching(browse_lines_for_test(&cards, &order, false), &cards);
        assert_eq!(
            lines,
            [BrowseLine::Category("Prayer".into()), BrowseLine::Card(1)]
        );
        cards[1].filter_match = false;
        let lines = super::retain_matching(browse_lines_for_test(&cards, &order, false), &cards);
        assert_eq!(lines, [BrowseLine::NoMatch]);
    }
}
