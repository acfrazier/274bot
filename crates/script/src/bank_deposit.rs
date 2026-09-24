//! Rust-owned deposit loop, frozen `Bank.depositAllMatching`: the
//! `bank_deposit` [`crate::machine`] family, and the [`Deposit`] piece the
//! bank-nearest run reuses.
//!
//! Each round reads the posted backpack side of the bank, waits up to
//! 1.2 s for it when it is empty, asks the matcher row by row in posted
//! order (a script predicate through the callback path, a keep list, or
//! everything) and sends one Deposit-All for the first match, awaiting the
//! host result. No match, a nameless match, a failed deposit or 32 rounds
//! end the loop.

use crate::bank_op::{self, Awaiting, BankView, Op, Sent};
use crate::machine::{Begin, Call, Cx, Family, Reply, Step};
use crate::observed::{self, Text};
use serde::Deserialize;
use serde_json::{json, Value};

/// Frozen wait for the side backpack to post.
pub const DEPOSIT_VIEW_MS: u64 = 1_200;
/// Frozen `for (let guard = 0; guard < 32; guard++)`.
const ROUNDS: u32 = 32;
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
    /// Every row whose lower-cased name is not kept (`depositAllExcept`).
    Keep(Vec<String>),
    /// Every row (`depositInventory`).
    All,
}

enum Phase {
    Scan,
    WaitRows,
    Match {
        rows: Vec<(Option<Text>, i32)>,
        index: usize,
        asked: bool,
    },
    Await(Awaiting),
}

/// One deposit loop.
pub(crate) struct Deposit {
    matcher: Matcher,
    /// The caller's `log` hook, if it passed one.
    log: Option<usize>,
    round: u32,
    phase: Phase,
}

impl Deposit {
    pub(crate) fn new(matcher: Matcher, log: Option<usize>) -> Self {
        Self {
            matcher,
            log,
            round: 0,
            phase: Phase::Scan,
        }
    }

    /// Drive the loop; the reply of a predicate call it asked for is read
    /// here, so a family embedding it routes its steps here while it runs.
    pub(crate) fn step(&mut self, cx: &mut Cx<'_>) -> Step<()> {
        loop {
            match &mut self.phase {
                Phase::Scan => {
                    if self.round >= ROUNDS {
                        return Step::Done(());
                    }
                    let rows = side_rows();
                    if rows.is_empty() && BankView::now().open {
                        self.phase = Phase::WaitRows;
                        cx.clock().arm(DEPOSIT_VIEW_MS);
                        if let Some(hook) = self.log.filter(|hook| cx.has(*hook)) {
                            return Step::Call(Call {
                                hook,
                                args: vec![json!(VIEW_NOT_READY)],
                            });
                        }
                        continue;
                    }
                    self.phase = Phase::Match {
                        rows,
                        index: 0,
                        asked: false,
                    };
                }
                Phase::WaitRows => {
                    if let Some(Reply::Threw(thrown)) = cx.reply() {
                        return Step::Fail(thrown);
                    }
                    let rows = side_rows();
                    if rows.is_empty() && BankView::now().open && !cx.clock().bound_reached() {
                        return Step::Wait;
                    }
                    self.phase = Phase::Match {
                        rows,
                        index: 0,
                        asked: false,
                    };
                }
                Phase::Match { rows, index, asked } => {
                    let Some((name, id)) = rows.get(*index).cloned() else {
                        return Step::Done(());
                    };
                    let hit = match &self.matcher {
                        Matcher::All => true,
                        Matcher::Keep(kept) => {
                            let lower = name.as_deref().unwrap_or_default().to_lowercase();
                            !kept.iter().any(|keep| *keep == lower)
                        }
                        Matcher::Hook {
                            hook,
                            with_id,
                            common,
                        } => {
                            if !*asked {
                                *asked = true;
                                let name = json!(name.as_deref().unwrap_or_default());
                                let args = if *with_id {
                                    vec![name, json!(id)]
                                } else {
                                    vec![name]
                                };
                                return Step::Call(Call { hook: *hook, args });
                            }
                            *asked = false;
                            match cx.reply() {
                                Some(Reply::Threw(thrown)) => return Step::Fail(thrown),
                                Some(Reply::Value(value)) => {
                                    truthy(&value)
                                        || (*common
                                            && api::content::matches_common_bank_loot(
                                                name.as_deref().unwrap_or_default(),
                                                id,
                                            ))
                                }
                                None => false,
                            }
                        }
                    };
                    if !hit {
                        *index += 1;
                        continue;
                    }
                    // The shim stops at a nameless row: it cannot name the deposit.
                    let Some(name) = name else {
                        return Step::Done(());
                    };
                    match (Op::Deposit {
                        name: name.to_string(),
                    })
                    .send(false, cx)
                    {
                        Sent::Awaiting(waiting) => {
                            self.phase = Phase::Await(waiting);
                            return Step::Wait;
                        }
                        Sent::Settled(_) => return Step::Done(()),
                    }
                }
                Phase::Await(waiting) => match waiting.poll(&BankView::now()) {
                    None => return Step::Wait,
                    Some(false) => return Step::Done(()),
                    Some(true) => {
                        self.round += 1;
                        self.phase = Phase::Scan;
                    }
                },
            }
        }
    }
}

fn side_rows() -> Vec<(Option<Text>, i32)> {
    observed::with(|scene| {
        scene
            .since_login()
            .bank_side()
            .map(|rows| rows.iter().map(|row| (row.name.clone(), row.id)).collect())
            .unwrap_or_default()
    })
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
    const AWAIT_CALLBACKS: bool = false;
    /// The first predicate calls and the first deposit join the caller's tick.
    const KICK_ON_START: bool = true;
    type Args = DepositArgs;
    type Output = Value;

    fn begin(args: DepositArgs, cx: &mut Cx<'_>) -> Begin<Self> {
        let matcher = if args.all {
            Matcher::All
        } else if let Some(keep) = args.keep {
            Matcher::Keep(keep.iter().map(|name| name.to_lowercase()).collect())
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
