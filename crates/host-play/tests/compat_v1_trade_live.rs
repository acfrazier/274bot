//! Engine L paired Trade cell. Three fresh accounts: one configured partner,
//! one unconfigured partner, and one receiver. The configured pair cancels one
//! trade before settlement, mutates an offer, then settles actual item exchange.
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
const PREP_DEADLINE: Duration = Duration::from_secs(300);
const TRADE_DEADLINE: Duration = Duration::from_secs(180);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
const GERRANT: (i32, i32, i32) = (3013, 3224, 0);

const TRADE_SOURCE: &str = r#"
import { Execution } from '../../api/execution/Execution.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Trade } from '../../api/trade/Trade.js';
import { stableClosedPoll } from '../../api/trade/drivePartnerTrade.js';

const ROLE = __TRADE_ROLE__;
const PARTNER = __TRADE_PARTNER__;
const STRANGER = __TRADE_STRANGER__;

function sameName(a, b) {
    const normalize = value => String(value || '').trim().toLowerCase().replace(/[ _]+/g, ' ');
    return normalize(a) !== '' && normalize(a) === normalize(b);
}

function offerCount(rows, itemName) {
    const wanted = itemName.toLowerCase();
    return rows
        .filter(row => row.name && row.name.toLowerCase() === wanted)
        .reduce((total, row) => total + row.count, 0);
}

async function waitTicks(predicate, label, ticks = 120) {
    if (!await Execution.delayUntilTicks(predicate, ticks)) {
        throw new Error(label + ' timed out');
    }
}

async function requestTrade(partner, label) {
    for (let attempt = 0; attempt < 8; attempt++) {
        if (Trade.active()) return;
        await Trade.request(partner);
        if (await Execution.delayUntilTicks(() => Trade.active(), 20)) return;
    }
    throw new Error(label + ' did not open with ' + partner);
}

async function waitStableClose(label) {
    if (!await Execution.delayUntil(stableClosedPoll(), 10000)) {
        throw new Error(label + ' did not remain closed for a full observed tick');
    }
}

function inventory() {
    return {
        coins: Inventory.count('Coins'),
        feathers: Inventory.count('Feather'),
    };
}

async function runUnexpectedPartner(bot) {
    await requestTrade(STRANGER, 'changed partner probe');
    await waitTicks(() => Trade.partner() !== null, 'changed partner header');
    const actual = Trade.partner();
    if (!sameName(actual, STRANGER) || sameName(actual, PARTNER)) {
        throw new Error('changed partner probe opened with ' + String(actual));
    }
    bot.log('CHANGED_PARTNER_OPEN:' + JSON.stringify({ expected: PARTNER, actual }));
    await Execution.delayTicks(3);
    await Trade.decline();
    await waitStableClose('changed partner decline');
    bot.log(
        'CHANGED_PARTNER_DECLINED:' +
            JSON.stringify({ expected: PARTNER, partner: actual, stable: true }),
    );
}

async function runReceiver(bot) {
    await runUnexpectedPartner(bot);

    for (let attempt = 1; attempt <= 2; attempt++) {
        await requestTrade(PARTNER, 'configured partner trade ' + attempt);
        await waitTicks(() => Trade.partner() !== null, 'configured partner header');
        const actual = Trade.partner();
        if (!sameName(actual, PARTNER)) {
            await Trade.decline();
            await waitStableClose('changed partner decline');
            throw new Error('configured trade changed partner: ' + String(actual));
        }
        const before = inventory();
        if (attempt === 1) {
            await waitTicks(() => offerCount(Trade.theirOffer(), 'Feather') === 1, 'first feather offer');
            const offered = await Trade.offer('Coins', 1);
            if (!offered) throw new Error('first coin offer was refused');
            if (!await Trade.accept()) throw new Error('first offer-screen accept was refused');
            await waitTicks(() => Trade.onConfirmScreen(), 'first confirm screen');
            bot.log('CANCEL_CONFIRM_SCREEN');
            await Trade.decline();
            await waitStableClose('cancel before settlement');
            const after = inventory();
            if (after.coins !== before.coins || after.feathers !== before.feathers) {
                throw new Error('cancelled trade changed receiver inventory: ' + JSON.stringify({ before, after }));
            }
            bot.log('CANCEL_NO_SETTLEMENT:' + JSON.stringify({ before, after, stable: true }));
        } else {
            await waitTicks(() => offerCount(Trade.theirOffer(), 'Feather') === 2, 'mutated feather offer');
            const acceptedCoins = await Trade.offer('Coins', 1);
            if (!acceptedCoins) throw new Error('settlement coin offer was refused');
            if (!await Trade.accept()) throw new Error('settlement offer-screen accept was refused');
            await waitTicks(() => Trade.onConfirmScreen(), 'settlement confirm screen');
            bot.log('SUCCESS_CONFIRM_SCREEN');
            if (!await Trade.accept()) throw new Error('settlement confirm accept was refused');
            await waitStableClose('successful settlement');
            const after = inventory();
            if (after.coins !== before.coins - 1 || after.feathers !== before.feathers + 2) {
                throw new Error('settled receiver inventory mismatch: ' + JSON.stringify({ before, after }));
            }
            bot.log('SUCCESS_SETTLEMENT:' + JSON.stringify({ before, after, stable: true }));
        }
    }
}

async function runGiver(bot) {
    for (let attempt = 1; attempt <= 2; attempt++) {
        await requestTrade(PARTNER, 'giver trade ' + attempt);
        await waitTicks(() => Trade.partner() !== null, 'giver partner header');
        const actual = Trade.partner();
        if (!sameName(actual, PARTNER)) {
            await Trade.decline();
            await waitStableClose('giver changed partner decline');
            throw new Error('giver trade changed partner: ' + String(actual));
        }
        const before = inventory();
        const first = await Trade.offer('Feather', 1);
        if (!first) throw new Error('initial feather offer was refused');
        let beforeMutation = 1;
        let afterRemoval = 1;
        let afterMutation = 1;
        if (attempt === 1) {
            await waitTicks(() => offerCount(Trade.theirOffer(), 'Coins') === 1, 'first coin offer');
            if (!await Trade.accept()) throw new Error('first offer-screen accept was refused');
            await waitTicks(() => !Trade.active(), 'cancel from receiver');
            await waitStableClose('giver cancel before settlement');
            const after = inventory();
            if (after.coins !== before.coins || after.feathers !== before.feathers) {
                throw new Error('cancelled trade changed giver inventory: ' + JSON.stringify({ before, after }));
            }
            bot.log('CANCEL_NO_SETTLEMENT:' + JSON.stringify({ before, after, stable: true }));
        } else {
            beforeMutation = offerCount(Trade.myOffer(), 'Feather');
            const removed = await Trade.removeAll();
            if (!removed) throw new Error('offer mutation could not remove the initial item');
            afterRemoval = offerCount(Trade.myOffer(), 'Feather');
            const reoffered = await Trade.offer('Feather', 2);
            if (!reoffered) throw new Error('mutated feather offer was refused');
            afterMutation = offerCount(Trade.myOffer(), 'Feather');
            if (beforeMutation !== 1 || afterRemoval !== 0 || afterMutation !== 2) {
                throw new Error('offer mutation did not settle exactly: ' + JSON.stringify({ beforeMutation, afterRemoval, afterMutation }));
            }
            bot.log('OFFER_MUTATION:' + JSON.stringify({ before: beforeMutation, afterRemoval, after: afterMutation }));
            await waitTicks(() => offerCount(Trade.theirOffer(), 'Coins') === 1, 'settlement coin offer');
            if (!await Trade.accept()) throw new Error('settlement offer-screen accept was refused');
            await waitTicks(() => Trade.onConfirmScreen(), 'giver settlement confirm screen');
            bot.log('SUCCESS_CONFIRM_SCREEN');
            if (!await Trade.accept()) throw new Error('settlement confirm accept was refused');
            await waitStableClose('giver successful settlement');
            const after = inventory();
            if (after.feathers !== before.feathers - 2 || after.coins !== before.coins + 1) {
                throw new Error('settled giver inventory mismatch: ' + JSON.stringify({ before, after }));
            }
            bot.log('SUCCESS_SETTLEMENT:' + JSON.stringify({ before, after, stable: true }));
        }
    }
}

async function runStranger(bot) {
    await requestTrade(PARTNER, 'changed partner counterpart');
    await waitTicks(() => Trade.partner() !== null, 'stranger partner header');
    const actual = Trade.partner();
    if (!sameName(actual, PARTNER)) throw new Error('stranger probe opened with ' + String(actual));
    await waitTicks(() => !Trade.active(), 'receiver decline of stranger');
    await waitStableClose('changed partner counterpart close observation');
    bot.log('CHANGED_PARTNER_CLOSED:' + JSON.stringify({ partner: actual, stable: true }));
}

export default class CompatV1TradeLive extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;
        if (ROLE === 'receiver') await runReceiver(this);
        else if (ROLE === 'giver') await runGiver(this);
        else if (ROLE === 'stranger') await runStranger(this);
        else throw new Error('unknown live role: ' + ROLE);
        this.log('DONE:' + ROLE);
    }
}
"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

struct TradeSlot {
    account: String,
    tile: (i32, i32, i32),
    expected_coins: i32,
    expected_feathers: i32,
    prep: Prep,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    seed_commands: Vec<String>,
    seed_cursor: usize,
    snapshot: GameSnapshot,
    error: Option<String>,
    ready: bool,
}

impl TradeSlot {
    fn new(account: String, tile: (i32, i32, i32), coins: i32, feathers: i32) -> Self {
        let mut seed_commands = Vec::new();
        if coins > 0 {
            seed_commands.push(format!("give coins {coins}"));
        }
        if feathers > 0 {
            seed_commands.push(format!("give feather {feathers}"));
        }
        seed_commands.push(interact::tele_args(tile.2, tile.0, tile.1));
        Self {
            account,
            tile,
            expected_coins: coins,
            expected_feathers: feathers,
            prep: Prep::Initial,
            tutorial_reseed: None,
            seed_commands,
            seed_cursor: 0,
            snapshot: GameSnapshot::new(),
            error: None,
            ready: false,
        }
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        if self.prep == Prep::WaitLogout && !client.ingame {
            self.prep = Prep::WaitRelog;
            return;
        }
        if self.prep == Prep::Ready || held {
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
        level == self.tile.2
            && (x - self.tile.0).abs().max((z - self.tile.1).abs()) <= 1
            && inventory_count(&self.snapshot, "Coins") == self.expected_coins
            && inventory_count(&self.snapshot, "Feather") == self.expected_feathers
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
        assert!(output.status.success(), "git {args:?} failed");
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

fn trade_source(role: &str, partner: &str, stranger: &str) -> String {
    TRADE_SOURCE
        .replace("__TRADE_ROLE__", &json!(role).to_string())
        .replace("__TRADE_PARTNER__", &json!(partner).to_string())
        .replace("__TRADE_STRANGER__", &json!(stranger).to_string())
}

fn mint_profile(account: &str, password: &str, offset: u32) -> Profile {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock");
    let uid = (now.as_millis() % (i32::MAX as u128 - 10)) as i32 + offset as i32;
    Profile {
        username: account.to_owned(),
        password: password.to_owned().into(),
        uid,
        settings: ProfileSettings::default(),
    }
}

fn start_trade_pair() -> (
    script::IsolatedEnv,
    Play,
    Vec<String>,
    Arc<Mutex<Vec<TradeSlot>>>,
) {
    verify_live_environment();
    script_js::install_cards().expect("install canonical script cards");
    api::hostlog::set_debug(true);

    let isolated = script::IsolatedEnv::enter("compat_v1_trade_engine_l");
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
    let names = mint_live_names(3);
    assert_eq!(
        names.len(),
        3,
        "mint the receiver, giver, and stranger accounts"
    );
    assert!(
        names.iter().enumerate().all(|(index, name)| names
            .iter()
            .skip(index + 1)
            .all(|other| !name.eq_ignore_ascii_case(other))),
        "live accounts must be distinct"
    );
    let credentials = mint_live_entries(&names);
    assert_eq!(credentials.len(), 3, "mint three local credentials");

    let tiles = [
        GERRANT,
        (GERRANT.0 + 1, GERRANT.1, GERRANT.2),
        (GERRANT.0 + 2, GERRANT.1, GERRANT.2),
    ];
    let state = Arc::new(Mutex::new(vec![
        TradeSlot::new(names[0].clone(), tiles[0], 5, 0),
        TradeSlot::new(names[1].clone(), tiles[1], 0, 3),
        TradeSlot::new(names[2].clone(), tiles[2], 0, 0),
    ]));
    let frame_state = Arc::clone(&state);
    let mut play = run_with_template(
        Arc::clone(&template),
        true,
        vec![],
        |_| (None, None),
        move |client, username, frame| {
            let mut slots = frame_state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(slot) = slots.iter_mut().find(|slot| slot.account == username) {
                slot.frame(client, frame.hold);
            }
        },
    )
    .expect("connect three live profiles to the already-running Engine L");
    for (index, (account, password)) in credentials.iter().enumerate() {
        play.try_spawn_slot(
            mint_profile(account, password, index as u32),
            None,
            None,
            None,
        )
        .expect("spawn fresh paired Trade account");
    }
    wait_for_trade_prep(&play, &state);
    (isolated, play, names, state)
}

/// Each slot logs its account out to re-seed the tutorial, and the host parks a bot with no script
/// after a logout nobody asked it for. An explicit login intent lifts that park.
fn wait_for_trade_prep(play: &Play, state: &Mutex<Vec<TradeSlot>>) {
    let deadline = Instant::now() + PREP_DEADLINE;
    loop {
        let slots = state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some((slot, error)) = slots
            .iter()
            .find_map(|slot| slot.error.as_ref().map(|error| (slot, error)))
        {
            panic!("Trade prep failed for {}: {error}", slot.account);
        }
        if slots.iter().all(|slot| slot.ready) {
            return;
        }
        for slot in slots.iter().filter(|slot| slot.prep == Prep::WaitRelog) {
            if let Some(arm) = play.arm(&slot.account) {
                if arm.login_latch_reason().is_some() {
                    arm.arm_explicit_login();
                }
            }
        }
        assert!(
            Instant::now() < deadline,
            "Trade prep timed out: {:?}",
            slots
                .iter()
                .map(|slot| (&slot.account, slot.prep))
                .collect::<Vec<_>>()
        );
        drop(slots);
        thread::sleep(POLL_INTERVAL);
    }
}

fn start_script(play: &Play, account: &str, role: &str, partner: &str, stranger: &str) {
    play.script_start_load(
        account,
        trade_source(role, partner, stranger),
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start paired Trade compatibility script");
}

fn wait_for_log(
    play: &Play,
    account: &str,
    timeout: Duration,
    marker: &str,
    logs: &mut Vec<String>,
) -> String {
    let deadline = Instant::now() + timeout;
    loop {
        logs.extend(play.script_take_pending_logs(account));
        if let Some(line) = logs.iter().find(|line| line.contains(marker)) {
            return line.clone();
        }
        if let Some(error) = play.script_last_error(account) {
            panic!("Trade script failed for {account}: {error}; logs={logs:?}");
        }
        assert!(
            Instant::now() < deadline,
            "waiting for {marker} timed out for {account}: logs={logs:?}, script={:?}, native={:?}",
            play.script_state(account),
            play.script_native_status(account)
        );
        thread::sleep(POLL_INTERVAL);
    }
}
fn parse_log_payload(line: &str, marker: &str) -> serde_json::Value {
    let (_, payload) = line
        .split_once(marker)
        .unwrap_or_else(|| panic!("live log line does not include {marker}: {line}"));
    serde_json::from_str(payload).expect("live log payload is JSON")
}

fn stop_script(play: &Play, account: &str) {
    play.script_stop(account);
    let deadline = Instant::now() + TRADE_DEADLINE;
    loop {
        if play.script_state(account) == script::RunState::Idle {
            return;
        }
        if let Some(error) = play.script_last_error(account) {
            panic!("Trade script failed while stopping {account}: {error}");
        }
        assert!(
            Instant::now() < deadline,
            "Trade script stop timed out for {account}: {:?}",
            play.script_state(account)
        );
        thread::sleep(POLL_INTERVAL);
    }
}

#[test]
#[ignore = "requires LIVE=1 and an already-running Engine L"]
fn compat_v1_trade_engine_l() {
    let (product_commit, head) = verify_product_commit();
    let (_isolated, mut play, names, state) = start_trade_pair();
    let receiver = names[0].clone();
    let giver = names[1].clone();
    let stranger = names[2].clone();
    let receiver_screen = client::util::jstring::JString::to_screen_name(&receiver);
    let giver_screen = client::util::jstring::JString::to_screen_name(&giver);
    let stranger_screen = client::util::jstring::JString::to_screen_name(&stranger);
    let mut receiver_logs = Vec::new();
    let mut giver_logs = Vec::new();
    let mut stranger_logs = Vec::new();

    start_script(
        &play,
        &receiver,
        "receiver",
        &giver_screen,
        &stranger_screen,
    );
    start_script(
        &play,
        &stranger,
        "stranger",
        &receiver_screen,
        &stranger_screen,
    );
    let changed_partner_open = wait_for_log(
        &play,
        &receiver,
        TRADE_DEADLINE,
        "CHANGED_PARTNER_OPEN:",
        &mut receiver_logs,
    );
    let changed_partner_declined = wait_for_log(
        &play,
        &receiver,
        TRADE_DEADLINE,
        "CHANGED_PARTNER_DECLINED:",
        &mut receiver_logs,
    );
    let changed_partner_closed = wait_for_log(
        &play,
        &stranger,
        TRADE_DEADLINE,
        "CHANGED_PARTNER_CLOSED:",
        &mut stranger_logs,
    );
    let opened = parse_log_payload(&changed_partner_open, "CHANGED_PARTNER_OPEN:");
    assert_eq!(
        opened["expected"].as_str(),
        Some(giver_screen.as_str()),
        "the receiver expected the configured giver"
    );
    assert_eq!(
        opened["actual"].as_str(),
        Some(stranger_screen.as_str()),
        "the open Trade screen exposed the changed partner"
    );
    let declined = parse_log_payload(&changed_partner_declined, "CHANGED_PARTNER_DECLINED:");
    assert_eq!(declined["expected"].as_str(), Some(giver_screen.as_str()));
    assert_eq!(declined["partner"].as_str(), Some(stranger_screen.as_str()));
    assert_eq!(declined["stable"].as_bool(), Some(true));
    let closed = parse_log_payload(&changed_partner_closed, "CHANGED_PARTNER_CLOSED:");
    assert_eq!(closed["partner"].as_str(), Some(receiver_screen.as_str()));
    assert_eq!(closed["stable"].as_bool(), Some(true));

    start_script(&play, &giver, "giver", &receiver_screen, &stranger_screen);
    let cancelled_receiver = wait_for_log(
        &play,
        &receiver,
        TRADE_DEADLINE,
        "CANCEL_NO_SETTLEMENT:",
        &mut receiver_logs,
    );
    let cancelled_giver = wait_for_log(
        &play,
        &giver,
        TRADE_DEADLINE,
        "CANCEL_NO_SETTLEMENT:",
        &mut giver_logs,
    );
    let mutation = wait_for_log(
        &play,
        &giver,
        TRADE_DEADLINE,
        "OFFER_MUTATION:",
        &mut giver_logs,
    );
    let settled_receiver = wait_for_log(
        &play,
        &receiver,
        TRADE_DEADLINE,
        "SUCCESS_SETTLEMENT:",
        &mut receiver_logs,
    );
    let settled_giver = wait_for_log(
        &play,
        &giver,
        TRADE_DEADLINE,
        "SUCCESS_SETTLEMENT:",
        &mut giver_logs,
    );
    wait_for_log(
        &play,
        &receiver,
        TRADE_DEADLINE,
        "DONE:receiver",
        &mut receiver_logs,
    );
    wait_for_log(&play, &giver, TRADE_DEADLINE, "DONE:giver", &mut giver_logs);
    wait_for_log(
        &play,
        &stranger,
        TRADE_DEADLINE,
        "DONE:stranger",
        &mut stranger_logs,
    );

    for account in [&receiver, &giver, &stranger] {
        stop_script(&play, account);
        play.stop_slot(account);
    }

    let slots = state.lock().unwrap_or_else(|error| error.into_inner());
    let receiver_inventory = json!({
        "coins": inventory_count(&slots[0].snapshot, "Coins"),
        "feathers": inventory_count(&slots[0].snapshot, "Feather"),
    });
    let giver_inventory = json!({
        "coins": inventory_count(&slots[1].snapshot, "Coins"),
        "feathers": inventory_count(&slots[1].snapshot, "Feather"),
    });
    assert_eq!(receiver_inventory["coins"], 4);
    assert_eq!(receiver_inventory["feathers"], 2);
    assert_eq!(giver_inventory["coins"], 1);
    assert_eq!(giver_inventory["feathers"], 1);
    let receipt = json!({
        "cell": "compat_v1_trade_engine_l",
        "product_commit": product_commit,
        "head": head,
        "changed_partner": {
            "expected": giver_screen,
            "actual": stranger_screen,
            "opened": changed_partner_open,
            "declined": changed_partner_declined,
            "closed": changed_partner_closed,
        },
        "cancelled_before_settlement": {
            "receiver": cancelled_receiver,
            "giver": cancelled_giver,
        },
        "offer_mutation": mutation,
        "settled": {
            "receiver": settled_receiver,
            "giver": settled_giver,
        },
        "final_inventory": {
            "receiver": receiver_inventory,
            "giver": giver_inventory,
        },
        "live_logs": {
            "receiver": receiver_logs,
            "giver": giver_logs,
            "stranger": stranger_logs,
        },
    });
    let path = save_receipt("trade-receipt.json", &receipt);
    println!("TRADE_RECEIPT={}", path.display());
}
