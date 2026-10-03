use super::select::TargetPlan;
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::gather_methods::{known_rows, GatherCatalog, GatherMethod, TargetClass};
use api::selected::{EntityId, Knowledge};
use std::sync::Arc;
use std::task::Poll;

pub const DEFAULT_STALL_TICKS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatherEnd {
    Full,
    Depleted,
    Hazard,
    TargetGone,
    Refused,
    Idle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatherResult {
    pub end: GatherEnd,
    pub gained: u32,
    pub xp: i32,
}

pub struct GatherRunArgs {
    pub target: TargetPlan,
    pub catalog: Arc<GatherCatalog>,
    pub stall_ticks: u64,
}

pub struct GatherRun {
    args: GatherRunArgs,
    request_id: Option<u64>,
    last_emit_tick: u64,
    chat_since: i32,
    baseline_products: i32,
    baseline_xp: i32,
    gained: u32,
    xp_gain: i32,
    quiet_ticks: u64,
    retried: bool,
}

impl NativeMachine for GatherRun {
    type Args = GatherRunArgs;
    type Output = GatherResult;

    fn begin(args: Self::Args, cx: &mut ActionContext<'_>) -> Result<Self, ActionError> {
        let snapshot = cx.snapshot();
        let Some(method) = args
            .catalog
            .methods()
            .get(usize::from(args.target.method_index))
        else {
            return Err(ActionError::Unavailable(
                "gather method is not in the prepared catalog".into(),
            ));
        };
        if target_class(method, args.target.entity) != Some(TargetClass::Resource) {
            return Err(ActionError::Unavailable(
                "gather target is not a resource".into(),
            ));
        }
        if matches!(args.target.entity, EntityId::Npc(_)) && args.target.npc_index < 0 {
            return Err(ActionError::Unavailable(
                "gather NPC target has no observed instance index".into(),
            ));
        }
        if observe_target(snapshot, &args.catalog, &args.target) != Some(TargetObservation::Active)
        {
            return Err(ActionError::Unavailable(
                "gather target is no longer observed at its selected identity".into(),
            ));
        }
        let Some(inventory) = snapshot.inventory() else {
            return Err(ActionError::Unavailable(
                "gather inventory is not ready".into(),
            ));
        };
        let Some(stats) = snapshot.stats() else {
            return Err(ActionError::Unavailable(
                "gather stats are not ready".into(),
            ));
        };
        let baseline_products = product_count(inventory.value, &args.target);
        let baseline_xp = stat_xp(stats.value, args.target.skill_stat);
        let chat_since = latest_chat(snapshot).unwrap_or(0);
        let mut machine = Self {
            args,
            request_id: None,
            last_emit_tick: cx.evidence().tick.wrapping_sub(1),
            chat_since,
            baseline_products,
            baseline_xp,
            gained: 0,
            xp_gain: 0,
            quiet_ticks: 0,
            retried: false,
        };
        machine.emit_click(cx)?;
        Ok(machine)
    }

    fn poll(&mut self, cx: &mut ActionContext<'_>) -> Poll<Result<Self::Output, ActionError>> {
        let snapshot = cx.snapshot();
        let refused = self
            .request_id
            .and_then(|request_id| cx.interaction_receipt(request_id))
            .map(|receipt| {
                self.request_id = None;
                !receipt.accepted
            })
            .unwrap_or(false);

        let Some(inventory) = snapshot.inventory() else {
            return Poll::Pending;
        };
        let Some(stats) = snapshot.stats() else {
            return Poll::Pending;
        };
        let current_products = product_count(inventory.value, &self.args.target);
        let current_xp = stat_xp(stats.value, self.args.target.skill_stat);
        let previous_gained = self.gained;
        let previous_xp_gain = self.xp_gain;
        self.gained = current_products
            .saturating_sub(self.baseline_products)
            .try_into()
            .unwrap_or(u32::MAX);
        self.xp_gain = current_xp.saturating_sub(self.baseline_xp);

        let Some(capacity) = snapshot.inventory_capacity() else {
            return Poll::Pending;
        };
        if inventory.value.len() as i32 >= i32::from(capacity.value) {
            return Poll::Ready(Ok(GatherResult {
                end: GatherEnd::Full,
                gained: self.gained,
                xp: self.xp_gain,
            }));
        }
        if let Some(reason) = refusal(snapshot, &mut self.chat_since) {
            return Poll::Ready(Ok(GatherResult {
                end: reason,
                gained: self.gained,
                xp: self.xp_gain,
            }));
        }

        let Some(target_state) = observe_target(snapshot, &self.args.catalog, &self.args.target)
        else {
            return Poll::Pending;
        };
        if target_state != TargetObservation::Active {
            return Poll::Ready(Ok(GatherResult {
                end: target_state.end(),
                gained: self.gained,
                xp: self.xp_gain,
            }));
        }
        if refused {
            return Poll::Ready(Ok(GatherResult {
                end: GatherEnd::Refused,
                gained: self.gained,
                xp: self.xp_gain,
            }));
        }

        let animated = snapshot
            .local_player()
            .is_some_and(|player| player.value.player.actor.animation != -1);
        // A prior yield stays in the receipt, but only a new delta keeps this attempt alive.
        let progressed = self.gained > previous_gained || self.xp_gain > previous_xp_gain;
        if progressed || animated {
            self.quiet_ticks = 0;
        } else {
            self.quiet_ticks = self.quiet_ticks.saturating_add(1);
        }
        if self.quiet_ticks < self.args.stall_ticks {
            return Poll::Pending;
        }
        if !self.retried {
            self.retried = true;
            self.quiet_ticks = 0;
            if cx.evidence().tick > self.last_emit_tick {
                self.emit_click(cx)?;
            }
            return Poll::Pending;
        }
        Poll::Ready(Ok(GatherResult {
            end: GatherEnd::Idle,
            gained: self.gained,
            xp: self.xp_gain,
        }))
    }

    fn cancel(&mut self) {}
}

impl GatherRun {
    fn emit_click(&mut self, cx: &mut ActionContext<'_>) -> Result<(), ActionError> {
        if cx.evidence().tick == self.last_emit_tick {
            return Ok(());
        }
        let request = match self.args.target.entity {
            EntityId::Loc(id) => InteractReq::Loc {
                x: self.args.target.tile.x,
                z: self.args.target.tile.z,
                level: self.args.target.tile.level,
                action: self.args.target.op.to_string(),
                id: Some(id),
            },
            EntityId::Npc(_) if self.args.target.npc_index >= 0 => InteractReq::Npc {
                name: self.args.target.alias.to_string(),
                action: self.args.target.op.to_string(),
                index: Some(self.args.target.npc_index),
            },
            EntityId::Npc(_) => {
                return Err(ActionError::Unavailable(
                    "gather NPC target has no observed instance index".into(),
                ));
            }
            EntityId::Obj(_) => InteractReq::Obj {
                x: self.args.target.tile.x,
                z: self.args.target.tile.z,
                level: self.args.target.tile.level,
                name: Some(self.args.target.alias.to_string()),
                action: self.args.target.op.to_string(),
            },
        };
        self.request_id = Some(cx.emit(request)?);
        self.last_emit_tick = cx.evidence().tick;
        Ok(())
    }
}

fn product_count(rows: &[api::snapshot::ItemView], target: &TargetPlan) -> i32 {
    rows.iter()
        .filter(|row| target.products[..target.products_len as usize].contains(&row.def.id))
        .map(|row| row.count.max(0))
        .sum()
}

fn stat_xp(rows: &[api::snapshot::StatView], index: i32) -> i32 {
    rows.iter()
        .find(|row| row.index == index)
        .map_or(0, |row| row.xp)
}

fn latest_chat(snapshot: api::snapshot::SnapshotView<'_>) -> Option<i32> {
    snapshot
        .chat_lines(0)
        .and_then(|lines| lines.value.iter().map(|line| line.sequence).max())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TargetObservation {
    Active,
    Depleted,
    Hazard,
    Gone,
}

impl TargetObservation {
    fn end(self) -> GatherEnd {
        match self {
            Self::Active => unreachable!("active targets do not end a gather run"),
            Self::Depleted => GatherEnd::Depleted,
            Self::Hazard => GatherEnd::Hazard,
            Self::Gone => GatherEnd::TargetGone,
        }
    }
}

fn observe_target(
    snapshot: api::snapshot::SnapshotView<'_>,
    catalog: &GatherCatalog,
    target: &TargetPlan,
) -> Option<TargetObservation> {
    let locs = snapshot.locs()?;
    let npcs = snapshot.npcs()?;
    let method = catalog.methods().get(usize::from(target.method_index))?;
    Some(classify_target(
        target,
        method,
        locs.value,
        npcs.value,
        catalog.hazard_npcs(),
    ))
}

fn classify_target(
    target: &TargetPlan,
    method: &GatherMethod,
    locs: &[api::snapshot::LocView],
    npcs: &[api::snapshot::NpcView],
    hazard_npcs: &[i32],
) -> TargetObservation {
    match target.entity {
        EntityId::Loc(id) | EntityId::Obj(id) => {
            if locs
                .iter()
                .any(|loc| loc.tile == target.tile && loc.id == id)
            {
                return TargetObservation::Active;
            }
            for loc in locs.iter().filter(|loc| loc.tile == target.tile) {
                if let Some(class) = target_class(method, EntityId::Loc(loc.id)) {
                    match class {
                        TargetClass::Depleted => return TargetObservation::Depleted,
                        TargetClass::Hazard => return TargetObservation::Hazard,
                        TargetClass::Resource | TargetClass::Unclassified => {}
                    }
                }
            }
            if has_hazard_npc(npcs, hazard_npcs, target.tile) {
                TargetObservation::Hazard
            } else {
                TargetObservation::Gone
            }
        }
        EntityId::Npc(type_id) => {
            if target.npc_index >= 0
                && npcs.iter().any(|npc| {
                    npc.index == target.npc_index as usize
                        && npc.r#type.and_then(|id| i32::try_from(id).ok()) == Some(type_id)
                        && npc.tile == target.tile
                })
            {
                return TargetObservation::Active;
            }
            for npc in npcs.iter().filter(|npc| npc.tile == target.tile) {
                let Some(id) = npc.r#type.and_then(|id| i32::try_from(id).ok()) else {
                    continue;
                };
                if let Some(class) = target_class(method, EntityId::Npc(id)) {
                    match class {
                        TargetClass::Depleted => return TargetObservation::Depleted,
                        TargetClass::Hazard => return TargetObservation::Hazard,
                        TargetClass::Resource | TargetClass::Unclassified => {}
                    }
                }
            }
            if has_hazard_npc(npcs, hazard_npcs, target.tile) {
                TargetObservation::Hazard
            } else {
                TargetObservation::Gone
            }
        }
    }
}

fn target_class(method: &GatherMethod, entity: EntityId) -> Option<TargetClass> {
    known_rows(&method.targets)
        .iter()
        .find(|target| target.entity == entity && matches!(target.respawn, Knowledge::Known(_)))
        .map(|target| target.class)
}

fn has_hazard_npc(
    npcs: &[api::snapshot::NpcView],
    hazard_npcs: &[i32],
    tile: api::snapshot::WorldTile,
) -> bool {
    npcs.iter().any(|npc| {
        npc.tile == tile
            && npc
                .r#type
                .and_then(|id| i32::try_from(id).ok())
                .is_some_and(|id| hazard_npcs.contains(&id))
    })
}

fn refusal(snapshot: api::snapshot::SnapshotView<'_>, chat_since: &mut i32) -> Option<GatherEnd> {
    if let Some(modal) = snapshot.main_modal() {
        if modal.value.root >= 0 {
            return Some(GatherEnd::Refused);
        }
    }
    if let Some(modal) = snapshot.chat_modal() {
        if modal.value.root >= 0 {
            return Some(GatherEnd::Refused);
        }
    }
    let lines = snapshot.chat_lines(*chat_since)?;
    let mut newest = *chat_since;
    for line in lines.value.iter() {
        if line.sequence <= *chat_since {
            continue;
        }
        newest = newest.max(line.sequence);
        if contains_any(
            &line.text,
            &[
                "too full",
                "inventory is full",
                "need a higher",
                "not members",
                "can't use",
                "cannot use",
                "need more",
                "no bait",
            ],
        ) {
            *chat_since = newest;
            return Some(GatherEnd::Refused);
        }
    }
    *chat_since = newest;
    None
}

fn contains_any(text: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| {
        text.as_bytes()
            .windows(needle.len())
            .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::gather_methods::{GatherSkill, GatherTarget};
    use api::snapshot::{LocLayer, LocView, NpcView, WorldTile};

    fn fixture_method() -> GatherMethod {
        let target = |entity, class| GatherTarget {
            entity,
            op: 1,
            class,
            respawn: Knowledge::Known(None),
        };
        GatherMethod {
            id: api::selected::FactKey::new("fixture.targets"),
            skill: GatherSkill::Fishing,
            resources: Arc::from([]),
            targets: Knowledge::Known(Arc::from([
                target(EntityId::Loc(10), TargetClass::Resource),
                target(EntityId::Loc(11), TargetClass::Depleted),
                target(EntityId::Loc(12), TargetClass::Hazard),
                target(EntityId::Npc(309), TargetClass::Resource),
            ])),
            products: Knowledge::Known(Arc::from([])),
            tools: Knowledge::Known(Arc::from([])),
            consumes: Knowledge::Known(Arc::from([])),
            requirements: Knowledge::Known(Arc::from([])),
            spots: Knowledge::Known(Arc::from([])),
        }
    }

    fn target(entity: EntityId, npc_index: i32, tile: WorldTile) -> TargetPlan {
        TargetPlan {
            entity,
            tile,
            op: Arc::from("Fish"),
            alias: Arc::from("spot"),
            products: [0; 8],
            products_len: 0,
            skill_stat: 10,
            method_index: 0,
            npc_index,
        }
    }

    fn loc(id: i32, tile: WorldTile) -> LocView {
        LocView {
            typecode: 0,
            info: 0,
            id,
            name: None,
            description: None,
            actions: vec![],
            tile,
            distance: 0,
            layer: LocLayer::Ground,
            shape: 10,
            angle: 0,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: true,
            block_range: true,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    fn npc(index: usize, type_id: usize, tile: WorldTile) -> NpcView {
        NpcView {
            index,
            r#type: Some(type_id),
            name: None,
            actions: vec![],
            tile,
            distance: 0,
            animation: -1,
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
            level: 0,
            size: 1,
            network: tile,
            x: 0,
            z: 0,
            yaw: 0,
        }
    }

    #[test]
    fn observed_loc_replacements_and_ent_ids_reach_real_gather_endings() {
        let method = fixture_method();
        let tile = WorldTile {
            x: 3201,
            z: 3202,
            level: 0,
        };
        let target = target(EntityId::Loc(10), -1, tile);
        let hazards = [900];

        assert_eq!(
            classify_target(&target, &method, &[loc(10, tile)], &[], &hazards),
            TargetObservation::Active
        );
        assert_eq!(
            classify_target(&target, &method, &[loc(11, tile)], &[], &hazards).end(),
            GatherEnd::Depleted
        );
        assert_eq!(
            classify_target(&target, &method, &[loc(12, tile)], &[], &hazards).end(),
            GatherEnd::Hazard
        );
        assert_eq!(
            classify_target(&target, &method, &[], &[npc(5, 900, tile)], &hazards).end(),
            GatherEnd::Hazard,
            "a content-derived ent at the old tree tile is a hazard, not a missing target"
        );
    }

    #[test]
    fn fishing_identity_requires_the_same_type_index_and_tile() {
        let method = fixture_method();
        let tile = WorldTile {
            x: 3210,
            z: 3211,
            level: 0,
        };
        let target = target(EntityId::Npc(309), 42, tile);
        assert_eq!(
            classify_target(&target, &method, &[], &[npc(42, 309, tile)], &[901]),
            TargetObservation::Active
        );
        assert_eq!(
            classify_target(
                &target,
                &method,
                &[],
                &[npc(42, 309, WorldTile { x: 3211, ..tile })],
                &[901],
            ),
            TargetObservation::Gone,
            "the same NPC slot after movement ends this attempt for re-acquisition"
        );
        assert_eq!(
            classify_target(&target, &method, &[], &[npc(43, 309, tile)], &[901]),
            TargetObservation::Gone,
            "a different instance of the same type is not the selected target"
        );
        assert_eq!(
            classify_target(&target, &method, &[], &[npc(42, 777, tile)], &[901]),
            TargetObservation::Gone,
            "re-use of the old index for another NPC is not the original target"
        );
        assert_eq!(
            classify_target(&target, &method, &[], &[npc(43, 901, tile)], &[901]).end(),
            GatherEnd::Hazard,
            "a whirlpool at the selected spot is not treated as a fishing target"
        );
    }
}
