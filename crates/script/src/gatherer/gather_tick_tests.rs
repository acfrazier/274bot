use super::tests::{loc, npc, prepared_catalog, supply_snapshot, target};
use super::*;
use crate::native::ledger::{Ledger, TickBudget};
use crate::native::tickcycle::{
    AnimClass, CycleSpec, CycleStats, RollSignal, Shape, TickCycle, TickPolicy,
};
use crate::native::{ActionHandle, HostEffect, InteractionReceipt, ServerOrderedCompletion};
use crate::quester::families::tests::{local_player, with_tick};
use crate::surface::ServerOrderedPayload;
use api::gather_methods::{GatherSkill, TickArmKind, TickRollSignal};
use api::interact::server_ordered::{
    RowDispatch, ServerOrderedDispatch, ServerOrderedError, ServerOrderedRequest,
};
use api::quest_progress::EvidenceStamp;
use api::snapshot::{AnimationUpdateView, ChatLineView, GameSnapshot, WorldTile};

struct Rig {
    snapshot: GameSnapshot,
    ledger: Option<Box<Ledger>>,
    budget: TickBudget,
    handle: ActionHandle<GatherRun>,
    catalog: Arc<GatherCatalog>,
    method_index: usize,
    skill_anim: i32,
    serial: u32,
    chat_sequence: i32,
    now: u64,
}

impl Rig {
    fn new(method_id: &str) -> Self {
        let catalog = prepared_catalog();
        let method_index = catalog
            .methods()
            .iter()
            .position(|row| row.id.0.as_ref() == method_id)
            .unwrap();
        let method = &catalog.methods()[method_index];
        let arm = &method.tick_arm;
        let tool = match method.skill {
            GatherSkill::Mining => 1273,
            GatherSkill::Fishing if method_id == "fishing.saltfish.op1" => 303,
            GatherSkill::Fishing => 309,
            GatherSkill::Woodcutting => 1349,
        };
        let tool_fact = arm.for_tool(tool);
        let skill_anim = tool_fact
            .map(|row| row.skill_anim)
            .or(arm.skill_anim)
            .unwrap_or(-2);
        let spec = CycleSpec::new(
            match arm.kind {
                TickArmKind::Continuing => Shape::Continuing,
                TickArmKind::Ending => Shape::Ending,
                TickArmKind::None => panic!("armable fixture"),
            },
            match arm.roll_signal {
                TickRollSignal::Always => RollSignal::Always,
                TickRollSignal::Never => RollSignal::Never,
                TickRollSignal::Sometimes => RollSignal::Sometimes,
            },
            arm.delay_drop,
            arm.depletes,
            tool_fact
                .map(|row| row.native_arm)
                .or(arm.native_arm)
                .unwrap(),
            arm.act_min_d,
            arm.act_silent,
        );
        let resource = known_rows(&method.targets)
            .iter()
            .find(|row| row.class == TargetClass::Resource)
            .unwrap();
        let tile = WorldTile {
            x: 3201,
            z: 3202,
            level: 0,
        };
        let mut plan = target(resource.entity, 42, tile);
        plan.method_index = method_index as u16;
        plan.skill_stat = match method.skill {
            GatherSkill::Mining => 14,
            GatherSkill::Fishing => 10,
            GatherSkill::Woodcutting => 8,
        };
        plan.op = catalog.op(method).unwrap().unwrap().1.into();
        for product in known_rows(&method.products) {
            plan.products[usize::from(plan.products_len)] = product.item;
            plan.products_len += 1;
        }
        let mut snapshot = supply_snapshot(99, &[tool, 946, 1511, 314]);
        snapshot.seed_inventory_packet_generations(Some(1), None);
        let mut player = local_player(tile);
        player.animation_update = Some(AnimationUpdateView {
            serial: 7,
            sequence: -1,
        });
        snapshot.seed_local_player(player);
        match resource.entity {
            EntityId::Loc(id) => {
                snapshot.seed_locs(vec![loc(id, tile)]);
                snapshot.seed_npcs(vec![]);
            }
            EntityId::Npc(id) => {
                snapshot.seed_locs(vec![]);
                snapshot.seed_npcs(vec![npc(42, id as usize, tile)]);
            }
            EntityId::Obj(_) => panic!("resource fixture"),
        }
        let mut ledger = None;
        let mut budget = TickBudget::default();
        budget.observe(100);
        let handle = with_tick(&snapshot, &mut ledger, 100, |tick| {
            std::mem::swap(&mut budget, tick.cx.budget);
            let result = tick
                .actions
                .begin::<GatherRun>(
                    GatherRunArgs {
                        target: plan,
                        catalog: Arc::clone(&catalog),
                        stall_ticks: DEFAULT_STALL_TICKS,
                        quest_owned: false,
                        timing: Some(GatherTiming {
                            cycle: TickCycle::new(spec, 100, 7, 0),
                            policy: TickPolicy::default(),
                            kit: [946, 1511],
                            skill_anim,
                            ordinary_npc: match resource.entity {
                                EntityId::Npc(id) => Some(id),
                                _ => None,
                            },
                        }),
                    },
                    &mut tick.cx,
                )
                .unwrap();
            std::mem::swap(&mut budget, tick.cx.budget);
            result
        });
        let mut rig = Self {
            snapshot,
            ledger,
            budget,
            handle,
            catalog,
            method_index,
            skill_anim,
            serial: 7,
            chat_sequence: 0,
            now: 100,
        };
        rig.complete_last(None, true);
        rig
    }

    fn animate(&mut self, class: AnimClass) {
        let arm = &self.catalog.methods()[self.method_index].tick_arm;
        let sequence = match class {
            AnimClass::Fletching => arm.fletch_anim.unwrap(),
            AnimClass::Skill => self.skill_anim,
            AnimClass::Roll => arm.roll_anim.unwrap_or(self.skill_anim),
            AnimClass::Cast => arm.cast_anim.unwrap(),
            AnimClass::Null => -1,
            AnimClass::Other => i32::MAX,
        };
        let mut player = self.snapshot.local_player().unwrap().clone();
        self.serial += 1;
        player.animation_update = Some(AnimationUpdateView {
            serial: self.serial,
            sequence,
        });
        player.player.actor.animation = sequence;
        self.snapshot.seed_local_player(player);
    }

    fn chat(&mut self, text: &str) {
        self.chat_sequence += 1;
        self.snapshot.seed_chat_lines(vec![ChatLineView {
            sequence: self.chat_sequence,
            text: text.into(),
            type_: 0,
            username: None,
        }]);
    }

    fn poll(&mut self, now: u64) -> Poll<Result<GatherResult, ActionError>> {
        self.now = now;
        self.budget.observe(now);
        let budget = &mut self.budget;
        with_tick(&self.snapshot, &mut self.ledger, now, |tick| {
            std::mem::swap(budget, tick.cx.budget);
            let result = tick.actions.poll(&self.handle, &mut tick.cx);
            std::mem::swap(budget, tick.cx.budget);
            result
        })
    }

    fn complete_last(&mut self, prefix: Option<u8>, accepted: bool) {
        let ledger = self.ledger.as_mut().unwrap();
        let action = ledger.outbox.last().unwrap();
        let authority = action.authority();
        let evidence = EvidenceStamp {
            run: authority.run(),
            tick: self.now,
            sequence: self.now,
        };
        if let HostEffect::ServerOrdered(envelope) = &action.effect {
            let ServerOrderedPayload::Content(request) = envelope.payload() else {
                panic!("content request")
            };
            let row_count = request.row_count();
            let dispatched_prefix = prefix.unwrap_or(row_count).min(row_count);
            let mut rows = [RowDispatch::NotAttempted; 5];
            let mut wire_events = 0;
            for (index, row) in rows
                .iter_mut()
                .take(usize::from(dispatched_prefix))
                .enumerate()
            {
                let cost = if index == 2 { 2 } else { 1 };
                *row = RowDispatch::HostQueued { wire_events: cost };
                wire_events += cost;
            }
            let terminal =
                (dispatched_prefix < row_count).then_some(ServerOrderedError::ContractUnavailable);
            if let Some(reason) = terminal {
                rows[usize::from(dispatched_prefix)] = RowDispatch::Refused { reason };
                for row in rows
                    .iter_mut()
                    .take(usize::from(row_count))
                    .skip(usize::from(dispatched_prefix) + 1)
                {
                    *row = RowDispatch::SkippedDependency;
                }
            }
            ledger.complete_server_ordered(
                &authority,
                ServerOrderedCompletion {
                    evidence,
                    dispatch: ServerOrderedDispatch {
                        request_id: authority.request_id().get(),
                        dispatch_tick: self.now,
                        rows,
                        row_count,
                        dispatched_prefix,
                        wire_events,
                        terminal,
                    },
                },
            );
        } else {
            ledger.complete_interaction(
                &authority,
                InteractionReceipt {
                    request_id: authority.request_id().get(),
                    evidence,
                    accepted,
                    chat_since: self.chat_sequence,
                },
            );
        }
    }

    fn stats(&self) -> CycleStats {
        self.handle
            .inspect(|machine| machine.args.timing.as_ref().unwrap().policy.stats())
            .unwrap()
    }

    fn arms(&self) -> usize {
        self.ledger.as_ref().unwrap().outbox.iter().filter(|action| matches!(&action.effect,
            HostEffect::ServerOrdered(envelope) if matches!(envelope.payload(), ServerOrderedPayload::Content(ServerOrderedRequest::KnifeArm { .. }))
        )).count()
    }

    fn clicks(&self) -> usize {
        self.ledger
            .as_ref()
            .unwrap()
            .outbox
            .iter()
            .filter(|action| {
                matches!(
                    &action.effect,
                    HostEffect::Interaction(InteractReq::Loc { .. } | InteractReq::Npc { .. })
                )
            })
            .count()
    }
}

#[test]
fn iron_cycle_uses_real_gather_run_and_reports_three_tick_verification() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 1);
    rig.complete_last(None, true);
    assert!(rig.poll(101).is_pending());
    rig.animate(AnimClass::Fletching);
    rig.chat("You swing your pick at the rock.");
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 1));
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(104).is_pending());
    assert_eq!(rig.arms(), 2);
    assert_eq!(rig.stats().verified, 1);
    assert_eq!(
        rig.handle.inspect(|machine| machine
            .args
            .timing
            .as_ref()
            .unwrap()
            .policy
            .last_cycle_ticks()),
        Some(Some(3))
    );
}

#[test]
fn retained_animation_is_not_a_new_landing_and_only_one_fallback_is_asked() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(None, true);
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    assert_eq!(rig.stats().lost, 1);
    rig.complete_last(None, true);
    assert!(rig.poll(103).is_pending());
    assert!(rig.poll(104).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(105).is_pending());
    assert_eq!(rig.arms(), 2);
}

#[test]
fn late_mining_landing_defers_split_menu_and_waits_for_its_real_roll() {
    for menu in [false, true] {
        for roll in [105, 106] {
            let mut rig = Rig::new("mining.iron");
            rig.animate(AnimClass::Skill);
            assert!(rig.poll(101).is_pending());
            rig.complete_last(None, true);
            assert!(rig.poll(102).is_pending());
            if menu {
                rig.snapshot.seed_chat_modal(8880, vec!["Make".into()]);
            }
            assert!(rig.poll(102).is_pending());
            assert_eq!((rig.arms(), rig.clicks()), (1, 1));
            rig.snapshot.seed_chat_modal(-1, vec![]);
            rig.animate(AnimClass::Fletching);
            rig.chat("You swing your pick at the rock.");
            assert!(rig.poll(103).is_pending());
            assert_eq!((rig.arms(), rig.clicks()), (1, 2));
            assert_eq!(rig.stats().slipped, 1);
            rig.complete_last(None, true);
            for now in 104..roll {
                assert!(rig.poll(now).is_pending());
                assert_eq!(rig.arms(), 1);
            }
            rig.animate(AnimClass::Skill);
            assert!(rig.poll(roll).is_pending());
            assert_eq!(rig.arms(), 2);
            assert_eq!(rig.stats().slipped, 1);
        }
    }
}

#[test]
fn ending_net_click_is_edge_only_and_its_blind_roll_is_unobserved() {
    let mut rig = Rig::new("fishing.saltfish.op1");
    rig.animate(AnimClass::Cast);
    assert!(rig.poll(101).is_pending());
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    assert!(rig.poll(104).is_pending());
    assert_eq!(rig.arms(), 1);
    rig.complete_last(None, true);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(105).is_pending());
    assert!(rig.poll(105).is_pending());
    assert_eq!(rig.clicks(), 1);
    assert!(rig.poll(106).is_pending());
    assert_eq!(rig.clicks(), 2);
    rig.complete_last(None, true);
    assert!(rig.poll(107).is_pending());
    assert_eq!(rig.arms(), 2);
    assert_eq!((rig.stats().verified, rig.stats().unobserved), (0, 1));
}

#[test]
fn zero_prefix_receipt_is_one_lost_cycle_and_one_refusable_fallback() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(Some(0), false);
    assert!(rig.poll(101).is_pending());
    assert_eq!(
        rig.clicks(),
        1,
        "the batch already spent this window's budget"
    );
    assert!(rig.poll(102).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    rig.complete_last(None, false);
    assert!(rig.poll(102).is_pending());
    assert_eq!((rig.stats().lost, rig.stats().fallback_failed), (1, 1));
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
}

#[test]
fn river_cast_at_second_edge_is_a_slip_without_a_predicted_roll() {
    let mut rig = Rig::new("fishing.freshfish.op1");
    rig.animate(AnimClass::Roll);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(None, true);
    assert!(rig.poll(102).is_pending());
    rig.animate(AnimClass::Cast);
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    assert_eq!((rig.stats().slipped, rig.stats().verified), (1, 0));
    rig.complete_last(None, true);
    for now in 104..106 {
        assert!(rig.poll(now).is_pending());
        assert_eq!(rig.arms(), 1);
    }
    rig.animate(AnimClass::Roll);
    assert!(rig.poll(106).is_pending());
    assert_eq!(rig.arms(), 2);
    assert_eq!(rig.stats().slipped, 1);
}

#[test]
fn lone_opener_consumes_its_receipt_before_stray_menu_tending() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(Some(1), false);
    rig.snapshot.seed_chat_modal(8880, vec!["Make".into()]);
    assert!(rig.poll(101).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 1));
    assert!(rig.poll(102).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    assert_eq!(rig.stats().lost, 1);
    assert!(!rig.ledger.as_ref().unwrap().outbox.iter().any(|action| {
        matches!(
            &action.effect,
            HostEffect::Interaction(InteractReq::CloseModal)
        )
    }));
    rig.complete_last(None, false);
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    assert_eq!(rig.stats().fallback_failed, 1);
}

#[test]
fn accepted_make_without_its_click_uses_one_checked_act_and_slips() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(Some(2), false);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(102).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    rig.complete_last(None, true);
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(104).is_pending());
    assert_eq!(rig.arms(), 2);
    assert_eq!(
        (rig.stats().verified, rig.stats().slipped, rig.stats().lost),
        (0, 1, 0)
    );
}

#[test]
fn refused_checked_act_delivered_after_deadline_has_one_fallback() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(Some(2), false);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(102).is_pending());
    assert_eq!(rig.clicks(), 2);
    assert!(rig.poll(103).is_pending());
    assert_eq!(
        rig.clicks(),
        2,
        "an Asked act is not retried at its deadline"
    );
    rig.complete_last(None, false);
    assert!(rig.poll(103).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 3));
    assert_eq!(rig.stats().lost, 1);
    rig.complete_last(None, true);
    assert!(rig.poll(103).is_pending());
    assert!(rig.poll(104).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 3));
}

#[test]
fn absent_packet_generation_does_not_start_a_batch_or_reuse_a_roll() {
    let mut rig = Rig::new("mining.iron");
    rig.snapshot.seed_inventory_packet_generations(None, None);
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 0);
    assert_eq!(
        rig.handle
            .inspect(|machine| machine.args.timing.unwrap().policy.cycles()),
        Some(0)
    );
    rig.snapshot
        .seed_inventory_packet_generations(Some(1), None);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 0);
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(102).is_pending());
    assert_eq!(rig.arms(), 1);
}

#[test]
fn fishing_knife_request_uses_the_current_published_npc_generation() {
    use api::interact::server_ordered::{ExactLocOrNpcRef, ExactNpcReference, KnifeArmShape};
    use api::snapshot::Family;
    use client::client::{Client, ClientConfig, ClientNpc, ClientRevision};

    let mut rig = Rig::new("fishing.freshfish.op1");
    let row = &rig.snapshot.npcs()[0];
    let tile = row.tile;
    let type_id = row.r#type.unwrap();
    let mut client = Client::from_shared_with_revision(
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 1,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        },
        Arc::new(client::config::Cache::default()),
        Arc::new(Vec::new()),
        Vec::new(),
        ClientRevision::R289,
    );
    let mut npc = ClientNpc::at(tile.x, tile.z);
    npc.r#type = Some(type_id);
    npc.entity.x = tile.x * 128 + npc.entity.size * 64;
    npc.entity.z = tile.z * 128 + npc.entity.size * 64;
    client.npc[42] = Some(Box::new(npc));
    client.npc_ids[0] = 42;
    client.npc_count = 1;
    client.gens.npc = 1;
    client.gens.npc_info = 37;
    assert!(rig.snapshot.rebuild_family(&client, Family::Npc));

    rig.animate(AnimClass::Roll);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 1);
    let HostEffect::ServerOrdered(envelope) =
        &rig.ledger.as_ref().unwrap().outbox.last().unwrap().effect
    else {
        panic!("the fishing cycle must emit its Knife request")
    };
    let ServerOrderedPayload::Content(ServerOrderedRequest::KnifeArm {
        shape:
            KnifeArmShape::Continuing {
                click: ExactLocOrNpcRef::Npc(reference),
            },
        ..
    }) = envelope.payload()
    else {
        panic!("the Knife request must retain its NPC click")
    };
    assert_eq!(
        *reference,
        ExactNpcReference::new(42, type_id as i32, tile, client.gens.npc_info, 1).unwrap()
    );
}

#[test]
fn transformed_fishing_types_are_not_armed_on_their_observed_click_tick() {
    for event_type in [390, 403, i32::MAX] {
        let mut rig = Rig::new("fishing.freshfish.op1");
        let tile = rig.snapshot.npcs()[0].tile;
        rig.snapshot
            .seed_npcs(vec![npc(42, event_type as usize, tile)]);
        rig.animate(AnimClass::Roll);
        assert!(matches!(rig.poll(101), Poll::Ready(Ok(_))));
        assert_eq!((rig.arms(), rig.clicks()), (0, 1));
    }
}

#[test]
fn ending_spot_type_change_disarms_before_the_owed_checked_click() {
    for event_type in [390, 403, i32::MAX] {
        let mut rig = Rig::new("fishing.saltfish.op1");
        rig.animate(AnimClass::Cast);
        for tick in 101..=104 {
            assert!(rig.poll(tick).is_pending());
        }
        assert_eq!(rig.arms(), 1);
        rig.complete_last(None, true);
        rig.animate(AnimClass::Fletching);
        assert!(rig.poll(105).is_pending());
        let tile = rig.snapshot.npcs()[0].tile;
        rig.snapshot
            .seed_npcs(vec![npc(42, event_type as usize, tile)]);
        assert!(matches!(rig.poll(106), Poll::Ready(Ok(_))));
        assert_eq!(
            rig.clicks(),
            1,
            "the old type's owed click is not sent to an event/unknown type"
        );
    }
}

#[test]
fn members_refusal_turns_knife_off_and_sends_only_one_native_fallback() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(None, true);
    rig.chat("To use this item please login to a members' server.");
    assert!(rig.poll(102).is_pending());
    assert_eq!((rig.arms(), rig.clicks()), (1, 2));
    rig.complete_last(None, true);
    assert!(rig.poll(102).is_pending());
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(103).is_pending());
    assert_eq!(rig.arms(), 1);
    assert_eq!(
        rig.handle
            .inspect(|machine| machine.args.timing.unwrap().policy.off_reason()),
        Some(Some(crate::native::tickcycle::OffReason::MembersOnlyArm))
    );
}

#[test]
fn late_ending_landings_count_once_per_shifted_cycle() {
    let mut rig = Rig::new("fishing.saltfish.op1");
    rig.animate(AnimClass::Cast);
    for tick in 101..=104 {
        assert!(rig.poll(tick).is_pending());
    }
    for (batch, slipped) in [(104, 1), (108, 2)] {
        rig.complete_last(None, true);
        assert!(rig.poll(batch + 1).is_pending());
        let clicks = rig.clicks();
        rig.animate(AnimClass::Fletching);
        assert!(rig.poll(batch + 2).is_pending());
        assert_eq!(
            rig.clicks(),
            clicks,
            "late landing shifts the ending click by one edge"
        );
        assert!(rig.poll(batch + 3).is_pending());
        assert_eq!(rig.clicks(), clicks + 1);
        rig.complete_last(None, true);
        assert!(rig.poll(batch + 4).is_pending());
        assert_eq!(rig.stats().slipped, slipped);
    }
}

#[test]
fn blind_net_fletch_failure_waits_for_a_native_cast_before_rearming() {
    let mut rig = Rig::new("fishing.saltfish.op1");
    rig.animate(AnimClass::Cast);
    for tick in 101..=104 {
        assert!(rig.poll(tick).is_pending());
    }
    rig.complete_last(None, true);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(105).is_pending());
    assert!(rig.poll(106).is_pending());
    rig.complete_last(None, true);
    assert!(rig.poll(107).is_pending());
    rig.complete_last(None, true);
    rig.animate(AnimClass::Fletching);
    rig.chat("You carefully cut the wood into 15 arrow shafts.");
    assert!(rig.poll(108).is_pending());
    assert_eq!((rig.stats().unobserved, rig.stats().lost), (1, 1));
    assert_eq!((rig.arms(), rig.clicks()), (2, 3));
    rig.complete_last(None, true);
    rig.animate(AnimClass::Cast);
    assert!(rig.poll(109).is_pending());
    for tick in 110..112 {
        assert!(rig.poll(tick).is_pending());
        assert_eq!(rig.arms(), 2);
    }
    assert!(rig.poll(112).is_pending());
    assert_eq!(rig.arms(), 3);
}

#[test]
fn spent_act_budget_uses_each_familys_real_deadline() {
    for (method, lost) in [("mining.iron", 0), ("fishing.freshfish.op1", 1)] {
        let mut rig = Rig::new(method);
        rig.animate(if lost == 0 {
            AnimClass::Skill
        } else {
            AnimClass::Roll
        });
        assert!(rig.poll(101).is_pending());
        rig.complete_last(Some(2), false);
        rig.animate(AnimClass::Fletching);
        rig.budget.observe(102);
        assert!(rig.budget.event(false));
        assert!(rig.poll(102).is_pending());
        assert_eq!(rig.clicks(), 1);
        assert!(rig.poll(103).is_pending());
        assert_eq!(rig.clicks(), 2);
        assert_eq!(rig.stats().lost, lost);
    }
}

#[test]
fn native_skill_multi_menu_is_closed_and_not_mistaken_for_a_split() {
    let mut rig = Rig::new("mining.iron");
    rig.snapshot.seed_chat_modal(8880, vec!["Make".into()]);
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 0);
    assert!(matches!(
        rig.ledger.as_ref().unwrap().outbox.last().unwrap().effect,
        HostEffect::Interaction(InteractReq::CloseModal)
    ));
    rig.complete_last(None, true);
    assert!(
        rig.poll(101).is_pending(),
        "an accepted close receipt alone does not prove the menu closed"
    );
    rig.snapshot.seed_chat_modal(-1, vec![]);
    rig.snapshot.seed_main_modal(-1, vec![]);
    assert!(matches!(
        rig.poll(102),
        Poll::Ready(Ok(GatherResult {
            end: GatherEnd::Refused,
            ..
        }))
    ));
}

fn replacement(rig: &mut Rig) -> TargetPlan {
    let old = rig
        .handle
        .inspect(|machine| machine.args.target.clone())
        .unwrap();
    let mut next = old.clone();
    next.tile.x += 1;
    let method = &rig.catalog.methods()[rig.method_index];
    let depleted = known_rows(&method.targets)
        .iter()
        .find_map(|row| match (row.class, row.entity) {
            (TargetClass::Depleted, EntityId::Loc(id)) => Some(id),
            _ => None,
        })
        .unwrap();
    let EntityId::Loc(id) = old.entity else {
        panic!("rock fixture")
    };
    rig.snapshot
        .seed_locs(vec![loc(depleted, old.tile), loc(id, next.tile)]);
    next
}

fn adopt(rig: &mut Rig, target: TargetPlan, result: GatherResult, now: u64) {
    let budget = &mut rig.budget;
    rig.now = now;
    rig.handle = with_tick(&rig.snapshot, &mut rig.ledger, now, |tick| {
        std::mem::swap(budget, tick.cx.budget);
        let handle = tick
            .actions
            .begin::<GatherRun>(
                GatherRunArgs {
                    target,
                    catalog: Arc::clone(&rig.catalog),
                    stall_ticks: DEFAULT_STALL_TICKS,
                    quest_owned: false,
                    timing: result.timing,
                },
                &mut tick.cx,
            )
            .unwrap();
        std::mem::swap(budget, tick.cx.budget);
        handle
    });
}

#[test]
fn copper_delayed_success_hands_the_parked_cycle_to_a_replacement_in_the_same_frame() {
    for wake_line in [true, false] {
        let mut rig = Rig::new("mining.copper");
        rig.animate(AnimClass::Skill);
        assert!(rig.poll(101).is_pending());
        rig.complete_last(None, true);
        if wake_line {
            rig.chat("You manage to mine some copper.");
            assert!(rig.poll(101).is_pending());
        }
        let target = replacement(&mut rig);
        rig.animate(AnimClass::Null);
        if !wake_line {
            rig.chat("You manage to mine some copper.");
        }
        let Poll::Ready(Ok(result)) = rig.poll(102) else {
            panic!("depleted copper run must hand off")
        };
        assert_eq!(result.end, GatherEnd::Depleted);
        assert_eq!((rig.arms(), rig.clicks()), (1, 1));
        adopt(&mut rig, target.clone(), result, 102);
        assert_eq!(
            (rig.arms(), rig.clicks()),
            (1, 0),
            "the old owner's queued effects are revoked; the replacement only arms"
        );
        rig.complete_last(None, true);
        assert!(rig.poll(102).is_pending());
        assert_eq!((rig.stats().delayed, rig.stats().wire_events), (1, 8));
        let action = rig.ledger.as_ref().unwrap().outbox.last().unwrap();
        let HostEffect::ServerOrdered(envelope) = &action.effect else {
            panic!("replacement arm")
        };
        let ServerOrderedPayload::Content(ServerOrderedRequest::KnifeArm {
            shape:
                api::interact::server_ordered::KnifeArmShape::Continuing {
                    click: api::interact::server_ordered::ExactLocOrNpcRef::Loc(loc),
                },
            ..
        }) = envelope.payload()
        else {
            panic!("replacement loc click")
        };
        let expected = with_tick(&rig.snapshot, &mut None, 102, |tick| {
            let snapshot = tick.cx.snapshot();
            let row = snapshot
                .locs()
                .unwrap()
                .value
                .iter()
                .find(|row| row.tile == target.tile)
                .unwrap();
            let (scene, static_locs, _) = snapshot.ground_collision_generations().unwrap();
            let op = known_rows(&rig.catalog.methods()[rig.method_index].targets)
                .iter()
                .find(|row| row.entity == target.entity)
                .unwrap()
                .op;
            api::interact::server_ordered::ExactLocReference::new(
                row.id,
                row.typecode,
                row.tile,
                scene,
                static_locs,
                op,
            )
            .unwrap()
        });
        assert_eq!(*loc, expected);
        rig.animate(AnimClass::Fletching);
        rig.chat("You swing your pick at the rock.");
        assert!(rig.poll(103).is_pending());
        assert!(rig.poll(104).is_pending());
        rig.animate(AnimClass::Skill);
        assert!(rig.poll(105).is_pending());
        assert_eq!(rig.stats().verified, 1);
    }
}

#[test]
fn iron_null_success_waits_for_the_loc_wake_then_arms_the_replacement_without_a_native_click() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(None, true);
    rig.animate(AnimClass::Fletching);
    rig.chat("You swing your pick at the rock.");
    assert!(rig.poll(102).is_pending());
    assert!(rig.poll(103).is_pending());
    rig.animate(AnimClass::Null);
    rig.chat("You manage to mine some iron.");
    assert!(rig.poll(104).is_pending());
    assert_eq!(
        rig.arms(),
        1,
        "the retained active loc is not a replacement target"
    );
    assert!(rig.poll(104).is_pending());
    assert_eq!(rig.arms(), 1);
    let target = replacement(&mut rig);
    let Poll::Ready(Ok(result)) = rig.poll(104) else {
        panic!("loc wake must finish the old rock")
    };
    assert_eq!((rig.arms(), rig.clicks()), (1, 1));
    adopt(&mut rig, target, result, 104);
    assert_eq!(
        (rig.arms(), rig.clicks()),
        (1, 0),
        "the replacement emits one batch and no native click"
    );
    assert_eq!(rig.stats().verified, 1);
}

#[test]
fn depletion_keeps_the_real_action_owner_until_its_checked_receipt_is_consumed() {
    for accepted in [true, false] {
        let mut rig = Rig::new("mining.iron");
        rig.animate(AnimClass::Skill);
        assert!(rig.poll(101).is_pending());
        rig.complete_last(Some(2), false);
        rig.animate(AnimClass::Fletching);
        assert!(rig.poll(102).is_pending());
        let authority = rig
            .ledger
            .as_ref()
            .unwrap()
            .outbox
            .last()
            .unwrap()
            .authority();
        assert!(rig.poll(103).is_pending());
        let target = replacement(&mut rig);
        rig.animate(AnimClass::Null);
        rig.chat("You manage to mine some iron.");
        assert!(
            rig.poll(104).is_pending(),
            "depletion must not revoke the pending act"
        );
        assert!(authority.live());
        assert!(rig.handle.inspect(|_| ()).is_some());
        rig.complete_last(None, accepted);
        let Poll::Ready(Ok(result)) = rig.poll(104) else {
            panic!("consume the old owner's receipt before handoff")
        };
        assert_eq!(result.end, GatherEnd::Depleted);
        assert!(!result
            .timing
            .as_ref()
            .unwrap()
            .cycle
            .has_unresolved_requests());
        adopt(&mut rig, target, result, 104);
        assert!(
            !authority.live(),
            "replacement revokes the old owner only after receipt consumption"
        );
        with_tick(&rig.snapshot, &mut rig.ledger, 104, |tick| {
            assert!(
                tick.cx
                    .interaction_receipt(authority.request_id().get())
                    .is_none(),
                "the replacement must not retrieve the previous owner's receipt"
            );
        });
    }
}

#[test]
fn depleting_idle_handoff_survives_two_tick_replacement_approach_with_real_receipt() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(Some(2), false);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(102).is_pending());
    let authority = rig
        .ledger
        .as_ref()
        .unwrap()
        .outbox
        .last()
        .unwrap()
        .authority();
    assert!(rig.poll(103).is_pending());

    let old_tile = rig
        .handle
        .inspect(|machine| machine.args.target.tile)
        .unwrap();
    let target = replacement(&mut rig);
    assert_eq!(target.tile.x.abs_diff(old_tile.x), 1);
    rig.animate(AnimClass::Null);
    rig.chat("You manage to mine some iron.");
    assert!(rig.poll(104).is_pending());
    assert!(
        authority.live(),
        "the old click owner waits for its receipt"
    );
    rig.complete_last(None, true);
    let Poll::Ready(Ok(result)) = rig.poll(104) else {
        panic!("the consumed receipt must release the depleted owner")
    };
    assert_eq!(result.end, GatherEnd::Depleted);
    assert!(!result
        .timing
        .as_ref()
        .unwrap()
        .cycle
        .has_unresolved_requests());

    adopt(&mut rig, target.clone(), result, 106);
    assert!(
        !authority.live(),
        "the replacement revokes the old action only after receipt consumption"
    );
    assert_eq!(
        (rig.arms(), rig.clicks()),
        (0, 0),
        "the replacement waits through the skipped approach edge"
    );
    assert!(rig.poll(106).is_pending());
    assert_eq!(
        (rig.arms(), rig.clicks()),
        (1, 0),
        "the proven idle cycle arms the adjacent replacement exactly once"
    );
    let HostEffect::ServerOrdered(envelope) =
        &rig.ledger.as_ref().unwrap().outbox.last().unwrap().effect
    else {
        panic!("the replacement must use its knife arm")
    };
    let ServerOrderedPayload::Content(ServerOrderedRequest::KnifeArm {
        shape:
            api::interact::server_ordered::KnifeArmShape::Continuing {
                click: api::interact::server_ordered::ExactLocOrNpcRef::Loc(loc),
            },
        ..
    }) = envelope.payload()
    else {
        panic!("the replacement arm must click the selected rock")
    };
    let expected = with_tick(&rig.snapshot, &mut None, 106, |tick| {
        let snapshot = tick.cx.snapshot();
        let row = snapshot
            .locs()
            .unwrap()
            .value
            .iter()
            .find(|row| row.tile == target.tile)
            .unwrap();
        let (scene, static_locs, _) = snapshot.ground_collision_generations().unwrap();
        let op = known_rows(&rig.catalog.methods()[rig.method_index].targets)
            .iter()
            .find(|row| row.entity == target.entity)
            .unwrap()
            .op;
        api::interact::server_ordered::ExactLocReference::new(
            row.id,
            row.typecode,
            row.tile,
            scene,
            static_locs,
            op,
        )
        .unwrap()
    });
    assert_eq!(*loc, expected);
    assert_eq!(
        (
            rig.stats().verified,
            rig.stats().delayed,
            rig.stats().slipped
        ),
        (0, 0, 1),
        "a recovered late cycle stays a slip while preserving the proven Idle handoff"
    );
}

#[test]
fn skipped_edge_during_unproven_live_cycle_stays_unobserved_without_rearming() {
    let mut rig = Rig::new("mining.iron");
    rig.animate(AnimClass::Skill);
    assert!(rig.poll(101).is_pending());
    assert_eq!(rig.arms(), 1);
    rig.complete_last(None, true);
    assert!(rig.poll(101).is_pending());

    assert!(rig.poll(103).is_pending());
    assert_eq!(rig.stats().unobserved, 1);
    assert!(rig.poll(103).is_pending());
    assert_eq!(
        rig.arms(),
        1,
        "a skipped live cycle must wait for a fresh native roll"
    );
}

#[test]
fn a_terminal_row_observed_after_landing_never_enters_the_verified_fraction() {
    let mut rig = Rig::new("fishing.freshfish.op1");
    rig.animate(AnimClass::Roll);
    assert!(rig.poll(101).is_pending());
    rig.complete_last(None, true);
    rig.animate(AnimClass::Fletching);
    assert!(rig.poll(102).is_pending());
    rig.complete_last(None, true);
    rig.animate(AnimClass::Skill);
    rig.chat("You attempt to catch a fish.");
    assert!(rig.poll(103).is_pending());
    assert_eq!(
        (rig.stats().verified, rig.stats().slipped, rig.stats().lost),
        (0, 0, 0),
        "the accepted recovery waits for its native roll before closing the cycle"
    );
    rig.complete_last(None, true);
    assert!(rig.poll(103).is_pending());
    rig.animate(AnimClass::Roll);
    assert!(rig.poll(104).is_pending());
    assert_eq!(
        rig.stats().slipped,
        1,
        "the recovered cycle enters the ring once"
    );
}
