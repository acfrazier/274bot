use super::FeatureCase;
#[cfg(feature = "js")]
use super::Observation;

#[cfg(not(feature = "js"))]
pub(super) fn build(_: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    Err(
        "active_route_navigation requires the shipped js feature for its accepted control isolate"
            .into(),
    )
}

#[cfg(feature = "js")]
pub(super) fn build(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    Ok(Box::new(ActiveRoute {
        members: names.iter().map(|_| Member::default()).collect(),
        stopped: false,
    }))
}

#[cfg(feature = "js")]
use api::snapshot::WorldTile;
#[cfg(feature = "js")]
use std::sync::{Arc, Mutex};

// Real open stands from the Gatherer and v2 seat live fixtures. The 81-tile
// crossing is routed by the host. No seed teleport can satisfy either arrival.
#[cfg(feature = "js")]
const ENDS: [WorldTile; 2] = [
    WorldTile {
        x: 3109,
        z: 3228,
        level: 0,
    },
    WorldTile {
        x: 3190,
        z: 3245,
        level: 0,
    },
];

#[cfg(feature = "js")]
#[derive(Default)]
struct Facts {
    ready: bool,
    tick: u32,
    tile: Option<WorldTile>,
    arm: Option<crate::script_runtime::ScriptWalkArm>,
    enabled: bool,
    next_end: usize,
    request: u64,
    inflight: Option<(u64, WorldTile)>,
    accepted: u64,
    arrivals: u64,
    observed_live_route: bool,
    live: bool,
    progress_tick: Option<u32>,
    error: Option<String>,
}

#[cfg(feature = "js")]
#[derive(Default)]
struct Member {
    facts: Arc<Mutex<Facts>>,
    control_started: bool,
}

#[cfg(feature = "js")]
struct ActiveRoute {
    members: Vec<Member>,
    stopped: bool,
}

#[cfg(feature = "js")]
fn observe_snapshot(facts: &mut Facts, snapshot: &api::snapshot::GameSnapshot) {
    facts.ready = snapshot.ingame() && snapshot.scene_state() == 2;
    let prior = facts.tile;
    facts.tile = snapshot
        .tile()
        .map(|(x, z, level)| WorldTile { x, z, level });
    facts.tick = snapshot.tick();
    if !facts.ready || !facts.enabled || facts.error.is_some() {
        return;
    }
    let Some(arm) = facts.arm.as_ref() else {
        return;
    };
    let Some(here) = facts.tile else { return };
    let nav = match arm.navs.lock() {
        Ok(navs) => navs.get(&arm.name).map(|bot| {
            (
                bot.has_route(),
                bot.walk_work_live(),
                bot.walk_outcome_request_id,
                bot.walk_outcome_failed,
            )
        }),
        Err(_) => {
            facts.error = Some("active route owner unavailable".into());
            return;
        }
    };
    if let Some((route, live, terminal_request, failed)) = nav {
        facts.observed_live_route |= route;
        facts.live = live;
        if let Some((request, destination)) = facts.inflight {
            if failed && terminal_request == request {
                facts.error = Some(format!(
                    "active route {request} failed through the production owner"
                ));
                return;
            }
            if prior != facts.tile && live {
                facts.progress_tick = Some(facts.tick);
            }
            if here == destination && !live {
                facts.arrivals += 1;
                facts.inflight = None;
                facts.next_end ^= 1;
                facts.progress_tick = Some(facts.tick);
            }
        }
    }
    if facts.inflight.is_some() {
        return;
    }
    let destination = ENDS[facts.next_end];
    let Some(mut arm) = facts.arm.clone() else {
        return;
    };
    arm.here = Some((here.x, here.z, here.level));
    // The production queue derives gating facts from this exact snapshot,
    // rather than retaining or manufacturing a second WorldState cache.
    arm.state = None;
    let request = facts
        .request
        .checked_add(1)
        .expect("memory route request count");
    if arm.queue_route_in_snapshot(
        snapshot,
        destination.x,
        destination.z,
        destination.level,
        nav_planner::router::FindOptions::default(),
        0,
        false,
        request,
    ) {
        facts.request = request;
        facts.inflight = Some((request, destination));
        facts.accepted += 1;
        facts.live = true;
    }
}

#[cfg(feature = "js")]
impl FeatureCase for ActiveRoute {
    fn seed(&self, slot: usize, _: &[String]) -> Result<scenario::Scenario, String> {
        let mut seed = scenario::get("gatherer").ok_or("missing real route account seed")?;
        let start = seed
            .steps
            .iter()
            .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
            .ok_or("route seed lacks its Start boundary")?;
        seed.steps.truncate(start);
        let facts = Arc::clone(&self.members[slot].facts);
        seed.steps.push(scenario::Step {
            name: "observe current snapshots and request genuine host routes",
            kind: scenario::StepKind::Await {
                evidence: "memory active route owner and ordinary arrival",
                ready: Box::new(move |snapshot| {
                    observe_snapshot(
                        &mut facts.lock().expect("memory active route facts"),
                        snapshot,
                    );
                    false
                }),
                summary: None,
            },
            wait: scenario::Wait {
                arm: scenario::Proof::IngameScene2,
                budget_ticks: u32::MAX,
            },
        });
        seed.name = "active_route_navigation";
        seed.settings.start_script = None;
        seed.settings.terminal_shot = None;
        // Persistent observation is not a content wait. The original seed
        // budgets and the production route owner/watchdog remain unchanged.
        seed.settings.deadline = std::time::Duration::MAX;
        Ok(seed)
    }

    fn prepare_unused(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String> {
        let world = play
            .world()
            .ok_or("active route navigation has no bound navigation world")?;
        let mut ready = true;
        for (member, name) in self.members.iter_mut().zip(names) {
            if !member
                .facts
                .lock()
                .map_err(|_| "route facts unavailable")?
                .ready
            {
                ready = false;
                continue;
            }
            {
                let mut facts = member.facts.lock().map_err(|_| "route facts unavailable")?;
                if facts.arm.is_none() {
                    // Establish benchmark bookkeeping before either phase.
                    facts.arm = Some(crate::script_runtime::ScriptWalkArm {
                        here: None,
                        world: Some(Arc::clone(&world)),
                        navs: Arc::clone(&play.navs),
                        name: name.clone(),
                        state: None,
                        bank: nav_planner::bank_fetch::BankRows::default(),
                    });
                    facts.next_end = 1;
                }
            }
            if !member.control_started {
                member.control_started =
                    super::native_api::start(play, name, super::native_api::UNUSED)?;
            }
            ready &= member.control_started && play.script_state(name) == script::RunState::Running;
        }
        Ok(ready)
    }

    fn activate(&mut self, _: &crate::Play, _: &[String]) -> Result<bool, String> {
        let mut admitted = true;
        for member in &mut self.members {
            let mut facts = member.facts.lock().map_err(|_| "route facts unavailable")?;
            if facts.arm.is_none() {
                return Err("active route control was not prepared".into());
            }
            facts.enabled = true;
            admitted &= facts.accepted > 0;
        }
        Ok(admitted)
    }

    fn observe(&mut self, play: &crate::Play, names: &[String]) -> Result<Observation, String> {
        let mut observation = Observation::default();
        for (member, name) in self.members.iter().zip(names) {
            let stopped_live = if self.stopped {
                Some(
                    play.navs
                        .lock()
                        .map_err(|_| "route owner unavailable")?
                        .get(name)
                        .is_some_and(|bot| bot.walk_work_live()),
                )
            } else {
                None
            };
            let mut facts = member.facts.lock().map_err(|_| "route facts unavailable")?;
            if let Some(live) = stopped_live {
                facts.live = live;
            }
            if let Some(error) = &facts.error {
                return Err(format!("{name}: {error}"));
            }
            let ready = facts.ready;
            let enabled = facts.enabled;
            let live = facts.live;
            let proved = facts.observed_live_route && facts.accepted >= 3 && facts.arrivals >= 2;
            let continuing = live
                && facts
                    .progress_tick
                    .is_some_and(|tick| facts.tick.wrapping_sub(tick) <= 240);
            drop(facts);
            observation.ready += usize::from(ready);
            if enabled && !self.stopped {
                if play.script_state(name) != script::RunState::Running {
                    return Err(format!("active route control isolate stopped: {name}"));
                }
                observation.active += usize::from(live);
                observation.proved += usize::from(proved);
                observation.continuing += usize::from(continuing);
            }
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &crate::Play, names: &[String]) -> Result<(), String> {
        for (member, name) in self.members.iter_mut().zip(names) {
            member
                .facts
                .lock()
                .map_err(|_| "route facts unavailable")?
                .enabled = false;
            play.script_stop(name);
        }
        self.stopped = true;
        Ok(())
    }

    fn evidence(&self, names: &[String]) -> serde_json::Value {
        serde_json::json!({
            "owner": "ScriptWalkArm -> shared route work -> NavBot -> Traveller",
            "snapshot_source": "existing scenario client snapshot",
            "control": "accepted v2 isolate without route requests",
            "post_stop": self.stopped,
            "members": self.members.iter().zip(names).map(|(member, name)| {
                let facts = member.facts.lock().expect("memory active route facts");
                serde_json::json!({"name": name, "game_tick": facts.tick,
                    "accepted_routes": facts.accepted, "ordinary_arrivals": facts.arrivals,
                    "observed_live_route": facts.observed_live_route, "route_live": facts.live,
                    "last_progress_tick": facts.progress_tick,
                    "tile": facts.tile.map(|tile| [tile.x, tile.z, tile.level])})
            }).collect::<Vec<_>>()
        })
    }
}
