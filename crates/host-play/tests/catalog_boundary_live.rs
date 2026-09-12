//! Headless frozen-catalog proof through production Play.
//!
//! The ignored live cell starts one real catalog card only after its registered
//! scenario has completed all preparation waits. The production slot thread
//! remains the sole owner of gameplay actions; this harness observes snapshots.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::obj_names::ObjNames;
use api::snapshot::{GameSnapshot, LocView, SceneView, WorldTile};
use host::Pump;
use host_play::catalog_core::*;
use host_play::{ProfileOptions, ScriptStartHandle, SharedClientTemplate};
use scenario::{RunnerStatus, ScenarioRunner};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const SUPPORT_MATRIX: &str = include_str!("fixtures/catalog-support-matrix.json");
const GNOME_WEST_MAGICS: (i32, i32, i32) = GNOME_SOUTH_BANK_MAGIC_STAND;
const STEEL_PICKAXE_ID: i32 = RUNE_PICKAXE_ID;

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
    /// The card's own frozen settings schema, as recorded in the matrix. Kept as
    /// raw JSON so a list/tile default cannot break the parse.
    #[serde(default)]
    settings_and_behavior_branches: Vec<DeclaredSetting>,
}

#[derive(Debug, Clone, Deserialize)]
struct DeclaredSetting {
    id: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    options: Vec<Value>,
    #[serde(default)]
    min: Option<Value>,
    #[serde(default)]
    max: Option<Value>,
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
    let canonical_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let canonical_card = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", card.path.display()))?;
    if canonical_card != canonical_source {
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

/// Accumulated CoreWitness plus latest observation for catalog FAIL/timeout
/// receipts. Does not change success, deadline, or lifecycle policy.
fn accumulated_core(witness: Option<&CoreWitness>) -> Value {
    witness
        .map(|witness| {
            witness
                .qualify()
                .unwrap_or_else(|error| json!({"error": error, "witness": witness}))
        })
        .unwrap_or_else(|| json!({"error": "no Start baseline"}))
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
    validate_case_catalog(case, &commit)?;
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
            let state = state.lock().unwrap();
            let diagnostic = failure_diagnostic(case, &state.snapshot, &state.names);
            break Err(format!("script error: {error}; diagnostic={diagnostic}"));
        }
        let mut state = state.lock().unwrap();
        if let Some(error) = state.start_error.take() {
            let diagnostic = failure_diagnostic(case, &state.snapshot, &state.names);
            break Err(format!("{error}; diagnostic={diagnostic}"));
        }
        match state.runner.status() {
            RunnerStatus::Failed(error) => {
                let core = accumulated_core(state.witness.as_ref());
                let diagnostic = failure_diagnostic(case, &state.snapshot, &state.names);
                break Err(format!(
                    "scenario failed: {error}; evidence={:?}; core={core}; diagnostic={diagnostic}",
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
            let core = accumulated_core(state.witness.as_ref());
            let diagnostic = failure_diagnostic(case, &state.snapshot, &state.names);
            break Err(format!(
                "bounded timeout; runner={:?}; core={core}; diagnostic={diagnostic}",
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn observation(items: &[(&str, i32)], xp: &[(&str, i32)], chat: &[(i32, &str)]) -> Observation {
        Observation {
            ingame: true,
            scene_state: 2,
            player: Some("catalogtest".into()),
            tile: Some((3220, 3212, 0)),
            combat_level: 0,
            tick: 10,
            bank: BTreeMap::new(),
            bank_ids: BTreeMap::new(),
            bank_open: false,
            bank_loaded: false,
            bank_generation: 0,
            items: items
                .iter()
                .map(|(name, count)| ((*name).to_string(), *count))
                .collect::<BTreeMap<_, _>>(),
            item_ids: BTreeMap::new(),
            levels: [
                ("thieving".to_string(), 99),
                ("hitpoints".to_string(), 99),
                ("magic".to_string(), 99),
                ("fletching".to_string(), 99),
            ]
            .into_iter()
            .collect(),
            effective_levels: [
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
            varps: BTreeMap::new(),
            chat: chat
                .iter()
                .map(|(sequence, text)| (*sequence, (*text).to_string()))
                .collect(),
            loc_facts: Vec::new(),
            npc_facts: Vec::new(),
            magic_tree_ready: false,
            dormant_rocks_seen: false,
            ground_loot: Vec::new(),
            local_in_combat: false,
            local_target_npc: None,
            local_health: 0,
            local_animation: 0,
            equipment_ids: BTreeMap::new(),
            main_modal: -1,
            widget_ids: BTreeSet::new(),
        }
    }

    fn observation_ids(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], xp: i32) -> Observation {
        let mut observation = observation(&[], &[("fletching", xp)], &[]);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
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
                CoreCase::DartFletcher,
                CoreCase::HerbCleaner,
                CoreCase::GemCutter,
                CoreCase::DoorOpener,
                CoreCase::GnomeCourse,
                CoreCase::WildyAgility,
                CoreCase::BrimhavenAgility,
                CoreCase::FlaxPicker,
                CoreCase::FlaxAio,
                CoreCase::HerbloreSecondaries,
                CoreCase::ChaosDruid,
                CoreCase::MossGiant,
                CoreCase::HillGiant,
                CoreCase::AutoFighter,
                CoreCase::RockCrab,
                CoreCase::GreenDragon,
                CoreCase::FireGiant,
                CoreCase::ArdyFighter,
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

    fn chicken_bank_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        strength_xp: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("strength", strength_xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn chicken_killer_bank_requires_combat_loot_fresh_deposit_return_and_further_work() {
        let mut baseline = chicken_bank_obs(FALADOR_CHICKENS, &[], &[], 100);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &baseline).is_err());
        baseline.levels.insert("attack".into(), 30);
        baseline.levels.insert("strength".into(), 30);
        validate_case_baseline(CoreCase::ChickenKillerBank, &baseline).unwrap();

        let looted = chicken_bank_obs(FALADOR_CHICKENS, &[(FEATHER_ID, 5)], &[], 104);
        let mut deposited = chicken_bank_obs((3012, 3355, 0), &[], &[(FEATHER_ID, 5)], 104);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = chicken_bank_obs(FALADOR_CHICKENS, &[], &[(FEATHER_ID, 5)], 104);
        returned.bank_generation = 2;
        let mut further = chicken_bank_obs(FALADOR_CHICKENS, &[(FEATHER_ID, 3)], &[], 108);
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::ChickenKillerBank, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::ChickenKillerBank, &baseline, [&looted])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &returned]
        )
        .qualify()
        .is_err());

        // A fabricated closed snapshot without a new modal generation is stale.
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let xp_only = chicken_bank_obs(FALADOR_CHICKENS, &[], &[], 104);
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut name_only = looted.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Feather".into(), 5);
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut far = chicken_bank_obs((3185, 3440, 0), &[], &[(FEATHER_ID, 5)], 104);
        far.bank_generation = 2;
        let mut further_far = further.clone();
        further_far.tile = Some((3185, 3440, 0));
        assert!(witness(
            CoreCase::ChickenKillerBank,
            &baseline,
            [&looted, &deposited, &far, &further_far]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(FEATHER_ID, 1);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &seeded).is_err());

        let lumbridge = chicken_bank_obs((3235, 3295, 0), &[], &[], 100);
        assert!(validate_case_baseline(CoreCase::ChickenKillerBank, &lumbridge).is_err());
        validate_case_baseline(CoreCase::ChickenKiller, &lumbridge).unwrap();
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
    fn bank_fletcher_requires_a_fresh_bank_cycle_and_further_crafting() {
        let baseline = observation(
            &[("Knife", 1), ("Willow logs", 27)],
            &[("fletching", 22_000)],
            &[],
        );
        let after = observation(
            &[("Willow logs", 26), ("Willow shortbow", 1)],
            &[("fletching", 22_033)],
            &[],
        );
        assert!(witness(CoreCase::BankFletcher, &baseline, [&after])
            .qualify()
            .is_err());
        let first_pack = observation(
            &[("Knife", 1), ("Willow shortbow", 27)],
            &[("fletching", 22_899)],
            &[],
        );
        let mut deposited = observation(&[("Knife", 1)], &[("fletching", 22_899)], &[]);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        deposited.bank =
            BTreeMap::from([("Willow shortbow".into(), 27), ("Willow logs".into(), 54)]);
        let mut withdrawn = deposited.clone();
        withdrawn.items.insert("Willow logs".into(), 27);
        withdrawn.bank.insert("Willow logs".into(), 27);
        let second_product = observation(
            &[("Knife", 1), ("Willow logs", 26), ("Willow shortbow", 1)],
            &[("fletching", 22_932)],
            &[],
        );
        let sequence = [&first_pack, &deposited, &withdrawn, &second_product];
        assert!(witness(CoreCase::BankFletcher, &baseline, sequence)
            .qualify()
            .is_ok());
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            sequence[..3].iter().copied()
        )
        .qualify()
        .is_err());
        // Stale bank rows and a changed bank session cannot prove transfer.
        deposited.bank_loaded = false;
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            [&first_pack, &deposited, &withdrawn, &second_product]
        )
        .qualify()
        .is_err());
        deposited.bank_loaded = true;
        withdrawn.bank_generation = 2;
        assert!(witness(
            CoreCase::BankFletcher,
            &baseline,
            [&first_pack, &deposited, &withdrawn, &second_product]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn bank_fletcher_string_requires_exact_ids_and_a_same_generation_bank_cycle() {
        let baseline = observation_ids(&[(60, 2), (1777, 2)], &[], 10_000);
        let first_pair = observation_ids(&[(849, 2)], &[], 10_066);
        let mut deposited = observation_ids(&[], &[(60, 28), (849, 2), (1777, 28)], 10_066);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut withdrawn = observation_ids(
            &[(60, 14), (1777, 14)],
            &[(60, 14), (849, 2), (1777, 14)],
            10_066,
        );
        withdrawn.bank_open = true;
        withdrawn.bank_loaded = true;
        withdrawn.bank_generation = 1;
        let crafted = observation_ids(&[(60, 13), (849, 1), (1777, 13)], &[], 10_099);

        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::BankFletcherString, &baseline, [&baseline])
                .qualify()
                .is_err()
        );
        assert!(
            witness(CoreCase::BankFletcherString, &baseline, [&first_pair])
                .qualify()
                .is_err()
        );

        let mut same_name_only = first_pair.clone();
        same_name_only.item_ids.clear();
        same_name_only.items.insert("Willow shortbow".into(), 2);
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&same_name_only, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());

        let mut stale_deposit = deposited.clone();
        stale_deposit.bank_loaded = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &stale_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut closed_deposit = deposited.clone();
        closed_deposit.bank_open = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &closed_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut loading_scene_deposit = deposited.clone();
        loading_scene_deposit.scene_state = 1;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &loading_scene_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
        let mut detached_deposit = deposited.clone();
        detached_deposit.ingame = false;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &detached_deposit, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());

        withdrawn.bank_generation = 2;
        assert!(witness(
            CoreCase::BankFletcherString,
            &baseline,
            [&first_pair, &deposited, &withdrawn, &crafted]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn bank_fletcher_cut_string_requires_cut_then_bank_then_string_by_id() {
        let mut baseline = observation_ids(&[(1519, 2)], &[], 20_000);
        baseline.items.insert("Knife".into(), 1);
        let cut = observation_ids(&[(60, 2)], &[], 20_066);
        let mut deposited = observation_ids(&[], &[(60, 2), (1777, 28)], 20_066);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut withdrawn = observation_ids(&[(60, 2), (1777, 14)], &[(1777, 14)], 20_066);
        withdrawn.bank_open = true;
        withdrawn.bank_loaded = true;
        withdrawn.bank_generation = 1;
        withdrawn.bank.insert("Knife".into(), 1);
        let finished = observation_ids(&[(849, 2), (1777, 12)], &[], 20_132);

        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &deposited, &withdrawn, &finished]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::BankFletcherCutString, &baseline, [&cut])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &withdrawn, &finished]
        )
        .qualify()
        .is_err());
        let mut wrong_id_finished = finished.clone();
        wrong_id_finished.item_ids.remove(&849);
        wrong_id_finished.items.insert("Willow shortbow".into(), 2);
        assert!(witness(
            CoreCase::BankFletcherCutString,
            &baseline,
            [&cut, &deposited, &withdrawn, &wrong_id_finished]
        )
        .qualify()
        .is_err());

        let mut seeded_outcome = baseline.clone();
        seeded_outcome.item_ids.insert(60, 2);
        assert!(validate_case_baseline(CoreCase::BankFletcherCutString, &seeded_outcome).is_err());
    }

    #[test]
    fn remaining_production_options_require_exact_ordered_native_outcomes() {
        let defaults = CoreCase::parse("alcher_defaults").unwrap();
        let mut baseline = alcher_generated_obs(&[], 10_000);
        baseline.bank_generation = 1;
        validate_case_baseline(defaults, &baseline).unwrap();
        let mut withdrawn = alcher_generated_obs(&[(856, 1), (NATURE_RUNE_ID, 1)], 10_000);
        withdrawn.bank_generation = 2;
        let mut consumed = alcher_generated_obs(&[(COINS_ID, 768)], 10_065);
        consumed.bank_generation = 2;
        assert!(witness(defaults, &baseline, [&withdrawn, &consumed])
            .qualify()
            .is_ok());
        assert!(witness(defaults, &baseline, [&withdrawn])
            .qualify()
            .is_err());
        let mut raw = withdrawn.clone();
        raw.item_ids.remove(&856);
        raw.item_ids.insert(855, 1);
        assert!(witness(defaults, &baseline, [&raw, &consumed])
            .qualify()
            .is_err());
        let mut chainbody = withdrawn.clone();
        chainbody.items.insert("Rune chainbody".into(), 1);
        assert!(witness(defaults, &baseline, [&chainbody, &consumed])
            .qualify()
            .is_err());

        for (name, initial, product, first_count, restock) in [
            (
                "bank_fletcher_shafts",
                vec![(1511, 27)],
                52,
                405,
                vec![(1511, 27)],
            ),
            (
                "bank_fletcher_headless",
                vec![(314, 30), (52, 30)],
                53,
                30,
                vec![(314, 60), (52, 60)],
            ),
        ] {
            let case = CoreCase::parse(name).unwrap();
            let mut baseline = observation_ids(&initial, &[], 20_000);
            baseline.tile = Some((3185, 3440, 0));
            baseline.levels.insert("fletching".into(), 1);
            if name == "bank_fletcher_shafts" {
                baseline.items.insert("Knife".into(), 1);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let first = observation_ids(&[(product, first_count)], &[], 20_100);
            let mut deposited = observation_ids(&[], &[(product, first_count)], 20_100);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            for (id, count) in &restock {
                deposited.bank_ids.insert(*id, *count * 2);
            }
            let mut withdrawn = observation_ids(&restock, &[(product, first_count)], 20_100);
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            for (id, count) in &restock {
                withdrawn.bank_ids.insert(*id, *count);
            }
            let mut further_ids = restock.clone();
            for (_, count) in &mut further_ids {
                *count -= 1;
            }
            further_ids.push((product, 1));
            let further = observation_ids(&further_ids, &[(product, first_count)], 20_101);
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &further])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            let mut seeded_product = baseline.clone();
            seeded_product.item_ids.insert(product, first_count);
            assert!(validate_case_baseline(case, &seeded_product).is_err());
        }
    }

    #[test]
    fn coal_trucks_requires_the_documented_native_combat_level() {
        let mut baseline = observation_ids(
            &[(STEEL_PICKAXE_ID, 1), (KNIFE_ID, COAL_BALLAST_KNIVES)],
            &[],
            0,
        );
        baseline.tile = Some(COAL_MINE);
        baseline.levels.insert("mining".into(), 60);
        baseline.effective_levels.insert("mining".into(), 60);
        baseline.combat_level = 54;
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &baseline).is_err());
        baseline.combat_level = 55;
        validate_case_baseline(CoreCase::CoalTrucks, &baseline).unwrap();
        let mut low_base_mining = baseline.clone();
        low_base_mining.levels.insert("mining".into(), 59);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &low_base_mining).is_err());
        let mut low_effective_mining = baseline.clone();
        low_effective_mining
            .effective_levels
            .insert("mining".into(), 59);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &low_effective_mining).is_err());
        let mut steel_pickaxe = baseline.clone();
        steel_pickaxe.item_ids.remove(&RUNE_PICKAXE_ID);
        steel_pickaxe.item_ids.insert(1269, 1);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &steel_pickaxe).is_err());
    }

    fn alcher_generated_obs(item_ids: &[(i32, i32)], magic_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("magic", magic_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("magic".into(), 55);
        observation.item_ids = item_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn alcher_generated_custom_requires_noted_id_then_exact_consumption() {
        for case in [CoreCase::AlcherCustomAlias, CoreCase::AlcherCustomName] {
            let mut baseline = alcher_generated_obs(&[], 10_000);
            baseline.bank_generation = 1;
            validate_case_baseline(case, &baseline).unwrap();

            let mut withdrawn = alcher_generated_obs(
                &[(CERT_ADAMANT_SCIMITAR_ID, 1), (NATURE_RUNE_ID, 1)],
                10_000,
            );
            withdrawn.bank_generation = 2;
            let mut consumed =
                alcher_generated_obs(&[(COINS_ID, ADAMANT_SCIMITAR_ALCH_COINS)], 10_065);
            consumed.bank_generation = 2;

            assert!(witness(case, &baseline, [&withdrawn, &consumed])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn]).qualify().is_err());

            let mut name_only = withdrawn.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Adamant scimitar".into(), 1);
            name_only.items.insert("Nature rune".into(), 1);
            assert!(witness(case, &baseline, [&name_only, &consumed])
                .qualify()
                .is_err());

            let mut unnoted = withdrawn.clone();
            unnoted.item_ids.remove(&CERT_ADAMANT_SCIMITAR_ID);
            unnoted.item_ids.insert(ADAMANT_SCIMITAR_ID, 1);
            assert!(witness(case, &baseline, [&unnoted, &consumed])
                .qualify()
                .is_err());

            let mut chainbody = alcher_generated_obs(&[(NATURE_RUNE_ID, 1)], 10_000);
            chainbody.items.insert("Rune chainbody".into(), 1);
            chainbody.items.insert("Nature rune".into(), 1);
            chainbody.bank_generation = 2;
            let mut chainbody_after = alcher_generated_obs(&[(COINS_ID, 30_000)], 10_065);
            chainbody_after.items.insert("Coins".into(), 30_000);
            chainbody_after.bank_generation = 2;
            assert!(witness(case, &baseline, [&chainbody, &chainbody_after])
                .qualify()
                .is_err());

            let mut wrong_coins = consumed.clone();
            wrong_coins.item_ids.insert(COINS_ID, 30_000);
            assert!(witness(case, &baseline, [&withdrawn, &wrong_coins])
                .qualify()
                .is_err());

            let mut no_bank_session = withdrawn.clone();
            no_bank_session.bank_generation = 1;
            assert!(witness(case, &baseline, [&no_bank_session, &consumed])
                .qualify()
                .is_err());

            let mut seeded_notes = baseline.clone();
            seeded_notes.item_ids.insert(CERT_ADAMANT_SCIMITAR_ID, 1);
            assert!(validate_case_baseline(case, &seeded_notes).is_err());
            let mut seeded_coins = baseline.clone();
            seeded_coins
                .item_ids
                .insert(COINS_ID, ADAMANT_SCIMITAR_ALCH_COINS);
            assert!(validate_case_baseline(case, &seeded_coins).is_err());
        }
    }

    fn dart_obs(item_ids: &[(i32, i32)], fletching_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("fletching", fletching_xp)], &[]);
        observation.item_ids = item_ids.iter().copied().collect();
        observation
    }

    fn herb_obs(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], herblore_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("herblore", herblore_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("herblore".into(), 5);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    fn gem_obs(item_ids: &[(i32, i32)], bank_ids: &[(i32, i32)], crafting_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("crafting", crafting_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("crafting".into(), 20);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn dart_fletcher_requires_two_observed_actions_and_both_inputs() {
        for (case, tips, product, wrong) in [
            (
                CoreCase::DartFletcher,
                BRONZE_DART_TIP_ID,
                BRONZE_DART_ID,
                IRON_DART_ID,
            ),
            (
                CoreCase::DartFletcherIron,
                IRON_DART_TIP_ID,
                IRON_DART_ID,
                BRONZE_DART_ID,
            ),
        ] {
            let baseline = dart_obs(&[(tips, 100), (FEATHER_ID, 100)], 10_000);
            validate_case_baseline(case, &baseline).unwrap();

            let first = dart_obs(&[(tips, 90), (FEATHER_ID, 90), (product, 10)], 10_018);
            let further = dart_obs(&[(tips, 80), (FEATHER_ID, 80), (product, 20)], 10_036);
            assert!(witness(case, &baseline, [&first, &further])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());

            let xp_only = dart_obs(&[(tips, 100), (FEATHER_ID, 100)], 10_018);
            assert!(witness(case, &baseline, [&xp_only, &xp_only])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Bronze dart".into(), 10);
            name_only.items.insert("Iron dart".into(), 10);
            let mut name_only_further = further.clone();
            name_only_further.item_ids.clear();
            name_only_further.items.insert("Bronze dart".into(), 20);
            assert!(witness(case, &baseline, [&name_only, &name_only_further])
                .qualify()
                .is_err());

            let wrong_tier = dart_obs(&[(tips, 90), (FEATHER_ID, 90), (wrong, 10)], 10_018);
            let wrong_further = dart_obs(&[(tips, 80), (FEATHER_ID, 80), (wrong, 20)], 10_036);
            assert!(witness(case, &baseline, [&wrong_tier, &wrong_further])
                .qualify()
                .is_err());

            let no_tips = dart_obs(&[(tips, 100), (FEATHER_ID, 90), (product, 10)], 10_018);
            let no_tips_further = dart_obs(&[(tips, 100), (FEATHER_ID, 80), (product, 20)], 10_036);
            assert!(witness(case, &baseline, [&no_tips, &no_tips_further])
                .qualify()
                .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(product, 10);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn herb_cleaner_requires_deposit_restock_and_exact_unid_ids() {
        for (case, named) in [
            (CoreCase::HerbCleaner, false),
            (CoreCase::HerbCleanerNamed, true),
        ] {
            let mut baseline = herb_obs(&[], &[(UNIDENTIFIED_GUAM_ID, 30)], 1_000);
            if named {
                baseline.bank_ids.insert(UNIDENTIFIED_MARENTILL_ID, 4);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let first = herb_obs(&[(GUAM_LEAF_ID, 28)], &[], 1_070);
            let mut deposited = herb_obs(
                &[],
                &[
                    (GUAM_LEAF_ID, 28),
                    (UNIDENTIFIED_GUAM_ID, 2),
                    (UNIDENTIFIED_MARENTILL_ID, if named { 4 } else { 0 }),
                ],
                1_070,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = herb_obs(
                &[(UNIDENTIFIED_GUAM_ID, 2)],
                &[
                    (GUAM_LEAF_ID, 28),
                    (UNIDENTIFIED_MARENTILL_ID, if named { 4 } else { 0 }),
                ],
                1_070,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let cleaned = herb_obs(&[(GUAM_LEAF_ID, 2)], &[], 1_075);

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cleaned])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Guam leaf".into(), 28);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &cleaned]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(
                witness(case, &baseline, [&first, &stale, &withdrawn, &cleaned])
                    .qualify()
                    .is_err()
            );
            let mut closed_deposit = deposited.clone();
            closed_deposit.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&first, &closed_deposit, &withdrawn, &cleaned]
            )
            .qualify()
            .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cleaned])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            if named {
                let mut took_filter = withdrawn.clone();
                took_filter.item_ids.insert(UNIDENTIFIED_MARENTILL_ID, 4);
                took_filter.bank_ids.insert(UNIDENTIFIED_MARENTILL_ID, 0);
                assert!(witness(
                    case,
                    &baseline,
                    [&first, &deposited, &took_filter, &cleaned]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(GUAM_LEAF_ID, 28);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn gem_cutter_requires_chisel_preservation_and_no_crush() {
        for (case, named) in [
            (CoreCase::GemCutter, false),
            (CoreCase::GemCutterNamed, true),
        ] {
            let mut baseline = gem_obs(&[], &[(CHISEL_ID, 1), (UNCUT_SAPPHIRE_ID, 28)], 20_000);
            if named {
                baseline.bank_ids.insert(UNCUT_OPAL_ID, 4);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let first = gem_obs(&[(CHISEL_ID, 1), (SAPPHIRE_ID, 27)], &[], 33_500);
            let mut deposited = gem_obs(
                &[(CHISEL_ID, 1)],
                &[
                    (SAPPHIRE_ID, 27),
                    (UNCUT_SAPPHIRE_ID, 1),
                    (UNCUT_OPAL_ID, if named { 4 } else { 0 }),
                ],
                33_500,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = gem_obs(
                &[(CHISEL_ID, 1), (UNCUT_SAPPHIRE_ID, 1)],
                &[
                    (SAPPHIRE_ID, 27),
                    (UNCUT_OPAL_ID, if named { 4 } else { 0 }),
                ],
                33_500,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let cut = gem_obs(&[(CHISEL_ID, 1), (SAPPHIRE_ID, 1)], &[], 34_000);

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Sapphire".into(), 27);
            name_only.items.insert("Chisel".into(), 1);
            assert!(
                witness(case, &baseline, [&name_only, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let crushed = gem_obs(
                &[(CHISEL_ID, 1), (SAPPHIRE_ID, 27), (CRUSHED_GEMSTONE_ID, 1)],
                &[],
                33_500,
            );
            assert!(
                witness(case, &baseline, [&crushed, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let mut no_chisel = first.clone();
            no_chisel.item_ids.remove(&CHISEL_ID);
            assert!(
                witness(case, &baseline, [&no_chisel, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(case, &baseline, [&first, &stale, &withdrawn, &cut])
                .qualify()
                .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &cut])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            if named {
                let mut took_opal = withdrawn.clone();
                took_opal.item_ids.insert(UNCUT_OPAL_ID, 4);
                took_opal.bank_ids.insert(UNCUT_OPAL_ID, 0);
                assert!(
                    witness(case, &baseline, [&first, &deposited, &took_opal, &cut])
                        .qualify()
                        .is_err()
                );
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(SAPPHIRE_ID, 27);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn bounded_loc(id: i32, tile: (i32, i32, i32), name: &str, open: bool) -> BoundedLoc {
        BoundedLoc {
            id,
            x: tile.0,
            z: tile.1,
            level: tile.2,
            name: Some(name.into()),
            open,
        }
    }

    fn door_obs(tile: (i32, i32, i32), locs: Vec<BoundedLoc>) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.loc_facts = locs;
        observation
    }

    fn gnome_obs(tile: (i32, i32, i32), agility_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("agility", agility_xp)], &[]);
        observation.tile = Some(tile);
        observation
    }

    #[test]
    fn wildy_agility_is_a_registered_catalog_core_case() {
        assert!(CoreCase::parse("wildy_agility").is_ok());
    }

    #[test]
    fn brimhaven_agility_is_a_registered_catalog_core_case() {
        assert!(CoreCase::parse("brimhaven_agility").is_ok());
    }

    fn wildy_obs(tile: (i32, i32, i32), agility_xp: i32) -> Observation {
        let mut observation = observation(&[], &[("agility", agility_xp)], &[]);
        observation.tile = Some(tile);
        observation
    }

    #[test]
    fn wildy_agility_requires_the_ordered_lap_bonus_and_second_pipe() {
        let mut baseline = wildy_obs(WILDY_START, 1_000);
        baseline.levels.insert("agility".into(), 52);
        baseline.item_ids.insert(LOBSTER_ID, 5);
        validate_case_baseline(CoreCase::WildyAgility, &baseline).unwrap();
        let mut under_level = baseline.clone();
        under_level.levels.insert("agility".into(), 51);
        assert!(validate_case_baseline(CoreCase::WildyAgility, &under_level).is_err());
        let mut under_food = baseline.clone();
        under_food.item_ids.insert(LOBSTER_ID, 4);
        assert!(validate_case_baseline(CoreCase::WildyAgility, &under_food).is_err());
        let mut low_base_hp = baseline.clone();
        low_base_hp.levels.insert("hitpoints".into(), 39);
        assert!(validate_case_baseline(CoreCase::WildyAgility, &low_base_hp).is_err());
        let mut low_effective_hp = baseline.clone();
        low_effective_hp
            .effective_levels
            .insert("hitpoints".into(), 39);
        assert!(validate_case_baseline(CoreCase::WildyAgility, &low_effective_hp).is_err());

        let ridge = wildy_obs((2998, 3933, 0), 1_015);
        let pipe = wildy_obs((3004, 3947, 0), 1_027);
        let rope = wildy_obs((3005, 3958, 0), 1_047);
        let stone = wildy_obs((2996, 3960, 0), 1_067);
        let log = wildy_obs((2994, 3945, 0), 1_087);
        let raw_scenery_plane_log = wildy_obs((2994, 3945, 1), 1_087);
        let rocks = wildy_obs((2994, 3933, 0), 1_586);
        let second_pipe = wildy_obs((3004, 3947, 0), 1_598);

        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [&ridge, &pipe, &rope, &stone, &log, &rocks, &second_pipe]
        )
        .qualify()
        .is_ok());
        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [
                &ridge,
                &pipe,
                &rope,
                &stone,
                &raw_scenery_plane_log,
                &rocks,
                &second_pipe,
            ]
        )
        .qualify()
        .is_err());
        assert!(witness(CoreCase::WildyAgility, &baseline, [&ridge])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [&ridge, &pipe, &rope, &stone, &log, &rocks]
        )
        .qualify()
        .is_err());

        let only_571 = wildy_obs((3004, 3947, 0), 1_571);
        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [&ridge, &pipe, &rope, &stone, &log, &rocks, &only_571]
        )
        .qualify()
        .is_err());

        let queued = wildy_obs((3004, 3947, 0), 1_598);
        assert!(witness(CoreCase::WildyAgility, &baseline, [&queued])
            .qualify()
            .is_err());

        let high_z_pit = wildy_obs((2998, 9933, 0), 1_015);
        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [
                &high_z_pit,
                &pipe,
                &rope,
                &stone,
                &log,
                &rocks,
                &second_pipe,
            ]
        )
        .qualify()
        .is_err());

        let wrong_side = wildy_obs((3004, 3937, 0), 1_027);
        assert!(witness(
            CoreCase::WildyAgility,
            &baseline,
            [
                &ridge,
                &wrong_side,
                &rope,
                &stone,
                &log,
                &rocks,
                &second_pipe
            ]
        )
        .qualify()
        .is_err());

        let mut wolf_pit = wildy_obs((2998, 3933, 0), 1_015);
        wolf_pit.chat = vec![(
            1,
            "You lose your footing and fall into the wolf pit.".into(),
        )];
        assert!(witness(CoreCase::WildyAgility, &baseline, [&wolf_pit])
            .qualify()
            .is_err());
    }

    fn brimhaven_obs(
        tile: (i32, i32, i32),
        agility_xp: i32,
        coins: i32,
        tickets: i32,
        varp: i32,
        chat: &[(i32, &str)],
    ) -> Observation {
        let mut observation = observation(&[], &[("agility", agility_xp)], chat);
        observation.tile = Some(tile);
        observation.item_ids = [
            (COINS_ID, coins),
            (LOBSTER_ID, 10),
            (BRIMHAVEN_TICKET_ID, tickets),
        ]
        .into_iter()
        .collect();
        observation.levels.insert("agility".into(), 52);
        observation.effective_levels.insert("agility".into(), 52);
        observation.varps.insert(BRIMHAVEN_ARENA_VARP, varp);
        observation
    }

    #[test]
    fn brimhaven_agility_requires_fee_movement_tag_ticket_and_subsequent_work() {
        let baseline = brimhaven_obs(
            BRIMHAVEN_START,
            1_000,
            1_000,
            0,
            0,
            &[(4, "Tag the next pillar")],
        );
        validate_case_baseline(CoreCase::BrimhavenAgility, &baseline).unwrap();
        let mut pre_paid = baseline.clone();
        pre_paid.varps.insert(BRIMHAVEN_ARENA_VARP, 2);
        assert!(validate_case_baseline(CoreCase::BrimhavenAgility, &pre_paid).is_err());
        let mut seeded_ticket = baseline.clone();
        seeded_ticket.item_ids.insert(BRIMHAVEN_TICKET_ID, 1);
        assert!(validate_case_baseline(CoreCase::BrimhavenAgility, &seeded_ticket).is_err());
        let mut underfunded = baseline.clone();
        underfunded.item_ids.insert(COINS_ID, 199);
        assert!(validate_case_baseline(CoreCase::BrimhavenAgility, &underfunded).is_err());
        let mut underqualified = baseline.clone();
        underqualified.levels.insert("agility".into(), 51);
        let error = validate_case_baseline(CoreCase::BrimhavenAgility, &underqualified)
            .expect_err("Agility 51 must not qualify Brimhaven");
        assert!(
            error.contains("surface entrance (2809,3194,0), Agility 52"),
            "{error}"
        );
        let mut drained = baseline.clone();
        drained.effective_levels.insert("agility".into(), 51);
        assert!(validate_case_baseline(CoreCase::BrimhavenAgility, &drained).is_err());

        let paid = brimhaven_obs(BRIMHAVEN_START, 1_000, 800, 0, 2, &[]);
        let entered = brimhaven_obs((2805, 9590, 3), 1_000, 800, 0, 2, &[]);
        let moved = brimhaven_obs((2805, 9579, 3), 1_006, 800, 0, 2, &[]);
        let first_tag = brimhaven_obs(
            (2805, 9579, 3),
            1_006,
            800,
            0,
            15,
            &[(5, "Tag the next pillar")],
        );
        let ticket = brimhaven_obs((2794, 9579, 3), 1_012, 800, 1, 7, &[]);
        let subsequent = brimhaven_obs((2783, 9579, 3), 1_018, 800, 1, 6, &[]);

        assert!(witness(
            CoreCase::BrimhavenAgility,
            &baseline,
            [&paid, &entered, &moved, &first_tag, &ticket, &subsequent]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::BrimhavenAgility, &baseline, [&paid])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::BrimhavenAgility, &baseline, [&paid, &entered])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::BrimhavenAgility,
            &baseline,
            [&paid, &entered, &moved, &first_tag]
        )
        .qualify()
        .is_err());
        assert!(witness(
            CoreCase::BrimhavenAgility,
            &baseline,
            [&paid, &entered, &moved, &first_tag, &ticket]
        )
        .qualify()
        .is_err());

        let stale_tag = brimhaven_obs(
            (2805, 9579, 3),
            1_006,
            800,
            0,
            15,
            &[(4, "Tag the next pillar")],
        );
        assert!(witness(
            CoreCase::BrimhavenAgility,
            &baseline,
            [&paid, &entered, &moved, &stale_tag, &ticket, &subsequent]
        )
        .qualify()
        .is_err());

        let wrong_hint = brimhaven_obs(
            (2805, 9579, 3),
            1_006,
            800,
            0,
            15,
            &[(
                5,
                "You can only get a ticket when the flashing arrow is above the pillar.",
            )],
        );
        assert!(witness(
            CoreCase::BrimhavenAgility,
            &baseline,
            [&paid, &entered, &moved, &wrong_hint, &ticket, &subsequent]
        )
        .qualify()
        .is_err());
    }

    fn flax_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn door_opener_requires_selected_world_change_not_queued_open() {
        for (case, stand, packed, closed, open, name) in [
            (
                CoreCase::DoorOpener,
                LUMBRIDGE_DOOR_STAND,
                LUMBRIDGE_DOOR,
                WOODEN_DOOR_CLOSED_ID,
                WOODEN_DOOR_OPEN_ID,
                "Door",
            ),
            (
                CoreCase::DoorOpenerGate,
                LUMBRIDGE_GATE_STAND,
                LUMBRIDGE_GATE,
                WOODEN_GATE_CLOSED_ID,
                WOODEN_GATE_OPEN_ID,
                "Gate",
            ),
        ] {
            let shut = bounded_loc(closed, packed, name, true);
            let baseline = door_obs(stand, vec![shut.clone()]);
            validate_case_baseline(case, &baseline).unwrap();

            let opened = door_obs(stand, vec![bounded_loc(open, packed, name, false)]);
            assert!(witness(case, &baseline, [&opened]).qualify().is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());

            let queued = door_obs(stand, vec![shut.clone()]);
            assert!(witness(case, &baseline, [&queued]).qualify().is_err());

            let unrelated = door_obs(
                stand,
                vec![
                    shut.clone(),
                    bounded_loc(open, (3100, 3100, 0), name, false),
                ],
            );
            assert!(witness(case, &baseline, [&unrelated]).qualify().is_err());

            let mut already_open = baseline.clone();
            already_open.loc_facts = vec![bounded_loc(open, packed, name, false)];
            assert!(validate_case_baseline(case, &already_open).is_err());
        }
    }

    #[test]
    fn gnome_course_requires_complete_lap_and_second_lap_progress() {
        for case in [CoreCase::GnomeCourse, CoreCase::GnomeCourseRadius] {
            let baseline = gnome_obs(GNOME_START, 0);
            validate_case_baseline(case, &baseline).unwrap();

            let log = gnome_obs(GNOME_AFTER_LOG, 7);
            let ground = gnome_obs(GNOME_GROUND_RETURN, 27);
            let pipe = gnome_obs(GNOME_PIPE, 32);
            let first_lap_return = gnome_obs((2482, 3437, 0), 86);
            let queued_next_log = gnome_obs(GNOME_AFTER_LOG, 86);
            let second = gnome_obs(GNOME_AFTER_LOG, 94);
            assert!(witness(case, &baseline, [&log, &ground, &pipe, &second])
                .qualify()
                .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&log]).qualify().is_err());
            assert!(witness(case, &baseline, [&log, &log]).qualify().is_err());
            assert!(witness(case, &baseline, [&log, &ground, &pipe])
                .qualify()
                .is_err());
            for incomplete in [&first_lap_return, &queued_next_log] {
                assert!(witness(case, &baseline, [&log, &ground, &pipe, incomplete])
                    .qualify()
                    .is_err());
            }
        }
    }

    #[test]
    fn flax_picker_requires_full_pack_deposit_return_and_further_pick() {
        let baseline = flax_obs(FLAX_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::FlaxPicker, &baseline).unwrap();

        let first = flax_obs(FLAX_FIELD, &[(FLAX_ID, 28)], &[]);
        let mut deposited = flax_obs(FLAX_FIELD, &[], &[(FLAX_ID, 28)]);
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = flax_obs(FLAX_FIELD, &[], &[]);
        returned.bank_generation = 2;
        let mut further = flax_obs(FLAX_FIELD, &[(FLAX_ID, 1)], &[]);
        further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::FlaxPicker, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::FlaxPicker, &baseline, [&first])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::FlaxPicker, &baseline, [&first, &deposited])
                .qualify()
                .is_err()
        );

        // A fabricated closed snapshot without a new modal generation is stale.
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::FlaxPicker,
            &baseline,
            [&first, &stale, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(FLAX_ID, 28);
        assert!(validate_case_baseline(CoreCase::FlaxPicker, &seeded).is_err());
    }

    fn superheater_obs(
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        equipment_ids: &[(i32, i32)],
        magic_xp: i32,
        smithing_xp: i32,
        smithing_level: i32,
    ) -> Observation {
        let mut observation =
            observation(&[], &[("magic", magic_xp), ("smithing", smithing_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("magic".into(), 43);
        observation.levels.insert("smithing".into(), smithing_level);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.equipment_ids = equipment_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn superheater_requires_exact_ids_staff_and_bank_cycle() {
        for (case, bar, primary, secondary, staff, steel, smithing) in [
            (
                CoreCase::Superheater,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                STAFF_OF_FIRE_ID,
                false,
                1,
            ),
            (
                CoreCase::SuperheaterSteel,
                STEEL_BAR_ID,
                IRON_ORE_ID,
                COAL_ID,
                STAFF_OF_FIRE_ID,
                true,
                30,
            ),
            (
                CoreCase::SuperheaterFireBattlestaff,
                BRONZE_BAR_ID,
                COPPER_ORE_ID,
                TIN_ORE_ID,
                FIRE_BATTLESTAFF_ID,
                false,
                1,
            ),
        ] {
            let mut baseline = superheater_obs(&[], &[], &[], 10_000, 1_000, smithing);
            if case == CoreCase::SuperheaterFireBattlestaff {
                assert!(validate_case_baseline(case, &baseline).is_err());
                baseline.levels.insert("attack".into(), 30);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let primary_n = 9;
            let secondary_n = if steel { 18 } else { 9 };
            let first = superheater_obs(
                &[(bar, 9), (NATURE_RUNE_ID, 41)],
                &[],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            let mut deposited = superheater_obs(
                &[(NATURE_RUNE_ID, 41)],
                &[
                    (bar, 9),
                    (primary, 91),
                    (secondary, if steel { 182 } else { 91 }),
                ],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = superheater_obs(
                &[
                    (primary, primary_n),
                    (secondary, secondary_n),
                    (NATURE_RUNE_ID, 41),
                ],
                &[
                    (bar, 9),
                    (primary, 91 - primary_n),
                    (secondary, if steel { 182 - 18 } else { 91 - 9 }),
                ],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let further = superheater_obs(
                &[
                    (bar, 1),
                    (primary, primary_n - 1),
                    (secondary, secondary_n - if steel { 2 } else { 1 }),
                    (NATURE_RUNE_ID, 40),
                ],
                &[],
                &[(staff, 1)],
                10_106,
                1_062,
                smithing,
            );

            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &further])
                    .qualify()
                    .is_ok()
            );
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&first]).qualify().is_err());
            assert!(witness(case, &baseline, [&first, &deposited, &withdrawn])
                .qualify()
                .is_err());

            let xp_only = superheater_obs(
                &[(NATURE_RUNE_ID, 50)],
                &[],
                &[(staff, 1)],
                10_053,
                1_056,
                smithing,
            );
            assert!(witness(
                case,
                &baseline,
                [&xp_only, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut name_only = first.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Bronze bar".into(), 9);
            name_only.items.insert("Nature rune".into(), 41);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(
                witness(case, &baseline, [&first, &stale, &withdrawn, &further])
                    .qualify()
                    .is_err()
            );
            let mut closed_deposit = deposited.clone();
            closed_deposit.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&first, &closed_deposit, &withdrawn, &further]
            )
            .qualify()
            .is_err());
            withdrawn.bank_generation = 2;
            assert!(
                witness(case, &baseline, [&first, &deposited, &withdrawn, &further])
                    .qualify()
                    .is_err()
            );
            withdrawn.bank_generation = 1;

            let mut no_staff = first.clone();
            no_staff.equipment_ids.clear();
            assert!(witness(
                case,
                &baseline,
                [&no_staff, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            if steel {
                let mut one_coal = withdrawn.clone();
                one_coal.item_ids.insert(COAL_ID, 9);
                assert!(
                    witness(case, &baseline, [&first, &deposited, &one_coal, &further])
                        .qualify()
                        .is_err()
                );
                let mut iron_bar = first.clone();
                iron_bar.item_ids.insert(IRON_BAR_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&iron_bar, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            } else {
                let mut steel_bar = first.clone();
                steel_bar.item_ids.insert(STEEL_BAR_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&steel_bar, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            }

            if case == CoreCase::SuperheaterFireBattlestaff {
                let mut default_staff = first.clone();
                default_staff.equipment_ids.clear();
                default_staff.equipment_ids.insert(STAFF_OF_FIRE_ID, 1);
                assert!(witness(
                    case,
                    &baseline,
                    [&default_staff, &deposited, &withdrawn, &further]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(bar, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn vial_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn vial_filler_requires_fountain_fill_deposit_restock_and_further_fill() {
        for (case, bank) in [
            (CoreCase::VialFiller, FALADOR_WEST_BANK),
            (CoreCase::VialFillerEast, FALADOR_EAST_BANK),
        ] {
            let baseline = vial_obs(bank, &[], &[]);
            validate_case_baseline(case, &baseline).unwrap();

            let filled = vial_obs(FALADOR_FOUNTAIN, &[(VIAL_OF_WATER_ID, 28)], &[]);
            let mut deposited = vial_obs(bank, &[], &[(VIAL_OF_WATER_ID, 28), (EMPTY_VIAL_ID, 28)]);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = vial_obs(bank, &[(EMPTY_VIAL_ID, 28)], &[(VIAL_OF_WATER_ID, 28)]);
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let mut returned = vial_obs(FALADOR_FOUNTAIN, &[(EMPTY_VIAL_ID, 28)], &[]);
            returned.bank_generation = 2;
            let mut further = vial_obs(FALADOR_FOUNTAIN, &[(VIAL_OF_WATER_ID, 1)], &[]);
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&filled, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&filled]).qualify().is_err());
            assert!(witness(
                case,
                &baseline,
                [&filled, &deposited, &withdrawn, &returned]
            )
            .qualify()
            .is_err());

            // A fabricated closed snapshot without a new modal generation is stale.
            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &filled,
                    &deposited,
                    &withdrawn,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = filled.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Vial of water".into(), 28);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&filled, &stale, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut not_fountain = filled.clone();
            not_fountain.tile = Some(bank);
            assert!(witness(
                case,
                &baseline,
                [&not_fountain, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(VIAL_OF_WATER_ID, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn potion_obs(
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        herblore_xp: i32,
        herblore_level: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("herblore", herblore_xp)], &[]);
        observation.tile = Some((3185, 3440, 0));
        observation.levels.insert("herblore".into(), herblore_level);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation
    }

    #[test]
    fn potion_maker_requires_staged_unf_finished_deposit_and_further_product() {
        for (case, herb, unf, secondary, finished, wrong_unf, wrong_finished, named, level) in [
            (
                CoreCase::PotionMaker,
                GUAM_LEAF_ID,
                GUAM_UNF_ID,
                EYE_OF_NEWT_ID,
                ATTACK_POTION_3_ID,
                RANARR_UNF_ID,
                PRAYER_POTION_3_ID,
                false,
                3,
            ),
            (
                CoreCase::PotionMakerNamed,
                RANARR_WEED_ID,
                RANARR_UNF_ID,
                SNAPE_GRASS_ID,
                PRAYER_POTION_3_ID,
                GUAM_UNF_ID,
                ATTACK_POTION_3_ID,
                true,
                38,
            ),
        ] {
            let mut baseline = potion_obs(&[], &[], 1_000, level);
            if named {
                baseline.bank_ids.insert(GUAM_LEAF_ID, 14);
            }
            validate_case_baseline(case, &baseline).unwrap();

            let unfinished = potion_obs(&[(unf, 14)], &[], 1_000, level);
            let made = potion_obs(&[(finished, 14)], &[], 1_025, level);
            let mut deposited_bank = vec![
                (finished, 14),
                (herb, 28),
                (VIAL_OF_WATER_ID, 28),
                (secondary, 28),
            ];
            if named {
                deposited_bank.push((GUAM_LEAF_ID, 14));
            }
            let mut deposited = potion_obs(&[], &deposited_bank, 1_025, level);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn_bank = vec![
                (finished, 14),
                (herb, 14),
                (VIAL_OF_WATER_ID, 14),
                (secondary, 28),
            ];
            if named {
                withdrawn_bank.push((GUAM_LEAF_ID, 14));
            }
            let mut withdrawn = potion_obs(
                &[(herb, 14), (VIAL_OF_WATER_ID, 14)],
                &withdrawn_bank,
                1_025,
                level,
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let further = potion_obs(&[(unf, 14), (VIAL_OF_WATER_ID, 0)], &[], 1_025, level);

            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&unfinished]).qualify().is_err());
            assert!(witness(case, &baseline, [&unfinished, &made])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &deposited, &withdrawn]
            )
            .qualify()
            .is_err());

            let mut name_only = unfinished.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Unfinished potion".into(), 14);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &made, &stale, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            let mut wrong = made.clone();
            wrong.item_ids.insert(wrong_finished, 1);
            assert!(witness(
                case,
                &baseline,
                [&unfinished, &wrong, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());
            let mut wrong_unfinished = unfinished.clone();
            wrong_unfinished.item_ids.insert(wrong_unf, 1);
            assert!(witness(
                case,
                &baseline,
                [&wrong_unfinished, &made, &deposited, &withdrawn, &further]
            )
            .qualify()
            .is_err());

            if named {
                let mut took_guam = withdrawn.clone();
                took_guam.item_ids.insert(GUAM_LEAF_ID, 14);
                took_guam.bank_ids.insert(GUAM_LEAF_ID, 0);
                assert!(witness(
                    case,
                    &baseline,
                    [&unfinished, &made, &deposited, &took_guam, &further]
                )
                .qualify()
                .is_err());
            }

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(finished, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
            let mut seeded_unf = baseline.clone();
            seeded_unf.item_ids.insert(unf, 1);
            assert!(validate_case_baseline(case, &seeded_unf).is_err());
        }
    }

    fn tanner_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        main_modal: i32,
        widgets: &[i32],
    ) -> Observation {
        let mut observation = observation(&[], &[], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.main_modal = main_modal;
        observation.widget_ids = widgets.iter().copied().collect();
        observation
    }

    #[test]
    fn tanner_bot_requires_widget_conversion_deposit_restock_and_further_tan() {
        for (case, product, wrong, tan_all) in [
            (
                CoreCase::TannerBot,
                SOFT_LEATHER_ID,
                HARD_LEATHER_ID,
                SOFT_TAN_ALL_COM,
            ),
            (
                CoreCase::TannerBotHard,
                HARD_LEATHER_ID,
                SOFT_LEATHER_ID,
                HARD_TAN_ALL_COM,
            ),
        ] {
            let baseline = tanner_obs(AL_KHARID_BANK, &[], &[], -1, &[]);
            validate_case_baseline(case, &baseline).unwrap();

            let widget = tanner_obs(
                TANNER_STAND,
                &[(COW_HIDE_ID, 27), (COINS_ID, 2000)],
                &[],
                TANNER_IF,
                &[tan_all],
            );
            let tanned = tanner_obs(
                TANNER_STAND,
                &[(product, 27), (COINS_ID, 1973)],
                &[],
                TANNER_IF,
                &[tan_all],
            );
            let mut deposited = tanner_obs(
                AL_KHARID_BANK,
                &[(COINS_ID, 1973)],
                &[(product, 27), (COW_HIDE_ID, 1), (COINS_ID, 3000)],
                -1,
                &[],
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut withdrawn = tanner_obs(
                AL_KHARID_BANK,
                &[(COW_HIDE_ID, 1), (COINS_ID, 1973)],
                &[(product, 27), (COINS_ID, 3000)],
                -1,
                &[],
            );
            withdrawn.bank_open = true;
            withdrawn.bank_loaded = true;
            withdrawn.bank_generation = 1;
            let mut returned = tanner_obs(
                TANNER_STAND,
                &[(COW_HIDE_ID, 1), (COINS_ID, 1973)],
                &[],
                -1,
                &[],
            );
            returned.bank_generation = 2;
            let mut further = tanner_obs(
                TANNER_STAND,
                &[(product, 1), (COINS_ID, 1972)],
                &[],
                -1,
                &[],
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&widget]).qualify().is_err());
            assert!(witness(case, &baseline, [&widget, &tanned])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &deposited, &withdrawn, &returned]
            )
            .qualify()
            .is_err());

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &widget,
                    &tanned,
                    &deposited,
                    &withdrawn,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = tanned.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Leather".into(), 27);
            assert!(witness(
                case,
                &baseline,
                [&widget, &name_only, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&widget, &tanned, &stale, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut coins_only = tanned.clone();
            coins_only.item_ids.remove(&product);
            assert!(witness(
                case,
                &baseline,
                [
                    &widget,
                    &coins_only,
                    &deposited,
                    &withdrawn,
                    &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let queued = widget.clone();
            assert!(witness(
                case,
                &baseline,
                [&queued, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut shop = widget.clone();
            shop.tile = Some(DOMMIK_STAND);
            shop.main_modal = SHOPMAIN;
            shop.widget_ids.clear();
            let mut shop_tanned = tanned.clone();
            shop_tanned.tile = Some(DOMMIK_STAND);
            shop_tanned.main_modal = SHOPMAIN;
            assert!(witness(
                case,
                &baseline,
                [
                    &shop,
                    &shop_tanned,
                    &deposited,
                    &withdrawn,
                    &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut wrong_obs = tanned.clone();
            wrong_obs.item_ids.insert(wrong, 1);
            assert!(witness(
                case,
                &baseline,
                [&widget, &wrong_obs, &deposited, &withdrawn, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(product, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    fn runecraft_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        xp: i32,
        rc_level: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("runecraft", xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.levels.insert("runecraft".into(), rc_level);
        observation
    }

    #[test]
    fn rune_crafter_requires_temple_conversion_portal_deposit_restock_and_further_craft() {
        for (case, bank, ruins, altar, rune, wrong, talisman, rc_level) in [
            (
                CoreCase::RuneCrafter,
                FALADOR_EAST_BANK,
                RUNECRAFTER_AIR_RUINS,
                AIR_ALTAR,
                AIR_RUNE_ID,
                EARTH_RUNE_ID,
                AIR_TALISMAN_ID,
                1,
            ),
            (
                CoreCase::RuneCrafterEarth,
                VARROCK_EAST_BANK,
                RUNECRAFTER_EARTH_RUINS,
                EARTH_ALTAR,
                EARTH_RUNE_ID,
                AIR_RUNE_ID,
                EARTH_TALISMAN_ID,
                9,
            ),
            (
                CoreCase::MuleCrafter,
                FALADOR_EAST_BANK,
                MULECRAFTER_AIR_RUINS,
                AIR_ALTAR,
                AIR_RUNE_ID,
                EARTH_RUNE_ID,
                AIR_TALISMAN_ID,
                1,
            ),
        ] {
            let baseline = runecraft_obs(bank, &[], &[], 0, rc_level);
            validate_case_baseline(case, &baseline).unwrap();

            let withdrawn = runecraft_obs(
                bank,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                0,
                rc_level,
            );
            let entered = runecraft_obs(
                altar,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                0,
                rc_level,
            );
            let crafted = runecraft_obs(altar, &[(rune, 27), (talisman, 1)], &[], 5, rc_level);
            let exited = runecraft_obs(ruins, &[(rune, 27), (talisman, 1)], &[], 5, rc_level);
            let mut deposited = runecraft_obs(
                bank,
                &[(talisman, 1)],
                &[(rune, 27), (RUNE_ESSENCE_ID, 173)],
                5,
                rc_level,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut restocked = runecraft_obs(
                bank,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[(rune, 27), (RUNE_ESSENCE_ID, 146)],
                5,
                rc_level,
            );
            restocked.bank_open = true;
            restocked.bank_loaded = true;
            restocked.bank_generation = 1;
            let mut returned = runecraft_obs(
                ruins,
                &[(RUNE_ESSENCE_ID, 27), (talisman, 1)],
                &[],
                5,
                rc_level,
            );
            returned.bank_generation = 2;
            let mut further = runecraft_obs(altar, &[(rune, 27), (talisman, 1)], &[], 10, rc_level);
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &crafted, &exited, &deposited, &restocked, &returned,
                    &further,
                ]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn]).qualify().is_err());
            assert!(witness(case, &baseline, [&withdrawn, &entered])
                .qualify()
                .is_err());
            assert!(witness(case, &baseline, [&withdrawn, &entered, &crafted])
                .qualify()
                .is_err());
            assert!(
                witness(case, &baseline, [&withdrawn, &entered, &crafted, &exited])
                    .qualify()
                    .is_err()
            );
            assert!(witness(
                case,
                &baseline,
                [&withdrawn, &entered, &crafted, &exited, &deposited, &restocked, &returned]
            )
            .qualify()
            .is_err());

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn,
                    &entered,
                    &crafted,
                    &exited,
                    &deposited,
                    &restocked,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = crafted.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Air rune".into(), 27);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &name_only, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &crafted, &exited, &stale, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut xp_only = crafted.clone();
            xp_only.item_ids.remove(&rune);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &xp_only, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let queued = entered.clone();
            assert!(witness(
                case,
                &baseline,
                [&withdrawn, &queued, &exited, &deposited, &restocked, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut wrong_obs = crafted.clone();
            wrong_obs.item_ids.insert(wrong, 1);
            assert!(witness(
                case,
                &baseline,
                [
                    &withdrawn, &entered, &wrong_obs, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut noted = withdrawn.clone();
            noted.item_ids.insert(NOTED_ESSENCE_ID, 27);
            noted.item_ids.remove(&RUNE_ESSENCE_ID);
            assert!(witness(
                case,
                &baseline,
                [
                    &noted, &entered, &crafted, &exited, &deposited, &restocked, &returned,
                    &further
                ]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(rune, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }
    }

    #[test]
    fn catalog_failure_emits_accumulated_core_witness_and_latest_observation() {
        assert_eq!(
            accumulated_core(None),
            json!({"error": "no Start baseline"})
        );

        let baseline = runecraft_obs(VARROCK_EAST_BANK, &[], &[], 0, 9);
        let withdrawn = runecraft_obs(
            VARROCK_EAST_BANK,
            &[(RUNE_ESSENCE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            0,
            9,
        );
        let entered = runecraft_obs(
            EARTH_ALTAR,
            &[(RUNE_ESSENCE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            0,
            9,
        );
        let crafted = runecraft_obs(
            EARTH_ALTAR,
            &[(EARTH_RUNE_ID, 27), (EARTH_TALISMAN_ID, 1)],
            &[],
            5,
            9,
        );
        let witness = witness(
            CoreCase::RuneCrafterEarth,
            &baseline,
            [&withdrawn, &entered, &crafted],
        );
        assert!(witness.qualify().is_err());
        let core = accumulated_core(Some(&witness));
        assert_eq!(
            core["error"],
            json!("rune_crafter_earth core post-Start delta incomplete")
        );
        assert!(core["witness"]["rune_crafter_cycle"]["crafted"].is_object());
        assert!(core["witness"]["rune_crafter_cycle"]["deposited"].is_null());
        assert_eq!(
            core["witness"]["rune_crafter_cycle"]["further"],
            json!(false)
        );
        assert_eq!(core["witness"]["latest"]["item_ids"]["557"], json!(27));
        assert_eq!(core["witness"]["latest"]["xp"]["runecraft"], json!(5));
        assert_eq!(core["witness"]["post_start_observations"], json!(3));
    }

    fn ardy_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        xp: i32,
        thieving: i32,
    ) -> Observation {
        let mut observation = observation(&[], &[("thieving", xp)], &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.levels.insert("thieving".into(), thieving);
        observation.levels.insert("hitpoints".into(), 40);
        observation
    }

    #[test]
    fn ardy_cakes_requires_stall_food_xp_fresh_deposit_return_and_further_steal() {
        let baseline = ardy_obs(
            ARDY_CAKES_STAND,
            &[(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES)],
            &[],
            0,
            5,
        );
        validate_case_baseline(CoreCase::ArdyCakes, &baseline).unwrap();

        let stolen = ardy_obs(
            ARDY_CAKES_STAND,
            &[
                (KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES),
                (CAKE_ID, 4),
                (BREAD_ID, 2),
            ],
            &[],
            64,
            5,
        );
        let mut deposited = ardy_obs(
            ARDY_BANK,
            &[],
            &[
                (KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES),
                (CAKE_ID, 4),
                (BREAD_ID, 2),
            ],
            64,
            5,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = ardy_obs(ARDY_CAKES_STAND, &[], &[], 64, 5);
        returned.bank_generation = 2;
        let mut further = ardy_obs(ARDY_CAKES_STAND, &[(CHOCOLATE_SLICE_ID, 1)], &[], 80, 5);
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::ArdyCakes, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::ArdyCakes, &baseline, [&stolen])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut name_only = stolen.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Cake".into(), 4);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = ardy_obs(ARDY_CAKES_STAND, &[], &[], 64, 5);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut wrong = stolen.clone();
        wrong.item_ids.insert(CHOCOLATE_CAKE_ID, 1);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&wrong, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = stolen.clone();
        noted.item_ids.insert(NOTED_CAKE_ID, 1);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(CAKE_ID, 1);
        assert!(validate_case_baseline(CoreCase::ArdyCakes, &seeded).is_err());
        let mut wrong_ballast = baseline.clone();
        wrong_ballast
            .item_ids
            .insert(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES - 1);
        assert!(validate_case_baseline(CoreCase::ArdyCakes, &wrong_ballast).is_err());
        let mut missing_banked_ballast = deposited.clone();
        missing_banked_ballast.bank_ids.remove(&KNIFE_ID);
        assert!(witness(
            CoreCase::ArdyCakes,
            &baseline,
            [&stolen, &missing_banked_ballast, &returned, &further]
        )
        .qualify()
        .is_err());
        let market = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 5);
        assert!(validate_case_baseline(CoreCase::ArdyCakes, &market).is_err());
    }

    #[test]
    fn ardy_thiever_requires_coins_xp_fresh_deposit_return_and_further_pickpocket() {
        for (case, thieving) in [
            (CoreCase::ArdyThiever, 40),
            (CoreCase::ArdyThieverKnight, 55),
        ] {
            let baseline = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, thieving);
            validate_case_baseline(case, &baseline).unwrap();

            let cakes_only = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 16, thieving);
            let pickpocketed = ardy_obs(
                ARDY_THIEVER_STAND,
                &[(COINS_ID, 30), (CAKE_ID, 1)],
                &[],
                484,
                thieving,
            );
            let mut deposited =
                ardy_obs(ARDY_BANK, &[(CAKE_ID, 1)], &[(COINS_ID, 30)], 484, thieving);
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut returned = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 484, thieving);
            returned.bank_generation = 2;
            let mut further = ardy_obs(
                ARDY_THIEVER_STAND,
                &[(COINS_ID, 30), (CAKE_ID, 1)],
                &[],
                952,
                thieving,
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &deposited, &returned, &further]
            )
            .qualify()
            .is_ok());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&cakes_only]).qualify().is_err());
            assert!(witness(case, &baseline, [&pickpocketed]).qualify().is_err());
            assert!(
                witness(case, &baseline, [&pickpocketed, &deposited, &returned])
                    .qualify()
                    .is_err()
            );

            let mut unclosed_return = returned.clone();
            unclosed_return.bank_generation = deposited.bank_generation;
            let mut unclosed_further = further.clone();
            unclosed_further.bank_generation = deposited.bank_generation;
            assert!(witness(
                case,
                &baseline,
                [
                    &pickpocketed,
                    &deposited,
                    &unclosed_return,
                    &unclosed_further
                ]
            )
            .qualify()
            .is_err());

            let mut name_only = pickpocketed.clone();
            name_only.item_ids.clear();
            name_only.items.insert("Coins".into(), 30);
            assert!(witness(
                case,
                &baseline,
                [&name_only, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut stale = deposited.clone();
            stale.bank_loaded = false;
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &stale, &returned, &further]
            )
            .qualify()
            .is_err());
            let mut closed = deposited.clone();
            closed.bank_open = false;
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &closed, &returned, &further]
            )
            .qualify()
            .is_err());

            let xp_only = ardy_obs(ARDY_THIEVER_STAND, &[(CAKE_ID, 1)], &[], 484, thieving);
            assert!(
                witness(case, &baseline, [&xp_only, &deposited, &returned, &further])
                    .qualify()
                    .is_err()
            );

            let mut far = ardy_obs((3185, 3440, 0), &[(CAKE_ID, 1)], &[], 484, thieving);
            far.bank_generation = 2;
            let mut further_far = further.clone();
            further_far.tile = Some((3185, 3440, 0));
            assert!(witness(
                case,
                &baseline,
                [&pickpocketed, &deposited, &far, &further_far]
            )
            .qualify()
            .is_err());

            let mut seeded = baseline.clone();
            seeded.item_ids.insert(COINS_ID, 1);
            assert!(validate_case_baseline(case, &seeded).is_err());
        }

        let guard_low = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 39);
        assert!(validate_case_baseline(CoreCase::ArdyThiever, &guard_low).is_err());
        let knight_low = ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 54);
        assert!(validate_case_baseline(CoreCase::ArdyThieverKnight, &knight_low).is_err());
        validate_case_baseline(
            CoreCase::ArdyThiever,
            &ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 40),
        )
        .unwrap();
        assert!(validate_case_baseline(
            CoreCase::ArdyThieverKnight,
            &ardy_obs(ARDY_THIEVER_STAND, &[], &[], 0, 40)
        )
        .is_err());
    }

    fn resource_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        equipment_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        levels: &[(&str, i32)],
    ) -> Observation {
        let mut observation = observation(&[], xp, &[]);
        observation.tile = Some(tile);
        observation.item_ids = item_ids.iter().copied().collect();
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.equipment_ids = equipment_ids.iter().copied().collect();
        observation.magic_tree_ready = tile == GNOME_SOUTH_BANK_MAGIC_STAND;
        for (name, level) in levels {
            observation.levels.insert((*name).into(), *level);
            observation.effective_levels.insert((*name).into(), *level);
        }
        observation
    }

    fn flax_loc(id: i32, x: i32, z: i32, actions: &[&str]) -> LocView {
        LocView {
            typecode: 0,
            info: 0,
            id,
            name: Some("Flax".into()),
            description: None,
            actions: actions
                .iter()
                .map(|action| Some((*action).into()))
                .collect(),
            tile: WorldTile { x, z, level: 0 },
            distance: 0,
            layer: api::snapshot::LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    #[test]
    fn flax_pick_failure_facts_include_all_scoped_locs_and_native_reachability() {
        let mut failure = resource_obs(SEERS_BANK, &[], &[(FLAX_ID, 28)], &[], &[], &[]);
        failure.bank_open = true;
        failure.bank_loaded = true;
        failure.bank_generation = 7;
        let scene = SceneView {
            available: true,
            base_x: 2710,
            base_z: 3430,
            level: 0,
            width: 64,
            height: 72,
            collision_flags: vec![0; 64 * 72],
        };
        let locs = [
            flax_loc(2646, 2741, 3444, &["Pick"]),
            flax_loc(2646, 2742, 3445, &["Examine", "Pick"]),
            flax_loc(2646, 2740, 3444, &["Examine"]),
            flax_loc(2646, 2754, 3444, &["Pick"]),
        ];

        let facts = flax_aio_pick_failure_facts(CoreCase::FlaxAioPick, &failure, &scene, &locs)
            .expect("FlaxAIO pick diagnostics");
        assert_eq!(facts.player_tile, Some(SEERS_BANK));
        assert_eq!(facts.field_center, FLAX_FIELD);
        assert_eq!(facts.field_scope, FLAX_FIELD_SCOPE);
        assert!(!facts.at_field);
        assert!(facts.bank_open);
        assert!(facts.bank_loaded);
        assert_eq!(facts.bank_generation, 7);
        assert!(facts.reachability_available);
        assert_eq!(
            facts.reachability_source,
            "api::query::SceneQuery::flood_reach().at(tile)"
        );
        assert_eq!(
            facts.adapter_query,
            "Reachability.canReach(tile,{adjacentOk:true,maxSteps:400})"
        );
        assert_eq!(facts.relevant_locs.len(), 2);
        assert!(facts.relevant_locs.iter().all(|loc| loc.reachable_adj));
        assert_eq!(facts.nearest_reachable, Some((2742, 3445, 0)));
        assert!(
            flax_aio_pick_failure_facts(CoreCase::FlaxAioSpin, &failure, &scene, &locs,).is_none()
        );
    }

    #[test]
    fn gnome_chop_requires_log_xp_upstairs_deposit_ground_return_and_further_chop() {
        let levels = [("woodcutting", 75), ("fletching", 1)];
        let baseline = resource_obs(
            GNOME_WEST_MAGICS,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        validate_case_baseline(CoreCase::GnomeChop, &baseline).unwrap();
        let mut missing_magic_tree = baseline.clone();
        missing_magic_tree.magic_tree_ready = false;
        assert!(validate_case_baseline(CoreCase::GnomeChop, &missing_magic_tree).is_err());

        let chopped = resource_obs(
            GNOME_WEST_MAGICS,
            &[
                (RUNE_AXE_ID, 1),
                (KNIFE_ID, GNOME_BALLAST_KNIVES),
                (MAGIC_LOGS_ID, 1),
            ],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        let mut deposited = resource_obs(
            GNOME_BANK_STAND,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[(MAGIC_LOGS_ID, 1)],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = resource_obs(
            GNOME_BANK_STAIR_SOUTH,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        returned.bank_generation = 2;
        let mut further = resource_obs(
            GNOME_WEST_MAGICS,
            &[
                (RUNE_AXE_ID, 1),
                (KNIFE_ID, GNOME_BALLAST_KNIVES),
                (MAGIC_LOGS_ID, 1),
            ],
            &[],
            &[],
            &[("woodcutting", 500)],
            &levels,
        );
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        let mut missing_tool = chopped.clone();
        missing_tool.item_ids.remove(&RUNE_AXE_ID);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&missing_tool, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut lost_ballast = chopped.clone();
        lost_ballast
            .item_ids
            .insert(KNIFE_ID, GNOME_BALLAST_KNIVES - 1);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&lost_ballast, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        assert!(witness(CoreCase::GnomeChop, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::GnomeChop, &baseline, [&chopped])
            .qualify()
            .is_err());
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &deposited, &unclosed_return, &unclosed_further]
        )
        .qualify()
        .is_err());

        let mut name_only = chopped.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Magic logs".into(), 4);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &stale, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&chopped, &closed, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            GNOME_WEST_MAGICS,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 250)],
            &levels,
        );
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut wrong = chopped.clone();
        wrong.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&wrong, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = chopped.clone();
        noted.item_ids.insert(NOTED_MAGIC_LOGS_ID, 1);
        assert!(witness(
            CoreCase::GnomeChop,
            &baseline,
            [&noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(MAGIC_LOGS_ID, 1);
        assert!(validate_case_baseline(CoreCase::GnomeChop, &seeded).is_err());
        let low = resource_obs(
            GNOME_WEST_MAGICS,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &[("woodcutting", 74), ("fletching", 1)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &low).is_err());
        let no_axe = resource_obs(
            GNOME_WEST_MAGICS,
            &[],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &no_axe).is_err());
        let wielded = resource_obs(
            GNOME_WEST_MAGICS,
            &[(KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[(RUNE_AXE_ID, 1)],
            &[("woodcutting", 0)],
            &levels,
        );
        validate_case_baseline(CoreCase::GnomeChop, &wielded).unwrap();
        let mut wrong_axe = baseline.clone();
        wrong_axe.item_ids.remove(&RUNE_AXE_ID);
        wrong_axe.item_ids.insert(STEEL_AXE_ID, 1);
        assert!(validate_case_baseline(CoreCase::GnomeChop, &wrong_axe).is_err());
        let mut too_few_knives = baseline.clone();
        too_few_knives
            .item_ids
            .insert(KNIFE_ID, GNOME_BALLAST_KNIVES - 1);
        assert!(validate_case_baseline(CoreCase::GnomeChop, &too_few_knives).is_err());
        let mut too_many_knives = baseline.clone();
        too_many_knives
            .item_ids
            .insert(KNIFE_ID, GNOME_BALLAST_KNIVES + 1);
        assert!(validate_case_baseline(CoreCase::GnomeChop, &too_many_knives).is_err());
        let flax = resource_obs(
            FLAX_FIELD,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::GnomeChop, &flax).is_err());
    }

    #[test]
    fn gnome_fletch_requires_unstrung_product_xp_deposit_return_and_further_chop() {
        for (case, fletching, product, other) in [
            (
                CoreCase::GnomeFletchShort,
                80,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
                UNSTRUNG_MAGIC_LONGBOW_ID,
            ),
            (
                CoreCase::GnomeFletchLong,
                85,
                UNSTRUNG_MAGIC_LONGBOW_ID,
                UNSTRUNG_MAGIC_SHORTBOW_ID,
            ),
        ] {
            let levels = [("woodcutting", 75), ("fletching", fletching)];
            let baseline = resource_obs(
                GNOME_WEST_MAGICS,
                &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
                &[],
                &[],
                &[("woodcutting", 0), ("fletching", 0)],
                &levels,
            );
            validate_case_baseline(case, &baseline).unwrap();

            let chopped = resource_obs(
                GNOME_WEST_MAGICS,
                &[
                    (RUNE_AXE_ID, 1),
                    (KNIFE_ID, GNOME_BALLAST_KNIVES),
                    (MAGIC_LOGS_ID, 1),
                ],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 0)],
                &levels,
            );
            let fletched = resource_obs(
                GNOME_WEST_MAGICS,
                &[
                    (RUNE_AXE_ID, 1),
                    (KNIFE_ID, GNOME_BALLAST_KNIVES),
                    (product, 1),
                ],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            let mut deposited = resource_obs(
                GNOME_BANK_STAND,
                &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
                &[(product, 1)],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            deposited.bank_open = true;
            deposited.bank_loaded = true;
            deposited.bank_generation = 1;
            let mut returned = resource_obs(
                GNOME_BANK_STAIR_SOUTH,
                &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
                &[],
                &[],
                &[("woodcutting", 250), ("fletching", 168)],
                &levels,
            );
            returned.bank_generation = 2;
            let mut further = resource_obs(
                GNOME_WEST_MAGICS,
                &[
                    (RUNE_AXE_ID, 1),
                    (KNIFE_ID, GNOME_BALLAST_KNIVES),
                    (MAGIC_LOGS_ID, 1),
                ],
                &[],
                &[],
                &[("woodcutting", 500), ("fletching", 168)],
                &levels,
            );
            further.bank_generation = 2;

            assert!(witness(
                case,
                &baseline,
                [&chopped, &fletched, &deposited, &returned, &further]
            )
            .qualify()
            .is_ok());
            let mut lost_ballast = fletched.clone();
            lost_ballast
                .item_ids
                .insert(KNIFE_ID, GNOME_BALLAST_KNIVES - 1);
            assert!(witness(
                case,
                &baseline,
                [&chopped, &lost_ballast, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());
            assert!(witness(case, &baseline, [&baseline]).qualify().is_err());
            assert!(witness(case, &baseline, [&chopped]).qualify().is_err());
            assert!(witness(case, &baseline, [&chopped, &fletched])
                .qualify()
                .is_err());
            assert!(witness(
                case,
                &baseline,
                [&chopped, &fletched, &deposited, &returned]
            )
            .qualify()
            .is_err());

            let mut strung = fletched.clone();
            strung.item_ids.insert(MAGIC_SHORTBOW_ID, 1);
            assert!(witness(
                case,
                &baseline,
                [&chopped, &strung, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut cross = fletched.clone();
            cross.item_ids.insert(other, 1);
            assert!(witness(
                case,
                &baseline,
                [&chopped, &cross, &deposited, &returned, &further]
            )
            .qualify()
            .is_err());

            let mut no_knife = baseline.clone();
            no_knife.item_ids.remove(&KNIFE_ID);
            assert!(validate_case_baseline(case, &no_knife).is_err());
            let mut wrong_knife_count = baseline.clone();
            wrong_knife_count
                .item_ids
                .insert(KNIFE_ID, GNOME_BALLAST_KNIVES - 1);
            assert!(validate_case_baseline(case, &wrong_knife_count).is_err());
        }

        let short_high = resource_obs(
            GNOME_WEST_MAGICS,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 0), ("fletching", 0)],
            &[("woodcutting", 75), ("fletching", 85)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeFletchShort, &short_high).is_err());
        validate_case_baseline(CoreCase::GnomeFletchLong, &short_high).unwrap();
        let long_low = resource_obs(
            GNOME_WEST_MAGICS,
            &[(RUNE_AXE_ID, 1), (KNIFE_ID, GNOME_BALLAST_KNIVES)],
            &[],
            &[],
            &[("woodcutting", 0), ("fletching", 0)],
            &[("woodcutting", 75), ("fletching", 84)],
        );
        assert!(validate_case_baseline(CoreCase::GnomeFletchLong, &long_low).is_err());
        validate_case_baseline(CoreCase::GnomeFletchShort, &long_low).unwrap();
    }

    #[test]
    fn coal_trucks_requires_mining_xp_truck_deposit_not_bank_and_further_mine() {
        let levels = [("mining", 60)];
        let mut baseline = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1), (KNIFE_ID, 26)],
            &[],
            &[],
            &[("mining", 0)],
            &levels,
        );
        baseline.combat_level = 55;
        validate_case_baseline(CoreCase::CoalTrucks, &baseline).unwrap();

        let mined = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1), (KNIFE_ID, 26), (COAL_ID, 1)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        let trucked = resource_obs(
            COAL_MINE_TRUCK_STAND,
            &[(STEEL_PICKAXE_ID, 1), (KNIFE_ID, 26)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        let further = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1), (KNIFE_ID, 26), (COAL_ID, 1)],
            &[],
            &[],
            &[("mining", 1400)],
            &levels,
        );

        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&mined, &trucked, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&mined])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CoalTrucks, &baseline, [&mined, &trucked])
            .qualify()
            .is_err());

        let mut name_only = mined.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Coal".into(), 27);
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&name_only, &trucked, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 1350)],
            &levels,
        );
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&xp_only, &trucked, &further]
        )
        .qualify()
        .is_err());

        let mut banked = trucked.clone();
        banked.bank_open = true;
        banked.bank_loaded = true;
        banked.bank_generation = 1;
        banked.bank_ids.insert(COAL_ID, 27);
        banked.tile = Some(SEERS_BANK);
        assert!(
            witness(CoreCase::CoalTrucks, &baseline, [&mined, &banked, &further])
                .qualify()
                .is_err()
        );

        let mut noted = mined.clone();
        noted.item_ids.insert(NOTED_COAL_ID, 1);
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&noted, &trucked, &further]
        )
        .qualify()
        .is_err());

        let mut lost_ballast = mined.clone();
        lost_ballast
            .item_ids
            .insert(KNIFE_ID, COAL_BALLAST_KNIVES - 1);
        assert!(witness(
            CoreCase::CoalTrucks,
            &baseline,
            [&lost_ballast, &trucked, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(COAL_ID, 1);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &seeded).is_err());
        let mut missing_ballast = baseline.clone();
        missing_ballast.item_ids.insert(KNIFE_ID, 25);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &missing_ballast).is_err());
        let mut excess_ballast = baseline.clone();
        excess_ballast.item_ids.insert(KNIFE_ID, 27);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &excess_ballast).is_err());
        let low = resource_obs(
            COAL_MINE,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 0)],
            &[("mining", 29)],
        );
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &low).is_err());
        let no_pick = resource_obs(COAL_MINE, &[], &[], &[], &[("mining", 0)], &levels);
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &no_pick).is_err());
        let wrong_stand = resource_obs(
            SEERS_BANK,
            &[(STEEL_PICKAXE_ID, 1)],
            &[],
            &[],
            &[("mining", 0)],
            &levels,
        );
        assert!(validate_case_baseline(CoreCase::CoalTrucks, &wrong_stand).is_err());
    }

    #[test]
    fn station_production_requires_xp_product_deposit_restock_return_and_further() {
        let cook_levels = [("cooking", COOKING_FIXTURE_LEVEL)];
        let baseline = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        validate_case_baseline(CoreCase::CookBot, &baseline).unwrap();

        let withdrawn = resource_obs(
            CATHERBY_BANK,
            &[(RAW_SALMON_ID, 28)],
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        let produced = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(SALMON_ID, 28)],
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        let mut deposited = resource_obs(
            CATHERBY_BANK,
            &[],
            &[(SALMON_ID, 28), (RAW_SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut restocked = resource_obs(
            CATHERBY_BANK,
            &[(RAW_SALMON_ID, 28)],
            &[(SALMON_ID, 28)],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        restocked.bank_open = true;
        restocked.bank_loaded = true;
        restocked.bank_generation = 1;
        let mut returned = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_SALMON_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        returned.bank_generation = 2;
        let mut further = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(SALMON_ID, 1)],
            &[],
            &[],
            &[("cooking", 500)],
            &cook_levels,
        );
        further.bank_generation = 2;

        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::CookBot, &baseline, [&baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::CookBot, &baseline, [&withdrawn])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::CookBot, &baseline, [&withdrawn, &produced])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &deposited, &restocked, &returned]
        )
        .qualify()
        .is_err());

        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [
                &withdrawn,
                &produced,
                &deposited,
                &restocked,
                &unclosed_return,
                &unclosed_further
            ]
        )
        .qualify()
        .is_err());

        let mut name_only = produced.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Salmon".into(), 28);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &name_only, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut stale = deposited.clone();
        stale.bank_loaded = false;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &stale, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut closed = deposited.clone();
        closed.bank_open = false;
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &produced, &closed, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let xp_only = resource_obs(
            CATHERBY_RANGE_STAND,
            &[],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &xp_only, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let no_consume = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_SALMON_ID, 28), (SALMON_ID, 1)],
            &[],
            &[],
            &[("cooking", 250)],
            &cook_levels,
        );
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [
                &withdrawn,
                &no_consume,
                &deposited,
                &restocked,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());

        let mut wrong = produced.clone();
        wrong.item_ids.insert(LOBSTER_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &wrong, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut noted = produced.clone();
        noted.item_ids.insert(NOTED_SALMON_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &noted, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut burnt = produced.clone();
        burnt.item_ids.insert(BURNT_FISH_2_ID, 1);
        assert!(witness(
            CoreCase::CookBot,
            &baseline,
            [&withdrawn, &burnt, &deposited, &restocked, &returned, &further]
        )
        .qualify()
        .is_err());

        let mut seeded = baseline.clone();
        seeded.item_ids.insert(SALMON_ID, 1);
        assert!(validate_case_baseline(CoreCase::CookBot, &seeded).is_err());
        let low = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &[("cooking", 79)],
        );
        assert!(validate_case_baseline(CoreCase::CookBot, &low).is_err());
        let wrong_stand = resource_obs(
            CATHERBY_RANGE_STAND,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &cook_levels,
        );
        assert!(validate_case_baseline(CoreCase::CookBot, &wrong_stand).is_err());

        let lobster_levels = [("cooking", COOKING_FIXTURE_LEVEL)];
        let lobster_baseline = resource_obs(
            CATHERBY_BANK,
            &[],
            &[],
            &[],
            &[("cooking", 0)],
            &lobster_levels,
        );
        validate_case_baseline(CoreCase::CookBotLobster, &lobster_baseline).unwrap();
        let lobster_withdrawn = resource_obs(
            CATHERBY_BANK,
            &[(RAW_LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 0)],
            &lobster_levels,
        );
        let lobster_produced = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        let mut lobster_deposited = resource_obs(
            CATHERBY_BANK,
            &[],
            &[(LOBSTER_ID, 28), (RAW_LOBSTER_ID, 28)],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_deposited.bank_open = true;
        lobster_deposited.bank_loaded = true;
        lobster_deposited.bank_generation = 1;
        let mut lobster_restocked = resource_obs(
            CATHERBY_BANK,
            &[(RAW_LOBSTER_ID, 28)],
            &[(LOBSTER_ID, 28)],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_restocked.bank_open = true;
        lobster_restocked.bank_loaded = true;
        lobster_restocked.bank_generation = 1;
        let mut lobster_returned = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(RAW_LOBSTER_ID, 28)],
            &[],
            &[],
            &[("cooking", 250)],
            &lobster_levels,
        );
        lobster_returned.bank_generation = 2;
        let mut lobster_further = resource_obs(
            CATHERBY_RANGE_STAND,
            &[(LOBSTER_ID, 1)],
            &[],
            &[],
            &[("cooking", 500)],
            &lobster_levels,
        );
        lobster_further.bank_generation = 2;
        assert!(witness(
            CoreCase::CookBotLobster,
            &lobster_baseline,
            [
                &lobster_withdrawn,
                &lobster_produced,
                &lobster_deposited,
                &lobster_restocked,
                &lobster_returned,
                &lobster_further
            ]
        )
        .qualify()
        .is_ok());
        let mut lobster_cross = lobster_produced.clone();
        lobster_cross.item_ids.insert(SALMON_ID, 1);
        assert!(witness(
            CoreCase::CookBotLobster,
            &lobster_baseline,
            [
                &lobster_withdrawn,
                &lobster_cross,
                &lobster_deposited,
                &lobster_restocked,
                &lobster_returned,
                &lobster_further
            ]
        )
        .qualify()
        .is_err());

        let bronze_levels = [("smithing", 1)];
        let bronze_baseline = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &bronze_levels,
        );
        validate_case_baseline(CoreCase::SmelterBot, &bronze_baseline).unwrap();
        let bronze_withdrawn = resource_obs(
            AL_KHARID_BANK,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[],
            &[],
            &[("smithing", 0)],
            &bronze_levels,
        );
        let bronze_produced = resource_obs(
            AL_KHARID_FURNACE,
            &[(BRONZE_BAR_ID, 14)],
            &[],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        let mut bronze_deposited = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[(BRONZE_BAR_ID, 14), (COPPER_ORE_ID, 42), (TIN_ORE_ID, 42)],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_deposited.bank_open = true;
        bronze_deposited.bank_loaded = true;
        bronze_deposited.bank_generation = 1;
        let mut bronze_restocked = resource_obs(
            AL_KHARID_BANK,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[(BRONZE_BAR_ID, 14), (COPPER_ORE_ID, 28), (TIN_ORE_ID, 28)],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_restocked.bank_open = true;
        bronze_restocked.bank_loaded = true;
        bronze_restocked.bank_generation = 1;
        let mut bronze_returned = resource_obs(
            AL_KHARID_FURNACE,
            &[(COPPER_ORE_ID, 14), (TIN_ORE_ID, 14)],
            &[],
            &[],
            &[("smithing", 87)],
            &bronze_levels,
        );
        bronze_returned.bank_generation = 2;
        let mut bronze_further = resource_obs(
            AL_KHARID_FURNACE,
            &[(BRONZE_BAR_ID, 1)],
            &[],
            &[],
            &[("smithing", 93)],
            &bronze_levels,
        );
        bronze_further.bank_generation = 2;
        assert!(witness(
            CoreCase::SmelterBot,
            &bronze_baseline,
            [
                &bronze_withdrawn,
                &bronze_produced,
                &bronze_deposited,
                &bronze_restocked,
                &bronze_returned,
                &bronze_further
            ]
        )
        .qualify()
        .is_ok());
        let mut bronze_wrong = bronze_produced.clone();
        bronze_wrong.item_ids.insert(STEEL_BAR_ID, 1);
        assert!(witness(
            CoreCase::SmelterBot,
            &bronze_baseline,
            [
                &bronze_withdrawn,
                &bronze_wrong,
                &bronze_deposited,
                &bronze_restocked,
                &bronze_returned,
                &bronze_further
            ]
        )
        .qualify()
        .is_err());
        let low_smith = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &[("smithing", 0)],
        );
        assert!(validate_case_baseline(CoreCase::SmelterBot, &low_smith).is_err());
        assert!(validate_case_baseline(CoreCase::SmelterBotSteel, &low_smith).is_err());
        validate_case_baseline(CoreCase::SmelterBot, &bronze_baseline).unwrap();
        assert!(validate_case_baseline(CoreCase::SmelterBotSteel, &bronze_baseline).is_err());

        let steel_levels = [("smithing", 30)];
        let steel_baseline = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[],
            &[],
            &[("smithing", 0)],
            &steel_levels,
        );
        validate_case_baseline(CoreCase::SmelterBotSteel, &steel_baseline).unwrap();
        let steel_withdrawn = resource_obs(
            AL_KHARID_BANK,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[],
            &[],
            &[("smithing", 0)],
            &steel_levels,
        );
        let steel_produced = resource_obs(
            AL_KHARID_FURNACE,
            &[(STEEL_BAR_ID, 9)],
            &[],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        let mut steel_deposited = resource_obs(
            AL_KHARID_BANK,
            &[],
            &[(STEEL_BAR_ID, 9), (IRON_ORE_ID, 47), (COAL_ID, 94)],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_deposited.bank_open = true;
        steel_deposited.bank_loaded = true;
        steel_deposited.bank_generation = 1;
        let mut steel_restocked = resource_obs(
            AL_KHARID_BANK,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[(STEEL_BAR_ID, 9), (IRON_ORE_ID, 38), (COAL_ID, 76)],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_restocked.bank_open = true;
        steel_restocked.bank_loaded = true;
        steel_restocked.bank_generation = 1;
        let mut steel_returned = resource_obs(
            AL_KHARID_FURNACE,
            &[(IRON_ORE_ID, 9), (COAL_ID, 18)],
            &[],
            &[],
            &[("smithing", 157)],
            &steel_levels,
        );
        steel_returned.bank_generation = 2;
        let mut steel_further = resource_obs(
            AL_KHARID_FURNACE,
            &[(STEEL_BAR_ID, 1)],
            &[],
            &[],
            &[("smithing", 175)],
            &steel_levels,
        );
        steel_further.bank_generation = 2;
        assert!(witness(
            CoreCase::SmelterBotSteel,
            &steel_baseline,
            [
                &steel_withdrawn,
                &steel_produced,
                &steel_deposited,
                &steel_restocked,
                &steel_returned,
                &steel_further
            ]
        )
        .qualify()
        .is_ok());

        let spin_levels = [("crafting", 1)];
        let spin_baseline = resource_obs(
            FLAX_SPINNER_BANK,
            &[],
            &[],
            &[],
            &[("crafting", 0)],
            &spin_levels,
        );
        validate_case_baseline(CoreCase::FlaxSpinner, &spin_baseline).unwrap();
        let spin_withdrawn = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &spin_levels,
        );
        let spin_produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        let mut spin_deposited = resource_obs(
            FLAX_SPINNER_BANK,
            &[],
            &[(BOW_STRING_ID, 28), (FLAX_ID, 28)],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_deposited.bank_open = true;
        spin_deposited.bank_loaded = true;
        spin_deposited.bank_generation = 1;
        let mut spin_restocked = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_restocked.bank_open = true;
        spin_restocked.bank_loaded = true;
        spin_restocked.bank_generation = 1;
        let mut spin_returned = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        spin_returned.bank_generation = 2;
        let mut spin_further = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &spin_levels,
        );
        spin_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_ok());
        let mut wool = spin_produced.clone();
        wool.item_ids.insert(BALL_OF_WOOL_ID, 1);
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &wool,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_err());
        let mut ground_return = resource_obs(
            FLAX_SPINNER_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &spin_levels,
        );
        ground_return.bank_generation = 2;
        let mut ground_further = resource_obs(
            FLAX_SPINNER_BANK,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &spin_levels,
        );
        ground_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxSpinner,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &ground_return,
                &ground_further
            ]
        )
        .qualify()
        .is_err());
        let mut seeded_string = spin_baseline.clone();
        seeded_string.item_ids.insert(BOW_STRING_ID, 1);
        assert!(validate_case_baseline(CoreCase::FlaxSpinner, &seeded_string).is_err());
    }

    #[test]
    fn flax_aio_and_secondary_cycles_require_exact_ids_deposit_return_and_further() {
        let craft = [("crafting", 1)];
        let aio_baseline = resource_obs(FLAX_FIELD, &[], &[], &[], &[("crafting", 0)], &craft);
        validate_case_baseline(CoreCase::FlaxAio, &aio_baseline).unwrap();
        let picked = resource_obs(
            FLAX_FIELD,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &craft,
        );
        let produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        let mut deposited = resource_obs(
            FLAX_AIO_BANK,
            &[],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = resource_obs(FLAX_FIELD, &[], &[], &[], &[("crafting", 420)], &craft);
        returned.bank_generation = 2;
        let mut further = resource_obs(
            FLAX_FIELD,
            &[(FLAX_ID, 1)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &produced, &deposited, &returned, &further]
        )
        .qualify()
        .is_ok());
        assert!(witness(CoreCase::FlaxAio, &aio_baseline, [&aio_baseline])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::FlaxAio, &aio_baseline, [&picked])
            .qualify()
            .is_err());
        assert!(
            witness(CoreCase::FlaxAio, &aio_baseline, [&picked, &produced])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &produced, &deposited, &returned]
        )
        .qualify()
        .is_err());
        let mut unclosed_return = returned.clone();
        unclosed_return.bank_generation = deposited.bank_generation;
        let mut unclosed_further = further.clone();
        unclosed_further.bank_generation = deposited.bank_generation;
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [
                &picked,
                &produced,
                &deposited,
                &unclosed_return,
                &unclosed_further
            ]
        )
        .qualify()
        .is_err());
        let no_consume = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28), (BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &no_consume, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let xp_only = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &xp_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut wool = produced.clone();
        wool.item_ids.insert(BALL_OF_WOOL_ID, 1);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &wool, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut noted = produced.clone();
        noted.item_ids.insert(NOTED_BOW_STRING_ID, 1);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &noted, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut name_only = produced.clone();
        name_only.item_ids.clear();
        name_only.items.insert("Bow string".into(), 28);
        assert!(witness(
            CoreCase::FlaxAio,
            &aio_baseline,
            [&picked, &name_only, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
        let mut seeded = aio_baseline.clone();
        seeded.item_ids.insert(BOW_STRING_ID, 1);
        assert!(validate_case_baseline(CoreCase::FlaxAio, &seeded).is_err());

        let pick_baseline = flax_obs(FLAX_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::FlaxAioPick, &pick_baseline).unwrap();
        let first = flax_obs(FLAX_FIELD, &[(FLAX_ID, 28)], &[]);
        let mut pick_deposited = flax_obs(FLAX_AIO_BANK, &[], &[(FLAX_ID, 28)]);
        pick_deposited.bank_open = true;
        pick_deposited.bank_loaded = true;
        pick_deposited.bank_generation = 1;
        let mut pick_returned = flax_obs(FLAX_FIELD, &[], &[]);
        pick_returned.bank_generation = 2;
        let mut pick_further = flax_obs(FLAX_FIELD, &[(FLAX_ID, 1)], &[]);
        pick_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioPick,
            &pick_baseline,
            [&first, &pick_deposited, &pick_returned, &pick_further]
        )
        .qualify()
        .is_ok());
        let mut spun = first.clone();
        spun.item_ids.insert(BOW_STRING_ID, 1);
        assert!(witness(
            CoreCase::FlaxAioPick,
            &pick_baseline,
            [&spun, &pick_deposited, &pick_returned, &pick_further]
        )
        .qualify()
        .is_err());
        let mut seeded_pick = pick_baseline.clone();
        seeded_pick.item_ids.insert(FLAX_ID, 28);
        assert!(validate_case_baseline(CoreCase::FlaxAioPick, &seeded_pick).is_err());

        let spin_baseline = resource_obs(FLAX_AIO_BANK, &[], &[], &[], &[("crafting", 0)], &craft);
        validate_case_baseline(CoreCase::FlaxAioSpin, &spin_baseline).unwrap();
        let spin_withdrawn = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 0)],
            &craft,
        );
        let spin_produced = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        let mut spin_deposited = resource_obs(
            FLAX_AIO_BANK,
            &[],
            &[(BOW_STRING_ID, 28), (FLAX_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_deposited.bank_open = true;
        spin_deposited.bank_loaded = true;
        spin_deposited.bank_generation = 1;
        let mut spin_restocked = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[(BOW_STRING_ID, 28)],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_restocked.bank_open = true;
        spin_restocked.bank_loaded = true;
        spin_restocked.bank_generation = 1;
        let mut spin_returned = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        spin_returned.bank_generation = 2;
        let mut spin_further = resource_obs(
            FLAX_SPINNER_WHEEL,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &craft,
        );
        spin_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioSpin,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &spin_returned,
                &spin_further
            ]
        )
        .qualify()
        .is_ok());
        let mut ground_return = resource_obs(
            FLAX_AIO_BANK,
            &[(FLAX_ID, 28)],
            &[],
            &[],
            &[("crafting", 420)],
            &craft,
        );
        ground_return.bank_generation = 2;
        let mut ground_further = resource_obs(
            FLAX_AIO_BANK,
            &[(BOW_STRING_ID, 1)],
            &[],
            &[],
            &[("crafting", 435)],
            &craft,
        );
        ground_further.bank_generation = 2;
        assert!(witness(
            CoreCase::FlaxAioSpin,
            &spin_baseline,
            [
                &spin_withdrawn,
                &spin_produced,
                &spin_deposited,
                &spin_restocked,
                &ground_return,
                &ground_further
            ]
        )
        .qualify()
        .is_err());

        let egg_baseline = flax_obs(EGG_FIELD, &[], &[]);
        validate_case_baseline(CoreCase::HerbloreSecondaries, &egg_baseline).unwrap();
        let taken = flax_obs(EGG_FIELD, &[(RED_SPIDERS_EGGS_ID, 18)], &[]);
        let mut egg_deposited = flax_obs(EGG_FIELD, &[], &[(RED_SPIDERS_EGGS_ID, 18)]);
        egg_deposited.bank_open = true;
        egg_deposited.bank_loaded = true;
        egg_deposited.bank_generation = 1;
        let mut egg_returned = flax_obs(EGG_FIELD, &[], &[]);
        egg_returned.bank_generation = 2;
        let mut egg_further = flax_obs(EGG_FIELD, &[(RED_SPIDERS_EGGS_ID, 1)], &[]);
        egg_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&taken, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::HerbloreSecondaries, &egg_baseline, [&taken])
                .qualify()
                .is_err()
        );
        let mut mixed = taken.clone();
        mixed.item_ids.insert(EYE_OF_NEWT_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&mixed, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_err());
        let mut noted_egg = taken.clone();
        noted_egg.item_ids.insert(NOTED_RED_SPIDERS_EGGS_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondaries,
            &egg_baseline,
            [&noted_egg, &egg_deposited, &egg_returned, &egg_further]
        )
        .qualify()
        .is_err());
        let mut seeded_egg = egg_baseline.clone();
        seeded_egg.item_ids.insert(RED_SPIDERS_EGGS_ID, 1);
        assert!(validate_case_baseline(CoreCase::HerbloreSecondaries, &seeded_egg).is_err());

        let newt_baseline = flax_obs(BETTY_SHOP, &[], &[]);
        validate_case_baseline(CoreCase::HerbloreSecondariesNewt, &newt_baseline).unwrap();
        let coins_held = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[]);
        let bought = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900), (EYE_OF_NEWT_ID, 10)], &[]);
        let mut newt_deposited = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900)], &[(EYE_OF_NEWT_ID, 10)]);
        newt_deposited.bank_open = true;
        newt_deposited.bank_loaded = true;
        newt_deposited.bank_generation = 1;
        let mut newt_returned = flax_obs(BETTY_SHOP, &[(COINS_ID, 4900)], &[]);
        newt_returned.bank_generation = 2;
        let mut newt_further = flax_obs(BETTY_SHOP, &[(COINS_ID, 4800), (EYE_OF_NEWT_ID, 1)], &[]);
        newt_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &bought,
                &newt_deposited,
                &newt_returned,
                &newt_further
            ]
        )
        .qualify()
        .is_ok());
        let no_spend = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000), (EYE_OF_NEWT_ID, 10)], &[]);
        let mut no_spend_deposited =
            flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[(EYE_OF_NEWT_ID, 10)]);
        no_spend_deposited.bank_open = true;
        no_spend_deposited.bank_loaded = true;
        no_spend_deposited.bank_generation = 1;
        let mut no_spend_returned = flax_obs(BETTY_SHOP, &[(COINS_ID, 5000)], &[]);
        no_spend_returned.bank_generation = 2;
        let mut no_spend_further =
            flax_obs(BETTY_SHOP, &[(COINS_ID, 5000), (EYE_OF_NEWT_ID, 1)], &[]);
        no_spend_further.bank_generation = 2;
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &no_spend,
                &no_spend_deposited,
                &no_spend_returned,
                &no_spend_further
            ]
        )
        .qualify()
        .is_err());
        let mut mixed_buy = bought.clone();
        mixed_buy.item_ids.insert(RED_SPIDERS_EGGS_ID, 1);
        assert!(witness(
            CoreCase::HerbloreSecondariesNewt,
            &newt_baseline,
            [
                &coins_held,
                &mixed_buy,
                &newt_deposited,
                &newt_returned,
                &newt_further
            ]
        )
        .qualify()
        .is_err());
        let mut seeded_newt = newt_baseline.clone();
        seeded_newt.item_ids.insert(EYE_OF_NEWT_ID, 1);
        assert!(validate_case_baseline(CoreCase::HerbloreSecondariesNewt, &seeded_newt).is_err());
    }

    fn combat_npc(
        index: usize,
        name: &str,
        health: i32,
        in_combat: bool,
        tile: (i32, i32, i32),
    ) -> BoundedNpc {
        BoundedNpc {
            index,
            name: Some(name.into()),
            health,
            total_health: 60,
            animation: if in_combat { 422 } else { 0 },
            in_combat,
            targeting_local: in_combat,
            tile,
            distance: 1,
        }
    }

    fn combat_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        levels: &[(&str, i32)],
        npcs: &[BoundedNpc],
        local_in_combat: bool,
        local_target_npc: Option<usize>,
    ) -> Observation {
        let mut observation =
            resource_obs(tile, item_ids, &[], &[(ADAMANT_SCIMITAR_ID, 1)], xp, levels);
        observation.npc_facts = npcs.to_vec();
        observation.local_in_combat = local_in_combat;
        observation.local_target_npc = local_target_npc;
        observation.local_health = 40;
        observation
    }

    fn auto_fighter_spec() -> CombatSpec {
        combat_spec(CoreCase::AutoFighter).expect("auto fighter combat spec")
    }

    fn auto_fighter_mage_obs(
        item_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        npcs: &[BoundedNpc],
        local_in_combat: bool,
        local_target_npc: Option<usize>,
        autocast_varp: i32,
    ) -> Observation {
        let mut observation = combat_obs(
            ARDY_THIEVER_STAND,
            item_ids,
            xp,
            &[("magic", 13), ("hitpoints", 40)],
            npcs,
            local_in_combat,
            local_target_npc,
        );
        observation.equipment_ids.clear();
        observation.equipment_ids.insert(STAFF_OF_FIRE_ID, 1);
        observation.varps.insert(AUTOCAST_MAGIC_VARP, autocast_varp);
        observation
    }

    #[test]
    fn auto_fighter_mage_requires_armed_rune_casts_on_selected_guard_lives() {
        let case = CoreCase::parse("auto_fighter_mage").expect("mage case registered");
        assert_eq!(case.scenario_name(), "auto_fighter_mage");
        assert_eq!(case.card_name(), "AutoFighter");

        let baseline = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 300)],
            &[("magic", 100)],
            &[],
            false,
            None,
            0,
        );
        validate_case_baseline(case, &baseline).unwrap();

        let first = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 149), (AIR_RUNE_ID, 298)],
            &[("magic", 106)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            3,
        );
        let death_and_respawn = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 148), (AIR_RUNE_ID, 296)],
            &[("magic", 112)],
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
            3,
        );
        assert!(witness(
            case,
            &baseline,
            [&first, &death_and_respawn, &death_and_respawn]
        )
        .qualify()
        .is_ok());

        let mut missing_staff = baseline.clone();
        missing_staff.equipment_ids.clear();
        assert!(validate_case_baseline(case, &missing_staff).is_err());
        let mut wrong_staff = baseline.clone();
        wrong_staff.equipment_ids.clear();
        wrong_staff.equipment_ids.insert(1381, 1);
        assert!(validate_case_baseline(case, &wrong_staff).is_err());
        let mut staff_only_in_inventory = baseline.clone();
        staff_only_in_inventory.equipment_ids.clear();
        staff_only_in_inventory.item_ids.insert(STAFF_OF_FIRE_ID, 1);
        assert!(validate_case_baseline(case, &staff_only_in_inventory).is_err());
        let mut short_runes = baseline.clone();
        short_runes.item_ids.insert(MIND_RUNE_ID, 149);
        assert!(validate_case_baseline(case, &short_runes).is_err());
        let mut surplus_runes = baseline.clone();
        surplus_runes.item_ids.insert(MIND_RUNE_ID, 151);
        assert!(validate_case_baseline(case, &surplus_runes).is_err());
        let mut short_air_runes = baseline.clone();
        short_air_runes.item_ids.insert(AIR_RUNE_ID, 298);
        assert!(validate_case_baseline(case, &short_air_runes).is_err());

        let selected_only = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 300)],
            &[("magic", 100)],
            &[],
            false,
            None,
            2,
        );
        assert!(witness(case, &baseline, [&selected_only])
            .qualify()
            .is_err());

        let selected_not_armed = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (MIND_RUNE_ID, 149), (AIR_RUNE_ID, 298)],
            &[("magic", 106)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            2,
        );
        let selected_not_armed_death = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (MIND_RUNE_ID, 148), (AIR_RUNE_ID, 296)],
            &[("magic", 112)],
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
            2,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &selected_not_armed,
                &selected_not_armed_death,
                &selected_not_armed_death,
            ]
        )
        .qualify()
        .is_err());

        let direct_staff_melee = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 300)],
            &[("magic", 100), ("strength", 108)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            3,
        );
        let direct_staff_death = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 300)],
            &[("magic", 100), ("strength", 112)],
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
            3,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &direct_staff_melee,
                &direct_staff_death,
                &direct_staff_death
            ]
        )
        .qualify()
        .is_err());

        let unchanged_xp = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 149), (AIR_RUNE_ID, 298)],
            &[("magic", 100)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            3,
        );
        let unchanged_xp_death = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 148), (AIR_RUNE_ID, 296)],
            &[("magic", 100)],
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
            3,
        );
        assert!(witness(
            case,
            &baseline,
            [&unchanged_xp, &unchanged_xp_death, &unchanged_xp_death]
        )
        .qualify()
        .is_err());

        let missing_rune_use = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 298)],
            &[("magic", 106)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            3,
        );
        let missing_rune_death = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 150), (AIR_RUNE_ID, 296)],
            &[("magic", 112)],
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
            3,
        );
        assert!(witness(
            case,
            &baseline,
            [&missing_rune_use, &missing_rune_death, &missing_rune_death]
        )
        .qualify()
        .is_err());

        let wrong_target = auto_fighter_mage_obs(
            &[(TROUT_ID, 8), (558, 149), (AIR_RUNE_ID, 298)],
            &[("magic", 106)],
            &[combat_npc(5, "Knight", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
            3,
        );
        assert!(witness(case, &baseline, [&wrong_target]).qualify().is_err());
        assert!(witness(case, &baseline, [&first, &death_and_respawn])
            .qualify()
            .is_err());
    }

    #[test]
    fn combat_observer_does_not_turn_unknown_zero_health_into_a_sticky_death() {
        let baseline = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[],
            false,
            None,
        );
        let mut unknown = combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND);
        unknown.total_health = 0;
        let unknown = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[unknown],
            false,
            None,
        );
        let selected = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 84)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );

        let mut cycle = CombatCoreCycle::default();
        cycle.observe(auto_fighter_spec(), &baseline, &unknown);
        cycle.observe(auto_fighter_spec(), &baseline, &selected);
        cycle.observe(auto_fighter_spec(), &baseline, &selected);

        assert_eq!(cycle.engagements, 1);
        assert_eq!(cycle.defeats, 0);
        assert!(!cycle.last.get(&5).expect("tracked guard").defeated);
    }

    #[test]
    fn combat_observer_does_not_count_disappearance_reappearance_as_a_new_life() {
        let levels = [("attack", 40), ("strength", 40), ("hitpoints", 40)];
        let baseline = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[],
            false,
            None,
        );
        let selected = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 84)],
            &levels,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let missing = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 84)],
            &levels,
            &[],
            false,
            None,
        );

        let mut cycle = CombatCoreCycle::default();
        for observation in [&selected, &missing, &selected] {
            cycle.observe(auto_fighter_spec(), &baseline, observation);
        }

        assert_eq!(cycle.engagements, 1);
        assert_eq!(cycle.defeats, 0);
    }

    #[test]
    fn combat_observer_rejects_other_actors_combat_and_stale_global_loot() {
        let levels = [("attack", 40), ("strength", 40), ("hitpoints", 40)];
        let baseline = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[],
            false,
            None,
        );
        let mut other_actor_npc = combat_npc(4, "Moss giant", 50, true, MOSS_GIANT_SAFESPOT);
        other_actor_npc.targeting_local = false;
        let other_actor = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[other_actor_npc],
            false,
            None,
        );
        let selected = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104)],
            &levels,
            &[combat_npc(4, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(4),
        );
        let stale_loot = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 104)],
            &levels,
            &[],
            false,
            None,
        );
        let spec = combat_spec(CoreCase::MossGiant).expect("moss giant combat spec");

        let mut cycle = CombatCoreCycle::default();
        cycle.observe(spec, &baseline, &other_actor);
        assert_eq!(cycle.engagements, 0);
        cycle.observe(spec, &baseline, &selected);
        cycle.observe(spec, &baseline, &stale_loot);
        assert_eq!(cycle.defeats, 0);
    }

    #[test]
    fn combat_observer_requires_a_fresh_matching_drop_for_disappearance() {
        let levels = [("attack", 40), ("strength", 40), ("hitpoints", 40)];
        let baseline = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[],
            false,
            None,
        );
        let selected = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104)],
            &levels,
            &[combat_npc(4, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(4),
        );
        let drop = BoundedGround {
            id: BIG_BONES_ID,
            count: 1,
            tile: MOSS_GIANT_SAFESPOT,
            distance: 1,
        };
        let mut stale_drop = selected.clone();
        stale_drop.ground_loot = vec![drop.clone()];
        let mut missing = selected.clone();
        missing.npc_facts.clear();
        missing.local_in_combat = false;
        missing.local_target_npc = None;
        missing.ground_loot = vec![drop];
        let spec = combat_spec(CoreCase::MossGiant).expect("moss giant combat spec");

        let mut cycle = CombatCoreCycle::default();
        for observation in [&selected, &stale_drop, &missing] {
            cycle.observe(spec, &baseline, observation);
        }

        assert_eq!(cycle.defeats, 0);

        let mut fresh_cycle = CombatCoreCycle::default();
        fresh_cycle.observe(spec, &baseline, &selected);
        fresh_cycle.observe(spec, &baseline, &missing);
        assert_eq!(fresh_cycle.defeats, 1);
    }

    #[test]
    fn combat_observer_clears_death_on_respawn_and_needs_a_later_work_frame() {
        let levels = [("attack", 40), ("strength", 40), ("hitpoints", 40)];
        let baseline = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[],
            false,
            None,
        );
        let first = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 84)],
            &levels,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let death = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 88)],
            &levels,
            &[combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND)],
            false,
            None,
        );
        let respawn = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 88)],
            &levels,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let spec = auto_fighter_spec();

        assert!(
            witness(CoreCase::AutoFighter, &baseline, [&first, &death, &respawn])
                .qualify()
                .is_err()
        );
        assert!(witness(
            CoreCase::AutoFighter,
            &baseline,
            [&first, &death, &respawn, &respawn]
        )
        .qualify()
        .is_ok());

        let mut cycle = CombatCoreCycle::default();
        for observation in [&first, &death, &respawn, &respawn] {
            cycle.observe(spec, &baseline, observation);
        }
        assert_eq!(cycle.engagements, 2);
        assert_eq!(cycle.defeats, 1);
        assert!(!cycle.last.get(&5).expect("respawned guard").defeated);
    }

    fn combat_witness_with_ground_drop(qualified: bool) -> CoreWitness {
        let levels = [("attack", 40), ("strength", 40), ("hitpoints", 40)];
        let chaos_base = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ChaosDruid, &chaos_base).unwrap();
        let mut chaos_first = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[combat_npc(1, "Chaos druid", 20, true, CHAOS_DRUID_FIELD)],
            true,
            Some(1),
        );
        let mut chaos_second = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[("strength", 54)],
            &levels,
            &[
                combat_npc(1, "Chaos druid", 0, false, CHAOS_DRUID_FIELD),
                combat_npc(8, "Chaos druid", 18, true, CHAOS_DRUID_FIELD),
            ],
            true,
            Some(8),
        );
        let drop = BoundedGround {
            id: UNIDENTIFIED_GUAM_ID,
            count: 2,
            tile: CHAOS_DRUID_FIELD,
            distance: 1,
        };
        chaos_first.ground_loot = vec![drop.clone()];
        chaos_second.ground_loot = vec![drop];
        witness(
            CoreCase::ChaosDruid,
            &chaos_base,
            if qualified {
                vec![&chaos_first, &chaos_second, &chaos_second]
            } else {
                vec![&chaos_first]
            },
        )
    }

    #[test]
    fn combat_success_receipt_preserves_nonempty_ground_counts() {
        let witness = combat_witness_with_ground_drop(true);
        let receipt = witness
            .qualify()
            .expect("qualified combat with ground drop");
        assert_eq!(
            receipt["combat_core_cycle"]["previous_ground"],
            json!([{
                "id": UNIDENTIFIED_GUAM_ID, "x": CHAOS_DRUID_FIELD.0,
                "z": CHAOS_DRUID_FIELD.1, "level": CHAOS_DRUID_FIELD.2, "count": 2
            }])
        );
    }

    #[test]
    fn combat_failure_receipt_preserves_nonempty_ground_counts() {
        let witness = combat_witness_with_ground_drop(false);
        assert!(witness.qualify().is_err());
        let receipt = accumulated_core(Some(&witness));
        assert!(receipt["error"].as_str().unwrap().contains("incomplete"));
        assert_eq!(
            receipt["witness"]["combat_core_cycle"]["previous_ground"],
            json!([{
                "id": UNIDENTIFIED_GUAM_ID, "x": CHAOS_DRUID_FIELD.0,
                "z": CHAOS_DRUID_FIELD.1, "level": CHAOS_DRUID_FIELD.2, "count": 2
            }])
        );
    }

    #[test]
    fn combat_cores_require_two_engagements_verified_defeat_style_xp_and_exact_loot() {
        let levels = [
            ("attack", COMBAT_ATTACK_LEVEL),
            ("strength", COMBAT_ATTACK_LEVEL),
            ("hitpoints", COMBAT_ATTACK_LEVEL),
        ];
        let moss_base = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::MossGiant, &moss_base).unwrap();

        let first = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 100), ("attack", 100)],
            &levels,
            &[combat_npc(4, "Moss giant", 50, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(4),
        );
        let second = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104), ("attack", 100)],
            &levels,
            &[
                combat_npc(4, "Moss giant", 0, false, MOSS_GIANT_SAFESPOT),
                combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT),
            ],
            true,
            Some(7),
        );
        let mut looted = second.clone();
        looted.item_ids.insert(BIG_BONES_ID, 1);
        looted.npc_facts = vec![combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT)];
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &second, &looted])
                .qualify()
                .is_ok()
        );

        assert!(witness(CoreCase::MossGiant, &moss_base, [&moss_base])
            .qualify()
            .is_err());
        assert!(witness(CoreCase::MossGiant, &moss_base, [&first])
            .qualify()
            .is_err());
        let xp_only = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 108), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        assert!(witness(CoreCase::MossGiant, &moss_base, [&xp_only])
            .qualify()
            .is_err());
        let attack_only = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 100), ("attack", 108)],
            &levels,
            &[
                combat_npc(4, "Moss giant", 0, true, MOSS_GIANT_SAFESPOT),
                combat_npc(7, "Moss giant", 40, true, MOSS_GIANT_SAFESPOT),
            ],
            true,
            Some(4),
        );
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &attack_only])
                .qualify()
                .is_err()
        );
        let despawn = combat_obs(
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_FOOD)],
            &[("strength", 104), ("attack", 100)],
            &levels,
            &[],
            false,
            None,
        );
        assert!(witness(CoreCase::MossGiant, &moss_base, [&first, &despawn])
            .qualify()
            .is_err());
        let mut noted = looted.clone();
        noted.item_ids.insert(NOTED_BIG_BONES_ID, 1);
        assert!(
            witness(CoreCase::MossGiant, &moss_base, [&first, &second, &noted])
                .qualify()
                .is_err()
        );
        let mut seeded = moss_base.clone();
        seeded.item_ids.insert(BIG_BONES_ID, 1);
        assert!(validate_case_baseline(CoreCase::MossGiant, &seeded).is_err());

        let hill_base = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BRASS_KEY_ID, 1)],
            &[("strength", 100)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::HillGiant, &hill_base).unwrap();
        let mut hill_without_key = hill_base.clone();
        hill_without_key.item_ids.remove(&BRASS_KEY_ID);
        assert!(validate_case_baseline(CoreCase::HillGiant, &hill_without_key).is_err());
        let hill_first = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD)],
            &[("strength", 100)],
            &levels,
            &[combat_npc(2, "Giant", 35, true, HILL_GIANT_PIT)],
            true,
            Some(2),
        );
        let hill_second = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 110)],
            &levels,
            &[
                combat_npc(2, "Giant", 0, false, HILL_GIANT_PIT),
                combat_npc(3, "Giant", 20, true, HILL_GIANT_PIT),
            ],
            true,
            Some(3),
        );
        assert!(witness(
            CoreCase::HillGiant,
            &hill_base,
            [&hill_first, &hill_second, &hill_second]
        )
        .qualify()
        .is_ok());
        let alias = combat_obs(
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BIG_BONES_ID, 1)],
            &[("strength", 110)],
            &levels,
            &[
                combat_npc(2, "Hill giant", 0, true, HILL_GIANT_PIT),
                combat_npc(3, "Hill giant", 20, true, HILL_GIANT_PIT),
            ],
            true,
            Some(2),
        );
        assert!(witness(CoreCase::HillGiant, &hill_base, [&alias])
            .qualify()
            .is_err());

        let chaos_base = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ChaosDruid, &chaos_base).unwrap();
        let chaos_first = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 50)],
            &levels,
            &[combat_npc(1, "Chaos druid", 20, true, CHAOS_DRUID_FIELD)],
            true,
            Some(1),
        );
        let chaos_second = combat_obs(
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[("strength", 54)],
            &levels,
            &[
                combat_npc(1, "Chaos druid", 0, false, CHAOS_DRUID_FIELD),
                combat_npc(8, "Chaos druid", 18, true, CHAOS_DRUID_FIELD),
            ],
            true,
            Some(8),
        );
        assert!(witness(
            CoreCase::ChaosDruid,
            &chaos_base,
            [&chaos_first, &chaos_second, &chaos_second]
        )
        .qualify()
        .is_ok());
        let mut noted_herb = chaos_second.clone();
        noted_herb.item_ids.insert(NOTED_HERB_ID, 1);
        assert!(witness(
            CoreCase::ChaosDruid,
            &chaos_base,
            [&chaos_first, &noted_herb]
        )
        .qualify()
        .is_err());

        let auto_base = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::AutoFighter, &auto_base).unwrap();
        let auto_first = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 80)],
            &levels,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let auto_second = combat_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[("strength", 88)],
            &levels,
            &[
                combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(9),
        );
        assert!(witness(
            CoreCase::AutoFighter,
            &auto_base,
            [&auto_first, &auto_second, &auto_second]
        )
        .qualify()
        .is_ok());

        let mut rock_base = combat_obs(
            ROCK_CRAB_SAFE_STAND,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[],
            false,
            None,
        );
        rock_base.dormant_rocks_seen = true;
        validate_case_baseline(CoreCase::RockCrab, &rock_base).unwrap();
        let mut awake_only = rock_base.clone();
        awake_only.dormant_rocks_seen = false;
        assert!(validate_case_baseline(CoreCase::RockCrab, &awake_only).is_err());
        let rocks = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[combat_npc(3, "Rocks", 50, false, ROCK_CRAB_SPOT)],
            false,
            None,
        );
        let woke = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 40)],
            &levels,
            &[combat_npc(3, "Rock Crab", 50, true, ROCK_CRAB_SPOT)],
            true,
            Some(3),
        );
        let rock_second = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 48)],
            &levels,
            &[
                combat_npc(3, "Rock Crab", 0, false, ROCK_CRAB_SPOT),
                combat_npc(6, "Rock Crab", 40, true, ROCK_CRAB_SPOT),
            ],
            true,
            Some(6),
        );
        assert!(witness(
            CoreCase::RockCrab,
            &rock_base,
            [&rocks, &woke, &rock_second, &rock_second]
        )
        .qualify()
        .is_ok());
        assert!(
            witness(CoreCase::RockCrab, &rock_base, [&woke, &rock_second])
                .qualify()
                .is_err()
        );
        let crab_alias = combat_obs(
            ROCK_CRAB_SPOT,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[("strength", 48)],
            &levels,
            &[
                combat_npc(3, "Rock crab", 0, true, ROCK_CRAB_SPOT),
                combat_npc(6, "Rock crab", 40, true, ROCK_CRAB_SPOT),
            ],
            true,
            Some(3),
        );
        assert!(
            witness(CoreCase::RockCrab, &rock_base, [&rocks, &crab_alias])
                .qualify()
                .is_err()
        );

        let mut dragon_base = combat_obs(
            GREEN_DRAGON_FIELD,
            &[(LOBSTER_ID, GREEN_DRAGON_FOOD), (DRAGONFIRE_SHIELD_ID, 1)],
            &[("strength", 90)],
            &levels,
            &[],
            false,
            None,
        );
        dragon_base.equipment_ids.clear();
        dragon_base.equipment_ids.insert(RUNE_SCIMITAR_ID, 1);
        assert!(validate_case_baseline(CoreCase::GreenDragon, &dragon_base).is_err());
        let mut worn_start = dragon_base.clone();
        worn_start.item_ids.remove(&DRAGONFIRE_SHIELD_ID);
        worn_start.equipment_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        validate_case_baseline(CoreCase::GreenDragon, &worn_start).unwrap();
        let mut no_shield = dragon_base.clone();
        no_shield.item_ids.remove(&DRAGONFIRE_SHIELD_ID);
        assert!(validate_case_baseline(CoreCase::GreenDragon, &no_shield).is_err());
        let mut worn_eq = dragon_base.equipment_ids.clone();
        worn_eq.insert(DRAGONFIRE_SHIELD_ID, 1);
        let dragon_first = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
                &[("strength", 90)],
                &levels,
                &[combat_npc(2, "Green dragon", 80, true, GREEN_DRAGON_FIELD)],
                true,
                Some(2),
            );
            observation.equipment_ids = worn_eq.clone();
            observation
        };
        let dragon_second = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD), (DRAGON_BONES_ID, 1)],
                &[("strength", 110)],
                &levels,
                &[
                    combat_npc(2, "Green dragon", 0, false, GREEN_DRAGON_FIELD),
                    combat_npc(4, "Green dragon", 70, true, GREEN_DRAGON_FIELD),
                ],
                true,
                Some(4),
            );
            observation.equipment_ids = worn_eq.clone();
            observation
        };
        assert!(witness(
            CoreCase::GreenDragon,
            &worn_start,
            [&dragon_first, &dragon_second, &dragon_second]
        )
        .qualify()
        .is_ok());
        let mut hide_ok = dragon_second.clone();
        hide_ok.item_ids.remove(&DRAGON_BONES_ID);
        hide_ok.item_ids.insert(GREEN_DRAGONHIDE_ID, 1);
        assert!(witness(
            CoreCase::GreenDragon,
            &worn_start,
            [&dragon_first, &hide_ok, &hide_ok]
        )
        .qualify()
        .is_ok());
        let mut wrong_hide = dragon_second.clone();
        wrong_hide.item_ids.insert(BLACK_DRAGONHIDE_ID, 1);
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&dragon_first, &wrong_hide]
        )
        .qualify()
        .is_err());
        let dragon_alias = {
            let mut observation = combat_obs(
                GREEN_DRAGON_FIELD,
                &[(LOBSTER_ID, GREEN_DRAGON_FOOD), (DRAGON_BONES_ID, 1)],
                &[("strength", 110)],
                &levels,
                &[
                    combat_npc(2, "Green Dragon", 0, true, GREEN_DRAGON_FIELD),
                    combat_npc(4, "Green Dragon", 70, true, GREEN_DRAGON_FIELD),
                ],
                true,
                Some(2),
            );
            observation.equipment_ids = worn_eq.clone();
            observation
        };
        assert!(
            witness(CoreCase::GreenDragon, &dragon_base, [&dragon_alias])
                .qualify()
                .is_err()
        );
        let mut unworn = dragon_second.clone();
        unworn.equipment_ids.remove(&DRAGONFIRE_SHIELD_ID);
        unworn.item_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        let mut unworn_first = dragon_first.clone();
        unworn_first.equipment_ids.remove(&DRAGONFIRE_SHIELD_ID);
        assert!(witness(
            CoreCase::GreenDragon,
            &dragon_base,
            [&unworn_first, &unworn]
        )
        .qualify()
        .is_err());

        let fire_base = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
            ],
            &[("strength", 70)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::FireGiant, &fire_base).unwrap();
        let mut no_amulet = fire_base.clone();
        no_amulet.item_ids.remove(&GLARIALS_AMULET_ID);
        assert!(validate_case_baseline(CoreCase::FireGiant, &no_amulet).is_err());
        let fire_first = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
            ],
            &[("strength", 70)],
            &levels,
            &[combat_npc(8, "Fire giant", 90, true, FIRE_GIANT_ROOM)],
            true,
            Some(8),
        );
        let fire_second = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
                (BIG_BONES_ID, 1),
            ],
            &[("strength", 86)],
            &levels,
            &[
                combat_npc(8, "Fire giant", 0, false, FIRE_GIANT_ROOM),
                combat_npc(11, "Fire giant", 80, true, FIRE_GIANT_ROOM),
            ],
            true,
            Some(11),
        );
        assert!(witness(
            CoreCase::FireGiant,
            &fire_base,
            [&fire_first, &fire_second, &fire_second]
        )
        .qualify()
        .is_ok());
        let fire_alias = combat_obs(
            FIRE_GIANT_ROOM,
            &[
                (LOBSTER_ID, FIRE_GIANT_FOOD),
                (GLARIALS_AMULET_ID, 1),
                (ROPE_ID, 1),
                (BIG_BONES_ID, 1),
            ],
            &[("strength", 86)],
            &levels,
            &[
                combat_npc(8, "Fire Giant", 0, true, FIRE_GIANT_ROOM),
                combat_npc(11, "Fire Giant", 80, true, FIRE_GIANT_ROOM),
            ],
            true,
            Some(8),
        );
        assert!(witness(CoreCase::FireGiant, &fire_base, [&fire_alias])
            .qualify()
            .is_err());

        let mut ardy_base = combat_obs(
            ARDY_THIEVER_STAND,
            &[],
            &[("strength", 60)],
            &levels,
            &[],
            false,
            None,
        );
        ardy_base.levels.insert("thieving".into(), 5);
        validate_case_baseline(CoreCase::ArdyFighter, &ardy_base).unwrap();
        let mut seeded_cake = ardy_base.clone();
        seeded_cake.item_ids.insert(CAKE_ID, 1);
        assert!(validate_case_baseline(CoreCase::ArdyFighter, &seeded_cake).is_err());
        let stolen = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 60)],
                &levels,
                &[],
                false,
                None,
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_first = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 60)],
                &levels,
                &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
                true,
                Some(5),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_second = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[(CAKE_ID, 1)],
                &[("strength", 72)],
                &levels,
                &[
                    combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                    combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
                ],
                true,
                Some(9),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&stolen, &ardy_first, &ardy_second, &ardy_second]
        )
        .qualify()
        .is_ok());
        let ardy_first_empty = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[],
                &[("strength", 60)],
                &levels,
                &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
                true,
                Some(5),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        let ardy_second_empty = {
            let mut observation = combat_obs(
                ARDY_THIEVER_STAND,
                &[],
                &[("strength", 72)],
                &levels,
                &[
                    combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND),
                    combat_npc(9, "Guard", 22, true, ARDY_THIEVER_STAND),
                ],
                true,
                Some(9),
            );
            observation.levels.insert("thieving".into(), 5);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&ardy_first_empty, &ardy_second_empty]
        )
        .qualify()
        .is_err());
        let chocolate = {
            let mut observation = ardy_second.clone();
            observation.item_ids.insert(CHOCOLATE_CAKE_ID, 1);
            observation
        };
        assert!(witness(
            CoreCase::ArdyFighter,
            &ardy_base,
            [&stolen, &ardy_first, &chocolate]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn old_catalog_explicitly_refuses_cut_string_mode() {
        let error =
            validate_case_catalog(CoreCase::BankFletcherCutString, CATALOG_COMMIT_A).unwrap_err();
        assert!(error.contains("has no mode setting"), "{error}");
        validate_case_catalog(CoreCase::BankFletcherCutString, CATALOG_COMMIT_B).unwrap();
        validate_case_catalog(CoreCase::BankFletcherString, CATALOG_COMMIT_A).unwrap();
        let staff_error =
            validate_case_catalog(CoreCase::SuperheaterFireBattlestaff, CATALOG_COMMIT_A)
                .unwrap_err();
        assert!(
            staff_error.contains("requires Staff of fire"),
            "{staff_error}"
        );
        validate_case_catalog(CoreCase::SuperheaterFireBattlestaff, CATALOG_COMMIT_B).unwrap();
        validate_case_catalog(CoreCase::Superheater, CATALOG_COMMIT_A).unwrap();
        validate_case_catalog(CoreCase::SuperheaterSteel, CATALOG_COMMIT_A).unwrap();
        for case in [
            CoreCase::AlcherCustomAlias,
            CoreCase::AlcherCustomName,
            CoreCase::DartFletcher,
            CoreCase::DartFletcherIron,
            CoreCase::HerbCleaner,
            CoreCase::HerbCleanerNamed,
            CoreCase::GemCutter,
            CoreCase::GemCutterNamed,
            CoreCase::DoorOpener,
            CoreCase::DoorOpenerGate,
            CoreCase::GnomeCourse,
            CoreCase::GnomeCourseRadius,
            CoreCase::FlaxPicker,
            CoreCase::Superheater,
            CoreCase::SuperheaterSteel,
            CoreCase::ChickenKillerBank,
            CoreCase::VialFiller,
            CoreCase::VialFillerEast,
            CoreCase::PotionMaker,
            CoreCase::PotionMakerNamed,
            CoreCase::TannerBot,
            CoreCase::TannerBotHard,
            CoreCase::RuneCrafter,
            CoreCase::RuneCrafterEarth,
            CoreCase::MuleCrafter,
            CoreCase::ArdyCakes,
            CoreCase::ArdyThiever,
            CoreCase::ArdyThieverKnight,
            CoreCase::GnomeChop,
            CoreCase::GnomeFletchShort,
            CoreCase::GnomeFletchLong,
            CoreCase::CoalTrucks,
            CoreCase::CookBot,
            CoreCase::CookBotLobster,
            CoreCase::SmelterBot,
            CoreCase::SmelterBotSteel,
            CoreCase::FlaxSpinner,
            CoreCase::FlaxAio,
            CoreCase::FlaxAioPick,
            CoreCase::FlaxAioSpin,
            CoreCase::HerbloreSecondaries,
            CoreCase::HerbloreSecondariesNewt,
            CoreCase::ChaosDruid,
            CoreCase::MossGiant,
            CoreCase::HillGiant,
            CoreCase::AutoFighter,
            CoreCase::AutoFighterRange,
            CoreCase::RockCrab,
            CoreCase::RockCrabRange,
            CoreCase::GreenDragon,
            CoreCase::GreenDragonSpecial,
            CoreCase::GreenDragonPotions,
            CoreCase::FireGiant,
            CoreCase::ArdyFighter,
        ] {
            validate_case_catalog(case, CATALOG_COMMIT_A).unwrap();
            validate_case_catalog(case, CATALOG_COMMIT_B).unwrap();
        }
    }

    /// One frame for the four option branches: exact items, worn gear, stats,
    /// varps, NPC lives, and the local combat flags the branch reads.
    #[allow(clippy::too_many_arguments)]
    fn branch_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        equipment_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        levels: &[(&str, i32)],
        effective: &[(&str, i32)],
        varps: &[(i32, i32)],
        npcs: &[BoundedNpc],
        in_combat: bool,
        dormant_rocks: bool,
    ) -> Observation {
        let mut observation = resource_obs(tile, item_ids, &[], equipment_ids, xp, levels);
        for (id, value) in varps {
            observation.varps.insert(*id, *value);
        }
        for (name, level) in effective {
            observation.effective_levels.insert((*name).into(), *level);
        }
        observation.npc_facts = npcs.to_vec();
        observation.local_in_combat = in_combat;
        observation.local_target_npc = npcs.first().map(|npc| npc.index);
        observation.local_health = 40;
        observation.dormant_rocks_seen = dormant_rocks;
        observation
    }

    fn bow_gear() -> Vec<(i32, i32)> {
        vec![(MAPLE_SHORTBOW_ID, 1), (BRONZE_ARROW_ID, 200)]
    }

    #[test]
    fn ranged_branches_need_the_sources_own_mode_and_projectile_spend() {
        for (name, card) in [
            ("auto_fighter_range", "AutoFighter"),
            ("rock_crab_range", "RockCrab"),
        ] {
            let case = CoreCase::parse(name).expect("range case registered");
            assert_eq!(case.scenario_name(), name);
            assert_eq!(case.card_name(), card);
            let spec = combat_spec(case).expect("range spec");
            assert_eq!(spec.style, CombatStyleWitness::Ranged);
            assert_eq!(spec.projectile, Some(BRONZE_ARROW_ID));
        }

        // A melee loadout, a low Ranged level, or a missing half of the ranged
        // pair must all refuse the Start baseline.
        let crab_baseline = branch_obs(
            ROCK_CRAB_SAFE_STAND,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &bow_gear(),
            &[("ranged", 0)],
            &[("ranged", 40), ("hitpoints", 40)],
            &[],
            &[],
            &[],
            false,
            true,
        );
        let crab_case = CoreCase::RockCrabRange;
        validate_case_baseline(crab_case, &crab_baseline).unwrap();
        let mut melee_kit = crab_baseline.clone();
        melee_kit.equipment_ids.clear();
        melee_kit.equipment_ids.insert(ADAMANT_SCIMITAR_ID, 1);
        assert!(validate_case_baseline(crab_case, &melee_kit).is_err());
        let mut bag_bow = crab_baseline.clone();
        bag_bow.equipment_ids.remove(&MAPLE_SHORTBOW_ID);
        bag_bow.item_ids.insert(MAPLE_SHORTBOW_ID, 1);
        assert!(validate_case_baseline(crab_case, &bag_bow).is_err());
        let mut no_arrows = crab_baseline.clone();
        no_arrows.equipment_ids.remove(&BRONZE_ARROW_ID);
        assert!(validate_case_baseline(crab_case, &no_arrows).is_err());
        let mut low_ranged = crab_baseline.clone();
        low_ranged.levels.insert("ranged".into(), 1);
        assert!(validate_case_baseline(crab_case, &low_ranged).is_err());
        let mut awake_rocks = crab_baseline.clone();
        awake_rocks.dormant_rocks_seen = false;
        assert!(validate_case_baseline(crab_case, &awake_rocks).is_err());

        // Rocks into a crab, rapid mode, and a stack that actually shrinks: the
        // real sweep branch qualifies.
        let rocks = ROCK_CRAB_SPOT;
        let first = branch_obs(
            ROCK_CRAB_SAFE_STAND,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[(MAPLE_SHORTBOW_ID, 1), (BRONZE_ARROW_ID, 199)],
            &[("ranged", 12)],
            &[("ranged", 40), ("hitpoints", 40)],
            &[],
            &[(COMBAT_MODE_VARP, RAPID_COMBAT_MODE)],
            &[
                combat_npc(3, "Rocks", 0, false, rocks),
                combat_npc(7, "Rock Crab", 30, true, rocks),
            ],
            true,
            true,
        );
        let defeat = branch_obs(
            ROCK_CRAB_SAFE_STAND,
            &[(LOBSTER_ID, ROCK_CRAB_FOOD)],
            &[(MAPLE_SHORTBOW_ID, 1), (BRONZE_ARROW_ID, 196)],
            &[("ranged", 34)],
            &[("ranged", 40), ("hitpoints", 40)],
            &[],
            &[(COMBAT_MODE_VARP, RAPID_COMBAT_MODE)],
            &[
                combat_npc(7, "Rock Crab", 0, false, rocks),
                combat_npc(9, "Rock Crab", 30, true, rocks),
            ],
            true,
            true,
        );
        assert!(
            witness(crab_case, &crab_baseline, [&first, &defeat, &defeat])
                .qualify()
                .is_ok()
        );

        // The witness only accumulates positive transitions, so a flaw has to
        // hold in every observed frame — a later frame that restores the
        // transition would mask a real false positive.
        fn flawed(
            first: &Observation,
            defeat: &Observation,
            flaw: &dyn Fn(&mut Observation),
        ) -> [Observation; 3] {
            let mut frames = [first.clone(), defeat.clone(), defeat.clone()];
            for frame in &mut frames {
                flaw(frame);
            }
            frames
        }

        // Strength XP instead of Ranged: the melee substitution this cell excludes.
        let melee_xp = flawed(&first, &defeat, &|frame| {
            frame.xp.insert("strength".into(), 40);
            frame.xp.insert("ranged".into(), 0);
        });
        assert!(witness(crab_case, &crab_baseline, melee_xp.iter())
            .qualify()
            .is_err());
        // A non-rapid combat mode never proves the source selected `rapid`.
        let slow_mode = flawed(&first, &defeat, &|frame| {
            frame.varps.insert(COMBAT_MODE_VARP, 0);
        });
        assert!(witness(crab_case, &crab_baseline, slow_mode.iter())
            .qualify()
            .is_err());
        // A stack that never shrinks is carried ammunition, not an arrow fired.
        let no_spend = flawed(&first, &defeat, &|frame| {
            frame.equipment_ids.insert(BRONZE_ARROW_ID, 200);
        });
        assert!(witness(crab_case, &crab_baseline, no_spend.iter())
            .qualify()
            .is_err());

        // AutoFighter's Guard branch shares the ranged witness, not the crab one.
        let guard_baseline = branch_obs(
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &bow_gear(),
            &[("ranged", 0)],
            &[("ranged", 40), ("hitpoints", 40)],
            &[],
            &[],
            &[],
            false,
            false,
        );
        validate_case_baseline(CoreCase::AutoFighterRange, &guard_baseline).unwrap();
        let mut crab_stand = guard_baseline.clone();
        crab_stand.tile = Some(ROCK_CRAB_SAFE_STAND);
        assert!(validate_case_baseline(CoreCase::AutoFighterRange, &crab_stand).is_err());
    }

    #[test]
    fn special_branch_needs_an_armed_bar_and_a_paid_pool() {
        let case = CoreCase::GreenDragonSpecial;
        let baseline = branch_obs(
            GREEN_DRAGON_FIELD,
            &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
            &[(DRAGONFIRE_SHIELD_ID, 1), (DRAGON_DAGGER_ID, 1)],
            &[("strength", 0)],
            &[("attack", 60), ("strength", 40), ("hitpoints", 40)],
            &[],
            &[(SA_ARMED_VARP, 0), (SA_ENERGY_VARP, 1000)],
            &[],
            false,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();

        // A weapon with no special, a shield-only kit, or an already-armed bar
        // cannot start this branch.
        let mut scimitar = baseline.clone();
        scimitar.equipment_ids.clear();
        scimitar.equipment_ids.insert(DRAGONFIRE_SHIELD_ID, 1);
        scimitar.equipment_ids.insert(RUNE_SCIMITAR_ID, 1);
        assert!(validate_case_baseline(case, &scimitar).is_err());
        let mut armed_start = baseline.clone();
        armed_start.varps.insert(SA_ARMED_VARP, SA_ARMED_VALUE);
        assert!(validate_case_baseline(case, &armed_start).is_err());

        let dragon = GREEN_DRAGON_FIELD;
        let engaged = branch_obs(
            GREEN_DRAGON_FIELD,
            &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
            &[(DRAGONFIRE_SHIELD_ID, 1), (DRAGON_DAGGER_ID, 1)],
            &[("strength", 60)],
            &[("attack", 60), ("strength", 40), ("hitpoints", 40)],
            &[],
            &[(SA_ARMED_VARP, SA_ARMED_VALUE), (SA_ENERGY_VARP, 1000)],
            &[combat_npc(4, "Green dragon", 30, true, dragon)],
            true,
            false,
        );
        let spent = branch_obs(
            GREEN_DRAGON_FIELD,
            &[(LOBSTER_ID, GREEN_DRAGON_FOOD)],
            &[(DRAGONFIRE_SHIELD_ID, 1), (DRAGON_DAGGER_ID, 1)],
            &[("strength", 120)],
            &[("attack", 60), ("strength", 40), ("hitpoints", 40)],
            &[],
            &[(SA_ARMED_VARP, 0), (SA_ENERGY_VARP, 750)],
            &[
                combat_npc(4, "Green dragon", 0, false, dragon),
                combat_npc(8, "Green dragon", 30, true, dragon),
            ],
            true,
            false,
        );
        assert!(witness(case, &baseline, [&engaged, &spent, &spent])
            .qualify()
            .is_ok());

        // A queued bar with no payment, or a payment with no arming, fails: the
        // flaw has to hold in every frame, because the witness only accumulates
        // positive transitions.
        let armed_but_unpaid = |frame: &mut Observation| {
            frame.varps.insert(SA_ARMED_VARP, SA_ARMED_VALUE);
            frame.varps.insert(SA_ENERGY_VARP, 1000);
        };
        let mut queued_engaged = engaged.clone();
        armed_but_unpaid(&mut queued_engaged);
        let mut queued_spent = spent.clone();
        armed_but_unpaid(&mut queued_spent);
        assert!(witness(
            case,
            &baseline,
            [&queued_engaged, &queued_spent, &queued_spent]
        )
        .qualify()
        .is_err());
        let paid_but_unarmed = |frame: &mut Observation| {
            frame.varps.insert(SA_ARMED_VARP, 0);
            frame.varps.insert(SA_ENERGY_VARP, 750);
        };
        let mut drained_engaged = engaged.clone();
        paid_but_unarmed(&mut drained_engaged);
        let mut drained_spent = spent.clone();
        paid_but_unarmed(&mut drained_spent);
        assert!(witness(
            case,
            &baseline,
            [&drained_engaged, &drained_spent, &drained_spent]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn potion_branch_needs_a_dose_and_its_native_boost() {
        let case = CoreCase::GreenDragonPotions;
        let baseline = branch_obs(
            GREEN_DRAGON_FIELD,
            &[
                (LOBSTER_ID, GREEN_DRAGON_FOOD),
                (SUPER_ATTACK_3_ID, 1),
                (SUPER_STRENGTH_3_ID, 1),
            ],
            &[(DRAGONFIRE_SHIELD_ID, 1), (RUNE_SCIMITAR_ID, 1)],
            &[("strength", 0)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[("attack", 40), ("strength", 40)],
            &[(SA_ENERGY_VARP, 1000)],
            &[],
            false,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();

        // A seeded two-dose flask, a pre-existing boost, or a mage kit is not a
        // prepared potion branch.
        let mut seeded_dose = baseline.clone();
        seeded_dose.item_ids.insert(SUPER_ATTACK_2_ID, 1);
        assert!(validate_case_baseline(case, &seeded_dose).is_err());
        let mut preboosted = baseline.clone();
        preboosted.effective_levels.insert("attack".into(), 47);
        assert!(validate_case_baseline(case, &preboosted).is_err());
        let mut bag_potions = baseline.clone();
        bag_potions.item_ids.remove(&SUPER_ATTACK_3_ID);
        assert!(validate_case_baseline(case, &bag_potions).is_err());

        let dragon = GREEN_DRAGON_FIELD;
        let fight = [
            (LOBSTER_ID, GREEN_DRAGON_FOOD),
            (SUPER_ATTACK_2_ID, 1),
            (SUPER_STRENGTH_3_ID, 1),
        ];
        let sip = branch_obs(
            GREEN_DRAGON_FIELD,
            &fight,
            &[(DRAGONFIRE_SHIELD_ID, 1), (RUNE_SCIMITAR_ID, 1)],
            &[("strength", 60)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[("attack", 47), ("strength", 40)],
            &[(SA_ENERGY_VARP, 1000)],
            &[combat_npc(4, "Green dragon", 30, true, dragon)],
            true,
            false,
        );
        let boosted = branch_obs(
            GREEN_DRAGON_FIELD,
            &fight,
            &[(DRAGONFIRE_SHIELD_ID, 1), (RUNE_SCIMITAR_ID, 1)],
            &[("strength", 120)],
            &[("attack", 40), ("strength", 40), ("hitpoints", 40)],
            &[("attack", 47), ("strength", 40)],
            &[(SA_ENERGY_VARP, 1000)],
            &[
                combat_npc(4, "Green dragon", 0, false, dragon),
                combat_npc(8, "Green dragon", 30, true, dragon),
            ],
            true,
            false,
        );
        assert!(witness(case, &baseline, [&sip, &boosted, &boosted])
            .qualify()
            .is_ok());

        // A dose with no boost, and a boost with no dose, both fail: the flaw has
        // to hold in every frame, because the witness only accumulates positive
        // transitions.
        let no_boost = |frame: &mut Observation| {
            frame.effective_levels.insert("attack".into(), 40);
        };
        let mut dose_sip = sip.clone();
        no_boost(&mut dose_sip);
        let mut dose_boosted = boosted.clone();
        no_boost(&mut dose_boosted);
        assert!(
            witness(case, &baseline, [&dose_sip, &dose_boosted, &dose_boosted])
                .qualify()
                .is_err()
        );
        let unswapped = |frame: &mut Observation| {
            frame.item_ids.insert(SUPER_ATTACK_3_ID, 1);
            frame.item_ids.remove(&SUPER_ATTACK_2_ID);
        };
        let mut carried_sip = sip.clone();
        unswapped(&mut carried_sip);
        let mut carried_boosted = boosted.clone();
        unswapped(&mut carried_boosted);
        assert!(witness(
            case,
            &baseline,
            [&carried_sip, &carried_boosted, &carried_boosted]
        )
        .qualify()
        .is_err());
    }

    #[allow(clippy::too_many_arguments)]
    fn ardy_fight_obs(
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        thieving_xp: i32,
        strength_xp: i32,
        thieving: i32,
        npcs: &[BoundedNpc],
        local_in_combat: bool,
        local_target_npc: Option<usize>,
    ) -> Observation {
        let mut observation = ardy_obs(tile, item_ids, bank_ids, thieving_xp, thieving);
        observation.xp.insert("strength".into(), strength_xp);
        observation
            .levels
            .insert("attack".into(), COMBAT_ATTACK_LEVEL);
        observation
            .levels
            .insert("strength".into(), COMBAT_ATTACK_LEVEL);
        observation
            .levels
            .insert("hitpoints".into(), COMBAT_ATTACK_LEVEL);
        observation.equipment_ids.insert(ADAMANT_SCIMITAR_ID, 1);
        observation.npc_facts = npcs.to_vec();
        observation.local_in_combat = local_in_combat;
        observation.local_target_npc = local_target_npc;
        observation.local_health = 40;
        observation
    }

    #[test]
    fn alternate_druid_camps_need_their_own_field_prereqs_and_target_identity() {
        let levels = [
            ("attack", 40),
            ("strength", 40),
            ("hitpoints", 40),
            ("thieving", 46),
            ("agility", 1),
        ];
        let tower_base = combat_obs(
            CHAOS_DRUID_TOWER_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 0)],
            &levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ChaosDruidTower, &tower_base).unwrap();
        let mut low_thieving = tower_base.clone();
        low_thieving.levels.insert("thieving".into(), 45);
        assert!(validate_case_baseline(CoreCase::ChaosDruidTower, &low_thieving).is_err());
        let mut edgeville = tower_base.clone();
        edgeville.tile = Some(CHAOS_DRUID_FIELD);
        assert!(validate_case_baseline(CoreCase::ChaosDruidTower, &edgeville).is_err());

        let tower_first = combat_obs(
            CHAOS_DRUID_TOWER_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 12)],
            &levels,
            &[combat_npc(
                2,
                "Chaos druid",
                20,
                true,
                CHAOS_DRUID_TOWER_FIELD,
            )],
            true,
            Some(2),
        );
        let tower_second = combat_obs(
            CHAOS_DRUID_TOWER_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[("strength", 40)],
            &levels,
            &[
                combat_npc(2, "Chaos druid", 0, false, CHAOS_DRUID_TOWER_FIELD),
                combat_npc(6, "Chaos druid", 18, true, CHAOS_DRUID_TOWER_FIELD),
            ],
            true,
            Some(6),
        );
        assert!(witness(
            CoreCase::ChaosDruidTower,
            &tower_base,
            [&tower_first, &tower_second, &tower_second]
        )
        .qualify()
        .is_ok());
        // Warrior name is the Yanille identity; Tower must keep Chaos druid.
        let mut warrior_first = tower_first.clone();
        warrior_first.npc_facts = vec![combat_npc(
            2,
            "Chaos druid warrior",
            20,
            true,
            CHAOS_DRUID_TOWER_FIELD,
        )];
        let mut warrior_second = tower_second.clone();
        warrior_second.npc_facts = vec![
            combat_npc(2, "Chaos druid warrior", 0, false, CHAOS_DRUID_TOWER_FIELD),
            combat_npc(6, "Chaos druid warrior", 18, true, CHAOS_DRUID_TOWER_FIELD),
        ];
        assert!(witness(
            CoreCase::ChaosDruidTower,
            &tower_base,
            [&warrior_first, &warrior_second, &warrior_second]
        )
        .qualify()
        .is_err());
        // Style XP without selected loot cannot qualify the tower cell.
        let mut no_loot = tower_second.clone();
        no_loot.item_ids.remove(&UNIDENTIFIED_GUAM_ID);
        assert!(witness(
            CoreCase::ChaosDruidTower,
            &tower_base,
            [&tower_first, &no_loot, &no_loot]
        )
        .qualify()
        .is_err());

        let yanille_levels = [
            ("attack", 40),
            ("strength", 40),
            ("hitpoints", 40),
            ("agility", 40),
            ("thieving", 1),
        ];
        let yanille_base = combat_obs(
            CHAOS_DRUID_YANILLE_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 0)],
            &yanille_levels,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ChaosDruidYanille, &yanille_base).unwrap();
        let mut low_agility = yanille_base.clone();
        low_agility.levels.insert("agility".into(), 39);
        assert!(validate_case_baseline(CoreCase::ChaosDruidYanille, &low_agility).is_err());
        let mut tower_stand = yanille_base.clone();
        tower_stand.tile = Some(CHAOS_DRUID_TOWER_FIELD);
        assert!(validate_case_baseline(CoreCase::ChaosDruidYanille, &tower_stand).is_err());

        let yanille_first = combat_obs(
            CHAOS_DRUID_YANILLE_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 12)],
            &yanille_levels,
            &[combat_npc(
                3,
                "Chaos druid warrior",
                22,
                true,
                CHAOS_DRUID_YANILLE_FIELD,
            )],
            true,
            Some(3),
        );
        let yanille_second = combat_obs(
            CHAOS_DRUID_YANILLE_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (NATURE_RUNE_ID, 1)],
            &[("strength", 44)],
            &yanille_levels,
            &[
                combat_npc(
                    3,
                    "Chaos druid warrior",
                    0,
                    false,
                    CHAOS_DRUID_YANILLE_FIELD,
                ),
                combat_npc(
                    9,
                    "Chaos druid warrior",
                    20,
                    true,
                    CHAOS_DRUID_YANILLE_FIELD,
                ),
            ],
            true,
            Some(9),
        );
        assert!(witness(
            CoreCase::ChaosDruidYanille,
            &yanille_base,
            [&yanille_first, &yanille_second, &yanille_second]
        )
        .qualify()
        .is_ok());
        // Non-warrior Chaos druid identity fails Yanille.
        let mut plain_first = yanille_first.clone();
        plain_first.npc_facts = vec![combat_npc(
            3,
            "Chaos druid",
            22,
            true,
            CHAOS_DRUID_YANILLE_FIELD,
        )];
        let mut plain_second = yanille_second.clone();
        plain_second.npc_facts = vec![
            combat_npc(3, "Chaos druid", 0, false, CHAOS_DRUID_YANILLE_FIELD),
            combat_npc(9, "Chaos druid", 20, true, CHAOS_DRUID_YANILLE_FIELD),
        ];
        assert!(witness(
            CoreCase::ChaosDruidYanille,
            &yanille_base,
            [&plain_first, &plain_second, &plain_second]
        )
        .qualify()
        .is_err());
    }

    #[test]
    fn fight_guard_response_needs_a_kill_and_rejects_the_flee_kite() {
        let cakes_base = ardy_fight_obs(
            ARDY_CAKES_STAND,
            &[(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES)],
            &[],
            0,
            0,
            5,
            &[],
            false,
            None,
        );
        validate_case_baseline(CoreCase::ArdyCakesFight, &cakes_base).unwrap();
        let mut no_weapon = cakes_base.clone();
        no_weapon.equipment_ids.clear();
        assert!(validate_case_baseline(CoreCase::ArdyCakesFight, &no_weapon).is_err());
        let mut low_attack = cakes_base.clone();
        low_attack.levels.insert("attack".into(), 1);
        assert!(validate_case_baseline(CoreCase::ArdyCakesFight, &low_attack).is_err());

        let stolen = ardy_fight_obs(
            ARDY_CAKES_STAND,
            &[(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES), (CAKE_ID, 1)],
            &[],
            16,
            0,
            5,
            &[],
            false,
            None,
        );
        let engaged = ardy_fight_obs(
            ARDY_CAKES_STAND,
            &[(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES), (CAKE_ID, 1)],
            &[],
            16,
            12,
            5,
            &[combat_npc(4, "Guard", 22, true, ARDY_CAKES_STAND)],
            true,
            Some(4),
        );
        let killed = ardy_fight_obs(
            ARDY_CAKES_STAND,
            &[(KNIFE_ID, ARDY_CAKES_BALLAST_KNIVES), (CAKE_ID, 1)],
            &[],
            16,
            40,
            5,
            &[combat_npc(4, "Guard", 0, false, ARDY_CAKES_STAND)],
            false,
            None,
        );
        assert!(witness(
            CoreCase::ArdyCakesFight,
            &cakes_base,
            [&stolen, &engaged, &killed]
        )
        .qualify()
        .is_ok());
        // Steal alone is the Flee core shape; Fight needs the kill.
        assert!(witness(CoreCase::ArdyCakesFight, &cakes_base, [&stolen])
            .qualify()
            .is_err());
        // Landing on the Flee kite tile fails this branch.
        let mut fled = killed.clone();
        fled.tile = Some(ARDY_FLEE_TILE);
        assert!(witness(
            CoreCase::ArdyCakesFight,
            &cakes_base,
            [&stolen, &engaged, &fled]
        )
        .qualify()
        .is_err());
        // Chocolate cake is not stall food.
        let mut wrong = stolen.clone();
        wrong.item_ids.insert(CHOCOLATE_CAKE_ID, 1);
        assert!(witness(
            CoreCase::ArdyCakesFight,
            &cakes_base,
            [&wrong, &engaged, &killed]
        )
        .qualify()
        .is_err());

        let thiever_base = ardy_fight_obs(ARDY_THIEVER_STAND, &[], &[], 0, 0, 40, &[], false, None);
        validate_case_baseline(CoreCase::ArdyThieverFight, &thiever_base).unwrap();
        let pickpocketed = ardy_fight_obs(
            ARDY_THIEVER_STAND,
            &[(COINS_ID, 30), (CAKE_ID, 1)],
            &[],
            484,
            0,
            40,
            &[],
            false,
            None,
        );
        let fight = ardy_fight_obs(
            ARDY_THIEVER_STAND,
            &[(COINS_ID, 30), (CAKE_ID, 1)],
            &[],
            484,
            20,
            40,
            &[combat_npc(5, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(5),
        );
        let killed_guard = ardy_fight_obs(
            ARDY_THIEVER_STAND,
            &[(COINS_ID, 30), (CAKE_ID, 1)],
            &[],
            484,
            48,
            40,
            &[combat_npc(5, "Guard", 0, false, ARDY_THIEVER_STAND)],
            false,
            None,
        );
        let mut deposited = ardy_fight_obs(
            ARDY_BANK,
            &[(CAKE_ID, 1)],
            &[(COINS_ID, 30)],
            484,
            48,
            40,
            &[],
            false,
            None,
        );
        deposited.bank_open = true;
        deposited.bank_loaded = true;
        deposited.bank_generation = 1;
        let mut returned = ardy_fight_obs(
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 1)],
            &[],
            484,
            48,
            40,
            &[],
            false,
            None,
        );
        returned.bank_generation = 2;
        let mut further = ardy_fight_obs(
            ARDY_THIEVER_STAND,
            &[(COINS_ID, 30), (CAKE_ID, 1)],
            &[],
            952,
            48,
            40,
            &[],
            false,
            None,
        );
        further.bank_generation = 2;
        assert!(witness(
            CoreCase::ArdyThieverFight,
            &thiever_base,
            [
                &pickpocketed,
                &fight,
                &killed_guard,
                &deposited,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_ok());
        // Flee kite before the kill fails the Fight branch even if banking completes.
        let mut fled_mid = fight.clone();
        fled_mid.tile = Some(ARDY_FLEE_TILE);
        assert!(witness(
            CoreCase::ArdyThieverFight,
            &thiever_base,
            [
                &pickpocketed,
                &fled_mid,
                &killed_guard,
                &deposited,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());
        // Bank cycle without a kill is the Flee cell, not Fight.
        assert!(witness(
            CoreCase::ArdyThieverFight,
            &thiever_base,
            [&pickpocketed, &deposited, &returned, &further]
        )
        .qualify()
        .is_err());
    }

    /// Camp/Fight option inject keys must be declared by the frozen card
    /// SETTINGS schema (or read via settings.* accessors). Matrix rows are not
    /// required for these four cells; matrix refresh is root-owned.
    #[test]
    fn camp_and_fight_option_injects_match_frozen_card_schemas() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (name, card, path_tail) in [
            (
                "chaos_druid_tower",
                "ChaosDruidKiller",
                "src/bot/scripts/ChaosDruidKiller/ChaosDruidKiller.ts",
            ),
            (
                "chaos_druid_yanille",
                "ChaosDruidKiller",
                "src/bot/scripts/ChaosDruidKiller/ChaosDruidKiller.ts",
            ),
            (
                "ardy_cakes_fight",
                "ArdyCakes",
                "src/bot/scripts/ArdyCakes/ArdyCakes.ts",
            ),
            (
                "ardy_thiever_fight",
                "ArdyThiever",
                "src/bot/scripts/ArdyThiever/ArdyThiever.ts",
            ),
        ] {
            let case = CoreCase::parse(name).expect("option branch registered");
            assert_eq!(case.card_name(), card);
            let scenario = scenario::get(name).expect("option scenario registered");
            let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject)
                .unwrap_or_else(|| panic!("{name} injects no settings"));
            for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
                let path = repo
                    .join(format!(".superpowers/inputs/rs2b0t-{commit}"))
                    .join(path_tail);
                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("{name}: read {}: {error}", path.display()));
                let declared = declared_source_settings(&source);
                assert!(
                    !declared.is_empty(),
                    "{name}: {card} at {commit} declares no SETTINGS schema"
                );
                for id in inject.keys() {
                    assert!(
                        declared.contains(id),
                        "{name}: {card} at {commit} does not declare setting {id:?}"
                    );
                }
            }
        }
    }

    /// The audit's seeds and injections are proposals, not verified
    /// implementation: every injected setting has to be one the frozen card
    /// itself declares, carrying a value its own schema allows. An undeclared key
    /// or an out-of-range value must fail here instead of riding along into a
    /// LIVE cell.
    ///
    /// The declaration authority is the identity-checked card source, because
    /// some matrix schema rows are incomplete (RockCrab's r274/r289 rows omit
    /// `bankStrategy` and `solveClues`, which both frozen sources do declare).
    /// Matrix rows are used for the value checks they do carry.
    #[test]
    fn option_branch_injects_match_both_frozen_card_schemas() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let matrix = support_matrix().unwrap();
        for (name, card) in [
            ("auto_fighter_range", "AutoFighter"),
            ("rock_crab_range", "RockCrab"),
            ("green_dragon_special", "GreenDragon"),
            ("green_dragon_potions", "GreenDragon"),
        ] {
            let case = CoreCase::parse(name).expect("option branch registered");
            assert_eq!(case.card_name(), card);
            let scenario = scenario::get(name).expect("option scenario registered");
            let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject)
                .unwrap_or_else(|| panic!("{name} injects no settings"));
            for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
                let root = repo.join(format!(".superpowers/inputs/rs2b0t-{commit}"));
                let catalog = catalog_ledger(&matrix, commit).unwrap();
                verify_registry_identity(&root, catalog).unwrap();
                for revision in [274_u16, 289_u16] {
                    let row = ledger_card(&matrix, commit, revision, case)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    let path = verify_source_identity(&root, &row)
                        .unwrap_or_else(|error| panic!("{name}: {error}"));
                    let source = std::fs::read_to_string(&path)
                        .unwrap_or_else(|error| panic!("{name}: read {}: {error}", path.display()));
                    let declared = declared_source_settings(&source);
                    assert!(
                        !declared.is_empty(),
                        "{name}: {card} at {commit} r{revision} declares no SETTINGS schema"
                    );
                    for (id, value) in &inject {
                        assert!(
                            declared.contains(id),
                            "{name}: {card} at {commit} r{revision} does not declare setting {id:?}"
                        );
                        let setting = row
                            .settings_and_behavior_branches
                            .iter()
                            .find(|setting| setting.id == *id);
                        if let Some(setting) = setting {
                            assert_declared_value(name, card, commit, revision, id, value, setting);
                        }
                    }
                }
            }
        }
    }

    /// Bank-cell observation helper: the shared combat shape plus the trip's
    /// bank session fields (open/loaded/generation).
    #[allow(clippy::too_many_arguments)]
    fn bank_obs(
        case: CoreCase,
        tile: (i32, i32, i32),
        item_ids: &[(i32, i32)],
        bank_ids: &[(i32, i32)],
        xp: &[(&str, i32)],
        npcs: &[BoundedNpc],
        local_in_combat: bool,
        local_target_npc: Option<usize>,
        generation: u64,
        open: bool,
    ) -> Observation {
        assert!(
            combat_bank_spec(case).is_some(),
            "{case:?} declares a bank spec"
        );
        let levels = [
            ("attack", COMBAT_ATTACK_LEVEL),
            ("strength", COMBAT_ATTACK_LEVEL),
            ("hitpoints", COMBAT_ATTACK_LEVEL),
            ("thieving", 5),
            ("agility", 1),
        ];
        let mut observation = combat_obs(
            tile,
            item_ids,
            xp,
            &levels,
            npcs,
            local_in_combat,
            local_target_npc,
        );
        observation.bank_ids = bank_ids.iter().copied().collect();
        observation.bank_open = open;
        observation.bank_loaded = open;
        observation.bank_generation = generation;
        observation
    }

    /// Every bank cell has to execute its card's own trip: the pack stock that
    /// belongs in the bank actually lands there, the card's restock line is met
    /// from that bank's own stock, the modal closes on a later session, the trip
    /// returns to the card's tile, and work resumes there. Seed-only banks,
    /// wrong-stand booths and dropped stages must all fail.
    #[test]
    fn bank_cells_require_their_own_deposit_restock_close_return_and_further_work() {
        // --- auto_fighter_bank: `banking=Auto`, East Ardougne, trout restock.
        let case = CoreCase::parse("auto_fighter_bank").expect("bank case registered");
        assert_eq!(case.card_name(), "AutoFighter");
        let baseline = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD), (UNCUT_SAPPHIRE_ID, 1)],
            &[(TROUT_ID, 20)],
            &[("strength", 0)],
            &[],
            false,
            None,
            0,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();
        let mut no_prepared_loot = baseline.clone();
        no_prepared_loot.item_ids.remove(&UNCUT_SAPPHIRE_ID);
        assert!(validate_case_baseline(case, &no_prepared_loot).is_err());
        let mut carried_weapon_only = baseline.clone();
        carried_weapon_only.equipment_ids.clear();
        assert!(validate_case_baseline(case, &carried_weapon_only).is_err());
        let mut short_food = baseline.clone();
        short_food.item_ids.insert(TROUT_ID, AUTO_FIGHTER_FOOD - 1);
        assert!(validate_case_baseline(case, &short_food).is_err());

        let first = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD), (UNCUT_SAPPHIRE_ID, 1)],
            &[(TROUT_ID, 20)],
            &[("strength", 12)],
            &[combat_npc(1, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(1),
            0,
            false,
        );
        let defeat = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD), (UNCUT_SAPPHIRE_ID, 1)],
            &[(TROUT_ID, 20)],
            &[("strength", 40)],
            &[
                combat_npc(1, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(2, "Guard", 20, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(2),
            0,
            false,
        );
        let deposited = bank_obs(
            case,
            ARDOUGNE_EAST_BANK,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[(TROUT_ID, 20), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 40)],
            &[],
            false,
            None,
            5,
            true,
        );
        let restocked = bank_obs(
            case,
            ARDOUGNE_EAST_BANK,
            &[(TROUT_ID, AUTO_FIGHTER_BANK_RESTOCK)],
            &[(TROUT_ID, 18), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 40)],
            &[],
            false,
            None,
            5,
            true,
        );
        let closed = bank_obs(
            case,
            ARDOUGNE_EAST_BANK,
            &[(TROUT_ID, AUTO_FIGHTER_BANK_RESTOCK)],
            &[(TROUT_ID, 18), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 40)],
            &[],
            false,
            None,
            6,
            false,
        );
        let returned = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_BANK_RESTOCK)],
            &[(TROUT_ID, 18), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 40)],
            &[],
            false,
            None,
            6,
            false,
        );
        let further = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_BANK_RESTOCK)],
            &[(TROUT_ID, 18), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 60)],
            &[combat_npc(3, "Guard", 20, true, ARDY_THIEVER_STAND)],
            true,
            Some(3),
            6,
            false,
        );
        assert!(witness(
            case,
            &baseline,
            [&first, &defeat, &deposited, &restocked, &closed, &returned, &further, &further]
        )
        .qualify()
        .is_ok());
        // The bank never took the pack stock: no deposit, nothing to qualify.
        assert!(witness(
            case,
            &baseline,
            [&first, &defeat, &first, &defeat, &first, &defeat]
        )
        .qualify()
        .is_err());
        // A bank that only ever opened (readiness/seed) fails: same pack loot,
        // no deposit move, no close, no return.
        let open_only = bank_obs(
            case,
            ARDOUGNE_EAST_BANK,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD), (UNCUT_SAPPHIRE_ID, 1)],
            &[(TROUT_ID, 20)],
            &[("strength", 40)],
            &[],
            false,
            None,
            5,
            true,
        );
        assert!(witness(
            case,
            &baseline,
            [&first, &defeat, &deposited, &open_only, &open_only]
        )
        .qualify()
        .is_err());
        // A loaded booth away from the card's own bank fails the cell.
        let wrong_bank = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(TROUT_ID, AUTO_FIGHTER_FOOD)],
            &[(TROUT_ID, 20), (UNCUT_SAPPHIRE_ID, 1)],
            &[("strength", 40)],
            &[],
            false,
            None,
            5,
            true,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &first,
                &defeat,
                &wrong_bank,
                &restocked,
                &closed,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());
        // The return without further work is not a completed trip.
        assert!(witness(
            case,
            &baseline,
            [&first, &defeat, &deposited, &restocked, &closed, &returned]
        )
        .qualify()
        .is_err());

        // --- moss_giant_bank: food-gone trip end, Ardougne West, lobster restock.
        let case = CoreCase::parse("moss_giant_bank").expect("bank case registered");
        assert_eq!(case.card_name(), "MossGiant");
        let baseline = bank_obs(
            case,
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_FOOD)],
            &[(LOBSTER_ID, 24)],
            &[("strength", 0)],
            &[],
            false,
            None,
            0,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();
        let mut seeded_loot = baseline.clone();
        seeded_loot.item_ids.insert(BIG_BONES_ID, 1);
        assert!(validate_case_baseline(case, &seeded_loot).is_err());
        let mut full_pack = baseline.clone();
        full_pack.item_ids.insert(LOBSTER_ID, MOSS_GIANT_FOOD + 1);
        assert!(validate_case_baseline(case, &full_pack).is_err());
        let mut foreign_stand = baseline.clone();
        foreign_stand.tile = Some(HILL_GIANT_PIT);
        assert!(validate_case_baseline(case, &foreign_stand).is_err());

        let first = bank_obs(
            case,
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_FOOD)],
            &[(LOBSTER_ID, 24)],
            &[("strength", 12)],
            &[combat_npc(4, "Moss giant", 22, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(4),
            0,
            false,
        );
        let bones_in_pack = bank_obs(
            case,
            MOSS_GIANT_SAFESPOT,
            &[(BIG_BONES_ID, 1)],
            &[(LOBSTER_ID, 24)],
            &[("strength", 44)],
            &[
                combat_npc(4, "Moss giant", 0, false, MOSS_GIANT_SAFESPOT),
                combat_npc(5, "Moss giant", 20, true, MOSS_GIANT_SAFESPOT),
            ],
            true,
            Some(5),
            0,
            false,
        );
        let deposited = bank_obs(
            case,
            MOSS_GIANT_BANK,
            &[],
            &[(LOBSTER_ID, 24), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            5,
            true,
        );
        let restocked = bank_obs(
            case,
            MOSS_GIANT_BANK,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_RESTOCK)],
            &[(LOBSTER_ID, 4), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            5,
            true,
        );
        let closed = bank_obs(
            case,
            MOSS_GIANT_BANK,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_RESTOCK)],
            &[(LOBSTER_ID, 4), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            6,
            false,
        );
        let returned = bank_obs(
            case,
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_RESTOCK)],
            &[(LOBSTER_ID, 4), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            6,
            false,
        );
        let further = bank_obs(
            case,
            MOSS_GIANT_SAFESPOT,
            &[(LOBSTER_ID, MOSS_GIANT_BANK_RESTOCK)],
            &[(LOBSTER_ID, 4), (BIG_BONES_ID, 1)],
            &[("strength", 68)],
            &[combat_npc(6, "Moss giant", 20, true, MOSS_GIANT_SAFESPOT)],
            true,
            Some(6),
            6,
            false,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &first,
                &bones_in_pack,
                &deposited,
                &restocked,
                &closed,
                &returned,
                &further,
                &further
            ]
        )
        .qualify()
        .is_ok());
        // A trip with no close/return is not the declared bank round trip.
        assert!(witness(
            case,
            &baseline,
            [&first, &bones_in_pack, &deposited, &restocked]
        )
        .qualify()
        .is_err());
        // Depositing without restocking the card's food line fails the cell.
        let loot_only_bank = bank_obs(
            case,
            MOSS_GIANT_BANK,
            &[],
            &[(LOBSTER_ID, 24), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            5,
            true,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &first,
                &bones_in_pack,
                &deposited,
                &loot_only_bank,
                &closed,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());

        // --- hill_giant_bank: `lootSlots=1` trip end, Varrock West, trout restock.
        let case = CoreCase::parse("hill_giant_bank").expect("bank case registered");
        assert_eq!(case.card_name(), "HillGiant");
        let baseline = bank_obs(
            case,
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BRASS_KEY_ID, 1)],
            &[(TROUT_ID, 12)],
            &[("strength", 0)],
            &[],
            false,
            None,
            0,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();
        let mut no_key = baseline.clone();
        no_key.item_ids.remove(&BRASS_KEY_ID);
        assert!(validate_case_baseline(case, &no_key).is_err());
        let mut note_holding = baseline.clone();
        note_holding.item_ids.insert(NOTED_BIG_BONES_ID, 1);
        assert!(validate_case_baseline(case, &note_holding).is_err());

        let first = bank_obs(
            case,
            HILL_GIANT_PIT,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BRASS_KEY_ID, 1)],
            &[(TROUT_ID, 12)],
            &[("strength", 12)],
            &[combat_npc(7, "Giant", 22, true, HILL_GIANT_PIT)],
            true,
            Some(7),
            0,
            false,
        );
        let looted = bank_obs(
            case,
            HILL_GIANT_PIT,
            &[
                (TROUT_ID, HILL_GIANT_FOOD),
                (BRASS_KEY_ID, 1),
                (BIG_BONES_ID, 1),
            ],
            &[(TROUT_ID, 12)],
            &[("strength", 44)],
            &[
                combat_npc(7, "Giant", 0, false, HILL_GIANT_PIT),
                combat_npc(8, "Giant", 20, true, HILL_GIANT_PIT),
            ],
            true,
            Some(8),
            0,
            false,
        );
        let deposited = bank_obs(
            case,
            HILL_GIANT_BANK,
            &[(TROUT_ID, HILL_GIANT_FOOD), (BRASS_KEY_ID, 1)],
            &[(TROUT_ID, 12), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            5,
            true,
        );
        let restocked = bank_obs(
            case,
            HILL_GIANT_BANK,
            &[
                (TROUT_ID, HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK),
                (BRASS_KEY_ID, 1),
            ],
            &[(TROUT_ID, 8), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            5,
            true,
        );
        let closed = bank_obs(
            case,
            HILL_GIANT_BANK,
            &[
                (TROUT_ID, HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK),
                (BRASS_KEY_ID, 1),
            ],
            &[(TROUT_ID, 8), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            6,
            false,
        );
        let returned = bank_obs(
            case,
            HILL_GIANT_PIT,
            &[
                (TROUT_ID, HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK),
                (BRASS_KEY_ID, 1),
            ],
            &[(TROUT_ID, 8), (BIG_BONES_ID, 1)],
            &[("strength", 44)],
            &[],
            false,
            None,
            6,
            false,
        );
        let further = bank_obs(
            case,
            HILL_GIANT_PIT,
            &[
                (TROUT_ID, HILL_GIANT_FOOD + HILL_GIANT_BANK_RESTOCK),
                (BRASS_KEY_ID, 1),
            ],
            &[(TROUT_ID, 8), (BIG_BONES_ID, 1)],
            &[("strength", 68)],
            &[combat_npc(9, "Giant", 20, true, HILL_GIANT_PIT)],
            true,
            Some(9),
            6,
            false,
        );
        assert!(witness(
            case,
            &baseline,
            [&first, &looted, &deposited, &restocked, &closed, &returned, &further, &further]
        )
        .qualify()
        .is_ok());
        // A banked drop with no return to the pit is incomplete.
        assert!(witness(
            case,
            &baseline,
            [&first, &looted, &deposited, &restocked, &closed]
        )
        .qualify()
        .is_err());
        // Every pit spot is a legitimate return; a surface bank tile is not.
        let mut surface_return = returned.clone();
        surface_return.tile = Some(HILL_GIANT_BANK);
        assert!(witness(
            case,
            &baseline,
            [
                &first,
                &looted,
                &deposited,
                &restocked,
                &closed,
                &surface_return,
                &surface_return
            ]
        )
        .qualify()
        .is_err());

        // --- chaos_druid_bank: `prepare-trip` end, Edgeville booth and trapdoor.
        let case = CoreCase::parse("chaos_druid_bank").expect("bank case registered");
        assert_eq!(case.card_name(), "ChaosDruidKiller");
        let baseline = bank_obs(
            case,
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            0,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();
        let mut full_trip_food = baseline.clone();
        full_trip_food.item_ids.insert(LOBSTER_ID, CHAOS_DRUID_FOOD);
        assert!(validate_case_baseline(case, &full_trip_food).is_err());
        let mut seeded_herb = baseline.clone();
        seeded_herb.item_ids.insert(UNIDENTIFIED_GUAM_ID, 1);
        assert!(validate_case_baseline(case, &seeded_herb).is_err());

        let deposited = bank_obs(
            case,
            CHAOS_DRUID_BANK,
            &[],
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD + CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            5,
            true,
        );
        let restocked = bank_obs(
            case,
            CHAOS_DRUID_BANK,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            5,
            true,
        );
        let closed = bank_obs(
            case,
            CHAOS_DRUID_BANK,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            6,
            false,
        );
        let returned = bank_obs(
            case,
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            6,
            false,
        );
        let first = bank_obs(
            case,
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 12)],
            &[combat_npc(10, "Chaos druid", 22, true, CHAOS_DRUID_FIELD)],
            true,
            Some(10),
            6,
            false,
        );
        let looted = bank_obs(
            case,
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 44)],
            &[
                combat_npc(10, "Chaos druid", 0, false, CHAOS_DRUID_FIELD),
                combat_npc(11, "Chaos druid", 20, true, CHAOS_DRUID_FIELD),
            ],
            true,
            Some(11),
            6,
            false,
        );
        let further = bank_obs(
            case,
            CHAOS_DRUID_FIELD,
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD), (UNIDENTIFIED_GUAM_ID, 1)],
            &[(LOBSTER_ID, CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 68)],
            &[combat_npc(12, "Chaos druid", 20, true, CHAOS_DRUID_FIELD)],
            true,
            Some(12),
            6,
            false,
        );
        assert!(witness(
            case,
            &baseline,
            [&deposited, &restocked, &closed, &returned, &first, &looted, &further, &further]
        )
        .qualify()
        .is_ok());
        // Depositing the pack but never withdrawing the food back fails.
        let no_restock = bank_obs(
            case,
            CHAOS_DRUID_BANK,
            &[],
            &[(LOBSTER_ID, CHAOS_DRUID_FOOD + CHAOS_DRUID_BANK_FOOD)],
            &[("strength", 0)],
            &[],
            false,
            None,
            5,
            true,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &deposited,
                &no_restock,
                &closed,
                &returned,
                &first,
                &looted,
                &further,
                &further
            ]
        )
        .qualify()
        .is_err());

        // --- ardy_fighter_bank: `bankStrategy=Loot count` PeriodicBank.
        let case = CoreCase::parse("ardy_fighter_bank").expect("bank case registered");
        assert_eq!(case.card_name(), "ArdyFighter");
        let baseline = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[],
            &[],
            &[("strength", 0), ("thieving", 0)],
            &[],
            false,
            None,
            0,
            false,
        );
        validate_case_baseline(case, &baseline).unwrap();
        let mut seeded_cake = baseline.clone();
        seeded_cake.item_ids.insert(CAKE_ID, 1);
        assert!(validate_case_baseline(case, &seeded_cake).is_err());
        let mut low_thieving = baseline.clone();
        low_thieving.levels.insert("thieving".into(), 4);
        assert!(validate_case_baseline(case, &low_thieving).is_err());
        let mut no_weapon = baseline.clone();
        no_weapon.equipment_ids.clear();
        assert!(validate_case_baseline(case, &no_weapon).is_err());

        let stolen = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 4)],
            &[],
            &[("strength", 0), ("thieving", 60)],
            &[],
            false,
            None,
            0,
            false,
        );
        let first = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 4)],
            &[],
            &[("strength", 12), ("thieving", 60)],
            &[combat_npc(13, "Guard", 22, true, ARDY_THIEVER_STAND)],
            true,
            Some(13),
            0,
            false,
        );
        let looted = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 4), (STEEL_ARROW_ID, 5)],
            &[],
            &[("strength", 44), ("thieving", 60)],
            &[
                combat_npc(13, "Guard", 0, false, ARDY_THIEVER_STAND),
                combat_npc(14, "Guard", 20, true, ARDY_THIEVER_STAND),
            ],
            true,
            Some(14),
            0,
            false,
        );
        let deposited = bank_obs(
            case,
            ARDY_BANK,
            &[(CAKE_ID, 4)],
            &[(STEEL_ARROW_ID, 5)],
            &[("strength", 44), ("thieving", 60)],
            &[],
            false,
            None,
            5,
            true,
        );
        let closed = bank_obs(
            case,
            ARDY_BANK,
            &[(CAKE_ID, 4)],
            &[(STEEL_ARROW_ID, 5)],
            &[("strength", 44), ("thieving", 60)],
            &[],
            false,
            None,
            6,
            false,
        );
        let returned = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 4)],
            &[(STEEL_ARROW_ID, 5)],
            &[("strength", 44), ("thieving", 60)],
            &[],
            false,
            None,
            6,
            false,
        );
        let further = bank_obs(
            case,
            ARDY_THIEVER_STAND,
            &[(CAKE_ID, 4)],
            &[(STEEL_ARROW_ID, 5)],
            &[("strength", 68), ("thieving", 60)],
            &[combat_npc(15, "Guard", 20, true, ARDY_THIEVER_STAND)],
            true,
            Some(15),
            6,
            false,
        );
        assert!(witness(
            case,
            &baseline,
            [&stolen, &first, &looted, &deposited, &closed, &returned, &further, &further]
        )
        .qualify()
        .is_ok());
        // The stall steal alone (no Guard kill, no trip) is the Flee/Off core,
        // not this bank cell.
        assert!(
            witness(case, &baseline, [&stolen, &stolen, &stolen, &stolen])
                .qualify()
                .is_err()
        );
        // A deposit that never closes and returns is not the declared trip.
        assert!(witness(
            case,
            &baseline,
            [&stolen, &first, &looted, &deposited, &deposited]
        )
        .qualify()
        .is_err());
        // Loot left in the pack (no deposit move) fails even with a close.
        let no_deposit = bank_obs(
            case,
            ARDY_BANK,
            &[(CAKE_ID, 4), (STEEL_ARROW_ID, 5)],
            &[],
            &[("strength", 44), ("thieving", 60)],
            &[],
            false,
            None,
            5,
            true,
        );
        assert!(witness(
            case,
            &baseline,
            [
                &stolen,
                &first,
                &looted,
                &no_deposit,
                &closed,
                &returned,
                &further
            ]
        )
        .qualify()
        .is_err());
    }

    /// Bank-cell inject keys and values must be declared by their own frozen
    /// card (schema key or `settings.*` accessor).
    #[test]
    fn bank_cell_injects_match_frozen_card_schemas() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for (name, card, path_tail) in [
            (
                "auto_fighter_bank",
                "AutoFighter",
                "src/bot/scripts/AutoFighter/AutoFighter.ts",
            ),
            (
                "moss_giant_bank",
                "MossGiant",
                "src/bot/scripts/MossGiant/MossGiant.ts",
            ),
            (
                "hill_giant_bank",
                "HillGiant",
                "src/bot/scripts/HillGiant/HillGiant.ts",
            ),
            (
                "chaos_druid_bank",
                "ChaosDruidKiller",
                "src/bot/scripts/ChaosDruidKiller/ChaosDruidKiller.ts",
            ),
            (
                "ardy_fighter_bank",
                "ArdyFighter",
                "src/bot/scripts/ArdyFighter/ArdyFighter.ts",
            ),
        ] {
            let case = CoreCase::parse(name).expect("bank case registered");
            assert_eq!(case.card_name(), card);
            let scenario = scenario::get(name).expect("bank scenario registered");
            let inject = scenario::settings_inject_map(scenario.settings.script_settings_inject)
                .unwrap_or_else(|| panic!("{name} injects no settings"));
            for commit in [CATALOG_COMMIT_A, CATALOG_COMMIT_B] {
                let path = repo
                    .join(format!(".superpowers/inputs/rs2b0t-{commit}"))
                    .join(path_tail);
                let source = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("{name}: read {}: {error}", path.display()));
                let declared = declared_source_settings(&source);
                assert!(
                    !declared.is_empty(),
                    "{name}: {card} at {commit} declares no SETTINGS schema"
                );
                for id in inject.keys() {
                    assert!(
                        declared.contains(id),
                        "{name}: {card} at {commit} does not declare setting {id:?}"
                    );
                }
            }
        }
    }

    /// The identifiers a frozen card actually reads as settings: the keys of its
    /// own `SETTINGS` schema plus the keys it pulls through `this.settings.<kind>`
    /// accessors, because a card can read a key it never lists in the schema
    /// (RockCrab's PeriodicBank strategy reads `bankStrategy`). Only four-space
    /// schema keys count: deeper lines are the members of a nested setting object,
    /// and a spread (the loadout schema) is not a setting id.
    fn declared_source_settings(source: &str) -> BTreeSet<String> {
        let start = source
            .find(": SettingsSchema = {")
            .expect("card declares a SETTINGS schema");
        let rest = &source[start..];
        let end = rest.find("\n};").expect("SETTINGS schema terminates");
        let mut declared = rest[..end]
            .lines()
            .filter_map(|line| {
                let key = line.strip_prefix("    ")?;
                if key.starts_with(' ') {
                    return None;
                }
                let (id, _) = key.split_once(':')?;
                let id = id.trim();
                (!id.is_empty()
                    && id
                        .chars()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == '_'))
                .then(|| id.to_string())
            })
            .collect::<BTreeSet<_>>();
        for accessor in ["str", "num", "bool", "list", "strList", "tile"] {
            let needle = format!("settings.{accessor}('");
            for (index, _) in source.match_indices(&needle) {
                let rest = &source[index + needle.len()..];
                if let Some((key, _)) = rest.split_once('\'') {
                    if !key.is_empty() && key.len() < 64 {
                        declared.insert(key.to_string());
                    }
                }
            }
        }
        declared
    }

    fn assert_declared_value(
        name: &str,
        card: &str,
        commit: &str,
        revision: u16,
        id: &str,
        value: &Value,
        setting: &DeclaredSetting,
    ) {
        let context = format!("{name}: {card} {id} at {commit} r{revision}");
        match value {
            Value::String(text) => {
                assert_eq!(setting.kind, "string", "{context}: declared type");
                if !setting.options.is_empty() {
                    assert!(
                        setting
                            .options
                            .iter()
                            .any(|option| option.as_str() == Some(text.as_str())),
                        "{context}: {text:?} is not one of {:?}",
                        setting.options
                    );
                }
            }
            Value::Number(number) => {
                assert_eq!(setting.kind, "number", "{context}: declared type");
                let actual = number.as_f64().unwrap_or(f64::NAN);
                let bound = |value: &Option<Value>| match value {
                    Some(Value::Number(number)) => number.as_f64(),
                    Some(Value::String(text)) => text.parse::<f64>().ok(),
                    _ => None,
                };
                if let Some(min) = bound(&setting.min) {
                    assert!(
                        actual >= min,
                        "{context}: {actual} below declared min {min}"
                    );
                }
                if let Some(max) = bound(&setting.max) {
                    assert!(
                        actual <= max,
                        "{context}: {actual} above declared max {max}"
                    );
                }
            }
            Value::Bool(_) => assert_eq!(setting.kind, "boolean", "{context}: declared type"),
            other => panic!("{context}: unsupported injected value {other:?}"),
        }
    }
}
