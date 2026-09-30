use super::select::TargetPlan;
use crate::native::{ActionContext, ActionError, NativeMachine};
use crate::shim::InteractReq;
use api::selected::EntityId;
#[cfg(test)]
use std::sync::Arc;
use std::task::Poll;

pub const DEFAULT_STALL_TICKS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatherEnd {
    Full,
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

#[derive(Debug, Clone)]
pub struct GatherRunArgs {
    pub target: TargetPlan,
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

        // Only the originally selected resource is actionable. Once it changes,
        // selection reclassifies the observed replacement (depleted or hazard).
        if scene_target(snapshot, &self.args.target).is_none() {
            return Poll::Ready(Ok(GatherResult {
                end: GatherEnd::TargetGone,
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
        let progressed = current_products > self.baseline_products || current_xp > self.baseline_xp;
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
            self.baseline_products = current_products;
            self.baseline_xp = current_xp;
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
            EntityId::Npc(id) => InteractReq::Npc {
                name: self.args.target.alias.to_string(),
                action: self.args.target.op.to_string(),
                index: Some(id),
            },
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

fn scene_target<'a>(
    snapshot: api::snapshot::SnapshotView<'a>,
    target: &TargetPlan,
) -> Option<&'a api::snapshot::LocView> {
    let locs = snapshot.locs()?;
    locs.value
        .iter()
        .find(|loc| loc.tile == target.tile && loc.id == entity_id(target.entity))
}

fn entity_id(entity: EntityId) -> i32 {
    match entity {
        EntityId::Loc(id) | EntityId::Npc(id) | EntityId::Obj(id) => id,
    }
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

    #[test]
    fn product_delta_never_goes_negative() {
        let target = TargetPlan {
            entity: EntityId::Loc(1),
            tile: api::snapshot::WorldTile {
                x: 1,
                z: 1,
                level: 0,
            },
            op: Arc::from("Chop down"),
            alias: Arc::from("Tree"),
            products: [1511, 0, 0, 0, 0, 0, 0, 0],
            products_len: 1,
            skill_stat: 8,
        };
        assert_eq!(product_count(&[], &target), 0);
    }
}
