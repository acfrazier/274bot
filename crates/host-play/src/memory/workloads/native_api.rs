use super::FeatureCase;
#[cfg(feature = "js")]
use super::Observation;

#[cfg(not(feature = "js"))]
pub(super) fn build(_: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    Err("merged_native_api requires the shipped js feature".into())
}

#[cfg(feature = "js")]
pub(super) fn build(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    Ok(Box::new(NativeApi {
        slots: names.iter().map(|_| Member::default()).collect(),
        stopped: false,
    }))
}

#[cfg(feature = "js")]
use std::sync::{Arc, Mutex};

// Both phases keep an accepted v2 isolate. Only the active phase requests the
// native owner. These are API consumers, not a second Gatherer implementation.
#[cfg(feature = "js")]
pub(super) const UNUSED: &str =
    "export const apiVersion = 2; export function tick(api) { void api.snapshot.ingame; }";
#[cfg(feature = "js")]
const ACTIVE: &str = r#"
export const apiVersion = 2;
let run;
export function tick(api) {
  if (!run && api.snapshot.ingame && api.snapshot.here) {
    run = api.gather.run({ skill: 'Woodcutting', woodcuttingResources: ['normal'], location: 'Start', disposition: 'Power', radius: 12 });
    run.then(result => {
      api.log('memory-native-api-terminal:' + JSON.stringify(result));
      api.stop('native Gatherer ended before benchmark Stop');
    });
  }
}
"#;

#[cfg(feature = "js")]
#[derive(Clone, Copy, Default)]
struct Facts {
    ready: bool,
    tick: u64,
    xp: Option<i32>,
}

#[cfg(feature = "js")]
#[derive(Default)]
struct Member {
    facts: Arc<Mutex<Facts>>,
    control_started: bool,
    control_stopped: bool,
    active_started: bool,
    baseline_xp: Option<i32>,
    activation_tick: Option<u64>,
    progress_tick: Option<u64>,
    last_xp: Option<i32>,
    last_yielded: i64,
    owned: bool,
    proved: bool,
}

#[cfg(feature = "js")]
struct NativeApi {
    slots: Vec<Member>,
    stopped: bool,
}

#[cfg(feature = "js")]
pub(super) fn start(play: &crate::Play, name: &str, source: &str) -> Result<bool, String> {
    let hold = play
        .script_start_hold
        .lock()
        .map_err(|_| "script start hold unavailable")?;
    if hold.upgrade().is_some() {
        return Ok(false);
    }
    drop(hold);
    match play.script_start_load_typed(
        name,
        source.into(),
        script_js::LoadShape::NativeTick,
        None,
        vec![],
    ) {
        Ok(()) => Ok(true),
        Err(script::StartLoadError::Waiting(_)) => Ok(false),
        Err(error) => Err(format!("merged_native_api start {name}: {error}")),
    }
}

#[cfg(feature = "js")]
impl FeatureCase for NativeApi {
    fn seed(&self, slot: usize, _: &[String]) -> Result<scenario::Scenario, String> {
        let mut seed = scenario::get("gatherer").ok_or("missing real Gatherer seed")?;
        let start = seed
            .steps
            .iter()
            .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
            .ok_or("Gatherer seed lacks its Start boundary")?;
        seed.steps.truncate(start);
        let facts = Arc::clone(&self.slots[slot].facts);
        seed.steps.push(scenario::Step {
            name: "observe the real native API workload without sending actions",
            kind: scenario::StepKind::Await {
                evidence: "memory native API client observations",
                ready: Box::new(move |snapshot| {
                    let mut facts = facts.lock().expect("memory native API facts");
                    facts.ready = snapshot.ingame() && snapshot.scene_state() == 2;
                    facts.tick = u64::from(snapshot.tick());
                    facts.xp = snapshot
                        .stats()
                        .iter()
                        .find(|stat| stat.index == 8)
                        .map(|stat| stat.xp);
                    false
                }),
                summary: None,
            },
            wait: scenario::Wait {
                arm: scenario::Proof::IngameScene2,
                budget_ticks: u32::MAX,
            },
        });
        seed.name = "merged_native_api";
        seed.settings.start_script = None;
        seed.settings.terminal_shot = None;
        // This final observer never waits for a content outcome. The original
        // seed steps retain their game-tick budgets. Admission/progress below
        // retain the Gatherer fixture's 240 observed-game-tick proof budget.
        seed.settings.deadline = std::time::Duration::MAX;
        Ok(seed)
    }

    fn prepare_unused(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String> {
        let mut ready = true;
        for (member, name) in self.slots.iter_mut().zip(names) {
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native API facts unavailable")?;
            if !facts.ready || facts.xp.is_none() {
                ready = false;
                continue;
            }
            if !member.control_started {
                member.control_started = start(play, name, UNUSED)?;
            }
            ready &= member.control_started && play.script_state(name) == script::RunState::Running;
        }
        Ok(ready)
    }

    fn activate(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String> {
        let mut admitted = true;
        for (member, name) in self.slots.iter_mut().zip(names) {
            if member.active_started {
                continue;
            }
            if !member.control_stopped {
                play.script_stop(name);
                member.control_stopped = true;
            }
            if play.script_state(name) != script::RunState::Idle {
                admitted = false;
                continue;
            }
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native API facts unavailable")?;
            member.baseline_xp = facts.xp;
            member.last_xp = facts.xp;
            member.activation_tick = Some(facts.tick);
            member.active_started = start(play, name, ACTIVE)?;
            admitted &= member.active_started;
        }
        Ok(admitted && self.slots.iter().all(|member| member.owned))
    }

    fn observe(&mut self, play: &crate::Play, names: &[String]) -> Result<Observation, String> {
        let mut observation = Observation::default();
        for (member, name) in self.slots.iter_mut().zip(names) {
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native API facts unavailable")?;
            observation.ready += usize::from(facts.ready && facts.xp.is_some());
            let Some(slot) = crate::script_slot(&play.scripts, name) else {
                if member.active_started {
                    return Err(format!("native API slot disappeared: {name}"));
                }
                continue;
            };
            let slot = slot.lock().map_err(|_| "native API slot unavailable")?;
            let owned = slot.api_gather_walk_permissions().is_some();
            let status = slot.native_status();
            let yielded = status
                .as_ref()
                .and_then(|status| {
                    status.fields.iter().find_map(|field| {
                        if field.key == "yielded" {
                            if let script::native::StatusValue::Integer(value) = &field.value {
                                return Some(*value);
                            }
                        }
                        None
                    })
                })
                .unwrap_or(0);
            member.owned = owned;
            let running = slot.state() == script::RunState::Running;
            if member.active_started && !self.stopped {
                if !running && slot.state() != script::RunState::Starting {
                    return Err(format!(
                        "merged_native_api stopped before collection: {name}"
                    ));
                }
                let xp_gain = facts
                    .xp
                    .zip(member.baseline_xp)
                    .is_some_and(|(xp, baseline)| xp > baseline);
                member.proved |= owned && yielded > 0 && xp_gain;
                if facts
                    .xp
                    .zip(member.last_xp)
                    .is_some_and(|(now, prior)| now > prior)
                    || yielded > member.last_yielded
                {
                    member.progress_tick = Some(facts.tick);
                }
                if !member.proved
                    && member
                        .activation_tick
                        .is_some_and(|tick| facts.tick.saturating_sub(tick) > 240)
                {
                    return Err(format!("merged_native_api lacks accepted owner and fresh yield within 240 game ticks: {name}"));
                }
                observation.active += usize::from(owned && running);
                observation.continuing += usize::from(
                    owned
                        && running
                        && member
                            .progress_tick
                            .is_some_and(|tick| facts.tick.saturating_sub(tick) <= 240),
                );
            }
            observation.proved += usize::from(member.proved);
            member.last_xp = facts.xp;
            member.last_yielded = yielded;
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &crate::Play, names: &[String]) -> Result<(), String> {
        for name in names {
            play.script_stop(name);
        }
        self.stopped = true;
        Ok(())
    }

    fn evidence(&self, names: &[String]) -> serde_json::Value {
        serde_json::json!({
            "owner": "SlotScript native API Gatherer seat",
            "control": "accepted NativeTick isolate without api.gather.run",
            "post_stop": self.stopped,
            "members": self.slots.iter().zip(names).map(|(member, name)| {
                let facts = *member.facts.lock().expect("memory native API facts");
                serde_json::json!({"name": name, "game_tick": facts.tick, "woodcutting_xp": facts.xp,
                    "baseline_xp": member.baseline_xp, "yielded": member.last_yielded,
                    "native_owner_live": member.owned, "proved": member.proved,
                    "last_progress_tick": member.progress_tick})
            }).collect::<Vec<_>>()
        })
    }
}
