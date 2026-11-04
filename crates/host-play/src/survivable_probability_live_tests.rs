//! Real Engine-L admission and poison-hold cells. Only account state is seeded.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::condition::PoisonState;
use api::interact::{self, Interactions};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::client::SessionExitReason;
use client::client::Client;
use nav_planner::router::{FindOptions, Leg, Route};
use nav_planner::world::NavWorld;
use nav_planner::zones::ZoneExempt;
use nav_planner::WorldState;
use script::combat::risk::{PoisonTest, RiskInput, RiskTables, Verdict};
use script::native::{RiskPolicy, WalkRefusal};
use serde_json::{json, Value};

use super::{assess, capture, condition_tables, permits, Admission};
use crate::script_runtime::NavBot;
use crate::{ProfileOptions, SharedClientTemplate, SlotStatus};

#[derive(Default)]
struct Logs(Mutex<Vec<Value>>);
static LOGS: Logs = Logs(Mutex::new(Vec::new()));
static INSTALL: std::sync::Once = std::sync::Once::new();
impl api::hostlog::Sink for Logs {
    fn record(&self, record: &api::hostlog::Record<'_>) {
        if matches!(record.source, api::hostlog::Source::Nav) {
            self.0.lock().unwrap().push(json!({
                "slot": record.slot, "tick": record.tick, "message": record.message,
            }));
        }
    }
}

fn tile(x: i32, z: i32) -> WorldTile {
    WorldTile { x, z, level: 0 }
}

struct Rig {
    client: Client,
    snapshot: GameSnapshot,
    pump: host::Pump,
    world: Arc<NavWorld>,
    navs: Arc<Mutex<HashMap<String, NavBot>>>,
    statuses: Arc<Mutex<Vec<SlotStatus>>>,
    name: String,
    members: bool,
    was_ingame: bool,
    frames: Vec<Value>,
    last_recorded: u32,
    setup_path: PathBuf,
    setup_events: Vec<Value>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    walk_attempts: Vec<Value>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    walk_attempt_capture_started_tick: Option<u32>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    walk_attempt_capture_ended_tick: Option<u32>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    diagnostic_retreat_index: Option<u16>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    walk_attempt_capture_first_clear_tick: Option<u32>,
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    walk_attempt_capture_retreat_tick: Option<u32>,
}

#[cfg(all(test, feature = "test-support", feature = "live-probe"))]
struct WalkAttemptCaptureGuard;

#[cfg(all(test, feature = "test-support", feature = "live-probe"))]
impl Drop for WalkAttemptCaptureGuard {
    fn drop(&mut self) {
        crate::script_runtime::NavBot::stop_walk_attempt_capture();
    }
}

#[cfg(all(test, feature = "test-support", feature = "live-probe"))]
impl WalkAttemptCaptureGuard {
    fn start() -> Self {
        crate::script_runtime::NavBot::start_walk_attempt_capture();
        Self
    }
}
impl Drop for Rig {
    fn drop(&mut self) {
        self.client.logout();
    }
}
impl Rig {
    fn frame(&mut self, follow: bool) {
        self.client.mainloop();
        let drain = self.pump.drain_client(&self.client);
        host::publish_snapshot(&mut self.snapshot, &self.client, drain);
        let ingame = self.snapshot.ingame();
        {
            let mut navs = self.navs.lock().unwrap();
            let bot = navs.get_mut(&self.name).unwrap();
            if ingame && !self.was_ingame {
                bot.condition.session_started(
                    self.snapshot.tick(),
                    self.client.chat_seq as i32,
                    condition_tables(&self.world),
                );
            }
            bot.condition.observe_frame(&self.snapshot, &drain, false);
        }
        self.was_ingame = ingame;
        api::hostlog::set_tick(self.snapshot.tick());
        let checkpoint = self.navs.lock().unwrap()[&self.name]
            .condition
            .dispatch_checkpoint(&self.client);
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        let mut tick_walk_attempts = Vec::new();
        if follow && drain.player_info {
            crate::script_runtime::step_nav_bot(
                &mut self.client,
                &self.name,
                self.snapshot.tile(),
                &self.snapshot,
                &self.navs,
                &self.statuses,
                Some(&self.world),
                false,
                self.members,
                None,
                || Arc::new(api::query::ReachQueryView::unavailable()),
            );
            #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
            {
                tick_walk_attempts = crate::script_runtime::NavBot::drain_walk_attempt_capture();
                self.walk_attempts
                    .extend(tick_walk_attempts.iter().cloned());
            }
        }
        if let Some(checkpoint) = checkpoint {
            self.navs
                .lock()
                .unwrap()
                .get_mut(&self.name)
                .unwrap()
                .condition
                .observe_dispatch(&self.client, checkpoint);
        }
        if self.snapshot.tick() != self.last_recorded {
            self.last_recorded = self.snapshot.tick();
            let navs = self.navs.lock().unwrap();
            let bot = &navs[&self.name];
            let fact = bot.condition.published();
            let hold = bot.poison_hold.as_ref().map(|hold| {
                json!({
                    "until_index": hold.until_index,
                    "stand_sent": hold.stand_sent,
                    "wait": hold.wait.map(|wait| json!({
                        "started": wait.started, "deadline": wait.deadline,
                        "remaining_at_start": wait.deadline.wrapping_sub(wait.started),
                        "waited": (self.snapshot.tick() as u16).wrapping_sub(wait.started),
                    })),
                })
            });
            let hits = self.client.local_player.as_ref().map(|player| {
                json!({
                    "values": player.entity.damage_values,
                    "types": player.entity.damage_types,
                    "expires_client_cycle": player.entity.damage_cycles,
                })
            });
            let frame = json!({
                "tick": self.snapshot.tick(), "tile": self.snapshot.tile(),
                "hp": fact.hp, "poison": format!("{:?}", fact.poison),
                "poison_window": format!("{:?}", fact.poison_window),
                "hold": hold, "hitmarks": hits,
                "route_armed": bot.route.is_some(),
                "refusal": format!("{:?}", bot.risk_refusal),
            });
            #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
            let mut frame = frame;
            #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
            if crate::script_runtime::NavBot::walk_attempt_capture_active() {
                let route_path_len = bot.route_path_for_follow().and_then(|route| {
                    let view = route.view_in(&self.world)?;
                    script::combat::risk::RoutePath::new(view)
                        .ok()
                        .map(|path| path.len())
                });
                let motion = self.snapshot.local_player().map(|player| {
                    json!({
                        "moving": player.player.actor.moving,
                        "running": player.player.actor.running,
                    })
                });
                frame["route_execution"] = json!({
                    "route_armed": bot.has_route(),
                    "route_identity": {
                        "generation": bot.route_generation,
                        "request_id": bot.route_request_id,
                        "walk_request_id": bot.walk_request_id,
                        "map_route_generation": bot.map_route_generation,
                    },
                    "route_destination": bot.route_destination(),
                    "route_path_len": route_path_len,
                    "safe_prefix_retreat_index": self.diagnostic_retreat_index,
                    "active_follow_index_limit":
                        bot.poison_hold.as_ref().map(|hold| hold.until_index),
                    "player_tile": self.snapshot.tile(),
                    "sampled_player_tile_route_index": null,
                    "sampled_player_tile_index_semantics":
                        "not treated as an exact RoutePath point",
                    "movement": motion,
                    "map_flag": self.snapshot.map_flag(),
                    "walk_attempts": tick_walk_attempts,
                });
            }
            self.frames.push(frame);
            if ingame && fact.hp_max != 0 {
                assert_ne!(fact.hp, 0, "death at {:?}", self.snapshot.tile());
            }
        }
    }

    fn until(&mut self, budget: u32, follow: bool, ready: impl Fn(&Self) -> bool) {
        let start = self.snapshot.tick();
        let watchdog = Instant::now() + Duration::from_secs(90 + u64::from(budget));
        loop {
            self.frame(follow);
            if ready(self) {
                return;
            }
            assert!(
                self.snapshot.tick().saturating_sub(start) <= budget,
                "observed-tick budget {budget} at {:?}",
                self.snapshot.tile()
            );
            assert!(
                Instant::now() < watchdog,
                "no game progress at {:?}",
                self.snapshot.tile()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn send(&mut self, action: impl FnOnce(&GameSnapshot, &mut Client)) {
        let checkpoint = self.navs.lock().unwrap()[&self.name]
            .condition
            .dispatch_checkpoint(&self.client);
        action(&self.snapshot, &mut self.client);
        if let Some(checkpoint) = checkpoint {
            self.navs
                .lock()
                .unwrap()
                .get_mut(&self.name)
                .unwrap()
                .condition
                .observe_dispatch(&self.client, checkpoint);
        }
    }

    fn cheat(&mut self, command: &str) {
        self.send(|_, client| assert!(interact::cheat(client, command).is_sent(), "{command}"));
    }

    fn logout_clean(&mut self) {
        self.send(|_, client| {
            let ifaces = Arc::clone(&client.ifaces);
            assert!(
                interact::logout(client, &ifaces),
                "logout button unavailable"
            );
        });
        self.until(150, false, |rig| !rig.snapshot.ingame());
    }

    fn record_setup(&mut self, event: Value) {
        eprintln!("P1C_SETUP {event}");
        self.setup_events.push(event);
        let setup = json!({"account": self.name, "events": self.setup_events});
        std::fs::write(&self.setup_path, serde_json::to_vec_pretty(&setup).unwrap()).unwrap();
    }

    fn relog(&mut self) {
        let request_tick = self.snapshot.tick();
        self.logout_clean();
        let logout_tick = self.snapshot.tick();
        let exit = self.client.take_session_exit_reason();
        self.record_setup(json!({
            "phase": "logout_observed", "request_tick": request_tick,
            "tick": logout_tick, "exit": format!("{exit:?}"),
        }));
        assert!(
            matches!(exit, Some(SessionExitReason::ServerLogout { .. })),
            "relog requires a server logout acknowledgement, not a local disconnect"
        );
        // World.ts sends Logout before its asynchronous save completes. Reuse
        // nav_gated_entrances_live's fixture-only code-5 release policy.
        let started = Instant::now();
        let budget = Duration::from_secs(30);
        let cadence = Duration::from_secs(5);
        let mut attempt = 0;
        loop {
            attempt += 1;
            let result = self.client.login(&self.name, &self.name, false);
            self.record_setup(json!({
                "phase": "login_after_logout", "logout_tick": logout_tick,
                "attempt": attempt, "elapsed_ms": started.elapsed().as_millis(),
                "accepted": result.is_ok(),
                "response_code": result.as_ref().err().map(|error| error.code),
            }));
            match result {
                Ok(()) => break,
                Err(error) => {
                    assert_eq!(error.code, 5, "relog refused: {error:?}");
                    assert!(
                        started.elapsed() + cadence < budget,
                        "account ownership was not released within {budget:?}"
                    );
                    let retry_at = Instant::now() + cadence;
                    self.until(150, false, |_| Instant::now() >= retry_at);
                }
            }
        }
        self.until(150, false, |rig| {
            rig.snapshot.ingame()
                && rig.snapshot.attached()
                && rig.snapshot.scene_state() == 2
                && rig
                    .snapshot
                    .side_tabs()
                    .iter()
                    .any(|tab| tab.index == 3 && tab.available)
                && rig.navs.lock().unwrap()[&rig.name]
                    .condition
                    .current()
                    .is_some()
        });
    }

    fn tele(&mut self, origin: WorldTile) {
        self.cheat(&interact::tele_args(origin.level, origin.x, origin.z));
        self.until(100, false, |rig| {
            rig.snapshot.scene_state() == 2
                && rig.snapshot.tile() == Some((origin.x, origin.z, origin.level))
        });
    }

    fn clear_from_observation(&mut self) {
        let (x, z, level) = self.snapshot.tile().unwrap();
        self.send(|snapshot, client| {
            assert!(matches!(
                Interactions::new(snapshot, client).walk(WorldTile { x, z, level }),
                interact::SendResult::Sent { .. }
            ));
        });
        self.until(40, false, |rig| {
            matches!(
                rig.navs.lock().unwrap()[&rig.name]
                    .condition
                    .current()
                    .map(|fact| fact.poison),
                Some(PoisonState::Clear)
            )
        });
    }
}

#[derive(Clone, Copy)]
enum Mode {
    Arrive,
    Refuse,
    Hold,
    LongApproach,
    PoisonDuringHold,
}
#[derive(Clone, Copy)]
struct Cell {
    name: &'static str,
    origin: WorldTile,
    destination: WorldTile,
    hp: u8,
    mode: Mode,
}

fn template() -> (Arc<crate::ServerProfile>, Arc<SharedClientTemplate>) {
    let required = |name| PathBuf::from(std::env::var_os(name).expect(name));
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(47594),
        http_port: Some(3080),
        engine_dir: Some(required("WORLD_ENGINE_DIR")),
        cache_dir: Some(required("BOT_CACHE_DIR")),
        unpack_dir: Some(required("BOT_UNPACK_DIR")),
        nav_pack: Some(required("WORLD_NAV_PACK")),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None).unwrap().bind_runtime().unwrap();
    let template = SharedClientTemplate::load(Arc::clone(&profile)).unwrap();
    (profile, template)
}

fn prepare(
    profile: &crate::ServerProfile,
    template: &SharedClientTemplate,
    name: &str,
    hp: u8,
    setup_path: PathBuf,
) -> Rig {
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    let mut client = template
        .prepare_client((serial % 1_000_000_000) as i32, true)
        .unwrap();
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading);
    assert!(interact::login(&mut client, name, name, false));
    let mut rig = Rig {
        client,
        snapshot: GameSnapshot::new(),
        pump: host::Pump::new(),
        world: template.world().unwrap(),
        navs: Arc::new(Mutex::new(HashMap::from([(
            name.into(),
            NavBot::default(),
        )]))),
        statuses: Arc::new(Mutex::new(Vec::new())),
        name: name.into(),
        members: profile.map_members(),
        was_ingame: false,
        frames: Vec::new(),
        last_recorded: 0,
        setup_path,
        setup_events: Vec::new(),
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        walk_attempts: Vec::new(),
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        walk_attempt_capture_started_tick: None,
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        walk_attempt_capture_ended_tick: None,
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        diagnostic_retreat_index: None,
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        walk_attempt_capture_first_clear_tick: None,
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        walk_attempt_capture_retreat_tick: None,
    };
    rig.until(150, false, |rig| {
        rig.snapshot.ingame() && rig.snapshot.attached() && rig.snapshot.scene_state() == 2
    });
    rig.send(|_, client| interact::mainland_hop(client));
    rig.cheat("getvar tutorial");
    rig.until(100, false, |rig| {
        rig.snapshot
            .chat_lines()
            .iter()
            .any(|line| line.text.contains("get tutorial: 1000"))
    });
    rig.relog();
    for (stat, value) in [
        ("attack", 40),
        ("strength", 40),
        ("defence", 40),
        ("hitpoints", i32::from(hp)),
        ("prayer", 1),
    ] {
        rig.cheat(&format!("setstat {stat} {value}"));
    }
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    for alias in [
        "rune_full_helm",
        "rune_chainbody",
        "rune_platelegs",
        "rune_kiteshield",
        "rune_scimitar",
    ] {
        let id = data.item_by_alias(alias).unwrap().id;
        if !rig
            .snapshot
            .equipment()
            .iter()
            .any(|item| item.def.id == id)
        {
            rig.cheat(&format!("give {alias} 1"));
            rig.until(30, false, |rig| {
                rig.snapshot
                    .inventory()
                    .iter()
                    .any(|item| item.def.id == id)
            });
            rig.send(|snapshot, client| {
                assert!(matches!(
                    Interactions::new(snapshot, client).wear(id),
                    interact::SendResult::Sent { .. }
                ))
            });
            rig.until(30, false, |rig| {
                rig.snapshot
                    .equipment()
                    .iter()
                    .any(|item| item.def.id == id)
            });
        }
    }
    rig.until(30, false, |rig| {
        let input = capture(
            &rig.snapshot,
            rig.members,
            rig.navs.lock().unwrap()[name].condition.current(),
            false,
        );
        input.hp == hp && input.hp_max == hp && input.defence == 40 && !input.missing_facts()
    });
    let input = capture(
        &rig.snapshot,
        rig.members,
        rig.navs.lock().unwrap()[name].condition.current(),
        false,
    );
    assert_eq!(input.worn_defence, [188, 202, 198, 190]);
    assert_eq!(
        input.food_len, 0,
        "no food or guard is credited in these cells"
    );
    rig
}

fn route(
    world: &NavWorld,
    snapshot: &GameSnapshot,
    members: bool,
    destination: WorldTile,
) -> Route {
    let (x, z, level) = snapshot.tile().unwrap();
    nav_planner::router::find_with(
        &world.collision,
        &world.graph,
        WorldTile { x, z, level },
        destination,
        FindOptions {
            zones: ZoneExempt::all(),
            ..FindOptions::default()
        },
        &WorldState::from_snapshot(snapshot).with_map_members(members),
    )
    .unwrap()
}

fn poison_start(
    world: &NavWorld,
    state: &WorldState,
    mut input: RiskInput,
    anchor: WorldTile,
    destination: WorldTile,
    long: bool,
    tables: (&script::combat::tables::CombatTables, &RiskTables),
) -> Result<(WorldTile, Value), String> {
    use nav_planner::producer;
    use script::combat::risk::{self, CrossingGeom, Estimate, RoutePath};

    let (combat, risks) = tables;
    let zones = producer::zones_view(world).unwrap().unwrap();
    let wilderness = producer::wilderness_view(world).unwrap();
    // The poison cells exercise a timing contract, not a particular NPC.
    // Derive their first crossing from any real route in this search region;
    // the separate rock-crab cell keeps its exact authored endpoints.
    let anchor_geometry: Vec<_> = zones
        .containing(anchor)
        .map(|zone| {
            json!({
                "zone": zones.name(zone.key()).unwrap(), "spawn": zone.spawn(),
                "bounds": zone.bounds(), "npc_id": zone.kind().npc_id(),
                "class": format!("{:?}", zone.class()), "cap": zone.cap(),
            })
        })
        .collect();
    input.poison = PoisonState::Unknown { since: input.tick };
    let options = FindOptions {
        zones: ZoneExempt::all(),
        ..FindOptions::default()
    };
    let mut searched = 0;
    let mut best_prefix = 0;
    let mut rejected = [0usize; 4];
    let mut sample_crossing = None;
    // A client can run two tiles per game tick. Eighty safe walk edges leave
    // at least forty observed ticks before retreat, even with running enabled.
    let minimum_prefix = if long { 80 } else { 1 };
    for radius in (4..=128).step_by(4) {
        for offset in (-radius..=radius).step_by(4) {
            for (side, origin) in [
                tile(anchor.x + offset, anchor.z - radius),
                tile(anchor.x + offset, anchor.z + radius),
                tile(anchor.x - radius, anchor.z + offset),
                tile(anchor.x + radius, anchor.z + offset),
            ]
            .into_iter()
            .enumerate()
            {
                if side >= 2 && offset.abs() == radius || !world.collision.standable(origin) {
                    continue;
                }
                searched += 1;
                let Ok(candidate) = nav_planner::router::find_with(
                    &world.collision,
                    &world.graph,
                    origin,
                    destination,
                    options,
                    state,
                ) else {
                    continue;
                };
                input.pos = origin;
                let view = producer::route_view(world, &candidate).unwrap();
                let path = RoutePath::new(view).unwrap();
                let Ok(plan) = risk::build_plan(path, zones, risks, wilderness, &input) else {
                    rejected[0] += 1;
                    continue;
                };
                let Some(crossing) = plan.crossings.first() else {
                    rejected[1] += 1;
                    continue;
                };
                let rows = &plan.intervals
                    [usize::from(crossing.intervals.0)..usize::from(crossing.intervals.1)];
                if !plan.complete {
                    rejected[2] += 1;
                    continue;
                }
                if sample_crossing.is_none() {
                    sample_crossing = Some(json!({
                        "start": [origin.x, origin.z, origin.level],
                        "retreat_index": crossing.retreat, "env_first": crossing.env_first,
                        "zones": rows.iter().map(|row| json!({
                            "name": zones.name(row.zone).unwrap(), "a": row.a, "e": row.e, "f": row.f,
                        })).collect::<Vec<_>>(),
                    }));
                }
                if crossing.retreat == CrossingGeom::NONE {
                    rejected[3] += 1;
                    continue;
                }
                best_prefix = best_prefix.max(crossing.retreat);
                if crossing.retreat < minimum_prefix {
                    continue;
                }
                let Some(Leg::Walk { tiles }) = candidate.legs.first() else {
                    continue;
                };
                if tiles.len() <= usize::from(crossing.retreat) {
                    continue;
                }
                let origin = if long {
                    origin
                } else {
                    path.point(crossing.retreat).unwrap().tile
                };
                // Replan a short start from the exact retreat. Reuse the
                // existing route when the long start has not changed.
                let candidate = if long {
                    candidate
                } else {
                    let Ok(route) = nav_planner::router::find_with(
                        &world.collision,
                        &world.graph,
                        origin,
                        destination,
                        options,
                        state,
                    ) else {
                        continue;
                    };
                    route
                };
                input.pos = origin;
                let view = producer::route_view(world, &candidate).unwrap();
                let path = RoutePath::new(view).unwrap();
                let assessment = risk::assess(
                    view,
                    &Estimate {
                        zones: Some(zones),
                        wilderness,
                        grants: None,
                        risks,
                        combat,
                        allow: Default::default(),
                        generation: 1,
                    },
                    input,
                );
                let PoisonTest::WaitForClear { until_index, .. } = assessment.poison_test else {
                    continue;
                };
                if assessment.verdict != Verdict::Survivable
                    || long && until_index < minimum_prefix
                    || !long && until_index != 0
                {
                    continue;
                }
                let crossing = &assessment.plan.crossings[0];
                let rows = &assessment.plan.intervals
                    [usize::from(crossing.intervals.0)..usize::from(crossing.intervals.1)];
                let retreat = path.point(until_index).unwrap().tile;
                let geometry: Vec<_> = rows
                    .iter()
                    .map(|row| {
                        let zone = zones.zone(row.zone).unwrap();
                        json!({
                            "zone": zones.name(row.zone).unwrap(), "spawn": zone.spawn(),
                            "bounds": zone.bounds(), "npc_id": zone.kind().npc_id(),
                            "r": row.r, "reach": row.reach, "a": row.a, "b": row.b,
                            "e": row.e, "f": row.f,
                        })
                    })
                    .collect();
                return Ok((
                    origin,
                    json!({
                        "anchor": [anchor.x, anchor.z, anchor.level],
                        "start": [origin.x, origin.z, origin.level],
                        "retreat": [retreat.x, retreat.z, retreat.level],
                        "retreat_index": until_index, "searched_standable_candidates": searched,
                        "first_crossing": {"first": crossing.first, "last": crossing.last,
                            "env_first": crossing.env_first, "env_last": crossing.env_last},
                        "zones": geometry,
                    }),
                ));
            }
        }
    }
    Err(format!(
        "no qualifying real start within 128 tiles of {anchor:?}: searched {searched} standable candidates, best safe prefix {best_prefix}, required {minimum_prefix}; rejections (plan error, no crossing, incomplete, no retreat)={rejected:?}; anchor geometry={anchor_geometry:?}; sample crossing={sample_crossing:?}"
    ))
}

fn captured_tile(value: &Value) -> Option<WorldTile> {
    let (x, z, level) = if let Some(values) = value.as_array() {
        let [x, z, level] = values.as_slice() else {
            return None;
        };
        (x, z, level)
    } else {
        (value.get("x")?, value.get("z")?, value.get("level")?)
    };
    Some(WorldTile {
        x: i32::try_from(x.as_i64()?).ok()?,
        z: i32::try_from(z.as_i64()?).ok()?,
        level: i32::try_from(level.as_i64()?).ok()?,
    })
}

/// The client can cut between authored points while following a valid aim.
/// Require an owned, acknowledged pre-retreat aim and observed progress toward
/// it instead of treating the player's tile as an exact RoutePath index.
/// Missing capture facts never fall back to authored-point membership.
fn clear_approach_aim(
    frames: &[Value],
    path: script::combat::risk::RoutePath<'_>,
    until_index: u16,
) -> Option<u16> {
    let clear_index = frames.iter().position(|frame| frame["poison"] == "Clear")?;
    let previous_index = clear_index.checked_sub(1)?;
    let current = frames[clear_index].get("route_execution")?;
    let previous = frames[previous_index].get("route_execution")?;
    let identity = current.get("route_identity")?;
    for field in [
        "generation",
        "request_id",
        "walk_request_id",
        "map_route_generation",
    ] {
        identity.get(field)?.as_u64()?;
    }
    let destination = path
        .point(u16::try_from(path.len().checked_sub(1)?).ok()?)?
        .tile;
    path.point(until_index)?;
    let (attempt_index, attempt) = (0..=clear_index).rev().find_map(|index| {
        frames[index]
            .get("route_execution")?
            .get("walk_attempts")?
            .as_array()?
            .last()
            .map(|attempt| (index, attempt))
    })?;
    let [index] = attempt.get("aim_route_indices")?.as_array()?.as_slice() else {
        return None;
    };
    let index = u16::try_from(index.as_u64()?).ok()?;
    let aim = captured_tile(attempt.get("aim")?)?;
    let at = captured_tile(attempt.get("at")?)?;
    if attempt.get("event")? != "WalkAttempt"
        || !attempt.get("refusal")?.is_null()
        || attempt.get("route_identity")? != identity
        || captured_tile(attempt.get("route_destination")?)? != destination
        || attempt.get("route_path_len")?.as_u64()? != path.len() as u64
        || attempt.get("route_index_limit")?.as_u64()? != u64::from(until_index)
        || attempt.get("aim_index_status")? != "unique"
        || index >= until_index
        || path.point(index)?.tile != aim
        || attempt.get("tick")?.as_u64()? != frames[attempt_index].get("tick")?.as_u64()?
        || frames[previous_index].get("tick")?.as_u64()? < attempt.get("tick")?.as_u64()?
        || frames[clear_index].get("tick")?.as_u64()?
            <= frames[previous_index].get("tick")?.as_u64()?
        || previous.get("active_follow_index_limit")?.as_u64()? != u64::from(until_index)
    {
        return None;
    }
    for frame in &frames[attempt_index..=clear_index] {
        let execution = frame.get("route_execution")?;
        if !frame.get("route_armed")?.as_bool()?
            || frame.get("refusal")? != "None"
            || !execution.get("route_armed")?.as_bool()?
            || execution.get("route_identity")? != identity
            || captured_tile(execution.get("route_destination")?)? != destination
            || execution.get("route_path_len")?.as_u64()? != path.len() as u64
            || execution.get("safe_prefix_retreat_index")?.as_u64()? != u64::from(until_index)
            || captured_tile(execution.get("player_tile")?)? != captured_tile(frame.get("tile")?)?
        {
            return None;
        }
        execution.get("walk_attempts")?.as_array()?;
    }
    let map_flag = current.get("map_flag")?;
    map_flag.get("lx")?.as_u64()?;
    map_flag.get("lz")?.as_u64()?;
    if frames[attempt_index]
        .get("route_execution")?
        .get("map_flag")?
        == map_flag
    {
        return None;
    }
    let acknowledged = (attempt_index + 1..clear_index)
        .find(|&index| frames[index]["route_execution"]["map_flag"] == *map_flag)?;
    if frames[acknowledged..=clear_index]
        .iter()
        .any(|frame| frame["route_execution"]["map_flag"] != *map_flag)
    {
        return None;
    }
    let before = captured_tile(previous.get("player_tile")?)?;
    let clear = captured_tile(current.get("player_tile")?)?;
    let toward_aim = |tile: WorldTile| tile.x.abs_diff(aim.x).max(tile.z.abs_diff(aim.z));
    if !previous.get("movement")?.get("moving")?.as_bool()?
        || !current.get("movement")?.get("moving")?.as_bool()?
        || toward_aim(clear) >= toward_aim(before)
        || [before, clear].iter().any(|tile| {
            tile.level != aim.level
                || tile.level != at.level
                || !(at.x.min(aim.x)..=at.x.max(aim.x)).contains(&tile.x)
                || !(at.z.min(aim.z)..=at.z.max(aim.z)).contains(&tile.z)
        })
    {
        return None;
    }
    Some(index)
}

mod clear_approach_tests {
    use super::*;

    fn fixture(aim_index: usize) -> (NavWorld, Route, Vec<Value>) {
        let world = NavWorld::from_parts(
            nav_planner::collision::WorldCollision {
                origin: tile(0, 0),
                width: 16,
                height: 16,
                walk: vec![0; 16 * 16],
                blocked: vec![0; (16_usize * 16).div_ceil(64)],
                flags: None,
            },
            Default::default(),
            Vec::new(),
        );
        let tiles = vec![
            tile(2, 0),
            tile(3, 1),
            tile(3, 2),
            tile(2, 3),
            tile(1, 4),
            tile(0, 5),
            tile(0, 6),
            tile(0, 7),
            tile(0, 8),
            tile(0, 9),
        ];
        let aim = tiles[aim_index];
        let destination = *tiles.last().unwrap();
        let route_len = tiles.len();
        let route = Route {
            legs: vec![Leg::Walk { tiles }],
            dest: destination,
            ticks: 9.0,
        };
        let identity = json!({"generation": 1, "request_id": 1, "walk_request_id": 1, "map_route_generation": 0});
        let attempt = json!({
            "event": "WalkAttempt", "tick": 17, "at": [2, 0, 0],
            "aim": [aim.x, aim.z, aim.level], "refusal": null,
            "route_identity": identity,
            "route_destination": [destination.x, destination.z, destination.level],
            "route_index_limit": 8, "route_path_len": route_len,
            "aim_route_indices": [aim_index], "aim_index_status": "unique",
        });
        let frames = [(17, [2, 0, 0]), (18, [2, 1, 0]), (31, [1, 2, 0]), (32, [1, 3, 0])]
            .into_iter()
            .map(|(tick, at)| json!({
                "tick": tick, "tile": at, "poison": if tick == 32 { "Clear" } else { "Unknown { since: 0 }" },
                "route_armed": true, "refusal": "None",
                "route_execution": {
                    "route_armed": true, "route_identity": identity,
                    "route_destination": destination, "route_path_len": route_len,
                    "safe_prefix_retreat_index": 8,
                    "active_follow_index_limit": if tick == 32 { None } else { Some(8) },
                    "player_tile": at, "movement": { "moving": true, "running": false },
                    "map_flag": if tick == 17 { json!({"lx": 8, "lz": 8}) } else { json!({"lx": aim.x, "lz": aim.z}) },
                    "walk_attempts": if tick == 17 { vec![attempt.clone()] } else { vec![] },
                },
            }))
            .collect();
        (world, route, frames)
    }

    fn proof(world: &NavWorld, route: &Route, frames: &[Value]) -> Option<u16> {
        clear_approach_aim(
            frames,
            script::combat::risk::RoutePath::new(
                nav_planner::producer::route_view(world, route).unwrap(),
            )
            .unwrap(),
            8,
        )
    }

    #[test]
    fn clear_progress_before_retreat_does_not_require_an_authored_player_tile() {
        let (world, route, frames) = fixture(5);
        let Leg::Walk { tiles } = &route.legs[0] else {
            unreachable!()
        };
        assert!(!tiles.contains(&tile(1, 3)));
        assert_eq!(proof(&world, &route, &frames), Some(5));
    }

    #[test]
    fn clear_progress_rejects_retreat_and_after_retreat_aims() {
        for index in [8, 9] {
            let (world, route, frames) = fixture(index);
            assert_eq!(proof(&world, &route, &frames), None);
        }
    }

    #[test]
    fn clear_progress_rejects_off_route_aims_and_movement() {
        let (world, route, mut frames) = fixture(5);
        frames[0]["route_execution"]["walk_attempts"][0]["aim"] = json!([8, 8, 0]);
        assert_eq!(proof(&world, &route, &frames), None);
        let (world, route, mut frames) = fixture(5);
        for (index, at) in [(2, [15, 2, 0]), (3, [14, 3, 0])] {
            frames[index]["tile"] = json!(at);
            frames[index]["route_execution"]["player_tile"] = json!(at);
        }
        assert_eq!(proof(&world, &route, &frames), None);
    }

    #[test]
    fn clear_progress_requires_correlated_ownership_acknowledgement_and_motion() {
        for field in ["route_identity", "map_flag", "movement"] {
            let (world, route, mut frames) = fixture(5);
            frames[3]["route_execution"]
                .as_object_mut()
                .unwrap()
                .remove(field);
            assert_eq!(proof(&world, &route, &frames), None, "missing {field}");
        }
        let (world, route, mut frames) = fixture(5);
        frames[3]["route_execution"]["route_identity"]["walk_request_id"] = json!(2);
        assert_eq!(proof(&world, &route, &frames), None);
        let (world, route, mut frames) = fixture(5);
        frames[3]["route_execution"]["movement"]["moving"] = json!(false);
        assert_eq!(proof(&world, &route, &frames), None);
        let (world, route, mut frames) = fixture(5);
        frames[3]["route_execution"]["map_flag"] = json!({"lx": 8, "lz": 8});
        assert_eq!(proof(&world, &route, &frames), None);
    }
}

fn exercise(rig: &mut Rig, mut cell: Cell, receipt: &mut Value) {
    let mut fixture_geometry = None;
    if matches!(
        cell.mode,
        Mode::Hold | Mode::LongApproach | Mode::PoisonDuringHold
    ) {
        // Reset the observed poison window while still at the safe setup tile.
        // A relog at the crab origin would be blocked by the combat logout guard.
        let input = capture(
            &rig.snapshot,
            rig.members,
            rig.navs.lock().unwrap()[&rig.name].condition.current(),
            false,
        );
        let tables = super::tables(&rig.world).unwrap();
        let (origin, geometry) = poison_start(
            &rig.world,
            &WorldState::from_snapshot(&rig.snapshot).with_map_members(rig.members),
            input,
            cell.origin,
            cell.destination,
            matches!(cell.mode, Mode::LongApproach),
            (&tables.combat, &tables.risks),
        )
        .unwrap();
        cell.origin = origin;
        fixture_geometry = Some(geometry);
        // Geometry search does not count as an observed safe approach.
        rig.relog();
    } else {
        rig.clear_from_observation();
    }
    rig.tele(cell.origin);
    let route = Arc::new(route(
        &rig.world,
        &rig.snapshot,
        rig.members,
        cell.destination,
    ));
    let input = capture(
        &rig.snapshot,
        rig.members,
        rig.navs.lock().unwrap()[&rig.name].condition.current(),
        false,
    );
    let admission = Admission {
        input,
        policy: RiskPolicy::Inherit,
        enforce: true,
        ..Admission::manual(
            FindOptions::default(),
            input,
            1,
            crate::WalkGlobals::default(),
        )
    };
    let assessment = assess(&route, &rig.world, &admission);
    super::log(Some(&rig.name), &assessment, false);
    *receipt = json!({"cell": cell.name, "account": rig.name, "engine": "127.0.0.1:47594",
        "origin": [cell.origin.x, cell.origin.z, cell.origin.level],
        "destination": [cell.destination.x, cell.destination.z, cell.destination.level],
        "seed_hp": cell.hp, "input": format!("{input:?}"),
        "verdict": format!("{:?}", assessment.verdict), "bound": assessment.bound,
        "poison_test": format!("{:?}", assessment.poison_test), "reason": assessment.reason,
        "route": format!("{route:?}"), "deaths": 0});
    receipt["fixture_geometry"] = json!(fixture_geometry);
    if matches!(cell.mode, Mode::Refuse) {
        assert_eq!(assessment.verdict, Verdict::Unsurvivable);
        assert!(!permits(&route, &rig.world, &admission, &assessment));
        assert!(
            (assessment.bound - 0.152).abs() < 0.05,
            "White Wolf Mountain bound drift"
        );
        receipt["route_armed"] = json!(false);
        receipt["refusal"] = json!(format!("{:?}", super::verdict_refusal(assessment.verdict)));
        return;
    }
    assert_eq!(
        assessment.verdict,
        Verdict::Survivable,
        "{}",
        assessment.reason
    );
    assert!(assessment.bound <= script::combat::risk::consts::DEATH_BOUND);
    assert!(permits(&route, &rig.world, &admission, &assessment));
    if matches!(
        cell.mode,
        Mode::Hold | Mode::LongApproach | Mode::PoisonDuringHold
    ) {
        assert!(matches!(input.poison, PoisonState::Unknown { .. }));
        assert!(matches!(
            assessment.poison_test,
            PoisonTest::WaitForClear { .. }
        ));
    }
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    let mut walk_attempt_capture =
        matches!(cell.mode, Mode::LongApproach).then(WalkAttemptCaptureGuard::start);
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    if walk_attempt_capture.is_some() {
        rig.walk_attempt_capture_started_tick = Some(rig.snapshot.tick());
        rig.diagnostic_retreat_index = fixture_geometry
            .as_ref()
            .and_then(|geometry| geometry["retreat_index"].as_u64())
            .and_then(|index| u16::try_from(index).ok());
    }
    {
        let mut navs = rig.navs.lock().unwrap();
        let bot = navs.get_mut(&rig.name).unwrap();
        bot.route = Some(Arc::clone(&route));
        bot.requested_route = Some((cell.destination, 0, false, false, false, ZoneExempt::NONE));
        bot.route_generation = 1;
        bot.route_request_id = 1;
        bot.walk_request_id = 1;
        bot.admission = Some(Box::new(admission));
        bot.assessment = Some(assessment);
        bot.admission_pending = true;
    }
    let mut seeded_poison = false;
    let start = rig.snapshot.tick();
    let accepted_frame_start = rig.frames.len();
    let watchdog = Instant::now() + Duration::from_secs(1200);
    loop {
        rig.frame(true);
        let hp = rig.navs.lock().unwrap()[&rig.name].condition.published().hp;
        if hp == 0 {
            receipt["deaths"] = json!(1);
            panic!("death observed during the accepted walk");
        }
        let (refusal, hold_wait, poison) = {
            let navs = rig.navs.lock().unwrap();
            let bot = &navs[&rig.name];
            (
                bot.risk_refusal
                    .as_deref()
                    .map(|(_, refusal)| refusal.clone()),
                bot.poison_hold.as_ref().and_then(|hold| hold.wait),
                bot.condition.published().poison,
            )
        };
        #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
        if walk_attempt_capture.is_some() {
            if rig.walk_attempt_capture_first_clear_tick.is_none()
                && matches!(poison, PoisonState::Clear)
            {
                rig.walk_attempt_capture_first_clear_tick = Some(rig.snapshot.tick());
            }
            if rig.walk_attempt_capture_retreat_tick.is_none()
                && rig.snapshot.tile().is_some_and(|(x, z, level)| {
                    fixture_geometry
                        .as_ref()
                        .is_some_and(|geometry| geometry["retreat"] == json!([x, z, level]))
                })
            {
                rig.walk_attempt_capture_retreat_tick = Some(rig.snapshot.tick());
            }
            if rig.walk_attempt_capture_first_clear_tick.is_some()
                && rig.walk_attempt_capture_retreat_tick.is_some()
            {
                drop(walk_attempt_capture.take());
                rig.walk_attempt_capture_ended_tick = Some(rig.snapshot.tick());
            }
        }
        if matches!(cell.mode, Mode::PoisonDuringHold) && !seeded_poison && hold_wait.is_some() {
            rig.cheat("~poison 30");
            seeded_poison = true;
            receipt["poison_seed_tick"] = json!(rig.snapshot.tick());
        }
        if let Some(refusal) = refusal {
            receipt["refusal_tick"] = json!(rig.snapshot.tick());
            receipt["refusal"] = json!(format!("{refusal:?}"));
            if matches!(cell.mode, Mode::PoisonDuringHold) {
                assert!(seeded_poison);
                assert_eq!(refusal, WalkRefusal::Unsurvivable);
                receipt["refusal_poison"] = json!(format!("{poison:?}"));
                if let PoisonState::Onset { since, .. } = poison {
                    receipt["poison_chat_tick"] = json!(since);
                    receipt["poison_chat_sequence"] = json!(rig
                        .snapshot
                        .chat_lines()
                        .iter()
                        .find(|line| line.text == "You have been poisoned!")
                        .map(|line| line.sequence));
                }
                // The content's onset line is already sufficient to refuse.
                // Observe the first mark only after the route is unarmed.
                assert!(rig.navs.lock().unwrap()[&rig.name].route.is_none());
                rig.until(40, false, |rig| {
                    matches!(
                        rig.navs.lock().unwrap()[&rig.name]
                            .condition
                            .published()
                            .poison,
                        PoisonState::Poisoned { .. }
                    )
                });
                let observed = rig.navs.lock().unwrap()[&rig.name]
                    .condition
                    .published()
                    .poison;
                receipt["first_poison_hitmark_tick"] = json!(rig.snapshot.tick());
                receipt["observed_poison"] = json!(format!("{observed:?}"));
                break;
            }
            panic!("unexpected live refusal {refusal:?}");
        }
        if rig.snapshot.tile()
            == Some((
                cell.destination.x,
                cell.destination.z,
                cell.destination.level,
            ))
        {
            assert!(
                !matches!(cell.mode, Mode::PoisonDuringHold),
                "poison probe arrived without refusing"
            );
            receipt["arrived"] = json!(true);
            break;
        }
        assert!(
            rig.snapshot.tick().saturating_sub(start) <= 1800,
            "walk exceeded observed-tick budget"
        );
        assert!(Instant::now() < watchdog, "walk watchdog expired");
        std::thread::sleep(Duration::from_millis(20));
    }
    let frames = &rig.frames[accepted_frame_start..];
    let waited: Vec<_> = frames
        .iter()
        .filter_map(|frame| frame["hold"]["wait"]["waited"].as_u64())
        .collect();
    if matches!(cell.mode, Mode::Hold) {
        assert!(!waited.is_empty(), "hold never stood at its retreat index");
        assert!(waited.iter().all(|&ticks| ticks <= 31));
        let retreat = &fixture_geometry.as_ref().unwrap()["retreat"];
        assert!(
            frames
                .iter()
                .take_while(|frame| frame["poison"] != "Clear")
                .all(|frame| frame["tile"] == *retreat),
            "the held walk must stay at its real retreat tile until Clear"
        );
        assert!(frames.iter().any(|frame| frame["poison"] == "Clear"));
    }
    if matches!(cell.mode, Mode::LongApproach) {
        assert!(
            waited.is_empty(),
            "clean approach stopped at the retreat tile"
        );
        let geometry = fixture_geometry.as_ref().unwrap();
        let until_index = geometry["retreat_index"].as_u64().unwrap() as u16;
        let first_clear = frames
            .iter()
            .find(|frame| frame["poison"] == "Clear")
            .expect("the observed approach must establish Clear");
        let path = script::combat::risk::RoutePath::new(
            nav_planner::producer::route_view(rig.world.as_ref(), &route).unwrap(),
        )
        .unwrap();
        let aim_index = clear_approach_aim(frames, path, until_index)
            .expect("Clear needs correlated progress toward an owned aim before retreat");
        receipt["clear_approach_aim_index"] = json!(aim_index);
        let PoisonState::Unknown { since } = input.poison else {
            unreachable!()
        };
        assert!((first_clear["tick"].as_u64().unwrap() as u16).wrapping_sub(since) >= 31);
        assert!(frames.iter().any(|frame| frame["poison"] == "Clear"));
    }
    receipt["waited_ticks"] = json!(waited);
    receipt["arrival_hp"] = json!(rig.navs.lock().unwrap()[&rig.name].condition.published().hp);
}

fn save(directory: &Path, rig: &Rig, receipt: &mut Value, log_start: usize) {
    receipt["frames"] = json!(rig.frames);
    receipt["nav_logs"] = json!(LOGS.0.lock().unwrap()[log_start..].to_vec());
    receipt["setup_events"] = json!(rig.setup_events);
    #[cfg(all(test, feature = "test-support", feature = "live-probe"))]
    if receipt["cell"] == "poison_long_approach" {
        receipt["walk_attempt_capture"] = json!({
            "started_tick": rig.walk_attempt_capture_started_tick,
            "ended_tick": rig.walk_attempt_capture_ended_tick,
            "safe_prefix_retreat_index": rig.diagnostic_retreat_index,
            "first_clear_tick": rig.walk_attempt_capture_first_clear_tick,
            "retreat_tile_tick": rig.walk_attempt_capture_retreat_tick,
            "ended_after_first_clear_and_retreat": rig.walk_attempt_capture_ended_tick.is_some(),
            "attempts": &rig.walk_attempts,
        });
    }
    std::fs::write(
        directory.join("receipt.json"),
        serde_json::to_vec_pretty(receipt).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "explicit real-pack poison-start qualification; requires WORLD_NAV_PACK"]
fn qualify_probability_poison_starts() {
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    let combat = script::combat::tables::CombatTables::build(Arc::clone(&selected)).unwrap();
    let risks = RiskTables::build(&combat);
    let world =
        NavWorld::load_pack(Path::new(&std::env::var_os("WORLD_NAV_PACK").unwrap())).unwrap();
    world.bind_named_bank_facts(&selected).unwrap();
    let state = WorldState::default().with_map_members(true);
    let mut input = RiskInput::default();
    input.hp = 40;
    input.hp_max = 40;
    input.defence = 40;
    input.worn_defence = [188, 202, 198, 190];
    input.combat = Some(46);
    input.set_map_members(true);
    input.set_missing_facts(false);
    input.set_unattributed(false);
    input.set_input_locked(false);
    input.set_overflow(false);
    let short = poison_start(
        &world,
        &state,
        input,
        tile(2704, 3726),
        tile(2725, 3492),
        false,
        (&combat, &risks),
    );
    eprintln!("POISON_GEOMETRY_SHORT={short:?}");
    let long = poison_start(
        &world,
        &state,
        input,
        tile(2704, 3726),
        tile(2725, 3492),
        true,
        (&combat, &risks),
    );
    eprintln!("POISON_GEOMETRY_LONG={long:?}");
    if let Some(path) = std::env::var_os("P1C_POISON_GEOMETRY_OUT") {
        std::fs::write(
            path,
            serde_json::to_vec_pretty(&json!({
                "short": short.as_ref().map(|(_, geometry)| geometry),
                "long": long.as_ref().map(|(_, geometry)| geometry),
            }))
            .unwrap(),
        )
        .unwrap();
    }
    short.unwrap();
    long.unwrap();
}

#[test]
#[ignore = "requires LIVE=1, Engine L 47594/3080, isolated cache/unpack, WORLD_NAV_PACK, owned BOT_LIVE_NAME_PREFIX, LIVE_EVIDENCE_DIR"]
fn probability_admission_engine_l() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("owned account prefix");
    assert!(!prefix.is_empty() && prefix.len() <= 4);
    INSTALL.call_once(|| assert!(api::hostlog::install_sink(&LOGS)));
    let directory = PathBuf::from(std::env::var_os("LIVE_EVIDENCE_DIR").unwrap());
    let (profile, template) = template();
    let account_file = directory.join("accounts.json");
    let names: Vec<String> = if account_file.exists() {
        serde_json::from_slice(&std::fs::read(&account_file).unwrap()).unwrap()
    } else {
        let names = crate::mint_live_names(2);
        std::fs::write(&account_file, serde_json::to_vec_pretty(&names).unwrap()).unwrap();
        names
    };
    assert_eq!(names.len(), 2);
    assert_ne!(names[0], names[1]);
    assert!(names
        .iter()
        .all(|name| name.starts_with(&prefix) && name.len() <= 12));
    let cells = [
        Cell {
            name: "rock_crabs_to_seers_bank",
            origin: tile(2704, 3726),
            destination: tile(2725, 3492),
            hp: 40,
            mode: Mode::Arrive,
        },
        Cell {
            name: "observatory_moss_giants",
            origin: tile(2438, 3185),
            destination: tile(2552, 3574),
            hp: 40,
            mode: Mode::Arrive,
        },
        Cell {
            name: "white_wolf_mountain_hp40",
            origin: tile(2893, 3429),
            destination: tile(2809, 3436),
            hp: 40,
            mode: Mode::Refuse,
        },
        Cell {
            name: "white_wolf_mountain_hp56",
            origin: tile(2893, 3429),
            destination: tile(2809, 3436),
            hp: 56,
            mode: Mode::Arrive,
        },
        Cell {
            name: "poison_wait_after_tele",
            origin: tile(2704, 3726),
            destination: tile(2725, 3492),
            hp: 40,
            mode: Mode::Hold,
        },
        Cell {
            name: "poison_long_approach",
            origin: tile(2704, 3726),
            destination: tile(2725, 3492),
            hp: 40,
            mode: Mode::LongApproach,
        },
        Cell {
            name: "poison_hit_during_hold",
            origin: tile(2704, 3726),
            destination: tile(2725, 3492),
            hp: 40,
            mode: Mode::PoisonDuringHold,
        },
    ];
    let selected = std::env::var("P1C_LIVE_CELLS").ok().map(|value| {
        let names: Vec<_> = value.split(',').map(str::to_owned).collect();
        assert!(!names.is_empty());
        for name in &names {
            assert!(
                cells.iter().any(|cell| cell.name == name),
                "unknown P1-C live cell: {name}"
            );
        }
        names
    });
    let mut failed = Vec::new();
    for cell in cells {
        if selected
            .as_ref()
            .is_some_and(|names| !names.iter().any(|name| name == cell.name))
        {
            continue;
        }
        let cell_dir = directory.join(cell.name);
        std::fs::create_dir_all(&cell_dir).unwrap();
        let log_start = LOGS.0.lock().unwrap().len();
        let name = &names[usize::from(matches!(cell.mode, Mode::PoisonDuringHold))];
        let mut rig = prepare(
            &profile,
            &template,
            name,
            cell.hp,
            cell_dir.join("setup.json"),
        );
        let mut receipt = json!({"cell":cell.name, "account":name});
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            exercise(&mut rig, cell, &mut receipt)
        }));
        receipt["passed"] = json!(result.is_ok());
        save(&cell_dir, &rig, &mut receipt, log_start);
        if result.is_err() {
            failed.push(cell.name);
        }
        // The acceptance receipt ends before cleanup. Keep our own account
        // clear of aggro, then let content's 16-tick combat logout guard expire.
        rig.tele(tile(3222, 3218));
        let cleanup_start = rig.snapshot.tick();
        rig.until(24, false, |rig| {
            rig.snapshot.tick().saturating_sub(cleanup_start) >= 17
        });
        rig.logout_clean();
    }
    assert!(failed.is_empty(), "failed P1-C live cells: {failed:?}");
}
