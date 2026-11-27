#![cfg(feature = "luau")]
//! Host-side Luau memory gates of slice L7 (ADR 0023 item 32, Promotion
//! Condition 2).
//!
//! The host keeps one `SlotScript` for each bot, with the Luau factory bound
//! (`script_runtime.rs`, `script_slot_or_insert`). These tests run the stated
//! workload of `script-luau` (`tests/support/l7_workload.luau`) in ten such
//! slots and bound the memory that each bot adds. They also prove that the
//! Luau script of the release memory A/B asks the native Gatherer for the same
//! settings as its feature-off control.

#[path = "../../script-luau/tests/support/mod.rs"]
pub mod support;
#[path = "../../script-luau/tests/support/workload.rs"]
pub mod workload;

use std::collections::BTreeSet;
use std::sync::Arc;

use script::native::seat_rows::SeatStartArgs;
use script::native::SettingsBag;
use script::surface::{LentOutcome, SurfaceRun};
use script::{SlotScript, StartOutcome, StartPoll};
use script_luau::LuauSurfaceFactory;
use workload::{used_bytes, Driver, TICKS_PER_TRIP};

const BOTS: usize = 10;
const TRIPS: u64 = 20;

/// The measured value plus the heap-total headroom of the project memory
/// rules: 5 %, rounded up to 8 bytes.
const fn with_heap_headroom(measured: u64) -> u64 {
    (measured + measured.div_ceil(20)).div_ceil(8) * 8
}

// Measured on 2026-11-26 in macOS arm64 and Linux x86_64 reference runs,
// debug test builds, 10 runs each; the L7 report has the runs. Each
// value is the highest of the 20 runs. The VM values depend on the clock of
// the Luau collector, so they differ from run to run.
/// Rust live bytes per bot after 400 ticks, the VM heap included (1,027,219
/// to 1,118,811).
const BOT_RUST_LIVE_MEASURED: u64 = 1_118_811;
/// The VM used bytes of the largest bot after 400 ticks (1,054,712 to
/// 1,167,288).
const BOT_VM_USED_MEASURED: u64 = 1_167_288;
/// The bytes that ten stopped Luau runs leave in their slots (Linux 5,432 to
/// 5,440, macOS 6,872 to 6,878).
const STOPPED_KEPT_MEASURED: u64 = 6_878;

fn started_slot(factory: &Arc<LuauSurfaceFactory>) -> SlotScript {
    let mut slot = SlotScript::new();
    slot.bind_surface_factory(Arc::clone(factory) as _);
    slot.start_surface(support::start_with_game_data(
        workload::SOURCE,
        Some(workload::settings()),
        None,
    ))
    .expect("Luau Start");
    assert_eq!(slot.poll_start(), StartPoll::Settled(StartOutcome::Ready));
    slot.on_is_up(true);
    slot
}

fn run_of(slot: &mut SlotScript, step: impl FnOnce(&mut dyn SurfaceRun)) {
    slot.with_surface_run_for_test(step)
        .expect("the slot holds its Luau run");
}

#[test]
fn ten_luau_bots_stay_within_the_per_bot_memory_bound() {
    // The host shares one factory between its slots. Binding it to an idle
    // slot allocates nothing and makes no VM.
    let factory = Arc::new(LuauSurfaceFactory);
    let mut idle = SlotScript::new();
    let bind = allocation_counter::measure(|| {
        idle.bind_surface_factory(Arc::clone(&factory) as _);
    });
    assert_eq!(bind.count_total, 0, "binding the factory allocated");
    assert!(idle.memory_metrics().is_none(), "an idle slot holds a VM");

    let resident_before = host_play::current_resident_bytes();
    let mut slots = Vec::with_capacity(BOTS);
    let mut drivers: Vec<Driver> = (0..BOTS).map(|_| Driver::new()).collect();
    let running = allocation_counter::measure(|| {
        for _ in 0..BOTS {
            slots.push(started_slot(&factory));
        }
        for _ in 0..TRIPS * TICKS_PER_TRIP {
            for (slot, driver) in slots.iter_mut().zip(&mut drivers) {
                run_of(slot, |run| driver.step(run));
            }
        }
    });
    let resident_after = host_play::current_resident_bytes();
    let mut vm_used = Vec::with_capacity(BOTS);
    for slot in &mut slots {
        run_of(slot, |run| vm_used.push(used_bytes(run)));
    }
    for driver in &drivers {
        assert_eq!(driver.answered.fights, 4 * TRIPS, "each bot fought");
    }

    let stopped = allocation_counter::measure(|| {
        for slot in &mut slots {
            slot.stop();
            assert!(slot.memory_metrics().is_none(), "Stop dropped the VM");
            drop(slot.take_pending_logs());
        }
    });
    drop(slots);

    let live = u64::try_from(running.bytes_current).expect("running bots hold bytes");
    let per_bot = live / BOTS as u64;
    let vm_max = vm_used.iter().copied().max().expect("ten bots");
    let resident_per_bot = resident_before
        .zip(resident_after)
        .map(|(before, after)| after.saturating_sub(before) / BOTS as u64);
    println!(
        "L7_HOST {}",
        serde_json::json!({
            "bots": BOTS,
            "ticks_per_bot": TRIPS * TICKS_PER_TRIP,
            "rust_live_per_bot": per_bot,
            "rust_peak_all_bots": running.bytes_max,
            "vm_used_per_bot_max": vm_max,
            "vm_used_per_bot": vm_used,
            "resident_before": resident_before,
            "resident_after": resident_after,
            "resident_per_bot": resident_per_bot,
            "rust_live_change_in_stop": stopped.bytes_current,
        })
    );

    let rust_limit = with_heap_headroom(BOT_RUST_LIVE_MEASURED);
    assert!(
        per_bot <= rust_limit,
        "Rust live bytes per Luau bot {per_bot} > {rust_limit}"
    );
    let vm_limit = with_heap_headroom(BOT_VM_USED_MEASURED);
    assert!(
        vm_max <= vm_limit,
        "VM used bytes of a Luau bot {vm_max} > {vm_limit}"
    );
    // Stop returns the VM of each bot. What stays is the slot's own record of
    // each stopped run.
    let kept = u64::try_from(live as i64 + stopped.bytes_current).unwrap_or(0);
    let kept_limit = with_heap_headroom(STOPPED_KEPT_MEASURED);
    assert!(
        kept <= kept_limit,
        "ten stopped bots kept {kept} bytes > {kept_limit}"
    );
}

const MEMORY_LUAU_GATHERER: &str = include_str!("../src/memory_luau_gatherer.luau");

/// The `luau_gatherer` arm of the release memory A/B must run the same
/// native Gatherer as its feature-off control: the session settings that the
/// Luau script queues, filled with the card defaults, decode to the same
/// settings as the bag that the control starts the card with.
#[test]
fn memory_luau_gatherer_asks_for_the_settings_of_its_native_control() {
    let scenario = scenario::get("gatherer").expect("gatherer scenario");
    let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject)
        .expect("the gatherer scenario injects settings");
    let schema = (script::gatherer::CARD.schema)();
    let control: SettingsBag =
        script::settings_store::merge_bag(schema, &serde_json::Map::new(), Some(&inject));

    let mut run = support::spawn_with(MEMORY_LUAU_GATHERER, Some(control.clone()))
        .expect("the memory Luau script starts");
    assert_eq!(
        support::tick(&mut *run, &support::snapshot(1)),
        LentOutcome::Accepted
    );
    let batches = run.drain_seat_batches();
    let starts: Vec<_> = batches
        .iter()
        .flat_map(|batch| &batch.batch.starts)
        .collect();
    assert_eq!(starts.len(), 1, "one Gatherer session");
    let SeatStartArgs::Gather(asked) = &starts[0].args else {
        panic!("the script starts a Gather session: {:?}", starts[0].args);
    };
    assert_eq!(
        asked.keys().collect::<BTreeSet<_>>(),
        inject.keys().collect::<BTreeSet<_>>(),
        "the script sets exactly the injected settings"
    );
    let luau = script::settings_store::merge_bag(schema, asked, None);
    assert_eq!(
        script::gatherer::GathererSettings::from_bag(&luau).expect("Luau session settings"),
        script::gatherer::GathererSettings::from_bag(&control).expect("control settings"),
    );
    assert!(run.paint().is_some(), "the overlay painted");
}
