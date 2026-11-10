//! Real Engine L witnesses for the wave-two compatibility dialogue, game-state,
//! and quest-log families. Each ignored cell owns a fresh account and writes a
//! complete receipt under LIVE_EVIDENCE_DIR; run every cell three times on one
//! product commit.

#![cfg(feature = "live-harness")]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, Interactions, OpTarget, SendResult};
use api::snapshot::{GameSnapshot, ReadContext};
use client::client::Client;
use host_play::{mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const GAME_PORT: u16 = 47_594;
const HTTP_PORT: u16 = 3_080;
const QUEST_JOURNAL_ROOT: i32 = 8_134;
const CELL_DEADLINE: Duration = Duration::from_secs(240);
const PREP_DEADLINE: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_millis(50);
// Match MakeOne's result wait when checking for late dispatches.
const MAKE_ONE_CANCEL_WINDOW_TICKS: u32 = 10;

const MAKE_SOURCE: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class CompatMakeOneProbe extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;
        this.log('MAKE_PENDING');
        const result = await ChatDialog.makeOne('Shortbow');
        this.log('MAKE_RESULT:' + JSON.stringify(result));
    }
}
"#;

// An IfButton already sent to Engine L cannot be revoked by local Stop.
// Engine L may still complete that one Make-1; that completion is content behaviour, not a bot action.
// The live cell checks the result and dispatch trace after stopping the pending wait.
const MAKE_CANCEL_SOURCE: &str = r#"
import { ChatDialog } from '../../api/ui/dialogue/ChatDialog.js';
export default class CompatMakeOneCancelProbe extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;
        this.log('MAKE_PENDING');
        const request = ChatDialog.makeOne('Shortbow');
        this.log('MAKE_CLICK_SENT');
        const result = await request;
        this.log('MAKE_RESULT:' + JSON.stringify(result));
    }
}
"#;

const CAST_SOURCE: &str = r#"
import { Game } from '../../api/game/Game.js';
import { Inventory } from '../../api/inventory/Inventory.js';
import { Npcs } from '../../api/npcs/Npcs.js';
import { Execution } from '../../api/execution/Execution.js';
export default class CompatCastOnNpcProbe extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;
        const target = Npcs.query().name('Cow').action('Attack').within(10).nearest();
        if (!target) throw new Error('no real attackable Cow near the seeded tile');
        const identity = { name: target.name, id: target.id, index: target.index };
        this.log('CAST_TARGET:' + JSON.stringify(identity));
        const missing = await Game.castOnNpc('WIND STRIKE', { index: target.index });
        this.log('CAST_MISSING_RUNES:' + JSON.stringify(missing));
        await Execution.delayUntilTicks(
            () => Inventory.count('Air rune') >= 100 && Inventory.count('Mind rune') >= 100,
            40,
        );
        const before = {
            air: Inventory.count('Air rune'),
            mind: Inventory.count('Mind rune'),
        };
        const cast = await Game.castOnNpc('WIND STRIKE', { index: target.index });
        const after = {
            air: Inventory.count('Air rune'),
            mind: Inventory.count('Mind rune'),
        };
        this.log('CAST_RESULT:' + JSON.stringify({ cast, before, after, target: identity }));
    }
}
"#;

const JOURNAL_SOURCE: &str = r#"
import { Quests } from '../../api/ui/questlog/Quests.js';
export default class CompatJournalProbe extends LoopingBot {
    async loop() {
        if (this.started) return;
        this.started = true;
        this.log('JOURNAL_PENDING');
        const lines = await Quests.journal("Cook's Assistant");
        this.log('JOURNAL_RESULT:' + JSON.stringify(lines));
    }
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    MakeOne,
    CastNpc,
    QuestJournal,
}

impl Cell {
    fn name(self) -> &'static str {
        match self {
            Self::MakeOne => "compat_v1_dialogue_make_one",
            Self::CastNpc => "compat_v1_game_cast_on_npc",
            Self::QuestJournal => "compat_v1_quest_journal",
        }
    }

    fn seed_commands(self) -> Vec<String> {
        let mut commands = vec!["setstat defence 98".into(), "setstat hitpoints 99".into()];
        match self {
            Self::MakeOne => commands.extend([
                "setstat fletching 20".into(),
                "give logs 3".into(),
                "give knife 1".into(),
            ]),
            Self::CastNpc => commands.extend([
                "setstat magic 20".into(),
                interact::tele_args(0, 3_255, 3_278),
            ]),
            Self::QuestJournal => commands.extend(["give logs 1".into(), "give knife 1".into()]),
        }
        commands
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PrepPhase {
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
    open_make_menu: bool,
    open_quest_tab: bool,
    close_modal: bool,
    add_runes: bool,
    open_other_quest: bool,
    logout: bool,
}

struct Fixture {
    cell: Cell,
    prep: PrepPhase,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
    seed_commands: Vec<String>,
    seed_cursor: usize,
    controls: Controls,
    snapshot: GameSnapshot,
    error: Option<String>,
    ready: bool,
    make_menu_attempted: bool,
    make_menu_ready: bool,
    close_sent: bool,
    runes_sent: usize,
    wrong_quest_name: Option<String>,
    wrong_quest_clicked: bool,
    wrong_page_ready: bool,
    logout_sent: bool,
    quest_tab_sent: bool,
    saw_logout: bool,
    relogged: bool,
    npc_rows: Vec<Value>,
    modal_samples: Vec<Value>,
}

impl Fixture {
    fn new(cell: Cell) -> Self {
        Self {
            cell,
            prep: PrepPhase::Initial,
            tutorial_reseed: None,
            seed_commands: cell.seed_commands(),
            seed_cursor: 0,
            controls: Controls::default(),
            snapshot: GameSnapshot::new(),
            error: None,
            ready: false,
            make_menu_attempted: false,
            make_menu_ready: false,
            close_sent: false,
            runes_sent: 0,
            wrong_quest_name: None,
            wrong_quest_clicked: false,
            wrong_page_ready: false,
            logout_sent: false,
            quest_tab_sent: false,
            saw_logout: false,
            relogged: false,
            npc_rows: Vec::new(),
            modal_samples: Vec::new(),
        }
    }

    fn frame(&mut self, client: &mut Client, held: bool) {
        self.snapshot.rebuild(client);
        self.record_observations(client);
        if self.error.is_some() || self.prep == PrepPhase::Ready {
            if !held {
                self.drive_controls(client);
            }
            return;
        }
        // A logged-out client is always held (no ready session), so the
        // preparation logout is observed before the held return.
        if self.prep == PrepPhase::WaitLogout && !client.ingame {
            self.prep = PrepPhase::WaitRelog;
            return;
        }
        if held {
            return;
        }
        if let Err(error) = self.prepare(client) {
            self.error = Some(error);
        }
    }

    fn record_observations(&mut self, client: &Client) {
        if self.cell == Cell::CastNpc {
            let rows = self
                .snapshot
                .npcs()
                .iter()
                .filter(|npc| npc.name.as_deref().is_some_and(|name| name == "Cow"))
                .map(|npc| {
                    json!({
                        "name": npc.name,
                        "id": npc.r#type,
                        "index": npc.index,
                        "tile": npc.tile,
                    })
                })
                .collect::<Vec<_>>();
            if !rows.is_empty() && self.npc_rows.last() != Some(&json!(rows)) {
                self.npc_rows.push(json!(rows));
            }
        }
        let modal = self.snapshot.modals();
        let sample = json!({
            "tick": self.snapshot.tick(),
            "main": modal.main,
            "chat": modal.chat,
            "texts": self.snapshot.main_modal_texts(),
            "make_products": self.snapshot.make_products().iter().map(|p| &p.name).collect::<Vec<_>>(),
        });
        if self.modal_samples.last() != Some(&sample) {
            self.modal_samples.push(sample);
        }
        if self.controls.open_quest_tab
            && self.snapshot.active_side_tab() == 2
            && !self.snapshot.quest_statuses().is_empty()
        {
            self.controls.open_quest_tab = false;
            self.quest_tab_sent = false;
        }
        if self.controls.open_make_menu
            && (modal.main != -1 || modal.chat != -1)
            && !self.snapshot.make_products().is_empty()
        {
            self.make_menu_ready = true;
            self.controls.open_make_menu = false;
        }
        if self.controls.close_modal && modal.main == -1 && modal.chat == -1 {
            self.controls.close_modal = false;
            self.close_sent = false;
        }
        if self.controls.add_runes
            && inventory_count(&self.snapshot, "Air rune") >= 100
            && inventory_count(&self.snapshot, "Mind rune") >= 100
        {
            self.controls.add_runes = false;
        }
        if self.controls.open_other_quest
            && self.wrong_quest_name.as_deref().is_some_and(|name| {
                modal.main == QUEST_JOURNAL_ROOT
                    && self
                        .snapshot
                        .main_modal_texts()
                        .iter()
                        .any(|text| text.strip_prefix("@dre@").unwrap_or(text).trim() == name)
            })
        {
            self.wrong_page_ready = true;
            self.controls.open_other_quest = false;
        }
        if self.controls.logout && !client.ingame {
            self.saw_logout = true;
        }
        if self.saw_logout && client.ingame && client.scene_state == 2 {
            self.relogged = true;
            self.controls.logout = false;
        }
    }

    fn prepare(&mut self, client: &mut Client) -> Result<(), String> {
        match self.prep {
            PrepPhase::Initial if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                self.prep = PrepPhase::SendInitialTutorial;
            }
            PrepPhase::SendInitialTutorial => {
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.prep = PrepPhase::WaitInitialTutorial;
            }
            PrepPhase::WaitInitialTutorial
                if scenario::tutorial::tutorial_confirmed(&self.snapshot, 0) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !interact::logout(client, &ifaces) {
                    return Err("tutorial preparation logout was refused".into());
                }
                self.prep = PrepPhase::WaitLogout;
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
                self.prep = PrepPhase::ReseedTutorial;
            }
            PrepPhase::ReseedTutorial => {
                let baseline = scenario::tutorial::chat_baseline(&self.snapshot);
                send_cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                send_cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                self.tutorial_reseed = Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                self.prep = PrepPhase::WaitReseed;
            }
            PrepPhase::WaitReseed => {
                if self
                    .tutorial_reseed
                    .as_ref()
                    .expect("tutorial reseed armed")
                    .check(&self.snapshot)?
                {
                    self.prep = PrepPhase::Seed;
                }
            }
            PrepPhase::Seed => {
                if let Some(command) = self.seed_commands.get(self.seed_cursor) {
                    send_cheat(client, command)?;
                    self.seed_cursor += 1;
                } else {
                    self.prep = PrepPhase::WaitSeed;
                }
            }
            PrepPhase::WaitSeed => {
                if self.seed_ready(client) {
                    self.ready = true;
                    self.prep = PrepPhase::Ready;
                }
            }
            PrepPhase::Initial
            | PrepPhase::WaitInitialTutorial
            | PrepPhase::WaitLogout
            | PrepPhase::WaitRelog
            | PrepPhase::Ready => {}
        }
        Ok(())
    }

    fn seed_ready(&self, client: &Client) -> bool {
        if !client.ingame || client.scene_state != 2 {
            return false;
        }
        let stat = |name: &str| {
            self.snapshot
                .stats()
                .iter()
                .find(|row| row.name.eq_ignore_ascii_case(name))
                .map_or(0, |row| row.base)
        };
        match self.cell {
            Cell::MakeOne => {
                stat("fletching") >= 20
                    && inventory_count(&self.snapshot, "Logs") >= 3
                    && inventory_count(&self.snapshot, "Knife") >= 1
            }
            Cell::CastNpc => {
                stat("magic") >= 20
                    && inventory_count(&self.snapshot, "Air rune") == 0
                    && inventory_count(&self.snapshot, "Mind rune") == 0
                    && self.snapshot.tile() == Some((3_255, 3_278, 0))
            }
            Cell::QuestJournal => {
                inventory_count(&self.snapshot, "Logs") >= 1
                    && inventory_count(&self.snapshot, "Knife") >= 1
            }
        }
    }

    fn drive_controls(&mut self, client: &mut Client) {
        if self.controls.open_make_menu && !self.make_menu_attempted {
            let inventory = self.snapshot.inventory();
            let knife = inventory
                .iter()
                .find(|item| item.def.name.as_deref() == Some("Knife"));
            let logs = inventory
                .iter()
                .find(|item| item.def.name.as_deref() == Some("Logs"));
            let Some((knife, logs)) = knife.zip(logs) else {
                self.error = Some("real knife/log pair is absent".into());
                return;
            };
            let result =
                Interactions::new(&self.snapshot, client).use_item_on(knife, OpTarget::Item(logs));
            if !matches!(&result, SendResult::Sent { .. }) {
                self.error = Some(format!("real knife-on-logs use refused: {result:?}"));
                return;
            }
            self.make_menu_attempted = true;
        }
        if self.controls.close_modal && !self.close_sent {
            let modal = self.snapshot.modals();
            if modal.main != -1 || modal.chat != -1 {
                self.close_sent = interact::close_modal(client);
                if !self.close_sent {
                    self.error = Some("make-modal close was refused".into());
                    return;
                }
            } else {
                self.controls.close_modal = false;
            }
        }
        if self.controls.add_runes && self.runes_sent < 2 {
            let command = if self.runes_sent == 0 {
                "give airrune 100"
            } else {
                "give mindrune 100"
            };
            if let Err(error) = send_cheat(client, command) {
                self.error = Some(error);
                return;
            }
            self.runes_sent += 1;
        }
        if self.controls.open_quest_tab && !self.quest_tab_sent {
            if self.snapshot.active_side_tab() == 2 && !self.snapshot.quest_statuses().is_empty() {
                self.controls.open_quest_tab = false;
                self.quest_tab_sent = false;
            } else {
                let result = Interactions::new(&self.snapshot, client).click_side_tab(2);
                if !matches!(&result, SendResult::Sent { .. }) {
                    self.error = Some(format!("quest-tab selection refused: {result:?}"));
                    return;
                }
                self.quest_tab_sent = true;
            }
        }
        if self.controls.open_other_quest {
            self.open_other_quest(client);
        }
        if self.controls.logout && !self.logout_sent {
            let ifaces = Arc::clone(&client.ifaces);
            if !interact::logout(client, &ifaces) {
                self.error = Some("live stop/relog logout was refused".into());
                return;
            }
            self.logout_sent = true;
        }
    }

    fn open_other_quest(&mut self, client: &mut Client) {
        if self.wrong_quest_clicked {
            return;
        }
        if self.snapshot.active_side_tab() != 2 || self.snapshot.quest_statuses().is_empty() {
            self.controls.open_quest_tab = true;
            self.quest_tab_sent = false;
            return;
        }
        let Some(row) = self
            .snapshot
            .quest_statuses()
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case("Black Knight's Fortress"))
        else {
            self.error = Some("Black Knight's Fortress is absent from the live quest tab".into());
            return;
        };
        let component_id = row.component_id;
        let name = row.name.clone();
        let ctx = ReadContext::new(&self.snapshot);
        let Some(widget) = ctx.component(component_id) else {
            self.error = Some("Black Knight's Fortress row has no live widget".into());
            return;
        };
        let result = Interactions::new(&self.snapshot, client).if_button(widget);
        if !matches!(&result, SendResult::Sent { .. }) {
            self.error = Some(format!(
                "Black Knight's Fortress row click refused: {result:?}"
            ));
            return;
        }
        self.wrong_quest_name = Some(name);
        self.wrong_quest_clicked = true;
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
fn modal_roots(snapshot: &GameSnapshot) -> (i32, i32) {
    let modal = snapshot.modals();
    (modal.main, modal.chat)
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

fn evidence_dir() -> PathBuf {
    required_path("LIVE_EVIDENCE_DIR")
}

fn save_receipt(name: &str, value: &Value) -> PathBuf {
    let dir = evidence_dir();
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
        "headless live capture requires BOT_CPU=1"
    );
    assert_eq!(
        required("BOT_NAV_BUILD"),
        "skip",
        "use the explicit local-data nav pack"
    );
    let prefix = required("BOT_LIVE_NAME_PREFIX");
    assert!(
        (1..=4).contains(&prefix.len()) && prefix.bytes().all(|byte| byte.is_ascii_alphanumeric()),
        "BOT_LIVE_NAME_PREFIX must be one to four ASCII alphanumeric characters"
    );
    let catalog_root = required_path("RS2B0T");
    assert!(
        catalog_root.is_dir(),
        "RS2B0T is not a directory: {}",
        catalog_root.display()
    );
    let evidence = evidence_dir();
    let home = required_path("HOME");
    assert!(
        !evidence.starts_with(&home),
        "evidence must be outside the throwaway HOME"
    );
    let nav_pack = required_path("WORLD_NAV_PACK");
    assert!(
        nav_pack.is_file(),
        "WORLD_NAV_PACK is not a file: {}",
        nav_pack.display()
    );
    assert!(
        nav_pack.ends_with("274bot.navpack"),
        "live cells require the revision-289 nav pack from tools/local-data"
    );
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

fn start_fixture(cell: Cell) -> (script::IsolatedEnv, Play, String, Arc<Mutex<Fixture>>) {
    verify_live_environment();
    script_js::install_cards().expect("install canonical script cards");
    api::hostlog::set_debug(true);

    let catalog_root = required_path("RS2B0T");
    let isolated = script::IsolatedEnv::enter(cell.name());
    isolated.set_rs2b0t(&catalog_root);
    let home = &isolated.home;
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(GAME_PORT),
        http_port: Some(HTTP_PORT),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join("unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("resolve Engine L local-289 profile")
        .prepare_template()
        .expect("prepare cached Engine L client template");

    let entries = mint_live_entries(&mint_live_names(1));
    assert_eq!(
        entries.len(),
        1,
        "mint one live account using the caller prefix"
    );
    let (account, password) = &entries[0];
    let state = Arc::new(Mutex::new(Fixture::new(cell)));
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
    .expect("start client against the already-running Engine L");
    let uid = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("wall clock");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: password.clone().into(),
            uid: (uid.as_millis() % i32::MAX as u128) as i32,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        None,
    )
    .expect("spawn fresh live account");
    play.focus(account);
    wait_for(&play, account, &state, PREP_DEADLINE, |_, fixture| {
        fixture.ready
    });
    (isolated, play, account.clone(), state)
}

/// The fixture logs its account out (tutorial re-seed, stop/relog), and the host parks a bot with
/// no script after a logout nobody asked it for. An explicit login intent lifts that park.
fn lift_logout_park(play: &Play, account: &str) {
    if let Some(arm) = play.arm(account) {
        if arm.login_latch_reason().is_some() {
            arm.arm_explicit_login();
        }
    }
}

fn wait_for(
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
        if state.prep == PrepPhase::WaitRelog || (state.saw_logout && !state.relogged) {
            lift_logout_park(play, account);
        }
        assert!(
            Instant::now() < deadline,
            "live cell timeout for {account}: state={:?}, script={:?}, native={:?}, tick={}, active_side_tab={}, open_other_quest={}, clicked_wrong_quest={}, wrong_quest={:?}, wrong_page_ready={}, modals={:?}, main_texts={:?}",
            state.prep,
            play.script_state(account),
            play.script_native_status(account),
            state.snapshot.tick(),
            state.snapshot.active_side_tab(),
            state.controls.open_other_quest,
            state.wrong_quest_clicked,
            state.wrong_quest_name,
            state.wrong_page_ready,
            state.snapshot.modals(),
            state.snapshot.main_modal_texts()
        );
        drop(state);
        thread::sleep(POLL_INTERVAL);
    }
}

fn request_make_menu(fixture: &Mutex<Fixture>) {
    let mut fixture = fixture.lock().unwrap_or_else(|error| error.into_inner());
    fixture.make_menu_attempted = false;
    fixture.make_menu_ready = false;
    fixture.controls.open_make_menu = true;
}

fn close_modal(play: &Play, account: &str, fixture: &Mutex<Fixture>) {
    {
        let mut fixture = fixture.lock().unwrap_or_else(|error| error.into_inner());
        fixture.close_sent = false;
        fixture.controls.close_modal = true;
    }
    wait_for(play, account, fixture, CELL_DEADLINE, |_, state| {
        !state.controls.close_modal
    });
}

fn start_script(play: &Play, account: &str, source: &str) {
    play.script_start_load(
        account,
        source.to_owned(),
        script_js::LoadShape::CompatClass,
        None,
        vec![],
    )
    .expect("start compatibility v1 Load script");
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
) -> u32 {
    let deadline = Instant::now() + timeout;
    loop {
        take_logs(play, account, logs);
        if logs.iter().any(|line| line.contains(marker)) {
            return fixture
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .snapshot
                .tick();
        }
        if let Some(error) = play.script_last_error(account) {
            panic!("compatibility script failed before {marker}: {error}");
        }
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(error) = &state.error {
            panic!("live fixture failed before {marker}: {error}");
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
    wait_for(play, account, fixture, CELL_DEADLINE, |play, _| {
        play.script_state(account) == script::RunState::Idle
    });
}

fn log_json(logs: &[String], prefix: &str) -> Value {
    let row = logs
        .iter()
        .find_map(|line| line.strip_prefix(prefix))
        .unwrap_or_else(|| panic!("missing {prefix:?} in logs {logs:?}"));
    serde_json::from_str(row)
        .unwrap_or_else(|error| panic!("invalid {prefix} JSON {row:?}: {error}"))
}
fn packet_count(opcodes: &[Value], opcode: u8, start_tick: u64, end_tick: u64) -> usize {
    opcodes
        .iter()
        .filter(|entry| {
            entry["opcode"].as_u64() == Some(u64::from(opcode))
                && entry["tick"]
                    .as_u64()
                    .is_some_and(|tick| start_tick <= tick && tick <= end_tick)
        })
        .count()
}

fn tick_boundary(receipt: &Value, name: &str) -> u64 {
    receipt["observed_tick_boundaries"][name]
        .as_u64()
        .unwrap_or_else(|| panic!("missing observed tick boundary {name}"))
}
fn trace_lines_between<'a>(
    text: &'a str,
    account: &str,
    start_marker: &str,
    end_marker: &str,
) -> Vec<&'a str> {
    let mut in_window = false;
    let mut start_seen = false;
    let mut end_seen = false;
    let mut selected = Vec::new();
    for line in text.lines() {
        if !in_window && line.contains(start_marker) && line.contains(account) {
            in_window = true;
            start_seen = true;
            continue;
        }
        if in_window && line.contains(end_marker) && line.contains(account) {
            end_seen = true;
            break;
        }
        if in_window {
            selected.push(line);
        }
    }
    assert!(
        start_seen && end_seen,
        "missing trace window markers {start_marker:?} and {end_marker:?}"
    );
    selected
}

fn trace_dispatch_lines_between<'a>(
    text: &'a str,
    account: &str,
    start_marker: &str,
    end_marker: &str,
) -> Vec<&'a str> {
    trace_lines_between(text, account, start_marker, end_marker)
        .into_iter()
        .filter(|line| {
            (line.contains("[interact ") && line.contains(account))
                || (line.contains("native-packet")
                    && line
                        .split_whitespace()
                        .any(|field| field.strip_prefix("account=") == Some(account)))
                || (line.contains("[shim-") && line.contains("sent=true"))
        })
        .collect()
}

fn trace_if_button_sends_between(
    text: &str,
    account: &str,
    start_marker: &str,
    end_marker: &str,
) -> usize {
    trace_lines_between(text, account, start_marker, end_marker)
        .into_iter()
        .filter(|line| line.contains("[shim-if-button]") && line.contains("sent=true"))
        .count()
}

fn run_make_one_cell() {
    let (_isolated, play, account, fixture) = start_fixture(Cell::MakeOne);
    let start_commit = verify_product_commit();
    request_make_menu(&fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.make_menu_ready
    });
    let make_page_root = modal_roots(&fixture.lock().unwrap().snapshot);
    assert!(
        make_page_root.0 != -1 || make_page_root.1 != -1,
        "the real make page is open"
    );

    let (before, make_start_tick) = {
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        (
            inventory_count(&state.snapshot, "Shortbow"),
            state.snapshot.tick(),
        )
    };
    assert_eq!(before, 0, "fixture starts without shortbows");
    let mut success_logs = Vec::new();
    eprintln!("COMPAT_MAKE_SUCCESS_START account={account}");
    start_script(&play, &account, MAKE_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "MAKE_RESULT:",
        &mut success_logs,
    );
    let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
    assert_eq!(
        log_json(&success_logs, "MAKE_RESULT:"),
        json!(true),
        "makeOne final state: roots={:?}, shortbows={}, tick={}, samples={:?}",
        modal_roots(&state.snapshot),
        inventory_count(&state.snapshot, "Shortbow"),
        state.snapshot.tick(),
        state.modal_samples
    );
    drop(state);

    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        inventory_count(&state.snapshot, "Shortbow") == 1
    });
    let success_end_tick = fixture.lock().unwrap().snapshot.tick();
    let success_page = modal_roots(&fixture.lock().unwrap().snapshot);
    eprintln!("COMPAT_MAKE_SUCCESS_END account={account}");
    stop_script(&play, &account, &fixture);

    close_modal(&play, &account, &fixture);
    let missing_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut missing_logs = Vec::new();
    eprintln!("COMPAT_MAKE_MISSING_START account={account}");
    start_script(&play, &account, MAKE_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "MAKE_RESULT:",
        &mut missing_logs,
    );
    assert_eq!(log_json(&missing_logs, "MAKE_RESULT:"), json!(false));
    assert_eq!(
        inventory_count(&fixture.lock().unwrap().snapshot, "Shortbow"),
        1
    );
    let missing_end_tick = fixture.lock().unwrap().snapshot.tick();
    eprintln!("COMPAT_MAKE_MISSING_END account={account}");
    stop_script(&play, &account, &fixture);

    request_make_menu(&fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.make_menu_ready
    });
    let (cancel_logs_remaining, cancel_knives_remaining) = {
        let state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        (
            inventory_count(&state.snapshot, "Logs"),
            inventory_count(&state.snapshot, "Knife"),
        )
    };
    assert!(
        cancel_logs_remaining >= 1 && cancel_knives_remaining >= 1,
        "cancel probe has enough ingredients for another product: logs={cancel_logs_remaining}, knives={cancel_knives_remaining}"
    );
    let cancel_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut cancel_logs = Vec::new();
    eprintln!("COMPAT_MAKE_CANCEL_START account={account}");
    start_script(&play, &account, MAKE_CANCEL_SOURCE);
    let click_seen_tick = wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "MAKE_CLICK_SENT",
        &mut cancel_logs,
    );
    assert_eq!(
        play.script_state(&account),
        script::RunState::Running,
        "MakeOne remains in flight after its click"
    );
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.tick() > click_seen_tick
    });
    let cancel_stop_tick = fixture.lock().unwrap().snapshot.tick();
    assert_eq!(
        cancel_stop_tick,
        click_seen_tick.saturating_add(1),
        "Stop follows the click on its first game tick"
    );
    let mut cancel_stop_calls = 0;
    cancel_stop_calls += 1;
    eprintln!("COMPAT_MAKE_CANCEL_STOP account={account} tick={cancel_stop_tick}");
    stop_script(&play, &account, &fixture);
    let cancel_stopped_tick = fixture.lock().unwrap().snapshot.tick();
    assert_eq!(
        cancel_stopped_tick, cancel_stop_tick,
        "Stop completed on the click's first following tick"
    );
    let cancel_deadline_tick = cancel_stop_tick.saturating_add(MAKE_ONE_CANCEL_WINDOW_TICKS);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.tick() >= cancel_deadline_tick
    });
    take_logs(&play, &account, &mut cancel_logs);
    assert_eq!(
        cancel_logs
            .iter()
            .filter(|line| line.starts_with("MAKE_PENDING"))
            .count(),
        1,
        "cancel probe started one request: {cancel_logs:?}"
    );
    assert_eq!(
        cancel_logs
            .iter()
            .filter(|line| line.starts_with("MAKE_CLICK_SENT"))
            .count(),
        1,
        "cancel probe dispatched one Make-1 click: {cancel_logs:?}"
    );
    assert!(
        !cancel_logs
            .iter()
            .any(|line| line.starts_with("MAKE_RESULT:")),
        "stopped MakeOne published a result: {cancel_logs:?}"
    );
    // Stop suppresses the pending JS result, but cannot revoke the sent click.
    // Permit at most that click's one product; the trace must reject later dispatches.
    assert_eq!(cancel_stop_calls, 1, "Stop is requested exactly once");
    assert_eq!(
        play.script_state(&account),
        script::RunState::Idle,
        "the stopped MakeOne script is idle"
    );
    let cancel_end_tick = fixture.lock().unwrap().snapshot.tick();
    eprintln!("COMPAT_MAKE_CANCEL_WINDOW_END account={account} tick={cancel_end_tick}");
    let after_cancel = inventory_count(&fixture.lock().unwrap().snapshot, "Shortbow");
    assert!(
        (1..=2).contains(&after_cancel),
        "the prior product remains and the sent click can add at most one: shortbows={after_cancel}"
    );

    request_make_menu(&fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.make_menu_ready
    });
    let replacement_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut replacement_logs = Vec::new();
    eprintln!("COMPAT_MAKE_REPLACEMENT_START account={account}");
    start_script(&play, &account, MAKE_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "MAKE_PENDING",
        &mut replacement_logs,
    );
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.close_modal = true;
    }
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "MAKE_RESULT:",
        &mut replacement_logs,
    );
    assert_eq!(log_json(&replacement_logs, "MAKE_RESULT:"), json!(false));
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.modals().main == -1 && state.snapshot.modals().chat == -1
    });
    let replacement_end_tick = fixture.lock().unwrap().snapshot.tick();
    let replacement_page_root = modal_roots(&fixture.lock().unwrap().snapshot);
    assert_eq!(
        replacement_page_root,
        (-1, -1),
        "the make page closed while the action was pending"
    );
    let after_replacement = inventory_count(&fixture.lock().unwrap().snapshot, "Shortbow");
    // The stopped click may have produced one item. Page replacement must not add another.
    assert_eq!(
        after_replacement, after_cancel,
        "replacing the make page made no product"
    );
    assert_ne!(
        replacement_page_root, make_page_root,
        "the make page was replaced"
    );
    eprintln!("COMPAT_MAKE_REPLACEMENT_END account={account}");
    stop_script(&play, &account, &fixture);

    let receipt = json!({
        "cell": Cell::MakeOne.name(),
        "product_commit": start_commit.0,
        "tested_tree": start_commit.1,
        "account": account,
        "engine": {"host": "127.0.0.1", "game_port": GAME_PORT, "http_port": HTTP_PORT},
        "real_inputs": {"logs": 3, "knife": 1},
        "cancel_inputs": {
            "logs": cancel_logs_remaining,
            "knives": cancel_knives_remaining
        },
        "product": {"name": "Shortbow", "id": 50, "before": before, "after_success": 1},
        "make_page_roots": {
            "before": {"main": make_page_root.0, "chat": make_page_root.1},
            "after_success": {"main": success_page.0, "chat": success_page.1},
            "after_replacement": {
                "main": replacement_page_root.0,
                "chat": replacement_page_root.1
            }
        },
        "observed_tick_boundaries": {
            "success_start": make_start_tick,
            "success_end": success_end_tick,
            "missing_start": missing_start_tick,
            "missing_end": missing_end_tick,
            "cancel_start": cancel_start_tick,
            "cancel_click_seen": click_seen_tick,
            "cancel_stop": cancel_stop_tick,
            "cancel_stop_completed": cancel_stopped_tick,
            "cancel_deadline": cancel_deadline_tick,
            "cancel_end": cancel_end_tick,
            "replacement_start": replacement_start_tick,
            "replacement_end": replacement_end_tick
        },
        "success_logs": success_logs,
        "missing_menu_result": log_json(&missing_logs, "MAKE_RESULT:"),
        "cancel_pending": cancel_logs,
        "cancel_result_suppressed": true,
        "cancel_stop_calls": cancel_stop_calls,
        "cancel_output_count": after_cancel,
        "cancel_output_count_max": 2,
        "replacement_result": log_json(&replacement_logs, "MAKE_RESULT:"),
        "replacement_output_count": after_replacement,
        "page_replaced": replacement_page_root != make_page_root,
        "modal_samples": fixture.lock().unwrap().modal_samples,
    });
    save_receipt("receipt.json", &receipt);
}

fn run_cast_npc_cell() {
    let (_isolated, play, account, fixture) = start_fixture(Cell::CastNpc);
    let product = verify_product_commit();
    let mut logs = Vec::new();
    start_script(&play, &account, CAST_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "CAST_MISSING_RUNES:",
        &mut logs,
    );
    assert_eq!(log_json(&logs, "CAST_MISSING_RUNES:"), json!(false));
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.add_runes = true;
    }
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "CAST_RESULT:",
        &mut logs,
    );
    let cast = log_json(&logs, "CAST_RESULT:");
    assert_eq!(
        cast["cast"],
        json!(true),
        "real Wind Strike dispatch must be acknowledged"
    );
    assert!(cast["before"]["air"].as_i64().unwrap_or_default() >= 100);
    assert!(cast["before"]["mind"].as_i64().unwrap_or_default() >= 100);
    assert_eq!(
        cast["before"]["air"].as_i64().unwrap() - cast["after"]["air"].as_i64().unwrap(),
        1,
        "Wind Strike consumes one real Air rune"
    );
    assert_eq!(
        cast["before"]["mind"].as_i64().unwrap() - cast["after"]["mind"].as_i64().unwrap(),
        1,
        "Wind Strike consumes one real Mind rune"
    );
    let target = log_json(&logs, "CAST_TARGET:");
    assert_eq!(cast["target"], target);
    let target_seen = fixture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .npc_rows
        .iter()
        .filter_map(Value::as_array)
        .flatten()
        .any(|row| {
            row["name"] == target["name"]
                && row["id"] == target["id"]
                && row["index"] == target["index"]
        });
    assert!(
        target_seen,
        "cast target must be an observed real NPC: {target}"
    );
    stop_script(&play, &account, &fixture);

    let receipt = json!({
        "cell": Cell::CastNpc.name(),
        "product_commit": product.0,
        "tested_tree": product.1,
        "account": account,
        "engine": {"host": "127.0.0.1", "game_port": GAME_PORT, "http_port": HTTP_PORT},
        "spell": "Wind Strike",
        "missing_runes_result": false,
        "target": target,
        "successful_cast": cast,
        "observed_real_npcs": fixture.lock().unwrap_or_else(|error| error.into_inner()).npc_rows,
        "native_no_dispatch_refusal": "missing runes returned false; driver trace must contain exactly one OPNPCT packet for the later funded cast",
    });
    save_receipt("receipt.json", &receipt);
}

fn run_quest_journal_cell() {
    let (_isolated, play, account, fixture) = start_fixture(Cell::QuestJournal);
    let product = verify_product_commit();
    open_quest_tab_live(&play, &account, &fixture);
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.open_other_quest = true;
    }
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.wrong_page_ready
    });
    let wrong_title = fixture
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .wrong_quest_name
        .clone()
        .expect("wrong journal title observed");
    assert!(!wrong_title.eq_ignore_ascii_case("Cook's Assistant"));

    let wrong_modal_tick = fixture.lock().unwrap().snapshot.tick();
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.tick() >= wrong_modal_tick.saturating_add(1)
    });
    let wrong_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut wrong_logs = Vec::new();
    start_script(&play, &account, JOURNAL_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "JOURNAL_RESULT:",
        &mut wrong_logs,
    );
    assert_eq!(log_json(&wrong_logs, "JOURNAL_RESULT:"), json!([]));
    let wrong_end_tick = fixture.lock().unwrap().snapshot.tick();
    stop_script(&play, &account, &fixture);
    close_modal(&play, &account, &fixture);

    request_make_menu(&fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.make_menu_ready
    });
    let foreign_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut foreign_logs = Vec::new();
    start_script(&play, &account, JOURNAL_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "JOURNAL_RESULT:",
        &mut foreign_logs,
    );
    assert_eq!(log_json(&foreign_logs, "JOURNAL_RESULT:"), json!([]));
    let foreign_end_tick = fixture.lock().unwrap().snapshot.tick();
    stop_script(&play, &account, &fixture);
    close_modal(&play, &account, &fixture);

    open_quest_tab_live(&play, &account, &fixture);
    let valid_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut valid_logs = Vec::new();
    start_script(&play, &account, JOURNAL_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "JOURNAL_RESULT:",
        &mut valid_logs,
    );
    let lines = log_json(&valid_logs, "JOURNAL_RESULT:");
    assert!(
        lines.is_array() && !lines.as_array().unwrap().is_empty(),
        "real journal lines required: {lines}"
    );
    stop_script(&play, &account, &fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.modals().main == -1
    });
    let valid_end_tick = fixture.lock().unwrap().snapshot.tick();

    open_quest_tab_live(&play, &account, &fixture);
    let stopped_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut stopped_logs = Vec::new();
    start_script(&play, &account, JOURNAL_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "JOURNAL_PENDING",
        &mut stopped_logs,
    );
    stop_script(&play, &account, &fixture);
    let stopped_end_tick = fixture.lock().unwrap().snapshot.tick();
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.logout_sent = false;
        state.saw_logout = false;
        state.relogged = false;
        state.controls.logout = true;
    }
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.relogged
    });
    take_logs(&play, &account, &mut stopped_logs);
    assert!(
        !stopped_logs
            .iter()
            .any(|line| line.starts_with("JOURNAL_RESULT:")),
        "stopped pre-capture read must not publish stale journal lines: {stopped_logs:?}"
    );
    open_quest_tab_live(&play, &account, &fixture);
    let post_relog_start_tick = fixture.lock().unwrap().snapshot.tick();
    let mut post_relog_logs = Vec::new();
    start_script(&play, &account, JOURNAL_SOURCE);
    wait_for_log(
        &play,
        &account,
        &fixture,
        CELL_DEADLINE,
        "JOURNAL_RESULT:",
        &mut post_relog_logs,
    );
    let post_relog_lines = log_json(&post_relog_logs, "JOURNAL_RESULT:");
    assert!(post_relog_lines.is_array() && !post_relog_lines.as_array().unwrap().is_empty());
    stop_script(&play, &account, &fixture);
    wait_for(&play, &account, &fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.modals().main == -1
    });
    let post_relog_end_tick = fixture.lock().unwrap().snapshot.tick();

    let receipt = json!({
        "cell": Cell::QuestJournal.name(),
        "product_commit": product.0,
        "tested_tree": product.1,
        "account": account,
        "engine": {"host": "127.0.0.1", "game_port": GAME_PORT, "http_port": HTTP_PORT},
        "actual_quest": "Cook's Assistant",
        "actual_journal_lines": lines,
        "observed_tick_boundaries": {
            "wrong_start": wrong_start_tick,
            "wrong_end": wrong_end_tick,
            "foreign_start": foreign_start_tick,
            "foreign_end": foreign_end_tick,
            "valid_start": valid_start_tick,
            "valid_end": valid_end_tick,
            "stopped_start": stopped_start_tick,
            "stopped_end": stopped_end_tick,
            "post_relog_start": post_relog_start_tick,
            "post_relog_end": post_relog_end_tick
        },
        "wrong_title": {"title": wrong_title, "result": log_json(&wrong_logs, "JOURNAL_RESULT:")},
        "foreign_modal": {"root": "real make menu", "result": log_json(&foreign_logs, "JOURNAL_RESULT:")},
        "stop_relog": {"pending": true, "relogged": true, "stale_result": false},
        "post_relog_lines": post_relog_lines,
        "modal_samples": fixture.lock().unwrap_or_else(|error| error.into_inner()).modal_samples,
    });
    save_receipt("receipt.json", &receipt);
}

fn open_quest_tab_live(play: &Play, account: &str, fixture: &Mutex<Fixture>) {
    {
        let mut state = fixture.lock().unwrap_or_else(|error| error.into_inner());
        state.controls.open_quest_tab = true;
        state.quest_tab_sent = false;
    }
    wait_for(play, account, fixture, CELL_DEADLINE, |_, state| {
        state.snapshot.active_side_tab() == 2 && !state.snapshot.quest_statuses().is_empty()
    });
}

fn run_captured(test_name: &str, cell: Cell, body: fn()) {
    verify_live_environment();
    let evidence = evidence_dir();
    std::fs::create_dir_all(&evidence).expect("create live evidence directory");
    if std::env::var("COMPAT_W2_CHILD").as_deref() == Ok(test_name) {
        body();
        return;
    }

    let trace = evidence.join(format!("{}-driver-trace.txt", cell.name()));
    let status = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            test_name,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("COMPAT_W2_CHILD", test_name)
        .stdout(Stdio::inherit())
        .stderr(std::fs::File::create(&trace).expect("create driver trace"))
        .status()
        .expect("spawn isolated compatibility live cell");
    assert!(
        status.success(),
        "live child failed; inspect {}",
        trace.display()
    );

    let text = std::fs::read_to_string(&trace).expect("read driver trace");
    let receipt_path = evidence.join("receipt.json");
    let mut receipt: Value =
        serde_json::from_slice(&std::fs::read(&receipt_path).expect("read child receipt"))
            .expect("parse child receipt");
    assert_eq!(
        receipt["cell"],
        json!(cell.name()),
        "child receipt cell mismatch"
    );
    let account = receipt["account"]
        .as_str()
        .expect("receipt account")
        .to_owned();
    let mut opcodes = Vec::new();
    let mut accepted_requests = 0usize;
    for line in text
        .lines()
        .filter(|line| line.contains(&format!("account={account} ")))
    {
        if line.contains("native-packet account=") {
            let field = |prefix: &str| {
                line.split_whitespace()
                    .find_map(|word| word.strip_prefix(prefix))
                    .unwrap_or_else(|| panic!("missing {prefix} in {line:?}"))
            };
            opcodes.push(json!({
                "tick": field("tick=").parse::<u64>().expect("packet tick"),
                "request": field("request=").parse::<u64>().expect("packet request"),
                "opcode": field("opcode=").parse::<u8>().expect("packet opcode"),
            }));
        } else if line.contains("native-packets account=") {
            assert!(
                line.contains("decoded=true accepted=true"),
                "Driver rejected or could not decode live request: {line}"
            );
            accepted_requests += 1;
        }
    }
    assert!(
        accepted_requests > 0 || cell == Cell::MakeOne,
        "no native requests were accepted: {text}"
    );

    let revision = client::io::ClientRevision::R289;
    if cell == Cell::CastNpc {
        let opcode = client::io::map_client_prot(revision, client::io::ClientProt::OPNPCT).id as u8;
        let casts = opcodes
            .iter()
            .filter(|entry| entry["opcode"].as_u64() == Some(u64::from(opcode)))
            .count();
        assert_eq!(
            casts, 1,
            "exactly one funded Wind Strike OPNPCT dispatch; trace={opcodes:?}"
        );
        receipt["driver"]["opnpct_opcode"] = json!(opcode);
        receipt["driver"]["opnpct_dispatches"] = json!(casts);
    }
    if cell == Cell::MakeOne {
        let clicks = json!({
            "success": trace_if_button_sends_between(
                &text,
                &account,
                "COMPAT_MAKE_SUCCESS_START",
                "COMPAT_MAKE_SUCCESS_END"
            ),
            "missing": trace_if_button_sends_between(
                &text,
                &account,
                "COMPAT_MAKE_MISSING_START",
                "COMPAT_MAKE_MISSING_END"
            ),
            "cancel": trace_if_button_sends_between(
                &text,
                &account,
                "COMPAT_MAKE_CANCEL_START",
                "COMPAT_MAKE_CANCEL_WINDOW_END"
            ),
            "replacement": trace_if_button_sends_between(
                &text,
                &account,
                "COMPAT_MAKE_REPLACEMENT_START",
                "COMPAT_MAKE_REPLACEMENT_END"
            )
        });
        let cancel_dispatches = trace_dispatch_lines_between(
            &text,
            &account,
            "COMPAT_MAKE_CANCEL_STOP",
            "COMPAT_MAKE_CANCEL_WINDOW_END",
        );
        assert_eq!(
            clicks["success"], 1,
            "one successful MakeOne IfButton dispatch; trace={text}"
        );
        assert_eq!(
            clicks["missing"], 0,
            "missing menu must not dispatch; trace={text}"
        );
        assert_eq!(
            clicks["cancel"], 1,
            "one MakeOne click started the stopped request; trace={text}"
        );
        assert!(
            cancel_dispatches.is_empty(),
            "no dispatch of any kind may follow Stop through the MakeOne deadline; dispatches={cancel_dispatches:?}"
        );
        assert_eq!(
            clicks["replacement"], 1,
            "page replacement begins with one real MakeOne dispatch; trace={text}"
        );
        receipt["driver"]["make_if_button_dispatches"] = clicks;
        receipt["driver"]["cancel_dispatches_after_stop"] = json!(cancel_dispatches);
    }
    if cell == Cell::QuestJournal {
        let button =
            client::io::map_client_prot(revision, client::io::ClientProt::IF_BUTTON).id as u8;
        let close =
            client::io::map_client_prot(revision, client::io::ClientProt::CLOSE_MODAL).id as u8;
        let counts = json!({
            "wrong_title": {
                "open": packet_count(
                    &opcodes,
                    button,
                    tick_boundary(&receipt, "wrong_start"),
                    tick_boundary(&receipt, "wrong_end")
                ),
                "close": packet_count(
                    &opcodes,
                    close,
                    tick_boundary(&receipt, "wrong_start"),
                    tick_boundary(&receipt, "wrong_end")
                )
            },
            "foreign_modal": {
                "open": packet_count(
                    &opcodes,
                    button,
                    tick_boundary(&receipt, "foreign_start"),
                    tick_boundary(&receipt, "foreign_end")
                ),
                "close": packet_count(
                    &opcodes,
                    close,
                    tick_boundary(&receipt, "foreign_start"),
                    tick_boundary(&receipt, "foreign_end")
                )
            },
            "valid": {
                "open": packet_count(
                    &opcodes,
                    button,
                    tick_boundary(&receipt, "valid_start"),
                    tick_boundary(&receipt, "valid_end")
                ),
                "close": packet_count(
                    &opcodes,
                    close,
                    tick_boundary(&receipt, "valid_start"),
                    tick_boundary(&receipt, "valid_end")
                )
            },
            "stopped": {
                "open": packet_count(
                    &opcodes,
                    button,
                    tick_boundary(&receipt, "stopped_start"),
                    tick_boundary(&receipt, "stopped_end")
                ),
                "close": packet_count(
                    &opcodes,
                    close,
                    tick_boundary(&receipt, "stopped_start"),
                    tick_boundary(&receipt, "stopped_end")
                )
            },
            "post_relog": {
                "open": packet_count(
                    &opcodes,
                    button,
                    tick_boundary(&receipt, "post_relog_start"),
                    tick_boundary(&receipt, "post_relog_end")
                ),
                "close": packet_count(
                    &opcodes,
                    close,
                    tick_boundary(&receipt, "post_relog_start"),
                    tick_boundary(&receipt, "post_relog_end")
                )
            }
        });
        for phase in ["wrong_title", "foreign_modal"] {
            assert_eq!(
                counts[phase]["open"], 0,
                "{phase} must not open quest journal; trace={opcodes:?}"
            );
            assert_eq!(
                counts[phase]["close"], 0,
                "{phase} must not close a modal; trace={opcodes:?}"
            );
        }
        for phase in ["valid", "post_relog"] {
            assert_eq!(
                counts[phase]["open"], 1,
                "{phase} must open the journal once; trace={opcodes:?}"
            );
            assert_eq!(
                counts[phase]["close"], 1,
                "{phase} must close the journal once; trace={opcodes:?}"
            );
        }
        receipt["driver"]["if_button_opcode"] = json!(button);
        receipt["driver"]["close_modal_opcode"] = json!(close);
        receipt["driver"]["quest_interaction_counts"] = counts;
    }
    receipt["driver"]["trace"] = json!(trace);
    receipt["driver"]["accepted_requests"] = json!(accepted_requests);
    receipt["driver"]["packet_events"] = json!(opcodes);
    receipt["driver"]["all_requests_accepted"] = json!(true);
    std::fs::write(
        &receipt_path,
        serde_json::to_vec_pretty(&receipt).expect("serialize receipt"),
    )
    .expect("update live receipt with packet trace");
    println!(
        "PASS {}: receipt={} trace={}",
        cell.name(),
        receipt_path.display(),
        trace.display()
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, aubaine Engine L, isolated HOME, the local-data 289 nav pack, and three fresh-account runs"]
fn compat_v1_dialogue_make_one_live() {
    run_captured(
        "compat_v1_dialogue_make_one_live",
        Cell::MakeOne,
        run_make_one_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, aubaine Engine L, isolated HOME, the local-data 289 nav pack, and three fresh-account runs"]
fn compat_v1_game_cast_on_npc_live() {
    run_captured(
        "compat_v1_game_cast_on_npc_live",
        Cell::CastNpc,
        run_cast_npc_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, aubaine Engine L, isolated HOME, the local-data 289 nav pack, and three fresh-account runs"]
fn compat_v1_quest_journal_live() {
    run_captured(
        "compat_v1_quest_journal_live",
        Cell::QuestJournal,
        run_quest_journal_cell,
    );
}
