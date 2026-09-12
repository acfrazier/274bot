//! Catalog identity, observations, and isolation witnesses for the two-slot
//! lifecycle proof. Not a shared catalog/scenario harness.

use std::path::{Component, Path, PathBuf};

use api::snapshot::{GameSnapshot, ItemView};
use serde::Serialize;
use serde_json::{json, Map, Value};

pub const SUPPORT_MATRIX: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/catalog-support-matrix.json"
));
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
pub const CARD_NAME: &str = "Alcher";

pub const RUNE_CHAINBODY_ID: i32 = 1113;
pub const CERT_RUNE_CHAINBODY_ID: i32 = 1114;
pub const ADAMANT_SCIMITAR_ID: i32 = 1331;
pub const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
pub const NATURE_RUNE_ID: i32 = 561;
pub const COINS_ID: i32 = 995;
pub const STAFF_OF_FIRE_ID: i32 = 1387;
pub const RUNE_CHAINBODY_ALCH_COINS: i32 = 30_000;
pub const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1_536;
pub const HIGH_ALCH_MAGIC_XP: i32 = 65;
pub const MAGIC_LEVEL: i32 = 55;
pub const FODDER_COUNT: i32 = 30;
pub const NATURE_COUNT: i32 = 200;
pub const VARROCK_WEST: (i32, i32, i32) = (3185, 3440, 0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlotKind {
    Chainbody,
    Scimitar,
}

impl SlotKind {
    pub fn own_unnoted(self) -> i32 {
        match self {
            Self::Chainbody => RUNE_CHAINBODY_ID,
            Self::Scimitar => ADAMANT_SCIMITAR_ID,
        }
    }

    pub fn own_noted(self) -> i32 {
        match self {
            Self::Chainbody => CERT_RUNE_CHAINBODY_ID,
            Self::Scimitar => CERT_ADAMANT_SCIMITAR_ID,
        }
    }

    pub fn foreign_unnoted(self) -> i32 {
        match self {
            Self::Chainbody => ADAMANT_SCIMITAR_ID,
            Self::Scimitar => RUNE_CHAINBODY_ID,
        }
    }

    pub fn foreign_noted(self) -> i32 {
        match self {
            Self::Chainbody => CERT_ADAMANT_SCIMITAR_ID,
            Self::Scimitar => CERT_RUNE_CHAINBODY_ID,
        }
    }

    pub fn alch_coins(self) -> i32 {
        match self {
            Self::Chainbody => RUNE_CHAINBODY_ALCH_COINS,
            Self::Scimitar => ADAMANT_SCIMITAR_ALCH_COINS,
        }
    }

    pub fn seed_obj(self) -> &'static str {
        match self {
            Self::Chainbody => "rune_chainbody",
            Self::Scimitar => "adamant_scimitar",
        }
    }

    pub fn settings(self) -> Map<String, Value> {
        let mut bag = Map::new();
        match self {
            Self::Chainbody => {
                bag.insert("items".into(), json!(["rune_chainbody"]));
            }
            Self::Scimitar => {
                bag.insert("items".into(), json!(["custom"]));
                bag.insert("customItem".into(), json!("adamant_scimitar"));
            }
        }
        bag
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Observation {
    pub ingame: bool,
    pub scene_state: i32,
    pub inventory_tab_available: bool,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub tick: u32,
    pub magic: i32,
    pub magic_xp: i32,
    pub coins: i32,
    pub natures: i32,
    pub own_unnoted: i32,
    pub own_noted: i32,
    pub foreign_unnoted: i32,
    pub foreign_noted: i32,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_own: i32,
    pub bank_foreign: i32,
    pub bank_natures: i32,
    pub bank_staff: i32,
}

impl Observation {
    pub fn from_snapshot(snapshot: &GameSnapshot, kind: SlotKind) -> Self {
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            inventory_tab_available: snapshot
                .side_tabs()
                .iter()
                .any(|tab| tab.index == 3 && tab.available),
            player: snapshot
                .local_player()
                .and_then(|local| local.player.actor.name.clone()),
            tile: snapshot.tile(),
            tick: snapshot.tick(),
            magic: snapshot
                .stats()
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case("magic"))
                .map(|stat| stat.base)
                .unwrap_or(0),
            magic_xp: snapshot
                .stats()
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case("magic"))
                .map(|stat| stat.xp)
                .unwrap_or(0),
            coins: count_id(snapshot.inventory(), COINS_ID),
            natures: count_id(snapshot.inventory(), NATURE_RUNE_ID),
            own_unnoted: count_id(snapshot.inventory(), kind.own_unnoted()),
            own_noted: count_id(snapshot.inventory(), kind.own_noted()),
            foreign_unnoted: count_id(snapshot.inventory(), kind.foreign_unnoted()),
            foreign_noted: count_id(snapshot.inventory(), kind.foreign_noted()),
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_own: count_id(snapshot.bank(), kind.own_unnoted())
                + count_id(snapshot.bank(), kind.own_noted()),
            bank_foreign: count_id(snapshot.bank(), kind.foreign_unnoted())
                + count_id(snapshot.bank(), kind.foreign_noted()),
            bank_natures: count_id(snapshot.bank(), NATURE_RUNE_ID),
            bank_staff: count_id(snapshot.bank(), STAFF_OF_FIRE_ID)
                + count_id(snapshot.equipment(), STAFF_OF_FIRE_ID),
        }
    }

    pub fn own_inv(&self) -> i32 {
        self.own_unnoted + self.own_noted
    }

    pub fn foreign_inv(&self) -> i32 {
        self.foreign_unnoted + self.foreign_noted
    }

    pub fn has_progress_over(&self, baseline: &Self) -> bool {
        self.magic_xp > baseline.magic_xp && self.coins > baseline.coins
    }

    pub fn has_stop_progress_over(&self, boundary: &Self) -> bool {
        self.tick > boundary.tick
            && self.has_progress_over(boundary)
            && self.own_inv() < boundary.own_inv()
            && self.natures < boundary.natures
    }

    pub fn isolated_from_peer(&self) -> bool {
        self.foreign_inv() == 0 && self.bank_foreign == 0
    }
}

pub fn count_id(items: &[ItemView], id: i32) -> i32 {
    items
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

pub fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotRecord {
    pub kind: SlotKind,
    pub account: String,
    pub settings: Map<String, Value>,
    pub baseline: Observation,
    pub first: Option<Observation>,
    pub drain: Option<Observation>,
    pub pause_end: Option<Observation>,
    pub further: Option<Observation>,
    pub stop_boundary: Option<Observation>,
    pub after_stop: Option<Observation>,
    pub peak_own: i32,
    pub peak_natures: i32,
    pub peak_coins: i32,
    pub peak_magic_xp: i32,
    pub post_start: u32,
    pub mixed_identity: bool,
    pub foreign_seen: bool,
}

impl SlotRecord {
    pub fn new(
        kind: SlotKind,
        account: String,
        settings: Map<String, Value>,
        baseline: Observation,
    ) -> Self {
        Self {
            peak_own: baseline.own_inv(),
            peak_natures: baseline.natures,
            peak_coins: baseline.coins,
            peak_magic_xp: baseline.magic_xp,
            kind,
            account,
            settings,
            baseline,
            first: None,
            drain: None,
            pause_end: None,
            further: None,
            stop_boundary: None,
            after_stop: None,
            post_start: 0,
            mixed_identity: false,
            foreign_seen: false,
        }
    }

    pub fn observe(&mut self, observation: Observation, expected_player: &str) {
        self.post_start = self.post_start.saturating_add(1);
        if observation
            .player
            .as_deref()
            .is_some_and(|player| !player.eq_ignore_ascii_case(expected_player))
        {
            self.mixed_identity = true;
        }
        if !observation.isolated_from_peer() {
            self.foreign_seen = true;
        }
        self.peak_own = self.peak_own.max(observation.own_inv());
        self.peak_natures = self.peak_natures.max(observation.natures);
        self.peak_coins = self.peak_coins.max(observation.coins);
        self.peak_magic_xp = self.peak_magic_xp.max(observation.magic_xp);
        if self.first.is_none() && self.progressed(&observation) {
            self.first = Some(observation);
        }
    }

    pub fn progressed(&self, observation: &Observation) -> bool {
        observation.has_progress_over(&self.baseline)
            && self.peak_own > self.baseline.own_inv()
            && observation.own_inv() < self.peak_own
            && self.peak_natures > self.baseline.natures
            && observation.natures < self.peak_natures
            && observation.coins >= self.kind.alch_coins()
            && (observation.magic_xp - self.baseline.magic_xp) >= HIGH_ALCH_MAGIC_XP
            && observation.isolated_from_peer()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordedState {
    Idle,
    Running,
    Paused,
    Other,
}

impl From<script::RunState> for RecordedState {
    fn from(state: script::RunState) -> Self {
        match state {
            script::RunState::Idle => Self::Idle,
            script::RunState::Running => Self::Running,
            script::RunState::Paused => Self::Paused,
            _ => Self::Other,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct IsolationWitness {
    pub a: SlotRecord,
    pub b: SlotRecord,
    pub pause_a: Option<RecordedState>,
    pub pause_b: Option<RecordedState>,
    pub resume_a: Option<RecordedState>,
    pub stop_a: Option<RecordedState>,
    pub stop_b: Option<RecordedState>,
}

impl IsolationWitness {
    pub fn new(a: SlotRecord, b: SlotRecord) -> Self {
        Self {
            a,
            b,
            pause_a: None,
            pause_b: None,
            resume_a: None,
            stop_a: None,
            stop_b: None,
        }
    }

    pub fn qualify(&self) -> Result<Value, String> {
        if self.a.account.eq_ignore_ascii_case(&self.b.account) {
            return Err("mixed identities: both slots share one account".into());
        }
        if self.a.settings == self.b.settings {
            return Err("settings contamination: both slots received the same bag".into());
        }
        if self.a.mixed_identity || self.b.mixed_identity {
            return Err("mixed identities: published player did not match the minted slot".into());
        }
        if self.a.foreign_seen || self.b.foreign_seen {
            return Err("cross-slot result contamination: peer fodder appeared".into());
        }
        if self.a.post_start == 0 || self.b.post_start == 0 {
            return Err("queued-only success: no post-Start observations".into());
        }
        if self.a.first.is_none() || self.b.first.is_none() {
            return Err("queued-only success: Start without observed item/coin/XP deltas".into());
        }
        let Some(a_drain) = self.a.drain.as_ref() else {
            return Err("missing drain snapshot before pause stability".into());
        };
        let Some(b_drain) = self.b.drain.as_ref() else {
            return Err("missing peer drain snapshot before pause stability".into());
        };
        let Some(a_pause) = self.a.pause_end.as_ref() else {
            return Err("missing paused-slot stability snapshot".into());
        };
        let Some(b_pause) = self.b.pause_end.as_ref() else {
            return Err(
                "no-progress control: continuing slot was not observed while peer paused".into(),
            );
        };
        if self.pause_a != Some(RecordedState::Paused) {
            return Err(format!(
                "pause isolation failed: paused slot state {:?}, expected paused",
                self.pause_a
            ));
        }
        if self.pause_b != Some(RecordedState::Running) {
            return Err(format!(
                "pause isolation failed: continuing slot state {:?}, expected running",
                self.pause_b
            ));
        }
        if a_pause.magic_xp > a_drain.magic_xp || a_pause.coins > a_drain.coins {
            return Err("pause isolation failed: paused slot kept progressing after drain".into());
        }
        if !b_pause.has_progress_over(b_drain) {
            return Err(
                "no-progress control: continuing slot did not progress while peer paused".into(),
            );
        }
        if self.resume_a != Some(RecordedState::Running) {
            return Err(format!(
                "resume failed: paused slot state {:?}, expected running",
                self.resume_a
            ));
        }
        let Some(a_further) = self.a.further.as_ref() else {
            return Err("no-progress control: resumed slot made no further progress".into());
        };
        if !a_further.has_progress_over(a_pause) {
            return Err("no-progress control: resumed slot made no further progress".into());
        }
        if self.stop_a != Some(RecordedState::Idle) {
            return Err(format!(
                "stop isolation failed: stopped slot state {:?}, expected idle",
                self.stop_a
            ));
        }
        if self.stop_b != Some(RecordedState::Running) {
            return Err(format!(
                "stop isolation failed: continuing slot state {:?}, expected running",
                self.stop_b
            ));
        }
        let Some(a_stop) = self.a.stop_boundary.as_ref() else {
            return Err("missing stop-boundary snapshot".into());
        };
        let Some(b_stop) = self.b.stop_boundary.as_ref() else {
            return Err("missing stop-boundary snapshot".into());
        };
        let Some(b_after) = self.b.after_stop.as_ref() else {
            return Err(
                "no-progress control: continuing slot was not observed after peer Stop".into(),
            );
        };
        if b_after.tick <= b_stop.tick {
            return Err(
                "stale stop-boundary: after-Stop observation is not later than the Stop boundary"
                    .into(),
            );
        }
        if !b_after.has_stop_progress_over(b_stop) {
            return Err(
                "no-progress control: continuing slot did not progress after peer Stop".into(),
            );
        }
        if let Some(a_after) = self.a.after_stop.as_ref() {
            if a_after.magic_xp > a_stop.magic_xp || a_after.coins > a_stop.coins {
                return Err("stop isolation failed: stopped slot kept progressing".into());
            }
        }
        Ok(json!({
            "a": {
                "account": self.a.account,
                "kind": self.a.kind,
                "settings": self.a.settings,
                "coins": self.a.peak_coins,
                "magic_xp": self.a.peak_magic_xp,
            },
            "b": {
                "account": self.b.account,
                "kind": self.b.kind,
                "settings": self.b.settings,
                "coins": self.b.peak_coins,
                "magic_xp": self.b.peak_magic_xp,
            },
            "pause_a": self.pause_a,
            "pause_b": self.pause_b,
            "resume_a": self.resume_a,
            "stop_a": self.stop_a,
            "stop_b": self.stop_b,
        }))
    }
}

#[derive(Debug, serde::Deserialize)]
struct SupportMatrix {
    catalogs: Vec<CatalogLedger>,
    rows: Vec<CardLedger>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CatalogLedger {
    pub commit: String,
    pub identity: String,
    pub read_only_path: String,
    pub registry_path: String,
    pub registry_sha256: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CardLedger {
    pub display_name: String,
    pub catalog_commit: String,
    pub source_path: String,
    pub source_sha256: String,
    pub revision: u16,
}

pub struct PreparedCard {
    pub js: String,
    pub shape: script::LoadShape,
    pub siblings: Vec<(String, String)>,
    pub schema: Vec<script::SettingDef>,
    pub identity: Value,
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
            "TWO_SLOT_CATALOG_COMMIT must be the exact lowercase 40-hex frozen commit, got {commit:?}"
        ));
    }
    if commit != CATALOG_COMMIT_A && commit != CATALOG_COMMIT_B {
        return Err(format!(
            "TWO_SLOT_CATALOG_COMMIT {commit} is not a frozen catalog"
        ));
    }
    Ok(())
}

pub fn catalog_ledger(commit: &str) -> Result<CatalogLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .catalogs
        .iter()
        .filter(|catalog| catalog.commit == commit)
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [catalog] => Ok(catalog.clone()),
        [] => Err(format!(
            "TWO_SLOT_CATALOG_COMMIT {commit} is not frozen in support-matrix.json"
        )),
        _ => Err(format!(
            "TWO_SLOT_CATALOG_COMMIT {commit} is duplicated in support-matrix.json"
        )),
    }
}

pub fn alcher_row(commit: &str, revision: u16) -> Result<CardLedger, String> {
    let matrix = support_matrix()?;
    validate_commit(commit)?;
    let rows = matrix
        .rows
        .iter()
        .filter(|row| {
            row.catalog_commit == commit
                && row.revision == revision
                && row.display_name == CARD_NAME
        })
        .cloned()
        .collect::<Vec<_>>();
    match rows.as_slice() {
        [row] => Ok(row.clone()),
        [] => Err(format!(
            "support matrix has no {CARD_NAME} row for catalog {commit} revision {revision}"
        )),
        _ => Err(format!(
            "support matrix has duplicate {CARD_NAME} rows for catalog {commit} revision {revision}"
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

pub fn hash_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    Ok(script::JsCache::origin_sha(&bytes))
}

pub fn verify_source_identity(root: &Path, row: &CardLedger) -> Result<PathBuf, String> {
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

pub fn verify_registry_identity(root: &Path, catalog: &CatalogLedger) -> Result<PathBuf, String> {
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

pub fn prepare_alcher(root: &Path, temp: &Path, row: &CardLedger) -> Result<PreparedCard, String> {
    let source_path = verify_source_identity(root, row)?;
    let mut library =
        script::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    let registered = library.register_rs2b0t(root, &temp.join("rs2b0t-path"))?;
    if registered == 0 {
        return Err("frozen catalog registered no script cards".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, CARD_NAME)?;
    let card = library
        .get(script::ScriptSource::Catalog, CARD_NAME)
        .cloned()
        .ok_or_else(|| format!("catalog registry has no {CARD_NAME} card"))?;
    let canonical_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", source_path.display()))?;
    let canonical_card = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize {}: {error}", card.path.display()))?;
    if canonical_card != canonical_source {
        return Err(format!(
            "registry path mismatch for {CARD_NAME}: ledger {}, loader {}",
            source_path.display(),
            card.path.display()
        ));
    }
    if card.sha256 != row.source_sha256 {
        return Err(format!(
            "loader source hash mismatch for {CARD_NAME}: ledger {}, loader {}",
            row.source_sha256, card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{CARD_NAME} is unloadable: {import}"));
    }
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
    Ok(PreparedCard {
        identity: json!({
            "card": CARD_NAME,
            "source_path": card.path,
            "source_sha256": card.sha256,
            "compiled_sha256": script::JsCache::origin_sha(card.js.as_bytes()),
            "siblings": sibling_hashes,
        }),
        js: card.js,
        shape: card.shape,
        siblings,
        schema: card.settings_schema,
    })
}

pub fn slot_settings(schema: &[script::SettingDef], kind: SlotKind) -> Map<String, Value> {
    script::merge_bag(schema, &kind.settings(), None)
}
