//! Operator commands: one vocabulary for pane keys, visible buttons, the
//! command palette, context menus and the help overlay. A command knows its
//! label, its target scope and whether it is available (with the reason
//! when not); `TuiApp::run_command` executes it. Keys are context
//! shortcuts: each table below is routed only while its pane has keyboard
//! focus, so no letter leaks into another pane or a text field.

use frontend_core::scripts::ScriptRunState;
use frontend_core::{FleetAction, MarkOp, MarkPredicate, OperatorCommand};

use crate::app::{ErrorNeed, Refusal, TuiApp, WalkSendMode};
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
    /// Show the next fleet order (`SetFleetSort`): Load order, Name, Status,
    /// then Load order again.
    CycleFleetSort,
    /// Choose the world the fleet row under the cursor moves to (opens the
    /// world picker over that profile).
    PickWorld,
    /// Choose the world the marked rows move to (opens the same picker, no
    /// profile staged; with nothing marked, the loaded shown rows).
    PickWorldMarked,
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
    #[cfg(feature = "js")]
    ImportProfiles,
    /// Add a profile to the vault with its own password (opens the
    /// shared New profile form).
    NewProfile,
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
    #[cfg(any(test, feature = "test-support"))]
    ScriptSkipQuest,
    #[cfg(feature = "js")]
    ScriptLoad,
    ScriptParams,
    #[cfg(feature = "js")]
    ScriptReload,
    #[cfg(feature = "js")]
    ScriptReloadCancel,
    #[cfg(feature = "js")]
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

/// Fleet pane shortcuts: the card picker, the sort, the shown-row marks and
/// the fleet actions that live on the pane.
pub const FLEET_KEYS: &[(char, Command)] = &[
    ('c', Command::PickFleetCard),
    ('s', Command::CycleFleetSort),
    ('a', Command::MarkAllShown),
    ('A', Command::UnmarkShown),
    ('m', Command::Fleet(FleetAction::Login)),
    ('U', Command::Fleet(FleetAction::Logout)),
    ('W', Command::PickWorld),
    ('N', Command::NewProfile),
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
        #[cfg(any(test, feature = "test-support"))]
        "Skip quest" => Command::ScriptSkipQuest,
        #[cfg(feature = "js")]
        "Load" => Command::ScriptLoad,
        "Params" => Command::ScriptParams,
        #[cfg(feature = "js")]
        "Reload" | "Confirm" => Command::ScriptReload,
        #[cfg(feature = "js")]
        "Cancel" => Command::ScriptReloadCancel,
        "Start fleet" => Command::Fleet(FleetAction::Start),
        "Stop fleet" => Command::Fleet(FleetAction::Stop),
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
        Command::CycleFleetSort,
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
        Command::PickWorld,
        Command::PickWorldMarked,
        #[cfg(feature = "js")]
        Command::ImportProfiles,
        Command::Login,
        Command::Logout,
        Command::NewProfile,
        Command::Remove,
        Command::Settings,
        Command::Loadouts,
        Command::ManualWalk,
        Command::DismissNotice,
        Command::ScriptBrowse,
        Command::ScriptStart,
        Command::ScriptPause,
        Command::ScriptStop,
        #[cfg(any(test, feature = "test-support"))]
        Command::ScriptSkipQuest,
        #[cfg(feature = "js")]
        Command::ScriptLoad,
        Command::ScriptParams,
        #[cfg(feature = "js")]
        Command::ScriptReload,
        #[cfg(feature = "js")]
        Command::ScriptReloadCancel,
        Command::Fleet(FleetAction::Start),
        Command::Fleet(FleetAction::Stop),
        Command::Fleet(FleetAction::Assign),
        Command::Fleet(FleetAction::AssignRestart),
        Command::Fleet(FleetAction::CopySettings),
        #[cfg(feature = "js")]
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
            | Command::CycleFleetSort
            | Command::PickWorld
            | Command::PickWorldMarked
            | Command::MarkAllShown
            | Command::UnmarkShown
            | Command::InvertShown
            | Command::MarkWhere(_)
            | Command::ClearMarks
            | Command::MoveProfileUp
            | Command::MoveProfileDown
            | Command::MoveMarkedToTop
            | Command::Fleet(
                FleetAction::Login | FleetAction::Logout | FleetAction::SwitchWorld(_),
            )
            | Command::NewProfile => Group::Fleet,
            #[cfg(feature = "js")]
            Command::ImportProfiles => Group::Fleet,
            Command::Login
            | Command::Logout
            | Command::Remove
            | Command::Settings
            | Command::Loadouts
            | Command::ManualWalk
            | Command::DismissNotice => Group::Bot,
            #[cfg(any(test, feature = "test-support"))]
            Command::ScriptSkipQuest => Group::Script,
            Command::ScriptBrowse
            | Command::ScriptStart
            | Command::ScriptPause
            | Command::ScriptStop
            | Command::ScriptParams
            | Command::Fleet(
                FleetAction::Start
                | FleetAction::Stop
                | FleetAction::Assign
                | FleetAction::AssignRestart
                | FleetAction::CopySettings,
            ) => Group::Script,
            #[cfg(feature = "js")]
            Command::ScriptLoad
            | Command::ScriptReload
            | Command::ScriptReloadCancel
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
            Command::CycleFleetSort => "Cycle fleet sort",
            Command::PickWorld => "Switch world of cursor row…",
            Command::PickWorldMarked => "Switch world of marked…",
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
            Command::Fleet(FleetAction::SwitchWorld(_)) => "Switch world",
            #[cfg(feature = "js")]
            Command::ImportProfiles => "Import rs2b0t profiles…",
            Command::Login => "Log in",
            Command::Logout => "Log out",
            Command::NewProfile => "New profile",
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
            #[cfg(any(test, feature = "test-support"))]
            Command::ScriptSkipQuest => "Skip quest",
            #[cfg(feature = "js")]
            Command::ScriptLoad => "Load script file",
            Command::ScriptParams => "Script parameters",
            #[cfg(feature = "js")]
            Command::ScriptReload if app.reload_confirm => "Confirm reload",
            #[cfg(feature = "js")]
            Command::ScriptReload => "Reload script",
            #[cfg(feature = "js")]
            Command::ScriptReloadCancel => "Cancel reload",
            #[cfg(feature = "js")]
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
            Command::Fleet(FleetAction::Logout) => "Log out",
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
            Command::CycleFleetSort => "fleet order".into(),
            Command::PickWorld => match app.cursor_profile() {
                Some(name) => format!("BOT {name}"),
                None => "no row under the cursor".to_string(),
            },
            Command::PickWorldMarked => fleet_scope(),
            Command::MarkAllShown
            | Command::UnmarkShown
            | Command::InvertShown
            | Command::MarkWhere(_) => "shown rows".into(),
            Command::NewProfile => "vault".into(),
            #[cfg(feature = "js")]
            Command::ImportProfiles => "vault".into(),
            Command::ScriptBrowse => "script library".into(),
            #[cfg(feature = "js")]
            Command::ScriptLoad | Command::ImportCatalog => "script library".into(),
            _ => match self.group() {
                Group::App => "app".into(),
                Group::Logs => "log".into(),
                Group::Fleet => "fleet table".into(),
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
                Group::Script | Group::Bot | Group::Chat | Group::Map => bot(),
            },
        }
    }

    /// `Err(reason)` when the command cannot run now: the first missing
    /// precondition, in the order the command checks them.
    pub fn availability(self, app: &TuiApp) -> Result<(), Unavailable> {
        use Unavailable::*;
        let preconditions: &[Unavailable] = match self {
            Command::ShowMessage => &[NoMessage],
            Command::MarkAllShown | Command::InvertShown | Command::MarkWhere(_) => &[NoRowsShown],
            Command::UnmarkShown => &[NoShownRowMarked],
            Command::ClearMarks => &[NoRowsMarked],
            Command::PickWorld => &[NoCursorRow],
            Command::PickWorldMarked => &[NoRowsShownOrMarked],
            Command::Fleet(action) if action.needs_card() => &[NoFleetCard],
            Command::MoveProfileUp => &[NotLoadOrder, NoRowAbove],
            Command::MoveProfileDown => &[NotLoadOrder, NoRowBelow],
            Command::MoveMarkedToTop => &[NotLoadOrder, NoRowsMarkedToMove],
            Command::Logout => &[NoFocusedBot, NotLoggedIn],
            Command::Login | Command::Remove => &[NoFocusedBot],
            Command::ManualWalk => &[NoFocusedBot, NoPosition],
            Command::DismissNotice => &[NoNotice],
            Command::ScriptStart => &[NoFocusedBot, NoScriptPicked],
            Command::ScriptPause => &[NoFocusedBot, NoScriptToPause],
            #[cfg(any(test, feature = "test-support"))]
            Command::ScriptSkipQuest => &[NoFocusedBot, NoQuesterQueue],
            Command::ScriptStop => &[NoFocusedBot, NoScriptToStop],
            Command::ScriptParams => &[NoParamsCard, ParamsUnavailable, NoParams],
            #[cfg(feature = "js")]
            Command::ScriptReload => &[NoReloadCard],
            #[cfg(feature = "js")]
            Command::ScriptReloadCancel => &[NoReloadWaiting],
            Command::ToggleGameChat => &[NoScriptPaint],
            Command::MapSearch
            | Command::MapRecenter
            | Command::MapGroup
            | Command::MapWilderness => &[MapClosed],
            Command::MapWalk | Command::MapTeleport => &[MapClosed, NoTile],
            _ => &[],
        };
        match preconditions.iter().find(|reason| reason.missing(app)) {
            Some(&reason) => Err(reason),
            None => Ok(()),
        }
    }

    /// The operator command `self` runs, for the refusal line the command
    /// boundary writes when `self` is unavailable ([`Self::availability`]).
    /// `None` for a command that only changes what the screen shows (a
    /// pane, a picker, a popup, a map view toggle): refusing it refuses no
    /// operator command.
    fn operator_command(self, app: &TuiApp) -> Option<OperatorCommand> {
        Some(match self {
            Command::Fleet(action) => OperatorCommand::PlanFleetAction(action),
            Command::MarkAllShown => OperatorCommand::MarkFleet(MarkOp::Shown),
            Command::UnmarkShown => OperatorCommand::MarkFleet(MarkOp::UnmarkShown),
            Command::InvertShown => OperatorCommand::MarkFleet(MarkOp::InvertShown),
            Command::MarkWhere(predicate) => OperatorCommand::MarkFleet(MarkOp::Where(predicate)),
            Command::ClearMarks => OperatorCommand::ClearMarks,
            Command::MoveProfileUp | Command::MoveProfileDown | Command::MoveMarkedToTop => {
                OperatorCommand::ReorderProfiles
            }
            Command::CycleFleetSort => {
                OperatorCommand::SetFleetSort(crate::fleet::next_order(app.fleet_model.order()))
            }
            Command::Login => OperatorCommand::Login,
            Command::Logout => OperatorCommand::Logout,
            Command::Remove => OperatorCommand::Remove,
            Command::DismissNotice => OperatorCommand::AcknowledgeNotice,
            Command::ScriptStart => OperatorCommand::StartScript,
            Command::ScriptPause if app.script_state == ScriptRunState::Paused => {
                OperatorCommand::ResumeScript
            }
            Command::ScriptPause => OperatorCommand::PauseScript,
            Command::ScriptStop => OperatorCommand::StopScript,
            #[cfg(any(test, feature = "test-support"))]
            Command::ScriptSkipQuest => OperatorCommand::SkipQuest {
                target: app
                    .focused_detail()
                    .and_then(|detail| detail.native_status())
                    .map(|native| native.run()),
            },
            #[cfg(feature = "js")]
            Command::ScriptReload if app.reload_confirm => OperatorCommand::ConfirmReload,
            #[cfg(feature = "js")]
            Command::ScriptReload => OperatorCommand::ReloadScripts,
            #[cfg(feature = "js")]
            Command::ScriptReloadCancel => OperatorCommand::CancelReload,
            Command::MapWalk if app.walk_send.mode == WalkSendMode::Group => {
                OperatorCommand::GroupWalk
            }
            Command::MapWalk => OperatorCommand::WalkTo,
            Command::MapTeleport => OperatorCommand::Teleport,
            Command::LogSave => OperatorCommand::SaveLog,
            Command::Help
            | Command::Palette
            | Command::Quit
            | Command::ShowFleet
            | Command::Show(_)
            | Command::FocusNext
            | Command::FocusPrev
            | Command::SelectNextBot
            | Command::SelectPrevBot
            | Command::ToggleMouse
            | Command::ShowMessage
            | Command::FilterFleet
            | Command::PickFleetCard
            | Command::PickWorld
            | Command::PickWorldMarked
            | Command::NewProfile
            | Command::Settings
            | Command::Loadouts
            | Command::ManualWalk
            | Command::ScriptBrowse
            | Command::ScriptParams
            | Command::ToggleGameChat
            | Command::MapSearch
            | Command::MapRecenter
            | Command::MapGroup
            | Command::MapWilderness
            | Command::LogSessionFile => return None,
            #[cfg(feature = "js")]
            Command::ImportProfiles | Command::ScriptLoad | Command::ImportCatalog => return None,
        })
    }

    /// The bot `self` acts on, named on its refusal line: the focused bot
    /// for a command on the selected bot, none for a fleet or app command.
    fn refused_bot(self, app: &TuiApp) -> Option<String> {
        match self {
            #[cfg(any(test, feature = "test-support"))]
            Command::ScriptSkipQuest => app.focused_name(),
            Command::Login
            | Command::Logout
            | Command::Remove
            | Command::DismissNotice
            | Command::ScriptStart
            | Command::ScriptPause
            | Command::ScriptStop
            | Command::MapTeleport => app.focused_name(),
            #[cfg(feature = "js")]
            Command::ScriptReload | Command::ScriptReloadCancel => app.focused_name(),
            Command::MapWalk if app.walk_send.mode != WalkSendMode::Group => app.focused_name(),
            _ => None,
        }
    }

    /// The refusal of `self` for `reason` ([`Self::availability`]). The
    /// message line shows `"{label}: {reason}"`, and the message waits for
    /// `reason`'s precondition ([`Unavailable::need`]), so every key,
    /// palette entry and menu item that refuses gets the same resolution.
    /// When `self` runs an operator command, the session also logs the
    /// refusal at the command boundary; a command that only changes the
    /// screen logs nothing.
    pub fn refusal(self, app: &TuiApp, reason: Unavailable) -> Refusal {
        Refusal {
            command: self.operator_command(app),
            bot: self.refused_bot(app),
            reason: format!("{}: {}", self.label(app), reason.text()).into(),
            need: reason.need(),
        }
    }
}

/// Why a command cannot run now ([`Command::availability`]): one missing
/// precondition each. The same value says what its refusal waits for
/// ([`Self::need`]) and checks whether that still holds ([`Self::missing`]),
/// so a refusal clears when, and only when, the gate that made it would
/// no longer give it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    NoMessage,
    NoRowsShown,
    NoShownRowMarked,
    NoRowsMarked,
    NoCursorRow,
    NoRowsShownOrMarked,
    NoFleetCard,
    NotLoadOrder,
    NoRowAbove,
    NoRowBelow,
    NoRowsMarkedToMove,
    NoFocusedBot,
    /// Log out has nothing to cancel on the focused bot
    /// ([`frontend_core::LogoutEligibility`]).
    NotLoggedIn,
    NoPosition,
    NoNotice,
    NoScriptPicked,
    NoScriptToPause,
    NoScriptToStop,
    NoQuesterQueue,
    NoParamsCard,
    ParamsUnavailable,
    NoParams,
    #[cfg(feature = "js")]
    NoReloadCard,
    #[cfg(feature = "js")]
    NoReloadWaiting,
    NoScriptPaint,
    MapClosed,
    NoTile,
    /// The map's group walk has no checked bot (`TuiApp::map_enter`).
    NoCheckedBots,
}

impl Unavailable {
    /// The reason as the message line and the palette show it.
    pub fn text(self) -> &'static str {
        match self {
            Unavailable::NoMessage => "no message",
            Unavailable::NoRowsShown => "no rows shown",
            Unavailable::NoShownRowMarked => "no shown row is marked",
            Unavailable::NoRowsMarked => "no rows selected",
            Unavailable::NoCursorRow => "no fleet row under the cursor",
            Unavailable::NoRowsShownOrMarked => "no rows shown or marked",
            Unavailable::NoFleetCard => "pick a card first",
            Unavailable::NotLoadOrder => "choose Load order to reorder",
            Unavailable::NoRowAbove => "no shown row above the cursor",
            Unavailable::NoRowBelow => "no shown row below the cursor",
            Unavailable::NoRowsMarkedToMove => "mark fleet rows first (Space)",
            Unavailable::NoFocusedBot => "no bot selected (Fleet: Enter or click a row)",
            Unavailable::NotLoggedIn => frontend_core::LogoutEligibility::REFUSAL,
            Unavailable::NoPosition => "the bot has no observed position yet",
            Unavailable::NoNotice => "no background-bots notice",
            Unavailable::NoScriptPicked => "browse to pick a script first",
            Unavailable::NoScriptToPause => "no running script",
            Unavailable::NoScriptToStop => "no script running",
            Unavailable::NoQuesterQueue => "no active Quester queue",
            Unavailable::NoParamsCard => "pick a script first",
            #[cfg(feature = "js")]
            Unavailable::NoReloadCard => "pick a script first",
            Unavailable::ParamsUnavailable => "the selected script parameters are unavailable",
            Unavailable::NoParams => "the selected script has no parameters",
            #[cfg(feature = "js")]
            Unavailable::NoReloadWaiting => "no reload awaiting confirmation",
            Unavailable::NoScriptPaint => "no script paint to toggle",
            Unavailable::MapClosed => "open the Map first (F4)",
            Unavailable::NoTile => "select a tile first (Enter or click)",
            Unavailable::NoCheckedBots => "no bots selected",
        }
    }

    /// Whether the precondition is still missing in `app`.
    pub fn missing(self, app: &TuiApp) -> bool {
        match self {
            Unavailable::NoMessage => app.error.is_none(),
            Unavailable::NoRowsShown => app.fleet_model.shown().is_empty(),
            Unavailable::NoShownRowMarked => app.fleet_model.counts().shown_marked() == 0,
            Unavailable::NoRowsMarked | Unavailable::NoRowsMarkedToMove => {
                app.fleet_model.marks().is_empty()
            }
            Unavailable::NoCursorRow => app.cursor_profile().is_none(),
            Unavailable::NoRowsShownOrMarked => {
                app.fleet_model.marks().is_empty() && app.fleet_model.shown().is_empty()
            }
            Unavailable::NoFleetCard => app.fleet_model.card().is_own(),
            Unavailable::NotLoadOrder => {
                app.fleet_model.order() != frontend_core::FleetOrder::LoadOrder
            }
            Unavailable::NoRowAbove => app.table.reorder_step(&app.fleet_model, -1).is_none(),
            Unavailable::NoRowBelow => app.table.reorder_step(&app.fleet_model, 1).is_none(),
            Unavailable::NoFocusedBot => !app.has_focus(),
            Unavailable::NotLoggedIn => !app.focused_logout_eligible(),
            Unavailable::NoPosition => app.here.is_none(),
            Unavailable::NoNotice => app.background_notice.is_none(),
            Unavailable::NoScriptPicked | Unavailable::NoParamsCard => app.script_sel.is_none(),
            Unavailable::NoScriptToPause => !matches!(
                app.script_state,
                ScriptRunState::Running | ScriptRunState::Paused
            ),
            Unavailable::NoQuesterQueue => {
                !matches!(
                    app.script_state,
                    ScriptRunState::Running | ScriptRunState::Paused
                ) || app
                    .focused_detail()
                    .and_then(|detail| detail.native_status())
                    .is_none_or(|status| {
                        status.card() != frontend_core::scripts::CompiledCardId::QUESTER
                    })
            }
            Unavailable::NoScriptToStop => {
                app.script_state == ScriptRunState::Idle && !app.script_queued
            }
            Unavailable::ParamsUnavailable => app.params_unavailable.is_some(),
            Unavailable::NoParams => app.params_schema.as_ref().is_empty(),
            #[cfg(feature = "js")]
            Unavailable::NoReloadCard => !app.reload_confirm && app.script_sel.is_none(),
            #[cfg(feature = "js")]
            Unavailable::NoReloadWaiting => !app.reload_confirm,
            Unavailable::NoScriptPaint => app.chat_data.script_paint.is_none(),
            Unavailable::MapClosed => !app.map_active,
            Unavailable::NoTile => app.map_model.pending().is_none(),
            Unavailable::NoCheckedBots => app.walk_send.checked_count(app.fleet_model.marks()) == 0,
        }
    }

    /// What a refusal for this reason waits for: the precondition itself,
    /// so the action that supplies it clears the message. "No message" is
    /// the one exception: its own refusal is a message, so it would clear
    /// itself at once; a newer message or the timeout replaces it.
    pub fn need(self) -> Option<ErrorNeed> {
        match self {
            Unavailable::NoMessage => None,
            reason => Some(ErrorNeed::Precondition(reason)),
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
