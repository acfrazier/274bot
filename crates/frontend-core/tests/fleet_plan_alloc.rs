//! Own test binary: what a waiting fleet plan costs. A 100-target plan
//! and a 100-target settings copy (its prepared scope with the copied
//! values) hold one bounded snapshot each, process-wide, and give it back
//! when cancelled; nothing is kept per bot beside the command record. A
//! 100-bot world switch keeps one record per bot only while it runs, and
//! frames while its relogs wait allocate nothing new.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::path::PathBuf;

use frontend_core::command::OperatorCommand;
use frontend_core::launch::{InstancePermit, PlayConfig};
use frontend_core::scripts::{CompiledCardId, ScriptChoice};
use frontend_core::{FleetAction, FleetCard, HeadlessDriver, MarkOp, Targets};
use vault::{Profile, ProfileSettings, Vault};

struct Counting;

thread_local! {
    static LIVE: Cell<isize> = const { Cell::new(0) };
}

fn note(delta: isize) {
    let _ = LIVE.try_with(|live| live.set(live.get() + delta));
}

// SAFETY: every method forwards its arguments unchanged to `System`, which
// upholds the `GlobalAlloc` contract; the counter uses a non-allocating TLS
// cell.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note(layout.size() as isize);
        // SAFETY: same layout the caller passed.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note(new_size as isize - layout.size() as isize);
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note(-(layout.size() as isize));
        // SAFETY: `ptr` and `layout` come from a prior allocation of ours.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn live() -> isize {
    LIVE.with(Cell::get)
}

const PASSPHRASE: &str = "test-passphrase-01";
const BOTS: usize = 100;

fn scratch(label: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("274bot-fleet-plan-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("test directory");
    dir
}

/// A driver over `BOTS` loaded profiles, every one marked.
fn driver(label: &str) -> HeadlessDriver {
    let dir = scratch(label);
    // Hermetic: no world pack from the operator's home (see memory_notice.rs).
    std::env::set_var("NAV_PACK", dir.join("empty.navpack"));
    let vault_path = dir.join("vault");
    let mut vault = Vault::create(&vault_path, PASSPHRASE).expect("vault");
    let names: Vec<String> = (0..BOTS).map(|index| format!("bot{index:04}")).collect();
    for (index, name) in names.iter().enumerate() {
        vault
            .upsert(Profile {
                username: name.clone(),
                password: "pw".into(),
                uid: 1_000 + index as i32,
                settings: ProfileSettings {
                    auto_login: false,
                    ..ProfileSettings::default()
                },
            })
            .expect("profile");
    }
    drop(vault);
    let mut d = HeadlessDriver::open(&dir.join("state"), InstancePermit::skip());
    d.session_mut().set_spawn_workers(false);
    d.unlock(
        vault_path,
        PASSPHRASE,
        &PlayConfig::direct("127.0.0.1", 43594, "/tmp", true),
    )
    .expect("unlock");
    d.submit(OperatorCommand::Load, Targets::Profiles(&names))
        .expect("load");
    d.poll();
    d.submit(
        OperatorCommand::MarkFleet(MarkOp::Shown),
        Targets::Application,
    )
    .expect("mark");
    d
}

fn settle_writes(d: &mut HeadlessDriver) {
    for _ in 0..50 {
        d.session_mut().flush_writes();
        d.poll();
    }
}

/// Measured 2026-10-21: 7,628 bytes for a 100-target plan (76 per target)
/// and 27,528 for a 100-target copy with its prepared scope (278 per
/// target); Cancel gives back all but the two command records (about
/// 1.5 KiB). The ceilings keep twice that headroom and fail on any second
/// per-bot copy.
#[test]
fn a_100_target_plan_and_copy_hold_one_bounded_snapshot() {
    const PLAN_CEILING: isize = 16 * 1024;
    const COPY_CEILING: isize = 56 * 1024;
    let mut d = driver("bounded");
    let gatherer = ScriptChoice::compiled(CompiledCardId::lookup("Gatherer").expect("Gatherer"));
    d.models_mut()
        .fleet_mut()
        .stage_card(FleetCard::pinned(gatherer));
    d.submit(OperatorCommand::SetFleetCard, Targets::Application)
        .expect("card");
    // Every bot takes the card, so the copy covers all of them.
    d.submit(
        OperatorCommand::PlanFleetAction(FleetAction::Assign),
        Targets::Application,
    )
    .expect("plan assign");
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan).expect("assign");
    settle_writes(&mut d);
    d.submit(OperatorCommand::Select, Targets::Profile("bot0000"))
        .expect("select");
    d.poll();

    // A waiting 100-target Log in plan (the bots are loaded, not logged in).
    let before = live();
    d.submit(
        OperatorCommand::PlanFleetAction(FleetAction::Login),
        Targets::Application,
    )
    .expect("plan");
    let plan_bytes = live() - before;
    assert_eq!(
        d.session()
            .fleet_plan()
            .expect("plan")
            .target_names()
            .count(),
        BOTS
    );
    d.submit(OperatorCommand::CancelFleetPlan, Targets::Application)
        .expect("cancel");
    let after_cancel = live() - before;

    // A waiting 100-target settings copy: the plan plus its prepared scope
    // (source, frozen values, receivers) until Confirm.
    let before = live();
    d.submit(
        OperatorCommand::PlanFleetAction(FleetAction::CopySettings),
        Targets::Application,
    )
    .expect("plan copy");
    let copy_bytes = live() - before;
    let waiting = d.session().fleet_plan().expect("copy plan");
    assert!(waiting.needs_confirm());
    assert_eq!(
        waiting.target_names().count(),
        BOTS - 1,
        "every bot but the source"
    );
    d.submit(OperatorCommand::CancelFleetPlan, Targets::Application)
        .expect("cancel copy");
    let copy_after_cancel = live() - before;

    eprintln!(
        "FLEET-PLAN-HEAP\nbots={BOTS} plan_bytes={plan_bytes} per_target={} after_cancel={after_cancel}\n\
         bots={BOTS} copy_bytes={copy_bytes} per_target={} after_cancel={copy_after_cancel}",
        plan_bytes / BOTS as isize,
        copy_bytes / (BOTS as isize - 1),
    );
    assert!(
        plan_bytes <= PLAN_CEILING,
        "{plan_bytes} bytes for a {BOTS}-target plan"
    );
    assert!(
        copy_bytes <= COPY_CEILING,
        "{copy_bytes} bytes for a {BOTS}-target copy"
    );
    // What stays after Cancel is the two command records (the plan row and
    // the Cancel row), not the plan.
    assert!(
        after_cancel < plan_bytes,
        "cancel gave the plan back: {after_cancel}"
    );
    assert!(
        copy_after_cancel < copy_bytes,
        "cancel gave the copy back: {copy_after_cancel}"
    );
}

/// Measured 2026-10-21: a 100-bot Switch world plan holds 7,736 bytes (77
/// per bot: the frozen targets and the prompt); the running switch (the
/// command record, the world saves and their operations, one switch
/// record per bot) 226,606 bytes (2,266 per bot) until it settles; twenty
/// frames while every relog waits allocate nothing net. The ceilings keep
/// twice that headroom.
#[test]
fn a_100_bot_world_switch_holds_bounded_state_and_quiet_frames_grow_nothing() {
    const PLAN_CEILING: isize = 16 * 1024;
    const RUNNING_CEILING: isize = 460 * 1024;
    const FRAME_CEILING: isize = 0;
    let mut d = driver("world-switch");
    d.session_mut().set_public_worlds_for_test(&[1, 2, 3]);
    let names: Vec<String> = d.session().members().to_vec();
    for name in &names {
        let arm = d
            .session()
            .play_for_test()
            .and_then(|p| p.arm(name))
            .expect("arm");
        arm.arm_explicit_login();
        arm.complete_login_for_test();
        d.session()
            .play_for_test()
            .expect("play")
            .statuses
            .lock()
            .unwrap()
            .push(host_play::SlotStatus {
                username: name.clone(),
                connected: true,
                ingame: true,
                scene_state: 2,
                world: Some(1),
                ..host_play::SlotStatus::default()
            });
    }
    d.poll();
    d.poll();

    let before = live();
    d.submit(
        OperatorCommand::PlanFleetAction(FleetAction::SwitchWorld(2)),
        Targets::Application,
    )
    .expect("plan");
    let plan_bytes = live() - before;
    let plan = d.session().fleet_plan().expect("plan");
    assert_eq!(plan.target_names().count(), BOTS);
    assert!(plan.needs_confirm());

    let before = live();
    let command = d.session().fleet_plan().expect("plan").command();
    d.submit(command, Targets::Plan).expect("switch");
    settle_writes(&mut d);
    let running_bytes = live() - before;
    for name in &names {
        let arm = d
            .session()
            .play_for_test()
            .and_then(|p| p.arm(name))
            .expect("arm");
        assert!(arm.wants_logout(), "{name}'s relog is posted");
    }
    let result = d.session().fleet_result().expect("result");
    assert_eq!(result.counts().pending, BOTS, "{}", result.text());

    // Frames while every relog waits: nothing grows.
    d.poll();
    let before = live();
    for _ in 0..20 {
        d.poll();
    }
    let frames_bytes = live() - before;

    eprintln!(
        "FLEET-SWITCH-HEAP\nbots={BOTS} plan_bytes={plan_bytes} per_target={} \
         running_bytes={running_bytes} per_bot={} frames_20={frames_bytes}",
        plan_bytes / BOTS as isize,
        running_bytes / BOTS as isize,
    );
    assert!(
        plan_bytes <= PLAN_CEILING,
        "{plan_bytes} bytes for a {BOTS}-bot switch plan"
    );
    assert!(
        running_bytes <= RUNNING_CEILING,
        "{running_bytes} bytes while a {BOTS}-bot switch runs"
    );
    assert!(
        frames_bytes <= FRAME_CEILING,
        "{frames_bytes} bytes grew over 20 frames while relogs waited"
    );
}
