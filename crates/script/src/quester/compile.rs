//! Lazy per-activation Path compiler and the `(pin, digest, ABI)` weak cache.
use super::families::{self, CompiledAcquireStep};
use super::path::{PathDocument, StepDocument};
pub use super::progress::CompiledProgress;
use crate::native::{ActionContext, ActionError, NativeActions, NativeTick};
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
    pub bank_items: &'a [i32],
    pub loadouts: &'a super::loadouts::LoadoutOverlay,
}
#[derive(Debug, Clone)]
pub struct CompileError {
    pub path: FactKey,
    pub role: Option<FactKey>,
    pub step: Option<FactKey>,
    pub code: Arc<str>,
    pub source: Option<SourceSpan>,
}

impl CompileError {
    pub fn code(code: &'static str) -> Self {
        Self {
            path: FactKey::new(""),
            role: None,
            step: None,
            code: Arc::from(code),
            source: None,
        }
    }

    pub fn with_path(mut self, path: FactKey) -> Self {
        self.path = path;
        self
    }
}

pub struct CompiledPath {
    pub id: FactKey,
    pub display_name: Arc<str>,
    pub digest: [u8; 32],
    pub colour_not_started: FactKey,
    pub colour_in_progress: FactKey,
    pub colour_complete: FactKey,
    pub progress: CompiledProgress,
    pub prelude: Vec<CompiledStep>,
    pub sequences: Vec<CompiledSequence>,
    pub warnings: Vec<Arc<str>>,
}

pub struct CompiledSequence {
    pub stage: FactKey,
    pub terminal: bool,
    pub steps: Vec<CompiledStep>,
}

pub struct CompiledStep {
    pub id: FactKey,
    pub kind: Arc<str>,
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
    let bank = match &header.bank {
        super::path::QuestBankDocument::Nearest(_) => None,
        super::path::QuestBankDocument::Tile { tile, .. } => Some(NamedBank::new(
            "Path bank",
            api::WorldTile {
                x: tile[0],
                z: tile[1],
                level: tile[2],
            },
        )),
    };
    let mut bank_items = Vec::new();
    for alias in header.items.iter().map(|item| item.obj.as_str()).chain(
        header
            .tools
            .iter()
            .filter_map(|tool| tool.strip_prefix("obj:")),
    ) {
        if let Some(item) = selected.item_by_alias(alias) {
            if !bank_items.contains(&item.id) {
                bank_items.push(item.id);
            }
        }
    }
    if bank_items.len() > super::bank_memo::MAX_BANK_MEMO {
        return Err(CompileError::code("bank-memo-too-large").with_path(document.id.clone()));
    }
    let compiled_loadouts: Vec<_> = header
        .loadouts
        .iter()
        .map(|(name, row)| {
            let mut out = crate::loadouts_store::Loadout::new(format!("{}/{name}", document.id.0));
            for (slot, item) in &row.worn {
                out = out.with_slot(slot, item);
            }
            for carry in &row.carry {
                out = out.with_carry(&carry.item, carry.qty);
            }
            out
        })
        .collect();
    let loadouts =
        super::loadouts::LoadoutOverlay::from_default_store(Arc::from(compiled_loadouts));
    let mut recipe_ctx = CompileContext {
        path: &document.id,
        progress: &compiled_progress,
        selected,
        quests,
        gathering: None,
        areas: &areas,
        recipes: &empty_recipes,
        bank,
        bank_items: &bank_items,
        loadouts: &loadouts,
    };
    let mut recipes = HashMap::new();
    for (name, steps) in &header.acquire {
        recipes.insert(
            name.clone(),
            compile_steps(steps, &recipe_ctx, document)?
                .into_iter()
                .map(|step| CompiledAcquireStep {
                    advances: step.advances,
                    skip_if: step.skip_if,
                    settle: step.settle,
                    plan: step.plan,
                })
                .collect(),
        );
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
    Ok(CompiledPath {
        id: document.id.clone(),
        display_name: Arc::from(document.display_name.as_str()),
        digest,
        colour_not_started: progress.colour.not_started.clone(),
        colour_complete: progress.colour.complete.clone(),
        colour_in_progress: progress.colour.in_progress.clone(),
        progress: compiled_progress,
        prelude,
        sequences,
        warnings,
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
                source: None,
            });
        }
        if step.skip_if.constant_truth() == Some(true) {
            return Err(CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: Arc::from("always-skipped"),
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
                source: None,
            })?
        };
        let skip_if =
            families::compile_predicate(&step.skip_if, cx).map_err(|err| CompileError {
                path: document.id.clone(),
                role: None,
                step: Some(step.id.clone()),
                code: err.code,
                source: None,
            })?;
        let settle = families::compile_predicate(&step.settle, cx).map_err(|err| CompileError {
            path: document.id.clone(),
            role: None,
            step: Some(step.id.clone()),
            code: err.code,
            source: None,
        })?;
        out.push(CompiledStep {
            id: step.id.clone(),
            kind: Arc::from(step.kind.as_str()),
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
    for loadout in header.loadouts.values() {
        for (slot, item) in &loadout.worn {
            if !crate::loadouts_store::is_worn_slot(slot) {
                return Err(CompileError::code("invalid-worn-slot"));
            }
            obj(item)?;
        }
        for item in &loadout.carry {
            obj(&item.item)?;
            if item.qty == 0 {
                return Err(CompileError::code("invalid-quantity"));
            }
        }
    }
    match &header.bank {
        super::path::QuestBankDocument::Nearest(name) if name == "nearest" => {}
        super::path::QuestBankDocument::Nearest(_) => {
            return Err(CompileError::code("invalid-bank"))
        }
        super::path::QuestBankDocument::Tile { tile, source } => {
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
pub const INDEX_JSON: &str = include_str!("../../paths/289/index.json");

pub fn path_bytes(id: &str) -> Option<&'static [u8]> {
    match id {
        "cook" => Some(COOK_JSON.as_bytes()),
        "sheep" => Some(SHEEP_JSON.as_bytes()),
        "runemysteries" => Some(RUNE_MYSTERIES_JSON.as_bytes()),
        "romeojuliet" => Some(ROMEO_AND_JULIET_JSON.as_bytes()),
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
