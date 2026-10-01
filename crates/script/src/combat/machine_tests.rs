use super::*;
use crate::native::{ledger, ActionHandle, HostEffect, NativeActions, RetainedMemory};
use api::game_data::SelectedGameData;
use api::obj_names::ItemDefView;
use api::quest_progress::EvidenceStamp;
use api::selected::{ClientRevision, RunKey, SelectedPin};
use api::snapshot::{
    ActorTargetView, ActorView, GameSnapshot, HitmarkView, HitmarksView, ItemActionFamily,
    ItemContainer, LocalPlayerView, NpcView, PlayerView, SnapshotView, StatView, VarpView,
    WorldStateView,
};
use std::time::Instant;

struct Lease;
impl NativeMachine for Lease {
    type Args = ();
    type Output = ();
    fn begin(_: (), _: &mut ActionContext<'_>) -> Result<Self, ActionError> { Ok(Self) }
    fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> { Poll::Pending }
    fn cancel(&mut self) {}
}

struct Runtime {
    pin: Arc<SelectedPin>,
    retained: RetainedMemory,
    ledger: Option<Box<ledger::Ledger>>,
    budget: ledger::TickBudget,
    wall: Instant,
}
impl Runtime {
    fn context<R>(&mut self, snapshot: &GameSnapshot, tick: u64, sequence: u64, f: impl FnOnce(&mut NativeActions, &mut ActionContext<'_>) -> R) -> R {
        self.budget.observe(tick);
        let evidence = EvidenceStamp { run: RunKey { slot: 1, run: 1, session: 1 }, tick, sequence };
        let action_id = self.ledger.as_ref().and_then(|ledger| ledger.owner.as_ref()).map_or(0, |owner| owner.id.get());
        let mut actions = NativeActions { _private: () };
        let mut cx = ActionContext {
            evidence, pin: &self.pin, snapshot: SnapshotView::new(Some(snapshot), evidence),
            retained: &mut self.retained, action_id, active_now: Duration::from_millis(tick * 600),
            wall_now: self.wall, ledger: &mut self.ledger, budget: &mut self.budget, eligible: true,
        };
        f(&mut actions, &mut cx)
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
            pin: scene.data.selected_pin().unwrap(), retained: RetainedMemory::default(),
            ledger: None, budget: ledger::TickBudget::default(), wall: Instant::now(),
        };
        let (machine, lease) = runtime.context(&scene.snapshot, origin, origin, |actions, cx| {
            let lease = actions.begin::<Lease>((), cx).unwrap();
            let machine = Combat::begin((Arc::new(request), Arc::clone(&scene.tables)), cx).unwrap();
            (machine, lease)
        });
        Self { machine, runtime, _lease: lease }
    }
    fn poll(&mut self, snapshot: &GameSnapshot, tick: u64) -> Poll<Result<CombatReport, ActionError>> {
        self.poll_stamp(snapshot, tick, tick)
    }
    fn poll_stamp(&mut self, snapshot: &GameSnapshot, tick: u64, sequence: u64) -> Poll<Result<CombatReport, ActionError>> {
        let machine = &mut self.machine;
        self.runtime.context(snapshot, tick, sequence, |_, cx| machine.poll(cx))
    }
    fn take(&mut self) -> Option<HostEffect> {
        let ledger = self.runtime.ledger.as_mut().unwrap();
        assert!(ledger.outbox.len() <= 1, "one request per observed tick");
        ledger.outbox.pop().map(|action| action.effect)
    }
    fn pending(&mut self, scene: &Scene, tick: u64) -> Option<HostEffect> {
        assert!(matches!(self.poll(&scene.snapshot, tick), Poll::Pending));
        self.take()
    }
    fn ready(&mut self, scene: &Scene, tick: u64) -> CombatReport {
        match self.poll(&scene.snapshot, tick) {
            Poll::Ready(Ok(report)) => { assert!(self.take().is_none()); report }
            result => panic!("expected completed combat, got {result:?}"),
        }
    }
}

struct Scene {
    data: Arc<SelectedGameData>,
    tables: Arc<CombatTables>,
    snapshot: GameSnapshot,
    local: LocalPlayerView,
    npcs: Vec<NpcView>,
    stats: Vec<StatView>,
    varps: Vec<VarpView>,
    inventory: Vec<ItemView>,
    equipment: Vec<ItemView>,
}
fn tile(x: i32, z: i32) -> api::WorldTile { api::WorldTile { x, z, level: 0 } }
fn actor(at: api::WorldTile) -> ActorView {
    ActorView {
        name: None, actions: Vec::new(), tile: at, distance: 0, animation: -1, animation_frame: -1,
        pose_animation: -1, orientation: 0, target_orientation: 0, overhead_text: None,
        spot_animation: -1, spot_animation_stamp: -1, health: 40, total_health: 40,
        face_entity: -1, target: None, moving: false, running: false, in_combat: false,
    }
}
impl Scene {
    fn new(config: &str) -> Self {
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let tables = CombatTables::build(Arc::clone(&data)).unwrap();
        let row = data.npc_by_config(config).unwrap();
        let at = tile(2600, 3200);
        let npc = NpcView {
            index: 7, r#type: Some(row.id as usize), name: row.display.clone(),
            actions: vec![Some("Attack".into())], tile: tile(at.x + 1, at.z), distance: 1,
            animation: -1, animation_frame: -1, pose_animation: -1, orientation: 0,
            target_orientation: 0, overhead_text: None, spot_animation: -1,
            spot_animation_stamp: -1, health: 30, total_health: 30, face_entity: -1,
            target: None, moving: false, running: false, in_combat: false, level: 1, size: 1,
            network: tile(at.x + 1, at.z), x: 0, z: 0, yaw: 0,
        };
        let varps = data.prayers().iter().map(|row| VarpView { index: row.varp, value: 0 })
            .chain([VarpView { index: super::super::OPTION_NODEF, value: 0 }]).collect();
        let mut scene = Self {
            data, tables, snapshot: GameSnapshot::new(), npcs: vec![npc],
            local: LocalPlayerView { player: PlayerView { index: 1, actor: actor(at), combat_level: 60, skill_level: 0, weapon: None }, energy: 100, weight: 0 },
            stats: (0..25).map(|index| StatView { index, name: String::new(), effective: if index == 5 { 1 } else { 40 }, base: if index == 5 { 1 } else { 40 }, xp: 0, used: api::snapshot::stat_used(index as usize) }).collect(),
            varps, inventory: Vec::new(), equipment: Vec::new(),
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
        self.snapshot.seed_world(WorldStateView { map_base_x: 2560, map_base_z: 3160, members: true, ..WorldStateView::default() });
        self.snapshot.seed_stats(self.stats.clone());
        self.snapshot.seed_varps(self.varps.clone());
        self.snapshot.seed_inventory(self.inventory.clone(), 28);
        self.snapshot.seed_equipment(self.equipment.clone());
        self.snapshot.seed_npcs(self.npcs.clone());
        self.snapshot.seed_players(Vec::new());
        self.snapshot.seed_projectiles(Vec::new());
        self.snapshot.seed_side_tabs(Vec::new(), 0);
        self.snapshot.seed_hitmarks(HitmarksView { marks: [HitmarkView { value: 0, kind: 0, cycle: 0 }; 4], loop_cycle: 0 });
        self.snapshot.seed_chat_lines(Vec::new());
    }
    fn request(&self) -> CombatRequest {
        CombatRequest { target: Target::Npc { types: Arc::from([self.npcs[0].r#type.unwrap() as i32]), pick: Pick::Nearest, not_targeting_others: true }, ..CombatRequest::default() }
    }
    fn stat(&mut self, index: usize, effective: i32, base: i32) {
        self.stats[index].effective = effective;
        self.stats[index].base = base;
    }
    fn prayer(&mut self, varp: i32, on: bool) {
        self.varps.iter_mut().find(|row| row.index == varp).unwrap().value = i32::from(on);
    }
    fn held(&self, alias: &str, slot: i32) -> ItemView {
        let item = self.data.item_by_alias(alias).unwrap();
        ItemView {
            def: ItemDefView { id: item.id, name: item.name.clone(), stackable: false, members: false, base_value: 1, noted: false, certificate_link: -1, certificate_template: -1 },
            container: ItemContainer::Inventory, action_family: ItemActionFamily::Held, slot,
            count: 1, actions: Vec::new(), component_id: 3214,
        }
    }
    fn face_us(&mut self) {
        self.npcs[0].target = Some(ActorTargetView { kind: ActorKind::Player, index: 1 });
        self.npcs[0].in_combat = true;
    }
    fn install(&mut self) {
        self.local.player.actor.target = Some(ActorTargetView { kind: ActorKind::Npc, index: 7 });
        self.local.player.actor.in_combat = true;
    }
    fn melee_seq(&self) -> i32 {
        self.data.style_seqs().iter().find(|row| self.tables.style_seq(row.seq_id).is_some_and(|mask| mask.contains(super::super::tables::StyleMask::MELEE))).unwrap().seq_id
    }
}
fn attack(effect: Option<HostEffect>) {
    assert!(matches!(effect, Some(HostEffect::Interaction(InteractReq::Npc { action, index: Some(7), .. })) if action == "Attack"));
}
fn held(effect: Option<HostEffect>, want: &str) {
    assert!(matches!(effect, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == want));
}
fn prayer(effect: Option<HostEffect>, button: i32) {
    assert!(matches!(effect, Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) if component_id == button));
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

#[test]
fn attacker_player_slot_reuse_cannot_retarget_an_existing_engagement() {
    let mut scene = Scene::new("imp");
    let mut attacker = PlayerView { index: 8, actor: actor(tile(2601, 3200)), combat_level: 60, skill_level: 0, weapon: None };
    attacker.actor.name = Some("Alpha".into());
    attacker.actor.actions = vec![Some("Attack".into())];
    attacker.actor.target = Some(ActorTargetView { kind: ActorKind::Player, index: 1 });
    attacker.actor.in_combat = true;
    scene.snapshot.seed_players(vec![attacker.clone()]);
    let mut request = scene.request();
    request.target = Target::Attacker { npcs: false, players: true };
    let mut harness = Harness::new(&scene, request);
    assert!(matches!(harness.pending(&scene, 1), Some(HostEffect::Interaction(InteractReq::Player { name, action })) if name == "Alpha" && action == "Attack"));
    attacker.actor.name = Some("Beta".into());
    attacker.actor.health = 0;
    scene.snapshot.seed_players(vec![attacker]);
    for tick in 2..5 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    let report = harness.ready(&scene, 5);
    assert_eq!(report.end, CombatEnd::TargetGone);
    assert_eq!(report.engaged, Some(ActorRef { kind: ActorKind::Player, index: 8 }));
    assert_eq!(report.engaged_npc_type, -1);
}

#[test]
fn fail_closed_later_slices_have_no_host_work() {
    let scene = Scene::new("imp");
    for change in [0, 1, 2, 3] {
        let mut request = scene.request();
        match change {
            0 => request.style = Style::Ranged,
            1 => request.style = Style::Mage,
            2 => request.prayer_mode = PrayerMode::Flick,
            _ => request.target = Target::Player { name: Arc::from("someone") },
        }
        let mut harness = Harness::new(&scene, scene.request());
        let result = harness.runtime.context(&scene.snapshot, 1, 1, |_, cx| Combat::begin((Arc::new(request), Arc::clone(&scene.tables)), cx));
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
        if dead { scene.stat(3, 0, 40); }
        scene.refresh();
        assert_eq!(harness.ready(&scene, 3).end, if dead { CombatEnd::Died } else { CombatEnd::Killed });
    }
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    scene.npcs.clear();
    scene.refresh();
    for tick in 3..6 { assert!(harness.pending(&scene, tick).is_none()); }
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
    for tick in 3..7 { assert!(matches!(harness.poll(&missing, tick), Poll::Pending)); assert!(harness.take().is_none()); }
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
    request.target = Target::Npc { types: Arc::from([scene.npcs[0].r#type.unwrap() as i32, other]), pick: Pick::Nearest, not_targeting_others: true };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.npcs[0].r#type = Some(other as usize);
    scene.npcs[0].health = 0;
    scene.refresh();
    assert!(matches!(harness.poll(&scene.snapshot, 2), Poll::Pending));
    assert_ne!(harness.machine.end, Some(CombatEnd::Killed));
    assert_eq!(harness.machine.engaged_type, other);
    harness.take();
    scene.npcs[0].r#type = Some(scene.data.npc_by_config("nasty_tree").unwrap().id as usize);
    scene.refresh();
    for tick in 3..6 { assert!(harness.pending(&scene, tick).is_none()); }
    assert_eq!(harness.ready(&scene, 6).end, CombatEnd::TargetGone);
}

#[test]
fn same_stamp_and_new_sequence_same_tick_cannot_duplicate_work() {
    let scene = Scene::new("imp");
    let mut harness = Harness::new(&scene, scene.request());
    attack(harness.pending(&scene, 1));
    assert!(matches!(harness.poll(&scene.snapshot, 1), Poll::Pending));
    assert!(harness.take().is_none());
    assert!(matches!(harness.poll_stamp(&scene.snapshot, 1, 2), Poll::Pending));
    assert!(harness.take().is_none());
    assert_eq!(harness.machine.counters.ticks, 1);
}

#[test]
fn case27e_refused_drink_cannot_create_input_lock_or_restoration() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("4dose1prayerrestore", 0));
    scene.refresh();
    harness.runtime.budget.observe(3);
    assert!(harness.runtime.budget.event(false));
    assert!(harness.pending(&scene, 3).is_none());
    assert_eq!(harness.machine.input_lock(), None);
    assert!(!harness.machine.schedule.restore_owed);
    assert!(!harness.machine.schedule.pending(OpKind::Drink));
    held(harness.pending(&scene, 4), "Drink");
    assert_eq!(harness.machine.input_lock(), Some(7));
    assert_eq!(harness.machine.schedule.restore_due, 7);
}

#[test]
fn case27_recovery_food_unlocks_protection_drink_next_tick() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("lobster", 0));
    scene.inventory.push(scene.held("1dose1prayerrestore", 1));
    scene.refresh();
    held(harness.pending(&scene, 3), "Eat");
    assert_eq!(harness.machine.schedule.restore_due, 4);
    scene.stat(3, 37, 40);
    scene.inventory.remove(0);
    scene.refresh();
    held(harness.pending(&scene, 4), "Drink");
    scene.stat(5, 17, 43);
    scene.inventory.clear();
    scene.refresh();
    for tick in 5..7 { assert!(harness.pending(&scene, tick).is_none()); }
    let protect = scene.data.prayers().iter().find(|row| row.name == "Protect from Melee").unwrap().clone();
    prayer(harness.pending(&scene, 7), protect.button_com);
    scene.prayer(protect.varp, true);
    scene.refresh();
    attack(harness.pending(&scene, 8));
    assert_eq!(harness.machine.counters.restorations, 1);
    assert_eq!(harness.machine.counters.locked, 2);
}

#[test]
fn karambwan_p_delay_holds_food_and_restoration_for_four_ticks() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 9, 40);
    scene.inventory.push(scene.held("tbwt_cooked_karambwan", 0));
    scene.refresh();
    held(harness.pending(&scene, 3), "Eat");
    assert_eq!(harness.machine.input_lock(), Some(7));
    assert_eq!(harness.machine.schedule.restore_due, 7);
    scene.stat(3, 27, 40);
    scene.inventory.clear();
    scene.refresh();
    for tick in 4..7 {
        assert!(harness.pending(&scene, tick).is_none());
    }
    attack(harness.pending(&scene, 7));
    assert_eq!(harness.machine.counters.locked, 3);
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
    held(harness.pending(&scene, 3), "Eat");
    scene.inventory[0].count = 1;
    scene.stat(3, 10, 40);
    scene.refresh();
    for tick in 4..6 {
        let effect = harness.pending(&scene, tick);
        assert!(!matches!(effect, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Eat"));
        assert_eq!(harness.machine.end, None);
    }
    held(harness.pending(&scene, 6), "Eat");
}

#[test]
fn case27_failed_or_insufficient_heal_never_licenses_drink() {
    for heal in [false, true] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(3, 24, 40);
        scene.stat(5, 0, 43);
        scene.face_us();
        scene.inventory.push(scene.held("lobster", 0));
        scene.inventory.push(scene.held("4dose1prayerrestore", 1));
        scene.refresh();
        held(harness.pending(&scene, 3), "Eat");
        if heal { scene.stat(3, 27, 40); }
        scene.refresh();
        let next = harness.pending(&scene, 4);
        assert!(!matches!(next, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Drink"));
        assert_eq!(harness.machine.input_lock(), None);
    }
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
        let effect = harness.pending(&scene, 3);
        if hp == 19 {
            assert!(matches!(effect, Some(HostEffect::Interaction(InteractReq::Held { name, action, .. })) if name == "Shark" && action == "Eat"));
        } else { assert!(effect.is_empty()); }
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
            assert_eq!(harness.ready(&scene, 2).end, CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoFood)));
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
    let prayers: Vec<_> = scene.data.prayers().iter().filter(|row| ["Protect from Melee", "Ultimate Strength", "Incredible Reflexes"].contains(&row.name.as_str())).map(|row| (row.varp, row.button_com)).collect();
    assert_eq!(prayers.len(), 3);
    for (varp, _) in &prayers { scene.prayer(*varp, true); }
    scene.npcs[0].health = 0;
    scene.refresh();
    for (offset, (varp, button)) in prayers.iter().enumerate() {
        prayer(harness.pending(&scene, 3 + offset as u64), *button);
        scene.prayer(*varp, false);
        scene.refresh();
    }
    let report = harness.ready(&scene, 6);
    assert_eq!(report.end, CombatEnd::Killed);
    assert_eq!(report.restorations, 0);
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
    assert!(std::mem::size_of::<Combat>() <= 384);
    assert!(std::mem::size_of::<CombatReport>() <= 80);
    println!("Combat={} CombatReport={} CombatRequest={} ArcRequest={} ActionHandle={} HostAction={} Ledger={} InteractionReceipt={} WalkReceipt={}",
        std::mem::size_of::<Combat>(), std::mem::size_of::<CombatReport>(), std::mem::size_of::<CombatRequest>(), std::mem::size_of::<Arc<CombatRequest>>(),
        std::mem::size_of::<ActionHandle<Combat>>(), std::mem::size_of::<ledger::HostAction>(),
        std::mem::size_of::<ledger::Ledger>(), std::mem::size_of::<crate::native::InteractionReceipt>(), std::mem::size_of::<crate::native::WalkReceipt>());
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
        scene.inventory.push(scene.held("1dose1prayerrestore", 1));
        scene.refresh();
        held(harness.pending(&scene, 3), "Drink");
        scene.inventory.pop();
        scene.stat(5, 17, 43);
        scene.stat(3, if hits == 3 { 11 } else { 21 }, 40);
        scene.refresh();
        assert!(harness.pending(&scene, 4).is_none());
        assert!(matches!(harness.poll(&scene.snapshot, 4), Poll::Pending));
        assert_eq!(harness.machine.counters.locked, 1);
        if hits >= 2 { scene.stat(3, if hits == 3 { 2 } else { 12 }, 40); }
        scene.refresh();
        assert!(harness.pending(&scene, 5).is_none());
        let protect = scene.data.prayers().iter().find(|row| row.name == "Protect from Melee").unwrap().clone();
        let protect_tick = if hits == 3 {
            held(harness.pending(&scene, 6), "Eat");
            scene.stat(3, 14, 40);
            scene.inventory.clear();
            scene.refresh();
            7
        } else { 6 };
        prayer(harness.pending(&scene, protect_tick), protect.button_com);
        scene.prayer(protect.varp, true);
        scene.refresh();
        attack(harness.pending(&scene, protect_tick + 1));
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
        scene.inventory.push(scene.held("1dose1prayerrestore", 1));
        scene.refresh();
        held(harness.pending(&scene, 3), "Eat");
    }
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(3, 25, 40);
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("shrimp", 0));
    scene.inventory.push(scene.held("1dose1prayerrestore", 1));
    scene.refresh();
    assert!(harness.pending(&scene, 3).is_none());
    assert!(harness.pending(&scene, 4).is_none());
    assert!(!harness.machine.schedule.pending(OpKind::Drink));
    assert!(!harness.machine.schedule.pending(OpKind::Eat));
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
        scene.local.player.actor.animation_frame = 1;
        scene.face_us();
        scene.stat(3, 19, 40);
        scene.inventory.push(scene.held("lobster", 0));
        scene.refresh();
        let eat_tick = if idle { 32 } else { 11 };
        held(harness.pending(&scene, eat_tick), "Eat");
        assert_eq!(harness.machine.cycle().deadline, if idle { 27 } else { 16 });
        scene.stat(3, 31, 40);
        scene.inventory.clear();
        scene.refresh();
        attack(harness.pending(&scene, eat_tick + 1));
        assert_eq!(harness.machine.counters.restorations, 1);
        if !idle {
            for tick in 13..16 { assert!(harness.pending(&scene, tick).is_none()); }
            scene.local.player.actor.animation_frame = 0;
            scene.refresh();
            assert!(harness.pending(&scene, 16).is_none());
            assert_eq!(harness.machine.cycle().deadline, 20);
        } else { assert!(harness.pending(&scene, 34).is_none()); }
    }
}

#[test]
fn case29_and38_safety_ops_form_one_restoration_run() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 43, 43);
    scene.stat(3, 19, 40);
    scene.face_us();
    scene.inventory.push(scene.held("lobster", 0));
    for name in ["Ultimate Strength", "Incredible Reflexes"] {
        let varp = scene.data.prayers().iter().find(|row| row.name == name).unwrap().varp;
        scene.prayer(varp, true);
    }
    scene.refresh();
    let protect = scene.data.prayers().iter().find(|row| row.name == "Protect from Melee").unwrap().clone();
    prayer(harness.pending(&scene, 3), protect.button_com);
    held(harness.pending(&scene, 4), "Eat");
    scene.prayer(protect.varp, true);
    scene.stat(3, 31, 40);
    scene.inventory.clear();
    scene.refresh();
    attack(harness.pending(&scene, 5));
    assert_eq!(harness.machine.counters.restorations, 1);
    assert_eq!(harness.machine.counters.food, 1);
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
        for tick in 3..=rate + 1 { assert!(harness.pending(&scene, tick).is_none()); }
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
    let request = CombatRequest { target: Target::Attacker { npcs: true, players: false }, ..CombatRequest::default() };
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
fn case11_r2_offensive_drop_alternates_with_restore_and_keeps_protect() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.stat(5, 26, 43);
    scene.face_us();
    let on: Vec<_> = scene.data.prayers().iter().filter(|row| ["Protect from Melee", "Ultimate Strength", "Incredible Reflexes"].contains(&row.name.as_str())).cloned().collect();
    for row in &on { scene.prayer(row.varp, true); }
    scene.refresh();
    for (offset, row) in on.iter().filter(|row| row.name != "Protect from Melee").enumerate() {
        prayer(harness.pending(&scene, 3 + offset as u64 * 2), row.button_com);
        scene.prayer(row.varp, false);
        scene.refresh();
        attack(harness.pending(&scene, 4 + offset as u64 * 2));
    }
    assert_eq!(harness.machine.counters.restorations, 2);
    let protect = on.iter().find(|row| row.name == "Protect from Melee").unwrap();
    assert_eq!(scene.varps.iter().find(|row| row.index == protect.varp).unwrap().value, 1);
}

#[test]
fn case5_prayer_floor_requires_active_or_wanted_prayer() {
    for (base, current, drink) in [(43, 26, true), (43, 27, false), (31, 17, true), (31, 18, false)] {
        let mut scene = Scene::new("khazard_warlord");
        let mut harness = fight(&mut scene);
        scene.stat(5, current, base);
        scene.inventory.push(scene.held("1dose1prayerrestore", 0));
        let strength = scene.data.prayers().iter().find(|row| row.name == "Ultimate Strength").unwrap().varp;
        scene.prayer(strength, true);
        scene.refresh();
        let op = harness.pending(&scene, 3);
        if drink { held(op, "Drink"); }
        else { assert!(!matches!(op, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Drink")); }
    }
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    scene.stat(5, 0, 43);
    scene.inventory.push(scene.held("1dose1prayerrestore", 0));
    scene.refresh();
    assert!(harness.pending(&scene, 3).is_none());
}

#[test]
fn case40_retreat_waits_through_drink_lock_and_observed_arrival() {
    let mut scene = Scene::new("khazard_warlord");
    let goal = tile(scene.local.player.actor.tile.x + 20, scene.local.player.actor.tile.z);
    let mut request = scene.request();
    request.fallback = Fallback::Retreat { tile: goal };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("1dose1prayerrestore", 0));
    scene.refresh();
    held(harness.pending(&scene, 3), "Drink");
    scene.inventory.clear();
    scene.stat(5, 17, 43);
    scene.stat(3, 9, 40);
    scene.refresh();
    for tick in 4..6 { assert!(harness.pending(&scene, tick).is_none()); }
    assert_eq!(harness.machine.phase, Phase::Escape);
    assert!(matches!(harness.pending(&scene, 6), Some(HostEffect::Interaction(InteractReq::SetRetaliate { on: false }))));
    scene.varps.iter_mut().find(|row| row.index == super::super::OPTION_NODEF).unwrap().value = 1;
    scene.refresh();
    assert!(matches!(harness.pending(&scene, 7), Some(HostEffect::Walk(request)) if request.target == goal));
    for tick in 8..20 { assert!(harness.pending(&scene, tick).is_none()); }
    scene.local.player.actor.tile = goal;
    scene.refresh();
    let report = harness.ready(&scene, 20);
    assert_eq!(report.end, CombatEnd::Aborted(AbortReason::Retreated));
    assert_eq!(report.locked_ticks, 2);
    assert_eq!(report.restorations, 0);
}

#[test]
fn case15_terminal_walk_receipt_without_arrival_is_retreat_failed() {
    let mut scene = Scene::new("khazard_warlord");
    let goal = tile(scene.local.player.actor.tile.x + 20, scene.local.player.actor.tile.z);
    let mut request = scene.request();
    request.fallback = Fallback::Retreat { tile: goal };
    let mut harness = Harness::new(&scene, request);
    attack(harness.pending(&scene, 1));
    scene.install();
    scene.face_us();
    scene.stat(3, 9, 40);
    scene.refresh();
    assert!(matches!(harness.pending(&scene, 2), Some(HostEffect::Interaction(InteractReq::SetRetaliate { on: false }))));
    scene.varps.iter_mut().find(|row| row.index == super::super::OPTION_NODEF).unwrap().value = 1;
    scene.refresh();
    assert!(matches!(harness.pending(&scene, 3), Some(HostEffect::Walk(_))));
    harness.runtime.ledger.as_mut().unwrap().walk = Some(crate::native::WalkReceipt {
        request_id: harness.machine.pending_walk.unwrap().get(),
        evidence: EvidenceStamp { run: RunKey { slot: 1, run: 1, session: 1 }, tick: 4, sequence: 4 },
        end: crate::native::WalkEnd::Blocked,
    });
    assert_eq!(harness.ready(&scene, 4).end, CombatEnd::Aborted(AbortReason::RetreatFailed));
}

#[test]
fn admitted_drink_lock_wraps_without_losing_observation_count() {
    let mut scene = Scene::new("khazard_warlord");
    scene.stat(5, 0, 43);
    scene.face_us();
    scene.inventory.push(scene.held("1dose1prayerrestore", 0));
    scene.refresh();
    let mut harness = Harness::new_at(&scene, scene.request(), 65_533);
    held(harness.pending(&scene, 65_534), "Drink");
    assert_eq!(harness.machine.input_lock(), Some(1));
    scene.inventory.clear();
    scene.stat(5, 17, 43);
    scene.refresh();
    for tick in 65_535..65_537 { assert!(harness.pending(&scene, tick).is_none()); }
    assert_eq!(harness.machine.counters.locked, 2);
    let protect = scene.data.prayers().iter().find(|row| row.name == "Protect from Melee").unwrap();
    prayer(harness.pending(&scene, 65_537), protect.button_com);
}

#[test]
fn case19_unsettled_food_waits_two_windows_and_eventually_aborts() {
    let mut scene = Scene::new("khazard_warlord");
    let mut harness = fight(&mut scene);
    scene.face_us();
    scene.stat(3, 19, 40);
    scene.inventory.push(scene.held("lobster", 0));
    scene.refresh();
    held(harness.pending(&scene, 3), "Eat");
    for tick in 4..9 {
        let op = harness.pending(&scene, tick);
        assert!(!matches!(op, Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Eat"));
    }
    held(harness.pending(&scene, 9), "Eat");
    for tick in 10..15 { harness.pending(&scene, tick); }
    held(harness.pending(&scene, 15), "Eat");
    for tick in 16..21 { harness.pending(&scene, tick); }
    assert_eq!(harness.ready(&scene, 21).end, CombatEnd::Aborted(AbortReason::Unresponsive));
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
        request.kit = Some(Arc::new(CompiledKit { worn: Arc::from([(slot, id)]), ..CompiledKit::default() }));
        let mut harness = Harness::new(&scene, request);
        assert!(matches!(harness.pending(&scene, 1), Some(HostEffect::Interaction(InteractReq::Wear { .. }))));
        for tick in 2..5 { assert!(harness.pending(&scene, tick).is_none()); }
        assert!(matches!(harness.pending(&scene, 5), Some(HostEffect::Interaction(InteractReq::Wear { .. }))));
        for tick in 6..9 { assert!(harness.pending(&scene, tick).is_none()); }
        if let Some(item) = expected {
            assert_eq!(harness.ready(&scene, 9).end, CombatEnd::Aborted(AbortReason::PrepFailed(item)));
        } else { attack(harness.pending(&scene, 9)); }
    }
    for (slot, fail) in [(3, true), (4, false)] {
        let scene = Scene::new("imp");
        let id = scene.data.item_by_alias("bronze_scimitar").unwrap().id;
        let mut request = scene.request();
        request.kit = Some(Arc::new(CompiledKit { worn: Arc::from([(slot, id)]), ..CompiledKit::default() }));
        let mut harness = Harness::new(&scene, request);
        if fail {
            assert_eq!(harness.ready(&scene, 1).end, CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Weapon)));
        } else { attack(harness.pending(&scene, 1)); }
    }
}

#[test]
fn case12_prep_boosts_once_each_then_respects_cycle_and_target_quarter() {
    let mut scene = Scene::new("khazard_warlord");
    for (slot, alias) in ["1dose2attack", "1dose2strength", "1dose2defense"].into_iter().enumerate() {
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
        if in_kit { request.kit = Some(Arc::new(CompiledKit { carry: Arc::from([(id, 1)]), ..CompiledKit::default() })); }
        let mut harness = Harness::new(&scene, request);
        if in_kit { held(harness.pending(&scene, 1), "Drink"); }
        else {
            attack(harness.pending(&scene, 1));
            scene.install();
            scene.refresh();
            for tick in 2..6 { assert!(harness.pending(&scene, tick).is_none()); }
        }
    }
}

#[test]
fn case13_dragon_shield_overrides_kit_and_rejects_two_handed_weapon() {
    for two_handed in [false, true] {
        let mut scene = Scene::new("elvarg");
        let weapon = scene.held(if two_handed { "bronze_2h_sword" } else { "bronze_scimitar" }, 0);
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
        request.kit = Some(Arc::new(CompiledKit { worn: Arc::from([(3, weapon_id), (5, kit_shield_id)]), ..CompiledKit::default() }));
        let mut harness = Harness::new(&scene, request);
        if two_handed {
            assert_eq!(harness.ready(&scene, 1).end, CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Shield)));
        } else {
            assert!(matches!(harness.pending(&scene, 1), Some(HostEffect::Interaction(InteractReq::Wear { name })) if Some(name.as_str()) == weapon.def.name.as_deref()));
            let mut worn = weapon;
            worn.slot = 3;
            worn.container = ItemContainer::Equipment;
            scene.equipment.push(worn);
            scene.inventory.remove(0);
            scene.refresh();
            assert!(matches!(harness.pending(&scene, 2), Some(HostEffect::Interaction(InteractReq::Wear { name })) if name == shield_name));
            let mut worn = shield;
            worn.slot = 5;
            worn.container = ItemContainer::Equipment;
            scene.equipment.retain(|row| row.slot != 5);
            scene.equipment.push(worn);
            scene.inventory.clear();
            scene.refresh();
            attack(harness.pending(&scene, 3));
            scene.install();
            scene.local.player.actor.animation = scene.melee_seq();
            for tick in 4..54 {
                scene.local.player.actor.animation_frame = (tick % 4) as i32;
                scene.refresh();
                assert!(harness.pending(&scene, tick).is_none());
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
        if equipment {
            assert!(matches!(harness.pending(&scene, 1), Some(HostEffect::Interaction(InteractReq::Wear { .. }))));
        } else { attack(harness.pending(&scene, 1)); }
        assert!(!harness.machine.schedule.pending(OpKind::Drink));
    }
}

#[test]
fn case20_and26_live_mismatch_emits_only_two_owned_strings_without_cooldown_wait() {
    let mut scene = Scene::new("imp");
    let mut harness = fight(&mut scene);
    let mut ignored = scene.npcs[0].clone();
    ignored.index = 8;
    scene.npcs.push(ignored);
    scene.local.player.actor.target = Some(ActorTargetView { kind: ActorKind::Npc, index: 8 });
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
    scene.inventory.push(scene.held("1dose1prayerrestore", 0));
    scene.inventory.push(scene.held("lobster", 1));
    let protect = scene.data.prayers().iter().find(|row| row.name == "Protect from Melee").unwrap().clone();
    let origin = scene.npcs[0].tile;
    let mut drink_tick = None;
    let mut food_tick = None;
    let mut prayer_tick = None;
    let mut routed = Vec::new();
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
        } else { scene.local.player.actor.animation_frame += 1; }
        scene.refresh();
        let op = harness.pending(&scene, tick);
        let opcodes: &[&str] = match &op {
            None => &[],
            Some(HostEffect::Interaction(InteractReq::Npc { action, .. })) if action == "Attack" =>
                &["MOVE_OPCLICK", "OPNPC2"],
            Some(HostEffect::Interaction(InteractReq::IfButton { .. })) => &["IF_BUTTON"],
            Some(HostEffect::Interaction(InteractReq::Held { .. })) => &["OPHELD"],
            _ => panic!("unexpected synthetic-driver request"),
        };
        assert!(opcodes.len() <= 2);
        if let Some(drink) = drink_tick {
            if tick == drink + 1 || tick == drink + 2 { assert_eq!(opcodes, &[] as &[&str]); }
        }
        match op {
            Some(HostEffect::Interaction(InteractReq::Npc { .. })) => {
                assert_eq!(opcodes, ["MOVE_OPCLICK", "OPNPC2"]);
                routed.push(tick);
                scene.install();
            }
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) => {
                assert_eq!(component_id, protect.button_com);
                prayer_tick = Some(tick);
                scene.prayer(protect.varp, true);
            }
            Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Drink" => {
                drink_tick = Some(tick);
                scene.inventory.retain(|row| row.def.id != scene.data.item_by_alias("1dose1prayerrestore").unwrap().id);
                scene.stat(5, 17, 43);
            }
            Some(HostEffect::Interaction(InteractReq::Held { action, .. })) if action == "Eat" => {
                food_tick = Some(tick);
                scene.inventory.clear();
                scene.stat(3, 31, 40);
            }
            None => {},
            _ => panic!("unexpected synthetic-driver acknowledgement"),
        }
    }
    assert_eq!(drink_tick, Some(3));
    assert_eq!(prayer_tick, Some(6));
    assert_eq!(food_tick, Some(11));
    assert!(routed.contains(&7));
    assert!(routed.contains(&12));
    assert_eq!(harness.machine.counters.locked, 2);
    assert_eq!(harness.machine.counters.restorations, 2);
}
#[test]
fn case41_protect_on_ready_deadline_restores_swing_same_tick() {
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
    // The onset is observed at 11, but the prayer only becomes available at
    // 12. A wanted click at 11 would not lose a swing under the old planner.
    scene.stat(5, 0, 43);
    scene.refresh();
    assert!(harness.pending(&scene, 11).is_none());
    scene.npcs[0].animation_frame = 1;
    scene.stat(5, 20, 43);
    scene.refresh();

    // Case 41 is the ready-tick oracle: the core must plan both the free
    // reducer and the restoring Attack at tick 12, before the player phase.
    assert!(matches!(harness.poll(&scene.snapshot, 12), Poll::Pending));
    let outbox = &harness.runtime.ledger.as_ref().unwrap().outbox;
    assert_eq!(outbox.len(), 2, "the ready tick must contain Protect + Attack");
    assert!(matches!(
        &outbox[0].effect,
        HostEffect::Interaction(InteractReq::IfButton { component_id })
            if *component_id == protect.button_com
    ));
    assert!(matches!(
        &outbox[1].effect,
        HostEffect::Interaction(InteractReq::Npc { action, index: Some(7), .. })
            if action == "Attack"
    ));
    assert_eq!(outbox[1].request_id.get(), outbox[0].request_id.get() + 1);
    assert_eq!(outbox[0].batch, outbox[1].batch);
    assert_eq!(harness.machine.cycle().deadline, 12);
}
