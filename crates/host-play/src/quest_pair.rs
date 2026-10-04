//! Play-owned bounded pair leases. Never takes a slot lock or retains a world.
use api::quest_progress::EvidenceStamp;
use api::selected::{Knowledge, RunKey};
use script::quester::pair::*;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

const INACTIVITY: Duration = Duration::from_secs(10 * 60);
const TOTAL: Duration = Duration::from_secs(60 * 60);
const MAX_PHASES: usize = 128;

#[derive(Clone, PartialEq, Eq)]
struct World {
    host: Arc<str>,
    port: u16,
}
struct Entry {
    world: World,
    registration: PairRegistration,
    lease: Option<u64>,
    cancelled: bool,
    gang: Option<(Knowledge<Option<Gang>>, EvidenceStamp)>,
}
struct Joined {
    role: usize,
    evidence: EvidenceStamp,
}
struct Lease {
    token: PairToken,
    plan: Arc<CompiledPairPlan>,
    joined: [Option<Joined>; 2],
    receipts: [Option<EvidenceStamp>; 2],
    offer: [Option<EvidenceStamp>; 2],
    confirm: [Option<EvidenceStamp>; 2],
    command: [u64; 2],
    actions: [Option<script::native::ActionRevoker>; 2],
    done_seen: [bool; 2],
    deadline: Instant,
    total_deadline: Instant,
    progress: [Option<EvidenceStamp>; 2],
}
impl Lease {
    fn side(&self, actor: RunKey) -> Result<usize, PairError> {
        if actor == self.token.left { Ok(0) }
        else if actor == self.token.right { Ok(1) }
        else { Err(PairError::Stale) }
    }
    fn complete(&self) -> bool {
        self.receipts.iter().all(Option::is_some)
    }
}
#[derive(Default)]
struct State {
    profiles: HashSet<AccountKey>,
    accounts: Arc<[AccountKey]>,
    authoritative: bool,
    entries: HashMap<AccountKey, Entry>,
    leases: HashMap<u64, Lease>,
    next: u64,
}
#[derive(Default)]
pub struct QuestPairCoordinator {
    state: Mutex<State>,
}
impl QuestPairCoordinator {
    pub(crate) fn remember(&self, account: &str) {
        let mut state = self.state.lock().unwrap();
        if !state.authoritative && !state.profiles.contains(account) {
            state.profiles.insert(AccountKey(Arc::from(account)));
            Self::refresh_accounts(&mut state);
        }
    }
    fn refresh_accounts(state: &mut State) {
        let mut accounts: Vec<_> = state.profiles.iter().cloned().collect();
        accounts.sort_unstable_by(|left, right| left.0.cmp(&right.0));
        state.accounts = accounts.into();
    }
    pub(crate) fn accounts(&self) -> Arc<[AccountKey]> {
        Arc::clone(&self.state.lock().unwrap().accounts)
    }
    pub(crate) fn set_accounts<'a>(&self, names: impl IntoIterator<Item = &'a str>) {
        let mut state = self.state.lock().unwrap();
        let profiles: HashSet<AccountKey> = names.into_iter().map(|name| {
            state.profiles.get(name).cloned().unwrap_or_else(|| AccountKey(Arc::from(name)))
        }).collect();
        state.authoritative = true;
        if state.profiles == profiles { return; }
        let mut retired = [0u64; MAX_PHASES * 2];
        let mut count = 0;
        state.entries.retain(|account, entry| {
            if profiles.contains(account) { return true; }
            if let Some(id) = entry.lease {
                // A bounded phase has at most two account entries.
                retired[count] = id;
                count += 1;
            }
            false
        });
        for id in &retired[..count] { Self::cancel_locked(&mut state, *id); }
        state.profiles = profiles;
        Self::refresh_accounts(&mut state);
    }
    pub(crate) fn seat(self: &Arc<Self>, account: &str, host: &str, port: u16) -> Arc<dyn QuestPairPort> {
        Arc::new_cyclic(|weak| Seat {
            coordinator: Arc::clone(self),
            account: AccountKey(Arc::from(account)),
            world: Mutex::new(World { host: Arc::from(host), port }),
            shared: weak.clone(),
        })
    }
    fn cancel_locked(state: &mut State, id: u64) {
        let complete = state.leases.get(&id).is_some_and(Lease::complete);
        if complete {
            for entry in state.entries.values_mut() {
                if entry.lease == Some(id) { entry.lease = None; }
            }
            return;
        }
        let Some(lease) = state.leases.remove(&id) else { return; };
        for action in lease.actions.iter().flatten() {
            action.revoke();
        }
        for entry in state.entries.values_mut() {
            if entry.registration.run == lease.token.left || entry.registration.run == lease.token.right {
                entry.cancelled = true;
                entry.lease = None;
            }
        }
    }
    fn validate(state: &State, token: &PairToken, actor: RunKey) -> Result<(), PairError> {
        let lease = state.leases.get(&token.id).ok_or(PairError::Cancelled)?;
        if lease.token != *token { return Err(PairError::Stale); }
        lease.side(actor)?;
        if lease.complete() { return Ok(()); }
        for run in [token.left, token.right] {
            let entry = state.entries.values().find(|entry| entry.registration.run == run)
                .ok_or(PairError::Stale)?;
            if entry.cancelled { return Err(PairError::Cancelled); }
            if !entry.registration.ready { return Err(PairError::NotReady); }
        }
        Ok(())
    }
    fn validate_live(state: &mut State, token: &PairToken, actor: RunKey) -> Result<(), PairError> {
        Self::validate(state, token, actor)?;
        let lease = state.leases.get(&token.id).unwrap();
        let now = Instant::now();
        if !lease.complete() && (now >= lease.deadline || now >= lease.total_deadline) {
            Self::cancel_locked(state, token.id);
            return Err(PairError::BarrierExpired);
        }
        Ok(())
    }
    fn begin_at(&self, account: &AccountKey, request: PairRequest, now: Instant) -> Result<PairToken, PairError> {
        let mut state = self.state.lock().unwrap();
        if account == &request.partner { return Err(PairError::SelfPartner); }
        if !state.profiles.contains(account) || !state.profiles.contains(&request.partner) {
            return Err(PairError::UnknownAccount);
        }
        let own = state.entries.get(account).ok_or(PairError::PartnerNotInPlay)?;
        let peer = state.entries.get(&request.partner).ok_or(PairError::PartnerNotInPlay)?;
        if own.registration.run != request.caller || request.evidence.run != request.caller {
            return Err(PairError::Stale);
        }
        if own.cancelled || peer.cancelled { return Err(PairError::Cancelled); }
        if !own.registration.ready || !peer.registration.ready { return Err(PairError::NotReady); }
        if !own.registration.evidence.meets(request.evidence) { return Err(PairError::Stale); }
        let settings = own.registration.settings.as_ref().ok_or(PairError::PartnerNotInPlay)?;
        let peer_settings = peer.registration.settings.as_ref().ok_or(PairError::PartnerNotInPlay)?;
        if settings.partner.as_ref() != Some(&request.partner) || peer_settings.partner.as_ref() != Some(account) {
            return Err(PairError::MissingPartner);
        }
        if request.declared_gang != settings.gang { return Err(PairError::WrongGang); }
        if own.world != peer.world { return Err(PairError::DifferentWorld); }
        if own.registration.pin != peer.registration.pin { return Err(PairError::WrongPin); }
        if request.phase != request.plan.phase { return Err(PairError::Stale); }
        match (&own.gang, &request.observed_gang) {
            (Some((Knowledge::Known(owned), evidence)), Knowledge::Known(reported))
                if owned == reported && evidence.run == request.caller => {}
            _ => return Err(PairError::UnknownGang),
        }
        let gang = match request.observed_gang {
            Knowledge::Known(Some(observed)) => {
                if settings.gang.is_some_and(|declared| declared != observed) { return Err(PairError::WrongGang); }
                observed
            }
            Knowledge::Known(None) => settings.gang.ok_or(PairError::WrongGang)?,
            Knowledge::Unknown(_) | Knowledge::Partial { .. } => return Err(PairError::UnknownGang),
        };
        let role = request.plan.roles.iter().position(|role| role.gang == gang && role.id == request.caller_role)
            .ok_or(PairError::WrongGang)?;
        let peer_run = peer.registration.run;
        let existing = own.lease.or(peer.lease);
        if own.lease.is_some() && peer.lease.is_some() && own.lease != peer.lease { return Err(PairError::Busy); }
        let (left, right) = if request.caller.slot < peer_run.slot {
            (request.caller, peer_run)
        } else { (peer_run, request.caller) };
        let side = usize::from(request.caller == right);
        if let Some(id) = existing {
            let lease = state.leases.get_mut(&id).ok_or(PairError::Stale)?;
            if lease.token.left != left || lease.token.right != right { return Err(PairError::Busy); }
            if now >= lease.deadline || now >= lease.total_deadline {
                Self::cancel_locked(&mut state, id);
                return Err(PairError::BarrierExpired);
            }
            if lease.plan.path != request.plan.path || lease.plan.protocol != request.plan.protocol
                || lease.plan.digest != request.plan.digest || lease.plan.phase != request.phase
                || lease.plan.signature != request.plan.signature {
                return Err(PairError::NotReady);
            }
            if let Some(joined) = &lease.joined[side] {
                if joined.role != role { return Err(PairError::WrongGang); }
                return Ok(lease.token);
            }
            if lease.joined[1 - side].as_ref().is_some_and(|joined| joined.role == role) {
                Self::cancel_locked(&mut state, id);
                return Err(PairError::WrongGang);
            }
            lease.joined[side] = Some(Joined { role, evidence: request.evidence });
            if lease.plan.actions.is_none() {
                lease.receipts = [lease.joined[0].as_ref().map(|j| j.evidence), lease.joined[1].as_ref().map(|j| j.evidence)];
            }
            let token = lease.token;
            state.entries.get_mut(account).unwrap().lease = Some(id);
            Ok(token)
        } else {
            let State { entries, leases, .. } = &mut *state;
            leases.retain(|_, lease| !lease.complete() || (
                !lease.done_seen.iter().all(|seen| *seen)
                && [lease.token.left, lease.token.right].iter().all(|run| {
                    entries.values().any(|entry| entry.registration.run == *run)
                })
            ));
            if state.leases.len() >= MAX_PHASES { return Err(PairError::Busy); }
            state.next = state.next.checked_add(1).ok_or(PairError::Busy)?;
            let id = state.next;
            let token = PairToken { id, generation: id, left, right };
            let mut joined = [None, None];
            joined[side] = Some(Joined { role, evidence: request.evidence });
            let command_base = id.checked_mul(2).ok_or(PairError::Busy)?;
            state.leases.insert(id, Lease {
                token, plan: request.plan, joined, receipts: [None, None],
                offer: [None, None], confirm: [None, None],
                command: [command_base, command_base + 1], done_seen: [false, false],
                actions: [None, None],
                deadline: now + INACTIVITY, total_deadline: now + TOTAL, progress: [None, None],
            });
            state.entries.get_mut(account).unwrap().lease = Some(id);
            state.entries.get_mut(&request.partner).unwrap().lease = Some(id);
            Ok(token)
        }
    }
    pub(crate) fn gameplay_progress(&self, actor: RunKey, evidence: EvidenceStamp, now: Instant) {
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state.entries.values().find(|entry| entry.registration.run == actor) else { return; };
        if !entry.registration.ready || evidence.run != actor || !entry.registration.evidence.meets(evidence) { return; }
        let Some(id) = entry.lease else { return; };
        let Some(lease) = state.leases.get_mut(&id) else { return; };
        let Ok(side) = lease.side(actor) else { return; };
        if lease.complete() || now >= lease.deadline || now >= lease.total_deadline { return; }
        if lease.progress[side].is_some_and(|before| !evidence.meets(before) || evidence == before) { return; }
        lease.progress[side] = Some(evidence);
        lease.deadline = (now + INACTIVITY).min(lease.total_deadline);
    }
}
struct Seat {
    coordinator: Arc<QuestPairCoordinator>,
    account: AccountKey,
    world: Mutex<World>,
    shared: std::sync::Weak<Seat>,
}
impl Seat {
    fn actor(&self, state: &State, actor: RunKey) -> Result<(), PairError> {
        if state.entries.get(&self.account).is_some_and(|entry| entry.registration.run == actor) {
            Ok(())
        } else {
            Err(PairError::Stale)
        }
    }
}
impl QuestPairPort for Seat {
    fn shared(&self) -> Arc<dyn QuestPairPort> {
        self.shared.upgrade().expect("live pair seat")
    }
    fn observe(&self, registration: PairRegistration) {
        let world = self.world.lock().unwrap().clone();
        let mut state = self.coordinator.state.lock().unwrap();
        if registration.evidence.run != registration.run { return; }
        if !state.profiles.contains(&self.account) { return; }
        if let Some(old) = state.entries.get(&self.account) {
            let changed = old.registration.run != registration.run || old.registration.pin != registration.pin
                || old.registration.settings != registration.settings || old.world != world || !registration.ready;
            if changed {
                if let Some(id) = old.lease { QuestPairCoordinator::cancel_locked(&mut state, id); }
            } else if !registration.evidence.meets(old.registration.evidence) { return; }
        }
        let old = state.entries.remove(&self.account);
        let (lease, cancelled, gang) = old
            .filter(|old| old.registration.run == registration.run && old.registration.pin == registration.pin)
            .map_or((None, false, None), |old| (old.lease, old.cancelled, old.gang));
        state.entries.insert(self.account.clone(), Entry { world, registration, lease, cancelled, gang });
    }
    fn invalidate(&self, run: RunKey) {
        let mut state = self.coordinator.state.lock().unwrap();
        let Some(entry) = state.entries.get(&self.account).filter(|entry| entry.registration.run == run) else { return; };
        if let Some(id) = entry.lease { QuestPairCoordinator::cancel_locked(&mut state, id); }
        if let Some(entry) = state.entries.get_mut(&self.account) { entry.registration.ready = false; }
    }
    fn busy(&self) -> bool {
        self.coordinator.state.lock().unwrap().entries.get(&self.account).is_some_and(|entry| entry.lease.is_some())
    }
    fn world_changed(&self, host: &str, port: u16) {
        let mut world = self.world.lock().unwrap();
        if world.host.as_ref() == host && world.port == port { return; }
        *world = World { host: Arc::from(host), port };
        drop(world);
        let run = self.coordinator.state.lock().unwrap().entries.get(&self.account).map(|entry| entry.registration.run);
        if let Some(run) = run { self.invalidate(run); }
    }
    fn settings(&self, caller: RunKey) -> Result<PairSettings, PairError> {
        let state = self.coordinator.state.lock().unwrap();
        let entry = state.entries.get(&self.account).ok_or(PairError::NotReady)?;
        if entry.registration.run != caller { return Err(PairError::Stale); }
        entry.registration.settings.clone().ok_or(PairError::PartnerNotInPlay)
    }
    fn observe_gang(&self, read: &api::quest_progress::JournalRead) -> Result<Knowledge<Option<Gang>>, PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, read.closed.run)?;
        let entry = state.entries.get_mut(&self.account).unwrap();
        if read.quest.0.as_ref() != "blackarmgang" || read.pin != entry.registration.pin
            || !read.closed.meets(read.acquired) || !entry.registration.evidence.meets(read.closed)
            || !entry.registration.ready {
            return Err(PairError::Stale);
        }
        let evidence = script::quester::gang::resolve(read);
        let gang = evidence.gang;
        entry.gang = Some((gang.clone(), read.closed));
        Ok(gang)
    }
    fn gang(&self, caller: RunKey) -> Result<(Knowledge<Option<Gang>>, EvidenceStamp), PairError> {
        let state = self.coordinator.state.lock().unwrap();
        self.actor(&state, caller)?;
        state.entries.get(&self.account).unwrap().gang.clone().ok_or(PairError::UnknownGang)
    }
    fn waiting(&self, caller: RunKey) -> bool {
        let state = self.coordinator.state.lock().unwrap();
        if self.actor(&state, caller).is_err() { return false; }
        let Some(lease) = state.entries.get(&self.account).and_then(|entry| entry.lease)
            .and_then(|id| state.leases.get(&id)) else { return false; };
        let Ok(side) = lease.side(caller) else { return false; };
        !lease.complete() && (lease.joined.iter().any(Option::is_none) || lease.receipts[side].is_some())
    }
    fn token(&self, caller: RunKey, phase: &api::selected::FactKey) -> Result<PairToken, PairError> {
        let state = self.coordinator.state.lock().unwrap();
        self.actor(&state, caller)?;
        let lease = state.entries.get(&self.account).and_then(|entry| entry.lease)
            .and_then(|id| state.leases.get(&id)).ok_or(PairError::Stale)?;
        if &lease.plan.phase != phase { return Err(PairError::Stale); }
        QuestPairCoordinator::validate(&state, &lease.token, caller)?;
        Ok(lease.token)
    }
    fn register_action(&self, token: &PairToken, actor: RunKey, action: script::native::ActionRevoker) -> Result<(), PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, actor)?;
        QuestPairCoordinator::validate_live(&mut state, token, actor)?;
        if action.run() != actor { return Err(PairError::Stale); }
        let lease = state.leases.get_mut(&token.id).unwrap();
        if lease.complete() { return Err(PairError::Stale); }
        let side = lease.side(actor)?;
        lease.actions[side] = Some(action);
        Ok(())
    }
    fn begin(&self, request: PairRequest) -> Result<PairToken, PairError> {
        self.coordinator.begin_at(&self.account, request, Instant::now())
    }
    fn poll(&self, token: &PairToken, caller: RunKey) -> Poll<Result<PairStep, PairError>> {
        let mut state = self.coordinator.state.lock().unwrap();
        if let Err(error) = self.actor(&state, caller) { return Poll::Ready(Err(error)); }
        if let Err(error) = QuestPairCoordinator::validate_live(&mut state, token, caller) { return Poll::Ready(Err(error)); }
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(caller).unwrap();
        if lease.complete() {
            let receipt = PairReceipt { token: *token, phase: lease.plan.phase.clone(), evidence: [lease.receipts[0].unwrap(), lease.receipts[1].unwrap()] };
            lease.done_seen[side] = true;
            let forget = lease.done_seen.iter().all(|seen| *seen);
            for entry in state.entries.values_mut() {
                if entry.lease == Some(token.id) { entry.lease = None; }
            }
            if forget { state.leases.remove(&token.id); }
            return Poll::Ready(Ok(PairStep::Done(receipt)));
        }
        if lease.joined.iter().all(Option::is_some) && lease.receipts[side].is_none() {
            let role = lease.joined[side].as_ref().unwrap().role;
            if let Some(actions) = &lease.plan.actions {
                return Poll::Ready(Ok(PairStep::Act(RoleCommand {
                    token: *token, recipient: caller, phase: lease.plan.phase.clone(),
                    command_id: lease.command[side], plan: Arc::clone(&actions[role]),
                })));
            }
        }
        Poll::Ready(Ok(PairStep::Waiting { phase: lease.plan.phase.clone(), deadline: lease.deadline }))
    }
    fn report(&self, token: &PairToken, receipt: RoleReceipt) -> Result<(), PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, receipt.actor)?;
        QuestPairCoordinator::validate_live(&mut state, token, receipt.actor)?;
        let observed = state.entries.get(&self.account).unwrap().registration.evidence;
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(receipt.actor)?;
        if lease.joined.iter().any(Option::is_none) || lease.command[side] != receipt.command_id { return Err(PairError::Stale); }
        if lease.receipts[side].is_some() { return Err(PairError::Stale); }
        match receipt.outcome {
            Ok(outcome) => {
                let before = lease.joined[side].as_ref().unwrap().evidence;
                if outcome.evidence.run != receipt.actor || !outcome.evidence.meets(before)
                    || outcome.evidence == before || !observed.meets(outcome.evidence) {
                    return Err(PairError::Stale);
                }
                lease.receipts[side] = Some(outcome.evidence);
                Ok(())
            }
            Err(_) => { QuestPairCoordinator::cancel_locked(&mut state, token.id); Err(PairError::Cancelled) }
        }
    }
    fn trade_ready(&self, token: &PairToken, actor: RunKey, confirm: bool, evidence: EvidenceStamp) -> Result<bool, PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, actor)?;
        QuestPairCoordinator::validate_live(&mut state, token, actor)?;
        let entry = state.entries.values().find(|entry| entry.registration.run == actor).ok_or(PairError::Stale)?;
        if evidence.run != actor || !entry.registration.evidence.meets(evidence) { return Err(PairError::Stale); }
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(actor)?;
        if lease.joined.iter().any(Option::is_none) { return Err(PairError::NotReady); }
        if confirm && lease.offer.iter().any(Option::is_none) { return Err(PairError::NotReady); }
        let screen = if confirm { &mut lease.confirm } else { &mut lease.offer };
        if screen[side].is_some_and(|before| !evidence.meets(before)) { return Err(PairError::Stale); }
        screen[side] = Some(evidence);
        Ok(screen.iter().all(Option::is_some))
    }
    fn gameplay_progress(&self, actor: RunKey, evidence: EvidenceStamp, now: Instant) {
        let owns = self.actor(&self.coordinator.state.lock().unwrap(), actor).is_ok();
        if owns { self.coordinator.gameplay_progress(actor, evidence, now); }
    }
    fn cancel(&self, token: &PairToken) {
        let mut state = self.coordinator.state.lock().unwrap();
        let owns = state.entries.get(&self.account).is_some_and(|entry| {
            entry.registration.run == token.left || entry.registration.run == token.right
        });
        if owns && state.leases.get(&token.id).is_some_and(|lease| lease.token == *token) {
            QuestPairCoordinator::cancel_locked(&mut state, token.id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::JournalRead;
    use api::selected::{ClientRevision, FactKey};
    use script::native::ActionError;
    use script::quester::compile::{StepContext, StepOutcome, StepPlan, StepRun};

    fn stamp(run: RunKey, tick: u64) -> EvidenceStamp {
        EvidenceStamp { run, tick, sequence: tick }
    }
    fn pin() -> Arc<api::selected::SelectedPin> {
        api::game_data::for_revision(ClientRevision::R289).unwrap().selected_pin().unwrap()
    }
    fn register(port: &dyn QuestPairPort, run: RunKey, partner: &str, gang: Gang, tick: u64) {
        port.observe(PairRegistration {
            run, pin: pin(), settings: Some(PairSettings { partner: Some(AccountKey(Arc::from(partner))), gang: Some(gang) }),
            ready: true, evidence: stamp(run, tick),
        });
    }
    fn read(run: RunKey) -> JournalRead {
        JournalRead {
            quest: FactKey::new("blackarmgang"), root: 8134,
            lines: Arc::from([Arc::from("I can start this quest by speaking to Reldo in Varrock's Palace Library, or by speaking to the Tramp near the Blue Moon Inn.")]),
            colour: Some(0), acquired: stamp(run, 1), closed: stamp(run, 2), pin: pin(),
        }
    }
    struct TestAction;
    impl StepPlan for TestAction {
        fn begin(&self, _: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError> {
            Err(ActionError::Unavailable(Arc::from("test sequencer does not execute gameplay")))
        }
    }
    fn plan(actions: bool) -> Arc<CompiledPairPlan> {
        Arc::new(CompiledPairPlan {
            path: FactKey::new("blackarmgang"), protocol: FactKey::new("arrav"), digest: [1; 32],
            phase: FactKey::new(if actions { "arrav:key" } else { "arrav:admission" }),
            roles: [
                PartnerRole { id: FactKey::new("phoenix"), gang: Gang::Phoenix },
                PartnerRole { id: FactKey::new("blackarm"), gang: Gang::BlackArm },
            ],
            signature: [2; 32],
            actions: actions.then(|| [Arc::new(TestAction) as Arc<dyn StepPlan>, Arc::new(TestAction)]),
        })
    }
    fn request(run: RunKey, partner: &str, role: usize, plan: &Arc<CompiledPairPlan>) -> PairRequest {
        PairRequest {
            caller: run, partner: AccountKey(Arc::from(partner)), caller_role: plan.roles[role].id.clone(),
            phase: plan.phase.clone(), plan: Arc::clone(plan), observed_gang: Knowledge::Known(None),
            declared_gang: Some(plan.roles[role].gang), evidence: stamp(run, 2),
        }
    }
    struct Pair {
        core: Arc<QuestPairCoordinator>,
        seats: [Arc<dyn QuestPairPort>; 2],
        runs: [RunKey; 2],
    }
    fn pair() -> Pair {
        let core = Arc::new(QuestPairCoordinator::default());
        core.remember("alice"); core.remember("bob");
        let seats = [core.seat("alice", "127.0.0.1", 44594), core.seat("bob", "127.0.0.1", 44594)];
        let runs = [RunKey { slot: 1, run: 1, session: 1 }, RunKey { slot: 2, run: 1, session: 1 }];
        register(seats[0].as_ref(), runs[0], "bob", Gang::Phoenix, 2);
        register(seats[1].as_ref(), runs[1], "alice", Gang::BlackArm, 2);
        for side in 0..2 {
            assert!(matches!(seats[side].observe_gang(&read(runs[side])), Ok(Knowledge::Known(None))));
        }
        Pair { core, seats, runs }
    }
    #[test]
    fn reciprocal_owned_admission_never_dispatches_before_both_join() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        assert!(matches!(pair.seats[0].poll(&token, pair.runs[0]), Poll::Ready(Ok(PairStep::Waiting { .. }))));
        assert!(pair.seats[0].busy());
        assert!(matches!(pair.seats[0].poll(&token, pair.runs[1]), Poll::Ready(Err(PairError::Stale))));
        assert_eq!(pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)).unwrap(), token);
        for side in 0..2 {
            assert!(matches!(pair.seats[side].poll(&token, pair.runs[side]), Poll::Ready(Ok(PairStep::Done(_)))));
        }
        assert!(!pair.seats[0].busy() && !pair.seats[1].busy());
    }
    #[test]
    fn forged_gang_same_gang_and_nonreciprocal_accounts_cannot_admit() {
        let pair = pair();
        let plan = plan(false);
        let mut forged = request(pair.runs[0], "bob", 0, &plan);
        forged.observed_gang = Knowledge::Known(Some(Gang::Phoenix));
        assert_eq!(pair.seats[0].begin(forged), Err(PairError::UnknownGang));
        register(pair.seats[1].as_ref(), pair.runs[1], "other", Gang::BlackArm, 2);
        assert_eq!(pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)), Err(PairError::MissingPartner));
        register(pair.seats[1].as_ref(), pair.runs[1], "alice", Gang::Phoenix, 2);
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        assert_eq!(pair.seats[1].begin(request(pair.runs[1], "alice", 0, &plan)), Err(PairError::WrongGang));
        assert!(matches!(pair.seats[0].poll(&token, pair.runs[0]), Poll::Ready(Err(PairError::Cancelled))));
    }
    #[test]
    fn stop_revokes_peer_and_old_session_journal_cannot_rearm() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        pair.seats[0].invalidate(pair.runs[0]);
        assert!(matches!(pair.seats[1].poll(&token, pair.runs[1]), Poll::Ready(Err(PairError::Cancelled))));
        assert_eq!(pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)), Err(PairError::Cancelled));
        let mut next = pair.runs[0]; next.session += 1;
        register(pair.seats[0].as_ref(), next, "bob", Gang::Phoenix, 3);
        assert!(matches!(pair.seats[0].observe_gang(&read(pair.runs[0])), Err(PairError::Stale)));
    }
    #[test]
    fn only_new_gameplay_feeds_rearm_and_total_phase_bound_never_moves() {
        let pair = pair();
        let plan = plan(false);
        let now = Instant::now();
        let token = pair.core.begin_at(&AccountKey(Arc::from("alice")), request(pair.runs[0], "bob", 0, &plan), now).unwrap();
        pair.seats[0].gameplay_progress(pair.runs[0], stamp(pair.runs[0], 2), now + Duration::from_secs(300));
        let first = pair.core.state.lock().unwrap().leases[&token.id].deadline;
        pair.seats[0].gameplay_progress(pair.runs[0], stamp(pair.runs[0], 2), now + Duration::from_secs(500));
        assert_eq!(pair.core.state.lock().unwrap().leases[&token.id].deadline, first);
        for step in 2..=11 {
            let tick = step + 1;
            register(pair.seats[0].as_ref(), pair.runs[0], "bob", Gang::Phoenix, tick);
            pair.seats[0].gameplay_progress(pair.runs[0], stamp(pair.runs[0], tick), now + Duration::from_secs(step * 300));
        }
        let state = pair.core.state.lock().unwrap();
        let lease = &state.leases[&token.id];
        assert_eq!(lease.total_deadline, now + TOTAL);
        assert_eq!(lease.deadline, now + TOTAL);
    }
    fn command(port: &dyn QuestPairPort, token: &PairToken, run: RunKey) -> u64 {
        match port.poll(token, run) {
            Poll::Ready(Ok(PairStep::Act(command))) => command.command_id,
            _ => panic!("expected own correlated role action"),
        }
    }
    fn receipt(run: RunKey, command_id: u64, tick: u64) -> RoleReceipt {
        RoleReceipt { actor: run, command_id, outcome: Ok(StepOutcome { progress: None, evidence: stamp(run, tick), receipt: None }) }
    }
    #[test]
    fn role_commands_deduplicate_and_stale_duplicate_or_foreign_receipts_never_advance() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)).unwrap();
        let ids = [command(pair.seats[0].as_ref(), &token, pair.runs[0]), command(pair.seats[1].as_ref(), &token, pair.runs[1])];
        assert_eq!(command(pair.seats[0].as_ref(), &token, pair.runs[0]), ids[0]);
        assert_eq!(pair.seats[0].report(&token, receipt(pair.runs[1], ids[1], 3)), Err(PairError::Stale));
        assert_eq!(pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 2)), Err(PairError::Stale));
        assert_eq!(pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 3)), Err(PairError::Stale));
        register(pair.seats[0].as_ref(), pair.runs[0], "bob", Gang::Phoenix, 3);
        pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 3)).unwrap();
        assert_eq!(pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 3)), Err(PairError::Stale));
        register(pair.seats[1].as_ref(), pair.runs[1], "alice", Gang::BlackArm, 3);
        pair.seats[1].report(&token, receipt(pair.runs[1], ids[1], 3)).unwrap();
        assert!(matches!(pair.seats[0].poll(&token, pair.runs[0]), Poll::Ready(Ok(PairStep::Done(_)))));
    }
    #[test]
    fn configured_roster_includes_saved_accounts_and_removal_revokes_the_pair() {
        let pair = pair();
        pair.core.set_accounts(["saved", "bob", "alice", "saved"]);
        let accounts = pair.core.accounts();
        assert_eq!(accounts.iter().map(|account| account.0.as_ref()).collect::<Vec<_>>(), ["alice", "bob", "saved"]);
        assert!(Arc::ptr_eq(&accounts, &pair.core.accounts()));
        let plan = plan(false);
        assert_eq!(pair.seats[0].begin(request(pair.runs[0], "saved", 0, &plan)), Err(PairError::PartnerNotInPlay));
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        pair.core.set_accounts(["alice", "saved"]);
        assert!(matches!(pair.seats[0].poll(&token, pair.runs[0]), Poll::Ready(Err(PairError::Cancelled))));
        pair.core.remember("bob");
        register(pair.seats[1].as_ref(), pair.runs[1], "alice", Gang::BlackArm, 3);
        assert_eq!(pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)), Err(PairError::UnknownAccount));
        assert_eq!(pair.core.accounts().len(), 2);
    }

    #[test]
    fn late_role_receipt_expires_and_revokes_before_it_can_complete() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)).unwrap();
        pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)).unwrap();
        let id = command(pair.seats[0].as_ref(), &token, pair.runs[0]);
        register(pair.seats[0].as_ref(), pair.runs[0], "bob", Gang::Phoenix, 3);
        pair.core.state.lock().unwrap().leases.get_mut(&token.id).unwrap().deadline = Instant::now();
        assert_eq!(pair.seats[0].report(&token, receipt(pair.runs[0], id, 3)), Err(PairError::BarrierExpired));
        assert!(matches!(pair.seats[1].poll(&token, pair.runs[1]), Poll::Ready(Err(PairError::Cancelled))));
    }
}
