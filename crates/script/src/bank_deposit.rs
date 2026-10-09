//! Rust-owned deposit loop, frozen `Bank.depositAllMatching`: the
//! `bank_deposit` [`crate::machine`] family, and the [`Deposit`] piece the
//! bank-nearest run reuses.
//!
//! Each round reads both the posted bank-side backpack and the current main
//! inventory. A side root down or a side view without the current candidate
//! rows waits up to 1.2 s and then fails rather than reporting success; stale
//! side-only rows are never clicked. The matcher picks the first usable row in
//! posted order (a script predicate through the callback path, every row whose
//! id is not kept, or every row) and presses that exact row's All op through
//! the [`crate::bank::ops`] kernel (an id, slot and component, never a name),
//! then waits for that id to leave the pack. No match, an unsettled deposit, a
//! closed or new bank session, or 32 rounds end the loop.

use crate::bank::ops::{
    self, DepositKind, DepositRequest, DepositScan, DepositSpec, DEPOSIT_VIEW_MS, MAX_DEPOSITS,
    TRANSFER_OBSERVED_TICK_LIMIT,
};
use crate::bank_op::{self, BankView};
use crate::machine::{Begin, Call, Cx, Family, Reply, Step, Thrown};
use crate::observed::{self, ItemRow};
use serde::Deserialize;
use serde_json::{json, Value};

const VIEW_NOT_READY: &str = "deposit view not ready — waiting for the side backpack";

/// Which backpack rows a deposit takes.
pub(crate) enum Matcher {
    /// A script predicate, `hook(name, id)` (`with_id`) or `hook(name)`
    /// (`depositMatcher`'s own); `common` then also takes the common bank
    /// loot the predicate refused.
    Hook {
        hook: usize,
        with_id: bool,
        common: bool,
    },
    /// `depositAllExcept(names)`: every row except the ids whose display
    /// name is kept. The names resolve to ids once, on the first posted
    /// side; a deposit only removes rows, so no kept id appears later.
    Keep {
        names: Vec<String>,
        ids: Option<Vec<i32>>,
    },
    /// Every row (`depositInventory`).
    All,
}

impl Matcher {
    fn keep(names: Vec<String>) -> Self {
        Self::Keep { names, ids: None }
    }
}

/// Every id in `side` whose display name is one of `names`.
fn kept_ids(side: &[ItemRow], names: &[String]) -> Vec<i32> {
    let mut ids = Vec::new();
    for id in names.iter().flat_map(|name| ops::ids_named(side, name)) {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
    ids
}

#[derive(Clone, Copy)]
enum Phase {
    /// Read the posted side for the next round.
    Scan,
    /// A predicate: asking about posted row `index`; `asked` is the
    /// `(id, slot)` the question named.
    Match {
        index: usize,
        asked: Option<(i32, Option<i32>)>,
    },
    Await {
        id: i32,
        before: i32,
        started_tick: u64,
    },
}

/// One deposit loop.
pub(crate) struct Deposit {
    matcher: Matcher,
    /// The caller's `log` hook, if it passed one.
    log: Option<usize>,
    common_casket_id: Option<i32>,
    round: u8,
    /// The bank session the loop deposits in, read on its first scan.
    session: Option<u64>,
    /// The side view's not-ready bound is armed.
    view_armed: bool,
    phase: Phase,
}

/// What a scan decided.
enum Next {
    Press(ops::DepositClick),
    Ask,
    WaitView,
    ViewExpired,
    Done,
}

impl Deposit {
    pub(crate) fn new(matcher: Matcher, log: Option<usize>) -> Self {
        Self {
            matcher,
            log,
            common_casket_id: crate::supply_v2::random_event_casket_id(),
            round: 0,
            session: None,
            view_armed: false,
            phase: Phase::Scan,
        }
    }

    /// Whether the posted bank is still the session this loop began in.
    fn same_session(&mut self, view: &BankView) -> bool {
        let session = *self.session.get_or_insert(view.generation);
        view.open && view.generation == session
    }

    /// One round's decision from the posted side and pack.
    fn scan(&mut self, same_session: bool, view_wait_expired: bool) -> Next {
        observed::with(|scene| {
            let session = scene.since_login();
            let side = bank_op::posted_side(session);
            let pack = session.inv().map(Vec::as_slice);
            if matches!(self.matcher, Matcher::Hook { .. }) {
                if !same_session {
                    return Next::Done;
                }
                let Some(pack) = pack else {
                    return Next::WaitView;
                };
                if !pack.iter().any(|row| row.count > 0) {
                    return Next::Done;
                }
                let usable = side.is_some_and(|side| {
                    pack.iter()
                        .filter(|row| row.count > 0)
                        .all(|row| ops::pack_has_current_row(side, row.id, row.slot))
                });
                return if usable {
                    Next::Ask
                } else if view_wait_expired {
                    Next::ViewExpired
                } else {
                    Next::WaitView
                };
            }
            let keep: &[i32] = match (&mut self.matcher, side) {
                (Matcher::All, _) => &[],
                (Matcher::Keep { names, ids }, Some(side)) => {
                    ids.get_or_insert_with(|| kept_ids(side, names))
                }
                // Not posted yet: `deposit_next` waits before any click.
                (Matcher::Keep { ids, .. }, None) => ids.as_deref().unwrap_or_default(),
                (Matcher::Hook { .. }, _) => unreachable!("hook scan returned above"),
            };
            let spec = DepositSpec {
                kind: DepositKind::UntilEmpty,
                keep,
                only: None,
            };
            match ops::deposit_next(&spec, side, pack, view_wait_expired, same_session) {
                DepositScan::Click(click) => Next::Press(click),
                DepositScan::WaitView => Next::WaitView,
                DepositScan::ViewExpired => Next::ViewExpired,
                DepositScan::Done | DepositScan::SessionGone => Next::Done,
                DepositScan::MissingRequired => {
                    unreachable!("until-empty selection has no capacity operation")
                }
            }
        })
    }

    /// Press `click` in this loop's session and wait for it to settle.
    fn press(&mut self, click: ops::DepositClick, generation: u64, cx: &mut Cx<'_>) -> bool {
        let (before, started_tick) = observed::with(|scene| {
            let started_tick = scene.tick().unwrap_or_default();
            let before = scene
                .since_login()
                .inv()
                .map(|inv| ops::count_id(inv, click.id));
            (before, started_tick)
        });
        let Some(before) = before.filter(|before| *before > 0) else {
            return false;
        };
        cx.emit(ops::deposit_req(click, generation));
        self.view_armed = false;
        self.phase = Phase::Await {
            id: click.id,
            before,
            started_tick,
        };
        true
    }

    /// Drive the loop; the reply of a predicate call it asked for is read
    /// here, so a family embedding it routes its steps here while it runs.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Step<()> {
        loop {
            match self.phase {
                Phase::Scan => {
                    // The reply to the not-ready log line, if one was asked.
                    if let Some(Reply::Threw(thrown)) = cx.reply() {
                        return Step::Fail(thrown);
                    }
                    if self.round >= MAX_DEPOSITS {
                        return Step::Done(());
                    }
                    let view = BankView::now();
                    let same_session = self.same_session(&view);
                    let view_wait_expired = self.view_armed && cx.clock().bound_reached();
                    match self.scan(same_session, view_wait_expired) {
                        Next::Done => return Step::Done(()),
                        Next::ViewExpired => {
                            return Step::Fail(Thrown::new("bank deposit view did not post"));
                        }
                        Next::Press(click) => {
                            self.press(click, view.generation, cx);
                            return Step::Wait;
                        }
                        Next::Ask => {
                            self.phase = Phase::Match {
                                index: 0,
                                asked: None,
                            };
                        }
                        Next::WaitView if self.view_armed => {
                            if cx.clock().bound_reached() {
                                return Step::Fail(Thrown::new("bank deposit view did not post"));
                            }
                            return Step::Wait;
                        }
                        Next::WaitView => {
                            self.view_armed = true;
                            cx.clock().arm(DEPOSIT_VIEW_MS);
                            if let Some(hook) = self.log.filter(|hook| cx.has(*hook)) {
                                return Step::Call(Call {
                                    hook,
                                    args: vec![json!(VIEW_NOT_READY)],
                                });
                            }
                            return Step::Wait;
                        }
                    }
                }
                Phase::Match { index, asked } => {
                    let Matcher::Hook {
                        hook,
                        with_id,
                        common,
                    } = self.matcher
                    else {
                        unreachable!("only a predicate is asked row by row");
                    };
                    let (side_posted, row) = observed::with(|scene| {
                        let session = scene.since_login();
                        match (bank_op::posted_side(session), session.inv()) {
                            (Some(side), Some(pack)) => (
                                true,
                                side.get(index).map(|row| {
                                    (
                                        row.name.clone(),
                                        row.id,
                                        row.slot,
                                        row.count > 0
                                            && ops::pack_has_current_row(pack, row.id, row.slot),
                                    )
                                }),
                            ),
                            _ => (false, None),
                        }
                    });
                    if !side_posted {
                        if asked.is_some() {
                            if let Some(Reply::Threw(thrown)) = cx.reply() {
                                return Step::Fail(thrown);
                            }
                        }
                        self.phase = Phase::Scan;
                        continue;
                    }
                    let Some((name, id, slot, current)) = row else {
                        if asked.is_some() {
                            if let Some(Reply::Threw(thrown)) = cx.reply() {
                                return Step::Fail(thrown);
                            }
                            self.phase = Phase::Scan;
                            continue;
                        }
                        return Step::Done(());
                    };
                    if asked.is_none() && !current {
                        self.phase = Phase::Match {
                            index: index + 1,
                            asked: None,
                        };
                        continue;
                    }
                    let name = name.as_deref().unwrap_or_default();
                    let Some(asked) = asked else {
                        self.phase = Phase::Match {
                            index,
                            asked: Some((id, slot)),
                        };
                        let args = if with_id {
                            vec![json!(name), json!(id)]
                        } else {
                            vec![json!(name)]
                        };
                        return Step::Call(Call { hook, args });
                    };
                    let hit = match cx.reply() {
                        Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
                        Some(Reply::Value(value)) => {
                            truthy(&value)
                                || (common
                                    && api::content::matches_common_bank_loot(
                                        self.common_casket_id,
                                        name,
                                        id,
                                    ))
                        }
                        None => false,
                    };
                    if !current {
                        self.phase = Phase::Scan;
                        continue;
                    }
                    if !hit {
                        self.phase = Phase::Match {
                            index: index + 1,
                            asked: None,
                        };
                        continue;
                    }
                    let view = BankView::now();
                    if !self.same_session(&view) {
                        return Step::Done(());
                    }
                    if asked != (id, slot) {
                        // The row moved while the predicate answered: a new
                        // round asks again from the top.
                        self.round += 1;
                        self.phase = Phase::Scan;
                        continue;
                    }
                    let (still_current, click) = observed::with(|scene| {
                        let session = scene.since_login();
                        let Some(pack) = session.inv() else {
                            return (false, None);
                        };
                        let Some(side) = bank_op::posted_side(session) else {
                            return (false, None);
                        };
                        let Some(row) = side.get(index) else {
                            return (false, None);
                        };
                        let still_current = row.count > 0
                            && (row.id, row.slot) == asked
                            && ops::pack_has_current_row(pack, row.id, row.slot);
                        (
                            still_current,
                            still_current
                                .then(|| ops::deposit_click(row, DepositRequest::Sweep))
                                .flatten(),
                        )
                    });
                    if !still_current {
                        self.phase = Phase::Scan;
                        continue;
                    }
                    // Frozen: a matched current row with no op ends the loop.
                    let Some(click) = click else {
                        return Step::Done(());
                    };
                    if self.press(click, view.generation, cx) {
                        return Step::Wait;
                    }
                    self.phase = Phase::Scan;
                }
                Phase::Await {
                    id,
                    before,
                    started_tick,
                } => {
                    let view = BankView::now();
                    if !self.same_session(&view) {
                        return Step::Done(());
                    }
                    let (held, current_tick) = observed::with(|scene| {
                        let tick = scene.tick();
                        let held = scene.since_login().inv().map(|inv| ops::count_id(inv, id));
                        (held, tick)
                    });
                    if held.is_some_and(|held| held < before) {
                        self.round += 1;
                        self.phase = Phase::Scan;
                        continue;
                    }
                    if current_tick.is_some_and(|tick| {
                        tick.saturating_sub(started_tick) >= TRANSFER_OBSERVED_TICK_LIMIT
                    }) {
                        return Step::Done(());
                    }
                    return Step::Wait;
                }
            }
        }
    }
}

/// JS truthiness of a callback's settled value.
pub(crate) fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(s) => !s.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// `Bank.depositAllMatching(match, log)` passes the predicate; the keep
/// list (`depositAllExcept`) and `all` (`depositInventory`) need none.
#[derive(Deserialize)]
pub(crate) struct DepositArgs {
    #[serde(default)]
    keep: Option<Vec<String>>,
    #[serde(default)]
    all: bool,
}

const MATCH: usize = 0;
const LOG: usize = 1;

/// One awaited `Bank.depositAllMatching` / `depositAllExcept` /
/// `depositInventory`.
pub(crate) struct BankDeposit(Deposit);

impl Family for BankDeposit {
    const NAME: &'static str = "bank_deposit";
    const CALLBACKS: &'static [&'static str] = &["match", "log"];
    /// Frozen calls `match(...)` inside `items.find` and `log?.()`
    /// without awaiting either: a returned promise is a (truthy) value.
    const SYNC_HOOKS: &'static [usize] = &[MATCH, LOG];
    /// The first predicate calls and the first deposit join the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = DepositArgs;
    type Output = Value;

    fn begin(args: DepositArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let matcher = if args.all {
            Matcher::All
        } else if let Some(keep) = args.keep {
            Matcher::keep(keep)
        } else if cx.has(MATCH) {
            Matcher::Hook {
                hook: MATCH,
                with_id: true,
                common: false,
            }
        } else {
            return Begin::Refuse("requires a function".into());
        };
        // The shim's pending guard: another op in flight deposits nothing.
        if bank_op::busy() {
            return Begin::Done(Value::Null);
        }
        Begin::Run(Self(Deposit::new(matcher, Some(LOG))))
    }

    fn step(&mut self, cx: &mut Cx<'_>) -> Step<Value> {
        match self.0.step(cx) {
            Step::Wait => Step::Wait,
            Step::Call(call) => Step::Call(call),
            Step::Done(()) => Step::Done(Value::Null),
            Step::Fail(thrown) => Step::Fail(thrown),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::callback_v8::HeldCallback;
    use crate::machine::{self, Called, Js, Outcome, Pending, Started, Take};
    use crate::observed::Ops;
    use crate::shim::InteractReq;
    use std::rc::Rc;

    /// A keep-list or sweep deposit calls no script callback.
    struct NoJs;

    impl Js for NoJs {
        fn queue_len(&mut self) -> usize {
            0
        }

        fn call(&mut self, _hook: Option<&HeldCallback>, _args: &[Value]) -> Called {
            panic!("a keep-list deposit calls no script callback");
        }

        fn poll(&mut self, _pending: &Pending) -> Option<Reply> {
            panic!("a keep-list deposit calls no script callback");
        }

        fn claimed(&mut self) -> bool {
            false
        }
    }

    const ALL_OPS: [&str; 5] = [
        "Deposit-1",
        "Deposit-5",
        "Deposit-10",
        "Deposit-All",
        "Deposit-X",
    ];

    fn side_row(name: Option<&str>, id: i32, count: i32, slot: i32) -> ItemRow {
        ItemRow {
            id,
            count,
            name: name.map(Rc::from),
            ops: ALL_OPS.iter().map(|op| Rc::from(*op)).collect::<Ops>(),
            slot: Some(slot),
            component_id: Some(701),
            ..ItemRow::default()
        }
    }

    fn post(tick: u64, side: Vec<ItemRow>) {
        post_views(tick, side.clone(), side);
    }

    /// The open bank in generation 5 with an explicitly independent main and
    /// side observation, including a deliberately stale side fixture.
    fn post_views(tick: u64, main: Vec<ItemRow>, side: Vec<ItemRow>) {
        observed::post(tick, |post| {
            post.session(true)
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(5)
                .side_modal_id(700)
                .inv_size(28)
                .inv(main)
                .bank(Vec::new())
                .bank_side(side);
        });
    }

    fn press(id: i32, slot: i32) -> InteractReq {
        InteractReq::InvButton {
            id,
            slot,
            component: 701,
            operation: 4,
            bank_generation: 5,
        }
    }

    fn step() -> Vec<InteractReq> {
        machine::step(&mut NoJs);
        machine::merge_ops(Vec::new())
    }
    #[test]
    fn stale_side_after_first_fish_selects_a_current_main_candidate() {
        machine::on_reset();
        observed::on_reset();
        let swordfish = side_row(Some("Swordfish"), 371, 13, 0);
        let tuna = side_row(Some("Tuna"), 359, 14, 1);
        let net = side_row(Some("Net"), 303, 1, 2);
        let beer = side_row(Some("Beer"), 1917, 1, 3);
        let harpoon = side_row(Some("Harpoon"), 311, 1, 4);
        let coins = side_row(Some("Coins"), 995, 6_191, 5);
        let side = vec![
            swordfish.clone(),
            tuna.clone(),
            net.clone(),
            beer.clone(),
            harpoon.clone(),
            coins.clone(),
        ];
        post_views(1, side.clone(), side.clone());
        let Started::Running(_handle) = machine::start(
            BankDeposit::NAME,
            json!({"keep": ["Harpoon", "Coins"]}),
            Vec::new(),
            0,
        ) else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(371, 0)]);

        post_views(2, vec![tuna, net, beer, harpoon, coins], side);
        assert_eq!(step(), vec![press(359, 1)]);
    }

    #[test]
    fn stale_last_fish_does_not_hide_current_net_and_beer_targets() {
        machine::on_reset();
        observed::on_reset();
        let swordfish = side_row(Some("Swordfish"), 371, 13, 0);
        let tuna = side_row(Some("Tuna"), 359, 14, 1);
        let net = side_row(Some("Net"), 303, 1, 2);
        let beer = side_row(Some("Beer"), 1917, 1, 3);
        let harpoon = side_row(Some("Harpoon"), 311, 1, 4);
        let coins = side_row(Some("Coins"), 995, 6_191, 5);
        let side = vec![
            swordfish.clone(),
            tuna.clone(),
            net.clone(),
            beer.clone(),
            harpoon.clone(),
            coins.clone(),
        ];
        post_views(1, side.clone(), side.clone());
        let Started::Running(_handle) = machine::start(
            BankDeposit::NAME,
            json!({"keep": ["Harpoon", "Coins"]}),
            Vec::new(),
            0,
        ) else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(371, 0)]);

        post_views(
            2,
            vec![
                tuna.clone(),
                net.clone(),
                beer.clone(),
                harpoon.clone(),
                coins.clone(),
            ],
            side.clone(),
        );
        assert_eq!(step(), vec![press(359, 1)]);

        post_views(3, vec![net, beer, harpoon, coins], side);
        assert_eq!(step(), vec![press(303, 2)]);
    }

    #[test]
    fn stale_side_rows_do_not_delay_completion_with_only_kept_items() {
        machine::on_reset();
        observed::on_reset();
        let harpoon = side_row(Some("Harpoon"), 311, 1, 4);
        let coins = side_row(Some("Coins"), 995, 6_191, 5);
        let kept = vec![harpoon, coins];
        let mut side = vec![
            side_row(Some("Swordfish"), 371, 13, 0),
            side_row(Some("Tuna"), 359, 14, 1),
            side_row(Some("Net"), 303, 1, 2),
            side_row(Some("Beer"), 1917, 1, 3),
        ];
        side.extend(kept.iter().cloned());
        post_views(1, kept, side);
        let Started::Running(handle) = machine::start(
            BankDeposit::NAME,
            json!({"keep": ["Harpoon", "Coins"]}),
            Vec::new(),
            0,
        ) else {
            panic!("expected a running deposit");
        };
        assert!(step().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }

    /// `depositAllExcept(names)` resolves the kept names to every id that
    /// carries them (a noted and an unnoted trout are two ids, both kept)
    /// and presses each other row by id, slot and component: a nameless row
    /// too. A press settles when its id leaves the pack; a posted side with
    /// only kept rows ends the loop at once.
    #[test]
    fn keep_names_resolve_to_ids_and_each_press_is_an_exact_row() {
        machine::on_reset();
        observed::on_reset();
        let coins = side_row(Some("Coins"), 995, 40, 0);
        let trout = side_row(Some("Trout"), 333, 1, 1);
        let noted_trout = side_row(Some("Trout"), 334, 6, 2);
        let bones = side_row(Some("Bones"), 526, 1, 3);
        let nameless = side_row(None, 4242, 1, 4);
        let kept = vec![coins.clone(), trout.clone(), noted_trout.clone()];
        let mut all = kept.clone();
        all.extend([bones, nameless.clone()]);
        post(1, all);
        let Started::Running(handle) = machine::start(
            BankDeposit::NAME,
            json!({ "keep": ["coins", "TROUT"] }),
            Vec::new(),
            0,
        ) else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(526, 3)]);
        // Inside the bound, the press is not settled.
        assert!(step().is_empty());
        let mut rest = kept.clone();
        rest.push(nameless);
        post(2, rest);
        assert_eq!(step(), vec![press(4242, 4)], "a nameless row by id");
        post(3, kept);
        assert!(step().is_empty(), "only kept ids remain");
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }

    /// A side root still down waits the 1.2 s view bound and fails while
    /// candidates remain; an unsettled press ends at the transfer bound, and
    /// a new bank session ends it with no further press.
    #[test]
    fn sweep_waits_for_the_side_root_and_ends_on_an_unsettled_press() {
        machine::on_reset();
        observed::on_reset();
        let bones = side_row(Some("Bones"), 526, 1, 0);
        observed::post(1, |post| {
            post.session(true)
                .bank_open(true)
                .bank_loaded(true)
                .bank_generation(5)
                .side_modal_id(-1)
                .inv_size(28)
                .inv(vec![bones.clone()])
                .bank_side(vec![bones.clone()]);
        });
        let start = || machine::start(BankDeposit::NAME, json!({ "all": true }), Vec::new(), 0);
        let Started::Running(handle) = start() else {
            panic!("expected a running deposit");
        };
        assert!(
            step().is_empty(),
            "a leftover list under a down root is not posted"
        );
        assert_eq!(machine::take(handle), Take::Pending);
        machine::age(handle, DEPOSIT_VIEW_MS);
        assert!(step().is_empty());
        let Take::Settled(Outcome::Failed(thrown)) = machine::take(handle) else {
            panic!("a candidate without a usable side view must fail, not succeed");
        };
        assert_eq!(thrown.message(), "bank deposit view did not post");

        post(2, vec![bones.clone()]);
        let Started::Running(handle) = start() else {
            panic!("expected a running deposit");
        };
        assert_eq!(
            step(),
            vec![press(526, 0)],
            "the fresh sweep emits the exact current row on its first poll"
        );
        assert_eq!(machine::take(handle), Take::Pending);
        for tick in 3..=9 {
            post(tick, vec![bones.clone()]);
            assert!(step().is_empty(), "tick {tick}: no second press");
            assert_eq!(
                machine::take(handle),
                Take::Pending,
                "tick {tick} is within the eight-tick transfer bound"
            );
        }
        post(10, vec![bones.clone()]);
        assert!(step().is_empty(), "the eighth fresh tick expires the press");
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );

        post(3, vec![bones.clone()]);
        let Started::Running(handle) = start() else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(526, 0)]);
        observed::post(4, |post| {
            post.bank_generation(6).inv(Vec::new());
        });
        assert!(step().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }
    #[test]
    fn sweep_transfer_is_pending_after_thirty_seconds_and_settles_on_count_delta() {
        machine::on_reset();
        observed::on_reset();
        let bones = side_row(Some("Bones"), 526, 1, 0);
        post(1, vec![bones.clone()]);
        let Started::Running(handle) =
            machine::start(BankDeposit::NAME, json!({ "all": true }), Vec::new(), 0)
        else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(526, 0)]);
        machine::age(handle, 30_000);

        post(2, vec![bones]);
        assert!(step().is_empty());
        assert_eq!(machine::take(handle), Take::Pending);
        post(3, Vec::new());
        assert!(step().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }

    #[test]
    fn sweep_same_tick_snapshot_repoll_does_not_consume_transfer_ticks() {
        machine::on_reset();
        observed::on_reset();
        let bones = side_row(Some("Bones"), 526, 1, 0);
        post(1, vec![bones.clone()]);
        let Started::Running(handle) =
            machine::start(BankDeposit::NAME, json!({ "all": true }), Vec::new(), 0)
        else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(526, 0)]);
        machine::age(handle, 30_000);

        for sequence in 1..=3 {
            observed::post(1, |_| {});
            machine::snapshot_step(&mut NoJs, 1, sequence);
            assert_eq!(
                machine::take(handle),
                Take::Pending,
                "same-tick snapshot sequence {sequence}"
            );
        }
        post(2, Vec::new());
        assert!(step().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }
    #[test]
    fn sweep_count_delta_on_eighth_tick_precedes_expiration() {
        machine::on_reset();
        observed::on_reset();
        let bones = side_row(Some("Bones"), 526, 1, 0);
        post(1, vec![bones.clone()]);
        let Started::Running(handle) =
            machine::start(BankDeposit::NAME, json!({ "all": true }), Vec::new(), 0)
        else {
            panic!("expected a running deposit");
        };
        assert_eq!(step(), vec![press(526, 0)]);
        for tick in 2..=8 {
            post(tick, vec![bones.clone()]);
            assert!(step().is_empty(), "tick {tick}");
            assert_eq!(machine::take(handle), Take::Pending, "tick {tick}");
        }
        post(9, Vec::new());
        assert!(step().is_empty());
        assert_eq!(
            machine::take(handle),
            Take::Settled(Outcome::Done(Value::Null))
        );
    }
}
