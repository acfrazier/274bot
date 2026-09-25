use api::interact::Driver;
use api::snapshot::{GameSnapshot, WorldTile};
use nav::world::NavWorld;

pub(super) fn dispatch_observed_bank_op(
    driver: &mut dyn Driver,
    snapshot: &GameSnapshot,
    obj_names: Option<&api::obj_names::ObjNames>,
    inventory: Option<&[(i32, i32)]>,
    req: &script::shim::InteractReq,
) -> Option<script::slot::PendingBankOp> {
    use api::interact::{ActionSpec, Interactions, OpTarget, SendResult};
    use script::shim::InteractReq;
    use script::slot::{PendingBankOp, PendingBankOpKind};

    if snapshot.bank_component_id() < 0 || !snapshot.bank_loaded() {
        return None;
    }
    let generation = snapshot.bank_session_generation();
    let inventory_count = |id| {
        inventory.map_or(0, |items| {
            items
                .iter()
                .filter(|(item_id, _)| *item_id == id)
                .map(|(_, count)| *count)
                .sum()
        })
    };
    match req {
        InteractReq::Deposit { name } => {
            let item = snapshot.bank_side().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = all_slot(&item.actions)?;
            let before_count = snapshot
                .bank_side()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    PendingBankOpKind::Deposit,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        InteractReq::Withdraw { name, action } => {
            let item = snapshot.bank().iter().find(|item| {
                obj_names
                    .and_then(|names| names.name(item.def.id))
                    .is_some_and(|actual| actual.eq_ignore_ascii_case(name))
            })?;
            let op = action_slot(&item.actions, action)?;
            let kind = if open_only_withdraw_x(&item.actions, op) {
                PendingBankOpKind::WithdrawXAction
            } else {
                PendingBankOpKind::Withdraw
            };
            let before_count = snapshot
                .bank()
                .iter()
                .filter(|row| row.def.id == item.def.id)
                .map(|row| row.count)
                .sum();
            matches!(
                Interactions::new(snapshot, driver)
                    .interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            )
            .then(|| {
                PendingBankOp::new(
                    kind,
                    item.def.id,
                    before_count,
                    inventory_count(item.def.id),
                    generation,
                )
            })
        }
        _ => None,
    }
}

pub(crate) fn nearest_bank_booth(
    world: &NavWorld,
    (x, z, level): (i32, i32, i32),
) -> Option<WorldTile> {
    world
        .banks()
        .iter()
        .filter(|stand| matches!(stand.access, nav::pack::BankAccess::Booth { .. }))
        .min_by_key(|stand| {
            let distance = stand.tile.x.abs_diff(x).max(stand.tile.z.abs_diff(z));
            if stand.tile.level == level {
                u64::from(distance)
            } else {
                (u64::MAX / 2).saturating_add(u64::from(distance))
            }
        })
        .map(|stand| stand.tile)
}

/// Action-label lookup matching rs2b0t's `norm` (lowercase, whitespace and
/// `-`/`_` separators gone): `Withdraw All`, `Withdraw-All` and
/// `Withdraw  All` all resolve to the same slot.
pub(super) fn action_slot(actions: &[Option<String>], wanted: &str) -> Option<i32> {
    let wanted = norm_action(wanted);
    actions
        .iter()
        .position(|a| a.as_deref().map(norm_action).as_deref() == Some(wanted.as_str()))
        .map(|i| i as i32 + 1)
}

/// Whether the resolved bank op is the open-only `Withdraw X` label: it
/// opens the amount dialog and cannot move inventory on its own, so the
/// accepted send *is* the result (frozen `Bank.withdraw` returns
/// `Input.invButton` → `actions.menuAction`). Every other withdraw label
/// settles on the observed delta.
fn open_only_withdraw_x(actions: &[Option<String>], op: i32) -> bool {
    let index = match usize::try_from(op) {
        Ok(index) if index >= 1 => index - 1,
        _ => return false,
    };
    actions
        .get(index)
        .and_then(|label| label.as_deref())
        .is_some_and(|label| norm_action(label) == norm_action("Withdraw X"))
}

/// The bank-side op slot whose label contains "all" (Deposit All / the
/// deposit window's bulk op), 1-based.
pub(super) fn all_slot(actions: &[Option<String>]) -> Option<i32> {
    actions
        .iter()
        .position(|a| a.as_deref().is_some_and(|s| norm_action(s).contains("all")))
        .map(|i| i as i32 + 1)
}

/// Select one bounded fill operation in compatibility order: an exact
/// amount, then All only when the requested fill consumes current stock,
/// then the host-owned X continuation.
pub(crate) fn fill_withdraw_action(
    actions: &[Option<String>],
    count: i32,
    stock: i32,
) -> Option<(i32, bool)> {
    action_slot(actions, &format!("Withdraw {count}"))
        .map(|op| (op, false))
        .or_else(|| {
            (count == stock)
                .then(|| all_slot(actions).map(|op| (op, false)))
                .flatten()
        })
        .or_else(|| action_slot(actions, "Withdraw X").map(|op| (op, true)))
}

fn norm_action(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
        .collect()
}

pub(super) fn open_bank_at_here<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    here: Option<(i32, i32, i32)>,
    world: Option<&NavWorld>,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    // Prefer a packed booth stand's tile; else any Use-quickly loc.
    let target_tile = world.and_then(|w| {
        here.and_then(|(hx, hz, hl)| {
            w.banks()
                .iter()
                .min_by_key(|s| {
                    (
                        s.tile.level != hl,
                        (s.tile.x - hx).abs().max((s.tile.z - hz).abs()),
                    )
                })
                .map(|s| (s.tile.x, s.tile.z, s.tile.level))
        })
    });
    if let Some((x, z, level)) = target_tile {
        if let Some(loc) = snapshot
            .locs()
            .iter()
            .find(|l| l.tile.x == x && l.tile.z == z && l.tile.level == level)
        {
            if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
                return matches!(
                    ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                    SendResult::Sent { .. }
                );
            }
        }
    }
    for loc in snapshot.locs() {
        if let Some(op) = action_slot(&loc.actions, "Use-quickly") {
            return matches!(
                ix.interact(OpTarget::Loc(loc), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    false
}

pub(super) fn deposit_all_backpack<D: Driver>(driver: &mut D, snapshot: &GameSnapshot) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let mut wrote = false;
    for item in snapshot.bank_side() {
        if let Some(op) = all_slot(&item.actions) {
            wrote |= matches!(
                ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
                SendResult::Sent { .. }
            );
        }
    }
    wrote
}

pub(super) fn withdraw_id<D: Driver>(
    driver: &mut D,
    snapshot: &GameSnapshot,
    id: i32,
    count: i32,
) -> bool {
    use api::interact::{ActionSpec, OpTarget, SendResult};
    let mut ix = api::interact::Interactions::new(snapshot, driver);
    let Some(item) = snapshot.bank().iter().find(|it| it.def.id == id) else {
        return false;
    };
    let label = match count {
        1 => "Withdraw 1",
        5 => "Withdraw 5",
        10 => "Withdraw 10",
        _ => "Withdraw All",
    };
    if let Some(op) = action_slot(&item.actions, label)
        .or_else(|| action_slot(&item.actions, "Withdraw 1"))
        .or_else(|| action_slot(&item.actions, "Withdraw All"))
    {
        return matches!(
            ix.interact(OpTarget::Item(item), ActionSpec::Operation(op)),
            SendResult::Sent { .. }
        );
    }
    false
}
