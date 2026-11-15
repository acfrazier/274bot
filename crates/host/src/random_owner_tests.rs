// V11a: one owner per Guardian event.
//
// The Guardian is replayed through traces of the checked ownership model
// (`tools/seat-model/src/guardian.rs`). After every step the model and the real
// Guardian must publish the same `claim`, `handling`, `ours` and `hold`, and the
// invariants of the model hold over the real observations: the publications
// equal their derivation, a script that stops, pauses or fails returns its claim
// within one tick, the toggle off publishes nothing, and the host emits nothing
// while a script owns the event.
use seat_model::guardian::{
    derive as model_derive, Family, GuardianBounds, GuardianMutation, GuardianState, GuardianStep,
    PublicationInputs as ModelInputs, ScriptExit,
};
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
struct Claims {
    run: Run,
    wants: bool,
    live: bool,
    callbacks: u32,
}

impl Claims {
    fn answer(&self) -> RandomClaim {
        if self.run == Run::Running && self.wants {
            RandomClaim::Handle
        } else {
            RandomClaim::Host
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
    sigs: u32,
    last_sig: Option<String>,
    emissions: usize,
    /// The Guardian is new and has not published: the old status stands.
    unpublished: bool,
    /// The fixture planted something the reset has to take away again.
    dirty: bool,
}

impl Rig {
    /// Back to the start state for the next trace on the same client.
    fn reset(&mut self, fx: &dyn Fixture) {
        if self.dirty {
            fx.remove(self);
        }
        fx.chat_close(self);
        self.c.chat_text.fill(String::new());
        self.c.chat_username.fill(String::new());
        self.c.chat_type.fill(0);
        plant_player(&mut self.c, "Test", 0, 0);
        self.snap = GameSnapshot::new();
        self.g = Guardian::new();
        self.drv = FakeDriver::default();
        self.settings = ProfileSettings::default();
        self.policies.clear();
        self.claims = Claims {
            run: Run::Stopped,
            wants: false,
            live: false,
            callbacks: 0,
        };
        self.last = RandomStatus::default();
        self.sigs = 0;
        self.last_sig = None;
        self.emissions = 0;
        self.unpublished = false;
        self.dirty = false;
        fx.prepare(self);
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
            },
            data,
            last: RandomStatus::default(),
            sigs: 0,
            last_sig: None,
            emissions: 0,
            unpublished: false,
            dirty: false,
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
        status
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
    fn install(&self, rig: &mut Rig, id: u8, forced_host: bool);
    fn remove(&self, rig: &mut Rig);
    fn chat_close(&self, _rig: &mut Rig) {}
    /// The host's work for the event has just started.
    fn started(&self, _rig: &mut Rig) {}
    /// The start state of one trace on a new or reset client.
    fn prepare(&self, _rig: &mut Rig) {}
    fn replicate(&self, _rig: &mut Rig, _fresh: bool) {}
}

fn data_available() -> bool {
    api::test_support::game_data_available()
}

fn real_data() -> Option<Arc<api::game_data::SelectedGameData>> {
    data_available().then(event_game_data)
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
        plant_npc(
            &mut rig.c,
            usize::from(id),
            name,
            -1,
            Some("Greetings Test!"),
        );
    }
    fn started(&self, rig: &mut Rig) {
        // The server opens the chat after the Talk-to.
        open_chat(&mut rig.c);
    }
    fn remove(&self, rig: &mut Rig) {
        clear_npcs(&mut rig.c);
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
        Family::Solver
    }
    fn bound(&self) -> u8 {
        // The shortest give-up is nine fresh ticks; traces stay below it.
        100
    }
    fn needs_data(&self) -> bool {
        true
    }
    // The base keeps one latched attacker: a second one inherits the act
    // without a release tick, which the abstract replacement does not model.
    fn distinct_signatures(&self) -> bool {
        false
    }
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
        plant_npc_with_op(&mut rig.c, usize::from(id), name, 32768, None, "Attack");
    }
    fn remove(&self, rig: &mut Rig) {
        clear_npcs(&mut rig.c);
    }
}

struct MazeFx;
impl Fixture for MazeFx {
    fn name(&self) -> &'static str {
        "maze"
    }
    fn family(&self) -> Family {
        Family::Trap
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
        Family::Trap
    }
    fn bound(&self) -> u8 {
        // The mime stage has no solver budget: the hold ends off the square.
        100
    }
    fn distinct_signatures(&self) -> bool {
        false
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_player(&mut rig.c, "Test", 31 * 64, 74 * 64);
        plant_npc(&mut rig.c, 0, "Mime", -1, None);
    }
    fn remove(&self, rig: &mut Rig) {
        clear_npcs(&mut rig.c);
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
        100
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
        plant_loc(&mut rig.c, 510, "Smoking rock", 0, 0);
        plant_player(&mut rig.c, "Test", 10, 10);
    }
    fn install(&self, rig: &mut Rig, _id: u8, _forced: bool) {
        plant_loc(&mut rig.c, 510, "Smoking rock", 0, 0);
        plant_player(&mut rig.c, "Test", 0, 0);
    }
    fn remove(&self, rig: &mut Rig) {
        plant_player(&mut rig.c, "Test", 10, 10);
    }
}

fn plant_ent(rig: &mut Rig) {
    plant_inv_named(&mut rig.c, 1351, Some("Bronze axe"));
    plant_npc_typed_with_op(&mut rig.c, 0, 448, "Oak", -1, None, "Chop down");
    rig.c
        .local_player
        .as_mut()
        .expect("local player")
        .entity
        .face_entity = 0;
}

fn fixtures() -> Vec<Box<dyn Fixture>> {
    vec![
        Box::new(DialogFx),
        Box::new(BoxFx),
        Box::new(EvadeFx),
        Box::new(MazeFx),
        Box::new(MimeFx),
        Box::new(LampFx),
    ]
}

// --- The replay -------------------------------------------------------------

fn model_for(fx: &dyn Fixture) -> GuardianState {
    GuardianState::new(
        GuardianBounds {
            wait: fx.bound(),
            ids: 2,
            sessions: 2,
        },
        GuardianMutation::None,
    )
}

/// Apply one model step to the real Guardian. A fresh `Tick` runs one pass on a
/// new observed game tick; `Frame` runs one on the same one.
fn drive(fx: &dyn Fixture, rig: &mut Rig, model: &GuardianState, step: &GuardianStep) {
    // The model has applied the step already: its event is the real one.
    let id = model.event_id().unwrap_or(0);
    match step {
        GuardianStep::Appear { forced_host, .. } => {
            rig.dirty = true;
            fx.install(rig, id, *forced_host);
        }
        GuardianStep::Replace { forced_host, .. } => {
            rig.dirty = true;
            fx.remove(rig);
            fx.install(rig, id, *forced_host);
        }
        GuardianStep::Disappear => fx.remove(rig),
        GuardianStep::ScriptStart { wants, live } => {
            rig.claims.run = Run::Running;
            rig.claims.wants = *wants;
            rig.claims.live = *live;
        }
        GuardianStep::ScriptExit(ScriptExit::Pause) => rig.claims.run = Run::Paused,
        GuardianStep::ScriptExit(_) => rig.claims.run = Run::Stopped,
        GuardianStep::ScriptResume => rig.claims.run = Run::Running,
        GuardianStep::ScriptWants(wants) => rig.claims.wants = *wants,
        GuardianStep::SetEnabled(enabled) => rig.settings.random_events = *enabled,
        GuardianStep::SessionReplace => {
            rig.g = Guardian::new();
            rig.snap = GameSnapshot::new();
            rig.unpublished = true;
            rig.claims.run = Run::Stopped;
            fx.chat_close(rig);
        }
        GuardianStep::ChatClose => fx.chat_close(rig),
        GuardianStep::Replicate { fresh } => fx.replicate(rig, *fresh),
        GuardianStep::LateResult => {}
        GuardianStep::Tick { .. } => {
            let talking = rig.g.in_flight();
            rig.pass(true);
            if !talking && rig.g.in_flight() {
                fx.started(rig);
            }
        }
        GuardianStep::Frame => {
            rig.pass(false);
        }
        GuardianStep::Seat(_) => {}
    }
}

fn inputs_of(rig: &Rig, status: &RandomStatus) -> ModelInputs {
    let inputs = rig.g.last_inputs;
    ModelInputs {
        toggle: status.toggle,
        detected: status.kind.is_some(),
        claim_script: rig.g.claim() == RandomClaim::Handle,
        dialog_in_flight: rig.g.in_flight(),
        solver_in_flight: rig.g.acting(),
        stalled: inputs.inert_hold || inputs.inert_detect,
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
    // The publications are the derivation of the one active event.
    let derived = derive_publication(rig.g.active.as_ref(), rig.g.last_inputs);
    if (derived.claim, derived.handling, derived.ours, derived.hold)
        != (status.claim, status.handling, status.ours, status.hold)
    {
        fail(&format!(
            "publications differ from their derivation {derived:?}"
        ));
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
}

/// Model and real code agree on the published ownership.
fn assert_same(model: &GuardianState, rig: &Rig, step: &GuardianStep, trace: &[GuardianStep]) {
    let expected = model.publication();
    let status = &rig.last;
    let got = (
        status.claim == RandomClaim::Handle,
        status.handling,
        status.ours,
        status.hold,
    );
    let want = (
        expected.claim_script,
        expected.handling,
        expected.ours,
        expected.hold,
    );
    if got != want {
        panic!(
            "model {want:?} (claim_script, handling, ours, hold) vs real {got:?}\nstatus {status:?}\nreal active {:?} inputs {:?}\nafter {step:?}\nmodel {}\ntrace {trace:#?}",
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
        (derived.claim_script, derived.handling, derived.ours, derived.hold),
        got,
        "the model's derivation differs from the real publication\nafter {step:?}\ntrace {trace:#?}"
    );
}

fn replay(fx: &dyn Fixture, steps: &[GuardianStep]) {
    let mut rig = Rig::new(real_data());
    fx.prepare(&mut rig);
    replay_in(&mut rig, fx, steps);
}

fn replay_in(rig: &mut Rig, fx: &dyn Fixture, steps: &[GuardianStep]) {
    let mut model = model_for(fx);
    let mut done: Vec<GuardianStep> = Vec::new();
    for step in steps {
        model
            .apply(step)
            .unwrap_or_else(|error| panic!("{}: {step:?} is disabled: {error}", fx.name()));
        model
            .check()
            .unwrap_or_else(|v| panic!("{}: the model fails {v:?} at {step:?}", fx.name()));
        drive(fx, rig, &model, step);
        done.push(step.clone());
        check_real(rig, step, &done);
        assert_same(&model, rig, step, &done);
    }
    // The rising-edge callback runs once per event, never replayed.
    assert!(
        rig.claims.callbacks <= rig.sigs,
        "{}: {} callbacks for {} events",
        fx.name(),
        rig.claims.callbacks,
        rig.sigs
    );
}

fn alphabet(fx: &dyn Fixture) -> Vec<GuardianStep> {
    let family = fx.family();
    let mut out = vec![GuardianStep::Appear {
        family,
        forced_host: false,
    }];
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
    out.push(GuardianStep::Disappear);
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
        GuardianStep::Tick { blocked: false },
        GuardianStep::Frame,
    ]);
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
        // The shortest Evade give-up is nine fresh ticks; stay below it.
        let bound = usize::from(fx.bound()).min(3);
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
        let steps = vec![
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
        fx.install(&mut rig, 0, false);
        rig.dirty = true;
        for _ in 0..4 {
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
