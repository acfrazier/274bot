//! Content-derived debug command catalog and its Settings-style panel.
//!
//! The panel deliberately owns only UI state.  Admission, target selection,
//! queueing and logging remain in [`Session::send_debug_command`].  Command
//! names and name pickers come from the selected game-data pin; this module
//! does not maintain a second cheat table.

use std::sync::Arc;

use api::debug_commands::{DebugCatalog, DebugCommand, DebugName};
use dear_imgui_rs::{StyleColor, TreeNodeFlags, Ui};
use serde::{Deserialize, Serialize};

use crate::session::Session;
use crate::theme::{ACCENT, ERROR, GREEN};

const NAME_PICKER_KINDS: &[&str] = &[
    "obj",
    "namedobj",
    "npc",
    "loc",
    "seq",
    "spotanim",
    "interface",
    "stat",
    "varp",
    "inv",
    "idkit",
];
const NAME_PICKER_LIMIT: usize = 40;
const MAX_RECENTS: usize = 8;
const MAX_FAVORITES: usize = 32;
const PICKER_POPUP: &str = "##debug-name-picker";
const CONFIRM_POPUP: &str = "##debug-destructive-confirm";
const CATEGORY_ORDER: &[&str] = &["Account", "Item", "Teleport", "Quest", "Client & Engine"];

/// Persisted command convenience state in [`crate::ui_state::PanelUiState`].
///
/// Entries are wire command names (including `~` for content debugprocs), not
/// formatted invocations.  Arguments are intentionally not persisted: a
/// command may change shape when the selected content pin changes and stale
/// argument values are unsafe to replay.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DebugPanelPrefs {
    /// Most recently successfully queued command names, newest first.
    pub recents: Vec<String>,
    /// Command names pinned by the operator, in insertion order.
    pub favorites: Vec<String>,
}

impl DebugPanelPrefs {
    /// Keep persisted state bounded and remove empty/duplicate entries.
    ///
    /// This is also used when loading old or hand-edited preferences, so a
    /// malformedly large list never becomes a per-frame UI allocation.
    pub fn normalize(&mut self) {
        normalize_names(&mut self.recents, MAX_RECENTS);
        normalize_names(&mut self.favorites, MAX_FAVORITES);
    }

    /// Remember one successfully queued command, newest first.
    pub fn remember_recent(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.recents.retain(|entry| entry != name);
        self.recents.insert(0, name.to_string());
        self.recents.truncate(MAX_RECENTS);
    }

    /// Toggle a command pin and return its new pinned state. When full, the
    /// oldest pin is evicted so the newly chosen command is visible.
    pub fn toggle_favorite(&mut self, name: &str) -> bool {
        let name = name.trim();
        if name.is_empty() {
            return false;
        }
        if let Some(index) = self.favorites.iter().position(|entry| entry == name) {
            self.favorites.remove(index);
            false
        } else {
            if self.favorites.len() >= MAX_FAVORITES {
                let remove = self.favorites.len() - (MAX_FAVORITES - 1);
                self.favorites.drain(..remove);
            }
            self.favorites.push(name.to_string());
            true
        }
    }

    pub fn is_favorite(&self, name: &str) -> bool {
        self.favorites.iter().any(|entry| entry == name)
    }
}

fn normalize_names(names: &mut Vec<String>, limit: usize) {
    let mut seen = std::collections::HashSet::with_capacity(names.len().min(limit));
    names.retain(|name| !name.trim().is_empty() && seen.insert(name.clone()));
    names.truncate(limit);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DebugTargetMode {
    #[default]
    Focused,
    Marked,
}

type TargetRow = (frontend_core::ProfileIdentity, String);

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSend {
    command_name: String,
    wire: String,
    targets: Vec<TargetRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PanelStatus {
    success: bool,
    text: String,
}

/// A catalog identity is a stable pointer/length pair for the generated
/// command slice. Filtered indices are rebuilt only when this or the query
/// changes, rather than cloning the command catalog every frame.
type CatalogKey = (usize, usize);

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandAction {
    Select(String),
    ToggleFavorite(String),
}

/// Ephemeral ImGui state for the Debug tab.
///
/// Command argument buffers intentionally live here rather than in
/// [`DebugPanelPrefs`].  They are reset whenever the selected command changes
/// or the selected game-data pin changes, preventing stale values from being
/// silently sent to a different content table.
#[derive(Debug, Clone, Default)]
pub struct DebugPanelState {
    search: String,
    selected_command: Option<String>,
    values: Vec<String>,
    picker_arg: Option<usize>,
    picker_query: String,
    pending_send: Option<PendingSend>,
    status: Option<PanelStatus>,
    target_mode: DebugTargetMode,
    filter_catalog: Option<CatalogKey>,
    filter_query: String,
    filtered_indices: Vec<usize>,
    selected_catalog: Option<CatalogKey>,
    picker_cached_kind: String,
    picker_cached_query: String,
    picker_hits: Vec<DebugName>,
    /// Decoded only while the Debug window is open. The selected game facts
    /// never retain this catalog.
    catalog: Option<Arc<DebugCatalog>>,
    catalog_error: Option<String>,
    catalog_profile: Option<usize>,
}

impl DebugPanelState {
    pub(crate) fn catalog(&self) -> Option<&DebugCatalog> {
        self.catalog.as_deref()
    }

    pub(crate) fn catalog_profile(&self) -> Option<usize> {
        self.catalog_profile
    }

    pub(crate) fn set_catalog_profile(&mut self, profile: Option<usize>) {
        self.catalog_profile = profile;
    }

    pub(crate) fn catalog_error(&self) -> Option<&str> {
        self.catalog_error.as_deref()
    }

    pub(crate) fn attach_catalog(&mut self, catalog: Arc<DebugCatalog>) {
        self.catalog = Some(catalog);
        self.catalog_error = None;
        self.filter_catalog = None;
        self.selected_catalog = None;
        self.picker_cached_kind.clear();
        self.picker_cached_query.clear();
        self.picker_hits.clear();
        reset_selection_state(self);
    }

    pub(crate) fn set_catalog_error(&mut self, error: String) {
        self.catalog = None;
        self.catalog_error = Some(error);
        self.filter_catalog = None;
        self.selected_catalog = None;
        self.filtered_indices.clear();
        reset_selection_state(self);
    }

    /// Drop the lazily decoded catalog and all command-derived ephemeral
    /// state. Persisted recents/favorites live in [`PanelUiState`] instead.
    pub(crate) fn release_catalog(&mut self) {
        self.catalog = None;
        self.catalog_error = None;
        self.catalog_profile = None;
        self.filter_catalog = None;
        self.selected_catalog = None;
        self.filtered_indices.clear();
        self.picker_cached_kind.clear();
        self.picker_cached_query.clear();
        self.picker_hits.clear();
        reset_selection_state(self);
    }

    pub(crate) fn clear_target_feedback(&mut self) {
        self.pending_send = None;
        self.status = None;
    }
}

/// Draw the body of the Settings-style Debug tab.
///
/// The containing window and the existing Teleports popup are owned by
/// `app.rs`; the link below only sets `Session::debug_open_teleports`.
pub fn draw(ui: &Ui, session: &mut Session) {
    draw_target_header(ui, session);
    if !session.debug_ui() {
        ui.text_colored(ERROR, "Debug commands require a Local profile.");
        return;
    }
    if let Err(error) = session.ensure_debug_catalog() {
        ui.text_colored(ERROR, format!("Debug catalog unavailable: {error}"));
        return;
    }
    // Clone only the Arc handle: command rows remain borrowed from the
    // lazily attached catalog while the panel's mutable UI state changes.
    let catalog = session.debug_panel.catalog.clone();
    let commands: &[DebugCommand] = catalog.as_deref().map_or(&[], DebugCatalog::commands);

    draw_target_controls(ui, session);
    draw_search_row(ui, session);
    if ui.button("Open Teleports") {
        session.debug_open_teleports = true;
    }
    ui.set_item_tooltip("Open the existing Teleports controls; this tab does not duplicate them.");

    let selected_catalog = catalog_key(commands);
    if session.debug_panel.selected_catalog != Some(selected_catalog) {
        session.debug_panel.selected_catalog = Some(selected_catalog);
        clear_selection(session);
    }

    let selected_name = session.debug_panel.selected_command.clone();
    if let Some(name) = selected_name.as_deref() {
        if command_for_name(commands, name).is_none_or(|command| command.production_only) {
            clear_selection(session);
        } else if let Some(command) = command_for_name(commands, name) {
            let expected = command.args.len();
            if session.debug_panel.values.len() != expected {
                session.debug_panel.values = default_argument_values(command);
            }
        }
    }

    refresh_filter_cache(&mut session.debug_panel, commands);
    let visible_indices = std::mem::take(&mut session.debug_panel.filtered_indices);
    let search_active = !session.debug_panel.search.trim().is_empty();
    draw_saved_commands(ui, session, commands);

    if let Some(name) = session.debug_panel.selected_command.clone() {
        if let Some(command) =
            command_for_name(commands, &name).filter(|command| !command.production_only)
        {
            draw_command_editor(ui, session, command);
        }
    }

    ui.separator();
    ui.text_colored(ACCENT, "Commands");
    if visible_indices.is_empty() {
        ui.text_disabled(if commands.is_empty() {
            "No content-derived debug commands are available."
        } else {
            "No commands match the search."
        });
    } else {
        draw_command_groups(ui, session, commands, &visible_indices, search_active);
    }

    draw_name_picker(ui, session, catalog.as_deref());
    draw_confirmation(ui, session);
    session.debug_panel.filtered_indices = visible_indices;
}

fn draw_target_header(ui: &Ui, session: &Session) {
    ui.text_colored(ACCENT, "Debug");
    ui.text_wrapped(format!(
        "Focused account: {}",
        session.focused_name().as_deref().unwrap_or("(none)")
    ));
}

fn target_rows(session: &Session) -> Vec<TargetRow> {
    let rows = session.debug_target_snapshot();
    match session.debug_panel.target_mode {
        DebugTargetMode::Focused => session
            .focused_name()
            .and_then(|name| rows.into_iter().find(|(_, row)| *row == name))
            .into_iter()
            .collect(),
        DebugTargetMode::Marked => rows
            .into_iter()
            .filter(|(identity, _)| session.fleet_selection.contains(*identity))
            .collect(),
    }
}

fn target_description(rows: &[TargetRow]) -> String {
    if rows.is_empty() {
        return "(none)".into();
    }
    let names = rows
        .iter()
        .map(|(_, name)| name.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if rows.len() == 1 {
        names
    } else {
        format!("{} marked bots: {names}", rows.len())
    }
}

fn draw_target_controls(ui: &Ui, session: &mut Session) {
    ui.text_colored(ACCENT, "Target");
    if ui.button("Focused bot##debug-target-focused") {
        session.debug_panel.target_mode = DebugTargetMode::Focused;
        session.debug_panel.clear_target_feedback();
    }
    ui.same_line();
    let marked_count = session.fleet_selection.len();
    let _disabled = (marked_count == 0).then(|| ui.begin_disabled());
    if ui.button(format!("Marked bots ({marked_count})##debug-target-marked")) {
        session.debug_panel.target_mode = DebugTargetMode::Marked;
        session.debug_panel.clear_target_feedback();
    }
    drop(_disabled);
    ui.text_wrapped(format!(
        "Actual targets: {}",
        target_description(&target_rows(session))
    ));
}

fn draw_search_row(ui: &Ui, session: &mut Session) {
    ui.set_next_item_width(-1.0);
    ui.input_text("##debug-search", &mut session.debug_panel.search)
        .hint("Search commands, categories, descriptions")
        .build();
}

fn command_for_name<'a>(commands: &'a [DebugCommand], name: &str) -> Option<&'a DebugCommand> {
    commands.iter().find(|command| command.name == name)
}

fn catalog_key(commands: &[DebugCommand]) -> CatalogKey {
    (commands.as_ptr() as usize, commands.len())
}

fn refresh_filter_cache(state: &mut DebugPanelState, commands: &[DebugCommand]) {
    let key = catalog_key(commands);
    if state.filter_catalog == Some(key) && state.filter_query == state.search {
        return;
    }
    state.filter_catalog = Some(key);
    state.filter_query.clone_from(&state.search);
    state.filtered_indices.clear();
    let query = state.filter_query.trim().to_ascii_lowercase();
    for (index, command) in commands.iter().enumerate() {
        if !command.production_only && command_matches(command, &query) {
            state.filtered_indices.push(index);
        }
    }
}

fn command_matches(command: &DebugCommand, query: &str) -> bool {
    query.is_empty()
        || command.name.to_ascii_lowercase().contains(query)
        || command.category.to_ascii_lowercase().contains(query)
        || command.description.to_ascii_lowercase().contains(query)
        || command.args.iter().any(|arg| {
            arg.name.to_ascii_lowercase().contains(query)
                || arg.kind.to_ascii_lowercase().contains(query)
        })
}

fn draw_saved_commands(ui: &Ui, session: &mut Session, commands: &[DebugCommand]) {
    let mut picked = None;
    {
        let prefs = &session.ui.debug_panel;
        if !prefs.favorites.is_empty() {
            ui.text_colored(ACCENT, "Favorites");
            picked = draw_saved_rows(ui, commands, &prefs.favorites, "favorite");
            ui.spacing();
        }
        if !prefs.recents.is_empty() {
            ui.text_colored(ACCENT, "Recent");
            picked = draw_saved_rows(ui, commands, &prefs.recents, "recent").or(picked);
            ui.spacing();
        }
    }
    if let Some(name) = picked {
        select_command(session, &name);
    }
}

fn draw_saved_rows(
    ui: &Ui,
    commands: &[DebugCommand],
    names: &[String],
    prefix: &str,
) -> Option<String> {
    let _prefix = ui.push_id(prefix);
    let mut picked = None;
    for (index, name) in names.iter().enumerate() {
        let Some(command) =
            command_for_name(commands, name).filter(|command| !command.production_only)
        else {
            continue;
        };
        let _index = ui.push_id(index);
        if ui.small_button(command.name.as_str()) {
            picked = Some(command.name.clone());
        }
        if command.destructive && ui.is_item_hovered() {
            destructive_tooltip(ui);
        }
        if !command.description.is_empty() {
            ui.text_wrapped(command.description.as_str());
        }
    }
    picked
}

fn draw_command_groups(
    ui: &Ui,
    session: &mut Session,
    commands: &[DebugCommand],
    indices: &[usize],
    search_active: bool,
) {
    let mut action = None;
    for category in CATEGORY_ORDER {
        action = action.or(draw_command_category(
            ui,
            session,
            commands,
            indices,
            category,
            search_active,
        ));
    }
    for (position, &index) in indices.iter().enumerate() {
        let category = commands[index].category.as_str();
        if CATEGORY_ORDER.contains(&category)
            || indices[..position]
                .iter()
                .any(|&prior| commands[prior].category == category)
        {
            continue;
        }
        action = action.or(draw_command_category(
            ui,
            session,
            commands,
            indices,
            category,
            search_active,
        ));
    }
    match action {
        Some(CommandAction::Select(name)) => select_command(session, &name),
        Some(CommandAction::ToggleFavorite(name)) => {
            session.ui.debug_panel.toggle_favorite(&name);
            save_prefs(session);
        }
        None => {}
    }
}

fn draw_command_category(
    ui: &Ui,
    session: &Session,
    commands: &[DebugCommand],
    indices: &[usize],
    category: &str,
    search_active: bool,
) -> Option<CommandAction> {
    let count = indices
        .iter()
        .filter(|&&index| commands[index].category == category)
        .count();
    if count == 0 {
        return None;
    }
    if search_active {
        ui.set_next_item_open(true);
    }
    let header = format!("{category} ({count})###debug-category-{category}");
    if !ui.collapsing_header(header, TreeNodeFlags::FRAME_PADDING) {
        return None;
    }

    let mut action = None;
    for &index in indices {
        let command = &commands[index];
        if command.category != category {
            continue;
        }
        let _id = ui.push_id(command.name.as_str());
        let selected = session
            .debug_panel
            .selected_command
            .as_deref()
            .is_some_and(|name| name == command.name);
        let command_width = (ui.content_region_avail()[0] - 34.0).max(110.0);
        if ui
            .selectable_config(command.name.as_str())
            .selected(selected)
            .size([command_width, 0.0])
            .build()
        {
            action = Some(CommandAction::Select(command.name.clone()));
        }
        if command.destructive && ui.is_item_hovered() {
            destructive_tooltip(ui);
        }
        ui.same_line();
        let favorite = session.ui.debug_panel.is_favorite(&command.name);
        let star_label = if favorite {
            "★##favorite"
        } else {
            "☆##favorite"
        };
        if ui.small_button(star_label) {
            action = Some(CommandAction::ToggleFavorite(command.name.clone()));
        }
        if !command.description.is_empty() {
            ui.text_wrapped(command.description.as_str());
        }
    }
    action
}

fn default_argument_values(command: &DebugCommand) -> Vec<String> {
    command
        .args
        .iter()
        .map(|argument| {
            if argument.kind == "int" && !argument.optional {
                "0".to_string()
            } else {
                String::new()
            }
        })
        .collect()
}

fn select_command(session: &mut Session, name: &str) {
    let values = session
        .debug_panel
        .catalog
        .as_ref()
        .and_then(|catalog| command_for_name(catalog.commands(), name).map(default_argument_values))
        .unwrap_or_default();
    session.debug_panel.selected_command = Some(name.to_string());
    session.debug_panel.values = values;
    session.debug_panel.picker_arg = None;
    session.debug_panel.picker_query.clear();
    session.debug_panel.picker_cached_kind.clear();
    session.debug_panel.picker_cached_query.clear();
    session.debug_panel.picker_hits.clear();
    session.debug_panel.pending_send = None;
    session.debug_panel.status = None;
}

fn reset_selection_state(state: &mut DebugPanelState) {
    state.selected_command = None;
    state.values.clear();
    state.picker_arg = None;
    state.picker_query.clear();
    state.picker_cached_kind.clear();
    state.picker_cached_query.clear();
    state.picker_hits.clear();
    state.pending_send = None;
    state.status = None;
}

fn clear_selection(session: &mut Session) {
    reset_selection_state(&mut session.debug_panel);
}

fn draw_command_editor(ui: &Ui, session: &mut Session, command: &DebugCommand) {
    ui.separator();
    ui.text_colored(ACCENT, format!("{}  [{}]", command.name, command.category));
    if command.destructive {
        ui.same_line();
        ui.text_colored(ERROR, "destructive");
    }
    if !command.description.is_empty() {
        ui.text_wrapped(command.description.as_str());
    }

    for (index, argument) in command.args.iter().enumerate() {
        let label = if argument.optional {
            format!("{} ({}, optional)", argument.name, argument.kind)
        } else {
            format!("{} ({})", argument.name, argument.kind)
        };
        ui.text_wrapped(label);
        let picker = is_picker_kind(&argument.kind);
        let trailing = if picker { 54.0 } else { 0.0 } + if argument.optional { 58.0 } else { 0.0 };
        ui.set_next_item_width((ui.content_region_avail()[0] - trailing).max(80.0));
        let mut changed = false;
        if argument.kind == "int" && !argument.optional {
            let mut typed = session
                .debug_panel
                .values
                .get(index)
                .and_then(|value| value.parse::<i32>().ok())
                .unwrap_or_default();
            if ui.input_int(format!("##debug-arg-{index}"), &mut typed) {
                if let Some(value) = session.debug_panel.values.get_mut(index) {
                    *value = typed.to_string();
                }
                changed = true;
            }
        } else if let Some(value) = session.debug_panel.values.get_mut(index) {
            changed = ui
                .input_text(format!("##debug-arg-{index}"), value)
                .hint(argument.kind.as_str())
                .build();
        }
        if changed && !picker {
            session.debug_panel.picker_query.clear();
        }
        if picker {
            ui.same_line();
            if ui.small_button(format!("Pick##debug-pick-{index}")) {
                session.debug_panel.picker_arg = Some(index);
                session.debug_panel.picker_query = session
                    .debug_panel
                    .values
                    .get(index)
                    .cloned()
                    .unwrap_or_default();
                ui.open_popup(PICKER_POPUP);
            }
        }
        if argument.optional {
            ui.same_line();
            if ui.small_button(format!("Clear##debug-clear-{index}")) {
                if let Some(value) = session.debug_panel.values.get_mut(index) {
                    value.clear();
                }
            }
        }
    }

    let formatted = command.format_command(&session.debug_panel.values);
    match formatted {
        Ok(wire) => {
            ui.text_disabled(format!("{} / 80 bytes", wire.len()));
            if command.destructive {
                ui.set_item_tooltip(
                    "Destructive command — confirmation is required before sending.",
                );
            }
            let targets = target_rows(session);
            let _disabled = (targets.is_empty()).then(|| ui.begin_disabled());
            if ui.button("Send##debug-send") {
                if command.destructive {
                    session.debug_panel.pending_send = Some(PendingSend {
                        command_name: command.name.clone(),
                        wire,
                        targets,
                    });
                    ui.open_popup(CONFIRM_POPUP);
                } else {
                    send_wire(session, &command.name, &wire, targets);
                }
            }
            drop(_disabled);
        }
        Err(error) => {
            let color = ui.push_style_color(StyleColor::Text, ERROR);
            ui.text_wrapped(error);
            color.pop();
            let _off = ui.begin_disabled();
            let _ = ui.button("Send##debug-send");
        }
    }

    if let Some(status) = &session.debug_panel.status {
        let _color =
            ui.push_style_color(StyleColor::Text, if status.success { GREEN } else { ERROR });
        ui.text_wrapped(&status.text);
    }
}

fn is_picker_kind(kind: &str) -> bool {
    NAME_PICKER_KINDS.contains(&kind)
}

fn destructive_tooltip(ui: &Ui) {
    ui.tooltip(|| {
        let _red = ui.push_style_color(StyleColor::Text, ERROR);
        ui.text("Destructive command: it may change or remove game state.");
    });
}

fn draw_name_picker(ui: &Ui, session: &mut Session, catalog: Option<&DebugCatalog>) {
    let Some(index) = session.debug_panel.picker_arg else {
        return;
    };
    let Some(command_name) = session.debug_panel.selected_command.clone() else {
        return;
    };
    let Some(kind) = catalog
        .and_then(|catalog| command_for_name(catalog.commands(), &command_name))
        .and_then(|command| command.args.get(index))
        .map(|argument| argument.kind.clone())
    else {
        return;
    };

    ui.popup(PICKER_POPUP, || {
        ui.text(format!("Pick {kind}"));
        let picker_width = ui.content_region_avail()[0].clamp(180.0, 460.0);
        ui.set_next_item_width(picker_width);
        ui.input_text(
            "##debug-picker-search",
            &mut session.debug_panel.picker_query,
        )
        .hint("Search name or alias")
        .build();

        if session.debug_panel.picker_cached_kind != kind
            || session.debug_panel.picker_cached_query != session.debug_panel.picker_query
        {
            let query = session.debug_panel.picker_query.clone();
            session.debug_panel.picker_cached_kind.clone_from(&kind);
            session.debug_panel.picker_cached_query = query.clone();
            session.debug_panel.picker_hits = catalog
                .map(|catalog| catalog.search_names(&kind, &query, NAME_PICKER_LIMIT))
                .unwrap_or_default();
        }
        if session.debug_panel.picker_hits.is_empty() {
            ui.text_disabled("No matches.");
        }

        let mut picked = None;
        ui.child_window("##debug-picker-hits")
            .size([picker_width, 180.0])
            .build(ui, || {
                for hit in &session.debug_panel.picker_hits {
                    let label = if hit.alias.is_empty() {
                        format!("#{}  {}", hit.id, hit.name)
                    } else {
                        format!("#{}  {}  {}", hit.id, hit.name, hit.alias)
                    };
                    if ui.selectable_config(&label).build() {
                        picked = Some(hit.alias.clone());
                    }
                    if ui.is_item_hovered() {
                        ui.tooltip_text(&label);
                    }
                }
            });

        if let Some(name) = picked {
            set_picked_value(session, index, &name);
            ui.close_current_popup();
        }
        if ui.button("Close##debug-picker-close") {
            session.debug_panel.picker_arg = None;
            ui.close_current_popup();
        }
    });
}

fn set_picked_value(session: &mut Session, index: usize, value: &str) {
    if let Some(slot) = session.debug_panel.values.get_mut(index) {
        *slot = value.trim().to_string();
    }
    session.debug_panel.picker_arg = None;
    session.debug_panel.picker_query.clear();
}

fn same_targets(left: &[TargetRow], right: &[TargetRow]) -> bool {
    let mut left = left.to_vec();
    let mut right = right.to_vec();
    left.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    right.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    left == right
}

fn draw_confirmation(ui: &Ui, session: &mut Session) {
    let Some(pending) = session.debug_panel.pending_send.clone() else {
        return;
    };
    let current_targets = target_rows(session);
    if !same_targets(&current_targets, &pending.targets) {
        session.debug_panel.pending_send = None;
        session.debug_panel.status = Some(PanelStatus {
            success: false,
            text: format!(
                "Confirmation cancelled: targets changed from {} to {}.",
                target_description(&pending.targets),
                target_description(&current_targets)
            ),
        });
        return;
    }

    ui.popup(CONFIRM_POPUP, || {
        let red = ui.push_style_color(StyleColor::Text, ERROR);
        ui.text("Destructive debug command");
        red.pop();
        ui.text_wrapped(format!(
            "Send {} to {}? This command may change or remove game state.",
            pending.wire,
            target_description(&pending.targets)
        ));
        if ui.button("Confirm send##debug-confirm") {
            if same_targets(&target_rows(session), &pending.targets) {
                send_wire(
                    session,
                    &pending.command_name,
                    &pending.wire,
                    pending.targets.clone(),
                );
            } else {
                session.debug_panel.pending_send = None;
                session.debug_panel.status = Some(PanelStatus {
                    success: false,
                    text: "Confirmation cancelled: targets changed.".into(),
                });
            }
            ui.close_current_popup();
        }
        ui.same_line();
        if ui.button("Cancel##debug-confirm-cancel") {
            session.debug_panel.pending_send = None;
            ui.close_current_popup();
        }
    });
}

fn send_wire(session: &mut Session, command_name: &str, wire: &str, targets: Vec<TargetRow>) {
    if !same_targets(&target_rows(session), &targets) {
        session.debug_panel.pending_send = None;
        session.debug_panel.status = Some(PanelStatus {
            success: false,
            text: format!(
                "Send cancelled: targets changed from {} to {}.",
                target_description(&targets),
                target_description(&target_rows(session))
            ),
        });
        return;
    }
    session.debug_panel.pending_send = None;
    match session.debug_panel.target_mode {
        DebugTargetMode::Focused => {
            let target = target_description(&targets);
            match session.send_debug_command(wire) {
                Ok(()) => {
                    session.ui.debug_panel.remember_recent(command_name);
                    save_prefs(session);
                    session.debug_panel.status = Some(PanelStatus {
                        success: true,
                        text: format!("Queued for {target}: {wire}"),
                    });
                }
                Err(error) => {
                    session.debug_panel.status = Some(PanelStatus {
                        success: false,
                        text: format!("Send failed: {error}"),
                    });
                }
            }
        }
        DebugTargetMode::Marked => {
            let report = session.send_debug_command_marked_snapshot(wire, targets.clone());
            if report.accepted > 0 {
                session.ui.debug_panel.remember_recent(command_name);
                save_prefs(session);
            }
            let mut text = if report.accepted > 0 {
                format!(
                    "Queued for {}/{} ({}) : {wire}",
                    report.accepted,
                    report.total(),
                    target_description(&targets)
                )
            } else {
                format!("Send failed for {}: ", target_description(&targets))
            };
            if !report.skipped.is_empty() || report.accepted == 0 {
                if report.accepted > 0 {
                    text.push_str("; ");
                }
                text.push_str(&report.summary("Admission"));
            }
            session.debug_panel.status = Some(PanelStatus {
                success: report.accepted > 0,
                text,
            });
        }
    }
}

fn save_prefs(session: &Session) {
    if session.persist_ui {
        crate::ui_state::save(&session.ui);
    }
}

#[cfg(test)]
mod tests {
    use super::{same_targets, DebugPanelPrefs, MAX_FAVORITES, MAX_RECENTS};

    #[test]
    fn target_snapshot_rejects_identity_or_name_changes() {
        let alice = frontend_core::ProfileIdentity::uid(11);
        let bob = frontend_core::ProfileIdentity::uid(22);
        assert!(same_targets(
            &[(alice, "alice".into()), (bob, "bob".into())],
            &[(bob, "bob".into()), (alice, "alice".into())],
        ));
        assert!(!same_targets(
            &[(alice, "alice".into())],
            &[(alice, "renamed".into())],
        ));
        assert!(!same_targets(
            &[(alice, "alice".into())],
            &[(bob, "bob".into())],
        ));
    }

    #[test]
    fn recent_commands_are_unique_newest_first_and_bounded() {
        let mut prefs = DebugPanelPrefs::default();
        for i in 0..(MAX_RECENTS + 2) {
            prefs.remember_recent(&format!("~cmd{i}"));
        }
        prefs.remember_recent("~cmd2");
        assert_eq!(prefs.recents.first().map(String::as_str), Some("~cmd2"));
        assert_eq!(prefs.recents.len(), MAX_RECENTS);
        assert_eq!(
            prefs.recents.iter().filter(|name| *name == "~cmd2").count(),
            1
        );
    }

    #[test]
    fn favorites_are_unique_and_bounded() {
        let mut prefs = DebugPanelPrefs::default();
        for i in 0..(MAX_FAVORITES + 2) {
            assert!(prefs.toggle_favorite(&format!("cmd{i}")));
        }
        assert_eq!(prefs.favorites.len(), MAX_FAVORITES);
        assert!(!prefs.toggle_favorite("cmd31"));
        assert!(!prefs.is_favorite("cmd31"));
        assert!(prefs.toggle_favorite("cmd31"));
    }

    #[test]
    fn serde_defaults_and_normalizes_bounds() {
        let mut prefs: DebugPanelPrefs = serde_json::from_str("{}").unwrap();
        assert!(prefs.recents.is_empty());
        assert!(prefs.favorites.is_empty());
        prefs.recents = vec!["a".into(), "a".into(), "".into()];
        prefs.normalize();
        assert_eq!(prefs.recents, vec!["a"]);
    }
}
