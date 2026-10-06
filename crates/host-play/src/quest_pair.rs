//! Play-owned bounded pair leases. Never takes a slot lock or retains a world.
use api::quest_progress::EvidenceStamp;
use api::selected::{Knowledge, RunKey};
use script::quester::pair::*;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::task::Poll;
use std::time::{Duration, Instant};

const INACTIVITY: Duration = Duration::from_secs(10 * 60);
const TOTAL: Duration = Duration::from_secs(60 * 60);
const TOTAL_LIMIT_REASON: &str =
    "pair phase exceeded 60-minute wall-clock limit; Stop and Start both accounts";
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
    cancelled: Option<PairError>,
    admission_wait: bool,
    gang: Option<(Knowledge<Option<Gang>>, EvidenceStamp)>,
    ready_after: EvidenceStamp,
    identity: Option<u64>,
    binding: Option<Binding>,
    inventory: Option<ItemReceipt>,
}
impl Entry {
    fn effective_gang(&self) -> Result<Gang, PairError> {
        let settings = self
            .registration
            .settings
            .as_ref()
            .ok_or(PairError::PartnerNotInPlay)?;
        match self.gang.as_ref() {
            Some((Knowledge::Known(Some(gang)), evidence)) if evidence.meets(self.ready_after) => {
                if settings.gang.is_some_and(|declared| declared != *gang) {
                    return Err(PairError::WrongGang);
                }
                Ok(*gang)
            }
            Some((Knowledge::Known(None), evidence)) if evidence.meets(self.ready_after) => {
                settings.gang.ok_or(PairError::WrongGang)
            }
            _ => Err(PairError::UnknownGang),
        }
    }
}
struct Binding {
    path: api::selected::FactKey,
    protocol: api::selected::FactKey,
    digest: [u8; 32],
    role: api::selected::FactKey,
}
impl Binding {
    fn matches(&self, current: PairBinding<'_>) -> bool {
        &self.path == current.path
            && &self.protocol == current.protocol
            && &self.digest == current.digest
            && &self.role == current.role
    }
    fn matches_request(&self, request: &PairItemRequest, role: &api::selected::FactKey) -> bool {
        self.path == request.path
            && self.protocol == request.protocol
            && self.digest == request.digest
            && &self.role == role
    }
    fn matches_plan(&self, plan: &CompiledPairPlan, role: &api::selected::FactKey) -> bool {
        self.path == plan.path
            && self.protocol == plan.protocol
            && self.digest == plan.digest
            && &self.role == role
    }
}
impl From<PairBinding<'_>> for Binding {
    fn from(current: PairBinding<'_>) -> Self {
        Self {
            path: current.path.clone(),
            protocol: current.protocol.clone(),
            digest: *current.digest,
            role: current.role.clone(),
        }
    }
}
struct ItemReceipt {
    slots: [(i32, i32); 28],
    evidence: EvidenceStamp,
}
impl ItemReceipt {
    fn capture(observed: api::snapshot::Observed<&[api::snapshot::ItemView]>) -> Option<Self> {
        if observed.value.len() > 28 {
            return None;
        }
        let mut slots = [(-1, 0); 28];
        for item in observed.value {
            let slot = usize::try_from(item.slot).ok()?;
            if slot >= slots.len()
                || slots[slot].0 >= 0
                || item.def.id < 0
                || item.count <= 0
                || item.container != api::snapshot::ItemContainer::Inventory
            {
                return None;
            }
            slots[slot] = (item.def.id, if item.def.noted { 0 } else { item.count });
        }
        Some(Self {
            slots,
            evidence: observed.stamp,
        })
    }
    fn count(&self, obj: i32) -> i32 {
        self.slots
            .iter()
            .filter(|(id, _)| *id == obj)
            .fold(0i32, |total, (_, count)| total.saturating_add(*count))
    }
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
    paused_since: Option<Instant>,
    progress: [Option<EvidenceStamp>; 2],
}
impl Lease {
    fn side(&self, actor: RunKey) -> Result<usize, PairError> {
        if actor == self.token.left {
            Ok(0)
        } else if actor == self.token.right {
            Ok(1)
        } else {
            Err(PairError::Stale)
        }
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
        let profiles: HashSet<AccountKey> = names
            .into_iter()
            .map(|name| {
                state
                    .profiles
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| AccountKey(Arc::from(name)))
            })
            .collect();
        state.authoritative = true;
        if state.profiles == profiles {
            return;
        }
        let mut retired = [0u64; MAX_PHASES * 2];
        let mut count = 0;
        state.entries.retain(|account, entry| {
            if profiles.contains(account) {
                return true;
            }
            if let Some(id) = entry.lease {
                // A bounded phase has at most two account entries.
                retired[count] = id;
                count += 1;
            }
            false
        });
        for id in &retired[..count] {
            Self::cancel_locked(&mut state, *id);
        }
        state.profiles = profiles;
        Self::refresh_accounts(&mut state);
    }
    pub(crate) fn seat(
        self: &Arc<Self>,
        account: &str,
        host: &str,
        port: u16,
    ) -> Arc<dyn QuestPairPort> {
        Arc::new_cyclic(|weak| Seat {
            coordinator: Arc::clone(self),
            account: AccountKey(Arc::from(account)),
            identity: api::snapshot::player_account_id(account),
            world: Mutex::new(World {
                host: Arc::from(host),
                port,
            }),
            shared: weak.clone(),
            registered: AtomicBool::new(false),
        })
    }
    fn cancel_locked(state: &mut State, id: u64) {
        Self::cancel_with_error(state, id, PairError::Cancelled);
    }
    fn cancel_with_error(state: &mut State, id: u64, error: PairError) {
        let complete = state.leases.get(&id).is_some_and(Lease::complete);
        if complete {
            for entry in state.entries.values_mut() {
                if entry.lease == Some(id) {
                    entry.lease = None;
                }
            }
            return;
        }
        let Some(lease) = state.leases.remove(&id) else {
            return;
        };
        for action in lease.actions.iter().flatten() {
            action.revoke();
        }
        for entry in state.entries.values_mut() {
            let Ok(side) = lease.side(entry.registration.run) else {
                continue;
            };
            entry.lease = None;
            if lease.joined[side].is_some() {
                // Reservation alone does not make the peer part of this attempt.
                entry.cancelled = Some(error.clone());
                entry.admission_wait = false;
            }
        }
    }
    fn update_hold(state: &mut State, id: u64, now: Instant) {
        let Some(lease) = state.leases.get(&id) else {
            return;
        };
        if lease.complete() {
            return;
        }
        if now >= lease.total_deadline {
            Self::cancel_with_error(state, id, PairError::Failed(Arc::from(TOTAL_LIMIT_REASON)));
            return;
        }
        let held = [lease.token.left, lease.token.right].iter().any(|run| {
            state
                .entries
                .values()
                .any(|entry| entry.registration.run == *run && !entry.registration.ready)
        });
        if held && lease.paused_since.is_none() && now >= lease.deadline {
            Self::cancel_locked(state, id);
            return;
        }
        let lease = state.leases.get_mut(&id).unwrap();
        if held {
            lease.paused_since.get_or_insert(now);
        } else if let Some(since) = lease.paused_since.take() {
            let elapsed = now.saturating_duration_since(since);
            lease.deadline = (lease.deadline + elapsed).min(lease.total_deadline);
        }
    }
    fn validate(state: &State, token: &PairToken, actor: RunKey) -> Result<(), PairError> {
        let lease = state.leases.get(&token.id).ok_or_else(|| {
            state
                .entries
                .values()
                .find(|entry| entry.registration.run == actor)
                .and_then(|entry| entry.cancelled.clone())
                .unwrap_or(PairError::Cancelled)
        })?;
        if lease.token != *token {
            return Err(PairError::Stale);
        }
        lease.side(actor)?;
        if lease.complete() {
            return Ok(());
        }
        let mut ready = true;
        for run in [token.left, token.right] {
            let entry = state
                .entries
                .values()
                .find(|entry| entry.registration.run == run)
                .ok_or(PairError::Stale)?;
            if let Some(error) = &entry.cancelled {
                return Err(error.clone());
            }
            ready &= entry.registration.ready;
            if let Some(joined) = &lease.joined[lease.side(run)?] {
                let binding = entry.binding.as_ref().ok_or(PairError::Stale)?;
                if !binding.matches_plan(&lease.plan, &lease.plan.roles[joined.role].id) {
                    return Err(PairError::Stale);
                }
            }
        }
        if !ready {
            return Err(PairError::NotReady);
        }
        Ok(())
    }
    fn validate_live(state: &mut State, token: &PairToken, actor: RunKey) -> Result<(), PairError> {
        let result = Self::validate(state, token, actor);
        if !matches!(result, Ok(()) | Err(PairError::NotReady)) {
            return result;
        }
        let lease = state.leases.get(&token.id).unwrap();
        let now = Instant::now();
        if !lease.complete() {
            if now >= lease.total_deadline {
                let error = PairError::Failed(Arc::from(TOTAL_LIMIT_REASON));
                Self::cancel_with_error(state, token.id, error.clone());
                return Err(error);
            }
            if result.is_ok() && now >= lease.deadline {
                Self::cancel_locked(state, token.id);
                return Err(PairError::BarrierExpired);
            }
        }
        result
    }
    /// Recording already-owned authority/results is safe while dispatch is
    /// fenced. In particular, a hold racing a completed role cannot lose it.
    fn validate_bookkeeping(
        state: &mut State,
        token: &PairToken,
        actor: RunKey,
    ) -> Result<(), PairError> {
        match Self::validate_live(state, token, actor) {
            Err(PairError::NotReady)
                if state
                    .leases
                    .get(&token.id)
                    .is_some_and(|lease| lease.paused_since.is_some()) =>
            {
                Ok(())
            }
            result => result,
        }
    }
    fn begin_at(
        &self,
        account: &AccountKey,
        request: PairRequest,
        now: Instant,
    ) -> Result<PairToken, PairError> {
        let mut state = self.state.lock().unwrap();
        if let Some(own) = state
            .entries
            .get_mut(account)
            .filter(|own| own.registration.run == request.caller)
        {
            own.admission_wait = false;
        }
        let own_identity = api::snapshot::player_account_id(&account.0);
        if account == &request.partner
            || own_identity
                .is_some_and(|id| api::snapshot::player_account_id(&request.partner.0) == Some(id))
        {
            return Err(PairError::SelfPartner);
        }
        if !state.profiles.contains(account) || !state.profiles.contains(&request.partner) {
            return Err(PairError::UnknownAccount);
        }
        let own = state
            .entries
            .get(account)
            .ok_or(PairError::PartnerNotInPlay)?;
        let Some(peer) = state.entries.get(&request.partner) else {
            if let Some(own) = state
                .entries
                .get_mut(account)
                .filter(|own| own.registration.run == request.caller)
            {
                own.admission_wait = true;
            }
            return Err(PairError::PartnerNotInPlay);
        };
        if own.registration.run != request.caller || request.evidence.run != request.caller {
            return Err(PairError::Stale);
        }
        if let Some(error) = own.cancelled.as_ref().or(peer.cancelled.as_ref()) {
            return Err(error.clone());
        }
        if !peer.binding.as_ref().is_some_and(|binding| {
            binding.path == request.plan.path
                && binding.protocol == request.plan.protocol
                && binding.digest == request.plan.digest
        }) {
            // An earlier queue row is not a participant in this phase.
            state.entries.get_mut(account).unwrap().admission_wait = true;
            return Err(PairError::Busy);
        }
        if !own.registration.ready || !peer.registration.ready {
            state.entries.get_mut(account).unwrap().admission_wait = true;
            return Err(PairError::NotReady);
        }
        if own.registration.evidence != request.evidence {
            return Err(PairError::Stale);
        }
        let settings = own
            .registration
            .settings
            .as_ref()
            .ok_or(PairError::PartnerNotInPlay)?;
        let peer_settings = peer
            .registration
            .settings
            .as_ref()
            .ok_or(PairError::PartnerNotInPlay)?;
        if settings.partner.as_ref() != Some(&request.partner)
            || peer_settings.partner.as_ref() != Some(account)
        {
            return Err(PairError::MissingPartner);
        }
        if request.declared_gang != settings.gang {
            return Err(PairError::WrongGang);
        }
        if own.world != peer.world {
            return Err(PairError::DifferentWorld);
        }
        if own.registration.pin != peer.registration.pin {
            return Err(PairError::WrongPin);
        }
        if request.phase != request.plan.phase {
            return Err(PairError::Stale);
        }
        match (&own.gang, &request.observed_gang) {
            (Some((Knowledge::Known(owned), evidence)), Knowledge::Known(reported))
                if owned == reported && evidence.run == request.caller => {}
            _ => return Err(PairError::UnknownGang),
        }
        let gang = own.effective_gang()?;
        let role = request
            .plan
            .roles
            .iter()
            .position(|role| role.gang == gang && role.id == request.caller_role)
            .ok_or(PairError::WrongGang)?;
        let binding = own.binding.as_ref().ok_or(PairError::NotReady)?;
        if !binding.matches_plan(&request.plan, &request.caller_role) {
            return Err(PairError::Stale);
        }
        let peer_run = peer.registration.run;
        let existing = own.lease.or(peer.lease);
        if own.lease.is_some() && peer.lease.is_some() && own.lease != peer.lease {
            return Err(PairError::Busy);
        }
        let (left, right) = if request.caller.slot < peer_run.slot {
            (request.caller, peer_run)
        } else {
            (peer_run, request.caller)
        };
        let side = usize::from(request.caller == right);
        if let Some(id) = existing {
            let lease = state.leases.get_mut(&id).ok_or(PairError::Stale)?;
            if lease.token.left != left || lease.token.right != right {
                return Err(PairError::Busy);
            }
            if now >= lease.total_deadline {
                let error = PairError::Failed(Arc::from(TOTAL_LIMIT_REASON));
                Self::cancel_with_error(&mut state, id, error.clone());
                return Err(error);
            }
            if now >= lease.deadline {
                Self::cancel_locked(&mut state, id);
                return Err(PairError::BarrierExpired);
            }
            if lease.plan.path != request.plan.path
                || lease.plan.protocol != request.plan.protocol
                || lease.plan.digest != request.plan.digest
                || lease.plan.phase != request.phase
                || lease.plan.signature != request.plan.signature
            {
                return Err(PairError::NotReady);
            }
            if let Some(joined) = &lease.joined[side] {
                if joined.role != role {
                    return Err(PairError::WrongGang);
                }
                return Ok(lease.token);
            }
            if lease.joined[1 - side]
                .as_ref()
                .is_some_and(|joined| joined.role == role)
            {
                Self::cancel_locked(&mut state, id);
                return Err(PairError::WrongGang);
            }
            lease.joined[side] = Some(Joined {
                role,
                evidence: request.evidence,
            });
            if lease.plan.actions.is_none() {
                lease.receipts = [
                    lease.joined[0].as_ref().map(|j| j.evidence),
                    lease.joined[1].as_ref().map(|j| j.evidence),
                ];
            }
            let token = lease.token;
            state.entries.get_mut(account).unwrap().lease = Some(id);
            Ok(token)
        } else {
            let State {
                entries, leases, ..
            } = &mut *state;
            leases.retain(|_, lease| {
                !lease.complete()
                    || (!lease.done_seen.iter().all(|seen| *seen)
                        && [lease.token.left, lease.token.right].iter().all(|run| {
                            entries.values().any(|entry| entry.registration.run == *run)
                        }))
            });
            if state.leases.len() >= MAX_PHASES {
                return Err(PairError::Busy);
            }
            state.next = state.next.checked_add(1).ok_or(PairError::Busy)?;
            let id = state.next;
            let token = PairToken {
                id,
                generation: id,
                left,
                right,
            };
            let mut joined = [None, None];
            joined[side] = Some(Joined {
                role,
                evidence: request.evidence,
            });
            let command_base = id.checked_mul(2).ok_or(PairError::Busy)?;
            state.leases.insert(
                id,
                Lease {
                    token,
                    plan: request.plan,
                    joined,
                    receipts: [None, None],
                    offer: [None, None],
                    confirm: [None, None],
                    command: [command_base, command_base + 1],
                    done_seen: [false, false],
                    actions: [None, None],
                    deadline: now + INACTIVITY,
                    total_deadline: now + TOTAL,
                    paused_since: None,
                    progress: [None, None],
                },
            );
            state.entries.get_mut(account).unwrap().lease = Some(id);
            state.entries.get_mut(&request.partner).unwrap().lease = Some(id);
            Ok(token)
        }
    }
    pub(crate) fn gameplay_progress(&self, actor: RunKey, evidence: EvidenceStamp, now: Instant) {
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state
            .entries
            .values()
            .find(|entry| entry.registration.run == actor)
        else {
            return;
        };
        if !entry.registration.ready
            || evidence.run != actor
            || !entry.registration.evidence.meets(evidence)
        {
            return;
        }
        let Some(id) = entry.lease else {
            return;
        };
        let Some(lease) = state.leases.get(&id) else {
            return;
        };
        let Ok(side) = lease.side(actor) else {
            return;
        };
        if lease.joined[side].is_none()
            && !entry.binding.as_ref().is_some_and(|binding| {
                lease
                    .plan
                    .roles
                    .iter()
                    .any(|role| binding.matches_plan(&lease.plan, &role.id))
            })
        {
            return;
        }
        if lease.complete()
            || lease.paused_since.is_some()
            || now >= lease.deadline
            || now >= lease.total_deadline
        {
            return;
        }
        if lease.progress[side].is_some_and(|before| !evidence.meets(before) || evidence == before)
        {
            return;
        }
        let lease = state.leases.get_mut(&id).unwrap();
        lease.progress[side] = Some(evidence);
        lease.deadline = (now + INACTIVITY).min(lease.total_deadline);
    }
}
struct Seat {
    coordinator: Arc<QuestPairCoordinator>,
    account: AccountKey,
    identity: Option<u64>,
    world: Mutex<World>,
    shared: std::sync::Weak<Seat>,
    /// Tracks prior authority until an unpaired observation retires its entry.
    registered: AtomicBool,
}
impl Seat {
    fn actor(&self, state: &State, actor: RunKey) -> Result<(), PairError> {
        if state
            .entries
            .get(&self.account)
            .is_some_and(|entry| entry.registration.run == actor)
        {
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
    fn observe(&self, registration: PairRegistration, frame: PairFrame<'_>) {
        if registration.settings.is_none()
            && frame.binding.is_none()
            && !self.registered.load(Ordering::Acquire)
        {
            return;
        }
        let world = self.world.lock().unwrap().clone();
        let mut state = self.coordinator.state.lock().unwrap();
        if registration.evidence.run != registration.run {
            return;
        }
        if !state.profiles.contains(&self.account) {
            return;
        }
        let mut admission_wait = false;
        if let Some(old) = state.entries.get(&self.account) {
            if old.registration.run == registration.run
                && !registration.evidence.meets(old.registration.evidence)
            {
                return;
            }
            let changed = old.registration.run != registration.run
                || old.registration.pin != registration.pin
                || old.registration.settings != registration.settings
                || old.world != world
                || old.binding.as_ref().is_some_and(|prior| {
                    !frame.binding.is_some_and(|current| prior.matches(current))
                });
            admission_wait = old.admission_wait && !changed;
            if changed {
                if let Some(id) = old.lease {
                    QuestPairCoordinator::cancel_locked(&mut state, id);
                }
            }
        }
        let mut old = state.entries.remove(&self.account);
        if registration.settings.is_none() && frame.binding.is_none() {
            self.registered.store(false, Ordering::Release);
            return;
        }
        let prior_binding = old.as_mut().and_then(|old| old.binding.take());
        let binding = frame.binding.map(|current| {
            prior_binding
                .filter(|prior| prior.matches(current))
                .unwrap_or_else(|| Binding::from(current))
        });
        let inventory = frame
            .inventory
            .filter(|observed| {
                registration.ready && binding.is_some() && observed.stamp == registration.evidence
            })
            .and_then(ItemReceipt::capture);
        let mut ready_after = registration.evidence;
        let (lease, cancelled, gang) = old
            .filter(|old| {
                old.registration.run == registration.run && old.registration.pin == registration.pin
            })
            .map_or((None, None, None), |old| {
                // A reserved role's owned membership survives a transient hold.
                // Re-reading it would compete with the retained phase action.
                let same_ready_epoch = old.world == world
                    && ((old.registration.ready && registration.ready) || old.lease.is_some());
                let gang = if same_ready_epoch {
                    ready_after = old.ready_after;
                    old.gang
                } else {
                    None
                };
                (old.lease, old.cancelled, gang)
            });
        state.entries.insert(
            self.account.clone(),
            Entry {
                world,
                registration,
                lease,
                cancelled,
                admission_wait,
                gang,
                ready_after,
                identity: self.identity,
                binding,
                inventory,
            },
        );
        self.registered.store(true, Ordering::Release);
        if let Some(id) = lease {
            QuestPairCoordinator::update_hold(&mut state, id, Instant::now());
        }
    }
    fn invalidate(&self, run: RunKey) {
        if !self.registered.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.coordinator.state.lock().unwrap();
        let Some(entry) = state
            .entries
            .get(&self.account)
            .filter(|entry| entry.registration.run == run)
        else {
            return;
        };
        if let Some(id) = entry.lease {
            QuestPairCoordinator::cancel_locked(&mut state, id);
        }
        if let Some(entry) = state.entries.get_mut(&self.account) {
            entry.registration.ready = false;
            entry.gang = None;
            entry.admission_wait = false;
            entry.inventory = None;
        }
    }
    fn busy(&self) -> bool {
        self.coordinator
            .state
            .lock()
            .unwrap()
            .entries
            .get(&self.account)
            .is_some_and(|entry| entry.lease.is_some())
    }
    fn world_changed(&self, host: &str, port: u16) {
        let mut world = self.world.lock().unwrap();
        if world.host.as_ref() == host && world.port == port {
            return;
        }
        *world = World {
            host: Arc::from(host),
            port,
        };
        drop(world);
        let run = self
            .coordinator
            .state
            .lock()
            .unwrap()
            .entries
            .get(&self.account)
            .map(|entry| entry.registration.run);
        if let Some(run) = run {
            self.invalidate(run);
        }
    }
    fn settings(&self, caller: RunKey) -> Result<PairSettings, PairError> {
        let state = self.coordinator.state.lock().unwrap();
        let entry = state
            .entries
            .get(&self.account)
            .ok_or(PairError::NotReady)?;
        if entry.registration.run != caller {
            return Err(PairError::Stale);
        }
        if let Some(error) = &entry.cancelled {
            return Err(error.clone());
        }
        if !entry.registration.ready {
            return Err(PairError::NotReady);
        }
        entry
            .registration
            .settings
            .clone()
            .ok_or(PairError::PartnerNotInPlay)
    }
    fn observe_gang(
        &self,
        read: &api::quest_progress::JournalRead,
    ) -> Result<Knowledge<Option<Gang>>, PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, read.closed.run)?;
        let entry = state.entries.get_mut(&self.account).unwrap();
        if read.quest.0.as_ref() != PairQuest::Arrav.path()
            || read.pin != entry.registration.pin
            || !read.acquired.meets(entry.ready_after)
            || !read.closed.meets(read.acquired)
            || read.closed == read.acquired
            || !entry.registration.evidence.meets(read.closed)
            || entry
                .gang
                .as_ref()
                .is_some_and(|(_, prior)| !read.closed.meets(*prior) || read.closed == *prior)
            || !entry.registration.ready
            || entry.cancelled.is_some()
        {
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
        let entry = state.entries.get(&self.account).unwrap();
        if let Some(error) = &entry.cancelled {
            return Err(error.clone());
        }
        if !entry.registration.ready {
            return Err(PairError::NotReady);
        }
        entry.gang.clone().ok_or(PairError::UnknownGang)
    }
    fn partner_item_count(
        &self,
        caller: EvidenceStamp,
        request: &PairItemRequest,
    ) -> Result<i32, PairError> {
        let state = self.coordinator.state.lock().unwrap();
        self.actor(&state, caller.run)?;
        if !PairQuest::from_path(request.path.0.as_ref())
            .is_some_and(|quest| !quest.recovery_items().is_empty())
        {
            return Err(PairError::Stale);
        }
        if !state.profiles.contains(&self.account) {
            return Err(PairError::UnknownAccount);
        }
        let own = state.entries.get(&self.account).unwrap();
        if own.registration.evidence != caller {
            return Err(PairError::Stale);
        }
        let settings = own
            .registration
            .settings
            .as_ref()
            .ok_or(PairError::PartnerNotInPlay)?;
        let partner = settings.partner.as_ref().ok_or(PairError::MissingPartner)?;
        if !state.profiles.contains(partner) {
            return Err(PairError::UnknownAccount);
        }
        let peer = state
            .entries
            .get(partner)
            .ok_or(PairError::PartnerNotInPlay)?;
        if own.registration.run == peer.registration.run
            || own.identity.is_some_and(|id| peer.identity == Some(id))
        {
            return Err(PairError::SelfPartner);
        }
        if let Some(error) = own.cancelled.as_ref().or(peer.cancelled.as_ref()) {
            return Err(error.clone());
        }
        if !own.registration.ready || !peer.registration.ready {
            return Err(PairError::NotReady);
        }
        let peer_settings = peer
            .registration
            .settings
            .as_ref()
            .ok_or(PairError::PartnerNotInPlay)?;
        if peer_settings.partner.as_ref() != Some(&self.account) {
            return Err(PairError::MissingPartner);
        }
        if own.world != peer.world {
            return Err(PairError::DifferentWorld);
        }
        if own.registration.pin != peer.registration.pin {
            return Err(PairError::WrongPin);
        }
        if !own
            .binding
            .as_ref()
            .is_some_and(|binding| binding.matches_request(request, &request.own.id))
            || !peer
                .binding
                .as_ref()
                .is_some_and(|binding| binding.matches_request(request, &request.peer.id))
        {
            return Err(PairError::Stale);
        }
        if request.own.gang == request.peer.gang
            || own.effective_gang()? != request.own.gang
            || peer.effective_gang()? != request.peer.gang
        {
            return Err(PairError::WrongGang);
        }
        let inventory = peer.inventory.as_ref().ok_or(PairError::NotReady)?;
        if inventory.evidence != peer.registration.evidence
            || !inventory.evidence.meets(peer.ready_after)
        {
            return Err(PairError::Stale);
        }
        Ok(inventory.count(request.obj))
    }
    fn waiting(&self, caller: RunKey) -> bool {
        if !self.registered.load(Ordering::Acquire) {
            return false;
        }
        let state = self.coordinator.state.lock().unwrap();
        if self.actor(&state, caller).is_err() {
            return false;
        }
        let entry = state.entries.get(&self.account).unwrap();
        let Some(lease) = entry.lease.and_then(|id| state.leases.get(&id)) else {
            return entry.admission_wait;
        };
        let Ok(side) = lease.side(caller) else {
            return false;
        };
        !lease.complete()
            && (lease.paused_since.is_some()
                || (lease.joined[side].is_some()
                    && (lease.joined.iter().any(Option::is_none)
                        || lease.receipts[side].is_some())))
    }
    fn held(&self, caller: RunKey) -> bool {
        if !self.registered.load(Ordering::Acquire) {
            return false;
        }
        let state = self.coordinator.state.lock().unwrap();
        state
            .entries
            .get(&self.account)
            .filter(|entry| entry.registration.run == caller)
            .and_then(|entry| entry.lease)
            .and_then(|id| state.leases.get(&id))
            .is_some_and(|lease| lease.paused_since.is_some())
    }
    fn token(
        &self,
        caller: RunKey,
        phase: &api::selected::FactKey,
    ) -> Result<PairToken, PairError> {
        let state = self.coordinator.state.lock().unwrap();
        self.actor(&state, caller)?;
        let lease = state
            .entries
            .get(&self.account)
            .and_then(|entry| entry.lease)
            .and_then(|id| state.leases.get(&id))
            .ok_or(PairError::Stale)?;
        if &lease.plan.phase != phase {
            return Err(PairError::Stale);
        }
        QuestPairCoordinator::validate(&state, &lease.token, caller)?;
        Ok(lease.token)
    }
    fn register_action(
        &self,
        token: &PairToken,
        actor: RunKey,
        action: script::native::ActionRevoker,
    ) -> Result<(), PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, actor)?;
        QuestPairCoordinator::validate_bookkeeping(&mut state, token, actor)?;
        if action.run() != actor {
            return Err(PairError::Stale);
        }
        let lease = state.leases.get_mut(&token.id).unwrap();
        if lease.complete() {
            return Err(PairError::Stale);
        }
        let side = lease.side(actor)?;
        lease.actions[side] = Some(action);
        Ok(())
    }
    fn begin(&self, request: PairRequest) -> Result<PairToken, PairError> {
        self.coordinator
            .begin_at(&self.account, request, Instant::now())
    }
    fn poll(&self, token: &PairToken, caller: RunKey) -> Poll<Result<PairStep, PairError>> {
        let mut state = self.coordinator.state.lock().unwrap();
        if let Err(error) = self.actor(&state, caller) {
            return Poll::Ready(Err(error));
        }
        if let Err(error) = QuestPairCoordinator::validate_live(&mut state, token, caller) {
            return Poll::Ready(Err(error));
        }
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(caller).unwrap();
        if lease.complete() {
            let receipt = PairReceipt {
                token: *token,
                phase: lease.plan.phase.clone(),
                evidence: [lease.receipts[0].unwrap(), lease.receipts[1].unwrap()],
            };
            lease.done_seen[side] = true;
            let forget = lease.done_seen.iter().all(|seen| *seen);
            for entry in state.entries.values_mut() {
                if entry.lease == Some(token.id) {
                    entry.lease = None;
                }
            }
            if forget {
                state.leases.remove(&token.id);
            }
            return Poll::Ready(Ok(PairStep::Done(receipt)));
        }
        if lease.joined.iter().all(Option::is_some) && lease.receipts[side].is_none() {
            let role = lease.joined[side].as_ref().unwrap().role;
            if let Some(actions) = &lease.plan.actions {
                return Poll::Ready(Ok(PairStep::Act(RoleCommand {
                    token: *token,
                    recipient: caller,
                    phase: lease.plan.phase.clone(),
                    command_id: lease.command[side],
                    plan: Arc::clone(&actions[role]),
                })));
            }
        }
        Poll::Ready(Ok(PairStep::Waiting {
            phase: lease.plan.phase.clone(),
            deadline: lease.deadline,
        }))
    }
    fn report(&self, token: &PairToken, receipt: RoleReceipt) -> Result<(), PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, receipt.actor)?;
        QuestPairCoordinator::validate_bookkeeping(&mut state, token, receipt.actor)?;
        let observed = state
            .entries
            .get(&self.account)
            .unwrap()
            .registration
            .evidence;
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(receipt.actor)?;
        if lease.joined.iter().any(Option::is_none) || lease.command[side] != receipt.command_id {
            return Err(PairError::Stale);
        }
        if lease.receipts[side].is_some() {
            return Err(PairError::Stale);
        }
        match receipt.outcome {
            Ok(outcome) => {
                let before = lease.joined[side].as_ref().unwrap().evidence;
                if outcome.evidence.run != receipt.actor
                    || !outcome.evidence.meets(before)
                    || outcome.evidence == before
                    || !observed.meets(outcome.evidence)
                {
                    return Err(PairError::Stale);
                }
                lease.receipts[side] = Some(outcome.evidence);
                Ok(())
            }
            Err(_) => {
                QuestPairCoordinator::cancel_locked(&mut state, token.id);
                Err(PairError::Cancelled)
            }
        }
    }
    fn trade_ready(
        &self,
        token: &PairToken,
        actor: RunKey,
        confirm: bool,
        evidence: EvidenceStamp,
    ) -> Result<bool, PairError> {
        let mut state = self.coordinator.state.lock().unwrap();
        self.actor(&state, actor)?;
        match QuestPairCoordinator::validate_live(&mut state, token, actor) {
            Err(PairError::NotReady) => return Ok(false),
            result => result?,
        }
        let entry = state
            .entries
            .values()
            .find(|entry| entry.registration.run == actor)
            .ok_or(PairError::Stale)?;
        if evidence.run != actor || !entry.registration.evidence.meets(evidence) {
            return Err(PairError::Stale);
        }
        let lease = state.leases.get_mut(&token.id).unwrap();
        let side = lease.side(actor)?;
        if lease.joined.iter().any(Option::is_none) {
            return Err(PairError::NotReady);
        }
        if confirm && lease.offer.iter().any(Option::is_none) {
            return Err(PairError::NotReady);
        }
        let before = lease.joined[side].as_ref().unwrap().evidence;
        if !evidence.meets(before) || evidence == before {
            return Err(PairError::Stale);
        }
        if confirm {
            let offered = lease.offer[side].unwrap();
            if !evidence.meets(offered) || evidence == offered {
                return Err(PairError::Stale);
            }
        }
        let screen = if confirm {
            &mut lease.confirm
        } else {
            &mut lease.offer
        };
        if screen[side].is_some_and(|before| !evidence.meets(before)) {
            return Err(PairError::Stale);
        }
        screen[side] = Some(evidence);
        Ok(screen.iter().all(Option::is_some))
    }
    fn gameplay_progress(&self, actor: RunKey, evidence: EvidenceStamp, now: Instant) {
        if !self.registered.load(Ordering::Acquire) {
            return;
        }
        let owns = self
            .actor(&self.coordinator.state.lock().unwrap(), actor)
            .is_ok();
        if owns {
            self.coordinator.gameplay_progress(actor, evidence, now);
        }
    }
    fn cancel(&self, token: &PairToken) {
        let mut state = self.coordinator.state.lock().unwrap();
        let owns = state.entries.get(&self.account).is_some_and(|entry| {
            entry.registration.run == token.left || entry.registration.run == token.right
        });
        if owns
            && state
                .leases
                .get(&token.id)
                .is_some_and(|lease| lease.token == *token)
        {
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
    mod runner_tests {
        include!("quest_pair_runner_tests.rs");
    }

    fn stamp(run: RunKey, tick: u64) -> EvidenceStamp {
        EvidenceStamp {
            run,
            tick,
            sequence: tick,
        }
    }
    fn pin() -> Arc<api::selected::SelectedPin> {
        api::game_data::for_revision(ClientRevision::R289)
            .unwrap()
            .selected_pin()
            .unwrap()
    }
    fn register(port: &dyn QuestPairPort, run: RunKey, partner: &str, gang: Gang, tick: u64) {
        register_ready(port, run, partner, gang, tick, true);
    }
    fn register_ready(
        port: &dyn QuestPairPort,
        run: RunKey,
        partner: &str,
        gang: Gang,
        tick: u64,
        ready: bool,
    ) {
        let plan = plan(false);
        let role = plan.roles.iter().find(|role| role.gang == gang).unwrap();
        port.observe(
            PairRegistration {
                run,
                pin: pin(),
                settings: Some(PairSettings {
                    partner: Some(AccountKey(Arc::from(partner))),
                    gang: Some(gang),
                }),
                ready,
                evidence: stamp(run, tick),
            },
            PairFrame {
                binding: Some(PairBinding {
                    path: &plan.path,
                    protocol: &plan.protocol,
                    digest: &plan.digest,
                    role: &role.id,
                }),
                inventory: None,
            },
        );
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
            Err(ActionError::Unavailable(Arc::from(
                "test sequencer does not execute gameplay",
            )))
        }
    }
    fn plan(actions: bool) -> Arc<CompiledPairPlan> {
        Arc::new(CompiledPairPlan {
            path: FactKey::new("blackarmgang"),
            protocol: FactKey::new("arrav"),
            digest: [1; 32],
            phase: FactKey::new(if actions {
                "arrav:key"
            } else {
                "arrav:admission"
            }),
            roles: [
                PartnerRole {
                    id: FactKey::new("phoenix"),
                    gang: Gang::Phoenix,
                },
                PartnerRole {
                    id: FactKey::new("blackarm"),
                    gang: Gang::BlackArm,
                },
            ],
            signature: [2; 32],
            actions: actions.then(|| {
                [
                    Arc::new(TestAction) as Arc<dyn StepPlan>,
                    Arc::new(TestAction),
                ]
            }),
        })
    }
    fn request(
        run: RunKey,
        partner: &str,
        role: usize,
        plan: &Arc<CompiledPairPlan>,
    ) -> PairRequest {
        PairRequest {
            caller: run,
            partner: AccountKey(Arc::from(partner)),
            caller_role: plan.roles[role].id.clone(),
            phase: plan.phase.clone(),
            plan: Arc::clone(plan),
            observed_gang: Knowledge::Known(None),
            declared_gang: Some(plan.roles[role].gang),
            evidence: stamp(run, 2),
        }
    }
    struct Pair {
        core: Arc<QuestPairCoordinator>,
        seats: [Arc<dyn QuestPairPort>; 2],
        runs: [RunKey; 2],
    }
    fn pair() -> Pair {
        let core = Arc::new(QuestPairCoordinator::default());
        core.remember("alice");
        core.remember("bob");
        let seats = [
            core.seat("alice", "127.0.0.1", 44594),
            core.seat("bob", "127.0.0.1", 44594),
        ];
        let runs = [
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            RunKey {
                slot: 2,
                run: 1,
                session: 1,
            },
        ];
        register(seats[0].as_ref(), runs[0], "bob", Gang::Phoenix, 1);
        register(seats[1].as_ref(), runs[1], "alice", Gang::BlackArm, 1);
        register(seats[0].as_ref(), runs[0], "bob", Gang::Phoenix, 2);
        register(seats[1].as_ref(), runs[1], "alice", Gang::BlackArm, 2);
        for side in 0..2 {
            assert!(matches!(
                seats[side].observe_gang(&read(runs[side])),
                Ok(Knowledge::Known(None))
            ));
        }
        Pair { core, seats, runs }
    }
    #[test]
    fn unpaired_observe_and_watchdog_waiting_never_lock_coordinator() {
        let core = Arc::new(QuestPairCoordinator::default());
        core.remember("alice");
        let seat = core.seat("alice", "127.0.0.1", 44594);
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let pin = pin();
        struct Solo(Arc<std::sync::atomic::AtomicUsize>);
        impl script::native::Script for Solo {
            fn tick(
                &mut self,
                _: &mut script::native::NativeTick<'_>,
            ) -> Result<script::native::ScriptFlow, script::native::ScriptFailure> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(script::native::ScriptFlow::Continue)
            }
        }
        let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut slot = script::SlotScript::new();
        slot.bind_quest_pairs(Arc::clone(&seat));
        slot.start_test_script(Box::new(Solo(Arc::clone(&ticks))), None)
            .unwrap();
        // Poisoning makes any accidental broker access fail synchronously, rather
        // than relying on timing or a synthetic port that bypasses the real seat.
        let _ = std::panic::catch_unwind(|| {
            let _guard = core.state.lock().unwrap();
            panic!("lock-free regression probe");
        });
        let observed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            seat.observe(
                PairRegistration {
                    run,
                    pin,
                    settings: None,
                    ready: true,
                    evidence: stamp(run, 1),
                },
                PairFrame::default(),
            );
        }));
        assert!(
            observed.is_ok(),
            "an unpaired compiled observation must not lock"
        );
        let waiting = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| seat.waiting(run)));
        assert!(
            matches!(waiting, Ok(false)),
            "an unpaired watchdog must not lock"
        );
        // Drive CompiledRun::tick too: every ordinary native card receives this
        // installed port, but none may enter the poisoned coordinator.
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(1);
        let mut driver = crate::tests::nav_client();
        let mut cx = script::ScriptCtx {
            driver: &mut driver,
            tick: 1,
            here: Some((0, 0, 0)),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(&snapshot),
            obj_names: None,
            compiled: script::CompiledTick::default(),
        };
        slot.on_game_tick(&mut cx);
        assert_eq!(ticks.load(Ordering::Relaxed), 1);
        assert_eq!(slot.state(), script::RunState::Running);
    }

    #[test]
    fn unpaired_replacement_retires_prior_lease_then_returns_to_lock_free_path() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        let next = RunKey {
            run: 2,
            ..pair.runs[0]
        };
        pair.seats[0].observe(
            PairRegistration {
                run: next,
                pin: pin(),
                settings: None,
                ready: true,
                evidence: stamp(next, 1),
            },
            PairFrame::default(),
        );
        assert!(matches!(
            pair.seats[1].poll(&token, pair.runs[1]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
        assert!(!pair
            .core
            .state
            .lock()
            .unwrap()
            .entries
            .contains_key("alice"));
        let _ = std::panic::catch_unwind(|| {
            let _guard = pair.core.state.lock().unwrap();
            panic!("retired entry lock-free probe");
        });
        pair.seats[0].observe(
            PairRegistration {
                run: next,
                pin: pin(),
                settings: None,
                ready: true,
                evidence: stamp(next, 2),
            },
            PairFrame::default(),
        );
        assert!(!pair.seats[0].waiting(next));
        assert!(!pair.seats[0].held(next));
    }

    fn held(obj: i32, count: i32, slot: i32) -> api::snapshot::ItemView {
        api::snapshot::ItemView {
            def: api::ItemDefView {
                id: obj,
                name: None,
                stackable: false,
                members: false,
                base_value: 0,
                noted: false,
                certificate_link: -1,
                certificate_template: -1,
            },
            container: api::snapshot::ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot,
            count,
            actions: Vec::new(),
            component_id: -1,
        }
    }

    fn publish_items(
        pair: &Pair,
        side: usize,
        query: &PairItemRequest,
        tick: u64,
        items: Vec<api::snapshot::ItemView>,
    ) {
        let run = pair.runs[side];
        let role = if side == 0 { &query.peer } else { &query.own };
        let partner = if side == 0 { "bob" } else { "alice" };
        let evidence = stamp(run, tick);
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(tick as i32);
        snapshot.seed_inventory(items, 28);
        pair.seats[side].observe(
            PairRegistration {
                run,
                pin: pin(),
                settings: Some(PairSettings {
                    partner: Some(AccountKey(Arc::from(partner))),
                    gang: Some(role.gang),
                }),
                ready: true,
                evidence,
            },
            PairFrame {
                binding: Some(PairBinding {
                    path: &query.path,
                    protocol: &query.protocol,
                    digest: &query.digest,
                    role: &role.id,
                }),
                inventory: api::snapshot::SnapshotView::new(Some(&snapshot), evidence).inventory(),
            },
        );
    }

    fn item_query() -> PairItemRequest {
        let plan = plan(false);
        PairItemRequest {
            path: plan.path.clone(),
            protocol: plan.protocol.clone(),
            digest: plan.digest,
            own: plan.roles[1].clone(),
            peer: plan.roles[0].clone(),
            obj: 700,
        }
    }

    #[test]
    fn partner_holdings_require_current_native_receipts_and_matching_active_roles() {
        let pair = pair();
        let mut query = item_query();
        let caller = stamp(pair.runs[1], 3);
        assert_eq!(
            pair.seats[1].partner_item_count(stamp(pair.runs[1], 2), &query),
            Err(PairError::NotReady)
        );
        publish_items(
            &pair,
            0,
            &query,
            3,
            vec![held(query.obj, 1, 0), held(query.obj, 1, 1)],
        );
        publish_items(&pair, 1, &query, 3, vec![]);
        assert_eq!(pair.seats[1].partner_item_count(caller, &query), Ok(2));
        assert_eq!(
            pair.seats[1].partner_item_count(stamp(pair.runs[1], 2), &query),
            Err(PairError::Stale)
        );
        query.digest = [9; 32];
        assert_eq!(
            pair.seats[1].partner_item_count(caller, &query),
            Err(PairError::Stale)
        );
        query.digest = [1; 32];
        let mut noted = held(query.obj, 10, 0);
        noted.def.noted = true;
        publish_items(&pair, 0, &query, 4, vec![noted]);
        assert_eq!(pair.seats[1].partner_item_count(caller, &query), Ok(0));
        publish_items(
            &pair,
            0,
            &query,
            5,
            vec![held(query.obj, 1, 0), held(query.obj, 1, 0)],
        );
        assert_eq!(
            pair.seats[1].partner_item_count(caller, &query),
            Err(PairError::NotReady)
        );
        publish_items(&pair, 0, &query, 6, vec![]);
        assert_eq!(pair.seats[1].partner_item_count(caller, &query), Ok(0));
        pair.seats[0].invalidate(pair.runs[0]);
        assert_eq!(
            pair.seats[1].partner_item_count(caller, &query),
            Err(PairError::NotReady)
        );
    }

    #[test]
    fn partner_holdings_reject_stale_future_foreign_and_mismatched_sequence_receipts() {
        let pair = pair();
        let query = item_query();
        let caller = stamp(pair.runs[1], 3);
        publish_items(&pair, 0, &query, 3, vec![held(query.obj, 1, 0)]);
        publish_items(&pair, 1, &query, 3, vec![]);
        assert_eq!(pair.seats[1].partner_item_count(caller, &query), Ok(1));
        let run = pair.runs[0];
        let receipts = [
            stamp(run, 2),
            stamp(run, 99),
            stamp(pair.runs[1], 6),
            EvidenceStamp {
                run,
                tick: 7,
                sequence: 99,
            },
        ];
        for (index, observed) in receipts.into_iter().enumerate() {
            let evidence = stamp(run, 4 + index as u64);
            let mut snapshot = api::snapshot::GameSnapshot::new();
            snapshot.seed_ingame(evidence.tick as i32);
            snapshot.seed_inventory(vec![held(query.obj, 1, 0)], 28);
            pair.seats[0].observe(
                PairRegistration {
                    run,
                    pin: pin(),
                    ready: true,
                    evidence,
                    settings: Some(PairSettings {
                        partner: Some(AccountKey(Arc::from("bob"))),
                        gang: Some(query.peer.gang),
                    }),
                },
                PairFrame {
                    binding: Some(PairBinding {
                        path: &query.path,
                        protocol: &query.protocol,
                        digest: &query.digest,
                        role: &query.peer.id,
                    }),
                    inventory: api::snapshot::SnapshotView::new(Some(&snapshot), observed)
                        .inventory(),
                },
            );
            assert_eq!(
                pair.seats[1].partner_item_count(caller, &query),
                Err(PairError::NotReady)
            );
        }
        publish_items(&pair, 0, &query, 8, vec![held(query.obj, 1, 0)]);
        assert_eq!(pair.seats[1].partner_item_count(caller, &query), Ok(1));
    }

    #[test]
    fn active_pair_binding_change_revokes_the_reserved_phase() {
        let pair = pair();
        let query = item_query();
        publish_items(&pair, 0, &query, 2, vec![]);
        publish_items(&pair, 1, &query, 2, vec![]);
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        let mut changed = item_query();
        changed.digest = [8; 32];
        publish_items(&pair, 0, &changed, 3, vec![]);
        assert!(matches!(
            pair.seats[1].poll(&token, pair.runs[1]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
    }

    #[test]
    fn world_and_ready_boundaries_require_a_new_owned_gang_transaction() {
        for boundary in 0..3 {
            let pair = pair();
            let port = pair.seats[0].as_ref();
            let run = pair.runs[0];
            match boundary {
                0 => port.invalidate(run),
                1 => port.world_changed("127.0.0.1", 45594),
                _ => port.observe(
                    PairRegistration {
                        run,
                        pin: pin(),
                        settings: Some(PairSettings {
                            partner: Some(AccountKey(Arc::from("bob"))),
                            gang: Some(Gang::Phoenix),
                        }),
                        ready: false,
                        evidence: stamp(run, 3),
                    },
                    PairFrame::default(),
                ),
            }
            assert!(matches!(port.gang(run), Err(PairError::NotReady)));
            register(port, run, "bob", Gang::Phoenix, 3);
            assert!(matches!(port.gang(run), Err(PairError::UnknownGang)));
            register(port, run, "bob", Gang::Phoenix, 2);
            assert!(matches!(
                port.observe_gang(&read(run)),
                Err(PairError::Stale)
            ));
            register(port, run, "bob", Gang::Phoenix, 4);
            let mut delayed = read(run);
            delayed.closed = stamp(run, 4);
            assert!(
                matches!(port.observe_gang(&delayed), Err(PairError::Stale)),
                "a callback acquired before this ready epoch cannot restore its old proof"
            );
            delayed.acquired = stamp(run, 3);
            assert!(matches!(
                port.observe_gang(&delayed),
                Ok(Knowledge::Known(None))
            ));
            assert!(matches!(port.observe_gang(&delayed), Err(PairError::Stale)));
            register(port, run, "bob", Gang::Phoenix, 5);
            let (gang, evidence) = port.gang(run).unwrap();
            assert!(matches!(gang, Knowledge::Known(None)));
            assert_eq!(evidence, stamp(run, 4));
        }
    }

    #[test]
    fn saved_profile_aliases_of_one_native_account_cannot_partner() {
        let core = Arc::new(QuestPairCoordinator::default());
        core.set_accounts(["alice_1", "Alice 1"]);
        let seat = core.seat("alice_1", "127.0.0.1", 44594);
        let run = RunKey {
            slot: 1,
            run: 1,
            session: 1,
        };
        let plan = plan(false);
        assert_eq!(
            seat.begin(request(run, "Alice 1", 0, &plan)),
            Err(PairError::SelfPartner)
        );
        assert!(core.state.lock().unwrap().leases.is_empty());
    }

    #[test]
    fn reciprocal_owned_admission_never_dispatches_before_both_join() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Ok(PairStep::Waiting { .. }))
        ));
        assert!(pair.seats[0].busy());
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[1]),
            Poll::Ready(Err(PairError::Stale))
        ));
        assert_eq!(
            pair.seats[1]
                .begin(request(pair.runs[1], "alice", 1, &plan))
                .unwrap(),
            token
        );
        for side in 0..2 {
            assert!(matches!(
                pair.seats[side].poll(&token, pair.runs[side]),
                Poll::Ready(Ok(PairStep::Done(_)))
            ));
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
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "other",
            Gang::BlackArm,
            2,
        );
        assert_eq!(
            pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan)),
            Err(PairError::MissingPartner)
        );
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::Phoenix,
            2,
        );
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        assert_eq!(
            pair.seats[1].begin(request(pair.runs[1], "alice", 0, &plan)),
            Err(PairError::WrongGang)
        );
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
    }
    #[test]
    fn stop_revokes_peer_and_old_session_journal_cannot_rearm() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[0].invalidate(pair.runs[0]);
        assert!(matches!(
            pair.seats[1].poll(&token, pair.runs[1]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
        assert_eq!(
            pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)),
            Err(PairError::Cancelled)
        );
        let mut next = pair.runs[0];
        next.session += 1;
        register(pair.seats[0].as_ref(), next, "bob", Gang::Phoenix, 3);
        assert!(matches!(
            pair.seats[0].observe_gang(&read(pair.runs[0])),
            Err(PairError::Stale)
        ));
    }
    #[test]
    fn only_new_gameplay_feeds_rearm_and_total_phase_bound_never_moves() {
        let pair = pair();
        let plan = plan(false);
        let now = Instant::now();
        let token = pair
            .core
            .begin_at(
                &AccountKey(Arc::from("alice")),
                request(pair.runs[0], "bob", 0, &plan),
                now,
            )
            .unwrap();
        pair.seats[0].gameplay_progress(
            pair.runs[0],
            stamp(pair.runs[0], 2),
            now + Duration::from_secs(300),
        );
        let first = pair.core.state.lock().unwrap().leases[&token.id].deadline;
        pair.seats[0].gameplay_progress(
            pair.runs[0],
            stamp(pair.runs[0], 2),
            now + Duration::from_secs(500),
        );
        assert_eq!(
            pair.core.state.lock().unwrap().leases[&token.id].deadline,
            first
        );
        for step in 2..=11 {
            let tick = step + 1;
            register(
                pair.seats[0].as_ref(),
                pair.runs[0],
                "bob",
                Gang::Phoenix,
                tick,
            );
            pair.seats[0].gameplay_progress(
                pair.runs[0],
                stamp(pair.runs[0], tick),
                now + Duration::from_secs(step * 300),
            );
        }
        let state = pair.core.state.lock().unwrap();
        let lease = &state.leases[&token.id];
        assert_eq!(lease.total_deadline, now + TOTAL);
        assert_eq!(lease.deadline, now + TOTAL);
    }
    #[test]
    fn foreign_path_binding_waits_without_reserving_or_poisoning_peer() {
        for ready in [true, false] {
            let pair = pair();
            let plan = plan(false);
            let hero = FactKey::new("hero");
            let hero_protocol = FactKey::new("heroes");
            pair.seats[1].observe(
                PairRegistration {
                    run: pair.runs[1],
                    pin: pin(),
                    ready,
                    settings: Some(PairSettings {
                        partner: Some(AccountKey(Arc::from("alice"))),
                        gang: Some(Gang::BlackArm),
                    }),
                    evidence: stamp(pair.runs[1], 2),
                },
                PairFrame {
                    binding: Some(PairBinding {
                        path: &hero,
                        protocol: &hero_protocol,
                        digest: &[3; 32],
                        role: &plan.roles[1].id,
                    }),
                    inventory: None,
                },
            );
            let result = pair.seats[0].begin(request(pair.runs[0], "bob", 0, &plan));
            assert_eq!(result, Err(PairError::Busy));
            assert!(!pair.seats[0].busy() && !pair.seats[1].busy());
            assert!(pair.seats[0].waiting(pair.runs[0]));
            assert!(!pair.seats[1].waiting(pair.runs[1]));
            assert!(
                pair.core.state.lock().unwrap().entries[&AccountKey(Arc::from("bob"))]
                    .cancelled
                    .is_none()
            );
        }
    }
    fn assert_unjoined_peer_can_join_again(pair: &Pair, plan: &Arc<CompiledPairPlan>) {
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            4,
        );
        let next_alice = RunKey {
            run: pair.runs[0].run + 1,
            ..pair.runs[0]
        };
        register(pair.seats[0].as_ref(), next_alice, "bob", Gang::Phoenix, 1);
        register(pair.seats[0].as_ref(), next_alice, "bob", Gang::Phoenix, 2);
        assert!(matches!(
            pair.seats[0].observe_gang(&read(next_alice)),
            Ok(Knowledge::Known(None))
        ));

        let mut bob_request = request(pair.runs[1], "alice", 1, plan);
        bob_request.evidence = stamp(pair.runs[1], 4);
        let token = pair.seats[1]
            .begin(bob_request)
            .expect("the unjoined peer can be admitted after the other side restarts");
        assert_eq!(
            pair.seats[0].begin(request(next_alice, "bob", 0, plan)),
            Ok(token)
        );
    }

    #[test]
    fn same_path_unjoined_peer_survives_lease_cancel_and_can_rejoin() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .expect("same-Path peer is reserved for admission");
        assert!(pair.seats[1].busy());
        assert!(
            pair.core.state.lock().unwrap().leases[&token.id].joined[1].is_none(),
            "bob has not joined"
        );

        let hero = FactKey::new("hero");
        let hero_protocol = FactKey::new("heroes");
        pair.seats[1].observe(
            PairRegistration {
                run: pair.runs[1],
                pin: pin(),
                ready: true,
                settings: Some(PairSettings {
                    partner: Some(AccountKey(Arc::from("alice"))),
                    gang: Some(Gang::BlackArm),
                }),
                evidence: stamp(pair.runs[1], 3),
            },
            PairFrame {
                binding: Some(PairBinding {
                    path: &hero,
                    protocol: &hero_protocol,
                    digest: &[3; 32],
                    role: &plan.roles[1].id,
                }),
                inventory: None,
            },
        );

        assert!(pair.seats[1].settings(pair.runs[1]).is_ok());
        assert!(
            pair.core.state.lock().unwrap().entries[&AccountKey(Arc::from("bob"))]
                .cancelled
                .is_none(),
            "a side that never joined is not sticky-cancelled"
        );
        assert!(!pair.seats[1].busy());
        assert!(
            matches!(
                pair.seats[0].poll(&token, pair.runs[0]),
                Poll::Ready(Err(PairError::Cancelled))
            ),
            "the joined side's attempt is cancelled"
        );
        assert_unjoined_peer_can_join_again(&pair, &plan);
    }

    #[test]
    fn same_path_unjoined_peer_survives_lease_expiry_and_can_rejoin() {
        let pair = pair();
        let plan = plan(false);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .expect("same-Path peer is reserved for admission");
        assert!(pair.seats[1].busy());
        assert!(
            pair.core.state.lock().unwrap().leases[&token.id].joined[1].is_none(),
            "bob has not joined"
        );
        pair.core
            .state
            .lock()
            .unwrap()
            .leases
            .get_mut(&token.id)
            .unwrap()
            .deadline = Instant::now();

        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Err(PairError::BarrierExpired))
        ));
        assert!(pair.seats[1].settings(pair.runs[1]).is_ok());
        assert!(
            pair.core.state.lock().unwrap().entries[&AccountKey(Arc::from("bob"))]
                .cancelled
                .is_none(),
            "expiry must not cancel a side that never joined"
        );
        assert!(!pair.seats[1].busy());
        assert_unjoined_peer_can_join_again(&pair, &plan);
    }

    #[test]
    fn unbound_peer_waits_without_reserving_or_rearming_a_phase() {
        let pair = pair();
        pair.seats[1].observe(
            PairRegistration {
                run: pair.runs[1],
                pin: pin(),
                ready: true,
                settings: Some(PairSettings {
                    partner: Some(AccountKey(Arc::from("alice"))),
                    gang: Some(Gang::BlackArm),
                }),
                evidence: stamp(pair.runs[1], 2),
            },
            PairFrame::default(),
        );
        let plan = plan(true);
        let now = Instant::now();
        assert_eq!(
            pair.core.begin_at(
                &AccountKey(Arc::from("alice")),
                request(pair.runs[0], "bob", 0, &plan),
                now,
            ),
            Err(PairError::Busy)
        );
        pair.seats[1].gameplay_progress(
            pair.runs[1],
            stamp(pair.runs[1], 2),
            now + Duration::from_secs(300),
        );
        assert!(pair.core.state.lock().unwrap().leases.is_empty());
        assert!(!pair.seats[0].busy() && !pair.seats[1].busy());
        assert!(pair.seats[0].waiting(pair.runs[0]));
        assert!(!pair.seats[1].waiting(pair.runs[1]));
    }

    #[test]
    fn transient_ready_hold_pauses_inactivity_but_keeps_wall_clock_total_bound() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        let (deadline, total) = {
            let state = pair.core.state.lock().unwrap();
            (
                state.leases[&token.id].deadline,
                state.leases[&token.id].total_deadline,
            )
        };
        pair.seats[1].observe(
            PairRegistration {
                run: pair.runs[1],
                pin: pin(),
                ready: false,
                settings: Some(PairSettings {
                    partner: Some(AccountKey(Arc::from("alice"))),
                    gang: Some(Gang::BlackArm),
                }),
                evidence: stamp(pair.runs[1], 3),
            },
            PairFrame {
                binding: Some(PairBinding {
                    path: &plan.path,
                    protocol: &plan.protocol,
                    digest: &plan.digest,
                    role: &plan.roles[1].id,
                }),
                inventory: None,
            },
        );
        assert!(
            pair.seats[0].busy() && pair.seats[1].busy(),
            "a transient hold must keep both reservations"
        );
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Err(PairError::NotReady))
        ));
        assert!(pair.seats[0].waiting(pair.runs[0]) && pair.seats[1].waiting(pair.runs[1]));
        // A different side's gameplay cannot advance a paused phase.
        pair.seats[0].gameplay_progress(
            pair.runs[0],
            stamp(pair.runs[0], 2),
            Instant::now() + Duration::from_secs(300),
        );
        assert_eq!(
            pair.core.state.lock().unwrap().leases[&token.id].deadline,
            deadline
        );
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            4,
        );
        assert!(matches!(
            pair.seats[1].gang(pair.runs[1]),
            Ok((Knowledge::Known(None), _))
        ));
        let state = pair.core.state.lock().unwrap();
        assert!(state.leases[&token.id].deadline > deadline);
        assert_eq!(state.leases[&token.id].total_deadline, total);
        drop(state);
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Ok(PairStep::Act(_)))
        ));
    }

    #[test]
    fn quest_pair_r2_wall_clock_limit_expires_even_while_partner_is_held() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        register_ready(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            3,
            false,
        );
        {
            let mut state = pair.core.state.lock().unwrap();
            let lease = state.leases.get_mut(&token.id).unwrap();
            assert!(lease.paused_since.is_some());
            lease.total_deadline = Instant::now() - Duration::from_secs(1);
        }
        for side in 0..2 {
            let Poll::Ready(Err(error)) = pair.seats[side].poll(&token, pair.runs[side]) else {
                panic!("the wall-clock bound must end the held phase");
            };
            assert!(
                matches!(error.action(), ActionError::Blocked(reason) if reason.contains("60-minute wall-clock limit")),
                "the held phase must park with the limit and recovery reason"
            );
        }
        assert!(!pair.seats[0].busy() && !pair.seats[1].busy());
        assert!(!pair.seats[0].waiting(pair.runs[0]) && !pair.seats[1].waiting(pair.runs[1]));
        assert!(!pair.seats[0].held(pair.runs[0]) && !pair.seats[1].held(pair.runs[1]));
    }

    #[test]
    fn hold_racing_a_role_receipt_keeps_result_but_refuses_trade_acceptance() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        let commands = [
            command(pair.seats[0].as_ref(), &token, pair.runs[0]),
            command(pair.seats[1].as_ref(), &token, pair.runs[1]),
        ];
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            3,
        );
        register_ready(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            3,
            false,
        );
        assert_eq!(
            pair.seats[0].trade_ready(&token, pair.runs[0], false, stamp(pair.runs[0], 3)),
            Ok(false)
        );
        pair.seats[0]
            .report(&token, receipt(pair.runs[0], commands[0], 3))
            .unwrap();
        assert!(pair.seats[0].busy() && pair.seats[1].busy());
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            4,
        );
        pair.seats[1]
            .report(&token, receipt(pair.runs[1], commands[1], 4))
            .unwrap();
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Ok(PairStep::Done(_)))
        ));
    }

    #[test]
    fn recovery_item_queries_reject_paths_without_compiler_recovery_policy() {
        use script::quester::compile::{
            predicate_handlers, CompileContext, CompiledProgress, PairCompileContext,
        };
        use script::quester::loadouts::LoadoutOverlay;
        let selected = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let quests =
            api::quest_facts::QuestCatalog::from_identity(selected.quest_identity()).unwrap();
        let declaration = PartnerDeclaration {
            protocol: FactKey::new("test-pair"),
            roles: [
                PartnerRole {
                    id: FactKey::new("phoenix"),
                    gang: Gang::Phoenix,
                },
                PartnerRole {
                    id: FactKey::new("blackarm"),
                    gang: Gang::BlackArm,
                },
            ],
        };
        let progress = CompiledProgress {
            binding: FactKey::new("test-journal"),
            role: Some(declaration.roles[1].id.clone()),
            colour_not_started: FactKey::new("not-started"),
            colour_in_progress: FactKey::new("in-progress"),
            colour_complete: FactKey::new("complete"),
            stage_keys: Arc::from([]),
            rules: Arc::from([]),
            flags: Arc::from([]),
            monotonic: false,
        };
        let areas = HashMap::new();
        let recipes = HashMap::new();
        let loadouts = LoadoutOverlay::new(Arc::from([]), Arc::from([]));
        let handler = predicate_handlers()
            .find(|handler| handler.kind == "partner_item_count_at_least")
            .unwrap();
        // Compare the real compiler's policy with the real broker, not two
        // copies of an expected list. Either side drifting must break this test.
        for path in ["blackarmgang", "hero", "barcrawl", "cook"] {
            let path = FactKey::new(path);
            let cx = CompileContext {
                path: &path,
                kind: script::quester::path::PathKind::Quest,
                pair: Some(PairCompileContext {
                    declaration: &declaration,
                    role: &declaration.roles[1].id,
                    digest: [3; 32],
                }),
                progress: &progress,
                selected: &selected,
                quests: &quests,
                gathering: None,
                areas: &areas,
                recipes: &recipes,
                bank: None,
                bank_required: false,
                keep_ids: &[],
                loadouts: &loadouts,
            };
            let supported =
                (handler.compile)(&serde_json::json!({"obj":"arravshield2","qty":1}), &cx).is_ok();
            let pair = pair();
            let query = PairItemRequest {
                path: path.clone(),
                protocol: declaration.protocol.clone(),
                digest: [3; 32],
                own: declaration.roles[1].clone(),
                peer: declaration.roles[0].clone(),
                obj: 120,
            };
            publish_items(&pair, 0, &query, 2, vec![held(120, 1, 0)]);
            publish_items(&pair, 1, &query, 2, vec![]);
            let result = pair.seats[1].partner_item_count(stamp(pair.runs[1], 2), &query);
            assert_eq!(
                result.is_ok(),
                supported,
                "compiler/broker recovery policy differs for {}",
                path.0
            );
            if supported {
                assert_eq!(result, Ok(1));
            } else {
                assert_eq!(result, Err(PairError::Stale));
            }
        }
    }

    fn command(port: &dyn QuestPairPort, token: &PairToken, run: RunKey) -> u64 {
        match port.poll(token, run) {
            Poll::Ready(Ok(PairStep::Act(command))) => command.command_id,
            _ => panic!("expected own correlated role action"),
        }
    }
    fn receipt(run: RunKey, command_id: u64, tick: u64) -> RoleReceipt {
        RoleReceipt {
            actor: run,
            command_id,
            outcome: Ok(StepOutcome {
                progress: None,
                evidence: stamp(run, tick),
                receipt: None,
            }),
        }
    }
    #[test]
    fn role_commands_deduplicate_and_stale_duplicate_or_foreign_receipts_never_advance() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        let ids = [
            command(pair.seats[0].as_ref(), &token, pair.runs[0]),
            command(pair.seats[1].as_ref(), &token, pair.runs[1]),
        ];
        assert_eq!(
            command(pair.seats[0].as_ref(), &token, pair.runs[0]),
            ids[0]
        );
        assert_eq!(
            pair.seats[0].report(&token, receipt(pair.runs[1], ids[1], 3)),
            Err(PairError::Stale)
        );
        assert_eq!(
            pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 2)),
            Err(PairError::Stale)
        );
        assert_eq!(
            pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 3)),
            Err(PairError::Stale)
        );
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            3,
        );
        pair.seats[0]
            .report(&token, receipt(pair.runs[0], ids[0], 3))
            .unwrap();
        assert_eq!(
            pair.seats[0].report(&token, receipt(pair.runs[0], ids[0], 3)),
            Err(PairError::Stale)
        );
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            3,
        );
        pair.seats[1]
            .report(&token, receipt(pair.runs[1], ids[1], 3))
            .unwrap();
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Ok(PairStep::Done(_)))
        ));
    }
    #[test]
    fn configured_roster_includes_saved_accounts_and_removal_revokes_the_pair() {
        let pair = pair();
        pair.core.set_accounts(["saved", "bob", "alice", "saved"]);
        let accounts = pair.core.accounts();
        assert_eq!(
            accounts
                .iter()
                .map(|account| account.0.as_ref())
                .collect::<Vec<_>>(),
            ["alice", "bob", "saved"]
        );
        assert!(Arc::ptr_eq(&accounts, &pair.core.accounts()));
        let plan = plan(false);
        assert_eq!(
            pair.seats[0].begin(request(pair.runs[0], "saved", 0, &plan)),
            Err(PairError::PartnerNotInPlay)
        );
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.core.set_accounts(["alice", "saved"]);
        assert!(matches!(
            pair.seats[0].poll(&token, pair.runs[0]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
        pair.core.remember("bob");
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            3,
        );
        assert_eq!(
            pair.seats[1].begin(request(pair.runs[1], "alice", 1, &plan)),
            Err(PairError::UnknownAccount)
        );
        assert_eq!(pair.core.accounts().len(), 2);
    }

    #[test]
    fn late_role_receipt_expires_and_revokes_before_it_can_complete() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        let id = command(pair.seats[0].as_ref(), &token, pair.runs[0]);
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            3,
        );
        pair.core
            .state
            .lock()
            .unwrap()
            .leases
            .get_mut(&token.id)
            .unwrap()
            .deadline = Instant::now();
        assert_eq!(
            pair.seats[0].report(&token, receipt(pair.runs[0], id, 3)),
            Err(PairError::BarrierExpired)
        );
        assert!(matches!(
            pair.seats[1].poll(&token, pair.runs[1]),
            Poll::Ready(Err(PairError::Cancelled))
        ));
    }

    #[test]
    fn remote_cancel_and_roster_removal_fence_native_outbox_while_actor_slot_is_locked() {
        use script::native::{
            ActionContext, ActionHandle, NativeMachine, NativeTick, Script, ScriptFailure,
            ScriptFlow,
        };

        struct Queued;
        impl NativeMachine for Queued {
            type Args = ();
            type Output = ();
            fn begin(_: (), cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
                cx.emit(script::shim::InteractReq::CloseModal)?;
                Ok(Self)
            }
            fn poll(&mut self, _: &mut ActionContext<'_>) -> Poll<Result<(), ActionError>> {
                Poll::Pending
            }
            fn cancel(&mut self) {}
        }
        struct Actor {
            core: Arc<QuestPairCoordinator>,
            peer: Arc<dyn QuestPairPort>,
            mode: u8,
            handle: Option<ActionHandle<Queued>>,
            binding: Arc<CompiledPairPlan>,
        }
        impl Script for Actor {
            fn pair_settings(&self) -> Option<PairSettings> {
                Some(PairSettings {
                    partner: Some(AccountKey(Arc::from("bob"))),
                    gang: Some(Gang::Phoenix),
                })
            }
            fn pair_binding(&self) -> Option<PairBinding<'_>> {
                Some(PairBinding {
                    path: &self.binding.path,
                    protocol: &self.binding.protocol,
                    digest: &self.binding.digest,
                    role: &self.binding.roles[0].id,
                })
            }
            fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
                if tick.cx.evidence().tick == 1 {
                    return Ok(ScriptFlow::Continue);
                }
                let own = tick.cx.run();
                let peer = RunKey {
                    slot: own.slot + 1,
                    run: own.run,
                    session: own.session,
                };
                let port = tick.pairs.unwrap();
                register(self.peer.as_ref(), peer, "alice", Gang::BlackArm, 1);
                register(self.peer.as_ref(), peer, "alice", Gang::BlackArm, 2);
                port.observe_gang(&read(own)).unwrap();
                self.peer.observe_gang(&read(peer)).unwrap();
                let plan = plan(true);
                let token = port.begin(request(own, "bob", 0, &plan)).unwrap();
                self.peer.begin(request(peer, "alice", 1, &plan)).unwrap();
                let handle = tick.actions.begin::<Queued>((), &mut tick.cx).unwrap();
                if self.mode != 3 {
                    port.register_action(&token, own, handle.revoker()).unwrap();
                }
                self.handle = Some(handle);
                let core = Arc::clone(&self.core);
                let remote = Arc::clone(&self.peer);
                let mode = self.mode;
                std::thread::spawn(move || match mode {
                    0 => remote.invalidate(peer),
                    1 => core.set_accounts(["alice"]),
                    _ => {
                        remote.observe(
                            PairRegistration {
                                run: peer,
                                pin: pin(),
                                ready: false,
                                settings: Some(PairSettings {
                                    partner: Some(AccountKey(Arc::from("alice"))),
                                    gang: Some(Gang::BlackArm),
                                }),
                                evidence: stamp(peer, 3),
                            },
                            PairFrame {
                                binding: Some(PairBinding {
                                    path: &plan.path,
                                    protocol: &plan.protocol,
                                    digest: &plan.digest,
                                    role: &plan.roles[1].id,
                                }),
                                inventory: None,
                            },
                        );
                    }
                })
                .join()
                .unwrap();
                if self.mode == 3 {
                    port.register_action(&token, own, self.handle.as_ref().unwrap().revoker())
                        .unwrap();
                }
                Ok(ScriptFlow::Continue)
            }
        }
        for mode in 0..4 {
            let core = Arc::new(QuestPairCoordinator::default());
            core.set_accounts(["alice", "bob"]);
            let own = core.seat("alice", "127.0.0.1", 44594);
            let peer = core.seat("bob", "127.0.0.1", 44594);
            let slot = Arc::new(Mutex::new(script::SlotScript::new()));
            let mut slot = slot.lock().unwrap();
            slot.bind_quest_pairs(own);
            slot.start_test_script(
                Box::new(Actor {
                    core,
                    peer: Arc::clone(&peer),
                    mode,
                    handle: None,
                    binding: plan(true),
                }),
                None,
            )
            .unwrap();
            let mut snapshot = api::snapshot::GameSnapshot::new();
            snapshot.seed_ingame(2);
            let mut driver = crate::tests::nav_client();
            let mut cx = script::ScriptCtx {
                driver: &mut driver,
                tick: 2,
                here: Some((0, 0, 0)),
                walk: None,
                walk_with: None,
                inv: None,
                snapshot: Some(&snapshot),
                obj_names: None,
                compiled: script::CompiledTick::default(),
            };
            cx.tick = 1;
            slot.on_game_tick(&mut cx);
            cx.tick = 2;
            slot.on_game_tick(&mut cx);
            assert_eq!(
                slot.state(),
                script::RunState::Running,
                "no local Stop or slot cleanup"
            );
            assert!(
                !slot.has_native_actions(),
                "remote cancellation or a transient hold must fence queued authority"
            );
            assert!(
                slot.take_native_action().is_none(),
                "the final host fence must not drain cancelled or held input"
            );
            if mode >= 2 {
                let own_run = slot.native_run().unwrap();
                let peer_run = RunKey {
                    slot: own_run.slot + 1,
                    ..own_run
                };
                register(peer.as_ref(), peer_run, "alice", Gang::BlackArm, 4);
                assert!(
                    slot.has_native_actions(),
                    "held work must remain queued, not revoked"
                );
                assert!(
                    slot.take_native_action().is_some(),
                    "resumption must reopen the final dispatch gate"
                );
            }
        }
    }

    #[test]
    fn reservation_pauses_only_the_joined_waiter_not_the_approaching_actor() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        assert!(pair.seats[0].waiting(pair.runs[0]));
        assert!(
            !pair.seats[1].waiting(pair.runs[1]),
            "the peer is still running its own quest action"
        );
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        assert!(!pair.seats[0].waiting(pair.runs[0]) && !pair.seats[1].waiting(pair.runs[1]));
        let id = command(pair.seats[0].as_ref(), &token, pair.runs[0]);
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            3,
        );
        pair.seats[0]
            .report(&token, receipt(pair.runs[0], id, 3))
            .unwrap();
        assert!(pair.seats[0].waiting(pair.runs[0]));
        assert!(
            !pair.seats[1].waiting(pair.runs[1]),
            "its own admitted action watchdog remains live"
        );
    }

    #[test]
    fn exact_trade_barriers_require_both_owned_fresh_screens() {
        let pair = pair();
        let plan = plan(true);
        let token = pair.seats[0]
            .begin(request(pair.runs[0], "bob", 0, &plan))
            .unwrap();
        pair.seats[1]
            .begin(request(pair.runs[1], "alice", 1, &plan))
            .unwrap();
        assert_eq!(
            pair.seats[0].trade_ready(&token, pair.runs[0], false, stamp(pair.runs[0], 2)),
            Err(PairError::Stale)
        );
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            3,
        );
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            3,
        );
        assert!(!pair.seats[0]
            .trade_ready(&token, pair.runs[0], false, stamp(pair.runs[0], 3))
            .unwrap());
        assert_eq!(
            pair.seats[1].trade_ready(&token, pair.runs[1], false, stamp(pair.runs[0], 3)),
            Err(PairError::Stale)
        );
        assert_eq!(
            pair.seats[1].trade_ready(&token, pair.runs[1], false, stamp(pair.runs[1], 4)),
            Err(PairError::Stale)
        );
        assert!(pair.seats[1]
            .trade_ready(&token, pair.runs[1], false, stamp(pair.runs[1], 3))
            .unwrap());
        assert_eq!(
            pair.seats[0].trade_ready(&token, pair.runs[0], true, stamp(pair.runs[0], 3)),
            Err(PairError::Stale)
        );
        register(
            pair.seats[0].as_ref(),
            pair.runs[0],
            "bob",
            Gang::Phoenix,
            4,
        );
        assert!(!pair.seats[0]
            .trade_ready(&token, pair.runs[0], true, stamp(pair.runs[0], 4))
            .unwrap());
        register(
            pair.seats[1].as_ref(),
            pair.runs[1],
            "alice",
            Gang::BlackArm,
            4,
        );
        assert!(pair.seats[1]
            .trade_ready(&token, pair.runs[1], true, stamp(pair.runs[1], 4))
            .unwrap());
    }
}
