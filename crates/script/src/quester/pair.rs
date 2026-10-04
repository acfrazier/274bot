//! Two explicitly started reciprocal runs; no idle-helper or cross-account Start.
use super::compile::{StepOutcome, StepPlan};
use crate::native::ActionError;
use api::quest_progress::EvidenceStamp;
use api::selected::{FactKey, Knowledge, RunKey};
use serde::{Deserialize, Serialize};
use std::{sync::Arc, task::Poll, time::Instant};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountKey(pub Arc<str>);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct CompiledPairPlan {
    _private: (),
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
    fn begin(&self, request: PairRequest) -> Result<PairToken, PairError>;
    fn poll(&self, token: &PairToken, caller: RunKey) -> Poll<Result<PairStep, PairError>>;
    fn report(&self, token: &PairToken, receipt: RoleReceipt) -> Result<(), PairError>;
    fn cancel(&self, token: &PairToken);
}
