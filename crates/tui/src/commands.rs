//! Operator commands: one vocabulary for pane keys, visible buttons, the
//! command palette, context menus and the help overlay. A command knows its
//! label, its target scope and whether it is available (with the reason
//! when not); `TuiApp::run_command` executes it. Keys are context
//! shortcuts: each table below is routed only while its pane has keyboard
//! focus, so no letter leaks into another pane or a text field.

use frontend_core::scripts::ScriptRunState;
use frontend_core::{FleetAction, MarkPredicate};

use crate::app::{TuiApp, WalkSendMode};
use crate::layout::Screen;
use crate::script_shape::SCRIPT_KEYS;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Palette,
    Quit,
    ShowFleet,
    Show(Screen),
    FocusNext,
    FocusPrev,
    SelectNextBot,
    SelectPrevBot,
    ToggleMouse,
    ShowMessage,
    FilterFleet,
    /// Choose the card the fleet actions use (opens the card picker).
    PickFleetCard,
    /// Mark every shown row (`MarkFleet(Shown)`).
    MarkAllShown,
    /// Unmark every shown row; hidden marks stay (`MarkFleet(UnmarkShown)`).
    UnmarkShown,
    /// Flip the mark of every shown row (`MarkFleet(InvertShown)`).
    InvertShown,
    /// Mark the shown rows in one status (`MarkFleet(Where(_))`).
    MarkWhere(MarkPredicate),
    ClearMarks,
    MoveProfileUp,
    MoveProfileDown,
    MoveMarkedToTop,
    ImportProfiles,
    /// A fleet action over the marked profiles, or the loaded shown rows
    /// with no marks: core plans it (`PlanFleetAction`), the confirm
    /// overlay shows its prompt when it asks first.
    Fleet(FleetAction),
    Login,
    Logout,
    Remove,
    Settings,
    Loadouts,
    ManualWalk,
    DismissNotice,
    ScriptBrowse,
    ScriptStart,
    ScriptPause,
    ScriptStop,
    ScriptLoad,
    ScriptParams,
    ScriptReload,
    ScriptReloadCancel,
    ImportCatalog,
    ToggleGameChat,
    MapSearch,
    MapRecenter,
    MapGroup,
    MapWalk,
    MapTeleport,
    MapWilderness,
    LogSave,
    LogSessionFile,
}

/// Palette and help grouping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    App,
    Fleet,
    Bot,
    Script,
    Chat,
    Map,
    Logs,
}

impl Group {
    pub fn label(self) -> &'static str {
        match self {
            Group::App => "app",
            Group::Fleet => "fleet",
            Group::Bot => "bot",
            Group::Script => "script",
            Group::Chat => "chat",
            Group::Map => "map",
            Group::Logs => "logs",
        }
    }
}

/// Fleet pane shortcuts: the card picker, the shown-row marks and the
/// fleet actions that live on the pane.
pub const FLEET_KEYS: &[(char, Command)] = &[
    ('c', Command::PickFleetCard),
    ('a', Command::MarkAllShown),
    ('A', Command::UnmarkShown),
    ('m', Command::Fleet(FleetAction::Login)),
    ('U', Command::Fleet(FleetAction::Logout)),
];

/// Overview shortcuts: they act on the selected bot, shown in the pane.
pub const OVERVIEW_KEYS: &[(char, Command)] = &[
    ('i', Command::Login),
    ('u', Command::Logout),
    ('x', Command::Remove),
    ('o', Command::Settings),
    ('l', Command::Loadouts),
    ('w', Command::ManualWalk),
    ('n', Command::DismissNotice),
];

pub const CHAT_KEYS: &[(char, Command)] = &[('p', Command::ToggleGameChat)];

pub const MAP_KEYS: &[(char, Command)] = &[
    ('/', Command::MapSearch),
    ('R', Command::MapRecenter),
    ('g', Command::MapGroup),
    ('t', Command::MapTeleport),
    ('w', Command::MapWilderness),
];

pub const LOG_KEYS: &[(char, Command)] = &[('w', Command::LogSave), ('F', Command::LogSessionFile)];

/// The script pane's command for one of `SCRIPT_KEYS`' names.
pub fn script_command(name: &str) -> Option<Command> {
    Some(match name {
        "Browse" => Command::ScriptBrowse,
        "Start" => Command::ScriptStart,
        "Pause" | "Resume" => Command::ScriptPause,
        "Stop" => Command::ScriptStop,
        "Load" => Command::ScriptLoad,
        "Params" => Command::ScriptParams,
        "Reload" | "Confirm" => Command::ScriptReload,
        "Cancel" => Command::ScriptReloadCancel,
        "Start all" => Command::Fleet(FleetAction::Start),
        "Stop all" => Command::Fleet(FleetAction::Stop),
        _ => return None,
    })
}

/// The script pane's key for `c`, if any (`SCRIPT_KEYS`, the same letters
/// the script buttons show).
pub fn script_key_command(c: char) -> Option<Command> {
    SCRIPT_KEYS
        .iter()
        .find(|(_, key)| *key == c)
        .and_then(|(name, _)| script_command(name))
}

fn table_key(table: &[(char, Command)], command: Command) -> Option<char> {
    table
        .iter()
        .find(|(_, bound)| *bound == command)
        .map(|(key, _)| *key)
}

/// The pane key bound to `command`, for palette and button hints.
pub fn shortcut(command: Command) -> Option<char> {
    [FLEET_KEYS, OVERVIEW_KEYS, CHAT_KEYS, MAP_KEYS, LOG_KEYS]
        .into_iter()
        .find_map(|table| table_key(table, command))
        .or_else(|| {
            SCRIPT_KEYS
                .iter()
                .find(|(name, _)| script_command(name) == Some(command))
                .map(|(_, key)| *key)
        })
}

impl Command {
    /// Every command the palette lists, in display order.
    pub const ALL: &'static [Command] = &[
        Command::Help,
        Command::Palette,
        Command::ShowFleet,
        Command::Show(Screen::Overview),
        Command::Show(Screen::Map),
        Command::Show(Screen::Script),
        Command::Show(Screen::Chat),
        Command::Show(Screen::Logs),
        Command::FocusNext,
        Command::FocusPrev,
        Command::SelectNextBot,
        Command::SelectPrevBot,
        Command::FilterFleet,
        Command::PickFleetCard,
        Command::MarkAllShown,
        Command::UnmarkShown,
        Command::InvertShown,
        Command::MarkWhere(MarkPredicate::Ready),
        Command::MarkWhere(MarkPredicate::Idle),
        Command::MarkWhere(MarkPredicate::Running),
        Command::MarkWhere(MarkPredicate::LoggedOut),
        Command::MarkWhere(MarkPredicate::LoginError),
        Command::MarkWhere(MarkPredicate::NotLoaded),
        Command::ClearMarks,
        Command::MoveProfileUp,
        Command::MoveProfileDown,
        Command::MoveMarkedToTop,
        Command::Fleet(FleetAction::Login),
        Command::Fleet(FleetAction::Logout),
        Command::ImportProfiles,
        Command::Login,
        Command::Logout,
        Command::Remove,
        Command::Settings,
        Command::Loadouts,
        Command::ManualWalk,
        Command::DismissNotice,
        Command::ScriptBrowse,
        Command::ScriptStart,
        Command::ScriptPause,
        Command::ScriptStop,
        Command::ScriptLoad,
        Command::ScriptParams,
        Command::ScriptReload,
        Command::ScriptReloadCancel,
        Command::Fleet(FleetAction::Start),
        Command::Fleet(FleetAction::Stop),
        Command::Fleet(FleetAction::Assign),
        Command::Fleet(FleetAction::AssignRestart),
        Command::Fleet(FleetAction::CopySettings),
        Command::ImportCatalog,
        Command::ToggleGameChat,
        Command::MapSearch,
        Command::MapRecenter,
        Command::MapGroup,
        Command::MapWalk,
        Command::MapTeleport,
        Command::MapWilderness,
        Command::LogSave,
        Command::LogSessionFile,
        Command::ShowMessage,
        Command::ToggleMouse,
        Command::Quit,
    ];

    pub fn group(self) -> Group {
        match self {
            Command::Help
            | Command::Palette
            | Command::Quit
            | Command::ShowFleet
            | Command::Show(_)
            | Command::FocusNext
            | Command::FocusPrev
            | Command::ToggleMouse
            | Command::ShowMessage => Group::App,
            Command::SelectNextBot
            | Command::SelectPrevBot
            | Command::FilterFleet
            | Command::PickFleetCard
            | Command::MarkAllShown
            | Command::UnmarkShown
            | Command::InvertShown
            | Command::MarkWhere(_)
            | Command::ClearMarks
            | Command::MoveProfileUp
            | Command::MoveProfileDown
            | Command::MoveMarkedToTop
            | Command::Fleet(FleetAction::Login | FleetAction::Logout)
            | Command::ImportProfiles => Group::Fleet,
            Command::Login
            | Command::Logout
            | Command::Remove
            | Command::Settings
            | Command::Loadouts
            | Command::ManualWalk
            | Command::DismissNotice => Group::Bot,
            Command::ScriptBrowse
            | Command::ScriptStart
            | Command::ScriptPause
            | Command::ScriptStop
            | Command::ScriptLoad
            | Command::ScriptParams
            | Command::ScriptReload
            | Command::ScriptReloadCancel
            | Command::Fleet(
                FleetAction::Start
                | FleetAction::Stop
                | FleetAction::Assign
                | FleetAction::AssignRestart
                | FleetAction::CopySettings,
            )
            | Command::ImportCatalog => Group::Script,
            Command::ToggleGameChat => Group::Chat,
            Command::MapSearch
            | Command::MapRecenter
            | Command::MapGroup
            | Command::MapWalk
            | Command::MapTeleport
            | Command::MapWilderness => Group::Map,
            Command::LogSave | Command::LogSessionFile => Group::Logs,
        }
    }

    pub fn label(self, app: &TuiApp) -> &'static str {
        match self {
            Command::Help => "Help",
            Command::Palette => "Command palette",
            Command::Quit => "Quit",
            Command::ShowFleet => "Fleet",
            Command::Show(screen) => screen.label(),
            Command::FocusNext => "Next pane",
            Command::FocusPrev => "Previous pane",
            Command::SelectNextBot => "Select next bot",
            Command::SelectPrevBot => "Select previous bot",
            Command::ToggleMouse if app.mouse_capture => "Mouse capture off (terminal copy)",
            Command::ToggleMouse => "Mouse capture on",
            Command::ShowMessage => "Show full message",
            Command::FilterFleet => "Filter fleet",
            Command::PickFleetCard => "Pick fleet card…",
            Command::MarkAllShown => "Mark shown rows",
            Command::UnmarkShown => "Unmark shown rows",
            Command::InvertShown => "Invert shown",
            Command::MarkWhere(predicate) => mark_where_label(predicate),
            Command::ClearMarks => "Clear row selection",
            Command::MoveProfileUp => "Move profile up",
            Command::MoveProfileDown => "Move profile down",
            Command::MoveMarkedToTop => "Move marked to top",
            Command::Fleet(FleetAction::Start) => "Start fleet",
            Command::Fleet(FleetAction::Stop) => "Stop fleet",
            Command::Fleet(FleetAction::Assign) => "Assign",
            Command::Fleet(FleetAction::AssignRestart) => "Assign & restart",
            Command::Fleet(FleetAction::Login) => "Load + log in fleet",
            Command::Fleet(FleetAction::Logout) => "Log out fleet",
            Command::Fleet(FleetAction::CopySettings) => "Copy settings",
            Command::ImportProfiles => "Import rs2b0t profiles…",
            Command::Login => "Log in",
            Command::Logout => "Log out",
            Command::Remove => "Remove from fleet…",
            Command::Settings => "Settings",
            Command::Loadouts => "Loadouts",
            Command::ManualWalk => "Manual walk (WASD)",
            Command::DismissNotice => "Got it (hide notice)",
            Command::ScriptBrowse => "Browse scripts",
            Command::ScriptStart => "Start script",
            Command::ScriptPause if app.script_state == ScriptRunState::Paused => "Resume script",
            Command::ScriptPause => "Pause script",
            Command::ScriptStop => "Stop script",
            Command::ScriptLoad => "Load script file",
            Command::ScriptParams => "Script parameters",
            Command::ScriptReload if app.reload_confirm => "Confirm reload",
            Command::ScriptReload => "Reload script",
            Command::ScriptReloadCancel => "Cancel reload",
            Command::ImportCatalog => "Import rs2b0t catalog",
            Command::ToggleGameChat => "Toggle game chat / script paint",
            Command::MapSearch => "Map: search",
            Command::MapRecenter => "Map: recenter",
            Command::MapGroup => "Map: toggle group send",
            Command::MapWalk => "Map: walk to selection",
            Command::MapTeleport => "Map: teleport (local debug)",
            Command::MapWilderness => "Map: toggle wilderness overlay",
            Command::LogSave => "Log: save",
            Command::LogSessionFile => "Log: session file on/off",
        }
    }

    /// Short button label (the visible `[Label k]` buttons).
    pub fn button(self, app: &TuiApp) -> &'static str {
        match self {
            Command::Fleet(FleetAction::Login) => "Load+login",
            Command::Fleet(FleetAction::Logout) => "Logout",
            Command::Remove => "Remove",
            Command::ManualWalk => "Manual walk",
            Command::DismissNotice => "Got it",
            Command::MapWalk => "Walk",
            Command::MapTeleport => "Teleport",
            Command::MapWilderness => "Wilderness",
            Command::MapGroup => "Group",
            Command::MapSearch => "Search",
            Command::ShowMessage => "Full message",
            other => other.label(app),
        }
    }

    /// The key hint the palette and help show (global keys or the pane key).
    pub fn hint(self) -> Option<&'static str> {
        Some(match self {
            Command::Help => "F1 ?",
            Command::Palette => "^P :",
            Command::Quit => "^Q q",
            Command::ShowFleet => "F2",
            Command::Show(screen) => screen.key(),
            Command::FocusNext => "Tab",
            Command::FocusPrev => "S-Tab",
            Command::MoveProfileUp => "Alt-Up",
            Command::MoveProfileDown => "Alt-Down",
            Command::FilterFleet => "/",
            _ => return None,
        })
    }

    /// Who the command acts on, spelled out for the palette. The counts
    /// are the core fleet model's; the targets themselves are core's plan.
    pub fn scope(self, app: &TuiApp) -> String {
        let bot = || match app.focused_name() {
            Some(name) => format!("BOT {name}"),
            None => "no bot selected".to_string(),
        };
        let fleet_scope = || {
            let counts = app.fleet_model.counts();
            if counts.marked() > 0 {
                format!("{} marked bots", counts.marked())
            } else {
                format!("all {} shown", counts.shown_loaded())
            }
        };
        match self {
            Command::Fleet(FleetAction::CopySettings) => {
                let source = app.focused_name().unwrap_or_else(|| "no bot".into());
                format!("{source} to {}", fleet_scope())
            }
            Command::Fleet(_) => fleet_scope(),
            Command::PickFleetCard => "fleet card".into(),
            Command::MarkAllShown
            | Command::UnmarkShown
            | Command::InvertShown
            | Command::MarkWhere(_) => "shown rows".into(),
            _ => match self.group() {
                Group::App => "app".into(),
                Group::Logs => "log".into(),
                Group::Fleet => match self {
                    Command::ImportProfiles => "vault".into(),
                    _ => "fleet table".into(),
                },
                Group::Script
                    if matches!(self, Command::ScriptBrowse | Command::ScriptLoad)
                        || self == Command::ImportCatalog =>
                {
                    "script library".into()
                }
                Group::Map
                    if matches!(self, Command::MapWalk)
                        && app.walk_send.mode == WalkSendMode::Group =>
                {
                    format!(
                        "{} selected bots",
                        app.walk_send.checked_count(app.fleet_model.marks())
                    )
                }
                Group::Map if !matches!(self, Command::MapWalk | Command::MapTeleport) => {
                    "map".into()
                }
                Group::Bot if self == Command::Loadouts => "loadout store".into(),
                _ => bot(),
            },
        }
    }

    /// `Err(reason)` when the command cannot run now.
    pub fn availability(self, app: &TuiApp) -> Result<(), &'static str> {
        let bot = || {
            app.focused_name()
                .map(|_| ())
                .ok_or("no bot selected (Fleet: Enter or click a row)")
        };
        let map = || {
            app.map_active
                .then_some(())
                .ok_or("open the Map first (F4)")
        };
        let load_order = || {
            (app.fleet_model.order() == frontend_core::FleetOrder::LoadOrder)
                .then_some(())
                .ok_or("choose Load order to reorder")
        };
        let rows_shown = || {
            (!app.fleet_model.shown().is_empty())
                .then_some(())
                .ok_or("no rows shown")
        };
        match self {
            Command::ShowMessage => app.error.as_ref().map(|_| ()).ok_or("no message"),
            Command::MarkAllShown | Command::InvertShown | Command::MarkWhere(_) => rows_shown(),
            Command::UnmarkShown => (app.fleet_model.counts().shown_marked() > 0)
                .then_some(())
                .ok_or("no shown row is marked"),
            Command::ClearMarks => (!app.fleet_model.marks().is_empty())
                .then_some(())
                .ok_or("no rows selected"),
            Command::Fleet(action) => (!action.needs_card() || !app.fleet_model.card().is_own())
                .then_some(())
                .ok_or("pick a card first"),
            Command::MoveProfileUp | Command::MoveProfileDown => {
                load_order()?;
                let (delta, reason) = if self == Command::MoveProfileUp {
                    (-1, "no shown row above the cursor")
                } else {
                    (1, "no shown row below the cursor")
                };
                app.table
                    .reorder_step(&app.fleet_model, delta)
                    .map(|_| ())
                    .ok_or(reason)
            }
            Command::MoveMarkedToTop => {
                load_order()?;
                (!app.fleet_model.marks().is_empty())
                    .then_some(())
                    .ok_or("mark fleet rows first (Space)")
            }
            Command::Login | Command::Logout | Command::Remove => bot(),
            Command::ManualWalk => {
                bot()?;
                app.here
                    .map(|_| ())
                    .ok_or("the bot has no observed position yet")
            }
            Command::DismissNotice => app
                .background_notice
                .as_ref()
                .map(|_| ())
                .ok_or("no background-bots notice"),
            Command::ScriptStart => {
                bot()?;
                app.script_sel
                    .as_ref()
                    .map(|_| ())
                    .ok_or("browse to pick a script first")
            }
            Command::ScriptPause => {
                bot()?;
                matches!(
                    app.script_state,
                    ScriptRunState::Running | ScriptRunState::Paused
                )
                .then_some(())
                .ok_or("no running script")
            }
            Command::ScriptStop => {
                bot()?;
                (app.script_state != ScriptRunState::Idle || app.script_queued)
                    .then_some(())
                    .ok_or("no script running")
            }
            Command::ScriptParams => {
                if app.params_card().is_none() {
                    Err("pick a script first")
                } else if app.params_unavailable.is_some() {
                    Err("the selected script parameters are unavailable")
                } else if app.params_schema.as_ref().is_empty() {
                    Err("the selected script has no parameters")
                } else {
                    Ok(())
                }
            }
            Command::ScriptReload => (app.reload_confirm || app.script_sel.is_some())
                .then_some(())
                .ok_or("pick a script first"),
            Command::ScriptReloadCancel => app
                .reload_confirm
                .then_some(())
                .ok_or("no reload awaiting confirmation"),
            Command::ToggleGameChat => app
                .chat_data
                .script_paint
                .as_ref()
                .map(|_| ())
                .ok_or("no script paint to toggle"),
            Command::MapSearch
            | Command::MapRecenter
            | Command::MapGroup
            | Command::MapWilderness => map(),
            Command::MapWalk | Command::MapTeleport => {
                map()?;
                app.map_model
                    .pending()
                    .map(|_| ())
                    .ok_or("select a tile first (Enter or click)")
            }
            _ => Ok(()),
        }
    }
}

/// The palette label of a status mark: `Mark by status: <label>`. The
/// label text is core's ([`MarkPredicate::label`]); the pair is spelled
/// out because a command label is a `&'static str`.
fn mark_where_label(predicate: MarkPredicate) -> &'static str {
    match predicate {
        MarkPredicate::Ready => "Mark by status: ready",
        MarkPredicate::Idle => "Mark by status: idle",
        MarkPredicate::Running => "Mark by status: running",
        MarkPredicate::LoggedOut => "Mark by status: logged out",
        MarkPredicate::LoginError => "Mark by status: login error",
        MarkPredicate::NotLoaded => "Mark by status: not loaded",
    }
}
