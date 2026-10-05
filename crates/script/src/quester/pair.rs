//! Two explicitly started reciprocal runs; no idle-helper or cross-account Start.
use super::compile::{StepOutcome, StepPlan};
use crate::native::ActionError;
use api::quest_progress::EvidenceStamp;
use api::selected::{FactKey, Knowledge, RunKey, SelectedPin};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, task::Poll, time::Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(transparent)]
pub struct AccountKey(pub Arc<str>);
impl std::borrow::Borrow<str> for AccountKey {
    fn borrow(&self) -> &str {
        self.0.as_ref()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
pub enum Gang {
    Phoenix,
    BlackArm,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PartnerRole {
    pub id: FactKey,
    pub gang: Gang,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "path-schema", derive(schemars::JsonSchema))]
#[serde(deny_unknown_fields)]
pub struct PartnerDeclaration {
    pub protocol: FactKey,
    pub roles: [PartnerRole; 2],
}

/// Immutable compiler-owned admission or exact two-role handoff.
pub struct CompiledPairPlan {
    pub path: FactKey,
    pub protocol: FactKey,
    pub digest: [u8; 32],
    pub phase: FactKey,
    pub roles: [PartnerRole; 2],
    /// Equality key includes the canonical role-ordered transfer and rendezvous.
    pub signature: [u8; 32],
    /// Admission is a barrier without gameplay effects; later phases own two actions.
    pub actions: Option<[Arc<dyn StepPlan>; 2]>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairSettings {
    pub partner: Option<AccountKey>,
    pub gang: Option<Gang>,
}
/// One current Quester incarnation. No snapshot/world or credentials are retained.
pub struct PairRegistration {
    pub run: RunKey,
    pub pin: Arc<SelectedPin>,
    pub settings: Option<PairSettings>,
    pub ready: bool,
    pub evidence: EvidenceStamp,
}

/// Borrowed identity of the paired Path currently active in this native run.
#[derive(Clone, Copy)]
pub struct PairBinding<'a> {
    pub path: &'a FactKey,
    pub protocol: &'a FactKey,
    pub digest: &'a [u8; 32],
    pub role: &'a FactKey,
}

/// Native frame data. The broker copies only bounded item ID/count receipts.
#[derive(Default)]
pub struct PairFrame<'a> {
    pub binding: Option<PairBinding<'a>>,
    pub inventory: Option<api::snapshot::Observed<&'a [api::snapshot::ItemView]>>,
}

/// Compiler-owned, quest-item-only query for a reciprocal role's current holdings.
pub struct PairItemRequest {
    pub path: FactKey,
    pub protocol: FactKey,
    pub digest: [u8; 32],
    pub own: PartnerRole,
    pub peer: PartnerRole,
    pub obj: i32,
}
pub struct PairRequest {
    pub caller: RunKey,
    pub partner: AccountKey,
    pub caller_role: FactKey,
    pub phase: FactKey,
    pub plan: Arc<CompiledPairPlan>,
    pub observed_gang: Knowledge<Option<Gang>>,
    pub declared_gang: Option<Gang>,
    pub evidence: EvidenceStamp,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PairToken {
    pub id: u64,
    pub generation: u64,
    pub left: RunKey,
    pub right: RunKey,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairError {
    MissingPartner,
    UnknownAccount,
    PartnerNotInPlay,
    SelfPartner,
    Busy,
    NotReady,
    DifferentWorld,
    WrongPin,
    WrongGang,
    UnknownGang,
    MissingRequirements,
    BarrierExpired,
    Stale,
    Cancelled,
    Failed(Arc<str>),
}
impl PairError {
    pub fn action(self) -> ActionError {
        let remedy = match &self {
            Self::MissingPartner => "select a configured partner account",
            Self::UnknownAccount => "fix the saved partner selection",
            Self::PartnerNotInPlay => {
                "load and explicitly Start the configured partner in this Play"
            }
            Self::SelfPartner => "select a different configured account",
            Self::Busy => "Stop this pair first",
            Self::NotReady => "both partners must be active and ready",
            Self::DifferentWorld => "load both partners in the same world",
            Self::WrongPin => "both partners must use the same selected content",
            Self::WrongGang => "choose opposite gangs matching the owned Arrav journals",
            Self::UnknownGang => "read the Arrav journal before choosing a role",
            Self::MissingRequirements => "satisfy both roles' handoff requirements",
            Self::BarrierExpired => "pair barrier expired; Stop and retry both accounts",
            Self::Stale | Self::Cancelled => "pair cancelled; Stop and freshly Start both accounts",
            Self::Failed(reason) => reason,
        };
        ActionError::Blocked(Arc::from(remedy))
    }
}
#[derive(Debug, Clone)]
pub struct PairReceipt {
    pub token: PairToken,
    pub phase: FactKey,
    pub evidence: [EvidenceStamp; 2],
}
pub struct RoleCommand {
    pub token: PairToken,
    pub recipient: RunKey,
    pub phase: FactKey,
    pub command_id: u64,
    pub plan: Arc<dyn StepPlan>,
}
pub struct RoleReceipt {
    pub actor: RunKey,
    pub command_id: u64,
    pub outcome: Result<StepOutcome, ActionError>,
}
pub enum PairStep {
    Act(RoleCommand),
    Waiting { phase: FactKey, deadline: Instant },
    Done(PairReceipt),
}
pub trait QuestPairPort: Send + Sync {
    fn shared(&self) -> Arc<dyn QuestPairPort>;
    fn observe(&self, registration: PairRegistration, frame: PairFrame<'_>);
    fn invalidate(&self, run: RunKey);
    fn busy(&self) -> bool;
    fn world_changed(&self, host: &str, port: u16);
    fn settings(&self, caller: RunKey) -> Result<PairSettings, PairError>;
    /// Accept only an owned Arrav read from this ready incarnation and pin.
    fn observe_gang(
        &self,
        read: &api::quest_progress::JournalRead,
    ) -> Result<Knowledge<Option<Gang>>, PairError>;
    fn gang(&self, caller: RunKey) -> Result<(Knowledge<Option<Gang>>, EvidenceStamp), PairError>;
    /// Read only the matching active reciprocal role's current native item receipt.
    fn partner_item_count(
        &self,
        caller: EvidenceStamp,
        request: &PairItemRequest,
    ) -> Result<i32, PairError>;
    fn waiting(&self, caller: RunKey) -> bool;
    fn token(&self, caller: RunKey, phase: &FactKey) -> Result<PairToken, PairError>;
    fn register_action(
        &self,
        token: &PairToken,
        actor: RunKey,
        action: crate::native::ActionRevoker,
    ) -> Result<(), PairError>;
    fn begin(&self, request: PairRequest) -> Result<PairToken, PairError>;
    fn poll(&self, token: &PairToken, caller: RunKey) -> Poll<Result<PairStep, PairError>>;
    fn report(&self, token: &PairToken, receipt: RoleReceipt) -> Result<(), PairError>;
    /// Only called after a fresh local screen proves exact counterpart and offers.
    fn trade_ready(
        &self,
        token: &PairToken,
        actor: RunKey,
        confirm: bool,
        evidence: EvidenceStamp,
    ) -> Result<bool, PairError>;
    fn gameplay_progress(&self, actor: RunKey, evidence: EvidenceStamp, now: Instant);
    fn cancel(&self, token: &PairToken);
}
