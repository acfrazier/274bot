use super::*;

use api::hostlog::{Category, Level};
const LOST_TOOL_BOUND_MS: u64 = 12_000;
/// The Swarm's `maxrange` 15 plus the `maxrange + 2` stop margin is 17 tiles
/// from the NPC start. The shared spawner places it one tile from the player,
/// so the first-choice route is 16 and fallback ring points are at most 18;
/// one 7-tick attack interval gives the 25-fresh-tick bound
/// (`content/scripts/macro events/configs/antimacro.npc:113-131`,
/// `content/scripts/macro events/scripts/macro_events.rs2:30-36`,
/// `content/scripts/engine.rs2:77-78`). The Dwarf's 10000 range is
/// Dialog/Talk-to and is excluded
/// (`content/scripts/macro events/configs/antimacro.npc:1385,1397`).
const MAX_EVADE_TICKS: u32 = 25;
/// NPC packets use the preferred 15-tile radius (`info.ts:310`, `build.ts:57`).
const NPC_VIEW_RADIUS: i32 = 15;

/// The chrome contract both views bind (guardian spec `RandomStatus`):
/// published every tick on the slot status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomStatus {
    /// The detected event; `None` when the snapshot shows nothing.
    pub kind: Option<RandomKind>,
    pub name: Option<String>,
    pub ours: bool,
    /// A dialog handle is in flight (the host is talking it through).
    pub handling: bool,
    /// The slot must skip script tick / follow (host-play enforces the
    /// freeze). A dialog handle in flight or a trapped kind holds, while
    /// the claim is Host and the toggle is on.
    pub hold: bool,
    pub toggle: bool,
    pub claim: RandomClaim,
    /// The shown NPC slot is in the 45 s wrong-talk bin.
    pub cooldown: bool,
}

impl Default for RandomStatus {
    fn default() -> Self {
        Self {
            kind: None,
            name: None,
            ours: false,
            handling: false,
            hold: false,
            toggle: false,
            claim: RandomClaim::Host,
            cooldown: false,
        }
    }
}
/// Per-frame inputs that change independently of a slot's static profile.
pub struct GuardianLiveContext<'a> {
    /// Live per-event choices; the profile arm updates this map.
    pub event_policies: &'a BTreeMap<String, RandomEventPolicy>,
    /// Stats from the bound client revision; missing data must not guess.
    pub game_data: Option<&'a SelectedGameData>,
}
/// One explicit ownership-probe machine. Identity-scoped rejected actors are
/// retained separately so they cannot shadow a different probe candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PlantProbe {
    #[default]
    Idle,
    AwaitingResponse {
        actor: PlantActor,
        deadline_ms: u64,
    },
    Authenticated {
        actor: PlantActor,
        retry_at_ms: u64,
        deadline_ms: u64,
        continues: u32,
    },
}

impl PlantProbe {
    fn actor(&self) -> Option<&PlantActor> {
        match self {
            Self::Idle => None,
            Self::AwaitingResponse { actor, .. } | Self::Authenticated { actor, .. } => Some(actor),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EvadeState {
    index: usize,
    npc_type: Option<usize>,
    name: String,
    /// Selected content's NPC `maxrange`, not a guessed flee ring.
    maxrange: Option<i32>,
    /// NPC world tile at its first sighting, the anchor for flee and chase.
    start_tile: (i32, i32),
    /// Fresh game ticks spent on this event, including the latch tick.
    ticks: u32,
    outside_ticks: u8,
    flee_target: Option<(i32, i32)>,
    fallback_distance: Option<i32>,
    fallback_exhausted: bool,
    has_fled: bool,
    finished: bool,
    policy_logged: bool,
}
/// Per-slot random-event guardian state.
pub struct Guardian {
    /// A Talk-to went out and the dialog may still be open.
    pub in_flight: bool,
    /// The snapshot tick the last act ran on (one scan per game tick).
    pub last_tick: u64,
    /// NPC slot → wrong-talk cooldown expiry `now_ms`.
    pub cooldown: CooldownMap,
    /// Signature (kind+name) of the last detected event: the rising-edge
    /// key the `on_random` knock fires on once per new event.
    pub sig: Option<String>,
    /// Who handles the current event. Filled by the rising-edge knock
    /// (host-play's script); `Host` when no event, no script, or the
    /// script did not claim it.
    pub claim: RandomClaim,
    /// The NPC slot the in-flight dialog targets (the cooldown key).
    in_flight_index: Option<usize>,
    /// Continues sent for the in-flight dialog ([`MAX_CONTINUES`] cap).
    continues: u32,
    /// Newest chat sequence already scanned for wrong-talk lines, so a
    /// stale rejection line cannot re-bin a later NPC.
    chat_seen: i32,
    /// A non-dialog act is in flight: an op or walk was sent for the
    /// current event and it has not resolved (evade/plant/hazard/
    /// lamp-rub/lost-gear/lost-tool, plus the out-of-range walk before a
    /// Talk-to / Pick). Holds the slot like the dialog handle.
    acting: bool,
    /// The kind the in-flight non-dialog act belongs to.
    acting_kind: RandomKind,
    /// Evade: the tile the flee started from — the walk-back target once
    /// the threat despawns (rs2b0t).
    flee_from: Option<(i32, i32)>,
    /// Identity and chase-range latch for the current hostile event NPC.
    evade: Option<EvadeState>,
    /// Lost-tool: the handle was worn, so the reattached tool is re-wielded.
    tool_was_worn: bool,
    /// Lost-tool: the handle's base tool name (the handle name minus the
    /// "handle" suffix), the re-wield target after reattach.
    tool_handle_base: Option<String>,
    /// One bounded recovery per handle observation; failure releases the
    /// native Gatherer's missing-tool supply trip instead of holding forever.
    tool_deadline_ms: Option<u64>,
    tool_stalled: bool,
    /// Mime: the last emote anim seq the mime NPC showed.
    mime_last_seen: Option<i32>,
    /// Mime: the emote button went out for the open chat — no repeat
    /// press until the chat closes.
    mime_answered: bool,
    /// Box: the held-box count when the answer went out; the solver
    /// waits for it to drop (the answer consumed a box) before handling
    /// the next held box (rs2b0t waits on the count drop the same way).
    box_answer_count: Option<i32>,
    /// Lamp: Rub went out — do not Rub again until the skill IF closes.
    lamp_rubbed: bool,
    /// Lamp: Rub attempts sent in the current episode (re-Rubs after a
    /// missed skill IF, [`MAX_LAMP_RUBS`] cap; reset when a pause starts a
    /// new episode and when the redemption resolves).
    lamp_rubs: u32,
    /// Lamp: the skill button went out — the next tick on the IF presses
    /// confirm (2831).
    lamp_skill_sent: bool,
    /// Lamp: Confirm went out — the redemption is in flight (the server
    /// closes the IF, consumes the lamp, advances the skill and opens the
    /// award dialogue).
    lamp_confirmed: bool,
    /// Lamp: the configured skill's (xp, base) as the Confirm tick's
    /// snapshot read them — the reward is witnessed by the advance past
    /// this baseline, never by a later stale read.
    lamp_reward: Option<(i32, i32)>,
    /// Lamp: award-dialogue continues sent since Confirm
    /// ([`MAX_LAMP_DIALOGUE`] cap).
    lamp_dialog: u32,
    /// Lamp: ticks the current phase has waited without progress
    /// ([`MAX_LAMP_WAIT`] cap).
    lamp_wait: u32,
    /// Lamp: the lamp count when Confirm went out. One unstacked lamp
    /// leaves the pack per redemption, so the consume is a drop below this
    /// baseline; a sibling lamp left in the pack does not block it.
    lamp_confirm_count: Option<i32>,
    /// Lamp: fresh ticks left in a pause before the next Rub. Inert while
    /// it counts down (no act, no hold, no `ours`), so the script runs
    /// between Rub attempts.
    lamp_park: u32,
    /// Lamp: the redemption failed after Confirm (or the skill is
    /// unclickable) with the lamp still held — no act and no hold (the
    /// status row still detects it) until the lamp leaves the pack or
    /// `lamp_auto` is switched off. Confirm is never replayed.
    lamp_stalled: bool,
    /// Box: Open went out — do not Open again until the cube IF closes
    /// and the answer is consumed.
    box_opened: bool,
    /// Maze: the active solve state, None while trapped without a route.
    maze: Option<maze::MazeSolve>,
    /// Maze: solver ticks spent on this visit ([`maze::SOLVE_TICKS`] cap).
    maze_ticks: u32,
    /// Maze: the solver gave this visit up with the player still on the
    /// square — no act, no hold and no `ours` until the player leaves it,
    /// so the slot's native run sees the trap and blocks instead of
    /// staying frozen.
    maze_stalled: bool,
    /// Strange Plant's bounded server-authenticated ownership probe.
    plant: PlantProbe,
    /// Exact foreign/refused/timed-out identities still present in the scene.
    plant_ignored: Vec<PlantActor>,
    /// Per-slot fishing-gear held/lost history used to prove `LostGear`.
    gear_loss: GearLoss,
}

impl Default for Guardian {
    fn default() -> Self {
        Self::new()
    }
}
impl Guardian {
    pub fn new() -> Self {
        Self {
            in_flight: false,
            last_tick: 0,
            cooldown: HashMap::new(),
            sig: None,
            claim: RandomClaim::Host,
            in_flight_index: None,
            continues: 0,
            chat_seen: 0,
            acting: false,
            acting_kind: RandomKind::Dialog,
            flee_from: None,
            evade: None,
            tool_was_worn: false,
            tool_handle_base: None,
            tool_deadline_ms: None,
            tool_stalled: false,
            mime_last_seen: None,
            mime_answered: false,
            box_answer_count: None,
            lamp_rubbed: false,
            lamp_rubs: 0,
            lamp_skill_sent: false,
            lamp_confirmed: false,
            lamp_reward: None,
            lamp_dialog: 0,
            lamp_wait: 0,
            lamp_confirm_count: None,
            lamp_park: 0,
            lamp_stalled: false,
            box_opened: false,
            maze: None,
            maze_ticks: 0,
            maze_stalled: false,
            plant: PlantProbe::Idle,
            plant_ignored: Vec::new(),
            gear_loss: GearLoss::new(),
        }
    }

    /// One guardian pass per caller frame: detect + publish the status
    /// every frame, but act at most once per snapshot `tick` (the
    /// PLAYER_INFO game-tick edge). Talk-to runs only for a dialog event
    /// that is ours on an un-binned slot; an open dialog then continues
    /// via `continue_dialog` / `answer_choice` (first option), max
    /// [`MAX_CONTINUES`] — the continue is keyed to the in-flight handle,
    /// not to a fresh detect, so a despawned genie cannot stall the chat.
    /// A wrong-talk chat line bins that NPC slot for 45 s. Toggle off:
    /// never act, never hold, still detect+publish. `knock` is the
    /// rising-edge `on_random` arm: once per detected event (kind+name
    /// signature), when the caller supplies it. The script's claim normally
    /// gates act + hold (`Host` claims both; `Handle` leaves the script
    /// available); stored Fight/Ignore are forced to Guardian Flee until
    /// their script-layer handlers exist.
    pub fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        now_ms: u64,
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
    ) -> RandomStatus {
        self.tick_with_live_policies(
            driver,
            snap,
            settings,
            &GuardianLiveContext {
                event_policies: &settings.random_event_policies,
                game_data: None,
            },
            now_ms,
            knock,
        )
    }

    /// Host-play supplies the live policy map and selected content through
    /// this context, without copying either into the static profile.
    pub fn tick_with_live_policies<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        context: &GuardianLiveContext<'_>,
        now_ms: u64,
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
    ) -> RandomStatus {
        let tick = snap.tick() as u64;
        let fresh = self.last_tick != tick;
        let active = snap.ingame() && snap.scene_state() == SCENE_READY;
        if active {
            self.refresh_ignored_plants(snap);
            self.revalidate_plant(snap);
        } else {
            self.clear_plant();
            self.plant_ignored.clear();
        }
        self.gear_loss.observe(snap, now_ms);
        let mut ev = detect_ignoring_plants(
            snap,
            now_ms,
            &self.cooldown,
            &self.plant_ignored,
            Some(&self.gear_loss),
        );
        self.pin_plant_event(snap, &mut ev);
        self.refresh_evade(snap, &mut ev, context.game_data, fresh);

        if fresh && active {
            // Fresh chat only: a stale wrong-talk line must not re-bin a
            // later NPC the guardian talks to.
            let head = snap
                .chat_lines()
                .first()
                .map(|l| l.sequence)
                .unwrap_or(self.chat_seen);
            let wrong_talk = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| WRONG_TALK_MARKERS.iter().any(|w| l.text.contains(w)));
            let plant_growing = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| l.text.contains(PLANT_GROWING_MARKER));
            let plant_rejected = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| l.text.contains(PLANT_REJECTION_MARKER));
            let plant_before_chat = self.plant.clone();
            if self.in_flight && wrong_talk {
                if let Some(index) = self.in_flight_index {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.clear_handle();
            } else if !matches!(self.plant, PlantProbe::Idle) && plant_rejected {
                // Plant rejection is pinned to the complete actor identity;
                // a reused slot must not inherit this bin.
                self.ignore_current_plant();
            } else if matches!(self.plant, PlantProbe::AwaitingResponse { .. }) && plant_growing {
                self.authenticate_plant(now_ms);
            } else if matches!(self.plant, PlantProbe::Authenticated { .. }) && plant_growing {
                self.note_plant_growing(now_ms);
            } else if self.acting && self.acting_kind == RandomKind::Pick && plant_rejected {
                if let Some(index) = ev.as_ref().and_then(|e| e.npc_index) {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.acting = false;
            }
            self.chat_seen = head;
            // The dialog ended: the chat is closed and the NPC is gone.
            if self.in_flight && self.dialog_done(snap) {
                self.clear_handle();
            }
            if self.plant != plant_before_chat {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
            self.pin_plant_event(snap, &mut ev);
            // Rising-edge knock: ask the running script once per detected
            // event. A vanished event resets the claim to Host (the host
            // owns whatever appears next). No knock supplied → Host.
            // An inert leftover lamp (`lamp_auto` off) does not knock —
            // it is XP in the pack, not a handler latch.
            let sig = ev.as_ref().map(|e| format!("{:?}:{}", e.kind, e.name));
            if sig != self.sig {
                self.sig = sig;
                self.claim = match (&ev, knock) {
                    (Some(ev), Some(knock))
                        if !(ev.kind == RandomKind::Lamp && !settings.lamp_auto) =>
                    {
                        knock(ev)
                    }
                    _ => RandomClaim::Host,
                };
            }
        }
        let unsupported_policy = ev.as_ref().and_then(|event| {
            if event.kind != RandomKind::Evade {
                return None;
            }
            let policy = context
                .event_policies
                .get(&event.name)
                .copied()
                .unwrap_or_default();
            (policy != RandomEventPolicy::Flee).then_some((
                event.npc_index?,
                event.name.as_str(),
                policy,
            ))
        });
        if let Some((index, name, policy)) = unsupported_policy {
            // Fight/Ignore have no script-layer arm yet. Keep the Guardian in
            // control so compiled scripts do not freeze behind `ours`.
            self.claim = RandomClaim::Host;
            if let Some(state) = self
                .evade
                .as_mut()
                .filter(|state| state.index == index && state.name.eq_ignore_ascii_case(name))
            {
                if !state.policy_logged {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Info,
                        "evade: stored {policy:?} policy for {name} is unavailable; treating it as Flee"
                    );
                    state.policy_logged = true;
                }
            }
        }
        if !has_lost_tool(snap) || !settings.random_events {
            self.tool_deadline_ms = None;
            self.tool_stalled = false;
        }
        let inert_tool = self.tool_stalled
            && ev
                .as_ref()
                .is_some_and(|event| event.kind == RandomKind::LostTool);

        // A pause counts down once per game tick; the Rub resumes when it
        // reaches zero.
        if fresh {
            self.lamp_park = self.lamp_park.saturating_sub(1);
        }
        // A stalled or paused redemption unlocks on the two operator-visible
        // state changes: the lamp left the pack, or lamp auto went off (the
        // next auto-on redemption starts clean).
        if (self.lamp_stalled || self.lamp_park > 0) && (!lamp_held(snap) || !settings.lamp_auto) {
            self.clear_lamp();
        }
        // An inert lamp is detect-only: `lamp_auto` off, or a redemption
        // that gave up or is paused between Rubs. No act, no hold, no `ours`
        // — the script must not stay frozen behind a lamp the host is not
        // driving right now.
        let inert_lamp = ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp)
            && (!settings.lamp_auto || self.lamp_inert());
        if inert_lamp {
            // Drop a previous auto-on latch the moment the operator
            // turns lamp auto off with the lamp still in inv, or the
            // redemption gave up with the lamp still held.
            self.acting = false;
        }
        // A trap square the host will not solve (random events off, or a Maze
        // visit it gave up) is inert like a stalled lamp: the square stays
        // detected, but nothing holds the slot on a trap the guardian is not
        // walking out of.
        let on_maze = ev.as_ref().is_some_and(|e| e.kind == RandomKind::Maze);
        if !on_maze {
            self.maze_ticks = 0;
            self.maze_stalled = false;
        }
        let on_trap_square = on_maze || ev.as_ref().is_some_and(|e| e.kind == RandomKind::Mime);
        let inert_trap =
            on_trap_square && (!settings.random_events || (on_maze && self.maze_stalled));

        if fresh
            && active
            && settings.random_events
            && self.claim == RandomClaim::Host
            && !inert_lamp
            && !inert_tool
            && !inert_trap
        {
            let plant_before_act = self.plant.clone();
            let ignored_before_act = self.plant_ignored.len();
            self.act(driver, snap, ev.as_ref(), settings, now_ms);
            if self.plant != plant_before_act || self.plant_ignored.len() != ignored_before_act {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
        }
        // `step_pick` may have timed out or seen a refused send. Reflect
        // that release in this same status frame instead of publishing a
        // stale authenticated `ours`.
        self.pin_plant_event(snap, &mut ev);
        self.last_tick = tick;

        // `act` may have paused or stalled the redemption just above: a lamp
        // the host is not driving this tick is inert in the same tick, so the
        // script's `EventSignal.pending` (hold OR ours) clears immediately.
        let inert_lamp = inert_lamp
            || (self.lamp_inert() && ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp));
        let inert_tool = inert_tool
            || (self.tool_stalled
                && ev
                    .as_ref()
                    .is_some_and(|event| event.kind == RandomKind::LostTool));
        let inert_trap = inert_trap || (on_maze && self.maze_stalled);
        let cooldown = ev
            .as_ref()
            .and_then(|e| e.npc_index)
            .is_some_and(|i| binned(i, now_ms, &self.cooldown));
        RandomStatus {
            kind: ev.as_ref().map(|e| e.kind),
            name: ev.as_ref().map(|e| e.name.clone()),
            // Inert leftover lamp still detects for the status row, but
            // must not publish ours — EventSignal.pending is hold OR ours.
            ours: !inert_lamp
                && !inert_tool
                && !inert_trap
                && ev.as_ref().map(|e| e.ours).unwrap_or(false),
            handling: self.in_flight,
            hold: settings.random_events
                && self.claim == RandomClaim::Host
                && !inert_trap
                && (self.in_flight
                    || self.acting
                    || ev.as_ref().is_some_and(|e| is_trapped(e.kind))),
            toggle: settings.random_events,
            claim: self.claim,
            cooldown,
        }
    }

    /// Keep a hostile event latched from live NPC facts, not hit feedback.
    /// `target` is `face_entity` decoded into NPC/player slots and matched to
    /// `self_slot` (`api/src/snapshot/decode.rs:118,1384-1411`); this is
    /// targeting state rather than hit feedback, so zero damage does not clear it.
    /// Latch the NPC's first world tile so flee distance and the engine chase
    /// stop use the same origin (`Npc.ts:661-670`). Missing NPCs beyond the
    /// 15-tile client view (`build.ts:57`) remain latched after release until
    /// a different event appears or the NPC is visible and no longer engaged.
    fn refresh_evade(
        &mut self,
        snap: &GameSnapshot,
        ev: &mut Option<DetectedRandom>,
        game_data: Option<&SelectedGameData>,
        fresh: bool,
    ) {
        if let Some(mut state) = self.evade.take() {
            let actor = snap.npcs().iter().find(|npc| npc.index == state.index);
            let same_actor = actor.is_some_and(|npc| {
                npc.r#type == state.npc_type
                    && npc
                        .name
                        .as_deref()
                        .is_some_and(|name| name.eq_ignore_ascii_case(&state.name))
            });
            let engaged = actor.is_some_and(|npc| {
                event_npc_targets_player(npc, snap.self_slot())
                    || (npc.target.is_none() && npc.in_combat)
            });
            let other_event = ev.as_ref().is_some_and(|event| {
                event.kind != RandomKind::Evade
                    || event.npc_index != Some(state.index)
                    || !event.name.eq_ignore_ascii_case(&state.name)
            });
            let beyond_view = snap
                .tile()
                .map(|(x, z, _)| (x, z))
                .is_some_and(|player| cheb(player, state.start_tile) > NPC_VIEW_RADIUS);

            if state.finished {
                if !other_event && engaged {
                    *ev = None;
                    self.evade = Some(state);
                    return;
                }
                if !other_event && actor.is_none() {
                    *ev = None;
                    self.evade = Some(state);
                    return;
                }
            } else if same_actor && engaged {
                if fresh {
                    state.ticks = state.ticks.saturating_add(1);
                }
                let outside = snap.tile().map(|(x, z, _)| (x, z)).is_some_and(|player| {
                    state
                        .maxrange
                        .is_some_and(|range| outside_chase_range(player, state.start_tile, range))
                });
                if outside {
                    if fresh {
                        state.outside_ticks = state.outside_ticks.saturating_add(1);
                    }
                } else {
                    state.outside_ticks = 0;
                }
                if state.outside_ticks > 1 {
                    state.finished = true;
                    state.flee_target = None;
                    *ev = None;
                    self.evade = Some(state);
                    return;
                }
                if state.ticks >= MAX_EVADE_TICKS {
                    self.give_up_evade(&mut state);
                    *ev = None;
                    self.evade = Some(state);
                    return;
                }
                let already_detected = ev.as_ref().is_some_and(|event| {
                    event.kind == RandomKind::Evade
                        && event.npc_index == Some(state.index)
                        && event.name.eq_ignore_ascii_case(&state.name)
                });
                if !already_detected {
                    *ev = Some(DetectedRandom {
                        kind: RandomKind::Evade,
                        name: state.name.clone(),
                        ours: true,
                        npc_index: Some(state.index),
                    });
                }
                self.evade = Some(state);
                return;
            } else if actor.is_none() && state.has_fled && beyond_view && !other_event {
                if fresh {
                    state.ticks = state.ticks.saturating_add(1);
                }
                let outside = snap.tile().map(|(x, z, _)| (x, z)).is_some_and(|player| {
                    state
                        .maxrange
                        .is_some_and(|range| outside_chase_range(player, state.start_tile, range))
                });
                if outside {
                    if fresh {
                        state.outside_ticks = state.outside_ticks.saturating_add(1);
                    }
                } else {
                    state.outside_ticks = 0;
                }
                if state.outside_ticks > 1 {
                    state.finished = true;
                    state.flee_target = None;
                    *ev = None;
                } else if state.ticks >= MAX_EVADE_TICKS {
                    self.give_up_evade(&mut state);
                    *ev = None;
                } else {
                    *ev = Some(DetectedRandom {
                        kind: RandomKind::Evade,
                        name: state.name.clone(),
                        ours: true,
                        npc_index: Some(state.index),
                    });
                }
                self.evade = Some(state);
                return;
            }
        }

        let Some(event) = ev.as_ref().filter(|event| event.kind == RandomKind::Evade) else {
            return;
        };
        let Some(index) = event.npc_index else {
            return;
        };
        let Some(npc) = snap.npcs().iter().find(|npc| npc.index == index) else {
            return;
        };
        if !event_npc_targets_player(npc, snap.self_slot()) {
            return;
        }
        let maxrange = npc
            .r#type
            .and_then(|id| i32::try_from(id).ok())
            .and_then(|id| game_data?.npc_name(id))
            .map(|row| row.maxrange);
        let state = EvadeState {
            index,
            npc_type: npc.r#type,
            name: event.name.clone(),
            maxrange,
            start_tile: (npc.tile.x, npc.tile.z),
            ticks: if fresh { 1 } else { 0 },
            outside_ticks: 0,
            flee_target: None,
            fallback_distance: None,
            fallback_exhausted: false,
            has_fled: false,
            finished: maxrange.is_none(),
            policy_logged: false,
        };
        if state.finished {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "evade: no selected content range for {}; not fleeing",
                state.name
            );
            *ev = None;
        }
        self.evade = Some(state);
    }

    /// A bounded flee failed: leave no host hold or walk-back latch behind.
    fn give_up_evade(&mut self, state: &mut EvadeState) {
        api::host_log!(
            Category::RandomEvent,
            Level::Warn,
            "evade: {} did not escape within {MAX_EVADE_TICKS} ticks; giving up and releasing the slot",
            state.name
        );
        state.finished = true;
        state.flee_target = None;
        state.fallback_distance = None;
        state.fallback_exhausted = true;
        self.acting = false;
        self.flee_from = None;
    }

    fn clear_plant(&mut self) {
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    fn ignore_current_plant(&mut self) {
        if let Some(actor) = self.plant.actor().cloned() {
            self.ignore_plant(actor);
        }
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    fn ignore_plant(&mut self, actor: PlantActor) {
        if let Some(existing) = self
            .plant_ignored
            .iter_mut()
            .find(|existing| existing.slot == actor.slot)
        {
            *existing = actor;
        } else {
            self.plant_ignored.push(actor);
        }
    }

    /// Remove a bin only after its slot is observed empty. If a structurally
    /// different pickable actor appears in that slot without an observed gap,
    /// replace the bin so the ambiguous replacement inherits no auth.
    fn refresh_ignored_plants(&mut self, snap: &GameSnapshot) {
        self.plant_ignored.retain_mut(|ignored| {
            let Some(current) =
                npc_by_index(snap.npcs(), ignored.slot).and_then(PlantActor::from_npc)
            else {
                return false;
            };
            *ignored = current;
            true
        });
    }

    fn authenticate_plant(&mut self, now_ms: u64) {
        let PlantProbe::AwaitingResponse { actor, .. } = &self.plant else {
            return;
        };
        self.plant = PlantProbe::Authenticated {
            actor: actor.clone(),
            retry_at_ms: now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS),
            deadline_ms: now_ms.saturating_add(PLANT_AUTH_TIMEOUT_MS),
            continues: 0,
        };
    }

    fn note_plant_growing(&mut self, now_ms: u64) {
        if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
        }
    }

    /// Drop authentication as soon as any structural actor field changes.
    /// If a different pickable actor reused the slot, bind an ignore to the
    /// replacement so stale state can never turn into a retarget.
    fn revalidate_plant(&mut self, snap: &GameSnapshot) {
        let Some(expected) = self.plant.actor().cloned() else {
            return;
        };
        let current = npc_by_index(snap.npcs(), expected.slot).and_then(PlantActor::from_npc);
        if current.as_ref() == Some(&expected) {
            return;
        }

        if let Some(replacement) = current {
            self.ignore_plant(replacement);
        }
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    /// Keep an in-flight probe pinned to its actor even after the player
    /// moves away and publish server authentication as `ours`. Established
    /// non-plant random kinds still preempt it.
    fn pin_plant_event(&mut self, snap: &GameSnapshot, ev: &mut Option<DetectedRandom>) {
        match self.plant.clone() {
            PlantProbe::AwaitingResponse { actor, .. }
            | PlantProbe::Authenticated { actor, .. } => {
                if ev.as_ref().is_some_and(|e| e.kind != RandomKind::Pick) {
                    return;
                }
                let Some(npc) = npc_by_index(snap.npcs(), actor.slot) else {
                    *ev = None;
                    return;
                };
                let display_name = snap
                    .local_player()
                    .and_then(|lp| lp.player.actor.name.clone());
                let authenticated = matches!(self.plant, PlantProbe::Authenticated { .. });
                *ev = Some(DetectedRandom {
                    kind: RandomKind::Pick,
                    name: PICK_NAME.to_string(),
                    ours: authenticated
                        || owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()),
                    npc_index: Some(actor.slot),
                });
            }
            PlantProbe::Idle => {}
        }
    }

    /// One send per game tick. The in-flight dialog continues on its own
    /// handle; everything else goes through the solver machine below.
    /// Refuses silently when the wire layer says no (not ingame, stale
    /// target, chat closed), so the machine is driven by the snapshot,
    /// not by error paths.
    fn act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        now_ms: u64,
    ) {
        // The in-flight dialog continues on its own handle, independent
        // of a fresh detect: the genie can despawn while the chat is
        // still open, and detect would then return None.
        if self.in_flight {
            if self.continues >= MAX_CONTINUES {
                self.clear_handle();
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            let result = if snap.chat_options().is_empty() {
                ix.continue_dialog(None)
            } else {
                ix.answer_choice(1)
            };
            match result {
                SendResult::Sent { .. } => {
                    self.continues += 1;
                }
                // Spec: stop when continue refuses. Keep one tick after
                // Talk-to for chat to open; clear once chat is still
                // closed and a continue/answer has refused.
                SendResult::Refused { .. } => {
                    if !chat_is_open(snap) {
                        if self.continues == 0 {
                            self.continues = 1;
                        } else {
                            self.clear_handle();
                        }
                    }
                }
            }
            return;
        }
        self.act_solver(driver, snap, ev, settings, now_ms);
    }

    /// The non-dialog act machine: drive the in-flight act to completion,
    /// or start one on a fresh event (the trapped kinds and the dialog
    /// handle never get here). `acting` latches the first send; the
    /// resolution rules are per kind — the walk-to-range kinds resolve
    /// when the event's signature changes (the NPC/plant/gear/tool left
    /// the scene), hazard resolves when the loc is no longer underfoot,
    /// lamp resolves when the lamp leaves the inventory.
    fn act_solver<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        now_ms: u64,
    ) {
        // A confirmed lamp redemption outlives its own detect: the server
        // consumes the lamp and opens the award dialogue in the same step,
        // so the drain must run while `detect` already shows nothing (or a
        // newly spawned event). It takes the slot, like the dialog handle,
        // until it completes or gives up.
        if self.acting && self.acting_kind == RandomKind::Lamp && self.lamp_confirmed {
            self.step_lamp(driver, snap, settings);
            return;
        }
        let Some(ev) = ev else {
            if self.acting {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind == RandomKind::Evade && self.evade.is_none(),
                );
            }
            return;
        };
        if !ev.ours && ev.kind != RandomKind::Pick {
            if self.acting {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind == RandomKind::Evade && self.evade.is_none(),
                );
            }
            return;
        }
        if self.acting {
            let same_evade = self.acting_kind != RandomKind::Evade
                || self
                    .evade
                    .as_ref()
                    .is_some_and(|state| ev.npc_index == Some(state.index));
            if ev.kind != self.acting_kind || !same_evade {
                self.resolve(
                    driver,
                    snap,
                    self.acting_kind == RandomKind::Evade && self.evade.is_none(),
                );
            } else {
                self.step_act(driver, snap, settings, ev, now_ms);
            }
            return;
        }
        self.acting = true;
        self.acting_kind = ev.kind;
        match ev.kind {
            RandomKind::Evade => {
                // The pre-flee tile: the walk-back target after despawn.
                self.flee_from = snap.tile().map(|(x, z, _)| (x, z));
            }
            RandomKind::LostTool => {
                self.tool_deadline_ms
                    .get_or_insert(now_ms.saturating_add(LOST_TOOL_BOUND_MS));
                // Remember whether the handle was worn so the reattached
                // tool is re-wielded.
                self.tool_was_worn = snap
                    .equipment()
                    .iter()
                    .any(|i| is_tool_handle(i.def.name.as_deref()));
                self.tool_handle_base = snap
                    .equipment()
                    .iter()
                    .chain(snap.inventory().iter())
                    .find(|i| is_tool_handle(i.def.name.as_deref()))
                    .and_then(|i| i.def.name.as_deref())
                    .map(|n| {
                        n.trim()
                            .trim_end_matches("handle")
                            .trim_end_matches("Handle")
                            .trim()
                            .to_lowercase()
                    });
            }
            _ => {}
        }
        self.step_act(driver, snap, settings, ev, now_ms);
    }

    /// One step of the in-flight (or freshly started) non-dialog act,
    /// per kind.
    fn step_act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        match self.acting_kind {
            RandomKind::Dialog => self.step_dialog(driver, snap, ev, now_ms),
            RandomKind::Pick => self.step_pick(driver, snap, ev, now_ms),
            RandomKind::Evade => self.step_evade(driver, snap, ev),
            RandomKind::Hazard => self.step_hazard(driver, snap),
            RandomKind::Lamp => self.step_lamp(driver, snap, settings),
            RandomKind::LostGear => self.step_lost_gear(driver, snap, ev),
            RandomKind::LostTool => self.step_lost_tool(driver, snap, now_ms),
            RandomKind::Mime => self.step_mime(driver, snap),
            RandomKind::Box => self.step_box(driver, snap),
            RandomKind::Maze => self.step_maze(driver, snap),
        }
    }

    /// The event resolved (gone, changed kind, or no longer ours): clear
    /// the in-flight latch and finish the kind's tail work — the evade
    /// walk-back toward the pre-flee tile, and the lost-tool re-wield of
    /// a handle that was worn.
    fn resolve<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, walk_back: bool) {
        if self.acting_kind == RandomKind::Evade && walk_back {
            if let Some((fx, fz)) = self.flee_from.take() {
                walk(driver, fx, fz);
            }
        } else if self.acting_kind == RandomKind::Evade {
            self.flee_from = None;
        }
        // Content handles omit the tier: "Axe handle" can restore "Rune axe".
        // Re-wield only a formerly worn tool; Gatherer also validates its gates.
        if self.acting_kind == RandomKind::LostTool && self.tool_was_worn {
            if let Some(base) = self
                .tool_handle_base
                .as_deref()
                .and_then(|n| n.rsplit(' ').next())
            {
                if let Some(tool) = snap.inventory().iter().find(|i| {
                    i.def.name.as_deref().is_some_and(|name| {
                        name.trim()
                            .rsplit(' ')
                            .next()
                            .is_some_and(|last| last.eq_ignore_ascii_case(base))
                    })
                }) {
                    let mut ix = Interactions::new(snap, driver);
                    let _ = ix.wear(tool.def.id);
                }
            }
        }
        self.acting = false;
        self.flee_from = None;
        self.tool_was_worn = false;
        self.tool_handle_base = None;
        self.mime_last_seen = None;
        self.mime_answered = false;
        self.box_answer_count = None;
        self.clear_lamp();
        self.box_opened = false;
        self.maze = None;
        self.plant = PlantProbe::Idle;
    }

    /// Talk-to, gated on range: an NPC further than Chebyshev 1 gets a
    /// ground walk to its tile first (the same `try_move` the nav bot
    /// uses); the Talk-to fires once the walk closes in.
    fn step_dialog<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
        // A slot binned this pass (the wrong-talk above) or earlier is
        // not re-engaged; detect skips binned slots, this guards the
        // same-tick bin.
        if binned(index, now_ms, &self.cooldown) {
            self.acting = false;
            return;
        }
        // `ev.npc_index` is the client NPC slot (`NpcView.index`), not
        // the dense view-vec position, so the lookup must scan by slot.
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.acting = false;
            return;
        };
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        if cheb((px, pz), (npc.tile.x, npc.tile.z)) > 1 {
            walk(driver, npc.tile.x, npc.tile.z);
            return;
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".to_string())) {
            SendResult::Sent { .. } => {
                self.in_flight = true;
                self.in_flight_index = Some(index);
                self.continues = 0;
                self.acting = false;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// Drive either the established hard-owner behavior or the bounded
    /// adjacent server probe. No featureless actor is walked to; only the
    /// exact actor authenticated by the growing response may later be chased.
    fn step_pick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.acting = false;
            return;
        };
        let display_name = snap
            .local_player()
            .and_then(|lp| lp.player.actor.name.clone());
        let Some(actor) = PlantActor::from_npc(npc) else {
            self.clear_plant();
            return;
        };
        let distance = cheb((px, pz), (npc.tile.x, npc.tile.z));

        // Preserve the existing hard evidence path, including its allowed
        // walk to range. It does not need or inherit probe authentication.
        if owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()) {
            self.plant = PlantProbe::Idle;
            self.plant_ignored.retain(|ignored| ignored != &actor);
            if distance > 1 {
                walk(driver, npc.tile.x, npc.tile.z);
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            if matches!(
                ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())),
                SendResult::Refused { .. }
            ) {
                self.acting = false;
            }
            return;
        }

        if !featureless_pickable_plant(npc) {
            self.ignore_current_plant();
            return;
        }

        match self.plant.clone() {
            PlantProbe::Idle => {
                if distance > 1 {
                    self.acting = false;
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        self.plant = PlantProbe::AwaitingResponse {
                            actor,
                            deadline_ms: now_ms.saturating_add(PLANT_PROBE_TIMEOUT_MS),
                        };
                    }
                    SendResult::Refused { .. } => {
                        self.ignore_plant(actor);
                        self.plant = PlantProbe::Idle;
                        self.acting = false;
                    }
                }
            }
            PlantProbe::AwaitingResponse {
                actor: expected,
                deadline_ms,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.plant = PlantProbe::Idle;
                    self.acting = false;
                } else if now_ms >= deadline_ms {
                    self.ignore_current_plant();
                }
            }
            PlantProbe::Authenticated {
                actor: expected,
                retry_at_ms,
                deadline_ms,
                continues,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.plant = PlantProbe::Idle;
                    self.acting = false;
                    return;
                }
                if now_ms >= deadline_ms || continues >= MAX_PLANT_CONTINUES {
                    self.ignore_current_plant();
                    return;
                }
                if chat_is_open(snap) {
                    let mut ix = Interactions::new(snap, driver);
                    match ix.continue_dialog(None) {
                        SendResult::Sent { .. } => {
                            if let PlantProbe::Authenticated { continues, .. } = &mut self.plant {
                                *continues += 1;
                            }
                        }
                        SendResult::Refused { .. } => self.ignore_current_plant(),
                    }
                    return;
                }
                if now_ms < retry_at_ms {
                    return;
                }
                if distance > 1 {
                    if !walk(driver, npc.tile.x, npc.tile.z) {
                        self.ignore_current_plant();
                    } else if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
                        *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                    }
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
                            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                        }
                    }
                    SendResult::Refused { .. } => self.ignore_current_plant(),
                }
            }
        }
    }

    /// Flee to `maxrange + 2` from the latched NPC tile, one tile beyond the
    /// engine's maxrange+1 chase stop (`Npc.ts:661-670`). When the exact ring
    /// is blocked, use nearest-path walks and then nearer rings. Retry after
    /// the route ends while the same NPC still targets the player.
    fn step_evade<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, _ev: &DetectedRandom) {
        let Some((index, maxrange, start_tile, finished, sent, fallback_exhausted)) =
            self.evade.as_ref().map(|state| {
                (
                    state.index,
                    state.maxrange,
                    state.start_tile,
                    state.finished,
                    state.flee_target.is_some(),
                    state.fallback_exhausted,
                )
            })
        else {
            self.acting = false;
            return;
        };
        if finished || fallback_exhausted {
            return;
        }
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let player = (px, pz);
        let Some(maxrange) = maxrange else {
            return;
        };
        if outside_chase_range(player, start_tile, maxrange) {
            return;
        }
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            return;
        };
        if !event_npc_targets_player(npc, snap.self_slot()) {
            return;
        }
        let moving = snap
            .local_player()
            .is_some_and(|local| local.player.actor.moving);
        if sent {
            if moving {
                return;
            }
            if let Some(state) = self.evade.as_mut() {
                state.flee_target = None;
                state.fallback_distance = None;
                state.fallback_exhausted = false;
            }
        }
        let distance = maxrange.max(0).saturating_add(2);
        let state = self.evade.as_mut().expect("latched evade state");
        if state.fallback_distance.is_none() {
            for (x, z) in flee_candidates_away(start_tile, player, distance) {
                if walk(driver, x, z) {
                    state.flee_target = Some((x, z));
                    state.has_fled = true;
                    return;
                }
            }
            state.fallback_distance = Some(distance);
        }
        let fallback_distance = state.fallback_distance.expect("fallback ring is set");
        for (x, z) in flee_candidates_away(start_tile, player, fallback_distance) {
            if walk_nearest(driver, x, z) {
                state.flee_target = Some((x, z));
                state.fallback_distance = None;
                state.has_fled = true;
                return;
            }
        }
        if fallback_distance > 1 {
            state.fallback_distance = Some(fallback_distance - 1);
        } else {
            state.fallback_distance = None;
            state.fallback_exhausted = true;
        }
    }

    /// Step off a hazard underfoot: use the hazard's posted tile as the
    /// threat and the frozen four-tile step ring (`RandomEvents.ts:607-614`).
    /// Stop once no hazard loc is within Chebyshev 2; the loc may still
    /// linger elsewhere in the loaded scene.
    fn step_hazard<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let Some(hazard) = snap.locs().iter().find(|loc| {
            is_hazard_loc_name(loc.name.as_deref()) && cheb((px, pz), (loc.tile.x, loc.tile.z)) <= 2
        }) else {
            self.acting = false;
            return;
        };
        for (x, z) in hazard_flee_candidates((px, pz), (hazard.tile.x, hazard.tile.z)) {
            if walk(driver, x, z) {
                break;
            }
        }
    }

    /// Copy the mime's performance: watch the mime NPC's anim; when the
    /// emote chat (6543) opens, press the button for the last seen
    /// emote, once per chat-open (rs2b0t `performMimeStage`). The
    /// trapped hold is the square's; the act ends when the player leaves
    /// the stage.
    fn step_mime<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        if !on_mime_square(snap) {
            self.acting = false;
            return;
        }
        // Watch the mime NPC (rs2b0t watches every frame).
        for npc in snap.npcs() {
            if npc
                .name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case("mime"))
                && mime_answer(npc.animation).is_some()
            {
                self.mime_last_seen = Some(npc.animation);
            }
        }
        // Emote chat up: answer with the last seen emote, then wait for
        // the chat to close (a still-open chat must not re-press).
        if snap.modals().chat == MIME_IF_ROOT {
            if !self.mime_answered {
                if let Some(answer) = self.mime_last_seen.and_then(mime_answer) {
                    press(driver, MIME_IF_BUTTONS[answer]);
                    self.mime_answered = true;
                }
            }
            return;
        }
        self.mime_answered = false;
    }

    /// Solve a held Strange box (rs2b0t `solveAllBoxes`): Open the box,
    /// read the cube question + three obj models, press the matching
    /// answer button, wait for one box to be consumed, then repeat while
    /// the inventory holds a box. Unknown question / missing model →
    /// fail closed: no click, the trapped hold stays.
    fn step_box<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        // Total held quantity (rs2b0t `Inventory.count('Strange box')`:
        // the box can sit as one row of a multi-box stack).
        let count: i32 = snap
            .inventory()
            .iter()
            .filter(|i| i.def.id == STRANGE_BOX_OBJ)
            .map(|i| i.count.max(0))
            .sum();
        if count == 0 {
            self.acting = false;
            return;
        }
        // An answer went out: wait for one box to be consumed before
        // acting again (rs2b0t waits on the count drop).
        if let Some(before) = self.box_answer_count {
            if count >= before {
                return;
            }
            self.box_answer_count = None;
            self.box_opened = false;
        }
        if snap.modals().main == CUBE_IF_ROOT {
            let ctx = ReadContext::new(snap);
            let question = ctx.component_text(CUBE_IF_QUESTION).unwrap_or("");
            let models = CUBE_IF_MODELS.map(|id| ctx.component_model_obj_id(id));
            if models.iter().any(|m| m.is_none()) {
                return;
            }
            let Some(answer) = solve_cube(question, models) else {
                return;
            };
            self.box_answer_count = Some(count);
            press(driver, CUBE_IF_BUTTONS[answer]);
            return;
        }
        if self.box_opened {
            return;
        }
        let Some(held) = snap
            .inventory()
            .iter()
            .find(|i| i.def.id == STRANGE_BOX_OBJ)
        else {
            self.acting = false;
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(held), ActionSpec::Label("Open".to_string())) {
            SendResult::Sent { .. } => {
                self.box_opened = true;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// One maze solver step per tick (rs2b0t `solveMaze`): solve the
    /// route from the observed tile, then drive the door / shrine phase
    /// machine. No route → log and keep the trapped hold (never replay a
    /// different spawn's route). The hold lifts on its own once the
    /// player is no longer on the maze square; a visit still on it after
    /// [`maze::SOLVE_TICKS`] solver ticks is given up (inert, no hold).
    fn step_maze<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, level)) = snap.tile() else {
            self.acting = false;
            return;
        };
        if api::random::trapped_area(px, pz, level) != Some(RandomKind::Maze) {
            self.acting = false;
            return;
        }
        self.maze_ticks += 1;
        if self.maze_ticks > maze::SOLVE_TICKS {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "maze: still inside after {} solver ticks at ({px},{pz}); giving the Maze up and releasing the slot",
                maze::SOLVE_TICKS
            );
            self.maze = None;
            self.maze_stalled = true;
            self.acting = false;
            return;
        }
        if self.maze.is_none() {
            let me = (px, pz);
            match maze::select_route(maze::graph(), me) {
                Some(doors) => {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Info,
                        "maze: spawn ({px},{pz}) -> {} doors, first ({},{})",
                        doors.len(),
                        doors[0].0,
                        doors[0].1
                    );
                    self.maze = Some(maze::MazeSolve::new(doors));
                }
                None => {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "maze: no route solvable from ({px},{pz}); the layout does not reach the shrine from here"
                    );
                }
            }
            return;
        }
        let keep = step_maze_phase(
            self.maze.as_mut().expect("checked above"),
            driver,
            snap,
            (px, pz),
        );
        if !keep {
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "maze: pass gave up; restarting the route from ({px},{pz})"
            );
            self.maze = None;
        }
    }

    /// Lamp auto-use, driven to a real redemption: Rub (held op 1,
    /// `opheld1`), wait for the skill IF (2808), press the vault
    /// `lamp_skill` button, then Confirm (2831) — after which the server
    /// runs `xplamp_confirm`: it closes the IF, consumes ONE lamp,
    /// `stat_advance`s the chosen skill and opens the award `mesbox`. A
    /// redemption completes on the count drop below the Confirm-tick count,
    /// the observed reward and the drained award dialogue; the next lamp in
    /// the pack then gets its own Rub. A Rub whose skill IF never opens is
    /// paused and re-Rubbed from a fresh snapshot (a resolving script burial
    /// can shift the lamp out from under the Rub); after [`MAX_LAMP_RUBS`]
    /// misses the episode pauses for [`LAMP_STALL_COOLDOWN`] and starts over.
    /// A refused Confirm (lamp still held) or an unknown skill latches the
    /// lamp until it leaves the pack or `lamp_auto` goes off. Confirm is
    /// never replayed. `lamp_auto` off keeps the 0.1.2 behavior (detect, no
    /// op, no hold).
    fn step_lamp<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        if !settings.lamp_auto {
            // Detect-only: tick() already skipped act + ours. Keep the
            // latch down if we still land here (live toggle mid-step).
            self.clear_lamp();
            self.acting = false;
            return;
        }
        let lamp_here = lamp_held(snap);
        if self.lamp_confirmed {
            // Confirm is out: the redemption is in flight.
            self.step_lamp_redeem(driver, snap, settings);
            return;
        }
        if !lamp_here {
            // The lamp left the pack without a Confirm of ours (another
            // actor consumed it): nothing left to drive.
            self.clear_lamp();
            self.acting = false;
            return;
        }
        if snap.modals().main == LAMP_IF_ROOT {
            self.lamp_wait = 0;
            if !self.lamp_skill_sent {
                let Some(btn) = lamp_skill_button(&settings.lamp_skill) else {
                    // An unknown `lamp_skill` cannot be clicked at all.
                    // No click (fail closed) — and no endless hold behind
                    // a lamp the host will not redeem.
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "lamp: unknown skill {:?}",
                        settings.lamp_skill
                    );
                    self.stall_lamp();
                    return;
                };
                press(driver, btn);
                self.lamp_skill_sent = true;
                return;
            }
            // The skill press has settled: confirm, and take the reward
            // baseline from *this* tick's snapshot.
            self.lamp_reward = stat_by_name(snap, &settings.lamp_skill).map(|s| (s.xp, s.base));
            self.lamp_dialog = 0;
            self.lamp_wait = 0;
            self.lamp_confirmed = true;
            self.lamp_confirm_count = Some(lamp_count(snap));
            press(driver, LAMP_IF_CONFIRM);
            return;
        }
        self.lamp_skill_sent = false;
        if self.lamp_rubbed {
            // Waiting for the skill IF to open after the Rub.
            self.lamp_wait += 1;
            if self.lamp_wait > MAX_LAMP_WAIT {
                if self.lamp_rubs < MAX_LAMP_RUBS {
                    // The Rub may have hit a stale slot (a script burial
                    // resolving after the snapshot it was built from
                    // compacts the pack and shifts the lamp, so the server
                    // ignores the op and no IF opens). Pause, then re-resolve
                    // and re-Rub from a fresh snapshot. Confirm is never
                    // replayed.
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Info,
                        "lamp: the skill interface never opened; re-rubbing (rub {} of {})",
                        self.lamp_rubs + 1,
                        MAX_LAMP_RUBS,
                    );
                    self.pause_lamp(LAMP_RETRY_COOLDOWN);
                } else {
                    api::host_log!(
                        Category::RandomEvent,
                        Level::Warn,
                        "lamp: the skill interface never opened in {} rubs; pausing {} ticks",
                        MAX_LAMP_RUBS,
                        LAMP_STALL_COOLDOWN,
                    );
                    self.lamp_rubs = 0;
                    self.pause_lamp(LAMP_STALL_COOLDOWN);
                }
            }
            return;
        }
        let Some(lamp) = snap.inventory().iter().find(|i| i.def.id == LAMP_OBJ) else {
            self.acting = false;
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(lamp), ActionSpec::Label("Rub".to_string())) {
            SendResult::Sent { .. } => {
                self.lamp_rubbed = true;
                self.lamp_wait = 0;
                self.lamp_rubs += 1;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// The post-Confirm half of [`Guardian::step_lamp`]. The redemption is
    /// complete when the pack count dropped below the Confirm-tick count
    /// (one lamp consumed), the chosen skill's stat advanced past the
    /// Confirm-tick baseline, and the award dialogue drained. The lamp
    /// leaving the pack alone is not completion, and a sibling lamp still in
    /// the pack does not block it: a completed redemption clears the per-lamp
    /// state so the next lamp is rubbed. Nothing here replays a confirmation
    /// or drains a dialogue the Confirm did not open.
    fn step_lamp_redeem<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        let consumed = self
            .lamp_confirm_count
            .is_some_and(|before| lamp_count(snap) < before);
        let rewarded = reward_landed(snap, &settings.lamp_skill, self.lamp_reward);
        let chat = chat_is_open(snap);
        if consumed {
            if rewarded && !chat {
                // Consumed, awarded and the award dialogue drained: the
                // redemption is real. Release the hold; the next lamp, if
                // any, starts from a fresh Rub.
                self.clear_lamp();
                self.acting = false;
                return;
            }
            if chat && self.lamp_dialog < MAX_LAMP_DIALOGUE {
                let mut ix = Interactions::new(snap, driver);
                if matches!(ix.continue_dialog(None), SendResult::Sent { .. }) {
                    self.lamp_dialog += 1;
                    self.lamp_wait = 0;
                    return;
                }
            }
        }
        self.lamp_wait += 1;
        if self.lamp_wait <= MAX_LAMP_WAIT {
            return;
        }
        if consumed {
            // The lamp is gone; its reward or award dialogue never showed.
            // Nothing is left to protect, so release without latching: a
            // sibling lamp still in the pack is rubbed next.
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "lamp: the reward never completed; releasing the hold"
            );
            self.clear_lamp();
            self.acting = false;
        } else {
            // Confirm did not take: the lamp is still held and the IF stayed
            // open or the selection was refused. Latch without replaying.
            api::host_log!(
                Category::RandomEvent,
                Level::Warn,
                "lamp: redemption did not complete; giving up"
            );
            self.stall_lamp();
        }
    }

    /// Drop the lamp flow state (a resolved, consumed or abandoned lamp).
    fn clear_lamp(&mut self) {
        self.lamp_rubbed = false;
        self.lamp_rubs = 0;
        self.lamp_skill_sent = false;
        self.lamp_confirmed = false;
        self.lamp_confirm_count = None;
        self.lamp_reward = None;
        self.lamp_dialog = 0;
        self.lamp_wait = 0;
        self.lamp_park = 0;
        self.lamp_stalled = false;
    }

    /// Pause the Rub flow for `ticks` fresh ticks: the lamp is inert while
    /// the pause counts down, so the script runs between attempts. The
    /// episode counter `lamp_rubs` survives the pause; the caller resets it
    /// when a new episode starts.
    fn pause_lamp(&mut self, ticks: u32) {
        self.lamp_rubbed = false;
        self.lamp_wait = 0;
        self.lamp_park = ticks;
        self.acting = false;
    }

    /// A stalled redemption or a paused Rub: detected, but not acted on or
    /// held until it resolves.
    fn lamp_inert(&self) -> bool {
        self.lamp_stalled || self.lamp_park > 0
    }

    /// Give up on this lamp: release the hold, keep the latch that stops
    /// `tick` from acting/holding again until the lamp leaves the pack or
    /// `lamp_auto` goes off.
    fn stall_lamp(&mut self) {
        self.clear_lamp();
        self.lamp_stalled = true;
        self.acting = false;
    }

    /// Take the named lost fishing gear from the ground (Chebyshev ≤ 10,
    /// the client walks the take). A full pack drops one sacrificial
    /// item first so the Take lands.
    fn step_lost_gear<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
    ) {
        let gear = ev.name.as_str();
        let Some(item) = snap
            .ground_items()
            .iter()
            .find(|g| g.distance <= LOST_GEAR_RADIUS && item_named(g.def.name.as_deref(), gear))
        else {
            self.acting = false;
            return;
        };
        if pack_full(snap) {
            if let Some(junk) = sacrificial_item(snap) {
                let mut ix = Interactions::new(snap, driver);
                let _ = ix.interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
                return;
            }
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(
            OpTarget::GroundItem(item),
            ActionSpec::Label("Take".to_string()),
        ) {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// Content reattachment is held-on-held only: take the head first.
    /// Failure releases the script to replace the missing tool at a bank.
    fn step_lost_tool<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, now_ms: u64) {
        if self
            .tool_deadline_ms
            .is_some_and(|deadline| now_ms >= deadline)
        {
            self.tool_stalled = true;
            self.acting = false;
            return;
        }
        let handle_inv = snap
            .inventory()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let handle_worn = snap
            .equipment()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let Some(handle) = handle_inv.or(handle_worn) else {
            self.acting = false;
            return;
        };
        let kind = tool_part(handle.def.name.as_deref(), "handle");
        let head_inv = snap
            .inventory()
            .iter()
            .find(|i| i.count > 0 && tool_part(i.def.name.as_deref(), "head") == kind);
        let head_ground = snap.ground_items().iter().find(|g| {
            g.distance <= LOST_GEAR_RADIUS && tool_part(g.def.name.as_deref(), "head") == kind
        });
        if head_ground.is_none() && head_inv.is_none() {
            // The content replaces the tool before its projectile lands.
            // Keep only this bounded grace period for the head to appear.
            return;
        }
        if (handle_worn.is_some() || head_inv.is_none()) && pack_full(snap) {
            let Some(junk) = sacrificial_item(snap) else {
                self.tool_stalled = true;
                self.acting = false;
                return;
            };
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
            return;
        }
        let mut ix = Interactions::new(snap, driver);
        let result = if handle_worn.is_some() {
            ix.interact(
                OpTarget::Item(handle),
                ActionSpec::Label("Remove".to_string()),
            )
        } else if let Some(head) = head_inv {
            ix.use_item_on(head, OpTarget::Item(handle))
        } else {
            ix.interact(
                OpTarget::GroundItem(head_ground.expect("head is observed above")),
                ActionSpec::Label("Take".to_string()),
            )
        };
        if matches!(result, SendResult::Refused { .. }) {
            self.tool_stalled = true;
            self.acting = false;
        }
    }

    /// The handle lifts when the chat is fully closed and the in-flight
    /// NPC has left the scene (the spec's "NPC gone and chat closed").
    /// Continue/answer refuse clears via [`Guardian::act`] when chat is
    /// closed (after a one-tick grace post Talk-to).
    fn dialog_done(&self, snap: &GameSnapshot) -> bool {
        let npc_here = self
            .in_flight_index
            .is_some_and(|i| snap.npcs().iter().any(|v| v.index == i));
        !chat_is_open(snap) && !npc_here
    }

    fn clear_handle(&mut self) {
        self.in_flight = false;
        self.in_flight_index = None;
        self.continues = 0;
    }
}
/// One `try_move` toward `target`; false when the walk is stuck
/// ([`maze::WALK_LIMIT`] sends without a tile change — the door is
/// walled off).
fn maze_walk_step<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    me: (i32, i32),
    target: (i32, i32),
) -> bool {
    if st.walk_from != Some(me) {
        st.walk_from = Some(me);
        st.walk_sends = 0;
    }
    st.walk_sends += 1;
    if st.walk_sends > maze::WALK_LIMIT {
        return false;
    }
    walk_nearest(driver, target.0, target.1);
    true
}

/// `oploc` Open on a route door (ids 3628–3632 at that tile).
fn maze_send_open<D: Driver>(driver: &mut D, tile: (i32, i32)) {
    let loc_id = maze::graph()
        .door_id
        .get(&tile)
        .copied()
        .unwrap_or(maze::MAZE_DOOR_IDS[0]);
    op_loc(driver, tile.0, tile.1, loc_id);
}

/// `oploc` Touch on the shrine (loc 3634) and wait to leave the square.
fn maze_send_touch<D: Driver>(st: &mut maze::MazeSolve, driver: &mut D, pass: u32) {
    op_loc(
        driver,
        maze::MAZE_SHRINE.0,
        maze::MAZE_SHRINE.1,
        maze::MAZE_SHRINE_LOC,
    );
    st.phase = maze::MazePhase::TouchWait;
    st.touch_pass = pass;
    st.wait_ticks = 0;
}

/// One maze phase step (rs2b0t `solveMaze` loop body). Returns false
/// when the pass gives up and the route restarts.
fn step_maze_phase<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    snap: &GameSnapshot,
    me: (i32, i32),
) -> bool {
    // A mesbox/briefing chat is drained first; while it is up nothing
    // else happens. A chat during an in-flight open is the wrong-door
    // refusal mesbox (rs2b0t clears it, then continues the route).
    if chat_is_open(snap) {
        if matches!(
            st.phase,
            maze::MazePhase::OpenDoor { .. }
                | maze::MazePhase::OpenResync { .. }
                | maze::MazePhase::OpenShrine { .. }
        ) {
            st.refused = true;
        }
        if st.continues < maze::MESBOX_LIMIT {
            st.continues += 1;
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.continue_dialog(None);
        }
        return true;
    }
    st.continues = 0;

    match st.phase {
        maze::MazePhase::WalkDoor => {
            let Some(door) = st.target() else {
                // A route ending at the chamber door already opened it.
                // Regenerated routes that missed it retain the fallback.
                st.phase = if st.doors.last() == Some(&maze::MAZE_SHRINE_DOOR) {
                    maze::MazePhase::Touch { pass: 0 }
                } else {
                    maze::MazePhase::ShrineDoor
                };
                st.touch_pass = 0;
                return true;
            };
            if cheb(me, door) <= 1 {
                maze_send_open(driver, door);
                st.phase = maze::MazePhase::OpenDoor { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, door) {
                // Walled off: step back through the previous door.
                if st.next == 0 || st.resyncs >= maze::MAX_RESYNCS {
                    return false;
                }
                st.resyncs += 1;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                st.phase = maze::MazePhase::Resync;
            }
            true
        }
        maze::MazePhase::OpenDoor { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.next += 1;
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Resync => {
            let Some(prev_door) = st.target() else {
                return false;
            };
            if cheb(me, prev_door) <= 1 {
                maze_send_open(driver, prev_door);
                st.phase = maze::MazePhase::OpenResync { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, prev_door) {
                // The previous door is walled off too: retry the route
                // door, which re-counts a resync (rs2b0t the same way).
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::OpenResync { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                // Back on the route: retry the walled-off door.
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::ShrineDoor => {
            if cheb(me, maze::MAZE_SHRINE_DOOR) <= 1 {
                maze_send_open(driver, maze::MAZE_SHRINE_DOOR);
                st.phase = maze::MazePhase::OpenShrine { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, maze::MAZE_SHRINE_DOOR) {
                // The chamber door is unreachable: give up this pass.
                return false;
            }
            true
        }
        maze::MazePhase::OpenShrine { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.phase = maze::MazePhase::Touch {
                    pass: st.touch_pass,
                };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Touch { pass } => {
            if pass >= maze::TOUCH_LIMIT {
                // Still inside after all passes: restart the route.
                return false;
            }
            // Pass 0 near the shrine (the post-door tile): touch now.
            if pass == 0 && cheb(me, maze::MAZE_SHRINE) <= 2 {
                maze_send_touch(st, driver, pass);
                return true;
            }
            let stand = maze::TOUCH_STANDS[pass as usize % maze::TOUCH_STANDS.len()];
            let onto = pass % 2 == 0;
            let reached = if onto {
                me == stand
            } else {
                cheb(me, stand) <= 1
            };
            if reached {
                maze_send_touch(st, driver, pass);
                return true;
            }
            if !maze_walk_step(st, driver, me, stand) {
                // A walled-off stand: the next pass.
                st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::TouchWait => {
            st.wait_ticks += 1;
            if st.wait_ticks >= maze::TOUCH_WAIT {
                st.wait_ticks = 0;
                let pass = st.touch_pass;
                st.touch_pass = pass + 1;
                // rs2b0t re-opens the chamber door on odd passes.
                if pass % 2 == 1 {
                    st.phase = maze::MazePhase::ShrineDoor;
                } else {
                    st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                }
            }
            true
        }
    }
}

/// Whether the NPC chat modal (continue button, choice buttons, or chat
/// root) is up — the same open check dialog_done / refuse-clear share.
fn chat_is_open(snap: &GameSnapshot) -> bool {
    snap.chat_continue_component_id() != -1
        || !snap.chat_options().is_empty()
        || snap.modals().chat != -1
}
#[cfg(test)]
#[path = "../random_tests.rs"]
mod tests;
