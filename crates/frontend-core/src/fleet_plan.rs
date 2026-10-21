//! Fleet decisions: a fleet action resolved to a frozen plan before it
//! runs, and the one result view the action then settles into.
//!
//! [`crate::OperatorCommand::PlanFleetAction`] freezes the target
//! identities (every marked profile, hidden ones included; with no marks,
//! the loaded rows the filter shows), the resolved fleet card, the bots it
//! would interrupt, the rows it skips and why, and the prompt text. A plan
//! that needs no confirmation runs at once; otherwise the front end shows
//! [`FleetPlan::prompt`] and either submits [`FleetPlan::command`] with
//! [`crate::Targets::Plan`] (Confirm) or
//! [`crate::OperatorCommand::CancelFleetPlan`]. A run never re-reads the
//! marks or the filter: a profile marked, loaded or made ready after the
//! plan does not join it, and a frozen profile that changed is skipped
//! with its new reason.

use crate::bulk::BulkRow;
use crate::command::CardKind;
use crate::command::OperatorCommand;
use crate::scripts::ScriptChoice;
use crate::selection::{MarkedSelection, ProfileIdentity};

/// A fleet button's action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FleetAction {
    /// Start the fleet card (each bot's own assignment for
    /// [`crate::FleetCard::Own`]), paced like Start all.
    Start,
    /// Stop the running scripts (a queued Start is cancelled).
    Stop,
    /// Save the fleet card as each bot's assignment without starting it.
    Assign,
    /// Save the fleet card as each bot's assignment and start it, stopping
    /// a running script first.
    AssignRestart,
    /// Log in, loading a profile that is not loaded first.
    Login,
    /// Log out (a running script stops with the logout).
    Logout,
    /// Copy the focused bot's settings for the fleet card to the targets.
    CopySettings,
}

impl FleetAction {
    /// Every action, in toolbar order.
    pub const ALL: [FleetAction; 7] = [
        Self::Start,
        Self::Stop,
        Self::Assign,
        Self::AssignRestart,
        Self::Login,
        Self::Logout,
        Self::CopySettings,
    ];

    /// The button verb.
    pub const fn verb(self) -> &'static str {
        match self {
            Self::Start => "Start",
            Self::Stop => "Stop",
            Self::Assign => "Assign",
            Self::AssignRestart => "Assign & restart",
            Self::Login => "Log in",
            Self::Logout => "Log out",
            Self::CopySettings => "Copy settings to",
        }
    }

    /// Whether the action needs a card (the fleet card must resolve to
    /// one: [`crate::FleetCard::Own`] cannot).
    pub const fn needs_card(self) -> bool {
        matches!(
            self,
            Self::Assign | Self::AssignRestart | Self::CopySettings
        )
    }
}

/// Which rows a plan acts on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanScope {
    /// Every marked profile; `hidden` of them the filter does not show.
    Marked { hidden: usize },
    /// No marks: the loaded rows the filter shows.
    AllShown,
}

/// A fleet action frozen before it runs.
#[derive(Debug)]
pub struct FleetPlan {
    pub(crate) action: FleetAction,
    pub(crate) scope: PlanScope,
    /// The admitted targets, frozen: identity and the name at plan time.
    pub(crate) targets: Vec<(ProfileIdentity, String)>,
    /// The same identities, as the run's mark set.
    pub(crate) marks: MarkedSelection,
    /// The resolved card (`None`: each bot's own assignment).
    pub(crate) card: Option<ScriptChoice>,
    /// The copy's card kind ([`FleetAction::CopySettings`]).
    pub(crate) copy_kind: Option<CardKind>,
    /// Targets a run would interrupt (their running script stops).
    pub(crate) interrupted: Vec<String>,
    /// Rows the plan skips, with the reason.
    pub(crate) skipped: Vec<BulkRow>,
    pub(crate) prompt: String,
    pub(crate) needs_confirm: bool,
    /// The catalog root staged with the click, for the Start or
    /// assignment that runs the plan.
    pub(crate) catalog_root: Option<std::path::PathBuf>,
}

impl FleetPlan {
    /// The action.
    pub fn action(&self) -> FleetAction {
        self.action
    }

    /// The scope.
    pub fn scope(&self) -> PlanScope {
        self.scope
    }

    /// The command that runs the plan (with [`crate::Targets::Plan`]).
    pub fn command(&self) -> OperatorCommand {
        match self.action {
            FleetAction::Start => OperatorCommand::StartScript,
            FleetAction::Stop => OperatorCommand::StopScript,
            FleetAction::Assign => OperatorCommand::AssignScript,
            FleetAction::AssignRestart => OperatorCommand::AssignAndRestart,
            FleetAction::Login => OperatorCommand::Login,
            FleetAction::Logout => OperatorCommand::Logout,
            FleetAction::CopySettings => {
                OperatorCommand::ApplySettingsToMarked(self.copy_kind.unwrap_or(CardKind::Native))
            }
        }
    }

    /// The frozen target names (as they were at plan time).
    pub fn target_names(&self) -> impl Iterator<Item = &str> + '_ {
        self.targets.iter().map(|(_, name)| name.as_str())
    }

    /// How many targets the plan admitted.
    pub fn target_count(&self) -> usize {
        self.targets.len()
    }

    /// The targets a run would interrupt.
    pub fn interrupted(&self) -> &[String] {
        &self.interrupted
    }

    /// The rows the plan skips.
    pub fn skipped(&self) -> &[BulkRow] {
        &self.skipped
    }

    /// The card label, or `None` for each bot's own card.
    pub fn card_label(&self) -> Option<&str> {
        self.card.as_ref().map(ScriptChoice::label)
    }

    /// The confirmation text (one wording for every front end).
    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    /// Whether the plan asks first: the scope is every shown bot, a target
    /// would be interrupted, or the action copies settings.
    pub fn needs_confirm(&self) -> bool {
        self.needs_confirm
    }
}

/// The one result of the latest fleet action: its command's members (and
/// the rows the plan skipped), failures first, then skips, then done.
#[derive(Debug)]
pub struct FleetResult {
    pub(crate) request: api::identity::RequestId,
    pub(crate) action: FleetAction,
    /// Plan-time skips and profiles gone at run time.
    pub(crate) extra: Vec<BulkRow>,
    /// The report text, rebuilt when a member settles.
    pub(crate) text: String,
    pub(crate) settled: bool,
    pub(crate) logged: bool,
    pub(crate) counts: ResultCounts,
}

/// The result's row counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ResultCounts {
    pub done: usize,
    pub skipped: usize,
    pub failed: usize,
    pub pending: usize,
}

impl FleetResult {
    /// The action.
    pub fn action(&self) -> FleetAction {
        self.action
    }

    /// The command record the result follows.
    pub fn request(&self) -> api::identity::RequestId {
        self.request
    }

    /// The report line: failures first, then skips, then done, with
    /// counts (and how many are still pending).
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Every member settled.
    pub fn settled(&self) -> bool {
        self.settled
    }

    /// The counts.
    pub fn counts(&self) -> ResultCounts {
        self.counts
    }
}
