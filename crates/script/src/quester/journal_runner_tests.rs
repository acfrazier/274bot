use super::*;
use crate::native::{
    ActionContext, ActionError, HostEffect, InteractionReceipt, NativeActions, NativeMachine,
    NativeOutput, RetainedMemory, ScriptStatus,
};
use crate::quester::conversation::CarriedConversation;
use crate::quester::families::tests::{
    with_tick, with_tick_bank, with_tick_output, with_tick_output_bank, with_tick_retained,
};
use crate::quester::path::{PredicateDocument, ProgressRuleDocument};
use api::selected::Truth;
use api::snapshot::{GameSnapshot, QuestStatusView, VarpView};
use std::sync::atomic::{AtomicBool, Ordering};

fn fixture(journal: bool) -> (Quester, GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut doc = super::super::compile::decode_cook().unwrap();
    // Isolate progress transitions from the independently tested provisioning service.
    doc.quest.as_mut().unwrap().owns_inventory = true;
    let step = &mut doc.roles[0].sequences[1].steps[0];
    step.kind = "wait".into();
    step.args = serde_json::json!({"until":{"All":[]},"max_ticks":10});
    step.skip_if = PredicateDocument::Any(vec![]);
    step.advances = Some(journal);
    step.settle = if journal {
        PredicateDocument::Fact {
            kind: "stage_in".into(),
            version: 1,
            args: serde_json::json!({"quest":"cook","any":["cook:mid"]}),
        }
    } else {
        PredicateDocument::All(vec![])
    };
    if journal {
        let mut mid = doc.roles[0].sequences[1].clone();
        mid.stage = FactKey::new("cook:mid");
        mid.terminal = true;
        mid.steps.clear();
        doc.roles[0].sequences.push(mid);
        doc.roles[0].progress.as_mut().unwrap().rules = vec![
            ProgressRuleDocument {
                stage: FactKey::new("cook:mid"),
                all: vec!["next branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(2),
            },
            ProgressRuleDocument {
                stage: FactKey::new("cook:1"),
                all: vec!["seeded branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(1),
            },
            ProgressRuleDocument {
                stage: FactKey::new("cook:0"),
                all: vec!["first branch".into()],
                any: vec![],
                not: vec![],
                varp: Some(0),
            },
        ];
    }
    let path = super::super::compile::compile_uncached_for_test(&doc, &data, &quests).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    (
        Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        ),
        snapshot,
    )
}

type Ledger = Option<Box<crate::native::ledger::Ledger>>;
fn drive(
    script: &mut Quester,
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
) -> ScriptFlow {
    with_tick(snapshot, ledger, tick, |t| script.tick(t).unwrap())
}
/// One tick whose interaction event is already spent before the runner
/// polls, as by an earlier action on the same observed tick. A begun journal
/// then waits for its click, and a capture waits for its close (TICK-FIX #6
/// sends both on the begin/capture tick when the event is free), so these
/// tests can still reach the Busy and lost-before-close paths.
fn drive_spent(
    script: &mut Quester,
    snapshot: &GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
) -> ScriptFlow {
    with_tick(snapshot, ledger, tick, |t| {
        assert!(t.cx.budget.event(false), "the tick's event was free");
        script.tick(t).unwrap()
    })
}
fn ack(ledger: &mut Ledger, tick: u64) -> HostEffect {
    let ledger = ledger.as_mut().unwrap();
    let action = ledger.outbox.remove(0);
    ledger.complete_interaction(
        &action.authority(),
        InteractionReceipt {
            request_id: action.request_id.get(),
            evidence: api::quest_progress::EvidenceStamp {
                run: action.run(),
                tick,
                sequence: tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    action.effect
}
fn journal(snapshot: &mut GameSnapshot, body: &str) {
    snapshot.seed_main_modal(
        8134,
        vec![
            crate::quest_journal::test_widget(8144, "@dre@The Cook's Quest"),
            crate::quest_journal::test_widget(8145, body),
        ],
    );
}
fn finish_read(
    script: &mut Quester,
    snapshot: &mut GameSnapshot,
    ledger: &mut Ledger,
    tick: u64,
    body: &str,
) {
    drive(script, snapshot, ledger, tick);
    assert!(matches!(
        ack(ledger, tick),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
    journal(snapshot, body);
    drive(script, snapshot, ledger, tick + 1);
    drive(script, snapshot, ledger, tick + 2);
    assert!(matches!(
        ack(ledger, tick + 2),
        HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    drive(script, snapshot, ledger, tick + 3);
}

struct PendingNativeAction {
    cancelled: Arc<AtomicBool>,
}

impl NativeMachine for PendingNativeAction {
    type Args = Arc<AtomicBool>;
    type Output = ();

    fn begin(cancelled: Self::Args, _cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        Ok(Self { cancelled })
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        match cx.emit(crate::shim::InteractReq::IfButton { component_id: 9000 }) {
            Ok(_) => Poll::Pending,
            Err(error) => Poll::Ready(Err(error)),
        }
    }

    fn cancel(&mut self) {
        self.cancelled.store(true, Ordering::Release);
    }
}

struct OwnedActionStep {
    handle: Option<ActionHandle<PendingNativeAction>>,
}

impl StepRun for OwnedActionStep {
    fn poll(&mut self, _cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Pending
    }

    fn cancel(&mut self, actions: &mut NativeActions) {
        if let Some(handle) = self.handle.take() {
            actions.cancel(handle);
        }
    }
}

/// A step that stays in flight: an instantly complete wait would now settle
/// and reread on its begin tick (TICK-FIX #5/#6), leaving no live step.
struct IdlePlan;

impl super::super::compile::StepPlan for IdlePlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(OwnedActionStep { handle: None }))
    }
}

fn random_event() -> DetectedRandom {
    DetectedRandom {
        kind: api::random::RandomKind::Dialog,
        name: "genie".into(),
        ours: true,
        npc_index: Some(0),
    }
}

#[test]
fn random_event_revokes_active_step_owner_and_rereads_progress() {
    let (mut script, mut snapshot) = fixture(true);
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].plan = Arc::new(IdlePlan);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(script.progress().is_some());

    let cancelled = Arc::new(AtomicBool::new(false));
    with_tick(&snapshot, &mut ledger, 6, |tick| {
        let handle = tick
            .actions
            .begin::<PendingNativeAction>(Arc::clone(&cancelled), &mut tick.cx)
            .unwrap();
        assert!(matches!(
            tick.actions.poll(&handle, &mut tick.cx),
            Poll::Pending
        ));
        script.step = Some(Box::new(OwnedActionStep {
            handle: Some(handle),
        }));
    });
    let authority = ledger.as_ref().unwrap().outbox[0].authority();
    assert!(authority.live());

    script.attempts = 3;
    script.journal_attempts = 2;
    script.journal_retry_pending = true;
    script.journal_quiet_since = NonZeroU32::new(3);
    script.selection_since = Some(Duration::from_secs(5));
    script.unreadable_since = Some(Duration::from_secs(6));
    script.dirty = false;

    assert_eq!(script.on_random(&random_event()), RandomClaim::Host);
    assert!(cancelled.load(Ordering::Acquire));
    assert!(!authority.live());
    assert!(script.step.is_none());
    assert!(script.progress.is_none());
    assert!(script.needs_read && script.dirty);
    assert!(!script.prayer_cleanup_pending);
    assert_eq!(script.attempts, 0);
    assert_eq!(script.journal_attempts, 0);
    assert!(!script.journal_retry_pending);
    assert!(script.journal_quiet_since.is_none());
    assert!(script.selection_since.is_none());
    assert!(script.unreadable_since.is_none());
    assert!(!script.parked);
    assert!(!ledger.as_mut().unwrap().outbox.remove(0).live());

    drive(&mut script, &snapshot, &mut ledger, 7);
    finish_read(&mut script, &mut snapshot, &mut ledger, 8, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    assert_eq!(
        script.last_journal().unwrap().lines[0].as_ref(),
        "next branch"
    );
}

#[test]
fn random_event_revokes_journal_owner_and_discards_settlement_state() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(script.settling && script.needs_read);
    assert!(script.last_outcome.is_some());
    drive(&mut script, &snapshot, &mut ledger, 7);
    drive(&mut script, &snapshot, &mut ledger, 8);
    assert!(script.journal.is_some());

    let authority = ledger.as_ref().unwrap().outbox[0].authority();
    assert!(authority.live());
    script.journal_attempts = 2;
    script.journal_retry_pending = true;
    script.journal_quiet_since = NonZeroU32::new(3);
    script.dirty = false;

    assert_eq!(script.on_random(&random_event()), RandomClaim::Host);
    assert!(!authority.live());
    assert!(script.journal.is_none());
    assert!(script.progress.is_none());
    assert!(script.last_outcome.is_none());
    assert!(!script.settling);
    assert_eq!(script.settle_deadline, Duration::ZERO);
    assert_eq!(script.journal_attempts, 0);
    assert!(!script.journal_retry_pending);
    assert!(script.journal_quiet_since.is_none());
    assert!(script.needs_read && script.dirty);
    assert!(!script.prayer_cleanup_pending);
    assert!(!ledger.as_mut().unwrap().outbox.remove(0).live());

    drive(&mut script, &snapshot, &mut ledger, 9);
    finish_read(&mut script, &mut snapshot, &mut ledger, 10, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    assert!(!script.settling);
}

#[test]
fn random_event_revokes_clear_prayer_owner_and_preserves_parked_state() {
    let (mut script, mut snapshot) = fixture(true);
    let prayer_varp = script.selected.prayers()[0].varp;
    snapshot.seed_varps(vec![VarpView {
        index: prayer_varp,
        value: 1,
    }]);
    script.parked = true;
    script.park_reason = "pre-existing park";
    script.last_error = Some(Arc::from("pre-existing failure"));
    let mut ledger = None;
    script.prayer_cleanup_owned.accepted(prayer_varp, true, 0);
    script.prayer_cleanup_pending = true;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    assert!(script.clear_prayers.is_some());
    let authority = ledger.as_ref().unwrap().outbox[0].authority();
    assert!(authority.live());

    assert_eq!(script.on_random(&random_event()), RandomClaim::Host);
    assert!(!authority.live());
    assert!(script.clear_prayers.is_none());
    assert!(script.prayer_cleanup_pending);
    assert!(script.needs_read && script.dirty);
    assert!(script.parked);
    assert_eq!(script.last_error.as_deref(), Some("pre-existing failure"));

    assert!(!ledger.as_mut().unwrap().outbox.remove(0).live());
}

#[test]
fn policy_s2_resume_hands_accepted_combat_raise_to_scoped_cleanup() {
    use api::snapshot::{ActorKind, ActorTargetView, NpcView, ProjectileView, StatView, WorldTile};
    for wraps_recipe in [false, true] {
        let (mut script, mut snapshot) = fixture(false);
        let skin = script
            .selected
            .prayer_by_name("Thick Skin")
            .unwrap()
            .clone();
        let original = script
            .selected
            .prayer_by_name("Protect from Melee")
            .unwrap()
            .clone();
        let replacement = script
            .selected
            .prayer_by_name("Protect from Missiles")
            .unwrap()
            .clone();
        let here = WorldTile {
            x: 3200,
            z: 3200,
            level: 0,
        };
        let mut local = super::super::families::tests::local_player(here);
        local.player.actor.health = 40;
        local.player.actor.total_health = 40;
        local.player.actor.in_combat = true;
        local.player.actor.target = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 7,
        });
        snapshot.seed_local_player(local);
        snapshot.seed_world(api::snapshot::WorldStateView::default());
        snapshot.seed_players(vec![]);
        snapshot.seed_hitmarks(api::snapshot::HitmarksView {
            marks: [api::snapshot::HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot.seed_chat_lines(vec![]);
        snapshot.seed_inventory(vec![], 28);
        snapshot.seed_equipment(vec![]);
        snapshot.seed_stats(
            (0..25)
                .map(|index| StatView {
                    index,
                    name: api::snapshot::stat_name(index as usize).into(),
                    effective: if index == 5 { 43 } else { 40 },
                    base: if index == 5 { 43 } else { 40 },
                    xp: 0,
                    used: api::snapshot::stat_used(index as usize),
                })
                .collect(),
        );
        let npc = script.selected.npc_by_config("imp").unwrap();
        snapshot.seed_npcs(vec![NpcView {
            index: 7,
            r#type: Some(npc.id as usize),
            name: npc.display.clone(),
            actions: vec![Some("Attack".into())],
            tile: here,
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
            health: 8,
            total_health: 8,
            face_entity: -1,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            moving: false,
            running: false,
            in_combat: true,
            level: 2,
            size: 1,
            network: here,
            x: here.x,
            z: here.z,
            yaw: 0,
        }]);
        snapshot.seed_projectiles(vec![ProjectileView {
            spotanim: 9,
            level: 0,
            src: here,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            t1: 0,
            t2: 30,
        }]);
        let prayers = |protect_varp| {
            script
                .selected
                .prayers()
                .iter()
                .map(|row| VarpView {
                    index: row.varp,
                    value: i32::from(row.varp == skin.varp || row.varp == protect_varp),
                })
                .chain([VarpView {
                    index: crate::combat::OPTION_NODEF,
                    value: 0,
                }])
                .collect()
        };
        snapshot.seed_varps(prayers(original.varp));
        let raised_varps = prayers(replacement.varp);
        let cleared_varps = prayers(-1);
        script.needs_read = false;
        script.stage = Some(FactKey::new("cook:1"));
        let mut ledger = None;
        script.step = Some(with_tick(&snapshot, &mut ledger, 0, |tick| {
            let required_after = tick.cx.evidence();
            let mut cx = StepContext {
                tick,
                quests: &script.quests,
                progress: &[],
                required_after,
                banks: &script.banks,
                choices: &script.choices,
            };
            let run = super::super::families::combat::tests::policy_s2_run_for_runner(&mut cx);
            if wraps_recipe {
                super::super::families::tests::policy_s2_recipe_run(run)
            } else {
                run
            }
        }));
        drive(&mut script, &snapshot, &mut ledger, 1);
        let buttons: Vec<_> = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter_map(|action| match &action.effect {
                HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id }) => {
                    Some(*component_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(buttons, vec![replacement.button_com]);
        let combat_authority = ledger.as_ref().unwrap().outbox[0].authority();
        while !ledger.as_ref().unwrap().outbox.is_empty() {
            ack(&mut ledger, 1);
        }
        snapshot.seed_varps(raised_varps);
        // No Combat poll consumes the receipt before this cancellation handoff.
        script.interrupt(Interrupt::Resume);
        assert!(!combat_authority.owner_live());
        assert!(script.prayer_cleanup_pending);
        assert!(script.step.is_none());
        script.parked = true;
        drive(&mut script, &snapshot, &mut ledger, 2);
        assert!(script.clear_prayers.is_some());
        drive(&mut script, &snapshot, &mut ledger, 3);
        assert_eq!(ledger.as_ref().unwrap().outbox.len(), 1);
        assert!(matches!(ack(&mut ledger, 3),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id })
            if component_id == replacement.button_com));
        snapshot.seed_varps(cleared_varps);
        drive(&mut script, &snapshot, &mut ledger, 4);
        assert!(!script.prayer_cleanup_pending);
        assert!(script.clear_prayers.is_none());
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
        assert_eq!(
            snapshot
                .varps()
                .iter()
                .find(|row| row.index == skin.varp)
                .unwrap()
                .value,
            1
        );
        assert_eq!(
            snapshot
                .varps()
                .iter()
                .find(|row| row.index == original.varp)
                .unwrap()
                .value,
            0
        );
        assert_eq!(
            snapshot
                .varps()
                .iter()
                .find(|row| row.index == replacement.varp)
                .unwrap()
                .value,
            0
        );
    }
}

#[test]
fn lifecycle_followups_death_preserves_a_respawn_user_prayer_across_pause_and_hold() {
    use api::snapshot::{
        ActorKind, ActorTargetView, ChatLineView, NpcView, ProjectileView, StatView, VarpView,
        WorldTile,
    };

    for (freeze, thaw) in [
        (Interrupt::Pause, Interrupt::Resume),
        (Interrupt::Hold(true), Interrupt::Hold(false)),
    ] {
        let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
        let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
        let path = super::super::compile::prepare_for_test({
            let data = Arc::clone(&data);
            let quests = Arc::clone(&quests);
            move |cap| {
                super::super::compile::compile_path(
                    include_bytes!("../../paths/289/fixtures/combat_melee_food_only.json"),
                    &data,
                    &quests,
                    cap,
                )
            }
        })
        .unwrap();
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&path),
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        script.stage = Some(path.colour_not_started.clone());
        script.needs_read = false;

        let here = WorldTile {
            x: 2458,
            z: 3303,
            level: 0,
        };
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![], 28);
        snapshot.seed_equipment(vec![]);
        let mut local = super::super::families::tests::local_player(here);
        local.player.actor.health = 40;
        local.player.actor.total_health = 40;
        local.player.actor.in_combat = true;
        local.player.actor.target = Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: 7,
        });
        snapshot.seed_local_player(local);
        snapshot.seed_world(api::snapshot::WorldStateView::default());
        snapshot.seed_players(vec![]);
        snapshot.seed_hitmarks(api::snapshot::HitmarksView {
            marks: [api::snapshot::HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 0,
        });
        snapshot.seed_chat_lines(vec![]);

        let warlord = data.npc_by_config("khazard_warlord").unwrap();
        snapshot.seed_npcs(vec![NpcView {
            index: 7,
            r#type: Some(warlord.id as usize),
            name: warlord.display.clone(),
            actions: vec![Some("Attack".into())],
            tile: here,
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
            health: 8,
            total_health: 8,
            face_entity: -1,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            moving: false,
            running: false,
            in_combat: true,
            level: 2,
            size: 1,
            network: here,
            x: here.x,
            z: here.z,
            yaw: 0,
        }]);
        snapshot.seed_projectiles(vec![ProjectileView {
            spotanim: 9,
            level: 0,
            src: here,
            target: Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            t1: 0,
            t2: 30,
        }]);
        let stats = |hitpoints| {
            (0..25)
                .map(|index| StatView {
                    index,
                    name: api::snapshot::stat_name(index as usize).into(),
                    effective: if index == 3 {
                        hitpoints
                    } else if index == 5 {
                        43
                    } else {
                        40
                    },
                    base: if index == 3 {
                        hitpoints
                    } else if index == 5 {
                        43
                    } else {
                        40
                    },
                    xp: 0,
                    used: api::snapshot::stat_used(index as usize),
                })
                .collect::<Vec<_>>()
        };
        snapshot.seed_stats(stats(40));

        let skin = data.prayer_by_name("Thick Skin").unwrap();
        let original = data.prayer_by_name("Protect from Melee").unwrap();
        let raised = data.prayer_by_name("Protect from Missiles").unwrap();
        let prayers = |protect_varp| {
            data.prayers()
                .iter()
                .map(|row| VarpView {
                    index: row.varp,
                    value: i32::from(row.varp == skin.varp || row.varp == protect_varp),
                })
                .chain([VarpView {
                    index: crate::combat::OPTION_NODEF,
                    value: 0,
                }])
                .collect()
        };
        snapshot.seed_varps(prayers(original.varp));

        let mut ledger = None;
        assert_eq!(
            drive(&mut script, &snapshot, &mut ledger, 1),
            ScriptFlow::Continue
        );
        assert_eq!(
            drive(&mut script, &snapshot, &mut ledger, 2),
            ScriptFlow::Continue
        );
        let buttons: Vec<_> = ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter_map(|action| match &action.effect {
                HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id }) => {
                    Some(*component_id)
                }
                _ => None,
            })
            .collect();
        assert_eq!(buttons, vec![raised.button_com]);
        assert!(matches!(
            ack(&mut ledger, 2),
            HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id })
                if component_id == raised.button_com
        ));
        assert!(script
            .step
            .as_ref()
            .expect("production Quester tick starts Combat")
            .prayer_cleanup()
            .contains(raised.varp));

        snapshot.seed_varps(prayers(-1));
        snapshot.seed_stats(stats(0));
        snapshot.seed_chat_lines(vec![ChatLineView {
            type_: 0,
            username: None,
            text: "Oh dear, you are dead!".into(),
            sequence: 1,
        }]);
        assert_eq!(
            drive(&mut script, &snapshot, &mut ledger, 3),
            ScriptFlow::Continue
        );
        assert_eq!(script.deaths, 1, "the native DeathLatch detects death");
        assert!(script.step.is_none(), "death cancels the live Combat step");

        script.interrupt(freeze);
        snapshot.seed_stats(stats(40));
        snapshot.seed_chat_lines(vec![]);
        let mut respawned = super::super::families::tests::local_player(here);
        respawned.player.actor.health = 40;
        respawned.player.actor.total_health = 40;
        snapshot.seed_local_player(respawned);
        snapshot.seed_varps(prayers(raised.varp));
        script.interrupt(thaw);
        drive(&mut script, &snapshot, &mut ledger, 4);
        drive(&mut script, &snapshot, &mut ledger, 5);

        assert!(
            script.clear_prayers.is_none(),
            "death must revoke any stale scoped clear before Resume"
        );
        assert!(!script.prayer_cleanup_pending);
        assert_eq!(
            snapshot
                .varps()
                .iter()
                .find(|row| row.index == raised.varp)
                .unwrap()
                .value,
            1,
            "the user's prayer raised after respawn stays on"
        );
        assert!(
            ledger
                .as_ref()
                .unwrap()
                .outbox
                .iter()
                .all(|action| !matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::shim::InteractReq::IfButton { component_id })
                        if *component_id == raised.button_com
                )),
            "resumption must not send the old Combat-owned prayer clear"
        );
    }
}

#[test]
fn unknown_skip_never_dispatches_and_wait_is_bounded() {
    let (mut script, mut snapshot) = fixture(false);
    let path = Arc::get_mut(&mut script.path).unwrap();
    struct InventoryUnknown;
    impl super::super::compile::PredicatePlan for InventoryUnknown {
        fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth {
            if cx.cx.snapshot().inventory().is_some() {
                Truth::False
            } else {
                Truth::Unknown
            }
        }
    }
    path.sequences[1].steps[0].skip_if = Arc::new(InventoryUnknown);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(
        script.step.is_none() && script.begun.is_empty(),
        "unknown inventory must not start a wait/action"
    );
    for tick in 2..20 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    assert!(!script.parked);
    snapshot.seed_inventory(vec![], 28);
    drive(&mut script, &snapshot, &mut ledger, 20);
    assert!(!script.begun.is_empty(), "the known skip begins the step");
    let (mut script, snapshot) = fixture(false);
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].skip_if =
        Arc::new(InventoryUnknown);
    for tick in 1..40 {
        drive(&mut script, &snapshot, &mut None, tick);
    }
    assert!(script.parked);
    assert_eq!(script.park_reason, "skip predicate evidence unavailable");
}

/// TICK-FIX #6 (E-Q5): a progress read clicks its quest row on the tick the
/// journal is begun, not a tick later.
#[test]
fn journal_read_clicks_its_row_on_the_begin_tick() {
    let (mut script, snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(script.journal_opened());
    assert!(ledger.as_ref().is_some_and(|ledger| matches!(
        ledger.outbox.first().map(|action| &action.effect),
        Some(HostEffect::Interaction(
            crate::shim::InteractReq::IfButton { .. }
        ))
    )));
}

/// TICK-FIX #4/#6 (E-Q2/E-Q3): the read's close tick selects the advancing
/// step, begins and polls it; it completes, and its reread clicks the quest
/// row on that same tick.
#[test]
fn advancing_step_rereads_on_its_completion_tick() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(script.settling && script.needs_read);
    assert!(ledger.as_ref().is_some_and(|ledger| matches!(
        ledger.outbox.first().map(|action| &action.effect),
        Some(HostEffect::Interaction(
            crate::shim::InteractReq::IfButton { .. }
        ))
    )));
}
/// An advancing step that killed its target (combat receipt, end Killed).
struct KillPlan;

impl super::super::compile::StepPlan for KillPlan {
    fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        Ok(Box::new(KillRun))
    }
}

struct KillRun;

impl StepRun for KillRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let evidence = cx.tick.cx.evidence();
        let report = crate::combat::CombatReport {
            end: crate::combat::CombatEnd::Killed,
            evidence,
            engaged: Some(crate::combat::ActorRef {
                kind: api::snapshot::ActorKind::Npc,
                index: 7,
            }),
            engaged_npc_type: 1047,
            ticks: 9,
            swings: 3,
            casts: 0,
            damage_taken: 0,
            food: 0,
            prayer_doses: 0,
            boost_doses: 0,
            antifire_doses: 0,
            hits_while_protected: 0,
            protect_switches: 0,
            intruders: 0,
            ammo_pickups: 0,
            restorations: 0,
            locked_ticks: 0,
            multi_op_plans: 0,
            melee_mode_fallback: None,
            flick_resets: 0,
            flick_misses: 0,
            flick_fallback: false,
        };
        Poll::Ready(Ok(StepOutcome {
            progress: None,
            evidence,
            receipt: Some(Arc::new(super::super::families::combat::CombatReceipt {
                report,
                target_gone_restarts: 0,
            })),
        }))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}
}

fn dying_npc(index: usize) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index,
        r#type: Some(1047),
        name: Some("Temple guardian".into()),
        actions: vec![],
        tile: api::WorldTile {
            x: 3431,
            z: 9897,
            level: 0,
        },
        distance: 1,
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 0,
        total_health: 1,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 1,
        size: 1,
        network: api::WorldTile {
            x: 3431,
            z: 9897,
            level: 0,
        },
        x: 0,
        z: 0,
        yaw: 0,
    }
}

/// The content writes quest progress from the killed NPC's death queue after
/// `npc_death` removes it (`npc_death.rs2:10-25`, Priest in Peril
/// `temple_guardian.rs2:1-12`), so an advancing combat step's read waits for
/// that NPC to leave the scene (bounded) instead of reading the old stage on
/// the kill tick and then waiting out the settle window (live pip2p: 336).
#[test]
fn advancing_kill_reads_progress_once_the_killed_npc_is_gone() {
    let (mut script, mut snapshot) = fixture(true);
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].plan = Arc::new(KillPlan);
    snapshot.seed_npcs(vec![dying_npc(7)]);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    // Tick 5 closed the first read, began the kill step and completed it.
    assert!(script.settling && script.needs_read);
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "no read while the NPC dies"
    );
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(
        ledger.as_ref().unwrap().outbox.is_empty(),
        "no read while the NPC dies"
    );
    snapshot.seed_npcs(Vec::new());
    drive(&mut script, &snapshot, &mut ledger, 7);
    assert!(matches!(
        ack(&mut ledger, 7),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
}

/// The wait is bounded: an NPC that lingers does not hold the read past
/// `KILL_READ_BOUND_TICKS`.
#[test]
fn advancing_kill_read_wait_is_bounded() {
    let (mut script, mut snapshot) = fixture(true);
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].plan = Arc::new(KillPlan);
    snapshot.seed_npcs(vec![dying_npc(7)]);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    for tick in 6..11 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(ledger.as_ref().unwrap().outbox.is_empty());
    }
    drive(&mut script, &snapshot, &mut ledger, 11);
    assert!(matches!(
        ack(&mut ledger, 11),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
}

#[test]
fn advances_rereads_before_stage_settle_then_retargets_new_sequence() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(
        &mut script,
        &mut snapshot,
        &mut ledger,
        2,
        "@str@seeded branch",
    );
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(script.settling && script.needs_read);
    drive(&mut script, &snapshot, &mut ledger, 7);
    assert!(script.settling, "old proof cannot settle advances");
    finish_read(&mut script, &mut snapshot, &mut ledger, 8, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    assert!(!script.settling);
    assert!(script.progress().unwrap().signals.is_empty());
    assert_eq!(script.last_journal().unwrap().closed.tick, 11);
    assert_eq!(
        drive(&mut script, &snapshot, &mut ledger, 12),
        ScriptFlow::Complete
    );
}

#[test]
fn no_match_parks_with_raw_lines_and_no_invented_stage() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(
        &mut script,
        &mut snapshot,
        &mut ledger,
        2,
        "unrecognised content branch",
    );
    assert!(script.parked);
    assert!(script.stage().is_none());
    assert!(script.step.is_none());
    assert_eq!(
        script.last_journal().unwrap().lines[0].as_ref(),
        "unrecognised content branch"
    );
    assert!(matches!(
        script.progress().unwrap().stage,
        Knowledge::Unknown(_)
    ));
    assert!(
        script.journal_text.is_none(),
        "published journal text is not resent"
    );
}

#[test]
fn explicit_read_is_boundary_latched_and_colour_only_never_opens_journal() {
    let (mut script, snapshot) = fixture(false);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    script.read_journal().unwrap();
    assert!(script.read_requested && !script.needs_read);
    // The latched read waits for a step boundary; the always-ready wait
    // reaches one within a tick or two now that a completed step settles on
    // its own tick (TICK-FIX #5).
    for tick in 2..=4 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    assert!(!script.read_requested);
    assert!(!script.journal_opened());
    assert!(script.last_journal().is_none());
    assert!(ledger.as_ref().is_none_or(|l| l.outbox.is_empty()));
}

#[test]
fn recipe_advances_preserves_recipe_until_fresh_stage_settles() {
    let (mut script, mut snapshot) = fixture(true);
    let step = &mut Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0];
    step.plan = Arc::new(super::super::families::AcquirePlan {
        recipe: Arc::from("synthetic-journal"),
        steps: Arc::from(vec![super::super::families::CompiledAcquireStep {
            id: step.id.clone(),
            advances: true,
            skip_if: Arc::clone(&step.skip_if),
            skip_if_summary: Arc::clone(&step.skip_if_summary),
            settle: Arc::clone(&step.settle),
            plan: Arc::clone(&step.plan),
        }]),
        ..super::super::families::AcquirePlan::default()
    });
    step.advances = false;
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    finish_read(&mut script, &mut snapshot, &mut ledger, 2, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(script.step.is_some() && script.needs_read && !script.settling);
    drive(&mut script, &snapshot, &mut ledger, 7);
    finish_read(&mut script, &mut snapshot, &mut ledger, 8, "next branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:mid");
    // The fresh stage settles the root step on the read's close tick, and the
    // terminal stage completes the run (TICK-FIX #5/#6).
    assert_eq!(
        drive(&mut script, &snapshot, &mut ledger, 12),
        ScriptFlow::Complete
    );
}

#[test]
fn stopping_mid_read_revokes_host_click_and_quiet_lease() {
    let (mut script, snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    script.on_stop(StopReason::Operator);
    let ledger = ledger.as_mut().unwrap();
    assert!(!ledger.outbox[0].live());
    assert!(ledger.quiet_read(std::time::Instant::now()).is_none());
    assert!(script.last_journal().is_none());
}

#[test]
fn blocked_slot_read_journal_is_terminal_and_retains_its_diagnostic() {
    let (mut script, snapshot) = fixture(true);
    script.parked = true;
    script.run.session = 0;
    let mut slot = crate::SlotScript::new();
    slot.bind_incarnation(1);
    slot.start_test_script(Box::new(script), None).unwrap();
    let run = slot.native_run().unwrap();
    slot.on_game_tick(&mut crate::ScriptCtx {
        driver: &mut crate::ctx::test_support::NullDriver::default(),
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: Some(&snapshot),
        obj_names: None,
        compiled: crate::CompiledTick::default(),
    });
    assert_eq!(slot.state(), crate::RunState::Idle);
    assert!(slot.native_run().is_none());
    let status = slot
        .native_status()
        .expect("terminal blocked diagnostic survives Stop");
    assert_eq!(status.run, run);
    assert_eq!(status.phase, NativePhase::Blocked);
    assert_eq!(
        status.failure.as_ref().unwrap().message.as_ref(),
        "no progress"
    );
}

#[test]
fn transient_busy_during_read_retries_but_repeated_busy_is_bounded() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    // The begin tick's event is spent, so the click waits for tick 2, where a
    // foreign modal makes the Click phase Busy.
    drive_spent(&mut script, &snapshot, &mut ledger, 1);
    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    assert!(!script.parked, "first in-flight Busy must not park");
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 3..=6 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    finish_read(&mut script, &mut snapshot, &mut ledger, 7, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);

    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    for tick in 1..40 {
        snapshot.seed_main_modal(-1, vec![]);
        drive_spent(&mut script, &snapshot, &mut ledger, tick * 2 - 1);
        snapshot.seed_main_modal(123, vec![]);
        drive(&mut script, &snapshot, &mut ledger, tick * 2);
        if script.parked {
            break;
        }
    }
    assert!(script.parked, "transient retry must still have a wall");
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
}

#[test]
fn repeated_busy_transactions_report_retry_limit_and_cause() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive_spent(&mut script, &snapshot, &mut ledger, 1);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 3..=5 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    drive_spent(&mut script, &snapshot, &mut ledger, 6);
    assert_eq!(script.journal_attempts, 2);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 7);
    snapshot.seed_main_modal(-1, vec![]);
    for tick in 8..=10 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    drive_spent(&mut script, &snapshot, &mut ledger, 11);
    assert_eq!(script.journal_attempts, 3);

    snapshot.seed_main_modal(123, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 12);
    assert!(script.parked);
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal read retry limit reached (journal remained busy during read)"
    );
}

#[test]
fn ownership_lost_before_close_retries_without_closing_another_modal() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    // A spent capture tick leaves the close for the next poll, where the page
    // is already gone (lost before close).
    drive_spent(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    assert!(!script.parked);
    assert!(ledger.as_ref().unwrap().outbox.is_empty());
    for tick in 5..=8 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    finish_read(&mut script, &mut snapshot, &mut ledger, 9, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
    assert_eq!(script.last_journal().unwrap().acquired.tick, 10);
}

#[test]
fn journal_modal_timeout_retries_the_read_instead_of_parking() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    assert!(matches!(
        ack(&mut ledger, 2),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
    // The accepted click never shows the journal within the acquire window.
    for tick in 3..=7 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    assert!(script.journal.is_none(), "the timed-out machine is dropped");
    assert!(!script.parked, "one missed acquire is not terminal");
    assert!(script.journal_retry_pending);
    for tick in 8..=11 {
        drive(&mut script, &snapshot, &mut ledger, tick);
    }
    finish_read(&mut script, &mut snapshot, &mut ledger, 12, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
}

#[test]
fn repeated_journal_modal_timeouts_cap_row_clicks_and_name_the_cause() {
    let (mut script, snapshot) = fixture(true);
    let mut ledger = None;
    let mut clicks = 0;
    for tick in 1..80 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            assert!(matches!(
                ack(&mut ledger, tick),
                HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
            ));
            clicks += 1;
        }
        if script.parked {
            break;
        }
    }
    assert!(script.parked);
    assert_eq!(clicks, 3, "a logical read must not spray row clicks");
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal read retry limit reached (journal modal repeatedly timed out)"
    );
}

#[test]
fn transient_journal_retry_requires_continuously_closed_observed_ticks() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    drive_spent(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    for tick in [5, 5, 5] {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(
            script.journal.is_none(),
            "same-frame retries must stay quiet"
        );
    }
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    drive(&mut script, &snapshot, &mut ledger, 6);
    snapshot.seed_chat_modal(-1, vec![]);
    for tick in 7..10 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(
            script.journal.is_none(),
            "chat must reset the quiet interval"
        );
    }
    drive(&mut script, &snapshot, &mut ledger, 10);
    finish_read(&mut script, &mut snapshot, &mut ledger, 11, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
}

#[test]
fn repeated_journal_ownership_loss_caps_row_clicks_per_read() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    let mut clicks = 0;
    let mut captured = false;
    for tick in 1..80 {
        // Each capture tick is spent, so the page vanishes before the close.
        if std::mem::take(&mut captured) {
            drive_spent(&mut script, &snapshot, &mut ledger, tick);
        } else {
            drive(&mut script, &snapshot, &mut ledger, tick);
        }
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            assert!(
                matches!(
                    ack(&mut ledger, tick),
                    HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
                ),
                "a lost page must never emit a compensating close"
            );
            clicks += 1;
            journal(&mut snapshot, "seeded branch");
            captured = true;
        } else {
            snapshot.seed_main_modal(-1, vec![]);
        }
        if script.parked {
            break;
        }
    }
    assert!(script.parked);
    assert_eq!(clicks, 3, "a logical read must not spray row clicks");
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal read retry limit reached (journal ownership repeatedly lost)"
    );
}

#[test]
fn colour_only_read_wait_preserves_colour_failure_with_open_chat() {
    let (mut script, mut snapshot) = fixture(false);
    snapshot.seed_quest_statuses(vec![], false);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    let mut ledger = None;
    for tick in 1..=35 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if script.parked {
            break;
        }
    }

    assert!(script.parked);
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "quest colour unavailable or unknown stage"
    );
}

#[test]
fn quiet_gate_wait_names_open_chat_when_it_parks() {
    let (mut script, mut snapshot) = fixture(true);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    drive(&mut script, &snapshot, &mut ledger, 2);
    ack(&mut ledger, 2);
    journal(&mut snapshot, "seeded branch");
    drive_spent(&mut script, &snapshot, &mut ledger, 3);
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 4);
    assert!(script.journal_retry_pending);

    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    for tick in 5..=40 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if script.parked {
            break;
        }
    }

    assert!(script.parked);
    assert_eq!(script.park_reason, "journal retry quiet period unavailable");
    assert!(script
        .blocked_failure()
        .message
        .contains("modal root 4882 (Aubury)"));
}

#[test]
fn named_chat_at_read_start_waits_then_recovers_or_names_the_timeout() {
    let (mut script, mut snapshot) = fixture(true);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(!script.parked, "a transient conversation tail must wait");
    snapshot.seed_chat_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, 2);
    finish_read(&mut script, &mut snapshot, &mut ledger, 3, "seeded branch");
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");

    let (mut script, mut snapshot) = fixture(true);
    snapshot.seed_chat_modal(4882, vec!["Aubury".into()]);
    for tick in 1..35 {
        drive(&mut script, &snapshot, &mut None, tick);
    }
    assert!(script.parked);
    assert!(script
        .last_error
        .as_ref()
        .is_some_and(|reason| reason.contains("4882") && reason.contains("Aubury")));
}

#[test]
fn rune_item_handoffs_reread_progress_before_selecting_recovery() {
    use crate::quester::families::tests::{def, seed_dialogue_combat};
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, WorldTile};

    fn finish_rune_read(
        script: &mut Quester,
        snapshot: &mut GameSnapshot,
        ledger: &mut Ledger,
        tick: u64,
        body: &str,
    ) {
        drive(script, snapshot, ledger, tick);
        assert!(
            matches!(
                ledger
                    .as_ref()
                    .and_then(|ledger| ledger.outbox.first())
                    .map(|action| &action.effect),
                Some(HostEffect::Interaction(
                    crate::shim::InteractReq::IfButton { .. }
                ))
            ),
            "a quest-changing handoff must query fresh journal progress before recovery"
        );
        ack(ledger, tick);
        snapshot.seed_main_modal(
            8134,
            vec![
                crate::quest_journal::test_widget(8144, "@dre@Rune Mysteries"),
                crate::quest_journal::test_widget(8145, body),
            ],
        );
        drive(script, snapshot, ledger, tick + 1);
        drive(script, snapshot, ledger, tick + 2);
        assert!(matches!(
            ack(ledger, tick + 2),
            HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
        ));
        snapshot.seed_main_modal(-1, vec![]);
        drive(script, snapshot, ledger, tick + 3);
    }

    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut document: crate::quester::path::PathDocument =
        serde_json::from_str(super::super::compile::RUNE_MYSTERIES_JSON).unwrap();
    document.quest.as_mut().unwrap().owns_inventory = true;
    let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    let spoken = "@str@I spoke to Duke Horacio";
    let talisman_pending =
        format!("{spoken}|I need to find the head wizard and give him the talisman");
    let package_pending =
        format!("{spoken}|I should take this Research Package to Aubury in Varrock");
    let package_delivered =
        format!("{spoken}|I took the research package to Varrock and delivered it.");
    let notes_received = format!("{package_delivered}|I should take the notes to Sedridor");
    let held = |alias: Option<&str>| {
        alias
            .map(|alias| ItemView {
                def: def(data.item_by_alias(alias).unwrap().id, alias),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            })
            .into_iter()
            .collect::<Vec<_>>()
    };
    for (step, input, output, before, after, next, here) in [
        (
            "deliver-talisman",
            Some("air_talisman"),
            Some("research_package"),
            talisman_pending.as_str(),
            package_pending.as_str(),
            "deliver-package",
            WorldTile {
                x: 3108,
                z: 9572,
                level: 0,
            },
        ),
        (
            "deliver-package",
            Some("research_package"),
            None,
            package_pending.as_str(),
            package_delivered.as_str(),
            "collect-notes",
            WorldTile {
                x: 3253,
                z: 3402,
                level: 0,
            },
        ),
        (
            "collect-notes",
            None,
            Some("research_notes"),
            package_delivered.as_str(),
            notes_received.as_str(),
            "deliver-notes",
            WorldTile {
                x: 3253,
                z: 3402,
                level: 0,
            },
        ),
    ] {
        let mut script = Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&path),
            Arc::clone(&data),
            Arc::clone(&quests),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        );
        let mut snapshot = GameSnapshot::new();
        snapshot.seed_ingame(2);
        seed_dialogue_combat(&mut snapshot, false);
        let mut player = snapshot.local_player().unwrap().clone();
        player.player.actor.tile = here;
        player.player.network = here;
        snapshot.seed_local_player(player);
        snapshot.seed_inventory(held(input), 28);
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: quests.quest("runemysteries").unwrap().display.to_string(),
                component_id: 42,
                colour: 0xf8f800,
            }],
            true,
        );
        let mut ledger = None;
        drive(&mut script, &snapshot, &mut ledger, 1);
        finish_rune_read(&mut script, &mut snapshot, &mut ledger, 2, before);
        assert_eq!(script.current_step().unwrap().id.0.as_ref(), step);
        drive(&mut script, &snapshot, &mut ledger, 6);
        assert!(matches!(
            ack(&mut ledger, 6),
            HostEffect::Interaction(crate::shim::InteractReq::Npc { .. })
        ));
        snapshot.seed_chat_modal(4893, vec!["A handoff page".into()]);
        snapshot.seed_chat_options(vec![], 4899);
        drive(&mut script, &snapshot, &mut ledger, 7);
        assert!(matches!(
            ack(&mut ledger, 7),
            HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog {
                component_id: None
            })
        ));
        snapshot.seed_chat_modal(-1, vec![]);
        snapshot.seed_chat_options(vec![], -1);
        snapshot.seed_inventory(held(output), 28);
        // The handoff dialogue completes one quiet tick after the close, and
        // the advancing step rereads on that same tick (TICK-FIX #6): drive
        // until the journal's row click is out, then finish that read.
        let mut tick = 8;
        while !ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.first().is_some_and(|action| {
                matches!(
                    action.effect,
                    HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
                )
            })
        }) {
            assert!(tick < 20, "{step}: the handoff must reread progress");
            drive(&mut script, &snapshot, &mut ledger, tick);
            tick += 1;
        }
        assert_eq!(
            tick, 10,
            "{step}: reread on the dialogue's completion tick 9"
        );
        finish_rune_read(&mut script, &mut snapshot, &mut ledger, tick, after);
        drive(&mut script, &snapshot, &mut ledger, tick + 4);
        assert_eq!(
            script.current_step().unwrap().id.0.as_ref(),
            next,
            "{step} must continue with fresh quest progress, not bank or Duke recovery"
        );
    }
}

fn sheep_complete_snapshot() -> (Quester, GameSnapshot) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut document: crate::quester::path::PathDocument =
        serde_json::from_str(super::super::compile::SHEEP_JSON).unwrap();
    document.quest.as_mut().unwrap().owns_inventory = true;
    let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Sheep Shearer".into(),
            component_id: 0,
            colour: 0x00F800,
        }],
        true,
    );
    (
        Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            path,
            Arc::clone(&data),
            quests,
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        ),
        snapshot,
    )
}

#[derive(Default)]
struct StatusCapture(Vec<ScriptStatus>);
impl NativeOutput for StatusCapture {
    fn status(&mut self, status: ScriptStatus) {
        self.0.push(status);
    }
    fn paint(&mut self, _: Arc<crate::shim::ScriptPaint>) {}
    fn log(&mut self, _: api::hostlog::Level, _: &str) {}
    fn settings_applied(&mut self, _: u64) {}
}

struct NeedsEvidenceStep {
    gates: Arc<[api::selected::QuestGate]>,
    name: Arc<str>,
}

impl StepRun for NeedsEvidenceStep {
    fn poll(&mut self, _cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        Poll::Ready(Err(ActionError::NeedsEvidence(Arc::clone(&self.gates))))
    }

    fn cancel(&mut self, _actions: &mut crate::native::NativeActions) {}

    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        Some(("Loadout observation", &self.name))
    }
}

fn run_needs_evidence_step(gates: Arc<[api::selected::QuestGate]>) -> (Quester, ScriptFlow) {
    let (mut script, snapshot) = fixture(false);
    script.step = Some(Box::new(NeedsEvidenceStep {
        gates,
        name: Arc::from("Waiting for inventory/equipment observation"),
    }));
    let mut ledger = None;
    let mut output = StatusCapture::default();
    let flow = with_tick_output(&snapshot, &mut ledger, 1, &mut output, |tick| {
        script.tick(tick).unwrap()
    });
    (script, flow)
}

#[test]
fn empty_gate_needs_evidence_uses_active_step_wait_detail_before_drop() {
    let empty_gates: Arc<[api::selected::QuestGate]> = Arc::from([]);
    let (script, flow) = run_needs_evidence_step(empty_gates);

    assert!(matches!(
        &flow,
        ScriptFlow::Blocked(failure) if failure.code.as_ref() == "needs-evidence"
    ));
    assert!(
        script.step.is_none(),
        "the refused owner must still be dropped"
    );
    assert_eq!(script.last_error_kind, QuesterFailureKind::NeedsEvidence);
    assert_eq!(
        script.last_error.as_deref(),
        Some("needs evidence: Loadout observation: Waiting for inventory/equipment observation")
    );
}

#[test]
fn nonempty_gate_needs_evidence_keeps_navigation_reason() {
    let gates: Arc<[api::selected::QuestGate]> = Arc::from([api::selected::QuestGate::Complete(
        api::selected::FactKey::new("cook:mid"),
    )]);
    let (script, flow) = run_needs_evidence_step(gates);

    assert!(matches!(
        &flow,
        ScriptFlow::Blocked(failure) if failure.code.as_ref() == "needs-evidence"
    ));
    assert_eq!(
        script.last_error.as_deref(),
        Some("walk needs authoritative quest-gate evidence")
    );
}

#[test]
fn provisioner_pending_preserves_semantic_outcome_and_status() {
    use crate::combat::{CombatEnd, CombatReport};
    use api::quest_progress::EvidenceStamp;

    let _isolated = crate::IsolatedEnv::enter("quester-provision-bank-semantic-outcome");
    let mut fixture = nested_bank_fixture(false);
    let mut injected_tick = None;
    let mut semantic_evidence = None;
    let mut post_injection_statuses = Vec::new();
    for tick in 1..=64 {
        let mut output = StatusCapture::default();
        fixture.drive_with_output(tick, &mut output);
        if injected_tick.is_none()
            && fixture.script.provisioner.status().phase
                == super::super::provision::ProvisionPhase::Acquiring
        {
            assert!(
                fixture.bank.known(),
                "the real provisioning scan must establish Session bank memory first"
            );
            assert_eq!(
                fixture.bank.origin(),
                api::bank_memory::Origin::Session,
                "the fixture bank is really opened, not hinted"
            );
            assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
            injected_tick = Some(tick);
            let evidence = EvidenceStamp {
                run: fixture.script.run,
                tick: 0,
                sequence: 0,
            };
            let report = CombatReport {
                end: CombatEnd::TargetGone,
                evidence,
                engaged: None,
                engaged_npc_type: 477,
                ticks: 1,
                swings: 0,
                casts: 0,
                damage_taken: 0,
                food: 0,
                prayer_doses: 0,
                boost_doses: 0,
                antifire_doses: 0,
                hits_while_protected: 0,
                protect_switches: 0,
                intruders: 0,
                ammo_pickups: 0,
                restorations: 0,
                locked_ticks: 0,
                multi_op_plans: 0,
                melee_mode_fallback: None,
                flick_resets: 0,
                flick_misses: 0,
                flick_fallback: false,
            };
            fixture.script.last_outcome = Some(StepOutcome {
                progress: None,
                evidence,
                receipt: Some(Arc::new(crate::quester::families::combat::CombatReceipt {
                    report,
                    target_gone_restarts: 1,
                })),
            });
            semantic_evidence = Some(evidence);
            continue;
        }

        if let Some(injected) = injected_tick {
            let semantic_evidence =
                semantic_evidence.expect("the semantic outcome must precede the acquisition");
            assert_eq!(
                fixture
                    .script
                    .last_outcome
                    .as_ref()
                    .map(|outcome| outcome.evidence),
                Some(semantic_evidence),
                "acquisition events (scan receipt, Acquired) must not replace the semantic outcome"
            );
            assert!(
                fixture.bank.known(),
                "the acquisition run must leave Session bank memory alone"
            );
            assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
            post_injection_statuses.extend(output.0);
            if tick >= injected + 6 {
                assert_eq!(
                    fixture.visits, 1,
                    "the known bank must not trigger a second scan trip"
                );
                assert!(
                    !fixture.script.parked,
                    "the acquisition cycles must not park: {:?}",
                    fixture.script.last_error
                );
                let status = post_injection_statuses
                    .last()
                    .expect("the acquisition cycle publishes status");
                assert!(
                    status.fields.iter().any(|field| {
                        field.key == "combat_end"
                        && matches!(&field.value, StatusValue::Text(value) if value.as_ref() == "TargetGone")
                    }),
                    "acquisition-cycle publication must retain the prior semantic status"
                );
                return;
            }
        }
    }
    panic!("the acquisition must run long enough to observe the preserved outcome");
}

#[test]
fn sheep_complete_colour_publishes_complete_and_slot_keeps_completed_receipt() {
    let (mut script, snapshot) = sheep_complete_snapshot();
    let mut ledger = None;
    let mut output = StatusCapture::default();
    let mut flow = ScriptFlow::Continue;
    for tick in 1..=8 {
        flow = with_tick_output(&snapshot, &mut ledger, tick, &mut output, |t| {
            script.tick(t).unwrap()
        });
        if matches!(flow, ScriptFlow::Complete) {
            break;
        }
    }
    assert!(
        matches!(flow, ScriptFlow::Complete),
        "colour Complete must end the sheep Path, got {flow:?}"
    );
    assert_eq!(script.stage().unwrap().0.as_ref(), "sheep:2");
    assert_eq!(script.progress().unwrap().complete, Truth::True);
    let last = output.0.last().expect("terminal status");
    assert_eq!(
        last.phase,
        NativePhase::Complete,
        "the terminal tick must publish Complete, not Working; status={last:?}"
    );

    let (script, snapshot) = sheep_complete_snapshot();
    let mut slot = crate::SlotScript::new();
    slot.bind_incarnation(1);
    slot.start_test_script(Box::new(script), None).unwrap();
    for tick in 1..=8 {
        slot.on_game_tick(&mut crate::ScriptCtx {
            driver: &mut crate::ctx::test_support::NullDriver::default(),
            tick,
            here: None,
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(&snapshot),
            obj_names: None,
            compiled: crate::CompiledTick::default(),
        });
        if slot.state() == crate::RunState::Idle {
            break;
        }
    }
    assert_eq!(slot.state(), crate::RunState::Idle);
    assert!(
        slot.native_status().is_none(),
        "teardown drops compiled status; observers must use the lifecycle receipt"
    );
    assert_eq!(
        slot.lifecycle_receipt().expect("completed receipt").state,
        crate::ScriptTerminalState::Completed
    );
}
struct NestedBankFixture {
    script: Quester,
    snapshot: GameSnapshot,
    ledger: Ledger,
    /// The account's bank memory, filled the way the host fills it.
    bank: api::bank_memory::BankMemory,
    bank_fixture: api::named_banks::NamedBank,
    bank_tile: api::snapshot::WorldTile,
    egg_id: i32,
    selected_bank: bool,
    opened_bank: bool,
    visits: usize,
}

impl NestedBankFixture {
    fn drive(&mut self, tick: u64) -> ScriptFlow {
        let mut output = StatusCapture::default();
        self.drive_with_output(tick, &mut output)
    }

    fn drive_with_output(&mut self, tick: u64, output: &mut dyn NativeOutput) -> ScriptFlow {
        // The host's per-frame observe (design-bank-snapshot §1.3).
        self.bank.track(&self.snapshot, tick);
        let flow = with_tick_output_bank(
            &self.snapshot,
            Some(&self.bank),
            &mut self.ledger,
            tick,
            output,
            |t| self.script.tick(t).unwrap(),
        );
        let bank_pick_pending = self.ledger.as_ref().is_some_and(|ledger| {
            ledger
                .outbox
                .first()
                .is_some_and(|action| matches!(&action.effect, HostEffect::BankPick(_)))
        });
        let open_stand_pending = self.ledger.as_ref().is_some_and(|ledger| {
            ledger.outbox.first().is_some_and(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(crate::shim::InteractReq::OpenStand { .. })
                )
            })
        });
        if bank_pick_pending {
            let action = self.ledger.as_mut().unwrap().outbox.remove(0);
            let authority = action.authority();
            self.ledger.as_mut().unwrap().complete_bank_pick(
                &authority,
                crate::bank::BankPickReceipt {
                    request_id: authority.request_id().get(),
                    evidence: api::quest_progress::EvidenceStamp {
                        run: authority.run(),
                        tick,
                        sequence: tick,
                    },
                    selected: crate::bank::SelectedBank {
                        bank_index: 0,
                        access_tile: self.bank_tile,
                        kind: crate::bank::PickKind::Reachable,
                        access: Some(Arc::new(crate::bank::BankStandAccess {
                            bank: self.bank_fixture,
                            stand_tile: self.bank_tile,
                            kind: crate::bank::AccessKind::Booth,
                            stand_op: 1,
                            name: None,
                            choose: None,
                        })),
                    },
                },
            );
            self.selected_bank = true;
        } else if open_stand_pending {
            assert!(matches!(
                ack(&mut self.ledger, tick),
                HostEffect::Interaction(crate::shim::InteractReq::OpenStand { .. })
            ));
            self.snapshot
                .seed_bank_observation(1, tick, Some(vec![]), vec![]);
            self.opened_bank = true;
            self.visits += 1;
        } else if self
            .ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            panic!("unexpected nested bank effect");
        }
        flow
    }
}

fn nested_bank_fixture(owns_inventory: bool) -> NestedBankFixture {
    use crate::quester::families::tests::{def, local_player};
    use crate::quester::path::StepDocument;
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView, LocLayer, LocView, WorldTile};

    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let mut document = super::super::compile::decode_cook().unwrap();
    let bank_has_egg = |qty| PredicateDocument::Fact {
        kind: "bank_has".into(),
        version: 1,
        args: serde_json::json!({"obj": "egg", "qty": qty}),
    };
    let bank_known = PredicateDocument::Fact {
        kind: "bank_known".into(),
        version: 1,
        args: serde_json::json!({}),
    };
    let bank_empty = PredicateDocument::All(vec![
        bank_known.clone(),
        PredicateDocument::Not(Box::new(bank_has_egg(1))),
    ]);

    let header = document.quest.as_mut().unwrap();
    header.owns_inventory = owns_inventory;
    header.acquire.insert(
        "acquire:egg".into(),
        vec![StepDocument {
            id: FactKey::new("nested-bank-acquire"),
            kind: "acquire".into(),
            version: 1,
            args: serde_json::json!({"recipe": "acquire:egg-bank-scan"}),
            comment: None,
            advances: Some(false),
            skip_if: PredicateDocument::Any(vec![]),
            settle: bank_empty.clone(),
        }],
    );
    header.acquire.insert(
        "acquire:egg-bank-scan".into(),
        vec![
            StepDocument {
                id: FactKey::new("real-empty-bank-scan"),
                kind: "bank".into(),
                version: 1,
                args: serde_json::json!({
                    "op": "scan",
                    "at": "nearest",
                    "items": [],
                    "keep": [],
                    "keep_ids": [],
                    "partial_ok": false
                }),
                comment: None,
                advances: Some(false),
                skip_if: PredicateDocument::Any(vec![]),
                settle: bank_known,
            },
            StepDocument {
                id: FactKey::new("skip-until-empty-bank-is-known"),
                kind: "wait".into(),
                version: 1,
                args: serde_json::json!({
                    "until": {
                        "Fact": {
                            "kind": "bank_has",
                            "version": 1,
                            "args": {"obj": "egg", "qty": 1}
                        }
                    },
                    "max_ticks": 4
                }),
                comment: None,
                advances: Some(false),
                skip_if: bank_empty.clone(),
                settle: bank_empty.clone(),
            },
        ],
    );
    let sequence = &mut document.roles[0].sequences[1];
    sequence.steps.truncate(1);
    sequence.terminal = true;
    sequence.steps[0].skip_if = bank_empty.clone();
    sequence.steps[0].settle = bank_empty;

    let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests)
        .expect("nested cook bank recipes compile");
    let bank_tile = WorldTile {
        x: 3092,
        z: 3242,
        level: 0,
    };
    let bank = api::named_banks::NamedBank::new("Nested receipt bank", bank_tile);
    let banks = Arc::new(api::named_banks::NamedBankFacts::from_banks(vec![bank]));
    let egg_id = data.item_by_alias("egg").unwrap().id;
    let script = Quester::new(
        RunKey {
            slot: 1,
            run: 1,
            session: 1,
        },
        path,
        Arc::clone(&data),
        quests,
        banks,
    );
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_local_player(local_player(bank_tile));
    // Flour leads the bundled acquisition order, so hold it and the milk:
    // these tests exercise the egg leg that follows.
    snapshot.seed_inventory(
        ["pot_flour", "bucket_milk"]
            .into_iter()
            .enumerate()
            .map(|(slot, alias)| ItemView {
                def: def(data.item_by_alias(alias).unwrap().id, alias),
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: slot as i32,
                count: 1,
                actions: Vec::new(),
                component_id: 0,
            })
            .collect(),
        28,
    );
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: "Cook's Assistant".into(),
            component_id: 42,
            colour: 0xf8f800,
        }],
        true,
    );
    snapshot.seed_locs(vec![LocView {
        id: 2213,
        name: Some("Bank booth".into()),
        actions: vec![Some("Use-quickly".into())],
        tile: bank_tile,
        distance: 0,
        typecode: 0,
        info: 0,
        description: None,
        layer: LocLayer::GroundDecoration,
        shape: 0,
        angle: 0,
        width: 1,
        length: 1,
        footprint_width: 1,
        footprint_length: 1,
        block_walk: false,
        block_range: false,
        active: true,
        animation: -1,
        map_function: -1,
        map_scene: -1,
        force_approach: 0,
    }]);
    NestedBankFixture {
        script,
        snapshot,
        ledger: None,
        bank: api::bank_memory::BankMemory::default(),
        bank_fixture: bank,
        bank_tile,
        egg_id,
        selected_bank: false,
        opened_bank: false,
        visits: 0,
    }
}

#[test]
fn nested_acquire_carries_empty_bank_receipt_to_dependent_and_outer_settle() {
    let _isolated = crate::IsolatedEnv::enter("quester-nested-bank-receipt");
    let mut fixture = nested_bank_fixture(true);
    let mut flow = ScriptFlow::Continue;
    for tick in 1..=48 {
        flow = fixture.drive(tick);
        if matches!(flow, ScriptFlow::Complete) {
            break;
        }
    }

    assert!(
        fixture.selected_bank,
        "the real scan must request bank selection"
    );
    assert!(
        fixture.opened_bank,
        "the selected stand must open before the real scan"
    );
    assert_eq!(
        flow,
        ScriptFlow::Complete,
        "the nested outer acquire must settle instead of timing out on unknown bank evidence"
    );
    assert!(fixture.bank.known());
    assert_eq!(fixture.bank.origin(), api::bank_memory::Origin::Session);
    assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
    assert!(!fixture.script.parked);
    assert!(fixture
        .script
        .last_error
        .as_ref()
        .is_none_or(|error| !error.contains("step settle timeout")));

    let dependent =
        &fixture.script.path.provisioning.recipes["acquire:egg-bank-scan"].steps[1].skip_if;
    let outer_settle = &fixture.script.path.sequences[1].steps[0].settle;
    let (dependent_truth, settle_truth) = with_tick_bank(
        &fixture.snapshot,
        Some(&fixture.bank),
        &mut fixture.ledger,
        49,
        |tick| {
            let cx = PredicateContext {
                cx: &tick.cx,
                pairs: tick.pairs,
                quests: &fixture.script.quests,
                progress: &[],
                required_after: tick.cx.evidence(),
                chat_since: 0,
                outcome: None,
            };
            (dependent.evaluate(&cx), outer_settle.evaluate(&cx))
        },
    );
    assert_eq!(dependent_truth, Truth::True);
    assert_eq!(settle_truth, Truth::True);
}

#[test]
fn provisioner_acquire_run_keeps_session_bank_memory_without_second_trip() {
    let _isolated = crate::IsolatedEnv::enter("quester-provision-bank-receipt");
    let mut fixture = nested_bank_fixture(false);
    let mut acquiring_seen = false;
    for tick in 1..=64 {
        fixture.drive(tick);
        if !acquiring_seen
            && fixture.script.provisioner.status().phase
                == super::super::provision::ProvisionPhase::Acquiring
        {
            assert!(
                fixture.bank.known(),
                "the real provisioning scan must first establish Session bank memory"
            );
            assert_eq!(
                fixture.bank.origin(),
                api::bank_memory::Origin::Session,
                "the fixture bank is really opened, not hinted"
            );
            assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
            acquiring_seen = true;
            continue;
        }

        if acquiring_seen {
            // Receipts carry no bank rows now; the acquisition run must leave
            // the Session memory exactly as the scan observed it.
            assert!(
                fixture.bank.known(),
                "the acquisition run must not erase Session bank memory"
            );
            assert_eq!(
                fixture.bank.origin(),
                api::bank_memory::Origin::Session,
                "Acquired must not touch the memory origin"
            );
            assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
        }
    }

    assert!(
        acquiring_seen,
        "the nested AcquireRun must start from the scanned bank"
    );
    assert!(
        fixture.selected_bank,
        "provisioning must select the real bank"
    );
    assert!(
        fixture.opened_bank,
        "provisioning must open the selected bank"
    );
    assert_eq!(
        fixture.visits, 1,
        "the known bank must not trigger a second scan trip"
    );
    assert!(
        fixture.bank.known(),
        "completed acquisition must retain the last observed bank stock; only inventory changed"
    );
    assert_eq!(fixture.bank.count(fixture.egg_id), Some(0));
    assert!(!fixture.script.parked);
    assert!(fixture
        .script
        .last_error
        .as_ref()
        .is_none_or(|error| !error.contains("settle timeout")));
}

#[derive(Clone, Copy, Debug)]
enum ProgressOutcomeStamp {
    Fresh,
    PreviousPoll,
    StepBegin,
    BeforeStep,
    Uncorrelated,
    Future,
    ForeignRun,
}

struct ProgressOutcomePlan {
    progress: QuestProgress,
    stamp: ProgressOutcomeStamp,
}

impl super::super::compile::StepPlan for ProgressOutcomePlan {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
        let mut progress = self.progress.clone();
        progress.evidence = cx.required_after;
        Ok(Box::new(ProgressOutcomeRun {
            progress,
            stamp: self.stamp,
            first_poll: true,
            previous_poll: None,
        }))
    }
}

struct ProgressOutcomeRun {
    progress: QuestProgress,
    stamp: ProgressOutcomeStamp,
    previous_poll: Option<api::quest_progress::EvidenceStamp>,
    first_poll: bool,
}

impl StepRun for ProgressOutcomeRun {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>> {
        let current = cx.tick.cx.evidence();
        if std::mem::take(&mut self.first_poll) {
            return Poll::Pending;
        }
        let begin = self.progress.evidence;
        self.progress.evidence = match self.stamp {
            ProgressOutcomeStamp::StepBegin => begin,
            ProgressOutcomeStamp::BeforeStep => api::quest_progress::EvidenceStamp {
                tick: begin.tick - 1,
                sequence: begin.sequence - 1,
                ..begin
            },
            ProgressOutcomeStamp::Future => api::quest_progress::EvidenceStamp {
                tick: current.tick + 1,
                sequence: current.sequence + 1,
                ..current
            },
            ProgressOutcomeStamp::ForeignRun => api::quest_progress::EvidenceStamp {
                run: RunKey {
                    session: current.run.session + 1,
                    ..current.run
                },
                ..current
            },
            ProgressOutcomeStamp::PreviousPoll => {
                let Some(previous) = self.previous_poll.replace(current) else {
                    return Poll::Pending;
                };
                previous
            }
            ProgressOutcomeStamp::Fresh | ProgressOutcomeStamp::Uncorrelated => current,
        };
        Poll::Ready(Ok(StepOutcome {
            evidence: if matches!(self.stamp, ProgressOutcomeStamp::Uncorrelated) {
                begin
            } else {
                self.progress.evidence
            },
            progress: Some(Arc::new(self.progress.clone())),
            receipt: None,
        }))
    }

    fn cancel(&mut self, _: &mut NativeActions) {}
}

fn step_progress_fixture(stamp: ProgressOutcomeStamp) -> (Quester, GameSnapshot, Ledger) {
    let (mut script, snapshot) = fixture(false);
    let mut ledger = None;
    let progress = with_tick(&snapshot, &mut ledger, 0, |tick| {
        resolve_colour(
            &script.path,
            QuestListStatus::Complete,
            tick.cx.evidence(),
            Arc::new(tick.cx.pin().clone()),
        )
    });
    Arc::get_mut(&mut script.path).unwrap().sequences[1].steps[0].plan =
        Arc::new(ProgressOutcomePlan { progress, stamp });
    // Begin on an admissible tick. The fixture's first poll stays pending,
    // so each progress stamp is tested against a later outcome poll.
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(
        script.step.is_some(),
        "the runner must begin the authored step"
    );
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    (script, snapshot, ledger)
}

#[test]
fn step_outcome_progress_accepts_fresh_same_stamp_and_settles_to_completion() {
    for stamp in [
        ProgressOutcomeStamp::Fresh,
        ProgressOutcomeStamp::PreviousPoll,
    ] {
        let (mut script, mut snapshot, mut ledger) = step_progress_fixture(stamp);
        let poll_tick = if matches!(stamp, ProgressOutcomeStamp::PreviousPoll) {
            drive(&mut script, &snapshot, &mut ledger, 2);
            assert!(script.step.is_some() && !script.settling);
            3
        } else {
            2
        };
        // Spent, so the completion stays in its settle window this tick
        // instead of settling and handing over at once (TICK-FIX #5).
        drive_spent(&mut script, &snapshot, &mut ledger, poll_tick);
        assert!(
            script.settling,
            "fresh progress correlated with its final outcome must settle: {stamp:?}, {:?}",
            script.last_error
        );
        let outcome = script.last_outcome.as_ref().expect("accepted outcome");
        assert_eq!(outcome.evidence, script.progress().unwrap().evidence);
        assert_eq!(outcome.evidence.tick, 2);
        assert_eq!(script.stage().unwrap().0.as_ref(), "cook:2");
        assert_eq!(script.progress().unwrap().complete, Truth::True);
        assert!(script.last_error.is_none());
        snapshot.seed_quest_statuses(
            vec![QuestStatusView {
                name: "Cook's Assistant".into(),
                component_id: 42,
                colour: 0x00f800,
            }],
            true,
        );
        drive(&mut script, &snapshot, &mut ledger, poll_tick + 1);
        assert!(!script.settling);
        assert!(matches!(
            drive(&mut script, &snapshot, &mut ledger, poll_tick + 2),
            ScriptFlow::Complete
        ));
    }
}

#[test]
fn step_outcome_progress_rejects_stale_uncorrelated_future_and_foreign_receipts() {
    for stamp in [
        ProgressOutcomeStamp::StepBegin,
        ProgressOutcomeStamp::BeforeStep,
        ProgressOutcomeStamp::Uncorrelated,
        ProgressOutcomeStamp::Future,
        ProgressOutcomeStamp::ForeignRun,
    ] {
        let (mut script, snapshot, mut ledger) = step_progress_fixture(stamp);
        let prior = script.progress().unwrap().evidence;
        drive(&mut script, &snapshot, &mut ledger, 2);
        assert!(
            !script.settling,
            "invalid progress must not settle: {stamp:?}"
        );
        assert!(script.last_outcome.is_none());
        assert_eq!(script.progress().unwrap().evidence, prior);
        assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
        assert_eq!(script.last_error.as_deref(), Some("step error: Stale"));
    }
}

#[test]
fn custom_progress_requires_fresh_correlated_declared_owned_evidence() {
    let (script, snapshot) = fixture(false);
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 5, |tick| {
        let after = api::quest_progress::EvidenceStamp {
            run: tick.cx.run(),
            tick: 4,
            sequence: 4,
        };
        let progress = resolve_colour(
            &script.path,
            QuestListStatus::NotStarted,
            tick.cx.evidence(),
            Arc::new(tick.cx.pin().clone()),
        );
        assert!(script.valid_progress(tick, &progress, after));
        let mut bad = progress.clone();
        bad.evidence = after;
        assert!(
            !script.valid_progress(tick, &bad, after),
            "same-frame cached progress is not a new owned read"
        );
        bad.evidence = api::quest_progress::EvidenceStamp {
            tick: 6,
            sequence: 6,
            ..after
        };
        assert!(
            !script.valid_progress(tick, &bad, after),
            "future receipts cannot be consumed"
        );
        bad = progress.clone();
        bad.evidence.run.session += 1;
        assert!(!script.valid_progress(tick, &bad, after), "foreign session");
        bad = progress.clone();
        bad.binding = FactKey::new("card:other");
        assert!(!script.valid_progress(tick, &bad, after), "foreign binding");
        bad = progress.clone();
        bad.role = Some(FactKey::new("other"));
        assert!(!script.valid_progress(tick, &bad, after), "foreign role");
        bad = progress.clone();
        bad.stage = Knowledge::Known(FactKey::new("unbound"));
        assert!(
            !script.valid_progress(tick, &bad, after),
            "undeclared stage"
        );
        bad = progress.clone();
        bad.complete = Truth::True;
        assert!(
            !script.valid_progress(tick, &bad, after),
            "completion must name the terminal stage"
        );
        bad = progress.clone();
        bad.flags = Arc::from([api::quest_progress::ProgressFlag {
            flag: FactKey::new("undeclared"),
            truth: Truth::True,
            count: None,
        }]);
        assert!(!script.valid_progress(tick, &bad, after), "undeclared flag");
    });
}

#[test]
fn ordinary_recovery_keeps_completed_admission_but_pause_and_new_run_do_not() {
    let (mut script, snapshot) = fixture(false);
    script.pair_admitted = true;
    let mut ledger = None;
    with_tick(&snapshot, &mut ledger, 5, |tick| script.cancel_step(tick));
    assert!(
        script.pair_admitted,
        "ordinary death/prayer cleanup does not invent a second peer barrier"
    );
    script.interrupt(Interrupt::Hold(true));
    assert!(script.pair_admitted);
    script.interrupt(Interrupt::Hold(false));
    assert!(script.pair_admitted);
    script.interrupt(Interrupt::Pause);
    assert!(!script.pair_admitted);
    script.pair_admitted = true;
    script.run.session += 1;
    drive(&mut script, &snapshot, &mut ledger, 6);
    assert!(
        !script.pair_admitted,
        "a different run/session must acquire a fresh reciprocal admission"
    );
}

/// The death `scout-secret-way` page: a zone trigger's player chat (`chat3`,
/// 289 root 979) with a continue and no owner once the walk has settled.
fn seed_player_chat(snapshot: &mut GameSnapshot, line: &str) {
    snapshot.seed_chat_modal(979, vec!["Ci7 0".into(), line.into()]);
    snapshot.seed_chat_options(vec![], 981);
}

fn close_chat(snapshot: &mut GameSnapshot) {
    snapshot.seed_chat_modal(-1, vec![]);
    snapshot.seed_chat_options(vec![], -1);
}

fn is_continue(effect: &HostEffect) -> bool {
    matches!(
        effect,
        HostEffect::Interaction(crate::shim::InteractReq::ContinueDialog { .. })
    )
}

#[test]
fn unowned_chat_continue_is_drained_then_the_journal_is_read() {
    let (mut script, mut snapshot) = fixture(true);
    seed_player_chat(&mut snapshot, "I think this is far enough.");
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(
        is_continue(&ack(&mut ledger, 1)),
        "no step owns the page, so the read clicks its continue instead of waiting"
    );
    close_chat(&mut snapshot);
    let mut tick = 2;
    while ledger.as_ref().unwrap().outbox.is_empty() {
        assert!(tick < 20 && !script.parked, "the read must resume");
        drive(&mut script, &snapshot, &mut ledger, tick);
        tick += 1;
    }
    let tick = tick - 1;
    assert!(matches!(
        ack(&mut ledger, tick),
        HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
    ));
    journal(&mut snapshot, "seeded branch");
    drive(&mut script, &snapshot, &mut ledger, tick + 1);
    drive(&mut script, &snapshot, &mut ledger, tick + 2);
    assert!(matches!(
        ack(&mut ledger, tick + 2),
        HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
    ));
    snapshot.seed_main_modal(-1, vec![]);
    drive(&mut script, &snapshot, &mut ledger, tick + 3);
    assert_eq!(script.stage().unwrap().0.as_ref(), "cook:1");
    assert!(!script.parked);
    assert_eq!(script.journal_drains, 0, "adoption resets the drain budget");
}

#[test]
fn a_live_step_keeps_its_latched_continue_busy_and_unclicked() {
    let (mut script, mut snapshot) = fixture(true);
    script.step = Some(Box::new(OwnedActionStep { handle: None }));
    seed_player_chat(&mut snapshot, "A page the step's dialogue owns.");
    let mut ledger = None;
    for tick in 1..=8 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        assert!(
            ledger.as_ref().unwrap().outbox.is_empty(),
            "tick {tick}: the owner advances its page; the read neither continues nor clicks"
        );
    }
    assert!(script.journal_drain.is_none());
    assert!(!script.parked);
}

#[test]
fn a_continue_page_that_ignores_clicks_parks_after_the_drain_cap() {
    let (mut script, mut snapshot) = fixture(true);
    seed_player_chat(&mut snapshot, "A page that will not advance.");
    let mut ledger = None;
    let mut continues = 0;
    for tick in 1..120 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            assert!(is_continue(&ack(&mut ledger, tick)));
            continues += 1;
        }
        if script.parked {
            break;
        }
    }
    assert!(script.parked);
    assert_eq!(
        continues, 3,
        "one continue per drain, three drains per read"
    );
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        "journal blocked by modal root 979 (Ci7 0): the chat continue reopened after 3 drains"
    );
}

#[test]
fn a_continue_page_that_keeps_reopening_parks_within_the_drain_window() {
    let (mut script, mut snapshot) = fixture(true);
    seed_player_chat(&mut snapshot, "page 0");
    let mut ledger = None;
    let mut continues = 0;
    for tick in 1..200 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        if ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            assert!(is_continue(&ack(&mut ledger, tick)));
            continues += 1;
            seed_player_chat(&mut snapshot, &format!("page {continues}"));
        }
        if script.parked {
            break;
        }
    }
    assert!(script.parked);
    assert!(
        (2..=50).contains(&continues),
        "bounded by the 30 s window (one Continue per observed page, so at most \
         one per tick), not the 120-page driver cap: {continues}"
    );
    let message = script.blocked_failure().message;
    assert!(
        message.starts_with("journal blocked by modal root 979 (Ci7 0)")
            && message.ends_with("the chat continue kept reopening for 30 s"),
        "{message}"
    );
}

/// REVIEW-QUESTER-CI7-FIXES P2: the drain owns no conversation, so it clicks
/// Chat continue only. When its accepted continue opens a selected scroll on
/// Main, the drain ends and leaves the document to its owner: no `CloseModal`
/// and no `IfButton` while the document is open.
#[test]
fn the_drain_leaves_a_scroll_its_continue_opened() {
    let ids = *api::game_data::for_revision(api::selected::ClientRevision::R289)
        .unwrap()
        .dialogue_ui()
        .unwrap();
    let (mut script, mut snapshot) = fixture(true);
    seed_player_chat(&mut snapshot, "A page whose continue opens a scroll.");
    let mut ledger = None;
    drive(&mut script, &snapshot, &mut ledger, 1);
    assert!(is_continue(&ack(&mut ledger, 1)));
    close_chat(&mut snapshot);
    snapshot.seed_main_modal(ids.scroll_root, vec![]);
    for tick in 2..60 {
        drive(&mut script, &snapshot, &mut ledger, tick);
        while ledger
            .as_ref()
            .is_some_and(|ledger| !ledger.outbox.is_empty())
        {
            let effect = ack(&mut ledger, tick);
            assert!(
                !matches!(
                    effect,
                    HostEffect::Interaction(
                        crate::shim::InteractReq::CloseModal
                            | crate::shim::InteractReq::IfButton { .. }
                    )
                ),
                "tick {tick}: the drain closed or clicked the scroll"
            );
        }
        if script.parked {
            break;
        }
    }
    assert!(script.journal_drain.is_none(), "the drain ended");
}

/// The 289 `multi3` choice root (live receipt 089 `run.log:1379`: "journal
/// blocked by modal root 2469 (Select an Option)").
const MULTI3_ROOT: i32 = 2469;
/// The Jolly Boar bartender's `p_choice3` (`area_varrock/scripts/bartender.rs2:7`).
const BEER_MENU: [&str; 3] = [
    "I'll have a beer please.",
    "Any hints where I can go adventuring?",
    "Heard any good gossip?",
];

fn seed_items(snapshot: &mut GameSnapshot, items: &[(&str, i32)]) {
    use crate::quester::families::tests::def;
    use api::snapshot::{ItemActionFamily, ItemContainer, ItemView};
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let rows = items
        .iter()
        .filter(|(_, count)| *count > 0)
        .enumerate()
        .map(|(slot, (alias, count))| ItemView {
            def: def(data.item_by_alias(alias).unwrap().id, alias),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: slot as i32,
            count: *count,
            actions: Vec::new(),
            component_id: 0,
        })
        .collect();
    snapshot.seed_inventory(rows, 28);
}

fn seed_coins_and_beer(snapshot: &mut GameSnapshot, coins: i32, beer: i32) {
    seed_items(snapshot, &[("coins", coins), ("beer", beer)]);
}

/// The 289 `multi2` and `multi4` choice roots (`p_choice2`/`p_choice4`,
/// `interface_chat/scripts/chat.rs2:1-16`), as in the review's regressions.
const MULTI2_ROOT: i32 = 2459;
const MULTI4_ROOT: i32 = 2480;
/// A `~chatnpc` page root and a `~chatplayer` page root; each continue
/// component is two after its root.
const NPC_PAGE_ROOT: i32 = 4882;
const PLAYER_PAGE_ROOT: i32 = 968;
/// The speaking NPC's index in every scene.
const NPC_INDEX: usize = 304;

/// A restart scene on one shipped Path: the quest row in progress, 200 coins,
/// a known empty bank, the local player at `here`, and `npc` one tile north,
/// facing the player while `facing` (`~chatnpc`'s `playerfaceclose`,
/// `interface_chat/scripts/chat.rs2:323-331`).
struct Scene {
    path: Arc<super::super::compile::CompiledPath>,
    data: Arc<api::game_data::SelectedGameData>,
    quests: Arc<QuestCatalog>,
    snapshot: GameSnapshot,
    bank: api::bank_memory::BankMemory,
    npc: &'static str,
    here: api::snapshot::WorldTile,
}

fn scene(
    json: &str,
    quest: &str,
    here: api::snapshot::WorldTile,
    npc: &'static str,
    facing: bool,
) -> Scene {
    use crate::quester::families::tests::local_player;
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let quests = Arc::new(QuestCatalog::from_identity(data.quest_identity()).unwrap());
    let document: crate::quester::path::PathDocument = serde_json::from_str(json).unwrap();
    let path = super::super::compile::compile_uncached_for_test(&document, &data, &quests).unwrap();
    let mut snapshot = GameSnapshot::new();
    snapshot.seed_ingame(2);
    snapshot.seed_tile(here);
    snapshot.seed_local_player(local_player(here));
    snapshot.seed_quest_statuses(
        vec![QuestStatusView {
            name: quests.quest(quest).unwrap().display.to_string(),
            component_id: 43,
            colour: 0xf8f800,
        }],
        true,
    );
    seed_coins_and_beer(&mut snapshot, 200, 0);
    close_chat(&mut snapshot);
    let mut scene = Scene {
        path,
        data,
        quests,
        snapshot,
        bank: api::bank_memory::BankMemory::seeded(&[], api::bank_memory::Origin::Session),
        npc,
        here,
    };
    scene.facing(facing);
    scene
}

impl Scene {
    fn npc_type(&self) -> i32 {
        self.data.npc_by_config(self.npc).unwrap().id
    }

    /// The NPC stays one tile north; `facing` is its face entity on the player.
    fn facing(&mut self, facing: bool) {
        use api::snapshot::{ActorKind, ActorTargetView, NpcView, WorldTile};
        let tile = WorldTile {
            z: self.here.z + 1,
            ..self.here
        };
        let npc_type = self.npc_type();
        self.snapshot.seed_npcs(vec![NpcView {
            index: NPC_INDEX,
            r#type: Some(npc_type as usize),
            name: Some("Speaker".into()),
            actions: vec![Some("Talk-to".into())],
            tile,
            distance: 1,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: if facing { 32768 } else { -1 },
            target: facing.then_some(ActorTargetView {
                kind: ActorKind::Player,
                index: 0,
            }),
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: 0,
            z: 0,
            yaw: 0,
        }]);
    }

    /// `options` open as a `p_choiceN` menu on `root` (`chat.rs2:1-16`).
    fn menu(&mut self, root: i32, options: &[&str]) {
        use api::snapshot::ChatOptionView;
        self.snapshot.seed_chat_modal(
            root,
            std::iter::once("Select an Option")
                .chain(options.iter().copied())
                .map(String::from)
                .collect(),
        );
        self.snapshot.seed_chat_options(
            options
                .iter()
                .zip(root + 1..)
                .map(|(text, component_id)| ChatOptionView {
                    component_id,
                    text: (*text).into(),
                })
                .collect(),
            -1,
        );
    }

    /// A continue page on `root`.
    fn page(&mut self, root: i32, texts: &[&str]) {
        self.snapshot
            .seed_chat_modal(root, texts.iter().copied().map(String::from).collect());
        self.snapshot.seed_chat_options(vec![], root + 2);
    }

    /// A runner as a Start creates it: no progress, nothing begun.
    fn runner(&self) -> Quester {
        Quester::new(
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            Arc::clone(&self.path),
            Arc::clone(&self.data),
            Arc::clone(&self.quests),
            Arc::new(api::named_banks::NamedBankFacts::empty()),
        )
    }

    /// One runner tick as the slot runs it: the carried conversation lent to
    /// the retained cell, the tick, then taken back (`poll_compiled_frame`).
    fn drive(&self, script: &mut Quester, memory: &mut SlotMemory, ledger: &mut Ledger, tick: u64) {
        memory.carried.lend(memory.cell.quester());
        with_tick_retained(
            &self.snapshot,
            Some(&self.bank),
            &mut memory.cell,
            ledger,
            tick,
            |t| {
                script.tick(t).unwrap();
            },
        );
        memory.carried.reclaim(memory.cell.quester());
    }

    /// One host frame: the slot's conversation watch, then the runner tick
    /// (`script_observe.rs`: `watch_held_conversation` before dispatch).
    fn frame(&self, script: &mut Quester, memory: &mut SlotMemory, ledger: &mut Ledger, tick: u64) {
        watch(memory, self, tick);
        self.drive(script, memory, ledger, tick);
    }
}

/// What the slot keeps for a Quester: its retained cell and the conversation
/// an operator Stop carried (`SlotScript::retained`,
/// `SlotScript::carried_conversation`).
#[derive(Default)]
struct SlotMemory {
    cell: RetainedMemory,
    carried: CarriedConversation,
}

impl SlotMemory {
    /// The conversation record, carried or live.
    fn held(&mut self) -> Option<&mut HeldConversation> {
        match self.carried.get_mut() {
            Some(held) => Some(held),
            None => self.cell.quester().conversation.as_deref_mut(),
        }
    }

    /// A logout, relog or world hop (`SlotScript::session_boundary`).
    fn relog(&mut self) {
        self.carried.clear();
        self.cell.quester().conversation = None;
    }
}

/// The slot's per-frame check of a carried conversation
/// (`SlotScript::watch_held_conversation`); `false` once it is gone.
fn watch(memory: &mut SlotMemory, scene: &Scene, tick: u64) -> bool {
    memory.carried.watch(Some(&scene.snapshot), tick);
    memory.carried.get_mut().is_some()
}

/// The slot's operator Stop: the conversation the live step recorded moves
/// to the slot, and the retained cell is discarded (`SlotScript::stop_with_reason`).
fn operator_stop(memory: &mut SlotMemory) {
    memory.carried.stop(Some(memory.cell.quester()));
    memory.cell = RetainedMemory::default();
}

fn answer_of(effect: &HostEffect) -> Option<i32> {
    match effect {
        HostEffect::Interaction(crate::shim::InteractReq::Answer { option }) => Some(*option),
        _ => None,
    }
}

/// A live run of `stage`'s `step` with the scene's NPC adjacent: its Talk-to,
/// the NPC's first page `page`, the driver's Continue on it and that page
/// latched, then an operator Stop. Returns the slot's retained memory after
/// the Stop, which carries the step's own conversation.
fn stop_mid_conversation(scene: &mut Scene, stage: &str, step: &str, page: &[&str]) -> SlotMemory {
    let mut memory = SlotMemory::default();
    let mut ledger = None;
    let mut live = scene.runner();
    let sequence = scene
        .path
        .sequences
        .iter()
        .position(|sequence| sequence.stage.0.as_ref() == stage)
        .unwrap();
    let index = scene.path.sequences[sequence]
        .steps
        .iter()
        .position(|candidate| candidate.id.0.as_ref() == step)
        .unwrap();
    live.in_prelude = false;
    live.seq_index = sequence;
    live.step_index = index;
    live.needs_read = false;
    let plan = Arc::clone(&scene.path.sequences[sequence].steps[index].plan);
    let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
    let choices = super::super::choices::QuestChoices::default();
    live.step = Some(with_tick_retained(
        &scene.snapshot,
        Some(&scene.bank),
        &mut memory.cell,
        &mut ledger,
        1,
        |t| {
            let required_after = t.cx.evidence();
            plan.begin(&mut StepContext {
                tick: t,
                quests: &scene.quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &choices,
            })
            .unwrap()
        },
    ));
    let mut tick = 2;
    while outbox_empty(&ledger) {
        assert!(
            tick < 30 && !live.parked,
            "{step}: the live step talks to its NPC"
        );
        scene.frame(&mut live, &mut memory, &mut ledger, tick);
        tick += 1;
    }
    assert!(
        matches!(
            ack(&mut ledger, tick - 1),
            HostEffect::Interaction(crate::shim::InteractReq::Npc { .. })
        ),
        "{step}: Talk-to first"
    );
    scene.page(NPC_PAGE_ROOT, page);
    scene.frame(&mut live, &mut memory, &mut ledger, tick);
    assert!(
        is_continue(&ack(&mut ledger, tick)),
        "{step}: the step continues its NPC's page"
    );
    // Latched until the content writes the next page.
    scene.snapshot.seed_chat_options(vec![], -1);
    scene.frame(&mut live, &mut memory, &mut ledger, tick + 1);
    assert!(outbox_empty(&ledger));
    let held = memory
        .cell
        .quester()
        .conversation
        .as_deref()
        .expect("the live step's conversation is recorded");
    assert!(!held.carried && held.menu.is_none());
    assert_eq!(held.npc_index, NPC_INDEX as i32);
    assert_eq!(held.page.root, NPC_PAGE_ROOT);
    live.on_stop(StopReason::Operator);
    operator_stop(&mut memory);
    memory
}

/// A runner restarted on `memory` must leave the open menu alone: no
/// action at all, and the read parks naming the page. Frames run without
/// the slot's watch, so the runner's own check is what refuses the menu.
fn assert_parks_without_clicking(scene: &Scene, memory: &mut SlotMemory, label: &str, page: &str) {
    let mut script = scene.runner();
    let mut ledger = None;
    for tick in 30..90 {
        scene.drive(&mut script, memory, &mut ledger, tick);
        assert!(
            outbox_empty(&ledger),
            "{label}: tick {tick}: a menu the Path cannot prove it owns is never answered or covered"
        );
        if script.parked {
            break;
        }
    }
    assert!(script.parked, "{label}");
    assert!(script.journal_drain.is_none(), "{label}");
    assert_eq!(
        script.blocked_failure().message.as_ref(),
        format!("journal blocked by modal root {page}"),
        "{label}"
    );
}

fn vampire_scene() -> Scene {
    scene(
        super::super::compile::VAMPIRE_JSON,
        "vampire",
        api::snapshot::WorldTile {
            x: 3277,
            z: 3487,
            level: 0,
        },
        "jollyboar_bartender",
        true,
    )
}

/// `stake` / `buy-harlow-beer` at `vampire:2`, stopped right after the step
/// continued the bartender's "Can I help you?" (`bartender.rs2:2`), with his
/// beer choice (`bartender.rs2:7`) on the frame after the Stop: the state of
/// live receipt 089 (`08-start-3.png`).
fn vampire_stopped_at_beer_menu() -> (Scene, SlotMemory) {
    let mut scene = vampire_scene();
    let mut memory = stop_mid_conversation(
        &mut scene,
        "vampire:2",
        "stake",
        &["Bartender", "Can I help you?"],
    );
    scene.menu(MULTI3_ROOT, &BEER_MENU);
    assert!(
        watch(&mut memory, &scene, 20),
        "the menu after the step's page"
    );
    (scene, memory)
}

fn vampire_journal(script: &Quester, snapshot: &mut GameSnapshot, body: &str) {
    let title = script
        .quests
        .quest("vampire")
        .unwrap()
        .journal_title
        .clone()
        .unwrap();
    snapshot.seed_main_modal(
        8134,
        vec![
            crate::quest_journal::test_widget(8144, &format!("@dre@{title}")),
            crate::quest_journal::test_widget(8145, body),
        ],
    );
}

fn outbox_empty(ledger: &Ledger) -> bool {
    ledger
        .as_ref()
        .is_none_or(|ledger| ledger.outbox.is_empty())
}

/// VAMPIRE-RESTART-0201: an operator Stop inside `buy-harlow-beer` leaves the
/// bartender's choice pending. The journal row's `if_openmain` would close it
/// and drop the suspended script (`quest_journal.rs2:55`, `Player.ts:1997-2020`),
/// so the restarted read first answers the stopped step's own "I'll have a
/// beer please." through the dialogue driver, follows the content's pages to
/// the end (`bartender.rs2:11-18`), and only then clicks the journal row.
#[test]
fn restart_inside_the_paths_own_choice_finishes_it_then_reads_the_journal() {
    let (mut scene, mut memory) = vampire_stopped_at_beer_menu();
    let mut script = scene.runner();
    let mut ledger = None;
    scene.frame(&mut script, &mut memory, &mut ledger, 21);
    assert_eq!(
        answer_of(&ack(&mut ledger, 21)),
        Some(1),
        "the restarted read answers the stopped step's beer choice before any journal click"
    );
    assert!(!script.parked);
    assert!(memory.held().is_none(), "the record answers once");
    // `bartender.rs2:11`: the player's line.
    scene.page(
        PLAYER_PAGE_ROOT,
        &["Player", "I'll have a pint of beer please."],
    );
    scene.frame(&mut script, &mut memory, &mut ledger, 22);
    assert!(is_continue(&ack(&mut ledger, 22)));
    // `bartender.rs2:12`: the price.
    scene.page(
        NPC_PAGE_ROOT,
        &["Bartender", "Ok, that'll be two coins please."],
    );
    scene.frame(&mut script, &mut memory, &mut ledger, 23);
    assert!(is_continue(&ack(&mut ledger, 23)));
    // `bartender.rs2:16-18`: two coins out, a beer in, and the script ends.
    close_chat(&mut scene.snapshot);
    seed_coins_and_beer(&mut scene.snapshot, 198, 1);
    let mut tick = 24;
    while outbox_empty(&ledger) {
        assert!(
            tick < 40 && !script.parked,
            "the read must resume once the chat closes"
        );
        scene.frame(&mut script, &mut memory, &mut ledger, tick);
        tick += 1;
    }
    let tick = tick - 1;
    assert!(
        matches!(
            ack(&mut ledger, tick),
            HostEffect::Interaction(crate::shim::InteractReq::IfButton { .. })
        ),
        "the journal row is clicked only after the conversation ended"
    );
    vampire_journal(
        &script,
        &mut scene.snapshot,
        "@str@I have spoken to Dr Harlow. He seemed terribly drunk, and",
    );
    scene.frame(&mut script, &mut memory, &mut ledger, tick + 1);
    scene.frame(&mut script, &mut memory, &mut ledger, tick + 2);
    assert!(matches!(
        ack(&mut ledger, tick + 2),
        HostEffect::Interaction(crate::shim::InteractReq::CloseModal)
    ));
    scene.snapshot.seed_main_modal(-1, vec![]);
    scene.frame(&mut script, &mut memory, &mut ledger, tick + 3);
    assert_eq!(script.stage().unwrap().0.as_ref(), "vampire:2");
    assert!(!script.parked);
}

/// Only the menu the stopped step's conversation reached, with its speaker
/// still facing the player, on the same Path, and singled out by that step's
/// text, is answered. Every other restart stays fail-closed: no action, and
/// the read parks naming the page. A fresh Start with the bartender facing
/// the player over his own menu is one of them: nothing is inferred from the
/// scene.
#[test]
fn restart_with_a_menu_the_path_does_not_own_parks_without_clicking() {
    let page = "2469 (Select an Option)";
    let mut scene = vampire_scene();
    scene.menu(MULTI3_ROOT, &BEER_MENU);
    assert_parks_without_clicking(
        &scene,
        &mut SlotMemory::default(),
        "a fresh Start with nothing recorded",
        page,
    );

    let (mut scene, mut memory) = vampire_stopped_at_beer_menu();
    scene.facing(false);
    assert_parks_without_clicking(
        &scene,
        &mut memory,
        "the step's NPC no longer facing the player",
        page,
    );

    let (scene, mut memory) = vampire_stopped_at_beer_menu();
    memory.held().unwrap().digest[0] ^= 1;
    assert_parks_without_clicking(&scene, &mut memory, "another Path compile", page);

    let (mut scene, mut memory) = vampire_stopped_at_beer_menu();
    scene.menu(
        MULTI3_ROOT,
        &[
            "Can you sell me a hat?",
            "Never mind.",
            "Heard any good gossip?",
        ],
    );
    assert_parks_without_clicking(
        &scene,
        &mut memory,
        "a menu other than the pinned one",
        page,
    );

    let mut scene = vampire_scene();
    let mut memory = stop_mid_conversation(
        &mut scene,
        "vampire:2",
        "stake",
        &["Bartender", "Can I help you?"],
    );
    scene.menu(
        MULTI3_ROOT,
        &[
            "I'll have a beer please.",
            "I'll have a beer please.",
            "Heard any good gossip?",
        ],
    );
    assert!(watch(&mut memory, &scene, 20));
    assert_parks_without_clicking(
        &scene,
        &mut memory,
        "the step's answer matching two options",
        page,
    );
}

/// A logout, relog or world hop ends the session's conversation
/// (`SlotScript::session_boundary` clears both the slot's carried record and
/// the cell's); the next Start then leaves the same menu alone.
#[test]
fn a_conversation_record_a_relog_cleared_never_answers() {
    let (scene, mut memory) = vampire_stopped_at_beer_menu();
    memory.relog();
    assert_parks_without_clicking(&scene, &mut memory, "relog", "2469 (Select an Option)");
}

/// Captain Tobias's `p_choice2` (`area_port_sarim/scripts/sailors.rs2:12-17`).
const SAIL_MENU: [&str; 2] = ["Yes please.", "No, thank you."];
/// The held Bervirius scroll's `p_choice2`, opened after a message with no NPC
/// speaker (`quests/quest_zombiequeen/scripts/quest_zombiequeen.rs2:862-865`).
const SCROLL_MENU: [&str; 2] = ["Yes please.", "No thanks."];

/// Port Sarim with Captain Tobias adjacent and still facing the player after
/// a conversation (`Npc.ts:896-905` keeps `playerfaceclose` within one tile),
/// 200 coins and the held Bervirius scroll. Pirate's Treasure's
/// `hunt-smuggle-rum` / `rum-sail-to-karamja` authors Tobias and "Yes please."
/// (`paths/289/hunt.json:358-378`).
fn sailor_scene() -> Scene {
    let mut scene = scene(
        super::super::compile::HUNT_JSON,
        "hunt",
        api::snapshot::WorldTile {
            x: 3028,
            z: 3215,
            level: 0,
        },
        "captain_tobias",
        true,
    );
    seed_items(
        &mut scene.snapshot,
        &[("coins", 200), ("zqberviriusscroll", 1)],
    );
    scene
}

/// REVIEW-VAMPIRE-RESTART-0201 R1: the scroll's menu beside a stale-facing
/// Tobias is not the Path's conversation; a fresh Start never answers it.
#[test]
fn a_foreign_item_menu_beside_a_stale_facing_sailor_is_never_answered() {
    let mut scene = sailor_scene();
    scene.menu(MULTI2_ROOT, &SCROLL_MENU);
    assert_parks_without_clicking(
        &scene,
        &mut SlotMemory::default(),
        "the scroll's menu beside Tobias",
        "2459 (Select an Option)",
    );
}

/// R1 with a record of Tobias's own conversation, stopped after its step
/// continued "The trip will cost you 30 coins." (`sailors.rs2:13`): the frames
/// that end that conversation (his menu, the operator's own "No, thank you.",
/// the scroll's message) drop the record before the scroll's menu opens; and
/// a record pinned on his menu does not answer the scroll's menu that
/// replaced it, even when no frame between them was watched.
#[test]
fn a_carried_sailor_conversation_never_answers_the_scroll_menu_after_it() {
    let carried = |scene: &Scene| {
        let texts = [
            "Captain Tobias".to_owned(),
            "The trip will cost you 30 coins.".to_owned(),
        ];
        let mut memory = SlotMemory::default();
        memory.cell.quester().conversation = Some(Box::new(HeldConversation {
            path: scene.path.id.clone(),
            digest: scene.path.digest,
            step: FactKey::new("hunt-smuggle-rum"),
            child: Some(FactKey::new("rum-sail-to-karamja")),
            npc_type: scene.npc_type(),
            npc_index: NPC_INDEX as i32,
            options: DialogueOptions {
                prefer: Arc::from([Arc::from("Yes please.")]),
                strict: true,
                chat_only: true,
                ..DialogueOptions::default()
            },
            page: ChatPage::observe(NPC_PAGE_ROOT, -1, &texts, &[], 10).unwrap(),
            menu: None,
            carried: false,
        }));
        operator_stop(&mut memory);
        memory
    };
    let page = "2459 (Select an Option)";

    let mut scene = sailor_scene();
    let mut memory = carried(&scene);
    scene.page(
        NPC_PAGE_ROOT,
        &["Captain Tobias", "The trip will cost you 30 coins."],
    );
    assert!(watch(&mut memory, &scene, 20), "the step's own page");
    scene.menu(MULTI2_ROOT, &SAIL_MENU);
    assert!(watch(&mut memory, &scene, 21), "his menu, pinned");
    scene.page(PLAYER_PAGE_ROOT, &["Player", "No, thank you."]);
    assert!(
        !watch(&mut memory, &scene, 22),
        "the operator's own answer ends it"
    );
    scene.page(
        519,
        &["This looks like part of a scroll about someone called Bervirius. Would you like to read it?"],
    );
    assert!(!watch(&mut memory, &scene, 23));
    scene.menu(MULTI2_ROOT, &SCROLL_MENU);
    assert!(memory.held().is_none());
    assert_parks_without_clicking(&scene, &mut memory, "after the watch dropped it", page);

    let mut scene = sailor_scene();
    let mut memory = carried(&scene);
    scene.menu(MULTI2_ROOT, &SAIL_MENU);
    assert!(watch(&mut memory, &scene, 21), "his menu, pinned");
    scene.menu(MULTI2_ROOT, &SCROLL_MENU);
    assert_parks_without_clicking(&scene, &mut memory, "pinned on his own menu", page);
}

/// Aggie's first `p_choice4` (`area_draynor/scripts/aggie.rs2:7`).
const AGGIE_MENU: [&str; 4] = [
    "What could you make for me?",
    "Cool, do you turn people into frogs?",
    "You mad old witch, you can't help me.",
    "Can you make dyes for me please?",
];
/// Her dye `p_choice4` (`aggie.rs2:72`): red, yellow, blue.
const DYE_MENU: [&str; 4] = [
    "What do you need to make red dye?",
    "What do you need to make yellow dye?",
    "What do you need to make blue dye?",
    "No thanks, I am happy the colour I am.",
];

fn aggie_scene() -> Scene {
    scene(
        super::super::compile::GOBLIN_DIPLOMACY_JSON,
        "gobdip",
        api::snapshot::WorldTile {
            x: 3086,
            z: 3258,
            level: 0,
        },
        "aggie",
        true,
    )
}

/// A restarted runner over Aggie's real pages from her first menu
/// (`aggie.rs2:1-8,20-23,71-84`): each click is acknowledged and the page the
/// content writes next is shown. Returns the first-menu answer, the dye-menu
/// answer, and whether the runner parked.
fn aggie_restart(scene: &mut Scene, memory: &mut SlotMemory) -> (Option<i32>, Option<i32>, bool) {
    scene.menu(MULTI4_ROOT, &AGGIE_MENU);
    let mut script = scene.runner();
    let mut ledger = None;
    let mut first = None;
    let mut page = 0;
    for tick in 30..90 {
        scene.frame(&mut script, memory, &mut ledger, tick);
        if !outbox_empty(&ledger) {
            let effect = ack(&mut ledger, tick);
            match (page, answer_of(&effect)) {
                (0, Some(4)) => {
                    first = Some(4);
                    scene.page(
                        PLAYER_PAGE_ROOT,
                        &["Player", "Can you make dyes for me please?"],
                    );
                    page = 1;
                }
                (0, Some(option)) => return (Some(option), None, script.parked),
                (1, None) if is_continue(&effect) => {
                    scene.page(
                        NPC_PAGE_ROOT,
                        &[
                            "Aggie",
                            "What sort of dye would you like? Red, yellow or blue?",
                        ],
                    );
                    page = 2;
                }
                (2, None) if is_continue(&effect) => {
                    scene.menu(MULTI4_ROOT, &DYE_MENU);
                    page = 3;
                }
                (3, Some(option)) => return (first, Some(option), script.parked),
                _ => panic!("tick {tick}: an unexpected action on Aggie's page {page}"),
            }
        }
        if script.parked {
            break;
        }
    }
    (first, None, script.parked)
}

/// REVIEW-VAMPIRE-RESTART-0201 R2: Goblin Diplomacy's red, yellow and blue
/// Aggie steps share "Can you make dyes for me please?" and then author
/// different colours (`paths/289/gobdip.json:165-236`). With the earlier dyes
/// consumed by the orange hand-in (`quest_gobdip.rs2:29-36`) the stage-2 red
/// and yellow steps look eligible again during the blue stage, and red spends
/// berries and coins on an unwanted dye (`aggie.rs2:104-115`). A restart must
/// answer blue or leave the menu alone, never red; with nothing recorded it
/// leaves it alone.
#[test]
fn a_blue_dye_restart_without_a_record_never_answers_red() {
    let mut scene = aggie_scene();
    let (first, dye, parked) = aggie_restart(&mut scene, &mut SlotMemory::default());
    assert_ne!(
        dye,
        Some(1),
        "restart of a blue-dye step adopted the earlier red-dye policy"
    );
    assert!(
        dye == Some(3) || (parked && first.is_none()),
        "blue, or park without a click: first {first:?}, dye {dye:?}, parked {parked}"
    );
}

/// R2 with the record of the stopped step, `acquire-blue-dye` /
/// `aggie-blue-dye` at `gobdip:3`: the restart answers with that step's own
/// policy, the shared first choice and then blue.
#[test]
fn a_blue_dye_restart_answers_with_the_stopped_steps_own_policy() {
    let mut scene = aggie_scene();
    let mut memory = stop_mid_conversation(
        &mut scene,
        "gobdip:3",
        "acquire-blue-dye",
        &["Aggie", "What can I help you with?"],
    );
    assert_eq!(
        memory
            .held()
            .and_then(|held| held.child.as_ref())
            .map(|child| child.0.as_ref()),
        Some("aggie-blue-dye")
    );
    let (first, dye, parked) = aggie_restart(&mut scene, &mut memory);
    assert_eq!(first, Some(4));
    assert_eq!(dye, Some(3), "the stopped step's own blue answer");
    assert!(!parked);
}
