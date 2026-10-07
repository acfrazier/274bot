use super::*;

/// The live target of a transport hop: the edge's loc view (doors, ladders,
/// stairs, agility shortcuts, spirit trees) or, for the shared NPC-backed
/// Boat/Npc/Glider path, the edge's NPC view.
pub(super) enum TransportTarget<'s> {
    Loc(&'s LocView),
    Npc(&'s NpcView),
}

impl TransportTarget<'_> {
    pub(super) fn footprint(&self) -> Option<(WorldTile, i32, i32)> {
        match self {
            Self::Loc(loc) => Some((
                loc.tile,
                loc.footprint_width.max(1),
                loc.footprint_length.max(1),
            )),
            Self::Npc(_) => None,
        }
    }
}

/// The snapshot target for a loc-backed transport edge.
pub(super) fn find_transport_target<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
) -> Option<TransportTarget<'s>> {
    find_transport_loc(snapshot, edge).map(TransportTarget::Loc)
}

/// Re-find the current NPC slot by identity and type. Before interaction,
/// recovery may explicitly select a reachable same-type replacement.
pub(super) fn find_transport_target_instance<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
    npc_index: Option<usize>,
) -> Option<TransportTarget<'s>> {
    if !npc_backed(edge) {
        return find_transport_target(snapshot, edge);
    }
    let index = npc_index?;
    snapshot
        .npcs()
        .iter()
        .find(|npc| {
            npc.index == index
                && npc.r#type == Some(edge.loc_id as usize)
                && npc.network.level == edge.at.level
        })
        .map(TransportTarget::Npc)
}

/// Send the transport hop's interact for the edge's target kind: an
/// `op_loc` for loc edges or an `op_npc` for NPC-backed edges, both with the
/// edge's `option`. `option` 0 on a loc hop uses the first `item_req` obj on
/// the loc (`oplocu` — unequippable knife on a web).
pub(super) fn interact_transport<'t, 'o>(
    snapshot: &'t GameSnapshot,
    ix: &mut Interactions<'t>,
    target: TransportTarget<'t>,
    edge: &TransportEdge,
    on_event: &mut Option<Box<dyn FnMut(TravelEvent) + 'o>>,
) -> SendResult<'t> {
    let (actual_id, tile) = match &target {
        TransportTarget::Loc(l) => (l.id, l.tile),
        TransportTarget::Npc(n) => (n.r#type.map(|id| id as i32).unwrap_or(-1), n.network),
    };
    let result = match target {
        TransportTarget::Loc(loc) if uses_held_on_loc(edge) => {
            let id = edge.item_req[0].0;
            match snapshot.inventory().iter().find(|it| it.def.id == id) {
                Some(item) => ix.use_item_on(item, OpTarget::Loc(loc)),
                None => SendResult::Refused {
                    tick: snapshot.tick() as u64,
                    reason: SendReason::StaleTarget,
                },
            }
        }
        TransportTarget::Loc(loc) => {
            ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(edge.option))
        }
        TransportTarget::Npc(npc) => {
            ix.interact(OpTarget::Npc(npc), ActionSpec::Operation(edge.option))
        }
    };
    if let Some(cb) = on_event.as_mut() {
        let refusal = match &result {
            SendResult::Sent { .. } => None,
            SendResult::Refused { reason, .. } => Some(*reason),
        };
        cb(TravelEvent::TransportAttempt {
            tick: snapshot.tick(),
            kind: edge.kind,
            expected_id: edge.loc_id,
            actual_id,
            target: tile,
            option: edge.option,
            refusal,
        });
    }
    result
}

/// Knife-on-web: `option` 0 + an `item_req` means `oplocu`, not `oploc1`.
pub(super) fn uses_held_on_loc(edge: &TransportEdge) -> bool {
    edge.option == 0 && !edge.item_req.is_empty()
}

/// The block message's target word: "npc" for NPC-backed edges, "loc" for
/// every loc-targeted kind.
pub(super) fn target_word(edge: &TransportEdge) -> &'static str {
    if npc_backed(edge) {
        "npc"
    } else {
        "loc"
    }
}

/// Dock sailors / cart drivers: `loc_id` is the npc.pack type. Boat was
/// packed as `TransportKind::Boat` with that id; looking it up as a loc
/// (Port Sarim seaman 378) is why live Follow died on the pier.
pub(super) fn npc_backed(edge: &TransportEdge) -> bool {
    matches!(
        edge.kind,
        TransportKind::Npc | TransportKind::Boat | TransportKind::Glider
    )
}

/// Closed or open leaf within chebyshev 3 of `edge.at` (live Catherby
/// open 1531 sits a tile off the derived `at`).
pub(super) fn find_door_loc<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
) -> Option<&'s LocView> {
    find_transport_loc(snapshot, edge)
}

/// Whether a transport hop drives the script's chat dialogs itself: an
/// Npc hop's `opnpc1` opens the ride's chat (the cart fare, Elkoy's
/// escort), a jewellery rub opens its destination choice, a spirit tree's
/// `oploc1` opens the dest dialog (`spirit_tree.rs2`: "Where can I go?"
/// then the sibling list, or the young tree's "Yes please."), and a
/// dialogue Door hop — the Al Kharid border toll (`border_gate.rs2`
/// `p_choice3`) or the Shantay henge (`shantay_pass.rs2` pass handover
/// and first-crossing disclaimer) — opens chat before the crossing.
/// A plain door hop never opens chat.
pub(super) fn drives_hop_dialogs(edge: &TransportEdge) -> bool {
    npc_backed(edge)
        || edge.kind == TransportKind::SpiritTree
        || (edge.kind == TransportKind::Teleport && edge.loc_id > 0)
        || (edge.kind == TransportKind::Door && is_dialogue_door_loc(edge.loc_id))
}

/// Whether the live loc family already reads **open**. Packed closed/open
/// ids are resolved by [`find_door_loc`] within chebyshev 3 of `at` (the
/// Catherby open leaf at (2816,3439) while `at` is (2816,3438)). When
/// that misses, an unpacked swing door with no `open_loc_id` may still
/// read open if the closed id is gone and a loc **on `edge.at`** offers
/// Close — not a nearby unrelated Close loc (sealed `dir=None` stand
/// hops such as ranging 2514 sit a tile off the door loc).
pub(super) fn edge_loc_open(snapshot: &GameSnapshot, edge: &TransportEdge) -> bool {
    if let Some(loc) = find_door_loc(snapshot, edge) {
        return loc.id != edge.loc_id;
    }
    snapshot.locs().iter().any(|loc| {
        loc.tile == edge.at
            && api::query::door_is_open(loc.actions.iter().flatten().map(String::as_str))
    })
}

/// The chat ring's latest sequence. NPC-backed hops refresh this watermark
/// when the interaction is actually sent, so the reachability watch cannot
/// attribute an older line to a delayed or re-armed interaction.
pub(super) fn chat_seq(snapshot: &GameSnapshot) -> i32 {
    Query::new(snapshot.chat_lines()).latest_sequence()
}

const WEB_CUT_FAILURE_MESSAGE: &str = "You fail to cut through it.";

/// A cut failure authorizes exactly one retry; the watermark advances on
/// the next successful send so the same chat line cannot arm another.
fn observed_web_cut_failure(snapshot: &GameSnapshot, after: i32) -> bool {
    snapshot
        .chat_lines()
        .iter()
        .any(|line| line.sequence > after && line.text == WEB_CUT_FAILURE_MESSAGE)
}

/// Re-evaluate both packed web actions against the snapshot at the moment
/// the interaction is sent. The route may have been planned with a different
/// loadout while its approach walk was in progress.
pub(super) fn current_web_action<'a>(
    snapshot: &GameSnapshot,
    edge: &TransportEdge,
    edges: Option<&'a [TransportEdge]>,
) -> Result<&'a TransportEdge, &'static str> {
    let edges = edges.ok_or("the packed transport edge list is unavailable")?;
    let state = crate::world_state::WorldState::from_snapshot(snapshot);
    crate::transport::select_web_action(edges.iter(), edge, |candidate| state.allows(candidate))
        .ok_or("neither a worn slash blade nor a carried knife qualifies")
}

pub(super) fn web_action_failure(
    at: WorldTile,
    leg: usize,
    edge: &TransportEdge,
    reason: &str,
) -> TravelOutcome {
    TravelOutcome::Blocked {
        at,
        leg,
        detail: format!("slashable web loc {} cannot be cut: {reason}", edge.loc_id),
    }
}

/// The door's own tile: the edge's `at` — in the new edge model `at` IS
/// the loc tile (the interact target), so no midpoint derivation. The
/// door-troll read compares the loc's live id at this tile against the
/// edge's closed id.
pub(super) fn door_tile(edge: &TransportEdge) -> WorldTile {
    edge.at
}

/// Whether `here` is on the arrival side of the wall. A reverse straight
/// crossing lands on `at` itself, so its boundary is inclusive. Other
/// crossings retain the strict half-plane test; proximity alone must
/// never count a near-side approach as a completed hop.
pub(super) fn door_crossed(edge: &TransportEdge, here: WorldTile) -> bool {
    let on_loc_side = edge.to == edge.at;
    match edge.dir {
        Some(DoorDir::N) => here.z > edge.at.z || (on_loc_side && here.z == edge.at.z),
        Some(DoorDir::S) => here.z < edge.at.z || (on_loc_side && here.z == edge.at.z),
        Some(DoorDir::E) => here.x > edge.at.x || (on_loc_side && here.x == edge.at.x),
        Some(DoorDir::W) => here.x < edge.at.x || (on_loc_side && here.x == edge.at.x),
        None => true,
    }
}

/// Whether an Open left the player on the door's own tile `at` with `to`
/// one cardinal step away on the crossing side: the post-Open step that
/// finishes the crossing. `open_and_close_door2` doors (Tenzing's 3745)
/// teleport the entering player onto `at` and swap in an inviswall for
/// three ticks, so the loc never reads open; ordinary doors reach the same
/// state when the player opens from `at`.
pub(super) fn door_step_pending(edge: &TransportEdge, here: WorldTile) -> bool {
    here == edge.at
        && edge.to.level == here.level
        && here.x.abs_diff(edge.to.x) + here.z.abs_diff(edge.to.z) == 1
        && edge.dir.is_some()
        && door_crossed(edge, edge.to)
}

/// The first closed window after an unanswered Open; it doubles with each
/// further unanswered Open, capped at `MIN << MAX_SHIFT` (32 ticks).
const DOOR_REOPEN_MIN_CLOSED_TICKS: u32 = 2;
const DOOR_REOPEN_MAX_SHIFT: u32 = 4;

/// Re-Open pacing for door recovery. A leaf seen open since the last Open
/// was really toggled, so a closer is racing us: re-Open on the next closed
/// read. A leaf that never opened (a locked, held-item or requirement
/// refusal, or an Open still queued behind a walk) is re-Opened only after
/// it has read closed for a window that doubles per unanswered Open. With
/// the default 60-tick budget, a door that never opens gets Opens at ticks
/// 0, 2, 6, 14 and 30, not one per tick.
#[derive(Clone, Copy, Debug, Default)]
pub(super) struct DoorRetry {
    /// The tick of the latest Open sent on this hop.
    sent: Option<u32>,
    /// The leaf has read open since `sent`.
    seen_open: bool,
    /// Consecutive earlier Opens that the leaf never answered by opening.
    unanswered: u32,
}

impl DoorRetry {
    /// Pacing for a hop whose arm sent the door's Open on `sent`, if any.
    pub(super) fn opened_at(sent: Option<u32>) -> Self {
        Self {
            sent,
            ..Self::default()
        }
    }

    /// Record an Open sent on `tick`.
    pub(super) fn sent(&mut self, tick: u32) {
        self.unanswered = if self.sent.is_some() && !self.seen_open {
            self.unanswered.saturating_add(1)
        } else {
            0
        };
        self.sent = Some(tick);
        self.seen_open = false;
    }

    /// Record this poll's read of the leaf.
    pub(super) fn observe(&mut self, open: bool) {
        self.seen_open |= open;
    }

    /// Whether a closed leaf may be re-Opened on `tick`.
    pub(super) fn may_reopen(&self, tick: u32) -> bool {
        match self.sent {
            None => true,
            Some(_) if self.seen_open => true,
            Some(sent) => {
                let window =
                    DOOR_REOPEN_MIN_CLOSED_TICKS << self.unanswered.min(DOOR_REOPEN_MAX_SHIFT);
                tick.saturating_sub(sent) >= window
            }
        }
    }
}

/// Door hops without a cardinal `dir` normally settle with
/// `arrived(to, close_enough)`. When the origin stand sits inside that
/// radius (`Cheb(at, to) <= close_enough` on the same level), the
/// tolerance is geometrically invalid: standing on `at` — including after
/// a script `~forcemove` and before `p_teleport` — looks like arrival.
/// Those short hops require the exact landing. Far dir=None doors
/// (Zanaris, levers, Shantay south) keep the runner's radius. Cardinal
/// `dir=Some` doors stay on [`door_crossed`].
pub(super) fn door_dir_none_arrived(
    edge: &TransportEdge,
    here: WorldTile,
    close_enough: i32,
) -> bool {
    if here.level != edge.to.level {
        return false;
    }
    let to_gap = (here.x - edge.to.x).abs().max((here.z - edge.to.z).abs());
    let hop_span = if edge.at.level == edge.to.level {
        (edge.at.x - edge.to.x)
            .abs()
            .max((edge.at.z - edge.to.z).abs())
    } else {
        i32::MAX
    };
    if hop_span <= close_enough {
        here == edge.to
    } else {
        to_gap <= close_enough
    }
}

impl FollowRun {
    /// One transport-hop settle step: match the positional `arrived(edge.to)`
    /// arm (level + proximity, so a level-changing transport completes only
    /// within `close_enough` of `to` on the destination level), recover an
    /// NPC reach failure within its attempt/leg bounds, retry a web only on
    /// its content failure message, or lapse the budget. Door-kind edges with
    /// a packed open leaf (swing doors, gates, held-item and scripted doors)
    /// recover while the hop still has time to observe and retry the crossing.
    pub(super) fn poll_transport<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
        essence: &mut Option<EssenceSession>,
    ) -> Poll {
        let mut hop = self.transport.take().expect("transport hop present");
        let here = here(snapshot);
        // Before the interaction: an approach still pending sends it on this
        // poll. Once sent, the crossing is not rechecked; aborting it would
        // report a physically completed crossing as unproven.
        if let (Some(_), Leg::Transport { edge }) = (&hop.approach, &hop.leg) {
            if let Some(outcome) =
                self.check_transport_gate(edge, here, snapshot, options.quest_evidence)
            {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Poll::Terminal(outcome);
            }
        }
        if let (Some(cb), Leg::Transport { edge }) = (options.on_event.as_mut(), &hop.leg) {
            cb(TravelEvent::TransportState {
                tick: snapshot.tick(),
                at: here,
                kind: edge.kind,
                loc_id: edge.loc_id,
                from: edge.at,
                to: edge.to,
                open: edge_loc_open(snapshot, edge),
                approach: hop.approach.is_some(),
                troll: hop.troll,
                waited: if npc_backed(edge) {
                    hop.npc_recovery.waited
                } else {
                    hop.ticks_waited
                },
                loc_wait: self.loc_wait,
                budget: self.budget,
                live_loc: find_transport_loc(snapshot, edge).map(|l| (l.id, l.tile)),
            });
        }
        let npc_hop = matches!(&hop.leg, Leg::Transport { edge } if npc_backed(edge));
        if npc_hop {
            if hop.npc_recovery.last_tick != Some(snapshot.tick()) {
                hop.npc_recovery.last_tick = Some(snapshot.tick());
                hop.npc_recovery.waited += 1;
            }
            if hop.approach.is_some() {
                if hop.npc_recovery.waited > self.budget {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Poll::Terminal(self.npc_expired(snapshot, &hop));
                }
                let poll = self.poll_npc_approach(d, snapshot, &mut hop, options);
                if matches!(poll, Poll::Watching) {
                    self.transport = Some(hop);
                }
                return poll;
            }
        }
        if hop.approach.is_none() {
            if let Leg::Transport { edge } = &hop.leg {
                // A leaf can close after an already-open walk armed, or
                // before our Open takes effect. Recover now, not after the
                // entire crossing budget has already been spent. Scripted
                // doors without a packed open leaf, dialogue and webs keep
                // their own settle/proof behavior.
                hop.troll |= edge.kind == TransportKind::Door
                    && edge.open_loc_id.is_some()
                    && !edge.is_slashable_web()
                    && !drives_hop_dialogs(edge);
            }
        }
        if hop.troll {
            if let Some(outcome) = self.troll_door(d, snapshot, &mut hop, options) {
                return Poll::Terminal(outcome);
            }
        } else if hop.approach.is_none() {
            if let Leg::Transport { edge } = &hop.leg {
                // Cheap hop: Open was sent; the live open leaf can sit a tile
                // off `at`. Walk through as soon as it reads open — do not sit
                // until the troll budget while the door is already open.
                if edge.kind == TransportKind::Door
                    && edge_loc_open(snapshot, edge)
                    && !door_crossed(edge, here)
                {
                    let mut ix = Interactions::new(snapshot, d);
                    let result = ix.walk(hop.to);
                    report_walk(options, snapshot, here, hop.to, &result);
                    match result {
                        SendResult::Sent { .. } => {
                            api::host_log!(
                                Category::NavTrace,
                                Level::Debug,
                                "cheap hop walk-through to {:?}",
                                hop.to
                            );
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                        }
                    }
                    self.transport = Some(hop);
                    return Poll::Watching;
                } else if edge.kind == TransportKind::Door
                    && hop
                        .open_sent_tick
                        .is_some_and(|sent| snapshot.tick() != sent)
                    && door_step_pending(edge, here)
                    && SceneQuery::new(snapshot.scene(), None).can_step(here, edge.to)
                {
                    // The Open already carried the player through the
                    // door's wall onto `at` (Tenzing's 3745
                    // `open_and_close_door2` teleports the entering player
                    // onto the loc tile, whose wall is on the far edge, and
                    // swaps in an inviswall for 3 ticks, so the door never
                    // reads open): take the clear step to `to` now instead
                    // of sitting out the cheap budget. A step still behind
                    // the wall is left to the open-door walk above — a walk
                    // packet there would cancel the queued Open.
                    hop.open_sent_tick = None;
                    let mut ix = Interactions::new(snapshot, d);
                    let result = ix.pending_door_step(edge.to);
                    report_walk(options, snapshot, here, edge.to, &result);
                    match result {
                        SendResult::Sent { .. } => {
                            api::host_log!(
                                Category::NavTrace,
                                Level::Debug,
                                "cheap hop door step to {:?}",
                                edge.to
                            );
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                        }
                    }
                    self.transport = Some(hop);
                    return Poll::Watching;
                } else if edge.open_loc_id.is_some()
                    && edge.kind != TransportKind::Door
                    && !npc_backed(edge)
                    && edge_loc_open(snapshot, edge)
                    && hop.tries == 0
                {
                    return match find_transport_target_instance(snapshot, edge, hop.npc_index) {
                        Some(target) => {
                            let arrival_footprint = target.footprint();
                            let mut ix = Interactions::new(snapshot, d);
                            match interact_transport(
                                snapshot,
                                &mut ix,
                                target,
                                edge,
                                &mut options.on_event,
                            ) {
                                SendResult::Sent { .. } => {
                                    hop.tries = 1;
                                    hop.ticks_waited = 0;
                                    hop.sent_tile = Some(here);
                                    hop.arrival_footprint = arrival_footprint;
                                    self.loc_wait = 0;
                                    self.transport = Some(hop);
                                    Poll::Watching
                                }
                                SendResult::Refused { reason, .. } => {
                                    fire_leg(options, &hop.leg, LegPhase::Failed);
                                    Poll::Terminal(TravelOutcome::Refused { at: here, reason })
                                }
                            }
                        }
                        None => {
                            self.transport = Some(hop);
                            Poll::Watching
                        }
                    };
                }
            }
        }
        if hop.approach.is_none() {
            let web_edge = match &hop.leg {
                Leg::Transport { edge } if edge.is_slashable_web() => Some(edge.clone()),
                _ => None,
            };
            if let Some(edge) = web_edge.filter(|edge| {
                !edge_loc_open(snapshot, edge) && observed_web_cut_failure(snapshot, hop.chat_seq)
            }) {
                let Some(target) = find_transport_target_instance(snapshot, &edge, hop.npc_index)
                else {
                    self.loc_wait += 1;
                    if self.loc_wait > self.budget {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        return Poll::Terminal(TravelOutcome::Blocked {
                            at: here,
                            leg: self.leg_index,
                            detail: format!(
                                "slashable web loc {} disappeared before retrying its observed cut failure",
                                edge.loc_id
                            ),
                        });
                    }
                    self.transport = Some(hop);
                    return Poll::Watching;
                };
                let action = match current_web_action(snapshot, &edge, options.edges) {
                    Ok(action) => action,
                    Err(reason) => {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        return Poll::Terminal(web_action_failure(
                            here,
                            self.leg_index,
                            &edge,
                            reason,
                        ));
                    }
                };
                let chat_seq_at_send = chat_seq(snapshot);
                let arrival_footprint = target.footprint();
                let mut ix = Interactions::new(snapshot, d);
                return match interact_transport(
                    snapshot,
                    &mut ix,
                    target,
                    action,
                    &mut options.on_event,
                ) {
                    SendResult::Sent { .. } => {
                        hop.chat_seq = chat_seq_at_send;
                        hop.ticks_waited = 0;
                        hop.sent_tile = Some(here);
                        hop.arrival_footprint = arrival_footprint;
                        hop.tries = hop.tries.saturating_add(1);
                        hop.open_sent_tick = Some(snapshot.tick());
                        self.loc_wait = 0;
                        self.transport = Some(hop);
                        Poll::Watching
                    }
                    SendResult::Refused { reason, .. } => {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Refused { at: here, reason })
                    }
                };
            }
        }
        // Loc-backed pre-interact approach; NPC approaches are driven above.
        if hop.approach.is_some() {
            match self.poll_approach(d, snapshot, &mut hop, options) {
                Poll::Watching => {
                    self.transport = Some(hop);
                    return Poll::Watching;
                }
                Poll::Terminal(outcome) => return Poll::Terminal(outcome),
                Poll::LegDone => {}
            }
            let edge = match &hop.leg {
                Leg::Transport { edge } => edge.clone(),
                Leg::Walk { .. } => unreachable!("transport hop holds a transport leg"),
            };
            if edge.kind == TransportKind::Door && edge_loc_open(snapshot, &edge) {
                // The leaf opened (another player, or a slashed web) while
                // the player walked to this stand. OP_LOC1 on an open leaf
                // is Close: send nothing, and let the next poll's open-leaf
                // walk (door recovery, or the cheap hop's) cross it.
                self.transport = Some(hop);
                return Poll::Watching;
            }
            let target = find_transport_target_instance(snapshot, &edge, hop.npc_index);
            return match target {
                Some(target) => {
                    let action = if edge.is_slashable_web() {
                        match current_web_action(snapshot, &edge, options.edges) {
                            Ok(action) => action,
                            Err(reason) => {
                                fire_leg(options, &hop.leg, LegPhase::Failed);
                                return Poll::Terminal(web_action_failure(
                                    here,
                                    self.leg_index,
                                    &edge,
                                    reason,
                                ));
                            }
                        }
                    } else {
                        &edge
                    };
                    let chat_seq_at_send =
                        (npc_backed(&edge) || edge.is_slashable_web()).then(|| chat_seq(snapshot));
                    let arrival_footprint = target.footprint();
                    let mut ix = Interactions::new(snapshot, d);
                    match interact_transport(
                        snapshot,
                        &mut ix,
                        target,
                        action,
                        &mut options.on_event,
                    ) {
                        SendResult::Sent { .. } => {
                            self.loc_wait = 0;
                            hop.ticks_waited = 0;
                            hop.sent_tile = Some(here);
                            hop.arrival_footprint = arrival_footprint;
                            if let Some(seq) = chat_seq_at_send {
                                hop.chat_seq = seq;
                            }
                            hop.approach = None;
                            if edge.kind == TransportKind::Door {
                                hop.open_sent_tick = Some(snapshot.tick());
                                if !edge.is_slashable_web() {
                                    hop.tries = hop.tries.saturating_add(1);
                                    hop.door_retry.sent(snapshot.tick());
                                }
                            }
                            if edge.open_loc_id.is_some()
                                && edge.kind != TransportKind::Door
                                && edge_loc_open(snapshot, &edge)
                            {
                                hop.tries = 1;
                            }
                            self.transport = Some(hop);
                            Poll::Watching
                        }
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            Poll::Terminal(TravelOutcome::Refused { at: here, reason })
                        }
                    }
                }
                None => {
                    self.loc_wait += 1;
                    if self.loc_wait > self.budget {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        Poll::Terminal(TravelOutcome::Blocked {
                            at: here,
                            leg: self.leg_index,
                            detail: format!(
                                "transport {} {} is not within 3 tiles of ({}, {}, {}) in the loaded scene",
                                target_word(&edge),
                                edge.loc_id,
                                edge.at.x,
                                edge.at.z,
                                edge.at.level
                            ),
                        })
                    } else {
                        self.transport = Some(hop);
                        Poll::Watching
                    }
                }
            };
        }
        // The "I can't reach that!" watch: the client pathfind failed
        // right after the send, so the hop is refused immediately instead
        // of sitting out the settle budget. NPC-backed hops refresh
        // `chat_seq` at interaction send, so only genuinely new lines count.
        let hop_seq = hop.chat_seq;
        let edge = match &hop.leg {
            Leg::Transport { edge } => edge.clone(),
            Leg::Walk { .. } => unreachable!("transport hop holds a transport leg"),
        };
        // The latch target, read before the arrive-arm builder below moves
        // `edge` into the door closure: a completed essence entry hop
        // records the wizard the player entered through.
        let entry_wizard = is_essence_entry_edge(&edge).then_some(edge.loc_id);
        if crate::debug_enabled() {
            api::host_log!(
                Category::NavTrace,
                Level::Debug,
                "transport here={here:?} to={:?} troll={} ticks_waited={} loc_id={} open={}",
                hop.to,
                hop.troll,
                hop.ticks_waited,
                edge.loc_id,
                edge_loc_open(snapshot, &edge),
            );
        }
        // An Npc hop's first op can open a chat dialog instead of riding
        // immediately (the live cart drivers' `opnpc1` says "Hello!", then
        // asks "Is that Ok?" with a "Yes please…" choice), a jewellery
        // rub opens the destination choice (the glory's "Where would you
        // like to teleport to?" with each location named — the dueling
        // ring's single arena first and "Nowhere." last), a Shantay henge
        // `oploc1` shows the pass handover (and the first-crossing
        // disclaimer choice), and the Al Kharid toll `oploc1` opens the
        // border-guard pay page. Drive the dialog the same way: press the
        // modal's continue button while it is up, and press the content-
        // defined choice exactly once when the choice page is up, then keep
        // watching `arrived(to)`. Unknown Door pages and an Al Kharid pay
        // page without 10 coins refuse instead of guessing.
        if drives_hop_dialogs(&edge) {
            if npc_hop
                && (snapshot.chat_continue_component_id() != -1
                    || !snapshot.chat_options().is_empty()
                    || snapshot.modals().main == GLIDER_MAP_ROOT)
            {
                // Once fare dialogue begins, a reach line cannot justify an
                // NPC/approach click that would cancel the pending journey.
                hop.npc_recovery.dialog_started = true;
            }
            let mut ix = Interactions::new(snapshot, d);
            if snapshot.chat_continue_component_id() != -1 {
                match ix.continue_dialog(None) {
                    SendResult::Sent { .. } => {
                        api::host_log!(
                            Category::NavEvent,
                            Level::Info,
                            "continued the npc {} dialog",
                            edge.loc_id
                        );
                    }
                    SendResult::Refused { .. } => {}
                }
            } else if !snapshot.chat_options().is_empty() {
                // A jewellery rub's destination choice is the edge's case
                // index: the script maps the answered option through
                // `switch_int($choice)`, so the choice of the edge being
                // followed is the 1-based index of its `to` among the
                // packed same-`loc_id` rub edges (the dueling ring — the
                // only sibling — answers 1). NPC ride dialogs select the
                // content-defined affirmative or packed destination choice,
                // independent of the NPC operation index. Dragon Slayer
                // puts Crandor before the normal sailor fare.
                // Spirit trees are two pages: adult trees ask "Where can I
                // go?" before the dest list; answering the dest index on
                // the first page is "No thanks, old tree." and the hop
                // returns. Answer each distinct option-page once.
                let page = chat_page_key(snapshot);
                if hop.dialog_page.as_deref() != Some(page.as_str()) {
                    if let Some(choice) = hop_dialog_choice(
                        &hop.leg,
                        options.teleports,
                        options.edges,
                        snapshot.chat_options(),
                    ) {
                        if let Some(detail) = door_hop_choice_blocked(&edge, snapshot, choice) {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            return Poll::Terminal(TravelOutcome::Blocked {
                                at: here,
                                leg: self.leg_index,
                                detail,
                            });
                        }
                        match ix.answer_choice(choice) {
                            SendResult::Sent { .. } => {
                                hop.dialog_page = Some(page);
                                api::host_log!(
                                    Category::NavEvent,
                                    Level::Info,
                                    "answered choice {} for {} {}",
                                    choice,
                                    match edge.kind {
                                        TransportKind::SpiritTree => "spirit tree",
                                        TransportKind::Teleport => "jewellery",
                                        TransportKind::Door => "door",
                                        _ => "npc",
                                    },
                                    edge.loc_id
                                );
                            }
                            SendResult::Refused { .. } => {}
                        }
                    } else if edge.kind == TransportKind::Door {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        return Poll::Terminal(TravelOutcome::Blocked {
                            at: here,
                            leg: self.leg_index,
                            detail: "unrecognized journey dialogue".into(),
                        });
                    } else {
                        hop.npc_recovery.detail = Some("unrecognized journey dialogue");
                    }
                }
            } else if snapshot.modals().main == GLIDER_MAP_ROOT {
                // Gnome glider: after "Can you take me on the glider?" the
                // script `if_openmain(glidermap)` and dests are IF_BUTTON
                // on com_21..=25, not chat. Boat `ship_journey` has no
                // dest buttons — the hop just waits for the telejump.
                if let Some(com) = dest_map_component(&edge) {
                    let page = format!("if:{com}");
                    if hop.dialog_page.as_deref() != Some(page.as_str()) {
                        if let Some(widget) =
                            snapshot.widgets().iter().find(|w| w.component_id == com)
                        {
                            match ix.press(widget) {
                                SendResult::Sent { .. } => {
                                    hop.dialog_page = Some(page);
                                    api::host_log!(
                                        Category::NavEvent,
                                        Level::Info,
                                        "pressed glidermap {} for dest ({}, {}, {})",
                                        com,
                                        edge.to.x,
                                        edge.to.z,
                                        edge.to.level
                                    );
                                }
                                SendResult::Refused { .. } => {}
                            }
                        }
                    }
                }
            }
        }
        let close_enough = self.close_enough;
        let hop_dialog_started = hop.npc_recovery.dialog_started;
        let arrived_arm: Evidence<'static> = if edge.takeoff.is_some() {
            arrived(edge.to, 0)
        } else if edge.kind == TransportKind::Door && edge.dir.is_some() {
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                let Some(here) = now.world_tile() else {
                    return false;
                };
                if here.level != edge.to.level {
                    return false;
                }
                door_crossed(&edge, here)
                    && (here.x - edge.to.x).abs().max((here.z - edge.to.z).abs()) <= close_enough
            })
        } else if edge.kind == TransportKind::Door && edge.dir.is_none() {
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile()
                    .is_some_and(|here| door_dir_none_arrived(&edge, here, close_enough))
            })
        } else if is_essence_entry_edge(&edge) {
            // The entry teleport lands at a random `essence_mine_teleports`
            // coord — never the pad exactly — so any tile inside the
            // enclosed mine completes the hop (and latches the session).
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile().is_some_and(in_essence_mine)
            })
        } else if edge.kind == TransportKind::EssenceExit {
            // The exit portal teleports to `map_findsquare(anchor, 0, 2,
            // lineofwalk)`: a random standable tile within chebyshev 2 of
            // the wizard's anchor, never the anchor exactly.
            let to = edge.to;
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile().is_some_and(|t| {
                    t.level == to.level
                        && (t.x - to.x).abs().max((t.z - to.z).abs())
                            <= ESSENCE_MINE_EXIT_ARRIVE_RADIUS
                })
            })
        } else if edge.kind == TransportKind::Teleport {
            // `player_teleport_normal` lands at `map_findsquare(to, 0, 2,
            // lineofwalk)`: a random standable tile within chebyshev 2 of
            // the packed landing, never the tile exactly — so the hop
            // accepts that radius regardless of the runner's exact
            // `close_enough`.
            arrived(edge.to, TELEPORT_ARRIVE_RADIUS)
        } else if edge.kind == TransportKind::Glider {
            arrived(edge.to, GLIDER_ARRIVE_RADIUS)
        } else if edge.player_delta.is_some() {
            // The content displaces the player's actual takeoff stand. A
            // route's planned landing is not proof if live admission used a
            // different stand, and an unchanged tile cannot prove a crossing.
            let landing = hop
                .sent_tile
                .and_then(|sent| edge.landing_from(sent).map(|to| (sent, to)));
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                landing.is_some_and(|(sent, to)| {
                    now.world_tile().is_some_and(|here| {
                        here != sent
                            && here.level == to.level
                            && (here.x - to.x).abs().max((here.z - to.z).abs()) <= close_enough
                    })
                })
            })
        } else if matches!(edge.kind, TransportKind::Ladder | TransportKind::Stairs)
            && edge.at.level != edge.to.level
            && edge.at.x == edge.to.x
            && edge.at.z == edge.to.z
        {
            // Vertical moves preserve the player's stand adjacent to any side
            // of the live loc, not only adjacent to its south-west origin.
            let (origin, width, length) = hop.arrival_footprint.unwrap_or((edge.at, 1, 1));
            let radius = VERTICAL_ARRIVE_RADIUS.max(close_enough);
            Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                now.world_tile().is_some_and(|here| {
                    here.level == edge.to.level
                        && (origin.x - radius..=origin.x + width - 1 + radius).contains(&here.x)
                        && (origin.z - radius..=origin.z + length - 1 + radius).contains(&here.z)
                })
            })
        } else if (edge.to.z - edge.at.z).abs() == CELLAR_SHIFT && edge.to.level == edge.at.level {
            // `movecoord(coord(), 0, 0, ±6400)` lands on the player's tile,
            // one Chebyshev off the loc-baked dest when the hop is taken
            // from an adjacent stand. Host WalkNear uses close_enough 0.
            arrived(edge.to, CELLAR_ARRIVE_RADIUS.max(close_enough))
        } else {
            arrived(edge.to, close_enough)
        };
        let arms: [(&str, Evidence<'static>); 2] = [
            ("arrived", arrived_arm),
            (
                "unreachable",
                Box::new(move |now: &ReadContext<'_>, _before: &ReadContext<'_>| {
                    if npc_hop && hop_dialog_started {
                        return false;
                    }
                    Query::new(now.chat())
                        .since(hop_seq)
                        .text_contains(&["i can't reach that"])
                        .exists()
                }),
            ),
        ];
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched {
                arm: "unreachable", ..
            }) => {
                if npc_hop {
                    if hop.npc_recovery.waited > self.budget {
                        fire_leg(options, &hop.leg, LegPhase::Failed);
                        return Poll::Terminal(self.npc_expired(snapshot, &hop));
                    }
                    let poll = self.retry_npc_reach(d, snapshot, &mut hop, options);
                    if matches!(poll, Poll::Watching) {
                        self.transport = Some(hop);
                    }
                    return poll;
                }
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused {
                    at: here,
                    reason: SendReason::Unreachable,
                })
            }
            Some(Outcome::Matched { .. }) => {
                // Agility forcemove holds the player after they land on
                // `to`. Completing on the first arrived poll sends the
                // next walk into a locked player (live Yanille ledge
                // Dropped at 2580,9512). Packed `edge.ticks` is the anim
                // delay — stay on this hop until it elapses.
                if let Leg::Transport { edge } = &hop.leg {
                    let delay = edge.ticks.max(0) as u32;
                    if edge.kind == TransportKind::AgilityShortcut && delay > 0 {
                        // Count packed ticks from the first landed poll, not
                        // from the interact — the forcemove can land on `to`
                        // while the player is still locked (live ledge
                        // Dropped the next walk).
                        if hop.sent_tile.is_some() {
                            hop.sent_tile = None;
                            hop.ticks_waited = 0;
                        }
                        hop.ticks_waited += 1;
                        if hop.ticks_waited < delay {
                            self.transport = Some(hop);
                            return Poll::Watching;
                        }
                    }
                }
                // A completed essence entry hop latches the mine session:
                // the exit portal may only return to this wizard.
                if let Some(wizard) = entry_wizard {
                    if let Some(session) = essence_session_for_wizard(wizard) {
                        *essence = Some(session);
                        api::host_log!(
                            Category::NavEvent,
                            Level::Info,
                            "essence entry latched wizard {} -> {:?}",
                            session.wizard_npc,
                            session.return_tile
                        );
                    }
                }
                fire_leg(options, &hop.leg, LegPhase::Done);
                self.leg_index += 1;
                Poll::LegDone
            }
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: hop.to,
                    why: if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    },
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                self.transport = Some(hop);
                Poll::Watching
            }
            None => {
                if !npc_hop {
                    hop.ticks_waited += 1;
                }
                if npc_hop && hop.npc_recovery.waited > self.budget {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Poll::Terminal(self.npc_expired(snapshot, &hop));
                }
                if hop.ticks_waited > self.budget {
                    let why = if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    };
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    Poll::Terminal(TravelOutcome::Stalled {
                        at: here,
                        aiming: hop.to,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else {
                    self.transport = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// Loc-backed approach settle: adjacency or per-arm budget expiry.
    pub(super) fn poll_approach<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        let mut approach = hop.approach.take().expect("approach hop present");
        let here = here(snapshot);
        // NPC-backed approaches use poll_npc_approach and one leg clock.
        let edge = match &hop.leg {
            Leg::Transport { edge } => edge,
            Leg::Walk { .. } => unreachable!("transport approach holds a transport leg"),
        };
        let at = approach.at;
        let ready = if let Some(takeoff) = edge.takeoff {
            here == takeoff && loc_transport_ready(snapshot, edge, here) != Some(false)
        } else {
            loc_transport_ready(snapshot, edge, here)
                .unwrap_or(here.level == at.level && cheb(here, at) <= 1)
        };
        if approach.retry_pending && !ready {
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(approach.tile);
            report_walk(options, snapshot, here, approach.tile, &result);
            match result {
                SendResult::Sent { .. } => {
                    approach.retry_pending = false;
                    approach.ticks_waited = 0;
                    approach.sent_tick = snapshot.tick();
                    hop.sent_tile = Some(here);
                    hop.approach = Some(approach);
                    return Poll::Watching;
                }
                SendResult::Refused {
                    reason:
                        SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                    ..
                } => {}
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                }
            }
        }
        let arms: [(&str, Evidence<'static>); 1] = [(
            "arrived",
            Box::new(move |_now: &ReadContext<'_>, _before: &ReadContext<'_>| ready),
        )];
        let mut settle = Settle::new(
            SettleOptions {
                arms: &arms,
                // The run enforces the per-hop budget; the settle only
                // reports a disconnect (an arm read alone never lapses).
                budget_ticks: u32::MAX,
                budget_ms: None,
            },
            ReadContext::new(snapshot),
        );
        match settle.poll(ReadContext::new(snapshot)) {
            Some(Outcome::Matched { .. }) => Poll::LegDone,
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: approach.tile,
                    why: if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    },
                    tries: hop.tries.max(1),
                })
            }
            // `poll` never produces `Refused` (only `Interactions` does);
            // keep watching defensively.
            Some(Outcome::Refused { .. }) => {
                hop.approach = Some(approach);
                Poll::Watching
            }
            None => {
                approach.ticks_waited += 1;
                if approach.ticks_waited > self.budget {
                    let why = if hop.sent_tile == Some(here) {
                        HopFailure::Dropped
                    } else {
                        HopFailure::Expired
                    };
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    Poll::Terminal(TravelOutcome::Stalled {
                        at: here,
                        aiming: approach.tile,
                        why,
                        tries: hop.tries.max(1),
                    })
                } else {
                    hop.approach = Some(approach);
                    Poll::Watching
                }
            }
        }
    }

    /// One door-troll poll: read the door's open/closed state from the
    /// snapshot's locs: Open a closed door, walk through an open door,
    /// and continue to the exit once crossed without reopening behind us. Returns a terminal
    /// outcome (a refused send, or a missing-loc block after the loc-wait
    /// budget) or `None` to keep polling.
    pub(super) fn troll_door<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Option<TravelOutcome> {
        let edge = match &hop.leg {
            Leg::Transport { edge } => edge,
            Leg::Walk { .. } => unreachable!("troll hop holds a transport leg"),
        };
        let here = here(snapshot);
        // Arrival is still polled below, but a lapsed hop must not send an
        // Open that has no remaining observation window.
        if hop.ticks_waited >= self.budget {
            return None;
        }
        let tile = door_tile(edge);
        if crate::debug_enabled() {
            api::host_log!(
                Category::NavTrace,
                Level::Debug,
                "troll here={here:?} at={tile:?} cheb={} ticks_waited={} loc_wait={}",
                cheb(here, edge.at),
                hop.ticks_waited,
                self.loc_wait
            );
        }
        // Once on the destination side, a closer behind us must not pull
        // us back. Use the same directional/level evidence as arrival;
        // directionless edges cannot establish crossing from position.
        let crossed = edge.dir.is_some() && here.level == edge.to.level && door_crossed(edge, here);
        if crossed {
            self.loc_wait = 0;
            if cheb(here, hop.to) <= self.close_enough {
                return None; // Let the normal settle arm finish the leg.
            }
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(hop.to);
            report_walk(options, snapshot, here, hop.to, &result);
            return match result {
                SendResult::Sent { .. } => None,
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    Some(TravelOutcome::Refused { at: here, reason })
                }
            };
        }
        // The live loc's tile can sit a tile or two off the derived `at`, so
        // match the edge's closed `loc_id`, or the open leaf's `open_loc_id`
        // when the door reads open, by footprint within chebyshev 3 of `at`,
        // nearest first ([`find_door_loc`]); shifted double-door leaves
        // (castle 1519 → 1520) included.
        let leaf = find_door_loc(snapshot, edge);
        let open = match leaf {
            Some(loc) => loc.id != edge.loc_id,
            None => edge_loc_open(snapshot, edge),
        };
        hop.door_retry.observe(open);
        if let Some(sent_tick) = hop.open_sent_tick.take() {
            if snapshot.tick() == sent_tick {
                hop.open_sent_tick = Some(sent_tick);
                return None;
            }
            if door_step_pending(edge, here) {
                if open || SceneQuery::new(snapshot.scene(), None).can_step(here, edge.to) {
                    let mut ix = Interactions::new(snapshot, d);
                    let result = ix.pending_door_step(edge.to);
                    report_walk(options, snapshot, here, edge.to, &result);
                    return match result {
                        SendResult::Sent { .. } => None,
                        SendResult::Refused { reason, .. } => {
                            fire_leg(options, &hop.leg, LegPhase::Failed);
                            Some(TravelOutcome::Refused { at: here, reason })
                        }
                    };
                }
                // The server walked the player onto `at` before the queued
                // Open fired, and the wall still blocks `at` → `to`: a walk
                // packet now would cancel that Open. Probe again next poll.
                hop.open_sent_tick = Some(sent_tick);
            }
        }
        // Adjacency is needed to Open a closed door, not to walk through
        // an open one. Re-approaching an open door countermanded the exit
        // walk whenever its destination was several tiles beyond the door.
        if cheb(here, edge.at) > 1 && !open {
            let Some(approach) = approach_tile(snapshot, edge.at, here) else {
                // No standable tile adjacent to the door in the loaded
                // scene: keep waiting, bounded by the hop budget.
                self.loc_wait += 1;
                if self.loc_wait > self.budget {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Blocked {
                        at: here,
                        leg: self.leg_index,
                        detail: format!(
                            "troll door loc {} has no standable tile within 1 of {tile:?} in the loaded scene",
                            edge.loc_id
                        ),
                    });
                }
                return None;
            };
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(approach);
            report_walk(options, snapshot, here, approach, &result);
            match result {
                SendResult::Sent { .. } => {}
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Refused { at: here, reason });
                }
            }
            return None;
        }
        let Some(loc) = leaf else {
            // The door's loc is not in the loaded scene yet (the loc
            // family is stale, or the door is out of view): keep waiting,
            // bounded by the hop budget.
            self.loc_wait += 1;
            if self.loc_wait > self.budget {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Some(TravelOutcome::Blocked {
                    at: here,
                    leg: self.leg_index,
                    detail: format!(
                        "troll door loc {} is not at {tile:?} in the loaded scene",
                        edge.loc_id
                    ),
                });
            }
            return None;
        };
        self.loc_wait = 0;
        let mut ix = Interactions::new(snapshot, d);
        if crate::debug_enabled() {
            api::host_log!(
                Category::NavTrace,
                Level::Debug,
                "troll door loc={} at={:?} open={} closed_id={}",
                loc.id,
                loc.tile,
                open,
                edge.loc_id
            );
        }
        // OP_LOC1 on an open door is Close. Closed: Open, paced by
        // [`DoorRetry`]. Open: walk through this tick (do not click — that
        // slams it in the walker's face and they turn back to the door).
        if !open {
            if !hop.door_retry.may_reopen(snapshot.tick()) {
                return None;
            }
            match interact_transport(
                snapshot,
                &mut ix,
                TransportTarget::Loc(loc),
                edge,
                &mut options.on_event,
            ) {
                SendResult::Sent { .. } => {
                    hop.open_sent_tick = Some(snapshot.tick());
                    hop.tries = hop.tries.saturating_add(1);
                    hop.door_retry.sent(snapshot.tick());
                    api::host_log!(Category::NavEvent, Level::Info, "door open sent");
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Some(TravelOutcome::Refused { at: here, reason });
                }
            }
            return None;
        }
        let result = ix.walk(hop.to);
        report_walk(options, snapshot, here, hop.to, &result);
        match result {
            SendResult::Sent { .. } => {
                api::host_log!(
                    Category::NavTrace,
                    Level::Debug,
                    "door walk-through sent to {:?}",
                    hop.to
                );
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Some(TravelOutcome::Refused { at: here, reason });
            }
        }
        None
    }
}
