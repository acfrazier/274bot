use super::*;

/// The live target of a transport hop: the edge's loc view (doors,
/// ladders, stairs, boats, agility, gliders, spirit trees) or — for an
/// NPC-triggered hop (`TransportKind::Npc`: cart, essence-mine wizard,
/// Elkoy) — the edge's NPC view.
pub(super) enum TransportTarget<'s> {
    Loc(&'s LocView),
    Npc(&'s NpcView),
}

/// The snapshot target for a transport edge: the edge's `loc_id` within
/// 3 tiles of `edge.at` for loc edges ([`find_transport_loc`]), or the
/// edge's npc type id within [`NPC_SEARCH_RADIUS`] of `edge.at` for
/// [`TransportKind::Npc`] edges ([`NPC_SEARCH_RADIUS`], nearest first).
/// An Npc edge's `loc_id` is the npc.pack type id, matched against the
/// live NPC's `r#type`.
pub(super) fn find_transport_target<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
) -> Option<TransportTarget<'s>> {
    if npc_backed(edge) {
        snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.r#type == Some(edge.loc_id as usize) && npc.tile.level == edge.at.level
            })
            .map(|npc| (npc, cheb(npc.tile, edge.at)))
            .filter(|(_, gap)| *gap <= NPC_SEARCH_RADIUS)
            .min_by_key(|(_, gap)| *gap)
            .map(|(npc, _)| TransportTarget::Npc(npc))
    } else {
        find_transport_loc(snapshot, edge).map(TransportTarget::Loc)
    }
}

/// Send the transport hop's interact for the edge's target kind: an
/// `op_loc` for loc edges, an `op_npc` for `TransportKind::Npc` edges,
/// both with the edge's `option`. `option` 0 on a loc hop uses the first
/// `item_req` obj on the loc (`oplocu` — unequippable knife on a web).
pub(super) fn interact_transport<'t>(
    snapshot: &'t GameSnapshot,
    ix: &mut Interactions<'t>,
    target: TransportTarget<'t>,
    edge: &TransportEdge,
    options: &mut TravelOptions<'_>,
) -> SendResult<'t> {
    let (actual_id, tile) = match &target {
        TransportTarget::Loc(l) => (l.id, l.tile),
        TransportTarget::Npc(n) => (n.r#type.map(|id| id as i32).unwrap_or(-1), n.tile),
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
    if let Some(cb) = options.on_event.as_mut() {
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

/// The block message's target word for an edge: "npc" for
/// [`TransportKind::Npc`] edges, "loc" for every loc-targeted kind.
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
/// then the sibling list, or the young tree's "Yes please."), and the
/// Shantay henge's gated branch (loc 4031 `oploc1` in `shantay_pass.rs2`)
/// shows the pass handover (`~chatnpc`/`~objbox`/`~chatplayer`, each a
/// `p_pausebutton` chat modal) before consuming the pass and teleporting.
/// A plain door hop never opens chat, and the toll gates' branch choices
/// differ (their follow is not driven here).
pub(super) fn drives_hop_dialogs(edge: &TransportEdge) -> bool {
    npc_backed(edge)
        || edge.kind == TransportKind::SpiritTree
        || (edge.kind == TransportKind::Teleport && edge.loc_id > 0)
        || (edge.kind == TransportKind::Door && edge.loc_id == SHANTAY_HENGE_LOC_ID)
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
            && loc
                .actions
                .iter()
                .flatten()
                .any(|action| action.trim().eq_ignore_ascii_case("close"))
    })
}

/// The chat ring's latest sequence — the hop-start watermark for the
/// "I can't reach that!" watch (the `settle::said` sequence delta; never
/// a stale-head check on the single most recent line).
pub(super) fn chat_seq(snapshot: &GameSnapshot) -> i32 {
    Query::new(snapshot.chat_lines()).latest_sequence()
}

/// The door's own tile: the edge's `at` — in the new edge model `at` IS
/// the loc tile (the interact target), so no midpoint derivation. The
/// door-troll read compares the loc's live id at this tile against the
/// edge's closed id.
pub(super) fn door_tile(edge: &TransportEdge) -> WorldTile {
    edge.at
}

/// Whether `here` has crossed a door edge to its far side. `dir` is the
/// wall's crossing direction; proximity alone can match a near-side
/// approach tile to `to` when `close_enough` is 2.
pub(super) fn door_crossed(edge: &TransportEdge, here: WorldTile) -> bool {
    match edge.dir {
        Some(DoorDir::N) => here.z > edge.at.z,
        Some(DoorDir::S) => here.z < edge.at.z,
        Some(DoorDir::E) => here.x > edge.at.x,
        Some(DoorDir::W) => here.x < edge.at.x,
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
    /// within `close_enough` of `to` on the destination level), fail fast on
    /// the game's "I can't reach that!" chat (a line new since the hop
    /// started — the `settle::said` sequence delta), or lapse the budget.
    /// A door hop that lapses its cheap budget escalates to the automatic
    /// troll (see [`FollowRun::troll_door`]); only a troll hop (or a
    /// non-door transport) lapses to the real `Stalled`.
    pub(super) fn poll_transport<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        options: &mut TravelOptions,
        essence: &mut Option<EssenceSession>,
    ) -> Poll {
        let mut hop = self.transport.take().expect("transport hop present");
        let here = here(snapshot);
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
                waited: hop.ticks_waited,
                loc_wait: self.loc_wait,
                budget: self.budget,
                live_loc: find_transport_loc(snapshot, edge).map(|l| (l.id, l.tile)),
            });
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
                            if crate::debug_enabled() {
                                eprintln!("[nav-transport] cheap hop walk-through to {:?}", hop.to);
                            }
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
                            if crate::debug_enabled() {
                                eprintln!("[nav-transport] cheap hop door step to {:?}", edge.to);
                            }
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
                    && edge_loc_open(snapshot, edge)
                    && hop.tries == 0
                {
                    return match find_transport_target(snapshot, edge) {
                        Some(target) => {
                            let mut ix = Interactions::new(snapshot, d);
                            match interact_transport(snapshot, &mut ix, target, edge, options) {
                                SendResult::Sent { .. } => {
                                    hop.tries = 1;
                                    hop.ticks_waited = 0;
                                    hop.sent_tile = Some(here);
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
        // The pre-interact approach: the player must stand within chebyshev
        // 1 of the loc (or the driver NPC) before the game accepts an
        // interact. While the approach is armed, watch it instead of the
        // arrive arm; only once adjacent does the hop find the target,
        // interact, and settle `arrived(to)`.
        if hop.approach.is_some() {
            match self.poll_approach(d, snapshot, &mut hop, options) {
                Poll::Watching => {
                    self.transport = Some(hop);
                    return Poll::Watching;
                }
                Poll::Terminal(outcome) => return Poll::Terminal(outcome),
                // The player is adjacent now: fall through, send the loc
                // interact, then watch `arrived(to)`.
                Poll::LegDone => {}
            }
            let edge = match &hop.leg {
                Leg::Transport { edge } => edge.clone(),
                Leg::Walk { .. } => unreachable!("transport hop holds a transport leg"),
            };
            return match find_transport_target(snapshot, &edge) {
                Some(target) => {
                    let mut ix = Interactions::new(snapshot, d);
                    match interact_transport(snapshot, &mut ix, target, &edge, options) {
                        SendResult::Sent { .. } => {
                            self.loc_wait = 0;
                            hop.ticks_waited = 0;
                            hop.sent_tile = Some(here);
                            if edge.kind == TransportKind::Door {
                                hop.open_sent_tick = Some(snapshot.tick());
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
                    // The target has not appeared in the loaded scene yet:
                    // keep waiting, bounded by the hop budget.
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
        // of sitting out the settle budget. `chat_seq` is the hop-start
        // watermark, so only genuinely new lines count.
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
            eprintln!(
                "[nav-transport] here={here:?} to={:?} troll={} ticks_waited={} loc_id={} open={}",
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
        // ring's single arena first and "Nowhere." last), and the Shantay
        // henge's gated branch (loc 4031 `oploc1`) shows the pass
        // handover (`~chatnpc`/`~objbox`/`~chatplayer`, each a
        // `p_pausebutton` chat modal) before consuming the pass and
        // teleporting. Drive the dialog the same way: press the modal's
        // continue button while it is up (each press advances a page —
        // including the post-choice "Great!" pages and mesboxes before
        // the ride), and press the ride choice exactly once when the
        // choice page is up, then keep watching `arrived(to)` for the
        // ride. A plain door hop (and the toll gates, whose branch
        // choices differ) never drives chat here.
        if drives_hop_dialogs(&edge) {
            let mut ix = Interactions::new(snapshot, d);
            if snapshot.chat_continue_component_id() != -1 {
                match ix.continue_dialog() {
                    SendResult::Sent { .. } => {
                        if crate::debug_enabled() {
                            eprintln!("[nav-transport] continued the npc {} dialog", edge.loc_id);
                        }
                    }
                    SendResult::Refused { .. } => {}
                }
            } else if !snapshot.chat_options().is_empty() {
                // A jewellery rub's destination choice is the edge's case
                // index: the script maps the answered option through
                // `switch_int($choice)`, so the choice of the edge being
                // followed is the 1-based index of its `to` among the
                // packed same-`loc_id` rub edges (the dueling ring — the
                // only sibling — answers 1). Npc ride dialogs (cart fare,
                // Elkoy escort) always answer the modal's FIRST choice,
                // independent of the NPC op index (`edge.option`: Talk-to
                // is op 1, the essence wizard's teleport op 3/4).
                // Spirit trees are two pages: adult trees ask "Where can I
                // go?" before the dest list; answering the dest index on
                // the first page is "No thanks, old tree." and the hop
                // returns. Answer each distinct option-page once.
                let page = chat_page_key(snapshot);
                if hop.dialog_page.as_deref() != Some(page.as_str()) {
                    let choice = hop_dialog_choice(
                        &hop.leg,
                        options.teleports,
                        options.edges,
                        snapshot.chat_options().len(),
                    );
                    match ix.answer_choice(choice) {
                        SendResult::Sent { .. } => {
                            hop.dialog_page = Some(page);
                            if crate::debug_enabled() {
                                eprintln!(
                                    "[nav-transport] answered choice {} for {} {}",
                                    choice,
                                    match edge.kind {
                                        TransportKind::SpiritTree => "spirit tree",
                                        TransportKind::Teleport => "jewellery",
                                        _ => "npc",
                                    },
                                    edge.loc_id
                                );
                            }
                        }
                        SendResult::Refused { .. } => {}
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
                                    if crate::debug_enabled() {
                                        eprintln!(
                                            "[nav-transport] pressed glidermap {} for dest ({}, {}, {})",
                                            com, edge.to.x, edge.to.z, edge.to.level
                                        );
                                    }
                                }
                                SendResult::Refused { .. } => {}
                            }
                        }
                    }
                }
            }
        }
        let close_enough = self.close_enough;
        let arrived_arm: Evidence<'static> = if edge.kind == TransportKind::Door
            && edge.dir.is_some()
        {
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
                        if crate::debug_enabled() {
                            eprintln!(
                                "[nav-transport] essence entry latched wizard {} -> {:?}",
                                session.wizard_npc, session.return_tile
                            );
                        }
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
                    why: HopFailure::Dropped,
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
                hop.ticks_waited += 1;
                if hop.ticks_waited > self.budget {
                    // The cheap one-interact door hop lapsed: a door the
                    // closer keeps slamming can never cross that way, so
                    // escalate this same leg to the automatic troll
                    // instead of stalling. Re-open while closed, probe the
                    // adjacent crossing after Open, and walk when open; only
                    // a troll hop that lapses again — or a
                    // non-door transport — returns the real `Stalled`.
                    let door_leg = matches!(
                        &hop.leg,
                        Leg::Transport { edge } if edge.kind == TransportKind::Door
                    );
                    if door_leg && !hop.troll {
                        hop.troll = true;
                        hop.ticks_waited = 0;
                        // The troll arms its own probe when it sends Open.
                        hop.open_sent_tick = None;
                        self.transport = Some(hop);
                        Poll::Watching
                    } else {
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
                    }
                } else {
                    self.transport = Some(hop);
                    Poll::Watching
                }
            }
        }
    }

    /// One approach-hop settle step: match the adjacency arm
    /// (`arrived(at, 1)` — only once the player stands within chebyshev 1
    /// of the loc may the hop send the interact), or lapse the budget. A
    /// door hop whose approach lapses escalates to the automatic troll
    /// (the cheap hop lapsed before its first interact — the troll walks
    /// the player to the door too); only a non-door transport lapses to
    /// the real `Stalled`. The approach walk itself was sent when the hop
    /// was armed; a known stun may rearm that walk once before settling.
    pub(super) fn poll_approach<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        let mut approach = hop.approach.take().expect("approach hop present");
        let here = here(snapshot);
        if approach.retry_pending
            && (here.level != approach.at.level || cheb(here, approach.at) > 1)
        {
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
        let arms = [("arrived", arrived(approach.at, 1))];
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
            Some(Outcome::Matched { .. }) => {
                // The player is adjacent: the caller sends the loc interact.
                Poll::LegDone
            }
            Some(Outcome::Expired { .. }) => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Stalled {
                    at: here,
                    aiming: approach.tile,
                    why: HopFailure::Dropped,
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
                    // The approach walk never landed: for a door, escalate
                    // this same leg to the automatic troll instead of
                    // stalling; only a troll hop that lapses again — or a
                    // non-door transport — returns the real `Stalled`.
                    let door_leg = matches!(
                        &hop.leg,
                        Leg::Transport { edge } if edge.kind == TransportKind::Door
                    );
                    if door_leg && !hop.troll {
                        hop.troll = true;
                        hop.approach = None;
                        hop.ticks_waited = 0;
                        Poll::Watching
                    } else {
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
                    }
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
            Leg::Transport { edge } => edge.clone(),
            Leg::Walk { .. } => unreachable!("troll hop holds a transport leg"),
        };
        let here = here(snapshot);
        let tile = door_tile(&edge);
        if crate::debug_enabled() {
            eprintln!(
                "[nav-troll] here={here:?} at={tile:?} cheb={} ticks_waited={} loc_wait={}",
                cheb(here, edge.at),
                hop.ticks_waited,
                self.loc_wait
            );
        }
        // Once on the destination side, a closer behind us must not pull
        // us back. Use the same directional/level evidence as arrival;
        // directionless edges cannot establish crossing from position.
        let crossed =
            edge.dir.is_some() && here.level == edge.to.level && door_crossed(&edge, here);
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
        if let Some(sent_tick) = hop.open_sent_tick.take() {
            if snapshot.tick() == sent_tick {
                hop.open_sent_tick = Some(sent_tick);
                return None;
            }
            if door_step_pending(&edge, here) {
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
        }
        // Adjacency is needed to Open a closed door, not to walk through
        // an open one. Re-approaching an open door countermanded the exit
        // walk whenever its destination was several tiles beyond the door.
        if cheb(here, edge.at) > 1 && !edge_loc_open(snapshot, &edge) {
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
        // The live loc's tile can sit a tile or two off the derived `at`
        // (the cheap hop's `find_transport_loc` already tolerates that),
        // so an exact-tile lookup misses it and the troll blocks while
        // the walker stands still. Search by id within chebyshev 3 of
        // `at` instead — the edge's closed `loc_id`, or the open leaf's
        // `open_loc_id` when the door reads open — nearest first, same
        // shape as `find_transport_loc`.
        let Some(loc) = snapshot
            .locs()
            .iter()
            .filter(|loc| {
                loc.tile.level == tile.level
                    && (loc.id == edge.loc_id
                        || edge.open_loc_id.is_some_and(|open_id| loc.id == open_id))
            })
            .map(|loc| (loc, cheb(loc.tile, tile)))
            .filter(|(_, gap)| *gap <= 3)
            .min_by_key(|(_, gap)| *gap)
            .map(|(loc, _)| loc)
        else {
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
        let open = loc.id != edge.loc_id;
        let mut ix = Interactions::new(snapshot, d);
        if crate::debug_enabled() {
            eprintln!(
                "[nav-troll] door loc={} at={:?} open={} closed_id={}",
                loc.id, loc.tile, open, edge.loc_id
            );
        }
        // OP_LOC1 on an open door is Close. Closed: Open. Open: walk
        // through this tick (do not click — that slams it in the walker's
        // face and they turn back to the door).
        if !open {
            match interact_transport(snapshot, &mut ix, TransportTarget::Loc(loc), &edge, options) {
                SendResult::Sent { .. } => {
                    hop.open_sent_tick = Some(snapshot.tick());
                    if crate::debug_enabled() {
                        eprintln!("[nav-troll] Open SENT");
                    }
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
                if crate::debug_enabled() {
                    eprintln!("[nav-troll] walk-through SENT to {:?}", hop.to);
                }
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                return Some(TravelOutcome::Refused { at: here, reason });
            }
        }
        None
    }
}
