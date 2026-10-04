//! Live slice-1 proof through the released Imp Path and compiled Quester Start.
//! Seeds supply only before Start; progress is the real quest hand-in, not setup.

use super::nav_arrival_live_tests::{
    absolute_env_path, live_profile, required_directory, write_capture,
};
use super::*;
use api::snapshot::{GameSnapshot, QuestListStatus, WorldTile};
use host::FrameBuf;
use serde_json::{json, Value};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const ORIGIN: WorldTile = WorldTile {
    x: 2809,
    z: 3441,
    level: 0,
};
const SETUP_LIMIT: Duration = Duration::from_secs(180);
const PROGRESS_LIMIT: Duration = Duration::from_secs(12 * 60);
const CONTROL_LIMIT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Teleport,
    Danger,
}
impl Cell {
    fn from_env() -> Self {
        match std::env::var("WALK_OPTINS_SCENARIO").as_deref() {
            Ok("teleport") => Self::Teleport,
            Ok("danger") => Self::Danger,
            other => panic!("WALK_OPTINS_SCENARIO must be teleport or danger: {other:?}"),
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Teleport => "teleport",
            Self::Danger => "danger",
        }
    }
}

#[derive(Default)]
struct LiveState {
    snapshot: GameSnapshot,
    primed: Option<Instant>,
    logout_sent: bool,
    saw_offline: bool,
    seed_sent: bool,
    ready: bool,
    started: bool,
    previous_tile: Option<WorldTile>,
    teleport_at: Option<WorldTile>,
    ingame: bool,
    scene_state: i32,
}
impl LiveState {
    fn tile(&self) -> Option<WorldTile> {
        self.snapshot
            .tile()
            .map(|(x, z, level)| WorldTile { x, z, level })
    }
    fn count(&self, name: &str) -> i32 {
        self.snapshot
            .inventory()
            .iter()
            .filter(|item| {
                item.def
                    .name
                    .as_deref()
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })
            .map(|item| item.count)
            .sum()
    }
    fn quest(&self) -> Option<QuestListStatus> {
        self.snapshot
            .quest_statuses()
            .iter()
            .find(|row| row.name.eq_ignore_ascii_case("Imp Catcher"))
            .map(|row| row.status())
    }
    fn frame(&mut self, client: &mut client::client::Client, cell: Cell) {
        client.set_draw(true);
        self.snapshot.rebuild(client);
        self.ingame = client.ingame;
        self.scene_state = client.scene_state;
        if !client.ingame {
            self.saw_offline |= self.logout_sent;
            return;
        }
        if client.scene_state != 2 {
            return;
        }
        if self.primed.is_none() {
            api::interact::mainland_hop(client);
            for command in [
                "minme",
                "~clearinv",
                "setvar imp 0",
                "give red_bead 1",
                "give yellow_bead 1",
                "give black_bead 1",
                "give white_bead 1",
            ] {
                assert_eq!(
                    api::interact::cheat(client, command),
                    client::CheatSend::Sent,
                    "seed only the owned account: {command}"
                );
            }
            if cell == Cell::Danger {
                // This cell proves permitted arrival, not low-level survival.
                // Normal defensive stats leave the real always-on WWM zone
                // active and do not grant protection or change world RNG.
                for command in ["setstat defence 99", "setstat hitpoints 99"] {
                    assert_eq!(
                        api::interact::cheat(client, command),
                        client::CheatSend::Sent
                    );
                }
            }
            let supply = match cell {
                Cell::Teleport => "give amulet_of_glory_4 1",
                Cell::Danger => "give coins 60",
            };
            assert_eq!(
                api::interact::cheat(client, supply),
                client::CheatSend::Sent
            );
            self.primed = Some(Instant::now());
            return;
        }
        if !self.logout_sent && self.primed.unwrap().elapsed() >= Duration::from_secs(2) {
            let ifaces = Arc::clone(&client.ifaces);
            self.logout_sent = api::interact::logout(client, &ifaces);
            return;
        }
        if !(self.logout_sent && self.saw_offline) {
            return;
        }
        if !self.seed_sent {
            api::interact::seed_at(client, ORIGIN.level, ORIGIN.x, ORIGIN.z);
            self.seed_sent = true;
            return;
        }
        let tile = self.tile();
        if !self.started {
            let supply_ready = match cell {
                Cell::Teleport => self.count("Amulet of glory(4)") == 1,
                Cell::Danger => self.count("Coins") == 60,
            };
            let stats_ready = cell != Cell::Danger
                || [1, 3].iter().all(|index| {
                    self.snapshot
                        .stats()
                        .iter()
                        .any(|stat| stat.index == *index && stat.base == 99 && stat.effective == 99)
                });
            self.ready = tile == Some(ORIGIN)
                && supply_ready
                && stats_ready
                && self.quest() == Some(QuestListStatus::NotStarted)
                && ["Red bead", "Yellow bead", "Black bead", "White bead"]
                    .iter()
                    .all(|name| self.count(name) == 1);
        } else if let (Some(before), Some(after)) = (self.previous_tile, tile) {
            if before.level == after.level
                && before.x.abs_diff(after.x).max(before.z.abs_diff(after.z)) > 16
            {
                self.teleport_at.get_or_insert(after);
            }
        }
        self.previous_tile = tile;
    }
}

fn nav_sample(play: &Play, account: &str) -> Option<(u64, Value)> {
    let navs = play.navs.lock().unwrap();
    let bot = navs.get(account)?;
    let (target, radius, teles, wild, fetch, zones) = bot.requested_route?;
    let legs = bot.route.as_ref().map(|route| route.legs.iter().map(|leg| match leg {
        nav::router::Leg::Walk { tiles } => json!({"kind": "walk", "tiles": tiles}),
        nav::router::Leg::Transport { edge } => json!({"kind": format!("{:?}", edge.kind), "at": edge.at, "to": edge.to, "option": edge.option, "ticks": edge.ticks}),
    }).collect::<Vec<_>>());
    Some((
        bot.walk_request_id,
        json!({
            "request_id": bot.walk_request_id, "target": target, "radius": radius,
            "native_owner": bot.native_walk.as_ref().map(|authority| format!("{:?}", authority.run())),
            "allow_teleports": teles, "allow_wilderness": wild, "allow_bank_fetch": fetch,
            "all_danger_zones": zones.is_all(), "bank_budget_active": bot.bank_fetch.is_some(),
            "guard_active": bot.walk_guard.is_some(), "route": legs,
        }),
    ))
}

fn document(
    play: &Play,
    account: &str,
    state: &LiveState,
    cell: Cell,
    granted: bool,
    admissions: &[Value],
) -> Value {
    json!({
        "request": "WALK-OPTINS-1-S1", "cell": cell.name(), "script_optin": granted,
        "account": account, "origin": ORIGIN, "globals": {
            "teleports": false, "wilderness": false, "danger_zones": false, "bank_fetch": true,
        },
        "seed_phase_finished_before_start": state.ready, "ingame": state.ingame,
        "scene_state": state.scene_state, "teleport_at": state.teleport_at,
        "quest_colour": state.quest(), "native_state": format!("{:?}", play.script_state(account)),
        "native_status": format!("{:?}", play.script_native_status(account)), "native_error": play.script_last_error(account),
        "lifecycle": format!("{:?}", play.script_lifecycle_receipt(account)),
        "admissions": admissions, "core": &state.snapshot,
    })
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, copied cache and explicit engine/nav paths"]
fn live_quester_walk_optins() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let cell = Cell::from_env();
    let root = absolute_env_path("BOT_EVIDENCE_DIR");
    let home = required_directory("HOME", &root);
    let cache = required_directory("BOT_CACHE_DIR", &root);
    let engine = absolute_env_path("WORLD_ENGINE_DIR");
    let pack = absolute_env_path("WORLD_NAV_PACK");
    let flags = absolute_env_path("WORLD_NAV_FLAGS");
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(
            std::env::var("WORLD_GAME_PORT")
                .expect("explicit game port")
                .parse()
                .unwrap(),
        ),
        http_port: Some(
            std::env::var("WORLD_HTTP_PORT")
                .expect("explicit asset port")
                .parse()
                .unwrap(),
        ),
        engine_dir: Some(engine),
        cache_dir: Some(cache.clone()),
        unpack_dir: Some(cache),
        nav_pack: Some(pack),
        nav_flags: Some(flags),
        vault_path: Some(home.join("vault-walk-optins")),
        ..Default::default()
    };
    let template = options.resolve(None).unwrap().prepare_template().unwrap();
    let names = mint_live_names(2);
    let entries = mint_live_entries(&names);
    let states: HashMap<_, _> = names
        .iter()
        .map(|name| {
            (
                name.clone(),
                Arc::new(parking_lot::Mutex::new(LiveState::default())),
            )
        })
        .collect();
    let mailboxes: HashMap<_, _> = names
        .iter()
        .map(|name| (name.clone(), FrameBuf::new()))
        .collect();
    let frame_states = states.clone();
    let frame_mailboxes = mailboxes.clone();
    let play = run_with_template(
        template,
        false,
        entries
            .into_iter()
            .map(|(name, password)| live_profile(name, password))
            .collect(),
        move |name| (None, frame_mailboxes.get(name).cloned()),
        move |client, name, _| {
            if let Some(state) = frame_states.get(name) {
                state.lock().frame(client, cell);
            }
        },
    )
    .unwrap();
    play.set_walk_globals(WalkGlobals {
        allow_bank_fetch: true,
        ..Default::default()
    });
    let stamp = format!(
        "{}Z",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    );
    let dirs: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(i, account)| {
            let dir = root.join(format!(
                "WALK-OPTINS-S1_{}-{}_{}_{}",
                cell.name(),
                if i == 0 { "on" } else { "off" },
                account,
                stamp
            ));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        })
        .collect();
    let setup_deadline = Instant::now() + SETUP_LIMIT;
    while !states.values().all(|state| state.lock().ready) && Instant::now() < setup_deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    for (index, account) in names.iter().enumerate() {
        let granted = index == 0;
        let ready = states[account].lock().ready;
        let report = document(&play, account, &states[account].lock(), cell, granted, &[]);
        assert!(write_capture(
            &dirs[index],
            &stamp,
            "01-origin",
            !ready,
            report,
            &mailboxes[account]
        ));
        assert!(ready, "owned account must reach live ingame scene 2, exact supply and not-started journal before native Start");
        let settings = serde_json::from_value(json!({
            "quests": ["imp"], "allow_teleports": granted && cell == Cell::Teleport,
            "allow_danger_zones": granted && cell == Cell::Danger,
        }))
        .unwrap();
        states[account].lock().started = true;
        play.script_start_handle()
            .start_compiled(account, script::CompiledId("Quester"), settings)
            .unwrap();
        play.wake(account);
    }
    let started = Instant::now();
    let mut admissions = [Vec::new(), Vec::new()];
    let mut previous_request = [None, None];
    let mut control_captured = false;
    loop {
        for (i, account) in names.iter().enumerate() {
            if let Some((request, sample)) = nav_sample(&play, account) {
                if previous_request[i] != Some(request) {
                    previous_request[i] = Some(request);
                    admissions[i].push(sample);
                } else if let Some(last) = admissions[i].last_mut() {
                    if last["route"].is_null() && !sample["route"].is_null() {
                        *last = sample;
                    }
                }
            }
        }
        if !control_captured && started.elapsed() >= CONTROL_LIMIT {
            let account = &names[1];
            let state = states[account].lock();
            let no_teleport = cell != Cell::Teleport
                || (state.teleport_at.is_none() && state.count("Amulet of glory(4)") == 1);
            let observed = !admissions[1].is_empty();
            drop(state);
            let report = document(
                &play,
                account,
                &states[account].lock(),
                cell,
                false,
                &admissions[1],
            );
            assert!(write_capture(
                &dirs[1],
                &stamp,
                "02-off-control",
                !no_teleport || !observed,
                report,
                &mailboxes[account]
            ));
            assert!(
                no_teleport && observed,
                "off control must emit a real native walk without consuming the charged teleport"
            );
            assert!(admissions[1]
                .iter()
                .all(|sample| sample["allow_teleports"] == false
                    && sample["all_danger_zones"] == false));
            play.script_stop(account);
            control_captured = true;
        }
        let complete = states[&names[0]].lock().quest() == Some(QuestListStatus::Complete);
        let failed = play.script_last_error(&names[0]).is_some();
        if (complete && control_captured) || failed || started.elapsed() >= PROGRESS_LIMIT {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let account = &names[0];
    let state = states[account].lock();
    let complete = state.quest() == Some(QuestListStatus::Complete);
    let teleported = state.teleport_at.is_some() && state.count("Amulet of glory(3)") == 1;
    let endpoint = state.tile().is_some_and(|tile| {
        tile.level == 2 && tile.x.abs_diff(3103).max(tile.z.abs_diff(3163)) <= 6
    });
    let scene_ready = state.ingame && state.scene_state == 2;
    drop(state);
    let success = complete && endpoint && scene_ready && (cell != Cell::Teleport || teleported);
    let report = document(
        &play,
        account,
        &states[account].lock(),
        cell,
        true,
        &admissions[0],
    );
    assert!(write_capture(
        &dirs[0],
        &stamp,
        "03-post-arrival-native-progress",
        !success,
        report,
        &mailboxes[account]
    ));
    assert!(success, "grant must reach the permitted endpoint and cause real native quest completion by {PROGRESS_LIMIT:?}; teleport requires real charge consumption and displacement");
    assert!(admissions[0].iter().any(|sample| match cell {
        Cell::Teleport => sample["allow_teleports"] == true,
        Cell::Danger => sample["all_danger_zones"] == true,
    }));
    assert!(
        admissions
            .iter()
            .flatten()
            .all(|sample| sample["allow_bank_fetch"] == false
                && sample["bank_budget_active"] == false
                && sample["guard_active"] == false),
        "global fetch never reaches native BankBudget; danger does not start WalkGuard"
    );
    play.script_stop(account);
}
