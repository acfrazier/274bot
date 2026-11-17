// V11a: one owner per Guardian event.
//
// The Guardian is replayed through traces of the checked ownership model
// (`tools/seat-model/src/guardian.rs`). After every step the model and the real
// Guardian must publish the same `claim`, `handling`, `acting`, `ours` and
// `hold`, and the invariants of the model hold over the real observations: the
// publications equal their derivation, a script that stops, pauses or fails
// returns its claim within one tick, a claim names the runtime that made it,
// the active event belongs to the current session, the toggle off publishes
// nothing, and the host emits nothing while a script owns the event.
//
// The model's `Seat` steps compose the published hold with the real gameplay
// seat. Host cannot reach the seat (it lives in `script`), so that replay is
// host-play's `guardian_seat_conformance_tests.rs`, which drives this Guardian
// through the real claim adapter and slot.
use seat_model::guardian::{
    derive as model_derive, Family, FightResult, GuardianBounds, GuardianMutation, GuardianState,
    GuardianStep, PublicationInputs as ModelInputs, ScriptExit,
};
use std::cell::Cell;
use std::collections::{HashSet, VecDeque};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Run {
    Stopped,
    Paused,
    Running,
}

/// The claim port of host-play's `ScriptRandomClaims` over a slot, reduced to
/// the script states that matter (`Slot::current_random_claim`, `Slot::on_random`
/// in `crates/script/src/slot.rs`). `live` is a Load script with cached ignore
/// membership; otherwise a compiled script answers only the rising edge.
///
/// Each double is one script runtime: replacing it (a new Start, a Stop, a
/// failure, a session replacement, or a test that swaps in another script)
/// is a new runtime identity, as the slot's runtime generation is. Pause and
/// Resume change the double in place and keep the runtime.
struct Claims {
    run: Run,
    wants: bool,
    live: bool,
    callbacks: u32,
    fight: FightSlot,
}

/// The Fight child the slot holds for the Guardian, reduced to what the
/// Guardian reads (`FightService` over the slot's `GuardianChild`): it stands
/// between an offer and the Guardian's release, and outlives a script runtime
/// (a card's Pause or Stop revokes it; the Guardian folds and releases it).
#[derive(Default)]
struct FightSlot {
    child: Option<(FightToken, FightOutcome)>,
    offers: u32,
}

thread_local! {
    /// The runtime generation of the current claim double on this test thread.
    static CLAIM_RUNTIME: Cell<u64> = const { Cell::new(0) };
}

impl Drop for Claims {
    /// The runtime ends with its double.
    fn drop(&mut self) {
        CLAIM_RUNTIME.with(|runtime| runtime.set(runtime.get() + 1));
    }
}

impl Claims {
    fn answer(&self) -> RandomClaim {
        if self.run == Run::Running && self.wants {
            RandomClaim::Handle
        } else {
            RandomClaim::Host
        }
    }

    fn current_runtime() -> ClaimRuntime {
        ClaimRuntime::new(0, CLAIM_RUNTIME.with(Cell::get))
    }

    /// The slot's cancellation of the child it holds (a card's Pause, Stop or
    /// failure). The Guardian folds and releases it on its next fresh pass.
    fn revoke_child(&mut self, cancel: FightCancel) {
        if let Some((token, _)) = self.fight.child {
            self.fight_revoke(token, cancel);
        }
    }
}

impl RandomClaimSource for Claims {
    fn on_random(&mut self, _event: &DetectedRandom) -> RandomClaim {
        if self.run == Run::Running && !self.live {
            self.callbacks += 1;
        }
        self.answer()
    }

    fn current_claim(&mut self, _event: &DetectedRandom) -> Option<RandomClaim> {
        if self.run != Run::Running {
            Some(RandomClaim::Host)
        } else if self.live {
            Some(self.answer())
        } else {
            None
        }
    }

    fn runtime(&mut self) -> Option<ClaimRuntime> {
        Some(Self::current_runtime())
    }

    fn fight_offer(&mut self, offer: &FightOffer) -> Result<(), FightReason> {
        self.fight.offers += 1;
        self.fight.child = Some((offer.token, FightOutcome::Pending));
        Ok(())
    }

    /// A revocation records the cancellation of a child that still runs; a
    /// result the child already holds stands.
    fn fight_revoke(&mut self, token: FightToken, cancel: FightCancel) {
        if let Some((held, outcome)) = self.fight.child.as_mut() {
            if *held == token && *outcome == FightOutcome::Pending {
                *outcome = FightOutcome::Cancelled(cancel);
            }
        }
    }

    fn fight_read(&mut self, token: FightToken) -> Option<FightRead> {
        let (held, outcome) = self.fight.child?;
        (held == token).then_some(FightRead {
            token,
            phase: if outcome == FightOutcome::Pending {
                FightPhase::Fighting
            } else {
                FightPhase::Done
            },
            outcome,
            blocked_until: None,
            observed_tick: 0,
        })
    }

    fn fight_release(&mut self, token: FightToken) {
        if self.fight.child.is_some_and(|(held, _)| held == token) {
            self.fight.child = None;
        }
    }
}

struct Rig {
    c: Client,
    snap: GameSnapshot,
    g: Guardian,
    drv: FakeDriver,
    settings: ProfileSettings,
    policies: BTreeMap<String, vault::RandomEventPolicy>,
    claims: Claims,
    data: Option<Arc<api::game_data::SelectedGameData>>,
    last: RandomStatus,
    /// `Guardian::acting` as the last published pass left it.
    last_acting: bool,
    sigs: u32,
    last_sig: Option<String>,
    emissions: usize,
    /// The Guardian is new and has not published: the old status stands.
    unpublished: bool,
    /// The fixture planted something the reset has to take away again.
    dirty: bool,
    /// The slot session the current Guardian was made for.
    session: u32,
    /// The event slot of the hostile actor the model retains.
    retained: Option<usize>,
    /// The saved policy of the listed events is Flee now.
    policy_flee: bool,
}

impl Rig {
    /// Back to the start state for the next trace on the same client.
    fn reset(&mut self, fx: &dyn Fixture) {
        self.reset_pair(fx, fx);
    }

    /// Back to the start state after a trace that may have replaced the event
    /// `first` raised by the one `second` raises. A `second` that needs
    /// detector history gets it before the trace, then `first` is prepared.
    fn reset_pair(&mut self, first: &dyn Fixture, second: &dyn Fixture) {
        if self.dirty {
            first.remove(self);
            second.remove(self);
        }
        first.chat_close(self);
        second.chat_close(self);
        self.c.chat_text.fill(String::new());
        self.c.chat_username.fill(String::new());
        self.c.chat_type.fill(0);
        // A new client: no message has arrived yet.
        self.c.chat_seq = 0;
        clear_npcs(&mut self.c);
        plant_player(&mut self.c, "Test", 0, 0);
        self.snap = GameSnapshot::new();
        self.session = 0;
        self.retained = None;
        self.policy_flee = false;
        self.g = Guardian::new();
        self.drv = FakeDriver::default();
        self.settings = ProfileSettings::default();
        self.policies.clear();
        self.start_runtime(Run::Stopped, false, false);
        self.claims.callbacks = 0;
        self.claims.fight = FightSlot::default();
        self.last = RandomStatus::default();
        self.last_acting = false;
        self.sigs = 0;
        self.last_sig = None;
        self.emissions = 0;
        self.unpublished = false;
        self.dirty = false;
        if second.name() != first.name() {
            second.prepare_second(self);
        }
        first.prepare(self);
    }

    /// A new script runtime answers the claim port from now on. The callback
    /// count is the trace's, so it carries over.
    fn start_runtime(&mut self, run: Run, wants: bool, live: bool) {
        let callbacks = self.claims.callbacks;
        let fight = std::mem::take(&mut self.claims.fight);
        self.claims = Claims {
            run,
            wants,
            live,
            callbacks,
            fight,
        };
    }
}

impl Rig {
    fn new(data: Option<Arc<api::game_data::SelectedGameData>>) -> Self {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        Self {
            c,
            snap: GameSnapshot::new(),
            g: Guardian::new(),
            drv: FakeDriver::default(),
            settings: ProfileSettings::default(),
            policies: BTreeMap::new(),
            claims: Claims {
                run: Run::Stopped,
                wants: false,
                live: false,
                callbacks: 0,
                fight: FightSlot::default(),
            },
            data,
            last: RandomStatus::default(),
            last_acting: false,
            sigs: 0,
            last_sig: None,
            emissions: 0,
            unpublished: false,
            dirty: false,
            session: 0,
            retained: None,
            policy_flee: false,
        }
    }

    fn sent(&self) -> usize {
        self.drv.menus.len() + self.drv.walks.len() + self.drv.actions.len()
    }

    /// One Guardian pass. A fresh pass advances the observed game tick.
    fn pass(&mut self, fresh: bool) -> RandomStatus {
        if fresh {
            tick_at(&mut self.c, &mut self.snap);
        }
        let before = self.sent();
        let status = self.g.tick_with_live_policies(
            &mut self.drv,
            &self.snap,
            &self.settings,
            &GuardianLiveContext {
                event_policies: &self.policies,
                game_data: self.data.as_deref(),
            },
            0,
            Some(&mut self.claims as &mut dyn RandomClaimSource),
        );
        self.emissions = self.sent() - before;
        self.unpublished = false;
        if self.g.sig != self.last_sig {
            self.last_sig = self.g.sig.clone();
            if self.last_sig.is_some() {
                self.sigs += 1;
            }
        }
        self.last = status.clone();
        self.last_acting = self.g.acting();
        status
    }

    /// A pass nobody publishes: a Guardian that starts new needs its detector
    /// history (a held tool, held fishing gear) before the world changes. The
    /// published status and the bookkeeping stay as they were.
    fn history_pass(&mut self) {
        let kept = (
            self.last.clone(),
            self.last_acting,
            self.emissions,
            self.sigs,
            self.last_sig.clone(),
            self.unpublished,
        );
        self.pass(true);
        (
            self.last,
            self.last_acting,
            self.emissions,
            self.sigs,
            self.last_sig,
            self.unpublished,
        ) = kept;
    }
}

trait Fixture {
    fn name(&self) -> &'static str;
    fn family(&self) -> Family;
    /// Host passes the model may spend before the family stalls or releases.
    fn bound(&self) -> u8;
    fn needs_data(&self) -> bool {
        false
    }
    /// Two different signatures: an episode can be replaced by another.
    fn distinct_signatures(&self) -> bool {
        true
    }
    fn has_forced_host(&self) -> bool {
        false
    }
    /// The fixture is the Ent guard: host work with no detected event, raised
    /// by `EntAppear` and ended by `EntRemove` instead of `Appear`.
    fn ent_only(&self) -> bool {
        false
    }
    /// The model's fight profile: the native child is a model state, and the
    /// saved policy binds a hostile event at the first pass of its episode.
    fn fight_profile(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, id: u8, forced_host: bool);
    /// Raise the event of `family` (a fixture of the fight profile raises an
    /// actor of the listed events for `Fight` and one Fight does not apply to
    /// for `Evade`).
    fn install_as(&self, rig: &mut Rig, id: u8, forced_host: bool, _family: Family) {
        self.install(rig, id, forced_host);
    }
    fn remove(&self, rig: &mut Rig);
    fn chat_close(&self, _rig: &mut Rig) {}
    /// The host's work for the event has just started.
    fn started(&self, _rig: &mut Rig) {}
    /// The start state of one trace on a new or reset client.
    fn prepare(&self, _rig: &mut Rig) {}
    /// The start state of a trace in which this family replaces another: the
    /// detector history it needs, planted so the first family's event does
    /// not destroy it.
    fn prepare_second(&self, _rig: &mut Rig) {}
    /// A new Guardian replaced the old one: give it the history it needs
    /// again, and bring the world back to what the model says it is.
    fn rebaseline(&self, _rig: &mut Rig, _present: bool) {}
    fn replicate(&self, _rig: &mut Rig, _fresh: bool) {}
}

fn data_available() -> bool {
    api::test_support::game_data_available()
}

fn real_data() -> Option<Arc<api::game_data::SelectedGameData>> {
    data_available().then(event_game_data)
}

/// Remove the NPCs in `slots` and keep every other one, as the client's NPC
/// list does when those slots are removed.
fn remove_npcs(c: &mut Client, slots: &[usize]) {
    for &slot in slots {
        if let Some(npc) = c.npc.get_mut(slot) {
            *npc = None;
        }
    }
    let count = c.npc_count as usize;
    let mut kept = 0;
    for index in 0..count {
        let id = c.npc_ids[index];
        if !slots.contains(&(id as usize)) {
            c.npc_ids[kept] = id;
            kept += 1;
        }
    }
    c.npc_count = kept as _;
}

/// Plant an NPC in `slot` alone: whatever stood there is removed first.
fn put_npc(c: &mut Client, slot: usize, plant: impl FnOnce(&mut Client)) {
    remove_npcs(c, &[slot]);
    plant(c);
}

/// The slots the NPC families use for their two episodes. The Ent guard and
/// the fishing spot of the gear family stand in their own slots.
const EVENT_SLOTS: [usize; 2] = [0, 1];
const ENT_SLOT: usize = 8;
const FISHING_SLOT: usize = 9;

/// Remove the event-slot NPCs named `names` and keep every other one: a
/// retained hostile actor in the other event slot stays when an overlapping
/// event ends.
fn remove_named(c: &mut Client, names: &[&str]) {
    let slots: Vec<usize> = EVENT_SLOTS
        .into_iter()
        .filter(|&slot| {
            c.npc
                .get(slot)
                .and_then(Option::as_ref)
                .and_then(|npc| npc.r#type)
                .and_then(|id| c.cache.npcs.get(id))
                .is_some_and(|npc| names.contains(&npc.name.as_str()))
        })
        .collect();
    remove_npcs(c, &slots);
}

/// The hostile actor between two swings: it no longer faces the player but
/// keeps its combat state (the inter-swing snapshot of
/// `random_tests.rs`'s chase regression). The raw detector does not select
/// it, and the Guardian keeps it as an engaged, retained actor. The test
/// client never advances `loop_cycle`, so the combat state lasts the trace.
fn between_swings(rig: &mut Rig, slot: usize) {
    let cycle = rig.c.loop_cycle;
    let npc = rig.c.npc[slot].as_mut().expect("the retained actor");
    npc.entity.face_entity = -1;
    npc.entity.combat_cycle = cycle + 300;
}

// --- The families -----------------------------------------------------------

struct DialogFx;
impl Fixture for DialogFx {
    fn name(&self) -> &'static str {
        "dialog"
    }
    fn family(&self) -> Family {
        Family::Dialog
    }
    fn bound(&self) -> u8 {
        // Talk-to, then `MAX_CONTINUES` continues, then one clearing tick.
        (MAX_CONTINUES + 1) as u8
    }
    fn install(&self, rig: &mut Rig, id: u8, _forced: bool) {
        let name = ["Genie", "Mysterious old man"][usize::from(id)];
        let slot = usize::from(id);
        put_npc(&mut rig.c, slot, |c| {
            plant_npc(c, slot, name, -1, Some("Greetings Test!"));
        });
    }
    fn started(&self, rig: &mut Rig) {
        // The server opens the chat after the Talk-to.
        open_chat(&mut rig.c);
    }
    fn remove(&self, rig: &mut Rig) {
        remove_named(&mut rig.c, &["Genie", "Mysterious old man"]);
    }
    fn chat_close(&self, rig: &mut Rig) {
        close_chat(&mut rig.c);
    }
}

struct BoxFx;
impl Fixture for BoxFx {
    fn name(&self) -> &'static str {
        "box"
    }
    fn family(&self) -> Family {
        // `is_trapped`: a held Strange box holds the slot like a trap square
        // until its bounded attempts end.
        Family::Trap
    }
    fn bound(&self) -> u8 {
        // Open at tick 1, three attempts of four ticks, released at tick 12.
        11
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_inv_box(&mut rig.c, 1);
    }
    fn remove(&self, rig: &mut Rig) {
        clear_inv(&mut rig.c);
    }
    fn replicate(&self, rig: &mut Rig, fresh: bool) {
        if fresh {
            rig.c.add_chat(
                0,
                "Oh dear, you have taken so long that the cube has replicated.",
                "",
            );
        } else {
            rig.c.add_chat(
                2,
                "Oh dear, you have taken so long that the cube has replicated.",
                "Other",
            );
        }
    }
}

/// River troll / Rock golem under the default Flee policy; Swarm / the angry
/// plant under a saved Fight, which the base keeps with the host.
struct EvadeFx;
impl Fixture for EvadeFx {
    fn name(&self) -> &'static str {
        "evade"
    }
    fn family(&self) -> Family {
        Family::Evade
    }
    fn bound(&self) -> u8 {
        // The shortest give-up is nine fresh ticks: the host works on nine
        // passes and lets go on the tenth.
        9
    }
    fn needs_data(&self) -> bool {
        true
    }
    // A second attacker inherits the flee without a release tick: the model's
    // `Evade` family hands the pending act on, and its hold with it.
    fn has_forced_host(&self) -> bool {
        true
    }
    fn install(&self, rig: &mut Rig, id: u8, forced_host: bool) {
        let name = match (forced_host, id) {
            (false, 0) => "River troll",
            (false, _) => "Rock golem",
            (true, 0) => "Swarm",
            (true, _) => "Strange plant",
        };
        if forced_host {
            rig.policies
                .insert(name.to_ascii_lowercase(), vault::RandomEventPolicy::Fight);
        }
        let slot = usize::from(id);
        put_npc(&mut rig.c, slot, |c| {
            plant_npc_with_op(c, slot, name, 32768, None, "Attack");
        });
    }
    /// The actor is observed gone: its slot now shows another NPC. A removal
    /// while the player stands beyond the view edge is not despawn evidence,
    /// and the Guardian keeps that actor for the content's cleanup interval
    /// (`a_retained_hostile_actor_resumes_after_an_overlapping_trap_ends`).
    fn remove(&self, rig: &mut Rig) {
        for slot in EVENT_SLOTS {
            if rig.c.npc.get(slot).is_some_and(Option::is_some) {
                put_npc(&mut rig.c, slot, |c| plant_npc(c, slot, "Man", -1, None));
            }
        }
    }
}

/// The saved Fight for a listed event (design V11D 3, 4): a River troll for
/// the first episode and a Rock golem for the next, both under the saved
/// Fight. The model's `Fight` family; the native child is the double the
/// claim port holds (`Claims::fight`).
struct FightFx;
impl Fixture for FightFx {
    fn name(&self) -> &'static str {
        "fight"
    }
    fn family(&self) -> Family {
        Family::Fight
    }
    fn bound(&self) -> u8 {
        // The flee that follows a handoff has the escape budget of an Evade.
        EvadeFx.bound()
    }
    fn needs_data(&self) -> bool {
        true
    }
    fn fight_profile(&self) -> bool {
        true
    }
    fn install(&self, rig: &mut Rig, id: u8, forced_host: bool) {
        self.install_as(rig, id, forced_host, Family::Fight);
    }
    /// An actor of the listed events, or Swarm / the angry plant, which Fight
    /// does not apply to. The saved policy of both is the rig's.
    fn install_as(&self, rig: &mut Rig, id: u8, _forced_host: bool, family: Family) {
        let (key, name) = match (family, id) {
            (Family::Fight, 0) => ("river troll", "River troll"),
            (Family::Fight, _) => ("rock golem", "Rock golem"),
            (_, 0) => ("swarm", "Swarm"),
            (_, _) => ("strange plant", "Strange plant"),
        };
        let saved = if rig.policy_flee {
            vault::RandomEventPolicy::Flee
        } else {
            vault::RandomEventPolicy::Fight
        };
        rig.policies.insert(key.to_string(), saved);
        let slot = usize::from(id);
        put_npc(&mut rig.c, slot, |c| {
            plant_npc_with_op(c, slot, name, 32768, None, "Attack");
        });
    }
    fn remove(&self, rig: &mut Rig) {
        EvadeFx.remove(rig);
    }
}

struct MazeFx;
impl Fixture for MazeFx {
    fn name(&self) -> &'static str {
        "maze"
    }
    fn family(&self) -> Family {
        // The teleport carries the player out of a hostile actor's chase range.
        Family::Teleport
    }
    fn bound(&self) -> u8 {
        maze::SOLVE_TICKS as u8
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_player(&mut rig.c, "Test", 2891, 4555);
    }
    fn remove(&self, rig: &mut Rig) {
        plant_player(&mut rig.c, "Test", 3222, 3222);
    }
}

struct MimeFx;
impl Fixture for MimeFx {
    fn name(&self) -> &'static str {
        "mime"
    }
    fn family(&self) -> Family {
        Family::Teleport
    }
    fn bound(&self) -> u8 {
        // The mime stage has no solver budget: the hold ends off the square.
        100
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, id: u8, _forced: bool) {
        plant_player(&mut rig.c, "Test", 31 * 64, 74 * 64);
        let slot = usize::from(id);
        put_npc(&mut rig.c, slot, |c| plant_npc(c, slot, "Mime", -1, None));
    }
    fn remove(&self, rig: &mut Rig) {
        remove_named(&mut rig.c, &["Mime"]);
        plant_player(&mut rig.c, "Test", 3222, 3222);
    }
}

struct LampFx;
impl Fixture for LampFx {
    fn name(&self) -> &'static str {
        "lamp"
    }
    fn family(&self) -> Family {
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // Rub, then nine ticks for the skill interface before it pauses; the
        // pause ends by its own countdown, which the model has no step for.
        (MAX_LAMP_WAIT + 1) as u8
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_inv_lamp(&mut rig.c);
    }
    fn remove(&self, rig: &mut Rig) {
        set_lamps(&mut rig.c, 0);
    }
}

struct HazardFx;
impl Fixture for HazardFx {
    fn name(&self) -> &'static str {
        "hazard"
    }
    fn family(&self) -> Family {
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // The host steps off until no hazard is within two tiles.
        100
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn prepare(&self, rig: &mut Rig) {
        plant_player(&mut rig.c, "Test", 10, 10);
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_loc(&mut rig.c, 510, "Smoking rock", 0, 0);
        plant_player(&mut rig.c, "Test", 0, 0);
    }
    fn remove(&self, rig: &mut Rig) {
        rig.c.world.del_wall(0, 0, 0);
        plant_player(&mut rig.c, "Test", 10, 10);
    }
}

/// A strange plant that is the player's own (its overhead text names the
/// player): the hard-evidence Pick, which walks to range and picks.
struct PickFx;
impl Fixture for PickFx {
    fn name(&self) -> &'static str {
        "pick"
    }
    fn family(&self) -> Family {
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // The pick has no solver budget: it ends when the plant leaves.
        100
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, id: u8, _forced: bool) {
        let slot = usize::from(id);
        put_npc(&mut rig.c, slot, |c| {
            plant_npc_with_op(c, slot, "Strange plant", -1, Some("Pick Test!"), "Pick");
        });
    }
    fn remove(&self, rig: &mut Rig) {
        remove_named(&mut rig.c, &["Strange plant"]);
    }
}

/// Fishing gear held beside a fishing spot, then lying on the ground: the loss
/// is proved by the detector's history, so every new Guardian first sees the
/// gear held.
struct LostGearFx;
impl LostGearFx {
    fn held(rig: &mut Rig) {
        put_npc(&mut rig.c, FISHING_SLOT, |c| {
            plant_npc(c, FISHING_SLOT, "Fishing spot", -1, None);
        });
        rig.c.ground_obj[0][3][0] = None;
        plant_inv_named(&mut rig.c, 311, Some("Harpoon"));
    }
}
impl Fixture for LostGearFx {
    fn name(&self) -> &'static str {
        "lost gear"
    }
    fn family(&self) -> Family {
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // The take has no solver budget: it ends when the gear is gone.
        100
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn prepare(&self, rig: &mut Rig) {
        Self::held(rig);
        rig.history_pass();
    }
    fn prepare_second(&self, rig: &mut Rig) {
        self.prepare(rig);
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        drop_harpoon_to_ground(&mut rig.c);
    }
    fn remove(&self, rig: &mut Rig) {
        Self::held(rig);
    }
    fn rebaseline(&self, rig: &mut Rig, present: bool) {
        Self::held(rig);
        rig.history_pass();
        if present {
            self.install(rig, 0, false);
        }
    }
}

/// A tool held, then its handle: the content refits it within a bounded
/// number of ticks. Like the gear, the loss needs the detector's history.
struct LostToolFx;
impl Fixture for LostToolFx {
    fn name(&self) -> &'static str {
        "lost tool"
    }
    fn family(&self) -> Family {
        // The head lands within the content's spawn window, which runs from
        // the loss whoever owns the event and whatever the toggle says.
        Family::Window
    }
    fn bound(&self) -> u8 {
        // The refit gives up after its three-tick spawn grace: the host works
        // on three passes and lets go on the fourth.
        3
    }
    fn needs_data(&self) -> bool {
        true
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn prepare(&self, rig: &mut Rig) {
        plant_inv_named(&mut rig.c, 1351, Some("Bronze axe"));
        rig.history_pass();
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_inv_named(&mut rig.c, 492, Some("Axe handle"));
    }
    fn remove(&self, rig: &mut Rig) {
        plant_inv_named(&mut rig.c, 1351, Some("Bronze axe"));
    }
    fn rebaseline(&self, rig: &mut Rig, present: bool) {
        self.remove(rig);
        rig.history_pass();
        if present {
            self.install(rig, 0, false);
        }
    }
}

/// An Ent the player chops, in its own slot. Its targeting is the content's
/// `p_opnpc(1)` (`macro_event_ent.rs2:20`), seen as the player's face target.
fn plant_ent(rig: &mut Rig) {
    put_npc(&mut rig.c, ENT_SLOT, |c| {
        plant_npc_typed_with_op(c, ENT_SLOT, 448, "Oak", -1, None, "Chop down");
    });
    rig.c
        .local_player
        .as_mut()
        .expect("local player")
        .entity
        .face_entity = ENT_SLOT as i32;
}

/// The Ent guard: host work for a forced Ent, with no detected event, so no
/// claim callback and no script owner.
struct EntFx;
impl Fixture for EntFx {
    fn name(&self) -> &'static str {
        "ent"
    }
    fn family(&self) -> Family {
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // The content gives the Ent 60 ticks; traces stay below that.
        100
    }
    fn ent_only(&self) -> bool {
        true
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_ent(rig);
    }
    fn remove(&self, rig: &mut Rig) {
        remove_npcs(&mut rig.c, &[ENT_SLOT]);
    }
}

fn fixtures() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(DialogFx),
        Box::new(BoxFx),
        Box::new(EvadeFx),
        Box::new(MazeFx),
        Box::new(MimeFx),
        Box::new(LampFx),
        Box::new(HazardFx),
        Box::new(PickFx),
        Box::new(LostGearFx),
        Box::new(LostToolFx),
        Box::new(EntFx),
    ]
}

// --- The replay -------------------------------------------------------------

/// The real Ent guard holds for the content's 60 ticks
/// (`macro_event_ent.rs2:18`); the model bounds it alike.
const ENT_HOLD_TICKS: u8 = 60;

fn bounds_for(fx: &dyn Fixture) -> GuardianBounds {
    GuardianBounds {
        wait: fx.bound(),
        ids: 2,
        sessions: 2,
        ent: false,
        window: false,
        retain: false,
        ent_hold: ENT_HOLD_TICKS,
        fight: fx.fight_profile(),
    }
}

fn model_for(fx: &dyn Fixture) -> GuardianState {
    GuardianState::new(bounds_for(fx), GuardianMutation::None)
}

/// Every server line a family settles a pending operation on: the wrong-talk
/// rejection, the plant's two answers, the refit rejection, the stun, and the
/// box's two answer outcomes.
const RESULT_LINES: [&str; 7] = [
    "It's not here for you.",
    "You are trying to talk to someone else's event.",
    "The fruit isn't ready to be picked yet",
    "Nothing interesting happens.",
    "You've been stunned!",
    "You have completed this cube, but there are more for you to do.",
    "Oh dear, you have answered the question incorrectly",
];

/// A late result: every settling line, carrying a token (chat sequence) at or
/// below the cutoff of the operation in flight. It arrives with the next chat
/// update, as a late packet does, and must settle nothing.
fn deliver_late_results(rig: &mut Rig) {
    let g = &rig.g;
    let mut cutoff = g.chat_seen;
    if let Some(after) = g.tool_run().and_then(|run| run.refit_after_sequence) {
        cutoff = cutoff.min(after);
    }
    if let BoxPhase::Answering { sequence, .. } = g.box_solve().phase {
        cutoff = cutoff.min(sequence);
    }
    let head = i64::try_from(rig.c.chat_seq).expect("chat sequence");
    let dense = rig
        .c
        .chat_text
        .iter()
        .take_while(|text| !text.is_empty())
        .count();
    let first = usize::try_from(head - i64::from(cutoff))
        .unwrap_or(0)
        .max(dense);
    assert!(
        first + RESULT_LINES.len() <= rig.c.chat_text.len(),
        "chat ring room"
    );
    for index in dense..first {
        rig.c.chat_text[index] = "Welcome to RuneScape.".into();
        rig.c.chat_type[index] = 0;
        rig.c.chat_username[index].clear();
    }
    for (offset, line) in RESULT_LINES.iter().enumerate() {
        rig.c.chat_text[first + offset] = (*line).into();
        rig.c.chat_type[first + offset] = 0;
        rig.c.chat_username[first + offset].clear();
    }
}

/// Apply one model step to the real Guardian. A fresh `Tick` runs one pass on a
/// new observed game tick; `Frame` runs one on the same one. `cur` is the
/// fixture that raised the event now standing; `Replace` removes it and raises
/// `next` in its place. `Overlap` keeps the hostile actor standing between two
/// swings and raises `next` beside it; `Return` removes `next` again. The Ent
/// guard's steps plant and remove an Ent.
fn drive(
    cur: &dyn Fixture,
    next: &dyn Fixture,
    rig: &mut Rig,
    model: &GuardianState,
    step: &GuardianStep,
) {
    // The model has applied the step already: its event is the real one.
    let id = model.event_id().unwrap_or(0);
    match step {
        GuardianStep::Appear {
            family,
            forced_host,
        } => {
            rig.dirty = true;
            cur.install_as(rig, id, *forced_host, *family);
        }
        GuardianStep::Replace {
            family,
            forced_host,
        } => {
            rig.dirty = true;
            cur.remove(rig);
            // The player is back where every fixture starts from.
            plant_player(&mut rig.c, "Test", 0, 0);
            next.install_as(rig, id, *forced_host, *family);
        }
        GuardianStep::Disappear => cur.remove(rig),
        GuardianStep::Overlap { forced_host, .. } => {
            rig.dirty = true;
            let slot = usize::from(model.retained_id().expect("the model retains the actor"));
            between_swings(rig, slot);
            rig.retained = Some(slot);
            next.install(rig, id, *forced_host);
        }
        GuardianStep::Return => {
            rig.retained = None;
            cur.remove(rig);
        }
        GuardianStep::RetainedGone => {
            let slot = rig.retained.take().expect("a retained actor");
            // Observed removal: its slot now shows another NPC.
            put_npc(&mut rig.c, slot, |c| plant_npc(c, slot, "Man", -1, None));
        }
        GuardianStep::EntAppear => {
            rig.dirty = true;
            EntFx.install(rig, 0, false);
        }
        GuardianStep::EntRemove => EntFx.remove(rig),
        GuardianStep::ScriptStart { wants, live } => {
            rig.start_runtime(Run::Running, *wants, *live);
        }
        // The slot revokes the child with its card (design V11D 4.4).
        GuardianStep::ScriptExit(ScriptExit::Pause) => {
            rig.claims.revoke_child(FightCancel::Paused);
            rig.claims.run = Run::Paused;
        }
        // The runtime ends: the slot's next Start is another runtime.
        GuardianStep::ScriptExit(exit) => {
            rig.claims.revoke_child(match exit {
                ScriptExit::Stop => FightCancel::Stopped,
                _ => FightCancel::CardFailed,
            });
            let (wants, live) = (rig.claims.wants, rig.claims.live);
            rig.start_runtime(Run::Stopped, wants, live);
        }
        GuardianStep::ScriptResume => rig.claims.run = Run::Running,
        GuardianStep::ScriptWants(wants) => rig.claims.wants = *wants,
        GuardianStep::SetEnabled(enabled) => rig.settings.random_events = *enabled,
        GuardianStep::SessionReplace => {
            rig.session += 1;
            // The new Guardian tracks no actor; the retained one stays in the
            // world, unseen by its raw detector.
            rig.retained = None;
            rig.g = Guardian::in_session(rig.session);
            rig.snap = GameSnapshot::new();
            rig.unpublished = true;
            // The slot's session end revokes and drops the old child.
            rig.claims.fight = FightSlot::default();
            let (wants, live) = (rig.claims.wants, rig.claims.live);
            rig.start_runtime(Run::Stopped, wants, live);
            cur.chat_close(rig);
            cur.rebaseline(rig, model.event_id().is_some());
        }
        GuardianStep::ChatClose => cur.chat_close(rig),
        GuardianStep::Replicate { fresh } => cur.replicate(rig, *fresh),
        GuardianStep::LateResult => deliver_late_results(rig),
        GuardianStep::Tick { .. } => {
            let talking = rig.g.in_flight();
            rig.pass(true);
            if !talking && rig.g.in_flight() {
                cur.started(rig);
            }
        }
        GuardianStep::Frame => {
            rig.pass(false);
        }
        // The child's typed end waits for the Guardian's next fresh pass.
        GuardianStep::FightEnd(result) => {
            if let Some((_, outcome)) = rig.claims.fight.child.as_mut() {
                *outcome = match result {
                    FightResult::Killed => FightOutcome::Killed,
                    FightResult::Handoff => FightOutcome::FleeRequired(FightReason::EscapeReserve),
                    FightResult::Budget => FightOutcome::Failed(FightReason::EpisodeBudget),
                };
            }
        }
        // The saved policy of every listed event changes.
        GuardianStep::PolicyFlee | GuardianStep::PolicyFight => {
            rig.policy_flee = matches!(step, GuardianStep::PolicyFlee);
            let saved = if rig.policy_flee {
                vault::RandomEventPolicy::Flee
            } else {
                vault::RandomEventPolicy::Fight
            };
            for policy in rig.policies.values_mut() {
                *policy = saved;
            }
        }
        // The host replay has no seat: the model here is not composed with
        // one, so its alphabet has no seat step. The seat replay is
        // host-play's `guardian_seat_conformance_tests.rs`.
        GuardianStep::Seat(step) => panic!("no seat in the host replay: {step:?}"),
    }
}

/// The real Guardian's five publications: `claim` as "a script owns it",
/// `handling`, `acting`, `ours` and `hold`.
fn published(rig: &Rig) -> (bool, bool, bool, bool, bool) {
    let status = &rig.last;
    (
        status.claim == RandomClaim::Handle,
        status.handling,
        rig.last_acting,
        status.ours,
        status.hold,
    )
}

/// The facts the model derives its publication from, read off the real
/// Guardian: the active event and the enablement inputs.
fn inputs_of(rig: &Rig, status: &RandomStatus) -> ModelInputs {
    let inputs = rig.g.last_inputs;
    let active = rig.g.active.as_ref();
    ModelInputs {
        toggle: status.toggle,
        detected: inputs.detected_ours,
        claim_script: rig.g.claim() == RandomClaim::Handle,
        dialog_in_flight: rig.g.in_flight(),
        solver_in_flight: rig.g.acting(),
        guard: active.is_some_and(|active| active.guard),
        // The active event's family gave up on its episode: the model's stall.
        // A detect-only event is `detected == false` and releases nothing: a
        // dialog handle in flight beside a returning actor whose budget is
        // spent still holds.
        stalled: rig.g.family().released(),
        trapped: inputs.trapped,
    }
}

/// The invariants of the model, over what the real Guardian published.
fn check_real(rig: &Rig, step: &GuardianStep, trace: &[GuardianStep]) {
    let status = &rig.last;
    if rig.unpublished {
        // The new Guardian has not published yet; the old status stands.
        return;
    }
    let fail =
        |what: &str| -> ! { panic!("{what}\nstatus {status:?}\nafter {step:?}\ntrace {trace:#?}") };
    // The publications are the derivation of the one active event and the
    // enablement inputs, and of nothing else.
    let derived = derive_publication(rig.g.active.as_ref(), rig.g.last_inputs);
    if (
        derived.claim == RandomClaim::Handle,
        derived.handling,
        derived.acting,
        derived.ours,
        derived.hold,
    ) != published(rig)
    {
        fail(&format!(
            "publications differ from their derivation {derived:?}"
        ));
    }
    // A released (stalled) family publishes no pending operation and no hold.
    if rig.g.family().released() && (rig.g.acting() || status.hold || status.ours) {
        fail("a released family published work");
    }
    if status.hold && status.claim != RandomClaim::Host {
        fail("a script-owned event published hold");
    }
    if !status.toggle && (status.hold || status.ours) {
        fail("the toggle is off but the Guardian published pending work");
    }
    if status.claim == RandomClaim::Handle && rig.emissions > 0 {
        fail("the host acted on an event a script owns");
    }
    if rig.last.claim == RandomClaim::Handle && rig.claims.run != Run::Running {
        // A script that is not running keeps no claim after a fresh pass; a
        // non-fresh frame cannot refresh it.
        if matches!(step, GuardianStep::Tick { .. }) {
            fail("a script that is not running still owns the event");
        }
    }
    // The active event belongs to this session and to an episode the Guardian
    // has observed; a script's claim names the runtime that answers now (a
    // fresh pass returns a claim whose runtime ended).
    if let Some(active) = rig.g.active.as_ref() {
        if active.key.session != rig.session {
            fail(&format!(
                "the active event belongs to session {} in session {}",
                active.key.session, rig.session
            ));
        }
        if active.key.generation > rig.g.episode.generation {
            fail("the active event names an episode the Guardian never observed");
        }
        if let Owner::Script { runtime } = active.owner {
            if matches!(step, GuardianStep::Tick { .. })
                && runtime != Some(Claims::current_runtime())
            {
                fail("a claim outlived the script runtime that made it");
            }
        }
    }
}

/// Model and real code agree on the five published fields.
fn assert_same(model: &GuardianState, rig: &Rig, step: &GuardianStep, trace: &[GuardianStep]) {
    let expected = model.publication();
    let status = &rig.last;
    let got = published(rig);
    let want = (
        expected.claim_script,
        expected.handling,
        expected.acting,
        expected.ours,
        expected.hold,
    );
    if got != want {
        panic!(
            "model {want:?} (claim_script, handling, acting, ours, hold) vs real {got:?}\nstatus {status:?}\nreal active {:?} inputs {:?}\nafter {step:?}\nmodel {}\ntrace {trace:#?}",
            rig.g.active,
            rig.g.last_inputs,
            model.describe()
        );
    }
    if rig.unpublished {
        return;
    }
    let derived = model_derive(inputs_of(rig, status));
    assert_eq!(
        (
            derived.claim_script,
            derived.handling,
            derived.acting,
            derived.ours,
            derived.hold
        ),
        got,
        "the model's derivation differs from the real publication\nafter {step:?}\ntrace {trace:#?}"
    );
    // The gate token and the child the slot holds are the model's child: a
    // revocation closes it in the pass that folds it, and an orphan in the
    // pass that orphans it.
    assert_eq!(
        status.fight.is_some(),
        model.child_id().is_some(),
        "the published gate token vs the model's child\nstatus {status:?}\nafter {step:?}\nmodel {}\ntrace {trace:#?}",
        model.describe()
    );
    assert_eq!(
        rig.claims.fight.child.is_some(),
        model.child_id().is_some(),
        "the child the slot holds vs the model's child\nafter {step:?}\nmodel {}\ntrace {trace:#?}",
        model.describe()
    );
    // The host solver sends nothing beside the child (design V11D 3.4).
    if model.child_id().is_some()
        && matches!(step, GuardianStep::Tick { .. } | GuardianStep::Frame)
    {
        assert_eq!(
            rig.emissions, 0,
            "the host sent while the child held the event\nafter {step:?}\nmodel {}\ntrace {trace:#?}",
            model.describe()
        );
    }
}

fn replay(fx: &dyn Fixture, steps: &[GuardianStep]) {
    let mut rig = Rig::new(real_data());
    fx.prepare(&mut rig);
    replay_in(&mut rig, fx, steps);
}

fn replay_in(rig: &mut Rig, fx: &dyn Fixture, steps: &[GuardianStep]) {
    replay_mixed(rig, fx, fx, steps, true);
}

/// Replay `steps` through the model and the real Guardian. The event starts
/// as `first` raises it; a `Replace` raises `second`. With `check_model` off
/// the model's own invariants are not asserted, so only the comparison with
/// the real code can fail.
fn replay_mixed(
    rig: &mut Rig,
    first: &dyn Fixture,
    second: &dyn Fixture,
    steps: &[GuardianStep],
    check_model: bool,
) {
    replay_with(
        rig,
        first,
        second,
        steps,
        GuardianMutation::None,
        check_model,
    );
}

fn replay_with(
    rig: &mut Rig,
    first: &dyn Fixture,
    second: &dyn Fixture,
    steps: &[GuardianStep],
    mutation: GuardianMutation,
    check_model: bool,
) {
    let mut model = GuardianState::new(bounds_for(first), mutation);
    let mut done: Vec<GuardianStep> = Vec::new();
    let mut cur = first;
    for step in steps {
        model
            .apply(step)
            .unwrap_or_else(|error| panic!("{}: {step:?} is disabled: {error}", first.name()));
        if check_model {
            model
                .check()
                .unwrap_or_else(|v| panic!("{}: the model fails {v:?} at {step:?}", first.name()));
        }
        drive(cur, second, rig, &model, step);
        match step {
            GuardianStep::Replace { .. } | GuardianStep::Overlap { .. } => cur = second,
            GuardianStep::Return => cur = first,
            _ => {}
        }
        done.push(step.clone());
        check_real(rig, step, &done);
        assert_same(&model, rig, step, &done);
    }
    // The rising-edge callback runs once per event, never replayed.
    assert!(
        rig.claims.callbacks <= rig.sigs,
        "{}: {} callbacks for {} events",
        first.name(),
        rig.claims.callbacks,
        rig.sigs
    );
}

fn alphabet(fx: &dyn Fixture) -> Vec<GuardianStep> {
    let family = fx.family();
    let mut out = Vec::new();
    if fx.ent_only() {
        out.extend([GuardianStep::EntAppear, GuardianStep::EntRemove]);
    } else {
        // In the fight profile a hostile event is what the actor is, an actor
        // of the listed events (`Fight`) or one Fight does not apply to
        // (`Evade`); the saved policy binds it.
        let kinds: &[Family] = if fx.fight_profile() {
            &[Family::Fight, Family::Evade]
        } else {
            &[family]
        };
        for &family in kinds {
            out.push(GuardianStep::Appear {
                family,
                forced_host: false,
            });
            if fx.has_forced_host() {
                out.push(GuardianStep::Appear {
                    family,
                    forced_host: true,
                });
            }
            if fx.distinct_signatures() {
                out.push(GuardianStep::Replace {
                    family,
                    forced_host: false,
                });
                if fx.has_forced_host() {
                    out.push(GuardianStep::Replace {
                        family,
                        forced_host: true,
                    });
                }
            }
        }
        out.push(GuardianStep::Disappear);
    }
    for wants in [false, true] {
        for live in [false, true] {
            out.push(GuardianStep::ScriptStart { wants, live });
        }
    }
    out.extend([
        GuardianStep::ScriptExit(ScriptExit::Pause),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        GuardianStep::ScriptExit(ScriptExit::Fail),
        GuardianStep::ScriptResume,
        GuardianStep::ScriptWants(false),
        GuardianStep::ScriptWants(true),
        GuardianStep::SetEnabled(false),
        GuardianStep::SetEnabled(true),
        GuardianStep::SessionReplace,
        GuardianStep::LateResult,
        GuardianStep::Tick { blocked: false },
        GuardianStep::Frame,
    ]);
    if fx.fight_profile() {
        out.extend([
            GuardianStep::FightEnd(FightResult::Killed),
            GuardianStep::FightEnd(FightResult::Handoff),
            GuardianStep::FightEnd(FightResult::Budget),
            GuardianStep::PolicyFlee,
            GuardianStep::PolicyFight,
        ]);
    }
    if family == Family::Dialog {
        out.push(GuardianStep::ChatClose);
    }
    if fx.name() == "box" {
        out.push(GuardianStep::Replicate { fresh: true });
        out.push(GuardianStep::Replicate { fresh: false });
    }
    out
}

/// The shortest model trace to every model state within `depth` steps.
fn generate(fx: &dyn Fixture, depth: usize) -> Vec<Vec<GuardianStep>> {
    let letters = alphabet(fx);
    let single = !fx.distinct_signatures();
    // After a Disappear a single-signature event needs a fresh pass in between,
    // or the real Guardian never sees the gap that makes a new episode.
    let mut seen: HashSet<(GuardianState, bool)> = HashSet::new();
    let mut queue: VecDeque<(GuardianState, bool, Vec<GuardianStep>)> = VecDeque::new();
    let start = model_for(fx);
    seen.insert((start.clone(), false));
    queue.push_back((start, false, Vec::new()));
    let mut traces = Vec::new();
    while let Some((state, gap, path)) = queue.pop_front() {
        if !path.is_empty() {
            traces.push(path.clone());
        }
        if path.len() >= depth {
            continue;
        }
        for step in &letters {
            let appears = matches!(
                step,
                GuardianStep::Appear { .. } | GuardianStep::Replace { .. }
            );
            if single && appears && gap {
                continue;
            }
            let mut next = state.clone();
            if next.apply(step).is_err() || next.check().is_err() {
                continue;
            }
            let next_gap = match step {
                GuardianStep::Disappear => true,
                GuardianStep::Tick { .. } => false,
                _ => gap,
            };
            if seen.insert((next.clone(), next_gap)) {
                let mut next_path = path.clone();
                next_path.push(step.clone());
                queue.push_back((next, next_gap, next_path));
            }
        }
    }
    traces
}

fn tick() -> GuardianStep {
    GuardianStep::Tick { blocked: false }
}

fn appear(fx: &dyn Fixture) -> GuardianStep {
    if fx.ent_only() {
        return GuardianStep::EntAppear;
    }
    GuardianStep::Appear {
        family: fx.family(),
        forced_host: false,
    }
}

fn run_family(fx: &dyn Fixture, depth: usize) {
    if fx.needs_data() && !data_available() {
        return;
    }
    let traces = generate(fx, depth);
    assert!(
        traces.len() > 100,
        "{}: only {} traces",
        fx.name(),
        traces.len()
    );
    // One client for the whole run: every trace opens a socket otherwise.
    let mut rig = Rig::new(real_data());
    for trace in &traces {
        rig.reset(fx);
        replay_in(&mut rig, fx, trace);
    }
}

#[test]
fn dialog_follows_the_ownership_model() {
    run_family(&DialogFx, 6);
}

#[test]
fn strange_box_follows_the_ownership_model() {
    run_family(&BoxFx, 6);
}

#[test]
fn evade_follows_the_ownership_model() {
    run_family(&EvadeFx, 6);
}

#[test]
fn maze_follows_the_ownership_model() {
    run_family(&MazeFx, 6);
}

#[test]
fn mime_follows_the_ownership_model() {
    run_family(&MimeFx, 6);
}

#[test]
fn lamp_follows_the_ownership_model() {
    run_family(&LampFx, 6);
}

#[test]
fn pick_follows_the_ownership_model() {
    run_family(&PickFx, 6);
}

#[test]
fn lost_gear_follows_the_ownership_model() {
    run_family(&LostGearFx, 6);
}

#[test]
fn lost_tool_follows_the_ownership_model() {
    run_family(&LostToolFx, 6);
}

#[test]
fn ent_follows_the_ownership_model() {
    run_family(&EntFx, 6);
}

/// A hazard stands wherever its loc is; the host steps off only within two
/// tiles. Through every claim, the toggle, a replaced session and the host
/// leaving it behind, the model and the real code publish the same.
#[test]
fn hazard_follows_the_ownership_model() {
    run_family(&HazardFx, 6);
}

/// A hazard is detected wherever its loc stands but is host work only while the
/// player is within two tiles of it: the claim, the toggle and stepping away.
#[test]
fn hazard_follows_the_script_claim_the_toggle_and_stepping_away() {
    let mut rig = Rig::new(None);
    rig.claims = Claims {
        run: Run::Running,
        wants: true,
        live: false,
        callbacks: 0,
        fight: FightSlot::default(),
    };
    HazardFx.install(&mut rig, 0, false);
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::Hazard));
    assert_eq!(status.claim, RandomClaim::Handle);
    assert!(!status.hold && rig.drv.walks.is_empty(), "a script owns it");
    assert_eq!(rig.claims.callbacks, 1);
    rig.claims.run = Run::Stopped;
    let status = rig.pass(true);
    assert_eq!(status.claim, RandomClaim::Host, "returned within one tick");
    assert!(status.hold);
    assert_eq!(rig.drv.walks.len(), 1, "the host steps off once");
    rig.settings.random_events = false;
    let status = rig.pass(true);
    assert!(!status.hold && !status.ours, "the toggle off releases it");
    rig.settings.random_events = true;
    assert!(rig.pass(true).hold);
    HazardFx.remove(&mut rig);
    let status = rig.pass(true);
    assert!(!status.hold, "the player is clear of it");
    assert!(rig.g.active.is_none());
    assert_eq!(rig.claims.callbacks, 1, "no replayed callback");
}

/// The Ent guard is host work without a detected event: no claim callback is
/// ever made for it, so a script cannot own it, and it holds only while the
/// toggle is on, the Ent stands and the claim is the host's.
#[test]
fn ent_is_host_work_that_follows_the_toggle_and_the_tree() {
    let mut rig = Rig::new(None);
    rig.claims = Claims {
        run: Run::Running,
        wants: true,
        live: false,
        callbacks: 0,
        fight: FightSlot::default(),
    };
    plant_ent(&mut rig);
    let status = rig.pass(true);
    assert!(status.hold && status.claim == RandomClaim::Host);
    assert_eq!(status.kind, None);
    assert_eq!(
        rig.claims.callbacks, 0,
        "there is no event to call back for"
    );
    rig.settings.random_events = false;
    let status = rig.pass(true);
    assert!(!status.hold && !status.ours, "the toggle off releases it");
    rig.settings.random_events = true;
    assert!(rig.pass(true).hold, "and the toggle on takes it back");
    rig.g = Guardian::new();
    rig.snap = GameSnapshot::new();
    assert!(
        !rig.pass(false).hold,
        "a new session has not seen the Ent yet"
    );
    clear_npcs(&mut rig.c);
    assert!(!rig.pass(true).hold, "the tree reverted");
}

/// Exact boundaries: every bounded wait ends in a release at the tick the model
/// says, and a stalled family never holds again.
#[test]
fn bounded_waits_end_where_the_model_ends_them() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        let bound = usize::from(fx.bound());
        if bound >= 100 {
            continue;
        }
        let mut steps = vec![appear(fx.as_ref())];
        steps.extend(std::iter::repeat_n(tick(), bound + 4));
        replay(fx.as_ref(), &steps);
        if fx.name() == "box" {
            // Fresh replication re-arms one budget; copied player text does not.
            let mut steps = vec![appear(fx.as_ref())];
            steps.extend(std::iter::repeat_n(tick(), bound + 2));
            steps.push(GuardianStep::Replicate { fresh: false });
            steps.push(tick());
            steps.push(GuardianStep::Replicate { fresh: true });
            steps.push(tick());
            steps.extend(std::iter::repeat_n(tick(), bound + 2));
            replay(fx.as_ref(), &steps);
        }
    }
}

/// Pause, Resume, Stop and failure through a claim and its release, at the
/// edge, mid-wait and at the bound.
#[test]
fn script_pause_resume_stop_and_failure_move_the_claim_for_every_family() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        // A wait can end while the script owns the event (a content clock)
        // or only under the host: each family is replayed with the exit at
        // the edge, mid-wait and past its bound.
        let bound = usize::from(fx.bound()).min(12);
        for live in [false, true] {
            for exit in [ScriptExit::Pause, ScriptExit::Stop, ScriptExit::Fail] {
                for at in [0, 1, bound] {
                    let mut steps = vec![
                        GuardianStep::ScriptStart { wants: true, live },
                        appear(fx.as_ref()),
                    ];
                    steps.extend(std::iter::repeat_n(tick(), at + 1));
                    steps.push(GuardianStep::ScriptExit(exit));
                    steps.push(tick());
                    if exit == ScriptExit::Pause {
                        steps.push(GuardianStep::ScriptResume);
                        steps.push(tick());
                    }
                    steps.push(tick());
                    replay(fx.as_ref(), &steps);
                }
            }
        }
    }
}

/// The toggle off mid-wait publishes nothing and keeps the work in place; the
/// toggle on resumes it. The model and the real code agree at every step.
#[test]
fn the_toggle_releases_every_family_and_resumes_it() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        // Off mid-wait for two ticks, and off across the whole bound.
        let short = vec![
            appear(fx.as_ref()),
            tick(),
            tick(),
            GuardianStep::SetEnabled(false),
            tick(),
            tick(),
            GuardianStep::Frame,
            GuardianStep::SetEnabled(true),
            tick(),
            tick(),
        ];
        replay(fx.as_ref(), &short);
        let mut long = vec![appear(fx.as_ref()), tick(), GuardianStep::SetEnabled(false)];
        long.extend(std::iter::repeat_n(
            tick(),
            usize::from(fx.bound()).min(12) + 1,
        ));
        long.extend([GuardianStep::SetEnabled(true), tick(), tick()]);
        let steps = long;
        replay(fx.as_ref(), &steps);
    }
}

/// A new session starts from nothing: no claim, no hold, no old event.
#[test]
fn session_replacement_drops_the_event_and_the_claim_for_every_family() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        let steps = vec![
            GuardianStep::ScriptStart {
                wants: true,
                live: true,
            },
            appear(fx.as_ref()),
            tick(),
            GuardianStep::SessionReplace,
            tick(),
            tick(),
        ];
        replay(fx.as_ref(), &steps);
    }
}

/// Frames that repeat one observation spend no wait tick and send nothing.
#[test]
fn repeated_observations_spend_no_wait_and_send_nothing() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        let mut rig = Rig::new(real_data());
        fx.prepare(&mut rig);
        fx.install(&mut rig, 0, false);
        rig.pass(true);
        let first = rig.last.clone();
        let sent = rig.sent();
        for _ in 0..20 {
            let again = rig.pass(false);
            assert_eq!(again, first, "{}", fx.name());
        }
        assert_eq!(rig.sent(), sent, "{}: a repeated frame sent", fx.name());
    }
}

// --- Acceptance tests --------------------------------------------------------

/// MAIN-ACCEPTANCE item 1: under a saved Fight, Swarm and the angry plant keep
/// the forced Host claim; a script's Handle claim does not take them.
#[test]
fn swarm_and_angry_plant_keep_the_forced_host_claim_under_saved_fight() {
    if !data_available() {
        return;
    }
    for name in ["Swarm", "Strange plant"] {
        let key = name.to_ascii_lowercase();
        let mut rig = Rig::new(real_data());
        rig.policies
            .insert(key.clone(), vault::RandomEventPolicy::Fight);
        rig.claims = Claims {
            run: Run::Running,
            wants: true,
            live: false,
            callbacks: 0,
            fight: FightSlot::default(),
        };
        plant_npc_with_op(&mut rig.c, 0, name, 32768, None, "Attack");
        let first = rig.pass(true);
        assert_eq!(first.kind, Some(RandomKind::Evade), "{name}");
        assert_eq!(
            first.claim,
            RandomClaim::Host,
            "{name}: the claim is forced"
        );
        assert!(first.hold && first.ours, "{name}: the host flees");
        assert_eq!(rig.drv.walks.len(), 1, "{name}: one flee walk");
        assert_eq!(rig.claims.callbacks, 1, "{name}: the script is told once");
        for _ in 0..3 {
            let again = rig.pass(true);
            assert_eq!(again.claim, RandomClaim::Host, "{name}");
            assert!(again.hold, "{name}");
        }
        assert_eq!(rig.claims.callbacks, 1, "{name}: no second callback");

        // The default Flee policy leaves the claim with the script.
        let mut rig = Rig::new(real_data());
        rig.claims = Claims {
            run: Run::Running,
            wants: true,
            live: false,
            callbacks: 0,
            fight: FightSlot::default(),
        };
        plant_npc_with_op(&mut rig.c, 0, name, 32768, None, "Attack");
        let status = rig.pass(true);
        assert_eq!(status.claim, RandomClaim::Handle, "{name} under Flee");
        assert!(
            !status.hold && rig.drv.walks.is_empty(),
            "{name} under Flee"
        );
    }
}

/// A claim needs a claim source: with none, nobody can own the event.
#[test]
fn a_claim_with_no_claim_source_returns_to_the_host() {
    let mut c = new_client();
    ingame_scene(&mut c);
    plant_player(&mut c, "Test", 0, 0);
    plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
    let mut g = Guardian::new();
    let mut drv = FakeDriver::default();
    let settings = ProfileSettings::default();
    let mut snap = GameSnapshot::new();
    let mut knock = |_: &DetectedRandom| RandomClaim::Handle;
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
    assert_eq!(status.claim, RandomClaim::Handle);
    assert!(drv.menus.is_empty());
    tick_at(&mut c, &mut snap);
    let status = g.tick(&mut drv, &snap, &settings, 0, None);
    assert_eq!(status.claim, RandomClaim::Host, "no source, no claim");
    assert!(
        status.hold && status.handling,
        "the host talks to the genie"
    );
    assert_eq!(drv.menus.len(), 1);
}

/// A host-owned Open survives a script that claims, stops and is replaced: one
/// Open, no duplicate, and no second rising-edge callback.
#[test]
fn host_owned_open_survives_script_stop_and_replacement_without_a_second_send() {
    let mut rig = Rig::new(None);
    rig.claims = Claims {
        run: Run::Running,
        wants: false,
        live: true,
        callbacks: 0,
        fight: FightSlot::default(),
    };
    plant_inv_box(&mut rig.c, 1);
    let first = rig.pass(true);
    assert!(first.hold);
    assert_eq!(rig.drv.menus.len(), 1, "the host sends Open");
    // A Load script takes the box from the next pass on, then stops.
    rig.claims.wants = true;
    for _ in 0..2 {
        let status = rig.pass(true);
        assert_eq!(status.claim, RandomClaim::Handle);
        assert!(!status.hold);
    }
    rig.claims.run = Run::Stopped;
    let status = rig.pass(true);
    assert_eq!(status.claim, RandomClaim::Host, "returned within one tick");
    assert!(status.hold);
    assert_eq!(rig.drv.menus.len(), 1, "the pending Open is not sent again");
    // A compiled replacement is not called back for the box already running.
    rig.claims = Claims {
        run: Run::Running,
        wants: true,
        live: false,
        callbacks: 0,
        fight: FightSlot::default(),
    };
    let status = rig.pass(true);
    assert_eq!(rig.claims.callbacks, 0, "no successor callback");
    assert_eq!(status.claim, RandomClaim::Host);
    assert_eq!(rig.drv.menus.len(), 1);
}

/// A released event leaves no owner state behind, for every family.
#[test]
fn a_released_event_leaves_no_owner_state_for_every_family() {
    for fx in fixtures() {
        if fx.needs_data() && !data_available() {
            continue;
        }
        let mut rig = Rig::new(real_data());
        fx.prepare(&mut rig);
        fx.install(&mut rig, 0, false);
        rig.dirty = true;
        for _ in 0..2 {
            rig.pass(true);
        }
        assert!(
            rig.g.active.is_some(),
            "{}: the host works on it",
            fx.name()
        );
        fx.remove(&mut rig);
        fx.chat_close(&mut rig);
        for _ in 0..40 {
            rig.pass(true);
        }
        assert!(rig.g.active.is_none(), "{}: owner state left", fx.name());
        assert!(rig.g.evade_starts.is_empty(), "{}", fx.name());
        let status = rig.pass(true);
        assert!(
            !status.hold && !status.ours && !status.handling,
            "{}",
            fx.name()
        );
    }
}

/// Unchanged observations spend the same fixed, small allocation every frame:
/// no more than the four of the base (the detected name, the signature, the
/// status row's name) measured at `e8006bd22`; the owner state adds none.
#[test]
fn unchanged_observations_allocate_the_same_every_frame() {
    let mut rig = Rig::new(None);
    plant_inv_box(&mut rig.c, 1);
    rig.pass(true);
    let measure = |rig: &mut Rig| {
        let Rig {
            g,
            drv,
            snap,
            settings,
            policies,
            ..
        } = rig;
        allocation_counter::measure(|| {
            g.tick_with_live_policies(
                drv,
                snap,
                settings,
                &GuardianLiveContext {
                    event_policies: policies,
                    game_data: None,
                },
                0,
                None,
            );
        })
    };
    let first = measure(&mut rig);
    for _ in 0..50 {
        assert_eq!(measure(&mut rig), first, "a repeated frame allocates alike");
    }
    assert!(first.count_total <= 4, "{first:?}");
}

/// The claim cycle of a family whose event needs detector history (a pick's
/// ownership evidence, a gear or tool loss baseline), so the model's single
/// `Appear` step cannot raise it. A compiled script owns the event at its
/// rising edge; the script pauses and the host takes it within one tick; the
/// toggle releases it and returns it; the callback ran once throughout.
fn claim_cycle(rig: &mut Rig, name: &str, work_outlasts: bool) {
    let status = rig.last.clone();
    assert_eq!(
        status.claim,
        RandomClaim::Handle,
        "{name}: the script owns it"
    );
    assert!(!status.hold, "{name}");
    assert_eq!(rig.emissions, 0, "{name}: the host stays out");
    assert_eq!(rig.claims.callbacks, 1, "{name}");
    rig.claims.run = Run::Paused;
    let status = rig.pass(true);
    assert_eq!(
        status.claim,
        RandomClaim::Host,
        "{name}: returned in one tick"
    );
    assert!(status.hold && status.ours, "{name}: the host owns it now");
    rig.claims.run = Run::Running;
    let status = rig.pass(true);
    assert_eq!(
        status.claim,
        RandomClaim::Host,
        "{name}: no callback, no claim"
    );
    assert_eq!(rig.claims.callbacks, 1, "{name}: the edge is not replayed");
    rig.settings.random_events = false;
    let status = rig.pass(true);
    assert!(
        !status.hold && !status.ours,
        "{name}: the toggle releases it"
    );
    rig.settings.random_events = true;
    let returned = rig.pass(true);
    if work_outlasts {
        assert!(returned.hold, "{name}: the toggle returns it");
    } else {
        assert_ne!(returned.claim, RandomClaim::Handle, "{name}");
    }
    let derived = derive_publication(rig.g.active.as_ref(), rig.g.last_inputs);
    assert_eq!(derived.hold, rig.last.hold, "{name}");
}

fn compiled_script(rig: &mut Rig) {
    rig.claims = Claims {
        run: Run::Running,
        wants: true,
        live: false,
        callbacks: 0,
        fight: FightSlot::default(),
    };
}

#[test]
fn lost_gear_follows_the_claim_cycle() {
    let mut rig = Rig::new(None);
    compiled_script(&mut rig);
    plant_npc(&mut rig.c, 0, "Fishing spot", -1, None);
    plant_inv_named(&mut rig.c, 311, Some("Harpoon"));
    assert_ne!(rig.pass(true).kind, Some(RandomKind::LostGear));
    drop_harpoon_to_ground(&mut rig.c);
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::LostGear));
    claim_cycle(&mut rig, "lost gear", true);
}

#[test]
fn lost_tool_follows_the_claim_cycle() {
    if !data_available() {
        return;
    }
    let mut rig = Rig::new(real_data());
    compiled_script(&mut rig);
    plant_inv_named(&mut rig.c, 1351, Some("Bronze axe"));
    rig.pass(true);
    plant_inv_named(&mut rig.c, 492, Some("Axe handle"));
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::LostTool));
    // The refit has a three-tick spawn grace: it ends in a release on its own
    // before the toggle comes back, which is the bounded wait ending.
    claim_cycle(&mut rig, "lost tool", false);
}

#[test]
fn strange_plant_pick_follows_the_claim_cycle() {
    let mut rig = Rig::new(None);
    compiled_script(&mut rig);
    plant_npc_with_op(
        &mut rig.c,
        0,
        "Strange plant",
        -1,
        Some("Pick Test!"),
        "Pick",
    );
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::Pick));
    claim_cycle(&mut rig, "strange plant", true);
}

// --- Overlap between families, content clocks, seeded divergence ------------

/// The ten event families. The Ent guard has no detected event; it overlaps
/// events instead of replacing them (`ent_overlap_traces`).
fn event_fixtures() -> Vec<Box<dyn Fixture>> {
    fixtures().into_iter().filter(|fx| !fx.ent_only()).collect()
}

fn replace_with(fx: &dyn Fixture) -> GuardianStep {
    GuardianStep::Replace {
        family: fx.family(),
        forced_host: false,
    }
}

/// One event replaced by another of a different family: with a script that
/// pauses, resumes and fails, with the toggle off, after an exhausted episode
/// and across a session replacement.
fn overlap_traces(a: &dyn Fixture, b: &dyn Fixture) -> Vec<Vec<GuardianStep>> {
    let script = |live| GuardianStep::ScriptStart { wants: true, live };
    let ending = vec![GuardianStep::Disappear, tick()];
    let exhausted = usize::from(a.bound()).min(12) + 1;
    let mut spent = vec![appear(a)];
    spent.extend(std::iter::repeat_n(tick(), exhausted));
    spent.extend([replace_with(b), tick(), tick()]);
    spent.extend(ending.clone());
    let mut plain = vec![appear(a), tick(), tick(), replace_with(b), tick(), tick()];
    plain.extend(ending);
    vec![
        plain,
        vec![
            script(false),
            appear(a),
            tick(),
            replace_with(b),
            tick(),
            GuardianStep::ScriptExit(ScriptExit::Pause),
            tick(),
            GuardianStep::ScriptResume,
            tick(),
        ],
        vec![
            appear(a),
            tick(),
            GuardianStep::SetEnabled(false),
            replace_with(b),
            tick(),
            GuardianStep::SetEnabled(true),
            tick(),
        ],
        vec![
            appear(a),
            tick(),
            replace_with(b),
            tick(),
            GuardianStep::SessionReplace,
            tick(),
            tick(),
        ],
        vec![
            script(true),
            appear(a),
            tick(),
            replace_with(b),
            tick(),
            GuardianStep::ScriptExit(ScriptExit::Fail),
            tick(),
            tick(),
        ],
        spent,
    ]
}

/// An event of one family replaced by an event of another, for every ordered
/// pair of the ten event families. The model's `Replace` removes the first
/// event: the old episode ends (or, for a dialog, finishes its handle) and the
/// new one is the host's or a script's, never masked by the old. A family that
/// needs detector history (lost gear) gets it before the trace. An overlap
/// that keeps the first actor is `a_retained_hostile_actor_overlapped_by_every_family_follows_the_model`.
#[test]
fn an_event_replaced_by_another_family_follows_the_model() {
    let fixtures = event_fixtures();
    let mut rig = Rig::new(real_data());
    let mut pairs = 0;
    for a in &fixtures {
        for b in &fixtures {
            if a.name() == b.name() || ((a.needs_data() || b.needs_data()) && !data_available()) {
                continue;
            }
            pairs += 1;
            for steps in overlap_traces(a.as_ref(), b.as_ref()) {
                // Whatever an earlier trace left standing goes first.
                for fx in &fixtures {
                    fx.remove(&mut rig);
                }
                clear_inv(&mut rig.c);
                rig.dirty = false;
                rig.reset_pair(a.as_ref(), b.as_ref());
                replay_mixed(&mut rig, a.as_ref(), b.as_ref(), &steps, true);
            }
        }
    }
    if data_available() {
        assert_eq!(pairs, 90, "every ordered pair of the ten event families");
    }
}

fn overlap_with(fx: &dyn Fixture) -> GuardianStep {
    GuardianStep::Overlap {
        family: fx.family(),
        forced_host: false,
    }
}

/// A hostile actor of `a` overlapped by an event of `b` while it stays
/// engaged, and the same actor again when the overlap ends: with budget left,
/// with the budget spent during the overlap (the round-3 review
/// counterexample) or before it, under a script claim, when the actor is
/// observed gone, and across a session replacement.
///
/// A trap that moves the player (maze, mime: `Family::Teleport`) takes the
/// player out of the actor's chase range, which the content counts as outward
/// progress (`Npc.ts:77-85,661-670`): the real no-progress counter resets and
/// does not run while the trap is the event, and the model's clock follows
/// (`teleported` in `GuardianState::content_clock`). A held Strange box
/// (`Family::Trap`) does not move the player. The last trace spends the budget
/// during the overlap unless the overlap is a teleport.
fn retained_traces(a: &dyn Fixture, b: &dyn Fixture) -> Vec<Vec<GuardianStep>> {
    let bound = usize::from(a.bound());
    let ending = [GuardianStep::Return, tick(), tick()];
    let mut left = vec![appear(a), tick(), tick(), overlap_with(b), tick(), tick()];
    left.extend(ending.clone());
    let mut before = vec![appear(a)];
    before.extend(std::iter::repeat_n(tick(), bound + 1));
    before.extend([overlap_with(b), tick(), tick()]);
    before.extend(ending.clone());
    let mut traces = vec![
        left,
        before,
        vec![
            GuardianStep::ScriptStart {
                wants: true,
                live: false,
            },
            appear(a),
            tick(),
            overlap_with(b),
            tick(),
            GuardianStep::Return,
            tick(),
            tick(),
        ],
        vec![
            appear(a),
            tick(),
            overlap_with(b),
            tick(),
            GuardianStep::RetainedGone,
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
        vec![
            appear(a),
            tick(),
            overlap_with(b),
            tick(),
            GuardianStep::SessionReplace,
            tick(),
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
    ];
    let mut during = vec![appear(a)];
    during.extend(std::iter::repeat_n(tick(), bound - 1));
    during.extend([overlap_with(b), tick(), tick()]);
    during.extend(ending);
    traces.push(during);
    traces
}

/// A hostile actor that stays engaged while an event of each other family wins
/// the detector keeps its identity and its escape clock: the model's `Overlap`
/// and `Return` against the real Guardian's retained actor.
#[test]
fn a_retained_hostile_actor_overlapped_by_every_family_follows_the_model() {
    if !data_available() {
        return;
    }
    let fixtures = event_fixtures();
    let a = &EvadeFx;
    let mut rig = Rig::new(real_data());
    let mut families = 0;
    for b in &fixtures {
        if b.name() == a.name() {
            continue;
        }
        families += 1;
        let traces = retained_traces(a, b.as_ref());
        let last = traces.len() - 1;
        for (index, steps) in traces.iter().enumerate() {
            for fx in &fixtures {
                fx.remove(&mut rig);
            }
            clear_inv(&mut rig.c);
            rig.dirty = false;
            rig.reset_pair(a, b.as_ref());
            replay_mixed(&mut rig, a, b.as_ref(), steps, true);
            if index == last {
                // The budget ran out during the overlap, unless the overlap
                // was a trap that carried the player out of the actor's range.
                assert_eq!(
                    rig.last.ours,
                    b.family() == Family::Teleport,
                    "{}: the returned actor's budget after the overlap",
                    b.name()
                );
            }
        }
    }
    assert_eq!(families, 9, "the hostile actor against every other family");
}

/// Round-3 review probe (`review-r3-sol/retained-evade-combat-probe.rs`): the
/// River troll stays in its slot between two swings while a held Strange box
/// wins the detector. Its original escape budget runs out during the box, and
/// the returning troll is detect-only. The model maps the box to `Overlap` and
/// its end to `Return`: the same actor with its clock, not a new one.
#[test]
fn review_retained_evade_budget_survives_a_box_overlap_with_combat_evidence() {
    if !data_available() {
        return;
    }
    let mut rig = Rig::new(real_data());
    let mut model = model_for(&EvadeFx);
    let mut trace = Vec::new();
    let mut advance = |rig: &mut Rig, model: &mut GuardianState, step: GuardianStep| {
        model.apply(&step).expect("enabled model step");
        model.check().expect("chosen model invariants");
        trace.push(step.clone());
        if matches!(step, GuardianStep::Tick { .. }) {
            rig.pass(true);
        }
        assert_same(model, rig, &step, &trace);
    };
    EvadeFx.install(&mut rig, 0, false);
    advance(&mut rig, &mut model, appear(&EvadeFx));
    for _ in 0..8 {
        advance(&mut rig, &mut model, tick());
    }
    // Existing production regressions use this inter-swing snapshot: no face
    // target but retained combat evidence. The actor remains engaged for the
    // Guardian's fallback, and the box now wins the raw detector.
    between_swings(&mut rig, 0);
    BoxFx.install(&mut rig, 1, false);
    advance(&mut rig, &mut model, overlap_with(&BoxFx));
    advance(&mut rig, &mut model, tick());
    advance(&mut rig, &mut model, tick());
    assert_eq!(rig.last.kind, Some(RandomKind::Box));
    assert!(rig.g.evade.as_ref().expect("the retained troll").gave_up);
    assert_eq!(model.retained_id(), Some(0));
    BoxFx.remove(&mut rig);
    // The detector again selects the same retained troll.
    advance(&mut rig, &mut model, GuardianStep::Return);
    advance(&mut rig, &mut model, tick());
    assert_eq!(rig.last.kind, Some(RandomKind::Evade));
    assert_eq!(model.event_id(), Some(0), "the same actor returns");
    assert!(!rig.last.ours && !model.publication().ours);
}

/// The defects seeded back into the model, replayed against the real Guardian:
/// `RetainedClockLost` (the round-3 defect: the returning actor opens a fresh
/// budget) over a Strange box, and `TeleportProgressLost` (the V11a acceptance
/// item: the model counts the ticks of a teleporting trap, so the returning
/// actor is given up although the content reset its counter) over Maze and
/// Mime. The comparison catches each; with the model's invariants on, its
/// `actor-budget` invariant catches it first.
#[test]
fn a_model_that_loses_the_retained_clock_or_teleport_progress_fails_the_replay() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    if !data_available() {
        return;
    }
    let mut rig = Rig::new(real_data());
    // The seeded faults of the model against the real Guardian: the box
    // overlap spends the budget (a model that forgets the clock gives the
    // returning actor a fresh one), a teleporting trap resets it (a model that
    // counts the trap's ticks gives the actor up).
    let cases: [(&dyn Fixture, GuardianMutation); 3] = [
        (&BoxFx, GuardianMutation::RetainedClockLost),
        (&MazeFx, GuardianMutation::TeleportProgressLost),
        (&MimeFx, GuardianMutation::TeleportProgressLost),
    ];
    for (b, mutation) in cases {
        let steps = retained_traces(&EvadeFx, b)
            .pop()
            .expect("the budget runs out or is reset during the overlap");
        for (check_model, expected) in [(false, "vs real"), (true, "actor-budget")] {
            rig.reset_pair(&EvadeFx, b);
            let failure = catch_unwind(AssertUnwindSafe(|| {
                replay_with(&mut rig, &EvadeFx, b, &steps, mutation, check_model);
            }))
            .expect_err("the seeded model must fail");
            let message = failure
                .downcast_ref::<String>()
                .expect("a formatted panic message");
            assert!(
                message.contains(expected),
                "{} {mutation:?} {expected}: {message}",
                b.name()
            );
        }
        // The unseeded model replays the same trace.
        rig.reset_pair(&EvadeFx, b);
        replay_mixed(&mut rig, &EvadeFx, b, &steps, true);
    }
}

/// The Ent guard overlapping an event of each family: the guard first or the
/// event first, a script claim that pauses and resumes under the guard, the
/// toggle, and a session replacement.
fn ent_overlap_traces(b: &dyn Fixture) -> Vec<Vec<GuardianStep>> {
    use GuardianStep::{EntAppear, EntRemove};
    vec![
        vec![
            EntAppear,
            tick(),
            appear(b),
            tick(),
            tick(),
            EntRemove,
            tick(),
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
        vec![
            appear(b),
            tick(),
            EntAppear,
            tick(),
            tick(),
            GuardianStep::Disappear,
            tick(),
            EntRemove,
            tick(),
        ],
        vec![
            GuardianStep::ScriptStart {
                wants: true,
                live: false,
            },
            EntAppear,
            tick(),
            appear(b),
            tick(),
            GuardianStep::ScriptExit(ScriptExit::Pause),
            tick(),
            GuardianStep::ScriptResume,
            tick(),
        ],
        vec![
            EntAppear,
            tick(),
            GuardianStep::SetEnabled(false),
            appear(b),
            tick(),
            GuardianStep::SetEnabled(true),
            tick(),
            tick(),
        ],
        vec![
            appear(b),
            tick(),
            EntAppear,
            tick(),
            GuardianStep::SessionReplace,
            tick(),
            tick(),
        ],
    ]
}

#[test]
fn the_ent_guard_overlapping_every_family_follows_the_model() {
    let fixtures = event_fixtures();
    let mut rig = Rig::new(real_data());
    for b in &fixtures {
        if b.needs_data() && !data_available() {
            continue;
        }
        for steps in ent_overlap_traces(b.as_ref()) {
            for fx in &fixtures {
                fx.remove(&mut rig);
            }
            EntFx.remove(&mut rig);
            clear_inv(&mut rig.c);
            rig.dirty = false;
            rig.reset(b.as_ref());
            replay_in(&mut rig, b.as_ref(), &steps);
        }
    }
}

/// A model that disagrees with the real code fails the conformance: seeded in
/// the model's publication (a stalled family keeps its hold), with the model's
/// own invariants off so that only the comparison can fail.
#[test]
#[should_panic(expected = "vs real")]
fn a_model_that_disagrees_with_the_real_code_fails_the_conformance() {
    let fx = BoxFx;
    let mut rig = Rig::new(None);
    let mut steps = vec![appear(&fx)];
    steps.extend(std::iter::repeat_n(tick(), usize::from(fx.bound()) + 2));
    replay_with(
        &mut rig,
        &fx,
        &fx,
        &steps,
        GuardianMutation::StalledHolds,
        false,
    );
}

/// The same seed with the model's invariants on fails in the model first, so
/// both guards exist: the invariant and the comparison.
#[test]
#[should_panic(expected = "the model fails")]
fn a_seeded_model_mutation_fails_its_invariant_before_the_comparison() {
    let fx = BoxFx;
    let mut rig = Rig::new(None);
    let mut steps = vec![appear(&fx)];
    steps.extend(std::iter::repeat_n(tick(), usize::from(fx.bound()) + 2));
    replay_with(
        &mut rig,
        &fx,
        &fx,
        &steps,
        GuardianMutation::StalledHolds,
        true,
    );
}

/// The give-up pass of an Evade: the first pass, counted from 1, that
/// publishes `ours == false`.
fn evade_give_up_pass(
    rig: &mut Rig,
    script_owns_until: usize,
    toggle_off: Option<(usize, usize)>,
) -> usize {
    EvadeFx.install(rig, 0, false);
    rig.dirty = true;
    for pass in 1..=30 {
        if let Some((from, to)) = toggle_off {
            rig.settings.random_events = !(from..to).contains(&pass);
        }
        if pass == script_owns_until + 1 {
            rig.claims.run = Run::Stopped;
        }
        let status = rig.pass(true);
        if status.toggle && !status.ours {
            return pass;
        }
    }
    panic!("the Evade never gave up");
}

/// An Evade's budget is the content's clock: it gives up on the same fresh
/// tick whoever holds the claim and whatever the toggle says.
#[test]
fn an_evade_gives_up_on_the_content_clock_whoever_holds_the_claim() {
    if !data_available() {
        return;
    }
    let host = evade_give_up_pass(&mut Rig::new(real_data()), 0, None);
    assert_eq!(host, usize::from(EvadeFx.bound()) + 1);
    let mut rig = Rig::new(real_data());
    compiled_script(&mut rig);
    let script = evade_give_up_pass(&mut rig, 30, None);
    assert_eq!(script, host, "a script claim does not pause the clock");
    let toggled = evade_give_up_pass(&mut Rig::new(real_data()), 0, Some((3, 7)));
    assert_eq!(toggled, host, "the toggle off does not pause the clock");
}

/// A lost tool's refit window is the content's clock once the host has begun:
/// it runs through a toggle off, and the host that finds the window closed
/// sends nothing and publishes nothing.
#[test]
fn a_lost_tool_deadline_runs_through_a_toggle_off() {
    if !data_available() {
        return;
    }
    let fx = LostToolFx;
    let mut rig = Rig::new(real_data());
    fx.prepare(&mut rig);
    // Host-owned: three working passes, then it lets go.
    fx.install(&mut rig, 0, false);
    rig.dirty = true;
    for pass in 1..=3 {
        assert!(rig.pass(true).hold, "pass {pass}");
    }
    assert!(!rig.pass(true).hold && !rig.last.ours);
    // The same window with the toggle off for two of its ticks.
    rig.reset(&fx);
    fx.install(&mut rig, 0, false);
    rig.dirty = true;
    assert!(rig.pass(true).hold, "the host begins");
    rig.settings.random_events = false;
    rig.pass(true);
    rig.pass(true);
    rig.settings.random_events = true;
    let sent = rig.sent();
    let status = rig.pass(true);
    assert!(
        !status.hold && !status.ours,
        "the window closed while the toggle was off"
    );
    assert_eq!(
        rig.sent(),
        sent,
        "the host sends nothing for a spent window"
    );
}

/// A hostile actor that went out of view because the player moved on is kept
/// for its content deadline: it does not replace a trap square, and it is the
/// event again once the trap ends.
#[test]
fn a_retained_hostile_actor_resumes_after_an_overlapping_trap_ends() {
    if !data_available() {
        return;
    }
    let mut rig = Rig::new(real_data());
    EvadeFx.install(&mut rig, 0, false);
    rig.dirty = true;
    rig.pass(true);
    rig.pass(true);
    // The actor leaves the client's NPC list as the player steps onto the
    // maze far away: a view-edge removal, not despawn evidence.
    remove_npcs(&mut rig.c, &EVENT_SLOTS);
    MazeFx.install(&mut rig, 0, false);
    let status = rig.pass(true);
    assert_eq!(
        status.kind,
        Some(RandomKind::Maze),
        "the trap is not masked"
    );
    assert!(status.hold);
    MazeFx.remove(&mut rig);
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::Evade), "the actor is back");
    assert!(status.ours);
}

// --- The active event is the owner (design 3.2) -------------------------------

/// The enablement inputs of the last publication, and nothing else: the
/// toggle, the detector's `ours` and whether the event is a trapped kind.
fn enablement(rig: &Rig) -> PublicationInputs {
    rig.g.last_inputs
}

#[test]
fn a_stalled_maze_is_released_by_its_active_event_alone() {
    let fx = MazeFx;
    let mut rig = Rig::new(None);
    fx.install(&mut rig, 0, false);
    rig.dirty = true;
    let mut status = rig.pass(true);
    assert!(status.hold && status.ours, "the visit is worked first");
    for _ in 0..=maze::SOLVE_TICKS {
        status = rig.pass(true);
    }
    assert_eq!(status.kind, Some(RandomKind::Maze), "still on the square");
    assert!(!status.hold && !status.ours, "the visit was given up");
    let active = rig
        .g
        .active
        .clone()
        .expect("the stall is kept in the active event");
    assert!(matches!(&active.family, super::Family::Maze(visit) if visit.stalled));
    assert!(active.solver.is_none() && active.dialog.is_none() && !active.guard);
    // The enablement inputs alone describe a trap square the host would hold;
    // the family phase of the active event is what releases it.
    assert!(enablement(&rig).trapped && enablement(&rig).detected_ours);
    let derived = derive_publication(Some(&active), enablement(&rig));
    assert!(!derived.hold && !derived.ours && !derived.acting);
    let mut unstalled = active.clone();
    unstalled.family = super::Family::None;
    assert!(derive_publication(Some(&unstalled), enablement(&rig)).hold);
    // Leaving the square ends the visit and its active event.
    fx.remove(&mut rig);
    rig.pass(true);
    assert!(rig.g.active.is_none());
}

#[test]
fn the_box_phase_and_its_outcome_live_in_the_active_event() {
    let fx = BoxFx;
    let mut rig = Rig::new(None);
    fx.install(&mut rig, 0, false);
    rig.dirty = true;
    rig.pass(true);
    let solve = |rig: &Rig| match &rig.g.active.as_ref().expect("active").family {
        super::Family::Box(solve) => *solve,
        other => panic!("not a box: {other:?}"),
    };
    assert!(matches!(solve(&rig).phase, BoxPhase::Opening { .. }));
    for _ in 0..usize::from(fx.bound()) {
        rig.pass(true);
    }
    let stalled = solve(&rig);
    assert_eq!(stalled.phase, BoxPhase::Stalled);
    assert_eq!(stalled.failures, BOX_MAX_FAILURES);
    assert_eq!(stalled.last_failure, Some(BoxFailure::OpenTimeout));
    let active = rig.g.active.clone().expect("active");
    assert!(active.solver.is_none() && !rig.last.hold && !rig.last.ours);
    // Only a fresh server replication re-arms it, with a fresh budget.
    fx.replicate(&mut rig, true);
    rig.pass(true);
    assert_eq!(solve(&rig).failures, 0);
    assert_ne!(solve(&rig).phase, BoxPhase::Stalled);
    // The episode ends with the box: no state is left.
    fx.remove(&mut rig);
    rig.pass(true);
    assert!(rig.g.active.is_none());
}

#[test]
fn the_lamp_redemption_lives_in_the_active_event_and_its_history_beside_it() {
    let fx = LampFx;
    let mut rig = Rig::new(None);
    fx.install(&mut rig, 0, false);
    rig.dirty = true;
    rig.pass(true);
    let active = rig.g.active.clone().expect("a Rub is in flight");
    assert_eq!(active.solver, Some(RandomKind::Lamp));
    assert!(matches!(&active.family, super::Family::Lamp(flow) if flow.rubbed));
    assert_eq!(
        rig.g.lamp.rubs, 1,
        "the Rub budget is history, not episode state"
    );
    // The skill interface never opens: the redemption pauses and the lamp is
    // detected but not worked.
    for _ in 0..=MAX_LAMP_WAIT {
        rig.pass(true);
    }
    assert!(
        rig.g.active.is_none(),
        "a paused lamp keeps no active event"
    );
    assert!(rig.g.lamp.park > 0);
    assert_eq!(rig.last.kind, Some(RandomKind::Lamp));
    assert!(!rig.last.ours && !rig.last.hold);
}

/// A latch that must outlive an episode stays outside the active event, is
/// named, and holds no authority: another family's episode publishes from its
/// own active event while the history of the lamp stands.
#[test]
fn history_that_outlives_an_episode_publishes_nothing_of_its_own() {
    let mut rig = Rig::new(None);
    LampFx.install(&mut rig, 0, false);
    rig.dirty = true;
    for _ in 0..=(MAX_LAMP_WAIT + 1) {
        rig.pass(true);
    }
    assert!(rig.g.lamp.park > 0 && !rig.last.ours);
    let parked = rig.g.lamp;
    // A genie arrives while the lamp stays in the pack.
    plant_npc(&mut rig.c, 0, "Genie", -1, Some("Greetings Test!"));
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::Dialog));
    assert!(status.hold && status.ours, "the genie is the host's");
    let active = rig.g.active.clone().expect("the host talks to the genie");
    let derived = derive_publication(Some(&active), enablement(&rig));
    assert_eq!((derived.hold, derived.ours), (status.hold, status.ours));
    assert!(
        rig.g.lamp.park <= parked.park && rig.g.lamp.rubs == parked.rubs,
        "the history is only counted down"
    );
    clear_npcs(&mut rig.c);
    close_chat(&mut rig.c);
    for _ in 0..4 {
        rig.pass(true);
    }
    assert!(
        rig.g.active.is_none(),
        "no episode state is left for the genie"
    );
    let status = rig.pass(true);
    assert_eq!(status.kind, Some(RandomKind::Lamp));
    assert!(
        !status.ours && !status.hold,
        "the parked lamp is still parked"
    );
}

/// A hostile actor's successor takes over the flee in a new episode, with no
/// pass between them that releases the slot.
#[test]
fn a_hostile_successor_inherits_the_flee_in_a_new_episode() {
    if !data_available() {
        return;
    }
    let mut rig = Rig::new(real_data());
    EvadeFx.install(&mut rig, 0, false);
    rig.dirty = true;
    let first = rig.pass(true);
    assert!(first.hold && rig.last_acting);
    let before = rig.g.active.as_ref().expect("active").key;
    assert_eq!(before.kind, Some(RandomKind::Evade));
    EvadeFx.remove(&mut rig);
    EvadeFx.install(&mut rig, 1, false);
    let second = rig.pass(true);
    assert_eq!(second.name.as_deref(), Some("rock golem"));
    assert!(second.hold && rig.last_acting, "the flee goes on");
    let after = rig.g.active.as_ref().expect("active").key;
    assert_eq!(after.generation, before.generation + 1, "a new episode");
    assert_eq!(after.kind, Some(RandomKind::Evade));
}

/// The key names the slot session: an episode of a replaced session never has
/// the key of the same event in the next one, and the new Guardian starts with
/// no active event.
#[test]
fn an_episode_key_names_its_session() {
    let mut rig = Rig::new(None);
    DialogFx.install(&mut rig, 0, false);
    let status = rig.pass(true);
    assert!(status.hold);
    let old = rig.g.active.as_ref().expect("the host talks").key;
    assert_eq!(old.session, 0);
    rig.g = Guardian::in_session(1);
    rig.snap = GameSnapshot::new();
    assert!(rig.g.active.is_none(), "a new session starts from nothing");
    rig.pass(true);
    let new = rig.g.active.as_ref().expect("the host talks again").key;
    assert_eq!((new.session, new.kind), (1, old.kind));
    assert_ne!(new, old);
}

// --- Round 3: runtime identity, content clocks, live state in the event ------

/// A claim belongs to the script runtime that made it (design 3.3). A
/// compiled script replaced before the next observation by one that never
/// claims does not inherit the claim; one that would claim is not called back
/// for the event already standing. The episode is the same throughout.
#[test]
fn a_replacement_runtime_returns_the_claim_without_a_callback() {
    for successor_wants in [false, true] {
        let mut rig = Rig::new(None);
        rig.start_runtime(Run::Running, true, false);
        DialogFx.install(&mut rig, 0, false);
        assert_eq!(rig.pass(true).claim, RandomClaim::Handle);
        assert_eq!(rig.claims.callbacks, 1);
        let active = rig.g.active.as_ref().expect("script claim");
        assert_eq!(
            active.owner,
            Owner::Script {
                runtime: Some(Claims::current_runtime())
            },
            "the claim names its runtime"
        );
        let before = active.key;
        rig.start_runtime(Run::Running, successor_wants, false);
        let callbacks = rig.claims.callbacks;
        let status = rig.pass(true);
        assert_eq!(
            status.claim,
            RandomClaim::Host,
            "successor wants {successor_wants}"
        );
        assert_eq!(rig.claims.callbacks, callbacks, "no successor callback");
        assert_eq!(rig.g.active.as_ref().expect("host work").key, before);
        assert!(
            status.hold && status.handling,
            "the host talks to the genie"
        );
    }
}

/// The same replacement against the model, compiled and Load, claiming and
/// not.
#[test]
fn a_replacement_runtime_follows_the_model() {
    for live in [false, true] {
        for successor_wants in [false, true] {
            replay(
                &DialogFx,
                &[
                    GuardianStep::ScriptStart { wants: true, live },
                    appear(&DialogFx),
                    tick(),
                    GuardianStep::ScriptExit(ScriptExit::Stop),
                    GuardianStep::ScriptStart {
                        wants: successor_wants,
                        live,
                    },
                    tick(),
                    tick(),
                ],
            );
        }
    }
}

/// An Evade a script owns gives up on the content's clock, as the model's
/// Evade budget does: the episode is detect-only for the script too.
#[test]
fn an_evade_a_script_owns_releases_on_the_content_clock_as_the_model_does() {
    if !data_available() {
        return;
    }
    let mut steps = vec![
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(&EvadeFx),
    ];
    steps.extend(std::iter::repeat_n(
        tick(),
        usize::from(EvadeFx.bound()) + 2,
    ));
    replay(&EvadeFx, &steps);
    // The script lets go after the budget: the host does not begin again.
    steps.extend([GuardianStep::ScriptExit(ScriptExit::Pause), tick(), tick()]);
    replay(&EvadeFx, &steps);
}

/// The lost tool's window runs through a toggle off; a host that begins after
/// it closed (the toggle, or a script that held the event) lets go at once;
/// and a pass that sees the toggle off re-arms one new window.
#[test]
fn a_lost_tool_window_follows_the_model_through_the_toggle_and_the_claim() {
    if !data_available() {
        return;
    }
    let toggle = [
        appear(&LostToolFx),
        tick(),
        GuardianStep::SetEnabled(false),
        tick(),
        tick(),
        GuardianStep::SetEnabled(true),
        tick(),
    ];
    replay(&LostToolFx, &toggle);
    let mut late = vec![
        GuardianStep::ScriptStart {
            wants: true,
            live: false,
        },
        appear(&LostToolFx),
    ];
    late.extend(std::iter::repeat_n(tick(), 6));
    late.extend([GuardianStep::ScriptExit(ScriptExit::Pause), tick(), tick()]);
    replay(&LostToolFx, &late);
    let mut rearm = vec![appear(&LostToolFx)];
    rearm.extend(std::iter::repeat_n(tick(), 5));
    rearm.extend([
        GuardianStep::SetEnabled(false),
        tick(),
        GuardianStep::SetEnabled(true),
        tick(),
        tick(),
        tick(),
        tick(),
        tick(),
    ]);
    replay(&LostToolFx, &rearm);
}

/// The live solver state of the flee, the lost-tool recovery and the Ent guard
/// is in the active event and ends with it; the Guardian keeps only detector
/// history beside it.
#[test]
fn live_solver_state_lives_in_the_active_event() {
    if !data_available() {
        return;
    }
    // The flee: its walk-back tile and walk target.
    let mut rig = Rig::new(real_data());
    EvadeFx.install(&mut rig, 0, false);
    rig.dirty = true;
    assert!(rig.pass(true).hold);
    let active = rig.g.active.clone().expect("the host flees");
    let super::Family::Evade(run) = active.family else {
        panic!("not a flee: {:?}", active.family);
    };
    assert!(
        run.flee_from.is_some() && run.flee_target.is_some(),
        "{run:?}"
    );
    assert!(rig.g.evade.as_ref().is_some_and(|track| track.has_fled));
    EvadeFx.remove(&mut rig);
    rig.pass(true);
    rig.pass(true);
    assert!(rig.g.active.is_none(), "the flee ended with its event");

    // The lost-tool recovery: its window and target.
    let mut rig = Rig::new(real_data());
    LostToolFx.prepare(&mut rig);
    LostToolFx.install(&mut rig, 0, false);
    rig.dirty = true;
    assert!(rig.pass(true).hold);
    let active = rig.g.active.clone().expect("the host recovers");
    let super::Family::LostTool(run) = &active.family else {
        panic!("not a recovery: {:?}", active.family);
    };
    assert!(run.recovery.is_some(), "{run:?}");
    LostToolFx.remove(&mut rig);
    rig.pass(true);
    assert!(rig.g.active.is_none(), "the recovery ended with its event");

    // The Ent guard: its latch, and the hold it asks for.
    let mut rig = Rig::new(None);
    plant_ent(&mut rig);
    assert!(rig.pass(true).hold);
    let active = rig.g.active.clone().expect("the guard holds");
    assert!(active.guard && active.ent.is_some());
    rig.settings.random_events = false;
    assert!(!rig.pass(true).hold);
    let active = rig.g.active.clone().expect("the latch stays");
    assert!(
        !active.guard && active.ent.is_some(),
        "the toggle releases the hold"
    );
    rig.settings.random_events = true;
    EntFx.remove(&mut rig);
    rig.pass(true);
    assert!(rig.g.active.is_none(), "the guard ended with its Ent");
}

/// A late result carries an earlier token: every settling server line, below
/// the cutoff of the operation in flight, settles nothing. Box, dialog and
/// tool replays against the model.
#[test]
fn a_late_result_settles_nothing_as_the_model_says() {
    let late = |fx: &dyn Fixture, ticks: usize| {
        let mut steps = vec![appear(fx)];
        steps.extend(std::iter::repeat_n(tick(), ticks));
        steps.extend([GuardianStep::LateResult, tick(), tick()]);
        steps
    };
    replay(&BoxFx, &late(&BoxFx, 1));
    replay(&BoxFx, &late(&BoxFx, 2));
    replay(&DialogFx, &late(&DialogFx, 1));
    replay(&PickFx, &late(&PickFx, 1));
    if data_available() {
        replay(&LostToolFx, &late(&LostToolFx, 1));
        replay(&EvadeFx, &late(&EvadeFx, 2));
    }
}

// --- The saved Fight (design V11D 3, 4 and 6) ---------------------------------

/// The saved Fight raises a hostile actor the Host fights through a native
/// child: the model's `OwnershipFight` rules against the real Guardian over
/// the claim port's child. Every model state within seven steps is replayed:
/// the publications, the published gate token and the child the slot holds
/// agree with the model after every step, the host solver sends nothing while
/// the child holds the event, and no child outlives the family that offered it.
#[test]
fn a_saved_fight_follows_the_ownership_model() {
    run_family(&FightFx, 7);
}

/// A hostile actor of the listed events: the Guardian fights it under a saved
/// Fight and flees it under a saved Flee.
fn fight() -> GuardianStep {
    GuardianStep::Appear {
        family: Family::Fight,
        forced_host: false,
    }
}

/// A saved Fight against an event of another family: the other event wins
/// the detector or replaces the fight, the retained actor returns or is gone,
/// and a session replaces the Guardian. The child closes with its owner in the
/// same pass and Fight never restarts for that actor.
fn fight_pair_traces(b: &dyn Fixture) -> Vec<Vec<GuardianStep>> {
    vec![
        vec![
            fight(),
            tick(),
            overlap_with(b),
            tick(),
            tick(),
            GuardianStep::Return,
            tick(),
            tick(),
            tick(),
        ],
        vec![
            fight(),
            tick(),
            replace_with(b),
            tick(),
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
        vec![
            fight(),
            tick(),
            overlap_with(b),
            tick(),
            GuardianStep::RetainedGone,
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
        vec![
            fight(),
            tick(),
            overlap_with(b),
            tick(),
            GuardianStep::SessionReplace,
            tick(),
            tick(),
            GuardianStep::Disappear,
            tick(),
        ],
    ]
}

#[test]
fn a_saved_fight_overlapped_or_replaced_by_every_other_family_follows_the_model() {
    if !data_available() {
        return;
    }
    let others: [&dyn Fixture; 2] = [&BoxFx, &MazeFx];
    let mut rig = Rig::new(real_data());
    for b in others {
        for steps in fight_pair_traces(b) {
            for fx in &event_fixtures() {
                fx.remove(&mut rig);
            }
            clear_inv(&mut rig.c);
            rig.dirty = false;
            rig.reset_pair(&FightFx, b);
            replay_mixed(&mut rig, &FightFx, b, &steps, true);
        }
    }
}

/// Under a saved Fight every hostile event is the host's, the excluded ones
/// (Swarm, the angry plant) included; once the saved policy is Flee a new
/// episode leaves the script its claim, an actor of the listed events
/// included, and the policy binds an episode at its first pass
/// (`MAIN-ACCEPTANCE.md` R3-1).
#[test]
fn the_saved_policy_gives_a_hostile_event_its_owner_as_the_model_says() {
    if !data_available() {
        return;
    }
    let claiming = GuardianStep::ScriptStart {
        wants: true,
        live: false,
    };
    let excluded = GuardianStep::Appear {
        family: Family::Evade,
        forced_host: false,
    };
    let mut rig = Rig::new(real_data());
    // A fight replaced by an excluded event under the saved Fight: the host
    // keeps it and flees; the script's claim does not take it.
    let replaced = [
        claiming.clone(),
        fight(),
        tick(),
        GuardianStep::Replace {
            family: Family::Evade,
            forced_host: false,
        },
        tick(),
        tick(),
    ];
    rig.reset_pair(&FightFx, &FightFx);
    replay_mixed(&mut rig, &FightFx, &FightFx, &replaced, true);
    assert_eq!(rig.last.claim, RandomClaim::Host, "the host keeps it");
    // The saved policy is Flee: a new hostile episode, an actor of the listed
    // events or not, is an ordinary Evade the script claims, and no Fight is
    // offered for it. A policy that flips to Flee before the Guardian first
    // sees the actor binds the episode the same way.
    for steps in [
        vec![GuardianStep::PolicyFlee, claiming.clone(), excluded.clone(), tick(), tick()],
        vec![GuardianStep::PolicyFlee, claiming.clone(), fight(), tick(), tick()],
        vec![claiming.clone(), fight(), GuardianStep::PolicyFlee, tick(), tick()],
    ] {
        rig.reset_pair(&FightFx, &FightFx);
        replay_mixed(&mut rig, &FightFx, &FightFx, &steps, true);
        assert_eq!(rig.last.claim, RandomClaim::Handle, "the script claims it");
        assert_eq!(rig.claims.fight.offers, 0, "no Fight under a saved Flee");
    }
    // A retained actor is bound again when it returns, by the policy then.
    let returned = [
        claiming,
        fight(),
        tick(),
        overlap_with(&BoxFx),
        tick(),
        GuardianStep::PolicyFlee,
        GuardianStep::Return,
        tick(),
        tick(),
    ];
    rig.reset_pair(&FightFx, &BoxFx);
    replay_mixed(&mut rig, &FightFx, &BoxFx, &returned, true);
    assert_eq!(rig.last.claim, RandomClaim::Handle);
}

/// The defects seeded back into the model, replayed against the real
/// Guardian: each model that restarts a spent Fight, keeps a revoked child,
/// keeps an orphan, or runs the escape clock under the fight disagrees with
/// the real code; the invariants of the model name the same defect first.
#[test]
fn a_model_that_gets_the_fight_wrong_fails_the_replay() {
    use std::panic::{catch_unwind, AssertUnwindSafe};
    if !data_available() {
        return;
    }
    let bound = usize::from(FightFx.bound());
    let exit = vec![
        GuardianStep::ScriptStart {
            wants: false,
            live: false,
        },
        fight(),
        tick(),
        GuardianStep::ScriptExit(ScriptExit::Stop),
        tick(),
    ];
    let orphan = vec![fight(), tick(), GuardianStep::Disappear, tick()];
    let restart = vec![
        fight(),
        tick(),
        overlap_with(&BoxFx),
        tick(),
        GuardianStep::Return,
        tick(),
    ];
    let mut clock = vec![fight()];
    clock.extend(std::iter::repeat_n(tick(), bound + 2));
    let cases: [(GuardianMutation, &[GuardianStep], &str); 4] = [
        (GuardianMutation::FightSurvivesExit, &exit, "fight-revoked-runs"),
        (GuardianMutation::FightOrphanKept, &orphan, "orphan-child"),
        (GuardianMutation::FightRestarts, &restart, "fight-restart"),
        (GuardianMutation::FightTimeCounts, &clock, "fight-clock"),
    ];
    let mut rig = Rig::new(real_data());
    for (mutation, steps, invariant) in cases {
        for (check_model, expected) in [(false, "vs"), (true, invariant)] {
            rig.reset_pair(&FightFx, &BoxFx);
            let failure = catch_unwind(AssertUnwindSafe(|| {
                replay_with(&mut rig, &FightFx, &BoxFx, steps, mutation, check_model);
            }))
            .expect_err("the seeded model must fail");
            let message = failure
                .downcast_ref::<String>()
                .expect("a formatted panic message");
            assert!(
                message.contains(expected),
                "{mutation:?} {expected}: {message}"
            );
        }
        // The unseeded model replays the same trace.
        rig.reset_pair(&FightFx, &BoxFx);
        replay_mixed(&mut rig, &FightFx, &BoxFx, steps, true);
    }
}
