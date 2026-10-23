//! Engine-A cells for the idle slot's condition, with no script owner.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use api::condition::{BotCondition, PoisonState};
use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::GameSnapshot;
use parking_lot::Mutex;
use vault::{Profile, ProfileSettings};

use super::bank_core_live::{self, Cell, Prep};
use super::{run_with_template, SharedClientTemplate};

#[derive(Clone, Copy)]
enum Manual {
    Poison,
    Drink(i32),
    Open,
    Deposit(i32),
}

#[derive(Default)]
struct Live {
    snapshot: GameSnapshot,
    pump: host::Pump,
    action: Option<Manual>,
    sent: Option<bool>,
    frame: u64,
    previous_line: bool,
    edge_line_visible_before: bool,
    tick: u32,
    player_info_generation: u64,
    npc_info_generation: u64,
    npc_rows_coherent: bool,
}

impl Live {
    fn observe(&mut self, client: &client::client::Client) {
        // Preparation relogs. Use the condition owner's session-local clock.
        let drain = self.pump.drain_client(client);
        host::publish_snapshot(&mut self.snapshot, client, drain);
        self.player_info_generation = client.gens.player_info;
        self.npc_info_generation = client.gens.npc_info;
        self.npc_rows_coherent = drain.npc_rows_coherent;
    }
}

struct PoisonOracle {
    deposit: bool,
    since: Option<u32>,
    marks: Vec<(u32, u8)>,
    last_mark: Option<u16>,
    last_tick: Option<u32>,
    immune_origin: Option<u32>,
}

impl PoisonOracle {
    fn new(deposit: bool) -> Self {
        Self {
            deposit,
            since: None,
            marks: Vec::new(),
            last_mark: None,
            last_tick: None,
            immune_origin: None,
        }
    }

    fn observe(
        &mut self,
        tick: u32,
        condition: BotCondition,
        drink_line: bool,
        dose_held: bool,
    ) -> Result<bool, String> {
        let elapsed = self
            .last_tick
            .map_or(1, |previous| tick.saturating_sub(previous));
        self.last_tick = Some(tick);
        if condition.hp == 0 {
            return Err(format!("death at tick {tick}"));
        }
        if let PoisonState::Onset { since, q, .. } = condition.poison {
            if self.since.is_none() {
                if q != Some(11) {
                    return Err(format!(
                        "wrong onset at tick {tick}: {:?}",
                        condition.poison
                    ));
                }
                self.since = Some(u32::from(since));
            }
        }
        if let PoisonState::Poisoned {
            per_tick,
            last_mark,
            hit_since_mark,
        } = condition.poison
        {
            if self.last_mark != Some(last_mark) {
                self.last_mark = Some(last_mark);
                self.marks.push((tick, per_tick));
                if hit_since_mark {
                    return Err(format!("unexpected ordinary hit after mark at {tick}"));
                }
                if !self.deposit {
                    let expected = [6, 6, 5];
                    if self.marks.len() > expected.len()
                        || per_tick != expected[self.marks.len() - 1]
                    {
                        return Err(format!("wrong poison fires: {:?}", self.marks));
                    }
                    if self.marks.len() == 1
                        && !self
                            .since
                            .is_some_and(|since| tick >= since + 30 && tick <= since + 32)
                    {
                        return Err(format!(
                            "timer-phase-uncontrolled: since={:?}, first={tick}",
                            self.since
                        ));
                    }
                    if self.marks.len() > 1 && tick - self.marks[self.marks.len() - 2].0 != 30 {
                        return Err(format!("poison fire interval is not 30: {:?}", self.marks));
                    }
                }
            }
        }
        if let PoisonState::Immune { .. } = condition.poison {
            self.immune_origin
                .get_or_insert(tick.saturating_sub(elapsed.max(1) - 1));
            if !drink_line || dose_held {
                return Err(format!(
                    "immunity lacks matching drink and dose drop at {tick}"
                ));
            }
        }
        if let Some(origin) = self.immune_origin {
            let expected = if tick <= origin + 120 {
                PoisonState::Immune {
                    until_lower: (origin + 120) as u16,
                }
            } else {
                PoisonState::Clear
            };
            if condition.poison != expected {
                return Err(format!(
                    "wrong immunity at {tick}: expected {expected:?}, got {:?}",
                    condition.poison
                ));
            }
            if !self.deposit && tick > origin + 120 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn deposited(&self, condition: BotCondition, drink_line: bool) -> Result<(), String> {
        if condition.cures.antipoison != 0
            || drink_line
            || !matches!(
                condition.poison,
                PoisonState::Onset { .. } | PoisonState::Poisoned { .. }
            )
        {
            return Err(format!(
                "deposit incorrectly credited a cure: {:?}",
                condition.poison
            ));
        }
        Ok(())
    }
}

fn run_condition_cell(super_dose: bool, deposit: bool) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let label = if deposit {
        "l-cond-deposit"
    } else if super_dose {
        "l-cond-poison-super"
    } else {
        "l-cond-poison-ordinary"
    };
    let root = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").expect("LIVE_EVIDENCE_DIR"));
    let account = super::mint_live_names(1).pop().unwrap();
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let evidence = root.join(format!("{label}_{account}_utc-{epoch}Z"));
    let scratch = evidence.join("scratch");
    std::fs::create_dir_all(&scratch).unwrap();
    let profile = bank_core_live::live_profile(&scratch)
        .unwrap()
        .resolve(None)
        .and_then(|profile| profile.bind_runtime())
        .unwrap();
    assert!(
        profile.map_members(),
        "HARNESS PREREQUISITE: map_members = true is required by poison.rs2:7-10"
    );
    assert_eq!(profile.client().game_host(), "127.0.0.1");
    let template = SharedClientTemplate::load(profile).unwrap();
    let seed: &'static [&'static str] = if super_dose {
        &["setstat hitpoints 60", "give 1dose2antipoison 1"]
    } else {
        &["setstat hitpoints 60", "give 1doseantipoison 1"]
    };
    let id = if super_dose { 185 } else { 179 };
    let prep = Arc::new(Mutex::new(Cell::new(
        Duration::from_secs(240),
        seed,
        |snapshot| {
            snapshot
                .stats()
                .iter()
                .any(|stat| stat.index == 3 && stat.effective == 60)
                && snapshot
                    .inventory()
                    .iter()
                    .any(|item| matches!(item.def.id, 179 | 185))
        },
        &[179, 185],
        serde_json::json!({ "scenario": label }),
        |_| false,
        evidence.clone(),
    )));
    let started = prep.lock().preparation_started;
    let live = Arc::new(Mutex::new(Live::default()));
    let frame_prep = Arc::clone(&prep);
    let frame_live = Arc::clone(&live);
    let frame_account = account.clone();
    let play = run_with_template(
        template,
        true,
        vec![Profile {
            username: account.clone(),
            password: account.clone().into(),
            uid: 274_279_111,
            settings: ProfileSettings::default(),
        }],
        |_| (None, None),
        move |client, _, _| {
            bank_core_live::frame(client, &frame_prep, &frame_account);
            let mut live = frame_live.lock();
            live.observe(client);
            let tick = live.snapshot.tick();
            if tick != live.tick {
                live.edge_line_visible_before = live.previous_line;
                live.tick = tick;
            }
            live.previous_line = live
                .snapshot
                .chat_lines()
                .iter()
                .any(|line| line.text == "You have been poisoned!");
            live.frame += 1;
            if let Some(action) = live.action.take() {
                let snapshot = &live.snapshot;
                let sent = match action {
                    Manual::Poison => {
                        // Engine A's [debugproc,poison] uses the configured "~" prefix.
                        interact::cheat(client, "~poison 27") == client::CheatSend::Sent
                    }
                    Manual::Open => matches!(
                        Interactions::new(snapshot, client).open_nearest_booth(),
                        SendResult::Sent { .. }
                    ),
                    Manual::Drink(id) | Manual::Deposit(id) => {
                        let deposit = matches!(action, Manual::Deposit(_));
                        let rows = if deposit {
                            snapshot.bank_side()
                        } else {
                            snapshot.inventory()
                        };
                        rows.iter()
                            .find(|item| item.def.id == id)
                            .is_some_and(|item| {
                                matches!(
                                    Interactions::new(snapshot, client).interact(
                                        OpTarget::Item(item),
                                        ActionSpec::Label(
                                            if deposit { "Deposit 1" } else { "Drink" }.into()
                                        )
                                    ),
                                    SendResult::Sent { .. }
                                )
                            })
                    }
                };
                live.sent = Some(sent);
            }
        },
    )
    .unwrap();
    let mut login_ready = false;
    let mut ready_tick = None;
    let mut seed_tick = None;
    let mut oracle = PoisonOracle::new(deposit);
    let mut drink_requested = false;
    let mut open_requested = false;
    let mut deposit_requested = false;
    let mut seen_tick = None;
    let mut rows = Vec::new();
    let mut failure = None;
    let mut passed = false;
    loop {
        bank_core_live::check_prerequisites(&play, &account, started, &mut login_ready, &prep);
        if started.elapsed() > Duration::from_secs(360) {
            failure = Some("watchdog: cell did not reach its observed-tick verdict".to_owned());
            break;
        }
        let status = play
            .statuses()
            .into_iter()
            .find(|status| status.username == account);
        let mut live = live.lock();
        if live.sent.take() == Some(false) {
            failure = Some("manual fixture action was refused".to_owned());
            break;
        }
        if prep.lock().phase != Prep::Ready {
            drop(live);
            std::thread::sleep(Duration::from_millis(100));
            continue;
        }
        let Some(status) = status else {
            continue;
        };
        let tick = live.snapshot.tick();
        if status.condition.tick != tick || seen_tick == Some(tick) {
            drop(live);
            std::thread::sleep(Duration::from_millis(50));
            continue;
        }
        seen_tick = Some(tick);
        let condition: BotCondition = status.condition;
        assert!(
            play.script_native_run(&account).is_none(),
            "this cell must have no script"
        );
        let ready = *ready_tick.get_or_insert(tick);
        let drink_line = live
            .snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text == "You drink some of your antipoison potion.");
        rows.push(serde_json::json!({
            "tick": tick,
            "condition": { "tick": condition.tick, "hp": condition.hp, "poison": format!("{:?}", condition.poison), "cures": format!("{:?}", condition.cures) },
            "line_visible_before_edge": live.edge_line_visible_before,
            "raw_chat": live.snapshot.chat_lines().iter().map(|line| serde_json::json!({ "sequence": line.sequence, "text": line.text })).collect::<Vec<_>>(),
            "raw_hitmarks": live.snapshot.hitmarks().map(|marks| marks.marks.iter().map(|mark| serde_json::json!({ "kind": mark.kind, "value": mark.value, "cycle": mark.cycle, "loop_cycle": marks.loop_cycle })).collect::<Vec<_>>()),
            "player_info_generation": live.player_info_generation,
            "npc_info_generation": live.npc_info_generation,
            "npc_rows_coherent": live.npc_rows_coherent,
            "drink_line": drink_line,
            "held": live.snapshot.inventory().iter().filter(|item| item.def.id == id).map(|item| item.count).sum::<i32>(),
            "bank_open": live.snapshot.bank_component_id() >= 0,
            "native_run": null
        }));
        let dose_held = live
            .snapshot
            .inventory()
            .iter()
            .any(|item| item.def.id == id);
        match oracle.observe(tick, condition, drink_line, dose_held) {
            Ok(true) => {
                passed = true;
                break;
            }
            Ok(false) => {}
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
        if seed_tick.is_none() && tick.saturating_sub(ready) >= 31 {
            seed_tick = Some(tick);
            live.action = Some(Manual::Poison);
        }
        if deposit && oracle.since.is_some() {
            if !open_requested {
                live.action = Some(Manual::Open);
                open_requested = true;
            } else if !deposit_requested
                && live
                    .snapshot
                    .bank_side()
                    .iter()
                    .any(|item| item.def.id == id)
            {
                live.action = Some(Manual::Deposit(id));
                deposit_requested = true;
            } else if deposit_requested && condition.cures.antipoison == 0 {
                match oracle.deposited(condition, drink_line) {
                    Ok(()) => passed = true,
                    Err(error) => failure = Some(error),
                }
                break;
            }
        } else if !deposit && oracle.marks.len() >= 3 && !drink_requested {
            live.action = Some(Manual::Drink(id));
            drink_requested = true;
        }
        if seed_tick.is_some_and(|seed| tick.saturating_sub(seed) > if deposit { 60 } else { 260 })
        {
            failure = Some(format!("observed-tick deadline at {tick}"));
            break;
        }
        drop(live);
        std::thread::sleep(Duration::from_millis(50));
    }
    let receipt = serde_json::json!({ "scenario": label, "account": account, "map_members": true, "no_script": true, "ready_tick": ready_tick, "seed_tick": seed_tick, "since": oracle.since, "marks": oracle.marks, "immune_origin": oracle.immune_origin, "passed": passed, "failure": failure, "frames": rows });
    std::fs::write(
        evidence.join("receipt.json"),
        serde_json::to_vec_pretty(&receipt).unwrap(),
    )
    .unwrap();
    println!(
        "{} {label}: {}",
        if passed { "PASS" } else { "FAIL" },
        evidence.join("receipt.json").display()
    );
    assert!(passed, "{failure:?}; evidence={}", evidence.display());
}

#[test]
#[ignore = "requires LIVE=1 and Engine A fixture paths"]
fn live_condition_poison_ordinary_no_script() {
    run_condition_cell(false, false);
}

#[test]
#[ignore = "requires LIVE=1 and Engine A fixture paths"]
fn live_condition_poison_super_no_script() {
    run_condition_cell(true, false);
}

#[test]
#[ignore = "requires LIVE=1 and Engine A fixture paths"]
fn live_condition_deposit_no_script() {
    run_condition_cell(false, true);
}

mod oracle_tests {
    use super::*;

    fn condition(poison: PoisonState) -> BotCondition {
        BotCondition {
            hp: 60,
            hp_max: 60,
            hp_known: true,
            poison,
            ..BotCondition::default()
        }
    }

    fn onset(q: u8, latched: bool) -> BotCondition {
        condition(PoisonState::Onset {
            since: 100,
            q: latched.then_some(q),
            hit_since_line: false,
        })
    }

    fn mark(tick: u32, value: u8, refreshed: bool) -> BotCondition {
        condition(PoisonState::Poisoned {
            per_tick: value,
            last_mark: tick as u16,
            hit_since_mark: refreshed,
        })
    }

    fn before_drink() -> PoisonOracle {
        let mut oracle = PoisonOracle::new(false);
        assert_eq!(oracle.observe(100, onset(11, true), false, true), Ok(false));
        for (tick, value) in [(130, 6), (160, 6), (190, 5)] {
            assert_eq!(
                oracle.observe(tick, mark(tick, value, false), false, true),
                Ok(false)
            );
        }
        oracle
    }

    #[test]
    fn live_observer_relog_keeps_condition_stamp_comparable() {
        use client::client::{Client, ClientConfig, ClientPlayer};

        let mut client = Client::new(ClientConfig {
            host: "127.0.0.1".into(),
            port: 0,
            cache_dir: String::new(),
            members: true,
            lowmem: true,
        });
        client.ingame = true;
        client.local_player = Some(ClientPlayer::at(1, 1));
        client.gens.player = 1;
        client.gens.player_info = 1;
        client.gens.npc_info = 1;
        let mut live = Live::default();
        live.observe(&client);
        assert_eq!(live.snapshot.tick(), 1);

        client.ingame = false;
        live.observe(&client);
        client.ingame = true;
        client.gens.player += 1;
        client.gens.player_info += 1;
        client.gens.npc_info += 1;
        live.observe(&client);
        assert_eq!(
            live.snapshot.tick(),
            1,
            "the relogged observer must use the condition's new session clock"
        );
        live.observe(&client);
        assert_eq!(live.snapshot.tick(), 1, "a quiet frame is not a game tick");
        client.gens.player += 1;
        client.gens.player_info += 1;
        client.gens.npc_info += 1;
        live.observe(&client);
        assert_eq!(live.snapshot.tick(), 2);
    }

    #[test]
    fn condition_oracle_accepts_three_fires_and_exact_ordinary_credit() {
        let mut oracle = before_drink();
        let immune = condition(PoisonState::Immune { until_lower: 311 });
        for tick in 191..=311 {
            assert_eq!(oracle.observe(tick, immune, true, false), Ok(false));
        }
        assert_eq!(
            oracle.observe(312, condition(PoisonState::Clear), true, false),
            Ok(true)
        );
    }

    #[test]
    fn condition_oracle_rejects_death_and_wrong_or_unlatched_onset() {
        for bad in [onset(3, true), onset(11, false), BotCondition::default()] {
            assert!(PoisonOracle::new(false)
                .observe(100, bad, false, true)
                .is_err());
        }
    }

    #[test]
    fn condition_oracle_rejects_wrong_fire_phase_value_interval_and_refresh() {
        for (tick, value, refreshed) in [
            (129, 6, false),
            (133, 6, false),
            (130, 5, false),
            (130, 6, true),
        ] {
            let mut oracle = PoisonOracle::new(false);
            oracle.observe(100, onset(11, true), false, true).unwrap();
            assert!(oracle
                .observe(tick, mark(tick, value, refreshed), false, true)
                .is_err());
        }
        for (tick, value) in [(159, 6), (161, 6), (160, 5)] {
            let mut oracle = PoisonOracle::new(false);
            oracle.observe(100, onset(11, true), false, true).unwrap();
            oracle
                .observe(130, mark(130, 6, false), false, true)
                .unwrap();
            assert!(oracle
                .observe(tick, mark(tick, value, false), false, true)
                .is_err());
        }
    }

    #[test]
    fn condition_oracle_rejects_missing_drink_drop_super_credit_and_early_or_late_expiry() {
        for (line, held, until) in [(false, false, 311), (true, true, 311), (true, false, 761)] {
            assert!(before_drink()
                .observe(
                    191,
                    condition(PoisonState::Immune { until_lower: until }),
                    line,
                    held
                )
                .is_err());
        }
        let immune = condition(PoisonState::Immune { until_lower: 311 });
        let mut early = before_drink();
        early.observe(191, immune, true, false).unwrap();
        assert!(early
            .observe(200, condition(PoisonState::Clear), true, false)
            .is_err());
        let mut late = before_drink();
        late.observe(191, immune, true, false).unwrap();
        assert!(late.observe(312, immune, true, false).is_err());
    }

    #[test]
    fn condition_oracle_deposit_keeps_poison_and_rejects_cure_credit() {
        let oracle = PoisonOracle::new(true);
        assert!(oracle.deposited(mark(130, 6, false), false).is_ok());
        assert!(oracle.deposited(onset(11, true), false).is_ok());
        for poison in [PoisonState::Clear, PoisonState::Immune { until_lower: 250 }] {
            assert!(oracle.deposited(condition(poison), false).is_err());
        }
        assert!(oracle.deposited(onset(11, true), true).is_err());
        let mut held = onset(11, true);
        held.cures.antipoison = 1;
        assert!(oracle.deposited(held, false).is_err());
    }
}
