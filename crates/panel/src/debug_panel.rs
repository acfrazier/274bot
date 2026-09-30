//! Content-derived debug command catalog and its Settings-style panel.
//!
//! The panel deliberately owns only UI state.  Admission, target selection,
//! queueing and logging remain in [`Session::send_debug_command`].  Command
//! names and name pickers come from the selected game-data pin; this module
//! does not maintain a second cheat table.

use api::debug_commands::{DebugCommand, DebugName};
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSend {
    command_name: String,
    wire: String,
    target: String,
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
    filter_catalog: Option<CatalogKey>,
    filter_query: String,
    filtered_indices: Vec<usize>,
    selected_catalog: Option<CatalogKey>,
    picker_cached_kind: String,
    picker_cached_query: String,
    picker_hits: Vec<DebugName>,
}

/// Draw the body of the Settings-style Debug tab.
///
/// The containing window and the existing Teleports popup are owned by
/// `app.rs`; the link below only sets `Session::debug_open_teleports`.
pub fn draw(ui: &Ui, session: &mut Session) {
    let data = session.selected_game_data();
    let commands: &[DebugCommand] = data.as_deref().map_or(&[], |data| data.debug_commands());

    draw_target_header(ui, session);
    if !session.debug_ui() {
        ui.text_colored(ERROR, "Debug commands require a Local profile.");
        return;
    }

    draw_search_row(ui, session);
    if ui.button("Open Teleports") {
        session.debug_open_teleports = true;
    }
    ui.set_item_tooltip("Open the existing Teleports controls; this tab does not duplicate them.");

    if data.is_none() {
        ui.text_wrapped(
            "No content-derived Debug catalog is available for this profile's cache identity. Rebind a profile whose cache matches the generated facts to enable Debug commands and name pickers.",
        );
    }

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

    draw_name_picker(ui, session, data.as_deref());
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
        .selected_game_data()
        .and_then(|data| command_for_name(data.debug_commands(), name).map(default_argument_values))
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

fn clear_selection(session: &mut Session) {
    session.debug_panel.selected_command = None;
    session.debug_panel.values.clear();
    session.debug_panel.picker_arg = None;
    session.debug_panel.picker_query.clear();
    session.debug_panel.picker_cached_kind.clear();
    session.debug_panel.picker_cached_query.clear();
    session.debug_panel.picker_hits.clear();
    session.debug_panel.pending_send = None;
    session.debug_panel.status = None;
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
            if ui.button("Send##debug-send") {
                if command.destructive {
                    let target = focused_target(session);
                    session.debug_panel.pending_send = Some(PendingSend {
                        command_name: command.name.clone(),
                        wire,
                        target,
                    });
                    ui.open_popup(CONFIRM_POPUP);
                } else {
                    send_wire(session, &command.name, &wire);
                }
            }
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

fn draw_name_picker(
    ui: &Ui,
    session: &mut Session,
    data: Option<&api::game_data::SelectedGameData>,
) {
    let Some(index) = session.debug_panel.picker_arg else {
        return;
    };
    let Some(command_name) = session.debug_panel.selected_command.clone() else {
        return;
    };
    let Some(kind) = data
        .and_then(|data| command_for_name(data.debug_commands(), &command_name))
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
            session.debug_panel.picker_hits = data
                .map(|data| data.search_debug_names(&kind, &query, NAME_PICKER_LIMIT))
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

fn draw_confirmation(ui: &Ui, session: &mut Session) {
    let Some(pending) = session.debug_panel.pending_send.clone() else {
        return;
    };
    let current_target = focused_target(session);
    if current_target != pending.target {
        session.debug_panel.pending_send = None;
        session.debug_panel.status = Some(PanelStatus {
            success: false,
            text: format!(
                "Confirmation cancelled: focused account changed from {} to {}.",
                pending.target, current_target
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
            pending.wire, pending.target
        ));
        if ui.button("Confirm send##debug-confirm") {
            if focused_target(session) == pending.target {
                send_wire(session, &pending.command_name, &pending.wire);
            } else {
                session.debug_panel.pending_send = None;
                session.debug_panel.status = Some(PanelStatus {
                    success: false,
                    text: "Confirmation cancelled: focused account changed.".into(),
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

fn focused_target(session: &Session) -> String {
    session
        .focused_name()
        .unwrap_or_else(|| "(none)".to_string())
}
fn send_wire(session: &mut Session, command_name: &str, wire: &str) {
    let target = focused_target(session);
    session.debug_panel.pending_send = None;
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

fn save_prefs(session: &Session) {
    if session.persist_ui {
        crate::ui_state::save(&session.ui);
    }
}

#[cfg(test)]
mod tests {
    use super::{DebugPanelPrefs, MAX_FAVORITES, MAX_RECENTS};

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
