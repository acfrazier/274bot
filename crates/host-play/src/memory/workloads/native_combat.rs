use super::FeatureCase;
#[cfg(feature = "js")]
use super::Observation;

#[cfg(not(feature = "js"))]
pub(super) fn build(_: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    Err("native_npc_combat requires the shipped js feature".into())
}

#[cfg(feature = "js")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "js")]
use std::time::Duration;

#[cfg(feature = "js")]
const COW_ID: i32 = 81;
#[cfg(feature = "js")]
const SETUP_WAIT_TICKS: u32 = 120;
#[cfg(feature = "js")]
const FIGHT_BUDGET_TICKS: u16 = 600;
#[cfg(feature = "js")]
const PROGRESS_BUDGET_TICKS: u64 = 240;

/// R289 `npc_placements` cows (id 81) grouped into separate real spawn areas
/// from candidate-game-data/289.json. Each account owns one non-overlapping
/// area; each request includes the content's five-tile Cow wander range.
/// Cow's eight hitpoints retain a longer real Combat session than the chicken fixture.
#[cfg(feature = "js")]
#[derive(Clone, Copy)]
struct Site {
    mapsquare: &'static str,
    spawns: &'static [(i32, i32)],
}

#[cfg(feature = "js")]
impl Site {
    fn anchor(self) -> (i32, i32) {
        self.spawns[0]
    }

    fn area(self) -> [i32; 5] {
        let (min_x, max_x, min_z, max_z) = self.spawns.iter().fold(
            (i32::MAX, i32::MIN, i32::MAX, i32::MIN),
            |(min_x, max_x, min_z, max_z), &(x, z)| {
                (min_x.min(x), max_x.max(x), min_z.min(z), max_z.max(z))
            },
        );
        [min_x - 5, min_z - 5, max_x + 5, max_z + 5, 0]
    }
}

#[cfg(feature = "js")]
const SITES: [Site; 10] = [
    Site {
        mapsquare: "m40_48",
        spawns: &[(2580, 3117), (2582, 3122), (2588, 3119)],
    },
    Site {
        mapsquare: "m40_48",
        spawns: &[(2604, 3114)],
    },
    Site {
        mapsquare: "m41_52",
        spawns: &[
            (2657, 3341),
            (2658, 3351),
            (2660, 3344),
            (2664, 3341),
            (2664, 3348),
            (2664, 3352),
            (2666, 3344),
            (2670, 3348),
            (2671, 3342),
            (2672, 3354),
        ],
    },
    Site {
        mapsquare: "m42_55",
        spawns: &[(2733, 3561)],
    },
    Site {
        mapsquare: "m45_51",
        spawns: &[
            (2917, 3289),
            (2920, 3285),
            (2921, 3287),
            (2924, 3277),
            (2924, 3288),
            (2925, 3282),
            (2926, 3271),
            (2930, 3269),
            (2933, 3274),
            (2936, 3274),
        ],
    },
    Site {
        mapsquare: "m47_51",
        spawns: &[
            (3027, 3307),
            (3029, 3305),
            (3029, 3312),
            (3030, 3300),
            (3033, 3305),
            (3037, 3306),
            (3040, 3310),
        ],
    },
    Site {
        mapsquare: "m49_51",
        spawns: &[(3154, 3325), (3160, 3318)],
    },
    Site {
        mapsquare: "m49_52",
        spawns: &[(3156, 3342)],
    },
    Site {
        mapsquare: "m50_50",
        spawns: &[(3254, 3258), (3258, 3260), (3261, 3259)],
    },
    Site {
        mapsquare: "m50_51",
        spawns: &[
            (3243, 3295),
            (3244, 3289),
            (3247, 3284),
            (3255, 3278),
            (3256, 3292),
            (3261, 3292),
        ],
    },
];

#[cfg(feature = "js")]
#[derive(Clone, Copy, Default)]
struct Facts {
    ready: bool,
    tick: u64,
    combat_xp: [Option<i32>; 4],
    /// Latest tick where the client itself observed a combat-XP increase.
    progress_tick: Option<u64>,
}

#[cfg(feature = "js")]
fn combat_xp(snapshot: &api::snapshot::GameSnapshot) -> [Option<i32>; 4] {
    let mut xp = [None; 4];
    for stat in snapshot.stats() {
        match stat.index {
            0 => xp[0] = Some(stat.xp),
            1 => xp[1] = Some(stat.xp),
            2 => xp[2] = Some(stat.xp),
            3 => xp[3] = Some(stat.xp),
            _ => {}
        }
    }
    xp
}

#[cfg(feature = "js")]
fn xp_advanced(before: [Option<i32>; 4], after: [Option<i32>; 4]) -> bool {
    before
        .into_iter()
        .zip(after)
        .any(|(before, after)| matches!((before, after), (Some(before), Some(after)) if after > before))
}

#[cfg(feature = "js")]
#[derive(Clone, Copy, Default)]
enum NativeStage {
    #[default]
    Unknown,
    Starting,
    Fighting,
    ClearingPrayers,
}

#[cfg(feature = "js")]
impl NativeStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Starting => "starting",
            Self::Fighting => "fighting",
            Self::ClearingPrayers => "clearing-prayers",
        }
    }
}

#[cfg(feature = "js")]
#[derive(Clone, Copy, Default)]
enum EngagedKind {
    #[default]
    None,
    Npc,
    Player,
    Other,
}

#[cfg(feature = "js")]
impl EngagedKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Npc => "npc",
            Self::Player => "player",
            Self::Other => "other",
        }
    }
}

#[cfg(feature = "js")]
#[derive(Default)]
struct Member {
    facts: Arc<Mutex<Facts>>,
    control_started: bool,
    control_stopped: bool,
    active_started: bool,
    baseline_xp: Option<[Option<i32>; 4]>,
    activation_tick: Option<u64>,
    proof_tick: Option<u64>,
    owned: bool,
    stage: NativeStage,
    engaged_kind: EngagedKind,
    engaged_index: Option<i64>,
    stop_tick: Option<u64>,
    post_stop_tick: Option<u64>,
    post_stop_owner: Option<bool>,
    post_stop_idle: Option<bool>,
}

#[cfg(feature = "js")]
struct NativeNpcCombat {
    slots: Vec<Member>,
    stopped: bool,
}

#[cfg(feature = "js")]
pub(super) fn build(names: &[String]) -> Result<Box<dyn FeatureCase>, String> {
    if !matches!(names.len(), 1 | 10) {
        return Err(format!(
            "native_npc_combat supports only its real N=1 and N=10 sites, not {} accounts",
            names.len()
        ));
    }
    Ok(Box::new(NativeNpcCombat {
        slots: names.iter().map(|_| Member::default()).collect(),
        stopped: false,
    }))
}

/// The active isolate continually re-issues the real native API request: a
/// Combat machine normally settles its one-NPC fight on a kill. The consumer
/// remains a NativeTick isolate between accepted Combat sessions.
#[cfg(feature = "js")]
fn active_source(site: Site) -> Result<String, String> {
    let request = request_value(site);
    let request = serde_json::to_string(&request)
        .map_err(|error| format!("native_npc_combat request encoding: {error}"))?;
    Ok(format!(
        r#"
export const apiVersion = 2;
const request = {request};
let waiting = false;
export function tick(api) {{
  if (waiting) return;
  waiting = true;
  api.combat.fight(request).then(
    () => {{ waiting = false; }},
    error => {{
      api.log('native_npc_combat fight failed: ' + String(error));
      api.stop('native_npc_combat native fight request failed');
    }}
  );
}}
"#
    ))
}

#[cfg(feature = "js")]
fn request_value(site: Site) -> serde_json::Value {
    let [min_x, min_z, max_x, max_z, level] = site.area();
    serde_json::json!({
        "target": {"npcId": COW_ID},
        "area": {"min_x": min_x, "min_z": min_z, "max_x": max_x, "max_z": max_z, "level": level},
        "radius": 30,
        "meleeMode": "aggressive",
        "prayer": false,
        "food": false,
        "potions": false,
        "budgetTicks": FIGHT_BUDGET_TICKS
    })
}

#[cfg(feature = "js")]
fn start(play: &crate::Play, name: &str, source: &str) -> Result<bool, String> {
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
        Err(error) => Err(format!("native_npc_combat start {name}: {error}")),
    }
}

#[cfg(feature = "js")]
fn combat_stat_step(command: &'static str, stat_id: i32) -> scenario::Step {
    scenario::Step {
        name: "set this benchmark account's combat stat before Start",
        kind: scenario::StepKind::Perform {
            send: Box::new(move |client, _| api::interact::cheat(client, command).is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::Stat {
                id: stat_id,
                min: 40,
            },
            budget_ticks: SETUP_WAIT_TICKS,
        },
    }
}

#[cfg(feature = "js")]
impl FeatureCase for NativeNpcCombat {
    fn seed(&self, slot: usize, _: &[String]) -> Result<scenario::Scenario, String> {
        let site = *SITES
            .get(slot)
            .ok_or("native_npc_combat has no independent NPC site for this account")?;
        let mut seed = scenario::get("gatherer").ok_or("missing real Gatherer account seed")?;
        let tree = seed
            .steps
            .iter()
            .position(|step| matches!(step.wait.arm, scenario::Proof::LocIdNear { id: 1276, .. }))
            .ok_or("Gatherer seed lacks its real normal-tree proof")?;
        let start = seed
            .steps
            .iter()
            .position(|step| matches!(step.kind, scenario::StepKind::StartScript))
            .ok_or("Gatherer seed lacks its Start boundary")?;
        seed.steps.truncate(start);
        let stand = tree
            .checked_sub(1)
            .filter(|&index| matches!(seed.steps[index].wait.arm, scenario::Proof::Arrived { .. }))
            .ok_or("Gatherer live-tree proof lacks its preceding stand")?;

        for (offset, (command, stat_id)) in [
            ("setstat attack 40", 0),
            ("setstat defence 40", 1),
            ("setstat strength 40", 2),
            ("setstat hitpoints 40", 3),
        ]
        .into_iter()
        .enumerate()
        {
            seed.steps
                .insert(stand + offset, combat_stat_step(command, stat_id));
        }
        let stand = stand + 4;
        let tree = tree + 4;
        let (x, z) = site.anchor();
        seed.steps[stand].name = "stand in this account's independent Cow neighbourhood";
        seed.steps[stand].kind = scenario::StepKind::Perform {
            send: Box::new(move |client, _| {
                api::interact::cheat(client, &api::interact::tele_args(0, x, z)).is_sent()
            }),
        };
        seed.steps[stand].wait.arm = scenario::Proof::Arrived { x, z, level: 0 };
        seed.steps[tree].name = "confirm this site's real Cow placement before Start";
        seed.steps[tree].wait.arm = scenario::Proof::NpcNear {
            r#type: COW_ID as usize,
            radius: 10,
        };

        let facts = Arc::clone(&self.slots[slot].facts);
        seed.steps.push(scenario::Step {
            name: "observe real native NPC Combat state without sending actions",
            kind: scenario::StepKind::Await {
                evidence: "native_npc_combat client observations",
                ready: Box::new(move |snapshot| {
                    let mut facts = facts.lock().expect("native NPC Combat facts");
                    let tick = u64::from(snapshot.tick());
                    let next_xp = combat_xp(snapshot);
                    if xp_advanced(facts.combat_xp, next_xp) {
                        facts.progress_tick = Some(tick);
                    }
                    facts.ready = snapshot.ingame()
                        && snapshot.scene_state() == 2
                        && next_xp.iter().all(Option::is_some);
                    facts.tick = tick;
                    facts.combat_xp = next_xp;
                    false
                }),
                summary: None,
            },
            wait: scenario::Wait {
                arm: scenario::Proof::IngameScene2,
                budget_ticks: u32::MAX,
            },
        });
        seed.name = "native_npc_combat";
        seed.settings.start_script = None;
        seed.settings.terminal_shot = None;
        seed.settings.deadline = Duration::MAX;
        Ok(seed)
    }

    fn prepare_unused(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String> {
        check_names(self.slots.len(), names)?;
        let mut ready = true;
        for (member, name) in self.slots.iter_mut().zip(names) {
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native NPC Combat facts unavailable")?;
            if !facts.ready {
                ready = false;
                continue;
            }
            if !member.control_started {
                member.control_started = start(play, name, super::native_api::UNUSED)?;
            }
            let slot = crate::script_slot(&play.scripts, name)
                .ok_or_else(|| format!("native_npc_combat control slot disappeared: {name}"))?;
            let slot = slot
                .lock()
                .map_err(|_| "native NPC Combat slot unavailable")?;
            ready &= member.control_started
                && slot.state() == script::RunState::Running
                && !slot.api_owns_foreground();
        }
        Ok(ready)
    }

    fn activate(&mut self, play: &crate::Play, names: &[String]) -> Result<bool, String> {
        check_names(self.slots.len(), names)?;
        let mut admitted = true;
        for (index, (member, name)) in self.slots.iter_mut().zip(names).enumerate() {
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
            let mut facts = member
                .facts
                .lock()
                .map_err(|_| "native NPC Combat facts unavailable")?;
            if !facts.ready {
                admitted = false;
                continue;
            }
            member.baseline_xp = Some(facts.combat_xp);
            member.activation_tick = Some(facts.tick);
            member.proof_tick = None;
            facts.progress_tick = None;
            drop(facts);
            member.active_started = start(play, name, &active_source(SITES[index])?)?;
            admitted &= member.active_started;
        }
        Ok(admitted && self.slots.iter().all(|member| member.owned))
    }

    fn observe(&mut self, play: &crate::Play, names: &[String]) -> Result<Observation, String> {
        check_names(self.slots.len(), names)?;
        let mut observation = Observation::default();
        for (member, name) in self.slots.iter_mut().zip(names) {
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native NPC Combat facts unavailable")?;
            observation.ready += usize::from(facts.ready);
            let Some(slot) = crate::script_slot(&play.scripts, name) else {
                if member.active_started {
                    return Err(format!("native_npc_combat slot disappeared: {name}"));
                }
                continue;
            };
            let slot = slot
                .lock()
                .map_err(|_| "native NPC Combat slot unavailable")?;
            let running = slot.state() == script::RunState::Running;
            let status = slot.native_status();
            let is_combat_status = status.as_ref().is_some_and(|status| {
                status.card.0 == "CombatSession"
                    && status.phase == script::native::NativePhase::Working
            });
            member.owned = slot.api_owns_foreground() && is_combat_status;
            member.stage = status
                .as_ref()
                .and_then(|status| status_text_field(status, "stage"))
                .map_or(NativeStage::Unknown, |stage| match stage {
                    "starting" => NativeStage::Starting,
                    "fighting" => NativeStage::Fighting,
                    "clearing-prayers" => NativeStage::ClearingPrayers,
                    _ => NativeStage::Unknown,
                });
            member.engaged_kind = status
                .as_ref()
                .and_then(|status| status_text_field(status, "engaged_kind"))
                .map_or(EngagedKind::None, |kind| match kind {
                    "npc" => EngagedKind::Npc,
                    "player" => EngagedKind::Player,
                    "none" => EngagedKind::None,
                    _ => EngagedKind::Other,
                });
            member.engaged_index = status.as_ref().and_then(|status| {
                status.fields.iter().find_map(|field| {
                    (field.key == "engaged_index").then(|| match &field.value {
                        script::native::StatusValue::Integer(index) if *index >= 0 => Some(*index),
                        _ => None,
                    })?
                })
            });

            if member.active_started && !self.stopped {
                if !running {
                    return Err(format!("native_npc_combat isolate stopped before Stop: {name}"));
                }
                let fresh_xp = member
                    .baseline_xp
                    .zip(Some(facts.combat_xp))
                    .is_some_and(|(baseline, current)| xp_advanced(baseline, current));
                let fresh_progress = member
                    .activation_tick
                    .zip(facts.progress_tick)
                    .is_some_and(|(activation, progress)| progress > activation);
                if member.proof_tick.is_none() && member.owned && fresh_xp && fresh_progress {
                    member.proof_tick = facts.progress_tick;
                }
                if member.proof_tick.is_none()
                    && member
                        .activation_tick
                        .is_some_and(|tick| facts.tick.saturating_sub(tick) > PROGRESS_BUDGET_TICKS)
                {
                    return Err(format!(
                        "native_npc_combat lacks accepted Combat ownership and fresh combat XP within {PROGRESS_BUDGET_TICKS} game ticks: {name}"
                    ));
                }
                observation.active += usize::from(member.owned && running);
                observation.continuing += usize::from(
                    member.owned
                        && running
                        && member.proof_tick.is_some()
                        && facts
                            .progress_tick
                            .is_some_and(|tick| facts.tick.saturating_sub(tick) <= PROGRESS_BUDGET_TICKS),
                );
            }
            observation.proved += usize::from(member.proof_tick.is_some());
            if let Some(stop_tick) = member.stop_tick {
                if facts.tick > stop_tick {
                    member.post_stop_tick = Some(facts.tick);
                    member.post_stop_owner = Some(member.owned);
                    member.post_stop_idle = Some(slot.state() == script::RunState::Idle);
                }
            }
        }
        Ok(observation)
    }

    fn stop(&mut self, play: &crate::Play, names: &[String]) -> Result<(), String> {
        check_names(self.slots.len(), names)?;
        for (member, name) in self.slots.iter_mut().zip(names) {
            play.script_stop(name);
            let facts = *member
                .facts
                .lock()
                .map_err(|_| "native NPC Combat facts unavailable")?;
            member.stop_tick = Some(facts.tick);
        }
        self.stopped = true;
        Ok(())
    }

    fn evidence(&self, names: &[String]) -> serde_json::Value {
        serde_json::json!({
            "scenario": "native_npc_combat",
            "feature_off_control": "native_npc_combat_feature_off",
            "native_owner": "SlotScript API Combat foreground and CombatSession Working status",
            "control_source": "accepted apiVersion 2 NativeTick isolate without api.combat.fight",
            "active_source": "accepted apiVersion 2 NativeTick isolate continually requesting api.combat.fight",
            "post_stop": self.stopped,
            "members": self.slots.iter().zip(names).enumerate().map(|(index, (member, name))| {
                let facts = *member.facts.lock().expect("native NPC Combat facts");
                serde_json::json!({
                    "name": name,
                    "mapsquare": SITES[index].mapsquare,
                    "game_tick": facts.tick,
                    "combat_xp": facts.combat_xp,
                    "baseline_combat_xp": member.baseline_xp,
                    "activation_game_tick": member.activation_tick,
                    "combat_xp_progress_game_tick": facts.progress_tick,
                    "proof_game_tick": member.proof_tick,
                    "native_card": if member.owned { "CombatSession" } else { "none" },
                    "native_phase": if member.owned { "working" } else { "none" },
                    "native_stage": member.stage.as_str(),
                    "engaged_kind": member.engaged_kind.as_str(),
                    "engaged_index": member.engaged_index,
                    "proved": member.proof_tick.is_some(),
                    "stop_game_tick": member.stop_tick,
                    "post_stop_observed_game_tick": member.post_stop_tick,
                    "post_stop_native_owner": member.post_stop_owner,
                    "post_stop_idle": member.post_stop_idle,
                })
            }).collect::<Vec<_>>()
        })
    }
}

#[cfg(feature = "js")]
fn status_text_field<'a>(status: &'a script::native::ScriptStatus, key: &str) -> Option<&'a str> {
    status
        .fields
        .iter()
        .find(|field| field.key == key)
        .and_then(|field| match &field.value {
            script::native::StatusValue::Text(value) => Some(value.as_ref()),
            _ => None,
        })
}

#[cfg(feature = "js")]
fn check_names(slot_count: usize, names: &[String]) -> Result<(), String> {
    if slot_count == names.len() {
        Ok(())
    } else {
        Err(format!(
            "native_npc_combat account list changed: expected {slot_count}, got {}",
            names.len()
        ))
    }
}

#[cfg(all(test, feature = "js"))]
mod tests {
    use super::*;
    use script::api_combat::{CombatSessionRequest, TargetSpec};

    #[test]
    fn pilot_and_fleet_seeds_use_distinct_real_cow_sites_and_no_start_script() {
        let pilot = build(&["pilot".to_owned()]).unwrap();
        let pilot_seed = pilot.seed(0, &["pilot".to_owned()]).unwrap();
        assert_eq!(pilot_seed.name, "native_npc_combat");
        assert!(pilot_seed.settings.start_script.is_none());
        assert!(pilot_seed
            .steps
            .iter()
            .all(|step| !matches!(step.kind, scenario::StepKind::StartScript)));

        let names: Vec<_> = (0..SITES.len()).map(|index| format!("bot{index}")).collect();
        let fleet = build(&names).unwrap();
        for (index, site) in SITES.iter().enumerate() {
            assert!(!site.spawns.is_empty());
            let [min_x, min_z, max_x, max_z, 0] = site.area() else {
                panic!("Cow sites are on the ground plane")
            };
            for &(x, z) in site.spawns {
                assert!((min_x..=max_x).contains(&x));
                assert!((min_z..=max_z).contains(&z));
                assert_eq!(site.mapsquare, format!("m{}_{}", x / 64, z / 64));
            }
            for other in &SITES[index + 1..] {
                let [other_min_x, other_min_z, other_max_x, other_max_z, 0] = other.area() else {
                    unreachable!()
                };
                assert!(max_x < other_min_x
                    || other_max_x < min_x
                    || max_z < other_min_z
                    || other_max_z < min_z);
            }
            let seed = fleet.seed(index, &names).unwrap();
            assert!(seed.steps.iter().any(|step| matches!(
                step.wait.arm,
                scenario::Proof::NpcNear {
                    r#type,
                    radius
                } if r#type == COW_ID as usize && radius == 10
            )));
            assert!(seed.settings.start_script.is_none());
            assert!(seed
                .steps
                .iter()
                .all(|step| !matches!(step.kind, scenario::StepKind::StartScript)));
        }
    }

    #[test]
    fn unsupported_counts_fail_closed() {
        for n in [0, 2, 9, 11, 16, 128] {
            let names: Vec<_> = (0..n).map(|i| format!("bot{i}")).collect();
            assert!(build(&names).is_err());
        }
    }

    #[test]
    fn generated_requests_decode_as_the_real_native_combat_request() {
        for site in SITES {
            let value = request_value(site);
            let request = CombatSessionRequest::from_value(&value).unwrap();
            assert_eq!(request.target, TargetSpec::Ids(Box::new([COW_ID])));
            assert_eq!(request.area.as_ref(), &[site.area()]);
            assert_eq!(request.radius, 30);
            assert_eq!(request.budget_ticks, FIGHT_BUDGET_TICKS);
            assert_eq!(request.melee_mode, Some(script::combat::MeleeMode::Aggressive));
            assert!(!request.prayer && !request.food && !request.potions);
        }
    }

    #[test]
    fn proof_accounting_requires_fresh_post_activation_xp_and_bounds_continuation() {
        let baseline = [Some(100), Some(100), Some(100), Some(100)];
        assert!(!xp_advanced(baseline, baseline));
        assert!(xp_advanced(baseline, [Some(100), Some(100), Some(101), Some(100)]));
        assert!(!xp_advanced([None; 4], [Some(100); 4]));

        let activation_tick = 50;
        assert!(!Some(50).is_some_and(|tick| tick > activation_tick));
        assert!(Some(51).is_some_and(|tick| tick > activation_tick));
        assert!(200_u64.saturating_sub(100) <= PROGRESS_BUDGET_TICKS);
        assert!(500_u64.saturating_sub(100) > PROGRESS_BUDGET_TICKS);
    }
}

#[cfg(all(test, not(feature = "js")))]
mod no_js_tests {
    use super::*;

    #[test]
    fn build_requires_the_shipped_js_feature() {
        assert!(build(&["pilot".into()]).is_err());
    }
}
