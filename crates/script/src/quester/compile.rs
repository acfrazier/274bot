//! Shared typed handler/runner seam; M-296 owns compilation and plan storage.
use crate::native::{ActionContext, ActionError, NativeActions, NativeTick};
use api::game_data::SelectedGameData;
use api::gather_methods::GatherCatalog;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceProvider, EvidenceStamp, QuestProgress};
use api::selected::{FactKey, SourceSpan, Truth};
use std::{any::Any, sync::Arc, task::Poll};

pub struct CompileContext<'a> {
    pub selected: &'a SelectedGameData,
    pub quests: &'a QuestCatalog,
    pub gathering: Option<&'a GatherCatalog>,
}
#[derive(Debug, Clone)]
pub struct CompileError {
    pub path: FactKey,
    pub role: Option<FactKey>,
    pub step: Option<FactKey>,
    pub code: Arc<str>,
    pub source: Option<SourceSpan>,
}
pub struct CompiledPath {
    _private: (),
}

pub struct PredicateContext<'a, 'frame> {
    pub cx: &'a ActionContext<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub outcome: Option<&'a StepOutcome>,
}
pub struct StepContext<'a, 'frame> {
    pub tick: &'a mut NativeTick<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub walk_evidence: &'a Arc<dyn EvidenceProvider>,
}
pub trait FamilyReceipt: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
}
pub struct StepOutcome {
    pub progress: Option<Arc<QuestProgress>>,
    pub evidence: EvidenceStamp,
    pub receipt: Option<Arc<dyn FamilyReceipt>>,
}
pub trait PredicatePlan: Send + Sync {
    fn evaluate(&self, cx: &PredicateContext<'_, '_>) -> Truth;
}
pub trait StepPlan: Send + Sync {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError>;
}
pub trait StepRun: Send {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>>;
    fn cancel(&mut self, actions: &mut NativeActions);
}
pub type CompileStep =
    fn(&serde_json::Value, &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError>;
pub type CompilePredicate =
    fn(&serde_json::Value, &CompileContext<'_>) -> Result<Arc<dyn PredicatePlan>, CompileError>;

pub struct StepHandler {
    pub kind: &'static str,
    pub version: u16,
    pub compile: CompileStep,
}
pub struct PredicateHandler {
    pub kind: &'static str,
    pub version: u16,
    pub compile: CompilePredicate,
}
