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
    shrimps: i32,
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
    state.hitpoints = snapshot
        .stats()
        .iter()
        .find(|stat| stat.index == 3)
        .map_or(0, |stat| stat.effective);
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
                script::LoadShape::NativeTick,
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

    fn receipt(&self, name: &str, receipt: serde_json::Value) {
        let path = self.evidence.join(name);
        std::fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        println!(
            "{name} ({:.1}s): {}\n{receipt}",
            self.started.elapsed().as_secs_f64(),
            path.display()
        );
    }
}

fn xp(observed: &Observed, index: i32) -> i32 {
    observed
        .xp
        .iter()
        .find(|(stat, _)| *stat == index)
        .map_or(0, |(_, xp)| *xp)
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

/// The external-path showcase (`examples/combat_showcase_v2.ts`) end to end,
/// seeded exactly as its header says.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_api_combat_showcase_script_runs_every_phase() {
    let transpiled =
        script::transpile_ts(include_str!("../../script/examples/combat_showcase_v2.ts"))
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
            "~stat_drain hitpoints 36 0",
            "~stat_drain prayer 17 0",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect(),
        vec![THICK_SKIN_VARP],
        |snapshot| {
            let stat = |index: i32| snapshot.stats().iter().find(|stat| stat.index == index);
            stat(3).is_some_and(|stat| stat.base == 40 && stat.effective <= 4)
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
            script::LoadShape::NativeTick,
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
            "script": "crates/script/examples/combat_showcase_v2.ts",
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
    let eat = end("eat");
    assert!(count_after(&eat, "ate") >= 1, "{eat}");
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
