//! The Guardian's Fight child in its slot (design V11D 4 and 6): offered with
//! no card loaded, it prices the escape from the live scene, fights only the
//! exact event actor, and every command revokes it before the next dispatch.
//! The host side of the seams is played here: the live scene the host posts
//! on every observation and the interaction receipts.
use super::*;
use crate::combat::frame::Frame;
use crate::combat::guardian::{escape_reserve, escape_walk, outcome_of_end, EscapeWalk};
use crate::combat::request::{AbortReason, CombatEnd, PrepItem, Unprotected};
use crate::combat::tables::CombatTables;
use crate::combat::EscapeCost;
use crate::combat_session::tests::World;
use crate::ctx::test_support::NullDriver;
use crate::interact::InteractReq;
use crate::native::{HostAction, HostEffect, InteractionReceipt};
use crate::reach_route::raw_walk_tests::{
    self as raw_walk, corners, maze, scattered, Prng, RawWalk,
};
use api::query::{pack_reach_query, ReachQueryView, SceneQuery};
use api::quest_progress::EvidenceStamp;
use api::random::{
    escape_candidates, escape_distance, first_escape_target, FightActor, FightCancel, FightEscape,
    FightEvent, FightOffer, FightOutcome, FightPhase, FightReason, FightToken,
};
use api::selected::{ClientRevision, RunKey};
use api::snapshot::{SceneView, SnapshotView};
use api::WorldTile;
use client::dash3d::CollisionFlag;

fn selected() -> Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(ClientRevision::R289).unwrap()
}

fn here_of(world: &World) -> WorldTile {
    world
        .snapshot
        .local_player()
        .expect("the bot is observed")
        .player
        .network
}

/// The live scene a test posts: an open square around the bot with some tiles
/// blocked, or a scene of its own (the real client's collision). The host
/// floods it from the player's tile on every observation, and so does
/// [`Ground::view`].
#[derive(Clone)]
struct Ground {
    base: (i32, i32),
    size: i32,
    blocked: Vec<(i32, i32)>,
    /// The tile the flood starts from when it is not the player's: a flood
    /// that does not start on the player is not live.
    flood_from: Option<WorldTile>,
    /// A scene posted as it is, in place of the square.
    scene: Option<SceneView>,
}

impl Ground {
    fn around(here: WorldTile) -> Self {
        Self {
            base: (here.x - 30, here.z - 30),
            size: 61,
            blocked: Vec::new(),
            flood_from: None,
            scene: None,
        }
    }

    /// `scene`, posted as it is.
    fn of_scene(scene: SceneView) -> Self {
        Self {
            base: (scene.base_x, scene.base_z),
            size: scene.width,
            blocked: Vec::new(),
            flood_from: None,
            scene: Some(scene),
        }
    }

    /// The open square with the blocked tiles closed.
    fn square(&self) -> SceneView {
        let mut scene = SceneView {
            available: true,
            base_x: self.base.0,
            base_z: self.base.1,
            level: 0,
            width: self.size,
            height: self.size,
            collision_flags: vec![0; (self.size * self.size) as usize],
        };
        for &(x, z) in &self.blocked {
            let index = ((x - self.base.0) * self.size + (z - self.base.1)) as usize;
            scene.collision_flags[index] |= CollisionFlag::SQ_BLOCKED;
        }
        scene
    }

    /// The posted reach view for a player standing on `player`.
    fn view(&self, player: WorldTile) -> ReachQueryView {
        let scene = match &self.scene {
            Some(scene) => scene.clone(),
            None => self.square(),
        };
        let flood = SceneQuery::new(&scene, Some(self.flood_from.unwrap_or(player))).flood_reach();
        pack_reach_query(&scene, flood.as_ref())
    }
}

/// A slot that never loaded a card, one world with an event actor beside the
/// bot, and the fight offered for that actor.
struct Fight {
    slot: SlotScript,
    world: World,
    token: FightToken,
    escape: FightEscape,
    tick: u64,
    /// The live scene posted on every observation; `None` posts no reach.
    ground: Option<Ground>,
}

fn offer_for(world: &World, config: &str, event: FightEvent, serial: u32) -> FightOffer {
    let here = here_of(world);
    let npc_type = world.data.npc_by_config(config).unwrap().id;
    FightOffer {
        token: FightToken { session: 1, serial },
        event,
        actor: FightActor {
            index: 7,
            npc_type: u16::try_from(npc_type).unwrap(),
        },
        escape: FightEscape {
            anchor: (here.x - 2, here.z),
            radius: 12,
        },
        budget_ticks: 200,
    }
}

impl Fight {
    fn offered(config: &str, event: FightEvent) -> Self {
        Self::offered_with(config, event, None)
    }

    /// As [`Fight::offered`], with the escape around `escape` when it is given.
    fn offered_with(config: &str, event: FightEvent, escape: Option<FightEscape>) -> Self {
        let world = World::new(config);
        let mut slot = SlotScript::new();
        slot.bind_incarnation(1);
        let mut offer = offer_for(&world, config, event, 1);
        if let Some(escape) = escape {
            offer.escape = escape;
        }
        slot.guardian_offer(offer, &selected())
            .expect("the offer is admitted");
        let ground = Ground::around(here_of(&world));
        Self {
            slot,
            world,
            token: offer.token,
            escape: offer.escape,
            tick: 0,
            ground: Some(ground),
        }
    }

    fn here(&self) -> WorldTile {
        here_of(&self.world)
    }

    /// One fresh observation of the child, as the host builds it: the live
    /// scene is flooded from the player's tile.
    fn tick(&mut self) {
        self.tick += 1;
        let reach = self.ground.as_ref().map(|ground| ground.view(self.here()));
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
                compiled: crate::CompiledTick {
                    reach: reach.as_ref(),
                    ..crate::CompiledTick::default()
                },
            },
            true,
        );
    }

    fn read(&self) -> api::random::FightRead {
        self.slot
            .guardian_read(self.token)
            .expect("the child is held")
    }

    /// The walk the escape takes from the bot's tile over `ground`, as the
    /// child prices it.
    fn priced(&self, ground: &Ground) -> Option<EscapeWalk> {
        self.priced_view(&ground.view(self.here()))
    }

    /// The walk the escape takes from the bot's tile over the posted `view`,
    /// as the child prices it.
    fn priced_view(&self, view: &ReachQueryView) -> Option<EscapeWalk> {
        let stamp = EvidenceStamp {
            run: RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            tick: 1,
            sequence: 1,
        };
        let frame = Frame::borrow(
            SnapshotView::new(Some(&self.world.snapshot), stamp).with_reach(Some(view)),
        )?;
        escape_walk(&self.escape, &frame)
    }

    /// The child's first observation, which prices the walk and attacks.
    fn first_attack(&mut self) -> HostAction {
        self.tick();
        self.slot
            .take_guardian_action()
            .expect("the first attack row")
    }

    /// The host accepts `action`.
    fn accept(&mut self, action: &HostAction) {
        let authority = action.authority();
        self.slot.complete_native_interaction(
            &authority,
            InteractionReceipt {
                request_id: action.request_id.get(),
                evidence: EvidenceStamp {
                    run: authority.run(),
                    tick: self.tick,
                    sequence: self.tick,
                },
                accepted: true,
                chat_since: 0,
            },
        );
    }

    fn set_hitpoints(&mut self, hitpoints: i32) {
        let mut stats = self.world.snapshot.stats().to_vec();
        stats
            .iter_mut()
            .find(|stat| stat.index == 3)
            .expect("hitpoints")
            .effective = hitpoints;
        self.world.snapshot.seed_stats(stats);
    }

    fn move_player(&mut self, to: WorldTile) {
        let mut local = self
            .world
            .snapshot
            .local_player()
            .expect("the bot is observed")
            .clone();
        local.player.network = to;
        local.player.actor.tile = to;
        self.world.snapshot.seed_local_player(local);
    }

    /// `count` cooked karambwan in the pack: the food that locks input.
    fn carry_karambwan(&mut self, count: i32) {
        let food = self
            .world
            .data
            .item_by_alias("tbwt_cooked_karambwan")
            .unwrap();
        self.world.snapshot.seed_inventory(
            vec![api::snapshot::ItemView {
                def: api::obj_names::ItemDefView {
                    id: food.id,
                    name: food.name.clone(),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: api::snapshot::ItemContainer::Inventory,
                action_family: api::snapshot::ItemActionFamily::Held,
                slot: 0,
                count,
                actions: Vec::new(),
                component_id: 3214,
            }],
            28,
        );
    }

    /// The tick of the karambwan bite the child admits on its next
    /// observation, with the host accepting every row it sends.
    fn bite(&mut self) -> u16 {
        self.tick();
        let mut bite = None;
        while let Some(action) = self.slot.take_guardian_action() {
            if matches!(
                &action.effect,
                HostEffect::Interaction(InteractReq::Held { action, .. }) if action == "Eat"
            ) {
                bite = Some(self.tick as u16);
            }
            self.accept(&action);
        }
        bite.expect("the Combat core admitted a karambwan bite")
    }
}

fn is_attack_on(action: &HostAction, index: i32) -> bool {
    matches!(
        &action.effect,
        HostEffect::Interaction(InteractReq::Npc { action: verb, index: Some(slot), .. })
            if verb == "Attack" && *slot == index
    )
}

/// The ring of candidates the Guardian's escape tries from `fight`'s bot.
fn ring_of(fight: &Fight) -> Vec<(i32, i32)> {
    let here = fight.here();
    let player = (here.x, here.z);
    let distance = escape_distance(fight.escape.anchor, fight.escape.radius, player);
    escape_candidates(fight.escape.anchor, player, distance).collect()
}

fn chebyshev(a: WorldTile, b: (i32, i32)) -> u32 {
    (a.x - b.0).abs().max((a.z - b.1).abs()) as u32
}

/// No card is loaded and none is needed: the first observation prices the
/// escape from the live scene, sends nothing but the attack, and attacks the
/// exact actor.
#[test]
fn an_offered_fight_prices_the_escape_from_the_live_scene_and_attacks_the_exact_actor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    assert_eq!(fight.read().phase, FightPhase::Offered);

    let attack = fight.first_attack();
    assert!(is_attack_on(&attack, 7), "the exact actor is attacked");
    assert_eq!(fight.read().phase, FightPhase::Fighting);
    while let Some(row) = fight.slot.take_guardian_action() {
        assert!(
            !matches!(row.effect, HostEffect::AssessWalk(_)),
            "no route assessment goes out: the walk is priced from the live scene"
        );
    }
}

/// A walk no one can price is not a walk the fight may bet on: with no live
/// scene, a scene that is not flooded from the player, or every candidate
/// walled off, the child hands over to Flee with a typed reason and never
/// clicks Attack on trust.
#[test]
fn a_walk_that_cannot_be_priced_means_flee_and_no_attack() {
    if !api::test_support::game_data_available() {
        return;
    }
    type Setup = fn(&Fight) -> Option<Ground>;
    let cases: [(&str, Setup); 3] = [
        ("no live scene is posted", |_| None),
        ("the flood starts away from the player", |fight| {
            let here = fight.here();
            let mut ground = Ground::around(here);
            ground.flood_from = Some(WorldTile {
                x: here.x + 4,
                ..here
            });
            Some(ground)
        }),
        ("every candidate is walled off", |fight| {
            let mut ground = Ground::around(fight.here());
            ground.blocked = ring_of(fight);
            Some(ground)
        }),
    ];
    for (why, ground) in cases {
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
        fight.ground = ground(&fight);
        fight.tick();
        assert_eq!(
            fight.read().outcome,
            FightOutcome::FleeRequired(FightReason::UnknownFacts),
            "{why}"
        );
        assert!(
            fight.slot.take_guardian_action().is_none(),
            "{why}: no attack follows an unpriced escape"
        );
    }
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
        let attack = fight.first_attack();
        assert!(is_attack_on(&attack, 7));
        // A child revoked by a command stays revoked: the command that
        // follows cannot change the record.
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
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

/// R3-2: a Pause, a Stop or the slot's removal that revokes a child while the
/// Combat core fights publishes the input lock of the bite the core admitted,
/// so the Guardian's own walk after the revocation never clicks inside it. The
/// child has no upkeep guard in this state: the lock is the core's.
#[test]
fn a_command_during_a_combat_bite_publishes_the_lock_the_core_admitted() {
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
        let attack = fight.first_attack();
        assert!(is_attack_on(&attack, 7));
        fight.accept(&attack);
        fight.world.attack();
        fight.set_hitpoints(2);
        fight.carry_karambwan(2);
        let bite = fight.bite();
        assert_eq!(fight.read().phase, FightPhase::Fighting);
        assert_eq!(
            fight.read().blocked_until,
            None,
            "{name}: nothing is published while the core runs"
        );

        command(&mut fight.slot);
        let read = fight.read();
        assert_eq!(read.outcome, FightOutcome::Cancelled(cancel), "{name}");
        // A karambwan bite locks input through the next three ticks: the
        // first click the Guardian may send is the fourth tick after it.
        assert_eq!(
            read.blocked_until,
            Some(bite.wrapping_add(4)),
            "{name}: the command publishes the core's admitted lock before it revokes"
        );
    }
}

/// A fight that has no bite in flight publishes no lock: a revocation of a
/// child whose core is free blocks nothing.
#[test]
fn a_command_while_the_core_is_free_publishes_no_lock() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let attack = fight.first_attack();
    fight.accept(&attack);
    fight.tick();
    fight.slot.pause();
    assert_eq!(
        fight.read().outcome,
        FightOutcome::Cancelled(FightCancel::Paused)
    );
    assert_eq!(fight.read().blocked_until, None);
}

/// A core that decided to hand over while its bite still locks input winds
/// down under the lock, and reports no end until the lock is over. A command
/// in that state still publishes the lock: the hand over the core committed is
/// not a release of the bite's lock. The walk can no longer be priced on the
/// observation after the bite, so the core commits to the hand over at once.
#[test]
fn a_command_while_the_core_winds_down_under_a_bite_lock_publishes_the_lock() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let attack = fight.first_attack();
    fight.accept(&attack);
    fight.world.attack();
    fight.set_hitpoints(2);
    fight.carry_karambwan(2);
    let bite = fight.bite();
    let mut walled = Ground::around(fight.here());
    walled.blocked = ring_of(&fight);
    fight.ground = Some(walled);
    fight.tick();
    let read = fight.read();
    assert_eq!(
        read.outcome,
        FightOutcome::Pending,
        "the lock holds the end"
    );
    assert_eq!(read.phase, FightPhase::Ending, "the core committed to it");

    fight.slot.pause();
    let read = fight.read();
    assert_eq!(read.outcome, FightOutcome::Cancelled(FightCancel::Paused));
    assert_eq!(
        read.blocked_until,
        Some(bite.wrapping_add(4)),
        "the command publishes the lock the winding-down core still holds"
    );
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
    let attack = fight.first_attack();
    assert!(is_attack_on(&attack, 7));
    fight.accept(&attack);
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

/// The walk is priced to the first outward candidate the client can walk to,
/// in the order the Guardian tries them (design V11D 5.3, 6.2): in open ground
/// the first of the ring, at its Chebyshev distance in steps; a candidate a
/// wall closes off is skipped, so the walk the Guardian takes is the walk the
/// fight priced.
#[test]
fn the_escape_is_priced_to_the_first_candidate_the_client_can_walk_to() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let here = fight.here();
    let ring = ring_of(&fight);
    let open = fight
        .priced(&Ground::around(here))
        .expect("open ground prices");
    assert_eq!((open.target.x, open.target.z), ring[0]);
    assert_eq!(open.steps, chebyshev(here, ring[0]));

    for walled in [1usize, 2, 5] {
        let mut ground = Ground::around(here);
        ground.blocked = ring[..walled].to_vec();
        let walk = fight.priced(&ground).expect("a later candidate prices");
        assert_eq!(
            (walk.target.x, walk.target.z),
            ring[walled],
            "{walled} candidates walled off: the walk is the next one's"
        );
        assert_eq!(walk.steps, chebyshev(here, ring[walled]));
    }

    let mut ground = Ground::around(here);
    ground.blocked = ring.clone();
    assert_eq!(fight.priced(&ground), None, "no candidate, no price");

    // The design's worked example: an open seven-step walk is priced at the
    // conservative eleven ticks, not at seven (design V11D 5.3).
    let mut seven = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    seven.escape = FightEscape {
        anchor: (here.x, here.z),
        radius: 7,
    };
    let walk = seven
        .priced(&Ground::around(here))
        .expect("open ground prices");
    assert_eq!((walk.steps, walk.ticks), (7, 11));
}

/// A detour is priced: a wall between the player and the same candidate keeps
/// the candidate and lengthens the route, and with it the bound.
#[test]
fn a_detour_around_a_wall_is_priced_in_steps() {
    if !api::test_support::game_data_available() {
        return;
    }
    let fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let here = fight.here();
    let open = fight.priced(&Ground::around(here)).unwrap();
    let mut wall = Ground::around(here);
    wall.blocked = (here.z - 6..=here.z + 6).map(|z| (here.x + 3, z)).collect();
    let detour = fight.priced(&wall).expect("the wall has a way round");
    assert_eq!(detour.target, open.target, "the same candidate");
    assert!(
        detour.steps > open.steps && detour.ticks > open.ticks,
        "{} steps and {} ticks round the wall against {} and {} in the open",
        detour.steps,
        detour.ticks,
        open.steps,
        open.ticks
    );
}

/// The first candidate follows the player around the anchor: the walk is
/// priced where the Guardian's escape would go from the tile the bot stands
/// on now, not from the tile it stood on at the offer.
#[test]
fn the_first_candidate_follows_the_player_around_the_anchor() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let here = fight.here();
    let ground = Ground::around(here);
    let before = fight.priced(&ground).unwrap();
    // The bot stands one tile beyond the anchor on the far side.
    let far = WorldTile {
        x: fight.escape.anchor.0 - 3,
        z: fight.escape.anchor.1,
        level: here.level,
    };
    fight.move_player(far);
    let ring = ring_of(&fight);
    let after = fight.priced(&ground).unwrap();
    assert_eq!((after.target.x, after.target.z), ring[0]);
    assert_ne!(
        after.target, before.target,
        "another side of the anchor, another first candidate"
    );
}

/// The Hitpoints line below which a fight with no food hands over, for `walk`
/// and the troll's content facts.
fn line_for(reserve: crate::combat::EscapeReserve, walk: &EscapeWalk) -> i32 {
    EscapeCost {
        hit: reserve.hit,
        interval: i32::from(reserve.interval),
        walk: i32::from(walk.ticks),
        lock: 0,
        poison_fire: 0,
        poison_fires: 0,
        poison_total: 0,
    }
    .flee_hp()
}

/// A changed route is priced before the next attack. The Hitpoints that fight
/// on over the route as it was hand over, with no attack, on the observation
/// after a wall made the walk longer; the same Hitpoints over an unchanged
/// route fight on.
#[test]
fn a_changed_route_is_priced_before_the_next_attack() {
    if !api::test_support::game_data_available() {
        return;
    }
    for changed in [false, true] {
        let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
        let here = fight.here();
        let tables = CombatTables::build(selected()).unwrap();
        let reserve = escape_reserve(&tables, fight.world.npc_type()).unwrap();
        let open = fight.priced(&Ground::around(here)).unwrap();
        let mut wall = Ground::around(here);
        wall.blocked = (here.z - 6..=here.z + 6).map(|z| (here.x + 3, z)).collect();
        let detour = fight.priced(&wall).unwrap();
        // One Hitpoint above the line of the open route, at or below the line
        // of the detour.
        let hitpoints = line_for(reserve, &open) + 1;
        assert!(
            hitpoints <= line_for(reserve, &detour),
            "the detour moves the line: {hitpoints} against {}",
            line_for(reserve, &detour)
        );

        fight.set_hitpoints(hitpoints);
        let attack = fight.first_attack();
        assert!(is_attack_on(&attack, 7), "the open route affords the fight");
        fight.accept(&attack);
        if changed {
            fight.ground = Some(wall);
        }
        fight.tick();
        if changed {
            assert_eq!(
                fight.read().outcome,
                FightOutcome::FleeRequired(FightReason::EscapeReserve),
                "the longer walk hands over before another attack"
            );
            assert!(
                fight.slot.take_guardian_action().is_none(),
                "no attack goes out on the stale bound"
            );
            assert_eq!(fight.read().phase, FightPhase::Escaping);
        } else {
            assert_eq!(fight.read().outcome, FightOutcome::Pending);
            assert_eq!(fight.read().phase, FightPhase::Fighting);
        }
    }
}

/// A walk that stops being priceable mid-fight prices zero, and the core hands
/// over before any attack: every candidate walled off after the first attack.
#[test]
fn an_unpriced_observation_hands_over_without_an_attack() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let attack = fight.first_attack();
    fight.accept(&attack);
    let mut walled = Ground::around(fight.here());
    walled.blocked = ring_of(&fight);
    fight.ground = Some(walled);
    fight.tick();
    assert_eq!(
        fight.read().outcome,
        FightOutcome::FleeRequired(FightReason::EscapeReserve)
    );
    assert_eq!(fight.read().phase, FightPhase::Escaping);
    assert!(
        fight.slot.take_guardian_action().is_none(),
        "no attack goes out on an unpriced walk"
    );
}

/// The reviewer's counterexample (Codex review of round 2): a 14 by 14 maze of
/// one-tile corridors, the rest of the build area closed. The top row is
/// z = 59, the left column x = 46. The bot stands on (46, 48), the anchor is
/// (52, 52) and the radius 7, so the Guardian's first escape tile is (59, 52).
const ROUTE_PREFIX_MAZE: [&str; 14] = [
    "######.#######", // z = 59
    "##.........###", // z = 58
    "##.#######.###", // z = 57
    ".......#...#.#", // z = 56
    "######.#.###.#", // z = 55
    "...#...#.#...#", // z = 54
    ".#.#.###.#.#.#", // z = 53
    ".#...#.#.#.#..", // z = 52
    ".#####.#.#.###", // z = 51
    "...#...#.#...#", // z = 50
    "##.#.###.###.#", // z = 49
    "...#.....#...#", // z = 48
    "####.#####.###", // z = 47
    "##.........###", // z = 46
];

fn route_prefix_maze() -> Vec<(i32, i32)> {
    ROUTE_PREFIX_MAZE
        .iter()
        .zip((46..=59).rev())
        .flat_map(|(row, z)| {
            row.bytes()
                .zip(46..)
                .filter(|&(cell, _)| cell == b'.')
                .map(move |(_, x)| (x, z))
        })
        .collect()
}

/// A walk the client cannot send in one movement packet is not priced (review
/// of round 2). In that maze the three first ring tiles are closed, and the
/// route to the first tile the client accepts, the one the Guardian clicks, is
/// 77 steps with 26 corners, 27 waypoints with its end. One packet carries 25:
/// the engine walks 74 steps and stops at (58, 54), six tiles from the anchor,
/// inside the chase area of an actor with a maxrange of 5 (`Npc.ts:652-674`).
/// The Guardian then picks another tile, (52, 59), 43 steps away: 117 steps in
/// all, more than the 116 ticks `(3 * 77 + 1) / 2` the whole route was priced
/// at. That walk has no bound, so the child hands over to Flee and attacks
/// nothing.
#[test]
fn a_walk_the_client_cannot_send_in_one_packet_is_not_priced() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (start, anchor) = ((46, 48), (52, 52));
    let mut walker = RawWalk::new();
    walker.open_only(route_prefix_maze());
    let distance = escape_distance(anchor, 7, start);
    let first = first_escape_target(anchor, start, distance, |tile| {
        walker.click(start, tile).is_some()
    })
    .expect("the client accepts a tile");
    assert_eq!(first, (59, 52), "the Guardian's first tile");

    // What the real client does with that click.
    let click = walker.click(start, first).expect("the client accepts it");
    assert_eq!(click.route.len() - 1, 77, "the steps of its route");
    assert_eq!(corners(&click.route).len(), 26, "and its corners");
    assert_eq!(click.waypoints.len(), 25, "one packet carries 25 waypoints");
    let (walked, stops) = raw_walk::walk(start, &click.waypoints);
    assert_eq!((walked, stops), (74, (58, 54)), "and the walk stops short");
    // A maxrange of 5 chases a target up to 5 + 1 tiles from the anchor.
    let reach = (stops.0 - anchor.0).abs().max((stops.1 - anchor.1).abs());
    assert!(reach <= 6, "inside the chase area: {reach} from the anchor");

    // The Guardian picks its tile again from where the walk stopped.
    let again = first_escape_target(anchor, stops, escape_distance(anchor, 7, stops), |tile| {
        walker.click(stops, tile).is_some()
    })
    .expect("a second tile");
    assert_eq!(again, (52, 59));
    let second = walker.click(stops, again).unwrap().route.len() as u32 - 1;
    assert_eq!(second, 43);
    assert!(
        walked + second > (3 * 77u32).div_ceil(2),
        "{} steps against the 116 ticks the whole route was priced at",
        walked + second
    );

    // The child prices nothing: over that scene, from that tile, with that
    // anchor, and at its admission.
    let escape = FightEscape { anchor, radius: 7 };
    let mut fight = Fight::offered_with(
        "macro_rivertrollguardian_1",
        FightEvent::RiverTroll,
        Some(escape),
    );
    fight.move_player(WorldTile {
        x: start.0,
        z: start.1,
        level: 0,
    });
    let ground = Ground::of_scene(walker.scene());
    assert_eq!(fight.priced(&ground), None, "no bound covers that walk");
    fight.ground = Some(ground);
    fight.tick();
    assert_eq!(
        fight.read().outcome,
        FightOutcome::FleeRequired(FightReason::UnknownFacts)
    );
    assert!(
        fight.slot.take_guardian_action().is_none(),
        "no attack follows a walk no bound covers"
    );
}

/// The child prices the walk the real client makes, and prices it only when
/// the client sends the whole walk in one packet: over mazes, thick and thin
/// obstacles, from random tiles around random anchors, a priced walk is the
/// route to the first tile the client accepts, the packet ends on that tile,
/// and its steps stay inside the priced ticks. When the client accepts a tile
/// whose route needs a second packet the child prices nothing, and does not
/// price a later tile instead: the Guardian clicks that first tile.
#[test]
fn the_priced_walk_is_the_walk_the_real_client_sends() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut fight = Fight::offered("macro_rivertrollguardian_1", FightEvent::RiverTroll);
    let mut walker = RawWalk::new();
    let mut rng = Prng(0x274b_0b0b_2745_2745);
    let (mut whole, mut cut, mut refused) = (0, 0, 0);
    for scene_index in 0..9 {
        let open = match scene_index % 3 {
            0 => maze(&mut rng),
            1 => scattered(&mut rng, 35),
            _ => scattered(&mut rng, 10),
        };
        walker.open_only(open.iter().copied());
        let ground = Ground::of_scene(walker.scene());
        for _ in 0..12 {
            let start = open[rng.below(open.len())];
            let beside = |rng: &mut Prng| i32::try_from(rng.below(11)).unwrap() - 5;
            let anchor = (start.0 + beside(&mut rng), start.1 + beside(&mut rng));
            fight.escape = FightEscape { anchor, radius: 7 };
            fight.move_player(WorldTile {
                x: start.0,
                z: start.1,
                level: 0,
            });
            let label = format!("scene {scene_index}, from {start:?} around {anchor:?}");
            let priced = fight.priced(&ground);
            let distance = escape_distance(anchor, 7, start);
            let accepted = first_escape_target(anchor, start, distance, |tile| {
                walker.click(start, tile).is_some()
            });
            let Some(tile) = accepted else {
                assert_eq!(priced, None, "no tile the client accepts: {label}");
                refused += 1;
                continue;
            };
            let click = walker.click(start, tile).unwrap();
            if corners(&click.route).len() < 25 {
                whole += 1;
                let walk =
                    priced.unwrap_or_else(|| panic!("a walk of one packet is priced: {label}"));
                assert_eq!(
                    walk.target,
                    WorldTile {
                        x: tile.0,
                        z: tile.1,
                        level: 0
                    },
                    "the first tile the client accepts: {label}"
                );
                assert_eq!(walk.steps as usize, click.route.len() - 1, "{label}");
                let (walked, stops) = raw_walk::walk(start, &click.waypoints);
                assert_eq!(stops, tile, "the packet ends on the tile: {label}");
                assert!(
                    walked <= u32::from(walk.ticks),
                    "{walked} steps inside {} ticks: {label}",
                    walk.ticks
                );
            } else {
                cut += 1;
                assert_eq!(priced, None, "a walk of two packets has no bound: {label}");
            }
        }
    }
    assert!(
        whole > 0 && cut > 0 && refused > 0,
        "the scenes reach each case: {whole} priced, {cut} too long, {refused} with no tile"
    );
}
