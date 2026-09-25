use super::*;
/// ClimbingBoots (frozen `ClimbingBoots.ts`) at Tenzing's hut. The buy is
/// only a purchase witness once the framed Tenzing projection and the sherpa
/// dialogue are in the observed window and carried boots rose while coins
/// fell by exactly 12 a pair. Walk and teleport are separate cells: the
/// teleport cell must show a real post-purchase return cast (magic XP, Law
/// spend, landing, still carrying the earned pack) and the walk cell must
/// not cast at all. Return, deposit, reopen and the further purchase stay
/// separate stages, so a failed full cycle keeps its purchase evidence
/// instead of silently downgrading to a smoke PASS.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClimbingBootsCycle {
    pub tenzing: Option<Observation>,
    pub cast: Option<Observation>,
    pub bought: Option<Observation>,
    /// Pairs gained at the first `bought` sample (each pair is exactly 12 coins).
    pub bought_pairs: i32,
    /// Peak carried boots after that purchase, until deposit. Deposit must
    /// bank this earned quantity, not the early 1-pair sample.
    pub earned_boots: i32,
    /// First loaded post-Start bank: `Some(true)` only when boots 3105 were 0.
    pub boot_bank_empty: Option<bool>,
    /// The sherpa sell/buy line seen in a bounded chat projection.
    pub dialogue: bool,
    pub returned: Option<Observation>,
    pub deposited: Option<Observation>,
    pub restocked: Option<Observation>,
    pub departed: Option<Observation>,
    pub further: bool,
}

pub struct ClimbingBootsSpec {
    /// Explicit typed fixture option, not the frozen default.
    pub use_teleport: bool,
    /// Exact `readyToBuy` trip money: 12 coins a pair for the whole pack.
    pub coins: i32,
    /// Falador landing the teleport branch must reach after a real cast.
    pub landing: (i32, i32, i32),
    /// Falador West bank stand the walk returns through.
    pub bank: (i32, i32, i32),
}

pub fn climbing_boots_spec(case: CoreCase) -> Option<ClimbingBootsSpec> {
    match case {
        // Walking cell: 28 boots * 12 coins, no runes, no Magic requirement.
        CoreCase::ClimbingBoots => Some(ClimbingBootsSpec {
            use_teleport: false,
            coins: CLIMBING_BOOTS_WALK_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        // Teleport cell: the `tripQty` is 28 minus the carried rune stacks
        // (Law, Air, Water), so the ready-to-buy money is 25 pairs.
        CoreCase::ClimbingBootsTeleport => Some(ClimbingBootsSpec {
            use_teleport: true,
            coins: CLIMBING_BOOTS_TELE_PACK_COINS,
            landing: FALADOR_TELE_LAND,
            bank: FALADOR_WEST_BANK,
        }),
        _ => None,
    }
}

/// The real framed Tenzing NPC near the hut inside tile. Route coordinates
/// and quest state never stand in for this projection.
fn tenzing_visible(now: &Observation) -> bool {
    now.npc_facts.iter().any(|npc| {
        npc.name.as_deref() == Some(TENZING_NAME) && near(Some(npc.tile), TENZING_INSIDE, 12)
    })
}

/// The sherpa sale line or the purchase confirmation, from the bounded chat
/// projection (`death_sherpa.rs2`: "for 12 gold" then "Tenzing has given you
/// some Climbing boots.").
fn climbing_boots_dialogue(now: &Observation) -> bool {
    now.chat
        .iter()
        .any(|(_, text)| text.contains("Climbing boots"))
}

impl ClimbingBootsCycle {
    pub fn observe(&mut self, spec: ClimbingBootsSpec, baseline: &Observation, now: &Observation) {
        let ClimbingBootsSpec {
            use_teleport,
            coins: _,
            landing,
            bank,
        } = spec;
        self.dialogue |= climbing_boots_dialogue(now);
        if self.tenzing.is_none() && tenzing_visible(now) {
            self.tenzing = Some(now.clone());
        }
        // The purchase: same player after Start, boots up, exactly 12 coins a
        // pair gone, with the framed Tenzing/dialogue evidence. The first
        // matching frame is typically one pair; earned_boots tracks the peak
        // pack after that until deposit.
        if self.tenzing.is_some() && self.dialogue {
            if self.bought.is_none() {
                let gained = now.item_id(CLIMBING_BOOTS_ID) - baseline.item_id(CLIMBING_BOOTS_ID);
                let spent = baseline.item_id(COINS_ID) - now.item_id(COINS_ID);
                if gained >= 1 && spent == gained * CLIMBING_BOOTS_PAIR_COINS {
                    self.bought_pairs = gained;
                    self.earned_boots = gained;
                    self.bought = Some(now.clone());
                }
            } else if self.deposited.is_none() {
                self.earned_boots = self.earned_boots.max(now.item_id(CLIMBING_BOOTS_ID));
            }
        }
        // A real cast, not the option: magic XP plus the whole Falador cost
        // (Law, Air and Water, no staff assumed) and the landing. Only after
        // the purchase, on a frame that still carries the earned pack, so a
        // pre-purchase Falador-shaped cast cannot satisfy the teleport cell.
        // Compare with the purchase snapshot: Start-relative deltas could
        // count an earlier cast when the earned pack later walks through here.
        // Recorded for either cell, so a walking cell that cast fails closed.
        if self.cast.is_none()
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && self.bought.as_ref().is_some_and(|bought| {
                now.skill_xp("magic") > bought.skill_xp("magic")
                    && now.item_id(LAW_RUNE_ID) < bought.item_id(LAW_RUNE_ID)
                    && now.item_id(AIR_RUNE_ID) < bought.item_id(AIR_RUNE_ID)
                    && now.item_id(WATER_RUNE_ID) < bought.item_id(WATER_RUNE_ID)
            })
            && near(now.tile, landing, 8)
        {
            self.cast = Some(now.clone());
        }
        // Return is genuine position after the purchase (the first arrival can
        // still be inside the baseline bank generation), never a generation
        // counter that would demand a second bank trip. Teleport return must
        // follow the post-purchase cast, still carrying the earned pack.
        if self.bought.is_some()
            && self.returned.is_none()
            && !now.bank_open
            && near(now.tile, bank, 8)
            && self.earned_boots >= 1
            && now.item_id(CLIMBING_BOOTS_ID) >= self.earned_boots
            && (!use_teleport || self.cast.is_some())
        {
            self.returned = Some(now.clone());
        }
        if now.bank_open && now.bank_loaded && self.boot_bank_empty.is_none() {
            self.boot_bank_empty = Some(now.bank_item_id(CLIMBING_BOOTS_ID) == 0);
        }
        // Deposit the earned peak against a verified empty boot bank, not the
        // first 1-pair bought sample and not leftover bank stock.
        if self.returned.is_some()
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && self.boot_bank_empty == Some(true)
            && now.bank_generation > baseline.bank_generation
            && self.earned_boots >= 1
        {
            let carried_lost = self.earned_boots - now.item_id(CLIMBING_BOOTS_ID);
            let bank_gained = now.bank_item_id(CLIMBING_BOOTS_ID);
            if carried_lost >= self.earned_boots && bank_gained >= self.earned_boots {
                self.deposited = Some(now.clone());
            }
        }
        if let Some(deposited) = &self.deposited {
            if self.restocked.is_none()
                && now.bank_open
                && now.bank_loaded
                && now.bank_generation == deposited.bank_generation
                && now.item_id(COINS_ID) > deposited.item_id(COINS_ID)
                && now.bank_item_id(COINS_ID) < deposited.bank_item_id(COINS_ID)
            {
                self.restocked = Some(now.clone());
            }
        }
        // Departure: a later bank session closed with the restocked pack.
        if let Some(restocked) = &self.restocked {
            if self.departed.is_none()
                && !now.bank_open
                && !now.bank_loaded
                && now.bank_generation > restocked.bank_generation
                && now.item_id(COINS_ID) >= CLIMBING_BOOTS_PAIR_COINS
            {
                self.departed = Some(now.clone());
            }
        }
        // A further pair bought back at Tenzing's hut after the departure.
        if let Some(departed) = &self.departed {
            let gained = now.item_id(CLIMBING_BOOTS_ID) - departed.item_id(CLIMBING_BOOTS_ID);
            let spent = departed.item_id(COINS_ID) - now.item_id(COINS_ID);
            self.further |= gained >= 1
                && spent == gained * CLIMBING_BOOTS_PAIR_COINS
                && near(now.tile, TENZING_HUT_DOOR, 12);
        }
    }

    /// The honest intermediate state: a real purchase with its Tenzing and
    /// dialogue evidence but no claim of return, deposit or further work.
    pub fn purchase_smoke(&self) -> bool {
        self.tenzing.is_some() && self.dialogue && self.bought.is_some()
    }

    pub fn qualified(&self, spec: ClimbingBootsSpec) -> bool {
        let branch = if spec.use_teleport {
            self.cast.is_some()
        } else {
            // A walking cell must not silently run the teleport branch.
            self.cast.is_none()
        };
        branch
            && self.purchase_smoke()
            && self.returned.is_some()
            && self.deposited.is_some()
            && self.restocked.is_some()
            && self.departed.is_some()
            && self.further
    }
}