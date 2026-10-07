//! Lazy per-activation Path compiler and the `(pin, digest, ABI)` weak cache.
use super::families::{self, CompiledAcquireStep};
use super::path::{
    PathDocument, PredicateDocument, QuestItemDocument, QuestRequirementDocument,
    QuestRequirementKindDocument, StepDocument,
};
pub use super::progress::CompiledProgress;
use crate::combat::RaisedPrayers;
use crate::native::{ActionContext, ActionError, NativeActions, NativeTick};
use crate::native_bank::BankItem;
use api::game_data::SelectedGameData;
use api::gather_methods::GatherCatalog;
use api::named_banks::NamedBank;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, QuestProgress};
use api::selected::{
    ClientRevision, FactKey, FamilyPreparation, ItemAmount, QuestGate, RequirementKind,
    SkillMinimum, SourceSpan, Truth,
};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    task::Poll,
};

pub struct CompileContext<'a> {
    pub path: &'a FactKey,
    pub kind: super::path::PathKind,
    pub progress: &'a CompiledProgress,
    pub pair: Option<PairCompileContext<'a>>,
    pub selected: &'a SelectedGameData,
    pub quests: &'a QuestCatalog,
    pub gathering: Option<&'a Arc<GatherCatalog>>,
    pub areas: &'a HashMap<String, Vec<[i32; 5]>>,
    pub recipes: &'a HashMap<String, Arc<[CompiledAcquireStep]>>,
    /// `None` uses the shared eligible-bank cost selector at step start.
    pub bank: Option<NamedBank>,
    pub bank_required: bool,
    pub keep_ids: &'a [i32],
    pub loadouts: &'a super::loadouts::LoadoutOverlay,
}

#[derive(Clone, Copy)]
pub struct PairCompileContext<'a> {
    pub declaration: &'a super::pair::PartnerDeclaration,
    pub role: &'a FactKey,
    pub digest: [u8; 32],
}
#[derive(Debug, Clone)]
pub struct CompileError {
    pub path: FactKey,
    pub role: Option<FactKey>,
    pub step: Option<FactKey>,
    pub code: Arc<str>,
    pub detail: Option<Arc<str>>,
    pub source: Option<SourceSpan>,
}

impl CompileError {
    pub fn code(code: &'static str) -> Self {
        Self {
            path: FactKey::new(""),
            role: None,
            step: None,
            code: Arc::from(code),
            detail: None,
            source: None,
        }
    }

    pub fn with_path(mut self, path: FactKey) -> Self {
        self.path = path;
        self
    }

    pub fn with_detail(mut self, detail: impl Into<Arc<str>>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdvanceClass {
    Explicit,
    Default,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressRead {
    None,
    Colour,
    Journal,
}

pub fn decode_args<A: serde::de::DeserializeOwned>(
    value: &serde_json::Value,
) -> Result<A, CompileError> {
    serde_json::from_value(value.clone())
        .map_err(|error| CompileError::code("invalid-args").with_detail(error.to_string()))
}

#[macro_export]
macro_rules! step {
    ($kind:literal, $version:literal, $advance:ident, $args:ty, $compile:path) => {{
        $crate::quester::compile::StepHandler {
            kind: $kind,
            version: $version,
            advance: $crate::quester::compile::AdvanceClass::$advance,
            args_schema: $crate::quester::schema::args_schema::<$args>(),
            compile: |value, cx| {
                $compile($crate::quester::compile::decode_args::<$args>(value)?, cx)
            },
        }
    }};
}

#[macro_export]
macro_rules! fact {
    ($kind:literal, $version:literal, $progress:path, $args:ty, $compile:path) => {{
        $crate::quester::compile::PredicateHandler {
            kind: $kind,
            version: $version,
            progress: $progress,
            args_schema: $crate::quester::schema::args_schema::<$args>(),
            compile: |value, cx| {
                $compile($crate::quester::compile::decode_args::<$args>(value)?, cx)
            },
        }
    }};
}

pub(crate) use crate::{fact, step};

pub struct CompiledPath {
    pub id: FactKey,
    pub role: Option<FactKey>,
    pub kind: super::path::PathKind,
    pub partner: Option<super::pair::PartnerDeclaration>,
    pub display_name: Arc<str>,
    pub tested_stats: Option<Arc<[api::selected::SkillMinimum]>>,
    pub digest: [u8; 32],
    pub colour_not_started: FactKey,
    pub colour_in_progress: FactKey,
    pub colour_complete: FactKey,
    pub progress: CompiledProgress,
    pub progress_reader: Option<Box<CompiledStep>>,
    pub eligibility: CompiledEligibility,
    pub provisioning: CompiledProvisioning,
    pub prelude: Vec<CompiledStep>,
    pub sequences: Vec<CompiledSequence>,
    pub warnings: Vec<Arc<str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledItemKind {
    MustHave,
    Acquirable,
}

#[derive(Clone)]
pub struct CompiledQuestItem {
    pub id: i32,
    pub name: Arc<str>,
    pub qty: u32,
    pub kind: CompiledItemKind,
    pub acquire: Option<Arc<str>>,
    pub stackable: bool,
    /// Authored gate key, if any.
    pub from_stage: Option<FactKey>,
    /// Index into the compiled sequence list from which the item is due.
    pub from_stage_index: Option<usize>,
}

pub struct CompiledRequirement {
    pub id: FactKey,
    pub at_start: bool,
    pub source: Arc<str>,
    pub kind: RequirementKind,
}

pub struct CompiledEligibility {
    pub members: bool,
    pub requirements: Arc<[CompiledRequirement]>,
    pub items: Arc<[CompiledQuestItem]>,
}

#[derive(Clone)]
pub struct CompiledCarry {
    pub item: BankItem,
    pub qty: i32,
    pub stackable: bool,
    /// Unique per-Path bit in `Provisioner::carry_drawn`.
    pub latch_index: u8,
}

#[derive(Clone)]
pub struct CompiledRecipeItem {
    pub item: BankItem,
    pub qty: i32,
    pub stackable: bool,
}

pub struct CompiledAcquireRecipe {
    pub steps: Arc<[CompiledAcquireStep]>,
    /// The largest simultaneous recipe-input inventory observed by the compiler.
    /// The final quest output is planned separately.
    pub peak_items: Arc<[CompiledRecipeItem]>,
    /// Items explicitly proven absent by an authored recipe settle predicate.
    pub consumed_ids: Arc<[i32]>,
}
#[derive(Clone)]
pub struct CompiledGatherToolNeed {
    pub item_id: i32,
    pub catalog: Arc<GatherCatalog>,
    pub methods: Arc<[usize]>,
    pub tools: Arc<[BankItem]>,
}

pub struct CompiledProvisioning {
    pub path: FactKey,
    pub owns_inventory: bool,
    pub bank: Option<NamedBank>,
    pub bank_required: bool,
    pub items: Arc<[CompiledQuestItem]>,
    /// Stage keys in compiled sequence order; item gates resolve against this.
    pub stages: Arc<[FactKey]>,
    pub tools: Arc<[BankItem]>,
    pub gather_tool_needs: Arc<[CompiledGatherToolNeed]>,
    pub keep_ids: Arc<[i32]>,
    pub coin_float: i32,
    pub coin: Option<CompiledCarry>,
    pub loadout_carry: HashMap<Arc<str>, Arc<[CompiledCarry]>>,
    pub base_spillover_keep: Arc<[i32]>,
    pub recipes: HashMap<Arc<str>, CompiledAcquireRecipe>,
}

pub struct CompiledSequence {
    pub stage: FactKey,
    pub terminal: bool,
    pub order: super::path::SequenceOrder,
    pub steps: Vec<CompiledStep>,
}

pub struct CompiledStep {
    pub id: FactKey,
    pub kind: Arc<str>,
    pub comment: Option<Arc<str>>,
    pub loadout: Option<Arc<str>>,
    pub tactic: Option<Arc<str>>,
    pub advances: bool,
    pub skip_if: Arc<dyn PredicatePlan>,
    pub skip_if_summary: Arc<str>,
    pub settle: Arc<dyn PredicatePlan>,
    pub plan: Arc<dyn StepPlan>,
}

/// The frame a predicate reads. Bank facts come from `cx.snapshot().stock()`:
/// the account's bank memory rides on the frame borrow
/// (design-bank-snapshot §1.2).
pub struct PredicateContext<'a, 'frame> {
    pub cx: &'a ActionContext<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub chat_since: i32,
    pub outcome: Option<&'a StepOutcome>,
    pub pairs: Option<&'a dyn super::pair::QuestPairPort>,
}
pub struct StepContext<'a, 'frame> {
    pub tick: &'a mut NativeTick<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub banks: &'a Arc<api::named_banks::NamedBankFacts>,
    /// Runtime account choices; never captured by a shared compiled plan.
    pub choices: &'a super::choices::QuestChoices,
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
    /// Whether an `Unknown` answer can be resolved by observing the bank: the
    /// runner then runs one provisioning scan while the memory is `Unknown`.
    fn requires_bank(&self) -> bool {
        false
    }
}
pub trait StepPlan: Send + Sync {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError>;
    /// Stable authored world anchor for nearest-first selection.
    fn anchor(&self) -> Option<api::WorldTile> {
        None
    }
    /// Post-machine predicate window, measured on the eligible clock.
    fn settle_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(8)
    }
    fn compile_warning(&self) -> Option<&'static str> {
        None
    }
    /// A copy of this plan that knows its step's own `skip_if` and `settle`.
    /// Only `acquire` uses it, to restart its recipe while the step would
    /// still run and its goal is still false; every other family returns
    /// `None` and keeps its plan.
    fn with_goal(&self, _goal: StepGoal) -> Option<Arc<dyn StepPlan>> {
        None
    }
}
/// A step's compiled `skip_if` and `settle`, plus the authored settle
/// summary, handed to the plan by [`StepPlan::with_goal`].
#[derive(Clone)]
pub struct StepGoal {
    pub skip_if: Arc<dyn PredicatePlan>,
    pub settle: Arc<dyn PredicatePlan>,
    pub summary: Arc<str>,
}
#[derive(Debug, Clone)]
pub enum StepTraceEvent {
    Acquisition {
        recipe: Arc<str>,
        child_step: Arc<str>,
        outcome: AcquisitionTraceOutcome,
    },
    CombatSubOperationEnd {
        target: crate::combat::Target,
        end: crate::combat::CombatEnd,
    },
}

#[derive(Debug, Clone)]
pub enum AcquisitionTraceOutcome {
    Begin,
    Skipped(Arc<str>),
    Settled,
    Failed(Arc<str>),
    /// The last child settled but the acquire step's goal is still false, so
    /// the recipe starts again from its first step (restart `restart` of `max`).
    Restarted {
        restart: u8,
        max: u8,
        goal: Arc<str>,
    },
}

pub trait StepRun: Send {
    fn poll(&mut self, cx: &mut StepContext<'_, '_>) -> Poll<Result<StepOutcome, ActionError>>;
    fn cancel(&mut self, actions: &mut NativeActions);
    /// Combat raises retained for a cancellation/error hygiene handoff.
    fn prayer_cleanup(&self) -> RaisedPrayers {
        RaisedPrayers::empty()
    }
    /// Recipe steps delegate journal ownership to the runner while remaining
    /// alive; no family opens a second journal transaction.
    fn needs_progress_read(&self) -> bool {
        false
    }
    fn progress_read_completed(&mut self, _now: std::time::Duration) {}
    fn needs_bank_scan(&self) -> bool {
        false
    }
    fn bank_scan_completed(&mut self) {}
    /// Borrowed wait detail; machines do not allocate on pending polls.
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        None
    }
    /// Latest completed sub-operation, borrowed for change-only status reporting.
    /// This does not replace the final outcome returned by `poll`.
    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
        None
    }
    /// Current acquisition child identity, without replacing the root step id.
    fn child_step_id(&self) -> Option<&FactKey> {
        None
    }
    /// Current acquisition recipe, available without formatting during polls.
    fn child_recipe_id(&self) -> Option<&Arc<str>> {
        None
    }
    fn take_trace_event(&mut self) -> Option<StepTraceEvent> {
        None
    }
}
pub type CompileStep =
    fn(&serde_json::Value, &CompileContext<'_>) -> Result<Arc<dyn StepPlan>, CompileError>;
pub type CompilePredicate =
    fn(&serde_json::Value, &CompileContext<'_>) -> Result<Arc<dyn PredicatePlan>, CompileError>;

pub struct StepHandler {
    pub kind: &'static str,
    pub version: u16,
    pub compile: CompileStep,
    pub advance: AdvanceClass,
    pub args_schema: super::schema::ArgsSchema,
}
pub struct PredicateHandler {
    pub kind: &'static str,
    pub version: u16,
    pub compile: CompilePredicate,
    pub progress: ProgressRead,
    pub args_schema: super::schema::ArgsSchema,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    revision: u16,
    engine: Arc<str>,
    content: Arc<str>,
    digest: [u8; 32],
    abi: u64,
    gang: Option<super::pair::Gang>,
}

static CACHE: std::sync::LazyLock<Mutex<HashMap<CacheKey, Weak<CompiledPath>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn step_handlers() -> impl Iterator<Item = &'static StepHandler> {
    families::handlers().iter().chain(super::handlers::steps())
}
pub fn predicate_handlers() -> impl Iterator<Item = &'static PredicateHandler> {
    families::predicate_handlers()
        .iter()
        .chain(super::handlers::predicates())
}

pub fn abi_set() -> u64 {
    let mut hasher = Sha256::new();
    let mut rows: Vec<_> = step_handlers()
        .map(|handler| ("step", handler.kind, handler.version))
        .chain(predicate_handlers().map(|handler| ("predicate", handler.kind, handler.version)))
        .collect();
    rows.sort_unstable();
    for (class, kind, version) in rows {
        hasher.update(class.as_bytes());
        hasher.update(kind.as_bytes());
        hasher.update(version.to_le_bytes());
    }
    let out = hasher.finalize();
    u64::from_le_bytes(out[..8].try_into().unwrap())
}

pub fn digest_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn compile_path(
    bytes: &[u8],
    selected: &SelectedGameData,
    quests: &QuestCatalog,
    worker: &mut FamilyPreparation,
) -> Result<Arc<CompiledPath>, CompileError> {
    compile_path_for_gang(bytes, selected, quests, worker, None)
}

/// Select an immutable gang role after the owned membership read.
pub fn compile_path_for_gang(
    bytes: &[u8],
    selected: &SelectedGameData,
    quests: &QuestCatalog,
    worker: &mut FamilyPreparation,
    gang: Option<super::pair::Gang>,
) -> Result<Arc<CompiledPath>, CompileError> {
    let digest = digest_bytes(bytes);
    let (revision, engine, content) = match selected.selected_pin() {
        Ok(pin) => {
            let revision = match pin.revision {
                ClientRevision::R274 => 274,
                ClientRevision::R289 => 289,
            };
            (
                revision,
                Arc::clone(&pin.engine_commit),
                Arc::clone(&pin.content_commit),
            )
        }
        Err(_) => return Err(CompileError::code("missing-pin")),
    };
    let key = CacheKey {
        revision,
        engine,
        content,
        digest,
        abi: abi_set(),
        gang,
    };
    if let Ok(cache) = CACHE.lock() {
        if let Some(hit) = cache.get(&key).and_then(Weak::upgrade) {
            return Ok(hit);
        }
    }
    let document: PathDocument = serde_json::from_slice(bytes)
        .map_err(|error| CompileError::code("invalid-json").with_detail(error.to_string()))?;
    let gathering = prepare_gathering(&document, selected, worker)?;
    let compiled = Arc::new(compile_uncached(
        &document, digest, selected, quests, gathering, gang,
    )?);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, weak| weak.strong_count() > 0);
        cache.insert(key, Arc::downgrade(&compiled));
    }
    Ok(compiled)
}

pub(super) fn prepare_gathering(
    document: &PathDocument,
    selected: &SelectedGameData,
    worker: &mut FamilyPreparation,
) -> Result<Option<Arc<GatherCatalog>>, CompileError> {
    uses_gathering(document)
        .then(|| selected.prepare_gathering(worker))
        .transpose()
        .map_err(|_| CompileError::code("gathering-unavailable").with_path(document.id.clone()))
}

fn uses_gathering(document: &PathDocument) -> bool {
    document.roles.iter().any(|role| {
        role.prelude
            .iter()
            .chain(role.sequences.iter().flat_map(|sequence| &sequence.steps))
            .chain(role.progress_reader.iter())
            .any(|step| step.kind == "gather")
    }) || document.quest.as_ref().is_some_and(|header| {
        header
            .acquire
            .values()
            .flatten()
            .any(|step| step.kind == "gather")
    })
}

/// Run test-only family preparation on the same off-pump worker as production.
#[cfg(any(test, feature = "test-hooks"))]
pub fn prepare_for_test<R: Send + 'static>(
    work: impl FnOnce(&mut FamilyPreparation) -> R + Send + 'static,
) -> R {
    FamilyPreparation::run(work)
        .expect("start test family preparation")
        .join()
        .expect("join test family preparation")
}

/// Test helper: compile without the process cache.
pub fn compile_uncached_for_test(
    document: &PathDocument,
    selected: &SelectedGameData,
    quests: &QuestCatalog,
) -> Result<Arc<CompiledPath>, CompileError> {
    let gathering = uses_gathering(document)
        .then(|| api::gather_methods::cached(selected))
        .flatten();
    compile_uncached(
        document,
        digest_bytes(b"test"),
        selected,
        quests,
        gathering,
        None,
    )
    .map(Arc::new)
}

fn compile_requirement(
    selected: &SelectedGameData,
    requirement: &QuestRequirementDocument,
) -> Result<CompiledRequirement, CompileError> {
    let kind = match &requirement.kind {
        QuestRequirementKindDocument::QuestPoints(points) => RequirementKind::QuestPoints(*points),
        QuestRequirementKindDocument::Skill { skill, level } => {
            let skill_id = skill_index(skill)
                .ok_or_else(|| CompileError::code("unknown-skill").with_detail(skill.as_str()))?;
            RequirementKind::Skill(SkillMinimum {
                skill: skill_id,
                level: *level,
            })
        }
        QuestRequirementKindDocument::Quest(quest) => {
            RequirementKind::Quest(QuestGate::Complete(FactKey::new(quest)))
        }
        QuestRequirementKindDocument::Item { obj, qty } => {
            let item = selected
                .item_by_alias(obj)
                .ok_or_else(|| CompileError::code("unresolved-item").with_detail(obj.as_str()))?;
            RequirementKind::Item(ItemAmount {
                item: item.id,
                count: *qty,
            })
        }
        QuestRequirementKindDocument::MembersWorld => RequirementKind::MembersWorld,
    };
    Ok(CompiledRequirement {
        id: requirement.id.clone(),
        at_start: requirement.at.eq_ignore_ascii_case("start"),
        source: Arc::from(requirement.source.as_str()),
        kind,
    })
}

fn skill_index(name: &str) -> Option<u8> {
    const SKILLS: [&str; 23] = [
        "attack",
        "defence",
        "strength",
        "hitpoints",
        "ranged",
        "prayer",
        "magic",
        "cooking",
        "woodcutting",
        "fletching",
        "fishing",
        "firemaking",
        "crafting",
        "smithing",
        "mining",
        "herblore",
        "agility",
        "thieving",
        "slayer",
        "farming",
        "runecraft",
        "hunter",
        "construction",
    ];
    SKILLS
        .iter()
        .position(|skill| skill.eq_ignore_ascii_case(name.trim()))
        .and_then(|id| u8::try_from(id).ok())
}

pub(super) fn compile_uncached(
    document: &PathDocument,
    digest: [u8; 32],
    selected: &SelectedGameData,
    quests: &QuestCatalog,
    gathering: Option<Arc<GatherCatalog>>,
    gang: Option<super::pair::Gang>,
) -> Result<CompiledPath, CompileError> {
    if document.schema != super::path::PATH_SCHEMA {
        return Err(CompileError::code("unsupported-schema").with_path(document.id.clone()));
    }
    let header = document
        .quest
        .as_ref()
        .ok_or_else(|| CompileError::code("missing-quest-header").with_path(document.id.clone()))?;
    validate_header(header, selected).map_err(|err| err.with_path(document.id.clone()))?;
    validate_nav_coverage(document).map_err(|err| err.with_path(document.id.clone()))?;
    let role = if let Some(declaration) = &document.partner {
        if document.roles.len() != 2
            || declaration.roles[0].id == declaration.roles[1].id
            || declaration.roles[0].gang == declaration.roles[1].gang
        {
            return Err(CompileError::code("invalid-partner-roles").with_path(document.id.clone()));
        }
        let gang =
            gang.ok_or_else(|| CompileError::code("missing-gang").with_path(document.id.clone()))?;
        let selected_role = declaration
            .roles
            .iter()
            .find(|role| role.gang == gang)
            .ok_or_else(|| CompileError::code("invalid-partner-role"))?;
        document
            .roles
            .iter()
            .find(|role| role.role.as_ref() == Some(&selected_role.id))
            .ok_or_else(|| CompileError::code("missing-partner-role"))?
    } else {
        if document.roles.len() != 1 {
            return Err(CompileError::code("invalid-solo-roles").with_path(document.id.clone()));
        }
        &document.roles[0]
    };
    if document.kind == super::path::PathKind::Miniquest && role.progress_reader.is_none() {
        return Err(CompileError::code("missing-miniquest-reader").with_path(document.id.clone()));
    }
    let progress = role
        .progress
        .as_ref()
        .ok_or_else(|| CompileError::code("missing-progress").with_path(document.id.clone()))?;
    let compiled_progress = super::progress::compile_progress(document, role, progress, quests)
        .map_err(|err| err.with_path(document.id.clone()))?;
    let mut areas = HashMap::new();
    for (name, area) in &header.areas {
        areas.insert(name.clone(), area.boxes.clone());
    }
    let empty_recipes = HashMap::new();
    let (bank, bank_required) = match &header.bank {
        super::path::QuestBankDocument::Nearest(_) => (None, false),
        super::path::QuestBankDocument::Tile { tile, required, .. } => (
            Some(NamedBank::new(
                "Path bank",
                api::WorldTile {
                    x: tile[0],
                    z: tile[1],
                    level: tile[2],
                },
            )),
            *required,
        ),
    };
    // Path headers use config aliases; the shared Loadouts consumer uses
    // display names. Resolve each kit row once, without an intermediate copy.
    let loadout_item_name = |alias: &str| -> Result<&str, CompileError> {
        let item = selected
            .item_by_alias(alias)
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))?;
        if item.is_certificate() {
            return Err(CompileError::code("certificate-loadout-item"));
        }
        item.name
            .as_deref()
            .ok_or_else(|| CompileError::code("unresolved-obj-name"))
    };
    let compiled_loadouts: Vec<_> = header
        .loadouts
        .iter()
        .map(|(name, row)| {
            let mut out = crate::loadouts_store::Loadout::new(format!("{}/{name}", document.id.0));
            for (slot, item) in &row.worn {
                if !crate::loadouts_store::is_worn_slot(slot) {
                    return Err(CompileError::code("invalid-worn-slot"));
                }
                out = out.with_slot(slot, loadout_item_name(item)?);
            }
            for carry in &row.carry {
                let item = loadout_item_name(&carry.item)?;
                if carry.qty == 0 {
                    return Err(CompileError::code("invalid-quantity"));
                }
                out = out.with_carry(item, carry.qty);
            }
            Ok(out)
        })
        .collect::<Result<Vec<_>, CompileError>>()
        .map_err(|error| error.with_path(document.id.clone()))?;
    let loadouts =
        super::loadouts::LoadoutOverlay::from_default_store(Arc::from(compiled_loadouts));
    let mut compiled_items: Vec<CompiledQuestItem> = header
        .items
        .iter()
        .map(|item| compile_quest_item(selected, item))
        .collect::<Result<Vec<_>, _>>()?;
    // Stage order is the compiled sequence order, so gates resolve here
    // against the authored role sequences before either consumer clones them.
    for item in compiled_items.iter_mut() {
        let Some(gate) = item.from_stage.as_ref() else {
            continue;
        };
        let index = role
            .sequences
            .iter()
            .position(|sequence| sequence.stage == *gate)
            .ok_or_else(|| {
                CompileError::code("unknown-from-stage")
                    .with_path(document.id.clone())
                    .with_detail(gate.0.as_ref())
            })?;
        item.from_stage_index = Some(index);
    }
    let mut recipe_peaks = HashMap::with_capacity(header.acquire.len());
    for (name, steps) in &header.acquire {
        let output_ids: Vec<_> = compiled_items
            .iter()
            .filter(|item| item.acquire.as_deref() == Some(name.as_str()))
            .map(|item| item.id)
            .collect();
        let (peak_items, consumed_ids) = recipe_inventory_peak(selected, steps, &output_ids)
            .map_err(|error| error.with_path(document.id.clone()))?;
        recipe_peaks.insert(name.clone(), (peak_items, consumed_ids));
    }
    let gather_steps = header
        .acquire
        .values()
        .flat_map(|steps| steps.iter())
        .chain(role.prelude.iter())
        .chain(
            role.sequences
                .iter()
                .flat_map(|sequence| sequence.steps.iter()),
        )
        .chain(role.progress_reader.iter());
    let mut gather_tool_needs = Vec::new();
    for step in gather_steps {
        if step.kind == "gather" && step.version == 1 {
            let catalog = gathering
                .as_ref()
                .ok_or_else(|| CompileError::code("gathering-unavailable"))?;
            gather_tool_needs.push(families::gather::provisioning_tool_need(
                &step.args,
                Arc::clone(catalog),
                selected,
            )?);
        }
    }
    let mut tools = Vec::with_capacity(header.tools.len());
    for tool in &header.tools {
        let alias = tool
            .strip_prefix("obj:")
            .ok_or_else(|| CompileError::code("invalid-tool"))?;
        tools.push(resolve_bank_item(selected, alias)?);
    }
    for need in &gather_tool_needs {
        for item in need.tools.iter() {
            if !tools.iter().any(|known: &BankItem| known.id == item.id) {
                tools.push(item.clone());
            }
        }
    }
    let mut loadout_carry: HashMap<Arc<str>, Arc<[CompiledCarry]>> = HashMap::new();
    let mut carry_row_count = 0usize;
    for name in header.loadouts.keys() {
        let qualified = format!("{}/{}", document.id.0, name);
        let row = loadouts
            .resolve(&qualified)
            .ok_or_else(|| CompileError::code("unknown-loadout"))?
            .row();
        let mut carry = Vec::with_capacity(row.carry.len());
        for entry in &row.carry {
            let (item, stackable) = resolve_bank_item_with_stackable(selected, &entry.item)?;
            let qty =
                i32::try_from(entry.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
            if carry_row_count >= u64::BITS as usize {
                return Err(CompileError::code("too-many-carry-rows"));
            }
            let latch_index = carry_row_count as u8;
            carry_row_count += 1;
            carry.push(CompiledCarry {
                item,
                qty,
                stackable,
                latch_index,
            });
        }
        loadout_carry.insert(Arc::from(qualified), Arc::from(carry));
    }
    let coin_float =
        i32::try_from(header.coin_float).map_err(|_| CompileError::code("invalid-coin-float"))?;
    let coin = (coin_float > 0)
        .then(|| resolve_bank_item_with_stackable(selected, "coins"))
        .transpose()?
        .map(|(item, stackable)| CompiledCarry {
            item,
            qty: coin_float,
            stackable,
            latch_index: u8::MAX,
        });
    let keep_ids = protected_item_ids(selected, &tools, &loadouts);
    let mut base_spillover_keep = keep_ids.clone();
    for item in compiled_items.iter() {
        push_unique_id(&mut base_spillover_keep, item.id);
    }
    for (peak_items, consumed_ids) in recipe_peaks.values() {
        for item in peak_items.iter() {
            push_unique_id(&mut base_spillover_keep, item.item.id);
        }
        for id in consumed_ids.iter().copied() {
            push_unique_id(&mut base_spillover_keep, id);
        }
    }
    if let Some(coin) = &coin {
        push_unique_id(&mut base_spillover_keep, coin.item.id);
    }
    let base_spillover_keep = Arc::from(base_spillover_keep);
    let keep_ids = Arc::from(keep_ids);
    let requirements = header
        .requirements
        .iter()
        .map(|requirement| compile_requirement(selected, requirement))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.with_path(document.id.clone()))?;
    let eligibility = CompiledEligibility {
        members: header.members,
        requirements: Arc::from(requirements),
        items: Arc::from(compiled_items.clone()),
    };
    let mut recipe_ctx = CompileContext {
        path: &document.id,
        kind: document.kind,
        progress: &compiled_progress,
        pair: document
            .partner
            .as_ref()
            .zip(role.role.as_ref())
            .map(|(declaration, role)| PairCompileContext {
                declaration,
                role,
                digest,
            }),
        selected,
        quests,
        gathering: gathering.as_ref(),
        areas: &areas,
        recipes: &empty_recipes,
        bank,
        bank_required,
        loadouts: &loadouts,
        keep_ids: &keep_ids,
    };
    let mut recipes: HashMap<String, Arc<[CompiledAcquireStep]>> =
        HashMap::with_capacity(header.acquire.len());
    let mut bindings = HashMap::with_capacity(header.acquire.len());
    let mut active = Vec::with_capacity(header.acquire.len().min(MAX_RECIPE_NESTING_DEPTH));
    for name in header.acquire.keys() {
        compile_recipe(
            name,
            document,
            &recipe_ctx,
            &mut recipes,
            &mut bindings,
            &mut active,
        )?;
    }
    recipe_ctx.recipes = &recipes;
    let progress_reader = role
        .progress_reader
        .as_ref()
        .map(|reader| {
            compile_steps(std::slice::from_ref(reader), &recipe_ctx, document)
                .map(|mut steps| Box::new(steps.remove(0)))
        })
        .transpose()?;
    let mut warnings: Vec<Arc<str>> = Vec::new();
    if role.prelude.len() > 4 {
        warnings.push(Arc::from("prelude-size"));
    }
    let prelude = compile_steps(&role.prelude, &recipe_ctx, document)?;
    let mut sequences = Vec::new();
    let mut seen_stages = std::collections::HashSet::new();
    let mut global_ids = std::collections::HashSet::new();
    for step in header.acquire.values().flatten() {
        if !global_ids.insert(step.id.0.clone()) {
            return Err(CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("duplicate-step"),
                detail: None,
                source: None,
            });
        }
    }
    for step in role
        .prelude
        .iter()
        .chain(role.sequences.iter().flat_map(|s| s.steps.iter()))
        .chain(role.progress_reader.iter())
    {
        if !global_ids.insert(step.id.0.clone()) {
            return Err(CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("duplicate-step"),
                detail: None,
                source: None,
            });
        }
    }
    for sequence in &role.sequences {
        if !seen_stages.insert(sequence.stage.0.clone()) {
            return Err(CompileError::code("duplicate-stage").with_path(document.id.clone()));
        }
        if sequence.steps.is_empty() && !sequence.terminal {
            return Err(CompileError::code("empty-nonterminal").with_path(document.id.clone()));
        }
        let steps = compile_steps(&sequence.steps, &recipe_ctx, document)?;
        if sequence.order == super::path::SequenceOrder::Nearest
            && steps.iter().any(|step| step.plan.anchor().is_none())
        {
            return Err(
                CompileError::code("nearest-step-missing-anchor").with_path(document.id.clone())
            );
        }
        sequences.push(CompiledSequence {
            stage: sequence.stage.clone(),
            terminal: sequence.terminal,
            order: sequence.order,
            steps,
        });
    }
    for plan in recipes
        .values()
        .flat_map(|steps| steps.iter())
        .map(|step| &step.plan)
        .chain(prelude.iter().map(|step| &step.plan))
        .chain(
            sequences
                .iter()
                .flat_map(|seq| seq.steps.iter().map(|step| &step.plan)),
        )
    {
        if let Some(warning) = plan.compile_warning() {
            if !warnings.iter().any(|existing| existing.as_ref() == warning) {
                warnings.push(Arc::from(warning));
            }
        }
    }
    let provisioning_recipes: HashMap<Arc<str>, CompiledAcquireRecipe> = recipes
        .into_iter()
        .map(|(name, steps)| {
            let (peak_items, consumed_ids) = recipe_peaks
                .remove(&name)
                .expect("compiled recipe peak came from the validated header");
            Ok((
                Arc::from(name.as_str()),
                CompiledAcquireRecipe {
                    steps,
                    peak_items,
                    consumed_ids,
                },
            ))
        })
        .collect::<Result<_, CompileError>>()?;
    let provisioning = CompiledProvisioning {
        path: document.id.clone(),
        owns_inventory: header.owns_inventory,
        bank,
        bank_required,
        items: Arc::from(compiled_items),
        stages: sequences
            .iter()
            .map(|sequence| sequence.stage.clone())
            .collect(),
        tools: Arc::from(tools),
        gather_tool_needs: Arc::from(gather_tool_needs),
        keep_ids,
        coin_float,
        coin,
        loadout_carry,
        base_spillover_keep,
        recipes: provisioning_recipes,
    };
    Ok(CompiledPath {
        id: document.id.clone(),
        role: role.role.clone(),
        kind: document.kind,
        partner: document.partner.clone(),
        display_name: Arc::from(document.display_name.as_str()),
        tested_stats: document.tested_stats.as_deref().map(Arc::from),
        digest,
        colour_not_started: progress.colour.not_started.clone(),
        colour_complete: progress.colour.complete.clone(),
        colour_in_progress: progress.colour.in_progress.clone(),
        progress: compiled_progress,
        progress_reader,
        eligibility,
        provisioning,
        prelude,
        sequences,
        warnings,
    })
}

fn push_unique_id(ids: &mut Vec<i32>, id: i32) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

pub(crate) fn protected_item_ids(
    selected: &SelectedGameData,
    path_tools: &[BankItem],
    loadouts: &super::loadouts::LoadoutOverlay,
) -> Vec<i32> {
    let mut ids = Vec::with_capacity(path_tools.len());
    for item in path_tools {
        push_unique_id(&mut ids, item.id);
    }
    for tool in api::gather_tools::AXES
        .iter()
        .chain(api::gather_tools::PICKAXES)
    {
        if let Some(item) = selected.item_by_alias(tool.alias) {
            push_unique_id(&mut ids, item.id);
        }
    }
    for loadout in loadouts.list() {
        let row = loadout.row();
        for name in row
            .worn
            .values()
            .chain(row.unassigned.iter())
            .chain(row.carry.iter().map(|carry| &carry.item))
        {
            if let Some(item) = selected.resolve_item_name(name) {
                push_unique_id(&mut ids, item.id);
            }
        }
    }
    ids
}

fn compile_quest_item(
    selected: &SelectedGameData,
    item: &QuestItemDocument,
) -> Result<CompiledQuestItem, CompileError> {
    let kind = match item.kind {
        super::path::QuestItemKindDocument::MustHave => CompiledItemKind::MustHave,
        super::path::QuestItemKindDocument::Acquirable => CompiledItemKind::Acquirable,
    };
    i32::try_from(item.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
    let (resolved, stackable) = resolve_bank_item_with_stackable(selected, &item.obj)?;
    Ok(CompiledQuestItem {
        id: resolved.id,
        name: resolved.name,
        qty: item.qty,
        kind,
        acquire: item.acquire.as_deref().map(Arc::from),
        stackable,
        from_stage: item.from_stage.clone(),
        from_stage_index: None,
    })
}

fn resolve_bank_item(selected: &SelectedGameData, name: &str) -> Result<BankItem, CompileError> {
    resolve_bank_item_with_stackable(selected, name).map(|(item, _)| item)
}

fn resolve_bank_item_with_stackable(
    selected: &SelectedGameData,
    name: &str,
) -> Result<(BankItem, bool), CompileError> {
    let item = selected
        .resolve_item_name(name)
        .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(name))?;
    let display = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-obj-name"))?;
    Ok((
        BankItem {
            id: item.id,
            name: Arc::from(display),
        },
        item.stackable,
    ))
}

fn compile_steps(
    steps: &[StepDocument],
    cx: &CompileContext<'_>,
    document: &PathDocument,
) -> Result<Vec<CompiledStep>, CompileError> {
    let mut out = Vec::new();
    let mut ids = std::collections::HashSet::new();
    for step in steps {
        if !ids.insert(step.id.0.clone()) {
            return Err(CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("duplicate-step"),
                detail: None,
                source: None,
            });
        }
        if step.skip_if.constant_truth() == Some(true) {
            return Err(CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("always-skipped"),
                detail: None,
                source: None,
            });
        }
        let handler = step_handlers()
            .find(|handler| handler.kind == step.kind && handler.version == step.version)
            .ok_or_else(|| CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("unknown-handler"),
                detail: None,
                source: None,
            })?;
        if handler.advance == AdvanceClass::Explicit && step.advances.is_none() {
            let mut error =
                CompileError::code("advances-undeclared").with_path(document.id.clone());
            error.step = Some(step.id.clone());
            return Err(error);
        }
        if step.advances != Some(true) {
            if let Some(fact) = direct_progress_fact(&step.settle) {
                let mut error = CompileError::code("settle-needs-advance")
                    .with_path(document.id.clone())
                    .with_detail(format!(
                        "settle contains direct progress fact `{fact}`; set advances: true"
                    ));
                error.step = Some(step.id.clone());
                return Err(error);
            }
        }
        let plan = (handler.compile)(&step.args, cx).map_err(|err| CompileError {
            path: document.id.clone(),
            role: None,
            step: Some(step.id.clone()),
            code: err.code,
            detail: err.detail,
            source: None,
        })?;
        let skip_if =
            families::compile_predicate(&step.skip_if, cx).map_err(|err| CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: err.code,
                detail: err.detail,
                source: None,
            })?;
        let settle = families::compile_predicate(&step.settle, cx).map_err(|err| CompileError {
            path: document.id.clone(),
            role: None,
            step: Some(step.id.clone()),
            code: err.code,
            detail: err.detail,
            source: None,
        })?;
        // A non-advancing step's settle is a live-frame goal the plan may poll
        // itself; an advancing settle waits on a progress read it cannot see.
        let plan = if step.advances == Some(true) {
            plan
        } else {
            plan.with_goal(StepGoal {
                skip_if: Arc::clone(&skip_if),
                settle: Arc::clone(&settle),
                summary: Arc::from(predicate_summary(&step.settle)),
            })
            .unwrap_or(plan)
        };
        let loadout: Option<Arc<str>> = if step.kind == "loadout" {
            step.args
                .get("loadout")
                .and_then(serde_json::Value::as_str)
                .map(|name| {
                    if name.contains('/') {
                        Arc::from(name)
                    } else {
                        Arc::from(format!("{}/{}", document.id.0, name))
                    }
                })
        } else {
            None
        };
        out.push(CompiledStep {
            id: step.id.clone(),
            kind: Arc::from(step.kind.as_str()),
            comment: step.comment.as_deref().map(Arc::from),
            loadout,
            tactic: (step.kind == "combat")
                .then(|| {
                    step.args
                        .get("tactic")?
                        .get("kind")?
                        .as_str()
                        .map(Arc::from)
                })
                .flatten(),
            advances: step.advances.unwrap_or(false),
            skip_if,
            skip_if_summary: Arc::from(predicate_summary(&step.skip_if)),
            settle,
            plan,
        });
    }
    Ok(out)
}
fn direct_progress_fact(predicate: &super::path::PredicateDocument) -> Option<&str> {
    match predicate {
        super::path::PredicateDocument::All(items) | super::path::PredicateDocument::Any(items) => {
            items.iter().find_map(direct_progress_fact)
        }
        super::path::PredicateDocument::Not(inner) => direct_progress_fact(inner),
        super::path::PredicateDocument::Fact { kind, version, .. } => predicate_handlers()
            .find(|handler| handler.kind == kind && handler.version == *version)
            .filter(|handler| handler.progress != ProgressRead::None)
            .map(|_| kind.as_str()),
    }
}

fn predicate_summary(predicate: &PredicateDocument) -> String {
    match predicate {
        PredicateDocument::All(items) | PredicateDocument::Any(items) => {
            let name = if matches!(predicate, PredicateDocument::All(_)) {
                "all"
            } else {
                "any"
            };
            format!(
                "{name}({})",
                items
                    .iter()
                    .map(predicate_summary)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
        PredicateDocument::Not(inner) => format!("not {}", predicate_summary(inner)),
        PredicateDocument::Fact { kind, args, .. } => format!(
            "{kind}({})",
            serde_json::to_string(args).unwrap_or_else(|_| "{}".to_owned())
        ),
    }
}

type RecipeInventoryPeak = (Arc<[CompiledRecipeItem]>, Arc<[i32]>);

fn recipe_inventory_peak(
    selected: &SelectedGameData,
    steps: &[StepDocument],
    output_ids: &[i32],
) -> Result<RecipeInventoryPeak, CompileError> {
    let mut current = Vec::<CompiledRecipeItem>::new();
    let mut peak = Vec::<CompiledRecipeItem>::new();
    let mut consumed = Vec::new();
    let mut peak_slots = 0usize;
    for step in steps {
        let used_item = if step.kind == "use_on" {
            step.args
                .get("item")
                .and_then(serde_json::Value::as_str)
                .and_then(|alias| {
                    selected
                        .item_by_alias(alias)
                        .map(|item| CompiledRecipeItem {
                            item: BankItem {
                                id: item.id,
                                name: Arc::from(item.name.as_deref().unwrap_or(alias)),
                            },
                            qty: 1,
                            stackable: item.stackable,
                        })
                })
        } else {
            None
        };
        if let Some(source) = used_item {
            let id = source.item.id;
            consumed.retain(|consumed_id| *consumed_id != id);
            if let Some(existing) = current.iter_mut().find(|item| item.item.id == id) {
                existing.qty = existing.qty.max(source.qty);
            } else {
                current.push(source);
            }
            current.sort_unstable_by_key(|item| item.item.id);
            record_recipe_peak(&current, &mut peak, &mut peak_slots);
        }

        let mut absent = Vec::new();
        collect_proven_absent_ids(selected, &step.settle, &mut absent);
        for id in absent {
            current.retain(|item| item.item.id != id);
            push_unique_id(&mut consumed, id);
        }

        let mut settled = Vec::new();
        collect_settled_items(selected, &step.settle, &mut settled)?;
        for item in settled {
            consumed.retain(|id| *id != item.item.id);
            if output_ids.contains(&item.item.id) {
                continue;
            }
            if let Some(existing) = current
                .iter_mut()
                .find(|existing| existing.item.id == item.item.id)
            {
                existing.qty = existing.qty.max(item.qty);
            } else {
                current.push(item);
            }
        }
        current.sort_unstable_by_key(|item| item.item.id);
        record_recipe_peak(&current, &mut peak, &mut peak_slots);
    }
    Ok((Arc::from(peak), Arc::from(consumed)))
}

fn record_recipe_peak(
    current: &[CompiledRecipeItem],
    peak: &mut Vec<CompiledRecipeItem>,
    peak_slots: &mut usize,
) {
    let slots = current.iter().fold(0usize, |slots, item| {
        let item_slots = if item.stackable {
            usize::from(item.qty > 0)
        } else {
            usize::try_from(item.qty.max(0)).unwrap_or(usize::MAX)
        };
        slots.saturating_add(item_slots)
    });
    if slots > *peak_slots {
        *peak_slots = slots;
        peak.clear();
        peak.extend_from_slice(current);
    }
}

fn collect_proven_absent_ids(
    selected: &SelectedGameData,
    predicate: &PredicateDocument,
    out: &mut Vec<i32>,
) {
    match predicate {
        PredicateDocument::All(items) => {
            for item in items {
                collect_proven_absent_ids(selected, item, out);
            }
        }
        PredicateDocument::Any(items) => {
            let Some((first, remaining)) = items.split_first() else {
                return;
            };
            let mut common = Vec::new();
            collect_proven_absent_ids(selected, first, &mut common);
            for item in remaining {
                let mut branch = Vec::new();
                collect_proven_absent_ids(selected, item, &mut branch);
                common.retain(|id| branch.contains(id));
            }
            for id in common {
                push_unique_id(out, id);
            }
        }
        PredicateDocument::Not(inner) => {
            let id = match inner.as_ref() {
                PredicateDocument::Fact { kind, args, .. } if kind == "has_item" => args
                    .get("obj")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|alias| selected.item_by_alias(alias))
                    .map(|item| item.id),
                _ => None,
            };
            if let Some(id) = id {
                push_unique_id(out, id);
            }
        }
        PredicateDocument::Fact { .. } => {}
    }
}

fn collect_settled_items(
    selected: &SelectedGameData,
    predicate: &PredicateDocument,
    out: &mut Vec<CompiledRecipeItem>,
) -> Result<(), CompileError> {
    match predicate {
        PredicateDocument::All(items) => {
            for item in items {
                collect_settled_items(selected, item, out)?;
            }
        }
        PredicateDocument::Fact { kind, args, .. } if kind == "has_item" => {
            let alias = args
                .get("obj")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| CompileError::code("invalid-args"))?;
            let item = selected
                .item_by_alias(alias)
                .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))?;
            let name = item
                .name
                .as_deref()
                .ok_or_else(|| CompileError::code("unresolved-obj-name"))?;
            if let Some(existing) = out.iter_mut().find(|entry| entry.item.id == item.id) {
                existing.qty = existing.qty.max(1);
            } else {
                out.push(CompiledRecipeItem {
                    item: BankItem {
                        id: item.id,
                        name: Arc::from(name),
                    },
                    qty: 1,
                    stackable: item.stackable,
                });
            }
        }
        PredicateDocument::Any(_) | PredicateDocument::Not(_) | PredicateDocument::Fact { .. } => {}
    }
    Ok(())
}
const MAX_RECIPE_NESTING_DEPTH: usize = 32;

enum RecipeBinding {
    Active { index: usize },
    Bound { depth: usize },
}

fn recipe_nesting_error(document: &PathDocument, name: &str, depth: usize) -> CompileError {
    let mut error = CompileError::code("recipe-nesting-limit").with_path(document.id.clone());
    error.detail = Some(Arc::from(format!(
        "Recipe {name} needs nesting depth {depth}. The limit is {MAX_RECIPE_NESTING_DEPTH}."
    )));
    error
}

fn compile_recipe<'a>(
    name: &'a str,
    document: &'a PathDocument,
    base: &CompileContext<'_>,
    recipes: &mut HashMap<String, Arc<[CompiledAcquireStep]>>,
    bindings: &mut HashMap<&'a str, RecipeBinding>,
    active: &mut Vec<&'a str>,
) -> Result<usize, CompileError> {
    match bindings.get(name) {
        Some(RecipeBinding::Bound { depth }) => return Ok(*depth),
        Some(RecipeBinding::Active { index }) => {
            let mut error = CompileError::code("recipe-cycle").with_path(document.id.clone());
            error.detail = Some(Arc::from(format!(
                "Acquisition recipe cycle: {} -> {name}",
                active[*index..].join(" -> ")
            )));
            return Err(error);
        }
        None => {}
    }
    let steps = document
        .quest
        .as_ref()
        .expect("recipe compilation follows header validation")
        .acquire
        .get(name)
        .ok_or_else(|| CompileError::code("unresolved-recipe").with_path(document.id.clone()))?;
    if active.len() >= MAX_RECIPE_NESTING_DEPTH {
        return Err(recipe_nesting_error(document, name, active.len() + 1));
    }
    bindings.insert(
        name,
        RecipeBinding::Active {
            index: active.len(),
        },
    );
    active.push(name);
    let mut depth = 1;
    for step in steps.iter().filter(|step| step.kind == "acquire") {
        let dependency = step
            .args
            .get("recipe")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                let mut error = CompileError::code("invalid-args").with_path(document.id.clone());
                error.step = Some(step.id.clone());
                error
            })?;
        let child_depth = compile_recipe(dependency, document, base, recipes, bindings, active)
            .map_err(|mut error| {
                if error.step.is_none() {
                    error.step = Some(step.id.clone());
                }
                error
            })?;
        depth = depth.max(child_depth + 1);
        if depth > MAX_RECIPE_NESTING_DEPTH {
            return Err(recipe_nesting_error(document, name, depth));
        }
    }
    let context = CompileContext { recipes, ..*base };
    let compiled: Arc<[CompiledAcquireStep]> = Arc::from(
        compile_steps(steps, &context, document)?
            .into_iter()
            .map(|step| CompiledAcquireStep {
                id: step.id,
                advances: step.advances,
                skip_if: step.skip_if,
                skip_if_summary: step.skip_if_summary,
                settle: step.settle,
                plan: step.plan,
            })
            .collect::<Vec<_>>(),
    );
    recipes.insert(name.to_owned(), compiled);
    bindings.insert(name, RecipeBinding::Bound { depth });
    active.pop();
    Ok(depth)
}

fn validate_header(
    header: &super::path::QuestHeaderDocument,
    selected: &SelectedGameData,
) -> Result<(), CompileError> {
    let obj = |alias: &str| {
        selected
            .item_by_alias(alias)
            .ok_or_else(|| CompileError::code("unresolved-obj").with_detail(alias))
    };
    for item in &header.items {
        obj(&item.obj)?;
        if item.qty == 0 {
            return Err(CompileError::code("invalid-quantity"));
        }
        if let Some(recipe) = &item.acquire {
            if !header.acquire.contains_key(recipe) {
                return Err(CompileError::code("unresolved-recipe"));
            }
        }
    }
    for tool in &header.tools {
        obj(tool
            .strip_prefix("obj:")
            .ok_or_else(|| CompileError::code("invalid-tool"))?)?;
    }
    match &header.bank {
        super::path::QuestBankDocument::Nearest(_) => {}
        super::path::QuestBankDocument::Tile { tile, source, .. } => {
            families::validate_tile(*tile, source)?;
        }
    }
    for area in header.areas.values() {
        if area.boxes.is_empty() {
            return Err(CompileError::code("invalid-area"));
        }
        for &[x1, z1, x2, z2, level] in &area.boxes {
            families::validate_tile([x1, z1, level], &area.source)?;
            families::validate_tile([x2, z2, level], &area.source)?;
            if x1 > x2 || z1 > z2 {
                return Err(CompileError::code("invalid-area"));
            }
        }
    }
    Ok(())
}

fn validate_nav_coverage(document: &PathDocument) -> Result<(), CompileError> {
    fn walk(value: &serde_json::Value) -> Result<(), CompileError> {
        match value {
            serde_json::Value::Object(map) => {
                if let Some(tile) = map.get("tile").and_then(serde_json::Value::as_array) {
                    if tile.len() == 3 {
                        let x = tile[0].as_i64().and_then(|x| i32::try_from(x).ok());
                        let z = tile[1].as_i64().and_then(|z| i32::try_from(z).ok());
                        if !x
                            .zip(z)
                            .is_some_and(|(x, z)| super::nav_coverage::covered_289(x, z))
                        {
                            return Err(CompileError::code("nav-tile-uncovered"));
                        }
                    }
                }
                for child in map.values() {
                    walk(child)?;
                }
            }
            serde_json::Value::Array(rows) => {
                for row in rows {
                    walk(row)?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    let header = document
        .quest
        .as_ref()
        .ok_or_else(|| CompileError::code("missing-quest-header"))?;
    if let super::path::QuestBankDocument::Tile { tile, .. } = &header.bank {
        if !super::nav_coverage::covered_289(tile[0], tile[1]) {
            return Err(CompileError::code("nav-tile-uncovered"));
        }
    }
    for area in header.areas.values() {
        for row in &area.boxes {
            if !super::nav_coverage::covered_289(row[0], row[1])
                || !super::nav_coverage::covered_289(row[2], row[3])
            {
                return Err(CompileError::code("nav-tile-uncovered"));
            }
        }
    }
    for step in header
        .acquire
        .values()
        .flatten()
        .chain(document.roles.iter().flat_map(|role| {
            role.prelude
                .iter()
                .chain(role.sequences.iter().flat_map(|seq| &seq.steps))
        }))
    {
        walk(&step.args)?;
    }
    Ok(())
}

pub const COOK_JSON: &str = include_str!("../../paths/289/cook.json");
pub const SHEEP_JSON: &str = include_str!("../../paths/289/sheep.json");
pub const RUNE_MYSTERIES_JSON: &str = include_str!("../../paths/289/runemysteries.json");
pub const ROMEO_AND_JULIET_JSON: &str = include_str!("../../paths/289/romeojuliet.json");
pub const IMP_JSON: &str = include_str!("../../paths/289/imp.json");
pub const VAMPIRE_JSON: &str = include_str!("../../paths/289/vampire.json");
pub const DORIC_JSON: &str = include_str!("../../paths/289/doric.json");
pub const GOBLIN_DIPLOMACY_JSON: &str = include_str!("../../paths/289/gobdip.json");
pub const HETTY_JSON: &str = include_str!("../../paths/289/hetty.json");
pub const PRINCE_JSON: &str = include_str!("../../paths/289/prince.json");
pub const HUNT_JSON: &str = include_str!("../../paths/289/hunt.json");
pub const DEMON_JSON: &str = include_str!("../../paths/289/demon.json");
pub const SQUIRE_JSON: &str = include_str!("../../paths/289/squire.json");
pub const DEATH_JSON: &str = include_str!("../../paths/289/death.json");
pub const DESERT_RESCUE_JSON: &str = include_str!("../../paths/289/desertrescue.json");
pub const PRIEST_PERIL_JSON: &str = include_str!("../../paths/289/priestperil.json");
pub const CLOCK_TOWER_JSON: &str = include_str!("../../paths/289/cog.json");
pub const MONKS_FRIEND_JSON: &str = include_str!("../../paths/289/drunkmonk.json");
pub const HAZEEL_CULT_JSON: &str = include_str!("../../paths/289/hazeelcult.json");
pub const PLAGUE_CITY_JSON: &str = include_str!("../../paths/289/elena.json");
pub const DRUID_JSON: &str = include_str!("../../paths/289/druid.json");
pub const FLUFFS_JSON: &str = include_str!("../../paths/289/fluffs.json");
pub const JUNGLE_POTION_JSON: &str = include_str!("../../paths/289/junglepotion.json");
pub const SEA_SLUG_JSON: &str = include_str!("../../paths/289/seaslug.json");
pub const TRIBAL_TOTEM_JSON: &str = include_str!("../../paths/289/totem.json");
pub const INDEX_JSON: &str = include_str!("../../paths/289/index.json");

pub fn path_bytes(id: &str) -> Option<&'static [u8]> {
    match id {
        "cook" => Some(COOK_JSON.as_bytes()),
        "sheep" => Some(SHEEP_JSON.as_bytes()),
        "runemysteries" => Some(RUNE_MYSTERIES_JSON.as_bytes()),
        "romeojuliet" => Some(ROMEO_AND_JULIET_JSON.as_bytes()),
        "imp" => Some(IMP_JSON.as_bytes()),
        "vampire" => Some(VAMPIRE_JSON.as_bytes()),
        "doric" => Some(DORIC_JSON.as_bytes()),
        "gobdip" => Some(GOBLIN_DIPLOMACY_JSON.as_bytes()),
        "hetty" => Some(HETTY_JSON.as_bytes()),
        "prince" => Some(PRINCE_JSON.as_bytes()),
        "hunt" => Some(HUNT_JSON.as_bytes()),
        "demon" => Some(DEMON_JSON.as_bytes()),
        "squire" => Some(SQUIRE_JSON.as_bytes()),
        "death" => Some(DEATH_JSON.as_bytes()),
        "desertrescue" => Some(DESERT_RESCUE_JSON.as_bytes()),
        "priestperil" => Some(PRIEST_PERIL_JSON.as_bytes()),
        "cog" => Some(CLOCK_TOWER_JSON.as_bytes()),
        "drunkmonk" => Some(MONKS_FRIEND_JSON.as_bytes()),
        "hazeelcult" => Some(HAZEEL_CULT_JSON.as_bytes()),
        "elena" => Some(PLAGUE_CITY_JSON.as_bytes()),
        "druid" => Some(DRUID_JSON.as_bytes()),
        "fluffs" => Some(FLUFFS_JSON.as_bytes()),
        "junglepotion" => Some(JUNGLE_POTION_JSON.as_bytes()),
        "seaslug" => Some(SEA_SLUG_JSON.as_bytes()),
        "totem" => Some(TRIBAL_TOTEM_JSON.as_bytes()),
        _ => None,
    }
}

pub fn cook_bytes() -> &'static [u8] {
    path_bytes("cook").unwrap()
}

pub fn decode_cook() -> Result<PathDocument, CompileError> {
    serde_json::from_str(COOK_JSON).map_err(|_| CompileError::code("invalid-json"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quester::path::PredicateDocument;
    use api::selected::ClientRevision;

    fn selected() -> Arc<SelectedGameData> {
        api::game_data::for_revision(ClientRevision::R289).unwrap()
    }

    fn quests(data: &SelectedGameData) -> QuestCatalog {
        QuestCatalog::from_identity(data.quest_identity()).unwrap_or_else(|_| QuestCatalog::empty())
    }

    #[test]
    fn compile_cache_reuses_identical_bytes_and_separates_changed_paths() {
        let _home = crate::IsolatedEnv::enter("quester-compile-cache");
        FamilyPreparation::run(move |worker| {
            let data = selected();
            let quests = quests(&data);
            let first = compile_path(cook_bytes(), &data, &quests, worker).unwrap();
            let hit = compile_path(cook_bytes(), &data, &quests, worker).unwrap();
            assert!(Arc::ptr_eq(&first, &hit));
            let mut changed: serde_json::Value = serde_json::from_slice(cook_bytes()).unwrap();
            changed["id"] = serde_json::json!("cook-cache-different");
            let bytes = serde_json::to_vec(&changed).unwrap();
            let miss = compile_path(&bytes, &data, &quests, worker).unwrap();
            assert!(!Arc::ptr_eq(&first, &miss));
            assert_eq!(first.id.0.as_ref(), "cook");
            assert_eq!(miss.id.0.as_ref(), "cook-cache-different");
            assert_ne!(first.digest, miss.digest);
        })
        .unwrap()
        .join()
        .unwrap();
    }

    #[test]
    fn gather_progress_reader_prepares_catalog_through_production_compiler() {
        let _home = crate::IsolatedEnv::enter("quester-gather-progress-reader");
        prepare_for_test(move |worker| {
            let data = selected();
            let quests = quests(&data);
            let mut document: serde_json::Value = serde_json::from_slice(cook_bytes()).unwrap();
            let mut reader = document["roles"][0]["sequences"][0]["steps"][0].clone();
            reader["id"] = serde_json::json!("gather-progress-reader");
            reader["kind"] = serde_json::json!("gather");
            reader["advances"] = serde_json::json!(false);
            reader["args"] = serde_json::json!({
                "skill": "mining",
                "resource": "copper",
                "until": {"obj": "copper_ore", "qty": 1}
            });
            reader["settle"] = serde_json::json!({
                "Fact": {
                    "kind": "item_count_at_least",
                    "version": 1,
                    "args": {"obj": "copper_ore", "qty": 1}
                }
            });
            document["roles"][0]["progress_reader"] = reader;
            let bytes = serde_json::to_vec(&document).unwrap();
            let compiled = compile_path(&bytes, &data, &quests, worker).unwrap();
            assert_eq!(
                compiled.progress_reader.as_ref().unwrap().id.0.as_ref(),
                "gather-progress-reader"
            );
        });
    }

    #[test]
    fn gather_preparation_covers_preludes_other_roles_and_acquisition_recipes() {
        let mut document = decode_cook().unwrap();
        assert!(!uses_gathering(&document));
        let mut gather = document.roles[0].sequences[0].steps[0].clone();
        gather.kind = "gather".into();

        document.roles[0].prelude.push(gather.clone());
        assert!(uses_gathering(&document));
        document.roles[0].prelude.pop();

        let mut other = document.roles[0].clone();
        other.sequences[0].steps[0] = gather.clone();
        document.roles.push(other);
        assert!(uses_gathering(&document));
        document.roles.pop();

        document
            .quest
            .as_mut()
            .unwrap()
            .acquire
            .insert("copper".into(), vec![gather]);
        assert!(uses_gathering(&document));
        document.quest.as_mut().unwrap().acquire.remove("copper");
        assert!(!uses_gathering(&document));
    }

    #[test]
    fn protected_items_union_path_gather_tools_and_every_loadout_field() {
        let data = selected();
        let gather_ids: std::collections::BTreeSet<_> = api::gather_tools::AXES
            .iter()
            .chain(api::gather_tools::PICKAXES)
            .filter_map(|tool| data.item_by_alias(tool.alias).map(|item| item.id))
            .collect();
        let mut loadout_items = Vec::new();
        let mut seen = gather_ids.clone();
        for item in data.items() {
            let Some(name) = item.name.as_deref() else {
                continue;
            };
            if data.item_by_alias(name).is_some() {
                continue;
            }
            let Some(resolved) = data.resolve_item_name(name) else {
                continue;
            };
            if seen.insert(resolved.id) {
                loadout_items.push((resolved.id, resolved.name.as_deref().unwrap().to_string()));
                if loadout_items.len() == 3 {
                    break;
                }
            }
        }
        assert_eq!(loadout_items.len(), 3);
        let row = crate::loadouts_store::Loadout::new("operator/protected")
            .with_slot("hat", loadout_items[0].1.clone())
            .with_slot("unassigned", loadout_items[1].1.clone())
            .with_carry(loadout_items[2].1.clone(), 1);
        let loadouts = super::super::loadouts::LoadoutOverlay::new(Arc::from([row]), Arc::from([]));
        let path_tool = BankItem {
            id: i32::MAX,
            name: Arc::from("Path tool"),
        };
        let ids = protected_item_ids(&data, &[path_tool], &loadouts);
        let expected: std::collections::BTreeSet<_> = gather_ids
            .into_iter()
            .chain([i32::MAX])
            .chain(loadout_items.into_iter().map(|(id, _)| id))
            .collect();
        assert_eq!(
            ids.into_iter().collect::<std::collections::BTreeSet<_>>(),
            expected
        );
    }

    #[test]
    fn path_bank_required_is_preserved_in_compiled_provisioning() {
        let mut document = decode_cook().unwrap();
        let data = selected();
        let quests = quests(&data);

        let optional = compile_uncached_for_test(&document, &data, &quests).unwrap();
        assert!(!optional.provisioning.bank_required);
        let super::super::path::QuestBankDocument::Tile { required, .. } =
            &mut document.quest.as_mut().unwrap().bank
        else {
            panic!("Cook Path has an authored bank tile");
        };
        *required = true;
        let required = compile_uncached_for_test(&document, &data, &quests).unwrap();
        assert!(required.provisioning.bank_required);
    }

    #[test]
    fn always_skipped_all_empty_is_rejected() {
        let mut document = decode_cook().unwrap();
        document.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::All(vec![]);
        let data = selected();
        let quests = quests(&data);
        let err = match compile_uncached_for_test(&document, &data, &quests) {
            Err(err) => err,
            Ok(_) => panic!("always-skipped must fail compile"),
        };
        assert_eq!(err.code.as_ref(), "always-skipped");
    }

    #[test]
    fn unknown_from_stage_is_rejected() {
        let err = compile_err(|document| {
            document.quest.as_mut().unwrap().items[0].from_stage = Some(FactKey::new("cook:99"));
        });
        assert_eq!(err.code.as_ref(), "unknown-from-stage");
    }

    #[test]
    fn from_stage_resolves_to_the_compiled_sequence_index() {
        let mut document = decode_cook().unwrap();
        document.quest.as_mut().unwrap().items[0].from_stage = Some(FactKey::new("cook:1"));
        let data = selected();
        let quests = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let gated = &compiled.provisioning.items[0];
        assert_eq!(gated.from_stage_index, Some(1));
        assert_eq!(
            compiled
                .provisioning
                .stages
                .iter()
                .map(|stage| stage.0.as_ref())
                .collect::<Vec<_>>(),
            vec!["cook:0", "cook:1", "cook:2"]
        );
    }

    fn compile_err(mut edit: impl FnMut(&mut PathDocument)) -> CompileError {
        let mut document = decode_cook().unwrap();
        edit(&mut document);
        let data = selected();
        let quests = quests(&data);
        match compile_uncached_for_test(&document, &data, &quests) {
            Err(err) => err,
            Ok(_) => panic!("compile must fail"),
        }
    }

    fn acquire_recipe_step(id: &str, recipe: &str) -> StepDocument {
        StepDocument {
            id: FactKey::new(id),
            kind: "acquire".into(),
            version: 1,
            args: serde_json::json!({"recipe": recipe}),
            comment: None,
            advances: Some(false),
            skip_if: PredicateDocument::Any(vec![]),
            settle: PredicateDocument::All(vec![]),
        }
    }

    #[test]
    fn recipe_peak_retains_use_on_source_without_explicit_absence() {
        let document = decode_cook().unwrap();
        let data = selected();
        let quests = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let grain_id = data.item_by_alias("grain").unwrap().id;
        let flour = &compiled.provisioning.recipes["acquire:flour"];
        assert!(flour.peak_items.iter().any(|item| item.item.id == grain_id));
        assert!(!flour.consumed_ids.contains(&grain_id));

        let absent: PredicateDocument = serde_json::from_value(serde_json::json!({
            "Not": {
                "Fact": {
                    "kind": "has_item",
                    "version": 1,
                    "args": { "obj": "grain" }
                }
            }
        }))
        .unwrap();
        let mut absent_ids = Vec::new();
        collect_proven_absent_ids(&data, &absent, &mut absent_ids);
        assert_eq!(absent_ids, vec![grain_id]);
        let uncertain = PredicateDocument::Any(vec![
            absent,
            PredicateDocument::Fact {
                kind: "message".into(),
                version: 1,
                args: serde_json::json!({"any": ["grain moved"]}),
            },
        ]);
        let mut uncertain_ids = Vec::new();
        collect_proven_absent_ids(&data, &uncertain, &mut uncertain_ids);
        assert!(uncertain_ids.is_empty());
    }

    #[test]
    fn acquire_recipe_forward_and_shared_dependencies_resolve() {
        let _home = crate::IsolatedEnv::enter("quester-recipe-forward");
        let mut document = decode_cook().unwrap();
        let header = document.quest.as_mut().unwrap();
        let mut leaf = header.acquire["acquire:egg"][0].clone();
        leaf.id = FactKey::new("recipe-leaf");
        header.acquire.insert("acquire:z-leaf".into(), vec![leaf]);
        header.acquire.insert(
            "acquire:a-root".into(),
            vec![
                acquire_recipe_step("nested-first", "acquire:z-leaf"),
                acquire_recipe_step("nested-second", "acquire:z-leaf"),
            ],
        );
        let data = selected();
        let quests = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quests)
            .unwrap_or_else(|error| panic!("nested recipe: {} {:?}", error.code, error.detail));
        assert_eq!(
            compiled.provisioning.recipes["acquire:a-root"].steps.len(),
            2
        );
        assert_eq!(
            compiled.provisioning.recipes["acquire:z-leaf"].steps.len(),
            1
        );
        assert_eq!(
            compiled.provisioning.recipes["acquire:a-root"].steps[0]
                .id
                .0
                .as_ref(),
            "nested-first"
        );
    }

    #[test]
    fn acquire_recipe_cycle_names_the_cycle() {
        let _home = crate::IsolatedEnv::enter("quester-recipe-cycle");
        let error = compile_err(|document| {
            let recipes = &mut document.quest.as_mut().unwrap().acquire;
            recipes.insert(
                "acquire:cycle-a".into(),
                vec![acquire_recipe_step("cycle-a", "acquire:cycle-b")],
            );
            recipes.insert(
                "acquire:cycle-b".into(),
                vec![acquire_recipe_step("cycle-b", "acquire:cycle-a")],
            );
        });
        assert_eq!(error.code.as_ref(), "recipe-cycle");
        assert!(error
            .detail
            .as_deref()
            .unwrap()
            .contains("acquire:cycle-a -> acquire:cycle-b -> acquire:cycle-a"));
    }

    #[test]
    fn acquire_recipe_missing_dependency_stays_unresolved_recipe() {
        let _home = crate::IsolatedEnv::enter("quester-recipe-missing");
        let error = compile_err(|document| {
            document.quest.as_mut().unwrap().acquire.insert(
                "acquire:root".into(),
                vec![acquire_recipe_step("missing-child", "acquire:absent")],
            );
        });
        assert_eq!(error.code.as_ref(), "unresolved-recipe");
    }

    #[test]
    fn acquire_recipe_depth_limit_is_order_independent() {
        let _home = crate::IsolatedEnv::enter("quester-recipe-depth");
        let data = selected();
        let quests = quests(&data);
        for leaf_first in [true, false] {
            let name = |depth| {
                let index = if leaf_first { depth } else { 32 - depth };
                format!("acquire:chain-{index:02}")
            };
            let mut document = decode_cook().unwrap();
            let header = document.quest.as_mut().unwrap();
            let mut leaf = header.acquire["acquire:egg"][0].clone();
            leaf.id = FactKey::new("depth-leaf");
            header.acquire.insert(name(0), vec![leaf]);
            // Leaf-first order tests memoized heights. Root-first order
            // tests the traversal stack bound before compilation.
            for depth in 1..32 {
                header.acquire.insert(
                    name(depth),
                    vec![acquire_recipe_step(
                        &format!("depth-{depth}"),
                        &name(depth - 1),
                    )],
                );
            }
            assert!(compile_uncached_for_test(&document, &data, &quests).is_ok());
            document
                .quest
                .as_mut()
                .unwrap()
                .acquire
                .insert(name(32), vec![acquire_recipe_step("depth-32", &name(31))]);
            let error = match compile_uncached_for_test(&document, &data, &quests) {
                Err(error) => error,
                Ok(_) => panic!("a 33-recipe chain must fail"),
            };
            assert_eq!(error.code.as_ref(), "recipe-nesting-limit");
            assert!(error.detail.as_deref().unwrap().contains("32"));
        }
    }

    #[test]
    fn schema_three_accepts_schema_reference_and_rejects_schema_two() {
        let _home = crate::IsolatedEnv::enter("quester-schema-three");
        let document = decode_cook().unwrap();
        assert_eq!(document.schema, super::super::path::PATH_SCHEMA);
        assert_eq!(document.schema_url.as_deref(), Some("../path.schema.json"));

        let err = compile_err(|document| document.schema = 2);
        assert_eq!(err.code.as_ref(), "unsupported-schema");
    }

    #[test]
    fn explicit_steps_require_advances_and_progress_settles_require_true() {
        let _home = crate::IsolatedEnv::enter("quester-schema-explicit-advances");
        let missing = compile_err(|document| {
            let step = document
                .roles
                .iter_mut()
                .flat_map(|role| &mut role.sequences)
                .flat_map(|sequence| &mut sequence.steps)
                .find(|step| step.kind == "talk")
                .expect("Cook has an explicit talk step");
            step.advances = None;
        });
        assert_eq!(missing.code.as_ref(), "advances-undeclared");

        let contradiction = compile_err(|document| {
            let step = document
                .roles
                .iter_mut()
                .flat_map(|role| &mut role.sequences)
                .flat_map(|sequence| &mut sequence.steps)
                .find(|step| step.kind == "talk")
                .expect("Cook has an explicit talk step");
            step.advances = Some(false);
            step.settle = PredicateDocument::All(vec![PredicateDocument::Not(Box::new(
                PredicateDocument::Fact {
                    kind: "quest_colour".into(),
                    version: 1,
                    args: serde_json::json!({}),
                },
            ))]);
        });
        assert_eq!(contradiction.code.as_ref(), "settle-needs-advance");
    }

    #[test]
    fn default_steps_allow_omitted_advances_and_dynamic_quantity_settles() {
        let _home = crate::IsolatedEnv::enter("quester-schema-default-advances");
        let mut document = decode_cook().unwrap();
        let walk = StepDocument {
            id: FactKey::new("schema-default-walk"),
            kind: "walk".into(),
            version: 1,
            args: serde_json::json!({
                "tile": [3209, 3215, 0],
                "source": "PATH-SCHEMA-1 test"
            }),
            comment: None,
            advances: None,
            skip_if: PredicateDocument::Any(vec![]),
            settle: PredicateDocument::Any(vec![]),
        };
        document.roles[0].prelude = vec![walk];
        let data = selected();
        let quest_catalog = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quest_catalog).unwrap();
        assert!(!compiled.prelude[0].advances);

        document.roles[0].prelude[0].advances = Some(true);
        let compiled = compile_uncached_for_test(&document, &data, &quest_catalog).unwrap();
        assert!(compiled.prelude[0].advances);

        let sheep: PathDocument = serde_json::from_str(SHEEP_JSON).unwrap();
        let compiled = compile_uncached_for_test(&sheep, &data, &quest_catalog).unwrap();
        for id in ["shear", "spin"] {
            let step = compiled
                .sequences
                .iter()
                .flat_map(|sequence| &sequence.steps)
                .find(|step| step.id.0.as_ref() == id)
                .expect("compiled Sheep step");
            assert!(!step.advances);
        }
    }

    #[test]
    fn path_walk_crossing_and_protection_compile_independently() {
        let data = selected();
        let quests = quests(&data);
        for args in [
            serde_json::json!({
                "tile": [3224, 3200, 0],
                "source": "regression test",
                "radius": 1,
                "cross": ["death-plateau-throwers"],
            }),
            serde_json::json!({
                "tile": [3224, 3200, 0],
                "source": "regression test",
                "radius": 1,
                "guard": "protect",
            }),
        ] {
            let mut document = decode_cook().unwrap();
            let step = &mut document.roles[0].sequences[0].steps[0];
            step.kind = "walk".into();
            step.args = args;
            step.skip_if = PredicateDocument::Fact {
                kind: "near".into(),
                version: 1,
                args: serde_json::json!({ "tile": [3224, 3200, 0], "radius": 1 }),
            };
            step.settle = PredicateDocument::Fact {
                kind: "near".into(),
                version: 1,
                args: serde_json::json!({ "tile": [3224, 3200, 0], "radius": 1 }),
            };
            compile_uncached_for_test(&document, &data, &quests)
                .expect("a named crossing or protection does not require the other");
        }
    }

    #[test]
    fn path_walk_cross_with_protect_guard_compiles() {
        let mut document = decode_cook().unwrap();
        let step = &mut document.roles[0].sequences[0].steps[0];
        step.kind = "walk".into();
        step.args = serde_json::json!({
            "tile": [3224, 3200, 0],
            "source": "regression test",
            "radius": 1,
            "cross": ["death-plateau-throwers"],
            "guard": "protect",
        });
        step.skip_if = PredicateDocument::Fact {
            kind: "near".into(),
            version: 1,
            args: serde_json::json!({ "tile": [3224, 3200, 0], "radius": 1 }),
        };
        step.settle = PredicateDocument::Fact {
            kind: "near".into(),
            version: 1,
            args: serde_json::json!({ "tile": [3224, 3200, 0], "radius": 1 }),
        };
        let data = selected();
        let quests = quests(&data);
        compile_uncached_for_test(&document, &data, &quests)
            .expect("guard protect admits a named zone crossing");
    }

    #[test]
    fn unknown_handler_is_rejected() {
        let err = compile_err(|document| {
            document.roles[0].sequences[0].steps[0].kind = "no_such_family".into();
        });
        assert_eq!(err.code.as_ref(), "unknown-handler");
    }

    #[test]
    fn unresolved_npc_obj_loc_and_area_are_rejected() {
        let _home = crate::IsolatedEnv::enter("quester-unresolved-alias-details");
        let npc = compile_err(|document| {
            document.roles[0].sequences[0].steps[0].args["npc"] = serde_json::json!("no_such_npc");
        });
        assert_eq!(npc.code.as_ref(), "unresolved-npc");
        assert_eq!(npc.detail.as_deref(), Some("no_such_npc"));
        assert_eq!(npc.step, Some(FactKey::new("start")));

        let obj = compile_err(|document| {
            document.roles[0].sequences[1].steps[0].skip_if = PredicateDocument::Fact {
                kind: "has_item".into(),
                version: 1,
                args: serde_json::json!({"obj": "no_such_obj"}),
            };
        });
        assert_eq!(obj.code.as_ref(), "unresolved-obj");
        assert_eq!(obj.detail.as_deref(), Some("no_such_obj"));
        assert!(obj.step.is_some());

        let loc = compile_err(|document| {
            document
                .quest
                .as_mut()
                .unwrap()
                .acquire
                .get_mut("acquire:flour")
                .unwrap()[3]
                .args["target"]["loc"] = serde_json::json!("no_such_loc");
        });
        assert_eq!(loc.code.as_ref(), "unresolved-loc");
        assert_eq!(loc.detail.as_deref(), Some("no_such_loc"));
        assert!(loc.step.is_some());
        assert_eq!(
            compile_err(|document| {
                document.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::Fact {
                    kind: "in_area".into(),
                    version: 1,
                    args: serde_json::json!({"area": "no_such_area"}),
                };
            })
            .code
            .as_ref(),
            "unresolved-area"
        );
    }

    #[test]
    fn empty_nonterminal_and_duplicate_step_are_rejected() {
        assert_eq!(
            compile_err(|document| {
                document.roles[0].sequences[0].steps.clear();
            })
            .code
            .as_ref(),
            "empty-nonterminal"
        );
        assert_eq!(
            compile_err(|document| {
                let step = document.roles[0].sequences[0].steps[0].clone();
                document.roles[0].sequences[1].steps.push(step);
            })
            .code
            .as_ref(),
            "duplicate-step"
        );
    }

    #[test]
    fn prelude_over_four_steps_warns() {
        let mut document = decode_cook().unwrap();
        let step = document.roles[0].sequences[0].steps[0].clone();
        document.roles[0].prelude = (0..5)
            .map(|i| {
                let mut row = step.clone();
                row.id = api::selected::FactKey::new(&format!("pre-{i}"));
                row.skip_if = PredicateDocument::Any(vec![]);
                row
            })
            .collect();
        let data = selected();
        let quests = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quests).unwrap();
        assert!(compiled
            .warnings
            .iter()
            .any(|warning| warning.as_ref() == "prelude-size"));
    }

    #[test]
    fn invalid_header_references_and_ignored_step_args_are_rejected() {
        for edit in [
            |d: &mut PathDocument| d.quest.as_mut().unwrap().items[0].obj = "bucket_milkk".into(),
            |d: &mut PathDocument| {
                d.quest
                    .as_mut()
                    .unwrap()
                    .tools
                    .push("obj:not_an_item".into())
            },
            |d: &mut PathDocument| {
                d.roles[0].sequences[0].steps[0].args["anchor"]["source"] = serde_json::json!("")
            },
            |d: &mut PathDocument| {
                d.quest
                    .as_mut()
                    .unwrap()
                    .acquire
                    .get_mut("acquire:egg")
                    .unwrap()[0]
                    .args["wait_if_mising"] = serde_json::json!(true)
            },
            |d: &mut PathDocument| {
                d.quest
                    .as_mut()
                    .unwrap()
                    .acquire
                    .get_mut("acquire:flour")
                    .unwrap()[4]
                    .args["target"]["name"] = serde_json::json!("Not a real display name")
            },
        ] {
            compile_err(edit);
        }
        for schema in [1, 2] {
            assert_eq!(
                compile_err(|d| d.schema = schema).code.as_ref(),
                "unsupported-schema"
            );
        }
    }

    #[test]
    fn predicate_argument_typos_do_not_silently_compile() {
        for (kind, mut args) in [
            ("npc_present", serde_json::json!({"npc":"cook"})),
            ("loc_present", serde_json::json!({"loc":"wheat"})),
            ("ground_item_near", serde_json::json!({"obj":"egg"})),
            ("item_count_at_least", serde_json::json!({"obj":"egg"})),
            ("on_level", serde_json::json!({"level":0})),
            ("in_area", serde_json::json!({"area":"area:dairy"})),
            (
                "skill_at_least",
                serde_json::json!({"skill":"Cooking","level":1}),
            ),
            ("hp_fraction_below", serde_json::json!({"below":0.5})),
            ("in_combat", serde_json::json!({})),
            ("modal_open", serde_json::json!({})),
        ] {
            args["mistyped"] = serde_json::json!(true);
            let error = compile_err(|document| {
                document.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::Fact {
                    kind: kind.into(),
                    version: 1,
                    args: args.clone(),
                };
            });
            assert_eq!(error.code.as_ref(), "invalid-args", "{kind}");
        }
    }

    #[test]
    fn progress_program_compiles_and_validates_authored_rules() {
        let mut document = decode_cook().unwrap();
        let progress = document.roles[0].progress.as_mut().unwrap();
        progress
            .rules
            .push(super::super::path::ProgressRuleDocument {
                stage: FactKey::new("cook:1"),
                all: vec!["journal".into()],
                any: vec![],
                not: vec!["blocked".into()],
                varp: Some(4),
            });
        progress
            .flags
            .push(super::super::path::ProgressFlagDocument {
                flag: FactKey::new("feather"),
                all: vec![],
                any: vec!["feather".into()],
                count: None,
            });
        progress
            .flags
            .push(super::super::path::ProgressFlagDocument {
                flag: FactKey::new("crystals"),
                all: vec![],
                any: vec!["crystals".into()],
                count: Some(r"(\d+)".into()),
            });
        let data = selected();
        let quests = quests(&data);
        let compiled = compile_uncached_for_test(&document, &data, &quests).unwrap();
        assert_eq!(compiled.progress.rules.len(), 1);
        assert_eq!(compiled.progress.flags.len(), 2);
        assert_eq!(
            compiled.progress.varp_hint(&FactKey::new("cook:1")),
            Some(4)
        );
        assert_eq!(compiled.progress.rules[0].all[0].as_ref(), "journal");
    }

    #[test]
    fn progress_program_rejects_bad_needles_counts_and_bindings() {
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress.as_mut().unwrap().rules.push(
                    super::super::path::ProgressRuleDocument {
                        stage: FactKey::new("cook:1"),
                        all: vec!["Not lower".into()],
                        any: vec![],
                        not: vec![],
                        varp: None,
                    },
                )
            })
            .code
            .as_ref(),
            "invalid-progress-needle"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress.as_mut().unwrap().flags.push(
                    super::super::path::ProgressFlagDocument {
                        flag: FactKey::new("crystals"),
                        all: vec![],
                        any: vec!["crystals".into()],
                        count: Some(".*".into()),
                    },
                )
            })
            .code
            .as_ref(),
            "invalid-progress-count"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress_binding = FactKey::new("journal:other");
            })
            .code
            .as_ref(),
            "progress-binding"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress.as_mut().unwrap().rules.push(
                    super::super::path::ProgressRuleDocument {
                        stage: FactKey::new("cook:99"),
                        all: vec!["journal".into()],
                        any: vec![],
                        not: vec![],
                        varp: None,
                    },
                )
            })
            .code
            .as_ref(),
            "unbound-progress-stage"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress.as_mut().unwrap().rules.push(
                    super::super::path::ProgressRuleDocument {
                        stage: FactKey::new("cook:1"),
                        all: vec![],
                        any: vec![],
                        not: vec!["blocked".into()],
                        varp: None,
                    },
                )
            })
            .code
            .as_ref(),
            "invalid-progress-rule"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].progress.as_mut().unwrap().flags.push(
                    super::super::path::ProgressFlagDocument {
                        flag: FactKey::new("empty-flag"),
                        all: vec![],
                        any: vec![],
                        count: None,
                    },
                )
            })
            .code
            .as_ref(),
            "invalid-progress-flag"
        );
    }
    #[test]
    fn progress_predicate_errors_keep_path_context() {
        let error = compile_err(|document| {
            document.roles[0].sequences[0].steps[0].skip_if = PredicateDocument::Fact {
                kind: "stage_in".into(),
                version: 1,
                args: serde_json::json!({"quest":"cook","any":["cook:99"]}),
            };
        });
        assert_eq!(error.code.as_ref(), "unresolved-progress-stage");
        assert_eq!(error.path, FactKey::new("cook"));
        assert!(error.step.is_some());
    }

    #[test]
    fn alias_loadout_compiles_and_begins_application() {
        let _home = crate::IsolatedEnv::enter("quester-alias-loadout");
        let data = selected();
        let quests = quests(&data);
        let mut document: serde_json::Value = serde_json::from_str(COOK_JSON).unwrap();
        document["quest"]["loadouts"] = serde_json::json!({
            "probe": {
                "worn": { "righthand": "rune_scimitar" },
                "carry": [{ "item": "4doseprayerrestore", "qty": 2 }]
            }
        });
        document["roles"][0]["prelude"] = serde_json::json!([{
            "id": "apply-alias-kit", "kind": "loadout", "version": 1,
            "args": { "loadout": "probe", "at": "nearest" },
            "skip_if": { "Any": [] }, "settle": { "All": [] }
        }]);
        let document: PathDocument = serde_json::from_value(document).unwrap();
        let path = compile_uncached_for_test(&document, &data, &quests).unwrap();
        let mut snapshot = api::snapshot::GameSnapshot::new();
        snapshot.seed_ingame(2);
        snapshot.seed_inventory(vec![], 28);
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let mut ledger = None;
        families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            let required_after = tick.cx.evidence();
            let mut context = StepContext {
                tick,
                quests: &quests,
                progress: &[],
                required_after,
                banks: &banks,
                choices: &super::super::choices::QuestChoices::default(),
            };
            if let Err(error) = path.prelude[0].plan.begin(&mut context) {
                panic!("valid alias loadout could not begin application: {error:?}");
            }
        });
    }

    #[test]
    fn certificate_worn_header_loadout_is_rejected() {
        let _home = crate::IsolatedEnv::enter("quester-cert-worn-header");
        let data = selected();
        let certificate = data.item_by_alias("cert_rune_scimitar").unwrap();
        assert!(certificate.is_certificate());
        assert_eq!(
            certificate.name.as_deref(),
            data.item_by_alias("rune_scimitar").unwrap().name.as_deref()
        );
        let error = compile_err(|document| {
            document.quest.as_mut().unwrap().loadouts = serde_json::from_value(serde_json::json!({
                "probe": {
                    "worn": { "righthand": "cert_rune_scimitar" },
                    "carry": []
                }
            }))
            .unwrap();
        });
        assert_eq!(error.code.as_ref(), "certificate-loadout-item");
        assert_eq!(error.path, FactKey::new("cook"));
    }

    #[test]
    fn certificate_carry_header_loadout_is_rejected() {
        let _home = crate::IsolatedEnv::enter("quester-cert-carry-header");
        let data = selected();
        let certificate = data.item_by_alias("cert_lobster").unwrap();
        assert!(certificate.is_certificate());
        assert_eq!(
            certificate.name.as_deref(),
            data.item_by_alias("lobster").unwrap().name.as_deref()
        );
        let error = compile_err(|document| {
            document.quest.as_mut().unwrap().loadouts = serde_json::from_value(serde_json::json!({
                "probe": {
                    "worn": {},
                    "carry": [{ "item": "cert_lobster", "qty": 6 }]
                }
            }))
            .unwrap();
        });
        assert_eq!(error.code.as_ref(), "certificate-loadout-item");
        assert_eq!(error.path, FactKey::new("cook"));
    }

    #[test]
    fn all_released_s2_paths_compile() {
        let data = selected();
        let quests = quests(&data);
        for (id, json) in [
            ("cook", COOK_JSON),
            ("sheep", SHEEP_JSON),
            ("runemysteries", RUNE_MYSTERIES_JSON),
            ("romeojuliet", ROMEO_AND_JULIET_JSON),
        ] {
            let document: PathDocument =
                serde_json::from_str(json).unwrap_or_else(|error| panic!("{id}: {error}"));
            compile_uncached_for_test(&document, &data, &quests)
                .unwrap_or_else(|error| panic!("{id} {:?}: {}", error.step, error.code));
        }
    }

    #[test]
    fn cook_loaded_hopper_reapproaches_the_upstairs_controls() {
        let document = decode_cook().unwrap();
        let step = document
            .quest
            .as_ref()
            .unwrap()
            .acquire
            .values()
            .flatten()
            .chain(
                document.roles[0]
                    .sequences
                    .iter()
                    .flat_map(|sequence| &sequence.steps),
            )
            .find(|step| step.id.0.as_ref() == "operate-controls")
            .expect("operate-controls");
        assert_eq!(
            step.args["anchor"]["tile"],
            serde_json::json!([3166, 3305, 2])
        );
        assert!(step.args["anchor"]["source"].as_str().is_some());
    }

    #[test]
    fn uncovered_authored_tile_is_rejected() {
        let mut document = decode_cook().unwrap();
        document.roles[0].sequences[0].steps[0].args["anchor"]["tile"] =
            serde_json::json!([0, 0, 0]);
        let data = selected();
        let quests = quests(&data);
        let error = match compile_uncached_for_test(&document, &data, &quests) {
            Err(error) => error,
            Ok(_) => panic!("uncovered tile compiled"),
        };
        assert_eq!(error.code.as_ref(), "nav-tile-uncovered");
    }
}
