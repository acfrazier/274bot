//! Live PvP V1 cells (PVP-DESIGN-1026 §2, §10.1 cases 1, 1d, 2, 4): two fresh
//! accounts of ours on real content, per-account cheats only (stats, hitpoints,
//! `::tele`), and the real Wilderness and Lumbridge the game ships. Nothing is
//! spawned and no rule of the game is bypassed: the attack verb is the label the
//! server put on the other player's row.
//!
//! ```sh
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<pfx> BOT_GAME_PORT=44594 BOT_HTTP_PORT=1080 \
//!   BOT_ENGINE_DIR=<engine> BOT_NAV_PACK=<274bot.navpack> \
//!   BOT_CACHE_DIR=<writable clone of the cache snapshot> LIVE_EVIDENCE_DIR=<dir> \
//!   cargo test -p host-play --features live-harness,test-support --lib \
//!   api_combat_live_tests::pvp:: -- --ignored --nocapture --test-threads=1
//! ```
use super::*;
use crate::local_state::{LocalStateService, PreferencePatch, StateRoot, Update};
use serde_json::{json, Value};

/// Wilderness level 3 in the open east of Edgeville, at least 20 tiles from
/// every NPC spawn of the region files (`m48_55.jm2` NPC section: the skeleton
/// at 3107,3528 and the Edgeville guards are the nearest), so no monster joins
/// the fight. Level 1 starts at z 3523.
const WILDERNESS: WorldTile = WorldTile {
    x: 3133,
    z: 3542,
    level: 0,
};
/// Lumbridge: neither the Wilderness nor a duel pen.
const LUMBRIDGE: WorldTile = WorldTile {
    x: 3222,
    z: 3218,
    level: 0,
};

/// What each account of the pair is seeded with.
struct Seat {
    observed: Arc<Mutex<Observed>>,
    cheats: Vec<String>,
    ready: Box<dyn Fn(&api::snapshot::GameSnapshot) -> bool + Send + Sync>,
    /// Set for the duel cell: the handshake this account drives after ready.
    drive: Option<Arc<Mutex<DuelDrive>>>,
    /// The other account's name as the game shows it.
    partner: String,
}

/// Equal levels for both accounts (attack, strength, defence and hitpoints 20:
/// the same combat level, so the level range of the Wilderness never decides).
fn healthy_seed() -> Vec<String> {
    ["attack", "strength", "defence", "hitpoints"]
        .into_iter()
        .map(|skill| format!("setstat {skill} 20"))
        .collect()
}

/// The same levels, then hitpoints drained to 3: a victim one hit from death,
/// so the opening fight ends by the content's own rules and no chance outcome
/// is waited for.
fn victim_seed() -> Vec<String> {
    let mut seed = healthy_seed();
    seed.push("~stat_drain hitpoints 17 0".to_owned());
    seed
}

fn healthy_ready(snapshot: &api::snapshot::GameSnapshot) -> bool {
    snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.base == 20 && stat.effective >= 18)
}

fn victim_ready(snapshot: &api::snapshot::GameSnapshot) -> bool {
    snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.base == 20 && stat.effective == 3)
}

/// The pen fight's attacker: far stronger than its opponent (attack and
/// strength 60, hitpoints 60 against 20), so that the chance of our own loss
/// is negligible and "until `Killed`" does not depend on a coin flip between
/// equals. A pen has no level range, so unequal levels do not matter there.
fn pen_attacker_seed() -> Vec<String> {
    [
        ("attack", 60),
        ("strength", 60),
        ("defence", 20),
        ("hitpoints", 60),
    ]
    .into_iter()
    .map(|(skill, level)| format!("setstat {skill} {level}"))
    .collect()
}

fn pen_attacker_ready(snapshot: &api::snapshot::GameSnapshot) -> bool {
    snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.base == 60 && stat.effective >= 58)
}

struct Pair {
    play: Play,
    accounts: [String; 2],
    observed: [Arc<Mutex<Observed>>; 2],
    drives: [Arc<Mutex<DuelDrive>>; 2],
    evidence: PathBuf,
    started: Instant,
}

/// Two fresh accounts teleported to `dest`. `global` writes the shared
/// preference `pvp.start_fights = true` through the service before either
/// account is spawned; otherwise the document is absent and the setting is off.
fn start_pair(
    label: &str,
    uids: [i32; 2],
    dest: WorldTile,
    victim: bool,
    global: bool,
    duel: bool,
) -> Pair {
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
    let names = play_bootstrap::mint_live_names(2);
    let credentials = mint_live_entries(&names);
    let observed = [
        Arc::new(Mutex::new(Observed::default())),
        Arc::new(Mutex::new(Observed::default())),
    ];
    let drives = [
        Arc::new(Mutex::new(DuelDrive::default())),
        Arc::new(Mutex::new(DuelDrive::default())),
    ];
    let screen = |name: &str| {
        client::util::JString::to_screen_name(name)
            .as_str()
            .to_owned()
    };
    let mut seats = std::collections::HashMap::new();
    seats.insert(
        names[0].clone(),
        Seat {
            observed: Arc::clone(&observed[0]),
            cheats: if duel {
                pen_attacker_seed()
            } else {
                healthy_seed()
            },
            ready: if duel {
                Box::new(pen_attacker_ready)
            } else {
                Box::new(healthy_ready)
            },
            drive: duel.then(|| Arc::clone(&drives[0])),
            partner: screen(&names[1]),
        },
    );
    seats.insert(
        names[1].clone(),
        Seat {
            observed: Arc::clone(&observed[1]),
            cheats: if victim {
                victim_seed()
            } else {
                healthy_seed()
            },
            ready: if victim {
                Box::new(victim_ready)
            } else {
                Box::new(healthy_ready)
            },
            drive: duel.then(|| Arc::clone(&drives[1])),
            partner: screen(&names[0]),
        },
    );
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame_input| {
            if frame_input.hold {
                return;
            }
            let Some(seat) = seats.get(username) else {
                return;
            };
            let mut state = seat.observed.lock().expect("observed lock");
            frame(client, &mut state, &seat.cheats, &[], &*seat.ready, dest);
            if state.ready {
                if let Some(drive) = &seat.drive {
                    duel_step(
                        client,
                        &mut drive.lock().expect("duel drive lock"),
                        &seat.partner,
                    );
                }
            }
        },
    )
    .expect("start live Play");
    if global {
        let service = LocalStateService::open_process(StateRoot::new(home.join(".274bot")));
        let preferences = service.preferences();
        let patch = PreferencePatch::member(&["pvp", "start_fights"], Some(json!(true)))
            .expect("the pvp member patch");
        match preferences.begin_patch(patch).expect("the pvp write") {
            Update::Durable => {}
            Update::Queued(mut watch) => watch
                .wait(Duration::from_secs(30))
                .expect("the pvp write settles")
                .expect("the pvp write is durable"),
        }
        assert!(
            preferences
                .pvp_globals()
                .expect("the pvp projection")
                .start_fights,
            "the setting reads back on"
        );
        play.set_walk_globals_store(preferences);
    }
    for (index, (username, password)) in credentials.iter().enumerate() {
        play.try_spawn_slot(
            Profile {
                username: username.clone(),
                password: password.clone().into(),
                uid: uids[index],
                settings: ProfileSettings::default(),
            },
            None,
            None,
            None,
        )
        .expect("spawn fresh pvp account");
    }
    let pair = Pair {
        play,
        accounts: [names[0].clone(), names[1].clone()],
        observed,
        drives,
        evidence,
        started,
    };
    let deadline = started + CELL_DEADLINE;
    wait_until(&format!("{label} fixture"), deadline, || {
        (0..2).all(|index| {
            let state = pair.snapshot(index);
            assert!(state.error.is_none(), "fixture error: {:?}", state.error);
            state.ready
        })
    });
    pair
}

/// The Duel Arena handshake one account drives, from the challenge area to the
/// pen (`minigames/game_duelarena/scripts/duel_arena.rs2:8-20,68-108,141-201,
/// 250-269`, `duel_arena_start.rs2`): both accounts offer the other, accept the
/// rules page (default rules, no stake) and the confirm page, and the server
/// teleports both into a pen and counts down to `FIGHT!`. This is the
/// fixture's own use of the game's interface, not the V5 trainer.
#[derive(Default, Clone, serde::Serialize)]
pub(super) struct DuelDrive {
    last_challenge: Option<u32>,
    challenges: u32,
    accept1_at: Option<u32>,
    accept2_at: Option<u32>,
    saw_select: bool,
    saw_confirm: bool,
    saw_pen: bool,
    pub(super) fight_seen: bool,
    refusals: Vec<String>,
}

pub(super) fn duel_step(client: &mut client::client::Client, drive: &mut DuelDrive, partner: &str) {
    use crate::paired_core::{
        fight_pen, DUEL_CONFIRM_ACCEPT, DUEL_CONFIRM_MODAL, DUEL_SELECT_ACCEPT, DUEL_SELECT_MODAL,
    };
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(client);
    if !(client.ingame && client.scene_state == 2) {
        return;
    }
    let tick = snapshot.tick();
    let main = snapshot.modals().main;
    if fight_pen(snapshot.tile()).is_some() {
        drive.saw_pen = true;
        let shout = snapshot
            .local_player()
            .and_then(|local| local.player.actor.overhead_text.as_deref());
        if shout == Some("FIGHT!") {
            drive.fight_seen = true;
        }
        return;
    }
    fn press(
        client: &mut client::client::Client,
        snapshot: &api::snapshot::GameSnapshot,
        id: i32,
        at: &mut Option<u32>,
        refusals: &mut Vec<String>,
    ) {
        let tick = snapshot.tick();
        if at.is_some_and(|at| tick.wrapping_sub(at) < 30) {
            return;
        }
        if let Some(widget) = snapshot.widgets().iter().find(|w| w.component_id == id) {
            if let SendResult::Refused { reason, .. } =
                Interactions::new(snapshot, client).if_button(widget)
            {
                refusals.push(format!("button {id}: {reason:?}"));
            }
            *at = Some(tick);
        }
    }
    if main == DUEL_SELECT_MODAL {
        drive.saw_select = true;
        let DuelDrive {
            accept1_at,
            refusals,
            ..
        } = drive;
        press(client, &snapshot, DUEL_SELECT_ACCEPT, accept1_at, refusals);
        return;
    }
    if main == DUEL_CONFIRM_MODAL {
        drive.saw_confirm = true;
        let DuelDrive {
            accept2_at,
            refusals,
            ..
        } = drive;
        press(client, &snapshot, DUEL_CONFIRM_ACCEPT, accept2_at, refusals);
        return;
    }
    if main > 0
        || drive
            .last_challenge
            .is_some_and(|at| tick.wrapping_sub(at) < 10)
    {
        return;
    }
    let Some(row) = snapshot
        .players()
        .iter()
        .find(|row| row.actor.name.as_deref() == Some(partner))
    else {
        return;
    };
    if !row
        .actor
        .actions
        .iter()
        .flatten()
        .any(|op| op == "Challenge")
    {
        return;
    }
    drive.last_challenge = Some(tick);
    drive.challenges += 1;
    if let SendResult::Refused { reason, .. } = Interactions::new(&snapshot, client)
        .interact(OpTarget::Player(row), ActionSpec::Label("Challenge".into()))
    {
        drive.refusals.push(format!("challenge: {reason:?}"));
    }
}

/// Ticks the pen fight may run before `api.combat.fight` ends it as a budget
/// result; a pen fight that is not `Killed` by then fails the cell.
const PEN_BUDGET_TICKS: u64 = 1_000;

/// The same fight in a Duel Arena pen: the server gives the row `Fight`, the
/// machine uses that label, and the fight runs until `Killed` (design §11). A
/// budget or any other end fails the cell. `global` turns the shared setting on
/// and leaves the request field out, so only the global source authorises it.
fn duel_cell(label: &str, uids: [i32; 2], global: bool, source: &str) {
    let anchor = WorldTile {
        x: crate::paired_core::DUEL_CHALLENGE_ANCHOR.0,
        z: crate::paired_core::DUEL_CHALLENGE_ANCHOR.1,
        level: 0,
    };
    let pair = start_pair(label, uids, anchor, false, global, true);
    wait_until(
        "the duel reaches FIGHT!",
        Instant::now() + Duration::from_secs(180),
        || {
            pair.drives
                .iter()
                .all(|drive| drive.lock().expect("duel drive lock").fight_seen)
        },
    );
    let victim = pair.display_name(1);
    pair.load(0);
    let pvp = if global { "" } else { "pvp: true, " };
    pair.arm(
        0,
        &format!(
            "{{ target: {{ player: {} }}, {pvp}meleeMode: 'aggressive', budgetTicks: {PEN_BUDGET_TICKS} }}",
            js_name(&victim)
        ),
    );
    let mut attack_op_seen = false;
    let mut fight_op_seen = false;
    let mut in_combat_seen = false;
    // 1 000 ticks of 600 ms, and a minute for the pen handshake's slack.
    let limit = Duration::from_millis(PEN_BUDGET_TICKS * 600) + Duration::from_secs(60);
    let outcome = pair.outcome_within(0, label, limit, |ours, _| {
        for row in ours.players.iter().filter(|row| row.name == victim) {
            attack_op_seen |= row.ops.iter().any(|op| op == "Attack");
            fight_op_seen |= row.ops.iter().any(|op| op == "Fight");
        }
        in_combat_seen |= ours.local_in_combat;
    });
    let drives: Vec<DuelDrive> = pair
        .drives
        .iter()
        .map(|drive| drive.lock().expect("duel drive lock").clone())
        .collect();
    pair.receipt(
        &format!("api-pvp-{label}-receipt.json"),
        json!({
            "cell": label,
            "accounts": pair.accounts,
            "victim_name": victim,
            "global_setting": global,
            "budget_ticks": PEN_BUDGET_TICKS,
            "outcome": outcome,
            "phases": pair.phases(0),
            "victim_row_had_attack_op": attack_op_seen,
            "victim_row_had_fight_op": fight_op_seen,
            "attacker_in_combat_seen": in_combat_seen,
            "handshake": drives,
            "attacker": pair.snapshot(0),
        }),
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert_eq!(
        outcome["value"]["report"]["end"], "killed",
        "the pen fight must end Killed, not budget or any other end: {outcome}"
    );
    assert_eq!(outcome["value"]["report"]["pvpSource"], source, "{outcome}");
    assert!(
        outcome["value"]["report"]["swings"].as_u64().unwrap_or(0) > 0,
        "{outcome}"
    );
    assert!(
        fight_op_seen && !attack_op_seen,
        "a duel gives the row the Fight op (fight {fight_op_seen}, attack {attack_op_seen})"
    );
    assert!(in_combat_seen, "the attacker was never in combat");
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_a_duel_pen_fight_uses_the_fight_op() {
    duel_cell("duel", [274_279_409, 274_279_410], false, "request");
}

/// The same pen fight with only the global setting on and no request field.
#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_the_global_setting_starts_the_pen_fight_with_the_fight_op() {
    duel_cell("duel-global", [274_279_411, 274_279_412], true, "global");
}

impl Pair {
    fn snapshot(&self, index: usize) -> Observed {
        self.observed[index].lock().expect("observed lock").clone()
    }

    /// The name account `index` shows to the other, as the other's frame sees
    /// it. A pen teleport or a fresh login takes a few frames to show the row,
    /// so this waits for it (bounded).
    fn display_name(&self, index: usize) -> String {
        let wanted = client::util::JString::to_screen_name(&self.accounts[index]);
        let wanted = wanted.as_str().to_owned();
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let seen = self.snapshot(1 - index);
            if seen.players.iter().any(|row| row.name == wanted) {
                return wanted;
            }
            assert!(
                Instant::now() < deadline,
                "the other account does not see {wanted:?}: rows {:?}; it stands at {:?} (ready {}), the target at {:?} (ready {}); duel drives {:?}",
                seen.players.iter().map(|row| &row.name).collect::<Vec<_>>(),
                seen.tile,
                seen.ready,
                self.snapshot(index).tile,
                self.snapshot(index).ready,
                json!(self
                    .drives
                    .iter()
                    .map(|drive| drive.lock().expect("duel drive lock").clone())
                    .collect::<Vec<_>>())
            );
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Load the phased consumer on account `index` and wait until it runs.
    fn load(&self, index: usize) {
        self.play
            .script_start_load(
                &self.accounts[index],
                PHASED_CONSUMER.to_string(),
                script_js::LoadShape::NativeTick,
                None,
                vec![],
            )
            .expect("start the api.combat Load consumer");
        wait_until("Load Running", Instant::now() + FIGHT_DEADLINE, || {
            self.play.script_state(&self.accounts[index]) == script::RunState::Running
        });
    }

    fn arm(&self, index: usize, request: &str) {
        let expr = format!(
            "globalThis.__outcome = null; globalThis.__phases = []; globalThis.__last = null; \
             globalThis.__stopNow = false; globalThis.__stopSent = false; \
             globalThis.__fight = null; globalThis.__request = {request}; true"
        );
        let armed = probe(&self.play, &self.accounts[index], &expr);
        assert_eq!(armed, true, "arm probe failed: {armed}");
    }

    /// Wait for account `index`'s `api.combat.fight` outcome, calling `sample`
    /// with both accounts' latest frames on every poll.
    fn outcome(
        &self,
        index: usize,
        label: &str,
        sample: impl FnMut(&Observed, &Observed),
    ) -> Value {
        self.outcome_within(index, label, FIGHT_DEADLINE, sample)
    }

    /// [`Self::outcome`] with its own wall-clock limit.
    fn outcome_within(
        &self,
        index: usize,
        label: &str,
        limit: Duration,
        mut sample: impl FnMut(&Observed, &Observed),
    ) -> Value {
        let deadline = Instant::now() + limit;
        loop {
            sample(&self.snapshot(0), &self.snapshot(1));
            let outcome = probe(
                &self.play,
                &self.accounts[index],
                "globalThis.__outcome || null",
            );
            if !outcome.is_null() {
                return outcome;
            }
            assert!(Instant::now() < deadline, "{label}: fight deadline passed");
            thread::sleep(Duration::from_millis(50));
        }
    }

    fn phases(&self, index: usize) -> Value {
        probe(
            &self.play,
            &self.accounts[index],
            "globalThis.__phases || []",
        )
    }

    fn receipt(&self, name: &str, receipt: Value) {
        let path = self.evidence.join(name);
        std::fs::write(
            &path,
            serde_json::to_vec_pretty(&receipt).expect("receipt json"),
        )
        .expect("write receipt");
        println!(
            "{name} ({:.1}s): {}\n{receipt}",
            self.started.elapsed().as_secs_f64(),
            path.display()
        );
    }
}

fn js_name(name: &str) -> String {
    serde_json::to_string(name).expect("a name is a JSON string")
}

/// The fight with a player that the opt-in authorises: Attack in the
/// Wilderness, killed, with the source the report names.
fn opt_in_cell(label: &str, uids: [i32; 2], global: bool, source: &str) {
    let pair = start_pair(label, uids, WILDERNESS, true, global, false);
    let victim = pair.display_name(1);
    pair.load(0);
    let pvp = if global { "" } else { "pvp: true, " };
    pair.arm(
        0,
        &format!(
            "{{ target: {{ player: {} }}, {pvp}meleeMode: 'aggressive', budgetTicks: 400 }}",
            js_name(&victim)
        ),
    );
    let mut attack_op_seen = false;
    let mut fight_op_seen = false;
    let mut in_combat_seen = false;
    let outcome = pair.outcome(0, label, |ours, _| {
        for row in ours.players.iter().filter(|row| row.name == victim) {
            attack_op_seen |= row.ops.iter().any(|op| op == "Attack");
            fight_op_seen |= row.ops.iter().any(|op| op == "Fight");
        }
        in_combat_seen |= ours.local_in_combat;
    });
    let phases = pair.phases(0);
    let after = (pair.snapshot(0), pair.snapshot(1));
    pair.receipt(
        &format!("api-pvp-{label}-receipt.json"),
        json!({
            "cell": label,
            "attacker": pair.accounts[0],
            "victim": pair.accounts[1],
            "victim_name": victim,
            "global_setting": global,
            "outcome": outcome,
            "phases": phases,
            "victim_row_had_attack_op": attack_op_seen,
            "victim_row_had_fight_op": fight_op_seen,
            "attacker_in_combat_seen": in_combat_seen,
            "attacker": after.0,
            "victim_after": after.1,
        }),
    );
    assert_eq!(outcome["kind"], "done", "{outcome}");
    assert_eq!(outcome["value"]["end"], "fought", "{outcome}");
    assert_eq!(outcome["value"]["report"]["end"], "killed", "{outcome}");
    assert_eq!(outcome["value"]["report"]["pvpSource"], source, "{outcome}");
    assert!(
        outcome["value"]["report"]["swings"].as_u64().unwrap_or(0) > 0,
        "{outcome}"
    );
    assert!(
        attack_op_seen && !fight_op_seen,
        "the Wilderness gives the row the Attack op (attack {attack_op_seen}, fight {fight_op_seen})"
    );
    assert!(in_combat_seen, "the attacker was never in combat");
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_the_request_opt_in_starts_a_wilderness_fight_with_the_attack_op() {
    opt_in_cell(
        "request-opt-in",
        [274_279_401, 274_279_402],
        false,
        "request",
    );
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_the_global_setting_starts_the_same_fight_for_a_script_without_pvp() {
    opt_in_cell("global-opt-in", [274_279_403, 274_279_404], true, "global");
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_without_the_opt_in_refuses_not_opted_in_and_still_fights_back() {
    let pair = start_pair(
        "no-opt-in",
        [274_279_405, 274_279_406],
        WILDERNESS,
        false,
        false,
        false,
    );
    let them = pair.display_name(1);
    let us = pair.display_name(0);
    pair.load(0);
    pair.load(1);

    // 1. No request field and the setting off: the start is refused, typed.
    pair.arm(
        0,
        &format!(
            "{{ target: {{ player: {} }}, meleeMode: 'aggressive', budgetTicks: 200 }}",
            js_name(&them)
        ),
    );
    let refused = pair.outcome(0, "refusal", |_, _| {});
    let (ours_after, theirs_after) = (pair.snapshot(0), pair.snapshot(1));
    // The refusal starts nothing: neither account is fighting a moment later.
    let settle = Instant::now() + Duration::from_secs(4);
    let mut fought = false;
    wait_until("refusal settle", settle + Duration::from_secs(5), || {
        let (a, b) = (pair.snapshot(0), pair.snapshot(1));
        fought |= a.local_in_combat || b.local_in_combat;
        Instant::now() >= settle
    });

    // 2. Defence needs no opt-in: we run a defender (no pvp, setting off), the
    //    other account starts a fight with us (its own request field).
    pair.arm(
        0,
        "{ target: { attackers: 'players' }, meleeMode: 'aggressive', budgetTicks: 80 }",
    );
    pair.arm(
        1,
        &format!(
            "{{ target: {{ player: {} }}, pvp: true, meleeMode: 'aggressive', budgetTicks: 40 }}",
            js_name(&us)
        ),
    );
    let mut we_were_in_combat = false;
    let attacker = pair.outcome(1, "attacker", |ours, _| {
        we_were_in_combat |= ours.local_in_combat;
    });
    let defender = pair.outcome(0, "defender", |ours, _| {
        we_were_in_combat |= ours.local_in_combat;
    });
    pair.receipt(
        "api-pvp-no-opt-in-receipt.json",
        json!({
            "cell": "no-opt-in",
            "accounts": pair.accounts,
            "refused": refused,
            "fight_started_after_refusal": fought,
            "attacker_outcome": attacker,
            "defender_outcome": defender,
            "defender_in_combat_seen": we_were_in_combat,
            "attacker_phases": pair.phases(1),
            "defender_phases": pair.phases(0),
            "before_defence": [ours_after, theirs_after],
        }),
    );
    assert_eq!(refused["kind"], "done", "{refused}");
    assert_eq!(refused["value"]["end"], "refused", "{refused}");
    assert_eq!(
        refused["value"]["reason"], "invalid-setting:target:pvp-not-allowed",
        "{refused}"
    );
    assert!(!fought, "a refused start must not start a fight");
    assert_eq!(attacker["value"]["end"], "fought", "{attacker}");
    assert_eq!(
        attacker["value"]["report"]["pvpSource"], "request",
        "{attacker}"
    );
    assert_eq!(defender["value"]["end"], "fought", "{defender}");
    assert!(
        defender["value"]["report"]["pvpSource"].is_null(),
        "self-defence reads neither opt-in source: {defender}"
    );
    // The machine reached its Fight phase on the attacker: it engaged a
    // player it had no opt-in for. Retaliation itself is also the game's own
    // auto-retaliate, so the oracle gates on the machine's engagement (phase,
    // `pvpSource == null`), never on a swing count.
    assert!(
        pair.phases(0)
            .as_array()
            .is_some_and(|phases| phases.iter().any(|phase| phase == "running:fighting")),
        "the defender never reached the fighting phase: {}",
        pair.phases(0)
    );
    assert!(we_were_in_combat, "the defender was never in combat");
}

#[test]
#[ignore = "requires LIVE=1 and the shared local R289 engine"]
fn live_pvp_outside_a_pvp_zone_refuses_with_the_zone_reason() {
    let pair = start_pair(
        "zone",
        [274_279_407, 274_279_408],
        LUMBRIDGE,
        false,
        false,
        false,
    );
    let them = pair.display_name(1);
    pair.load(0);
    // Even with the script's own opt-in the game does not allow it here.
    pair.arm(
        0,
        &format!(
            "{{ target: {{ player: {} }}, pvp: true, meleeMode: 'aggressive', budgetTicks: 200 }}",
            js_name(&them)
        ),
    );
    let refused = pair.outcome(0, "zone refusal", |_, _| {});
    let (ours, theirs) = (pair.snapshot(0), pair.snapshot(1));
    pair.receipt(
        "api-pvp-zone-receipt.json",
        json!({
            "cell": "zone",
            "accounts": pair.accounts,
            "refused": refused,
            "attacker": ours,
            "other": theirs,
        }),
    );
    assert_eq!(refused["kind"], "done", "{refused}");
    assert_eq!(refused["value"]["end"], "refused", "{refused}");
    assert_eq!(
        refused["value"]["reason"], "invalid-setting:target:pvp-outside-wilderness",
        "{refused}"
    );
    assert!(
        !ours.local_in_combat && !theirs.local_in_combat,
        "a refused start must not start a fight"
    );
}
