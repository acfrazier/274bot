//! Live PvP V2 smoke (PVP-DESIGN-1026 §4, §11 V2 row): two native bots in a
//! Duel Arena pen on real content, the accounts' own state seeded with per-account
//! cheats only (stats, items). One account holds Protect from Melee, the other
//! runs the native `Combat` machine with a sword kit and a bow kit.
//!
//! 1. T2: the machine opens in melee, the opponent's Protect from Melee shows on
//!    two frames, and the machine converges to its bow and completes.
//! 2. T3: the opponent drops the prayer; with nothing protected and the opponent
//!    in reach the machine returns to its first preference, melee.
//! 3. T4: the opponent runs away; out of reach for two attack intervals with no
//!    swing of ours, the machine switches to its bow.
//!
//! Every transition is read from the live machine (`ActionHandle::inspect`) with
//! the host tick of the frame that showed it, next to the worn items, the
//! opponent's head icons and the distance between the two players.
//!
//! ```sh
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=<pfx> \
//!   BOT_GAME_PORT=44594 BOT_HTTP_PORT=1080 BOT_ENGINE_DIR=<engine> \
//!   BOT_NAV_PACK=<274bot.navpack> BOT_CACHE_DIR=<writable cache clone> \
//!   LIVE_EVIDENCE_DIR=<dir> cargo test -p host-play \
//!   --features live-harness,test-support,live-probe --lib \
//!   api_combat_live_tests::pvp_switch:: -- --ignored --nocapture --test-threads=1
//! ```
use super::pvp::{duel_step, DuelDrive};
use super::*;
use script::combat::{
    Combat, CombatRequest, CombatTables, CompiledKit, Fallback, Style, StyleKits, SwitchPolicy,
    SwitchState, Target,
};
use script::native::{ActionHandle, NativeTick, PvpGlobals, Script, ScriptFailure, ScriptFlow};
use serde_json::{json, Value};
use std::task::Poll;

/// Ticks each phase may take before the cell fails (600 ms each).
const PHASE_TICKS: u64 = 160;
/// Observed ticks the machine must stay `Idle` before the next phase starts.
const QUIET_TICKS: u32 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, serde::Serialize)]
enum Phase {
    #[default]
    Setup,
    /// The partner holds Protect from Melee; the machine opens in melee.
    Hold,
    /// The partner drops it.
    Drop,
    /// The partner runs away.
    Away,
    Done,
}

#[derive(Clone, Default, serde::Serialize)]
struct TraceRow {
    tick: u64,
    phase: Phase,
    active: String,
    switch: String,
    switches: u8,
    failures: u8,
    /// Wear refusals counted for the right hand and the quiver.
    refusals: (u8, u8),
    /// `(slot, obj id)` worn by the machine's account.
    worn: Vec<(i32, i32)>,
    /// The partner's head icon bits (bit 3 melee, 4 ranged, 5 magic).
    partner_headicons: i32,
    /// Chebyshev distance between the two players.
    distance: i32,
    own_tile: Option<(i32, i32, i32)>,
    partner_tile: Option<(i32, i32, i32)>,
}

#[derive(Clone, serde::Serialize)]
struct ActionRow {
    tick: u64,
    what: String,
}

#[derive(Default)]
struct Shared {
    phase: Phase,
    phase_since: u64,
    trace: Vec<TraceRow>,
    actions: Vec<ActionRow>,
    begin_error: Option<String>,
    begun_tick: Option<u64>,
    failure: Option<String>,
    last: Option<(Style, SwitchState, u8, u8, u8, u8)>,
    /// The Chebyshev distance to the holder at every observed tick.
    series: Vec<(u64, i32)>,
    quiet: u32,
}

impl Shared {
    fn advance(&mut self, to: Phase, tick: u64) {
        self.phase = to;
        self.phase_since = tick;
        self.actions.push(ActionRow {
            tick,
            what: format!("phase {to:?}"),
        });
    }
}

/// The machine's native script: begins one `Combat` against the partner and
/// samples it after every poll.
struct SwitchFight {
    shared: Arc<Mutex<Shared>>,
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    handle: Option<ActionHandle<Combat>>,
    tried: bool,
    /// The holder's screen name: other accounts may share the arena.
    partner: String,
}

impl Script for SwitchFight {
    fn tick(&mut self, tick: &mut NativeTick<'_>) -> Result<ScriptFlow, ScriptFailure> {
        let now = tick.cx.evidence().tick;
        if !self.tried {
            self.tried = true;
            match tick.actions.begin::<Combat>(
                (
                    Arc::clone(&self.request),
                    Arc::clone(&self.tables),
                    PvpGlobals::fail_closed(),
                ),
                &mut tick.cx,
            ) {
                Ok(handle) => {
                    self.handle = Some(handle);
                    self.shared.lock().expect("shared lock").begun_tick = Some(now);
                }
                Err(error) => {
                    self.shared.lock().expect("shared lock").begin_error =
                        Some(format!("{error:?}"));
                }
            }
        }
        if self.handle.is_some() {
            let mut shared = self.shared.lock().expect("shared lock");
            if shared.actions.len() < 80 {
                let view = tick
                    .frame()
                    .snapshot()
                    .map(|snapshot| {
                        format!(
                            "own {:?} players {:?}",
                            snapshot.tile(),
                            snapshot
                                .players()
                                .iter()
                                .map(|row| (
                                    row.actor.name.clone(),
                                    row.actor.tile.x,
                                    row.actor.tile.z,
                                    row.actor
                                        .actions
                                        .iter()
                                        .flatten()
                                        .cloned()
                                        .collect::<Vec<_>>(),
                                ))
                                .collect::<Vec<_>>()
                        )
                    })
                    .unwrap_or_else(|| "no snapshot".into());
                shared.actions.push(ActionRow {
                    tick: now,
                    what: format!("view {view}"),
                });
            }
        }
        let polled = match self.handle.as_ref() {
            Some(handle) => tick.actions.poll(handle, &mut tick.cx),
            None => return Ok(ScriptFlow::Continue),
        };
        if let Poll::Ready(result) = polled {
            self.shared.lock().expect("shared lock").failure = Some(match result {
                Ok(report) => format!("the fight ended: {:?}", report.end),
                Err(error) => format!("the fight failed: {error:?}"),
            });
            self.handle = None;
            return Ok(ScriptFlow::Continue);
        }
        let state = self.handle.as_ref().and_then(|handle| {
            handle.inspect(|combat| {
                (
                    combat.active_style(),
                    combat.switch_state(),
                    combat.switches(),
                    combat.switch_failures(),
                    combat.wear_refusals(3),
                    combat.wear_refusals(13),
                )
            })
        });
        let Some(state) = state else {
            return Ok(ScriptFlow::Continue);
        };
        let mut shared = self.shared.lock().expect("shared lock");
        let snapshot = tick.frame().snapshot();
        let partner = snapshot.and_then(|snapshot| {
            snapshot
                .players()
                .iter()
                .find(|row| row.actor.name.as_deref() == Some(self.partner.as_str()))
                .cloned()
        });
        let own_tile = snapshot.and_then(|snapshot| snapshot.tile());
        let partner_tile = partner
            .as_ref()
            .map(|row| (row.actor.tile.x, row.actor.tile.z, row.actor.tile.level));
        let distance = match (own_tile, partner_tile) {
            (Some(a), Some(b)) => (a.0 - b.0).abs().max((a.1 - b.1).abs()),
            _ => -1,
        };
        if shared.series.last().is_none_or(|(at, _)| *at != now) {
            shared.series.push((now, distance));
        }
        if shared.last != Some(state) {
            let row = TraceRow {
                tick: now,
                phase: shared.phase,
                active: format!("{:?}", state.0),
                switch: format!("{:?}", state.1),
                switches: state.2,
                failures: state.3,
                refusals: (state.4, state.5),
                worn: snapshot
                    .map(|snapshot| {
                        snapshot
                            .equipment()
                            .iter()
                            .map(|item| (item.slot, item.def.id))
                            .collect()
                    })
                    .unwrap_or_default(),
                partner_headicons: partner.as_ref().map_or(0, |row| row.headicons),
                distance,
                own_tile,
                partner_tile,
            };
            shared.trace.push(row);
            shared.last = Some(state);
        }
        shared.quiet = if state.1 == SwitchState::Idle {
            shared.quiet + 1
        } else {
            0
        };
        let (active, switches) = (state.0, state.2);
        let waited = now.saturating_sub(shared.phase_since);
        match shared.phase {
            Phase::Hold
                if switches >= 1 && active == Style::Ranged && shared.quiet >= QUIET_TICKS =>
            {
                shared.advance(Phase::Drop, now)
            }
            Phase::Drop
                if switches >= 2 && active == Style::Melee && shared.quiet >= QUIET_TICKS =>
            {
                shared.advance(Phase::Away, now)
            }
            // The third completed switch is the out-of-reach one; a later return to
            // melee (the chaser is adjacent again) is the machine behaving.
            Phase::Away if switches >= 3 => shared.advance(Phase::Done, now),
            Phase::Hold | Phase::Drop | Phase::Away if waited > PHASE_TICKS => {
                shared.failure = Some(format!(
                    "phase {:?} did not complete in {PHASE_TICKS} ticks",
                    shared.phase
                ));
                shared.advance(Phase::Done, now);
            }
            _ => {}
        }
        Ok(ScriptFlow::Continue)
    }
}

/// What the partner account's frame hook does in each phase.
#[derive(Default)]
struct PartnerState {
    clicked_tick: Option<u64>,
    /// The two waypoints of the run, fixed on the first Away tick.
    legs: Option<(api::snapshot::WorldTile, api::snapshot::WorldTile)>,
    at_second: bool,
    sent: Option<api::snapshot::WorldTile>,
    run_set: bool,
    walked_tick: Option<u64>,
    refusals: Vec<String>,
}

fn partner_step(
    client: &mut client::client::Client,
    shared: &Arc<Mutex<Shared>>,
    state: &mut PartnerState,
    protect_varp: i32,
    protect_button: i32,
) {
    use api::interact::Interactions;
    let mut snapshot = api::snapshot::GameSnapshot::new();
    snapshot.rebuild(client);
    if !(client.ingame && client.scene_state == 2) {
        return;
    }
    let tick = u64::from(snapshot.tick());
    let phase = shared.lock().expect("shared lock").phase;
    let pace = |at: &Option<u64>, ticks: u64| at.is_some_and(|at| tick.saturating_sub(at) < ticks);
    match phase {
        Phase::Hold | Phase::Drop => {
            let on = varp(&snapshot, protect_varp) == 1;
            let wanted = phase == Phase::Hold;
            if on != wanted && !pace(&state.clicked_tick, 5) {
                if let Some(widget) = snapshot
                    .widgets()
                    .iter()
                    .chain(
                        snapshot
                            .side_tabs()
                            .iter()
                            .flat_map(|tab| tab.widgets.iter()),
                    )
                    .find(|widget| widget.component_id == protect_button)
                {
                    state.clicked_tick = Some(tick);
                    match Interactions::new(&snapshot, client).if_button(widget) {
                        api::interact::SendResult::Refused { reason, .. } => {
                            state.refusals.push(format!("prayer {phase:?}: {reason:?}"));
                        }
                        _ => shared.lock().expect("shared lock").actions.push(ActionRow {
                            tick,
                            what: format!("prayer click ({phase:?}), varp was {on}"),
                        }),
                    }
                }
            }
        }
        Phase::Away => {
            if !state.run_set {
                state.run_set = true;
                let result = Interactions::new(&snapshot, client).set_run(true);
                shared.lock().expect("shared lock").actions.push(ActionRow {
                    tick,
                    what: format!(
                        "run on: {}",
                        matches!(result, api::interact::SendResult::Sent { .. })
                    ),
                });
                return;
            }
            let Some(own) = snapshot.tile() else {
                return;
            };
            let Some((min_x, max_x, min_z, max_z)) = crate::paired_core::fight_pen(Some(own))
            else {
                return;
            };
            // Two legs: the pen's far end along its long side, then the far end
            // of that wall, so a runner outpaces a walker for long enough.
            let (first, second) = *state.legs.get_or_insert_with(|| {
                let x = if own.0 - min_x > max_x - own.0 {
                    min_x + 1
                } else {
                    max_x - 1
                };
                let z = if own.1 - min_z > max_z - own.1 {
                    min_z + 1
                } else {
                    max_z - 1
                };
                let tile = |z| api::snapshot::WorldTile { x, z, level: own.2 };
                // A tile on the pen's own edge row is not walkable: stay inside it.
                (tile(own.1.clamp(min_z + 1, max_z - 1)), tile(z))
            });
            let arrived = (own.0 - first.x).abs().max((own.1 - first.z).abs()) <= 1;
            let far = if arrived || state.at_second {
                second
            } else {
                first
            };
            state.at_second |= arrived;
            if !pace(&state.walked_tick, 12) || (arrived && state.sent != Some(far)) {
                state.walked_tick = Some(tick);
                state.sent = Some(far);
                let result = Interactions::new(&snapshot, client).walk_nearest(far);
                shared.lock().expect("shared lock").actions.push(ActionRow {
                    tick,
                    what: format!(
                        "walk to {},{}: {}",
                        far.x,
                        far.z,
                        matches!(result, api::interact::SendResult::Sent { .. })
                    ),
                });
            }
        }
        Phase::Setup | Phase::Done => {}
    }
}

fn pvp_prayer(name: &str) -> (i32, i32) {
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let row = data.prayer_by_name(name).expect("prayer fact");
    (row.varp, row.button_com)
}

/// Stats for the holder: hard to hurt, Protect from Melee available.
fn holder_seed() -> Vec<String> {
    [
        "setstat prayer 60",
        "setstat defence 99",
        "setstat hitpoints 99",
        "setstat attack 1",
        "setstat strength 1",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// Stats and items for the machine's account: a sword, a bow and arrows.
fn fighter_seed() -> Vec<String> {
    [
        "setstat attack 60",
        "setstat strength 1",
        "setstat defence 99",
        "setstat ranged 60",
        "setstat hitpoints 99",
        "give rune_scimitar 1",
        "give maple_shortbow 1",
        "give iron_arrow 400",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn holder_ready(snapshot: &api::snapshot::GameSnapshot) -> bool {
    snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.base == 99 && stat.effective >= 97)
        && snapshot
            .stats()
            .iter()
            .any(|stat| stat.index == 5 && stat.base >= 43 && stat.effective >= 43)
}

fn fighter_ready(snapshot: &api::snapshot::GameSnapshot) -> bool {
    let held = |name: &str| {
        let id = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .unwrap()
            .item_by_alias(name)
            .unwrap()
            .id;
        snapshot.inventory().iter().any(|item| item.def.id == id)
    };
    snapshot
        .stats()
        .iter()
        .any(|stat| stat.index == 3 && stat.base == 99 && stat.effective >= 97)
        && snapshot
            .stats()
            .iter()
            .any(|stat| stat.index == 4 && stat.base >= 60)
        && held("rune_scimitar")
        && held("maple_shortbow")
        && held("iron_arrow")
}

fn kit(worn: &[(u8, i32)]) -> Arc<CompiledKit> {
    Arc::new(CompiledKit {
        worn: Arc::from(worn),
        ..CompiledKit::default()
    })
}

#[test]
#[ignore = "requires LIVE=1 and a shared 289 engine (Engine C from aubaine)"]
fn live_pvp_switch_t2_t3_t4_in_a_pen() {
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
        .expect("resolve the engine profile")
        .prepare_template()
        .expect("prepare the live template");
    let names = play_bootstrap::mint_live_names(2);
    let credentials = mint_live_entries(&names);
    let (holder, fighter) = (names[0].clone(), names[1].clone());
    let screen = |name: &str| {
        client::util::JString::to_screen_name(name)
            .as_str()
            .to_owned()
    };
    let observed = [
        Arc::new(Mutex::new(Observed::default())),
        Arc::new(Mutex::new(Observed::default())),
    ];
    let drives = [
        Arc::new(Mutex::new(DuelDrive::default())),
        Arc::new(Mutex::new(DuelDrive::default())),
    ];
    let shared = Arc::new(Mutex::new(Shared::default()));
    let partner_state = Arc::new(Mutex::new(PartnerState::default()));
    let (protect_varp, protect_button) = pvp_prayer("Protect from Melee");
    let anchor = WorldTile {
        x: crate::paired_core::DUEL_CHALLENGE_ANCHOR.0,
        z: crate::paired_core::DUEL_CHALLENGE_ANCHOR.1,
        level: 0,
    };
    let seats: std::collections::HashMap<String, (usize, Vec<String>, bool)> =
        [(holder.clone(), 0usize), (fighter.clone(), 1usize)]
            .into_iter()
            .map(|(name, index)| {
                (
                    name,
                    (
                        index,
                        if index == 0 {
                            holder_seed()
                        } else {
                            fighter_seed()
                        },
                        index == 0,
                    ),
                )
            })
            .collect();
    let partners = [screen(&fighter), screen(&holder)];
    let hook_observed = [Arc::clone(&observed[0]), Arc::clone(&observed[1])];
    let hook_drives = [Arc::clone(&drives[0]), Arc::clone(&drives[1])];
    let hook_shared = Arc::clone(&shared);
    let hook_partner = Arc::clone(&partner_state);
    let hook_walk_set = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut play = run_with_template(
        template,
        false,
        vec![],
        |_| (None, None),
        move |client, username, frame_input| {
            if frame_input.hold {
                return;
            }
            let Some((index, cheats, is_holder)) = seats.get(username) else {
                return;
            };
            let mut state = hook_observed[*index].lock().expect("observed lock");
            let ready: &(dyn Fn(&api::snapshot::GameSnapshot) -> bool + Send + Sync) = if *is_holder
            {
                &holder_ready
            } else {
                &fighter_ready
            };
            frame(client, &mut state, cheats, &[protect_varp], ready, anchor);
            if state.ready {
                duel_step(
                    client,
                    &mut hook_drives[*index].lock().expect("duel drive lock"),
                    &partners[*index],
                );
                if !*is_holder
                    && client.ingame
                    && client.scene_state == 2
                    && !hook_walk_set.swap(true, std::sync::atomic::Ordering::Relaxed)
                {
                    // The machine's account walks, so a runner can leave its reach.
                    let mut snapshot = api::snapshot::GameSnapshot::new();
                    snapshot.rebuild(client);
                    let _ = api::interact::Interactions::new(&snapshot, client).set_run(false);
                }
                if *is_holder {
                    partner_step(
                        client,
                        &hook_shared,
                        &mut hook_partner.lock().expect("partner lock"),
                        protect_varp,
                        protect_button,
                    );
                }
            }
        },
    )
    .expect("start live Play");
    for (index, (username, password)) in credentials.iter().enumerate() {
        play.try_spawn_slot(
            Profile {
                username: username.clone(),
                password: password.clone().into(),
                uid: 274_279_501 + index as i32,
                settings: ProfileSettings::default(),
            },
            None,
            None,
            None,
        )
        .expect("spawn fresh pvp account");
    }
    let deadline = started + CELL_DEADLINE;
    let states = || {
        json!({
            "holder": observed[0].lock().expect("observed lock").clone(),
            "fighter": observed[1].lock().expect("observed lock").clone(),
            "drives": drives
                .iter()
                .map(|drive| drive.lock().expect("duel drive lock").clone())
                .collect::<Vec<_>>(),
        })
    };
    let mut announced = Instant::now();
    loop {
        let ready = observed.iter().all(|observed| {
            let state = observed.lock().expect("observed lock");
            assert!(state.error.is_none(), "fixture error: {:?}", state.error);
            state.ready
        });
        if ready {
            break;
        }
        if announced.elapsed() > Duration::from_secs(20) {
            announced = Instant::now();
            let both = states();
            println!(
                "fixture: holder step {} ready {}, fighter step {} ready {}",
                both["holder"]["step"],
                both["holder"]["ready"],
                both["fighter"]["step"],
                both["fighter"]["ready"]
            );
            for status in play.statuses().iter() {
                println!(
                    "  slot {}: phase {:?} connected {} ingame {} scene {} error {:?}",
                    status.username,
                    status.startup_phase,
                    status.connected,
                    status.ingame,
                    status.scene_state,
                    status.error
                );
                assert!(
                    status.error.is_none(),
                    "slot {} cannot log in: {:?}",
                    status.username,
                    status.error
                );
            }
        }
        assert!(
            Instant::now() < deadline,
            "both accounts ready: cell deadline passed: {}",
            states()
        );
        thread::sleep(Duration::from_millis(100));
    }
    wait_until(
        "the duel reaches FIGHT!",
        Instant::now() + Duration::from_secs(180),
        || {
            drives
                .iter()
                .all(|drive| drive.lock().expect("duel drive lock").fight_seen)
        },
    );
    // The machine's request: sword kit and bow kit, melee preferred.
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let tables = CombatTables::build(Arc::clone(&data)).expect("combat tables");
    let id = |alias: &str| data.item_by_alias(alias).unwrap().id;
    let request = CombatRequest {
        target: Target::Player {
            name: Arc::from(screen(&holder).as_str()),
        },
        pvp: true,
        fallback: Fallback::Fight,
        lost_radius: 32,
        style: Style::Melee,
        kits: StyleKits::default()
            .with(Style::Melee, kit(&[(3, id("rune_scimitar"))]))
            .with(
                Style::Ranged,
                kit(&[(3, id("maple_shortbow")), (13, id("iron_arrow"))]),
            ),
        switch: SwitchPolicy::for_style(Style::Melee),
        budget_ticks: 1_500,
        ..CombatRequest::default()
    };
    {
        let mut shared = shared.lock().expect("shared lock");
        shared.advance(Phase::Hold, 0);
    }
    let handle = play.script_start_handle();
    match handle.start_test_script(
        &fighter,
        Box::new(SwitchFight {
            shared: Arc::clone(&shared),
            request: Arc::new(request),
            tables,
            handle: None,
            tried: false,
            partner: screen(&holder),
        }),
        None,
    ) {
        Ok(_) => play.wake(&fighter),
        Err(error) => panic!("start the fighter's native script: {error}"),
    }
    // Phases are measured in observed ticks against the first poll's tick.
    let limit = Instant::now() + Duration::from_millis((PHASE_TICKS * 3 + 60) * 600);
    loop {
        {
            let mut shared = shared.lock().expect("shared lock");
            if shared.phase == Phase::Hold && shared.phase_since == 0 {
                if let Some(begun) = shared.begun_tick {
                    shared.phase_since = begun;
                }
            }
            if shared.phase == Phase::Done
                || shared.begin_error.is_some()
                || shared.failure.is_some()
            {
                break;
            }
        }
        if Instant::now() >= limit {
            let shared = shared.lock().expect("shared lock");
            let dump = json!({
                "phase": shared.phase,
                "phase_since": shared.phase_since,
                "begun_tick": shared.begun_tick,
                "failure": shared.failure,
                "trace": shared.trace,
                "actions": shared.actions,
                "states": states(),
            });
            std::fs::write(
                evidence.join("pvp-switch-timeout.json"),
                serde_json::to_vec_pretty(&dump).expect("timeout json"),
            )
            .expect("write timeout dump");
            panic!(
                "the cell timed out: phase {:?}, begun tick {:?}",
                shared.phase, shared.begun_tick
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
    // Let the last frames land, then write the receipt.
    thread::sleep(Duration::from_secs(2));
    let shared = shared.lock().expect("shared lock");
    let refusals = partner_state.lock().expect("partner lock").refusals.clone();
    let rows = &shared.trace;
    let first = |predicate: &dyn Fn(&TraceRow) -> bool| rows.iter().find(|row| predicate(row));
    let t2 = first(&|row| row.phase == Phase::Hold && row.switches >= 1 && row.active == "Ranged");
    let t2_start = first(&|row| {
        row.phase == Phase::Hold
            && row.switch.starts_with("Converging")
            && row.switch.contains("Protect")
    });
    let t3 = first(&|row| row.phase == Phase::Drop && row.switches >= 2 && row.active == "Melee");
    let t3_start = first(&|row| {
        row.phase == Phase::Drop
            && row.switch.starts_with("Converging")
            && row.switch.contains("Prefer")
    });
    let t4 = first(&|row| row.phase == Phase::Away && row.switches >= 3 && row.active == "Ranged");
    let t4_start = first(&|row| {
        row.phase == Phase::Away
            && row.switch.starts_with("Converging")
            && row.switch.contains("Reach")
    });
    let bow = id("maple_shortbow");
    let sword = id("rune_scimitar");
    let arrows = id("iron_arrow");
    let worn = |row: &TraceRow, slot: i32, item: i32| row.worn.contains(&(slot, item));
    let verdict = json!({
        "t2_started_by_protect": t2_start.is_some(),
        "t2_completed_with_bow_and_quiver_worn": t2.is_some_and(|row| worn(row, 3, bow) && worn(row, 13, arrows)),
        "t2_partner_showed_protect_from_melee": t2_start.is_some_and(|row| row.partner_headicons & (1 << 3) != 0),
        "t3_started_by_prefer": t3_start.is_some(),
        "t3_completed_with_sword_worn": t3.is_some_and(|row| worn(row, 3, sword)),
        "t3_partner_showed_no_protect": t3_start.is_some_and(|row| row.partner_headicons & 0b11_1000 == 0),
        "t4_started_by_reach": t4_start.is_some(),
        // The machine counts eight ticks out of reach from its own frames, which
        // the cell cannot sample (its distance lags the machine's by a tick or two
        // while the runner moves): the cause `Reach` proves the count, and this
        // reads the cell's own series as corroboration, at least five of the ten
        // ticks up to the switch more than one tile apart.
        "t4_partner_out_of_reach": t4_start.is_some_and(|start| {
            shared
                .series
                .iter()
                .filter(|(at, distance)| {
                    *at + 9 >= start.tick && *at <= start.tick && *distance > 1
                })
                .count()
                >= 5
        }),
        "t4_completed_with_bow_worn": t4.is_some_and(|row| worn(row, 3, bow)),
        "switch_failures": rows.last().map_or(0, |row| row.failures),
    });
    let passed = verdict
        .as_object()
        .expect("verdict object")
        .iter()
        .all(|(key, value)| key == "switch_failures" || value == &Value::Bool(true))
        && verdict["switch_failures"] == 0
        && shared.failure.is_none()
        && shared.begin_error.is_none()
        && refusals.is_empty();
    let receipt = json!({
        "cell": "live_pvp_switch_t2_t3_t4_in_a_pen",
        "engine": std::env::var("PVP_ENGINE_LABEL").unwrap_or_else(|_| "127.0.0.1".into()),
        "game_port": std::env::var("BOT_GAME_PORT").ok(),
        "accounts": names,
        "holder": holder,
        "fighter": fighter,
        "begun_tick": shared.begun_tick,
        "begin_error": shared.begin_error,
        "failure": shared.failure,
        "partner_refusals": refusals,
        "trace": rows,
        "actions": shared.actions,
        "distance_series": shared.series,
        "ids": { "rune_scimitar": sword, "maple_shortbow": bow, "iron_arrow": arrows },
        "verdict": verdict,
        "passed": passed,
        "seconds": started.elapsed().as_secs(),
    });
    std::fs::write(
        evidence.join("pvp-switch-receipt.json"),
        serde_json::to_vec_pretty(&receipt).expect("receipt json"),
    )
    .expect("write receipt");
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt["verdict"]).unwrap()
    );
    assert!(
        passed,
        "the live smoke failed: see pvp-switch-receipt.json: {receipt}"
    );
}
