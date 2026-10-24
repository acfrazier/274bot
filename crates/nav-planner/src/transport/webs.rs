use super::TransportEdge;

/// `oplocu` option used to use a held knife on slashable webs.
pub const WEB_USE_OPTION: i32 = 0;

/// Select the action a live web should use from its matching packed action
/// edges. A qualifying worn Slash takes precedence; the held knife is the
/// fallback. `qualifies` is shared with the planner's strict edge gate and
/// the follower's current snapshot gate.
pub(crate) fn select_web_action<'a>(
    candidates: impl Iterator<Item = &'a TransportEdge>,
    edge: &TransportEdge,
    mut qualifies: impl FnMut(&TransportEdge) -> bool,
) -> Option<&'a TransportEdge> {
    if !edge.is_slashable_web() {
        return None;
    }

    let mut knife = None;
    let mut slash = None;
    for candidate in candidates {
        if !same_web_crossing(edge, candidate) {
            continue;
        }
        match candidate.option {
            WEB_USE_OPTION if !candidate.item_req.is_empty() => knife = Some(candidate),
            1 if candidate.item_req.is_empty() && !candidate.worn_req.is_empty() => {
                slash = Some(candidate);
            }
            _ => {}
        }
    }

    slash
        .filter(|candidate| qualifies(candidate))
        .or_else(|| knife.filter(|candidate| qualifies(candidate)))
}

fn same_web_crossing(a: &TransportEdge, b: &TransportEdge) -> bool {
    a.is_slashable_web()
        && b.is_slashable_web()
        && a.at == b.at
        && a.to == b.to
        && a.dir == b.dir
        && a.ticks == b.ticks
        && a.open_loc_id == b.open_loc_id
}
