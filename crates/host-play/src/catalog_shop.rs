use super::*;
/// Posted shop stock down, matching inv up, coins down, then deposit except
/// coins, retain or top up funding, and a second buy. Queued if-button without
/// stock movement fails.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ShopFunding {
    Retained {
        carried_coins: i32,
    },
    TopUp {
        carried_before: i32,
        carried_after: i32,
        bank_before: i32,
        bank_after: i32,
    },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ShopBuyoutCycle {
    pub opened: Option<Observation>,
    pub bought: Option<Observation>,
    pub bought_id: Option<i32>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    /// Explicitly distinguishes already-retained coins from a bank withdrawal.
    pub funding: Option<ShopFunding>,
    pub returned: bool,
    pub reopened: Option<Observation>,
    pub further: bool,
    last_tick: Option<u32>,
    /// Peak carried product after the first purchase, until deposit. Deposit
    /// must bank this earned load, not the first 10/5/1 inventory delta.
    pub earned_quantity: i32,
    /// Product count on the first loaded bank session of this trip while the
    /// pack still held that product. None if that first loaded bank already
    /// had an empty pack: fail closed rather than invent a pre-deposit count.
    pub trip_bank_product: Option<i32>,
}

pub struct ShopBuyoutSpec {
    pub stand: (i32, i32, i32),
    pub stand_radius: i32,
    pub restock: (i32, i32, i32),
}

pub fn shop_buyout_spec(case: CoreCase) -> Option<ShopBuyoutSpec> {
    match case {
        CoreCase::ShopBuyout => Some(ShopBuyoutSpec {
            stand: AEMAD_STAND,
            stand_radius: 6,
            restock: ARDOUGNE_EAST_BANK,
        }),
        CoreCase::ShopBuyoutAubury => Some(ShopBuyoutSpec {
            stand: AUBURY_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutLowe => Some(ShopBuyoutSpec {
            stand: LOWE_STAND,
            stand_radius: 6,
            restock: VARROCK_EAST_BANK,
        }),
        CoreCase::ShopBuyoutHickton => Some(ShopBuyoutSpec {
            stand: HICKTON_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutHarry => Some(ShopBuyoutSpec {
            stand: HARRY_STAND,
            stand_radius: 6,
            restock: CATHERBY_BANK,
        }),
        CoreCase::ShopBuyoutBetty => Some(ShopBuyoutSpec {
            stand: BETTY_STAND,
            stand_radius: 6,
            restock: FALADOR_WEST_BANK,
        }),
        CoreCase::ShopBuyoutGerrant => Some(ShopBuyoutSpec {
            stand: GERRANT_STAND,
            stand_radius: 6,
            restock: DRAYNOR_BANK,
        }),
        _ => None,
    }
}

fn shop_bought_id(
    before: &Observation,
    now: &Observation,
    stand: (i32, i32, i32),
    stand_radius: i32,
) -> Option<(i32, i32)> {
    if !now.shop_open
        || now.main_modal != SHOPMAIN
        || now.shop_stock.is_empty()
        || !near(now.tile, stand, stand_radius)
    {
        return None;
    }
    if now.item_id(COINS_ID) >= before.item_id(COINS_ID) {
        return None;
    }
    let mut found = None;
    for row in &before.shop_stock {
        if now.shop_item_id(row.id) < row.count && now.item_id(row.id) > before.item_id(row.id) {
            if row.id == EMPTY_VIAL_ID {
                return Some((
                    EMPTY_VIAL_ID,
                    now.item_id(EMPTY_VIAL_ID) - before.item_id(EMPTY_VIAL_ID),
                ));
            }
            if found.is_none() {
                found = Some((row.id, now.item_id(row.id) - before.item_id(row.id)));
            }
        }
    }
    found
}

/// First-purchase replay: equal shop stock, carried product, and coins.
fn shop_same_purchase(left: &Observation, right: &Observation, id: i32) -> bool {
    left.item_id(id) == right.item_id(id)
        && left.item_id(COINS_ID) == right.item_id(COINS_ID)
        && left.shop_stock == right.shop_stock
}

impl ShopBuyoutCycle {
    pub fn observe(&mut self, spec: ShopBuyoutSpec, baseline: &Observation, now: &Observation) {
        let ShopBuyoutSpec {
            stand,
            stand_radius,
            restock,
        } = spec;
        // Inv/shop/bank packets can publish later frames in the same game tick.
        // Reject only backwards ticks; identical no-delta replays must still
        // fail the stage predicates below rather than this cadence gate.
        if now.tick < self.last_tick.unwrap_or(baseline.tick) {
            return;
        }
        self.last_tick = Some(now.tick);
        if self.opened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
            && !now.shop_stock.is_empty()
        {
            self.opened = Some(now.clone());
        }
        if let Some(opened) = &self.opened {
            if self.bought.is_none() {
                if let Some((id, _)) = shop_bought_id(opened, now, stand, stand_radius) {
                    self.bought_id = Some(id);
                    self.earned_quantity = now.item_id(id);
                    self.bought = Some(now.clone());
                }
            }
        }
        if let Some(id) = self.bought_id {
            if self.deposited.is_none() {
                self.earned_quantity = self.earned_quantity.max(now.item_id(id));
            }
            // First loaded bank of this trip: latch the product count only
            // while the pack still holds the earned product.
            if self.trip_bank_product.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) > 0
            {
                self.trip_bank_product = Some(now.bank_item_id(id));
            }
            if self.deposited.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation > baseline.bank_generation
                && near(now.tile, restock, 8)
                && now.item_id(id) == 0
                && self.earned_quantity >= 1
                && self
                    .trip_bank_product
                    .is_some_and(|before| now.bank_item_id(id) >= before + self.earned_quantity)
                && now.item_id(COINS_ID) >= 1
            {
                self.deposited = Some(now.clone());
            }
        }
        // Retained funding is recordable on the deposit observation when
        // carried and bank coins did not move, so a later open-bank tick is
        // not required. A real withdrawal in the same session overwrites as
        // TopUp so retained coins are not mislabeled as withdrawn.
        if let Some(deposited) = &self.deposited {
            if now.bank_open && now.bank_loaded && now.bank_generation == deposited.bank_generation
            {
                let carried_now = now.item_id(COINS_ID);
                let carried_then = deposited.item_id(COINS_ID);
                let bank_now = now.bank_item_id(COINS_ID);
                let bank_then = deposited.bank_item_id(COINS_ID);
                if carried_now > carried_then && bank_now < bank_then {
                    self.funding = Some(ShopFunding::TopUp {
                        carried_before: carried_then,
                        carried_after: carried_now,
                        bank_before: bank_then,
                        bank_after: bank_now,
                    });
                    self.restocked = Some(now.clone());
                } else if self.funding.is_none()
                    && carried_now == carried_then
                    && bank_now == bank_then
                    && carried_now >= 1
                {
                    self.funding = Some(ShopFunding::Retained {
                        carried_coins: carried_now,
                    });
                    self.restocked = Some(now.clone());
                }
            }
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= self.restocked.is_some()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, stand, stand_radius);
        }
        if self.returned
            && self.reopened.is_none()
            && now.shop_open
            && now.main_modal == SHOPMAIN
            && near(now.tile, stand, stand_radius)
        {
            self.reopened = Some(now.clone());
        }
        if let Some(reopened) = &self.reopened {
            let stale = match (self.bought.as_ref(), self.bought_id) {
                (Some(bought), Some(id)) => shop_same_purchase(bought, now, id),
                _ => false,
            };
            self.further |= self.funding.is_some()
                && now.tick > reopened.tick
                && !stale
                && shop_bought_id(reopened, now, stand, stand_radius).is_some();
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
            && self.opened.is_some()
            && self.bought.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.funding.is_some()
    }
}
