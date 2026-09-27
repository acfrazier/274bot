use super::*;

/// Selected shut loc must lose Open through a same-session world change.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DoorOpenerCycle {
    pub selected: Option<BoundedLoc>,
    pub opened: bool,
}

impl DoorOpenerCycle {
    pub fn observe(
        &mut self,
        gate: bool,
        packed: (i32, i32, i32),
        closed_id: i32,
        open_id: i32,
        baseline: &Observation,
        now: &Observation,
    ) {
        if self.selected.is_none() {
            self.selected = baseline
                .loc_facts
                .iter()
                .find(|loc| {
                    loc.id == closed_id
                        && loc.open
                        && loc.level == packed.2
                        && (loc.x - packed.0).abs().max((loc.z - packed.1).abs()) <= 1
                        && loc_name_matches(loc.name.as_deref(), gate)
                })
                .cloned();
        }
        let Some(selected) = &self.selected else {
            return;
        };
        let still_shut = loc_at(
            now,
            selected.id,
            (selected.x, selected.z, selected.level),
            1,
        )
        .is_some_and(|loc| loc.open);
        let opened_leaf =
            loc_at(now, open_id, (selected.x, selected.z, selected.level), 3).is_some();
        self.opened |= !still_shut && opened_leaf;
    }

    pub fn qualified(&self) -> bool {
        self.selected.as_ref().is_some_and(|loc| loc.open) && self.opened
    }
}
