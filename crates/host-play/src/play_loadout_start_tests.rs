use super::*;
use api::local_state::{LoadoutKey, LocalStateDomain};
use script::loadouts_store::LoadoutsBacking;

const WAIT: Duration = Duration::from_secs(10);

fn fixture(
    data: Option<Arc<api::game_data::SelectedGameData>>,
) -> (
    Play,
    Arc<local_state::LocalStateService>,
    script::IsolatedEnv,
) {
    let scratch = script::IsolatedEnv::enter("start-loadout-readback");
    let service = local_state::LocalStateService::open(
        local_state::StateRoot::new(scratch.home.join(".274bot")),
        &[
            LocalStateDomain::LoadoutsReferences,
            LocalStateDomain::ScriptRuntimeSettings,
        ],
        Arc::new(api::identity::RequestIdMinter::new()),
    );
    let mut play = run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: client::Transport::Tcp,
            port: 43594,
            cache_dir: scratch.home.to_string_lossy().into_owned(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    );
    if let Some(data) = data {
        play.bind_script_test_data(data);
    }
    play.local_state = Some(Arc::new(local_state::StateComposition::new(
        Arc::clone(&service),
        "local-289",
        api::local_state::ContentScope::try_new(client::io::ClientRevision::R289, "test-content")
            .unwrap(),
    )));
    play.attach_arm("alice", SlotArm::new(7, false));
    let slot_state = play
        .local_state
        .as_ref()
        .unwrap()
        .slot_state("alice")
        .unwrap();
    script_slot_or_insert(&play.scripts, "alice")
        .lock()
        .unwrap()
        .bind_local_state(slot_state);
    (play, service, scratch)
}

fn save(
    service: &Arc<local_state::LocalStateService>,
    name: &str,
) -> api::local_state::StoreVersion {
    let backing = service.operator_loadouts();
    let baseline = backing.load().unwrap();
    backing
        .commit(
            vec![api::local_state::LoadoutOp::Upsert(
                api::loadout::Loadout::new(name).with_carry("Lobster", 17),
            )],
            baseline.version,
        )
        .unwrap()
        .version
}

fn posted(play: &Play, name: &str) -> serde_json::Value {
    script_slot(&play.scripts, "alice")
        .unwrap()
        .lock()
        .unwrap()
        .probe(&format!(
            "rustyscript.functions.__rs2b0t_selected_loadout({})",
            serde_json::to_string(name).unwrap()
        ))
        .unwrap()
}

#[test]
fn loadout_start_does_not_wait_for_an_inflight_save_or_hold_start_locks() {
    let _state_lease = crate::test_process::state();
    let (play, service, _scratch) = fixture(None);
    save(&service, "first");
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = Arc::new(Mutex::new(release_rx));
    service.set_write_stall_for_test(Arc::new(move || {
        entered_tx.send(()).unwrap();
        release_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
    }));
    let writer = {
        let service = Arc::clone(&service);
        std::thread::spawn(move || save(&service, "second"))
    };
    entered_rx.recv_timeout(WAIT).unwrap();
    let handle = play.script_start_handle();
    let card = native_test_card("export function tick() {}");
    handle
        .start_load("alice", card.source.to_string(), card.checked, None, vec![])
        .unwrap();
    assert_eq!(handle.poll_start("alice"), script::StartPoll::Pending);
    assert!(script_slot(&play.scripts, "alice")
        .unwrap()
        .try_lock()
        .is_ok());
    assert!(play.script_start_hold.try_lock().is_ok());
    let allocations = allocation_counter::measure(|| {
        for _ in 0..32 {
            assert_eq!(handle.poll_start("alice"), script::StartPoll::Pending);
        }
    });
    assert_eq!(
        allocations.count_total, 0,
        "unchanged pending Start polls allocate"
    );
    release_tx.send(()).unwrap();
    let version = writer.join().unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let slot = script_slot(&play.scripts, "alice").unwrap();
    let retained = slot
        .lock()
        .unwrap()
        .local_state()
        .unwrap()
        .operator_loadouts()
        .unwrap();
    assert_eq!(retained.version(), version);
    assert_eq!(retained.rows().last().unwrap().name, "second");
    assert_eq!(posted(&play, "second")["carry"][0]["qty"], 17);
    let snapshot = service
        .loadout_snapshot(&LoadoutKey::OperatorPresets)
        .unwrap();
    assert!(Arc::ptr_eq(&retained.shared(), &snapshot.shared_rows()));
    assert_eq!(
        retained.digest(),
        script::local_state::OperatorLoadouts::from_rows(
            snapshot.shared_rows(),
            snapshot.version()
        )
        .digest()
    );
    handle.stop("alice").unwrap();
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn loadout_start_drops_a_held_stale_generation_and_keeps_posted_native_and_compat_rows() {
    let _state_lease = crate::test_process::state();
    let (play, service, _scratch) = fixture(None);
    let first_version = save(&service, "first");
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
    let release_rx = Arc::new(Mutex::new(release_rx));
    let once = Arc::new(std::sync::atomic::AtomicBool::new(false));
    service.set_read_stall_for_test(Arc::new(move || {
        if !once.swap(true, std::sync::atomic::Ordering::SeqCst) {
            entered_tx.send(()).unwrap();
            release_rx.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));
    let handle = play.script_start_handle();
    let native = native_test_card("export function tick() {}");
    handle
        .start_load(
            "alice",
            native.source.to_string(),
            native.checked,
            None,
            vec![],
        )
        .unwrap();
    entered_rx.recv_timeout(WAIT).unwrap();
    let stale = handle.started_generation("alice").unwrap();
    handle.stop("alice").unwrap();
    let replacement = [api::loadout::Loadout::new("replacement").with_carry("Shark", 9)];
    let compat = prepared_test_card(
        "export default class Bot extends LoopingBot { loop() {} }",
        script_js::LoadShape::CompatClass,
    );
    handle
        .start_load_with_loadouts(
            "alice",
            compat.source.to_string(),
            compat.checked,
            None,
            vec![],
            &replacement,
        )
        .unwrap();
    release_tx.send(()).unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    assert!(handle.started_generation("alice").unwrap() > stale);
    assert_eq!(posted(&play, "replacement")["carry"][0]["item"], "Shark");
    let key = LoadoutKey::OperatorPresets;
    let ticket = service.start_loadout_readback(&key).unwrap();
    service.wait_loadout_readback(&key, ticket, WAIT).unwrap();
    assert_eq!(posted(&play, "replacement")["carry"][0]["qty"], 9);
    assert_eq!(
        handle.poll_start("alice"),
        script::StartPoll::Settled(script::StartOutcome::Ready)
    );
    handle.stop("alice").unwrap();
    wait_script_state(&play, "alice", script::RunState::Idle);
    handle
        .start_load(
            "alice",
            native.source.to_string(),
            native.checked,
            None,
            vec![],
        )
        .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let slot = script_slot(&play.scripts, "alice").unwrap();
    let frozen = slot
        .lock()
        .unwrap()
        .local_state()
        .unwrap()
        .operator_loadouts()
        .unwrap();
    assert_eq!(frozen.version(), first_version);
    let second_version = save(&service, "newer");
    assert_ne!(second_version, first_version);
    assert_eq!(
        slot.lock()
            .unwrap()
            .local_state()
            .unwrap()
            .operator_loadouts()
            .unwrap()
            .version(),
        first_version
    );
    assert_eq!(posted(&play, "first")["carry"][0]["qty"], 17);
    handle.stop("alice").unwrap();
    wait_script_state(&play, "alice", script::RunState::Idle);
}

#[test]
fn loadout_start_compiled_receives_a_frozen_durable_version() {
    if !api::test_support::game_data_available() {
        return;
    }
    let _state_lease = crate::test_process::state();
    let (play, service, _scratch) = fixture(Some(
        api::game_data::for_revision(client::io::ClientRevision::R289).unwrap(),
    ));
    script_js::install_cards().unwrap();
    let first = save(&service, "first");
    let handle = play.script_start_handle();
    handle
        .start_compiled(
            "alice",
            script::CompiledId("Sherlock"),
            serde_json::Map::new(),
        )
        .unwrap();
    wait_script_state(&play, "alice", script::RunState::Running);
    let slot = script_slot(&play.scripts, "alice").unwrap();
    let frozen = slot
        .lock()
        .unwrap()
        .local_state()
        .unwrap()
        .operator_loadouts()
        .unwrap();
    assert_eq!(frozen.version(), first);
    assert_eq!(frozen.rows()[0].name, "first");
    save(&service, "second");
    assert_eq!(
        slot.lock()
            .unwrap()
            .local_state()
            .unwrap()
            .operator_loadouts()
            .unwrap()
            .version(),
        first
    );
    assert_eq!(frozen.rows().len(), 1);
    handle.stop("alice").unwrap();
}
