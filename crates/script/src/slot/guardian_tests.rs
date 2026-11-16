//! The Guardian's Fight child in its slot (design V11D 4 and 6): offered with
//! no card loaded, it prices the escape, fights only the exact event actor,
//! and every command revokes it before the next dispatch. The host side of the
//! seams is played here: the estimate answer and the interaction receipts.
use super::*;
use crate::combat::guardian::{escape_reserve, outcome_of_end};
use crate::combat::request::{AbortReason, CombatEnd, PrepItem, Unprotected};
use crate::combat::tables::CombatTables;
use crate::combat_session::tests::World;
use crate::ctx::test_support::NullDriver;
use crate::interact::InteractReq;
use crate::native::{AssessReceipt, HostAction, HostEffect, InteractionReceipt};
use api::quest_progress::EvidenceStamp;
use api::random::{
    FightActor, FightCancel, FightEscape, FightEvent, FightOffer, FightOutcome, FightPhase,
    FightReason, FightToken,
};
use api::selected::ClientRevision;

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R289).unwrap()
}

/// A slot that never loaded a card, one world with an event actor beside the
/// bot, and the fight offered for that actor.
struct Fight {
    slot: SlotScript,
    world: World,
    token: FightToken,
    tick: u64,
}

fn offer_for(world: &World, config: &str, event: FightEvent, serial: u32) -> FightOffer {
    let here = world
        .snapshot
        .local_player()
        .expect("the bot is observed")
        .player
        .network;
    let (x, z) = (here.x, here.z);
    let ring = [
        (x + 6, z),
        (x - 6, z),
        (x, z + 6),
        (x, z - 6),
        (x + 5, z + 5),
        (x - 5, z + 5),
        (x + 5, z - 5),
        (x - 5, z - 5),
    ];
    let npc_type = world.data.npc_by_config(config).unwrap().id;
    FightOffer {
        token: FightToken { session: 1, serial },
        event,
        actor: FightActor {
            index: 7,
            npc_type: u16::try_from(npc_type).unwrap(),
        },
        escape: FightEscape {
            anchor: (x - 2, z),
            radius: 12,
            ring,
        },
        budget_ticks: 200,
    }
}

impl Fight {
    fn offered(config: &str, event: FightEvent) -> Self {
        let world = World::new(config);
        let mut slot = SlotScript::new();
        slot.bind_incarnation(1);
        let offer = offer_for(&world, config, event, 1);
        slot.guardian_offer(offer, &selected())
            .expect("the offer is admitted");
        Self {
            slot,
            world,
            token: offer.token,
            tick: 0,
        }
    }

    /// One fresh observation of the child, as the host builds it.
    fn tick(&mut self) {
        self.tick += 1;
        self.slot.guardian_tick(
            &mut ScriptCtx {
                driver: &mut NullDriver::default(),
                tick: self.tick,
                here: None,
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&self.world.snapshot),
                obj_names: None,
                compiled: crate::CompiledTick::default(),
            },
            true,
        );
    }

    fn read(&self) -> api::random::FightRead {
        self.slot
            .guardian_read(self.token)
            .expect("the child is held")
    }

    /// The host answers the estimate with a route of `ticks` walking ticks.
    fn answer_estimate(&mut self, action: &HostAction, ticks: u32) {
        let authority = action.authority();
        self.slot.complete_native_assess_walk(
            &authority,
            AssessReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: self.tick,
                    sequence: self.tick,
                },
                assessment: None,
                refusal: None,
                detail: None,
                route_ticks: ticks,
                route_moves: ticks,
            },
        );
    }

    /// Price the escape and take the child to its first attack row.
    fn run_to_first_attack(&mut self) -> HostAction {
        self.tick();
        let estimate = self
            .slot
            .take_guardian_action()
            .expect("the estimate request");
        assert!(matches!(estimate.effect, HostEffect::AssessWalk(_)));
        self.answer_estimate(&estimate, 7);
        self.tick();
        self.slot
            .take_guardian_action()
            .expect("the first attack row")
    }
}

fn is_attack_on(action: &HostAction, index: i32) -> bool {
    matches!(
        &action.effect,
        HostEffect::Interaction(InteractReq::Npc { action: verb, index: Some(slot), .. })
            if verb == "Attack" && *slot == index
    )
}

/// No card is loaded and none is needed: the offer prices the escape on the
/// first observation, sends nothing else, and attacks the exact actor once the
/// walk is priced.
#[test]
fn an_offered_fight_prices_the_escape_before_it_attacks_the_exact_actor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    assert_eq!(fight.read().phase, FightPhase::Offered);

    fight.tick();
    let estimate = fight
        .slot
        .take_guardian_action()
        .expect("the estimate request");
    let HostEffect::AssessWalk(request) = &estimate.effect else {
        panic!("the first row is the estimate");
    };
    assert_eq!(
        request
            .candidate_goals
            .as_ref()
            .map(|goals| goals.as_slice().len()),
        Some(8),
        "the estimate prices the Guardian's own eight candidates"
    );
    assert!(
        fight.slot.take_guardian_action().is_none(),
        "nothing else goes out before the walk is priced"
    );
    assert_eq!(fight.read().outcome, FightOutcome::Pending);

    fight.answer_estimate(&estimate, 7);
    fight.tick();
    let attack = fight.slot.take_guardian_action().expect("the first attack");
    assert!(is_attack_on(&attack, 7), "the exact actor is attacked");
    assert_eq!(fight.read().phase, FightPhase::Fighting);
}

/// No estimate by the next fresh observation is no fight: the child hands
/// over to Flee with a typed reason and never clicks Attack on trust.
#[test]
fn no_estimate_by_the_next_fresh_observation_means_flee_and_no_attack() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    fight.tick();
    let estimate = fight
        .slot
        .take_guardian_action()
        .expect("the estimate request");
    assert!(matches!(estimate.effect, HostEffect::AssessWalk(_)));
    fight.tick();
    assert_eq!(
        fight.read().outcome,
        FightOutcome::FleeRequired(FightReason::UnknownFacts)
    );
    assert!(
        fight.slot.take_guardian_action().is_none(),
        "no attack follows an unpriced escape"
    );
}

/// A command revokes the child before the next dispatch: the admitted attack
/// row is dropped unsent, the cancellation is the record the Guardian reads,
/// and a later observation cannot overwrite it.
#[test]
fn a_revocation_drops_the_admitted_row_and_keeps_its_cancellation() {
    if !api::test_support::game_data_available() {
        return;
    }
    for cancel in [
        FightCancel::Paused,
        FightCancel::Stopped,
        FightCancel::Replaced,
        FightCancel::CardFailed,
        FightCancel::ManualTakeover,
        FightCancel::SessionEnded,
    ] {
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
        fight.tick();
        let estimate = fight
            .slot
            .take_guardian_action()
            .expect("the estimate request");
        fight.answer_estimate(&estimate, 7);
        fight.tick();
        assert!(
            fight.slot.guardian_has_actions(),
            "an attack row waits for dispatch"
        );

        fight.slot.guardian_revoke(cancel);
        assert!(
            fight.slot.take_guardian_action().is_none(),
            "{cancel:?}: no stale attack is dispatched"
        );
        assert_eq!(fight.read().outcome, FightOutcome::Cancelled(cancel));
        fight.tick();
        fight.tick();
        assert_eq!(
            fight.read().outcome,
            FightOutcome::Cancelled(cancel),
            "{cancel:?}: a later observation cannot replace the cancellation"
        );
        assert!(fight.slot.take_guardian_action().is_none());
    }
}

/// The slot's own command entry points revoke the child with no card loaded
/// and none running: Pause, Stop and the slot's removal.
#[test]
fn pause_stop_and_removal_revoke_the_child_of_a_slot_with_no_card() {
    if !api::test_support::game_data_available() {
        return;
    }
    type Command = fn(&mut SlotScript);
    let commands: [(&str, Command, FightCancel); 3] = [
        ("pause", SlotScript::pause, FightCancel::Paused),
        ("stop", SlotScript::stop, FightCancel::Stopped),
        (
            "removal",
            SlotScript::stop_removed,
            FightCancel::SessionEnded,
        ),
    ];
    for (name, command, cancel) in commands {
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
        let attack = fight.run_to_first_attack();
        assert!(is_attack_on(&attack, 7));
        fight
            .slot
            .guardian_revoke_token(fight.token, FightCancel::Replaced);
        // A child revoked by a command stays revoked: the command that
        // follows cannot change the record.
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
        fight.tick();
        let estimate = fight
            .slot
            .take_guardian_action()
            .expect("the estimate request");
        fight.answer_estimate(&estimate, 7);
        fight.tick();
        assert!(fight.slot.guardian_has_actions());
        command(&mut fight.slot);
        assert_eq!(
            fight.read().outcome,
            FightOutcome::Cancelled(cancel),
            "{name}: the command revokes the child"
        );
        assert!(
            fight.slot.take_guardian_action().is_none(),
            "{name}: no stale attack after the command"
        );
    }
}

/// A token of another fight changes nothing: only the held child answers to
/// its own token.
#[test]
fn a_stale_token_neither_revokes_nor_reads_the_held_child() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let stale = FightToken {
        session: 1,
        serial: 99,
    };
    fight.slot.guardian_revoke_token(stale, FightCancel::Paused);
    assert!(fight.slot.guardian_read(stale).is_none());
    assert_eq!(
        fight.read().outcome,
        FightOutcome::Pending,
        "the held child is untouched"
    );
}

/// Fight applies only to the six listed event families: a type of the other
/// events, a world namesake, or an event named for another family's actor is
/// refused at the offer, whatever the Guardian believed.
#[test]
fn an_offer_for_anything_but_a_listed_event_actor_is_refused() {
    if !api::test_support::game_data_available() {
        return;
    }
    let world = World::new("macro_rivertrollguardian_1");
    let mut slot = SlotScript::new();
    slot.bind_incarnation(1);
    let mismatched = |config: &str, event: FightEvent| {
        let mut offer = offer_for(
            &world,
            "macro_rivertrollguardian_1",
            FightEvent::RiverTroll,
            2,
        );
        offer.actor.npc_type = u16::try_from(world.data.npc_by_config(config).unwrap().id).unwrap();
        offer.event = event;
        offer
    };
    for (config, event, why) in [
        ("macro_swarm", FightEvent::RiverTroll, "the Swarm"),
        (
            "macro_triffidseed_angry",
            FightEvent::RiverTroll,
            "the angry Strange plant",
        ),
        ("zombie_unarmed", FightEvent::Zombie, "the world zombie"),
        (
            "macro_golemguardian_1",
            FightEvent::RiverTroll,
            "a golem offered as a troll",
        ),
    ] {
        let offer = mismatched(config, event);
        assert_eq!(
            slot.guardian_offer(offer, &selected()),
            Err(FightReason::NotEligible),
            "{why} is not fought"
        );
        assert!(
            slot.guardian_token().is_none(),
            "{why}: no child was formed"
        );
    }
}

/// Every actor type of the six families prices its escape from content facts:
/// a reserve exists, and a higher tier never hits softer than a lower one.
#[test]
fn every_listed_family_prices_its_escape_from_content() {
    if !api::test_support::game_data_available() {
        return;
    }
    let tables = CombatTables::build(selected()).unwrap();
    for event in FightEvent::ALL {
        let types: Vec<i32> = (0..=500)
            .filter(|ty| FightEvent::of_npc_type(*ty as usize) == Some(event))
            .collect();
        assert_eq!(types.len(), 6, "{event:?} has six tiers");
        let mut hits = Vec::new();
        for ty in &types {
            let reserve = escape_reserve(&tables, *ty)
                .unwrap_or_else(|| panic!("{event:?} type {ty} has no escape reserve"));
            assert!(reserve.hit > 0, "{event:?} type {ty} can hurt");
            assert!(
                (1..=8).contains(&reserve.interval),
                "{event:?} type {ty} interval"
            );
            hits.push(reserve.hit);
        }
        assert!(
            hits.windows(2).all(|pair| pair[0] <= pair[1]),
            "{event:?}: a higher tier never hits softer: {hits:?}"
        );
    }
}

/// Every end of the shared core maps to one typed outcome, and a core end
/// that is not a kill never reads as a win.
#[test]
fn every_combat_end_maps_to_a_typed_outcome() {
    assert_eq!(
        outcome_of_end(CombatEnd::Killed, true, true),
        FightOutcome::Killed
    );
    assert_eq!(
        outcome_of_end(CombatEnd::Died, true, true),
        FightOutcome::Died
    );
    assert_eq!(
        outcome_of_end(CombatEnd::TargetGone, false, true),
        FightOutcome::TargetGone
    );
    // No target while the actor still shows is a refusal to fight it; with the
    // actor out of view it is gone.
    assert_eq!(
        outcome_of_end(CombatEnd::NoTarget, true, true),
        FightOutcome::FleeRequired(FightReason::NoTarget)
    );
    assert_eq!(
        outcome_of_end(CombatEnd::NoTarget, false, true),
        FightOutcome::TargetGone
    );
    // The combat budget hands over while the episode has time and fails once
    // the episode budget is spent.
    assert_eq!(
        outcome_of_end(CombatEnd::Budget, true, true),
        FightOutcome::FleeRequired(FightReason::CombatBudget)
    );
    assert_eq!(
        outcome_of_end(CombatEnd::Budget, true, false),
        FightOutcome::Failed(FightReason::EpisodeBudget)
    );
    // The escape reserve is the no-food abort of an Open, Abort fight.
    assert_eq!(
        outcome_of_end(
            CombatEnd::Aborted(AbortReason::Unprotected(Unprotected::NoFood)),
            true,
            true
        ),
        FightOutcome::FleeRequired(FightReason::EscapeReserve)
    );
    assert_eq!(
        outcome_of_end(
            CombatEnd::Aborted(AbortReason::PrepFailed(PrepItem::Weapon)),
            true,
            true
        ),
        FightOutcome::FleeRequired(FightReason::Preparation(api::random::FightPrep::Weapon))
    );
    // A refused PvP initiation is not an event the child may fight.
    assert!(matches!(
        outcome_of_end(
            CombatEnd::Aborted(AbortReason::PvpRefused(
                crate::combat::request::PvpRefusal::NotOptedIn
            )),
            true,
            true
        ),
        FightOutcome::Failed(FightReason::InvalidEventTarget)
    ));
}

/// The host answers a refused attack: the interaction receipt reaches the
/// child's own ledger, not a card's.
#[test]
fn a_receipt_for_the_childs_attack_reaches_the_childs_ledger() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let attack = fight.run_to_first_attack();
    assert!(is_attack_on(&attack, 7));
    let authority = attack.authority();
    fight.slot.complete_native_interaction(
        &authority,
        InteractionReceipt {
            request_id: attack.request_id.get(),
            evidence: EvidenceStamp {
                run: authority.run(),
                tick: fight.tick,
                sequence: fight.tick,
            },
            accepted: true,
            chat_since: 0,
        },
    );
    fight.tick();
    assert_eq!(
        fight.read().phase,
        FightPhase::Fighting,
        "the accepted attack keeps the fight running on the child's own ledger"
    );
    assert!(
        fight.slot.native_status().is_none(),
        "the slot's card status never saw the child"
    );
}
