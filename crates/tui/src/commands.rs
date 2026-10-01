//! Operator commands: one vocabulary for pane keys, visible buttons, the
//! command palette, context menus and the help overlay. A command knows its
//! label, its target scope and whether it is available (with the reason
//! when not); `TuiApp::run_command` executes it. Keys are context
//! shortcuts: each table below is routed only while its pane has keyboard
//! focus, so no letter leaks into another pane or a text field.

use script::RunState;

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
    MarkAllShown,
    ClearMarks,
    LoadLoginAll,
    LogoutAll,
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
    ScriptStartAll,
    ScriptStopAll,
    ScriptAssignMarked,
    ScriptRestartMarked,
    ScriptApplyMarked,
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

/// `1 member` / `N members`.
pub fn members_text(n: usize) -> String {
    if n == 1 {
        "1 member".into()
    } else {
        format!("{n} members")
    }
}

/// Fleet pane shortcuts (fleet-wide commands; both confirm their scope).
pub const FLEET_KEYS: &[(char, Command)] =
    &[('m', Command::LoadLoginAll), ('U', Command::LogoutAll)];

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
        "Start all" => Command::ScriptStartAll,
        "Stop all" => Command::ScriptStopAll,
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
        Command::MarkAllShown,
        Command::ClearMarks,
        Command::LoadLoginAll,
        Command::LogoutAll,
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
        Command::ScriptStartAll,
        Command::ScriptStopAll,
        Command::ScriptAssignMarked,
        Command::ScriptRestartMarked,
        Command::ScriptApplyMarked,
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
            | Command::MarkAllShown
            | Command::ClearMarks
            | Command::LoadLoginAll
            | Command::LogoutAll => Group::Fleet,
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
            | Command::ScriptStartAll
            | Command::ScriptStopAll
            | Command::ScriptAssignMarked
            | Command::ScriptRestartMarked
            | Command::ScriptApplyMarked
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
            Command::MarkAllShown => "Select all shown rows",
            Command::ClearMarks => "Clear row selection",
            Command::LoadLoginAll => "Load + log in all…",
            Command::LogoutAll => "Log out all…",
            Command::Login => "Log in",
            Command::Logout => "Log out",
            Command::Remove => "Remove from fleet…",
            Command::Settings => "Settings",
            Command::Loadouts => "Loadouts",
            Command::ManualWalk => "Manual walk (WASD)",
            Command::DismissNotice => "Got it (hide notice)",
            Command::ScriptBrowse => "Browse scripts",
            Command::ScriptStart => "Start script",
            Command::ScriptPause if app.script_state == RunState::Paused => "Resume script",
            Command::ScriptPause => "Pause script",
            Command::ScriptStop => "Stop script",
            Command::ScriptLoad => "Load script file",
            Command::ScriptParams => "Script parameters",
            Command::ScriptReload if app.reload_confirm => "Confirm reload",
            Command::ScriptReload => "Reload script",
            Command::ScriptReloadCancel => "Cancel reload",
            Command::ScriptStartAll => "Start all…",
            Command::ScriptStopAll => "Stop all…",
            Command::ScriptAssignMarked => "Assign script to marked…",
            Command::ScriptRestartMarked => "Assign & restart marked…",
            Command::ScriptApplyMarked => "Apply focused bot's settings to marked…",
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
            Command::LoadLoginAll => "Load+login all",
            Command::LogoutAll => "Logout all",
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
            Command::FilterFleet => "/",
            _ => return None,
        })
    }

    /// Fleet-wide commands: they confirm a frozen scope before running.
    pub fn is_bulk(self) -> bool {
        matches!(
            self,
            Command::LoadLoginAll
                | Command::LogoutAll
                | Command::ScriptStartAll
                | Command::ScriptStopAll
                | Command::ScriptAssignMarked
                | Command::ScriptRestartMarked
        )
    }

    /// Who the command acts on, spelled out for the palette.
    pub fn scope(self, app: &TuiApp) -> String {
        let bot = || match app.focused_name() {
            Some(name) => format!("BOT {name}"),
            None => "no bot selected".to_string(),
        };
        let selected_scope = || {
            if app.table.selection.is_empty() {
                format!("all {}", members_text(app.names.len()))
            } else {
                format!("{} marked bots", app.table.selection.len())
            }
        };
        match self.group() {
            Group::App => "app".into(),
            Group::Logs => "log".into(),
            Group::Fleet => match self {
                Command::LoadLoginAll if !app.table.selection.is_empty() => selected_scope(),
                Command::LoadLoginAll => "every vault profile".into(),
                Command::LogoutAll if !app.table.selection.is_empty() => selected_scope(),
                Command::LogoutAll => format!("all {}", members_text(app.names.len())),
                _ => "fleet table".into(),
            },
            Group::Script
                if matches!(
                    self,
                    Command::ScriptStartAll
                        | Command::ScriptStopAll
                        | Command::ScriptAssignMarked
                        | Command::ScriptRestartMarked
                ) =>
            {
                selected_scope()
            }
            Group::Script if self == Command::ScriptApplyMarked => {
                let source = app.focused_name().unwrap_or_else(|| "no bot".into());
                format!("{source} to {} marked", app.table.selection.len())
            }
            Group::Script if matches!(self, Command::ScriptBrowse | Command::ScriptLoad) => {
                "script library".into()
            }
            Group::Script if self == Command::ImportCatalog => "script library".into(),
            Group::Map
                if matches!(self, Command::MapWalk)
                    && app.walk_send.mode == WalkSendMode::Group =>
            {
                format!("{} selected bots", app.walk_send.checked_count())
            }
            Group::Map if !matches!(self, Command::MapWalk | Command::MapTeleport) => "map".into(),
            Group::Bot if self == Command::Loadouts => "loadout store".into(),
            _ => bot(),
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
        let members = || {
            (!app.names.is_empty())
                .then_some(())
                .ok_or("no bots loaded")
        };
        match self {
            Command::ShowMessage => app.error.as_ref().map(|_| ()).ok_or("no message"),
            Command::MarkAllShown => members(),
            Command::ClearMarks => (!app.table.selection.is_empty())
                .then_some(())
                .ok_or("no rows selected"),
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
                matches!(app.script_state, RunState::Running | RunState::Paused)
                    .then_some(())
                    .ok_or("no running script")
            }
            Command::ScriptStop => {
                bot()?;
                (app.script_state != RunState::Idle || app.script_queued)
                    .then_some(())
                    .ok_or("no script running")
            }
            Command::ScriptParams => {
                if app.params_card().is_none() {
                    Err("pick a script first")
                } else if app.params_unavailable.is_some() {
                    Err("the selected script parameters are unavailable")
                } else if app.params_schema.is_empty() {
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
            Command::ScriptStartAll | Command::ScriptStopAll => members(),
            Command::ScriptAssignMarked | Command::ScriptRestartMarked => {
                if app.table.selection.is_empty() {
                    Err("mark fleet rows first")
                } else {
                    app.script_sel
                        .as_ref()
                        .map(|_| ())
                        .ok_or("browse to pick a script first")
                }
            }
            Command::ScriptApplyMarked => {
                if app.table.selection.is_empty() {
                    Err("mark fleet rows first")
                } else if app.focused_name().is_none() {
                    Err("no bot selected (Fleet: Enter or click a row)")
                } else {
                    app.script_sel
                        .as_ref()
                        .map(|_| ())
                        .ok_or("browse to pick a script first")
                }
            }
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
