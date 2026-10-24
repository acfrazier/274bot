//! One bot's application projection: the separate facts a front end shows
//! and the command decisions taken from the same facts. Panel and TUI render
//! its accessors; the Fleet row label, the status filters and Stop
//! eligibility are all derived from the row facts below, so no surface and
//! no command can disagree with another about what a bot is doing.
//!
//! This is a read-only view over the projected [`FleetRow`] and the Scripts
//! model, not a new owner of runtime state. The facts stay separate:
//!
//! - script lifecycle ([`BotProjection::script`]);
//! - script-walk activity, tied to the run ([`BotProjection::script_walk`]);
//! - manual WalkTo activity ([`BotProjection::manual_walk`]);
//! - client login and connection ([`BotProjection::phase`],
//!   [`BotProjection::connected`]);
//! - the displayed script choice ([`BotProjection::displayed_script`]) and
//!   command eligibility ([`BotProjection::stop_eligibility`]).
//!
//! Marks are a separate set of profile identities and never enter this view.

use crate::scripts::{ScriptRunState as RunState, Scripts};
use crate::session::OperatorSession;
use crate::views::{FleetRow, Phase};

/// Whether Fleet Stop acts on a bot, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopEligibility {
    /// A live script run: Stop interrupts it.
    Running,
    /// No run yet, but a Start still waits to run: Stop withdraws it.
    StartQueued,
    /// Nothing to stop. A walk alone is not a script.
    NoScript,
}

impl StopEligibility {
    /// The refusal text of a bot Stop cannot act on.
    pub const REFUSAL: &'static str = "no script running";

    /// Whether Stop acts on the bot at all.
    pub fn eligible(self) -> bool {
        self != Self::NoScript
    }
}

/// One bot's facts and decisions as of the last projection.
#[derive(Debug, Clone, Copy)]
pub struct BotProjection<'a> {
    row: &'a FleetRow,
    displayed_script: Option<&'a str>,
    start_waiting: bool,
}

impl<'a> BotProjection<'a> {
    /// The projection of `row`, with the script choice and the waiting Start
    /// read from `scripts`.
    pub fn new<Io>(
        row: &'a FleetRow,
        scripts: Option<&'a Scripts>,
        core: &'a OperatorSession<Io>,
    ) -> Self {
        Self {
            row,
            displayed_script: scripts.and_then(|scripts| scripts.heading_label(core, row.name())),
            start_waiting: scripts.is_some_and(|scripts| scripts.start_waiting(row.name())),
        }
    }

    /// Profile / slot name.
    pub fn name(&self) -> &'a str {
        self.row.name()
    }

    /// Script lifecycle state.
    pub fn script(&self) -> RunState {
        self.row.script()
    }

    /// The script run's published walk is live. Tied to the run: never true
    /// while the script is idle or failed.
    pub fn script_walk(&self) -> bool {
        self.row.script_walk()
    }

    /// A WalkTo walk is armed for this bot. Independent of any script run.
    pub fn manual_walk(&self) -> bool {
        self.row.manual_walk()
    }

    /// Where the client is in its login lifecycle.
    pub fn phase(&self) -> Phase {
        self.row.phase()
    }

    /// Authenticated client session, independent of game readiness.
    pub fn connected(&self) -> bool {
        self.row.connected()
    }

    /// The client is in game and can act.
    pub fn in_game(&self) -> bool {
        self.row.loaded() && self.row.phase() == Phase::Ready
    }

    /// The script card the bot's Script section shows: its pending Browse
    /// selection, else its last assignment.
    pub fn displayed_script(&self) -> Option<&'a str> {
        self.displayed_script
    }

    /// The Fleet row label both front ends draw.
    pub fn label(&self) -> &'a str {
        self.row.brief()
    }

    /// Whether Fleet Stop acts on this bot, from the lifecycle fact alone:
    /// a walk is not a script, and a mark is not a fact about the bot.
    pub fn stop_eligibility(&self) -> StopEligibility {
        if self.row.script().is_live() {
            StopEligibility::Running
        } else if self.start_waiting {
            StopEligibility::StartQueued
        } else {
            StopEligibility::NoScript
        }
    }
}

impl<Io> OperatorSession<Io> {
    /// `name`'s projection as of the last poll, or `None` for a profile the
    /// fleet model does not project.
    pub fn bot_projection<'a>(
        &'a self,
        scripts: Option<&'a Scripts>,
        name: &str,
    ) -> Option<BotProjection<'a>> {
        let row = self.fleet_view().row(name)?;
        Some(BotProjection::new(row, scripts, self))
    }
}
