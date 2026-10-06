//! Group walk: one WalkTo destination for every marked bot.
//!
//! The panel picker and the TUI map both call [`walk_marked`], so the marked
//! set, the per-bot origin, eligibility and the bulk report are the same
//! whichever front end asked. The host routes each eligible bot from its own
//! observed tile ([`host_play::Play::map_walk_group`]); a bot that is not
//! logged in, has no position yet or is running a script is skipped with
//! that reason, and a bot the router refuses is reported as failed. A bot
//! that is marked more than once, or whose profile is gone, is still counted
//! once.

use host_play::walk_map::{
    ActionError, MapContext, MapWalkPlan, WalkRequest, WalkSlotOutcomeKind, WalkSlotRequest,
};
use host_play::{WalkArms, WorldMembersFact};
use nav::router::FindOptions;
use nav::tile::Tile;
use nav::WorldState;

use crate::bulk::{BulkOutcome, BulkReport, BulkRow};
use crate::selection::{profile_rows, MarkedSelection};
use crate::session::OperatorSession;

const LABEL: &str = "Walk marked";
const DONE: &str = "walking";

/// One bot's inputs to the router, supplied by the front end that owns the
/// per-bot world snapshot.
pub struct WalkInputs {
    pub state: WorldState,
    /// The bot's bank memory rows ([`host_play::Play::bank_rows`]).
    pub bank: nav::bank_fetch::BankRows,
    pub risk_input: host_play::admission::RiskInput,
}

/// The consumed destination plus the routing options that produced it.
pub struct MarkedWalk<'a> {
    /// The consumed map selection with its binding, or why none could be
    /// built (the confirmation was stale, nothing was selected, ...).
    pub prepared: Result<(MapContext, MapWalkPlan), ActionError>,
    /// The pending destination as read before it was consumed, for the
    /// per-bot receipts of a refused request.
    pub destination: Option<Tile>,
    pub options: FindOptions,
    pub arms: &'a WalkArms,
}

/// A report for a click with nothing marked.
pub fn none_marked() -> BulkReport {
    BulkReport::none_marked(LABEL, DONE)
}

/// Walk every marked bot to one destination. `inputs` supplies each bot's
/// own world state and bank list. Front ends check
/// [`MarkedSelection::is_empty`] before they consume the map selection.
///
/// "Done" in the report means the route is armed, not that the bot arrived.
pub fn walk_marked<Io>(
    selection: &MarkedSelection,
    core: &OperatorSession<Io>,
    walk: MarkedWalk<'_>,
    mut inputs: impl FnMut(&str) -> WalkInputs,
) -> BulkReport {
    let profiles = profile_rows(core);
    let mut rows = Vec::with_capacity(selection.len());
    let mut names: Vec<String> = Vec::with_capacity(selection.len());
    for identity in selection.iter() {
        match profiles.iter().find(|(_, id)| *id == identity) {
            Some((name, _)) if !names.contains(name) => names.push(name.clone()),
            Some(_) => {}
            None => rows.push(BulkRow::new(
                format!("profile#{}", identity.raw()),
                BulkOutcome::skipped("profile unavailable"),
            )),
        }
    }
    names.sort();

    let play = core.play();
    let profile = play.and_then(|play| play.server_profile());
    let members = profile
        .as_ref()
        .map_or(&WorldMembersFact::Unknown, |p| p.world_members());
    let refuse = |error: ActionError, mut rows: Vec<BulkRow>| {
        WalkRequest::refuse_group(
            play,
            &names,
            walk.destination,
            walk.options,
            members,
            error.clone(),
        );
        rows.extend(
            names
                .iter()
                .map(|name| BulkRow::new(name, BulkOutcome::failed(error.short()))),
        );
        BulkReport::new(LABEL, DONE, rows, Some(error.to_string()))
    };
    let (context, plan) = match walk.prepared {
        Ok(prepared) => prepared,
        Err(error) => return refuse(error, rows),
    };
    let Some(play) = play else {
        return refuse(ActionError::NoFocus, rows);
    };

    let bots: Vec<WalkInputs> = names.iter().map(|name| inputs(name)).collect();
    let requests: Vec<WalkSlotRequest<'_>> = names
        .iter()
        .zip(&bots)
        .map(|(name, bot)| WalkSlotRequest {
            name,
            state: &bot.state,
            bank: &bot.bank,
            risk_input: bot.risk_input,
        })
        .collect();
    let report = play.map_walk_group(plan, &context, &requests, walk.arms);
    let legacy_zones_unavailable = report.legacy_zones_unavailable;
    rows.extend(
        report
            .outcomes
            .into_iter()
            .enumerate()
            .map(|(index, outcome)| {
                let reason = outcome.kind.reason().unwrap_or("not eligible");
                let (result, detail) = match outcome.kind {
                    WalkSlotOutcomeKind::Walking => (BulkOutcome::Done, None),
                    WalkSlotOutcomeKind::Excluded(_) => (BulkOutcome::skipped(reason), None),
                    WalkSlotOutcomeKind::Failed(ActionError::BlockedByZones { detail }) => {
                        (BulkOutcome::failed(reason), detail)
                    }
                    WalkSlotOutcomeKind::Failed(ActionError::RiskRefused { detail, .. }) => {
                        (BulkOutcome::failed(reason), Some(detail))
                    }
                    WalkSlotOutcomeKind::Failed(_) => (BulkOutcome::failed(reason), None),
                };
                let row = BulkRow::new(outcome.name, result);
                match detail {
                    Some(detail) => row.with_detail(detail),
                    None if legacy_zones_unavailable && index == 0 => {
                        row.with_detail("zones: unavailable (legacy grid pack)")
                    }
                    None => row,
                }
            }),
    );
    BulkReport::new(LABEL, DONE, rows, None)
}

#[cfg(test)]
#[path = "group_walk_tests.rs"]
mod tests;
