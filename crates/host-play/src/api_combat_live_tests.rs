//! Live `api.combat` cells on real content: a fresh account, per-account
//! cheats only (stats, items, the user's own prayer, `::tele`), and the
//! Lumbridge cows the game ships. Nothing is spawned.
//!
//! ```sh
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<pfx> BOT_GAME_PORT=44594 BOT_HTTP_PORT=1080 \
//!   BOT_ENGINE_DIR=<engine> BOT_NAV_PACK=<274bot.navpack> \
//!   BOT_CACHE_DIR=<writable clone of the cache snapshot> LIVE_EVIDENCE_DIR=<dir> \
//!   cargo test -p host-play --features live-harness,test-support --lib live_api_combat_ \
//!   -- --ignored --nocapture --test-threads=1
//! ```
use super::*;
use api::snapshot::WorldTile;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vault::{Profile, ProfileSettings};

/// A cow spawn tile inside the Lumbridge east cow field (selected
/// `npc_placements`: cow at 3255,3278).
const COW_FIELD: WorldTile = WorldTile {
    x: 3255,
    z: 3278,
    level: 0,
};
/// The field's cow spawns (3243..3261, 3258..3295) with a margin.
const COW_AREA: &str = "{ min_x: 3240, min_z: 3254, max_x: 3266, max_z: 3299, level: 0 }";
/// Inside the Lumbridge pen, north of the south fence. The gate tile
/// 3235,3295 can see a chicken across that fence, and the machine keeps
/// clicking it.
const CHICKEN_AREA: &str = "{ min_x: 3226, min_z: 3296, max_x: 3238, max_z: 3302, level: 0 }";
const CHICKEN_STANDS: [(i32, i32); 3] = [(3231, 3298), (3233, 3298), (3232, 3299)];
/// Thick Skin, the user's own prayer (varp `prayer0`).
const THICK_SKIN_VARP: i32 = 83;
const SHRIMP_ID: i32 = 315;
/// The whole cell, from login to verdict.
const CELL_DEADLINE: Duration = Duration::from_secs(420);
/// The fight itself, from Load Running to the settled outcome.
const FIGHT_DEADLINE: Duration = Duration::from_secs(240);

fn env_path(key: &str) -> PathBuf {
    PathBuf::from(std::env::var_os(key).unwrap_or_else(|| panic!("live cell requires {key}")))
}

fn profile_options(home: &std::path::Path) -> ProfileOptions {
    let port = std::env::var("BOT_GAME_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(44_594);
    let http_port = std::env::var("BOT_HTTP_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_080);
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(port),
        http_port: Some(http_port),
        vault_path: Some(home.join("vault")),
        cache_dir: Some(env_path("BOT_CACHE_DIR")),
        unpack_dir: Some(home.join("unpack")),
        nav_pack: Some(env_path("BOT_NAV_PACK")),
        engine_dir: Some(env_path("BOT_ENGINE_DIR")),
        ..ProfileOptions::default()
    }
}

/// What the frame hook observed of the account, every frame.
#[derive(Default, Clone, serde::Serialize)]
struct Observed {
    step: usize,
    error: Option<String>,
    ready: bool,
    tile: Option<(i32, i32, i32)>,
    xp: Vec<(i32, i32)>,
    hitpoints: i32,
    hitpoints_base: i32,
    shrimps: i32,
    /// An NPC is in combat and targets this account.
    npc_on_us: bool,
    /// Varp 172. Zero is auto-retaliate on.
    retaliate: i32,
    /// The frame hook presses the retaliate-off toggle until varp 172 is 1.
    want_retaliate_off: bool,
    /// The local player's current animation, or -1 when unknown.
    local_animation: i32,
    /// Varp values of the watched prayer varps, latest frame.
    prayers: Vec<(i32, i32)>,
    /// Every watched prayer varp seen on at least once after `ready`.
    seen_on: Vec<i32>,
    /// `(index, root, available, active)` of the posted side tabs, latest frame.
    side_tabs: Vec<(i32, i32, bool, bool)>,
    /// `(slot, id, count)` of worn items, latest frame.
    equipment: Vec<(i32, i32, i32)>,
    /// The newest chat sequence before the final-session tutorial proof.
    chat_mark: i32,
}

/// The first fixture step that sends a per-cell seed cheat.
const SEED_BASE: usize = 7;

fn varp(snapshot: &api::snapshot::GameSnapshot, index: i32) -> i32 {
    snapshot
        .varps()
        .iter()
        .find(|row| row.index == index)
        .map_or(0, |row| row.value)
}

/// Fresh account → tutorial skip → relog → `cheats` one per frame → tele →
/// wait until on the tile with `ready_check`.
fn frame(
    client: &mut client::client::Client,
    state: &mut Observed,
    cheats: &[String],
    watched: &[i32],
    ready_check: &dyn Fn(&api::snapshot::GameSnapshot) -> bool,
) {
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(client);
    state.tile = snapshot.tile();
    state.xp = snapshot
        .stats()
        .iter()
        .map(|stat| (stat.index, stat.xp))
        .collect();
    let hitpoints = snapshot.stats().iter().find(|stat| stat.index == 3);
    state.hitpoints = hitpoints.map_or(0, |stat| stat.effective);
    state.hitpoints_base = hitpoints.map_or(0, |stat| stat.base);
    state.npc_on_us = snapshot.local_player().is_some_and(|local| {
        let index = local.player.index;
        snapshot.npcs().iter().any(|npc| {
            npc.in_combat
                && npc.target.is_some_and(|target| {
                    target.kind == api::snapshot::ActorKind::Player && target.index == index
                })
        })
    });
    state.retaliate = varp(&snapshot, AUTO_RETALIATE_VARP);
    state.local_animation = snapshot
        .local_player()
        .map_or(-1, |local| local.player.actor.animation);
    state.shrimps = snapshot
        .inventory()
        .iter()
        .filter(|item| item.def.id == SHRIMP_ID)
        .map(|item| item.count)
        .sum();
    state.side_tabs = snapshot
        .side_tabs()
        .iter()
        .map(|tab| (tab.index, tab.root_component_id, tab.available, tab.active))
        .collect();
    state.equipment = snapshot
        .equipment()
        .iter()
        .map(|item| (item.slot, item.def.id, item.count))
        .collect();
    state.prayers = watched
        .iter()
        .map(|index| (*index, varp(&snapshot, *index)))
        .collect();
    if state.ready {
        for (index, value) in state.prayers.clone() {
            if value != 0 && !state.seen_on.contains(&index) {
                state.seen_on.push(index);
            }
        }
        if state.want_retaliate_off && state.retaliate != 1 {
            let _ = api::interact::Interactions::new(&snapshot, client).set_retaliate(false);
        }
        return;
    }
    if state.error.is_some() {
        return;
    }
    let send = |client: &mut client::client::Client, command: &str| -> Result<(), String> {
        if api::interact::cheat(client, command).is_sent() {
            Ok(())
        } else {
            Err(format!("fixture command refused: {command}"))
        }
    };
    let ingame = client.ingame && client.scene_state == 2;
    let result = (|| -> Result<(), String> {
        match state.step {
            0 if ingame => {
                api::interact::mainland_hop(client);
                state.step = 1;
            }
            1 => {
                send(client, "getvar tutorial")?;
                state.step = 2;
            }
            2 if snapshot.chat_lines().iter().any(|line| {
                line.text
                    .to_ascii_lowercase()
                    .contains("get tutorial: 1000")
            }) || snapshot
                .chat_modal_texts()
                .iter()
                .any(|line| line.to_ascii_lowercase().contains("get tutorial: 1000")) =>
            {
                let ifaces = Arc::clone(&client.ifaces);
                if !api::interact::logout(client, &ifaces) {
                    return Err("tutorial logout interface unavailable".into());
                }
                state.step = 3;
            }
            3 if ingame
                && snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available) =>
            {
                state.step = 4;
            }
            // The first session's character-design close can queue
            // `tutorial = 1` after the early proof (content tutorial.rs2), and
            // weapon combat tabs update only past the tutorial
            // (appearance.rs2 update_all). Seed and prove it in this session.
            4 => {
                send(client, "setvar tutorial 1000")?;
                state.chat_mark = snapshot
                    .chat_lines()
                    .iter()
                    .map(|line| line.sequence)
                    .max()
                    .unwrap_or(-1);
                state.step = 5;
            }
            5 => {
                send(client, "getvar tutorial")?;
                state.step = 6;
            }
            6 if snapshot.chat_lines().iter().any(|line| {
                line.sequence > state.chat_mark
                    && line
                        .text
                        .to_ascii_lowercase()
                        .contains("get tutorial: 1000")
            }) =>
            {
                state.step = SEED_BASE;
            }
            step if (SEED_BASE..SEED_BASE + cheats.len()).contains(&step) => {
                send(client, &cheats[step - SEED_BASE])?;
                state.step += 1;
            }
            step if step == SEED_BASE + cheats.len() => {
                send(
                    client,
                    &api::interact::tele_args(COW_FIELD.level, COW_FIELD.x, COW_FIELD.z),
                )?;
                state.step += 1;
            }
            _ if state.step > SEED_BASE + cheats.len() => {
                let near = snapshot.tile().is_some_and(|(x, z, level)| {
                    level == COW_FIELD.level
                        && x.abs_diff(COW_FIELD.x) <= 6
                        && z.abs_diff(COW_FIELD.z) <= 6
                });
                if ingame && near && ready_check(&snapshot) {
                    state.ready = true;
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

fn wait_until(label: &str, deadline: Instant, mut ready: impl FnMut() -> bool) {
    loop {
        if ready() {
            return;
        }
        assert!(Instant::now() < deadline, "{label}: cell deadline passed");
        thread::sleep(Duration::from_millis(50));
    }
}

fn probe(play: &Play, name: &str, expression: &str) -> serde_json::Value {
    script_slot(&play.scripts, name)
        .expect("live script slot")
        .lock()
        .expect("live script slot lock")
        .probe(expression)
        .expect("live Load probe")
}

/// The Load consumer: one fight, its phases from `api.snapshot.combat`, and
/// the settled outcome.
fn consumer(request: &str) -> String {
    format!(
        r#"export const apiVersion = 2;
export function tick(api) {{
  const page = api.snapshot.combat;
  const label = page ? `${{page.phase}}:${{page.status ? page.status.stage : '-'}}` : 'idle';
  if (label !== globalThis.__last) {{
    globalThis.__last = label;
    (globalThis.__phases ||= []).push(label);
  }}
  if (!globalThis.__fight) {{
    globalThis.__fight = api.combat.fight({request});
    globalThis.__fight.then((value) => {{ globalThis.__outcome = value; }});
  }}
}}"#
    )
}

struct Cell {
    play: Play,
    account: String,
    observed: Arc<Mutex<Observed>>,
    evidence: PathBuf,
    started: Instant,
}

fn start_cell(
    label: &str,
    uid: i32,
    cheats: Vec<String>,
    watched: Vec<i32>,
    ready: impl Fn(&api::snapshot::GameSnapshot) -> bool + Send + Sync + 'static,
) -> Cell {
    assert!(
        std::env::var("LIVE").is_ok_and(|value| value == "1"),
        "run this ignored live cell only with LIVE=1"
    );
    let started = Instant::now();
    let evidence = env_path("LIVE_EVIDENCE_DIR");
    std::fs::create_dir_all(&evidence).expect("create evidence dir");
    let home = std::env::var_os("HOME").map(PathBuf::from).expect("HOME");
    let template = profile_options(&home)
        .resolve(None)
        .expect("resolve local-289 profile")
        .prepare_template()
        .expect("prepare local-289 template");
    let names = play_bootstrap::mint_live_names(1);
    let account = names[0].clone();
    let credentials = mint_live_entries(&names);
    let observed = Arc::new(Mutex::new(Observed::default()));
    let frame_observed = Arc::clone(&observed);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame_input| {
            if username != frame_account || frame_input.hold {
                return;
            }
            let mut state = frame_observed.lock().expect("observed lock");
            frame(client, &mut state, &cheats, &watched, &ready);
        },
    )
    .expect("start live Play");
    let (username, password) = &credentials[0];
    let mut cell = Cell {
        play,
        account,
        observed,
        evidence,
        started,
    };
    cell.play
        .try_spawn_slot(
            Profile {
                username: username.clone(),
                password: password.clone().into(),
                uid,
                settings: ProfileSettings::default(),
            },
            None,
            None,
            None,
        )
        .expect("spawn fresh combat account");
    let deadline = started + CELL_DEADLINE;
    wait_until(&format!("{label} fixture"), deadline, || {
        let state = cell.observed.lock().expect("observed lock");
        assert!(state.error.is_none(), "fixture error: {:?}", state.error);
        state.ready
    });
    cell
}

impl Cell {
    fn fight(
        &mut self,
        request: &str,
    ) -> (serde_json::Value, serde_json::Value, Observed, Observed) {
        let before = self.observed.lock().expect("observed lock").clone();
        self.play
            .script_start_load(
                &self.account,
                consumer(request),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .expect("start the api.combat Load consumer");
        let deadline = Instant::now() + FIGHT_DEADLINE;
        wait_until("Load Running", deadline, || {
            self.play.script_state(&self.account) == script::RunState::Running
        });
        wait_until("api.combat outcome", deadline, || {
            !probe(&self.play, &self.account, "globalThis.__outcome || null").is_null()
        });
        // The settled session releases the foreground; let the frame hook
        // observe the post-fight prayer state for a few frames.
        let settle = Instant::now() + Duration::from_secs(3);
        wait_until("post-fight frames", deadline, || Instant::now() >= settle);
        let outcome = probe(&self.play, &self.account, "globalThis.__outcome");
        let phases = probe(&self.play, &self.account, "globalThis.__phases || []");
        let after = self.observed.lock().expect("observed lock").clone();
        (outcome, phases, before, after)
    }

    /// Start the consumer, wait until `trigger` holds while the session is
    /// still live, run `act` once, then wait for the settled outcome.
    /// Returns `(outcome, phases, act evidence, at act, after)`.
    fn fight_and_interrupt(
        &mut self,
        request: &str,
        trigger: impl Fn(&Observed, &Play, &str) -> bool,
        act: impl FnOnce(&Play, &str, &Observed) -> serde_json::Value,
    ) -> (
        serde_json::Value,
        serde_json::Value,
        serde_json::Value,
        Observed,
        Observed,
    ) {
        self.play
            .script_start_load(
                &self.account,
                consumer(request),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .expect("start the api.combat Load consumer");
        let deadline = Instant::now() + FIGHT_DEADLINE;
        wait_until("Load Running", deadline, || {
            self.play.script_state(&self.account) == script::RunState::Running
        });
        wait_until("interruption trigger", deadline, || {
            let outcome = probe(&self.play, &self.account, "globalThis.__outcome || null");
            assert!(
                outcome.is_null(),
                "the fight settled before the interruption: {outcome}"
            );
            let state = self.observed.lock().expect("observed lock").clone();
            trigger(&state, &self.play, &self.account)
        });
        let at = self.observed.lock().expect("observed lock").clone();
        let evidence = act(&self.play, &self.account, &at);
        wait_until("api.combat outcome", deadline, || {
            !probe(&self.play, &self.account, "globalThis.__outcome || null").is_null()
        });
        let settle = Instant::now() + Duration::from_secs(3);
        wait_until("post-fight frames", deadline, || Instant::now() >= settle);
        let outcome = probe(&self.play, &self.account, "globalThis.__outcome");
        let phases = probe(&self.play, &self.account, "globalThis.__phases || []");
        let after = self.observed.lock().expect("observed lock").clone();
        (outcome, phases, evidence, at, after)
    }

    fn receipt(&self, name: &str, receipt: serde_json::Value) {
        let path = self.evidence.join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        println!(
            "{name} ({:.1}s): {}\n{receipt}",
            self.started.elapsed().as_secs_f64(),
            path.display()
        );
    }

    fn snapshot(&self) -> Observed {
        self.observed.lock().expect("observed lock").clone()
    }

    fn cheat_until(&self, label: &str, command: &str, ready: impl Fn(&Observed) -> bool) {
        self.play
            .cheat(&self.account, command)
            .unwrap_or_else(|error| panic!("{label}: cheat {command} refused: {error}"));
        let deadline = Instant::now() + Duration::from_secs(20);
        wait_until(label, deadline, || ready(&self.snapshot()));
    }

    /// `setstat` restores both numbers. One drain then sets effective HP.
    /// The next cheat waits until that pair has landed, so a stale reading
    /// cannot queue a second drain on top of the first. `also` is the extra
    /// ready check. A hit in the window retries the whole pair.
    fn seed_hitpoints(&self, effective: i32, also: impl Fn(&Observed) -> bool) {
        for _ in 0..3 {
            self.play
                .cheat(&self.account, "setstat hitpoints 40")
                .expect("restore hitpoints");
            let restored = Instant::now() + Duration::from_secs(8);
            while Instant::now() < restored {
                let state = self.snapshot();
                if state.hitpoints_base == 40 && state.hitpoints == 40 {
                    break;
                }
                thread::sleep(Duration::from_millis(50));
            }
            if self.snapshot().hitpoints != 40 || self.snapshot().hitpoints_base != 40 {
                continue;
            }
            if effective != 40 {
                self.play
                    .cheat(
                        &self.account,
                        &format!("~stat_drain hitpoints {} 0", 40 - effective),
                    )
                    .expect("drain hitpoints");
            }
            let landed = Instant::now() + Duration::from_secs(8);
            while Instant::now() < landed {
                let state = self.snapshot();
                if state.hitpoints_base == 40 && state.hitpoints == effective && also(&state) {
                    return;
                }
                thread::sleep(Duration::from_millis(50));
            }
        }
        let state = self.snapshot();
        panic!(
            "hitpoints did not settle at {effective}/40 (have {}/{}, npc_on_us {})",
            state.hitpoints, state.hitpoints_base, state.npc_on_us
        );
    }

    fn near(tile: WorldTile) -> impl Fn(&Observed) -> bool {
        move |state| {
            state.tile.is_some_and(|(x, z, level)| {
                level == tile.level && x.abs_diff(tile.x) <= 6 && z.abs_diff(tile.z) <= 6
            })
        }
    }
}

fn xp(observed: &Observed, index: i32) -> i32 {
    observed
        .xp
        .iter()
        .find(|(stat, _)| *stat == index)
        .map_or(0, |(_, xp)| *xp)
}

fn latest_varp(observed: &Observed, index: i32) -> i32 {
    observed
        .prayers
        .iter()
        .find(|(row, _)| *row == index)
        .map_or(-1, |(_, value)| *value)
}

/// A long fight a cow keeps fighting back in: low offence, Prayer for
/// Protect from Melee, the user's own Thick Skin on before Start.
fn interruptible_cell(label: &str, uid: i32, protect: i32) -> Cell {
    start_cell(
        label,
        uid,
        vec![
            "setstat prayer 43".into(),
            "setstat hitpoints 20".into(),
            "setvar prayer0 1".into(),
        ],
        vec![protect, THICK_SKIN_VARP],
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 5 && stat.base >= 43)
                && snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.index == 3 && stat.base >= 20)
                && varp(snapshot, THICK_SKIN_VARP) == 1
        },
    )
}

fn protect_from_melee() -> i32 {
    api::game_data::for_revision(api::selected::ClientRevision::R289)
        .unwrap()
        .prayer_by_name("Protect from Melee")
        .expect("Protect from Melee")
        .varp
}

/// Operator Pause mid-fight, once the bot's own protection is on: the fight
/// is cancelled, and after Resume the bot clears only its raise and the
/// session settles `interrupted` / `pause`.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_pause_mid_fight_settles_interrupted_and_clears_only_the_bot_prayer() {
    let protect = protect_from_melee();
    let mut cell = interruptible_cell("pause", 274_279_216, protect);
    let (outcome, phases, evidence, at, after) = cell.fight_and_interrupt(
        &format!("{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, budgetTicks: 600 }}"),
        |state, _, _| state.seen_on.contains(&protect) && latest_varp(state, protect) == 1,
        |play, account, _| {
            play.script_pause(account);
            let paused = play.script_state(account);
            // Four game ticks paused: a Pause settles only after Resume.
            thread::sleep(Duration::from_millis(2_400));
            let while_paused = probe(play, account, "globalThis.__outcome || null");
            play.script_resume(account);
            serde_json::json!({
                "state_after_pause": format!("{paused:?}"),
                "outcome_while_paused": while_paused,
            })
        },
    );
    cell.receipt(
        "api-combat-pause-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_pause_mid_fight_settles_interrupted_and_clears_only_the_bot_prayer",
            "account": cell.account,
            "protect_from_melee_varp": protect,
            "thick_skin_varp": THICK_SKIN_VARP,
            "outcome": outcome,
            "phases": phases,
            "pause": evidence,
            "prayers_at_pause": at.prayers,
            "after_prayers": after.prayers,
        }),
    );
    assert_eq!(evidence["state_after_pause"], "Paused", "{evidence}");
    assert!(evidence["outcome_while_paused"].is_null(), "{evidence}");
    assert_eq!(
        latest_varp(&at, protect),
        1,
        "the bot's raise was on at Pause"
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "interrupted", "{outcome}");
    assert_eq!(outcome["value"]["cause"], "pause", "{outcome}");
    assert_eq!(
        latest_varp(&after, protect),
        0,
        "the bot's raise is cleared"
    );
    assert_eq!(
        latest_varp(&after, THICK_SKIN_VARP),
        1,
        "the user's prayer stays on"
    );
}

/// Manual movement mid-fight with no walk in flight, with “Pause script on
/// manual movement” off so nothing but the movement can end the fight: one
/// TUI Manual step (the real host intent path) cancels it, the bot clears
/// only its raise and the session settles `interrupted` / `user-input`.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_manual_movement_mid_fight_settles_interrupted_user_input() {
    let protect = protect_from_melee();
    let mut cell = interruptible_cell("manual", 274_279_217, protect);
    cell.play.set_pause_script_on_manual_walk_abort(false);
    let walking = |play: &Play, account: &str| {
        let slot = script_slot(&play.scripts, account)
            .expect("live script slot")
            .lock()
            .expect("live script slot lock")
            .live_walking_operation();
        let host = play.navs.lock().unwrap().get(account).is_some_and(|bot| {
            bot.route.is_some()
                || bot.pending_route.is_some()
                || bot.native_walk.as_ref().is_some_and(|owner| owner.live())
        });
        (slot, host)
    };
    let intent = |play: &Play, account: &str| {
        play.navs
            .lock()
            .unwrap()
            .get(account)
            .map_or(0, |bot| bot.user_move_intent_seq)
    };
    let (outcome, phases, evidence, at, after) = cell.fight_and_interrupt(
        &format!("{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, budgetTicks: 600 }}"),
        |state, play, account| {
            state.seen_on.contains(&protect)
                && latest_varp(state, protect) == 1
                && walking(play, account) == (false, false)
        },
        |play, account, at| {
            let (x, z, level) = at.tile.expect("tile at the manual step");
            let before = intent(play, account);
            play.queue_wire(account, WireCmd::Walk { x: x + 1, z, level });
            let deadline = Instant::now() + Duration::from_secs(10);
            wait_until("manual intent", deadline, || intent(play, account) > before);
            serde_json::json!({
                "step_to": [x + 1, z, level],
                "walking_at_step": { "slot": false, "host": false },
                "intent_seq": { "before": before, "after": intent(play, account) },
                "state_after_step": format!("{:?}", play.script_state(account)),
            })
        },
    );
    let state = cell.play.script_state(&cell.account);
    cell.receipt(
        "api-combat-manual-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_manual_movement_mid_fight_settles_interrupted_user_input",
            "account": cell.account,
            "protect_from_melee_varp": protect,
            "thick_skin_varp": THICK_SKIN_VARP,
            "outcome": outcome,
            "phases": phases,
            "manual": evidence,
            "prayers_at_step": at.prayers,
            "after_prayers": after.prayers,
            "script_state_after": format!("{state:?}"),
        }),
    );
    assert_eq!(
        latest_varp(&at, protect),
        1,
        "the bot's raise was on at the step"
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "interrupted", "{outcome}");
    assert_eq!(outcome["value"]["cause"], "user-input", "{outcome}");
    assert_eq!(
        state,
        script::RunState::Running,
        "with the preference off the script is not paused"
    );
    assert_eq!(
        latest_varp(&after, protect),
        0,
        "the bot's raise is cleared"
    );
    assert_eq!(
        latest_varp(&after, THICK_SKIN_VARP),
        1,
        "the user's prayer stays on"
    );
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_melee_kills_a_lumbridge_cow_for_xp() {
    let mut cell = start_cell(
        "melee",
        274_279_211,
        ["attack", "strength", "defence"]
            .into_iter()
            .map(|skill| format!("setstat {skill} 20"))
            .chain(["setstat hitpoints 20".to_owned()])
            .collect(),
        Vec::new(),
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 2 && stat.base >= 20)
        },
    );
    let (outcome, phases, before, after) = cell.fight(&format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, meleeMode: 'aggressive', budgetTicks: 300 }}"
    ));
    let strength = xp(&after, 2) - xp(&before, 2);
    let hitpoints = xp(&after, 3) - xp(&before, 3);
    cell.receipt(
        "api-combat-melee-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_melee_kills_a_lumbridge_cow_for_xp",
            "account": cell.account,
            "outcome": outcome,
            "phases": phases,
            "xp_gained": { "strength": strength, "hitpoints": hitpoints },
            "before": before,
            "after": after,
        }),
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert_eq!(outcome["value"]["report"]["end"], "killed", "{outcome}");
    assert!(
        strength > 0,
        "aggressive melee must gain Strength XP: {strength}"
    );
    assert!(
        phases
            .as_array()
            .is_some_and(|phases| phases.iter().any(|phase| phase == "running:fighting")),
        "{phases}"
    );
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_raises_and_clears_its_prayer_keeps_the_users_and_eats() {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let protect = data
        .prayer_by_name("Protect from Melee")
        .expect("Protect from Melee")
        .varp;
    let mut cell = start_cell(
        "prayer",
        274_279_212,
        vec![
            "setstat attack 20".into(),
            "setstat strength 20".into(),
            "setstat defence 20".into(),
            "setstat prayer 43".into(),
            "setstat hitpoints 10".into(),
            "give shrimp 5".into(),
            // The user's own prayer, on before the session starts.
            "setvar prayer0 1".into(),
            // Low HP so the shared policy's emergency line makes the bot eat
            // as soon as the cow fights back.
            "~stat_drain hitpoints 8 0".into(),
        ],
        vec![protect, THICK_SKIN_VARP],
        |snapshot| {
            let hitpoints = snapshot.stats().iter().find(|stat| stat.index == 3);
            hitpoints.is_some_and(|stat| stat.base == 10 && stat.effective <= 3)
                && snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.index == 5 && stat.base >= 43)
                && snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == SHRIMP_ID)
                && varp(snapshot, THICK_SKIN_VARP) == 1
        },
    );
    {
        let state = cell.observed.lock().unwrap();
        assert_eq!(
            state
                .prayers
                .iter()
                .find(|(index, _)| *index == THICK_SKIN_VARP),
            Some(&(THICK_SKIN_VARP, 1)),
            "the user's Thick Skin is on before Start"
        );
    }
    let (outcome, phases, before, after) = cell.fight(&format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, prayer: true, food: true, budgetTicks: 400 }}"
    ));
    let latest = |index: i32| {
        after
            .prayers
            .iter()
            .find(|(row, _)| *row == index)
            .map_or(-1, |(_, value)| *value)
    };
    cell.receipt(
        "api-combat-prayer-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_raises_and_clears_its_prayer_keeps_the_users_and_eats",
            "account": cell.account,
            "protect_from_melee_varp": protect,
            "thick_skin_varp": THICK_SKIN_VARP,
            "outcome": outcome,
            "phases": phases,
            "seen_on_during_fight": after.seen_on,
            "after_prayers": after.prayers,
            "shrimps": { "before": before.shrimps, "after": after.shrimps },
            "hitpoints": { "before": before.hitpoints, "after": after.hitpoints },
        }),
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert!(
        after.seen_on.contains(&protect),
        "the bot raised Protect from Melee during the fight"
    );
    assert_eq!(latest(protect), 0, "the bot's prayer is cleared at the end");
    assert_eq!(latest(THICK_SKIN_VARP), 1, "the user's prayer stays on");
    assert!(
        outcome["value"]["report"]["food"].as_u64().unwrap_or(0) >= 1
            && after.shrimps < before.shrimps,
        "the bot ate: {outcome}"
    );
}

/// Test-only wrapper around the unmodified showcase: `api.log` lines are also
/// kept in the isolate, and the closing `api.stop` is recorded instead of
/// tearing the isolate down, so the cell can read the log back.
const SHOWCASE_HARNESS: &str = r#"
export function tick(api) {
  const wrapped = new Proxy(api, {
    get(target, key) {
      if (key === 'log') {
        return (message) => {
          (globalThis.__showcaseLog ||= []).push(String(message));
          target.log(message);
        };
      }
      if (key === 'stop') {
        return (reason) => { globalThis.__showcaseStop ??= String(reason); };
      }
      const value = target[key];
      return typeof value === 'function' ? value.bind(target) : value;
    },
  });
  if (globalThis.__showcaseStop == null) {
    __showcaseTick(wrapped);
  }
}
"#;

/// One number after `label` in a showcase line (`ate 1`, `prayer doses 1`).
fn count_after(line: &str, label: &str) -> i64 {
    line.split(label)
        .nth(1)
        .and_then(|rest| {
            rest.trim_start()
                .split(|c: char| !c.is_ascii_digit())
                .next()
        })
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(-1)
}

/// One number before `label` in a showcase line (`9 casts`).
fn count_before(line: &str, label: &str) -> i64 {
    line.split(label)
        .next()
        .and_then(|head| head.trim_end().rsplit(|c: char| !c.is_ascii_digit()).next())
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(-1)
}

/// Cow eat line: `2 × danger + 1` with danger 1 (`select.rs`, js-api-v2).
const COW_EAT_LINE: i64 = 3;
/// One shrimp heals 3. An eat at the line therefore finishes at 6 or below.
const SHRIMP_HEAL: i64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EatVerdict {
    /// The fight started at or below the eat line, the bot ate, and
    /// hitpoints finished above the line but not above one shrimp.
    AteAtLine,
    /// The fight started above the eat line and the bot did not eat.
    NoEatAboveLine,
}

/// Hitpoints in a showcase line (`HP 3/40` → 3). `-1` when the line has none.
fn hp_of(line: &str) -> i64 {
    line.split("HP ")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .and_then(|digits| digits.trim().parse().ok())
        .unwrap_or(-1)
}

/// The documented eat rule for one showcase phase.
///
/// `eat_line` is `2 × danger + 1`. A receipt passes when the fight started
/// at or below that line, the bot ate at least once, and hitpoints finished
/// above the line and at or below one shrimp over it. It fails when the bot
/// never ate at or below the line, when the end line has no `ate` count,
/// when hitpoints did not finish above the line, when they finished above
/// one shrimp, or when the bot ate while already above the line.
fn showcase_eat_verdict(
    start_line: &str,
    end_line: &str,
    eat_line: i64,
) -> Result<EatVerdict, &'static str> {
    let start_hp = hp_of(start_line);
    let end_hp = hp_of(end_line);
    let ate = count_after(end_line, "ate");
    if start_hp < 0 || end_hp < 0 || ate < 0 {
        return Err("eat receipt is missing ate or hitpoints");
    }
    if start_hp > eat_line {
        return if ate == 0 {
            Ok(EatVerdict::NoEatAboveLine)
        } else {
            Err("ate while above the eat line")
        };
    }
    if ate < 1 {
        return Err("never ate at or below the eat line");
    }
    if end_hp <= eat_line {
        return Err("end hitpoints are not above the eat line");
    }
    if end_hp > eat_line + SHRIMP_HEAL {
        return Err("end hitpoints are above one shrimp");
    }
    Ok(EatVerdict::AteAtLine)
}

/// The external-path showcase (`examples/combat_showcase_v2.ts`) end to end,
/// seeded exactly as its header says.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_showcase_script_runs_every_phase() {
    let transpiled = script_js::transpile_ts(include_str!(
        "../../script-js/examples/combat_showcase_v2.ts"
    ))
    .expect("transpile the showcase");
    assert!(transpiled.contains("export function tick("));
    let source = transpiled.replacen("export function tick(", "function __showcaseTick(", 1)
        + SHOWCASE_HARNESS;
    let cell = start_cell(
        "showcase",
        274_279_213,
        [
            "setstat attack 40",
            "setstat strength 40",
            "setstat defence 40",
            "setstat hitpoints 40",
            "setstat ranged 30",
            "setstat magic 30",
            "setstat prayer 43",
            "give bronze_scimitar 1",
            "give shortbow 1",
            "give bronze_arrow 300",
            "give airrune 300",
            "give mindrune 300",
            "give shrimp 20",
            "give 3doseprayerrestore 2",
            "setvar prayer0 1",
            "~stat_drain hitpoints 37 0",
            "~stat_drain prayer 17 0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        vec![THICK_SKIN_VARP],
        |snapshot| {
            let stat = |index: i32| snapshot.stats().iter().find(|stat| stat.index == index);
            stat(3).is_some_and(|stat| stat.base == 40 && stat.effective == 3)
                && stat(5).is_some_and(|stat| stat.base == 43 && stat.effective <= 26)
                && snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == SHRIMP_ID)
                && varp(snapshot, THICK_SKIN_VARP) == 1
        },
    );
    cell.play
        .script_start_load(
            &cell.account,
            source,
            script_js::LoadShape::NativeTick,
            None,
            vec![],
        )
        .expect("start the showcase");
    let deadline = Instant::now() + Duration::from_secs(600);
    let finished = loop {
        let stop = probe(
            &cell.play,
            &cell.account,
            "globalThis.__showcaseStop ?? null",
        );
        if !stop.is_null() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        thread::sleep(Duration::from_millis(200));
    };
    let stop = probe(
        &cell.play,
        &cell.account,
        "globalThis.__showcaseStop ?? null",
    );
    let lines: Vec<String> = serde_json::from_value(probe(
        &cell.play,
        &cell.account,
        "globalThis.__showcaseLog ?? []",
    ))
    .expect("showcase log lines");
    cell.receipt(
        "api-combat-showcase-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_showcase_script_runs_every_phase",
            "account": cell.account,
            "script": "crates/script-js/examples/combat_showcase_v2.ts",
            "finished_within_deadline": finished,
            "stop_reason": stop,
            "log": lines,
        }),
    );
    assert!(finished, "showcase did not finish within 600 s: {lines:#?}");
    assert_eq!(stop, "combat showcase complete", "{lines:#?}");
    let end = |phase: &str| {
        lines
            .iter()
            .rfind(|line| line.contains(&format!("[showcase] {phase} end: ")))
            .cloned()
            .unwrap_or_else(|| panic!("no {phase} end line: {lines:#?}"))
    };
    let eat_start = lines
        .iter()
        .rfind(|line| line.contains("[showcase] eat start:"))
        .cloned()
        .unwrap_or_else(|| panic!("no eat start line: {lines:#?}"));
    let eat = end("eat");
    assert_eq!(
        showcase_eat_verdict(&eat_start, &eat, COW_EAT_LINE),
        Ok(EatVerdict::AteAtLine),
        "start={eat_start} end={eat}"
    );
    let prayer = end("prayer");
    assert!(count_after(&prayer, "protect switches") >= 1, "{prayer}");
    assert!(count_after(&prayer, "prayer doses") >= 1, "{prayer}");
    assert!(
        prayer.contains("Thick Skin on, Protect from Melee off"),
        "{prayer}"
    );
    for phase in ["aggressive", "defensive", "ranged"] {
        let line = end(phase);
        assert!(line.contains("fought → killed"), "{line}");
    }
    let magic = end("magic");
    assert!(count_before(&magic, "casts") >= 1, "{magic}");
    let stop = end("stop");
    assert!(stop.contains("stop end: stopped"), "{stop}");
    let done = lines
        .iter()
        .find(|line| line.contains("[showcase] done;"))
        .expect("showcase done line");
    assert!(
        done.contains("Thick Skin on, Protect from Melee off"),
        "{done}"
    );
}

/// Ranged on its own: a fresh account carrying a bow and arrows. Combat
/// wields them in Prep and must land hits on a real cow.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_ranged_wields_and_kills_a_cow() {
    let bow = std::env::var("COMBAT_LIVE_BOW").unwrap_or_else(|_| "shortbow".into());
    let ammo = std::env::var("COMBAT_LIVE_AMMO").unwrap_or_else(|_| "bronze_arrow".into());
    let mut cell = start_cell(
        "ranged",
        274_279_214,
        vec![
            "setstat ranged 30".into(),
            "setstat defence 20".into(),
            "setstat hitpoints 20".into(),
            format!("give {bow} 1"),
            format!("give {ammo} 200"),
        ],
        Vec::new(),
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 4 && stat.base >= 30)
                && snapshot.inventory().len() >= 2
        },
    );
    let (outcome, phases, before, after) = cell.fight(&format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, radius: 30, style: 'ranged', rangedMode: 'rapid', budgetTicks: 300 }}"
    ));
    let ranged = xp(&after, 4) - xp(&before, 4);
    cell.receipt(
        "api-combat-ranged-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_ranged_wields_and_kills_a_cow",
            "account": cell.account,
            "bow": bow,
            "ammo": ammo,
            "outcome": outcome,
            "phases": phases,
            "xp_gained": { "ranged": ranged },
            "after_side_tabs": after.side_tabs,
            "after_equipment": after.equipment,
        }),
    );
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert_eq!(outcome["value"]["report"]["end"], "killed", "{outcome}");
    assert!(ranged > 0, "ranged XP: {ranged}");
}

/// The tutorial guard can leave a worn bow bound to the unarmed root. Prep
/// must explain that readiness failure instead of spending the full budget.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_ranged_unready_tutorial_aborts_with_readiness_reason() {
    let bow = std::env::var("COMBAT_LIVE_BOW").unwrap_or_else(|_| "shortbow".into());
    let ammo = std::env::var("COMBAT_LIVE_AMMO").unwrap_or_else(|_| "bronze_arrow".into());
    let mut cell = start_cell(
        "ranged-unready",
        274_279_218,
        vec![
            "setstat ranged 30".into(),
            "setstat defence 20".into(),
            "setstat hitpoints 20".into(),
            format!("give {bow} 1"),
            format!("give {ammo} 200"),
            // Leave the final session below the content's weapon-tab update
            // threshold, reproducing the intentionally unready fixture.
            "setvar tutorial 1".into(),
            "getvar tutorial".into(),
        ],
        Vec::new(),
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 4 && stat.base >= 30)
                && snapshot.inventory().len() >= 2
                && snapshot
                    .chat_lines()
                    .iter()
                    .any(|line| line.text.trim().eq_ignore_ascii_case("get tutorial: 1"))
        },
    );
    let (outcome, phases, before, after) = cell.fight(&format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, radius: 30, style: 'ranged', rangedMode: 'rapid', budgetTicks: 300 }}"
    ));
    let report = &outcome["value"]["report"];
    let reason = report["reason"].as_str().unwrap_or("").to_owned();
    let ticks = report["ticks"].as_u64().unwrap_or(u64::MAX);
    cell.receipt(
        "api-combat-ranged-readiness-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_ranged_unready_tutorial_aborts_with_readiness_reason",
            "account": cell.account,
            "tutorial_fixture_value": 1,
            "bow": bow,
            "ammo": ammo,
            "outcome": outcome.clone(),
            "phases": phases,
            "observed_side_tabs": after.side_tabs,
            "after_equipment": after.equipment,
            "before": before,
            "after": after,
        }),
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert_eq!(report["end"], "aborted", "{outcome}");
    assert!(
        matches!(
            reason.as_str(),
            "prep-readiness:combat-root-wrong" | "prep-readiness:combat-root-missing"
        ),
        "tutorial-1 readiness result: {outcome}"
    );
    assert!(
        ticks <= 16,
        "readiness should fail near eight observed ticks, not the 300-tick budget: {outcome}"
    );
}

/// Magic on its own: one manual spell from carried runes.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_magic_casts_wind_strike_on_a_cow() {
    let mut cell = start_cell(
        "magic",
        274_279_215,
        vec![
            "setstat magic 30".into(),
            "setstat defence 20".into(),
            "setstat hitpoints 20".into(),
            "give airrune 200".into(),
            "give mindrune 200".into(),
        ],
        Vec::new(),
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 6 && stat.base >= 30)
                && snapshot.inventory().len() >= 2
        },
    );
    let (outcome, phases, before, after) = cell.fight(&format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, radius: 30, style: 'magic', spells: ['wind_strike'], budgetTicks: 300 }}"
    ));
    let magic = xp(&after, 6) - xp(&before, 6);
    cell.receipt(
        "api-combat-magic-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_magic_casts_wind_strike_on_a_cow",
            "account": cell.account,
            "outcome": outcome,
            "phases": phases,
            "xp_gained": { "magic": magic },
        }),
    );
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert!(
        outcome["value"]["report"]["casts"].as_u64().unwrap_or(0) >= 1,
        "{outcome}"
    );
    assert!(magic > 0, "magic XP: {magic}");
}

#[test]
fn showcase_line_counts_read_the_logged_numbers() {
    let line = "[showcase] magic end: fought → killed: 44 ticks, 0 swings, 9 casts, \
                0 damage taken, ate 2, prayer doses 1, protect switches 3; HP 24/40";
    assert_eq!(count_before(line, "casts"), 9);
    assert_eq!(count_after(line, "ate"), 2);
    assert_eq!(count_after(line, "prayer doses"), 1);
    assert_eq!(count_after(line, "protect switches"), 3);
    let none = "[showcase] magic end: fought → budget: 400 ticks, 0 swings, 0 casts, \
                0 damage taken, ate 0, prayer doses 0, protect switches 0; HP 24/40";
    assert_eq!(count_before(none, "casts"), 0);
    assert_eq!(count_after(none, "ate"), 0);
    assert_eq!(count_before("[showcase] stop end: stopped", "casts"), -1);
    assert_eq!(count_after("[showcase] stop end: stopped", "ate"), -1);
}

fn eat_lines(start_hp: i64, end_hp: i64, ate: i64) -> (String, String) {
    (
        format!("[showcase] eat start: HP {start_hp}/40; Thick Skin on"),
        format!(
            "[showcase] eat end: fought → killed: 12 ticks, 4 swings, 0 casts, \
             0 damage taken, ate {ate}, prayer doses 0, protect switches 0; HP {end_hp}/40"
        ),
    )
}

#[test]
fn showcase_eat_oracle_accepts_an_eat_at_the_line() {
    let (start, end) = eat_lines(COW_EAT_LINE, 6, 1);
    assert_eq!(
        showcase_eat_verdict(&start, &end, COW_EAT_LINE),
        Ok(EatVerdict::AteAtLine)
    );
}

#[test]
fn showcase_eat_oracle_rejects_a_receipt_that_never_eats_below_the_line() {
    let (start, end) = eat_lines(2, 2, 0);
    assert_eq!(
        showcase_eat_verdict(&start, &end, COW_EAT_LINE),
        Err("never ate at or below the eat line")
    );
    let (at_line, still) = eat_lines(COW_EAT_LINE, COW_EAT_LINE, 0);
    assert_eq!(
        showcase_eat_verdict(&at_line, &still, COW_EAT_LINE),
        Err("never ate at or below the eat line")
    );
}

#[test]
fn showcase_eat_oracle_rejects_an_eat_above_the_line_and_a_missing_count() {
    let (start, end) = eat_lines(4, 7, 1);
    assert_eq!(
        showcase_eat_verdict(&start, &end, COW_EAT_LINE),
        Err("ate while above the eat line")
    );
    let (above, stayed) = eat_lines(4, 4, 0);
    assert_eq!(
        showcase_eat_verdict(&above, &stayed, COW_EAT_LINE),
        Ok(EatVerdict::NoEatAboveLine)
    );
    let (low, unfinished) = eat_lines(COW_EAT_LINE, COW_EAT_LINE, 1);
    assert_eq!(
        showcase_eat_verdict(&low, &unfinished, COW_EAT_LINE),
        Err("end hitpoints are not above the eat line")
    );
    let missing = "[showcase] eat end: fought → killed: 12 ticks; HP 6/40";
    assert_eq!(
        showcase_eat_verdict(&start, missing, COW_EAT_LINE),
        Err("eat receipt is missing ate or hitpoints")
    );
    // The old half-max line ate six shrimps from 3 HP and finished at 21.
    let (low, over_ate) = eat_lines(COW_EAT_LINE, 21, 6);
    assert_eq!(
        showcase_eat_verdict(&low, &over_ate, COW_EAT_LINE),
        Err("end hitpoints are above one shrimp")
    );
}

/// Cow emergency line: danger 1 + 1. The old unknown-danger line for 40 HP is 20.
const COW_EMERGENCY: i32 = 2;
/// Classic varp 172: 0 is auto-retaliate on, 1 is off.
const AUTO_RETALIATE_VARP: i32 = 172;
/// The old half-max line `lines(None, 40)` used before these stats were known.
const OLD_UNKNOWN_LINE: i32 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeCase {
    /// Chicken, max hit 0. Food and Protect from Melee stay unused. The fight gains XP.
    ZeroDanger,
    /// Cow, no food, 3/40. The old line was 20, so that rule aborts at the start.
    NoFoodOpen,
    /// Cow, no food, 2/40. The fight aborts for no food before it attacks.
    NoFoodStop,
}

/// Facts the live cell reads off one fight. Unit tests fill the old-rule cases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NativeReceipt {
    start_hp: i32,
    food_before: i32,
    food_after: i32,
    protect_raised: bool,
    xp_gained: bool,
    killed: bool,
    swings: u32,
    aborted_no_food: bool,
}

/// The three changed decisions. A correct receipt passes. Each old-rule receipt fails.
fn native_line_verdict(case: NativeCase, receipt: &NativeReceipt) -> Result<(), &'static str> {
    match case {
        NativeCase::ZeroDanger => {
            if receipt.start_hp > COW_EAT_LINE as i32 {
                return Err("zero-danger seed is above 3 hitpoints");
            }
            if receipt.food_before < 1 {
                return Err("zero-danger seed has no food");
            }
            if receipt.food_after < receipt.food_before {
                return Err("ate during a max-hit-0 fight");
            }
            if receipt.protect_raised {
                return Err("raised Protect from Melee during a max-hit-0 fight");
            }
            if receipt.aborted_no_food {
                return Err("aborted a max-hit-0 fight for lack of food");
            }
            if !receipt.xp_gained {
                return Err("max-hit-0 fight did not gain XP");
            }
            Ok(())
        }
        NativeCase::NoFoodOpen => {
            if receipt.start_hp != COW_EAT_LINE as i32 {
                return Err("open-line seed is not 3 hitpoints");
            }
            if receipt.food_before != 0 || receipt.food_after != 0 {
                return Err("open-line seed is not empty of food");
            }
            if receipt.aborted_no_food && receipt.swings == 0 {
                return Err("aborted for lack of food at the start");
            }
            if !receipt.xp_gained && !receipt.killed {
                return Err("open-line fight gave neither XP nor a kill");
            }
            Ok(())
        }
        NativeCase::NoFoodStop => {
            if receipt.start_hp != COW_EMERGENCY {
                return Err("stop seed is not the cow emergency line");
            }
            if receipt.food_before != 0 {
                return Err("stop seed is not empty of food");
            }
            if !receipt.aborted_no_food {
                return Err("did not stop for lack of food");
            }
            if receipt.swings != 0 {
                return Err("attacked before the no-food stop");
            }
            if receipt.xp_gained || receipt.killed {
                return Err("the stopped fight still gained XP or a kill");
            }
            Ok(())
        }
    }
}

/// The product's no-food predicate. `None` is the old unknown danger.
fn stops_for_no_food(danger: Option<i32>, hp: i32, hp_max: i32, food: bool) -> bool {
    let emergency = script::combat::select::lines(danger, hp_max).emergency;
    danger != Some(0) && hp <= emergency && !food
}

/// One Load script, several fights. The test arms `__request` and reads `__outcome`.
const PHASED_CONSUMER: &str = r#"export const apiVersion = 2;
export function tick(api) {
  const page = api.snapshot.combat;
  const label = page ? `${page.phase}:${page.status ? page.status.stage : '-'}` : 'idle';
  if (label !== globalThis.__last) {
    globalThis.__last = label;
    (globalThis.__phases ||= []).push(label);
  }
  if (globalThis.__stopNow && globalThis.__fight && !globalThis.__stopSent) {
    globalThis.__stopSent = true;
    api.combat.stop();
  }
  if (!globalThis.__fight && globalThis.__request) {
    const request = globalThis.__request;
    globalThis.__request = null;
    globalThis.__fight = api.combat.fight(request);
    globalThis.__fight.then((value) => {
      globalThis.__outcome = value;
      globalThis.__fight = null;
    });
  }
}
"#;

fn combat_xp(before: &Observed, after: &Observed) -> i32 {
    [0, 1, 2, 3]
        .into_iter()
        .map(|index| xp(after, index) - xp(before, index))
        .sum()
}

fn report_flag(outcome: &serde_json::Value, field: &str) -> u64 {
    outcome["value"]["report"][field].as_u64().unwrap_or(0)
}

fn aborted_no_food(outcome: &serde_json::Value) -> bool {
    outcome["kind"] == "done"
        && outcome["value"]["end"] == "fought"
        && outcome["value"]["report"]["end"] == "aborted"
        && outcome["value"]["report"]["reason"] == "no-food"
}

fn killed_report(outcome: &serde_json::Value) -> bool {
    outcome["kind"] == "done"
        && outcome["value"]["end"] == "fought"
        && outcome["value"]["report"]["end"] == "killed"
}

/// Chickens at 3/40 with food, then a cow at 3/40 with none, then a cow at
/// 2/40 with none. Real NPCs. Only this account's stats, food and position
/// are seeded.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_native_lines_skip_zero_danger_and_hold_the_cow_food_line() {
    let protect = protect_from_melee();
    let cell = start_cell(
        "native-lines",
        274_279_219,
        vec![
            "setstat attack 40".into(),
            "setstat strength 40".into(),
            "setstat defence 40".into(),
            "setstat prayer 43".into(),
            "setstat hitpoints 40".into(),
            "give shrimp 5".into(),
            "~stat_drain hitpoints 37 0".into(),
        ],
        vec![protect],
        |snapshot| {
            let hitpoints = snapshot.stats().iter().find(|stat| stat.index == 3);
            hitpoints.is_some_and(|stat| stat.base == 40 && stat.effective == 3)
                && snapshot
                    .stats()
                    .iter()
                    .any(|stat| stat.index == 5 && stat.base >= 43)
                && snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == SHRIMP_ID)
        },
    );
    cell.play
        .script_start_load(
            &cell.account,
            PHASED_CONSUMER.to_string(),
            script::LoadShape::NativeTick,
            None,
            vec![],
        )
        .expect("start the phased combat consumer");
    let deadline = Instant::now() + FIGHT_DEADLINE;
    wait_until("phased consumer running", deadline, || {
        cell.play.script_state(&cell.account) == script::RunState::Running
    });

    let mut zero = None;
    for (i, (x, z)) in CHICKEN_STANDS.iter().enumerate() {
        let stand = WorldTile {
            x: *x,
            z: *z,
            level: 0,
        };
        cell.cheat_until(
            "tele inside the chicken pen",
            &api::interact::tele_args(stand.level, stand.x, stand.z),
            Cell::near(stand),
        );
        let proof = cell.phase(
            "zero-danger",
            &format!(
                "{{ target: {{ npc: 'Chicken' }}, area: {CHICKEN_AREA}, meleeMode: 'aggressive', prayer: true, food: true, budgetTicks: 80 }}"
            ),
            protect,
        );
        let hit = proof.facts.xp_gained || proof.facts.killed;
        zero = Some(proof);
        if hit || i + 1 == CHICKEN_STANDS.len() {
            break;
        }
    }
    let zero = zero.expect("a chicken stand");

    cell.cheat_until("clear food", "~clearinv", |state| state.shrimps == 0);
    cell.seed_hitpoints(3, |_| true);
    cell.cheat_until(
        "tele to the cow field",
        &api::interact::tele_args(COW_FIELD.level, COW_FIELD.x, COW_FIELD.z),
        Cell::near(COW_FIELD),
    );
    let open = cell.phase(
        "no-food-open",
        &format!(
            "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, meleeMode: 'aggressive', prayer: true, food: true, budgetTicks: 300 }}"
        ),
        protect,
    );

    // The measured stop must see the cow as a threat before its own attack.
    // A short fight makes a real cow retaliate. Stop that fight, then start
    // the measured one while the cow is still on us.
    cell.seed_hitpoints(40, |_| true);
    let aggro_request = format!(
        "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, meleeMode: 'aggressive', food: true, budgetTicks: 80 }}"
    );
    cell.arm(&aggro_request);
    let aggro_deadline = Instant::now() + Duration::from_secs(90);
    wait_until("cow attacks us", aggro_deadline, || {
        let settled = probe(&cell.play, &cell.account, "globalThis.__outcome || null");
        assert!(
            settled.is_null(),
            "the aggro fight settled before a cow attacked: {settled}"
        );
        cell.snapshot().npc_on_us
    });
    probe(
        &cell.play,
        &cell.account,
        "globalThis.__stopNow = true; true",
    );
    let stop_deadline = Instant::now() + Duration::from_secs(30);
    wait_until("aggro fight stopped", stop_deadline, || {
        !probe(&cell.play, &cell.account, "globalThis.__outcome || null").is_null()
    });
    // A new machine counts a playing attack animation as its own swing, and
    // auto-retaliate starts another in the gap. Turn retaliate off while the
    // cow is still on us, wait for that swing to end, then drain and arm.
    cell.observed
        .lock()
        .expect("observed lock")
        .want_retaliate_off = true;
    let attack_anims: Vec<i32> = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .unwrap()
        .style_seqs()
        .iter()
        .map(|row| row.seq_id)
        .collect();
    let quiet = Instant::now() + Duration::from_secs(12);
    wait_until("retaliate off and the setup swing ended", quiet, || {
        let state = cell.snapshot();
        state.npc_on_us && state.retaliate == 1 && !attack_anims.contains(&state.local_animation)
    });
    let hp = cell.snapshot().hitpoints;
    assert!(hp >= 2, "hitpoints {hp} are already below the stop line");
    if hp != 2 {
        cell.cheat_until(
            "drain to the cow emergency line",
            &format!("~stat_drain hitpoints {} 0", hp - 2),
            |state| state.hitpoints_base == 40 && state.hitpoints == 2 && state.npc_on_us,
        );
    }
    let quiet_again = Instant::now() + Duration::from_secs(8);
    wait_until("stop seed is quiet at 2 hitpoints", quiet_again, || {
        let state = cell.snapshot();
        state.hitpoints == 2
            && state.hitpoints_base == 40
            && state.npc_on_us
            && state.retaliate == 1
            && !attack_anims.contains(&state.local_animation)
    });
    let stop = cell.phase(
        "no-food-stop",
        &format!(
            "{{ target: {{ npc: 'Cow' }}, area: {COW_AREA}, prayer: true, food: true, retaliate: false, budgetTicks: 40 }}"
        ),
        protect,
    );

    cell.receipt(
        "api-combat-native-lines-receipt.json",
        serde_json::json!({
            "cell": "live_api_combat_native_lines_skip_zero_danger_and_hold_the_cow_food_line",
            "account": cell.account,
            "protect_from_melee_varp": protect,
            "zero_danger": zero.receipt,
            "no_food_open": open.receipt,
            "no_food_stop": stop.receipt,
        }),
    );
    assert_eq!(
        native_line_verdict(NativeCase::ZeroDanger, &zero.facts),
        Ok(()),
        "{:?}",
        zero.facts
    );
    assert_eq!(
        native_line_verdict(NativeCase::NoFoodOpen, &open.facts),
        Ok(()),
        "{:?}",
        open.facts
    );
    assert_eq!(
        native_line_verdict(NativeCase::NoFoodStop, &stop.facts),
        Ok(()),
        "{:?}",
        stop.facts
    );
}

struct PhaseProof {
    facts: NativeReceipt,
    receipt: serde_json::Value,
}

impl Cell {
    fn arm(&self, request: &str) {
        let expr = format!(
            "globalThis.__outcome = null; globalThis.__phases = []; globalThis.__last = null; \
             globalThis.__stopNow = false; globalThis.__stopSent = false; \
             globalThis.__fight = null; globalThis.__request = {request}; true"
        );
        let armed = probe(&self.play, &self.account, &expr);
        assert_eq!(armed, true, "arm probe failed: {armed}");
    }

    fn phase(&self, label: &str, request: &str, protect: i32) -> PhaseProof {
        let before = self.snapshot();
        self.arm(request);
        let deadline = Instant::now() + FIGHT_DEADLINE;
        wait_until(&format!("{label} outcome"), deadline, || {
            !probe(&self.play, &self.account, "globalThis.__outcome || null").is_null()
        });
        let settle = Instant::now() + Duration::from_secs(2);
        wait_until(&format!("{label} frames"), deadline, || {
            Instant::now() >= settle
        });
        let outcome = probe(&self.play, &self.account, "globalThis.__outcome");
        let phases = probe(&self.play, &self.account, "globalThis.__phases || []");
        let after = self.snapshot();
        let xp_gained = combat_xp(&before, &after) > 0;
        let facts = NativeReceipt {
            start_hp: before.hitpoints,
            food_before: before.shrimps,
            food_after: after.shrimps,
            protect_raised: after.seen_on.contains(&protect) && !before.seen_on.contains(&protect),
            xp_gained,
            killed: killed_report(&outcome),
            swings: u32::try_from(report_flag(&outcome, "swings")).unwrap_or(u32::MAX),
            aborted_no_food: aborted_no_food(&outcome),
        };
        PhaseProof {
            facts,
            receipt: serde_json::json!({
                "label": label,
                "outcome": outcome,
                "phases": phases,
                "start_hp": facts.start_hp,
                "start_base": before.hitpoints_base,
                "food_before": facts.food_before,
                "food_after": facts.food_after,
                "protect_raised": facts.protect_raised,
                "xp_gained": xp_gained,
                "npc_on_us_before": before.npc_on_us,
                "npc_on_us_after": after.npc_on_us,
            }),
        }
    }
}

#[test]
fn native_line_oracle_accepts_the_new_rules() {
    assert_eq!(
        native_line_verdict(
            NativeCase::ZeroDanger,
            &NativeReceipt {
                start_hp: 3,
                food_before: 5,
                food_after: 5,
                protect_raised: false,
                xp_gained: true,
                killed: true,
                swings: 2,
                aborted_no_food: false,
            }
        ),
        Ok(())
    );
    assert_eq!(
        native_line_verdict(
            NativeCase::NoFoodOpen,
            &NativeReceipt {
                start_hp: 3,
                food_before: 0,
                food_after: 0,
                protect_raised: false,
                xp_gained: true,
                killed: false,
                swings: 2,
                aborted_no_food: false,
            }
        ),
        Ok(())
    );
    assert_eq!(
        native_line_verdict(
            NativeCase::NoFoodStop,
            &NativeReceipt {
                start_hp: 2,
                food_before: 0,
                food_after: 0,
                protect_raised: false,
                xp_gained: false,
                killed: false,
                swings: 0,
                aborted_no_food: true,
            }
        ),
        Ok(())
    );
}

#[test]
fn native_line_oracle_rejects_the_old_unknown_danger_rule() {
    assert_eq!(
        script::combat::select::lines(None, 40).emergency,
        OLD_UNKNOWN_LINE
    );
    assert_eq!(
        script::combat::select::lines(Some(1), 40).emergency,
        COW_EMERGENCY
    );
    assert_eq!(script::combat::select::lines(Some(0), 40).eat, 1);
    assert!(stops_for_no_food(None, 3, 40, false));
    assert!(!stops_for_no_food(Some(1), 3, 40, false));
    assert!(stops_for_no_food(Some(1), 2, 40, false));
    assert!(!stops_for_no_food(Some(0), 3, 40, false));
    assert!(!stops_for_no_food(Some(1), OLD_UNKNOWN_LINE, 40, false));

    assert_eq!(
        native_line_verdict(
            NativeCase::ZeroDanger,
            &NativeReceipt {
                start_hp: 3,
                food_before: 5,
                food_after: 4,
                protect_raised: true,
                xp_gained: true,
                killed: false,
                swings: 1,
                aborted_no_food: false,
            }
        ),
        Err("ate during a max-hit-0 fight")
    );
    assert_eq!(
        native_line_verdict(
            NativeCase::ZeroDanger,
            &NativeReceipt {
                start_hp: 3,
                food_before: 5,
                food_after: 5,
                protect_raised: true,
                xp_gained: true,
                killed: false,
                swings: 1,
                aborted_no_food: false,
            }
        ),
        Err("raised Protect from Melee during a max-hit-0 fight")
    );
    assert_eq!(
        native_line_verdict(
            NativeCase::NoFoodOpen,
            &NativeReceipt {
                start_hp: 3,
                food_before: 0,
                food_after: 0,
                protect_raised: false,
                xp_gained: false,
                killed: false,
                swings: 0,
                aborted_no_food: true,
            }
        ),
        Err("aborted for lack of food at the start")
    );
    assert_eq!(
        native_line_verdict(
            NativeCase::NoFoodStop,
            &NativeReceipt {
                start_hp: OLD_UNKNOWN_LINE,
                food_before: 0,
                food_after: 0,
                protect_raised: false,
                xp_gained: false,
                killed: false,
                swings: 0,
                aborted_no_food: true,
            }
        ),
        Err("stop seed is not the cow emergency line")
    );
}
