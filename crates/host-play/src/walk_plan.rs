use std::collections::VecDeque;

use api::snapshot::WorldTile;
use nav::bank_fetch::{plan_bank_fetch, BankStep};
use nav::router::{find_missing_item_reqs, find_with, FindOptions, Route};
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
        Err(_) => bank_fetch_after_no_path(world, from, to, opts, state, bank),
    }
}

/// BankBudget diagnosis after the caller's strict search already proved
/// `NoPath`. Keeping this separate prevents a multi-target caller from
/// repeating the same full strict flood once per candidate.
pub(super) fn bank_fetch_after_no_path(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    opts: FindOptions,
    state: &WorldState,
    bank: &[(i32, i32)],
) -> RouteOutcome {
    if !opts.allow_bank_fetch {
        return RouteOutcome::NoPath;
    }
    let Some(missing) =
        find_missing_item_reqs(&world.collision, &world.graph, from, to, opts, state)
    else {
        return RouteOutcome::NoPath;
    };
    let Some(fetch) = plan_bank_fetch(&missing, state, bank, world.banks(), from) else {
        return RouteOutcome::NoPath;
    };
    // Re-find against the post-session state (ADR 0005: find itself stayed
    // fail-closed; the session is what unblocks).
    let Ok(route) = find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions {
            allow_bank_fetch: false,
            ..opts
        },
        &fetch.state,
    ) else {
        return RouteOutcome::NoPath;
    };
    RouteOutcome::BankSession {
        pending: PendingBankFetch {
            steps: fetch.steps.into(),
            dest: to,
            opts: FindOptions {
                allow_bank_fetch: false,
                ..opts
            },
            final_route: route.clone(),
        },
        route,
    }
}
