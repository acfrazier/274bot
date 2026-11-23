//! Allocation gates for slices L4a and L4b (design-r3.md 8.1).
//! G2 and G3 use the accepted configuration-S limit from decision D22(a).
//! Each dispatch is measured alone. The slot drains batches and tick outcomes
//! outside the measurement.

pub mod support;

use std::collections::BTreeMap;

use script::bank::request::BankSeatOperation;
use script::interact::InteractReq;
use script::native::request::NativeKind;
use script::native::seat_rows::SeatStartArgs;
use script::surface::{LentOutcome, SurfaceRun};
use support::{snapshot, spawn, tick};

const TICKS: u64 = 1_000;
const WARM_UP: u64 = 10;

fn used_bytes(run: &dyn SurfaceRun) -> u64 {
    run.memory_metrics().expect("live VM")["used_bytes"]
        .as_u64()
        .expect("used bytes")
}

/// One measured game tick, then the slot's own drains.
fn measured_tick(
    run: &mut dyn SurfaceRun,
    tick_number: u64,
    expected_interactions: Option<usize>,
) -> u64 {
    let input = snapshot(tick_number);
    let mut outcome = LentOutcome::Refused;
    let info = allocation_counter::measure(|| {
        outcome = tick(&mut *run, &input);
    });
    assert_eq!(outcome, LentOutcome::Accepted);
    let batches = run.drain_seat_batches();
    if let Some(expected) = expected_interactions {
        let interactions = batches
            .iter()
            .flat_map(|batch| batch.batch.reqs.iter())
            .filter(|row| !matches!(&row.request, InteractReq::LoopSettled))
            .count();
        assert_eq!(
            interactions, expected,
            "the measured tick queues every interaction request"
        );
    }
    drop(batches);
    drop(run.drain_tick_outcomes());
    info.count_total
}

#[test]
fn g2_suspended_tick_without_completion_allocates_nothing_and_stays_out_of_the_vm() {
    let mut run = spawn(
        r#"
        return { tick = function(api)
            api.walk.to(3200, 3201, 0)
            api.log("resumed")
        end }
        "#,
    );
    assert_eq!(tick(&mut *run, &snapshot(1)), LentOutcome::Accepted);
    assert!(run.live_walking_operation(), "the walk is awaited");
    drop(run.drain_seat_batches());
    let (entries_before, _) = run.execution_sequence();
    let mut total = 0;
    for number in 2..2 + TICKS {
        total += measured_tick(&mut *run, number, None);
    }
    let (entries_after, _) = run.execution_sequence();
    println!("G2: {TICKS} suspended ticks made {total} Rust allocations");
    assert_eq!(total, 0, "G2: a suspended tick allocates");
    assert_eq!(
        entries_after, entries_before,
        "G2: a suspended tick entered the VM"
    );
    assert!(run.drain_logs().is_empty());
}

/// The VM-entering ticks of one workload after its warm-up.
struct Profile {
    counts: Vec<u64>,
    /// The used-bytes change of each tick: a rise is the tick's new VM
    /// objects, a fall is a collection step of the VM.
    changes: Vec<i64>,
    used_after_warm_up: u64,
    used_after: u64,
    used_peak: u64,
}

fn profile(source: &str, ticks: u64, expected_interactions: Option<usize>) -> Profile {
    let mut run = spawn(source);
    for number in 1..=WARM_UP {
        measured_tick(&mut *run, number, expected_interactions);
    }
    let used_after_warm_up = used_bytes(&*run);
    let mut used = used_after_warm_up;
    let mut used_peak = used;
    let mut counts = Vec::new();
    let mut changes = Vec::new();
    for number in WARM_UP + 1..=WARM_UP + ticks {
        counts.push(measured_tick(
            &mut *run,
            number,
            expected_interactions,
        ));
        let now = used_bytes(&*run);
        changes.push(now as i64 - used as i64);
        used = now;
        used_peak = used_peak.max(now);
    }
    assert!(run.drain_logs().is_empty());
    Profile {
        counts,
        changes,
        used_after_warm_up,
        used_after: used,
        used_peak,
    }
}

impl Profile {
    fn minimum(&self) -> u64 {
        self.counts.iter().copied().min().unwrap_or(0)
    }

    fn maximum(&self) -> u64 {
        self.counts.iter().copied().max().unwrap_or(0)
    }

    fn peak_growth(&self) -> u64 {
        self.used_peak - self.used_after_warm_up
    }

    fn print(&self, name: &str) {
        let total = self.counts.iter().sum::<u64>();
        let mut changes = BTreeMap::new();
        for change in &self.changes {
            *changes.entry(*change).or_insert(0) += 1;
        }
        let gross_rise = self
            .changes
            .iter()
            .filter(|change| **change > 0)
            .sum::<i64>();
        println!(
            "{name}: {} ticks made {total} Rust allocations (min {}, max {} per tick); \
             VM used bytes {} after warm-up, {} at the end, peak {} (+{}); \
             gross rise {gross_rise} bytes ({} per tick); changes per tick {changes:?}",
            self.counts.len(),
            self.minimum(),
            self.maximum(),
            self.used_after_warm_up,
            self.used_after,
            self.used_peak,
            self.peak_growth(),
            gross_rise / self.changes.len().max(1) as i64,
        );
    }
}

const SCALAR_READ: &str = r#"
    local last = 0
    return { tick = function(api)
        last = api.status.tick()
    end }
"#;

const EMPTY_TICK: &str = r#"
    return { tick = function(api) end }
"#;

/// G3 of `design-r3.md` 8.1, held as the documented limit of operator
/// decision D22(a) (2026-11-21). G3 as designed is zero Rust allocations and
/// unchanged VM used bytes after ten ticks. Configuration S, the safe scoped
/// lend, cannot reach it: each VM-entering dispatch opens one `Lua::scope`
/// with one scoped reader, and mlua 0.12.2 boxes that callback, registers
/// its destructor and tears both down through vectors
/// (`mlua-0.12.2/src/scope.rs:50-62,256-271,299-314`). The accepted limit:
///
/// 1. At most 8 Rust allocations on each VM-entering tick (measured: 7 on
///    each tick, 8 on a tick where the VM takes a new heap page).
/// 2. VM used bytes stay below the warm-up level plus 16 small Luau pages
///    (`16 * 16_360` bytes; `kSmallPageSize`, `luau/VM/src/lmem.cpp:147`)
///    over 10,000 ticks. Why: the scoped reader that each entry creates is
///    VM garbage when the scope ends. Luau takes heap in whole pages and its
///    incremental collector frees that garbage at its own steps, so used
///    bytes rise and fall by pages but have no per-entry trend. The gross
///    rise was 328 to 375 bytes per entry (measured at L4a); an entry that kept
///    even 27 bytes alive would pass the bound within 10,000 ticks. The
///    empty-tick control makes the same Rust allocations and the same page
///    changes, so the scope causes both, not the scalar read.
///
/// The follow-up slice "configuration E" (`design-r3.md` 8.1 case 2, an
/// `unsafe` frame holder) removes the limit before 0.3.0 ships. That slice
/// first tries a safe dependency change: an mlua scope teardown without the
/// temporary vectors may remove some of the allocations (review of L4a).
const G3_ACCEPTED_RUST_ALLOCATIONS_PER_TICK: u64 = 8;
const LUAU_SMALL_PAGE_BYTES: u64 = 16 * 1024 - 24;
const G3_ACCEPTED_VM_GROWTH_BYTES: u64 = 16 * LUAU_SMALL_PAGE_BYTES;
const LONG_TICKS: u64 = 10 * TICKS;

#[test]
fn g3_scalar_read_tick_stays_within_the_accepted_limit() {
    let read = profile(SCALAR_READ, TICKS, None);
    read.print("G3 scalar read");
    let empty = profile(EMPTY_TICK, TICKS, None);
    empty.print("G3 empty tick (control: the scope alone)");
    let long = profile(SCALAR_READ, LONG_TICKS, None);
    long.print("G3 scalar read, long");
    println!(
        "G3 accepted limit: at most {G3_ACCEPTED_RUST_ALLOCATIONS_PER_TICK} Rust allocations per tick; \
         VM used bytes at most {G3_ACCEPTED_VM_GROWTH_BYTES} above the warm-up level"
    );

    for (name, profile) in [("read", &read), ("empty", &empty), ("long", &long)] {
        assert!(
            profile.maximum() <= G3_ACCEPTED_RUST_ALLOCATIONS_PER_TICK,
            "G3 limit: a {name} tick made {} Rust allocations",
            profile.maximum()
        );
        assert!(
            profile.peak_growth() <= G3_ACCEPTED_VM_GROWTH_BYTES,
            "G3 limit: {name} VM used bytes rose {} above the warm-up level",
            profile.peak_growth()
        );
    }
    assert_eq!(
        read.minimum(),
        empty.minimum(),
        "the scalar read itself makes a Rust allocation"
    );
}

const L4B_INTERACTIONS: &str = r#"
    return { tick = function(api)
        api.interact.npc("Banker", "Bank", 1)
        api.interact.loc(3093, 3243, 0, "Use-quickly", 2214)
        api.interact.obj(3093, 3243, 0, "Take", 995)
        api.interact.player("alice", "Trade")
        api.interact.held("Bronze axe", "Wield", 0, 1351)
    end }
"#;

fn bank_start_allocations(member: &str, expected: BankSeatOperation) -> u64 {
    let source = format!("return {{ tick=function(api) api.bank.{member}() end }}");
    let mut run = spawn(&source);
    let input = snapshot(1);
    let mut outcome = LentOutcome::Refused;
    let info = allocation_counter::measure(|| {
        outcome = tick(&mut *run, &input);
    });
    assert_eq!(outcome, LentOutcome::Accepted);
    let batches = run.drain_seat_batches();
    assert_eq!(batches.len(), 1);
    let starts = &batches[0].batch.starts;
    assert_eq!(starts.len(), 1);
    assert_eq!(starts[0].kind, NativeKind::BankPlan);
    let SeatStartArgs::Bank(args) = &starts[0].args else {
        panic!("bank binding starts typed bank arguments");
    };
    args.validate(starts[0].kind)
        .expect("bank arguments validate");
    assert_eq!(args.operation(), expected);
    drop(run.drain_tick_outcomes());
    assert!(run.drain_logs().is_empty());
    info.count_total
}

#[test]
fn l4b_bank_and_interaction_ticks_stay_within_the_allocation_budget() {
    let interactions = profile(L4B_INTERACTIONS, TICKS, Some(5));
    interactions.print("L4b five interaction requests");
    // Keep the 10-allocation cushion; these five rows own eight text strings.
    let interaction_limit = G3_ACCEPTED_RUST_ALLOCATIONS_PER_TICK + 10 + 8;
    assert!(
        interactions.maximum() <= interaction_limit,
        "L4b interaction tick made {} Rust allocations, limit {interaction_limit}",
        interactions.maximum()
    );
    assert!(
        interactions.peak_growth() <= G3_ACCEPTED_VM_GROWTH_BYTES,
        "L4b interaction VM grew {} bytes above warm-up",
        interactions.peak_growth()
    );

    for (member, operation) in [
        ("openNearest", BankSeatOperation::OpenNearestWorld),
        ("depositAll", BankSeatOperation::DepositAll),
        ("close", BankSeatOperation::Close),
    ] {
        let allocations = bank_start_allocations(member, operation);
        let limit = G3_ACCEPTED_RUST_ALLOCATIONS_PER_TICK + 2;
        println!("L4b api.bank.{member}: {allocations} Rust allocations (limit {limit})");
        assert!(
            allocations <= limit,
            "L4b api.bank.{member} made {allocations} Rust allocations, limit {limit}"
        );
    }
}
