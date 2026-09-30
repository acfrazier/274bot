use super::*;

const NPC_HOP_ATTEMPTS: u32 = 3;

#[derive(Default)]
pub(super) struct NpcRecovery {
    /// One leg clock, including approach, retries and fare dialogue.
    pub waited: u32,
    pub last_tick: Option<u32>,
    pub failed: [Option<(usize, WorldTile)>; NPC_HOP_ATTEMPTS as usize],
    pub dialog_started: bool,
    pub detail: Option<&'static str>,
}

fn cardinal_stand(snapshot: &GameSnapshot, npc: &NpcView, stand: WorldTile) -> bool {
    if stand.level != npc.network.level || !scene_standable(snapshot, stand) {
        return false;
    }
    let at = npc.network;
    let size = npc.size.max(1);
    let east = at.x + size - 1;
    let north = at.z + size - 1;
    let (mask, target, opposite) = if stand.x == at.x - 1 && (at.z..=north).contains(&stand.z) {
        (
            CollisionFlag::W_E,
            WorldTile { x: at.x, ..stand },
            CollisionFlag::W_W,
        )
    } else if stand.x == east + 1 && (at.z..=north).contains(&stand.z) {
        (
            CollisionFlag::W_W,
            WorldTile { x: east, ..stand },
            CollisionFlag::W_E,
        )
    } else if stand.z == at.z - 1 && (at.x..=east).contains(&stand.x) {
        (
            CollisionFlag::W_N,
            WorldTile { z: at.z, ..stand },
            CollisionFlag::W_S,
        )
    } else if stand.z == north + 1 && (at.x..=east).contains(&stand.x) {
        (
            CollisionFlag::W_S,
            WorldTile { z: north, ..stand },
            CollisionFlag::W_N,
        )
    } else {
        return false;
    };
    let scene = SceneQuery::new(snapshot.scene(), None);
    scene
        .collision_at(stand)
        .is_some_and(|flags| flags & mask == 0)
        && scene
            .collision_at(target)
            .is_some_and(|flags| flags & opposite == 0)
}

fn npc_ring(npc: &NpcView) -> impl Iterator<Item = WorldTile> {
    let at = npc.network;
    let size = npc.size.max(1);
    (-1..=size).flat_map(move |dx| {
        (-1..=size).filter_map(move |dz| {
            (dx == -1 || dx == size || dz == -1 || dz == size).then_some(WorldTile {
                x: at.x + dx,
                z: at.z + dz,
                level: at.level,
            })
        })
    })
}

/// Both stand selection and readiness use the engine's rectangle sides and
/// shared-edge wall masks. Diagonals are a last resort, never the NPC tile.
pub(super) fn npc_interaction_ready(
    snapshot: &GameSnapshot,
    here: WorldTile,
    npc: &NpcView,
) -> bool {
    if cardinal_stand(snapshot, npc, here) {
        return true;
    }
    if npc_ring(npc).any(|stand| cardinal_stand(snapshot, npc, stand)) {
        return false;
    }
    // A diagonal fallback still needs a clear corner approach in the scene.
    let at = npc.network;
    let size = npc.size.max(1);
    let corner = WorldTile {
        x: here.x.clamp(at.x, at.x + size - 1),
        z: here.z.clamp(at.z, at.z + size - 1),
        level: at.level,
    };
    (here.x - corner.x).abs() == 1
        && (here.z - corner.z).abs() == 1
        && scene_standable(snapshot, here)
        && SceneQuery::new(snapshot.scene(), None).can_step(here, corner)
}

/// One scene flood ranks every candidate stand/instance by actual reachable
/// route cost (BFS dequeue order), not distance from the packed spawn. The
/// tracked slot may wander beyond the initial search radius. Failed stands
/// and instances are lower priority, but remain usable when no alternative
/// exists. No flood is built when the only candidate is already operable.
pub(super) fn select_npc_approach<'s>(
    snapshot: &'s GameSnapshot,
    edge: &TransportEdge,
    here: WorldTile,
    tracked: Option<usize>,
    failed: &[Option<(usize, WorldTile)>],
) -> Option<(&'s NpcView, WorldTile)> {
    let candidates = || {
        snapshot.npcs().iter().filter(|npc| {
            npc.r#type == Some(edge.loc_id as usize)
                && npc.network.level == edge.at.level
                && (tracked == Some(npc.index) || cheb(npc.network, edge.at) <= NPC_SEARCH_RADIUS)
        })
    };
    let mut iter = candidates();
    let first = iter.next()?;
    if iter.next().is_none()
        && failed.iter().all(Option::is_none)
        && npc_interaction_ready(snapshot, here, first)
    {
        return Some((first, here));
    }
    let scene = snapshot.scene();
    let flood = SceneQuery::new(scene, Some(here)).flood_reach()?;
    let ranks = flood.ranks().0;
    let flood = &flood;
    candidates()
        .flat_map(|npc| {
            npc_ring(npc).filter_map(move |stand| {
                if !npc_interaction_ready(snapshot, stand, npc) || !flood.at(&stand).0 {
                    return None;
                }
                let index =
                    ((stand.x - scene.base_x) * scene.height + stand.z - scene.base_z) as usize;
                let cost = *ranks.get(index)?;
                let failed_instance = failed.iter().flatten().any(|(id, _)| *id == npc.index);
                let failed_stand = failed
                    .iter()
                    .flatten()
                    .any(|pair| *pair == (npc.index, stand));
                Some((npc, stand, (failed_instance, failed_stand, cost, npc.index)))
            })
        })
        .min_by_key(|(_, _, cost)| *cost)
        .map(|(npc, stand, _)| (npc, stand))
}

impl FollowRun {
    pub(super) fn npc_expired(&self, snapshot: &GameSnapshot, hop: &TransportHop) -> TravelOutcome {
        let Leg::Transport { edge } = &hop.leg else {
            unreachable!()
        };
        let missing = find_transport_target_instance(snapshot, edge, hop.npc_index).is_none();
        if let Some(detail) = missing
            .then_some("tracked target missing")
            .or(hop.npc_recovery.detail)
        {
            TravelOutcome::Blocked {
                at: here(snapshot),
                leg: self.leg_index,
                detail: format!(
                    "npc hop {}: {detail}; {} attempts, leg budget {} ticks exhausted",
                    edge.loc_id, hop.tries, self.budget
                ),
            }
        } else {
            TravelOutcome::Stalled {
                at: here(snapshot),
                aiming: hop.approach.as_ref().map_or(hop.to, |a| a.tile),
                why: if hop.sent_tile == Some(here(snapshot)) {
                    HopFailure::Dropped
                } else {
                    HopFailure::Expired
                },
                tries: hop.tries,
            }
        }
    }

    pub(super) fn poll_npc_approach<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        let Leg::Transport { edge } = &hop.leg else {
            unreachable!()
        };
        let here = here(snapshot);
        let tracked = find_transport_target_instance(snapshot, edge, hop.npc_index);
        let current = match tracked {
            Some(TransportTarget::Npc(npc)) => Some(npc),
            _ => None,
        };
        let approach = hop.approach.as_ref().expect("NPC approach armed");
        // Retarget before the obsolete walk settles. Otherwise a valid current
        // stand stays in flight without doing another flood or sending a click.
        let stable = current.is_some_and(|npc| {
            npc.network == approach.at && npc_interaction_ready(snapshot, approach.tile, npc)
        });
        let ready = current.is_some_and(|npc| npc_interaction_ready(snapshot, here, npc));
        if stable && !ready && !approach.retry_pending {
            return Poll::Watching;
        }
        let Some((npc, stand)) = select_npc_approach(
            snapshot,
            edge,
            here,
            hop.npc_index,
            &hop.npc_recovery.failed,
        ) else {
            hop.npc_recovery.detail = Some(if current.is_none() {
                "tracked target missing"
            } else {
                "no reachable operable stand"
            });
            return Poll::Watching;
        };
        hop.npc_index = Some(npc.index);
        hop.npc_recovery.detail = None;
        if here != stand || !npc_interaction_ready(snapshot, here, npc) {
            let old = hop.approach.as_mut().unwrap();
            if old.tile == stand && old.at == npc.network && !old.retry_pending {
                return Poll::Watching;
            }
            let mut ix = Interactions::new(snapshot, d);
            let result = ix.walk(stand);
            report_walk(options, snapshot, here, stand, &result);
            match result {
                SendResult::Sent { .. } => {
                    *old = ApproachHop {
                        tile: stand,
                        at: npc.network,
                        sent_tick: snapshot.tick(),
                        ticks_waited: 0,
                        retry_pending: false,
                    };
                    hop.sent_tile = Some(here);
                }
                SendResult::Refused {
                    reason:
                        SendReason::OffScene | SendReason::Unreachable | SendReason::SceneUnavailable,
                    ..
                } => {
                    old.retry_pending = true;
                    hop.npc_recovery.detail = Some("approach route refused");
                }
                SendResult::Refused { reason, .. } => {
                    fire_leg(options, &hop.leg, LegPhase::Failed);
                    return Poll::Terminal(TravelOutcome::Refused { at: here, reason });
                }
            }
            return Poll::Watching;
        }
        let watermark = chat_seq(snapshot);
        let mut ix = Interactions::new(snapshot, d);
        match interact_transport(snapshot, &mut ix, TransportTarget::Npc(npc), edge, options) {
            SendResult::Sent { .. } => {
                hop.tries += 1;
                hop.chat_seq = watermark;
                hop.sent_tile = Some(here);
                hop.approach = None;
                hop.dialog_page = None;
                self.loc_wait = 0;
                Poll::Watching
            }
            SendResult::Refused { reason, .. } => {
                fire_leg(options, &hop.leg, LegPhase::Failed);
                Poll::Terminal(TravelOutcome::Refused { at: here, reason })
            }
        }
    }

    pub(super) fn retry_npc_reach<D: Driver>(
        &mut self,
        d: &mut D,
        snapshot: &GameSnapshot,
        hop: &mut TransportHop,
        options: &mut TravelOptions<'_>,
    ) -> Poll {
        if hop.tries >= NPC_HOP_ATTEMPTS {
            let Leg::Transport { edge } = &hop.leg else {
                unreachable!()
            };
            fire_leg(options, &hop.leg, LegPhase::Failed);
            return Poll::Terminal(TravelOutcome::Blocked {
                at: here(snapshot),
                leg: self.leg_index,
                detail: format!(
                    "npc hop {} gave up after {} attempts: reach failure",
                    edge.loc_id, hop.tries
                ),
            });
        }
        if let Some(index) = hop.npc_index {
            hop.npc_recovery.failed[(hop.tries.saturating_sub(1)) as usize] =
                Some((index, hop.sent_tile.unwrap_or_else(|| here(snapshot))));
        }
        hop.chat_seq = chat_seq(snapshot);
        hop.npc_recovery.detail = Some("reach failure");
        hop.approach = Some(ApproachHop {
            tile: here(snapshot),
            at: here(snapshot),
            sent_tick: snapshot.tick(),
            ticks_waited: 0,
            retry_pending: true,
        });
        self.poll_npc_approach(d, snapshot, hop, options)
    }
}
