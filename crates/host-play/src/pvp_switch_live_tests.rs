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
//! The cell reads the machine's state (`ActionHandle::inspect`) and, apart from
//! it, the fighter's own frame before every poll: `Frame::borrow` of the snapshot
//! the poll receives (the local actor's tile, the holder's actor tile, the worn
//! items, the holder's head icons) and the fighter's own swing onsets from a
//! detector with its own stamps. The verdict ([`judge`]) is computed from those
//! samples and the state changes together; the machine's cause `Reach` is only
//! one of its clauses. T4 is rejected unless the holder stayed more than one
//! tile away for `2 * rate` consecutive observed ticks with no swing of ours
//! (design §4.3 T4, §10.1 case 7d; the rate is the rune scimitar's content
//! attack rate, so the window is 8 ticks) before any wear of the bow appears.
//! [`receipt_tests`] run the verdict on a correct trace and on each broken one.
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
use script::combat::frame::Frame;
use script::combat::style::melee;
use script::combat::{
    ActorKind, ActorRef, Combat, CombatRequest, CombatTables, CompiledKit, Fallback, Style,
    StyleKits, SwitchPolicy, SwitchState, Target,
};
use script::native::{ActionHandle, NativeTick, PvpGlobals, Script, ScriptFailure, ScriptFlow};
use serde_json::json;
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
    /// The fighter's frame at every observed tick, before that tick's poll.
    samples: Vec<Sample>,
    /// Observed ticks whose frame could not be borrowed (the machine stays
    /// pending on those too).
    missing_samples: u32,
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

/// One observed tick of the fighter's frame, taken before that tick's poll of
/// the machine from the same snapshot (`Frame::borrow`).
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct Sample {
    tick: u64,
    phase: Phase,
    /// `Frame::here`, the local actor's tile: where the machine measures from.
    own: (i32, i32, i32),
    /// The holder's actor tile.
    holder: Option<(i32, i32, i32)>,
    /// Chebyshev distance on one level; -1 without the holder's row or across levels.
    distance: i32,
    /// An onset of our own swing, from a detector with its own stamps.
    swing: bool,
    /// The holder's head icon bits (bit 3 melee, 4 ranged, 5 magic).
    headicons: i32,
    /// `(slot, obj id)` worn by the fighter.
    worn: Vec<(i32, i32)>,
}

/// Protect from Melee, and any protection prayer, in the head icon bits.
const MELEE_PROTECT: i32 = 1 << 3;
const ANY_PROTECT: i32 = 0b11_1000;

#[derive(Clone, Copy)]
struct Ids {
    sword: i32,
    bow: i32,
    arrows: i32,
}

/// The clauses of the smoke, each computed from the samples and the state
/// changes. [`Verdict::passed`] is their conjunction.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
struct Verdict {
    t2_started_by_protect: bool,
    /// The holder showed Protect from Melee on the frame of the start and on the one before.
    t2_holder_showed_protect_on_two_frames: bool,
    t2_no_bow_before_the_start: bool,
    t2_completed_with_bow_and_quiver_worn: bool,
    t3_started_by_prefer: bool,
    /// No protection on the frame of the start and the one before, and the holder within reach.
    t3_holder_showed_no_protect_and_stood_in_reach: bool,
    t3_completed_with_sword_worn: bool,
    t4_started_by_reach: bool,
    /// The tick at which `2 * rate` consecutive observed ticks with the holder more than
    /// one tile away and no swing of ours were complete, computed from the samples alone.
    t4_reach_window_ended_at: Option<u64>,
    t4_started_after_the_window: bool,
    t4_no_bow_before_the_window_ended: bool,
    t4_completed_with_bow_and_quiver_worn: bool,
    switches: u8,
    switch_failures: u8,
}

impl Verdict {
    fn passed(&self) -> bool {
        self.t2_started_by_protect
            && self.t2_holder_showed_protect_on_two_frames
            && self.t2_no_bow_before_the_start
            && self.t2_completed_with_bow_and_quiver_worn
            && self.t3_started_by_prefer
            && self.t3_holder_showed_no_protect_and_stood_in_reach
            && self.t3_completed_with_sword_worn
            && self.t4_started_by_reach
            && self.t4_reach_window_ended_at.is_some()
            && self.t4_started_after_the_window
            && self.t4_no_bow_before_the_window_ended
            && self.t4_completed_with_bow_and_quiver_worn
            && self.switches >= 3
            && self.switch_failures == 0
    }
}

fn sample_at(samples: &[Sample], tick: u64) -> Option<&Sample> {
    samples.iter().find(|sample| sample.tick == tick)
}

/// Whether `holds` is true of the samples at `tick - 1` and at `tick`.
fn on_two_frames(samples: &[Sample], tick: u64, holds: impl Fn(&Sample) -> bool) -> bool {
    tick > 0
        && [tick - 1, tick]
            .iter()
            .all(|at| sample_at(samples, *at).is_some_and(&holds))
}

/// The tick at which `window` consecutive observed ticks of the Away phase have
/// the holder more than one tile away and no swing of ours; a tick missing from
/// the samples, a sample with the holder in reach or unseen, and a swing each
/// start the count again.
fn reach_window_end(samples: &[Sample], window: usize) -> Option<u64> {
    let (mut run, mut previous) = (0usize, None::<u64>);
    for sample in samples.iter().filter(|sample| sample.phase == Phase::Away) {
        if previous.is_some_and(|at| sample.tick != at + 1) {
            run = 0;
        }
        previous = Some(sample.tick);
        if sample.distance > 1 && !sample.swing {
            run += 1;
            if run >= window {
                return Some(sample.tick);
            }
        } else {
            run = 0;
        }
    }
    None
}

/// The verdict of one run: `rows` are the state changes of the machine,
/// `samples` the fighter's frames, `window` is `2 * rate` of the melee weapon.
fn judge(rows: &[TraceRow], samples: &[Sample], ids: Ids, window: usize) -> Verdict {
    let first = |predicate: &dyn Fn(&TraceRow) -> bool| rows.iter().find(|row| predicate(row));
    let converging = |row: &TraceRow, phase: Phase, cause: &str| {
        row.phase == phase && row.switch.starts_with("Converging") && row.switch.contains(cause)
    };
    let worn = |row: &TraceRow, slot: i32, item: i32| row.worn.contains(&(slot, item));
    let t2_start = first(&|row| converging(row, Phase::Hold, "Protect"));
    let t2 = first(&|row| row.phase == Phase::Hold && row.switches >= 1 && row.active == "Ranged");
    let t3_start = first(&|row| converging(row, Phase::Drop, "Prefer"));
    let t3 = first(&|row| row.phase == Phase::Drop && row.switches >= 2 && row.active == "Melee");
    let t4_start = first(&|row| converging(row, Phase::Away, "Reach"));
    let t4 = first(&|row| row.phase == Phase::Away && row.switches >= 3 && row.active == "Ranged");
    let window_end = reach_window_end(samples, window);
    let first_bow_in_away = samples
        .iter()
        .filter(|sample| sample.phase == Phase::Away)
        .find(|sample| sample.worn.contains(&(3, ids.bow)))
        .map(|sample| sample.tick);
    Verdict {
        t2_started_by_protect: t2_start.is_some(),
        t2_holder_showed_protect_on_two_frames: t2_start.is_some_and(|start| {
            on_two_frames(samples, start.tick, |sample| {
                sample.headicons & MELEE_PROTECT != 0
            })
        }),
        t2_no_bow_before_the_start: t2_start.is_some_and(|start| {
            !samples.iter().any(|sample| {
                sample.phase == Phase::Hold
                    && sample.tick <= start.tick
                    && sample.worn.contains(&(3, ids.bow))
            })
        }),
        t2_completed_with_bow_and_quiver_worn: t2
            .is_some_and(|row| worn(row, 3, ids.bow) && worn(row, 13, ids.arrows)),
        t3_started_by_prefer: t3_start.is_some(),
        t3_holder_showed_no_protect_and_stood_in_reach: t3_start.is_some_and(|start| {
            on_two_frames(samples, start.tick, |sample| {
                sample.headicons & ANY_PROTECT == 0
            }) && sample_at(samples, start.tick)
                .is_some_and(|sample| (0..=1).contains(&sample.distance))
        }),
        t3_completed_with_sword_worn: t3.is_some_and(|row| worn(row, 3, ids.sword)),
        t4_started_by_reach: t4_start.is_some(),
        t4_reach_window_ended_at: window_end,
        t4_started_after_the_window: window_end
            .zip(t4_start)
            .is_some_and(|(end, start)| start.tick >= end),
        t4_no_bow_before_the_window_ended: window_end
            .zip(first_bow_in_away)
            .is_some_and(|(end, bow)| bow > end),
        t4_completed_with_bow_and_quiver_worn: t4
            .is_some_and(|row| worn(row, 3, ids.bow) && worn(row, 13, ids.arrows)),
        switches: rows.last().map_or(0, |row| row.switches),
        switch_failures: rows.last().map_or(0, |row| row.failures),
    }
}

/// The machine's native script: begins one `Combat` against the partner,
/// records the fighter's frame before every poll and samples the machine after it.
struct SwitchFight {
    shared: Arc<Mutex<Shared>>,
    request: Arc<CombatRequest>,
    tables: Arc<CombatTables>,
    handle: Option<ActionHandle<Combat>>,
    tried: bool,
    /// The holder's screen name: other accounts may share the arena.
    partner: String,
    /// The stamps of the cell's own swing-onset detector.
    swing_id: i32,
    swing_frame: i32,
}

impl SwitchFight {
    /// The fighter's frame as the poll below reads it (`Frame::borrow` of the
    /// same snapshot): one sample per observed tick.
    fn observe(&mut self, tick: &NativeTick<'_>, now: u64) -> Option<Sample> {
        let Some(frame) = Frame::borrow(tick.cx.snapshot()) else {
            let mut shared = self.shared.lock().expect("shared lock");
            shared.missing_samples += 1;
            return None;
        };
        let mut shared = self.shared.lock().expect("shared lock");
        if let Some(last) = shared.samples.last().filter(|last| last.tick == now) {
            return Some(last.clone());
        }
        let holder = frame
            .players
            .iter()
            .find(|row| row.actor.name.as_deref() == Some(self.partner.as_str()));
        let engaged = holder.map(|row| ActorRef {
            kind: ActorKind::Player,
            index: row.index as u16,
        });
        let swing = melee::onset(
            &frame.local.player.actor,
            engaged,
            &self.tables,
            &mut self.swing_id,
            &mut self.swing_frame,
        );
        let tile = |tile: &api::snapshot::WorldTile| (tile.x, tile.z, tile.level);
        let distance = holder.map_or(-1, |row| {
            let (a, b) = (frame.here, row.network);
            if a.level == b.level {
                (a.x - b.x).abs().max((a.z - b.z).abs())
            } else {
                -1
            }
        });
        let sample = Sample {
            tick: now,
            phase: shared.phase,
            own: tile(&frame.here),
            holder: holder.map(|row| tile(&row.network)),
            distance,
            swing,
            headicons: holder.map_or(0, |row| row.headicons),
            worn: frame
                .equipment
                .iter()
                .map(|item| (item.slot, item.def.id))
                .collect(),
        };
        shared.samples.push(sample.clone());
        Some(sample)
    }
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
        let sample = if self.handle.is_some() {
            self.observe(tick, now)
        } else {
            None
        };
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
        if shared.last != Some(state) {
            let row = TraceRow {
                tick: now,
                phase: shared.phase,
                active: format!("{:?}", state.0),
                switch: format!("{:?}", state.1),
                switches: state.2,
                failures: state.3,
                refusals: (state.4, state.5),
                worn: sample
                    .as_ref()
                    .map(|sample| sample.worn.clone())
                    .unwrap_or_default(),
                partner_headicons: sample.as_ref().map_or(0, |sample| sample.headicons),
                distance: sample.as_ref().map_or(-1, |sample| sample.distance),
                own_tile: sample.as_ref().map(|sample| sample.own),
                partner_tile: sample.as_ref().and_then(|sample| sample.holder),
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
    // The fixture relogs scriptless bots before any script starts, so each
    // account owns the arm that posts the operator Login after its own logout.
    let arms = [
        SlotArm::new(274_279_501, true),
        SlotArm::new(274_279_502, true),
    ];
    let hook_arms = [Arc::clone(&arms[0]), Arc::clone(&arms[1])];
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
            frame(
                client,
                &hook_arms[*index],
                &mut state,
                cheats,
                &[protect_varp],
                ready,
                anchor,
            );
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
            Some(Arc::clone(&arms[index])),
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
            swing_id: -1,
            swing_frame: -1,
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
    let ids = Ids {
        sword: id("rune_scimitar"),
        bow: id("maple_shortbow"),
        arrows: id("iron_arrow"),
    };
    // T4's window is `2 * rate` observed ticks (design §4.3): the rate is the
    // melee weapon's content attack rate, read from the selected game data.
    let rate = data
        .weapon_styles()
        .iter()
        .find(|row| row.obj_id == ids.sword)
        .map_or(0, |row| usize::from(row.attackrate));
    assert!(
        rate > 0,
        "the rune scimitar has an attack rate in the content"
    );
    let window = 2 * rate;
    let verdict = judge(rows, &shared.samples, ids, window);
    let passed = verdict.passed()
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
        "samples": shared.samples,
        "missing_samples": shared.missing_samples,
        "reach_window_ticks": window,
        "ids": { "rune_scimitar": ids.sword, "maple_shortbow": ids.bow, "iron_arrow": ids.arrows },
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

/// The verdict on recorded traces (PVP-DESIGN-1026 §10.6): a correct trace
/// passes and each named broken trace fails the clause it breaks.
#[cfg(test)]
mod receipt_tests {
    use super::*;
    use script::combat::SwitchCause;

    const IDS: Ids = Ids {
        sword: 1333,
        bow: 853,
        arrows: 884,
    };
    /// `2 * rate` of the rune scimitar (content attack rate 4).
    const WINDOW: usize = 8;
    const SWORD_ONLY: [(i32, i32); 1] = [(3, IDS.sword)];
    /// The sword with the quiver (the arrows stay worn after the return to melee).
    const SWORD: [(i32, i32); 2] = [(3, IDS.sword), (13, IDS.arrows)];
    const BOW: [(i32, i32); 2] = [(3, IDS.bow), (13, IDS.arrows)];

    fn converging(to: Style, cause: SwitchCause, since: u16) -> String {
        format!(
            "{:?}",
            SwitchState::Converging {
                to,
                cause,
                since,
                mask: 0,
                rate_old: 4
            }
        )
    }

    fn settling(since: u16) -> String {
        format!("{:?}", SwitchState::Settling { since, rate_old: 4 })
    }

    fn row(
        tick: u64,
        phase: Phase,
        active: &str,
        switch: String,
        switches: u8,
        worn: &[(i32, i32)],
    ) -> TraceRow {
        TraceRow {
            tick,
            phase,
            active: active.into(),
            switch,
            switches,
            worn: worn.to_vec(),
            ..TraceRow::default()
        }
    }

    fn sample(
        tick: u64,
        phase: Phase,
        distance: i32,
        headicons: i32,
        swing: bool,
        worn: &[(i32, i32)],
    ) -> Sample {
        Sample {
            tick,
            phase,
            own: (3340, 3250, 0),
            holder: Some((3340 + distance, 3250, 0)),
            distance,
            swing,
            headicons,
            worn: worn.to_vec(),
        }
    }

    /// A correct run: T2 starts at tick 23 (Protect from Melee on ticks 22 and
    /// 23) and completes at 25; T3 starts at 33 (nothing protected on 32 and 33,
    /// the holder adjacent) and completes at 35; the holder runs from tick 42,
    /// the eight ticks 42 to 49 are the T4 window, T4 starts at 49 and the bow
    /// is worn from tick 51.
    fn correct() -> (Vec<TraceRow>, Vec<Sample>) {
        let idle = format!("{:?}", SwitchState::Idle);
        let rows = vec![
            row(20, Phase::Hold, "Melee", idle.clone(), 0, &SWORD_ONLY),
            row(
                23,
                Phase::Hold,
                "Melee",
                converging(Style::Ranged, SwitchCause::Protect(MELEE_PROTECT as u8), 23),
                0,
                &SWORD_ONLY,
            ),
            row(25, Phase::Hold, "Ranged", settling(25), 1, &BOW),
            row(27, Phase::Hold, "Ranged", idle.clone(), 1, &BOW),
            row(
                33,
                Phase::Drop,
                "Ranged",
                converging(Style::Melee, SwitchCause::Prefer, 33),
                1,
                &BOW,
            ),
            row(35, Phase::Drop, "Melee", settling(35), 2, &SWORD),
            row(37, Phase::Drop, "Melee", idle, 2, &SWORD),
            row(
                49,
                Phase::Away,
                "Melee",
                converging(Style::Ranged, SwitchCause::Reach, 49),
                2,
                &SWORD,
            ),
            row(51, Phase::Away, "Ranged", settling(51), 3, &BOW),
        ];
        let samples = (20..=53u64)
            .map(|tick| {
                let phase = match tick {
                    ..=29 => Phase::Hold,
                    30..=39 => Phase::Drop,
                    _ => Phase::Away,
                };
                let headicons = if (22..=30).contains(&tick) {
                    MELEE_PROTECT
                } else {
                    0
                };
                let distance = if tick >= 42 { 3 } else { 1 };
                let worn: &[(i32, i32)] = match tick {
                    ..=24 => &SWORD_ONLY,
                    25..=34 => &BOW,
                    35..=50 => &SWORD,
                    _ => &BOW,
                };
                sample(
                    tick,
                    phase,
                    distance,
                    headicons,
                    matches!(tick, 21 | 36 | 41),
                    worn,
                )
            })
            .collect();
        (rows, samples)
    }

    fn verdict(rows: &[TraceRow], samples: &[Sample]) -> Verdict {
        judge(rows, samples, IDS, WINDOW)
    }

    fn sample_mut(samples: &mut [Sample], tick: u64) -> &mut Sample {
        samples
            .iter_mut()
            .find(|sample| sample.tick == tick)
            .expect("sample")
    }

    #[test]
    fn a_correct_trace_passes() {
        let (rows, samples) = correct();
        let verdict = verdict(&rows, &samples);
        assert!(verdict.passed(), "{verdict:?}");
        assert_eq!(verdict.t4_reach_window_ended_at, Some(49));
    }

    #[test]
    fn the_window_is_two_attack_intervals_of_consecutive_ticks() {
        let (_, samples) = correct();
        assert_eq!(reach_window_end(&samples, 8), Some(49));
        assert_eq!(reach_window_end(&samples, 9), Some(50));
        assert_eq!(reach_window_end(&samples, 12), Some(53));
        assert_eq!(reach_window_end(&samples, 13), None);
    }

    #[test]
    fn no_t2_switch_fails() {
        let (mut rows, samples) = correct();
        rows.retain(|row| !(23..=27).contains(&row.tick));
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t2_started_by_protect);
        assert!(!verdict.t2_completed_with_bow_and_quiver_worn);
        assert!(!verdict.passed());
    }

    #[test]
    fn no_t4_switch_fails() {
        let (mut rows, samples) = correct();
        rows.retain(|row| row.phase != Phase::Away || row.tick < 49);
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t4_started_by_reach);
        assert!(!verdict.t4_completed_with_bow_and_quiver_worn);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_t4_switch_by_another_cause_fails() {
        let (mut rows, samples) = correct();
        let at = rows.iter().position(|row| row.tick == 49).expect("row");
        rows[at].switch = converging(Style::Ranged, SwitchCause::Exhausted, 49);
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t4_started_by_reach);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_switch_that_wears_nothing_fails() {
        // The counters and causes are right, but no bow is ever worn.
        let (mut rows, mut samples) = correct();
        for row in &mut rows {
            row.worn = SWORD_ONLY.to_vec();
        }
        for sample in &mut samples {
            sample.worn = SWORD_ONLY.to_vec();
        }
        let verdict = verdict(&rows, &samples);
        assert!(verdict.t2_started_by_protect && verdict.t4_started_by_reach);
        assert!(!verdict.t2_completed_with_bow_and_quiver_worn);
        assert!(!verdict.t4_completed_with_bow_and_quiver_worn);
        assert!(!verdict.t4_no_bow_before_the_window_ended);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_t4_completion_without_the_quiver_fails() {
        let (mut rows, samples) = correct();
        let at = rows.iter().position(|row| row.tick == 51).expect("row");
        rows[at].worn = vec![(3, IDS.bow)];
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t4_completed_with_bow_and_quiver_worn);
        assert!(!verdict.passed());
    }

    #[test]
    fn an_early_t4_switch_fails() {
        // The machine starts after four distant ticks and the bow is worn at 47,
        // two ticks before the eight-tick window is complete.
        let (mut rows, mut samples) = correct();
        for row in &mut rows {
            match row.tick {
                49 => {
                    row.tick = 45;
                    row.switch = converging(Style::Ranged, SwitchCause::Reach, 45);
                }
                51 => row.tick = 47,
                _ => {}
            }
        }
        for sample in &mut samples {
            if sample.tick >= 47 {
                sample.worn = BOW.to_vec();
            }
        }
        let verdict = verdict(&rows, &samples);
        assert!(verdict.t4_started_by_reach);
        assert_eq!(verdict.t4_reach_window_ended_at, Some(49));
        assert!(!verdict.t4_started_after_the_window);
        assert!(!verdict.t4_no_bow_before_the_window_ended);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_wear_before_the_window_ends_fails_even_when_the_start_is_late() {
        let (rows, mut samples) = correct();
        for sample in &mut samples {
            if sample.tick >= 46 {
                sample.worn = BOW.to_vec();
            }
        }
        let verdict = verdict(&rows, &samples);
        assert!(verdict.t4_started_after_the_window);
        assert!(!verdict.t4_no_bow_before_the_window_ended);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_nonconsecutive_distance_series_fails() {
        // One tick inside the window with the holder back in reach: the machine
        // switched at 49 all the same.
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 45).distance = 1;
        let verdict = verdict(&rows, &samples);
        assert!(verdict.t4_started_by_reach);
        assert!(verdict.t4_reach_window_ended_at != Some(49));
        assert!(!verdict.t4_started_after_the_window);
        assert!(!verdict.passed());
    }

    #[test]
    fn an_intervening_swing_of_ours_fails() {
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 46).swing = true;
        let verdict = verdict(&rows, &samples);
        assert!(verdict.t4_reach_window_ended_at != Some(49));
        assert!(!verdict.t4_started_after_the_window);
        assert!(!verdict.passed());
    }

    /// The machine's T4 start at `start`, the bow worn from `start + 2`.
    fn t4_at(rows: &mut [TraceRow], samples: &mut [Sample], start: u64) {
        for row in rows.iter_mut() {
            match row.tick {
                49 => {
                    row.tick = start;
                    row.switch = converging(Style::Ranged, SwitchCause::Reach, start as u16);
                }
                51 => row.tick = start + 2,
                _ => {}
            }
        }
        for sample in samples.iter_mut() {
            sample.worn = if sample.tick >= start + 2 {
                BOW.to_vec()
            } else if sample.tick >= 35 {
                SWORD.to_vec()
            } else {
                sample.worn.clone()
            };
        }
    }

    #[test]
    fn a_missing_tick_breaks_the_run() {
        // Tick 44 was never observed: the ticks around it are not consecutive, so
        // the window ends at 52 and a machine that started at 50 counted too early.
        let (mut rows, mut samples) = correct();
        samples.retain(|sample| sample.tick != 44);
        t4_at(&mut rows, &mut samples, 50);
        let verdict = verdict(&rows, &samples);
        assert_eq!(verdict.t4_reach_window_ended_at, Some(52));
        assert!(!verdict.t4_started_after_the_window);
        assert!(!verdict.passed());
    }

    #[test]
    fn an_unseen_holder_breaks_the_run() {
        let (mut rows, mut samples) = correct();
        sample_mut(&mut samples, 44).distance = -1;
        t4_at(&mut rows, &mut samples, 50);
        let verdict = verdict(&rows, &samples);
        assert_eq!(verdict.t4_reach_window_ended_at, Some(52));
        assert!(!verdict.t4_started_after_the_window);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_start_on_the_last_tick_of_the_window_passes() {
        // The bound is tight, not loose: a start at the window's end is correct.
        let (mut rows, mut samples) = correct();
        t4_at(&mut rows, &mut samples, 49);
        let verdict = verdict(&rows, &samples);
        assert!(verdict.passed(), "{verdict:?}");
    }

    #[test]
    fn a_t2_start_on_one_protected_frame_fails() {
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 22).headicons = 0;
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t2_holder_showed_protect_on_two_frames);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_bow_worn_before_the_t2_start_fails() {
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 22).worn = BOW.to_vec();
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t2_no_bow_before_the_start);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_t3_return_while_the_holder_protects_or_stands_away_fails() {
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 32).headicons = 1 << 4;
        assert!(!verdict(&rows, &samples).t3_holder_showed_no_protect_and_stood_in_reach);
        let (rows, mut samples) = correct();
        sample_mut(&mut samples, 33).distance = 3;
        let verdict = verdict(&rows, &samples);
        assert!(!verdict.t3_holder_showed_no_protect_and_stood_in_reach);
        assert!(!verdict.passed());
    }

    #[test]
    fn a_failed_switch_fails() {
        let (mut rows, samples) = correct();
        rows.last_mut().expect("row").failures = 1;
        let verdict = verdict(&rows, &samples);
        assert_eq!(verdict.switch_failures, 1);
        assert!(!verdict.passed());
    }

    #[test]
    fn fewer_than_three_switches_fail() {
        let (mut rows, samples) = correct();
        rows.last_mut().expect("row").switches = 2;
        assert!(!verdict(&rows, &samples).passed());
    }
}
