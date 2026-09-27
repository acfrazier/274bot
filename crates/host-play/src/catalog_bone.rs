use super::*;

/// Ordered observations: seed depletion alone must never qualify this card.
#[derive(Debug, Clone, Default, Serialize)]
pub struct BoneBankCycle {
    pub first_batch_buried: bool,
    pub opened: Option<Observation>,
    pub withdrawn: Option<Observation>,
    pub buried_after_withdrawal: bool,
}

impl BoneBankCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.first_batch_buried |=
            now.item("Bones") == 0 && now.skill_xp("prayer") - baseline.skill_xp("prayer") >= 22;
        if self.first_batch_buried
            && self.opened.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item("Bones") == 0
            && now.bank_item("Bones") == 28
            && now.tile != baseline.tile
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.withdrawn.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == opened.bank_generation
                && now.item("Bones") == 28
                && now.bank_item("Bones") == 0
            {
                self.withdrawn = Some(now.clone());
            }
        }
        if let Some(withdrawn) = &self.withdrawn {
            self.buried_after_withdrawal |= !now.bank_open
                && !now.bank_loaded
                && now.item("Bones") < withdrawn.item("Bones")
                && now.skill_xp("prayer") > withdrawn.skill_xp("prayer");
        }
    }
}
