use super::*;
use crate::combat::RangedMode;
use crate::native::{ledger, ActionHandle, HostEffect, NativeActions, RetainedMemory};
use api::game_data::NpcAttackKind;
use api::game_data::SelectedGameData;
use api::obj_names::ItemDefView;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey, SelectedPin};
use api::snapshot::{
    ActorTargetView, ActorView, GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily,
    ItemContainer, LocalPlayerView, NpcView, PlayerView, SideTabView, SnapshotView, StatView,
    VarpView, WorldStateView,
};
use std::time::Instant;

struct Lease;
impl NativeMachine for Lease {
    type Args = ();
    type Output = ();
    fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self)
    }
    fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
        Poll::Pending
    }
    fn cancel(&mut self) {}
}

struct Runtime {
    pin: Arc<SelectedPin>,
    retained: RetainedMemory,
    ledger: Option<Box<ledger::Ledger>>,
    budget: ledger::TickBudget,
    wall: Instant,
    evidence: Option<EvidenceStamp>,
    reach: Option<api::query::ReachQueryView>,
}
impl Runtime {
    fn context<R>(
        &mut self,
        snapshot: &GameSnapshot,
        tick: u64,
        sequence: u64,
        f: impl FnOnce(&mut NativeActions, &mut ActionContext<'_>) -> R,
    ) -> R {
        self.budget.observe(tick);
        let evidence = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick,
            sequence,
        };
        self.evidence = Some(evidence);
        let action_id = self
            .ledger
            .as_ref()
            .and_then(|ledger| ledger.owner.as_ref())
            .map_or(0, |owner| owner.id.get());
        let mut actions = NativeActions { _private: () };
        let mut cx = ActionContext {
            evidence,
            pin: &self.pin,
            snapshot: SnapshotView::new(Some(snapshot), evidence).with_reach(self.reach.as_ref()),
            retained: &mut self.retained,
            action_id,
            active_now: Duration::from_millis(tick * 600),
            wall_now: self.wall,
            ledger: &mut self.ledger,
            budget: &mut self.budget,
            eligible: true,
            observed_walk_outcome_seq: 0,
        };
        f(&mut actions, &mut cx)
    }
}

#[derive(Default)]
struct DrainedBatch {
    effects: [Option<HostEffect>; 5],
    request_ids: [u64; 5],
    batch_ids: [u64; 5],
    accepted: [bool; 5],
    dispatched: [bool; 5],
    len: usize,
}
impl DrainedBatch {
    fn get(&self, index: usize) -> Option<&HostEffect> {
        self.effects.get(index).and_then(Option::as_ref)
    }
    fn len(&self) -> usize {
        self.len
    }
    fn assert_ordered(&self) {
        if self.len == 0 {
            return;
        }
        let first = self.request_ids[0];
        let batch = self.batch_ids[0];
        for offset in 0..self.len {
            assert_eq!(self.request_ids[offset], first + offset as u64);
            assert_eq!(self.batch_ids[offset], batch);
            if matches!(self.get(offset), Some(HostEffect::Interaction(_))) {
                assert_ne!(
                    batch, 0,
                    "combat interactions use emit_batch, including one-row plans"
                );
            }
        }
        if batch != 0 {
            assert_eq!(batch, first);
        }
    }
    fn into_single(mut self) -> Option<HostEffect> {
        assert!(
            self.len <= 1,
            "ordered batch has {} rows; inspect it with pending_batch",
            self.len
        );
        self.effects[0].take()
    }
}

struct Harness {
    machine: Combat,
    runtime: Runtime,
    _lease: ActionHandle<Lease>,
}
impl Harness {
    fn new(scene: &Scene, request: CombatRequest) -> Self {
        Self::new_at(scene, request, 0)
    }
    fn new_at(scene: &Scene, request: CombatRequest, origin: u64) -> Self {
        let mut runtime = Runtime {
            pin: scene.data.selected_pin().unwrap(),
            retained: RetainedMemory::default(),
            ledger: None,
            budget: ledger::TickBudget::default(),
            wall: Instant::now(),
            evidence: None,
            reach: None,
        };
        let (machine, lease) = runtime.context(&scene.snapshot, origin, origin, |actions, cx| {
            let lease = actions.begin::<Lease>((), cx).unwrap();
            let machine =
                Combat::begin((Arc::new(request), Arc::clone(&scene.tables)), cx).unwrap();
            (machine, lease)
        });
        Self {
            machine,
            runtime,
            _lease: lease,
        }
    }
    fn poll(
        &mut self,
        snapshot: &GameSnapshot,
        tick: u64,
    ) -> Poll<Result<CombatReport, ActionError>> {
        self.poll_stamp(snapshot, tick, tick)
    }
    fn poll_stamp(
        &mut self,
        snapshot: &GameSnapshot,
        tick: u64,
        sequence: u64,
    ) -> Poll<Result<CombatReport, ActionError>> {
        let machine = &mut self.machine;
        self.runtime
            .context(snapshot, tick, sequence, |_, cx| machine.poll(cx))
    }
    fn drain(&mut self, refuse_at: Option<usize>) -> DrainedBatch {
        let evidence = self
            .runtime
            .evidence
            .expect("poll establishes dispatch evidence");
        let ledger = self.runtime.ledger.as_mut().unwrap();
        let mut drained = DrainedBatch::default();
        let mut stopped = false;
        while !ledger.outbox.is_empty() {
            let action = ledger.outbox.remove(0);
            let index = drained.len;
            assert!(
                index < drained.effects.len(),
                "combat batch exceeds five rows"
            );
            let authority = action.authority();
            let dispatched = !stopped;
            let accepted = dispatched && refuse_at != Some(index);
            if dispatched && !accepted {
                stopped = true;
            }
            if matches!(&action.effect, HostEffect::Interaction(_)) {
                ledger.complete_interaction(
                    &authority,
                    crate::native::InteractionReceipt {
                        request_id: action.request_id.get(),
                        evidence: EvidenceStamp {
                            run: authority.run(),
                            tick: evidence.tick,
                            sequence: evidence.sequence,
                        },
                        accepted,
                        chat_since: 0,
                    },
                );
            }
            drained.request_ids[index] = action.request_id.get();
            drained.batch_ids[index] = action.batch;
            drained.accepted[index] = accepted;
            drained.dispatched[index] = dispatched;
            drained.effects[index] = Some(action.effect);
            drained.len += 1;
        }
        if let Some(refuse_at) = refuse_at {
            assert!(
                refuse_at < drained.len,
                "refused row must be in the admitted batch"
            );
        }
        drained.assert_ordered();
        drained
    }
    fn take(&mut self) -> Option<HostEffect> {
        self.drain(None).into_single()
    }
    fn pending(&mut self, scene: &Scene, tick: u64) -> Option<HostEffect> {
        assert!(matches!(self.poll(&scene.snapshot, tick), Poll::Pending));
        self.take()
    }
    fn pending_batch(&mut self, scene: &Scene, tick: u64) -> DrainedBatch {
        assert!(matches!(self.poll(&scene.snapshot, tick), Poll::Pending));
        self.drain(None)
    }
    fn pending_with_refusal(&mut self, scene: &Scene, tick: u64, refuse_at: usize) -> DrainedBatch {
        assert!(matches!(self.poll(&scene.snapshot, tick), Poll::Pending));
        self.drain(Some(refuse_at))
    }
    fn ready(&mut self, scene: &Scene, tick: u64) -> CombatReport {
        match self.poll(&scene.snapshot, tick) {
            Poll::Ready(Ok(report)) => {
                assert_eq!(self.drain(None).len(), 0);
                report
            }
            result => panic!("expected completed combat, got {result:?}; failures={:?}, pending={:?}, phase={:?}",
                self.machine.schedule.unsettled, self.machine.pending.map(|row| (row.row.kind, row.age)), self.machine.phase),
        }
    }
}

struct Scene {
    data: Arc<SelectedGameData>,
    tables: Arc<CombatTables>,
    snapshot: GameSnapshot,
    local: LocalPlayerView,
    npcs: Vec<NpcView>,
    players: Vec<PlayerView>,
    stats: Vec<StatView>,
    varps: Vec<VarpView>,
    inventory: Vec<ItemView>,
    equipment: Vec<ItemView>,
}
fn tile(x: i32, z: i32) -> api::WorldTile {
    api::WorldTile { x, z, level: 0 }
}
fn actor(at: api::WorldTile) -> ActorView {
    ActorView {
        name: None,
        actions: Vec::new(),
        tile: at,
        distance: 0,
        animation: -1,
        animation_frame: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 40,
        total_health: 40,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    }
}
impl Scene {
    fn new(config: &str) -> Self {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let tables = CombatTables::build(Arc::clone(&data)).unwrap();
        let row = data.npc_by_config(config).unwrap();
        let at = tile(2600, 3200);
        let npc = NpcView {
            index: 7,
            r#type: Some(row.id as usize),
            name: row.display.clone(),
            actions: vec![Some("Attack".into())],
            tile: tile(at.x + 1, at.z),
            distance: 1,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 30,
            total_health: 30,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 1,
            size: 1,
            network: tile(at.x + 1, at.z),
            x: 0,
            z: 0,
            yaw: 0,
        };
        let varps = data
            .prayers()
            .iter()
            .map(|row| VarpView {
                index: row.varp,
                value: 0,
            })
            .chain([VarpView {
                index: super::super::OPTION_NODEF,
                value: 0,
            }])
            .collect();
        let mut scene = Self {
            data,
            tables,
            snapshot: GameSnapshot::new(),
            npcs: vec![npc],
            players: Vec::new(),
            local: LocalPlayerView {
                player: PlayerView {
                    index: 1,
                    network: at,
                    actor: actor(at),
                    combat_level: 60,
                    skill_level: 0,
                    headicons: 0,
                    weapon: None,
                },
                energy: 100,
                weight: 0,
            },
            stats: (0..25)
                .map(|index| StatView {
                    index,
                    name: String::new(),
                    effective: if index == 5 { 1 } else { 40 },
                    base: if index == 5 { 1 } else { 40 },
                    xp: 0,
                    used: api::snapshot::stat_used(index as usize),
                })
                .collect(),
            varps,
            inventory: Vec::new(),
            equipment: Vec::new(),
        };
        scene.refresh();
        scene
    }
    fn refresh(&mut self) {
        self.refresh_without_local();
        self.snapshot.seed_local_player(self.local.clone());
    }
    fn refresh_without_local(&mut self) {
        self.snapshot.seed_ingame(2);
        self.snapshot.seed_world(WorldStateView {
            map_base_x: 2560,
            map_base_z: 3160,
            members: true,
            ..WorldStateView::default()
        });
        self.snapshot.seed_stats(self.stats.clone());
        self.snapshot.seed_varps(self.varps.clone());
        self.snapshot.seed_inventory(self.inventory.clone(), 28);
        self.snapshot.seed_equipment(self.equipment.clone());
        self.snapshot.seed_npcs(self.npcs.clone());
        self.snapshot.seed_players(self.players.clone());
        self.snapshot.seed_projectiles(Vec::new());
        self.snapshot.seed_side_tabs(Vec::new(), 0);
        self.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        self.snapshot.seed_chat_lines(Vec::new());
    }
    fn request(&self) -> CombatRequest {
        CombatRequest {
            target: Target::Npc {
                types: Arc::from([self.npcs[0].r#type.unwrap() as i32]),
                pick: Pick::Nearest,
                not_targeting_others: true,
            },
            ..CombatRequest::default()
        }
    }
    fn stat(&mut self, index: usize, effective: i32, base: i32) {
        self.stats[index].effective = effective;
        self.stats[index].base = base;
    }
    fn prayer(&mut self, varp: i32, on: bool) {
        self.varps
            .iter_mut()
            .find(|row| row.index == varp)
            .unwrap()
            .value = i32::from(on);
    }
    fn held(&self, alias: &str, slot: i32) -> ItemView {
        let item = self.data.item_by_alias(alias).unwrap();
        ItemView {
            def: ItemDefView {
                id: item.id,
                name: item.name.clone(),
                stackable: false,
                members: false,
                base_value: 1,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot,
            count: 1,
            actions: Vec::new(),
            component_id: 3214,
        }
    }
    fn face_us(&mut self) {
        self.npcs[0].target = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 1,
        });
        self.npcs[0].in_combat = true;
    }
    fn combat_tab(&mut self, root_component_id: i32) {
        self.snapshot.seed_side_tabs(
            vec![SideTabView {
                index: 0,
                root_component_id,
                available: true,
                active: true,
                visible: true,
                widgets: Vec::new(),
            }],
            0,
        );
    }
    fn install(&mut self) {
        self.local.player.actor.target = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 7,
        });
        self.local.player.actor.in_combat = true;
    }
    fn melee_seq(&self) -> i32 {
        self.data
            .style_seqs()
            .iter()
            .find(|row| {
                self.tables
                    .style_seq(row.seq_id)
                    .is_some_and(|mask| mask.contains(super::super::tables::StyleMask::MELEE))
            })
            .unwrap()
            .seq_id
    }
}
fn attack(effect: Option<HostEffect>) {
    assert!(
        matches!(effect, Some(HostEffect::Interaction(InteractReq::Npc { action, index: Some(7), .. })) if action == "Attack")
    );
}
fn held(effect: Option<HostEffect>, want: &str) {
    assert!(
        matches!(effect, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == want)
    );
}
fn attack_row(batch: &DrainedBatch, index: usize) {
    assert!(matches!(
        batch.get(index),
        Some(HostEffect::Interaction(InteractReq::Npc {
            action,
            index: Some(7),
            ..
        })) if action == "Attack"
    ));
}
fn held_row(batch: &DrainedBatch, index: usize, want: &str) {
    assert!(matches!(
        batch.get(index),
        Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == want
    ));
}
fn prayer_row(batch: &DrainedBatch, index: usize, button: i32) {
    assert!(matches!(
        batch.get(index),
        Some(HostEffect::Interaction(InteractReq::IfButton { component_id }))
            if *component_id == button
    ));
}
fn assert_len(batch: &DrainedBatch, want: usize) {
    assert_eq!(batch.len(), want);
    assert_eq!(
        batch.effects[want..]
            .iter()
            .filter(|row| row.is_some())
            .count(),
        0
    );
}
fn fight(scene: &mut Scene) -> Harness {
    let mut harness = Harness::new(scene, scene.request());
    attack(harness.pending(scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(scene, 2).is_none());
    assert_eq!(harness.machine.phase, Phase::Fight);
    harness
}
fn user_input_walk_receipt(harness: &mut Harness, tick: u64) {
    let request_id = harness
        .machine
        .pending_walk
        .expect("combat walk request was issued")
        .get();
    harness.runtime.ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        request_id,
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick,
            sequence: tick.saturating_add(1),
        },
        end: crate::native::WalkEnd::UserInput,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });
}

#[test]
fn attacker_player_slot_reuse_cannot_retarget_an_existing_engagement() {
    let mut scene = Scene::new("imp");
    let mut attacker = PlayerView {
        index: 8,
        network: tile(2601, 3200),
        actor: actor(tile(2601, 3200)),
        combat_level: 60,
        skill_level: 0,
        headicons: 0,
        weapon: None,
    };
    attacker.actor.name = Some("Alpha".into());
    attacker.actor.actions = vec![Some("Attack".into())];
    attacker.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 1,
    });
    attacker.actor.animation = scene.melee_seq();
    attacker.actor.in_combat = true;
    scene.snapshot.seed_players(vec![attacker.clone()]);
    let mut request = scene.request();
    request.target = Target::Attacker {
        npcs: false,
        players: true,
    };
    request.fallback = Fallback::Fight;
    let mut harness = Harness::new(&scene, request);
    assert!(
        matches!(harness.pending(&scene, 1), Some(HostEffect::Interaction(InteractReq::Player { name, action })) if name == "Alpha" && action == "Attack"),
        "engaged={:?}, phase={:?}, end={:?}",
        harness.machine.engaged,
        harness.machine.phase,
        harness.machine.end
    );
    attacker.actor.name = Some("Beta".into());
    attacker.actor.health = 0;
    scene.snapshot.seed_players(vec![attacker]);
    for tick in 2..5 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    let report = harness.ready(&scene, 5);
    assert_eq!(report.end, CombatEnd::TargetGone);
    assert_eq!(
        report.engaged,
        Some(ActorRef {
            kind: ActorKind::Player,
            index: 8
        })
    );
    assert_eq!(report.engaged_npc_type, -1);
}

#[test]
fn fail_closed_flick_and_pvp_modes_have_no_host_work() {
    let scene = Scene::new("imp");
    for change in [2, 3] {
        let mut request = scene.request();
        match change {
            2 => request.prayer_mode = PrayerMode::Flick,
            _ => {
                request.target = Target::Player {
                    name: Arc::from("someone"),
                }
            }
        }
        let mut harness = Harness::new(&scene, scene.request());
        let result = harness.runtime.context(&scene.snapshot, 1, 1, |_, cx| {
            Combat::begin((Arc::new(request), Arc::clone(&scene.tables)), cx)
        });
        assert!(matches!(result, Err(ActionError::Unavailable(_))));
        assert!(harness.take().is_none());
    }
}

#[test]
fn case14_kill_proof_beats_plain_disappearance_but_not_our_death() {
    for dead in [false, true] {
        let mut scene = Scene::new("imp");
        let mut harness = fight(&mut scene);
        scene.npcs[0].health = 0;
        if dead {
            scene.stat(3, 0, 40);
        }
        scene.refresh();
        assert_eq!(
            harness.ready(&scene, 3).end,
            if dead {
                CombatEnd::Died
            } else {
                CombatEnd::Killed
            }
        );
    }
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    scene.npcs.clear();
    scene.refresh();
    for tick in 3..6 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.ready(&scene, 6).end, CombatEnd::TargetGone);
}

#[test]
fn case40_missing_local_observations_do_not_latch_target_gone() {
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    let missing = {
        let saved = std::mem::replace(&mut scene.snapshot, GameSnapshot::new());
        scene.refresh_without_local();
        std::mem::replace(&mut scene.snapshot, saved)
    };
    assert!(missing.local_player().is_none());
    harness.runtime.context(&missing, 3, 3, |_, cx| {
        assert!(cx.snapshot().stats().is_some());
        assert!(cx.snapshot().local_player().is_none());
    });
    for tick in 3..7 {
        assert!(matches!(harness.poll(&missing, tick), Poll::Pending));
        assert!(harness.take().is_none());
    }
    scene.npcs[0].health = 0;
    scene.refresh();
    let report = harness.ready(&scene, 7);
    assert_eq!(report.end, CombatEnd::Killed);
    assert_eq!(report.ticks, 3);
}

#[test]
fn case35_listed_transform_is_not_a_kill_and_unlisted_transform_is_gone() {
    let mut scene = Scene::new("imp");
    let other = scene.data.npc_by_config("khazard_warlord").unwrap().id;
    let mut request = scene.request();
    request.target = Target::Npc {
        types: Arc::from([scene.npcs[0].r#type.unwrap() as i32, other]),
        pick: Pick::Nearest,
        not_targeting_others: true,
    };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.npcs[0].r#type = Some(other as usize);
    scene.npcs[0].health = 0;
    scene.refresh();
    assert!(matches!(harness.poll(&scene.snapshot, 2), Poll::Pending));
    assert_ne!(harness.machine.end, Some(CombatEnd::Killed));
    assert_eq!(harness.machine.engaged_type(), other);
    harness.take();
    scene.npcs[0].r#type = Some(scene.data.npc_by_config("nasty_tree").unwrap().id as usize);
    scene.refresh();
    for tick in 3..6 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.ready(&scene, 6).end, CombatEnd::TargetGone);
}

#[test]
fn same_stamp_and_new_sequence_same_tick_cannot_duplicate_work() {
    let scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack(harness.pending(&scene, 1));
    assert!(matches!(harness.poll(&scene.snapshot, 1), Poll::Pending));
    assert!(harness.take().is_none());
    assert!(matches!(
        harness.poll_stamp(&scene.snapshot, 1, 2),
        Poll::Pending
    ));
    assert!(harness.take().is_none());
    assert_eq!(harness.machine.counters.ticks, 1);
}

#[test]
fn case27e_refused_drink_cannot_create_input_lock_or_restoration() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("4doseprayerrestore", 0));
    scene.refresh();
    harness.runtime.budget.observe(3);
    assert!(harness.runtime.budget.event(false));
    assert!(harness.pending(&scene, 3).is_none());
    assert_eq!(harness.machine.input_lock(), None);
    assert!(!harness.machine.schedule.restore_owed);
    assert!(!harness.machine.schedule.pending(OpKind::Drink));
    held(harness.pending(&scene, 4), "Drink");
    assert_eq!(harness.machine.input_lock(), Some(7));
}

#[test]
fn case27_recovery_food_and_drink_share_a_ready_plan() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("lobster", 0));
    scene.inventory.push(scene.held("1doseprayerrestore", 1));
    scene.refresh();
    let plan = harness.pending_batch(&scene, 3);
    assert_len(&plan, 2);
    held_row(&plan, 0, "Eat");
    held_row(&plan, 1, "Drink");
    assert_eq!(harness.machine.input_lock(), Some(6));

    // Both item counts and the capped heal settle in the output observation.
    scene.stat(3, 37, 40);
    scene.stat(5, 17, 43);
    scene.inventory.clear();
    scene.refresh();
    for tick in 4..6 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.machine.counters.locked, 2);
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let restore = harness.pending_batch(&scene, 6);
    assert_len(&restore, 2);
    prayer_row(&restore, 0, protect.button_com);
    attack_row(&restore, 1);
    scene.prayer(protect.varp, true);
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 7).is_none());
    assert_eq!(harness.machine.counters.restorations, 1);
    assert_eq!(harness.machine.counters.locked, 2);
}

#[test]
fn case44_lone_karambwan_uses_the_reachable_danger_four_gate() {
    let mut scene = Scene::new("imp");
    let danger_four = scene
        .data
        .npc_names()
        .unwrap()
        .rows
        .iter()
        .find(|row| super::select::facts::npc_max_hit(row) == Some(4))
        .expect("selected content includes a non-bespoke danger-four NPC");
    scene.npcs[0].r#type = Some(danger_four.id as usize);
    scene.npcs[0].name = danger_four.display.clone();
    scene.stat(5, 1, 1);
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.face_us();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());

    scene.stat(3, 9, 30);
    scene.inventory.push(scene.held("tbwt_cooked_karambwan", 0));
    scene.refresh();
    let combo = harness.pending_batch(&scene, 3);
    assert_len(&combo, 1);
    held_row(&combo, 0, "Eat");
    assert_eq!(harness.machine.input_lock(), Some(7));
    scene.stat(3, 27, 30);
    scene.inventory.clear();
    scene.refresh();
    for tick in 4..7 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    attack(harness.pending(&scene, 7));
    assert_eq!(harness.machine.counters.locked, 3);
}

#[test]
fn unsafe_combo_only_food_does_not_hide_no_food_abort() {
    let mut scene = Scene::new("khazard_warlord");
    let mut request = scene.request();
    request.fallback = Fallback::Abort;
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    assert_eq!(harness.machine.phase, Phase::Fight);
    scene.face_us();
    scene.stat(3, 15, 40);
    scene.inventory.push(scene.held("tbwt_cooked_karambwan", 0));
    scene.refresh();

    let held_only_combo = harness.pending_batch(&scene, 3);
    assert_len(&held_only_combo, 0);
    assert_eq!(harness.machine.end, None);

    scene.stat(3, 9, 40);
    scene.refresh();
    assert!(matches!(
        harness.poll(&scene.snapshot, 4),
        Poll::Ready(Ok(CombatReport {
            end: CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoFood)),
            ..
        }))
    ));
    assert_len(&harness.drain(None), 0);
}

#[test]
fn held_food_on_cooldown_does_not_become_a_no_food_abort() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 9, 40);
    let mut lobster = scene.held("lobster", 0);
    lobster.count = 2;
    scene.inventory.push(lobster);
    scene.refresh();
    let first = harness.pending_batch(&scene, 3);
    assert_len(&first, 2);
    held_row(&first, 0, "Eat");
    attack_row(&first, 1);
    scene.install();
    scene.inventory[0].count = 1;
    scene.stat(3, 10, 40);
    scene.refresh();
    for tick in 4..6 {
        assert_len(&harness.pending_batch(&scene, tick), 0);
        assert_eq!(harness.machine.end, None);
    }
    let retry = harness.pending_batch(&scene, 6);
    assert_len(&retry, 2);
    held_row(&retry, 0, "Eat");
    attack_row(&retry, 1);
}

#[test]
fn case43_capped_recovery_gates_the_drink_strictly() {
    for (hp, max, drink) in [(20, 30, true), (16, 30, false), (35, 40, true)] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(3, hp, max);
        scene.stat(5, 0, 43);
        scene.face_us();
        scene.inventory.push(scene.held("lobster", 0));
        scene.inventory.push(scene.held("4doseprayerrestore", 1));
        scene.refresh();
        let plan = harness.pending_batch(&scene, 3);
        if hp == 35 {
            assert_len(&plan, 1);
            held_row(&plan, 0, "Drink");
        } else {
            assert_len(&plan, 2);
            held_row(&plan, 0, "Eat");
            if drink {
                held_row(&plan, 1, "Drink");
            } else {
                attack_row(&plan, 1);
            }
        }
        assert_eq!(plan.effects.iter().flatten().any(|row|
            matches!(row, HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drink")), drink);
        if !drink {
            assert_eq!(harness.machine.input_lock(), None);
        }
    }

    let mut scene = Scene::new("khazard_warlord");
    let mut request = scene.request();
    request.allow.food = false;
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 3), 0);
}

#[test]
fn case3_food_only_line_and_real_heal_choose_largest_fitting_food() {
    for hp in [20, 19] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(3, hp, 40);
        scene.face_us();
        scene.inventory.push(scene.held("lobster", 0));
        scene.inventory.push(scene.held("shark", 1));
        scene.refresh();
        let plan = harness.pending_batch(&scene, 3);
        if hp == 19 {
            assert_len(&plan, 2);
            assert!(matches!(
                plan.get(0),
                Some(HostEffect::Interaction(InteractReq::Held { name, action, .. }))
                    if name == "Shark" && action == "Eat"
            ));
            attack_row(&plan, 1);
        } else {
            assert_len(&plan, 0);
        }
    }
}

#[test]
fn case18_no_food_allowance_obeys_abort_or_fight_policy() {
    for fallback in [Fallback::Abort, Fallback::Fight] {
        let mut scene = Scene::new("khazard_warlord");
        let mut request = scene.request();
        request.allow.food = false;
        request.fallback = fallback.clone();
        let mut harness = Harness::new(&scene, request);
        attack(harness.pending(&scene, 1));
        scene.install();
        scene.face_us();
        scene.stat(3, 1, 40);
        scene.inventory.push(scene.held("lobster", 0));
        scene.refresh();
        if matches!(fallback, Fallback::Abort) {
            assert_eq!(
                harness.ready(&scene, 2).end,
                CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoFood))
            );
        } else {
            assert!(harness.pending(&scene, 2).is_none());
            assert_eq!(harness.machine.end, None);
        }
    }
}

#[test]
fn case38_terminal_prayer_offs_are_unpaced_and_never_restore_attack() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 43, 43);
    scene.face_us();
    let prayers: Vec<_> = scene
        .data
        .prayers()
        .iter()
        .filter(|row| {
            [
                "Protect from Melee",
                "Ultimate Strength",
                "Incredible Reflexes",
            ]
            .contains(&row.name.as_str())
        })
        .map(|row| (row.varp, row.button_com))
        .collect();
    assert_eq!(prayers.len(), 3);
    scene.refresh();
    let raised = harness.pending_batch(&scene, 3);
    assert_eq!(policy_s2_buttons(&raised).len(), prayers.len());
    for (_, button) in &prayers {
        assert!(policy_s2_buttons(&raised).contains(button));
    }
    for (varp, _) in &prayers {
        scene.prayer(*varp, true);
    }
    scene.npcs[0].health = 0;
    scene.refresh();
    let off = harness.pending_batch(&scene, 4);
    assert_len(&off, prayers.len());
    for (index, (_, button)) in prayers.iter().enumerate() {
        prayer_row(&off, index, *button);
    }
    for (varp, _) in &prayers {
        scene.prayer(*varp, false);
    }
    scene.refresh();
    let report = harness.ready(&scene, 5);
    assert_eq!(report.end, CombatEnd::Killed);
    // The accepted raises restored once; terminal offs add no restoration.
    assert_eq!(report.restorations, 1);
}

#[test]
fn case20_steady_nonemitting_combat_ticks_allocate_nothing() {
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    scene.local.player.actor.animation = scene.melee_seq();
    scene.local.player.actor.animation_frame = 0;
    scene.snapshot.seed_local_player(scene.local.clone());
    assert!(harness.pending(&scene, 3).is_none());
    let allocations = allocation_counter::measure(|| {
        for tick in 4..204 {
            scene.local.player.actor.animation_frame = (tick % 4) as i32;
            scene.snapshot.seed_local_player(scene.local.clone());
            assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
            assert!(harness.runtime.ledger.as_ref().unwrap().outbox.is_empty());
        }
    });
    assert_eq!(allocations.count_total, 0);
    assert!(harness.machine.counters.swings >= 50);
    assert_eq!(harness.machine.counters.food, 0);
    assert!(std::mem::size_of::<Combat>() <= 512);
    assert!(std::mem::size_of::<CombatReport>() <= 80);
    println!("Combat={} CombatReport={} CombatRequest={} ArcRequest={} ActionHandle={} HostAction={} Ledger={} InteractionReceipt={} WalkReceipt={}",
        std::mem::size_of::<Combat>(), std::mem::size_of::<CombatReport>(), std::mem::size_of::<CombatRequest>(), std::mem::size_of::<Arc<CombatRequest>>(),
        std::mem::size_of::<ActionHandle<Combat>>(), std::mem::size_of::<ledger::HostAction>(),
        std::mem::size_of::<ledger::Ledger>(), std::mem::size_of::<crate::native::InteractionReceipt>(), std::mem::size_of::<crate::native::WalkReceipt>());
    let native_ledger = harness.runtime.ledger.as_ref().unwrap();
    println!(
        "Owner={} TickBudget={} OutboxHeader={} OutboxReserved={} BatchReceiptRing={}",
        std::mem::size_of_val(native_ledger.owner.as_deref().unwrap()),
        std::mem::size_of_val(&harness.runtime.budget),
        std::mem::size_of_val(&native_ledger.outbox),
        native_ledger.outbox.capacity() * std::mem::size_of::<ledger::HostAction>(),
        std::mem::size_of_val(&native_ledger.batch_receipts),
    );
}

#[test]
fn offensive_prayers_wait_for_live_combat_before_the_floor_sip() {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(5, 26, 43);
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack(harness.pending(&scene, 1));
    // Facing our selected target proves engagement, not a live attack on us.
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 2), 0);

    scene.face_us();
    scene.npcs[0].animation = scene.melee_seq();
    scene.npcs[0].animation_frame = 0;
    scene.refresh();
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let sip = harness.pending_batch(&scene, 3);
    assert_len(&sip, 2);
    prayer_row(&sip, 0, protect.button_com);
    held_row(&sip, 1, "Drink");
    scene.prayer(protect.varp, true);
    scene.inventory.clear();
    scene.stat(5, 43, 43);
    scene.refresh();
    for tick in 4..6 {
        assert_len(&harness.pending_batch(&scene, tick), 0);
    }
    let restore = harness.pending_batch(&scene, 6);
    assert_len(&restore, 3);
    for (index, name) in ["Ultimate Strength", "Incredible Reflexes"]
        .into_iter()
        .enumerate()
    {
        let row = scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == name)
            .unwrap();
        prayer_row(&restore, index, row.button_com);
    }
    attack_row(&restore, 2);
    assert_eq!(harness.machine.counters.restorations, 1);
}

#[test]
fn combat_does_not_sip_above_the_native_c5_floor() {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(5, 27, 43);
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    attack(harness.pending(&scene, 1));
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 7,
    });
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 2), 0);

    scene.face_us();
    scene.npcs[0].animation = scene.melee_seq();
    scene.npcs[0].animation_frame = 0;
    scene.refresh();
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let batch = harness.pending_batch(&scene, 3);
    assert!(!batch.effects.iter().flatten().any(|row| matches!(
        row,
        HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drink"
    )));
    assert!(batch.effects.iter().flatten().any(|row| matches!(
        row,
        HostEffect::Interaction(InteractReq::IfButton { component_id })
            if *component_id == protect.button_com
    )));
}

#[test]
fn combat_and_guard_share_projectile_first_protect_policy() {
    let mut scene = Scene::new("cow");
    scene.stat(5, 43, 43);
    scene.refresh();
    let mut harness = fight(&mut scene);

    scene.face_us();
    scene.refresh();
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: 9,
            level: 0,
            src: scene.npcs[0].tile,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: scene.local.player.index,
            }),
            t1: 0,
            t2: 1,
        }]);
    let combat_batch = harness.pending_batch(&scene, 3);
    let evidence = harness.runtime.evidence.expect("Combat frame evidence");
    let snapshot = SnapshotView::new(Some(&scene.snapshot), evidence);
    let request = crate::native::WalkRequest {
        target: scene.local.player.actor.tile,
        loc_id: None,
        radius: 1,
        arrival: nav::arrival::ArrivalKind::Reach,
        options: crate::native::WalkOptions::default(),
        required_after: evidence,
        evidence: None,
        cross: Vec::new().into_boxed_slice(),
        protect: true,
        food_guard: false,
        allow: crate::native::WalkAllow::default(),
    };
    let mut guard =
        crate::combat::WalkGuard::begin_with(&request, &snapshot, Arc::clone(&scene.tables))
            .expect("Guard starts with level 43 Prayer");
    let guard_op = guard.tick(&snapshot);
    let expected_button = scene
        .data
        .prayer_by_name("Protect from Missiles")
        .expect("selected Protect from Missiles")
        .button_com;
    let combat_button = (0..combat_batch.len()).find_map(|index| match combat_batch.get(index) {
        Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
            Some(*component_id)
        }
        _ => None,
    });
    assert_eq!(
        combat_button,
        Some(expected_button),
        "Combat's chosen protect"
    );
    assert_eq!(
        guard_op,
        Some(crate::combat::GuardOp::IfButton {
            component: expected_button,
        }),
        "Guard's chosen protect"
    );
}

#[test]
fn disagreeing_classified_projectiles_fall_back_to_the_saved_score() {
    let mut scene = Scene::new("cow");
    scene.face_us();
    scene.refresh();
    scene.snapshot.seed_projectiles(
        [9, 91]
            .into_iter()
            .map(|spotanim| api::snapshot::ProjectileView {
                spotanim,
                level: 0,
                src: scene.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: scene.local.player.index,
                }),
                t1: 0,
                t2: 1,
            })
            .collect(),
    );
    let evidence = EvidenceStamp {
        run: RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        tick: 3,
        sequence: 3,
    };
    let frame = Frame::borrow(SnapshotView::new(Some(&scene.snapshot), evidence))
        .expect("complete disagreement frame");
    let score_frame = Frame {
        projectiles: &[],
        ..frame
    };
    let mut threats = crate::combat::threats::ThreatSet::default();
    let local_slot = scene.local.player.index as i32;
    let cow_id = scene.npcs[0].r#type.unwrap() as i32;
    threats.observe_hunt(
        [(7, cow_id, true, 2, local_slot)],
        local_slot,
        &scene.tables,
        3,
    );

    let scored = crate::combat::policy::wanted_protect(
        &threats,
        &score_frame,
        &scene.tables,
        3,
        false,
        false,
    )
    .expect("the live cow supplies a melee score");
    let selected =
        crate::combat::policy::wanted_protect(&threats, &frame, &scene.tables, 3, false, false)
            .expect("the disagreeing volley falls back to the live score");
    assert_eq!(selected.varp, scored.varp);
    assert_eq!(selected.name, "Protect from Melee");
}

#[test]
fn lock_end_swing_cannot_discharge_the_explicit_drink_restoration() {
    for onset_offset in 1..=3 {
        let mut scene = Scene::new("khazard_warlord");
        let protect = scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == "Protect from Melee")
            .unwrap()
            .varp;
        scene.stat(5, 17, 43);
        scene.prayer(protect, true);
        scene.face_us();
        scene.refresh();
        let mut harness = fight(&mut scene);
        scene.stat(5, 0, 43);
        scene.inventory.push(scene.held("1doseprayerrestore", 0));
        scene.refresh();
        held(harness.pending(&scene, 3), "Drink");

        // M2's target and drinking animation clear after the sip, then
        // auto-retaliation supplies a fresh swing before/at the lock end.
        scene.inventory.clear();
        scene.stat(5, 17, 43);
        scene.local.player.actor.target = None;
        scene.local.player.actor.animation = -1;
        for tick in 4..=6 {
            if tick >= 3 + onset_offset {
                scene.install();
                scene.local.player.actor.animation = scene.melee_seq();
                scene.local.player.actor.animation_frame = (tick - 3 - onset_offset) as i32;
            }
            scene.refresh();
            let plan = harness.pending_batch(&scene, tick);
            if tick < 6 {
                assert_len(&plan, 0);
            } else {
                assert_len(&plan, 1);
                attack_row(&plan, 0);
            }
        }
        assert_eq!(harness.machine.counters.locked, 2);
        assert_eq!(harness.machine.counters.restorations, 1);
        scene.local.player.actor.animation_frame += 1;
        scene.refresh();
        assert_len(&harness.pending_batch(&scene, 7), 0);
        assert_eq!(harness.machine.counters.restorations, 1);
    }
}

#[test]
fn case27_drink_lock_preserves_safety_and_coalesces_restoration() {
    for hits in [1, 2, 3] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(3, if hits == 3 { 29 } else { 30 }, 40);
        scene.stat(5, 0, 43);
        scene.face_us();
        scene.inventory.push(scene.held("lobster", 0));
        scene.inventory.push(scene.held("1doseprayerrestore", 1));
        scene.refresh();
        held(harness.pending(&scene, 3), "Drink");
        scene.inventory.pop();
        scene.stat(5, 17, 43);
        scene.stat(3, if hits == 3 { 11 } else { 21 }, 40);
        scene.refresh();
        assert!(harness.pending(&scene, 4).is_none());
        assert!(matches!(harness.poll(&scene.snapshot, 4), Poll::Pending));
        assert_eq!(harness.machine.counters.locked, 1);
        if hits >= 2 {
            scene.stat(3, if hits == 3 { 2 } else { 12 }, 40);
        }
        scene.refresh();
        assert!(harness.pending(&scene, 5).is_none());
        let protect = scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == "Protect from Melee")
            .unwrap()
            .clone();
        let restored = harness.pending_batch(&scene, 6);
        let protect_index = usize::from(hits == 3);
        assert_len(&restored, protect_index + 2);
        if hits == 3 {
            held_row(&restored, 0, "Eat");
            scene.stat(3, 14, 40);
            scene.inventory.clear();
        }
        prayer_row(&restored, protect_index, protect.button_com);
        attack_row(&restored, protect_index + 1);
        scene.prayer(protect.varp, true);
        scene.install();
        scene.refresh();
        assert!(harness.pending(&scene, 7).is_none());
        assert_eq!(harness.machine.counters.locked, 2);
        assert_eq!(harness.machine.counters.restorations, 1);
        assert_eq!(harness.machine.counters.food, u8::from(hits == 3));
    }
}

#[test]
fn case27_recovery_food_is_narrow_and_uses_strict_heal_gate() {
    for hp in [9, 18, 25] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(3, hp, 40);
        scene.stat(5, 0, 43);
        scene.face_us();
        scene.inventory.push(scene.held("lobster", 0));
        scene.inventory.push(scene.held("1doseprayerrestore", 1));
        scene.refresh();
        let plan = harness.pending_batch(&scene, 3);
        if hp == 9 {
            assert_len(&plan, 2);
            held_row(&plan, 0, "Eat");
            attack_row(&plan, 1);
            assert_eq!(harness.machine.input_lock(), None);
        } else {
            assert_len(&plan, 2);
            held_row(&plan, 0, "Eat");
            held_row(&plan, 1, "Drink");
        }
    }
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("shrimp", 0));
    scene.inventory.push(scene.held("1doseprayerrestore", 1));
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 3), 0);
    assert!(harness.pending(&scene, 4).is_none());
}

#[test]
fn case28_food_extends_known_and_past_deadlines_without_delaying_restore() {
    for idle in [false, true] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.local.player.actor.animation = scene.melee_seq();
        scene.local.player.actor.animation_frame = 0;
        scene.refresh();
        assert!(harness.pending(&scene, if idle { 20 } else { 9 }).is_none());
        assert_eq!(harness.machine.cycle().deadline, if idle { 24 } else { 13 });
        let swing_tick = if idle { 20 } else { 9 };
        let eat_tick = if idle { 32 } else { 11 };
        // A gap intentionally invalidates this clock; the past-deadline
        // variant therefore keeps observing the same advancing animation.
        for tick in swing_tick + 1..eat_tick {
            scene.local.player.actor.animation_frame = (tick - swing_tick) as i32;
            scene.refresh();
            let plan = harness.pending_batch(&scene, tick);
            for index in 0..plan.len() {
                attack_row(&plan, index);
            }
        }
        scene.local.player.actor.animation_frame = (eat_tick - swing_tick) as i32;
        scene.face_us();
        scene.stat(3, 19, 40);
        scene.inventory.push(scene.held("lobster", 0));
        scene.refresh();
        let eat = harness.pending_batch(&scene, eat_tick);
        assert_len(&eat, 2);
        held_row(&eat, 0, "Eat");
        attack_row(&eat, 1);
        assert_eq!(harness.machine.cycle().deadline, if idle { 27 } else { 16 });
        scene.stat(3, 31, 40);
        scene.inventory.clear();
        scene.install();
        scene.refresh();
        assert!(harness.pending(&scene, eat_tick + 1).is_none());
        assert_eq!(harness.machine.counters.restorations, 1);
        if !idle {
            for tick in 13..16 {
                assert!(harness.pending(&scene, tick).is_none());
            }
            scene.local.player.actor.animation_frame = 0;
            scene.refresh();
            assert!(harness.pending(&scene, 16).is_none());
            assert_eq!(harness.machine.cycle().deadline, 20);
        } else {
            assert!(harness.pending(&scene, 34).is_none());
        }
    }
}

#[test]
fn case29_projected_protection_suppresses_nonemergency_food() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 43, 43);
    scene.stat(3, 19, 40);
    scene.face_us();
    scene.inventory.push(scene.held("lobster", 0));
    for name in ["Ultimate Strength", "Incredible Reflexes"] {
        let varp = scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == name)
            .unwrap()
            .varp;
        scene.prayer(varp, true);
    }
    scene.refresh();
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let plan = harness.pending_batch(&scene, 3);
    assert_len(&plan, 2);
    prayer_row(&plan, 0, protect.button_com);
    attack_row(&plan, 1);
    scene.prayer(protect.varp, true);
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 4).is_none());
    assert_eq!(harness.machine.counters.restorations, 1);
    assert_eq!(harness.machine.counters.food, 0);
}

#[test]
fn case30_stale_attack_pacing_uses_the_worn_weapon_rate() {
    for (alias, rate) in [("bronze_2h_sword", 7), ("bronze_scimitar", 4)] {
        let mut scene = Scene::new("imp");
        let mut weapon = scene.held(alias, 3);
        weapon.container = ItemContainer::Equipment;
        scene.equipment.push(weapon);
        scene.refresh();
        let mut harness = fight(&mut scene);
        for tick in 3..=rate + 1 {
            assert!(harness.pending(&scene, tick).is_none());
        }
        attack(harness.pending(&scene, rate + 2));
    }
}

#[test]
fn case34_unattackable_rows_are_threats_not_targets_and_terminal_food_is_allowed() {
    let mut scene = Scene::new("nasty_tree");
    scene.npcs[0].actions.clear();
    scene.face_us();
    scene.stat(3, 7, 30);
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();
    let request = CombatRequest {
        target: Target::Attacker {
            npcs: true,
            players: false,
        },
        ..CombatRequest::default()
    };
    let mut harness = Harness::new(&scene, request);
    held(harness.pending(&scene, 1), "Eat");
    scene.stat(3, 19, 30);
    scene.inventory.clear();
    scene.refresh();
    let report = harness.ready(&scene, 2);
    assert_eq!(report.end, CombatEnd::Aborted(AbortReason::Unattackable));
    assert_eq!(report.food, 1);
    assert_eq!(report.restorations, 0);
    let unavailable = harness.runtime.context(&scene.snapshot, 3, 3, |_, cx| {
        Combat::begin((Arc::new(scene.request()), Arc::clone(&scene.tables)), cx)
    });
    assert!(matches!(unavailable, Err(ActionError::Unavailable(_))));
    assert!(harness.take().is_none());
}

#[test]
fn counter_protect_saradomin_raises_magic_and_zamorak_is_magic() {
    let zamorak = Scene::new("trail_hard");
    let zamorak_row = zamorak
        .tables
        .npc(zamorak.npcs[0].r#type.unwrap() as i32)
        .unwrap();
    assert_eq!(zamorak_row.attack_kind, Some(NpcAttackKind::Magic));
    assert!(!zamorak_row.counter_protect);

    let mut scene = Scene::new("trail_hard2");
    let row = scene
        .tables
        .npc(scene.npcs[0].r#type.unwrap() as i32)
        .unwrap();
    assert_eq!(row.attack_kind, Some(NpcAttackKind::Mixed));
    assert!(row.counter_protect);
    scene.stat(5, 43, 43);
    scene.face_us();
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    let magic = scene
        .data
        .prayers()
        .iter()
        .find(|prayer| prayer.name == "Protect from Magic")
        .unwrap();
    let melee = scene
        .data
        .prayers()
        .iter()
        .find(|prayer| prayer.name == "Protect from Melee")
        .unwrap();
    let plan = harness.pending_batch(&scene, 1);
    let buttons: Vec<i32> = (0..plan.len)
        .filter_map(|index| match plan.get(index) {
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                Some(*component_id)
            }
            _ => None,
        })
        .collect();
    assert!(
        buttons.contains(&magic.button_com),
        "counter_protect holds Magic, not oscillating Melee: {buttons:?}"
    );
    assert!(
        !buttons.contains(&melee.button_com),
        "a held Magic protect must not also raise Melee: {buttons:?}"
    );
}

#[test]
fn case11_offensive_drop_batches_keep_protect_and_restore_last() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 43, 43);
    scene.face_us();
    scene.refresh();
    let raised = harness.pending_batch(&scene, 3);
    assert_eq!(policy_s2_buttons(&raised).len(), 3);
    scene.stat(5, 26, 43);
    scene.face_us();
    let on: Vec<_> = scene
        .data
        .prayers()
        .iter()
        .filter(|row| {
            [
                "Protect from Melee",
                "Ultimate Strength",
                "Incredible Reflexes",
            ]
            .contains(&row.name.as_str())
        })
        .cloned()
        .collect();
    for row in &on {
        scene.prayer(row.varp, true);
    }
    scene.refresh();
    let mut tick = 4;
    let mut plans = 0_u8;
    while on.iter().any(|row| {
        row.name != "Protect from Melee"
            && scene
                .varps
                .iter()
                .find(|varp| varp.index == row.varp)
                .unwrap()
                .value
                != 0
    }) {
        let plan = harness.pending_batch(&scene, tick);
        assert!(plan.len() >= 2);
        attack_row(&plan, plan.len() - 1);
        for index in 0..plan.len() - 1 {
            let component = match plan.get(index) {
                Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                    *component_id
                }
                _ => panic!("only prayer deactivations may precede the restoring Attack"),
            };
            let row = scene
                .data
                .prayers()
                .iter()
                .find(|row| row.button_com == component)
                .expect("deactivation resolves to a prayer component");
            assert_ne!(row.name, "Protect from Melee");
            assert_ne!(
                scene
                    .varps
                    .iter()
                    .find(|varp| varp.index == row.varp)
                    .unwrap()
                    .value,
                0
            );
            scene.prayer(row.varp, false);
        }
        scene.install();
        scene.refresh();
        plans = plans.saturating_add(1);
        tick += 1;
    }
    assert_len(&harness.pending_batch(&scene, tick), 0);
    assert_eq!(harness.machine.counters.restorations, plans + 1);
    let protect = on
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap();
    assert_eq!(
        scene
            .varps
            .iter()
            .find(|row| row.index == protect.varp)
            .unwrap()
            .value,
        1
    );
}

#[test]
fn case5_prayer_floor_requires_active_or_wanted_prayer() {
    for (base, current, drink) in [
        (43, 26, true),
        (43, 27, false),
        (31, 17, true),
        (31, 18, false),
    ] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(5, current, base);
        scene.inventory.push(scene.held("1doseprayerrestore", 0));
        let strength = scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == "Ultimate Strength")
            .unwrap()
            .varp;
        scene.prayer(strength, true);
        scene.refresh();
        let plan = harness.pending_batch(&scene, 3);
        assert_eq!(plan.effects.iter().flatten().any(|row|
            matches!(row, HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drink")), drink);
    }
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    scene.stat(5, 0, 43);
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    assert!(harness.pending(&scene, 3).is_none());
}

#[test]
fn case40_retreat_waits_through_drink_lock_and_observed_arrival() {
    let mut scene = Scene::new("khazard_warlord");
    let goal = tile(
        scene.local.player.actor.tile.x + 20,
        scene.local.player.actor.tile.z,
    );
    let mut request = scene.request();
    request.fallback = Fallback::Retreat { tile: goal };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    held(harness.pending(&scene, 3), "Drink");
    scene.inventory.clear();
    scene.stat(5, 17, 43);
    scene.stat(3, 9, 40);
    scene.refresh();
    for tick in 4..6 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.machine.phase, Phase::Escape);
    let obligated_before_escape_work = harness.machine.counters.restorations;
    assert!(matches!(
        harness.pending(&scene, 6),
        Some(HostEffect::Interaction(InteractReq::SetRetaliate {
            on: false
        }))
    ));
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::OPTION_NODEF)
        .unwrap()
        .value = 1;
    scene.refresh();
    assert!(
        matches!(harness.pending(&scene, 7), Some(HostEffect::Walk(request)) if request.target == goal)
    );
    for tick in 8..20 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    scene.local.player.actor.tile = goal;
    scene.refresh();
    let report = harness.ready(&scene, 20);
    assert_eq!(report.end, CombatEnd::Aborted(AbortReason::Retreated));
    assert_eq!(report.locked_ticks, 2);
    assert_eq!(report.restorations, obligated_before_escape_work);
}

#[test]
fn case15_terminal_walk_receipt_without_arrival_is_retreat_failed() {
    let mut scene = Scene::new("khazard_warlord");
    let goal = tile(
        scene.local.player.actor.tile.x + 20,
        scene.local.player.actor.tile.z,
    );
    let mut request = scene.request();
    request.fallback = Fallback::Retreat { tile: goal };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.face_us();
    scene.stat(3, 9, 40);
    scene.refresh();
    assert!(matches!(
        harness.pending(&scene, 2),
        Some(HostEffect::Interaction(InteractReq::SetRetaliate {
            on: false
        }))
    ));
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::OPTION_NODEF)
        .unwrap()
        .value = 1;
    scene.refresh();
    assert!(matches!(
        harness.pending(&scene, 3),
        Some(HostEffect::Walk(_))
    ));
    harness.runtime.ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        request_id: harness.machine.pending_walk.unwrap().get(),
        evidence: EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 4,
            sequence: 4,
        },
        end: crate::native::WalkEnd::Blocked,
        blocked: None,
        detail: None,
        refusal: None,
        assessment: None,
        escape: None,
    });
    assert_eq!(
        harness.ready(&scene, 4).end,
        CombatEnd::Aborted(AbortReason::RetreatFailed)
    );
}

#[test]
fn admitted_drink_lock_wraps_without_losing_observation_count() {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    let mut harness = Harness::new_at(&scene, scene.request(), 65_533);
    held(harness.pending(&scene, 65_534), "Drink");
    assert_eq!(harness.machine.input_lock(), Some(1));
    scene.inventory.clear();
    scene.stat(5, 17, 43);
    scene.refresh();
    for tick in 65_535..65_537 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    assert_eq!(harness.machine.counters.locked, 2);
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap();
    let restored = harness.pending_batch(&scene, 65_537);
    assert_len(&restored, 2);
    prayer_row(&restored, 0, protect.button_com);
    attack_row(&restored, 1);
}

#[test]
fn case19_unsettled_food_waits_two_windows_and_eventually_aborts() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 19, 40);
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();
    for tick in [3, 9, 15] {
        let attempt = harness.pending_batch(&scene, tick);
        assert_len(&attempt, 2);
        held_row(&attempt, 0, "Eat");
        attack_row(&attempt, 1);
        scene.install();
        for quiet in tick + 1..tick + 6 {
            let plan = harness.pending_batch(&scene, quiet);
            for index in 0..plan.len() {
                assert!(!matches!(
                    plan.get(index),
                    Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Eat"
                ));
            }
        }
    }
    assert_eq!(
        harness.ready(&scene, 21).end,
        CombatEnd::Aborted(AbortReason::Unresponsive)
    );
    assert_eq!(harness.machine.counters.food, 0);
}

#[test]
fn case1_and2_prep_distinguishes_essential_and_optional_wear() {
    for (alias, slot, expected) in [
        ("bronze_scimitar", 3, Some(PrepItem::Weapon)),
        ("iron_platebody", 4, None),
    ] {
        let mut scene = Scene::new("imp");
        let item = scene.held(alias, 0);
        let id = item.def.id;
        scene.inventory.push(item);
        scene.refresh();
        let mut request = scene.request();
        request.kit = Some(Arc::new(CompiledKit {
            worn: Arc::from([(slot, id)]),
            ..CompiledKit::default()
        }));
        let mut harness = Harness::new(&scene, request);
        let first = harness.pending_batch(&scene, 1);
        assert_len(&first, 2);
        assert!(matches!(
            first.get(0),
            Some(HostEffect::Interaction(InteractReq::Wear { .. }))
        ));
        attack_row(&first, 1);
        // Dispatching Attack succeeds while the server refuses the wear.
        scene.install();
        scene.refresh();
        for tick in 2..5 {
            assert!(harness.pending(&scene, tick).is_none());
        }
        let retry = harness.pending_batch(&scene, 5);
        assert_len(&retry, 2);
        assert!(matches!(
            retry.get(0),
            Some(HostEffect::Interaction(InteractReq::Wear { .. }))
        ));
        attack_row(&retry, 1);
        for tick in 6..9 {
            assert!(harness.pending(&scene, tick).is_none());
        }
        if let Some(item) = expected {
            assert_eq!(
                harness.ready(&scene, 9).end,
                CombatEnd::Aborted(AbortReason::PrepFailed(item))
            );
        } else {
            assert!(harness.pending(&scene, 9).is_none());
            scene.npcs[0].health = 0;
            scene.refresh();
            assert_eq!(harness.ready(&scene, 10).end, CombatEnd::Killed);
        }
    }
    for (slot, fail) in [(3, true), (4, false)] {
        let scene = Scene::new("imp");
        let id = scene.data.item_by_alias("bronze_scimitar").unwrap().id;
        let mut request = scene.request();
        request.kit = Some(Arc::new(CompiledKit {
            worn: Arc::from([(slot, id)]),
            ..CompiledKit::default()
        }));
        let mut harness = Harness::new(&scene, request);
        if fail {
            assert_eq!(
                harness.ready(&scene, 1).end,
                CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Weapon))
            );
        } else {
            attack(harness.pending(&scene, 1));
        }
    }
}

#[test]
fn case12_prep_boosts_once_each_then_respects_cycle_and_target_quarter() {
    let mut scene = Scene::new("khazard_warlord");
    for (slot, alias) in ["1dose2attack", "1dose2strength", "1dose2defense"]
        .into_iter()
        .enumerate()
    {
        scene.inventory.push(scene.held(alias, slot as i32));
    }
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    for (tick, stat) in [(1, 0), (4, 2), (7, 1)] {
        held(harness.pending(&scene, tick), "Drink");
        scene.stat(stat, 51, 40);
        scene.inventory.remove(0);
        scene.refresh();
        assert!(harness.pending(&scene, tick + 1).is_none());
        assert!(harness.pending(&scene, tick + 2).is_none());
    }
    attack(harness.pending(&scene, 10));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 11).is_none());
    scene.local.player.actor.animation = scene.melee_seq();
    scene.local.player.actor.animation_frame = 0;
    scene.refresh();
    assert!(harness.pending(&scene, 12).is_none());
    scene.stat(0, 40, 40);
    scene.inventory.push(scene.held("1dose2attack", 0));
    scene.npcs[0].health = 7;
    scene.local.player.actor.animation_frame = 1;
    scene.refresh();
    assert!(harness.pending(&scene, 13).is_none());
    scene.npcs[0].health = 30;
    scene.refresh();
    assert!(harness.pending(&scene, 14).is_none());
    assert_eq!(harness.machine.counters.boost, 3);
    assert_eq!(harness.machine.counters.locked, 6);
}

#[test]
fn case6_imp_skips_offensive_upkeep_unless_the_kit_carries_boosts() {
    for in_kit in [false, true] {
        let mut scene = Scene::new("imp");
        scene.stat(5, 43, 43);
        let potion = scene.held("1dose2attack", 0);
        let id = potion.def.id;
        scene.inventory.push(potion);
        scene.refresh();
        let mut request = scene.request();
        if in_kit {
            request.kit = Some(Arc::new(CompiledKit {
                carry: Arc::from([(id, 1)]),
                ..CompiledKit::default()
            }));
        }
        let mut harness = Harness::new(&scene, request);
        if in_kit {
            held(harness.pending(&scene, 1), "Drink");
        } else {
            attack(harness.pending(&scene, 1));
            scene.install();
            scene.refresh();
            for tick in 2..6 {
                assert!(harness.pending(&scene, tick).is_none());
            }
        }
    }
}

#[test]
fn case13_dragon_shield_overrides_kit_and_rejects_two_handed_weapon() {
    for two_handed in [false, true] {
        let mut scene = Scene::new("elvarg");
        let weapon = scene.held(
            if two_handed {
                "bronze_2h_sword"
            } else {
                "bronze_scimitar"
            },
            0,
        );
        let weapon_id = weapon.def.id;
        let shield = scene.held("antidragonbreathshield", 1);
        let shield_name = shield.def.name.clone().unwrap();
        let mut kit_shield = scene.held("rune_kiteshield", 5);
        let kit_shield_id = kit_shield.def.id;
        kit_shield.container = ItemContainer::Equipment;
        scene.equipment.push(kit_shield);
        scene.inventory.extend([weapon.clone(), shield.clone()]);
        scene.refresh();
        let mut request = scene.request();
        request.kit = Some(Arc::new(CompiledKit {
            worn: Arc::from([(3, weapon_id), (5, kit_shield_id)]),
            ..CompiledKit::default()
        }));
        let mut harness = Harness::new(&scene, request);
        if two_handed {
            assert_eq!(
                harness.ready(&scene, 1).end,
                CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Shield))
            );
        } else {
            let prep = harness.pending_batch(&scene, 1);
            assert_len(&prep, 3);
            assert!(
                matches!(prep.get(0), Some(HostEffect::Interaction(InteractReq::Wear { name })) if Some(name.as_str()) == weapon.def.name.as_deref())
            );
            assert!(
                matches!(prep.get(1), Some(HostEffect::Interaction(InteractReq::Wear { name })) if name == &shield_name)
            );
            attack_row(&prep, 2);
            let mut worn = weapon;
            worn.slot = 3;
            worn.container = ItemContainer::Equipment;
            scene.equipment.push(worn);
            let mut worn = shield;
            worn.slot = 5;
            worn.container = ItemContainer::Equipment;
            scene.equipment.retain(|row| row.slot != 5);
            scene.equipment.push(worn);
            scene.inventory.clear();
            scene.install();
            scene.local.player.actor.animation = scene.melee_seq();
            scene.refresh();
            assert!(harness.pending(&scene, 2).is_none());
            scene.local.player.actor.animation_frame = 1;
            scene.refresh();
            assert!(harness.pending(&scene, 3).is_none());
            for tick in 4..54 {
                scene.local.player.actor.animation_frame = ((tick - 2) % 4) as i32;
                scene.refresh();
                assert!(
                    harness.pending(&scene, tick).is_none(),
                    "tick {tick}, row {:?}",
                    harness.machine.plan.rows[0]
                );
            }
        }
    }
}

#[test]
fn case18_disallowed_potions_and_equipment_do_not_create_resource_intents() {
    for equipment in [false, true] {
        let mut scene = Scene::new("khazard_warlord");
        scene.inventory.push(scene.held("bronze_scimitar", 0));
        scene.inventory.push(scene.held("1dose2attack", 1));
        scene.refresh();
        let mut request = scene.request();
        request.allow.equipment = equipment;
        request.allow.potions = false;
        let mut harness = Harness::new(&scene, request);
        let plan = harness.pending_batch(&scene, 1);
        if equipment {
            assert_len(&plan, 2);
            assert!(matches!(
                plan.get(0),
                Some(HostEffect::Interaction(InteractReq::Wear { .. }))
            ));
            attack_row(&plan, 1);
        } else {
            assert_len(&plan, 1);
            attack_row(&plan, 0);
        }
        assert!(!plan.effects.iter().flatten().any(|row|
            matches!(row, HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Drink")));
    }
}

#[test]
fn case20_and26_live_mismatch_emits_only_two_owned_strings_without_cooldown_wait() {
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    let mut ignored = scene.npcs[0].clone();
    ignored.index = 8;
    scene.npcs.push(ignored);
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Npc,
        index: 8,
    });
    scene.refresh();
    let allocations = allocation_counter::measure(|| attack(harness.pending(&scene, 3)));
    assert_eq!(allocations.count_total, 2);
    assert_eq!(harness.machine.counters.restorations, 0);
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    assert!(harness.take().is_none());
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 4).is_none());
}

#[test]
fn case39_synthetic_wire_cost_driver_bounds_fifty_fight_ticks() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(5, 0, 43);
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.inventory.push(scene.held("lobster", 1));
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let origin = scene.npcs[0].tile;
    let mut drink_tick = None;
    let mut food_tick = None;
    let mut prayer_tick = None;
    let mut routed = Vec::new();
    let mut multi_plan_count = 0u8;
    for tick in 3..53 {
        if [20, 35].contains(&tick) {
            scene.npcs[0].tile.x = origin.x + 4;
            scene.npcs[0].distance = 5;
            scene.local.player.actor.animation = -1;
        } else if [23, 38].contains(&tick) {
            scene.npcs[0].tile = origin;
            scene.npcs[0].distance = 1;
        }
        if tick == 11 {
            scene.stat(3, 19, 40);
            scene.stat(5, 0, 43);
            scene.prayer(protect.varp, false);
        }
        if tick % 4 == 0 && scene.npcs[0].distance == 1 {
            scene.local.player.actor.animation = scene.melee_seq();
            scene.local.player.actor.animation_frame = 0;
        } else {
            scene.local.player.actor.animation_frame += 1;
        }
        scene.refresh();
        let plan = harness.pending_batch(&scene, tick);
        let mut wire_events = 0;
        for index in 0..plan.len() {
            match plan.get(index) {
                Some(HostEffect::Interaction(InteractReq::Npc { action, .. }))
                    if action == "Attack" =>
                {
                    assert_eq!(index + 1, plan.len(), "Attack is the terminal plan row");
                    wire_events += 2;
                    routed.push(tick);
                    scene.install();
                }
                Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                    assert_eq!(*component_id, protect.button_com);
                    wire_events += 1;
                    prayer_tick = Some(tick);
                    scene.prayer(protect.varp, true);
                }
                Some(HostEffect::Interaction(InteractReq::Held { action, .. }))
                    if action == "Drink" =>
                {
                    wire_events += 1;
                    drink_tick = Some(tick);
                    scene.inventory.retain(|row| {
                        row.def.id != scene.data.item_by_alias("1doseprayerrestore").unwrap().id
                    });
                    scene.stat(5, 17, 43);
                }
                Some(HostEffect::Interaction(InteractReq::Held { action, .. }))
                    if action == "Eat" =>
                {
                    wire_events += 1;
                    food_tick = Some(tick);
                    scene.inventory.clear();
                    scene.stat(3, 31, 40);
                }
                _ => panic!("unexpected synthetic-driver request"),
            }
        }
        if plan.len() >= 2 {
            multi_plan_count = multi_plan_count.saturating_add(1);
        }
        assert!(
            wire_events <= 5,
            "tick {tick} has {wire_events} user events"
        );
        if let Some(drink) = drink_tick {
            if tick == drink + 1 || tick == drink + 2 {
                assert_eq!(plan.len(), 0, "the potion lock excludes every plan row");
            }
        }
    }
    assert_eq!(harness.machine.counters.multi, multi_plan_count);
    assert_eq!(drink_tick, Some(3));
    assert_eq!(prayer_tick, Some(6));
    assert_eq!(food_tick, Some(11));
    assert!(routed.contains(&6));
    assert!(routed.contains(&11));
    assert_eq!(harness.machine.counters.locked, 2);
    assert_eq!(harness.machine.counters.restorations, 2);
}
#[test]
fn case41_protect_on_ready_deadline_restores_swing_same_plan() {
    let mut scene = Scene::new("khazard_warlord");
    // Protection is available, while the current points stay below the
    // offensive-prayer floor so the ready-tick fixture has no unrelated upkeep.
    scene.stat(5, 20, 43);
    scene.refresh();
    let mut harness = fight(&mut scene);
    let melee = scene.melee_seq();
    for tick in 3..8 {
        if tick >= 4 {
            scene.local.player.actor.animation = melee;
            scene.local.player.actor.animation_frame = (tick - 4) as i32;
            scene.refresh();
        }
        assert!(harness.pending(&scene, tick).is_none());
    }
    scene.local.player.actor.animation = melee;
    scene.local.player.actor.animation_frame = 0;
    scene.refresh();
    assert!(harness.pending(&scene, 8).is_none());
    assert_eq!(harness.machine.cycle().deadline, 12);
    for (tick, frame) in [(9, 1), (10, 2)] {
        scene.local.player.actor.animation_frame = frame;
        scene.refresh();
        assert!(harness.pending(&scene, tick).is_none());
    }

    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    scene.face_us();
    scene.npcs[0].animation = melee;
    scene.npcs[0].animation_frame = 0;
    scene.local.player.actor.animation_frame = 3;
    // The threat onset is observed at 11, but prayer is available only at 12.
    scene.stat(5, 0, 43);
    scene.refresh();
    assert!(harness.pending(&scene, 11).is_none());
    scene.npcs[0].animation_frame = 1;
    scene.stat(5, 20, 43);
    scene.refresh();

    // A single ordered batch puts Protect and the due restoring Attack into
    // the tick-12 input phase. The client exposes that newly installed swing
    // on the next PLAYER_INFO observation (13); evidence-based product timing
    // therefore records deadline 17. The old single-op path delivered Attack
    // at 13 and first observed its swing at 14 (deadline 18).
    let plan = harness.pending_batch(&scene, 12);
    assert_len(&plan, 2);
    prayer_row(&plan, 0, protect.button_com);
    attack_row(&plan, 1);

    scene.prayer(protect.varp, true);
    scene.install();
    scene.local.player.actor.animation = melee;
    scene.local.player.actor.animation_frame = 0;
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 13), 0);
    assert_eq!(harness.machine.cycle().deadline, 17);
    assert!(harness.machine.cycle().known);
    assert_eq!(harness.machine.counters.restorations, 1);
    assert_eq!(harness.machine.counters.multi, 1);
}

#[test]
fn case42_offensives_fill_five_events_without_clearing_user_prayer() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 40, 43);
    scene.face_us();
    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let strength = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Ultimate Strength")
        .unwrap()
        .clone();
    let attack = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Incredible Reflexes")
        .unwrap()
        .clone();
    let extra = scene
        .data
        .prayers()
        .iter()
        .find(|row| {
            ![
                "Protect from Melee",
                "Ultimate Strength",
                "Incredible Reflexes",
            ]
            .contains(&row.name.as_str())
        })
        .unwrap()
        .clone();
    scene.prayer(extra.varp, true);
    scene.refresh();

    let full = harness.pending_batch(&scene, 3);
    assert_len(&full, 4);
    prayer_row(&full, 0, protect.button_com);
    prayer_row(&full, 1, strength.button_com);
    prayer_row(&full, 2, attack.button_com);
    attack_row(&full, 3);
    assert_eq!(harness.machine.plan.events, 5);
    assert_eq!(harness.machine.counters.multi, 1);

    for row in [&protect, &strength, &attack] {
        scene.prayer(row.varp, true);
    }
    scene.install();
    scene.refresh();
    let deferred = harness.pending_batch(&scene, 4);
    assert_len(&deferred, 0);
    assert_eq!(harness.machine.counters.multi, 1);
    assert!(scene
        .varps
        .iter()
        .any(|row| row.index == extra.varp && row.value == 1));
}

#[test]
fn case46_retaliate_turns_on_before_the_fight_attack() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::OPTION_NODEF)
        .unwrap()
        .value = 1;
    scene.refresh();

    let plan = harness.pending_batch(&scene, 3);
    assert_len(&plan, 2);
    assert!(matches!(
        plan.get(0),
        Some(HostEffect::Interaction(InteractReq::SetRetaliate {
            on: true
        }))
    ));
    attack_row(&plan, 1);
    assert_eq!(harness.machine.plan.events, 3);

    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::OPTION_NODEF)
        .unwrap()
        .value = 0;
    scene.install();
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 4), 0);
    assert_eq!(harness.machine.counters.multi, 1);
}

#[test]
fn case47_winddown_preserves_six_unowned_prayers_after_a_drink_lock() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(5, 0, 43);
    scene.inventory.push(scene.held("1doseprayerrestore", 0));
    scene.refresh();
    held(harness.pending(&scene, 3), "Drink");
    assert_eq!(harness.machine.input_lock(), Some(6));
    scene.inventory.clear();
    scene.stat(5, 17, 43);

    let prayers: Vec<_> = scene
        .data
        .prayers()
        .iter()
        .take(6)
        .map(|row| (row.varp, row.button_com))
        .collect();
    scene.npcs[0].health = 0;
    for (varp, _) in &prayers {
        scene.prayer(*varp, true);
    }
    scene.refresh();
    assert_len(&harness.pending_batch(&scene, 4), 0);
    assert_len(&harness.pending_batch(&scene, 5), 0);
    assert_eq!(harness.machine.counters.locked, 2);
    let obligated_before_cleanup = harness.machine.counters.restorations;

    let report = harness.ready(&scene, 6);
    assert!(prayers.iter().all(|(varp, _)| scene
        .varps
        .iter()
        .any(|row| row.index == *varp && row.value == 1)));
    assert_eq!(report.end, CombatEnd::Killed);
    assert_eq!(report.locked_ticks, 2);
    assert_eq!(report.restorations, obligated_before_cleanup);
}

#[test]
fn case49_fail_stop_preserves_accepted_food_and_retries_the_restoration() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 9, 40);
    scene.stat(5, 1, 43);
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();

    let refused = harness.pending_with_refusal(&scene, 3, 1);
    assert_len(&refused, 3);
    held_row(&refused, 0, "Eat");
    prayer_row(
        &refused,
        1,
        scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == "Protect from Melee")
            .unwrap()
            .button_com,
    );
    attack_row(&refused, 2);
    assert!(refused.accepted[0]);
    assert!(refused.dispatched[1]);
    assert!(!refused.accepted[1]);
    assert!(!refused.dispatched[2]);
    assert!(!refused.accepted[2]);
    assert_eq!(harness.machine.counters.food, 0);
    assert_eq!(harness.machine.counters.restorations, 0);

    scene.inventory.clear();
    scene.stat(3, 21, 40);
    scene.refresh();
    let retry = harness.pending_batch(&scene, 4);
    assert_len(&retry, 2);
    prayer_row(
        &retry,
        0,
        scene
            .data
            .prayers()
            .iter()
            .find(|row| row.name == "Protect from Melee")
            .unwrap()
            .button_com,
    );
    attack_row(&retry, 1);
    assert_eq!(harness.machine.counters.food, 1);
    assert_eq!(harness.machine.counters.restorations, 1);
}

#[test]
fn case50_foreign_event_spills_attack_without_same_tick_reemission() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(5, 40, 43);
    scene.refresh();

    let protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .clone();
    let strength = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Ultimate Strength")
        .unwrap()
        .clone();
    let attack = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Incredible Reflexes")
        .unwrap()
        .clone();
    let batch = harness.pending_batch(&scene, 3);
    assert_len(&batch, 4);
    prayer_row(&batch, 0, protect.button_com);
    prayer_row(&batch, 1, strength.button_com);
    prayer_row(&batch, 2, attack.button_com);
    attack_row(&batch, 3);
    assert_eq!(harness.machine.plan.events, 5);

    // One already-queued foreign event consumes the first wire slot. The
    // first four plan events (three clicks plus MOVE_OPCLICK) land at tick 3;
    // only the terminal OPNPC2 crosses the engine's five-handler boundary.
    let mut delivered_events = 1usize;
    let mut spilled_attack = false;
    for index in 0..batch.len() {
        let cost = if index == 3 { 2 } else { 1 };
        let accepted_now = cost.min(5 - delivered_events);
        delivered_events += accepted_now;
        if index == 3 && accepted_now < cost {
            spilled_attack = true;
        }
    }
    assert_eq!(delivered_events, 5);
    assert!(spilled_attack);

    scene.prayer(protect.varp, true);
    scene.prayer(strength.varp, true);
    scene.prayer(attack.varp, true);
    scene.install();
    scene.refresh();
    let settled = harness.pending_batch(&scene, 4);
    assert_len(&settled, 0);
    assert_eq!(harness.machine.counters.multi, 1);
    assert!(!harness.machine.pending_row(RowKind::Attack));
}

fn observe_melee_mode(scene: &mut Scene, value: u8) {
    let index = scene.tables.melee_mode_varp().unwrap();
    if let Some(row) = scene.varps.iter_mut().find(|row| row.index == index) {
        row.value = i32::from(value);
    } else {
        scene.varps.push(VarpView {
            index,
            value: i32::from(value),
        });
    }
}

fn weapon_tab_root(scene: &Scene, weapon_id: i32) -> i32 {
    let tab = scene
        .tables
        .weapon_style(weapon_id)
        .unwrap()
        .tab
        .expect("weapon combat tab");
    scene.tables.combat_tab_root(tab).unwrap()
}

fn style_button(scene: &Scene, weapon_id: i32, observed: u8) -> i32 {
    let tab = scene
        .tables
        .weapon_style(weapon_id)
        .unwrap()
        .tab
        .expect("weapon combat tab");
    scene
        .tables
        .melee_mode(tab, MeleeMode::Aggressive, Some(observed))
        .unwrap()
        .button
}

fn settle_weapon(scene: &mut Scene, weapon_id: i32) {
    let index = scene
        .inventory
        .iter()
        .position(|item| item.def.id == weapon_id)
        .unwrap();
    let mut weapon = scene.inventory.remove(index);
    weapon.slot = 3;
    weapon.container = ItemContainer::Equipment;
    scene.equipment.retain(|item| item.slot != 3);
    scene.equipment.push(weapon);
}

fn measured_weapon_rate(harness: &Harness, scene: &Scene) -> u8 {
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        harness.runtime.evidence.unwrap(),
    ))
    .unwrap();
    harness.machine.rate(&frame)
}

#[test]
fn case45_wear_style_waits_for_the_exact_observed_combat_tab() {
    let mut same_tab = Scene::new("imp");
    let bronze = same_tab.held("bronze_scimitar", 3);
    let bronze_id = bronze.def.id;
    let mut equipped = bronze;
    equipped.container = ItemContainer::Equipment;
    same_tab.equipment.push(equipped);
    let rune = same_tab.held("rune_scimitar", 0);
    let rune_id = rune.def.id;
    same_tab.inventory.push(rune.clone());
    let (bronze_category, bronze_tab) = {
        let fact = same_tab.tables.weapon_style(bronze_id).unwrap();
        (fact.category, fact.tab)
    };
    let (rune_category, rune_tab) = {
        let fact = same_tab.tables.weapon_style(rune_id).unwrap();
        (fact.category, fact.tab)
    };
    assert_eq!(bronze_category, rune_category);
    assert_eq!(bronze_tab, rune_tab);
    let hack_root = same_tab
        .tables
        .combat_tab_root(bronze_tab.expect("scimitar combat tab"))
        .unwrap();
    observe_melee_mode(&mut same_tab, 0);
    same_tab.refresh();
    same_tab.combat_tab(hack_root);
    let mut request = same_tab.request();
    request.melee_mode = Some(MeleeMode::Aggressive);
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, rune_id)]),
        ..CompiledKit::default()
    }));
    let mut same_harness = Harness::new(&same_tab, request);
    same_harness.machine.phase = Phase::Fight;

    let same = same_harness.pending_batch(&same_tab, 1);
    assert_len(&same, 3);
    assert!(
        matches!(same.get(0), Some(HostEffect::Interaction(InteractReq::Wear { name })) if Some(name.as_str()) == rune.def.name.as_deref())
    );
    prayer_row(&same, 1, style_button(&same_tab, bronze_id, 0));
    attack_row(&same, 2);
    assert_eq!(same_harness.machine.plan.rows[2].aux, 4);
    settle_weapon(&mut same_tab, rune_id);
    same_tab.install();
    let aggressive = same_tab
        .tables
        .melee_mode(
            rune_tab.expect("scimitar combat tab"),
            MeleeMode::Aggressive,
            Some(0),
        )
        .unwrap();
    observe_melee_mode(&mut same_tab, aggressive.slot);
    same_tab.refresh();
    same_tab.combat_tab(hack_root);
    assert_len(&same_harness.pending_batch(&same_tab, 2), 0);

    let mut changed_tab = Scene::new("imp");
    let old = changed_tab.held("bronze_scimitar", 3);
    let old_id = old.def.id;
    let mut equipped = old;
    equipped.container = ItemContainer::Equipment;
    changed_tab.equipment.push(equipped);
    let twohand = changed_tab.held("bronze_2h_sword", 0);
    let twohand_id = twohand.def.id;
    changed_tab.inventory.push(twohand.clone());
    let old_fact = changed_tab.tables.weapon_style(old_id).unwrap();
    let twohand_fact = changed_tab.tables.weapon_style(twohand_id).unwrap();
    assert_eq!(old_fact.category, twohand_fact.category);
    assert_ne!(old_fact.tab, twohand_fact.tab);
    let old_root = changed_tab
        .tables
        .combat_tab_root(old_fact.tab.expect("old weapon combat tab"))
        .unwrap();
    let heavy_root = changed_tab
        .tables
        .combat_tab_root(twohand_fact.tab.expect("new weapon combat tab"))
        .unwrap();
    observe_melee_mode(&mut changed_tab, 0);
    changed_tab.refresh();
    changed_tab.combat_tab(old_root);
    let mut request = changed_tab.request();
    request.melee_mode = Some(MeleeMode::Aggressive);
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, twohand_id)]),
        ..CompiledKit::default()
    }));
    let mut changed_harness = Harness::new(&changed_tab, request);
    changed_harness.machine.phase = Phase::Fight;

    let first = changed_harness.pending_batch(&changed_tab, 1);
    assert_len(&first, 2);
    assert!(
        matches!(first.get(0), Some(HostEffect::Interaction(InteractReq::Wear { name })) if Some(name.as_str()) == twohand.def.name.as_deref())
    );
    attack_row(&first, 1);
    assert_eq!(changed_harness.machine.plan.rows[1].aux, 7);
    settle_weapon(&mut changed_tab, twohand_id);
    changed_tab.install();
    changed_tab.refresh();
    changed_tab.combat_tab(old_root);
    assert_len(&changed_harness.pending_batch(&changed_tab, 2), 0);
    assert_eq!(measured_weapon_rate(&changed_harness, &changed_tab), 7);

    changed_tab.combat_tab(heavy_root);
    let style_tick = changed_harness.pending_batch(&changed_tab, 3);
    assert_len(&style_tick, 2);
    prayer_row(&style_tick, 0, style_button(&changed_tab, twohand_id, 0));
    attack_row(&style_tick, 1);
    assert_eq!(changed_harness.machine.plan.rows[1].aux, 7);

    let mut stale_tab = Scene::new("imp");
    let heavy = stale_tab.held("bronze_2h_sword", 3);
    let heavy_id = heavy.def.id;
    let mut equipped = heavy;
    equipped.container = ItemContainer::Equipment;
    stale_tab.equipment.push(equipped);
    let rune = stale_tab.held("rune_scimitar", 0);
    let rune_id = rune.def.id;
    stale_tab.inventory.push(rune.clone());
    let heavy_root = weapon_tab_root(&stale_tab, heavy_id);
    let hack_root = weapon_tab_root(&stale_tab, rune_id);
    observe_melee_mode(&mut stale_tab, 0);
    stale_tab.refresh();
    stale_tab.combat_tab(heavy_root);
    let mut request = stale_tab.request();
    request.melee_mode = Some(MeleeMode::Aggressive);
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, rune_id)]),
        ..CompiledKit::default()
    }));
    let mut stale_harness = Harness::new(&stale_tab, request);
    stale_harness.machine.phase = Phase::Fight;

    let first = stale_harness.pending_batch(&stale_tab, 1);
    assert_len(&first, 2);
    assert!(
        matches!(first.get(0), Some(HostEffect::Interaction(InteractReq::Wear { name })) if Some(name.as_str()) == rune.def.name.as_deref())
    );
    attack_row(&first, 1);
    settle_weapon(&mut stale_tab, rune_id);
    stale_tab.install();
    stale_tab.refresh();
    stale_tab.combat_tab(heavy_root);
    assert_len(&stale_harness.pending_batch(&stale_tab, 2), 0);
    stale_tab.combat_tab(hack_root);
    let observed = stale_harness.pending_batch(&stale_tab, 3);
    assert_len(&observed, 2);
    prayer_row(&observed, 0, style_button(&stale_tab, rune_id, 0));
    attack_row(&observed, 1);
}

#[test]
fn case53_script_refused_recovery_food_never_enters_a_drink_lock() {
    let mut scene = Scene::new("khazard_warlord");
    scene.face_us();
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.inventory.push(scene.held("lobster", 0));
    scene.inventory.push(scene.held("1doseprayerrestore", 1));
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());

    // The prior owner may have written %eat_delay at T-1. The first poll
    // conservatively emits the recovery food alone; the harness accepts the
    // request while the synthetic script leaves its count and HP unchanged.
    let first = harness.pending_batch(&scene, 1);
    assert_len(&first, 2);
    held_row(&first, 0, "Eat");
    attack_row(&first, 1);
    assert_eq!(harness.machine.eat_ready(), 3);
    assert_eq!(harness.machine.input_lock(), None);

    scene.install();
    scene.refresh();
    let refused_by_script = harness.pending_batch(&scene, 2);
    assert_len(&refused_by_script, 0);
    assert!(harness.machine.pending_row(RowKind::Eat));
    assert_eq!(harness.machine.input_lock(), None);
    assert_eq!(harness.machine.counters.locked, 0);
}

#[test]
fn case54_readiness_covers_first_poll_handoff_gap_and_spilled_eat() {
    let scene = Scene::new("imp");
    let mut first_owner = Harness::new_at(&scene, scene.request(), 0);
    let initial = first_owner.pending_batch(&scene, 1);
    assert_len(&initial, 1);
    attack_row(&initial, 0);
    assert_eq!(first_owner.machine.eat_ready(), 3);

    let scene = Scene::new("imp");
    let mut request = scene.request();
    request.until_ticks = 1;
    let mut ending_owner = Harness::new_at(&scene, request, 0);
    assert_eq!(ending_owner.ready(&scene, 1).end, CombatEnd::Budget);
    let mut next_owner = Harness::new_at(&scene, scene.request(), 1);
    let handoff = next_owner.pending_batch(&scene, 2);
    assert_len(&handoff, 1);
    attack_row(&handoff, 0);
    assert_eq!(next_owner.machine.eat_ready(), 4);

    let mut scene = Scene::new("imp");
    let mut gap = fight(&mut scene);
    let missing = {
        let saved = std::mem::replace(&mut scene.snapshot, GameSnapshot::new());
        scene.refresh_without_local();
        std::mem::replace(&mut scene.snapshot, saved)
    };
    assert!(matches!(gap.poll(&missing, 5), Poll::Pending));
    assert!(gap.take().is_none());
    scene.refresh();
    let after_gap = gap.pending_batch(&scene, 6);
    assert_len(&after_gap, 1);
    attack_row(&after_gap, 0);
    assert_eq!(gap.machine.eat_ready(), 8);
    assert_eq!(gap.machine.schedule.interaction, Interaction::Unknown);
    assert!(!gap.machine.schedule.cycle.known);

    let mut scene = Scene::new("khazard_warlord");
    let mut own = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 9, 40);
    let mut lobster = scene.held("lobster", 0);
    lobster.count = 2;
    scene.inventory.push(lobster);
    scene.refresh();
    let eat = own.pending_batch(&scene, 3);
    assert_len(&eat, 2);
    held_row(&eat, 0, "Eat");
    attack_row(&eat, 1);
    scene.install();
    scene.inventory[0].count = 1;
    scene.stat(3, 21, 40);
    scene.refresh();
    assert_len(&own.pending_batch(&scene, 4), 0);
    assert_eq!(own.machine.eat_ready(), 6);

    let mut scene = Scene::new("khazard_warlord");
    scene.face_us();
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    let mut lobster = scene.held("lobster", 0);
    lobster.count = 2;
    scene.inventory.push(lobster);
    scene.inventory.push(scene.held("1doseprayerrestore", 1));
    scene.refresh();
    let mut spilled = Harness::new_at(&scene, scene.request(), 0);
    let emitted = spilled.pending_batch(&scene, 1);
    assert_len(&emitted, 2);
    held_row(&emitted, 0, "Eat");
    attack_row(&emitted, 1);
    assert_eq!(spilled.machine.eat_ready(), 3);
    // One hit of nine after the spilled eat leaves HP at 28. Keep the pack
    // baseline stale through E+1, then expose its decrease at E+2.
    scene.install();
    scene.stat(3, 28, 40);
    scene.refresh();
    assert_len(&spilled.pending_batch(&scene, 2), 0);
    scene.inventory[0].count = 1;
    scene.refresh();
    assert_len(&spilled.pending_batch(&scene, 3), 0);
    assert_eq!(spilled.machine.eat_ready(), 5);
    scene.stat(3, 28, 40);
    scene.refresh();
    let before_proven = spilled.pending_batch(&scene, 4);
    assert_len(&before_proven, 2);
    held_row(&before_proven, 0, "Eat");
    attack_row(&before_proven, 1);
    assert_eq!(spilled.machine.eat_ready(), 5);
    assert_eq!(spilled.machine.input_lock(), None);
}

#[test]
fn case44_ordinary_and_combo_food_share_only_a_proven_safe_lock() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 16, 40);
    let mut lobster = scene.held("lobster", 0);
    lobster.count = 1;
    let mut karambwan = scene.held("tbwt_cooked_karambwan", 1);
    karambwan.count = 1;
    scene.inventory.extend([lobster, karambwan]);
    scene.local.player.actor.animation = scene.melee_seq();
    scene.local.player.actor.animation_frame = 0;
    scene.refresh();

    let combo = harness.pending_batch(&scene, 3);
    assert_len(&combo, 2);
    assert!(matches!(
        combo.get(0),
        Some(HostEffect::Interaction(InteractReq::Held { action, slot: Some(0), .. })) if action == "Eat"
    ));
    assert!(matches!(
        combo.get(1),
        Some(HostEffect::Interaction(InteractReq::Held { action, slot: Some(1), .. })) if action == "Eat"
    ));
    assert_eq!(harness.machine.input_lock(), Some(7));
    assert_eq!(harness.machine.cycle().deadline, 10);
    assert_eq!(harness.machine.counters.multi, 1);

    scene.inventory.clear();
    scene.stat(3, 40, 40);
    scene.refresh();
    for tick in 4..7 {
        assert_len(&harness.pending_batch(&scene, tick), 0);
    }
    assert_eq!(harness.machine.counters.food, 2);
    assert_eq!(harness.machine.counters.locked, 3);
    attack(harness.pending(&scene, 7));
    assert_eq!(harness.machine.counters.restorations, 1);

    let mut strict = Scene::new("khazard_warlord");
    let mut strict_harness = fight(&mut strict);
    strict.face_us();
    strict.stat(3, 7, 40);
    strict.inventory.push(strict.held("lobster", 0));
    strict
        .inventory
        .push(strict.held("tbwt_cooked_karambwan", 1));
    strict.refresh();
    let plain = strict_harness.pending_batch(&strict, 3);
    assert_len(&plain, 2);
    held_row(&plain, 0, "Eat");
    attack_row(&plain, 1);
    assert_eq!(strict_harness.machine.input_lock(), None);
}

fn leash_fight(scene: &mut Scene, budget_ticks: u16) -> Harness {
    let stand = scene.local.player.actor.tile;
    let mut request = scene.request();
    request.stand = Some(stand);
    request.lost_radius = 12;
    request.budget_ticks = budget_ticks;
    let mut harness = Harness::new(scene, request);
    attack(harness.pending(scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(scene, 2).is_none());
    assert_eq!(harness.machine.phase, Phase::Fight);
    harness
}

#[test]
fn stand_leash_loss_walks_off_the_server_chase_before_target_gone() {
    let mut scene = Scene::new("imp");
    let stand = scene.local.player.actor.tile;
    let mut harness = leash_fight(&mut scene, 50);
    scene.npcs[0].tile = tile(stand.x + 13, stand.z);
    scene.npcs[0].distance = 13;
    scene.refresh();

    let mut walked = false;
    for tick in 3..=5 {
        match harness.pending(&scene, tick) {
            Some(HostEffect::Walk(request)) => {
                assert_eq!(request.target, tile(stand.x - 2, stand.z));
                assert_eq!(request.radius, 1);
                walked = true;
            }
            Some(HostEffect::Interaction(InteractReq::Npc { action, .. }))
                if action == "Attack" =>
            {
                panic!("an out-of-leash target must not receive another Attack");
            }
            _ => {}
        }
    }
    assert!(
        walked,
        "leash cancellation must move the player off the live chase"
    );
    assert_eq!(harness.ready(&scene, 6).end, CombatEnd::TargetGone);
}

#[test]
fn leash_cancellation_grace_kill_beats_the_exact_budget_boundary() {
    let mut scene = Scene::new("imp");
    let stand = scene.local.player.actor.tile;
    let mut harness = leash_fight(&mut scene, 4);
    scene.npcs[0].tile = tile(stand.x + 13, stand.z);
    scene.npcs[0].distance = 13;
    scene.refresh();
    assert!(matches!(
        harness.pending(&scene, 3),
        Some(HostEffect::Walk(request))
            if request.target == tile(stand.x - 2, stand.z)
    ));

    scene.npcs[0].health = 0;
    scene.refresh();
    assert_eq!(harness.ready(&scene, 4).end, CombatEnd::Killed);
}

#[test]
fn user_input_ends_combat_approach_without_rewalk_or_attack() {
    let mut scene = Scene::new("imp");
    let here = scene.local.player.actor.tile;
    let stand = tile(here.x + 20, here.z);
    scene.npcs[0].tile = tile(stand.x + 10, stand.z);
    scene.npcs[0].distance = 30;
    scene.refresh();
    let mut request = scene.request();
    request.stand = Some(stand);
    request.engage_radius = 1;
    let mut harness = Harness::new(&scene, request);

    assert!(matches!(
        harness.pending(&scene, 1),
        Some(HostEffect::Walk(walk)) if walk.target == stand
    ));
    user_input_walk_receipt(&mut harness, 1);
    assert!(matches!(
        harness.poll_stamp(&scene.snapshot, 1, 2),
        Poll::Ready(Err(ActionError::UserInput))
    ));
    assert_len(&harness.drain(None), 0);
}

#[test]
fn same_tick_combat_approach_defers_after_an_interaction_spends_budget() {
    let mut scene = Scene::new("imp");
    let here = scene.local.player.actor.tile;
    let stand = tile(here.x + 20, here.z);
    scene.npcs[0].tile = tile(stand.x + 10, stand.z);
    scene.npcs[0].distance = 30;
    scene.refresh();
    let mut request = scene.request();
    request.stand = Some(stand);
    request.engage_radius = 1;
    let mut harness = Harness::new(&scene, request);
    harness.runtime.context(&scene.snapshot, 1, 1, |_, cx| {
        cx.emit(InteractReq::CloseModal).unwrap();
    });
    assert!(harness.poll_stamp(&scene.snapshot, 1, 2).is_pending());
    assert!(harness.machine.pending_walk.is_none());
    assert_eq!(harness.runtime.ledger.as_ref().unwrap().outbox.len(), 1);
    assert!(harness.poll_stamp(&scene.snapshot, 1, 3).is_pending());
    assert!(harness.poll_stamp(&scene.snapshot, 2, 4).is_pending());
    assert!(harness.machine.pending_walk.is_some());
    assert!(matches!(
        &harness.runtime.ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Walk(walk) if walk.target == stand
    ));
}

#[test]
fn user_input_ends_combat_retreat_without_relatching_failure_or_rewalking() {
    let mut scene = Scene::new("khazard_warlord");
    let here = scene.local.player.actor.tile;
    let goal = tile(here.x + 20, here.z);
    let mut request = scene.request();
    request.fallback = Fallback::Retreat { tile: goal };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.face_us();
    scene.stat(3, 9, 40);
    scene.refresh();

    assert!(matches!(
        harness.pending(&scene, 2),
        Some(HostEffect::Interaction(InteractReq::SetRetaliate {
            on: false
        }))
    ));
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == super::super::OPTION_NODEF)
        .unwrap()
        .value = 1;
    scene.refresh();
    assert!(matches!(
        harness.pending(&scene, 3),
        Some(HostEffect::Walk(walk)) if walk.target == goal
    ));
    user_input_walk_receipt(&mut harness, 3);
    assert!(matches!(
        harness.poll_stamp(&scene.snapshot, 3, 4),
        Poll::Ready(Err(ActionError::UserInput))
    ));
    assert_len(&harness.drain(None), 0);
}

#[test]
fn user_input_ends_combat_leash_walk_without_retarget_or_rewalk() {
    let mut scene = Scene::new("imp");
    let stand = scene.local.player.actor.tile;
    let mut harness = leash_fight(&mut scene, 50);
    scene.npcs[0].tile = tile(stand.x + 13, stand.z);
    scene.npcs[0].distance = 13;
    scene.refresh();

    let walk_tick = (3..=5).find(|tick| match harness.pending(&scene, *tick) {
        Some(HostEffect::Walk(walk)) => {
            assert_eq!(walk.target, tile(stand.x - 2, stand.z));
            true
        }
        Some(HostEffect::Interaction(InteractReq::Npc { action, .. })) if action == "Attack" => {
            panic!("an out-of-leash target must not receive another Attack");
        }
        _ => false,
    });
    let tick = walk_tick.expect("lost leash cancels the chase with a walk");
    user_input_walk_receipt(&mut harness, tick);
    assert!(matches!(
        harness.poll_stamp(&scene.snapshot, tick, tick.saturating_add(1)),
        Poll::Ready(Err(ActionError::UserInput))
    ));
    assert_len(&harness.drain(None), 0);
}

fn policy_s2_incoming(scene: &mut Scene, spotanim: i32) {
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim,
            level: 0,
            src: scene.npcs[0].tile,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: scene.local.player.index,
            }),
            t1: 0,
            t2: 30,
        }]);
}

fn policy_s2_buttons(batch: &DrainedBatch) -> Vec<i32> {
    batch.effects[..batch.len()]
        .iter()
        .filter_map(|effect| match effect {
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                Some(*component_id)
            }
            _ => None,
        })
        .collect()
}

#[test]
fn policy_s2_sweep_preserves_user_prayers_and_never_restores_displaced_protect() {
    for switch_back in [false, true] {
        let mut scene = Scene::new("imp");
        scene.stat(5, 43, 43);
        let skin = scene.data.prayer_by_name("Thick Skin").unwrap().clone();
        let original = scene
            .data
            .prayer_by_name(if switch_back {
                "Protect from Missiles"
            } else {
                "Protect from Melee"
            })
            .unwrap()
            .clone();
        let replacement = scene
            .data
            .prayer_by_name(if switch_back {
                "Protect from Magic"
            } else {
                "Protect from Missiles"
            })
            .unwrap()
            .clone();
        scene.prayer(skin.varp, true);
        scene.prayer(original.varp, true);
        scene.face_us();
        scene.refresh();
        policy_s2_incoming(&mut scene, if switch_back { 88 } else { 9 });
        let mut harness = Harness::new(&scene, scene.request());

        let first = harness.pending_batch(&scene, 1);
        assert_eq!(policy_s2_buttons(&first), vec![replacement.button_com]);
        scene.install();
        scene.prayer(original.varp, false);
        scene.prayer(replacement.varp, true);
        scene.refresh();
        policy_s2_incoming(&mut scene, if switch_back { 88 } else { 9 });
        assert!(policy_s2_buttons(&harness.pending_batch(&scene, 2)).is_empty());

        let cleanup_prayer = if switch_back {
            scene.refresh();
            policy_s2_incoming(&mut scene, 9);
            let back = harness.pending_batch(&scene, 3);
            assert_eq!(policy_s2_buttons(&back), vec![original.button_com]);
            scene.prayer(replacement.varp, false);
            scene.prayer(original.varp, true);
            scene.refresh();
            original.clone()
        } else {
            replacement.clone()
        };
        scene.npcs[0].health = 0;
        scene.refresh();
        let cleanup = harness.pending_batch(&scene, 4);
        assert_eq!(policy_s2_buttons(&cleanup), vec![cleanup_prayer.button_com]);
        scene.prayer(cleanup_prayer.varp, false);
        scene.refresh();
        assert_eq!(harness.ready(&scene, 5).end, CombatEnd::Killed);
        assert_eq!(
            prayer_observation(
                &Frame::borrow(api::snapshot::SnapshotView::new(
                    Some(&scene.snapshot),
                    harness.runtime.evidence.unwrap()
                ))
                .unwrap()
            )
            .varp(skin.varp),
            1
        );
        assert_eq!(
            scene
                .varps
                .iter()
                .find(|row| row.index == original.varp)
                .unwrap()
                .value,
            0,
            "the displaced protect is not restored, including after a Combat-owned re-raise"
        );
    }
}

#[test]
fn policy_s2_incomplete_baseline_emits_no_toggle_before_delayed_user_row() {
    let mut scene = Scene::new("imp");
    scene.stat(5, 43, 43);
    let skin = scene.data.prayer_by_name("Thick Skin").unwrap().clone();
    let protect = scene
        .data
        .prayer_by_name("Protect from Melee")
        .unwrap()
        .clone();
    scene.varps.retain(|row| row.index != skin.varp);
    scene.face_us();
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    assert!(policy_s2_buttons(&harness.pending_batch(&scene, 1)).is_empty());
    scene.install();
    scene.refresh();
    assert!(policy_s2_buttons(&harness.pending_batch(&scene, 2)).is_empty());

    scene.varps.push(VarpView {
        index: skin.varp,
        value: 1,
    });
    scene.refresh();
    let ready = harness.pending_batch(&scene, 3);
    assert_eq!(policy_s2_buttons(&ready), vec![protect.button_com]);
    scene.prayer(protect.varp, true);
    scene.npcs[0].health = 0;
    scene.refresh();
    let cleanup = harness.pending_batch(&scene, 4);
    assert_eq!(policy_s2_buttons(&cleanup), vec![protect.button_com]);
    scene.prayer(protect.varp, false);
    scene.refresh();
    assert_eq!(harness.ready(&scene, 5).end, CombatEnd::Killed);
    assert_eq!(
        scene
            .varps
            .iter()
            .find(|row| row.index == skin.varp)
            .unwrap()
            .value,
        1
    );
}

#[test]
fn policy_s2_offensive_tier_never_displaces_user_strength() {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(5, 43, 43);
    let burst = scene
        .data
        .prayer_by_name("Burst of Strength")
        .unwrap()
        .clone();
    let ultimate = scene
        .data
        .prayer_by_name("Ultimate Strength")
        .unwrap()
        .clone();
    scene.prayer(burst.varp, true);
    scene.face_us();
    scene.refresh();
    let mut harness = Harness::new(&scene, scene.request());
    let first = harness.pending_batch(&scene, 1);
    let mut buttons = policy_s2_buttons(&first);
    for row in scene.data.prayers().to_vec() {
        if buttons.contains(&row.button_com) {
            scene.prayer(row.varp, true);
        }
    }
    scene.install();
    scene.refresh();
    buttons.extend(policy_s2_buttons(&harness.pending_batch(&scene, 2)));
    assert!(
        !buttons.contains(&burst.button_com),
        "user prayer must not be switched off"
    );
    assert!(
        !buttons.contains(&ultimate.button_com),
        "higher tier would displace the user prayer"
    );
    assert!(first.len() > 0, "the engaged machine still makes progress");
}

#[test]
fn policy_s2_refused_switch_leaves_baseline_unowned_at_winddown() {
    let mut scene = Scene::new("imp");
    scene.stat(5, 43, 43);
    let original = scene
        .data
        .prayer_by_name("Protect from Melee")
        .unwrap()
        .clone();
    let replacement = scene
        .data
        .prayer_by_name("Protect from Missiles")
        .unwrap()
        .clone();
    scene.prayer(original.varp, true);
    scene.face_us();
    scene.refresh();
    policy_s2_incoming(&mut scene, 9);
    let mut harness = Harness::new(&scene, scene.request());
    let refused = harness.pending_with_refusal(&scene, 1, 0);
    assert_eq!(policy_s2_buttons(&refused), vec![replacement.button_com]);
    scene.npcs[0].health = 0;
    scene.refresh();
    assert_eq!(harness.ready(&scene, 2).end, CombatEnd::Killed);
    assert_eq!(
        scene
            .varps
            .iter()
            .find(|row| row.index == original.varp)
            .unwrap()
            .value,
        1
    );
}

#[test]
fn policy_s2_prayer_disallowed_preserves_user_overlays() {
    let mut scene = Scene::new("imp");
    let skin = scene.data.prayer_by_name("Thick Skin").unwrap().clone();
    scene.prayer(skin.varp, true);
    scene.refresh();
    let mut request = scene.request();
    request.allow.prayer = false;
    let mut harness = Harness::new(&scene, request);
    assert!(policy_s2_buttons(&harness.pending_batch(&scene, 1)).is_empty());
    scene.npcs[0].health = 0;
    scene.refresh();
    assert_eq!(harness.ready(&scene, 2).end, CombatEnd::Killed);
    assert_eq!(
        scene
            .varps
            .iter()
            .find(|row| row.index == skin.varp)
            .unwrap()
            .value,
        1
    );
}

#[test]
fn policy_s2_missing_raise_observation_does_not_relinquish_accepted_ownership() {
    let mut scene = Scene::new("imp");
    scene.stat(5, 43, 43);
    scene.face_us();
    scene.refresh();
    let protect = scene
        .data
        .prayer_by_name("Protect from Melee")
        .unwrap()
        .clone();
    let mut harness = Harness::new(&scene, scene.request());
    assert_eq!(
        policy_s2_buttons(&harness.pending_batch(&scene, 1)),
        vec![protect.button_com]
    );
    scene.install();
    scene.varps.retain(|row| row.index != protect.varp);
    scene.refresh();
    for tick in 2..=10 {
        harness.pending_batch(&scene, tick);
    }
    assert!(harness.machine.prayer_cleanup(0).contains(protect.varp));
    scene.varps.push(VarpView {
        index: protect.varp,
        value: 1,
    });
    scene.npcs[0].health = 0;
    scene.refresh();
    assert_eq!(
        policy_s2_buttons(&harness.pending_batch(&scene, 11)),
        vec![protect.button_com]
    );
}

#[test]
fn case_31_ranged_launch_advances_cycle_but_impact_does_not() {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    let mut arrows = scene.held("steel_arrow", 13);
    arrows.container = ItemContainer::Equipment;
    arrows.count = 150;
    let tab = scene.tables.weapon_style(bow.def.id).unwrap().tab.unwrap();
    let root = scene.tables.combat_tab_root(tab).unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap();
    scene.varps.push(VarpView {
        index: scene.data.ranged_mode_varp().unwrap(),
        value: i32::from(rapid.slot),
    });
    scene.equipment = vec![bow, arrows];
    scene.refresh();
    scene.combat_tab(root);
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    scene.combat_tab(root);
    assert!(harness.pending(&scene, 2).is_none());
    scene.equipment[1].count -= 1;
    scene.refresh();
    scene.combat_tab(root);
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: 9,
            level: 0,
            src: scene.local.player.actor.tile,
            target: Some(ActorTargetView {
                kind: ActorKind::Npc,
                index: 7,
            }),
            // The packet arrives now; bow flight starts 41 client cycles later.
            t1: 41,
            t2: 51,
        }]);
    assert!(harness.pending(&scene, 3).is_none());
    assert_eq!(harness.machine.counters.swings, 1);
    assert!(harness.machine.schedule.cycle.known);
    assert_eq!(harness.machine.schedule.cycle.deadline, 6);
    assert_eq!(scene.equipment[1].count, 149);
    scene.npcs[0].health -= 1;
    scene.local.player.actor.tile.x += 1;
    scene.refresh();
    scene.combat_tab(root);
    assert!(harness.pending(&scene, 6).is_none());
    assert_eq!(harness.machine.counters.swings, 1);
    assert_eq!(harness.machine.schedule.cycle.deadline, 6);
}

#[test]
fn ranged_wrong_ammo_aborts_prep_without_attack() {
    let mut scene = Scene::new("cow");
    let bow = scene.held("maple_shortbow", 0);
    let mut bolts = scene.held("bolt", 1);
    bolts.count = 50;
    scene.inventory = vec![bow, bolts];
    scene.refresh();
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    assert_eq!(
        harness.ready(&scene, 1).end,
        CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Ammo))
    );
}

fn ranged_readiness_fixture(style_echo: bool) -> (Scene, CombatRequest, i32) {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    let bow_id = bow.def.id;
    let mut arrows = scene.held("steel_arrow", 13);
    arrows.container = ItemContainer::Equipment;
    arrows.count = 50;
    let arrow_id = arrows.def.id;
    let tab = scene.tables.weapon_style(bow_id).unwrap().tab.unwrap();
    let root = scene.tables.combat_tab_root(tab).unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap();
    if style_echo {
        scene.varps.push(VarpView {
            index: scene.data.ranged_mode_varp().unwrap(),
            value: i32::from(rapid.slot),
        });
    }
    scene.equipment = vec![bow, arrows];
    scene.refresh();
    let mut request = scene.request();
    request.style = Style::Ranged;
    request.ranged_style = RangedMode::Rapid;
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, bow_id), (13, arrow_id)]),
        ..CompiledKit::default()
    }));
    (scene, request, root)
}

fn assert_readiness_aborts_after_eight(
    scene: &Scene,
    request: CombatRequest,
    expected_reason: &str,
) {
    let mut harness = Harness::new(scene, request);
    for tick in 1_u64..8 {
        assert!(
            matches!(harness.poll(&scene.snapshot, tick), Poll::Pending),
            "readiness must remain pending through observed tick {tick}"
        );
        assert_eq!(harness.machine.end, None);
        harness.drain(None);
    }
    let report = harness.ready(scene, 8);
    let summary = crate::api_combat::CombatSummary::from(&report);
    assert_eq!(summary.reason.as_deref(), Some(expected_reason));
    assert_eq!(report.ticks, 8);
}

#[test]
fn ranged_combat_root_missing_aborts_after_eight_observed_ticks() {
    let (scene, request, _) = ranged_readiness_fixture(false);
    assert_readiness_aborts_after_eight(&scene, request, "prep-readiness:combat-root-missing");
}

#[test]
fn ranged_combat_root_wrong_aborts_after_eight_observed_ticks() {
    let (mut scene, request, _) = ranged_readiness_fixture(false);
    scene.combat_tab(5855);
    assert_readiness_aborts_after_eight(&scene, request, "prep-readiness:combat-root-wrong");
}

#[test]
fn ranged_style_echo_missing_aborts_after_eight_observed_ticks() {
    let (mut scene, request, root) = ranged_readiness_fixture(false);
    scene.combat_tab(root);
    assert_readiness_aborts_after_eight(&scene, request, "prep-readiness:style-echo-missing");
}

#[test]
fn ranged_root_arriving_on_seventh_observed_tick_still_fights() {
    let (mut scene, request, root) = ranged_readiness_fixture(true);
    let mut harness = Harness::new(&scene, request);
    for tick in 1..7 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.drain(None);
    }
    scene.combat_tab(root);
    let batch = harness.pending_batch(&scene, 7);
    assert!(
        (0..batch.len()).any(|index| matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Npc { action, .. })) if action == "Attack"
        )),
        "the requested style echo and a root arriving on tick 7 must let combat proceed"
    );
    assert_eq!(harness.machine.end, None);
}

#[test]
fn ranged_readiness_bound_ignores_same_tick_repolls() {
    let (scene, request, _) = ranged_readiness_fixture(false);
    let mut harness = Harness::new(&scene, request);
    assert!(matches!(harness.poll(&scene.snapshot, 1), Poll::Pending));
    for sequence in 2..=32 {
        assert!(matches!(
            harness.poll_stamp(&scene.snapshot, 1, sequence),
            Poll::Pending
        ));
    }
    assert_eq!(harness.machine.counters.ticks, 1);
    harness.drain(None);
    for tick in 2..8 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        assert_eq!(harness.machine.end, None);
        harness.drain(None);
    }
    let report = harness.ready(&scene, 8);
    let summary = crate::api_combat::CombatSummary::from(&report);
    assert_eq!(
        summary.reason.as_deref(),
        Some("prep-readiness:combat-root-missing")
    );
    assert_eq!(report.ticks, 8);
}

#[test]
fn ranged_readiness_bound_does_not_outlive_request_budget() {
    let (scene, mut request, _) = ranged_readiness_fixture(false);
    request.budget_ticks = 4;
    let mut harness = Harness::new(&scene, request);
    for tick in 1..4 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.drain(None);
    }
    assert_eq!(harness.ready(&scene, 4).end, CombatEnd::Budget);
}

#[test]
fn ranged_prep_waits_for_style_observation_then_ammo_out_is_explicit() {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    let mut arrows = scene.held("steel_arrow", 13);
    arrows.container = ItemContainer::Equipment;
    arrows.count = 2;
    let tab = scene.tables.weapon_style(bow.def.id).unwrap().tab.unwrap();
    let root = scene.tables.combat_tab_root(tab).unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap();
    let (button, slot) = (rapid.button, rapid.slot);
    let varp = scene.data.ranged_mode_varp().unwrap();
    scene.varps.retain(|row| row.index != varp);
    scene.varps.push(VarpView {
        index: varp,
        value: 0,
    });
    scene.equipment = vec![bow, arrows];
    scene.refresh();
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    assert!(
        harness.pending(&scene, 1).is_none(),
        "missing tab cannot arm"
    );
    scene.combat_tab(root);
    assert!(matches!(harness.pending(&scene, 2),
        Some(HostEffect::Interaction(InteractReq::IfButton { component_id }))
        if component_id == button));
    assert_eq!(harness.machine.phase, Phase::Prep);
    assert!(harness.pending(&scene, 3).is_none());
    scene
        .varps
        .iter_mut()
        .find(|row| row.index == varp)
        .unwrap()
        .value = i32::from(slot);
    scene.refresh();
    scene.combat_tab(root);
    attack(harness.pending(&scene, 4));
    scene.install();
    scene.equipment[1].count = 0;
    scene.refresh();
    scene.combat_tab(root);
    assert_eq!(
        harness.ready(&scene, 5).end,
        CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoAmmo))
    );
}

#[test]
fn ranged_winddown_caps_even_unobserved_pickups_at_four() {
    ranged_winddown_fixture(false, false);
}

#[test]
fn ranged_winddown_aborts_for_another_live_threat() {
    ranged_winddown_fixture(true, false);
}

#[test]
fn ranged_winddown_does_not_ignore_a_respawned_target() {
    ranged_winddown_fixture(false, true);
}

fn ranged_winddown_fixture(other_threat: bool, respawned: bool) {
    let mut scene = Scene::new("cow");
    let ammo = scene.held("steel_arrow", 0);
    let here = scene.local.player.actor.tile;
    let mut held = ammo.clone();
    held.count = 40_000;
    scene.inventory.push(held);
    scene.npcs[0].health = if respawned { 30 } else { 0 };
    if other_threat {
        let mut other = scene.npcs[0].clone();
        other.index = 8;
        other.health = 30;
        other.target = Some(ActorTargetView {
            kind: ActorKind::Player,
            index: 1,
        });
        other.in_combat = true;
        scene.npcs.push(other);
    }
    scene.refresh();
    scene
        .snapshot
        .seed_ground_items(vec![api::snapshot::GroundItemView {
            def: ammo.def.clone(),
            count: 5,
            actions: vec![Some("Take".into())],
            tile: here,
            distance: 0,
        }]);
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    harness.runtime.reach = Some(api::query::ReachQueryView {
        available: true,
        base_x: here.x,
        base_z: here.z,
        level: here.level,
        width: 1,
        height: 1,
        walkable: vec![1],
        reachable: vec![1],
        reachable_adj: vec![1],
        exact_rank: vec![0],
        adjacent_rank: vec![0],
        step: vec![0],
        canlight: Vec::new(),
    });
    harness.machine.ranged_mut().ammo_pick = ammo.def.id;
    harness.machine.engaged = Some(ActorRef {
        kind: ActorKind::Npc,
        index: 7,
    });
    harness.machine.threats.observe_hunt(
        [(7, scene.npcs[0].r#type.unwrap() as i32, true, 2, 1)],
        1,
        &scene.tables,
        0,
    );
    harness.machine.finish(CombatEnd::Killed, 0);
    let mut pickups = 0;
    let mut completed = false;
    for tick in 1..=24 {
        match harness.poll(&scene.snapshot, tick) {
            Poll::Pending => {
                if let Some(effect) = harness.drain(None).into_single() {
                    assert!(matches!(
                        effect,
                        HostEffect::Interaction(InteractReq::Obj { .. })
                    ));
                    pickups += 1;
                }
            }
            Poll::Ready(Ok(report)) => {
                assert_eq!(report.end, CombatEnd::Killed);
                assert_eq!(
                    report.ammo_pickups, 0,
                    "unobserved pickup is not counted as recovered"
                );
                completed = true;
                break;
            }
            other => panic!("unexpected sweep result: {other:?}"),
        }
    }
    assert!(completed);
    assert_eq!(pickups, if other_threat || respawned { 0 } else { 4 });
}

#[test]
fn ranged_thrown_uses_weapon_stack_without_ammo_slot() {
    let mut scene = Scene::new("cow");
    let mut darts = scene.held("bronze_dart", 3);
    darts.container = ItemContainer::Equipment;
    darts.count = 50;
    let tab = scene
        .tables
        .weapon_style(darts.def.id)
        .unwrap()
        .tab
        .unwrap();
    let root = scene.tables.combat_tab_root(tab).unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap();
    scene.varps.push(VarpView {
        index: scene.data.ranged_mode_varp().unwrap(),
        value: i32::from(rapid.slot),
    });
    let id = darts.def.id;
    scene.equipment = vec![darts];
    scene.refresh();
    scene.combat_tab(root);
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    assert_eq!(harness.machine.ranged().ammo_pick, id);
    assert_eq!(harness.machine.desired(13), None);
    scene.install();
    // Route-head launch coordinates lead the rendered pose during a chase.
    scene.local.player.actor.tile.x += 3;
    scene.refresh();
    scene.combat_tab(root);
    assert!(harness.pending(&scene, 2).is_none());
    for tick in 3..203 {
        let cycle = i32::try_from(tick).unwrap() * 30;
        scene.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: cycle,
        });
        scene
            .snapshot
            .seed_projectiles(vec![api::snapshot::ProjectileView {
                spotanim: 9,
                level: 0,
                src: scene.local.player.network,
                target: Some(ActorTargetView {
                    kind: ActorKind::Npc,
                    index: 7,
                }),
                t1: cycle,
                t2: cycle + 10,
            }]);
        let allocations = allocation_counter::measure(|| {
            assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
            assert!(harness.runtime.ledger.as_ref().unwrap().outbox.is_empty());
        });
        assert_eq!(allocations.count_total, 0);
    }
    scene.equipment.clear();
    scene.refresh();
    scene.combat_tab(root);
    assert_eq!(
        harness.ready(&scene, 203).end,
        CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoAmmo))
    );
}

fn contains_wear(batch: &DrainedBatch, wanted: &str) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Wear { name })) if name.as_str() == wanted
        )
    })
}

#[test]
fn ranged_prep_waits_for_torn_thrown_weapon_wear_frame() {
    let mut scene = Scene::new("cow");
    let mut darts = scene.held("bronze_dart", 0);
    darts.count = 50;
    let id = darts.def.id;
    scene.inventory.push(darts.clone());
    scene.refresh();

    let mut request = scene.request();
    request.style = Style::Ranged;
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, id)]),
        ..CompiledKit::default()
    }));
    let mut harness = Harness::new(&scene, request);
    let first = harness.pending_batch(&scene, 1);
    assert!(contains_wear(&first, darts.def.name.as_deref().unwrap()));

    scene.inventory.clear();
    scene.refresh();
    assert!(
        matches!(harness.poll(&scene.snapshot, 2), Poll::Pending),
        "an in-flight thrown-weapon Wear must cover the split frame"
    );
    harness.drain(None);

    darts.container = ItemContainer::Equipment;
    darts.slot = 3;
    scene.equipment.push(darts);
    scene.refresh();
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    harness.drain(None);
}

#[test]
fn ranged_fight_waits_for_torn_bow_ammo_wear_frame() {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    let bow_id = bow.def.id;
    let mut arrows = scene.held("steel_arrow", 13);
    arrows.container = ItemContainer::Equipment;
    arrows.count = 50;
    let arrow_id = arrows.def.id;
    let tab = scene.tables.weapon_style(bow_id).unwrap().tab.unwrap();
    let root = scene.tables.combat_tab_root(tab).unwrap();
    let rapid = scene
        .data
        .ranged_modes()
        .iter()
        .find(|row| row.tab == tab as u8 && row.mode == RangedMode::Rapid as u8)
        .unwrap();
    scene.varps.push(VarpView {
        index: scene.data.ranged_mode_varp().unwrap(),
        value: i32::from(rapid.slot),
    });
    scene.equipment = vec![bow, arrows.clone()];
    scene.refresh();
    scene.combat_tab(root);

    let mut request = scene.request();
    request.style = Style::Ranged;
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, bow_id), (13, arrow_id)]),
        ..CompiledKit::default()
    }));
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();

    scene.equipment.retain(|item| item.slot != 13);
    let mut carried_arrows = arrows.clone();
    carried_arrows.container = ItemContainer::Inventory;
    carried_arrows.slot = 0;
    scene.inventory.push(carried_arrows);
    scene.refresh();
    scene.combat_tab(root);

    let mut wear_tick = None;
    for tick in 2..=8 {
        let batch = harness.pending_batch(&scene, tick);
        if contains_wear(&batch, arrows.def.name.as_deref().unwrap()) {
            wear_tick = Some(tick);
            break;
        }
    }
    let wear_tick = wear_tick.expect("Fight should re-wear the carried arrows");

    scene.inventory.clear();
    scene.refresh();
    scene.combat_tab(root);
    let torn = harness.pending_batch(&scene, wear_tick + 1);
    assert!(
        !(0..torn.len()).any(|index| matches!(
            torn.get(index),
            Some(HostEffect::Interaction(InteractReq::Npc { action, .. }))
                if action == "Attack"
        )),
        "do not attack while the ammo Wear is still in flight"
    );

    arrows.container = ItemContainer::Equipment;
    scene.equipment.push(arrows);
    scene.refresh();
    scene.combat_tab(root);
    assert!(matches!(
        harness.poll(&scene.snapshot, wear_tick + 2),
        Poll::Pending
    ));
    harness.drain(None);
}

#[test]
fn ranged_missing_bow_ammo_wear_times_out_as_prep_failed_ammo() {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    let bow_id = bow.def.id;
    let mut arrows = scene.held("steel_arrow", 0);
    arrows.count = 50;
    let arrow_id = arrows.def.id;
    scene.equipment.push(bow);
    scene.inventory.push(arrows.clone());
    scene.refresh();

    let mut request = scene.request();
    request.style = Style::Ranged;
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, bow_id), (13, arrow_id)]),
        ..CompiledKit::default()
    }));
    let mut harness = Harness::new(&scene, request);
    assert!(contains_wear(
        &harness.pending_batch(&scene, 1),
        arrows.def.name.as_deref().unwrap()
    ));

    scene.inventory.clear();
    scene.refresh();
    for tick in 2..=4 {
        assert!(
            matches!(harness.poll(&scene.snapshot, tick), Poll::Pending),
            "missing ammo remains pending only while its Wear is settling"
        );
        harness.drain(None);
    }
    assert_eq!(
        harness.ready(&scene, 5).end,
        CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Ammo))
    );
}

#[test]
fn ranged_missing_thrown_weapon_wear_times_out_as_prep_failed_weapon() {
    let mut scene = Scene::new("cow");
    let mut darts = scene.held("bronze_dart", 0);
    darts.count = 50;
    let id = darts.def.id;
    scene.inventory.push(darts.clone());
    scene.refresh();

    let mut request = scene.request();
    request.style = Style::Ranged;
    request.kit = Some(Arc::new(CompiledKit {
        worn: Arc::from([(3, id)]),
        ..CompiledKit::default()
    }));
    let mut harness = Harness::new(&scene, request);
    assert!(contains_wear(
        &harness.pending_batch(&scene, 1),
        darts.def.name.as_deref().unwrap()
    ));

    scene.inventory.clear();
    scene.refresh();
    for tick in 2..=4 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.drain(None);
    }
    assert_eq!(
        harness.ready(&scene, 5).end,
        CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Weapon))
    );
}

#[test]
fn ranged_stacked_shooter_uses_rate_clock_then_resumes_launch_evidence() {
    let mut scene = Scene::new("cow");
    let mut bow = scene.held("maple_shortbow", 3);
    bow.container = ItemContainer::Equipment;
    scene.equipment.push(bow);
    scene.install();
    let mut other = scene.local.player.clone();
    other.index = 2;
    // A co-located network shooter can still have a different rendered pose.
    other.actor.tile.x += 2;
    scene.players.push(other);
    scene.refresh();
    let mut request = scene.request();
    request.style = Style::Ranged;
    let mut harness = Harness::new(&scene, request);
    harness.machine.engaged = Some(ActorRef {
        kind: ActorKind::Npc,
        index: 7,
    });
    let rate = measured_weapon_rate(&harness, &scene);
    harness.machine.schedule.observe_swing(1, rate);
    for tick in 2..=30 {
        let frame = Frame::borrow(SnapshotView::new(
            Some(&scene.snapshot),
            harness.runtime.evidence.unwrap(),
        ))
        .unwrap();
        harness.machine.settle(&frame, tick);
        assert_eq!(harness.machine.schedule.last_swing, 1);
        assert_eq!(harness.machine.counters.swings, 0);
        assert!(!harness.machine.schedule.stale_ready(tick, rate));
        assert!(!reached(tick, harness.machine.schedule.cycle.deadline));
    }
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: 9,
            src: scene.local.player.actor.tile,
            level: scene.local.player.actor.tile.level,
            target: scene.local.player.actor.target,
            t1: 32,
            t2: 40,
        }]);
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        harness.runtime.evidence.unwrap(),
    ))
    .unwrap();
    harness.machine.settle(&frame, 31);
    assert_eq!(harness.machine.ranged().launch_cycle, 32);
    assert_eq!(harness.machine.counters.swings, 0);
    scene.snapshot.seed_players(Vec::new());
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        harness.runtime.evidence.unwrap(),
    ))
    .unwrap();
    harness.machine.settle(&frame, 32);
    assert_eq!(
        harness.machine.counters.swings, 0,
        "old ambiguous launch stays consumed"
    );
    scene
        .snapshot
        .seed_projectiles(vec![api::snapshot::ProjectileView {
            spotanim: 9,
            src: scene.local.player.actor.tile,
            level: scene.local.player.actor.tile.level,
            target: scene.local.player.actor.target,
            t1: 35,
            t2: 45,
        }]);
    let frame = Frame::borrow(SnapshotView::new(
        Some(&scene.snapshot),
        harness.runtime.evidence.unwrap(),
    ))
    .unwrap();
    harness.machine.settle(&frame, 33);
    assert_eq!(harness.machine.counters.swings, 1);
    assert_eq!(harness.machine.schedule.last_swing, 33);
    assert_eq!(
        harness.machine.schedule.cycle.deadline,
        33 + u16::from(rate)
    );
}
#[path = "magic_machine_tests.rs"]
mod magic_tests;
