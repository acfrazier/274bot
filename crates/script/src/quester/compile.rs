//! Lazy per-activation Path compiler and the `(pin, digest, ABI)` weak cache.
use super::families::{self, CompiledAcquireStep};
use super::path::{PathDocument, QuestItemDocument, QuestRequirementDocument, StepDocument};
pub use super::progress::CompiledProgress;
use crate::combat::RaisedPrayers;
use crate::native::{ActionContext, ActionError, NativeActions, NativeTick};
use crate::native_bank::BankItem;
use api::game_data::SelectedGameData;
use api::gather_methods::GatherCatalog;
use api::named_banks::NamedBank;
use api::quest_facts::QuestCatalog;
use api::quest_progress::{EvidenceStamp, QuestProgress};
use api::selected::{ClientRevision, FactKey, SourceSpan, Truth};
use sha2::{Digest, Sha256};
use std::{
    any::Any,
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    task::Poll,
};

pub struct CompileContext<'a> {
    pub path: &'a FactKey,
    pub progress: &'a CompiledProgress,
    pub selected: &'a SelectedGameData,
    pub quests: &'a QuestCatalog,
    pub gathering: Option<&'a GatherCatalog>,
    pub areas: &'a HashMap<String, Vec<[i32; 5]>>,
    pub recipes: &'a HashMap<String, Vec<CompiledAcquireStep>>,
    /// `None` uses the shared eligible-bank cost selector at step start.
    pub bank: Option<NamedBank>,
    pub bank_required: bool,
    pub bank_items: &'a [i32],
    pub keep_ids: &'a [i32],
    pub loadouts: &'a super::loadouts::LoadoutOverlay,
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

    pub fn with_detail(mut self, detail: &'static str) -> Self {
        self.detail = Some(Arc::from(detail));
        self
    }
}

pub struct CompiledPath {
    pub id: FactKey,
    pub role: Option<FactKey>,
    pub display_name: Arc<str>,
    pub tested_stats: Option<Arc<[api::selected::SkillMinimum]>>,
    pub digest: [u8; 32],
    pub colour_not_started: FactKey,
    pub colour_in_progress: FactKey,
    pub colour_complete: FactKey,
    pub progress: CompiledProgress,
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
}

pub struct CompiledEligibility {
    pub members: bool,
    pub requirements: Arc<[QuestRequirementDocument]>,
    pub items: Arc<[CompiledQuestItem]>,
}

#[derive(Clone)]
pub struct CompiledCarry {
    pub item: BankItem,
    pub qty: i32,
    /// Unique per-Path bit in `Provisioner::carry_drawn`.
    pub latch_index: u8,
}

pub struct CompiledProvisioning {
    pub path: FactKey,
    pub owns_inventory: bool,
    pub bank: Option<NamedBank>,
    pub bank_required: bool,
    pub items: Arc<[CompiledQuestItem]>,
    pub tools: Arc<[BankItem]>,
    pub keep_ids: Arc<[i32]>,
    pub coin_float: i32,
    pub coin: Option<CompiledCarry>,
    pub loadout_carry: HashMap<Arc<str>, Arc<[CompiledCarry]>>,
    pub base_spillover_keep: Arc<[i32]>,
    pub recipes: HashMap<Arc<str>, Arc<[CompiledAcquireStep]>>,
    pub memo_ids: Arc<[i32]>,
}

pub struct CompiledSequence {
    pub stage: FactKey,
    pub terminal: bool,
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
    pub settle: Arc<dyn PredicatePlan>,
    pub plan: Arc<dyn StepPlan>,
}

pub struct PredicateContext<'a, 'frame> {
    pub cx: &'a ActionContext<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub chat_since: i32,
    pub outcome: Option<&'a StepOutcome>,
    pub bank: &'a super::bank_memo::BankMemo,
}
pub struct StepContext<'a, 'frame> {
    pub tick: &'a mut NativeTick<'frame>,
    pub quests: &'a QuestCatalog,
    pub progress: &'a [QuestProgress],
    pub required_after: EvidenceStamp,
    pub bank: &'a super::bank_memo::BankMemo,
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
}
pub trait StepPlan: Send + Sync {
    fn begin(&self, cx: &mut StepContext<'_, '_>) -> Result<Box<dyn StepRun>, ActionError>;
    /// Post-machine predicate window, measured on the eligible clock.
    fn settle_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(8)
    }
    fn compile_warning(&self) -> Option<&'static str> {
        None
    }
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
    /// Borrowed wait detail; machines do not allocate on pending polls.
    fn waiting_for(&self) -> Option<(&'static str, &Arc<str>)> {
        None
    }
    /// Latest completed sub-operation, borrowed for change-only status reporting.
    /// This does not replace the final outcome returned by `poll`.
    fn in_flight_outcome(&self) -> Option<&StepOutcome> {
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
}
pub struct PredicateHandler {
    pub kind: &'static str,
    pub version: u16,
    pub compile: CompilePredicate,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CacheKey {
    revision: u16,
    engine: Arc<str>,
    content: Arc<str>,
    digest: [u8; 32],
    abi: u64,
}

static CACHE: std::sync::LazyLock<Mutex<HashMap<CacheKey, Weak<CompiledPath>>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn abi_set() -> u64 {
    let mut hasher = Sha256::new();
    let mut rows: Vec<_> = families::handlers()
        .iter()
        .map(|h| ("step", h.kind, h.version))
        .chain(
            families::predicate_handlers()
                .iter()
                .map(|h| ("predicate", h.kind, h.version)),
        )
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
    };
    if let Ok(cache) = CACHE.lock() {
        if let Some(hit) = cache.get(&key).and_then(Weak::upgrade) {
            return Ok(hit);
        }
    }
    let document: PathDocument =
        serde_json::from_slice(bytes).map_err(|_| CompileError::code("invalid-json"))?;
    let compiled = Arc::new(compile_uncached(&document, digest, selected, quests)?);
    if let Ok(mut cache) = CACHE.lock() {
        cache.retain(|_, weak| weak.strong_count() > 0);
        cache.insert(key, Arc::downgrade(&compiled));
    }
    Ok(compiled)
}

/// Test helper: compile without the process cache.
pub fn compile_uncached_for_test(
    document: &PathDocument,
    selected: &SelectedGameData,
    quests: &QuestCatalog,
) -> Result<Arc<CompiledPath>, CompileError> {
    compile_uncached(document, digest_bytes(b"test"), selected, quests).map(Arc::new)
}

fn compile_uncached(
    document: &PathDocument,
    digest: [u8; 32],
    selected: &SelectedGameData,
    quests: &QuestCatalog,
) -> Result<CompiledPath, CompileError> {
    if document.schema != 2 {
        return Err(CompileError::code("unsupported-schema").with_path(document.id.clone()));
    }
    let header = document
        .quest
        .as_ref()
        .ok_or_else(|| CompileError::code("missing-quest-header").with_path(document.id.clone()))?;
    validate_header(header, selected).map_err(|err| err.with_path(document.id.clone()))?;
    validate_nav_coverage(document).map_err(|err| err.with_path(document.id.clone()))?;
    let role = document
        .roles
        .first()
        .ok_or_else(|| CompileError::code("missing-role").with_path(document.id.clone()))?;
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
        selected
            .item_by_alias(alias)
            .ok_or_else(|| CompileError::code("unresolved-obj"))?
            .name
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
    let compiled_items: Arc<[CompiledQuestItem]> = Arc::from(
        header
            .items
            .iter()
            .map(|item| compile_quest_item(selected, item))
            .collect::<Result<Vec<_>, _>>()?,
    );
    let mut tools = Vec::with_capacity(header.tools.len());
    for tool in &header.tools {
        let alias = tool
            .strip_prefix("obj:")
            .ok_or_else(|| CompileError::code("invalid-tool"))?;
        tools.push(resolve_bank_item(selected, alias)?);
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
            let item = resolve_bank_item(selected, &entry.item)?;
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
                latch_index,
            });
        }
        loadout_carry.insert(Arc::from(qualified), Arc::from(carry));
    }
    let coin_float =
        i32::try_from(header.coin_float).map_err(|_| CompileError::code("invalid-coin-float"))?;
    let coin = (coin_float > 0)
        .then(|| resolve_bank_item(selected, "coins"))
        .transpose()?
        .map(|item| CompiledCarry {
            item,
            qty: coin_float,
            latch_index: u8::MAX,
        });
    let keep_ids = protected_item_ids(selected, &tools, &loadouts);
    let mut bank_items = Vec::new();
    let mut base_spillover_keep = keep_ids.clone();
    for item in compiled_items.iter() {
        push_unique_id(&mut bank_items, item.id);
        push_unique_id(&mut base_spillover_keep, item.id);
    }
    for item in &tools {
        push_unique_id(&mut bank_items, item.id);
    }
    if let Some(coin) = &coin {
        push_unique_id(&mut bank_items, coin.item.id);
        push_unique_id(&mut base_spillover_keep, coin.item.id);
    }
    for carry in loadout_carry.values().flat_map(|carry| carry.iter()) {
        push_unique_id(&mut bank_items, carry.item.id);
    }
    if bank_items.len() > super::bank_memo::MAX_BANK_MEMO {
        return Err(CompileError::code("bank-memo-too-large").with_path(document.id.clone()));
    }
    let base_spillover_keep = Arc::from(base_spillover_keep);
    let keep_ids = Arc::from(keep_ids);
    let eligibility = CompiledEligibility {
        members: header.members,
        requirements: Arc::from(header.requirements.clone()),
        items: Arc::clone(&compiled_items),
    };
    let mut recipe_ctx = CompileContext {
        path: &document.id,
        progress: &compiled_progress,
        selected,
        quests,
        gathering: None,
        areas: &areas,
        recipes: &empty_recipes,
        bank,
        bank_required,
        bank_items: &bank_items,
        loadouts: &loadouts,
        keep_ids: &keep_ids,
    };
    let mut recipes = HashMap::with_capacity(header.acquire.len());
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
        sequences.push(CompiledSequence {
            stage: sequence.stage.clone(),
            terminal: sequence.terminal,
            steps,
        });
    }
    for plan in recipes
        .values()
        .flatten()
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
    let provisioning = CompiledProvisioning {
        path: document.id.clone(),
        owns_inventory: header.owns_inventory,
        bank,
        bank_required,
        items: compiled_items,
        tools: Arc::from(tools),
        keep_ids,
        coin_float,
        coin,
        loadout_carry,
        base_spillover_keep,
        recipes: recipes
            .into_iter()
            .map(|(name, steps)| (Arc::from(name.as_str()), Arc::from(steps)))
            .collect(),
        memo_ids: Arc::from(bank_items),
    };
    Ok(CompiledPath {
        id: document.id.clone(),
        role: role.role.clone(),
        display_name: Arc::from(document.display_name.as_str()),
        tested_stats: document.tested_stats.as_deref().map(Arc::from),
        digest,
        colour_not_started: progress.colour.not_started.clone(),
        colour_complete: progress.colour.complete.clone(),
        colour_in_progress: progress.colour.in_progress.clone(),
        progress: compiled_progress,
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
    let kind = match item.kind.as_str() {
        "mustHave" | "must_have" => CompiledItemKind::MustHave,
        "acquirable" => CompiledItemKind::Acquirable,
        _ => return Err(CompileError::code("invalid-item-kind")),
    };
    i32::try_from(item.qty).map_err(|_| CompileError::code("invalid-quantity"))?;
    let resolved = resolve_bank_item(selected, &item.obj)?;
    Ok(CompiledQuestItem {
        id: resolved.id,
        name: resolved.name,
        qty: item.qty,
        kind,
        acquire: item.acquire.as_deref().map(Arc::from),
    })
}

fn resolve_bank_item(selected: &SelectedGameData, name: &str) -> Result<BankItem, CompileError> {
    let item = selected
        .resolve_item_name(name)
        .ok_or_else(|| CompileError::code("unresolved-obj"))?;
    let display = item
        .name
        .as_deref()
        .ok_or_else(|| CompileError::code("unresolved-obj-name"))?;
    Ok(BankItem {
        id: item.id,
        name: Arc::from(display),
    })
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
        let handler = families::handlers()
            .iter()
            .find(|handler| handler.kind == step.kind && handler.version == step.version)
            .ok_or_else(|| CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("unknown-handler"),
                detail: None,
                source: None,
            })?;
        let plan = if step.kind == "acquire" {
            compile_acquire_step(&step.args, cx, document)?
        } else {
            (handler.compile)(&step.args, cx).map_err(|err| CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: err.code,
                detail: err.detail,
                source: None,
            })?
        };
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
            advances: step.advances,
            skip_if,
            settle,
            plan,
        });
    }
    Ok(out)
}

fn compile_acquire_step(
    args: &serde_json::Value,
    cx: &CompileContext<'_>,
    document: &PathDocument,
) -> Result<Arc<dyn StepPlan>, CompileError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct AcquireArgs {
        recipe: String,
    }
    let arg: AcquireArgs =
        serde_json::from_value(args.clone()).map_err(|_| CompileError::code("invalid-args"))?;
    let steps =
        cx.recipes.get(&arg.recipe).cloned().ok_or_else(|| {
            CompileError::code("unresolved-recipe").with_path(document.id.clone())
        })?;
    Ok(Arc::new(families::AcquirePlan {
        recipe: Arc::from(arg.recipe),
        steps,
    }))
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
    recipes: &mut HashMap<String, Vec<CompiledAcquireStep>>,
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
    let compiled = compile_steps(steps, &context, document)?
        .into_iter()
        .map(|step| CompiledAcquireStep {
            advances: step.advances,
            skip_if: step.skip_if,
            settle: step.settle,
            plan: step.plan,
        })
        .collect();
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
            .ok_or_else(|| CompileError::code("unresolved-obj"))
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
        super::path::QuestBankDocument::Nearest(name) if name == "nearest" => {}
        super::path::QuestBankDocument::Nearest(_) => {
            return Err(CompileError::code("invalid-bank"))
        }
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
pub const INDEX_JSON: &str = include_str!("../../paths/289/index.json");

pub fn path_bytes(id: &str) -> Option<&'static [u8]> {
    match id {
        "cook" => Some(COOK_JSON.as_bytes()),
        "sheep" => Some(SHEEP_JSON.as_bytes()),
        "runemysteries" => Some(RUNE_MYSTERIES_JSON.as_bytes()),
        "romeojuliet" => Some(ROMEO_AND_JULIET_JSON.as_bytes()),
        "imp" => Some(IMP_JSON.as_bytes()),
        "vampire" => Some(VAMPIRE_JSON.as_bytes()),
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
        let data = selected();
        let quests = quests(&data);
        let first = compile_path(cook_bytes(), &data, &quests).unwrap();
        let hit = compile_path(cook_bytes(), &data, &quests).unwrap();
        assert!(Arc::ptr_eq(&first, &hit));
        let mut changed: serde_json::Value = serde_json::from_slice(cook_bytes()).unwrap();
        changed["id"] = serde_json::json!("cook-cache-different");
        let bytes = serde_json::to_vec(&changed).unwrap();
        let miss = compile_path(&bytes, &data, &quests).unwrap();
        assert!(!Arc::ptr_eq(&first, &miss));
        assert_eq!(first.id.0.as_ref(), "cook");
        assert_eq!(miss.id.0.as_ref(), "cook-cache-different");
        assert_ne!(first.digest, miss.digest);
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
            advances: false,
            skip_if: PredicateDocument::Any(vec![]),
            settle: PredicateDocument::All(vec![]),
        }
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
        assert_eq!(compiled.provisioning.recipes["acquire:a-root"].len(), 2);
        assert_eq!(compiled.provisioning.recipes["acquire:z-leaf"].len(), 1);
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
        assert_eq!(
            compile_err(|document| {
                document.roles[0].sequences[0].steps[0].args["npc"] =
                    serde_json::json!("no_such_npc");
            })
            .code
            .as_ref(),
            "unresolved-npc"
        );
        assert_eq!(
            compile_err(|document| {
                document.roles[0].sequences[1].steps[0].skip_if = PredicateDocument::Fact {
                    kind: "has_item".into(),
                    version: 1,
                    args: serde_json::json!({"obj": "no_such_obj"}),
                };
            })
            .code
            .as_ref(),
            "unresolved-obj"
        );
        assert_eq!(
            compile_err(|document| {
                document
                    .quest
                    .as_mut()
                    .unwrap()
                    .acquire
                    .get_mut("acquire:flour")
                    .unwrap()[3]
                    .args["target"]["loc"] = serde_json::json!("no_such_loc");
            })
            .code
            .as_ref(),
            "unresolved-loc"
        );
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
        assert_eq!(
            compile_err(|d| d.schema = 1).code.as_ref(),
            "unsupported-schema"
        );
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
        let bank = super::super::bank_memo::BankMemo::default();
        let banks = Arc::new(api::named_banks::NamedBankFacts::empty());
        let mut ledger = None;
        families::tests::with_tick(&snapshot, &mut ledger, 1, |tick| {
            let required_after = tick.cx.evidence();
            let mut context = StepContext {
                tick,
                quests: &quests,
                progress: &[],
                required_after,
                bank: &bank,
                banks: &banks,
                choices: &super::super::choices::QuestChoices::default(),
            };
            if let Err(error) = path.prelude[0].plan.begin(&mut context) {
                panic!("valid alias loadout could not begin application: {error:?}");
            }
        });
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
