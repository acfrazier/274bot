//! Shared native Select input construction and isolated CPU replay.
//!
//! The integration target uses this code without the lib-test counting allocator.
use super::*;
use api::named_banks::NamedBank;

pub(super) struct HeldDriver;
impl Driver for HeldDriver {
    fn set_menu(&mut self, _: i32, _: i32, _: i32, _: i32, _: i32) {
        panic!("held bank walk sent a menu action");
    }
    fn do_action(&mut self, _: i32) -> bool {
        panic!("held bank walk dispatched an action");
    }
    fn try_move(
        &mut self,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: bool,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
        _: i32,
    ) -> bool {
        panic!("held bank walk sent movement");
    }
    fn local_route(&self) -> Option<(i32, i32)> {
        panic!("held follow must not run");
    }
    fn build_base(&self) -> (i32, i32) {
        panic!("held follow must not run");
    }
    fn loc_typecode(&self, _: i32, _: i32) -> Option<i32> {
        panic!("held follow must not run");
    }
    fn send_packet(&mut self, _: &api::prot::Send) -> Result<(), api::prot::PacketError> {
        panic!("held bank walk wrote a packet");
    }
    fn login(&mut self, _: &str, _: &str, _: bool) -> bool {
        panic!("held bank walk attempted login");
    }
}
pub(super) fn native_pick_action(
    banks: Vec<NamedBank>,
    from: WorldTile,
    explicit: Option<&str>,
    allow_air_fallback: bool,
) -> (script::SlotScript, script::native::HostAction) {
    use script::native::{ActionHandle, NativeTick, Script, ScriptFailure, ScriptFlow};
    use script::native_bank::{Select, SelectArgs};

    struct Selecting {
        args: Option<SelectArgs>,
        handle: Option<ActionHandle<Select>>,
    }
    impl Script for Selecting {
        fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
            if let Some(args) = self.args.take() {
                self.handle = Some(tick.actions.begin::<Select>(args, &mut tick.cx).unwrap());
            }
            assert!(tick
                .actions
                .poll(self.handle.as_ref().unwrap(), &mut tick.cx)
                .is_pending());
            Ok(ScriptFlow::Continue)
        }
    }
    let mut slot = script::SlotScript::new();
    slot.start_test_script(
        Box::new(Selecting {
            args: Some(SelectArgs {
                facts: Arc::new(NamedBankFacts::from_banks(banks)),
                from,
                preferences: BankPreferences::default(),
                options: script::native::WalkOptions::default(),
                explicit: explicit.map(Arc::from),
                allow_air_fallback,
            }),
            handle: None,
        }),
        None,
    )
    .unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    slot.on_game_tick(&mut script::ScriptCtx {
        driver: &mut HeldDriver,
        tick: 1,
        here: Some((from.x, from.z, from.level)),
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(&snapshot),
        obj_names: None,
        compiled: script::CompiledTick::default(),
    });
    let action = slot
        .take_native_action()
        .expect("real native Select request");
    assert!(action.live());
    (slot, action)
}
pub(super) fn request(
    world: &Arc<NavWorld>,
    request_id: u64,
    generation: u64,
    from: WorldTile,
    state: WorldState,
    walking: bool,
    native: Option<NativeBankPickInput>,
) -> Arc<BankPickRequest> {
    let facts = native
        .as_ref()
        .map(|native| Arc::clone(&native.request.facts))
        .unwrap_or_else(|| Arc::clone(world.named_bank_facts().unwrap()));
    let from = native.as_ref().map_or(from, |native| native.request.from);
    let preferences = native
        .as_ref()
        .map_or_else(BankPreferences::default, |native| {
            native.request.preferences
        });
    let opts = FindOptions::default();
    let BankCandidates {
        order,
        target_count,
    } = super::bank_candidates(
        world,
        &facts,
        from,
        &state,
        preferences,
        None,
        native.as_ref().map(|native| &native.request),
    );
    let lease = nav_planner::producer::lease(world).unwrap();
    let native = native.map(|native| NativePickContext {
        authority: native.authority,
        evidence: native.evidence,
        allow_air_fallback: native.request.allow_air_fallback,
    });
    super::make_bank_pick_request(
        request_id,
        lease,
        facts,
        from,
        opts,
        state,
        order,
        target_count,
        (walking && target_count != 0).then_some(generation),
        native,
    )
}
/// Replay the captured live Select input and write its per-job CPU evidence.
#[cfg(target_os = "linux")]
pub fn measures_captured_native_bank_pick_cpu() {
    use api::identity::Deadline;
    use api::work::{MonotonicClock, SharedCancellation, SliceBudget, SystemClock};
    use std::num::NonZeroU32;

    fn thread_runtime_ns() -> u64 {
        std::fs::read_to_string("/proc/thread-self/schedstat")
            .expect("Linux exports this thread's accumulated CPU runtime")
            .split_whitespace()
            .next()
            .expect("schedstat contains the CPU runtime")
            .parse()
            .expect("the CPU runtime is an unsigned nanosecond count")
    }

    struct FixedReplayClock(Instant);

    impl MonotonicClock for FixedReplayClock {
        fn now(&self) -> Instant {
            self.0
        }
    }

    struct MeasuredReplayClock {
        previous: std::cell::Cell<Option<Instant>>,
        largest_gap: std::cell::Cell<Duration>,
        reads: std::cell::Cell<u64>,
    }

    impl MonotonicClock for MeasuredReplayClock {
        fn now(&self) -> Instant {
            let now = Instant::now();
            if let Some(previous) = self.previous.replace(Some(now)) {
                self.largest_gap
                    .set(self.largest_gap.get().max(now.duration_since(previous)));
            }
            self.reads.set(self.reads.get() + 1);
            now
        }
    }

    let input_path = std::env::var_os("BOT_BANK_PICK_CPU_INPUT")
        .expect("set BOT_BANK_PICK_CPU_INPUT to the captured live selection input");
    let capture: serde_json::Value =
        serde_json::from_slice(&std::fs::read(input_path).unwrap()).unwrap();
    assert_eq!(capture["schema"], 1);
    assert_eq!(capture["preferences"], "default");
    assert_eq!(capture["walk_options"], "default");
    let captured = &capture["world_state"];
    assert!(captured["quest_evidence"].is_null());
    let state = WorldState {
        inv: serde_json::from_value(captured["inv"].clone()).unwrap(),
        worn: serde_json::from_value(captured["worn"].clone()).unwrap(),
        inventory_ready: captured["inventory_ready"].as_bool().unwrap(),
        equipment_ready: captured["equipment_ready"].as_bool().unwrap(),
        allow_fetchable_worn: captured["allow_fetchable_worn"].as_bool().unwrap(),
        stats: serde_json::from_value(captured["stats"].clone()).unwrap(),
        combat_level: serde_json::from_value(captured["combat_level"].clone()).unwrap(),
        varps: serde_json::from_value(captured["varps"].clone()).unwrap(),
        quests: serde_json::from_value(captured["quests"].clone()).unwrap(),
        map_members: captured["map_members"].as_bool().unwrap(),
        quest_evidence: None,
    };
    let [x, z, level]: [i32; 3] = serde_json::from_value(capture["from"].clone()).unwrap();
    let from = WorldTile { x, z, level };
    let explicit = capture["explicit_bank"].as_str().unwrap();
    let pack = std::env::var_os("BOT_BANK_PICK_CPU_NAVPACK")
        .expect("set BOT_BANK_PICK_CPU_NAVPACK to the same immutable live navpack");
    let loaded = NavWorld::load_pack(std::path::Path::new(&pack)).unwrap();
    loaded
        .bind_named_bank_facts(
            &api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap(),
        )
        .unwrap();
    let world = Arc::new(loaded);
    let (slot, action) = native_pick_action(
        world.named_bank_facts().unwrap().banks().to_vec(),
        from,
        Some(explicit),
        false,
    );
    let authority = action.authority();
    let script::native::HostEffect::BankPick(native_request) = action.effect else {
        panic!("the CPU replay must use the actual native Select action");
    };
    let evidence = EvidenceStamp {
        run: authority.run(),
        tick: 1,
        sequence: 1,
    };
    let request = request(
        &world,
        authority.request_id().get(),
        1,
        from,
        state,
        false,
        Some(NativeBankPickInput {
            request: native_request,
            authority,
            evidence,
        }),
    );
    assert!(request.native.is_some());
    assert!(request.walk_generation.is_none());
    let mode = std::env::var("BOT_BANK_PICK_CPU_MODE").unwrap_or_else(|_| "system".to_owned());
    assert!(matches!(
        mode.as_str(),
        "system" | "fixed" | "facade" | "checkpoint-gaps"
    ));
    if mode == "facade" {
        let wall_started = Instant::now();
        let cpu_started = thread_runtime_ns();
        let result = request.legacy_calculate(&world, false);
        let output = serde_json::json!({
            "schema": 1,
            "mode": mode,
            "capture": "current synchronous facade from the captured live native Select input",
            "cpu_source": "/proc/thread-self/schedstat runtime_ns",
            "cpu_scope": "current-thread synchronous facade calculation",
            "excludes": "pack loading, game-data binding, input decoding and native action construction",
            "from": [x, z, level],
            "explicit_bank": explicit,
            "cpu_ns": thread_runtime_ns().checked_sub(cpu_started).unwrap(),
            "wall_ns": wall_started.elapsed().as_nanos(),
            "result_kind": format!("{:?}", result.kind),
            "has_route": result.route.is_some(),
            "selected": format!("{:?}", request.selected(result.index, result.kind, result.access_tile)),
        });
        let out = std::env::var_os("BOT_BANK_PICK_CPU_OUT")
            .expect("set BOT_BANK_PICK_CPU_OUT to the CPU evidence path");
        std::fs::write(out, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
        eprintln!("bank_pick_captured_cpu {output}");
        assert_eq!(result.kind, PickKind::Reachable);
        assert_eq!(request.facts.banks()[result.index.unwrap()].name, explicit);
        drop(slot);
        return;
    }
    let fixed_clock = FixedReplayClock(Instant::now());
    let measured_clock = MeasuredReplayClock {
        previous: std::cell::Cell::new(None),
        largest_gap: std::cell::Cell::new(Duration::ZERO),
        reads: std::cell::Cell::new(0),
    };
    let clock: &dyn MonotonicClock = match mode.as_str() {
        "fixed" => &fixed_clock,
        "checkpoint-gaps" => &measured_clock,
        _ => &SystemClock,
    };
    let cancellation = SharedCancellation::new();
    let wall_started = Instant::now();
    let cpu_started = thread_runtime_ns();
    let (input, preflights) = request.work_input(1, false, "test").unwrap();
    let charge_bytes = input.charge.total_bytes().unwrap();
    let mut worker = BankPickWorkerJob::for_job(
        "test",
        BankPickJob {
            generation: 1,
            request: Arc::clone(&request),
            deadline: None,
        },
        input,
        preflights,
    );
    let mut slices = 0;
    let mut largest_slice = Duration::ZERO;
    let result = loop {
        assert!(
            slices < 100_000,
            "the replay exceeded its finite slice bound"
        );
        let budget = SliceBudget::new(
            NonZeroU32::new(64).unwrap(),
            Deadline::after(clock.now(), Duration::from_millis(10)),
        );
        let mut context = StepContext::new(clock, budget, &cancellation);
        let slice_started = Instant::now();
        let outcome = worker.step(&mut context);
        largest_slice = largest_slice.max(slice_started.elapsed());
        slices += 1;
        match outcome {
            StepOutcome::Yielded => {}
            StepOutcome::Completed(result) => break result,
            StepOutcome::Failed(never) => match never {},
        }
    };
    let cpu_ns = thread_runtime_ns().checked_sub(cpu_started).unwrap();
    let wall_ns = wall_started.elapsed().as_nanos();
    let selected = request.selected(result.index, result.kind, result.access_tile);
    let output = serde_json::json!({
        "schema": 1,
        "mode": mode,
        "clock_reads": measured_clock.reads.get(),
        "largest_checkpoint_gap_ns": measured_clock.largest_gap.get().as_nanos(),
        "capture": "one production BankPickWorkerJob from the captured live native Select input",
        "cpu_source": "/proc/thread-self/schedstat runtime_ns",
        "cpu_scope": "current-thread preflight, worker construction and cooperative steps",
        "excludes": "pack loading, game-data binding, input decoding and native action construction",
        "deadline": "isolated CPU replay has no live deadline; the product live window remains 5000 ms",
        "from": [x, z, level],
        "explicit_bank": explicit,
        "cpu_ns": cpu_ns,
        "wall_ns": wall_ns,
        "charge_bytes": charge_bytes,
        "slices": slices,
        "largest_slice_ns": largest_slice.as_nanos(),
        "selected": format!("{selected:?}"),
        "result_kind": format!("{:?}", result.kind),
        "result_bytes": result.owner_result_bytes().unwrap(),
    });
    let out = std::env::var_os("BOT_BANK_PICK_CPU_OUT")
        .expect("set BOT_BANK_PICK_CPU_OUT to the CPU evidence path");
    std::fs::write(out, serde_json::to_vec_pretty(&output).unwrap()).unwrap();
    eprintln!("bank_pick_captured_cpu {output}");
    assert_eq!(result.kind, PickKind::Reachable);
    let index = result.index.expect("the actual search returns a bank");
    assert_eq!(request.facts.banks()[index].name, explicit);
    assert!(charge_bytes <= BANK_PICK_JOB_BYTES);
    drop(slot);
}
