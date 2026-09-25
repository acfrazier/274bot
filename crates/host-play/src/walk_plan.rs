use std::collections::VecDeque;

use api::snapshot::WorldTile;
use nav::bank_fetch::{plan_bank_fetch, BankFetch, BankStep};
use nav::router::{
    find_first_missing_item_reqs, find_missing_item_reqs, find_with, FindOptions, MissingReq, Route,
};
use nav::world::NavWorld;
use nav::WorldState;

/// A latched BankBudget session: remaining [`BankStep`]s plus the dest
/// the arm re-finds after the session lands. Follow freezes only for
/// Open / Deposit / Withdraw / Wear / Close; [`BankStep::Walk`] follows
/// the stand sub-route. Wear-from-inv and bank-trip deposit/withdraw
/// both pump through the same path. `final_route` is the post-session
/// route (status row + follow once steps clear); a Walk-to-stand may
/// temporarily replace `WalkArm::route` / `NavBot::route`.
#[derive(Debug, Clone)]
pub struct PendingBankFetch {
    pub steps: VecDeque<BankStep>,
    pub dest: WorldTile,
    pub opts: FindOptions,
    pub final_route: Route,
}

/// Outcome of a walk-arm route attempt: a direct route, a BankBudget
/// session plus the post-session route, or no path.
pub(super) enum RouteOutcome {
    Routed(Route),
    BankSession {
        pending: PendingBankFetch,
        route: Route,
    },
    NoPath,
}

/// Strict `find_with`, then — only when `allow_bank_fetch` is on and the
/// failure is solely missing item/worn reqs — plan a BankBudget session
/// and re-find against the session's post state. Never inserts a virtual
/// bank edge into Dijkstra.
pub(super) fn route_or_bank_fetch(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    bank: &[(i32, i32)],
) -> RouteOutcome {
    match find_with(&world.collision, &world.graph, from, to, opts, state) {
        Ok(route) => RouteOutcome::Routed(route),
        Err(_) if opts.allow_bank_fetch => {
            find_missing_item_reqs(&world.collision, &world.graph, from, to, opts, state)
                .and_then(|missing| plan_bank_fetch(&missing, state, bank, world.banks(), from))
                .and_then(|fetch| session_route(world, from, to, fetch, opts))
                .unwrap_or(RouteOutcome::NoPath)
        }
        Err(_) => RouteOutcome::NoPath,
    }
}

/// BankBudget diagnosis after the caller's first-goal search over `targets`
/// failed: one relaxed first-goal search names the cheapest target it
/// reaches and that target's missing facts, instead of one relaxed flood per
/// target. A target that needs no missing fact was only past the strict
/// search's budget, so it routes strictly. Whether a session can be planned
/// depends only on the missing facts, so the rest of the targets are
/// diagnosed again only until an unplannable set repeats; a target whose
/// post-session re-find fails is dropped.
pub(super) fn bank_fetch_after_first_no_path(
    world: &NavWorld,
    from: WorldTile,
    targets: &[WorldTile],
    opts: FindOptions,
    state: &WorldState,
    bank: &[(i32, i32)],
) -> RouteOutcome {
    if !opts.allow_bank_fetch {
        return RouteOutcome::NoPath;
    }
    let mut remaining = targets.to_vec();
    let mut unplannable: Vec<Vec<MissingReq>> = Vec::new();
    while !remaining.is_empty() {
        let diagnosis = find_first_missing_item_reqs(
            &world.collision,
            &world.graph,
            from,
            &remaining,
            opts,
            state,
        );
        let Some((target, missing)) = diagnosis.found() else {
            break;
        };
        if missing.is_empty() {
            return find_with(&world.collision, &world.graph, from, target, opts, state)
                .map_or(RouteOutcome::NoPath, RouteOutcome::Routed);
        }
        if unplannable.iter().any(|failed| failed == missing) {
            break;
        }
        match plan_bank_fetch(missing, state, bank, world.banks(), from) {
            Some(fetch) => {
                if let Some(outcome) = session_route(world, from, target, fetch, opts) {
                    return outcome;
                }
            }
            None => unplannable.push(missing.to_vec()),
        }
        remaining.retain(|&tile| tile != target);
    }
    RouteOutcome::NoPath
}

/// Re-find `to` against a planned session's post state (ADR 0005: find
/// itself stays fail-closed; the session is what unblocks).
fn session_route(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    fetch: BankFetch,
    opts: FindOptions,
) -> Option<RouteOutcome> {
    let opts = FindOptions {
        allow_bank_fetch: false,
        ..opts
    };
    let route = find_with(&world.collision, &world.graph, from, to, opts, &fetch.state).ok()?;
    Some(RouteOutcome::BankSession {
        pending: PendingBankFetch {
            steps: fetch.steps.into(),
            dest: to,
            opts,
            final_route: route.clone(),
        },
        route,
    })
}
