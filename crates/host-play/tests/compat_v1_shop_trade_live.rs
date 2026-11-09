//! Engine L live cells for Shop and paired Trade.
#![cfg(feature = "live-harness")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact;
use api::snapshot::GameSnapshot;
use client::client::Client;
use host_play::{mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const GAME_PORT: u16 = 47_594;
const HTTP_PORT: u16 = 3_080;
const SHOP_CELL_DEADLINE: Duration = Duration::from_secs(240);
const PREP_DEADLINE: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const GERRANT: (i32, i32, i32) = (3013, 3224, 0);
const SHOP_SOURCE: &str = r#"
import { Inventory } from '../../api/inventory/Inventory.js';
import { Execution } from '../../api/execution/Execution.js';
import { Shop } from '../../api/shop/Shop.js';

export default class CompatShopBuyByIdLive extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;

        await Shop.open('Gerrant');
        if (!Shop.isOpen()) throw new Error('Gerrant shop did not open');
        this.log('SHOP_OPENED');

        const insufficient = await Shop.buyById(311, 1);
        this.log('INSUFFICIENT:' + insufficient);
        await Execution.delayUntilTicks(() => Inventory.count('Coins') >= 100, 40);

        const exact = await Shop.buyById(314, 1);
        this.log('EXACT:' + JSON.stringify({
            result: exact,
            feathers: Inventory.count('Feather'),
        }));

        const net = Shop.stock().find(row =>
            row.name?.toLowerCase().includes('net')
            && !row.name?.toLowerCase().includes('big')
        );
        if (!net || net.count < 1) throw new Error('Gerrant has no small fishing net stock');
        const requested = net.count + 1;
        const partial = await Shop.buyById(303, requested);
        this.log('PARTIAL:' + JSON.stringify({
            requested,
            stockBefore: net.count,
            result: partial,
            nets: Inventory.count('Small fishing net'),
        }));

        const pending = Shop.buyById(314, 1000);
        this.log('CANCEL_PENDING');
        const cancelled = await pending;
        this.log('CANCEL_RESULT:' + JSON.stringify({
            result: cancelled,
            feathers: Inventory.count('Feather'),
        }));
    }
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Prep {
    Initial,
    SendInitialTutorial,
    WaitInitialTutorial,
    WaitLogout,
    WaitRelog,
    ReseedTutorial,
    WaitReseed,
    Seed,
    WaitSeed,
    Ready,
}

#[derive(Default)]
struct Controls {
    grant_coins: bool,
    grant_sent: bool,
    observe_cancel: bool,
}

struct Fixture {
    prep: Prep,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    seed_commands: Vec<String>,
    seed_cursor: usize,
    controls: Controls,
    snapshot: GameSnapshot,
    error: Option<String>,
    ready: bool,
    cancel_ready: bool,
}

impl Fixture {
    fn new() -> Self {
        Self {
            prep: Prep::Initial,
            tutorial_reseed: None,
            seed_commands: vec![
                "give coins 1".into(),
                interact::tele_args(GERRANT.2, GERRANT.0, GERRANT.1),
            ],
            seed_cursor: 0,
            controls: Controls::default(),
            snapshot: GameSnapshot::new(),
            error: None,
            ready: false,
            cancel_ready: false,
        }
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        if self.controls.observe_cancel && inventory_count(&self.snapshot, "Feather") > 1 {
            self.cancel_ready = true;
        }
        if self.prep == Prep::WaitLogout && !client.ingame {
            self.prep = Prep::WaitRelog;
            return;
        }
        if self.prep == Prep::Ready {
            self.drive_controls(client);
            return;
        }
        if held {
            return;
        }
        if let Err(error) = self.prepare(client) {
            self.error = Some(error);
        }
    }

    fn prepare(&mut self, client: &mut Client) -> Result<(), String> {
        match self.prep {
            Prep::Initial if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                self.prep = Prep::SendInitialTutorial;
            }
            Prep::SendInitialTutorial => {
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.prep = Prep::WaitInitialTutorial;
            }
            Prep::WaitInitialTutorial
                if scenario::tutorial::tutorial_confirmed(&self.snapshot, 0) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("tutorial preparation logout was refused".into());
                }
                self.prep = Prep::WaitLogout;
            }
            Prep::WaitRelog
                if client.ingame
                    && client.scene_state == 2
                    && self
                        .snapshot
                        .side_tabs()
                        .iter()
                        .any(|tab| tab.index == 3 && tab.available) =>
            {
                self.prep = Prep::ReseedTutorial;
            }
            Prep::ReseedTutorial => {
                let baseline = scenario::tutorial::chat_baseline(&self.snapshot);
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                self.prep = Prep::WaitReseed;
            }
            Prep::WaitReseed => {
                if self
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed armed")
                    .check(&self.snapshot)?
                {
                    self.prep = Prep::Seed;
                }
            }
            Prep::Seed => {
                if let Some(command) = self.seed_commands.get(self.seed_cursor) {
                    send_cheat(client, command)?;
                    self.seed_cursor += 1;
                } else {
                    self.prep = Prep::WaitSeed;
                }
            }
            Prep::WaitSeed => {
                if self.seed_ready(client) {
                    self.ready = true;
                    self.prep = Prep::Ready;
                }
            }
            Prep::Initial
            | Prep::WaitInitialTutorial
            | Prep::WaitLogout
            | Prep::WaitRelog
            | Prep::Ready => {}
        }
        Ok(())
    }

    fn seed_ready(&self, client: &Client) -> bool {
        if !client.ingame || client.scene_state != 2 {
            return false;
        }
        let Some((x, z, level)) = self.snapshot.tile() else {
            return false;
        };
        level == GERRANT.2
            && (x - GERRANT.0).abs().max((z - GERRANT.1).abs()) <= 1
            && inventory_count(&self.snapshot, "Coins") == 1
    }

    fn drive_controls(&mut self, client: &mut Client) {
        if self.controls.grant_coins && !self.controls.grant_sent {
            if let Err(error) = send_cheat(client, "give coins 100") {
                self.error = Some(error);
                return;
            }
            self.controls.grant_sent = true;
        }
    }
}

fn inventory_count(snapshot: &GameSnapshot, name: &str) -> i32 {
    snapshot
        .inventory()
        .iter()
        .filter(|item| {
            item.def
                .name
                .as_deref()
                .is_some_and(|item_name| item_name.eq_ignore_ascii_case(name))
        })
        .map(|item| item.count.max(0))
        .sum()
}

fn send_cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("live seed command refused: {command}"))
    }
}

fn required(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("live cell requires {key}"))
}

fn required_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("live cell requires {key}")))
}

fn save_receipt(name: &str, value: &Value) -> PathBuf {
    let dir = required_path("LIVE_EVIDENCE_DIR");
    let home = required_path("HOME");
    assert!(
        !dir.starts_with(home),
        "live evidence must be outside throwaway HOME"
    );
    std::fs::create_dir_all(&dir).expect("create live evidence directory");
    let path = dir.join(name);
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(value).expect("serialize receipt"),
    )
    .expect("write live receipt");
    path
}

fn verify_live_environment() {
    assert_eq!(required("LIVE"), "1", "ignored live cells require LIVE=1");
    assert_eq!(
        required("BOT_CPU"),
        "1",
        "headless capture requires BOT_CPU=1"
    );
    assert_eq!(
        required("BOT_NAV_BUILD"),
        "skip",
        "use the explicit v17 nav pack"
    );
    let prefix = required("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "BOT_LIVE_NAME_PREFIX must be one to four ASCII alphanumeric characters"
    );
    assert!(required_path("RS2B0T").is_dir());
    assert!(required_path("WORLD_NAV_PACK").ends_with("274bot-289-v17.navpack"));
    assert!(required_path("WORLD_ENGINE_DIR").is_dir());
    assert!(required_path("BOT_CACHE_DIR").is_dir());
}

fn verify_product_commit() -> (String, String) {
    let product = required("COMPAT_W2_PRODUCT_COMMIT");
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = |args: &[&str]| {
        let output = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(args)
            .output()
            .expect("git live commit check");
        assert!(output.status.success(), "git {:?} failed", args);
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_owned()
    };
    let head = output(&["rev-parse", "HEAD"]);
    let product_full = output(&["rev-parse", "--verify", &format!("{product}^{{commit}}")]);
    let ancestor = Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args(["merge-base", "--is-ancestor", &product_full, &head])
        .status()
        .expect("check product commit ancestry");
    assert!(
        ancestor.success(),
        "product commit {product_full} is not an ancestor of {head}"
    );
    (product_full, head)
}

fn start_shop_fixture() -> (script::IsolatedEnv, Play, String, Arc<Mutex<Fixture>>) {
    verify_live_environment();
    script_js::install_cards().expect("install canonical script cards");
    api::hostlog::set_debug(true);

    let isolated = script::IsolatedEnv::enter("compat_v1_shop_buy_by_id_engine_l");
    isolated.set_rs2b0t(&required_path("RS2B0T"));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(GAME_PORT),
        http_port: Some(HTTP_PORT),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(isolated.home.join("unpack-289")),
        vault_path: Some(isolated.home.join("vault")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve Engine L local-289 profile")
        .prepare_template()
        .expect("prepare cached Engine L client template");
    let names = mint_live_names(1);
    assert_eq!(names.len(), 1, "mint one live account");
    let entries = mint_live_entries(&names);
    let (account, password) = &entries[0];
    let state = Arc::new(Mutex::new(Fixture::new()));
    let callback_state = Arc::clone(&state);
    let callback_account = account.clone();
    let mut play = run_with_template(
        template,
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
    )
    .expect("connect to the already-running Engine L");
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: password.clone().into(),
            uid: (now.as_millis() % i32::MAX as u128) as i32,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh live account");
    play.focus(account);
    wait_for_state(&play, account, &state, PREP_DEADLINE, |_, fixture| {
        fixture.ready
    });
    (isolated, play, account.clone(), state)
}

fn wait_for_state(
    play: &Play,
    account: &str,
    fixture: &Mutex<Fixture>,
    timeout: Duration,
    mut condition: impl FnMut(&Play, &Fixture) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &state.error {
            panic!("live fixture failed: {error}");
        }
        if condition(play, &state) {
            return;
        }
        if state.prep == Prep::WaitRelog {
            lift_logout_park(play, account);
        }
        assert!(
            Instant::now() < deadline,
            "live cell timeout for {account}: prep={:?}, script={:?}, native={:?}",
            state.prep,
            play.script_state(account),
            play.script_native_status(account)
        );
        drop(state);
        thread::sleep(POLL_INTERVAL);
    }
}

/// The fixture logs its account out to re-seed the tutorial, and the host parks a bot with no
/// script after a logout nobody asked it for. An explicit login intent lifts that park.
fn lift_logout_park(play: &Play, account: &str) {
    if let Some(arm) = play.arm(account) {
        if arm.login_latch_reason().is_some() {
            arm.arm_explicit_login();
        }
    }
}

fn take_logs(play: &Play, account: &str, logs: &mut Vec<String>) {
    logs.extend(play.script_take_pending_logs(account));
}

fn wait_for_log(
    play: &Play,
    account: &str,
    fixture: &Mutex<Fixture>,
    timeout: Duration,
    marker: &str,
    logs: &mut Vec<String>,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        take_logs(play, account, logs);
        if let Some(line) = logs.iter().find(|line| line.contains(marker)) {
            return line.clone();
        }
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &state.error {
            panic!("live fixture failed: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {marker} timed out: logs={logs:?}, script={:?}, native={:?}",
            play.script_state(account),
            play.script_native_status(account)
        );
        drop(state);
        thread::sleep(POLL_INTERVAL);
    }
}

fn stop_script(play: &Play, account: &str, fixture: &Mutex<Fixture>) {
    play.script_stop(account);
    wait_for_state(play, account, fixture, SHOP_CELL_DEADLINE, |play, _| {
        play.script_state(account) == script::RunState::Idle
    });
}

fn marker_value<'a>(line: &'a str, marker: &str) -> &'a str {
    line.split_once(marker)
        .map(|(_, value)| value.trim())
        .unwrap_or_else(|| panic!("live log line lacks {marker}: {line}"))
}

#[test]
#[ignore = "requires LIVE=1 and an already-running Engine L"]
fn compat_v1_shop_buy_by_id_engine_l() {
    let (product_commit, head) = verify_product_commit();
    let (_isolated, play, account, fixture) = start_shop_fixture();
    let mut logs = Vec::new();

    play.script_start_load(
        &account,
        SHOP_SOURCE.to_owned(),
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start Shop compatibility Load script");
    wait_for_log(
        &play,
        &account,
        &fixture,
        SHOP_CELL_DEADLINE,
        "SHOP_OPENED",
        &mut logs,
    );

    let insufficient = wait_for_log(
        &play,
        &account,
        &fixture,
        SHOP_CELL_DEADLINE,
        "INSUFFICIENT:",
        &mut logs,
    );
    assert_eq!(marker_value(&insufficient, "INSUFFICIENT:"), "0");
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.grant_coins = true;
    }
    wait_for_state(&play, &account, &fixture, SHOP_CELL_DEADLINE, |_, state| {
        state.controls.grant_sent && inventory_count(&state.snapshot, "Coins") >= 100
    });

    let exact = wait_for_log(
        &play,
        &account,
        &fixture,
        SHOP_CELL_DEADLINE,
        "EXACT:",
        &mut logs,
    );
    let exact: Value = serde_json::from_str(marker_value(&exact, "EXACT:")).expect("exact JSON");
    assert_eq!(exact["result"], 1);
    assert_eq!(exact["feathers"], 1);

    let partial = wait_for_log(
        &play,
        &account,
        &fixture,
        SHOP_CELL_DEADLINE,
        "PARTIAL:",
        &mut logs,
    );
    let partial: Value =
        serde_json::from_str(marker_value(&partial, "PARTIAL:")).expect("partial JSON");
    let requested = partial["requested"].as_i64().expect("requested quantity");
    let stock_before = partial["stockBefore"].as_i64().expect("observed stock");
    let bought = partial["result"].as_i64().expect("settled quantity");
    assert_eq!(requested, stock_before + 1);
    assert_eq!(bought, stock_before);
    assert!(bought > 0, "the real shop must settle some stock");
    assert_eq!(partial["nets"], bought);

    wait_for_log(
        &play,
        &account,
        &fixture,
        SHOP_CELL_DEADLINE,
        "CANCEL_PENDING",
        &mut logs,
    );
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.observe_cancel = true;
    }
    wait_for_state(&play, &account, &fixture, SHOP_CELL_DEADLINE, |_, state| {
        state.cancel_ready
    });
    let feathers_at_cancel = {
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        inventory_count(&state.snapshot, "Feather")
    };
    assert!(feathers_at_cancel > 1);
    assert!(
        feathers_at_cancel < 1001,
        "the 1,000-unit request remains unsettled"
    );
    stop_script(&play, &account, &fixture);

    let final_state = fixture.lock().unwrap_or_else(|error| error.into_inner());
    let receipt = json!({
        "cell": "compat_v1_shop_buy_by_id_engine_l",
        "product_commit": product_commit,
        "head": head,
        "account": account,
        "shop_tile": GERRANT,
        "insufficient_coins_result": marker_value(&insufficient, "INSUFFICIENT:"),
        "exact_quantity": exact,
        "partial_stock": partial,
        "cancel_requested": 1000,
        "feathers_at_cancel": feathers_at_cancel,
        "script_state_after_cancel": format!("{:?}", play.script_state(&account)),
        "live_logs": logs,
        "remaining_inventory": {
            "coins": inventory_count(&final_state.snapshot, "Coins"),
            "feathers": inventory_count(&final_state.snapshot, "Feather"),
            "small_fishing_nets": inventory_count(&final_state.snapshot, "Small fishing net"),
        },
    });
    let path = save_receipt("receipt.json", &receipt);
    println!("SHOP_RECEIPT={}", path.display());
    assert!(final_state.error.is_none());
}
