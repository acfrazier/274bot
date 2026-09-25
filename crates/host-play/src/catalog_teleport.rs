use super::*;
/// Magic XP and destination tile, then native bank restock and a further
/// teleport. Walking or a queued if-button without XP/law spend fails.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AioTeleportCycle {
    pub teleported: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub closed: bool,
    pub further: bool,
}

pub struct AioTeleportSpec {
    pub landing: (i32, i32, i32),
    pub restock: (i32, i32, i32),
    pub staff: Option<i32>,
    pub air_from_pack: bool,
}

pub fn aio_teleport_spec(case: CoreCase) -> Option<AioTeleportSpec> {
    match case {
        CoreCase::AioTeleport => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: Some(STAFF_OF_AIR_ID),
            air_from_pack: false,
        }),
        CoreCase::AioTeleportFalador => Some(AioTeleportSpec {
            landing: FALADOR_TELE_LAND,
            restock: FALADOR_WEST_BANK,
            staff: Some(STAFF_OF_WATER_ID),
            air_from_pack: true,
        }),
        CoreCase::AioTeleportNoStaff => Some(AioTeleportSpec {
            landing: VARROCK_TELE_LAND,
            restock: VARROCK_EAST_BANK,
            staff: None,
            air_from_pack: true,
        }),
        _ => None,
    }
}

impl AioTeleportCycle {
    pub fn observe(&mut self, spec: AioTeleportSpec, baseline: &Observation, now: &Observation) {
        let AioTeleportSpec {
            landing,
            restock,
            staff,
            air_from_pack,
        } = spec;
        if self.teleported.is_none()
            && now.skill_xp("magic") > baseline.skill_xp("magic")
            && now.item_id(LAW_RUNE_ID) < baseline.item_id(LAW_RUNE_ID)
            && near(now.tile, landing, 8)
            && staff.is_none_or(|id| now.equipment_id(id) >= 1)
            && (!air_from_pack || now.item_id(AIR_RUNE_ID) < baseline.item_id(AIR_RUNE_ID))
        {
            self.teleported = Some(now.clone());
        }
        if self.teleported.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && near(now.tile, restock, 8)
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(LAW_RUNE_ID) > deposited.item_id(LAW_RUNE_ID)
                && now.bank_item_id(LAW_RUNE_ID) < deposited.bank_item_id(LAW_RUNE_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            self.closed |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation;
        }
        if let Some(teleported) = &self.teleported {
            if let Some(restocked) = &self.restocked {
                self.further |= self.closed
                    && !now.bank_open
                    && now.skill_xp("magic") > teleported.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < restocked.item_id(LAW_RUNE_ID);
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.teleported.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
    }
}