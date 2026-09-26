//! Live proof gate shared by every scenario watch: the headed panel
//! (`catalog_watch`, `pair_watch`, `external_watch`, ordinary panel-play)
//! and headless `tui-play`. It owns which shared witness a run qualifies
//! under, the per-poll core-gate decisions, and what holds a scenario PASS:
//! a core gate that is still Pending first, then the bounded clean-stop
//! grace. Proof infrastructure, not gameplay: the front ends keep their own
//! captures, proof-line routing and exit codes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::catalog_core::{CoreCase, CoreWatch, CoreWatchStatus};
use crate::external_loader::{ExternalWatch, ExternalWatchStatus};
use crate::paired_core::{PairCase, PairWatch, PairWatchStatus, DUEL_WEAPON_ALIAS};
use crate::Play;

/// After the scenario PASS and every configured core gate, a card that names
/// a clean stop has this long to reach Idle with its stop reason.
pub const SCRIPT_STOP_WAIT: Duration = Duration::from_secs(45);

/// Proof-line tags printed next to the `PASS: live` / `FAIL: live` receipt:
/// `<TAG>: <live name> <compact witness JSON>`.
pub const CATALOG_CORE_TAG: &str = "CATALOG_CORE";
pub const PAIRED_CORE_TAG: &str = "PAIRED_CORE";
pub const EXTERNAL_LOADER_TAG: &str = "EXTERNAL_LOADER";

/// The shared witness one live scenario run qualifies under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveCore {
    /// Scenario proof and clean stop only.
    Off,
    /// One driven slot under the catalog core witness.
    Catalog(CoreCase),
    /// Two driven slots under the paired full-cycle witness.
    Pair(PairCase),
}

impl LiveCore {
    /// Resolve the requested gates for `scenario` driving `slots` profiles.
    /// A paired proof refuses to run without the pair gate: its scenario
    /// proof holds at Start, so only the pair witness can qualify it.
    pub fn resolve(
        scenario: &str,
        slots: usize,
        catalog_core: bool,
        pair_core: bool,
    ) -> Result<Self, String> {
        if catalog_core && pair_core {
            return Err("catalog core and pair core watches are mutually exclusive".into());
        }
        if catalog_core {
            if slots != 1 {
                return Err("catalog core watch supports exactly one driven slot".into());
            }
            return CoreCase::parse(scenario).map(Self::Catalog);
        }
        if pair_core {
            if slots != 2 {
                return Err("pair core watch supports exactly two driven slots".into());
            }
            return PairCase::parse(scenario).map(Self::Pair);
        }
        if PairCase::parse(scenario).is_ok() {
            return Err(format!(
                "{scenario} is a paired proof: its scenario proof already holds at Start, so it \
                 only qualifies under the pair core gate (pair_watch, or tui-play --pair-core)"
            ));
        }
        Ok(Self::Off)
    }

    /// Arm the Play-owned watch for the run's driven accounts, before either
    /// slot publishes its pre-Start observations.
    pub fn configure(self, play: &Play, names: &[String]) -> Result<(), String> {
        match self {
            Self::Off => Ok(()),
            Self::Catalog(case) => {
                let account = names
                    .first()
                    .ok_or("catalog core watch needs its driven account")?;
                play.catalog_core_watch().configure(case, account.clone());
                Ok(())
            }
            Self::Pair(case) => {
                let [a, b] = names else {
                    return Err("pair core watch supports exactly two driven slots".into());
                };
                let watch = play.paired_core_watch();
                watch.configure(case, a.clone(), b.clone());
                if case == PairCase::Duel {
                    let weapon = play
                        .game_data()
                        .and_then(|data| data.item_by_alias(DUEL_WEAPON_ALIAS).map(|item| item.id))
                        .ok_or_else(|| format!("selected cache has no {DUEL_WEAPON_ALIAS}"))?;
                    watch.install_duel_weapon(weapon)?;
                }
                Ok(())
            }
        }
    }
}

/// Wall-clock ceiling for a run whose scenario PASS may arrive before its
/// core witness qualifies: `BUDGET_S` when set, else the scenario deadline.
/// `None` unless a catalog or pair watch is configured.
pub fn core_deadline(
    catalog: Option<&CoreWatch>,
    pair: Option<&PairWatch>,
    budget: Option<Duration>,
    scenario_deadline: Duration,
    now: Instant,
) -> Option<Instant> {
    let configured =
        catalog.is_some_and(CoreWatch::configured) || pair.is_some_and(PairWatch::configured);
    configured.then(|| now + budget.unwrap_or(scenario_deadline))
}

/// One shared witness's verdict on this poll.
#[derive(Debug, PartialEq)]
pub enum CoreGate {
    /// No watch is configured for this run.
    Disabled,
    /// The witness has not qualified yet and its deadline has not lapsed.
    Pending,
    /// Qualified; the receipt when this poll built it at the deadline.
    Qualified(Option<Arc<Value>>),
    Failed(String),
}

pub fn catalog_core_gate(
    watch: Option<&CoreWatch>,
    deadline: Option<Instant>,
    now: Instant,
) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    match watch.status() {
        CoreWatchStatus::Disabled => CoreGate::Disabled,
        CoreWatchStatus::Qualified => CoreGate::Qualified(None),
        CoreWatchStatus::Failed => CoreGate::Failed(
            watch
                .failure()
                .unwrap_or_else(|| "catalog core failed".into()),
        ),
        CoreWatchStatus::Ready | CoreWatchStatus::Running
            if deadline.is_some_and(|deadline| now >= deadline) =>
        {
            match watch.qualify() {
                Ok(evidence) => CoreGate::Qualified(Some(evidence)),
                Err(error) => CoreGate::Failed(format!(
                    "catalog core did not qualify before the headed deadline: {error}"
                )),
            }
        }
        CoreWatchStatus::Ready | CoreWatchStatus::Running => CoreGate::Pending,
    }
}

pub fn pair_core_gate(
    watch: Option<&PairWatch>,
    deadline: Option<Instant>,
    now: Instant,
) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    match watch.status() {
        PairWatchStatus::Disabled => CoreGate::Disabled,
        PairWatchStatus::Qualified => CoreGate::Qualified(None),
        PairWatchStatus::Failed => {
            CoreGate::Failed(watch.failure().unwrap_or_else(|| "pair core failed".into()))
        }
        PairWatchStatus::Ready | PairWatchStatus::Running
            if deadline.is_some_and(|deadline| now >= deadline) =>
        {
            match watch.qualify() {
                Ok(evidence) => CoreGate::Qualified(Some(evidence)),
                Err(error) => CoreGate::Failed(format!(
                    "pair core did not qualify before the headed deadline: {error}"
                )),
            }
        }
        PairWatchStatus::Ready | PairWatchStatus::Running => CoreGate::Pending,
    }
}

pub fn external_core_gate(watch: Option<&ExternalWatch>) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    match watch.status() {
        ExternalWatchStatus::Disabled => CoreGate::Disabled,
        ExternalWatchStatus::Qualified => CoreGate::Qualified(None),
        ExternalWatchStatus::Failed => CoreGate::Failed(
            watch
                .failure()
                .unwrap_or_else(|| "external loader failed".into()),
        ),
        // Inner 180s/10s live on the watch. BUDGET_S must not replace them.
        ExternalWatchStatus::Ready | ExternalWatchStatus::Running => CoreGate::Pending,
    }
}

/// What holds the scenario runner's PASS on this poll.
#[derive(Debug, PartialEq)]
pub enum PassHold {
    /// A configured core gate is still Pending; the clean-stop grace has
    /// not started.
    Core,
    /// Inside the clean-stop grace, waiting for Idle and the stop reason.
    CleanStop,
    /// The grace lapsed without the clean stop.
    TimedOut(String),
    /// Nothing holds the PASS. `clean_stop` is set when the scenario named a
    /// stop reason and the card reached it.
    Release { clean_stop: bool },
}

/// Decide the PASS hold. `grace_started` is the run's clean-stop clock: it
/// starts on the first poll that finds every gate settled and the stop
/// missing, never while a gate is Pending.
pub fn pass_hold(
    gates: &[&CoreGate],
    wait_script_stop: Option<&str>,
    stopped: impl FnOnce(&str) -> bool,
    grace_started: &mut Option<Instant>,
    now: Instant,
) -> PassHold {
    if gates.iter().any(|gate| matches!(gate, CoreGate::Pending)) {
        return PassHold::Core;
    }
    let Some(needle) = wait_script_stop else {
        return PassHold::Release { clean_stop: false };
    };
    if stopped(needle) {
        return PassHold::Release { clean_stop: true };
    }
    let started = *grace_started.get_or_insert(now);
    if now.saturating_duration_since(started) >= SCRIPT_STOP_WAIT {
        PassHold::TimedOut(format!(
            "timed out waiting for script Idle and clean stop reason {needle:?}"
        ))
    } else {
        PassHold::CleanStop
    }
}

/// The card is Idle and its lifecycle receipt names `needle`. Not a
/// game-chat read.
pub fn script_self_stop_observed(
    state: script::RunState,
    receipt: Option<&script::ScriptLifecycleReceipt>,
    needle: &str,
) -> bool {
    matches!(state, script::RunState::Idle)
        && receipt.is_some_and(|receipt| receipt.reason.contains(needle))
}

/// The read side the three proof watches share.
trait ProofWatch {
    fn configured(&self) -> bool;
    fn qualify(&self) -> Result<Arc<Value>, String>;
    fn evidence(&self) -> Value;
}

macro_rules! proof_watch {
    ($($watch:ty),*) => {$(
        impl ProofWatch for $watch {
            fn configured(&self) -> bool {
                <$watch>::configured(self)
            }
            fn qualify(&self) -> Result<Arc<Value>, String> {
                <$watch>::qualify(self)
            }
            fn evidence(&self) -> Value {
                <$watch>::evidence(self)
            }
        }
    )*};
}

proof_watch!(CoreWatch, PairWatch, ExternalWatch);

/// Compact witness for a terminal decision, built off the gameplay
/// observation thread: the receipt this poll qualified, else a fresh
/// qualification, else the bounded failure/timeout evidence. `None` unless
/// the watch is configured.
fn record<W: ProofWatch>(watch: Option<&W>, gate: &CoreGate) -> Option<String> {
    let watch = watch.filter(|watch| watch.configured())?;
    Some(match gate {
        CoreGate::Qualified(Some(evidence)) => evidence.to_string(),
        _ => watch
            .qualify()
            .map(|evidence| evidence.to_string())
            .unwrap_or_else(|_| watch.evidence().to_string()),
    })
}

pub fn catalog_core_record(watch: Option<&CoreWatch>, gate: &CoreGate) -> Option<String> {
    record(watch, gate)
}

pub fn pair_core_record(watch: Option<&PairWatch>, gate: &CoreGate) -> Option<String> {
    record(watch, gate)
}

pub fn external_core_record(watch: Option<&ExternalWatch>, gate: &CoreGate) -> Option<String> {
    record(watch, gate)
}

#[cfg(test)]
#[path = "live_gate_tests.rs"]
mod tests;
