//! The allocation gates of `LUAU-DESIGN-1114/design-r3.md` 8.1 for plan slice
//! L4a: G2 (a suspended tick with no completion) and M1/G3 (a tick that reads
//! one scalar through the lent frame). Each dispatch is measured alone; the
//! seat batch and the tick outcome that the slot drains between two ticks
//! are drained outside the measurement, as the slot does.

pub mod support;

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
fn measured_tick(run: &mut dyn SurfaceRun, tick_number: u64) -> u64 {
    let input = snapshot(tick_number);
    let mut outcome = LentOutcome::Refused;
    let info = allocation_counter::measure(|| {
        outcome = tick(&mut *run, &input);
    });
    assert_eq!(outcome, LentOutcome::Accepted);
    drop(run.drain_seat_batches());
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
        total += measured_tick(&mut *run, number);
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

/// M1 of `design-r3.md` 8.1: configuration S (the safe scoped lend) on the
/// G3 workload. The count is pinned so that a change of it shows. G3 needs
/// zero; this configuration cannot reach it, because the scope of the safe
/// lend allocates its reader on every entry (`design-r3.md` rule B8 item 7).
/// The count is a measurement, not a pass of G3.
///
/// Measured on merlot at L4a: 7 allocations on every tick, plus one on a
/// tick where the VM grows its heap for the scope's garbage (7,022 in 1,000
/// ticks). The VM frees that garbage at its own collection steps.
const M1_ALLOCATIONS_PER_ENTRY: u64 = 7;
const M1_HEAP_GROWTH_PER_ENTRY: u64 = 1;

#[test]
fn m1_scalar_read_tick_allocation_count_in_configuration_s() {
    let mut run = spawn(
        r#"
        local last = 0
        return { tick = function(api)
            last = api.status.tick()
        end }
        "#,
    );
    for number in 1..=WARM_UP {
        measured_tick(&mut *run, number);
    }
    let used_after_warm_up = used_bytes(&*run);
    let mut counts = Vec::new();
    for number in WARM_UP + 1..=WARM_UP + TICKS {
        counts.push(measured_tick(&mut *run, number));
    }
    let used_after = used_bytes(&*run);
    let total = counts.iter().sum::<u64>();
    let minimum = counts.iter().copied().min().unwrap_or(0);
    let maximum = counts.iter().copied().max().unwrap_or(0);
    println!(
        "M1: {TICKS} scalar-read ticks made {total} Rust allocations (min {minimum}, max {maximum} per tick); \
         VM used bytes {used_after_warm_up} after {WARM_UP} ticks, {used_after} after {}",
        WARM_UP + TICKS
    );
    assert!(run.drain_logs().is_empty());
    assert_eq!(
        minimum, M1_ALLOCATIONS_PER_ENTRY,
        "M1 changed; update the pin with the measured value and report it"
    );
    assert!(
        maximum <= M1_ALLOCATIONS_PER_ENTRY + M1_HEAP_GROWTH_PER_ENTRY,
        "M1 changed: {maximum} allocations on one tick"
    );
}
