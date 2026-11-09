//! Real Engine L proofs for pinned BankSorter and a local-source probe of
//! GatheringBot's banking start path and baseline gathering loop.
//!
//! Each ignored test owns one fresh account and writes a complete JSON receipt
//! to LIVE_EVIDENCE_DIR. Run three isolated invocations per product commit.

#![cfg(feature = "live-harness")]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::{GameSnapshot, ItemView};
use client::client::Client;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, ProfileOptions, SharedClientTemplate,
};
use script::bank::sort::{
    category_of, is_unmatched, rank_within, BankCategory, BankSortRow, CATEGORY_ORDER,
};
use serde::Serialize;
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings};

const CATALOG_SHORT: &str = "ae56ccc95";
const QUEST_RED_BEAD: i32 = 1470;
const RED_BEAD: i32 = 1470;
const ASHES: i32 = 592;
const COINS: i32 = 995;
const LOGS: i32 = 1511;
const DRAYNOR_BANK: (i32, i32, i32) = (3093, 3243, 0);
const DRAYNOR_TREES: (i32, i32, i32) = (3098, 3242, 0);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const PREPARATION_DEADLINE: Duration = Duration::from_secs(240);
const SORT_DEADLINE: Duration = Duration::from_secs(240);
const GATHER_DEADLINE: Duration = Duration::from_secs(900);
const STOP_DEADLINE: Duration = Duration::from_secs(15);
// References in the pinned rs2b0t source: GatheringBot.ts:956-987 calls
// purgePackAtBank; :1279-1297 opens and waits for the startup bank; :1402-1408
// passes inventory/bank counts to withdrawCoins; :1561-1573 paces before close.
// Its loop equips tools before Gather (:1117-1142, GatheringBotTasks.ts:1770-1795);
// equipTools uses Equipment.equip (GatheringBot.ts:3283-3359). GatheringBotTasks.ts:2436-2454 and
// :3000-3011 use Locs.query and Loc.interact for the real gathering action.
const GATHERING_PROBE_SOURCE: &str = r#"import { LoopingBot } from '../../api/bot/Bot.js';
import { Banking, bankPace, purgePackAtBank, waitBankReady, withdrawCoins } from '../../api/bank/Banking.js';
import { Bank } from '../../api/bank/Bank.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import { Execution } from '../../api/execution/Execution.js';
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Locs } from '../../api/locs/Locs.js';

const TOOL = 'Bronze axe';
const COINS = 'Coins';
const NEED = 200;
const TARGET = 'Tree';
const ACTION = 'Chop down';
const PRODUCT = 'Logs';
const SITE = { x: 3098, z: 3242, level: 0 };

export default class GatheringBotStartPathProbe extends LoopingBot {
    started = false;

    async loop() {
        if (this.started) return;
        this.started = true;
        const purgeLog = message => this.log(message);
        const log = message => this.log(`  ${message}`);

        if (!(await purgePackAtBank({
            keep: [TOOL],
            preferNearby: true,
            obstacles: ['door', 'gate'],
            log: purgeLog,
        }))) {
            throw new Error('start pack purge failed');
        }

        if (!(await Banking.open({ log })) || !(await waitBankReady(log))) {
            throw new Error('startup bank did not become ready');
        }
        const invCoins = Inventory.count(COINS);
        const bankCoins = Bank.isOpen() ? Bank.count(COINS) : 0;
        if (!(await withdrawCoins(NEED, { invCoins, bankCoins, coinName: COINS, log }))) {
            throw new Error('startup coin withdrawal failed');
        }

        await bankPace(log);
        if (!(await Bank.close())) throw new Error('startup bank close failed');
        await Execution.delayTicks(1);
        await Game.openSideTab(3);
        await Execution.delayTicks(1);
        if (!(await Equipment.equip(TOOL))) throw new Error('could not equip real gather tool');
        await Execution.delayTicks(1);

        const before = Inventory.count(PRODUCT);
        const tree = Locs.query()
            .name(TARGET)
            .action(ACTION)
            .withinOf(SITE, 6)
            .nearest();
        if (!tree) throw new Error('no real Draynor tree in gathering range');
        if (!(await tree.interact(ACTION))) throw new Error('could not start real tree gather');
        await Execution.delayUntilTicks(() => Inventory.count(PRODUCT) > before, 80);
        if (Inventory.count(PRODUCT) <= before) {
            throw new Error('real tree gather did not produce Logs within 80 ticks');
        }
        this.log(`gather: produced ${Inventory.count(PRODUCT) - before} Logs`);
    }
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    BankSorter,
    GatheringBot,
}

impl Cell {
    fn name(self) -> &'static str {
        match self {
            Self::BankSorter => "banksorter_sorts_and_reports",
            Self::GatheringBot => "gatheringbot_start_path_purges_withdraws_coins_and_gathers",
        }
    }

    fn seed_commands(self) -> Vec<String> {
        let mut commands = vec![
            "setstat woodcutting 20".into(),
            "setstat attack 20".into(),
            "setstat defence 98".into(),
            "setstat hitpoints 99".into(),
        ];
        match self {
            Self::BankSorter => commands.extend([
                "givebank ashes 1".into(),
                "givebank maple_logs 12".into(),
                "givebank red_bead 1".into(),
                "givebank coins 300".into(),
                "givebank bronze_scimitar 1".into(),
                "givebank iron_ore 1".into(),
                interact::tele_args(DRAYNOR_BANK.2, DRAYNOR_BANK.0, DRAYNOR_BANK.1),
            ]),
            Self::GatheringBot => commands.extend([
                "give bronze_axe 1".into(),
                "give red_bead 1".into(),
                "give ashes 1".into(),
                "givebank coins 300".into(),
                interact::tele_args(DRAYNOR_TREES.2, DRAYNOR_TREES.0, DRAYNOR_TREES.1),
            ]),
        }
        commands
    }

    fn expected_tile(self) -> (i32, i32, i32) {
        match self {
            Self::BankSorter => DRAYNOR_BANK,
            Self::GatheringBot => DRAYNOR_TREES,
        }
    }

    fn settings(self) -> Map<String, Value> {
        let raw = match self {
            Self::BankSorter => json!({
                "sortBank": true,
                "reportQuestJunk": true,
                "dropQuestJunk": false
            }),
            Self::GatheringBot => json!({}),
        };
        raw.as_object().expect("cell settings object").clone()
    }

    fn deadline(self) -> Duration {
        match self {
            Self::BankSorter => SORT_DEADLINE,
            Self::GatheringBot => GATHER_DEADLINE,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct ItemRow {
    slot: i32,
    id: i32,
    name: Option<String>,
    count: i32,
    cost: i32,
}

impl From<&ItemView> for ItemRow {
    fn from(item: &ItemView) -> Self {
        Self {
            slot: item.slot,
            id: item.def.id,
            name: item.def.name.clone(),
            count: item.count,
            cost: item.def.base_value,
        }
    }
}

impl BankSortRow for ItemRow {
    fn slot(&self) -> i32 {
        self.slot
    }

    fn id(&self) -> i32 {
        self.id
    }

    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    fn cost(&self) -> i32 {
        self.cost
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
struct SnapshotSample {
    seq: u64,
    elapsed_ms: u64,
    tick: u32,
    tile: Option<(i32, i32, i32)>,
    bank_open: bool,
    bank_loaded: bool,
    bank: Vec<ItemRow>,
    pack: Vec<ItemRow>,
    varp_304: Option<i32>,
    woodcutting_xp: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
struct TimedLog {
    elapsed_ms: u64,
    tick: u32,
    sample_seq: u64,
    line: String,
}

#[derive(Debug, Serialize)]
struct Receipt {
    cell: &'static str,
    verdict: &'static str,
    product_commit: Option<String>,
    tested_tree_commit: Option<String>,
    catalog_root: Option<String>,
    catalog_commit: Option<String>,
    catalog_card: Option<Value>,
    account: Option<String>,
    settings: Option<Value>,
    seed_commands: Vec<String>,
    preparation: Option<Value>,
    logs: Vec<TimedLog>,
    observations: Vec<SnapshotSample>,
    bank_before_sort: Option<Vec<ItemRow>>,
    bank_after_sort: Option<Vec<ItemRow>>,
    pack_after_purge: Option<Vec<ItemRow>>,
    result: Option<Value>,
    failure: Option<String>,
}

impl Receipt {
    fn new(cell: Cell) -> Self {
        Self {
            cell: cell.name(),
            verdict: "FAIL",
            product_commit: None,
            tested_tree_commit: None,
            catalog_root: None,
            catalog_commit: None,
            catalog_card: None,
            account: None,
            settings: None,
            seed_commands: Vec::new(),
            preparation: None,
            logs: Vec::new(),
            observations: Vec::new(),
            bank_before_sort: None,
            bank_after_sort: None,
            pack_after_purge: None,
            result: None,
            failure: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrepPhase {
    InitialReady,
    InitialTutorial,
    WaitInitialTutorial,
    WaitLogout,
    WaitRelog,
    ReseedTutorial,
    WaitReseed,
    SeedAccount,
    WaitSeed,
    Ready,
}

struct FrameState {
    cell: Cell,
    phase: PrepPhase,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    seed_commands: Vec<String>,
    seed_cursor: usize,
    last_seed_tick: Option<u32>,
    seed_commands_sent: Vec<String>,
    snapshot: GameSnapshot,
    samples: Vec<SnapshotSample>,
    started_at: Instant,
    error: Option<String>,
    ready_sample_seq: Option<u64>,
    initial_varp_304: Option<i32>,
}

impl FrameState {
    fn new(cell: Cell) -> Self {
        Self {
            cell,
            phase: PrepPhase::InitialReady,
            tutorial_reseed: None,
            seed_commands: cell.seed_commands(),
            seed_cursor: 0,
            last_seed_tick: None,
            seed_commands_sent: Vec::new(),
            snapshot: GameSnapshot::new(),
            samples: Vec::new(),
            started_at: Instant::now(),
            error: None,
            ready_sample_seq: None,
            initial_varp_304: None,
        }
    }

    fn elapsed_ms(&self) -> u64 {
        self.started_at
            .elapsed()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        self.record_sample();
        if self.error.is_some() || self.phase == PrepPhase::Ready {
            return;
        }
        if self.phase == PrepPhase::WaitLogout && !client.ingame {
            self.phase = PrepPhase::WaitRelog;
            return;
        }
        if held {
            return;
        }
        if let Err(error) = self.advance(client) {
            self.error = Some(error);
        }
    }

    fn record_sample(&mut self) {
        let snapshot = SnapshotSample {
            seq: self.samples.len() as u64,
            elapsed_ms: self.elapsed_ms(),
            tick: self.snapshot.tick(),
            tile: self.snapshot.tile(),
            bank_open: self.snapshot.bank_component_id() >= 0,
            bank_loaded: self.snapshot.bank_loaded(),
            bank: self.snapshot.bank().iter().map(ItemRow::from).collect(),
            pack: self
                .snapshot
                .inventory()
                .iter()
                .map(ItemRow::from)
                .collect(),
            varp_304: self
                .snapshot
                .varps()
                .iter()
                .find(|varp| varp.index == 304)
                .map(|varp| varp.value),
            woodcutting_xp: self
                .snapshot
                .stats()
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case("woodcutting"))
                .map(|stat| stat.xp),
        };
        let changed = self.samples.last().is_none_or(|previous| {
            previous.tick != snapshot.tick
                || previous.tile != snapshot.tile
                || previous.bank_open != snapshot.bank_open
                || previous.bank_loaded != snapshot.bank_loaded
                || previous.bank != snapshot.bank
                || previous.pack != snapshot.pack
                || previous.varp_304 != snapshot.varp_304
                || previous.woodcutting_xp != snapshot.woodcutting_xp
        });
        if changed {
            self.samples.push(snapshot);
        }
    }

    fn advance(&mut self, client: &mut Client) -> Result<(), String> {
        match self.phase {
            PrepPhase::InitialReady if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                self.phase = PrepPhase::InitialTutorial;
            }
            PrepPhase::InitialTutorial => {
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.phase = PrepPhase::WaitInitialTutorial;
            }
            PrepPhase::WaitInitialTutorial
                if scenario::tutorial::tutorial_confirmed(&self.snapshot, 0) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("logout interface unavailable during seed preparation".into());
                }
                self.phase = PrepPhase::WaitLogout;
            }
            PrepPhase::WaitRelog
                if client.ingame
                    && client.scene_state == 2
                    && self
                        .snapshot
                        .side_tabs()
                        .iter()
                        .any(|tab| tab.index == 3 && tab.available) =>
            {
                self.phase = PrepPhase::ReseedTutorial;
            }
            PrepPhase::ReseedTutorial => {
                let baseline = scenario::tutorial::chat_baseline(&self.snapshot);
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                self.phase = PrepPhase::WaitReseed;
            }
            PrepPhase::WaitReseed => {
                let confirmed = self
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed is armed")
                    .check(&self.snapshot)?;
                if confirmed {
                    self.phase = PrepPhase::SeedAccount;
                }
            }
            PrepPhase::SeedAccount => {
                if self.last_seed_tick == Some(self.snapshot.tick()) {
                    return Ok(());
                }
                let Some(command) = self.seed_commands.get(self.seed_cursor) else {
                    self.phase = PrepPhase::WaitSeed;
                    return Ok(());
                };
                send_cheat(client, command)?;
                self.seed_commands_sent.push(command.clone());
                self.seed_cursor += 1;
                self.last_seed_tick = Some(self.snapshot.tick());
                if self.seed_cursor == self.seed_commands.len() {
                    self.phase = PrepPhase::WaitSeed;
                }
            }
            PrepPhase::WaitSeed => self.qualify_seed(),
            _ => {}
        }
        Ok(())
    }

    fn qualify_seed(&mut self) {
        if !self.snapshot_is_expected() {
            return;
        }
        self.phase = PrepPhase::Ready;
        self.ready_sample_seq = self.samples.last().map(|sample| sample.seq);
        self.initial_varp_304 = self.samples.last().and_then(|sample| sample.varp_304);
    }

    fn snapshot_is_expected(&self) -> bool {
        if self.snapshot.tile() != Some(self.cell.expected_tile()) {
            return false;
        }
        let expected_stats = [
            ("woodcutting", 20),
            ("attack", 20),
            ("defence", 98),
            ("hitpoints", 99),
        ];
        if expected_stats.iter().any(|(name, level)| {
            self.snapshot
                .stats()
                .iter()
                .find(|stat| stat.name.eq_ignore_ascii_case(name))
                .map(|stat| stat.base)
                != Some(*level)
        }) {
            return false;
        }
        if self.cell == Cell::BankSorter
            && self.snapshot.varps().iter().all(|varp| varp.index != 304)
        {
            return false;
        }
        let inventory = self.snapshot.inventory();
        let has_id = |id: i32| {
            inventory
                .iter()
                .any(|item| item.def.id == id && item.count > 0)
        };
        match self.cell {
            Cell::BankSorter => !has_id(RED_BEAD) && !has_id(ASHES),
            Cell::GatheringBot => {
                let bronze = inventory
                    .iter()
                    .any(|item| item.def.name.as_deref() == Some("Bronze axe") && item.count > 0);
                bronze && has_id(RED_BEAD) && has_id(ASHES)
            }
        }
    }
}

struct PreparedCard {
    js: String,
    shape: script_js::LoadShape,
    settings: Map<String, Value>,
    siblings: Vec<(String, String)>,
    identity: Value,
}

fn required(name: &str) -> Result<String, String> {
    std::env::var(name).map_err(|_| format!("{name} is required for the live cell"))
}

fn required_path(name: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(required(name)?);
    if !path.is_absolute() {
        return Err(format!("{name} must be absolute: {}", path.display()));
    }
    Ok(path)
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| format!("run git in {}: {error}", root.display()))?;
    if !output.status.success() {
        return Err(format!(
            "git -C {} {} failed: {}",
            root.display(),
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map(|value| value.trim().to_owned())
        .map_err(|error| format!("git output is not UTF-8: {error}"))
}

fn verify_product_commit() -> Result<(String, String), String> {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let product = required("COMPAT_BANK_PRODUCT_COMMIT")?;
    let test_commit = git_output(&repo, &["rev-parse", "HEAD"])?;
    let product_full = git_output(
        &repo,
        &["rev-parse", "--verify", &format!("{product}^{{commit}}")],
    )?;
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["merge-base", "--is-ancestor", &product_full, &test_commit])
        .status()
        .map_err(|error| format!("check tested product commit ancestry: {error}"))?;
    if !ancestor.success() {
        return Err(format!(
            "product commit {product_full} is not an ancestor of tested tree {test_commit}"
        ));
    }
    Ok((product_full, test_commit))
}

fn validate_inputs(
    nav_pack: &Path,
    engine_dir: &Path,
    catalog_root: &Path,
    cache_dir: &Path,
    evidence_dir: &Path,
    isolated_home: &Path,
) -> Result<(), String> {
    for (name, path, file) in [
        ("WORLD_NAV_PACK", nav_pack, true),
        ("WORLD_ENGINE_DIR", engine_dir, false),
        ("RS2B0T", catalog_root, false),
        ("BOT_CACHE_DIR", cache_dir, false),
    ] {
        if file && !path.is_file() {
            return Err(format!("{name} is not a file: {}", path.display()));
        }
        if !file && !path.is_dir() {
            return Err(format!("{name} is not a directory: {}", path.display()));
        }
    }
    if evidence_dir.starts_with(isolated_home) {
        return Err("LIVE_EVIDENCE_DIR must be outside the throwaway HOME".into());
    }
    std::fs::create_dir_all(evidence_dir).map_err(|error| {
        format!(
            "create evidence directory {}: {error}",
            evidence_dir.display()
        )
    })
}

fn selected_profile(
    nav_pack: PathBuf,
    engine_dir: PathBuf,
    catalog_root: PathBuf,
    cache_dir: PathBuf,
    temp: &Path,
) -> Result<(Arc<host_play::ServerProfile>, Arc<SharedClientTemplate>), String> {
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("WORLD_GAME_PORT")
                .ok()
                .and_then(|port| port.parse().ok())
                .unwrap_or(44594),
        ),
        http_port: Some(
            std::env::var("WORLD_HTTP_PORT")
                .ok()
                .and_then(|port| port.parse().ok())
                .unwrap_or(1080),
        ),
        cache_dir: Some(cache_dir),
        nav_pack: Some(nav_pack),
        engine_dir: Some(engine_dir),
        unpack_dir: Some(temp.join("unpack")),
        vault_path: Some(temp.join("vault")),
        catalog_root: Some(catalog_root),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind_runtime()?;
    if profile.profile_class() != host_play::ProfileClass::Local
        || profile.client().game_host() != "127.0.0.1"
    {
        return Err("bank compatibility proof requires a loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    if template.world().is_none() {
        return Err("selected 289 template has no navigation world".into());
    }
    Ok((profile, template))
}

fn prepare_catalog_card(root: &Path, temp: &Path) -> Result<(PreparedCard, String), String> {
    let catalog_commit = git_output(root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected rs2b0t catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    let card_name = "BankSorter";
    let source_path = root.join("src/bot/scripts/BankSorter/BankSorter.ts");
    if !source_path.is_file() {
        return Err(format!(
            "catalog source is missing: {}",
            source_path.display()
        ));
    }
    let source_sha = script_js::JsCache::origin_sha(
        &std::fs::read(&source_path)
            .map_err(|error| format!("read {}: {error}", source_path.display()))?,
    );
    let mut library =
        script_js::JsLibrary::with_cache(temp.join("js-scripts.json"), temp.join("js-cache"));
    if library.register_rs2b0t(root, &temp.join("rs2b0t-path"))? == 0 {
        return Err("pinned catalog registered no scripts".into());
    }
    library.ensure_js(script::ScriptSource::Catalog, card_name)?;
    let card = library
        .get(script::ScriptSource::Catalog, card_name)
        .cloned()
        .ok_or_else(|| format!("pinned catalog has no {card_name} card"))?;
    let expected_source = source_path
        .canonicalize()
        .map_err(|error| format!("canonicalize catalog source: {error}"))?;
    let loaded_source = card
        .path
        .canonicalize()
        .map_err(|error| format!("canonicalize loaded catalog source: {error}"))?;
    if loaded_source != expected_source || card.sha256 != source_sha {
        return Err(format!(
            "catalog identity mismatch for {card_name}: source={}, loaded={}, expected_sha={}, loaded_sha={}",
            expected_source.display(),
            loaded_source.display(),
            source_sha,
            card.sha256
        ));
    }
    if let Some(import) = &card.unloadable {
        return Err(format!("{card_name} is unloadable: {import}"));
    }
    let explicit = Cell::BankSorter.settings();
    for key in explicit.keys() {
        if !card.settings_schema.iter().any(|field| field.id == *key) {
            return Err(format!(
                "{card_name} catalog settings do not declare {key:?}"
            ));
        }
    }
    let settings = script::merge_bag(&card.settings_schema, &Map::new(), Some(&explicit));
    let siblings = script_js::resolve_sibling_modules(
        &card.path,
        &card.origin,
        library.cache(),
        script_js::CacheMeta {
            kind: card.kind,
            source: card.source,
            shape: None,
            api_family: Some(card.api_family.as_str().into()),
        },
    )?;
    let sibling_hashes = siblings
        .iter()
        .map(|(module, source)| {
            json!({
                "module": module,
                "compiled_sha256": script_js::JsCache::origin_sha(source.as_bytes())
            })
        })
        .collect::<Vec<_>>();
    let identity = json!({
        "registered_card": card_name,
        "shared_class": "BankSorter",
        "source_path": card.path,
        "source_sha256": card.sha256,
        "compiled_sha256": script_js::JsCache::origin_sha(card.js.as_bytes()),
        "load_shape": format!("{:?}", card.shape),
        "siblings": sibling_hashes,
        "settings": Value::Object(settings.clone())
    });
    Ok((
        PreparedCard {
            js: card.js,
            shape: card.shape,
            settings,
            siblings,
            identity,
        },
        catalog_commit,
    ))
}

fn prepare_local_gathering_source(root: &Path) -> Result<(Value, String), String> {
    let catalog_commit = git_output(root, &["rev-parse", "--short=10", "HEAD"])?;
    if !catalog_commit.starts_with(CATALOG_SHORT) {
        return Err(format!(
            "expected rs2b0t reference catalog {CATALOG_SHORT}, found {catalog_commit}"
        ));
    }
    let reference_source = root.join("src/bot/scripts/GatheringBot/GatheringBot.ts");
    if !reference_source.is_file() {
        return Err(format!(
            "GatheringBot reference source is missing: {}",
            reference_source.display()
        ));
    }
    let reference_sha = script_js::JsCache::origin_sha(
        &std::fs::read(&reference_source)
            .map_err(|error| format!("read {}: {error}", reference_source.display()))?,
    );
    let identity = json!({
        "script_kind": "local-source",
        "script_name": "GatheringBotStartPathProbe",
        "source_sha256": script_js::JsCache::origin_sha(GATHERING_PROBE_SOURCE.as_bytes()),
        "load_shape": "CompatClass",
        "reference_catalog_commit": catalog_commit.clone(),
        "reference_source_path": reference_source.display().to_string(),
        "reference_source_sha256": reference_sha,
        "probe_options": {
            "keep": ["Bronze axe"],
            "coin_target": 200,
            "target": "Tree",
            "action": "Chop down",
            "site": "Draynor (trees)"
        }
    });
    Ok((identity, catalog_commit))
}

fn send_cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture command refused: {command}"))
    }
}

fn take_logs(
    play: &host_play::Play,
    account: &str,
    state: &Arc<Mutex<FrameState>>,
    receipt: &mut Receipt,
) {
    let lines = play.script_take_pending_logs(account);
    let frame = state.lock().unwrap_or_else(|error| error.into_inner());
    let sample = frame.samples.last();
    let elapsed_ms = frame.elapsed_ms();
    let tick = sample.map_or(0, |sample| sample.tick);
    let sample_seq = sample.map_or(0, |sample| sample.seq);
    receipt.logs.extend(lines.into_iter().map(|line| TimedLog {
        elapsed_ms,
        tick,
        sample_seq,
        line,
    }));
    receipt.observations.clone_from(&frame.samples);
    receipt.seed_commands.clone_from(&frame.seed_commands_sent);
}

fn wait_for_seed(state: &Arc<Mutex<FrameState>>, receipt: &mut Receipt) -> Result<(), String> {
    let deadline = Instant::now() + PREPARATION_DEADLINE;
    loop {
        let frame = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &frame.error {
            receipt.observations.clone_from(&frame.samples);
            receipt.seed_commands.clone_from(&frame.seed_commands_sent);
            return Err(format!("seed preparation failed: {error}"));
        }
        if frame.phase == PrepPhase::Ready {
            let prep = json!({
                "tutorial_relogin_confirmed": true,
                "seed_ready_sample_seq": frame.ready_sample_seq,
                "starting_varp_304": frame.initial_varp_304,
                "observed_tile": frame.snapshot.tile(),
                "observed_stats": frame.snapshot.stats(),
                "observed_pack": frame.snapshot.inventory().iter().map(ItemRow::from).collect::<Vec<_>>(),
                "commands_sent": frame.seed_commands_sent
            });
            receipt.preparation = Some(prep);
            receipt.observations.clone_from(&frame.samples);
            receipt.seed_commands.clone_from(&frame.seed_commands_sent);
            return Ok(());
        }
        let phase = frame.phase;
        receipt.observations.clone_from(&frame.samples);
        drop(frame);
        if Instant::now() >= deadline {
            return Err(format!(
                "seed preparation deadline expired in phase {phase:?}"
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn item_count(rows: &[ItemRow], id: i32) -> i32 {
    rows.iter()
        .filter(|row| row.id == id)
        .map(|row| row.count.max(0))
        .sum()
}

fn bank_rows(
    receipt: &Receipt,
    predicate: impl Fn(&SnapshotSample) -> bool,
) -> Option<Vec<ItemRow>> {
    receipt
        .observations
        .iter()
        .filter(|sample| predicate(sample))
        .last()
        .map(|sample| sample.bank.clone())
}

fn bank_category(row: &ItemRow) -> BankCategory {
    if row.id == RED_BEAD {
        BankCategory::QuestLive
    } else {
        category_of(row)
    }
}

fn planner_order(rows: &[ItemRow]) -> Vec<i32> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| {
        let category = bank_category(row);
        let category_index = CATEGORY_ORDER
            .iter()
            .position(|entry| *entry == category)
            .unwrap_or(usize::MAX);
        (category_index, rank_within(category, row), row.slot)
    });
    sorted.into_iter().map(|row| row.id).collect()
}

fn check_no_error_logs(logs: &[TimedLog]) -> Result<(), String> {
    let bad = logs.iter().find(|entry| {
        let line = entry.line.to_ascii_lowercase();
        line.contains("not impl")
            || line.contains("throw")
            || line.contains("uncaught")
            || line.contains("exception")
    });
    if let Some(bad) = bad {
        Err(format!(
            "error-like script log at tick {}: {}",
            bad.tick, bad.line
        ))
    } else {
        Ok(())
    }
}

fn bank_sort_result(receipt: &mut Receipt) -> Result<(), String> {
    let junk =
        api::quest_junk::quest_junk().map_err(|error| format!("quest-junk catalog: {error:?}"))?;
    if !junk.iter().any(|entry| {
        entry.id == QUEST_RED_BEAD && entry.name == "Red bead" && entry.quest == "Imp Catcher"
    }) {
        return Err("red bead seed is not present in the generated quest-junk catalog".into());
    }
    let initial = receipt
        .observations
        .iter()
        .find(|sample| sample.bank_open && sample.bank_loaded)
        .map(|sample| sample.bank.clone())
        .ok_or_else(|| "BankSorter never exposed a loaded bank snapshot".to_string())?;
    let expected_seed = [
        "Ashes",
        "Maple logs",
        "Red bead",
        "Coins",
        "Bronze scimitar",
        "Iron ore",
    ];
    for name in expected_seed {
        if !initial.iter().any(|row| row.name.as_deref() == Some(name)) {
            return Err(format!("seeded bank row {name:?} is missing: {initial:?}"));
        }
    }
    if !initial
        .iter()
        .any(|row| row.id == ASHES && is_unmatched(row))
    {
        return Err("Ashes did not exercise the planner's unmatched-item path".into());
    }
    let expected_ids = planner_order(&initial);
    let initial_ids = initial.iter().map(|row| row.id).collect::<Vec<_>>();
    if initial_ids == expected_ids {
        return Err(
            "bank fixture was already in planner order; sort proof would be vacuous".into(),
        );
    }
    receipt.bank_before_sort = Some(initial.clone());

    let summary = receipt
        .logs
        .iter()
        .find(|entry| entry.line.starts_with("BankSorter: sorted,"))
        .ok_or_else(|| "BankSorter did not log a sorted result".to_string())?;
    let moves = summary
        .line
        .strip_prefix("BankSorter: sorted, ")
        .and_then(|rest| rest.split_once(" moves"))
        .and_then(|(count, _)| count.parse::<u32>().ok())
        .ok_or_else(|| {
            format!(
                "could not parse positive move count from {:?}",
                summary.line
            )
        })?;
    if moves == 0 {
        return Err("BankSorter reported zero sort moves".into());
    }
    let report = receipt
        .logs
        .iter()
        .find(|entry| entry.line.contains("BankSorter: quest leftovers"))
        .ok_or_else(|| "BankSorter did not log its quest-junk report".to_string())?;
    let report_lower = report.line.to_ascii_lowercase();
    if !report.line.contains("Red bead")
        || !report.line.contains("Imp Catcher")
        || !(report_lower.contains("not started") || report_lower.contains("notstarted"))
    {
        return Err(format!(
            "quest-junk report omitted the live Red bead: {}",
            report.line
        ));
    }
    let unfiled = summary.line.contains("1 unfiled");
    if !unfiled {
        return Err(format!(
            "summary did not report exactly the unmatched Ashes row: {}",
            summary.line
        ));
    }
    let after = bank_rows(receipt, |sample| sample.bank_open && sample.bank_loaded)
        .ok_or_else(|| "no loaded bank snapshot after BankSorter ran".to_string())?;
    let after_ids = after.iter().map(|row| row.id).collect::<Vec<_>>();
    if after_ids != expected_ids {
        return Err(format!(
            "bank order differs from CATEGORY_ORDER/rank planner: expected {expected_ids:?}, observed {after_ids:?}"
        ));
    }
    let start_mode = receipt
        .preparation
        .as_ref()
        .and_then(|prep| prep.get("starting_varp_304"))
        .and_then(Value::as_i64)
        .ok_or_else(|| "arrange-mode varp 304 was not recorded before Start".to_string())?;
    let end_mode = receipt
        .observations
        .iter()
        .rev()
        .find_map(|sample| sample.varp_304)
        .ok_or_else(|| "arrange-mode varp 304 was not observed after sorting".to_string())?;
    if i64::from(end_mode) != start_mode {
        return Err(format!("varp 304 changed from {start_mode} to {end_mode}"));
    }
    let round_logs = receipt
        .logs
        .iter()
        .filter(|entry| entry.line.contains("bank sort: round"))
        .collect::<Vec<_>>();
    if round_logs.len() != moves as usize {
        return Err(format!(
            "sort summary has {moves} moves but {} bank-sort round receipts were observed",
            round_logs.len()
        ));
    }
    let accepted = round_logs
        .iter()
        .filter(|entry| entry.line.contains(", 1 sent,"))
        .count();
    if accepted != moves as usize {
        return Err(format!(
            "only {accepted} of {moves} sort requests were observed sent: {round_logs:?}"
        ));
    }
    let mut per_tick = BTreeMap::<u32, usize>::new();
    for entry in &round_logs {
        *per_tick.entry(entry.tick).or_default() += 1;
    }
    if let Some((tick, count)) = per_tick.iter().find(|(_, count)| **count > 1) {
        return Err(format!(
            "{count} bank drags were logged on observed game tick {tick}"
        ));
    }
    check_no_error_logs(&receipt.logs)?;
    receipt.bank_after_sort = Some(after.clone());
    receipt.result = Some(json!({
        "sorted": true,
        "moves": moves,
        "reported_reason": "sorted",
        "report_line": report.line,
        "unmatched_rows": 1,
        "expected_bank_order_ids": expected_ids,
        "observed_bank_order_ids": after_ids,
        "starting_varp_304": start_mode,
        "ending_varp_304": end_mode,
        "drag_log_count_by_observed_tick": per_tick,
        "summary_line": summary.line,
        "basis": "the catalog logs reason `sorted` and positive moves; the loaded bank matches the shared planner"
    }));
    Ok(())
}

fn parse_coin_withdraw(line: &str) -> Option<i32> {
    let rest = line.trim_start().strip_prefix("acquire: withdraw ")?;
    let (amount, _) = rest.split_once("gp")?;
    amount.trim().parse().ok()
}

fn bank_action_events(samples: &[SnapshotSample]) -> Vec<&SnapshotSample> {
    samples
        .windows(2)
        .filter_map(|pair| {
            let previous = &pair[0];
            let current = &pair[1];
            (previous.bank_open != current.bank_open
                || previous.bank_loaded != current.bank_loaded
                || previous.bank != current.bank
                || previous.pack != current.pack)
                .then_some(current)
        })
        .collect()
}

fn observed_pace_gap(samples: &[SnapshotSample], logs: &[TimedLog]) -> Option<Value> {
    let actions = bank_action_events(samples);
    for pause in logs
        .iter()
        .filter(|entry| entry.line.trim_start() == "bank: pause 1t")
    {
        let before = actions
            .iter()
            .copied()
            .rfind(|sample| sample.seq <= pause.sample_seq);
        let after = actions
            .iter()
            .copied()
            .find(|sample| sample.seq > pause.sample_seq);
        if let (Some(before), Some(after)) = (before, after) {
            if after.tick > before.tick {
                return Some(json!({
                    "pause_log": pause,
                    "previous_bank_action": before,
                    "next_bank_action": after,
                    "observed_tick_delta": after.tick - before.tick
                }));
            }
        }
    }
    None
}

fn gathering_result(receipt: &mut Receipt, script_started_ms: u64) -> Result<(), String> {
    let clear_log = receipt
        .logs
        .iter()
        .find(|entry| entry.line.contains("start purge: pack cleared"))
        .ok_or_else(|| "GatheringBot did not log `start purge: pack cleared`".to_string())?;
    let start_log = receipt
        .logs
        .iter()
        .find(|entry| entry.line.contains("start purge: banking "))
        .ok_or_else(|| "GatheringBot did not log its startup purge count".to_string())?;
    let stack_count = start_log
        .line
        .split("start purge: banking ")
        .nth(1)
        .and_then(|rest| rest.split_once(" stack(s)").map(|(count, _)| count))
        .and_then(|count| count.parse::<u32>().ok())
        .ok_or_else(|| {
            format!(
                "could not parse startup purge stack count: {}",
                start_log.line
            )
        })?;
    if stack_count == 0 {
        return Err("startup purge did not contain any junk stacks".into());
    }
    let initial = receipt
        .preparation
        .as_ref()
        .and_then(|prep| prep.get("observed_pack"))
        .and_then(Value::as_array)
        .ok_or_else(|| "seeded pack was not recorded".to_string())?;
    let keep_ids = initial
        .iter()
        .filter_map(|row| {
            let name = row.get("name")?.as_str()?;
            (name == "Bronze axe")
                .then(|| row.get("id")?.as_i64().map(|id| id as i32))
                .flatten()
        })
        .collect::<Vec<_>>();
    if keep_ids.len() != 1 {
        return Err(format!(
            "expected seeded Bronze axe as the only kept item: {initial:?}"
        ));
    }
    if !initial
        .iter()
        .any(|row| row.get("id").and_then(Value::as_i64) == Some(i64::from(RED_BEAD)))
        || !initial
            .iter()
            .any(|row| row.get("id").and_then(Value::as_i64) == Some(i64::from(ASHES)))
    {
        return Err(format!(
            "junk pack did not contain Red bead and Ashes: {initial:?}"
        ));
    }

    let withdraw = receipt
        .logs
        .iter()
        .find_map(|entry| parse_coin_withdraw(&entry.line).map(|amount| (entry, amount)))
        .ok_or_else(|| "GatheringBot probe did not log `acquire: withdraw Ngp`".to_string())?;
    if withdraw.1 != 200 {
        return Err(format!(
            "expected startup coin target 200gp, got {}gp",
            withdraw.1
        ));
    }
    let pack_after = receipt
        .observations
        .iter()
        .filter(|sample| {
            sample.seq >= clear_log.sample_seq
                && sample.elapsed_ms >= clear_log.elapsed_ms
                && sample.seq < withdraw.0.sample_seq
        })
        .find(|sample| {
            !sample
                .pack
                .iter()
                .any(|row| row.id == RED_BEAD || row.id == ASHES)
        })
        .map(|sample| sample.pack.clone())
        .ok_or_else(|| "no observed post-purge pack snapshot before coin withdrawal".to_string())?;
    if pack_after.iter().any(|row| !keep_ids.contains(&row.id)) {
        return Err(format!(
            "post-purge pack contains an item outside the kept tool: {pack_after:?}"
        ));
    }
    receipt.pack_after_purge = Some(pack_after.clone());
    let max_held_coins = receipt
        .observations
        .iter()
        .map(|sample| item_count(&sample.pack, COINS))
        .max()
        .unwrap_or_default();
    if max_held_coins < withdraw.1 {
        return Err(format!(
            "held coins peaked at {max_held_coins}gp, below requested {}gp",
            withdraw.1
        ));
    }
    let initial_bank = receipt
        .observations
        .iter()
        .find(|sample| sample.bank_open && sample.bank_loaded)
        .map(|sample| sample.bank.clone())
        .ok_or_else(|| "GatheringBot did not observe its loaded bank".to_string())?;
    if item_count(&initial_bank, COINS) < withdraw.1 {
        return Err(format!(
            "bank seed had {} coins but the script requested {}",
            item_count(&initial_bank, COINS),
            withdraw.1
        ));
    }
    let pace_gap = observed_pace_gap(&receipt.observations, &receipt.logs).ok_or_else(|| {
        "bank: pause 1t had no observed bank actions separated by a game tick".to_string()
    })?;

    let initial_logs = receipt
        .preparation
        .as_ref()
        .and_then(|prep| prep.get("observed_pack"))
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| row.get("id").and_then(Value::as_i64) == Some(i64::from(LOGS)))
                .map(|row| row.get("count").and_then(Value::as_i64).unwrap_or_default() as i32)
                .sum::<i32>()
        })
        .unwrap_or_default();
    let gathered = receipt
        .observations
        .iter()
        .filter(|sample| sample.elapsed_ms >= script_started_ms)
        .find(|sample| {
            item_count(&sample.pack, LOGS) > initial_logs
                && sample.woodcutting_xp.is_some_and(|xp| {
                    receipt
                        .observations
                        .iter()
                        .rev()
                        .find(|before| before.elapsed_ms <= script_started_ms)
                        .and_then(|before| before.woodcutting_xp)
                        .is_some_and(|baseline| xp > baseline)
                })
                && sample.tile.is_some_and(|(x, z, level)| {
                    level == DRAYNOR_TREES.2
                        && (x - DRAYNOR_TREES.0).abs() <= 20
                        && (z - DRAYNOR_TREES.1).abs() <= 20
                })
        })
        .ok_or_else(|| {
            "GatheringBot did not gather a Logs resource at the seeded real Draynor tree site"
                .to_string()
        })?;
    check_no_error_logs(&receipt.logs)?;
    receipt.result = Some(json!({
        "purge_log": start_log,
        "purge_cleared_log": clear_log,
        "purge_stack_count": stack_count,
        "kept_item_ids": keep_ids,
        "pack_after_purge": pack_after,
        "coin_withdraw_log": withdraw.0,
        "requested_coins": withdraw.1,
        "max_observed_held_coins": max_held_coins,
        "bank_pace_gap": pace_gap,
        "gathered_resource": {
            "item_id": LOGS,
            "name": "Logs",
            "observed_count": item_count(&gathered.pack, LOGS),
            "tick": gathered.tick,
            "tile": gathered.tile,
            "woodcutting_xp": gathered.woodcutting_xp
        },
        "site": "Draynor (trees)",
        "reference_script": "GatheringBot",
        "probe_model": "local-source LoopingBot",
    }));
    Ok(())
}
fn gathering_gate_inputs_ready(receipt: &Receipt, script_started_ms: u64) -> bool {
    let has_purge_start = receipt
        .logs
        .iter()
        .any(|entry| entry.line.contains("start purge: banking "));
    let has_purge_clear = receipt
        .logs
        .iter()
        .any(|entry| entry.line.contains("start purge: pack cleared"));
    let has_coin_withdraw = receipt
        .logs
        .iter()
        .any(|entry| parse_coin_withdraw(&entry.line).is_some());
    let has_pace = receipt
        .logs
        .iter()
        .any(|entry| entry.line.trim_start() == "bank: pause 1t");
    let baseline_xp = receipt
        .observations
        .iter()
        .rev()
        .find(|sample| sample.elapsed_ms <= script_started_ms)
        .and_then(|sample| sample.woodcutting_xp);
    let has_gathered = receipt.observations.iter().any(|sample| {
        sample.elapsed_ms >= script_started_ms
            && item_count(&sample.pack, LOGS) > 0
            && sample
                .woodcutting_xp
                .zip(baseline_xp)
                .is_some_and(|(current, initial)| current > initial)
            && sample.tile.is_some_and(|(x, z, level)| {
                level == DRAYNOR_TREES.2
                    && (x - DRAYNOR_TREES.0).abs() <= 20
                    && (z - DRAYNOR_TREES.1).abs() <= 20
            })
    });
    has_purge_start && has_purge_clear && has_coin_withdraw && has_pace && has_gathered
}

fn run_cell(cell: Cell, receipt: &mut Receipt) -> Result<(), String> {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return Err("LIVE=1 is required; ignored live tests must not pass vacuously".into());
    }
    if std::env::var("BOT_CPU").as_deref() != Ok("1") {
        return Err("BOT_CPU=1 is required for headless live scene capture".into());
    }
    if std::env::var("BOT_NAV_BUILD").as_deref() != Ok("skip") {
        return Err("BOT_NAV_BUILD=skip and the explicit v17 WORLD_NAV_PACK are required".into());
    }
    let name_prefix = required("BOT_LIVE_NAME_PREFIX")?;
    if name_prefix.is_empty() || name_prefix.len() > 4 {
        return Err("BOT_LIVE_NAME_PREFIX must contain one to four characters".into());
    }
    let (product_commit, tested_tree) = verify_product_commit()?;
    receipt.product_commit = Some(product_commit);
    receipt.tested_tree_commit = Some(tested_tree);
    let nav_pack = required_path("WORLD_NAV_PACK")?;
    let engine_dir = required_path("WORLD_ENGINE_DIR")?;
    let catalog_root = required_path("RS2B0T")?;
    let cache_dir = required_path("BOT_CACHE_DIR")?;
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    receipt.catalog_root = Some(catalog_root.display().to_string());
    let isolated = script::IsolatedEnv::enter(cell.name());
    validate_inputs(
        &nav_pack,
        &engine_dir,
        &catalog_root,
        &cache_dir,
        &evidence_dir,
        &isolated.home,
    )?;
    isolated.set_rs2b0t(&catalog_root);
    api::hostlog::set_debug(true);

    let names = mint_live_names(1);
    if names.len() != 1 {
        return Err("could not mint one live account name".into());
    }
    let entries = mint_live_entries(&names);
    if entries.len() != 1 {
        return Err("could not mint one live account credential".into());
    }
    let (account, password) = &entries[0];
    receipt.account = Some(account.clone());
    let temp = isolated.home.join("compat-bank-live").join(account);
    std::fs::create_dir_all(&temp)
        .map_err(|error| format!("create per-account cache {}: {error}", temp.display()))?;
    let (_profile, template) =
        selected_profile(nav_pack, engine_dir, catalog_root.clone(), cache_dir, &temp)?;
    let prepared_card = if cell == Cell::BankSorter {
        let (card, catalog_commit) = prepare_catalog_card(&catalog_root, &temp)?;
        receipt.catalog_commit = Some(catalog_commit);
        receipt.catalog_card = Some(card.identity.clone());
        receipt.settings = Some(Value::Object(card.settings.clone()));
        Some(card)
    } else {
        let (identity, catalog_commit) = prepare_local_gathering_source(&catalog_root)?;
        receipt.catalog_commit = Some(catalog_commit);
        receipt.catalog_card = Some(identity);
        None
    };

    let frame_state = Arc::new(Mutex::new(FrameState::new(cell)));
    let callback_state = Arc::clone(&frame_state);
    let callback_account = account.clone();
    let mut play = run_with_template(
        Arc::clone(&template),
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            if username == callback_account {
                callback_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .frame(client, frame.hold);
            }
        },
    )?;
    let account_profile = Profile {
        username: account.clone(),
        password: password.clone().into(),
        uid: (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("clock: {error}"))?
            .as_millis()
            % i32::MAX as u128) as i32,
        settings: ProfileSettings::default(),
    };
    play.try_spawn_slot(account_profile, None, None, None)?;
    play.focus(account);
    wait_for_seed(&frame_state, receipt)?;

    match prepared_card {
        Some(card) => {
            play.script_start_handle().start_load(
                account,
                card.js,
                card.shape,
                Some(card.settings),
                card.siblings,
            )?;
        }
        None => {
            play.script_start_load(
                account,
                GATHERING_PROBE_SOURCE.to_owned(),
                script_js::LoadShape::CompatClass,
                None,
                vec![],
            )?;
        }
    }
    let script_started_ms = frame_state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .elapsed_ms();
    let deadline = Instant::now() + cell.deadline();
    let result = loop {
        take_logs(&play, account, &frame_state, receipt);
        if let Some(error) = play.script_last_error(account) {
            // BankSorter ends through ScriptRunner.stop; only its exact success reason is normal.
            let expected_bank_sort_stop = cell == Cell::BankSorter
                && error.starts_with("script requested stop on tick ")
                && error.ends_with(": BankSorter: sorted");
            if !expected_bank_sort_stop {
                let source_kind = if cell == Cell::BankSorter {
                    "catalog"
                } else {
                    "local-source"
                };
                break Err(format!("{source_kind} script threw: {error}"));
            }
        }
        if let Err(error) = check_no_error_logs(&receipt.logs) {
            break Err(error);
        }
        let frame = frame_state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &frame.error {
            break Err(format!("live frame preparation failed: {error}"));
        }
        receipt.observations.clone_from(&frame.samples);
        drop(frame);
        match cell {
            Cell::BankSorter => {
                let summary_seen = receipt
                    .logs
                    .iter()
                    .any(|entry| entry.line.starts_with("BankSorter: sorted,"));
                let state = play.script_state(account);
                let latest = frame_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .samples
                    .last()
                    .cloned();
                if summary_seen
                    && state == script::RunState::Idle
                    && latest.as_ref().is_some_and(|sample| !sample.bank_open)
                {
                    break bank_sort_result(receipt);
                }
                if state == script::RunState::Idle
                    && receipt
                        .logs
                        .iter()
                        .any(|entry| entry.line.starts_with("BankSorter:"))
                    && !summary_seen
                {
                    break Err("BankSorter stopped without a successful sort result".into());
                }
            }
            Cell::GatheringBot => {
                if gathering_gate_inputs_ready(receipt, script_started_ms) {
                    match gathering_result(receipt, script_started_ms) {
                        Ok(()) => break Ok(()),
                        Err(error)
                            if error.starts_with("no observed post-purge")
                                || error.starts_with("bank: pause 1t had no observed")
                                || error.starts_with("GatheringBot did not gather") => {}
                        Err(error) => break Err(error),
                    }
                }
            }
        }
        if Instant::now() >= deadline {
            break Err(format!("{} fixed live deadline expired", cell.name()));
        }
        if play.script_state(account) == script::RunState::Error {
            break Err(format!(
                "script entered Error state: {:?}",
                play.script_last_error(account)
            ));
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    take_logs(&play, account, &frame_state, receipt);
    if cell == Cell::GatheringBot && result.is_ok() {
        play.script_stop(account);
        let stop_deadline = Instant::now() + STOP_DEADLINE;
        while play.script_state(account) != script::RunState::Idle && Instant::now() < stop_deadline
        {
            take_logs(&play, account, &frame_state, receipt);
            std::thread::sleep(POLL_INTERVAL);
        }
        if play.script_state(account) != script::RunState::Idle {
            play.stop_slot(account);
            return Err("GatheringBot did not stop within the cleanup deadline".into());
        }
    }
    receipt.observations = frame_state
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .samples
        .clone();
    if cell == Cell::BankSorter {
        receipt.bank_before_sort = receipt
            .observations
            .iter()
            .find(|sample| sample.bank_open && sample.bank_loaded)
            .map(|sample| sample.bank.clone());
        receipt.bank_after_sort =
            bank_rows(receipt, |sample| sample.bank_open && sample.bank_loaded);
    }
    result
}

fn evidence_file(evidence_dir: &Path, receipt: &Receipt) -> Result<PathBuf, String> {
    let path = evidence_dir.join("receipt.json");
    let bytes = serde_json::to_vec_pretty(receipt)
        .map_err(|error| format!("serialize receipt: {error}"))?;
    std::fs::write(&path, bytes)
        .map_err(|error| format!("write evidence {}: {error}", path.display()))?;
    Ok(path)
}

fn run_and_record(cell: Cell) -> Result<(), String> {
    let mut receipt = Receipt::new(cell);
    let evidence_dir = required_path("LIVE_EVIDENCE_DIR")?;
    std::fs::create_dir_all(&evidence_dir).map_err(|error| {
        format!(
            "create evidence directory {}: {error}",
            evidence_dir.display()
        )
    })?;
    let result = run_cell(cell, &mut receipt);
    match result {
        Ok(()) => {
            receipt.verdict = "PASS";
            let path = evidence_file(&evidence_dir, &receipt)?;
            println!(
                "PASS {} account={} product={} evidence={}",
                cell.name(),
                receipt.account.as_deref().unwrap_or("unknown"),
                receipt.product_commit.as_deref().unwrap_or("unknown"),
                path.display()
            );
            Ok(())
        }
        Err(error) => {
            receipt.verdict = "FAIL";
            receipt.failure = Some(error.clone());
            let path = evidence_file(&evidence_dir, &receipt)?;
            Err(format!(
                "{} failed: {error}; evidence={}",
                cell.name(),
                path.display()
            ))
        }
    }
}

#[test]
#[ignore = "requires LIVE=1, a throwaway HOME, the pinned catalog, and real 289 Engine L"]
fn banksorter_sorts_and_reports() {
    run_and_record(Cell::BankSorter).unwrap_or_else(|error| panic!("{error}"));
}

#[test]
#[ignore = "requires LIVE=1, a throwaway HOME, the pinned catalog, and real 289 Engine L"]
fn gatheringbot_start_path_purges_withdraws_coins_and_gathers() {
    run_and_record(Cell::GatheringBot).unwrap_or_else(|error| panic!("{error}"));
}
