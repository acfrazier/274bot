//! The shop-open rule shared by compat `Shop.open` ([`crate::shop`]) and the
//! native [`crate::native_shop::BuyMachine`]. Both adapters feed it the same
//! facts and carry out its decisions with their own walk and emit paths.
//!
//! - The keeper is a visible NPC with a Trade op. Its route is the walk to a
//!   stand beside it, from [`crate::reach_route::bounded_steps`] inside the
//!   engine's NPC view box.
//! - A reachable keeper gets one Trade. That attempt runs until a tick
//!   deadline. With a measured route it is the route at walk speed plus
//!   [`OPEN_SLACK_TICKS`], capped at [`OPEN_CAP_TICKS`]. Without one (no live
//!   reach view) it is [`OPEN_CAP_TICKS`]. "I can't reach that!" or a
//!   session change ends it at once. A new Trade follows only an ended
//!   attempt, at most [`OPEN_ATTEMPTS`] in all.
//! - An unreachable keeper (a closed door) is walked to once by the
//!   adapter's nav walk, then checked again.
//! - The shop is open only when a usable stock page shows on a tick after
//!   the Trade.

use crate::reach_route;
use api::query::ReachQueryView;
use api::WorldTile;

/// Existing bound on Trade requests for one shop open (rs2b0t contract).
pub const OPEN_ATTEMPTS: u32 = 3;
/// Policy margin, in ticks, that one Trade with a measured route may take
/// beyond the route length before its attempt expires. It is not an engine
/// bound on how long an interaction can take. The route length covers the
/// walk: a player steps at most one tile per cycle at walk speed
/// (`PathingEntity.ts:144-149`, `Player.ts:729-731`); running only arrives
/// sooner. On top of the walk:
/// - 1 (engine tick order): even a 0-tile route opens no earlier than the
///   next cycle. `OpNpc` is decoded in `processClientsIn` (`World.ts:355`)
///   and the interaction runs in `processPlayers` (`World.ts:378`,
///   `Player.ts:1247-1314`).
/// - 1 (engine input budget): the `OpNpc` can be decoded one cycle after its
///   `MOVE_OPCLICK` when that cycle's five user events are spent
///   (`ClientGameProtCategory.ts:6`, `NetworkPlayer.ts:69`; named in
///   `MoveClickHandler.ts:26-29`).
/// - 10 (policy): room for the keeper wandering and the player re-pathing.
///   A wander destination stays within `wanderrange` (default 5,
///   `NpcType.ts:101`) of spawn (`Npc.ts:705-714`), but that does not bound
///   the extra walking or how often it re-paths. Measured instead: every
///   un-attacked live open of a keeper 0-5 steps away settled 1-3 ticks
///   after the Trade that opened it, and a 12-step one in 7.
pub const OPEN_SLACK_TICKS: u64 = 12;
/// Policy bound on one attempt's ticks: the deadline when the route is not
/// measured, and the ceiling for a measured one. It is not a maximum walk
/// time. 104 echoes the engine's first-waypoint distance check for one
/// click path (`MoveClickHandler.ts:20`), which does not bound a walk's
/// length. Kept as a generous wait: about 70 s per attempt at 600 ms ticks,
/// so a three-attempt call ends within about 3.5 minutes of game time.
pub const OPEN_CAP_TICKS: u64 = 104 + OPEN_SLACK_TICKS;

/// Frozen `CANT_REACH` (`/^i can't reach that/i`): the engine's
/// `Player.ts:1293-1296` verdict that an interaction cannot path.
pub fn is_cant_reach(text: &str) -> bool {
    const PREFIX: &[u8] = b"i can't reach that";
    let bytes = text.as_bytes();
    bytes.len() >= PREFIX.len() && bytes[..PREFIX.len()].eq_ignore_ascii_case(PREFIX)
}

/// Ticks a Trade may take to open the shop: a measured `tiles`-long route
/// plus the slack, capped.
pub(crate) fn open_deadline_ticks(route: KeeperRoute) -> u64 {
    match route {
        KeeperRoute::Steps(tiles) => (u64::from(tiles) + OPEN_SLACK_TICKS).min(OPEN_CAP_TICKS),
        // A straight-line distance is a lower bound on the walk, not an
        // upper one: a keeper two tiles away can be a long walk round a
        // wall. The core never trades an unreachable keeper.
        KeeperRoute::Unknown(_) | KeeperRoute::Unreachable => OPEN_CAP_TICKS,
    }
}

/// One observed session. A Trade sent in another session is gone with it
/// on the server.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Session {
    pub(crate) epoch: u64,
    pub(crate) login: u64,
}

impl Session {
    pub const fn new(epoch: u64, login: u64) -> Self {
        Self { epoch, login }
    }
}

/// How the keeper stands from the player.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeeperRoute {
    /// A stand beside the keeper is reachable now, this many steps away.
    Steps(u32),
    /// No stand is reachable inside the NPC view box.
    Unreachable,
    /// No live reach view (none posted, or its flood starts off the
    /// player's tile). The straight-line distance ranks keepers only; the
    /// attempt waits the full [`OPEN_CAP_TICKS`].
    Unknown(u32),
}

/// The keeper a call trades with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Keeper {
    pub(crate) name: String,
    pub(crate) action: String,
    pub(crate) index: i32,
    /// The keeper's network south-west tile (the client's `route_x[0]`,
    /// `client.rs:3988-3998`).
    pub(crate) tile: WorldTile,
    pub(crate) route: KeeperRoute,
}

impl Keeper {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn into_name(self) -> String {
        self.name
    }

    pub fn action(&self) -> &str {
        &self.action
    }

    pub const fn index(&self) -> i32 {
        self.index
    }

    pub const fn route(&self) -> KeeperRoute {
        self.route
    }
}

/// One posted NPC row, as either adapter reads it.
pub struct Candidate<'a> {
    pub(crate) name: &'a str,
    /// The row's Trade op, if it has one.
    pub(crate) action: Option<&'a str>,
    pub(crate) index: i32,
    pub(crate) tile: WorldTile,
    pub(crate) size: i32,
    pub(crate) distance: i32,
}

impl<'a> Candidate<'a> {
    pub fn new(
        name: &'a str,
        action: Option<&'a str>,
        index: i32,
        tile: WorldTile,
        size: i32,
        distance: i32,
    ) -> Self {
        Self {
            name,
            action,
            index,
            tile,
            size,
            distance,
        }
    }
}

/// Whether `op` opens a shop: frozen `Shop.open` presses `Trade`. Some
/// keepers (the native callers' content) label it `Trade-with`.
pub(crate) fn is_trade_op(op: &str) -> bool {
    op.eq_ignore_ascii_case("trade") || op.eq_ignore_ascii_case("trade-with")
}

/// The candidate with a Trade op to trade with: the reachable one with the
/// shortest walk first; without a live reach view, the nearest. `view` is
/// the live reach view and the player's tile ([`reach_route::live`]).
pub fn choose_keeper<'a>(
    candidates: impl IntoIterator<Item = Candidate<'a>>,
    view: Option<(&ReachQueryView, WorldTile)>,
) -> Option<Keeper> {
    candidates
        .into_iter()
        .filter_map(|row| {
            let action = row.action?;
            let size = row.size.max(1);
            let distance = u32::try_from(row.distance).unwrap_or(0);
            let route = match view {
                Some((view, here)) => {
                    let fp = reach_route::Footprint {
                        tile: row.tile,
                        width: size,
                        length: size,
                    };
                    reach_route::bounded_steps(view, here, fp, reach_route::NPC_VIEW_DISTANCE)
                        .steps
                        .map_or(KeeperRoute::Unreachable, KeeperRoute::Steps)
                }
                None => KeeperRoute::Unknown(distance),
            };
            let key = match route {
                KeeperRoute::Steps(steps) | KeeperRoute::Unknown(steps) => (0, steps),
                KeeperRoute::Unreachable => (1, distance),
            };
            Some((
                key,
                Keeper {
                    name: row.name.to_string(),
                    action: action.to_string(),
                    index: row.index,
                    tile: row.tile,
                    route,
                },
            ))
        })
        .min_by_key(|(key, _)| *key)
        .map(|(_, keeper)| keeper)
}

/// What the adapter observed this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Facts {
    /// The game tick, `None` before the session's first tick or on a logout post.
    pub(crate) tick: Option<u64>,
    pub(crate) session: Session,
    /// Highest chat line seq this session (a Trade's mark).
    pub(crate) chat_seq: i32,
    /// Highest seq of an "I can't reach that!" line this session.
    pub(crate) cant_reach_seq: i32,
    /// The shop modal is open with a posted, non-empty stock page.
    pub(crate) usable: bool,
    /// Another caller already sent a Trade on this tick.
    pub(crate) trade_taken: bool,
}

impl Facts {
    pub const fn new(
        tick: Option<u64>,
        session: Session,
        chat_seq: i32,
        cant_reach_seq: i32,
        usable: bool,
        trade_taken: bool,
    ) -> Self {
        Self {
            tick,
            session,
            chat_seq,
            cant_reach_seq,
            usable,
            trade_taken,
        }
    }
}

/// One outstanding Trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Attempt {
    trade_tick: u64,
    session: Session,
    chat_mark: i32,
    /// Last tick on which this attempt may still open the shop.
    deadline: u64,
}

impl Attempt {
    /// `tick` is this Trade's own tick (or earlier) in its session. Nothing
    /// observed there answers this Trade, and the Trade will close any modal
    /// already open (`OpNpcHandler.ts:47` -> `Player.ts:970-973`).
    fn pending_on(&self, tick: u64, session: Session) -> bool {
        session == self.session && tick <= self.trade_tick
    }

    fn ended(&self, tick: u64, facts: &Facts) -> bool {
        facts.session != self.session
            || facts.cant_reach_seq > self.chat_mark
            || tick > self.deadline
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    /// The modal was up at the start without a usable page. The page is
    /// flushed with the modal (`World.ts:1096-1120`), so it is posted by the
    /// next tick; after `until` this falls back to a Trade.
    Page {
        until: u64,
        session: Session,
    },
    Walking,
    Trade(Attempt),
}

/// What the adapter does next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Wait,
    /// The shop is open with a usable page: settle.
    Opened,
    /// Send this keeper's Trade, then report it with [`OpenCore::traded`].
    Trade(Keeper),
    /// Walk to the keeper's tile (radius 1), then report the outcome with
    /// [`OpenCore::walked`].
    Walk(WorldTile),
    /// The open failed for this reason.
    Failed(&'static str),
}

/// The shop-open state of one call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenCore {
    state: State,
    attempts_left: u32,
    walked: bool,
}

impl Default for OpenCore {
    fn default() -> Self {
        Self {
            state: State::Idle,
            attempts_left: OPEN_ATTEMPTS,
            walked: false,
        }
    }
}

impl OpenCore {
    /// Start with the modal already open but its page not usable.
    pub fn awaiting_page(facts: &Facts) -> Self {
        Self {
            state: State::Page {
                until: facts.tick.unwrap_or(0) + 1,
                session: facts.session,
            },
            ..Self::default()
        }
    }

    #[cfg(test)]
    fn walking(&self) -> bool {
        self.state == State::Walking
    }

    /// A usable page on a tick later than this call's Trade, on a tick no
    /// other Trade went out. A snapshot may settle on this. Trades and
    /// expiry belong to real ticks ([`Self::tick`]).
    pub fn ready(&self, facts: &Facts) -> bool {
        let Some(tick) = facts.tick else {
            return false;
        };
        match self.state {
            State::Trade(attempt) if attempt.pending_on(tick, facts.session) => return false,
            // No Trade went out yet: a shop seen mid-walk is not this call's.
            State::Walking => return false,
            _ => {}
        }
        facts.usable && !facts.trade_taken
    }

    /// One real tick. `keeper` is read only when a Trade or walk is due.
    /// While walking this waits; the adapter polls its walk and reports with
    /// [`Self::walked`].
    pub fn tick(&mut self, facts: &Facts, keeper: impl FnOnce() -> Option<Keeper>) -> Decision {
        let Some(tick) = facts.tick else {
            return Decision::Wait;
        };
        if self.ready(facts) {
            return Decision::Opened;
        }
        match self.state {
            State::Idle => {}
            State::Walking => return Decision::Wait,
            State::Page { until, session } => {
                if session == facts.session && tick <= until {
                    return Decision::Wait;
                }
                self.state = State::Idle;
            }
            State::Trade(attempt) => {
                if attempt.pending_on(tick, facts.session) || !attempt.ended(tick, facts) {
                    return Decision::Wait;
                }
                self.state = State::Idle;
                self.attempts_left = self.attempts_left.saturating_sub(1);
                if self.attempts_left == 0 {
                    return Decision::Failed("no usable shop after 3 Trade attempts");
                }
            }
        }
        if facts.trade_taken {
            return Decision::Wait;
        }
        self.next(keeper())
    }

    fn next(&mut self, keeper: Option<Keeper>) -> Decision {
        let Some(keeper) = keeper else {
            return Decision::Failed("the keeper is gone");
        };
        match keeper.route {
            KeeperRoute::Unreachable if self.walked => {
                Decision::Failed("the keeper is still unreachable after the walk to it")
            }
            KeeperRoute::Unreachable => {
                self.walked = true;
                self.state = State::Walking;
                Decision::Walk(keeper.tile)
            }
            KeeperRoute::Steps(_) | KeeperRoute::Unknown(_) => Decision::Trade(keeper),
        }
    }

    /// The adapter sent `keeper`'s Trade on `facts.tick`.
    pub fn traded(&mut self, facts: &Facts, keeper: &Keeper) {
        let Some(tick) = facts.tick else {
            return;
        };
        self.state = State::Trade(Attempt {
            trade_tick: tick,
            session: facts.session,
            chat_mark: facts.chat_seq,
            deadline: tick + open_deadline_ticks(keeper.route),
        });
    }

    /// The deadline of the outstanding Trade, if any (for traces). Only the
    /// compat `Shop.open` adapter (`load`) and tests read it.
    pub fn deadline(&self) -> Option<u64> {
        match self.state {
            State::Trade(attempt) => Some(attempt.deadline),
            _ => None,
        }
    }

    /// The adapter's walk to the keeper ended. An arrival continues on the
    /// same tick with a fresh keeper read. A failed walk fails the open.
    pub fn walked(&mut self, arrived: bool, keeper: impl FnOnce() -> Option<Keeper>) -> Decision {
        if !arrived {
            return Decision::Failed("the walk to the unreachable keeper failed");
        }
        self.state = State::Idle;
        self.next(keeper())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(tick: u64) -> Facts {
        Facts {
            tick: Some(tick),
            session: Session::default(),
            chat_seq: 0,
            cant_reach_seq: 0,
            usable: false,
            trade_taken: false,
        }
    }

    fn keeper(route: KeeperRoute) -> Keeper {
        Keeper {
            name: "Shop keeper".into(),
            action: "Trade".into(),
            index: 3,
            tile: WorldTile {
                x: 10,
                z: 10,
                level: 0,
            },
            route,
        }
    }

    /// Drive one tick, sending any Trade the core asks for.
    fn run(core: &mut OpenCore, facts: &Facts, route: KeeperRoute) -> Decision {
        let decision = core.tick(facts, || Some(keeper(route)));
        if let Decision::Trade(keeper) = &decision {
            core.traded(facts, keeper);
        }
        decision
    }

    #[test]
    fn deadline_is_route_plus_slack_capped_and_the_cap_without_a_route() {
        assert_eq!(open_deadline_ticks(KeeperRoute::Steps(0)), OPEN_SLACK_TICKS);
        assert_eq!(
            open_deadline_ticks(KeeperRoute::Steps(30)),
            30 + OPEN_SLACK_TICKS
        );
        assert_eq!(open_deadline_ticks(KeeperRoute::Steps(104)), OPEN_CAP_TICKS);
        assert_eq!(
            open_deadline_ticks(KeeperRoute::Steps(5_000)),
            OPEN_CAP_TICKS
        );
        for tiles in [0, 2, 5_000] {
            assert_eq!(
                open_deadline_ticks(KeeperRoute::Unknown(tiles)),
                OPEN_CAP_TICKS
            );
        }
    }

    /// Review R1: a keeper two tiles away in a straight line with no live
    /// reach view gets the cap, not 2 + slack: the walk round a wall may be
    /// far longer. No second Trade before Trade + cap.
    #[test]
    fn an_unknown_route_waits_the_cap_before_a_second_trade() {
        let mut core = OpenCore::default();
        let route = KeeperRoute::Unknown(2);
        assert!(matches!(
            run(&mut core, &facts(99), route),
            Decision::Trade(_)
        ));
        assert_eq!(core.deadline(), Some(99 + OPEN_CAP_TICKS));
        for tick in 100..=99 + OPEN_CAP_TICKS {
            assert_eq!(
                run(&mut core, &facts(tick), route),
                Decision::Wait,
                "{tick}"
            );
        }
        assert!(matches!(
            run(&mut core, &facts(100 + OPEN_CAP_TICKS), route),
            Decision::Trade(_)
        ));
    }

    #[test]
    fn one_trade_per_attempt_until_its_deadline_then_false_after_three() {
        let mut core = OpenCore::default();
        let route = KeeperRoute::Steps(4);
        let mut trades = Vec::new();
        let mut tick = 1;
        let result = loop {
            match run(&mut core, &facts(tick), route) {
                Decision::Trade(_) => trades.push(tick),
                Decision::Wait => {}
                other => break other,
            }
            tick += 1;
        };
        let step = 4 + OPEN_SLACK_TICKS + 1;
        assert_eq!(trades, vec![1, 1 + step, 1 + 2 * step]);
        assert_eq!(tick, 1 + 3 * step);
        assert!(matches!(result, Decision::Failed(_)));
    }

    #[test]
    fn the_trade_tick_never_settles_and_another_trade_this_tick_defers() {
        let mut core = OpenCore::default();
        assert!(matches!(
            run(&mut core, &facts(1), KeeperRoute::Steps(0)),
            Decision::Trade(_)
        ));
        let usable = Facts {
            usable: true,
            ..facts(1)
        };
        assert!(!core.ready(&usable));
        assert_eq!(
            core.tick(
                &Facts {
                    usable: true,
                    ..facts(2)
                },
                || None
            ),
            Decision::Opened
        );
        let mut fresh = OpenCore::default();
        let taken = Facts {
            trade_taken: true,
            ..facts(5)
        };
        assert_eq!(
            fresh.tick(&taken, || Some(keeper(KeeperRoute::Steps(0)))),
            Decision::Wait
        );
    }

    #[test]
    fn an_unreachable_keeper_is_walked_to_once() {
        let mut core = OpenCore::default();
        assert_eq!(
            run(&mut core, &facts(1), KeeperRoute::Unreachable),
            Decision::Walk(keeper(KeeperRoute::Unreachable).tile)
        );
        assert!(core.walking());
        assert_eq!(
            run(&mut core, &facts(2), KeeperRoute::Steps(0)),
            Decision::Wait
        );
        assert!(matches!(
            core.walked(true, || Some(keeper(KeeperRoute::Steps(1)))),
            Decision::Trade(_)
        ));
        let mut blocked = OpenCore::default();
        let _ = run(&mut blocked, &facts(1), KeeperRoute::Unreachable);
        assert!(matches!(
            blocked.walked(true, || Some(keeper(KeeperRoute::Unreachable))),
            Decision::Failed(_)
        ));
        let mut failed = OpenCore::default();
        let _ = run(&mut failed, &facts(1), KeeperRoute::Unreachable);
        assert!(matches!(failed.walked(false, || None), Decision::Failed(_)));
    }

    #[test]
    fn cant_reach_and_session_change_end_an_attempt_at_once() {
        let mut core = OpenCore::default();
        let _ = run(&mut core, &facts(1), KeeperRoute::Steps(20));
        let refused = Facts {
            chat_seq: 1,
            cant_reach_seq: 1,
            ..facts(2)
        };
        assert!(matches!(
            run(&mut core, &refused, KeeperRoute::Steps(20)),
            Decision::Trade(_)
        ));
        let moved = Facts {
            session: Session { epoch: 9, login: 0 },
            ..facts(1)
        };
        assert!(matches!(
            run(&mut core, &moved, KeeperRoute::Steps(20)),
            Decision::Trade(_)
        ));
    }

    #[test]
    fn cant_reach_text_rule() {
        assert!(is_cant_reach("I can't reach that!"));
        assert!(is_cant_reach("i can't reach that"));
        assert!(!is_cant_reach("You can't reach that."));
    }
}
