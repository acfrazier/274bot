// V11b: the Guardian's saved Fight (design V11D 3, 4 and 6).
//
// The Guardian keeps Host ownership of the event the whole time. These tests
// play the script layer's side of the port with a double that answers offers,
// reports a child's phase and outcome, and records revocations, so the
// Guardian is judged only by what it offers, publishes and sends.

/// The script layer's side of the Fight port, scripted by the test.
struct FightPort {
    knocks: u32,
    /// What the script's own claim answers for the event.
    claim: RandomClaim,
    /// `Some` refuses every offer with that reason.
    refuse: Option<FightReason>,
    offers: Vec<FightOffer>,
    child: Option<FightToken>,
    phase: FightPhase,
    outcome: FightOutcome,
    blocked_until: Option<u16>,
    revoked: Vec<(FightToken, FightCancel)>,
    released: Vec<FightToken>,
    staged: Vec<EscapeIntent>,
    sent: Option<SentEscape>,
}

impl Default for FightPort {
    fn default() -> Self {
        Self {
            knocks: 0,
            claim: RandomClaim::Host,
            refuse: None,
            offers: Vec::new(),
            child: None,
            phase: FightPhase::Offered,
            outcome: FightOutcome::Pending,
            blocked_until: None,
            revoked: Vec::new(),
            released: Vec::new(),
            staged: Vec::new(),
            sent: None,
        }
    }
}

impl RandomClaimSource for FightPort {
    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        self.knocks += 1;
        self.claim
    }

    fn runtime(&mut self) -> Option<ClaimRuntime> {
        Some(ClaimRuntime::new(0, 1))
    }

    fn fight_offer(&mut self, offer: &FightOffer) -> Result<(), FightReason> {
        self.offers.push(*offer);
        if let Some(reason) = self.refuse {
            return Err(reason);
        }
        self.child = Some(offer.token);
        self.phase = FightPhase::Offered;
        self.outcome = FightOutcome::Pending;
        Ok(())
    }

    fn fight_revoke(&mut self, token: FightToken, cancel: FightCancel) {
        self.revoked.push((token, cancel));
        let live = matches!(
            self.outcome,
            FightOutcome::Pending | FightOutcome::FleeRequired(_)
        );
        if self.child == Some(token) && live {
            self.outcome = FightOutcome::Cancelled(cancel);
            self.phase = FightPhase::Done;
        }
    }

    fn fight_read(&mut self, token: FightToken) -> Option<FightRead> {
        (self.child == Some(token)).then_some(FightRead {
            token,
            phase: self.phase,
            outcome: self.outcome,
            blocked_until: self.blocked_until,
            observed_tick: 0,
        })
    }

    fn fight_release(&mut self, token: FightToken) {
        self.released.push(token);
        if self.child == Some(token) {
            self.child = None;
        }
    }

    fn fight_stage_escape(&mut self, intent: EscapeIntent) {
        self.staged.push(intent);
    }

    fn fight_take_sent(&mut self, token: FightToken) -> Option<SentEscape> {
        self.sent.take().filter(|sent| sent.token() == token)
    }
}

/// One Guardian, one client and the double, over real content.
struct FightRig {
    c: Client,
    snap: GameSnapshot,
    g: Guardian,
    drv: FakeDriver,
    settings: ProfileSettings,
    policies: BTreeMap<String, vault::RandomEventPolicy>,
    data: Arc<api::game_data::SelectedGameData>,
    port: FightPort,
}

impl FightRig {
    /// A saved Fight for `key` and one event actor of cache type `type_id`.
    fn with_event(key: &str, name: &str, type_id: usize, saved: vault::RandomEventPolicy) -> Self {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc_typed_with_op(&mut c, 0, type_id, name, 32768, None, "Attack");
        let mut policies = BTreeMap::new();
        policies.insert(key.to_string(), saved);
        Self {
            c,
            snap: GameSnapshot::new(),
            g: Guardian::new(),
            drv: FakeDriver::default(),
            settings: ProfileSettings::default(),
            policies,
            data: event_game_data(),
            port: FightPort::default(),
        }
    }

    fn troll(saved: vault::RandomEventPolicy) -> Self {
        Self::with_event("river troll", "River troll", 391, saved)
    }

    /// One Guardian pass. A fresh pass advances the observed game tick; a
    /// repeated one polls the same observation again.
    fn pass(&mut self, fresh: bool) -> RandomStatus {
        if fresh {
            tick_at(&mut self.c, &mut self.snap);
        }
        self.g.tick_with_live_policies(
            &mut self.drv,
            &self.snap,
            &self.settings,
            &GuardianLiveContext {
                event_policies: &self.policies,
                game_data: Some(&self.data),
            },
            0,
            Some(&mut self.port as &mut dyn RandomClaimSource),
        )
    }

    /// The event actor leaves its slot, as the client's NPC list shows a removal.
    fn remove_actor(&mut self) {
        self.c.npc[0] = None;
        self.c.npc_count = 0;
    }

    /// Passes until the Guardian lets go of the event, counting them.
    fn passes_until_released(&mut self, limit: u32) -> u32 {
        for pass in 1..=limit {
            if !self.pass(true).hold {
                return pass;
            }
        }
        limit
    }
}

fn saved_fight() -> vault::RandomEventPolicy {
    vault::RandomEventPolicy::Fight
}

/// Fight is offered only for the six listed event families, for the exact
/// actor of the content type that family has. The Swarm, the angry Strange
/// plant and a world namesake never get an offer; an excluded event keeps the
/// forced Host claim and the flee it always had.
#[test]
fn fight_is_offered_only_for_the_six_listed_events_and_their_exact_actor() {
    if !data_available() {
        return;
    }
    use api::random::FightEvent::*;
    let listed = [
        ("river troll", "River troll", 391, Some(RiverTroll)),
        ("river troll", "River troll", 396, Some(RiverTroll)),
        ("rock golem", "Rock golem", 413, Some(RockGolem)),
        ("rock golem", "Rock golem", 418, Some(RockGolem)),
        ("zombie", "Zombie", 419, Some(Zombie)),
        ("shade", "Shade", 425, Some(Shade)),
        ("watchman", "Watchman", 431, Some(Watchman)),
        ("tree spirit", "Tree spirit", 438, Some(TreeSpirit)),
        ("swarm", "Swarm", 411, None),
        ("strange plant", "Strange plant", 408, None),
    ];
    for (key, name, type_id, event) in listed {
        let mut rig = FightRig::with_event(key, name, type_id, saved_fight());
        let status = rig.pass(true);
        assert_eq!(status.kind, Some(RandomKind::Evade), "{name} {type_id}");
        assert_eq!(
            status.claim,
            RandomClaim::Host,
            "{name} {type_id}: a saved Fight keeps the host in charge"
        );
        assert!(status.hold && status.ours, "{name} {type_id}");
        match event {
            Some(event) => {
                assert_eq!(rig.port.offers.len(), 1, "{name} {type_id}: one offer");
                let offer = rig.port.offers[0];
                assert_eq!(offer.event, event, "{name} {type_id}");
                assert_eq!(
                    (offer.actor.index, offer.actor.npc_type),
                    (0, u16::try_from(type_id).unwrap()),
                    "{name} {type_id}: the offer names the exact actor"
                );
                assert!(
                    rig.drv.walks.is_empty(),
                    "{name} {type_id}: no escape walk goes out while the fight is offered"
                );
            }
            None => {
                assert!(
                    rig.port.offers.is_empty(),
                    "{name} {type_id}: Fight does not apply to this event"
                );
                assert_eq!(
                    rig.drv.walks.len(),
                    1,
                    "{name} {type_id}: it flees as it always did"
                );
            }
        }
    }

    // A zombie of the world is not the event zombie: type 73 raises no event.
    let mut rig = FightRig::with_event("zombie", "Zombie", 73, saved_fight());
    let status = rig.pass(true);
    assert_eq!(status.kind, None, "a world zombie raises no event");
    assert!(rig.port.offers.is_empty());

    // An event named for one family but of another family's content type is
    // not offered either: the name alone never admits an actor.
    let mut rig = FightRig::with_event("river troll", "River troll", 413, saved_fight());
    rig.pass(true);
    assert!(
        rig.port.offers.is_empty(),
        "a golem type under the troll's name is not a fightable troll"
    );
}

/// The default and Flee policies never reach the script layer: a Fight is
/// something the operator saved for this event.
#[test]
fn flee_policy_makes_no_offer_and_keeps_the_script_claim() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(vault::RandomEventPolicy::Flee);
    rig.port.claim = RandomClaim::Handle;
    let status = rig.pass(true);
    assert!(rig.port.offers.is_empty());
    assert_eq!(status.claim, RandomClaim::Handle, "the script's claim stands");
    assert!(!status.hold && rig.drv.walks.is_empty());
}

/// One offer per episode: a repeated frame poll does not offer again, a refused
/// offer is a typed reason and a flee, and the refusal is never retried while
/// the same actor stays.
#[test]
fn one_offer_per_episode_and_a_refusal_flees_without_retrying() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.pass(false);
    rig.pass(false);
    assert_eq!(rig.port.offers.len(), 1, "repeated polls do not offer again");

    let mut rig = FightRig::troll(saved_fight());
    rig.port.refuse = Some(FightReason::NoService);
    let first = rig.pass(true);
    assert_eq!(rig.port.offers.len(), 1);
    assert_eq!(rig.drv.walks.len(), 1, "a refused offer flees at once");
    let response = first.response.expect("a response is published");
    assert_eq!(response.state, ResponseState::Fleeing);
    for _ in 0..3 {
        rig.pass(true);
    }
    assert_eq!(rig.port.offers.len(), 1, "the refusal is not retried");
}

/// While the child fights, no escape counter runs: thirty fresh ticks of fight
/// (more than any no-progress limit) release nothing and send no walk. When the
/// fight hands over to Flee, the escape counters start at the handoff, so the
/// Guardian lets go the same number of passes after it as a plain Flee episode.
#[test]
fn escape_counters_start_at_the_handoff_not_at_the_event() {
    if !data_available() {
        return;
    }
    // The control: a plain Flee episode and how long it takes to give up.
    let mut control = FightRig::troll(vault::RandomEventPolicy::Flee);
    let plain = control.passes_until_released(60);
    assert!(plain < 60, "the control gives up");

    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    for tick in 0..30 {
        let status = rig.pass(true);
        assert!(status.hold && status.ours, "tick {tick}: the fight holds the event");
        let response = status.response.expect("a response is published");
        assert_eq!(response.state, ResponseState::Fighting);
        assert_eq!(status.fight, rig.port.child, "the child's token is published");
    }
    assert!(rig.drv.walks.is_empty(), "no escape walk during the fight");

    rig.port.outcome = FightOutcome::FleeRequired(FightReason::EscapeReserve);
    rig.port.phase = FightPhase::Done;
    let handoff = rig.pass(true);
    assert_eq!(
        handoff.response.map(|response| (response.state, response.reason)),
        Some((ResponseState::Fleeing, Some(ResponseReason::EscapeReserve)))
    );
    assert!(!rig.drv.walks.is_empty(), "the walk out starts at the handoff");
    // The handoff pass is the first pass of the escape: it counts too.
    let after = 1 + rig.passes_until_released(60);
    assert!(
        after.abs_diff(plain) <= 1,
        "{after} escape passes after the fight, {plain} for a plain flee"
    );
}

/// A kill keeps the hold until the removal of the actor is observed: the
/// event does not end on the child's word alone.
#[test]
fn a_kill_holds_the_event_until_the_actors_removal_is_observed() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    rig.port.outcome = FightOutcome::Killed;
    rig.port.phase = FightPhase::Done;
    let won = rig.pass(true);
    assert!(won.hold, "the kill alone does not lift the hold");
    assert!(rig.drv.walks.is_empty(), "a kill sends no walk");
    assert_eq!(rig.port.released.len(), 1, "the child is released");
    rig.remove_actor();
    let gone = rig.pass(true);
    assert_eq!(gone.kind, None);
    assert!(!gone.hold && !gone.ours, "the removal releases the event");
}

/// Death belongs to the death system: the Guardian walks nowhere and holds
/// nothing once the child reports it.
#[test]
fn death_releases_the_event_with_no_walk_back() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    rig.port.outcome = FightOutcome::Died;
    rig.port.phase = FightPhase::Done;
    // The player respawns: the actor is gone from the scene.
    rig.remove_actor();
    let status = rig.pass(true);
    assert!(!status.hold && !status.ours, "the death system owns what follows");
    assert!(rig.drv.walks.is_empty(), "no escape walk after a death");
    assert_eq!(rig.port.released.len(), 1);
}

/// A cancellation returns the event to the escape solver for this episode and
/// never restarts the Fight: not for the same actor after a Resume, and not
/// through a second offer while the setting still says Fight.
#[test]
fn a_cancellation_latches_flee_for_the_episode() {
    if !data_available() {
        return;
    }
    for cancel in [
        FightCancel::Paused,
        FightCancel::Stopped,
        FightCancel::Replaced,
        FightCancel::CardFailed,
        FightCancel::ManualTakeover,
    ] {
        let mut rig = FightRig::troll(saved_fight());
        rig.pass(true);
        rig.port.phase = FightPhase::Fighting;
        rig.pass(true);
        assert!(rig.drv.walks.is_empty());
        rig.port.outcome = FightOutcome::Cancelled(cancel);
        rig.port.phase = FightPhase::Done;
        let status = rig.pass(true);
        assert!(status.hold, "{cancel:?}: the host still protects the account");
        assert!(!rig.drv.walks.is_empty(), "{cancel:?}: the escape solver walks out");
        for _ in 0..4 {
            rig.pass(true);
        }
        assert_eq!(rig.port.offers.len(), 1, "{cancel:?}: Fight never restarts");

        // A later episode with a new actor is offered again.
        rig.remove_actor();
        rig.pass(true);
        plant_npc_typed_with_op(&mut rig.c, 0, 391, "River troll", 32768, None, "Attack");
        rig.pass(true);
        assert_eq!(rig.port.offers.len(), 2, "{cancel:?}: the next episode fights");
    }
}

/// The saved setting changes while a fight runs: the child is revoked before
/// anything else, the claim stays with the host, and the compiled callback is
/// not called again or restored.
#[test]
fn a_live_change_to_flee_revokes_the_child_and_keeps_the_episode_with_the_host() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.port.claim = RandomClaim::Handle;
    let first = rig.pass(true);
    assert_eq!(first.claim, RandomClaim::Host, "the saved Fight wins the claim");
    assert_eq!(rig.port.knocks, 1, "the script is told once");
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);

    rig.policies
        .insert("river troll".into(), vault::RandomEventPolicy::Flee);
    let changed = rig.pass(true);
    let token = rig.port.child.or(rig.port.revoked.first().map(|(token, _)| *token));
    assert_eq!(
        rig.port.revoked,
        vec![(token.unwrap(), FightCancel::SettingChanged)],
        "the child is revoked by the change"
    );
    assert_eq!(
        changed.claim,
        RandomClaim::Host,
        "the episode keeps its Host owner: the discarded Handle is never restored"
    );
    for _ in 0..3 {
        let status = rig.pass(true);
        assert_eq!(status.claim, RandomClaim::Host);
    }
    assert_eq!(rig.port.knocks, 1, "no second callback for this episode");
    assert_eq!(rig.port.offers.len(), 1, "no new Fight after the change");
}

/// Handling off revokes the child and publishes nothing the host would act on.
#[test]
fn handling_off_revokes_the_child_and_holds_nothing() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    let sent_before = rig.drv.walks.len() + rig.drv.menus.len();
    rig.settings.random_events = false;
    let off = rig.pass(true);
    assert_eq!(rig.port.revoked.len(), 1);
    assert_eq!(rig.port.revoked[0].1, FightCancel::HandlingOff);
    assert!(!off.hold && !off.ours && !off.toggle);
    assert_eq!(
        off.response.map(|response| response.state),
        Some(ResponseState::HandlingOff)
    );
    assert_eq!(
        rig.drv.walks.len() + rig.drv.menus.len(),
        sent_before,
        "nothing is sent while handling is off"
    );
}

/// A child that is gone (its session or slot ended) is a flee, not a wait.
#[test]
fn a_child_the_slot_no_longer_holds_falls_back_to_flee() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.child = None;
    let status = rig.pass(true);
    assert!(status.hold, "the host protects the account");
    assert!(!rig.drv.walks.is_empty(), "the escape solver takes over");
    assert_eq!(rig.port.offers.len(), 1, "and never offers this actor again");
}

/// Design 3.3: a trap that replaces the owner while a fight runs revokes and
/// releases the child in the pass it replaces the family, before the status
/// publishes (the status carries no token), so no child outlives the Evade
/// run that offered it and none fights on unobserved.
#[test]
fn a_trap_that_replaces_a_fighting_event_revokes_and_releases_the_child() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    let fighting = rig.pass(true);
    let token = rig.port.child.expect("the child fights");
    assert_eq!(fighting.fight, Some(token), "the gate admits the child");

    // The player is carried onto the maze square: the troll leaves the list.
    rig.remove_actor();
    plant_player(&mut rig.c, "Test", 2891, 4555);
    let trap = rig.pass(true);
    assert_eq!(trap.kind, Some(RandomKind::Maze), "the trap is not masked");
    assert_eq!(trap.fight, None, "the gate is shut the pass the family leaves");
    assert_eq!(
        rig.port.revoked,
        vec![(token, FightCancel::Replaced)],
        "the orphaned child is revoked in that pass"
    );
    assert_eq!(rig.port.released, vec![token], "and released");

    rig.pass(true);
    rig.pass(true);
    assert_eq!(rig.port.revoked.len(), 1, "no second revocation");
    assert_eq!(rig.port.released.len(), 1, "no second release");
}

/// Fight never restarts for an actor in its episode (design 4.4): the troll
/// stays between two swings while a trap carries the player away, its child is
/// revoked with the owner, and the retained troll is the event again when the
/// trap ends. It is fled from, not offered a second Fight.
#[test]
fn a_retained_actor_whose_fight_a_trap_revoked_is_not_fought_again() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    let token = rig.port.child.expect("the child fights");

    // The troll keeps its combat state between swings while the trap wins.
    let cycle = rig.c.loop_cycle;
    let troll = rig.c.npc[0].as_mut().expect("the troll stands");
    troll.entity.face_entity = -1;
    troll.entity.combat_cycle = cycle + 300;
    plant_player(&mut rig.c, "Test", 2891, 4555);
    let trap = rig.pass(true);
    assert_eq!(trap.kind, Some(RandomKind::Maze));
    assert_eq!(rig.port.revoked, vec![(token, FightCancel::Replaced)]);

    // The trap ends and the player is back by the troll.
    plant_player(&mut rig.c, "Test", 0, 0);
    let walks = rig.drv.walks.len();
    let back = rig.pass(true);
    assert_eq!(back.kind, Some(RandomKind::Evade), "the retained troll returns");
    assert_eq!(rig.port.offers.len(), 1, "Fight is not offered again");
    assert_eq!(back.fight, None);
    assert!(
        rig.drv.walks.len() > walks,
        "the escape solver works the returned actor"
    );
}

/// R3-2: the walk out of a fight that handed over is the observation's to send
/// through the child's upkeep guard. The Guardian stages it once, sends no raw
/// walk while the child serves it, and folds the sent walk on its next tick.
#[test]
fn the_walk_out_of_a_handed_over_fight_goes_through_the_staged_intent() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    rig.port.outcome = FightOutcome::FleeRequired(FightReason::EscapeReserve);
    rig.port.phase = FightPhase::Escaping;
    let handoff = rig.pass(true);
    assert!(handoff.hold);
    assert!(
        rig.drv.walks.is_empty(),
        "the Guardian sends no raw walk while the child's guard serves the escape"
    );
    assert_eq!(rig.port.staged.len(), 1, "one intent is staged");
    // The same tick, polled again, stages nothing more.
    rig.pass(false);
    assert_eq!(rig.port.staged.len(), 1);

    // The observation sends it once no lock blocks the tick.
    let intent = rig.port.staged.remove(0);
    assert_eq!(Some(intent.token()), rig.port.child);
    let sent = intent.send(&mut rig.drv, &rig.snap);
    assert!(sent.fled(), "the walk out is accepted");
    assert_eq!(rig.drv.walks.len(), 1, "the intent is what walks");
    rig.port.sent = Some(sent);
    rig.pass(true);
    assert_eq!(
        rig.port.staged.len(),
        1,
        "the sent walk is folded and the next one is staged"
    );
    assert_eq!(rig.drv.walks.len(), 1, "still no raw walk");
}

/// R3-2: after a Pause or Stop revokes the child during a bite that locks
/// input, the Guardian's own escape continues, but not on a tick the admitted
/// lock still blocks: the lock end the child published before its guard was
/// dropped gates the first walk.
#[test]
fn a_revocation_during_an_input_lock_delays_the_first_walk_to_the_lock_end() {
    if !data_available() {
        return;
    }
    for cancel in [FightCancel::Paused, FightCancel::Stopped] {
        let mut rig = FightRig::troll(saved_fight());
        rig.pass(true);
        rig.port.phase = FightPhase::Fighting;
        rig.pass(true);
        // A karambwan bite locks input for three more ticks.
        let tick = rig.snap.tick() as u16;
        rig.port.blocked_until = Some(tick.wrapping_add(3));
        rig.port.outcome = FightOutcome::Cancelled(cancel);
        rig.port.phase = FightPhase::Done;
        let mut walked_at = None;
        for step in 1..=6u16 {
            let status = rig.pass(true);
            assert!(status.hold, "{cancel:?}: the host holds the event");
            if walked_at.is_none() && !rig.drv.walks.is_empty() {
                walked_at = Some(step);
            }
        }
        assert_eq!(
            walked_at,
            Some(3),
            "{cancel:?}: no click goes out while the admitted lock runs, the first walk follows it"
        );
    }
}

/// A walk that already went out is not lost to the lock gate: with no lock the
/// first walk goes out on the handoff pass itself.
#[test]
fn without_a_lock_the_first_walk_goes_out_on_the_handoff_pass() {
    if !data_available() {
        return;
    }
    let mut rig = FightRig::troll(saved_fight());
    rig.pass(true);
    rig.port.phase = FightPhase::Fighting;
    rig.pass(true);
    rig.port.outcome = FightOutcome::Cancelled(FightCancel::Paused);
    rig.port.phase = FightPhase::Done;
    rig.pass(true);
    assert_eq!(rig.drv.walks.len(), 1);
}
