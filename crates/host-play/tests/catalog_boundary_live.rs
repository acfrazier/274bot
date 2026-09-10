//! Headless frozen-catalog proof through production Play.
//!
//! The ignored live cell starts one real catalog card only after its registered
//! scenario has completed all preparation waits. The production slot thread
//! remains the sole owner of gameplay actions; this harness observes snapshots.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::obj_names::ObjNames;
use api::snapshot::GameSnapshot;
use host::Pump;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const SUPPORT_MATRIX: &str = include_str!("../../../docs/compat/support-matrix.json");
const CORE_SCENARIOS: &str = "bone_burier|chicken_killer|thiever|alcher|alcher_custom|alcher_ordered|alcher_large_batch|bank_fletcher";
const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum CoreCase {
    BoneBurier,
    ChickenKiller,
    Thiever,
    Alcher,
    AlcherCustom,
    AlcherOrdered,
    AlcherLargeBatch,
    BankFletcher,
}

impl CoreCase {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "bone_burier" => Ok(Self::BoneBurier),
            "chicken_killer" => Ok(Self::ChickenKiller),
            "thiever" => Ok(Self::Thiever),
            "alcher" => Ok(Self::Alcher),
            "alcher_custom" => Ok(Self::AlcherCustom),
            "alcher_ordered" => Ok(Self::AlcherOrdered),
            "alcher_large_batch" => Ok(Self::AlcherLargeBatch),
            "bank_fletcher" => Ok(Self::BankFletcher),
            _ => Err(format!(
                "unknown CATALOG_SCENARIO {value:?}; expected {CORE_SCENARIOS}"
            )),
        }
    }

    fn scenario_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "bone_burier",
            Self::ChickenKiller => "chicken_killer",
            Self::Thiever => "thiever",
            Self::Alcher => "alcher",
            Self::AlcherCustom => "alcher_custom",
            Self::AlcherOrdered => "alcher_ordered",
            Self::AlcherLargeBatch => "alcher_large_batch",
            Self::BankFletcher => "bank_fletcher",
        }
    }

    fn card_name(self) -> &'static str {
        match self {
            Self::BoneBurier => "BoneBurier",
            Self::ChickenKiller => "ChickenKiller",
            Self::Thiever => "Thiever",
            Self::Alcher => "Alcher",
            Self::AlcherCustom | Self::AlcherOrdered | Self::AlcherLargeBatch => "Alcher",
            Self::BankFletcher => "BankFletcher",
        }
    }
}

#[derive(Debug, Deserialize)]
struct SupportMatrix {
    catalogs: Vec<CatalogLedger>,
    rows: Vec<CardLedger>,
}

#[derive(Debug, Clone, Deserialize)]
struct CatalogLedger {
    commit: String,
    identity: String,
    read_only_path: String,
    registry_path: String,
    registry_sha256: String,
}

#[derive(Debug, Clone, Deserialize)]
struct CardLedger {
    display_name: String,
    catalog_commit: String,
    source_path: String,
    source_sha256: String,
    revision: u16,
}

fn support_matrix() -> Result<SupportMatrix, String> {
    serde_json::from_str(SUPPORT_MATRIX).map_err(|error| format!("support matrix: {error}"))
}

fn validate_commit(commit: &str) -> Result<(), String> {
    if commit.len() != 40
        || !commit
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!(
            "CATALOG_COMMIT must be the exact lowercase 40-hex frozen commit, got {commit:?}"
        ));
    }
    Ok(())
}

fn catalog_ledger<'a>(
    matrix: &'a SupportMatrix,
    commit: &str,
) -> Result<&'a CatalogLedger, String> {
    validate_commit(commit)?;
    let rows = matrix
        .catalogs
        .iter()
        .filter(|catalog| catalog.commit == commit)
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [catalog] => Ok(*catalog),
        [] => Err(format!(
            "CATALOG_COMMIT {commit} is not frozen in support-matrix.json"
        )),
        _ => Err(format!(
            "CATALOG_COMMIT {commit} is duplicated in support-matrix.json"
        )),
    }
}

fn ledger_card(
    matrix: &SupportMatrix,
    commit: &str,
    revision: u16,
    case: CoreCase,
) -> Result<CardLedger, String> {
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            row.catalog_commit == commit
                && row.revision == revision
                && row.display_name == case.card_name()
        })
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(format!(
            "support matrix has no {} row for catalog {commit} revision {revision}",
            case.card_name()
        )),
        _ => Err(format!(
            "support matrix has duplicate {} rows for catalog {commit} revision {revision}",
            case.card_name()
        )),
    }
}

fn safe_catalog_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative.components().any(|part| {
            matches!(
                part,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!("unsafe support-matrix catalog path {relative:?}"));
    }
    Ok(root.join(relative))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

fn verify_source_identity(root: &Path, row: &CardLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &row.source_path)?;
    let actual = hash_file(&path)?;
    if actual != row.source_sha256 {
        return Err(format!(
            "source SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            row.source_sha256,
            actual
        ));
    }
    Ok(path)
}

fn verify_registry_identity(root: &Path, catalog: &CatalogLedger) -> Result<PathBuf, String> {
    let path = safe_catalog_path(root, &catalog.registry_path)?;
    let actual = hash_file(&path)?;
    if actual != catalog.registry_sha256 {
        return Err(format!(
            "registry SHA-256 mismatch for {}: expected {}, got {}",
            path.display(),
            catalog.registry_sha256,
            actual
        ));
    }
    Ok(path)
}

#[derive(Debug, Clone, Serialize)]
struct Observation {
    ingame: bool,
    scene_state: i32,
    player: Option<String>,
    tile: Option<(i32, i32, i32)>,
    tick: u32,
    items: BTreeMap<String, i32>,
    bank: BTreeMap<String, i32>,
    bank_open: bool,
    bank_loaded: bool,
    bank_generation: u64,
    levels: BTreeMap<String, i32>,
    xp: BTreeMap<String, i32>,
    chat: Vec<(i32, String)>,
}

impl Observation {
    fn from_snapshot(snapshot: &GameSnapshot, names: &ObjNames) -> Self {
        let mut items = BTreeMap::new();
        for (id, count) in snapshot.inv() {
            let name = names
                .name(*id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{id}"));
            *items.entry(name).or_insert(0) += *count;
        }
        let xp = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.xp))
            .collect();
        let mut bank = BTreeMap::new();
        for row in snapshot.bank() {
            let name = names
                .name(row.def.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{}", row.def.id));
            *bank.entry(name).or_insert(0) += row.count;
        }
        let levels = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.base))
            .collect();
        let chat = snapshot
            .chat_lines()
            .iter()
            .map(|line| (line.sequence, line.text.clone()))
            .collect();
        let player = snapshot
            .local_player()
            .and_then(|local| local.player.actor.name.clone());
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            player,
            tile: snapshot.tile(),
            tick: snapshot.tick(),
            items,
            bank,
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_generation: snapshot.bank_session_generation(),
            levels,
            xp,
            chat,
        }
    }

    fn item(&self, name: &str) -> i32 {
        self.items.get(name).copied().unwrap_or(0)
    }

    fn skill_xp(&self, name: &str) -> i32 {
        self.xp.get(name).copied().unwrap_or(0)
    }

    fn bank_item(&self, name: &str) -> i32 {
        self.bank.get(name).copied().unwrap_or(0)
    }

    fn level(&self, name: &str) -> i32 {
        self.levels.get(name).copied().unwrap_or(0)
    }
}

fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

fn validate_case_baseline(case: CoreCase, baseline: &Observation) -> Result<(), String> {
    let ready = match case {
        CoreCase::BoneBurier => {
            near(baseline.tile, (3220, 3212, 0), 8) && baseline.item("Bones") >= 5
        }
        CoreCase::ChickenKiller => near(baseline.tile, (3235, 3295, 0), 8),
        CoreCase::Thiever => {
            near(baseline.tile, (2661, 3306, 0), 10)
                && baseline.item("Lobster") >= 10
                && baseline.level("thieving") >= 50
                && baseline.level("hitpoints") >= 50
        }
        CoreCase::Alcher
        | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => {
            near(baseline.tile, (3185, 3440, 0), 6) && baseline.level("magic") >= 55
        }
        CoreCase::BankFletcher => {
            near(baseline.tile, (3185, 3440, 0), 6)
                && baseline.item("Knife") >= 1
                && baseline.item("Willow logs") >= 27
                && baseline.level("fletching") >= 35
        }
    };
    if ready {
        return Ok(());
    }
    let requirement = match case {
        CoreCase::BoneBurier => "Lumbridge mainland and five Bones",
        CoreCase::ChickenKiller => "Lumbridge chicken pen",
        CoreCase::Thiever => "Ardougne guard stand, ten Lobsters, and prepared stats",
        CoreCase::Alcher
        | CoreCase::AlcherCustom
        | CoreCase::AlcherOrdered
        | CoreCase::AlcherLargeBatch => "Varrock West bank and Magic 55",
        CoreCase::BankFletcher => {
            "Varrock West bank, Knife, twenty-seven Willow logs, and Fletching 35"
        }
    };
    Err(format!(
        "{} Start baseline lacks required preparation ({requirement}): {baseline:?}",
        case.scenario_name()
    ))
}

#[derive(Debug, Clone, Serialize)]
struct CoreWitness {
    case: CoreCase,
    baseline: Observation,
    latest: Observation,
    max_items: BTreeMap<String, i32>,
    max_xp: BTreeMap<String, i32>,
    saw_bury_chat: bool,
    post_start_observations: u64,
    bone_bank_cycle: BoneBankCycle,
    ordered_first_exhausted: bool,
}

/// Ordered observations: seed depletion alone must never qualify this card.
#[derive(Debug, Clone, Default, Serialize)]
struct BoneBankCycle {
    first_batch_buried: bool,
    opened: Option<Observation>,
    withdrawn: Option<Observation>,
    buried_after_withdrawal: bool,
}

impl BoneBankCycle {
    fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_batch_buried |=
            now.item("Bones") == 0 && now.skill_xp("prayer") - baseline.skill_xp("prayer") >= 22;
        if self.first_batch_buried
            && self.opened.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item("Bones") == 0
            && now.bank_item("Bones") == 28
            && now.tile != baseline.tile
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == opened.bank_generation
                && now.item("Bones") == 28
                && now.bank_item("Bones") == 0
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.buried_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item("Bones") < withdrawn.item("Bones")
                && now.skill_xp("prayer") > withdrawn.skill_xp("prayer");
        }
    }
}

impl CoreWitness {
    fn new(case: CoreCase, baseline: Observation) -> Self {
        Self {
            max_items: baseline.items.clone(),
            max_xp: baseline.xp.clone(),
            latest: baseline.clone(),
            baseline,
            case,
            saw_bury_chat: false,
            post_start_observations: 0,
            bone_bank_cycle: BoneBankCycle::default(),
            ordered_first_exhausted: false,
        }
    }

    fn observe(&mut self, observation: &Observation) {
        if !observation.ingame
            || observation.scene_state != 2
            || observation.player != self.baseline.player
        {
            return;
        }
        if matches!(self.case, CoreCase::BoneBurier) {
            self.bone_bank_cycle.observe(&self.baseline, observation);
        }
        let baseline_sequence = self
            .baseline
            .chat
            .iter()
            .map(|(sequence, _)| *sequence)
            .max()
            .unwrap_or(i32::MIN);
        self.saw_bury_chat |= observation.chat.iter().any(|(sequence, text)| {
            *sequence > baseline_sequence && text.to_ascii_lowercase().contains("bury the bones")
        });
        for (name, count) in &observation.items {
            let peak = self.max_items.entry(name.clone()).or_insert(*count);
            *peak = (*peak).max(*count);
        }
        for (name, xp) in &observation.xp {
            let peak = self.max_xp.entry(name.clone()).or_insert(*xp);
            *peak = (*peak).max(*xp);
        }
        self.ordered_first_exhausted |= self.max_items.get("Rune platebody").copied().unwrap_or(0)
            > 0
            && observation.item("Rune platebody") == 0
            && observation.item("Rune chainbody") > 0;
        self.latest = observation.clone();
        self.post_start_observations += 1;
    }

    fn peak_item(&self, name: &str) -> i32 {
        self.max_items.get(name).copied().unwrap_or(0)
    }

    fn xp_gained(&self, skill: &str) -> bool {
        self.latest.skill_xp(skill) > self.baseline.skill_xp(skill)
    }

    fn item_increased(&self, name: &str) -> bool {
        self.peak_item(name) > self.baseline.item(name)
    }

    fn item_consumed_from_baseline(&self, name: &str) -> bool {
        self.latest.item(name) < self.baseline.item(name)
    }

    fn acquired_then_consumed(&self, name: &str) -> bool {
        self.item_increased(name) && self.latest.item(name) < self.peak_item(name)
    }

    fn qualify(&self) -> Result<Value, String> {
        if self.post_start_observations == 0 {
            return Err("no post-Start observations".into());
        }
        let ok = match self.case {
            CoreCase::BoneBurier => {
                self.bone_bank_cycle.buried_after_withdrawal && self.saw_bury_chat
            }
            CoreCase::ChickenKiller => {
                self.xp_gained("strength")
                    && self.acquired_then_consumed("Bones")
                    && self.xp_gained("prayer")
                    && self.saw_bury_chat
            }
            CoreCase::Thiever => self.xp_gained("thieving") && self.item_increased("Coins"),
            CoreCase::Alcher
            | CoreCase::AlcherCustom
            | CoreCase::AlcherOrdered
            | CoreCase::AlcherLargeBatch => {
                self.xp_gained("magic")
                    && self.acquired_then_consumed("Nature rune")
                    && self.item_increased("Coins")
                    && match self.case {
                        CoreCase::AlcherOrdered => {
                            self.peak_item("Rune platebody") >= 1
                                && self.peak_item("Rune chainbody") >= 1
                                && self.ordered_first_exhausted
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        CoreCase::AlcherLargeBatch => {
                            self.peak_item("Rune chainbody") >= 1000
                                && self.peak_item("Nature rune") >= 1000
                                && self.acquired_then_consumed("Rune chainbody")
                        }
                        _ => self.acquired_then_consumed("Rune chainbody"),
                    }
            }
            CoreCase::BankFletcher => {
                self.xp_gained("fletching")
                    && self.item_consumed_from_baseline("Willow logs")
                    && self.item_increased("Willow shortbow")
            }
        };
        if !ok {
            return Err(format!(
                "{} core post-Start delta incomplete",
                self.case.scenario_name()
            ));
        }
        Ok(json!({
            "case": self.case,
            "baseline": self.baseline,
            "latest": self.latest,
            "max_items": self.max_items,
            "max_xp": self.max_xp,
            "saw_bury_chat": self.saw_bury_chat,
            "post_start_observations": self.post_start_observations,
            "bone_bank_cycle": self.bone_bank_cycle,
            "ordered_first_exhausted": self.ordered_first_exhausted,
        }))
    }
}

fn witness<'a>(
    case: CoreCase,
    baseline: &Observation,
    observations: impl IntoIterator<Item = &'a Observation>,
) -> CoreWitness {
    let mut witness = CoreWitness::new(case, baseline.clone());
    for observation in observations {
        witness.observe(observation);
    }
    witness
}

#[derive(Clone)]
struct PendingStart {
    js: String,
    shape: script::LoadShape,
    settings: Option<Map<String, Value>>,
    siblings: Vec<(String, String)>,
}

struct LiveState {
    runner: ScenarioRunner,
    case: CoreCase,
    account: String,
    names: Arc<ObjNames>,
    snapshot: GameSnapshot,
    pump: Pump,
    start_handle: Option<ScriptStartHandle>,
    pending: Option<PendingStart>,
    baseline: Option<Observation>,
    witness: Option<CoreWitness>,
    start_error: Option<String>,
    start_count: u32,
}

impl LiveState {
    fn publish(&mut self, client: &client::client::Client) -> Observation {
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        Observation::from_snapshot(&self.snapshot, &self.names)
    }

    fn capture_baseline(&mut self, observation: &Observation) -> Result<(), String> {
        if !observation.ingame || observation.scene_state != 2 {
            return Err(format!(
                "Start baseline is not attached ingame scene2: {observation:?}"
            ));
        }
        let player = observation
            .player
            .as_deref()
            .ok_or_else(|| "Start baseline has no local player".to_string())?;
        let expected_player = client::util::jstring::JString::to_screen_name(&self.account);
        if !player.eq_ignore_ascii_case(&expected_player) {
            return Err(format!(
                "Start baseline player {player:?} is not fresh account {:?}",
                self.account
            ));
        }
        validate_case_baseline(self.case, observation)?;
        self.baseline = Some(observation.clone());
        self.witness = Some(CoreWitness::new(self.case, observation.clone()));
        println!(
            "{}",
            json!({
                "phase": "baseline-after-preparation",
                "scenario": self.case,
                "account": self.account,
                "observation": observation,
            })
        );
        Ok(())
    }

    fn frame(&mut self, client: &mut client::client::Client, hold: bool) {
        let observation = self.publish(client);
        let at_start = self.runner.on_start_script();
        if at_start && self.baseline.is_none() {
            if let Err(error) = self.capture_baseline(&observation) {
                self.start_error = Some(error);
                return;
            }
        } else if self.start_count == 1 {
            if let Some(witness) = self.witness.as_mut() {
                witness.observe(&observation);
            }
        }

        if at_start && self.pending.is_some() {
            let Some(handle) = self.start_handle.as_ref() else {
                self.start_error =
                    Some("StartScript reached before ScriptStartHandle install".into());
                return;
            };
            let pending = self.pending.as_ref().expect("pending checked");
            match handle.start_load(
                &self.account,
                pending.js.clone(),
                pending.shape,
                pending.settings.clone(),
                pending.siblings.clone(),
            ) {
                Ok(()) => {
                    self.pending = None;
                    self.start_count += 1;
                    println!(
                        "{}",
                        json!({"phase": "start", "account": self.account, "count": self.start_count})
                    );
                }
                Err(error) => {
                    self.start_error = Some(format!("catalog Start refused: {error}"));
                    return;
                }
            }
        }

        if !matches!(
            self.runner.status(),
            RunnerStatus::Passed | RunnerStatus::Failed(_)
        ) {
            self.runner.tick_with_hold(client, hold);
        }
    }
}

struct TempRoot(PathBuf);

impl TempRoot {
    fn new() -> Result<Self, String> {
        let serial = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "274bot-catalog-boundary-{}-{serial}",
            std::process::id()
        ));
        std::fs::create_dir_all(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required"))
}

fn selected_profile(
    revision: u16,
    nav_pack: PathBuf,
    catalog_root: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        revision: Some(revision.to_string()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(if revision == 289 { 44594 } else { 43594 }),
        http_port: Some(if revision == 289 { 1080 } else { 80 }),
        nav_pack: Some(nav_pack),
        nav_flags: std::env::var_os("CATALOG_NAV_FLAGS").map(PathBuf::from),
        engine_dir: std::env::var_os("CATALOG_ENGINE_DIR").map(PathBuf::from),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.target() != client::BotTarget::Local || profile.client().game_host() != "127.0.0.1" {
        return Err("catalog proof requires a loopback-only local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected template has no navigation world".into());
    }
    Ok((profile, template))
}

fn explicit_settings(schema: &[script::SettingDef]) -> Result<Map<String, Value>, String> {
    let Some(raw) = std::env::var_os("CATALOG_SETTINGS_JSON") else {
        return Ok(Map::new());
    };
    let raw = raw
        .into_string()
        .map_err(|_| "CATALOG_SETTINGS_JSON is not UTF-8".to_string())?;
    let value: Value = serde_json::from_str(&raw)
        .map_err(|error| format!("malformed CATALOG_SETTINGS_JSON: {error}"))?;
    let map = value
        .as_object()
        .cloned()
        .ok_or_else(|| "CATALOG_SETTINGS_JSON must be a JSON object".to_string())?;
    for key in map.keys() {
        if !schema.iter().any(|field| field.id == *key) {
            return Err(format!("CATALOG_SETTINGS_JSON has unknown setting {key:?}"));
        }
    }
    Ok(map)
}

fn printable_settings(settings: &Map<String, Value>) -> Map<String, Value> {
    settings
        .iter()
        .map(|(key, value)| {
            let lower = key.to_ascii_lowercase();
            let value = if ["password", "passphrase", "token", "secret", "private_key"]
                .iter()
                .any(|needle| lower.contains(needle))
            {
                Value::String("[REDACTED]".into())
            } else {
                value.clone()
            };
            (key.clone(), value)
        })
        .collect()
}

fn prepare_catalog_card(
    root: &Path,
    temp: &Path,
    row: &CardLedger,
    case: CoreCase,
    scenario_inject: Option<&'static [scenario::ScriptSettingInject]>,
) -> Result<(PendingStart, Value), String> {
    let source_path = verify_source_identity(root, row)?;
    let mut library =
        script::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let registered = library.register_rs2b0t(root, &temp.join("rs2b0t-path"))?;
    if registered == 0 {
        return Err("frozen catalog registered no script cards".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, case.card_name())?;
    let card = library
        .get(script::ScriptSource::Catalog, case.card_name())
        .cloned()
        .ok_or_else(|| format!("catalog registry has no {} card", case.card_name()))?;
    if card.path != source_path {
        return Err(format!(
            "registry path mismatch for {}: ledger {}, loader {}",
            case.card_name(),
            source_path.display(),
            card.path.display()
        ));
    }
    if card.sha256 != row.source_sha256 {
        return Err(format!(
            "loader source hash mismatch for {}: ledger {}, loader {}",
            case.card_name(),
            row.source_sha256,
            card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{} is unloadable: {import}", case.card_name()));
    }

    let scenario_settings = scenario::settings_inject_map(scenario_inject).unwrap_or_default();
    let explicit = explicit_settings(&card.settings_schema)?;
    // Required order for this controlled proof: schema defaults, scenario
    // injections, then explicit one-cell settings. Nothing is persisted.
    let settings = script::merge_bag(&card.settings_schema, &scenario_settings, Some(&explicit));
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
        },
    )?;
    let sibling_hashes = siblings
        .iter()
        .map(|(url, js)| {
            json!({
                "module": url,
                "compiled_sha256": script::JsCache::origin_sha(js.as_bytes()),
            })
        })
        .collect::<Vec<_>>();
    let identity = json!({
        "card": case.card_name(),
        "source_path": card.path,
        "source_sha256": card.sha256,
        "compiled_sha256": script::JsCache::origin_sha(card.js.as_bytes()),
        "siblings": sibling_hashes,
        "settings": printable_settings(&settings),
    });
    Ok((
        PendingStart {
            js: card.js,
            shape: card.shape,
            settings: (!settings.is_empty()).then_some(settings),
            siblings,
        },
        identity,
    ))
}

fn run_cell() -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Ok(());
    }
    let revision = required("CATALOG_REVISION")?
        .parse::<u16>()
        .map_err(|_| "CATALOG_REVISION must be 274 or 289".to_string())?;
    if !matches!(revision, 274 | 289) {
        return Err("CATALOG_REVISION must be 274 or 289".into());
    }
    let root = PathBuf::from(required("CATALOG_ROOT")?);
    if !root.is_dir() {
        return Err(format!(
            "CATALOG_ROOT is not a directory: {}",
            root.display()
        ));
    }
    let commit = required("CATALOG_COMMIT")?;
    let case = CoreCase::parse(&required("CATALOG_SCENARIO")?)?;
    let nav_pack = PathBuf::from(required("CATALOG_NAV_PACK")?);
    let matrix = support_matrix()?;
    let catalog = catalog_ledger(&matrix, &commit)?;
    let row = ledger_card(&matrix, &commit, revision, case)?;
    let registry_path = verify_registry_identity(&root, catalog)?;

    let mut scenario = scenario::get(case.scenario_name())
        .ok_or_else(|| format!("scenario registry has no {}", case.scenario_name()))?;
    if scenario.settings.start_script != Some(case.card_name()) {
        return Err(format!(
            "scenario {} resolves card {:?}, expected {}",
            case.scenario_name(),
            scenario.settings.start_script,
            case.card_name()
        ));
    }
    let scenario_inject = scenario.settings.script_settings_inject;
    let deadline = scenario.settings.deadline;
    let mainland = scenario.seed.mainland;
    scenario.settings.nav.engine_speed_ms = None;

    let temp = TempRoot::new()?;
    let (profile, template) = selected_profile(revision, nav_pack, root.clone(), temp.path())?;
    let (pending, card_identity) =
        prepare_catalog_card(&root, temp.path(), &row, case, scenario_inject)?;
    let names = host_play::mint_live_names(1);
    let account = names
        .first()
        .cloned()
        .ok_or_else(|| "failed to mint live account".to_string())?;
    let credentials = host_play::mint_live_entries_for_target(&names, profile.target());
    let password = credentials
        .first()
        .map(|(_, password)| password.clone())
        .ok_or_else(|| "failed to mint local credential".to_string())?;

    let mut runner = ScenarioRunner::with_world(scenario, template.world());
    runner.set_live_names(&names);
    runner.set_shot_sink(Box::new(|_, _| {}));

    let state = Arc::new(Mutex::new(LiveState {
        runner,
        case,
        account: account.clone(),
        names: Arc::new(ObjNames::default()),
        snapshot: GameSnapshot::new(),
        pump: Pump::new(),
        start_handle: None,
        pending: Some(pending),
        baseline: None,
        witness: None,
        start_error: None,
        start_count: 0,
    }));
    let frame_state = Arc::clone(&state);
    let mut play = host_play::run_with_template(
        Arc::clone(&template),
        mainland,
        vec![],
        |_| (None, None),
        move |client, username, hold| {
            let mut state = frame_state.lock().unwrap();
            if username == state.account {
                state.frame(client, hold);
            }
        },
    )?;
    let obj_names = play.obj_names();
    {
        let mut state = state.lock().unwrap();
        state.names = Arc::clone(&obj_names);
        state.runner.set_obj_names(obj_names);
        state.start_handle = Some(play.script_start_handle());
    }
    let nav_sha256 = hash_file(profile.nav_pack())?;

    println!(
        "{}",
        json!({
            "phase": "identity",
            "revision": profile.revision().as_i32(),
            "profile": profile.label(),
            "cache_id": profile.cache_id(),
            "nav_pack": profile.nav_pack(),
            "nav_sha256": nav_sha256,
            "catalog_commit": commit,
            "catalog_identity": catalog.identity,
            "catalog_read_only_path": catalog.read_only_path,
            "catalog_root": root,
            "registry_path": registry_path,
            "registry_sha256": catalog.registry_sha256,
            "card": card_identity,
            "account": account,
            "engine_speed_ms": null,
        })
    );

    let account_profile = Profile {
        username: account.clone(),
        password,
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    play.try_spawn_slot(account_profile, None, None, None)?;
    play.focus(&account);

    let outer_deadline = Instant::now() + deadline + Duration::from_secs(5);
    let outcome = loop {
        if let Some(error) = play.script_last_error(&account) {
            break Err(format!("script error: {error}"));
        }
        let mut state = state.lock().unwrap();
        if let Some(error) = state.start_error.take() {
            break Err(error);
        }
        match state.runner.status() {
            RunnerStatus::Failed(error) => {
                break Err(format!(
                    "scenario failed: {error}; evidence={:?}",
                    state.runner.evidence()
                ));
            }
            RunnerStatus::Passed => {
                if state.start_count != 1 {
                    break Err(format!(
                        "scenario passed with {} catalog Starts; expected exactly one",
                        state.start_count
                    ));
                }
                let Some(witness) = state.witness.as_ref() else {
                    break Err("scenario passed without a Start baseline".into());
                };
                if let Ok(core) = witness.qualify() {
                    let evidence = format!("{:?}", state.runner.evidence());
                    break Ok(json!({
                        "scenario": case,
                        "runner": evidence,
                        "core": core,
                    }));
                }
            }
            RunnerStatus::Seeding | RunnerStatus::Running { .. } => {}
        }
        if Instant::now() >= outer_deadline {
            let core = state
                .witness
                .as_ref()
                .map(|witness| {
                    witness
                        .qualify()
                        .unwrap_or_else(|error| json!({"error": error, "witness": witness}))
                })
                .unwrap_or_else(|| json!({"error": "no Start baseline"}));
            break Err(format!(
                "bounded timeout; runner={:?}; core={core}",
                state.runner.status()
            ));
        }
        drop(state);
        std::thread::sleep(Duration::from_millis(20));
    };

    play.stop_slot(&account);
    let outcome = outcome?;
    println!("PASS: catalog_boundary_live: {outcome}");
    Ok(())
}

#[test]
#[ignore = "requires LIVE=1, CATALOG_REVISION/ROOT/COMMIT/SCENARIO/NAV_PACK, and local engine"]
fn catalog_boundary_live() {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_cell));
    match result {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            eprintln!("FAIL: catalog_boundary_live: {error}");
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!("FAIL: catalog_boundary_live: {error:?}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn observation(items: &[(&str, i32)], xp: &[(&str, i32)], chat: &[(i32, &str)]) -> Observation {
        Observation {
            ingame: true,
            scene_state: 2,
            player: Some("catalogtest".into()),
            tile: Some((3220, 3212, 0)),
            tick: 10,
            bank: BTreeMap::new(),
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            items: items
                .iter()
                .map(|(name, count)| ((*name).to_string(), *count))
                .collect::<BTreeMap<_, _>>(),
            levels: [
                ("thieving".to_string(), 99),
                ("hitpoints".to_string(), 99),
                ("magic".to_string(), 99),
                ("fletching".to_string(), 99),
            ]
            .into_iter()
            .collect(),
            xp: xp
                .iter()
                .map(|(name, value)| ((*name).to_string(), *value))
                .collect::<BTreeMap<_, _>>(),
            chat: chat
                .iter()
                .map(|(sequence, text)| (*sequence, (*text).to_string()))
                .collect(),
        }
    }

    #[test]
    fn source_identity_rejects_bytes_that_do_not_match_the_frozen_ledger() {
        let matrix = support_matrix().unwrap();
        let row = ledger_card(
            &matrix,
            "100adccc037d9f6898080e1cad58fcfc43364775",
            274,
            CoreCase::BoneBurier,
        )
        .unwrap();
        let root = std::env::temp_dir().join(format!(
            "catalog-boundary-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = root.join(&row.source_path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"not the frozen card").unwrap();

        let error = verify_source_identity(&root, &row).unwrap_err();

        assert!(error.contains("source SHA-256 mismatch"), "{error}");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn checked_in_frozen_core_sources_match_both_catalog_ledgers() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let matrix = support_matrix().unwrap();
        for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
            let root = repo.join(format!(".superpowers/inputs/rs2b0t-{commit}"));
            let catalog = catalog_ledger(&matrix, commit).unwrap();
            verify_registry_identity(&root, catalog).unwrap();
            for case in [
                CoreCase::BoneBurier,
                CoreCase::ChickenKiller,
                CoreCase::Thiever,
                CoreCase::Alcher,
                CoreCase::BankFletcher,
            ] {
                let row = ledger_card(&matrix, commit, 274, case).unwrap();
                verify_source_identity(&root, &row).unwrap();
            }
        }
    }

    #[test]
    fn start_baseline_rejects_a_seeded_outcome_without_case_preparation() {
        let baseline = observation(&[("Bones", 0)], &[("prayer", 100)], &[]);

        let error = validate_case_baseline(CoreCase::BoneBurier, &baseline).unwrap_err();

        assert!(error.contains("five Bones"), "{error}");
    }

    #[test]
    fn bone_burier_rejects_first_burial_and_requires_observed_bank_cycle() {
        let baseline = observation(&[("Bones", 5)], &[("prayer", 100)], &[]);
        let first = observation(
            &[("Bones", 4)],
            &[("prayer", 104)],
            &[(2, "You bury the bones.")],
        );
        assert!(witness(CoreCase::BoneBurier, &baseline, [&first])
            .qualify()
            .is_err());
        let empty = observation(&[], &[("prayer", 122)], &[(6, "You bury the bones.")]);
        assert!(witness(CoreCase::BoneBurier, &baseline, [&first, &empty])
            .qualify()
            .is_err());
        let mut opened = empty.clone();
        opened.tile = Some((3092, 3245, 0));
        opened.bank_open = true;
        opened.bank_loaded = true;
        opened.bank_generation = 1;
        opened.bank.insert("Bones".into(), 28);
        let mut withdrawn = opened.clone();
        withdrawn.items.insert("Bones".into(), 28);
        withdrawn.bank.clear();
        let mut buried = withdrawn.clone();
        buried.bank_open = false;
        buried.bank_loaded = false;
        buried.items.insert("Bones".into(), 27);
        buried.xp.insert("prayer".into(), 127);
        buried.chat.push((7, "You bury the bones.".into()));
        let observations = [&first, &empty, &opened, &withdrawn, &buried];
        assert!(witness(CoreCase::BoneBurier, &baseline, observations)
            .qualify()
            .is_ok());
        // An inventory rise without the observed bank transfer is insufficient.
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &withdrawn, &buried]
        )
        .qualify()
        .is_err());
        let mut stale = opened.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &stale, &withdrawn, &buried]
        )
        .qualify()
        .is_err());
        let mut no_consumption = buried.clone();
        no_consumption.items.insert("Bones".into(), 28);
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &opened, &withdrawn, &no_consumption]
        )
        .qualify()
        .is_err());
        let mut no_xp = buried.clone();
        no_xp.xp.insert("prayer".into(), 122);
        assert!(witness(
            CoreCase::BoneBurier,
            &baseline,
            [&empty, &opened, &withdrawn, &no_xp]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn chicken_killer_requires_combat_xp_and_a_post_start_loot_bury_cycle() {
        let baseline = observation(&[("Bones", 0)], &[("strength", 100), ("prayer", 100)], &[]);
        let looted = observation(&[("Bones", 1)], &[("strength", 104), ("prayer", 100)], &[]);
        let buried = observation(
            &[("Bones", 0)],
            &[("strength", 104), ("prayer", 104)],
            &[(3, "You bury the bones.")],
        );
        let changed = witness(CoreCase::ChickenKiller, &baseline, [&looted, &buried]);
        assert!(changed.qualify().is_ok());

        let preseeded_only = witness(CoreCase::ChickenKiller, &buried, [&buried]);
        assert!(preseeded_only.qualify().is_err());
    }

    #[test]
    fn thiever_requires_post_start_xp_and_coin_progress() {
        let baseline = observation(&[("Coins", 0)], &[("thieving", 100)], &[]);
        let after = observation(&[("Coins", 25)], &[("thieving", 108)], &[]);
        let changed = witness(CoreCase::Thiever, &baseline, [&after]);
        assert!(changed.qualify().is_ok());
    }

    #[test]
    fn alcher_requires_acquired_fuel_then_consumption_xp_and_coins() {
        let baseline = observation(
            &[("Rune chainbody", 0), ("Nature rune", 0), ("Coins", 0)],
            &[("magic", 10_000)],
            &[],
        );
        let stocked = observation(
            &[("Rune chainbody", 30), ("Nature rune", 200), ("Coins", 0)],
            &[("magic", 10_000)],
            &[],
        );
        let after = observation(
            &[
                ("Rune chainbody", 29),
                ("Nature rune", 199),
                ("Coins", 30_000),
            ],
            &[("magic", 10_065)],
            &[],
        );
        let changed = witness(CoreCase::Alcher, &baseline, [&stocked, &after]);
        assert!(changed.qualify().is_ok());
    }

    #[test]
    fn alcher_option_witnesses_reject_seeded_only_and_require_order_and_large_stack() {
        let baseline = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let seeded = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 1),
                ("Nature rune", 2),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        assert!(witness(CoreCase::AlcherOrdered, &seeded, [&seeded])
            .qualify()
            .is_err());

        let plate_only = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 0),
                ("Nature rune", 1),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let plate_only_after = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherOrdered,
            &baseline,
            [&plate_only, &plate_only_after]
        )
        .qualify()
        .is_err());

        let first = observation(
            &[
                ("Rune platebody", 1),
                ("Rune chainbody", 0),
                ("Nature rune", 2),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let second = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 1),
                ("Nature rune", 1),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        let ordered_after = observation(
            &[
                ("Rune platebody", 0),
                ("Rune chainbody", 0),
                ("Nature rune", 0),
                ("Coins", 2_000),
            ],
            &[("magic", 10_066)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherOrdered,
            &baseline,
            [&first, &second, &ordered_after]
        )
        .qualify()
        .is_ok());
        assert!(witness(
            CoreCase::AlcherLargeBatch,
            &baseline,
            [&second, &ordered_after]
        )
        .qualify()
        .is_err());

        let large = observation(
            &[
                ("Rune chainbody", 1000),
                ("Nature rune", 1000),
                ("Coins", 0),
            ],
            &[("magic", 10_000)],
            &[],
        );
        let large_after = observation(
            &[
                ("Rune chainbody", 999),
                ("Nature rune", 999),
                ("Coins", 1_000),
            ],
            &[("magic", 10_033)],
            &[],
        );
        assert!(witness(
            CoreCase::AlcherLargeBatch,
            &baseline,
            [&large, &large_after]
        )
        .qualify()
        .is_ok());
    }

    #[test]
    fn bank_fletcher_requires_logs_consumed_product_created_and_xp() {
        let baseline = observation(
            &[("Willow logs", 27), ("Willow shortbow", 0)],
            &[("fletching", 22_000)],
            &[],
        );
        let after = observation(
            &[("Willow logs", 26), ("Willow shortbow", 1)],
            &[("fletching", 22_033)],
            &[],
        );
        let changed = witness(CoreCase::BankFletcher, &baseline, [&after]);
        assert!(changed.qualify().is_ok());
    }
}
