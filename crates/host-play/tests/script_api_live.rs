//! Public Load-script witnesses for Script API slices C and D (local revision 289).
//! Each cell runs in a child test process so the production Driver's stderr-only
//! packet trace is captured without changing logging policy or sharing HOME state.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use api::interact::{self, Interactions};
use api::snapshot::{ChatLineView, GameSnapshot, QuestListStatus, WorldTile};
use client::client::Client;
use host_play::{
    mint_live_entries, mint_live_names, run_with_template, Play, ProfileOptions, SlotArm,
};
use script::native::{ScriptStatus, StatusValue};
use serde_json::{json, Value};
use vault::{Profile, ProfileSettings};

const TREE_TILE: WorldTile = WorldTile {
    x: 3190,
    z: 3245,
    level: 0,
};

const WALK_START_TILE: WorldTile = WorldTile {
    x: 2895,
    z: 3450,
    level: 0,
};
const CATHERBY: (i32, i32, i32, i32) = (2791, 3438, 2814, 3475);

const WALK_OPTIONS_SOURCE: &str = r#"
let started = false;
export function tick(api) {
  if (!started) {
    started = true;
    const run = api.gather.run({
      skill: 'Woodcutting',
      woodcuttingResources: ['normal'],
      disposition: 'Power',
      location: 'Site',
      site: 'woodcutting.catherby',
      allowTeleports: false,
      allowWilderness: false,
    });
    run.then(value => api.log('walk option outcome: ' + JSON.stringify(value)));
  }
}
"#;

fn in_walk_target_site(tile: WorldTile) -> bool {
    tile.level == 0
        && (CATHERBY.0..=CATHERBY.2).contains(&tile.x)
        && (CATHERBY.1..=CATHERBY.3).contains(&tile.z)
}

const AXE: i32 = 1351;
const LOGS: i32 = 1511;
const JOURNAL_ROOT: i32 = 8134;
const JOURNAL_SCRIPT: &str = r#"
let first = false;
let second = false;
export async function tick(api) {
  if (!first) {
    first = true;
    api.log('quest paths: ' + JSON.stringify(api.questPaths()));
    api.log('journal 30: ' + JSON.stringify(await api.questProgress({quest:'romeojuliet'})));
    return;
  }
  if (!second && globalThis.__script_api_read_again) {
    second = true;
    api.log('journal 40: ' + JSON.stringify(await api.questProgress({quest:'romeojuliet'})));
  }
}
"#;

#[derive(Default)]
struct AdmissionLog(Mutex<Vec<(String, String)>>);
impl api::hostlog::Sink for AdmissionLog {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if record.message.contains("foreground: dropped ") {
            self.0
                .lock()
                .unwrap()
                .push((record.slot.unwrap_or("").into(), record.message.into()));
        }
    }
}
static ADMISSION: AdmissionLog = AdmissionLog(Mutex::new(Vec::new()));

#[derive(Default)]
struct Fixture {
    phase: u8,
    error: Option<String>,
    ready: bool,
    observed_tick: u32,
    started: bool,
    start_tile: Option<WorldTile>,
    #[cfg(feature = "luau")]
    bank_booth: Option<L4bBankBooth>,
    #[cfg(feature = "luau")]
    seed_bank_axe: bool,
    #[cfg(feature = "luau")]
    bank_axe_seeded: bool,
    #[cfg(feature = "luau")]
    bank_axe_visible: bool,
    #[cfg(feature = "luau")]
    bank_axe_inventory_before: Option<i32>,
    #[cfg(feature = "luau")]
    bank_axe_inventory_after_seed: Option<i32>,
    #[cfg(feature = "luau")]
    bank_axe_before: Option<i32>,
    #[cfg(feature = "luau")]
    bank_axe_after: Option<i32>,
    #[cfg(feature = "luau")]
    bank_open: bool,
    /// Seed combat stats when the fixture needs combat-level routing.
    seed_combat_stats: bool,
    walk_arrival: Option<WorldTile>,
    /// The player's tile on the latest frame, from the client.
    observed_tile: Option<WorldTile>,
    advance: bool,
    advance_phase: u8,
    advance_ready: bool,
    inventory: BTreeMap<i32, i32>,
    emptied: Vec<(u64, i32)>,
    hidden_journals: usize,
    normal_closed: bool,
    initial: Option<Value>,
    /// Chat sequence captured before the pre-relog `getvar tutorial`.
    tutorial_baseline: i32,
    /// Chat sequence captured before the `getvar` that follows the one
    /// pre-relog reseed.
    tutorial_reseed_baseline: i32,
    tutorial_reseed: Option<scenario::tutorial::PostRelogTutorial>,
}
#[cfg(feature = "luau")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct L4bBankBooth {
    id: i32,
    tile: WorldTile,
    op: i32,
    distance: i32,
}

fn cheat(client: &mut Client, command: &str) -> Result<(), String> {
    if interact::cheat(client, command).is_sent() {
        Ok(())
    } else {
        Err(format!("fixture cheat refused: {command}"))
    }
}
fn chat_has(snapshot: &GameSnapshot, needle: &str) -> bool {
    snapshot
        .chat_lines()
        .iter()
        .any(|line| line.text.contains(needle))
        || snapshot
            .chat_modal_texts()
            .iter()
            .any(|line| line.contains(needle))
}

#[cfg(feature = "luau")]
fn nearby_bank_booth(snapshot: &GameSnapshot) -> Option<L4bBankBooth> {
    snapshot
        .locs()
        .iter()
        .filter_map(|loc| {
            if loc.distance > 8
                || !loc
                    .name
                    .as_deref()
                    .is_some_and(|name| name.eq_ignore_ascii_case("Bank booth"))
            {
                return None;
            }
            let action_index = loc.actions.iter().position(|action| {
                action
                    .as_deref()
                    .is_some_and(|action| action.eq_ignore_ascii_case("Use-quickly"))
            })?;
            let op = i32::try_from(action_index + 1).ok()?;
            (1..=5).contains(&op).then_some(L4bBankBooth {
                id: loc.id,
                tile: loc.tile,
                op,
                distance: loc.distance,
            })
        })
        .min_by_key(|booth| booth.distance)
}

/// Which pre-relog `getvar tutorial` reply the gate is waiting for.
///
/// The kit-close queue assigns `tutorial=1` after the fixture's own hop seed
/// (`content/scripts/tutorial/scripts/tutorial.rs2:133-141`), so the first
/// reply can legitimately be `1`. The fixture then reseeds completion exactly
/// once and requires the reseed's own fresh reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreRelogTutorialPhase {
    /// The first fresh reply after the mainland hop (fixture phase 2).
    Initial,
    /// The fresh reply after the one reseed (fixture phase 10).
    AfterReseed,
}

impl PreRelogTutorialPhase {
    /// Phase name used in refusal messages and logs.
    fn label(self) -> &'static str {
        match self {
            Self::Initial => "initial getvar reply (fixture phase 2)",
            Self::AfterReseed => "post-reseed getvar reply (fixture phase 10)",
        }
    }
}

/// What the pre-relog tutorial gate must do with the chat it can see.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PreRelogTutorialAction {
    /// Log out: the fresh reply is `get tutorial: 1000`.
    Logout,
    /// Run the one allowed reseed: the initial fresh reply is the
    /// kit-close `get tutorial: 1`.
    Reseed,
}

/// Newest fresh `tutorial` value in `chat`, or `None`.
///
/// Only lines with `sequence > baseline` count, so a reply latched before
/// the baseline is ignored; among fresh replies the newest wins.
fn fresh_tutorial_reply(chat: &[ChatLineView], baseline: i32) -> Option<i32> {
    chat.iter()
        .filter(|line| line.sequence > baseline)
        .filter_map(|line| {
            scenario::tutorial::parse_tutorial_reply(&line.text).map(|value| (line.sequence, value))
        })
        .max_by_key(|&(sequence, _)| sequence)
        .map(|(_, value)| value)
}

/// Pure pre-relog gate: the next action, `Ok(None)` while still waiting, or
/// the refusal message when this phase must not accept the observed value.
///
/// A stale reply (sequence not after `baseline`) is ignored, never accepted.
/// After the one reseed a second `1` is a refusal, not another reseed.
fn pre_relog_tutorial_step(
    phase: PreRelogTutorialPhase,
    baseline: i32,
    chat: &[ChatLineView],
) -> Result<Option<PreRelogTutorialAction>, String> {
    let Some(value) = fresh_tutorial_reply(chat, baseline) else {
        return Ok(None);
    };
    if value == scenario::tutorial::TUTORIAL_COMPLETE {
        return Ok(Some(PreRelogTutorialAction::Logout));
    }
    if value == 1 && phase == PreRelogTutorialPhase::Initial {
        return Ok(Some(PreRelogTutorialAction::Reseed));
    }
    Err(format!(
        "fixture tutorial gate: fresh get tutorial: {value} in the {}; expected \
         get tutorial: {} (directly or after the one reseed)",
        phase.label(),
        scenario::tutorial::TUTORIAL_COMPLETE,
    ))
}

/// Press the fixture logout, then post the host's explicit Login for this
/// slot. Every fixture logout is a relog before Start; the Login is a real
/// one-shot command, needed where that relog consumed the login. An unlatched
/// slot with auto-login would also reconnect by itself.
fn logout_then_login(client: &mut Client, arm: &SlotArm) -> Result<(), String> {
    let ifaces = Arc::clone(&client.ifaces);
    if interact::logout(client, &ifaces) {
        arm.arm_explicit_login();
        Ok(())
    } else {
        Err("fixture logout unavailable".into())
    }
}

fn fixture_frame(
    client: &mut Client,
    held: bool,
    journal: bool,
    shared: &Mutex<Fixture>,
    arm: &SlotArm,
) {
    let mut state = shared.lock().unwrap();
    if state.error.is_some() {
        return;
    }
    let mut snapshot = GameSnapshot::new();
    snapshot.rebuild(client);
    state.observed_tick = snapshot.tick();
    state.observed_tile = snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level });
    #[cfg(feature = "luau")]
    if state.seed_bank_axe && !state.started {
        state.bank_booth = nearby_bank_booth(&snapshot);
    }
    if state.start_tile.is_some() {
        if let Some((x, z, level)) = snapshot.tile() {
            let tile = WorldTile { x, z, level };
            if in_walk_target_site(tile) {
                state.walk_arrival = Some(tile);
            }
        }
    }
    if state.started {
        #[cfg(feature = "luau")]
        if state.seed_bank_axe && !state.bank_axe_seeded && !held {
            state.bank_axe_inventory_before = Some(
                snapshot
                    .inventory()
                    .iter()
                    .filter(|item| item.def.id == AXE)
                    .map(|item| item.count)
                    .sum(),
            );
            if let Err(error) = cheat(client, "give bronze_axe 1") {
                state.error = Some(error);
                return;
            }
            state.bank_axe_seeded = true;
        }
        let inventory = snapshot
            .inventory()
            .iter()
            .filter(|item| item.count > 0)
            .map(|item| (item.slot, item.def.id))
            .collect::<BTreeMap<_, _>>();
        let emptied = state
            .inventory
            .iter()
            .filter_map(|(&slot, &id)| {
                (id == LOGS && !inventory.contains_key(&slot)).then_some((client.gens.player, slot))
            })
            .collect::<Vec<_>>();
        state.emptied.extend(emptied);
        state.inventory = inventory;
        #[cfg(feature = "luau")]
        if state.seed_bank_axe && state.bank_axe_seeded {
            let axe_count = snapshot
                .inventory()
                .iter()
                .filter(|item| item.def.id == AXE)
                .map(|item| item.count)
                .sum::<i32>();
            if state.bank_axe_inventory_after_seed.is_none()
                && state
                    .bank_axe_inventory_before
                    .is_some_and(|before| axe_count > before)
            {
                state.bank_axe_inventory_after_seed = Some(axe_count);
                state.bank_axe_visible = true;
            }
            state.bank_open = snapshot.bank_component_id() >= 0;
            if state.bank_open && snapshot.bank_loaded() {
                let bank_axe_count = snapshot
                    .bank()
                    .iter()
                    .filter(|item| item.def.id == AXE)
                    .map(|item| item.count)
                    .sum();
                state.bank_axe_before.get_or_insert(bank_axe_count);
                state.bank_axe_after = Some(bank_axe_count);
            }
        }
        if client.main_modal_id == JOURNAL_ROOT && client.journal_paint_hidden() {
            state.hidden_journals += 1;
            state.normal_closed = false;
        }
        if state.hidden_journals > 0 && client.main_modal_id == -1 && !client.journal_paint_hidden()
        {
            state.normal_closed = true;
        }
        if state.advance && !held {
            let result = match state.advance_phase {
                0 => {
                    state.advance_phase = 1;
                    cheat(client, "setvar rjquest 40")
                }
                1 => {
                    state.advance_phase = 2;
                    cheat(client, "getvar rjquest")
                }
                _ => Ok(()),
            };
            if let Err(error) = result {
                state.error = Some(error);
            }
        }
        if state.advance_phase == 2 && chat_has(&snapshot, "get rjquest: 40") {
            state.advance_ready = true;
        }
        return;
    }
    // Offline/welcome frames are held. Observe the actual logout edge before
    // respecting that gate; otherwise the fixture can never reach WaitRelog.
    if !client.ingame && (state.phase == 3 || (journal && state.phase == 7)) {
        state.phase += 1;
        return;
    }
    if held || state.ready {
        return;
    }
    let result = (|| -> Result<(), String> {
        match state.phase {
            0 if client.ingame && client.scene_state == 2 => {
                interact::mainland_hop(client);
                state.phase = 1;
            }
            1 => {
                // Only replies newer than this sequence prove this getvar's
                // value; a line latched before it must not satisfy the gate.
                state.tutorial_baseline = scenario::tutorial::chat_baseline(&snapshot);
                cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                state.phase = 2;
            }
            2 => match pre_relog_tutorial_step(
                PreRelogTutorialPhase::Initial,
                state.tutorial_baseline,
                snapshot.chat_lines(),
            )? {
                None => {}
                Some(PreRelogTutorialAction::Logout) => {
                    println!(
                        "fixture tutorial gate: fresh get tutorial: 1000 in the {}; no reseed needed",
                        PreRelogTutorialPhase::Initial.label()
                    );
                    logout_then_login(client, arm)?;
                    state.phase = 3;
                }
                Some(PreRelogTutorialAction::Reseed) => {
                    // The kit-close queue set tutorial=1 after the hop seed.
                    // Seed completion exactly once, then require the reseed's
                    // own fresh reply in phase 10; that phase refuses a
                    // second `1`, so the reseed cannot loop.
                    state.tutorial_reseed_baseline = scenario::tutorial::chat_baseline(&snapshot);
                    cheat(client, scenario::tutorial::TUTORIAL_SETVAR)?;
                    cheat(client, scenario::tutorial::TUTORIAL_GETVAR)?;
                    println!(
                        "fixture tutorial gate: kit-close get tutorial: 1 in the {}; reseeded once with setvar tutorial 1000 + getvar tutorial, waiting in phase 10",
                        PreRelogTutorialPhase::Initial.label()
                    );
                    state.phase = 10;
                }
            },
            10 => match pre_relog_tutorial_step(
                PreRelogTutorialPhase::AfterReseed,
                state.tutorial_reseed_baseline,
                snapshot.chat_lines(),
            )? {
                None => {}
                Some(PreRelogTutorialAction::Logout) => {
                    println!(
                        "fixture tutorial gate: fresh get tutorial: 1000 in the {}",
                        PreRelogTutorialPhase::AfterReseed.label()
                    );
                    logout_then_login(client, arm)?;
                    state.phase = 3;
                }
                Some(PreRelogTutorialAction::Reseed) => {
                    // The step reseeds only in the initial phase; fail closed
                    // rather than ever reseeding a second time.
                    return Err(
                        "fixture tutorial gate: a second pre-relog reseed is refused".into(),
                    );
                }
            },
            4 if client.ingame
                && client.scene_state == 2
                && snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available) =>
            {
                // Closing a fresh character-design modal queues tutorial=1 after
                // the initial hop. Seed completion on this clean relog before
                // preparing equipment or the quest witness.
                if !journal {
                    // Baseline before the reseed send: only a strictly newer
                    // same-session `getvar` reply proves the durable value.
                    // The bronze axe wear below needs `tutorial > 400`.
                    let baseline = scenario::tutorial::chat_baseline(&snapshot);
                    state.tutorial_reseed =
                        Some(scenario::tutorial::PostRelogTutorial::new(baseline));
                }
                cheat(client, "setvar tutorial 1000")?;
                if journal {
                    cheat(client, "setvar rjquest 30")?;
                } else {
                    cheat(client, "setstat woodcutting 1")?;
                    if state.seed_combat_stats {
                        // Keep combat below White Wolf Mountain's level-50
                        // route cutoff while retaining high HP and defence.
                        cheat(client, "setstat attack 1")?;
                        cheat(client, "setstat strength 1")?;
                        cheat(client, "setstat defence 98")?;
                        cheat(client, "setstat hitpoints 99")?;
                    }
                }
                state.phase = 5;
            }
            5 if journal => {
                cheat(client, "getvar rjquest")?;
                state.phase = 6;
            }
            6 if journal && chat_has(&snapshot, "get rjquest: 30") => {
                logout_then_login(client, arm)?;
                state.phase = 7;
            }
            8 if journal
                && client.ingame
                && client.scene_state == 2
                && snapshot.quest_statuses().iter().any(|row| {
                    row.name == "Romeo & Juliet" && row.status() == QuestListStatus::InProgress
                }) =>
            {
                state.ready = true;
                state.initial = Some(json!({"colour":"inProgress", "seed":"setvar rjquest 30"}));
            }
            5 if !journal => {
                cheat(client, "getvar tutorial")?;
                state.phase = 9;
            }
            9 if !journal => {
                let confirmed = match state.tutorial_reseed.as_ref() {
                    Some(reseed) => reseed.check(&snapshot)?,
                    None => false,
                };
                if confirmed {
                    println!("{}", scenario::tutorial::confirmation_log());
                    cheat(client, "give bronze_axe 1")?;
                    state.phase = 6;
                }
            }
            6 if !journal => {
                let tile = state.start_tile.unwrap_or(TREE_TILE);
                cheat(client, &interact::tele_args(tile.level, tile.x, tile.z))?;
                state.phase = 7;
            }
            7 if !journal => {
                if snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == AXE && item.count > 0)
                {
                    if !matches!(
                        Interactions::new(&snapshot, client).wear(AXE),
                        api::interact::SendResult::Sent { .. }
                    ) {
                        return Err("fixture bronze axe Wield refused".into());
                    }
                    state.phase = 8;
                }
            }
            8 if !journal => {
                let start_tile = state.start_tile.unwrap_or(TREE_TILE);
                let wielded = snapshot
                    .equipment()
                    .iter()
                    .any(|item| item.def.id == AXE && item.count == 1);
                let empty = snapshot.inventory().iter().all(|item| item.count <= 0);
                let level_one = snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.name.eq_ignore_ascii_case("woodcutting") && stat.base == 1);
                let combat_level = snapshot
                    .local_player()
                    .map(|player| player.player.combat_level);
                let hitpoints_base = snapshot
                    .stats()
                    .iter()
                    .find(|stat| stat.name.eq_ignore_ascii_case("hitpoints"))
                    .map(|stat| stat.base);
                let seeded_combat =
                    combat_level.is_some_and(|level| level > GATHER_ZONE_COMBAT_CAP);
                let seeded_hitpoints = hitpoints_base == Some(99);
                let here = snapshot.tile() == Some((start_tile.x, start_tile.z, start_tile.level));
                let cook_red = snapshot.quest_statuses().iter().any(|row| {
                    row.name == "Cook's Assistant" && row.status() == QuestListStatus::NotStarted
                });
                if client.ingame
                    && client.scene_state == 2
                    && wielded
                    && empty
                    && level_one
                    && (!state.seed_combat_stats || seeded_combat)
                    && (!state.seed_combat_stats || seeded_hitpoints)
                    && here
                    && cook_red
                {
                    state.ready = true;
                    state.initial = Some(json!({
                        "woodcutting": 1,
                        "combat_level": combat_level,
                        "hitpoints_base": hitpoints_base,
                        "bronze_axe_wielded": true,
                        "inventory_empty": true,
                        "tile": [start_tile.x, start_tile.z, start_tile.level],
                        "cook_colour": "notStarted"
                    }));
                }
            }
            _ => {}
        }
        Ok(())
    })();
    if let Err(error) = result {
        state.error = Some(error);
    }
}

fn evidence() -> PathBuf {
    PathBuf::from(std::env::var_os("BOT_EVIDENCE_DIR").expect("BOT_EVIDENCE_DIR"))
}
fn start_fixture(journal: bool) -> (Play, String, Arc<Mutex<Fixture>>) {
    start_fixture_with_options(journal, None, false)
}

fn start_fixture_at(
    journal: bool,
    start_tile: Option<WorldTile>,
) -> (Play, String, Arc<Mutex<Fixture>>) {
    let seed_combat_stats = start_tile.is_some();
    start_fixture_with_options(journal, start_tile, seed_combat_stats)
}

fn start_fixture_with_options(
    journal: bool,
    start_tile: Option<WorldTile>,
    seed_combat_stats: bool,
) -> (Play, String, Arc<Mutex<Fixture>>) {
    script_js::install_cards().expect("install canonical Sherlock card for live API fixtures");
    api::hostlog::set_debug(true);
    assert!(api::hostlog::install_sink(&ADMISSION));
    let home = PathBuf::from(std::env::var_os("HOME").expect("throwaway HOME"));
    assert!(
        home.starts_with(evidence()),
        "HOME must be under BOT_EVIDENCE_DIR"
    );
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(45594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2080);
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        cache_dir: Some(
            api::test_support::local_asset_path(
                "BOT_CACHE_DIR",
                "",
                "copied local R289 cache snapshot for Script API live proof",
            )
            .expect("Script API cache should be preflighted"),
        ),
        unpack_dir: Some(home.join(".274bot/unpack-289")),
        vault_path: Some(home.join("vault")),
        nav_pack: Some(
            api::test_support::local_asset_path(
                "WORLD_NAV_PACK",
                "",
                "local R289 navigation pack for Script API live proof",
            )
            .expect("Script API navigation pack should be preflighted"),
        ),
        engine_dir: Some(
            api::test_support::local_asset_path(
                "WORLD_ENGINE_DIR",
                "",
                "local R289 engine content for Script API live proof",
            )
            .expect("Script API engine content should be preflighted"),
        ),
        ..ProfileOptions::default()
    };
    let template = options
        .resolve(None)
        .expect("local 289 profile")
        .prepare_template()
        .expect("cached client template");
    let entries = mint_live_entries(&mint_live_names(1));
    let account = entries[0].0.clone();
    let state = Arc::new(Mutex::new(Fixture {
        start_tile,
        seed_combat_stats,
        ..Fixture::default()
    }));
    let frame_state = Arc::clone(&state);
    let frame_account = account.clone();
    // The fixture relogs a scriptless bot before its script starts, so it
    // owns the arm that posts the operator Login after its own logout.
    let arm = SlotArm::new(274_289_303, true);
    let frame_arm = Arc::clone(&arm);
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, name, input| {
            if name == frame_account {
                fixture_frame(client, input.hold, journal, &frame_state, &frame_arm);
            }
        },
    )
    .expect("start public Play");
    play.try_spawn_slot(
        Profile {
            username: account.clone(),
            password: entries[0].1.clone().into(),
            uid: 274_289_303,
            settings: ProfileSettings::default(),
        },
        None,
        None,
        Some(arm),
    )
    .expect("spawn fresh local fixture account");
    wait_for(&play, &account, &state, Duration::from_secs(240), |_| {
        state.lock().unwrap().ready
    });
    (play, account, state)
}
fn wait_for(
    play: &Play,
    account: &str,
    fixture: &Mutex<Fixture>,
    timeout: Duration,
    mut ready: impl FnMut(&Play) -> bool,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let error = fixture.lock().unwrap().error.clone();
        assert!(error.is_none(), "fixture error: {error:?}");
        if ready(play) {
            return;
        }
        let phase = fixture.lock().unwrap().phase;
        assert!(Instant::now() < deadline, "live cell timeout: account={account}, fixture phase={phase}, state={:?}, error={:?}, native={:?}", play.script_state(account), play.script_last_error(account), play.script_native_status(account));
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn field(status: &ScriptStatus, key: &str) -> i64 {
    status
        .fields
        .iter()
        .find_map(|field| match (&field.value, field.key == key) {
            (StatusValue::Integer(value), true) => Some(*value),
            _ => None,
        })
        .unwrap_or(0)
}
fn take_logs(play: &Play, account: &str, logs: &mut Vec<String>) {
    logs.extend(play.script_take_pending_logs(account));
}
fn logged(logs: &[String], prefix: &str) -> Value {
    let rows = logs
        .iter()
        .filter_map(|line| line.strip_prefix(prefix))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "expected one {prefix:?}, logs={logs:?}");
    serde_json::from_str(rows[0]).expect("exact public JSON log")
}
fn no_drops(account: &str) {
    assert!(
        ADMISSION
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|(slot, _)| slot != account),
        "sample emitted competing game rows"
    );
}
fn save(name: &str, value: &Value) {
    let path = evidence().join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    println!("receipt={}\n{value}", path.display());
}

fn prepared_native(source: &str) -> script_js::PreparedTestOrigin {
    let origin = script_js::native_fixture(source);
    script_js::prepare_raw_origin_for_test("script-api-live.js", &origin, &[])
        .expect("native script API fixture prepares")
}

/// The nearby Giant spider zone is a LevelRule zone with combat cap 4.
const GATHER_ZONE_COMBAT_CAP: i32 = 4;

/// The October 15 live pass yielded 58 logs in 1,116 game ticks. The
/// 900-second window is 1,500 ticks: 56 yields need
/// ceil(56 * 1,116 / 58) = 1,078 ticks and 54 drops need
/// ceil(54 * 1,116 / 58) = 1,040 ticks, leaving 422 and 460 ticks of
/// headroom. The sample keeps its 56-log stop target; require 54 drops.
const GATHER_LOG_TARGET: i64 = 56;
const GATHER_DROP_TARGET: i64 = 54;

fn gather_cell() {
    let (play, account, fixture) = start_fixture_with_options(false, None, true);
    fixture.lock().unwrap().started = true;
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../script-js/examples/gather_quest_v2.ts"),
    )
    .expect("runnable sample");
    let prepared = script_js::prepare_raw_origin_for_test("gather_quest_v2.ts", &source, &[])
        .expect("manifest-bearing gather sample prepares");
    let mut settings_bag = serde_json::Map::new();
    settings_bag.insert("logs".to_owned(), json!(GATHER_LOG_TARGET));
    play.script_start_load(
        &account,
        prepared.js,
        prepared.checked,
        Some(settings_bag),
        vec![],
    )
    .expect("Load authoritative sample");
    let mut running_seen = false;
    let mut gathering_seen = false;
    let mut status_yielded = 0;
    let mut status_dropped = 0;
    let mut logs = Vec::new();
    let mut phases = Vec::new();
    let mut snapshot_phases = Vec::new();
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(900),
        |play| {
            take_logs(play, &account, &mut logs);
            let page = play.script_api_live_probe(&account);
            if page["gather"]["phase"] == "running" {
                let phase = page["gather"]["status"]["phase"]
                    .as_str()
                    .unwrap_or("no-status")
                    .to_owned();
                if snapshot_phases.last() != Some(&phase) {
                    snapshot_phases.push(phase.clone());
                }
                if phase == "gathering" && running_seen {
                    gathering_seen = true;
                }
                if phase != "gathering" {
                    running_seen = true;
                }
            }
            if let Some(status) = play.script_native_status(&account) {
                status_yielded = status_yielded.max(field(&status, "yielded"));
                status_dropped = status_dropped.max(field(&status, "dropped"));
                if let Some(phase) = status.fields.iter().find_map(|field| match &field.value {
                    StatusValue::Text(value) if field.key == "phase" => Some(value.to_string()),
                    _ => None,
                }) {
                    if phases.last() != Some(&phase) {
                        phases.push(phase);
                    }
                }
            }
            if let Some(receipt) = play.script_lifecycle_receipt(&account) {
                assert_eq!(
                    receipt.reason, "done",
                    "sample stopped early: {receipt:?}; logs={logs:?}"
                );
                return play.script_state(&account) == script::RunState::Idle;
            }
            assert_ne!(
                play.script_state(&account),
                script::RunState::Error,
                "sample failed: {:?}; logs={logs:?}",
                play.script_last_error(&account)
            );
            false
        },
    );
    take_logs(&play, &account, &mut logs);
    let stopped = logged(&logs, "gather stop: ");
    let terminal = logged(&logs, "gather outcome: ");
    let paths = logged(&logs, "quest paths: ");
    let progress = logged(&logs, "quest progress: ");
    assert!(
        running_seen && gathering_seen,
        "public snapshot must progress running -> gathering"
    );
    assert!(
        status_yielded >= GATHER_LOG_TARGET && status_dropped >= GATHER_DROP_TARGET,
        "measured-rate targets: yielded>={GATHER_LOG_TARGET}, dropped>={GATHER_DROP_TARGET}; observed {status_yielded}/{status_dropped}"
    );
    assert_eq!(stopped, json!({"ok":true,"value":null}));
    assert_eq!(terminal["kind"], "done");
    assert_eq!(terminal["value"]["end"], "stopped");
    assert!(terminal["value"]["counts"]["yielded"].as_u64().unwrap() >= GATHER_LOG_TARGET as u64);
    assert!(terminal["value"]["counts"]["dropped"].as_u64().unwrap() >= GATHER_DROP_TARGET as u64);
    assert_eq!(progress["kind"], "done");
    assert_eq!(progress["value"]["end"], "done");
    let row = &progress["value"]["row"];
    assert_eq!(row["colour"], "notStarted");
    assert_eq!(row["stage"], json!({"state":"known","value":"cook:0"}));
    assert_eq!(row["complete"], "false");
    assert_eq!(row["journal_read"], false);
    assert_eq!(paths["ok"], true);
    let after = play.script_api_live_probe(&account);
    assert_eq!(
        after["seat_present"], false,
        "script stop must remove the API seat"
    );
    assert_eq!(after["foreground"], false);
    no_drops(&account);
    let state = fixture.lock().unwrap();
    assert_eq!(
        state.emptied.len() as u64,
        terminal["value"]["counts"]["dropped"].as_u64().unwrap(),
        "every dropped log slot must be observed empty"
    );
    let receipt = json!({
        "cell": "script_api_gather_and_quest",
        "account": account,
        "fixture": state.initial,
        "running_seen": running_seen,
        "gathering_seen": gathering_seen,
        "logs_target": GATHER_LOG_TARGET,
        "drops_target": GATHER_DROP_TARGET,
        "snapshot_phases": snapshot_phases,
        "status_yielded": status_yielded,
        "status_dropped": status_dropped,
        "phases": phases,
        "empty_slots": state.emptied,
        "stop": stopped,
        "gather_outcome": terminal,
        "paths": paths,
        "progress": progress,
        "after_stop": after,
        "lifecycle": play.script_lifecycle_receipt(&account),
        "competing_game_rows": 0,
    });
    drop(state);
    save("script-api-gather-and-quest-receipt.json", &receipt);
}

fn assert_journal(out: &Value, ordinal: u32, probe: &Value) {
    assert_eq!(out["kind"], "done");
    assert_eq!(out["value"]["end"], "done");
    let row = &out["value"]["row"];
    let stage = format!("romeojuliet:{ordinal}");
    assert_eq!(row["colour"], "inProgress");
    assert_eq!(row["stage"], json!({"state":"known","value":stage}));
    assert_eq!(row["rule"], row["stage"]);
    assert_eq!(row["complete"], "false");
    assert_eq!(row["journal_read"], true);
    assert_eq!(
        row["evidence"], probe["journal"]["closed"],
        "public progress must use the actual closed-journal stamp"
    );
    assert!(
        row["evidence"]["sequence"].as_u64().unwrap()
            > probe["journal"]["acquired"]["sequence"].as_u64().unwrap()
    );
    assert_eq!(
        probe["paint_hidden"], false,
        "completed read must release quiet lease"
    );
    assert_eq!(probe["foreground"], false);
}
fn journal_cell() {
    let (play, account, fixture) = start_fixture(true);
    fixture.lock().unwrap().started = true;
    let prepared = prepared_native(JOURNAL_SCRIPT);
    play.script_start_load(&account, prepared.js, prepared.checked, None, vec![])
        .expect("Load public journal consumer");
    let mut logs = Vec::new();
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        logs.iter().any(|line| line.starts_with("journal 30: "))
    });
    let paths = logged(&logs, "quest paths: ");
    let romeo = paths["value"]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "romeojuliet")
        .expect("released R&J");
    assert_eq!(romeo["journal"], true);
    assert_eq!(
        romeo["stages"],
        json!([
            "romeojuliet:0",
            "romeojuliet:10",
            "romeojuliet:20",
            "romeojuliet:30",
            "romeojuliet:40",
            "romeojuliet:50",
            "romeojuliet:60",
            "romeojuliet:100"
        ])
    );
    let first = logged(&logs, "journal 30: ");
    let first_probe = play.script_api_live_probe(&account);
    assert_journal(&first, 30, &first_probe);
    let first_hidden = fixture.lock().unwrap().hidden_journals;
    assert!(
        first_hidden > 0,
        "S1b client paint-hidden seam never observed the owned journal"
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(5), |_| {
        fixture.lock().unwrap().normal_closed
    });
    {
        let mut state = fixture.lock().unwrap();
        state.normal_closed = false;
        state.advance = true;
    }
    wait_for(&play, &account, &fixture, Duration::from_secs(20), |_| {
        fixture.lock().unwrap().advance_ready
    });
    play.script_api_live_read_again(&account).unwrap();
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        logs.iter().any(|line| line.starts_with("journal 40: "))
    });
    let second = logged(&logs, "journal 40: ");
    let second_probe = play.script_api_live_probe(&account);
    assert_journal(&second, 40, &second_probe);
    assert_ne!(first["value"]["token"], second["value"]["token"]);
    wait_for(&play, &account, &fixture, Duration::from_secs(5), |_| {
        fixture.lock().unwrap().normal_closed
    });
    let state = fixture.lock().unwrap();
    assert!(
        state.hidden_journals > first_hidden,
        "second read must also own hidden journal paint"
    );
    let receipt = json!({"cell":"script_api_progress_journal", "account":account, "seed":"setvar rjquest 30", "advance":"setvar rjquest 40", "paths":paths, "first":first, "first_host":first_probe, "second":second, "second_host":second_probe, "first_hidden_frames":first_hidden, "hidden_frames":state.hidden_journals, "normal_closed":state.normal_closed});
    drop(state);
    no_drops(&account);
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        play.script_state(&account) == script::RunState::Idle
    });
    assert_eq!(play.script_api_live_probe(&account)["seat_present"], false);
    save("script-api-progress-journal-receipt.json", &receipt);
}

/// R4S-2 live: the compatibility journal (`api.questJournalBegin` and
/// `api.questJournalRun`) reads the real Romeo & Juliet journal twice in a
/// row. The second begin runs in the same isolate tick in which the first
/// read settles, so the host can see the second read's lease without a free
/// sample between them. Each read must own its own hold.
const COMPAT_JOURNAL_TWICE_SCRIPT: &str = r#"
let started = false;
export async function tick(api) {
  if (started) return;
  started = true;
  for (const read of [1, 2]) {
    const begin = api.questJournalBegin({ name: 'Romeo & Juliet' });
    api.log('compat journal ' + read + ' begin: ' + JSON.stringify(begin));
    if (!begin.ok) return;
    api.log('compat journal ' + read + ': ' + JSON.stringify(await api.questJournalRun(begin.value)));
  }
}
"#;

fn compat_journal_twice_cell() {
    let (play, account, fixture) = start_fixture(true);
    fixture.lock().unwrap().started = true;
    let prepared = prepared_native(COMPAT_JOURNAL_TWICE_SCRIPT);
    play.script_start_load(&account, prepared.js, prepared.checked, None, vec![])
        .expect("Load the compatibility journal consumer");
    let mut logs = Vec::new();
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |play| {
        take_logs(play, &account, &mut logs);
        logs.iter()
            .any(|line| line.starts_with("compat journal 2: "))
            || logs
                .iter()
                .any(|line| line.contains(" begin: ") && line.contains("\"ok\":false"))
    });
    let reads = [1, 2].map(|read| {
        let begin = logged(&logs, &format!("compat journal {read} begin: "));
        assert_eq!(begin["ok"], true, "read {read} begin: {begin}");
        let outcome = logged(&logs, &format!("compat journal {read}: "));
        assert_eq!(outcome["kind"], "done", "read {read}: {outcome}");
        assert_eq!(outcome["value"]["kind"], "done", "read {read}: {outcome}");
        let lines = outcome["value"]["lines"].as_array().expect("journal lines");
        assert!(!lines.is_empty(), "read {read} returned no journal lines");
        assert_eq!(
            outcome["value"]["token"], begin["value"]["token"],
            "read {read} settled its own token"
        );
        outcome
    });
    assert_ne!(
        reads[0]["value"]["token"], reads[1]["value"]["token"],
        "each read is its own transaction"
    );
    assert_eq!(
        reads[0]["value"]["lines"], reads[1]["value"]["lines"],
        "both reads saw the same quest stage"
    );
    wait_for(&play, &account, &fixture, Duration::from_secs(5), |_| {
        fixture.lock().unwrap().normal_closed
    });
    let hidden = fixture.lock().unwrap().hidden_journals;
    assert!(hidden > 0, "the reads never owned the hidden journal paint");
    no_drops(&account);
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        play.script_state(&account) == script::RunState::Idle
    });
    save(
        "script-api-compat-journal-twice-receipt.json",
        &json!({"cell":"script_api_compat_journal_twice", "account":account,
               "seed":"setvar rjquest 30", "reads":reads, "hidden_frames":hidden}),
    );
}

const RUNTIME_SOURCE: &str = r#"
let ticks = 0;
export function tick(api) {
  if (++ticks === 1) {
    const keep = api.combatKeepNames({ food: 'Trout', style: 'melee', extra: ['Coins'] });
    api.log('runtime-keep: ' + JSON.stringify(keep));
    api.log('runtime-started');
  }
}
"#;

fn runtime_cell(restart: bool) {
    let (play, account, fixture) = start_fixture(false);
    let mut generations = Vec::new();
    let mut observed_logs = Vec::new();
    let prepared = prepared_native(RUNTIME_SOURCE);
    for _ in 0..if restart { 2 } else { 1 } {
        play.script_start_load(
            &account,
            prepared.js.clone(),
            prepared.checked,
            None,
            vec![],
        )
        .expect("start real Load script");
        generations.push(play.script_runtime_generation(&account).unwrap());
        let mut logs = Vec::new();
        wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
            take_logs(play, &account, &mut logs);
            play.script_state(&account) == script::RunState::Running
                && logs.iter().any(|line| line.contains("runtime-started"))
        });
        let keep = logged(&logs, "runtime-keep: ");
        assert_eq!(keep["ok"], true, "{keep}");
        let names = keep["value"].as_array().expect("native keep-list result");
        // `keep_list::food_forms` lowercases food names; extras retain their spelling.
        assert!(names.iter().any(|name| name == "trout"), "{keep}");
        assert!(names.iter().any(|name| name == "Coins"), "{keep}");
        observed_logs.push(logs);
        play.script_stop(&account);
        wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
            play.script_state(&account) == script::RunState::Idle
        });
    }
    if restart {
        assert_ne!(
            generations[0], generations[1],
            "restart needs a new generation"
        );
    }
    let status = script_js::isolate_capacity_status().unwrap();
    assert_eq!(status.retiring_gameplay, 0, "physical join: {status:?}");
    no_drops(&account);
    save(
        if restart {
            "script-runtime-restart-receipt.json"
        } else {
            "script-runtime-load-receipt.json"
        },
        &json!({"cell":if restart {"script_runtime_restart"} else {"script_runtime_load"},
                "account":account, "generations":generations, "logs":observed_logs,
                "retiring_gameplay":status.retiring_gameplay, "quarantined":status.quarantined}),
    );
}

fn runtime_load_cell() {
    runtime_cell(false);
}

fn runtime_restart_cell() {
    runtime_cell(true);
}

#[cfg(feature = "luau")]
const LUAU_WAIT_SOURCE: &str = r#"
local did_wait = false
return {
    tick = function(api)
        if not did_wait then
            did_wait = true
            api.log("luau-first")
            api.wait()
            api.log("luau-resumed")
        else
            api.log("luau-next-tick")
        end
    end,
    onStop = function(reason)
        api.log("luau-hook:" .. reason)
    end,
}
"#;

#[cfg(feature = "luau")]
const LUAU_SELF_STOP_SOURCE: &str = r#"
return {
    tick = function(api)
        api.log("luau-self-first")
        api.stop("live done")
        api.log("luau-after-stop")
    end,
    onStop = function(reason)
        api.log("luau-hook:" .. reason)
    end,
}
"#;

#[cfg(feature = "luau")]
fn luau_lifecycle_cell() {
    let (play, account, fixture) = start_fixture(false);
    play.script_start_luau(&account, LUAU_WAIT_SOURCE.into(), None)
        .expect("start real Luau run");
    let mut logs = Vec::new();
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Running
            && logs.iter().any(|line| line.contains("luau-first"))
            && play.script_wait_inflight_for_test(&account) > 0
    });
    let wait_inflight_observed = play.script_wait_inflight_for_test(&account);
    assert!(
        wait_inflight_observed > 0,
        "WaitEnqueued did not reach the slot watchdog"
    );
    play.script_pause(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        play.script_state(&account) == script::RunState::Paused
    });
    let paused_at = fixture.lock().unwrap().observed_tick;
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        fixture
            .lock()
            .unwrap()
            .observed_tick
            .wrapping_sub(paused_at)
            >= 3
    });
    take_logs(&play, &account, &mut logs);
    assert!(
        !logs.iter().any(|line| line.contains("luau-resumed")),
        "paused run resumed its waiting tick: {logs:?}"
    );
    play.script_resume(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        logs.iter().any(|line| line.contains("luau-resumed"))
            && logs.iter().any(|line| line.contains("luau-next-tick"))
            && play.script_wait_inflight_for_test(&account) == 0
    });
    let (wait_enqueued_drained, wait_settled_drained) =
        play.script_surface_wait_drained_for_test(&account);
    assert!(
        wait_enqueued_drained > 0 && wait_settled_drained > 0,
        "surface/client.rs wait lifecycle did not drain through the slot: enqueued={wait_enqueued_drained}, settled={wait_settled_drained}"
    );
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Idle
            && logs.iter().any(|line| line.contains("luau-hook:stopped"))
    });

    play.script_start_luau(&account, LUAU_SELF_STOP_SOURCE.into(), None)
        .expect("start second real Luau run");
    let mut self_logs = Vec::new();
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut self_logs);
        play.script_state(&account) == script::RunState::Idle
            && play
                .script_lifecycle_receipt(&account)
                .is_some_and(|receipt| receipt.state == script::slot::ScriptTerminalState::Stopped)
    });
    take_logs(&play, &account, &mut self_logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("self-stop receipt");
    assert_eq!(receipt.reason, "live done");
    assert_eq!(
        self_logs
            .iter()
            .filter(|line| line.contains("luau-hook:live done"))
            .count(),
        1,
        "self-stop runs the hook once: {self_logs:?}"
    );
    assert!(
        !self_logs
            .iter()
            .any(|line| line.contains("luau-after-stop")),
        "api.stop must end the tick: {self_logs:?}"
    );
    no_drops(&account);
    save(
        "luau-lifecycle-receipt.json",
        &json!({"cell":"luau_lifecycle","account":account,"logs":logs,"self_logs":self_logs,
                "paused_game_ticks":3,"wait_inflight_observed":wait_inflight_observed,
                "wait_enqueued_drained":wait_enqueued_drained,
                "wait_settled_drained":wait_settled_drained,
                "wait_lifecycle_path":"Luau SeatClient lifecycle -> seat batch -> SlotScript.seat_lifecycle -> drain_lifecycle",
                "host_commit":build_stamp::COMMIT,"host_dirty":build_stamp::DIRTY,
                "receipt":receipt}),
    );
}

/// The landing tile of the mainland hop in the Lumbridge courtyard; the
/// `walk` scenario walks south from it over open tiles to (3220, 3216, 0)
/// and (3220, 3212, 0), with 90 game ticks for each step
/// (`crates/scenario/src/scenarios/navigation.rs:6-63`).
#[cfg(feature = "luau")]
const LUAU_WALK_START: WorldTile = WorldTile {
    x: 3220,
    z: 3222,
    level: 0,
};
/// Draynor approach tile used to exercise the nearby Bank booth on Engine A.
#[cfg(feature = "luau")]
const LUAU_BANK_START: WorldTile = WorldTile {
    x: 3093,
    z: 3243,
    level: 0,
};
#[cfg(feature = "luau")]
const LUAU_WALK_DEST: WorldTile = WorldTile {
    x: 3220,
    z: 3212,
    level: 0,
};

/// Two awaited walks of the `walk` scenario route, each reported with the
/// walk result and the status reads of the frame that resumed it.
#[cfg(feature = "luau")]
const LUAU_WALK_SOURCE: &str = r#"
local started = false
local function report(label, result)
    local x, z, level = api.status.tile()
    api.log(label, tostring(result.ok), tostring(result.failed), result.x, result.z,
        result.level, tostring(result.no_path), result.cancel_reason, result.request_id,
        result.seq, x, z, level, api.status.tick())
end
return {
    tick = function(api)
        if started then
            return
        end
        started = true
        api.log("luau-walk-start", api.status.tick(), api.status.tile())
        report("luau-walk-mid", api.walk.to(3220, 3216, 0))
        report("luau-walk-dest", api.walk.to(3220, 3212, 0))
        api.stop("luau walk done")
    end,
}
"#;

#[cfg(feature = "luau")]
fn logged_fields(logs: &[String], label: &str) -> Vec<String> {
    let prefix = format!("{label}\t");
    let rows = logs
        .iter()
        .filter_map(|line| line.strip_prefix(&prefix))
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 1, "expected one {label:?}, logs={logs:?}");
    rows[0].split('\t').map(str::to_owned).collect()
}

#[cfg(feature = "luau")]
fn luau_walk_cell() {
    let (play, account, fixture) = start_fixture_with_options(false, Some(LUAU_WALK_START), false);
    play.script_start_luau(&account, LUAU_WALK_SOURCE.into(), None)
        .expect("start real Luau walk run");
    let mut logs = Vec::new();
    // Two steps of at most 90 game ticks each (the `walk` scenario budget)
    // are 108 s at 600 ms per tick; the harness deadline adds start-up.
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(240),
        |play| {
            take_logs(play, &account, &mut logs);
            play.script_state(&account) == script::RunState::Idle
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| {
                        receipt.state == script::slot::ScriptTerminalState::Stopped
                    })
        },
    );
    take_logs(&play, &account, &mut logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("self-stop receipt");
    assert_eq!(receipt.reason, "luau walk done", "logs={logs:?}");

    let start = logged_fields(&logs, "luau-walk-start");
    assert_eq!(start[1..], ["3220", "3222", "0"], "seeded start tile");
    let mid = logged_fields(&logs, "luau-walk-mid");
    let dest = logged_fields(&logs, "luau-walk-dest");
    for (fields, tile) in [(&mid, (3220, 3216)), (&dest, (3220, 3212))] {
        let (x, z) = (tile.0.to_string(), tile.1.to_string());
        assert_eq!(
            fields[..7],
            [
                "true",
                "false",
                x.as_str(),
                z.as_str(),
                "0",
                "false",
                "none"
            ],
            "walk result: ok, not failed, its own target, a route, no manual cancel"
        );
        assert_ne!(fields[7], "0", "the seat client stamped the walk id");
        assert_eq!(
            fields[9..12],
            [x.as_str(), z.as_str(), "0"],
            "api.status.tile on the frame that resumed the walk is the target"
        );
    }
    let observed = fixture.lock().unwrap().observed_tile;
    assert_eq!(
        observed,
        Some(LUAU_WALK_DEST),
        "the client observes the player on the arrival tile"
    );
    no_drops(&account);
    save(
        "luau-walk-receipt.json",
        &json!({"cell":"luau_walk","account":account,"logs":logs,
                "start":[LUAU_WALK_START.x,LUAU_WALK_START.z,LUAU_WALK_START.level],
                "dest":[LUAU_WALK_DEST.x,LUAU_WALK_DEST.z,LUAU_WALK_DEST.level],
                "observed_tile":observed.map(|tile| [tile.x, tile.z, tile.level]),
                "walk_mid":mid,"walk_dest":dest,
                "walk_path":"api.walk.to -> SeatClient::queue_walk -> seat batch -> SlotScript seat admission -> host route job -> lent walk outcome -> coroutine resume",
                "host_commit":build_stamp::COMMIT,"host_dirty":build_stamp::DIRTY,
                "receipt":receipt}),
    );
}

#[cfg(feature = "luau")]
fn luau_bank_interact_cell() {
    let (play, account, fixture) = start_fixture_with_options(false, Some(LUAU_BANK_START), false);
    fixture.lock().unwrap().seed_bank_axe = true;
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        fixture.lock().unwrap().bank_booth.is_some()
    });
    let booth = fixture
        .lock()
        .unwrap()
        .bank_booth
        .expect("the fixture found a nearby Bank booth");
    assert_eq!(booth.op, 2, "Use-quickly is the booth's second action");
    fixture.lock().unwrap().started = true;
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.bank_axe_visible && state.bank_axe_inventory_after_seed.is_some()
    });
    let (inventory_before_seed, inventory_after_seed) = {
        let state = fixture.lock().unwrap();
        (
            state
                .bank_axe_inventory_before
                .expect("fixture recorded the pack before seeding"),
            state
                .bank_axe_inventory_after_seed
                .expect("fixture observed the seeded axe"),
        )
    };
    assert!(
        inventory_after_seed > inventory_before_seed,
        "the fixture added a bronze axe to its own pack"
    );
    let interaction_source = format!(
        r#"
local started = false
return {{
    tick = function(api)
        if started then return end
        started = true
        api.log("l4b:interact", api.status.tick())
        api.interact.loc({}, {}, {}, "Use-quickly", {})
        api.wait()
        api.log("l4b:interaction-wait", api.status.tick())
        api.stop("l4b interaction complete")
    end,
}}
"#,
        booth.tile.x, booth.tile.z, booth.tile.level, booth.id
    );
    let mut logs = Vec::new();
    play.script_start_luau(&account, interaction_source, None)
        .expect("start real Luau Bank booth interaction");
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Idle
            && play
                .script_lifecycle_receipt(&account)
                .is_some_and(|receipt| receipt.state == script::slot::ScriptTerminalState::Stopped)
    });
    take_logs(&play, &account, &mut logs);
    let interaction_receipt = play
        .script_lifecycle_receipt(&account)
        .expect("interaction self-stop receipt");
    assert_eq!(
        interaction_receipt.reason, "l4b interaction complete",
        "logs={logs:?}"
    );
    let interaction_stop_reason = interaction_receipt.reason.clone();
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        let state = fixture.lock().unwrap();
        state.bank_open && state.bank_axe_before.is_some()
    });
    let bank_open_after_interaction = fixture.lock().unwrap().bank_open;
    assert!(
        bank_open_after_interaction,
        "OPLOC2 opened the real Bank booth before the native bank plan"
    );

    let bank_source = r#"
local started = false
return {
    tick = function(api)
        if started then return end
        started = true
        api.log("l4b:bank-close-before-open", api.status.tick())
        api.bank.close()
        api.log("l4b:bank-open", api.status.tick())
        api.bank.openNearest()
        api.log("l4b:deposit-all", api.status.tick())
        api.bank.depositAll()
        api.log("l4b:bank-close", api.status.tick())
        api.bank.close()
        api.stop("l4b bank operations complete")
    end,
}
"#;
    play.script_start_luau(&account, bank_source.into(), None)
        .expect("start real Luau bank operations");
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |play| {
        take_logs(play, &account, &mut logs);
        logs.iter()
            .any(|line| line.starts_with("l4b:bank-close-before-open\t"))
    });
    wait_for(
        &play,
        &account,
        &fixture,
        Duration::from_secs(240),
        |play| {
            take_logs(play, &account, &mut logs);
            play.script_state(&account) == script::RunState::Idle
                && play
                    .script_lifecycle_receipt(&account)
                    .is_some_and(|receipt| {
                        receipt.state == script::slot::ScriptTerminalState::Stopped
                    })
        },
    );
    take_logs(&play, &account, &mut logs);
    let receipt = play
        .script_lifecycle_receipt(&account)
        .expect("bank self-stop receipt");
    assert_eq!(
        receipt.reason, "l4b bank operations complete",
        "logs={logs:?}"
    );
    let marker_position = |prefix: &str| {
        logs.iter()
            .position(|line| line.starts_with(prefix))
            .unwrap_or_else(|| panic!("missing marker {prefix:?}: {logs:?}"))
    };
    assert!(
        marker_position("l4b:interact\t") < marker_position("l4b:interaction-wait\t")
            && marker_position("l4b:interaction-wait\t")
                < marker_position("l4b:bank-close-before-open\t")
            && marker_position("l4b:bank-close-before-open\t")
                < marker_position("l4b:bank-open\t")
            && marker_position("l4b:bank-open\t") < marker_position("l4b:deposit-all\t")
            && marker_position("l4b:deposit-all\t") < marker_position("l4b:bank-close\t"),
        "the script interacted, confirmed the open bank, then closed, opened, deposited and closed it: {logs:?}"
    );
    let interaction = logged_fields(&logs, "l4b:interact");
    let interaction_tick = interaction[0]
        .parse::<u64>()
        .expect("interaction marker reports its game tick");
    let (bank_axe_before, bank_axe_after, bank_open, inventory_has_axe, observed_tile) = {
        let state = fixture.lock().unwrap();
        (
            state
                .bank_axe_before
                .expect("fixture observed the open bank before deposit"),
            state
                .bank_axe_after
                .expect("fixture observed the deposited axe"),
            state.bank_open,
            state.inventory.values().any(|&id| id == AXE),
            state.observed_tile,
        )
    };
    assert_eq!(
        bank_axe_after,
        bank_axe_before + (inventory_after_seed - inventory_before_seed),
        "the bank gained the bronze axe the fixture seeded"
    );
    assert!(!bank_open, "the script closed the bank");
    assert!(
        !inventory_has_axe,
        "the pack no longer contains a bronze axe"
    );
    no_drops(&account);
    save(
        "luau-bank-interact-receipt.json",
        &json!({
            "cell": "luau_bank_interact",
            "account": account,
            "logs": logs,
            "interaction_tick": interaction_tick,
            "interaction_stop_reason": interaction_stop_reason,
            "bank_open_after_interaction": bank_open_after_interaction,
            "bank_booth": {
                "name": "Bank booth",
                "id": booth.id,
                "tile": [booth.tile.x, booth.tile.z, booth.tile.level],
                "action": "Use-quickly",
                "operation": booth.op,
                "distance": booth.distance,
            },
            "seeded_axe": {
                "id": AXE,
                "inventory_before": inventory_before_seed,
                "inventory_after_seed": inventory_after_seed,
                "increase": inventory_after_seed - inventory_before_seed,
            },
            "bank_axe": {
                "count_before": bank_axe_before,
                "count_after": bank_axe_after,
                "expected_count_after": bank_axe_before + inventory_after_seed,
                "inventory_contains_axe_after": inventory_has_axe,
                "bank_open_after": bank_open,
            },
            "observed_tile": observed_tile.map(|tile| [tile.x, tile.z, tile.level]),
            "content_evidence": {
                "loc_config": "content/scripts/interface_bank/configs/bank_booth.loc:2,5-6",
                "loc_script": "content/scripts/interface_bank/scripts/bank_booth.rs2:7",
                "deposit_ui": "content/scripts/interface_bank/scripts/bank_deposit.rs2:1-5,8-27",
                "deposit_effect": "content/scripts/interface_bank/scripts/bank.rs2:87-107",
            },
            "host_commit": build_stamp::COMMIT,
            "host_dirty": build_stamp::DIRTY,
            "receipt": receipt,
        }),
    );
}

const RUNTIME_COMPAT_SOURCE: &str = r#"
import { LoopingBot } from '../../api/bot/Bot.js';
import { BotHost } from '../../runtime/BotHost.js';
import { Scheduler } from '../../runtime/Scheduler.js';
import { ScriptActivity } from '../../runtime/ScriptActivity.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';

class RuntimeActivityChild extends LoopingBot {
    constructor(owner) {
        super();
        this.owner = owner;
    }

    onStart() {
        this.owner.childMode = this.settings.str('mode');
    }

    loop() {
        this.owner.childLoops += 1;
        this.finish('activity complete');
    }

    onStop() {
        this.owner.childStops += 1;
    }
}

export default class RuntimeCompatProbe extends LoopingBot {
    frameCount = 0;
    childLoops = 0;
    childStops = 0;
    childMode = null;
    activity = null;
    removeFrame = null;
    completed = false;

    async onStart() {
        this.removeFrame = BotHost.addFrameListener(() => {
            this.frameCount += 1;
        });
        this.activity = new ScriptActivity(message => this.log(message));
        const owner = this;
        await this.activity.start({
            name: 'Runtime activity child',
            settingsSchema: { mode: { default: 'catalog' } },
            create: () => new RuntimeActivityChild(owner),
        }, { mode: 'override' });
    }

    async loop() {
        if (this.completed || this.frameCount < 8) return;
        await this.activity.loop();
        const context = ScriptRunner.ctx;
        const facts = {
            frames: this.frameCount,
            tickCount: BotHost.tickCount,
            state: ScriptRunner.state,
            contextState: context?.state ?? null,
            contextStable: Scheduler.active === context,
            activeEvent: context?.activeEvent ?? null,
            runMeta: ScriptRunner.meta?.name ?? null,
            childMode: this.childMode,
            childLoops: this.childLoops,
            childStops: this.childStops,
            activityOutcome: this.activity.outcome,
            activityChildReleased: this.activity.child === null,
            currentTask: this.activity.currentTask,
        };
        if (facts.activityOutcome?.kind !== 'finished') return;
        this.log('runtime-compat-pass: ' + JSON.stringify(facts));
        this.removeFrame?.();
        this.removeFrame = null;
        this.completed = true;
    }

    onStop() {
        this.removeFrame?.();
        this.removeFrame = null;
        this.activity?.stop();
    }
}
"#;

fn runtime_compat_cell() {
    let (play, account, fixture) = start_fixture(false);
    let prepared =
        script_js::prepare_raw_origin_for_test("runtime-compat.js", RUNTIME_COMPAT_SOURCE, &[])
            .expect("compatibility runtime fixture prepares");
    play.script_start_load(&account, prepared.js, prepared.checked, None, vec![])
        .expect("start runtime compatibility probe");
    let mut logs = Vec::new();
    wait_for(&play, &account, &fixture, Duration::from_secs(45), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Running
            && logs
                .iter()
                .any(|line| line.contains("runtime-compat-pass: "))
    });
    let facts = logged(&logs, "runtime-compat-pass: ");
    assert!(facts["frames"].as_u64().unwrap() >= 8, "{facts}");
    assert!(
        facts["frames"].as_u64().unwrap() > facts["tickCount"].as_u64().unwrap(),
        "client-loop frames must not be a server-tick alias: {facts}"
    );
    assert_eq!(facts["state"], "running", "{facts}");
    assert_eq!(facts["contextState"], "running", "{facts}");
    assert_eq!(facts["contextStable"], true, "{facts}");
    assert_eq!(facts["childMode"], "override", "{facts}");
    assert_eq!(facts["childLoops"], 1, "{facts}");
    assert_eq!(facts["childStops"], 1, "{facts}");
    assert_eq!(
        facts["activityOutcome"],
        json!({"kind":"finished","reason":"activity complete"}),
        "{facts}"
    );
    assert_eq!(facts["activityChildReleased"], true, "{facts}");
    assert_eq!(facts["currentTask"], Value::Null, "{facts}");
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        take_logs(play, &account, &mut logs);
        play.script_state(&account) == script::RunState::Idle
    });
    take_logs(&play, &account, &mut logs);
    no_drops(&account);
    save(
        "script-runtime-compat-receipt.json",
        &json!({
            "cell":"script_runtime_compat",
            "account":account,
            "facts":facts,
            "logs":logs,
            "script_stopped":true,
        }),
    );
}

fn owner_lifecycle_cell() {
    let (mut play, account, fixture) = start_fixture(false);
    let arm = play.arm(&account).expect("fixture owner arm");
    let first = arm.session_key().expect("initial real session");
    assert!(play.slot_connected(&account));

    arm.request_logout();
    play.wake(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        arm.session_key().is_none() && !play.slot_connected(&account)
    });
    assert!(arm.login_latched(), "operator Logout parks without relog");
    let exit = arm.latest_lifecycle_event().expect("owner logout event");
    assert_eq!(exit.scope(), api::identity::Scope::Session(first));
    assert!(
        matches!(
            exit.payload(),
            api::lifecycle::SessionEventOut::SessionEnded {
                cause: api::lifecycle::SessionEndCause::ServerLogout,
                ..
            }
        ),
        "an operator Logout must retain the real server-logout cause: {exit:?}"
    );
    let ended = format!("{exit:?}");

    arm.request_relog();
    play.wake(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |_| {
        arm.session_key().is_some_and(|key| key != first) && play.slot_connected(&account)
    });
    let second = arm.session_key().unwrap();
    assert!(!arm.login_latched());

    // The local profile has one fixed Engine A endpoint; this exercises
    // SwitchWorld's turnover/fence, not public-world routing.
    arm.switch_world(1);
    play.wake(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(60), |_| {
        arm.session_key().is_some_and(|key| key != second) && play.slot_connected(&account)
    });
    let third = arm.session_key().unwrap();
    assert_eq!(arm.world_choice(), Some(1));

    play.begin_stop_slot(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |_| {
        !play.slot_stopping(&account)
    });
    play.reap_stopped_slots();
    assert!(play.arm(&account).is_none());
    assert!(arm.stop.load(std::sync::atomic::Ordering::Acquire));
    no_drops(&account);
    save(
        "owner-lifecycle-receipt.json",
        &json!({"cell":"owner_lifecycle", "account":account,
            "sessions":[format!("{first:?}"),format!("{second:?}"),format!("{third:?}")],
            "operator_logout_event":ended, "operator_latch_observed":true,
            "local_world_turnover":1, "stop_joined":true}),
    );
}

fn start_during_catalogue_refresh_cell() {
    use api::identity::Scope;
    use script::preparation::{self, PreparationInput, PreparationKind};

    let (play, account, fixture) = start_fixture(false);
    let root = evidence().join(format!(
        "start-during-catalogue-refresh-{}",
        std::process::id()
    ));
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(scripts.join("Witness")).expect("create isolated catalogue");
    std::fs::write(
        scripts.join("index.ts"),
        "import Witness from './Witness/Witness.js';\nScriptRegistry.register({ name: 'Witness', create: () => new Witness() });\n",
    )
    .expect("write registry");
    std::fs::write(
        scripts.join("Witness/Witness.ts"),
        "export default class Witness extends LoopingBot { loop() {} }\n",
    )
    .expect("write catalogue card");
    let mut library =
        script_js::JsLibrary::with_cache(root.join("scripts.json"), root.join("cache"));
    library
        .register_rs2b0t(&root, &root.join("catalog-path"))
        .expect("register initial fixture");
    std::fs::write(
        scripts.join("Witness/Witness.ts"),
        "export default class Witness extends LoopingBot { loop() { this.revision = 2; } }\n",
    )
    .expect("change catalogued source before refresh");
    let snapshot = Arc::new(
        library
            .catalog_preparation_snapshot(&root)
            .expect("derive bounded catalogue charge"),
    );
    let charge = PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .expect("catalogue charge");
    let mut prepare = Arc::clone(&snapshot).cooperative_prepare();
    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(0);
    let (release_tx, release_rx) = std::sync::mpsc::sync_channel(0);
    let mut first = true;
    let refresh = preparation::submit_cooperative(
        PreparationKind::Catalogue,
        Scope::Process,
        charge,
        move |worker| {
            if std::mem::take(&mut first) {
                entered_tx.send(()).expect("announce catalogue refresh");
                release_rx.recv().expect("release catalogue refresh");
            }
            prepare(worker)
        },
    )
    .expect("admit bounded catalogue refresh");
    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("catalogue refresh entered work service");
    play.script_start(
        &account,
        script::CompiledId("Sherlock"),
        serde_json::Map::new(),
    )
    .expect("accept operator Start during catalogue refresh");
    wait_for(&play, &account, &fixture, Duration::from_secs(10), |play| {
        play.script_state(&account) == script::RunState::Starting
    });
    release_tx.send(()).expect("release catalogue job");
    let mut result = refresh
        .join()
        .expect("catalogue job completed")
        .expect("catalogue preparation");
    assert_eq!(
        result.diff.changed.len(),
        1,
        "real catalogue refresh completed"
    );
    assert!(result.prepare_failed.is_empty(), "catalogue card prepared");
    assert_eq!(result.prepared.len(), 1, "changed card prepared");
    for prepared in std::mem::take(&mut result.prepared) {
        library
            .commit_prepared(prepared)
            .expect("publish prepared catalogue card");
    }
    result.diff.changed.clear();
    library.apply_catalog_diff(result.diff);
    assert!(
        library
            .get(script::ScriptSource::Catalog, "Witness")
            .is_some_and(|card| card.origin.contains("revision = 2")),
        "the refreshed card was published",
    );
    let mut settled = None;
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        if let script::StartPoll::Settled(outcome) = play.script_poll_start(&account) {
            settled = Some(outcome);
            true
        } else {
            false
        }
    });
    assert_eq!(settled, Some(script::StartOutcome::Ready));
    assert_eq!(play.script_state(&account), script::RunState::Running);
    play.script_stop(&account);
    wait_for(&play, &account, &fixture, Duration::from_secs(30), |play| {
        play.script_state(&account) == script::RunState::Idle
    });
    no_drops(&account);
    save(
        "script-api-start-during-catalogue-refresh-receipt.json",
        &json!({"cell":"script_api_start_during_catalogue_refresh", "account":account,
               "catalogue_changed":1, "start_outcome":"Ready"}),
    );
    std::fs::remove_dir_all(&root).expect("remove isolated catalogue fixture");
}

fn bank_interaction_trace_matches(
    trace: &str,
    tick: u64,
    x: i64,
    z: i64,
    level: i64,
    id: i64,
) -> bool {
    let lines = trace.lines().collect::<Vec<_>>();
    let marker = format!("l4b:interact\t{tick}");
    let Some(start) = lines.iter().position(|line| line.ends_with(&marker)) else {
        return false;
    };
    let Some(end) = lines
        .iter()
        .position(|line| line.contains("l4b:interaction-wait\t"))
    else {
        return false;
    };
    if start >= end {
        return false;
    }
    let request = format!(
        "interact Loc {{ x: {x}, z: {z}, level: {level}, action: \"Use-quickly\", id: Some({id}) }}"
    );
    lines[start + 1..end]
        .iter()
        .any(|line| line.contains(&request))
}

#[test]
fn bank_interaction_trace_requires_exact_tick_action_and_target() {
    let trace = "[echo] l4b:interact\t10\n[interact] interact Loc { x: 3091, z: 3242, level: 0, action: \"Use-quickly\", id: Some(2213) }\n[echo] l4b:interaction-wait\t11\n";
    assert!(bank_interaction_trace_matches(
        trace, 10, 3091, 3242, 0, 2213
    ));
    assert!(!bank_interaction_trace_matches(
        trace, 11, 3091, 3242, 0, 2213
    ));
    assert!(!bank_interaction_trace_matches(
        &trace.replace("Use-quickly", "Open"),
        10,
        3091,
        3242,
        0,
        2213
    ));
    assert!(!bank_interaction_trace_matches(
        &trace.replace("Some(2213)", "Some(2214)"),
        10,
        3091,
        3242,
        0,
        2213
    ));
}

fn run_captured(cell: &str, receipt_name: &str, body: fn()) {
    assert_eq!(
        std::env::var("LIVE").as_deref(),
        Ok("1"),
        "ignored live cells require LIVE=1"
    );
    if !api::test_support::game_data_available() {
        return;
    }
    for (name, description) in [
        (
            "BOT_CACHE_DIR",
            "copied local R289 cache snapshot for Script API live proof",
        ),
        (
            "WORLD_NAV_PACK",
            "local R289 navigation pack for Script API live proof",
        ),
        (
            "WORLD_ENGINE_DIR",
            "local R289 engine content for Script API live proof",
        ),
    ] {
        if api::test_support::local_asset_path(name, "", description).is_none() {
            return;
        }
    }
    if std::env::var("SCRIPT_API_CHILD").as_deref() == Ok(cell) {
        body();
        return;
    }
    let trace = evidence().join(format!("{cell}-driver-trace.txt"));
    let status = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            cell,
            "--ignored",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("SCRIPT_API_CHILD", cell)
        .stderr(std::fs::File::create(&trace).unwrap())
        .status()
        .expect("spawn isolated live test process");
    assert!(
        status.success(),
        "live child failed; read {}",
        trace.display()
    );
    let text = std::fs::read_to_string(&trace).unwrap();
    let path = evidence().join(receipt_name);
    let mut receipt: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    let account = receipt["account"].as_str().unwrap();
    let mut packets = BTreeMap::<u64, Vec<u8>>::new();
    let mut requests = 0usize;
    for line in text
        .lines()
        .filter(|line| line.contains(&format!("account={account} ")))
    {
        if line.contains("native-packet account=") {
            let number = |prefix: &str| {
                line.split_whitespace()
                    .find_map(|part| part.strip_prefix(prefix))
                    .unwrap()
                    .parse::<u64>()
                    .unwrap()
            };
            packets
                .entry(number("tick="))
                .or_default()
                .push(number("opcode=") as u8);
        } else if line.contains("native-packets account=") {
            assert!(
                line.contains("decoded=true accepted=true"),
                "Driver rejected or could not decode: {line}"
            );
            requests += 1;
        }
    }
    let revision = client::io::ClientRevision::R289;
    let drop_opcode =
        client::io::map_client_prot(revision, client::io::ClientProt::OPHELD5).id as u8;
    let drop_batches = packets
        .iter()
        .filter(|(_, rows)| rows.contains(&drop_opcode))
        .map(|(&tick, rows)| {
            assert!(
                rows.len() <= 5,
                "Gatherer sent {} packets on drop tick {tick}",
                rows.len()
            );
            json!({"tick":tick,"opcodes":rows})
        })
        .collect::<Vec<_>>();
    if cell == "script_api_gather_and_quest" {
        let drops = packets
            .values()
            .flatten()
            .filter(|&&opcode| opcode == drop_opcode)
            .count();
        assert_eq!(
            drops as u64,
            receipt["gather_outcome"]["value"]["counts"]["dropped"]
                .as_u64()
                .unwrap()
        );
        assert!(drops >= 54 && !drop_batches.is_empty());
        receipt["driver"] = json!({"trace":trace,"accepted_requests":requests,"drop_packets":drops,"drop_batches":drop_batches,"max_packets_on_drop_tick":packets.values().filter(|rows| rows.contains(&drop_opcode)).map(Vec::len).max()});
    } else if cell == "luau_bank_interact" {
        let interaction_tick = receipt["interaction_tick"].as_u64().unwrap();
        assert_eq!(
            receipt["bank_booth"]["operation"].as_i64(),
            Some(2),
            "the selected Bank booth uses its Use-quickly OPLOC2 action"
        );
        assert_eq!(
            receipt["bank_booth"]["action"].as_str(),
            Some("Use-quickly")
        );
        let tile = receipt["bank_booth"]["tile"].as_array().unwrap();
        let id = receipt["bank_booth"]["id"].as_i64().unwrap();
        assert!(
            bank_interaction_trace_matches(
                &text,
                interaction_tick,
                tile[0].as_i64().unwrap(),
                tile[1].as_i64().unwrap(),
                tile[2].as_i64().unwrap(),
                id,
            ),
            "the driver trace records the exact Bank booth interaction on tick {interaction_tick}"
        );
        assert_eq!(
            receipt["bank_open_after_interaction"].as_bool(),
            Some(true),
            "the real Bank booth opened before the native bank plan"
        );
        receipt["driver"] = json!({
            "trace": trace,
            "accepted_requests": requests,
            "interaction_tick": interaction_tick,
            "interaction_log_matches": true,
            "bank_open_after_interaction": true,
        });
    } else if matches!(
        cell,
        "script_api_gather_walk_options"
            | "script_runtime_load"
            | "script_runtime_restart"
            | "script_runtime_compat"
            | "script_api_start_during_catalogue_refresh"
            | "owner_lifecycle"
            | "script_api_compat_journal_twice"
            | "luau_lifecycle"
            | "luau_walk"
    ) {
        receipt["driver"] =
            json!({"trace":trace,"accepted_requests":requests,"packet_ticks":packets});
    } else {
        let button =
            client::io::map_client_prot(revision, client::io::ClientProt::IF_BUTTON).id as u8;
        let close =
            client::io::map_client_prot(revision, client::io::ClientProt::CLOSE_MODAL).id as u8;
        assert_eq!(
            packets
                .values()
                .flatten()
                .filter(|&&opcode| opcode == button)
                .count(),
            2,
            "exactly one open per journal read"
        );
        assert_eq!(
            packets
                .values()
                .flatten()
                .filter(|&&opcode| opcode == close)
                .count(),
            2,
            "exactly one close per journal read"
        );
        receipt["driver"] =
            json!({"trace":trace,"accepted_requests":requests,"packet_ticks":packets});
    }
    assert!(
        !text.contains("foreground: dropped "),
        "sample emitted competing game rows"
    );
    receipt["verdict"] = json!("PASS");
    std::fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    println!(
        "PASS {cell}: receipt={} trace={}",
        path.display(),
        trace.display()
    );
}

#[cfg(feature = "luau")]
#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and Engine A"]
fn luau_lifecycle() {
    run_captured(
        "luau_lifecycle",
        "luau-lifecycle-receipt.json",
        luau_lifecycle_cell,
    );
}

#[cfg(feature = "luau")]
#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and the real R289 nav pack"]
fn luau_walk() {
    run_captured("luau_walk", "luau-walk-receipt.json", luau_walk_cell);
}

#[cfg(feature = "luau")]
#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and the real R289 nav pack"]
fn luau_bank_interact() {
    run_captured(
        "luau_bank_interact",
        "luau-bank-interact-receipt.json",
        luau_bank_interact_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and Engine A"]
fn script_runtime_load() {
    run_captured(
        "script_runtime_load",
        "script-runtime-load-receipt.json",
        runtime_load_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and Engine A"]
fn script_runtime_restart() {
    run_captured(
        "script_runtime_restart",
        "script-runtime-restart-receipt.json",
        runtime_restart_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine L and baked R289 WORLD_NAV_PACK"]
fn script_runtime_compat() {
    run_captured(
        "script_runtime_compat",
        "script-runtime-compat-receipt.json",
        runtime_compat_cell,
    );
}
#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and Engine A"]
fn owner_lifecycle() {
    run_captured(
        "owner_lifecycle",
        "owner-lifecycle-receipt.json",
        owner_lifecycle_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and Engine A"]
fn script_api_start_during_catalogue_refresh() {
    run_captured(
        "script_api_start_during_catalogue_refresh",
        "script-api-start-during-catalogue-refresh-receipt.json",
        start_during_catalogue_refresh_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, local R289 engine and baked WORLD_NAV_PACK"]
fn script_api_gather_and_quest() {
    run_captured(
        "script_api_gather_and_quest",
        "script-api-gather-and-quest-receipt.json",
        gather_cell,
    );
}
#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and released R289 Romeo & Juliet journal rules"]
fn script_api_progress_journal() {
    run_captured(
        "script_api_progress_journal",
        "script-api-progress-journal-receipt.json",
        journal_cell,
    );
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache and released R289 Romeo & Juliet journal rules"]
fn script_api_compat_journal_twice() {
    run_captured(
        "script_api_compat_journal_twice",
        "script-api-compat-journal-twice-receipt.json",
        compat_journal_twice_cell,
    );
}

fn gather_walk_default(play: &Play, account: &str, fixture: &Arc<Mutex<Fixture>>) -> Value {
    let prepared = prepared_native(WALK_OPTIONS_SOURCE);
    play.script_start_load(account, prepared.js, prepared.checked, None, vec![])
        .expect("start Load Gather walk-options proof");

    wait_for(play, account, fixture, Duration::from_secs(300), |_| {
        fixture.lock().unwrap().walk_arrival.is_some()
    });
    let arrival = fixture
        .lock()
        .unwrap()
        .walk_arrival
        .expect("the default Danger routing reaches Catherby");
    let page = play.script_api_live_probe(account);
    assert_eq!(page["gather"]["phase"], "running");
    play.script_stop(account);
    wait_for(play, account, fixture, Duration::from_secs(30), |play| {
        play.script_state(account) == script::RunState::Idle
    });
    no_drops(account);
    json!({
        "account": account,
        "allowDangerZones": "omitted",
        "allowWilderness": false,
        "arrival": [arrival.x, arrival.z, arrival.level],
        "page": page,
    })
}

fn gather_walk_options_cell() {
    let (play, account, fixture) = start_fixture_at(false, Some(WALK_START_TILE));
    fixture.lock().unwrap().started = true;
    let default = gather_walk_default(&play, &account, &fixture);
    let account = account.to_owned();
    let receipt = json!({
        "account": account,
        "cell": "script_api_gather_walk_options",
        "site": "woodcutting.catherby",
        "start": [WALK_START_TILE.x, WALK_START_TILE.z, WALK_START_TILE.level],
        "default": default,
    });
    save("script-api-gather-walk-options-receipt.json", &receipt);
}

#[test]
#[ignore = "requires LIVE=1, disposable HOME/cache, Engine A and the real R289 nav pack"]
fn script_api_gather_walk_options() {
    run_captured(
        "script_api_gather_walk_options",
        "script-api-gather-walk-options-receipt.json",
        gather_walk_options_cell,
    );
}

/// Offline tests for the pre-relog tutorial gate: the fixture phases only
/// touch the engine through chat, so the decision is a pure function.
#[cfg(test)]
mod pre_relog_tutorial_tests {
    use super::*;

    fn line(sequence: i32, text: &str) -> ChatLineView {
        ChatLineView {
            type_: 0,
            username: None,
            text: text.to_owned(),
            sequence,
        }
    }

    const COMPLETE_REPLY: &str = "get tutorial: 1000";
    const KIT_CLOSE_REPLY: &str = "get tutorial: 1";

    #[test]
    fn fresh_complete_reply_logs_out_without_a_reseed() {
        let chat = [line(7, COMPLETE_REPLY)];
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::Initial, 6, &chat),
            Ok(Some(PreRelogTutorialAction::Logout)),
        );
    }

    #[test]
    fn kit_close_reply_reseeds_then_the_fresh_complete_reply_logs_out() {
        let first = [line(7, KIT_CLOSE_REPLY)];
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::Initial, 6, &first),
            Ok(Some(PreRelogTutorialAction::Reseed)),
        );
        // After the reseed, the pre-reseed `1` (sequence 7) is at the new
        // baseline and the reply after it proves the seeded value.
        let after = [line(9, COMPLETE_REPLY), line(7, KIT_CLOSE_REPLY)];
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::AfterReseed, 8, &after),
            Ok(Some(PreRelogTutorialAction::Logout)),
        );
    }

    #[test]
    fn a_second_kit_close_reply_after_the_one_reseed_is_refused_by_phase() {
        let chat = [line(9, KIT_CLOSE_REPLY)];
        let error = pre_relog_tutorial_step(PreRelogTutorialPhase::AfterReseed, 8, &chat)
            .expect_err("a second tutorial-1 reply must refuse");
        assert!(error.contains("get tutorial: 1"), "{error}");
        assert!(error.contains("fixture phase 10"), "{error}");
    }

    #[test]
    fn a_stale_reply_from_before_the_baseline_is_ignored() {
        let chat = [line(7, COMPLETE_REPLY), line(5, KIT_CLOSE_REPLY)];
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::Initial, 7, &chat),
            Ok(None),
        );
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::AfterReseed, 7, &chat),
            Ok(None),
        );
    }

    #[test]
    fn the_setvar_confirmation_line_is_not_a_get_reply() {
        let chat = [line(8, "set tutorial: to 1000")];
        assert_eq!(
            pre_relog_tutorial_step(PreRelogTutorialPhase::AfterReseed, 7, &chat),
            Ok(None),
        );
    }

    #[test]
    fn any_other_value_is_refused_with_its_value_and_phase() {
        let chat = [line(7, "get tutorial: 250")];
        let error = pre_relog_tutorial_step(PreRelogTutorialPhase::Initial, 6, &chat)
            .expect_err("tutorial 250 must refuse");
        assert!(error.contains("get tutorial: 250"), "{error}");
        assert!(error.contains("fixture phase 2"), "{error}");

        let chat = [line(9, "get tutorial: 2")];
        let error = pre_relog_tutorial_step(PreRelogTutorialPhase::AfterReseed, 8, &chat)
            .expect_err("tutorial 2 after the reseed must refuse");
        assert!(error.contains("get tutorial: 2"), "{error}");
        assert!(error.contains("fixture phase 10"), "{error}");
    }
}
