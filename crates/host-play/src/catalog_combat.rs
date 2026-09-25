use super::*;
/// Bank-only MossGiant dart Start. Worn-gear combat_baseline_ready(Ranged) is
/// the wrong predicate here: the script has to withdraw and equip 806.
pub fn moss_giant_dart_baseline_ready(baseline: &Observation) -> bool {
    let bank_ready = baseline.bank_open
        && baseline.bank_loaded
        && baseline.bank_item_id(BRONZE_DART_ID) == MOSS_GIANT_DART_SUPPLY
        && baseline.bank_item_id(LOBSTER_ID) == MOSS_GIANT_DART_BANK_FOOD;
    near(baseline.tile, MOSS_GIANT_BANK, 2)
        && empty_pack(baseline)
        && empty_worn(baseline)
        && baseline.level("ranged") >= MOSS_GIANT_DART_RANGED
        && baseline.level("defence") >= COMBAT_ATTACK_LEVEL
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && held_id(baseline, BRONZE_DART_ID) == 0
        && held_id(baseline, RUNE_ARROW_ID) == 0
        && baseline.item_id(LOBSTER_ID) == 0
        && baseline.bank_item_id(RUNE_ARROW_ID) == 0
        && bank_ready
}
#[derive(Debug, Clone, Copy)]
pub struct CombatSpec {
    pub target: &'static str,
    pub stand: (i32, i32, i32),
    pub radius: i32,
    pub food_id: i32,
    pub food_count: i32,
    pub weapon_id: i32,
    pub style: CombatStyleWitness,
    pub loot: CombatLoot,
    pub extra: CombatExtra,
    /// Worn projectile the ranged branch consumes (arrows in the ammo slot).
    pub projectile: Option<i32>,
    /// The extra consumable transition this branch must actually execute.
    pub consumable: CombatConsumable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatStyleWitness {
    Strength,
    FireStrike,
    Ranged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatConsumable {
    None,
    /// `useSpecial=true`: the spec bar arms and `%sa_energy` pays the cost.
    Special,
    /// `usePotions=true`: a dose leaves the flask and the boost lands mid-fight.
    Potions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatLoot {
    HerbLawNature,
    BigBones,
    BigBonesOrLimpwurt,
    DragonBonesOrHide,
    /// Both guaranteed Green dragon drops must increase after Start.
    /// `DragonBonesOrHide` cannot distinguish that pair from two of one id.
    DragonBonesAndHide,
    /// Guard-drop pack stock: any one of the six verifiable Guard drops
    /// ([`GUARD_DROP_IDS`]) landing in the pack. ArdyFighter lists the same
    /// names in `DEFAULT_LOOT`.
    GuardDrop,
    /// Plain Bones (526) in the pack: the Guard's guaranteed death drop that
    /// AutoFighter bank injects as its only `loot` with burial off.
    Bones,
    /// RockCrab PeriodicBank listed loot the bank cell can verify by id.
    SapphireOrCasket,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombatExtra {
    None,
    DungeonKey,
    RockActivation,
    WornShield,
    DungeonAmulet,
    StolenFood,
    /// FireGiant `EnterDungeon` from the raft: Start is on the surface.
    DungeonApproach,
}

pub fn combat_spec(case: CoreCase) -> Option<CombatSpec> {
    match case {
        CoreCase::ChaosDruid => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_FIELD,
            radius: 14,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `location=Chaos Druid Tower`: surface camp r4, Thieving 46 door, same
        // Chaos druid display name and Herb/Law/Nature pickup as Edgeville.
        CoreCase::ChaosDruidTower => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_TOWER_FIELD,
            radius: 4,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `location=Yanille Dungeon`: warrior room target identity differs; loot
        // still the card's selected Herb/Law/Nature set.
        CoreCase::ChaosDruidYanille => Some(CombatSpec {
            target: "Chaos druid warrior",
            stand: CHAOS_DRUID_YANILLE_FIELD,
            radius: 8,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiant => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiantPrepared => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Observe reuse only. Bank-only dart Start is empty; do not feed this
        // spec to combat_baseline_ready(Ranged) or CombatCoreCycle::qualified().
        CoreCase::MossGiantDart => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: MOSS_GIANT_DART_FIELD_RADIUS,
            food_id: LOBSTER_ID,
            food_count: 0,
            weapon_id: BRONZE_DART_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: Some(BRONZE_DART_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::HillGiant => Some(CombatSpec {
            target: "Giant",
            stand: HILL_GIANT_PIT,
            radius: 16,
            food_id: TROUT_ID,
            food_count: HILL_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBonesOrLimpwurt,
            extra: CombatExtra::DungeonKey,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::AutoFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::AutoFighterMage => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: STAFF_OF_FIRE_ID,
            style: CombatStyleWitness::FireStrike,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `combatStyle=range`: the source selects the rapid combat mode itself
        // and fires the worn Bronze arrow stack, so both transitions are the
        // witnessed outcome instead of a prepared state.
        CoreCase::AutoFighterRange => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: MAPLE_SHORTBOW_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::None,
            projectile: Some(BRONZE_ARROW_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::RockCrab => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::RockActivation,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // RockCrab's own `AmmoLogic.sweepPlan` branch: dormant Rocks into a crab,
        // rapid combat mode, and a worn projectile stack that actually shrinks.
        CoreCase::RockCrabRange => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: MAPLE_SHORTBOW_ID,
            style: CombatStyleWitness::Ranged,
            loot: CombatLoot::None,
            extra: CombatExtra::RockActivation,
            projectile: Some(BRONZE_ARROW_ID),
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragon | CoreCase::GreenDragonPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BASE_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Fire Strike branch on the enabled GreenDragon card. Loot stays None:
        // this cell does not claim a full kill unless a later hop proves one.
        CoreCase::GreenDragonMagePrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: STAFF_OF_FIRE_ID,
            style: CombatStyleWitness::FireStrike,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // `useSpecial=true` with the dragon dagger: the bar arms and `%sa_energy`
        // pays the 250 cost, so a queued-but-unspent arming cannot qualify.
        CoreCase::GreenDragonSpecial | CoreCase::GreenDragonSpecialPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: DRAGON_DAGGER_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::Special,
        }),
        // `usePotions=true`: a three-dose flask becomes its two-dose form and the
        // native boost has to land on the matching skill during the fight.
        CoreCase::GreenDragonPotions | CoreCase::GreenDragonPotionsPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::Potions,
        }),
        CoreCase::FireGiant => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantPrepared => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ArdyFighter => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 12,
            food_id: CAKE_ID,
            food_count: 0,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::StolenFood,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // The five bank cells keep their card's combat shape; the bank trip
        // itself is declared in `combat_bank_spec`. Prepared food differs from
        // the plain core where the card's own trip rule needs a shortfall
        // (MossGiant banks when the pack runs dry, ChaosDruidKiller when the
        // carried food is under `foodWithdraw`). AutoFighter finishes its
        // fights instantly at these levels, so the cell's trip end is the
        // injected `bankAtLootSlots=1`: the loot class is the card's injected
        // `loot=[Bones]` (the Guard's guaranteed drop, burial off), and the
        // pack starts empty of it.
        CoreCase::AutoFighterBank => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 8,
            food_id: TROUT_ID,
            food_count: AUTO_FIGHTER_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::Bones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::MossGiantBank => Some(CombatSpec {
            target: "Moss giant",
            stand: MOSS_GIANT_SAFESPOT,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: MOSS_GIANT_BANK_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::HillGiantBank | CoreCase::HillGiantBankPrepared => Some(CombatSpec {
            target: "Giant",
            stand: HILL_GIANT_PIT,
            radius: 16,
            food_id: TROUT_ID,
            food_count: HILL_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBonesOrLimpwurt,
            extra: CombatExtra::DungeonKey,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ChaosDruidBank => Some(CombatSpec {
            target: "Chaos druid",
            stand: CHAOS_DRUID_FIELD,
            radius: 14,
            food_id: LOBSTER_ID,
            food_count: CHAOS_DRUID_BANK_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::HerbLawNature,
            extra: CombatExtra::None,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::ArdyFighterBank => Some(CombatSpec {
            target: "Guard",
            stand: ARDY_THIEVER_STAND,
            radius: 12,
            food_id: CAKE_ID,
            food_count: 0,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            // The card's own DEFAULT_LOOT is already the Guard-reachable
            // names; `bankEveryItems=1` ends the trip on the first of those
            // drops. Pack starts empty of the class so the deposit cannot be
            // a pre-Start seed. StolenFood extra stays: the Baker's stall is
            // this card's restock.
            loot: CombatLoot::GuardDrop,
            extra: CombatExtra::StolenFood,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::RockCrabBank => Some(CombatSpec {
            target: "Rock Crab",
            stand: ROCK_CRAB_SAFE_STAND,
            radius: 2,
            food_id: LOBSTER_ID,
            food_count: ROCK_CRAB_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::SapphireOrCasket,
            extra: CombatExtra::RockActivation,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBank => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBankPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BANK_PREPARED_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesOrHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonBankDefaultPrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_BANK_PREPARED_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::DragonBonesAndHide,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        // Teleport escape is Magic XP + Varrock land, then the Edgeville bank
        // trip. Combat kills are not this cell: a flee-walk to Edgeville
        // without Magic XP / Varrock land fails it.
        CoreCase::GreenDragonTele => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: GREEN_DRAGON_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::GreenDragonTelePrepared => Some(CombatSpec {
            target: "Green dragon",
            stand: GREEN_DRAGON_FIELD,
            radius: 22,
            food_id: LOBSTER_ID,
            food_count: 0,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::WornShield,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantApproach => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_RAFT,
            radius: 5,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::None,
            extra: CombatExtra::DungeonApproach,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantBank => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_FOOD,
            weapon_id: ADAMANT_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        CoreCase::FireGiantBankPrepared | CoreCase::FireGiantCamelotPrepared => Some(CombatSpec {
            target: "Fire giant",
            stand: FIRE_GIANT_ROOM,
            radius: 10,
            food_id: LOBSTER_ID,
            food_count: FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD,
            weapon_id: RUNE_SCIMITAR_ID,
            style: CombatStyleWitness::Strength,
            loot: CombatLoot::BigBones,
            extra: CombatExtra::DungeonAmulet,
            projectile: None,
            consumable: CombatConsumable::None,
        }),
        _ => None,
    }
}

/// The declared bank trip a bank cell has to execute: the pack stock that has
/// to land in the bank, the card's own booth stand, the food the card restocks
/// there (when it restocks at all) and the tile the trip returns to before
/// further work. Values are the frozen cards' own tiles and keep-lists; see
/// `docs/compat/05-options-149.md` and `docs/compat/05-options-150.md`.
#[derive(Debug, Clone, Copy)]
pub struct CombatBankSpec {
    /// Pack-to-bank stock: the card's own loot or carried-food class. Any one
    /// of these ids counts, because the source's matcher is per item.
    pub deposit: &'static [i32],
    /// The bank the card's own walk opens a booth at.
    pub stand: (i32, i32, i32),
    pub stand_radius: i32,
    /// Food the trip withdraws back into the pack, when the card restocks.
    pub restock: Option<i32>,
    /// Exact post-withdrawal pack count when this fixture selects one.
    pub restock_count: Option<i32>,
    /// Where the trip returns before further work.
    pub ret: (i32, i32, i32),
    pub ret_radius: i32,
    /// Optional waypoint the trip has to visit before the booth: Varrock land
    /// for GreenDragon tele, barrel wash-up for FireGiant barrel exit.
    pub via: Option<(i32, i32, i32)>,
    pub via_radius: i32,
    /// The via tile is a spellbook teleport: Magic XP has to land with it.
    pub via_magic: bool,
    /// When false, the bank trip itself is the cell (tele escape). The
    /// reviewed two-engagement combat core is not required.
    pub require_combat: bool,
    /// A death/recovery path cannot substitute for this fixture's intended
    /// escape route.
    pub forbid_death: bool,
}

pub fn combat_bank_spec(case: CoreCase) -> Option<CombatBankSpec> {
    match case {
        // `banking=Auto`: BankRun walks to the nearest bank from the anchor and
        // deposits everything its keep-list does not hold, then restocks food.
        // The deposit class is the card's own injected `loot=[Bones]`: burial
        // is off, so the pack only ever holds Bones because a Guard dropped
        // them and the script looted them.
        CoreCase::AutoFighterBank => Some(CombatBankSpec {
            deposit: &[BONES_ID],
            stand: ARDOUGNE_EAST_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: None,
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Script-owned Ardougne West trip: deposits everything but
        // food/runes/ammo/weapon and withdraws lobster back to the safespot.
        CoreCase::MossGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: MOSS_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: MOSS_GIANT_SAFESPOT,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Always-on trip end: Varrock West keeps only trout and the Brass key.
        CoreCase::HillGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: HILL_GIANT_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: None,
            ret: HILL_GIANT_PIT,
            ret_radius: 16,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::HillGiantBankPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID, LIMPWURT_ROOT_ID],
            stand: HILL_GIANT_BANK,
            stand_radius: 6,
            restock: Some(TROUT_ID),
            restock_count: Some(HILL_GIANT_BANK_PREPARED_RESTOCK),
            ret: HILL_GIANT_PIT,
            ret_radius: 16,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Edgeville trip end: `depositInventory` empties the pack, then the
        // card withdraws exactly its food back and returns through the trapdoor.
        CoreCase::ChaosDruidBank => Some(CombatBankSpec {
            deposit: &[LOBSTER_ID],
            stand: CHAOS_DRUID_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: CHAOS_DRUID_FIELD,
            ret_radius: 14,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // `bankStrategy=Loot count`: PeriodicBank deposits the card's own
        // Guard-reachable loot list and walks back to the market anchor. No
        // food restock (the Baker's stall is this card's food). The class
        // only ever enters the pack as a Guard drop.
        CoreCase::ArdyFighterBank => Some(CombatBankSpec {
            deposit: &GUARD_DROP_IDS,
            stand: ARDY_BANK,
            stand_radius: 6,
            restock: None,
            restock_count: None,
            ret: ARDY_THIEVER_STAND,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // RockCrab PeriodicBank Loot-count at Seers. The task deposits listed
        // loot and returns to `currentSpot()`; it does not restock food
        // (`BankRun` is the food-gone restock, a different trip).
        CoreCase::RockCrabBank => Some(CombatBankSpec {
            deposit: &[UNCUT_SAPPHIRE_ID, CASKET_ID],
            stand: SEERS_BANK,
            stand_radius: 6,
            restock: None,
            restock_count: None,
            ret: ROCK_CRAB_BANK_RET,
            ret_radius: 6,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // GreenDragon BankRun to Edgeville: deposit except keep-list, withdraw
        // food to `foodWithdraw` 20, walk back past the ditch.
        CoreCase::GreenDragonBank | CoreCase::GreenDragonBankPrepared => Some(CombatBankSpec {
            deposit: &[DRAGON_BONES_ID, GREEN_DRAGONHIDE_ID],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::GreenDragonBankDefaultPrepared => Some(CombatBankSpec {
            deposit: &[GREEN_DRAGONHIDE_ID],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: Some(GREEN_DRAGON_BANK_PREPARED_RESTOCK),
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: None,
            via_radius: 0,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // `escape=Teleport to Varrock`: Magic XP and a Varrock land, then the
        // Edgeville booth. A south-walk flee without the teleport fails.
        CoreCase::GreenDragonTele => Some(CombatBankSpec {
            deposit: &[],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: Some(VARROCK_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: false,
            forbid_death: false,
        }),
        CoreCase::GreenDragonTelePrepared => Some(CombatBankSpec {
            deposit: &[],
            stand: GREEN_DRAGON_BANK,
            stand_radius: 8,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: GREEN_DRAGON_FIELD,
            ret_radius: 22,
            via: Some(VARROCK_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: false,
            forbid_death: true,
        }),
        // Barrel exit to (2527,3413,0) then Ardougne West restock and re-entry.
        CoreCase::FireGiantBank => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: FIRE_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: None,
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(FIRE_GIANT_WASH),
            via_radius: 6,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        CoreCase::FireGiantBankPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: FIRE_GIANT_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: Some(FIRE_GIANT_BANK_PREPARED_RESTOCK),
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(FIRE_GIANT_WASH),
            via_radius: 6,
            via_magic: false,
            require_combat: true,
            forbid_death: false,
        }),
        // Source-specific Camelot escape: land + Magic XP, then Seers restock.
        // The Seers endpoint alone is not this via.
        CoreCase::FireGiantCamelotPrepared => Some(CombatBankSpec {
            deposit: &[BIG_BONES_ID],
            stand: SEERS_BANK,
            stand_radius: 6,
            restock: Some(LOBSTER_ID),
            restock_count: Some(FIRE_GIANT_CAMELOT_PREPARED_RESTOCK),
            ret: FIRE_GIANT_ROOM,
            ret_radius: 10,
            via: Some(CAMELOT_TELE_LAND),
            via_radius: 8,
            via_magic: true,
            require_combat: true,
            forbid_death: false,
        }),
        _ => None,
    }
}

pub fn combat_noted(observation: &Observation) -> bool {
    noted_herb_id_held(observation)
        || observation.item_id(NOTED_BIG_BONES_ID) > 0
        || observation.bank_item_id(NOTED_BIG_BONES_ID) > 0
        || observation.item_id(NOTED_LIMPWURT_ROOT_ID) > 0
        || observation.bank_item_id(NOTED_LIMPWURT_ROOT_ID) > 0
        || observation.item_id(NOTED_BONES_ID) > 0
        || observation.bank_item_id(NOTED_BONES_ID) > 0
        || observation.item_id(NOTED_DRAGON_BONES_ID) > 0
        || observation.bank_item_id(NOTED_DRAGON_BONES_ID) > 0
        || observation.item_id(NOTED_GREEN_DRAGONHIDE_ID) > 0
        || observation.bank_item_id(NOTED_GREEN_DRAGONHIDE_ID) > 0
        || observation.item_id(NOTED_DRAGONFIRE_SHIELD_ID) > 0
        || observation.bank_item_id(NOTED_DRAGONFIRE_SHIELD_ID) > 0
        || observation.item_id(NOTED_CASKET_ID) > 0
        || observation.bank_item_id(NOTED_CASKET_ID) > 0
        || observation.item_id(NOTED_UNCUT_SAPPHIRE_ID) > 0
        || observation.bank_item_id(NOTED_UNCUT_SAPPHIRE_ID) > 0
        || noted_stall_food(observation) > 0
}

pub fn noted_herb_id_held(observation: &Observation) -> bool {
    observation
        .item_ids
        .keys()
        .copied()
        .chain(observation.bank_ids.keys().copied())
        .any(noted_herb_id)
}

pub fn combat_loot_count(observation: &Observation, loot: CombatLoot) -> i32 {
    match loot {
        CombatLoot::HerbLawNature => {
            observation.item_id(NATURE_RUNE_ID)
                + observation.item_id(LAW_RUNE_ID)
                + observation
                    .item_ids
                    .iter()
                    .filter(|(id, _)| unidentified_herb_id(**id))
                    .map(|(_, count)| *count)
                    .sum::<i32>()
        }
        CombatLoot::BigBones => observation.item_id(BIG_BONES_ID),
        CombatLoot::BigBonesOrLimpwurt => {
            observation.item_id(BIG_BONES_ID) + observation.item_id(LIMPWURT_ROOT_ID)
        }
        CombatLoot::DragonBonesOrHide | CombatLoot::DragonBonesAndHide => {
            observation.item_id(DRAGON_BONES_ID) + observation.item_id(GREEN_DRAGONHIDE_ID)
        }
        CombatLoot::GuardDrop => GUARD_DROP_IDS
            .iter()
            .map(|id| observation.item_id(*id))
            .sum(),
        CombatLoot::Bones => observation.item_id(BONES_ID),
        CombatLoot::SapphireOrCasket => {
            observation.item_id(UNCUT_SAPPHIRE_ID) + observation.item_id(CASKET_ID)
        }
        CombatLoot::None => 0,
    }
}

pub fn combat_baseline_ready(baseline: &Observation, spec: CombatSpec) -> bool {
    let food_ok = match spec.style {
        CombatStyleWitness::FireStrike => baseline.item_id(spec.food_id) == spec.food_count,
        CombatStyleWitness::Ranged | CombatStyleWitness::Strength if spec.food_count == 0 => true,
        CombatStyleWitness::Ranged | CombatStyleWitness::Strength => {
            baseline.item_id(spec.food_id) >= spec.food_count
        }
    };
    let style_ok = match spec.style {
        CombatStyleWitness::Strength => {
            baseline.level("attack") >= COMBAT_ATTACK_LEVEL
                && baseline.level("strength") >= COMBAT_ATTACK_LEVEL
                && held_id(baseline, spec.weapon_id) >= 1
        }
        CombatStyleWitness::FireStrike => {
            baseline.level("magic") >= AUTO_FIGHTER_MAGE_LEVEL
                && baseline.effective_level("magic") >= AUTO_FIGHTER_MAGE_LEVEL
                && baseline.equipment_id(spec.weapon_id) == 1
                && baseline.item_id(spec.weapon_id) == 0
                && baseline.item_id(MIND_RUNE_ID) == AUTO_FIGHTER_MAGE_CASTS
                && baseline.item_id(AIR_RUNE_ID) == AUTO_FIGHTER_MAGE_AIR_RUNES
        }
        // Both ranged pieces worn: the bow in the weapon slot and the projectile
        // stack in the ammo slot the source fires from.
        CombatStyleWitness::Ranged => {
            baseline.level("ranged") >= RANGED_LEVEL
                && baseline.equipment_id(spec.weapon_id) >= 1
                && spec
                    .projectile
                    .is_some_and(|ammo| baseline.equipment_id(ammo) >= 1)
        }
    };
    let extra_ok = match spec.extra {
        CombatExtra::None => true,
        CombatExtra::RockActivation => baseline.dormant_rocks_seen,
        CombatExtra::DungeonKey => baseline.item_id(BRASS_KEY_ID) == 1,
        CombatExtra::WornShield => {
            baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
                && baseline.tile.is_some_and(|tile| tile.1 >= WILDERNESS_MIN_Z)
        }
        CombatExtra::DungeonAmulet => {
            held_id(baseline, GLARIALS_AMULET_ID) >= 1
                && held_id(baseline, ROPE_ID) >= 1
                && baseline.tile.is_some_and(|tile| tile.1 >= DUNGEON_MIN_Z)
        }
        CombatExtra::StolenFood => {
            baseline.level("thieving") >= 5
                && stall_food(baseline) == 0
                && baseline.item_id(CHOCOLATE_CAKE_ID) == 0
        }
        CombatExtra::DungeonApproach => {
            held_id(baseline, GLARIALS_AMULET_ID) >= 1
                && held_id(baseline, ROPE_ID) >= 1
                && baseline.tile.is_some_and(|tile| tile.1 < DUNGEON_MIN_Z)
        }
    };
    // The consumable branch starts from an unspent, unboosted state: a seeded
    // boost or an already-armed bar would let the run "pass" without the sip
    // or the special it is meant to prove.
    let consumable_ok = match spec.consumable {
        CombatConsumable::None => true,
        CombatConsumable::Special => {
            baseline.equipment_id(spec.weapon_id) >= 1
                && baseline.varp(SA_ARMED_VARP) != SA_ARMED_VALUE
        }
        CombatConsumable::Potions => {
            baseline.item_id(SUPER_ATTACK_3_ID) >= 1
                && baseline.item_id(SUPER_ATTACK_2_ID) == 0
                && baseline.item_id(SUPER_STRENGTH_3_ID) >= 1
                && baseline.item_id(SUPER_STRENGTH_2_ID) == 0
                && baseline.effective_level("attack") <= baseline.level("attack")
                && baseline.effective_level("strength") <= baseline.level("strength")
        }
    };
    near(baseline.tile, spec.stand, spec.radius)
        && baseline.level("hitpoints") >= COMBAT_ATTACK_LEVEL
        && food_ok
        && style_ok
        && combat_loot_count(baseline, spec.loot) == 0
        && !combat_noted(baseline)
        && extra_ok
        && consumable_ok
}

pub(super) fn prepared_combat_baseline_ready(case: CoreCase, baseline: &Observation) -> bool {
    // Mage is 70 Magic/Defence/HP with staff+shield. Rune armour and a
    // scimitar would falsify this branch; check it before armour_ready.
    if case == CoreCase::GreenDragonMagePrepared {
        return ["magic", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL)
            && baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_FOOD
            && baseline.equipment_id(STAFF_OF_FIRE_ID) == 1
            && baseline.item_id(STAFF_OF_FIRE_ID) == 0
            && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
            && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
            && baseline.item_id(MIND_RUNE_ID) == AUTO_FIGHTER_MAGE_CASTS
            && baseline.item_id(AIR_RUNE_ID) == AUTO_FIGHTER_MAGE_AIR_RUNES
            && baseline.item_id(RUNE_SCIMITAR_ID) == 0
            && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            && [RUNE_CHAINBODY_ID, RUNE_PLATELEGS_ID, RUNE_FULL_HELM_ID]
                .into_iter()
                .all(|id| baseline.equipment_id(id) == 0 && baseline.item_id(id) == 0);
    }
    let armour_ready = [RUNE_CHAINBODY_ID, RUNE_PLATELEGS_ID, RUNE_FULL_HELM_ID]
        .into_iter()
        .all(|id| baseline.equipment_id(id) == 1);
    let exact_stats = match case {
        CoreCase::GreenDragonSpecialPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL),
        CoreCase::GreenDragonPrepared | CoreCase::FireGiantPrepared => {
            baseline.level("defence") == COMBAT_ATTACK_LEVEL
        }
        CoreCase::GreenDragonPotionsPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL),
        CoreCase::GreenDragonTelePrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == GREEN_DRAGON_TELE_PREPARED_LEVEL),
        CoreCase::GreenDragonBankPrepared
        | CoreCase::GreenDragonBankDefaultPrepared
        | CoreCase::FireGiantBankPrepared
        | CoreCase::FireGiantCamelotPrepared => ["attack", "strength", "defence", "hitpoints"]
            .into_iter()
            .all(|stat| baseline.level(stat) == BANK_PRESSURE_PREPARED_LEVEL),
        CoreCase::MossGiantPrepared | CoreCase::HillGiantBankPrepared => {
            ["attack", "strength", "defence", "hitpoints"]
                .into_iter()
                .all(|stat| baseline.level(stat) == REMAINING_COMBAT_PREPARED_LEVEL)
        }
        _ => return false,
    };
    let exact_food = match case {
        CoreCase::GreenDragonPrepared => baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_BASE_FOOD,
        CoreCase::GreenDragonSpecialPrepared | CoreCase::GreenDragonPotionsPrepared => {
            baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_FOOD
        }
        CoreCase::GreenDragonBankPrepared | CoreCase::GreenDragonBankDefaultPrepared => {
            baseline.item_id(LOBSTER_ID) == GREEN_DRAGON_BANK_PREPARED_FOOD
        }
        CoreCase::GreenDragonTelePrepared => baseline.item_id(LOBSTER_ID) == 0,
        CoreCase::FireGiantPrepared => baseline.item_id(LOBSTER_ID) == FIRE_GIANT_FOOD,
        CoreCase::FireGiantBankPrepared | CoreCase::FireGiantCamelotPrepared => {
            baseline.item_id(LOBSTER_ID) == FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD
        }
        CoreCase::MossGiantPrepared => baseline.item_id(LOBSTER_ID) == MOSS_GIANT_FOOD,
        CoreCase::HillGiantBankPrepared => baseline.item_id(TROUT_ID) == HILL_GIANT_FOOD,
        _ => return false,
    };
    let exact_profile = exact_stats && armour_ready && exact_food;
    exact_profile
        && match case {
            CoreCase::GreenDragonPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            }
            CoreCase::GreenDragonSpecialPrepared => {
                baseline.item_id(DRAGON_DAGGER_ID) == 0
                    && baseline.equipment_id(DRAGON_DAGGER_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.varp(SA_ENERGY_VARP) >= DRAGON_DAGGER_SPECIAL_COST
            }
            CoreCase::GreenDragonPotionsPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 0
            }
            CoreCase::GreenDragonBankPrepared | CoreCase::GreenDragonBankDefaultPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
            }
            CoreCase::GreenDragonTelePrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.item_id(DRAGONFIRE_SHIELD_ID) == 0
                    && baseline.equipment_id(DRAGONFIRE_SHIELD_ID) == 1
                    && baseline.level("magic") >= VARROCK_TELE_MAGIC
                    && baseline.item_id(LAW_RUNE_ID) >= 1
                    && baseline.item_id(AIR_RUNE_ID) >= 3
                    && baseline.item_id(FIRE_RUNE_ID) >= 1
                    && baseline.effective_level("hitpoints") == GREEN_DRAGON_TELE_PREPARED_HP
            }
            CoreCase::FireGiantPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::FireGiantBankPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::FireGiantCamelotPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
                    && baseline.equipment_id(GLARIALS_AMULET_ID) == 1
                    && baseline.item_id(GLARIALS_AMULET_ID) == 0
                    && baseline.level("magic") >= CAMELOT_TELE_MAGIC
                    && baseline.effective_level("magic") >= CAMELOT_TELE_MAGIC
                    && baseline.item_id(AIR_RUNE_ID) >= CAMELOT_AIR_CARRY
                    && baseline.item_id(LAW_RUNE_ID) >= CAMELOT_LAW_CARRY
            }
            CoreCase::MossGiantPrepared => {
                baseline.item_id(RUNE_SCIMITAR_ID) == 0
                    && baseline.equipment_id(RUNE_SCIMITAR_ID) == 1
            }
            CoreCase::HillGiantBankPrepared => {
                baseline.item_id(ADAMANT_SCIMITAR_ID) == 0
                    && baseline.equipment_id(ADAMANT_SCIMITAR_ID) == 1
                    && baseline.item_id(BRASS_KEY_ID) == 1
            }
            _ => false,
        }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatNpcLast {
    pub name: String,
    pub health: i32,
    pub animation: i32,
    pub tile: (i32, i32, i32),
    pub engaged: bool,
    pub defeated: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct CombatFact {
    pub kind: &'static str,
    pub index: usize,
    pub name: String,
    pub health: i32,
    pub animation: i32,
    pub loot_id: Option<i32>,
    pub zero_health: bool,
}

// JSON objects cannot encode tuple keys. Retain each ground-count fact as a
// named row while keeping the observer's keyed map unchanged.
pub fn serialize_ground_counts<S: serde::Serializer>(
    counts: &BTreeMap<(i32, i32, i32, i32), i32>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    #[derive(Serialize)]
    struct GroundCount {
        id: i32,
        x: i32,
        z: i32,
        level: i32,
        count: i32,
    }
    serializer.collect_seq(
        counts
            .iter()
            .map(|(&(id, x, z, level), &count)| GroundCount {
                id,
                x,
                z,
                level,
                count,
            }),
    )
}

/// Sustained selected-target combat: two engagements, a verified defeat
/// that is not mere despawn, fresh selected work after that defeat,
/// selected-style XP, and exact loot where required.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatCoreCycle {
    pub engagements: u32,
    pub defeats: u32,
    pub further_work: bool,
    pub style_xp: bool,
    pub autocast_armed: bool,
    pub mind_rune_consumed: bool,
    pub air_runes_consumed: bool,
    /// Rapid `com_mode` observed, the ranged branches' own style selection.
    pub combat_mode: bool,
    /// Highest worn projectile count seen; a later lower count means arrows fired.
    pub projectile_peak: i32,
    pub projectile_fired: bool,
    /// Spec bar armed (`%sa_attack`) and `%sa_energy` actually paid the cost.
    pub special_armed: bool,
    pub special_spent: bool,
    /// Highest `%sa_energy` seen; a later fall of a full cost means a real spend.
    pub special_energy_peak: i32,
    /// A three-dose flask became its two-dose form while in combat.
    pub potion_dose: bool,
    /// The matching native attack/strength boost landed after that dose.
    pub potion_boost: bool,
    pub looted: bool,
    pub noted: bool,
    pub activated: bool,
    pub shield_worn: bool,
    /// Mage-branch only: latched when Start wore 1540 and a later WornShield frame saw it off.
    /// Re-equipping does not clear this; `qualified()` still uses sticky `shield_worn` only.
    pub shield_continuity_break: bool,
    pub stolen_food: bool,
    pub wrong_item: bool,
    pub dormant_indexes: BTreeSet<usize>,
    pub dormant_tiles: BTreeMap<usize, (i32, i32, i32)>,
    pub last: BTreeMap<usize, CombatNpcLast>,
    pub currently_engaged: BTreeSet<usize>,
    #[serde(serialize_with = "serialize_ground_counts")]
    pub previous_ground: BTreeMap<(i32, i32, i32, i32), i32>,
    pub facts: Vec<CombatFact>,
}

impl CombatCoreCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        self.noted |= combat_noted(now);
        match spec.style {
            CombatStyleWitness::Strength => {
                self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength");
            }
            CombatStyleWitness::FireStrike => {
                self.style_xp |= now.skill_xp("magic") > baseline.skill_xp("magic");
                self.autocast_armed |= now.varp(AUTOCAST_MAGIC_VARP) == AUTOCAST_ARMED_VALUE;
                self.mind_rune_consumed |=
                    now.item_id(MIND_RUNE_ID) < baseline.item_id(MIND_RUNE_ID);
                self.air_runes_consumed |= baseline
                    .item_id(AIR_RUNE_ID)
                    .saturating_sub(now.item_id(AIR_RUNE_ID))
                    >= 2;
            }
            // `combatStyle=range`: the source picks the rapid combat mode and fires
            // the worn stack down, so both are observed transitions of this branch.
            CombatStyleWitness::Ranged => {
                self.style_xp |= now.skill_xp("ranged") > baseline.skill_xp("ranged");
                self.combat_mode |= now.varp(COMBAT_MODE_VARP) == RAPID_COMBAT_MODE;
                if let Some(ammo) = spec.projectile {
                    let worn = now.equipment_id(ammo);
                    self.projectile_fired |= self.projectile_peak > worn;
                    self.projectile_peak = self.projectile_peak.max(worn);
                }
            }
        }
        self.looted |= match spec.loot {
            CombatLoot::DragonBonesAndHide => {
                now.item_id(DRAGON_BONES_ID) > baseline.item_id(DRAGON_BONES_ID)
                    && now.item_id(GREEN_DRAGONHIDE_ID) > baseline.item_id(GREEN_DRAGONHIDE_ID)
            }
            _ => combat_loot_count(now, spec.loot) > combat_loot_count(baseline, spec.loot),
        };
        self.wrong_item |= now.item_id(BLACK_DRAGONHIDE_ID) > 0
            || now.item_id(RED_DRAGONHIDE_ID) > 0
            || now.item_id(BLUE_DRAGONHIDE_ID) > 0
            || now.item_id(CHOCOLATE_CAKE_ID) > 0;
        if spec.extra == CombatExtra::WornShield {
            self.shield_worn |= now.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1;
            if baseline.equipment_id(DRAGONFIRE_SHIELD_ID) >= 1
                && now.equipment_id(DRAGONFIRE_SHIELD_ID) == 0
            {
                self.shield_continuity_break = true;
            }
        }
        if spec.extra == CombatExtra::StolenFood {
            self.stolen_food |= stall_food(now) > stall_food(baseline);
        }
        if spec.extra == CombatExtra::RockActivation {
            for npc in &now.npc_facts {
                let Some(name) = npc.name.as_deref().map(str::trim) else {
                    continue;
                };
                if name == ROCK_CRAB_DORMANT_NAME {
                    self.dormant_indexes.insert(npc.index);
                    self.dormant_tiles.insert(npc.index, npc.tile);
                }
                // Preserve the existing index OR tile identity match. An
                // unrelated index and tile cannot count as activation; this
                // predicate does not separately rule out reuse of an old slot.
                if name == spec.target
                    && (self.dormant_indexes.contains(&npc.index)
                        || self.dormant_tiles.values().any(|tile| *tile == npc.tile))
                {
                    self.activated = true;
                }
            }
        }
        match spec.consumable {
            CombatConsumable::None => {}
            // `useSpecial=true`: arming alone is not an execution, the pool has to
            // pay the wielded weapon's cost. A full cost below either the pre-Start
            // pool or the highest pool seen since is that payment; the energy only
            // ever regenerates, so a fall of this size cannot be anything else.
            CombatConsumable::Special => {
                self.special_armed |= now.varp(SA_ARMED_VARP) == SA_ARMED_VALUE;
                let energy = now.varp(SA_ENERGY_VARP);
                self.special_energy_peak = self.special_energy_peak.max(energy);
                self.special_spent |= baseline.varp(SA_ENERGY_VARP).saturating_sub(energy)
                    >= DRAGON_DAGGER_SPECIAL_COST
                    || self.special_energy_peak.saturating_sub(energy)
                        >= DRAGON_DAGGER_SPECIAL_COST;
            }
            // `usePotions=true`: the sip shows up as the three-dose flask becoming
            // its two-dose form in a fight, and the boost has to land afterwards.
            CombatConsumable::Potions => {
                if !self.potion_dose {
                    let attack_sip = baseline.item_id(SUPER_ATTACK_3_ID)
                        > now.item_id(SUPER_ATTACK_3_ID)
                        && now.item_id(SUPER_ATTACK_2_ID) > baseline.item_id(SUPER_ATTACK_2_ID);
                    let strength_sip = baseline.item_id(SUPER_STRENGTH_3_ID)
                        > now.item_id(SUPER_STRENGTH_3_ID)
                        && now.item_id(SUPER_STRENGTH_2_ID) > baseline.item_id(SUPER_STRENGTH_2_ID);
                    if (attack_sip || strength_sip) && now.local_in_combat {
                        self.potion_dose = true;
                    }
                }
                if self.potion_dose {
                    self.potion_boost |= now.effective_level("attack") > now.level("attack")
                        || now.effective_level("strength") > now.level("strength");
                }
            }
        }
        let mut seen = BTreeSet::new();
        for npc in &now.npc_facts {
            let Some(name) = npc.name.as_deref().map(str::trim) else {
                continue;
            };
            if name != spec.target {
                continue;
            }
            seen.insert(npc.index);
            let prev = self.last.get(&npc.index).cloned();
            let selected_combat = npc.targeting_local
                || (now.local_target_npc == Some(npc.index) && now.local_in_combat);
            let health_drop = prev.as_ref().is_some_and(|prev| {
                prev.health > 0 && prev.health > npc.health && npc.health >= 0 && prev.engaged
            });
            let new_spawn = prev
                .as_ref()
                .is_some_and(|prev| prev.defeated && npc.total_health > 0 && npc.health > 0);
            if new_spawn {
                self.currently_engaged.remove(&npc.index);
            }
            // Defeated indexes stay dead until a positive-health respawn. A known
            // zero-health bar (total_health > 0) is a corpse/death frame — selected
            // combat on it must not open a new engagement or mark further work.
            // total_health == 0 remains unknown (no bar), not universally dead.
            let stale_defeated = prev.as_ref().is_some_and(|prev| prev.defeated) && !new_spawn;
            let known_corpse = npc.total_health > 0 && npc.health == 0;
            let was_engaged = self.currently_engaged.contains(&npc.index);
            let actual_work = !stale_defeated && !known_corpse && (selected_combat || health_drop);
            if self.defeats > 0 && was_engaged && actual_work {
                self.further_work = true;
            }
            if actual_work && !was_engaged {
                self.engagements += 1;
                self.currently_engaged.insert(npc.index);
                if self.facts.len() < 16 {
                    self.facts.push(CombatFact {
                        kind: "engage",
                        index: npc.index,
                        name: name.to_string(),
                        health: npc.health,
                        animation: npc.animation,
                        loot_id: None,
                        zero_health: false,
                    });
                }
            }
            if npc.total_health > 0
                && npc.health == 0
                && prev
                    .as_ref()
                    .is_some_and(|prev| prev.engaged && prev.health > 0)
                && self.currently_engaged.contains(&npc.index)
            {
                self.record_defeat(npc.index, name, npc.health, npc.animation, None, true);
            }
            let defeated =
                self.last.get(&npc.index).is_some_and(|last| last.defeated) && !new_spawn;
            self.last.insert(
                npc.index,
                CombatNpcLast {
                    name: name.to_string(),
                    health: npc.health,
                    animation: npc.animation,
                    tile: npc.tile,
                    engaged: self.currently_engaged.contains(&npc.index),
                    defeated,
                },
            );
        }
        let missing: Vec<(usize, CombatNpcLast)> = self
            .last
            .iter()
            .filter(|(index, prev)| prev.engaged && !prev.defeated && !seen.contains(index))
            .map(|(index, prev)| (*index, prev.clone()))
            .collect();
        for (index, prev) in missing {
            let loot_id = now.ground_loot.iter().find_map(|item| {
                let key = (item.id, item.tile.0, item.tile.1, item.tile.2);
                let baseline_count = baseline
                    .ground_loot
                    .iter()
                    .filter(|old| old.id == item.id && old.tile == item.tile)
                    .map(|old| old.count)
                    .sum::<i32>();
                let previous_count = self.previous_ground.get(&key).copied().unwrap_or(0);
                (item.tile == prev.tile
                    && combat_loot_id(item.id, spec.loot)
                    && item.count > baseline_count.max(previous_count))
                .then_some(item.id)
            });
            if loot_id.is_some() {
                self.record_defeat(
                    index,
                    &prev.name,
                    prev.health,
                    prev.animation,
                    loot_id,
                    false,
                );
            }
        }
        self.previous_ground.clear();
        for item in &now.ground_loot {
            let key = (item.id, item.tile.0, item.tile.1, item.tile.2);
            *self.previous_ground.entry(key).or_insert(0) += item.count;
        }
    }

    pub fn record_defeat(
        &mut self,
        index: usize,
        name: &str,
        health: i32,
        animation: i32,
        loot_id: Option<i32>,
        zero_health: bool,
    ) {
        if self.last.get(&index).is_some_and(|prev| prev.defeated) {
            return;
        }
        self.defeats += 1;
        self.currently_engaged.remove(&index);
        if let Some(prev) = self.last.get_mut(&index) {
            prev.defeated = true;
            prev.engaged = false;
        }
        if self.facts.len() < 16 {
            self.facts.push(CombatFact {
                kind: "defeat",
                index,
                name: name.to_string(),
                health,
                animation,
                loot_id,
                zero_health,
            });
        }
    }

    pub fn qualified(&self, spec: CombatSpec) -> bool {
        let style_ok = match spec.style {
            CombatStyleWitness::Strength => self.style_xp,
            CombatStyleWitness::FireStrike => {
                self.style_xp
                    && self.autocast_armed
                    && self.mind_rune_consumed
                    && self.air_runes_consumed
            }
            // Ranged XP alone is not the branch: the source's rapid combat mode and
            // a projectile that actually left the quiver are both required.
            CombatStyleWitness::Ranged => {
                self.style_xp && self.combat_mode && self.projectile_fired
            }
        };
        let extra_ok = match spec.extra {
            CombatExtra::None | CombatExtra::DungeonKey | CombatExtra::DungeonAmulet => true,
            CombatExtra::RockActivation => self.activated,
            CombatExtra::WornShield => self.shield_worn,
            CombatExtra::StolenFood => self.stolen_food,
            CombatExtra::DungeonApproach => true,
        };
        let consumable_ok = match spec.consumable {
            CombatConsumable::None => true,
            CombatConsumable::Special => self.special_armed && self.special_spent,
            CombatConsumable::Potions => self.potion_dose && self.potion_boost,
        };
        self.engagements >= 2
            && self.defeats >= 1
            && self.further_work
            && style_ok
            && (spec.loot == CombatLoot::None || self.looted)
            && !self.noted
            && !self.wrong_item
            && extra_ok
            && consumable_ok
    }
}

/// Fire Strike prepared branch: one correct-target engagement, autocast,
/// Mind+Air spend, and the shield staying worn. Not a full-kill claim.
pub fn qualified_mage_branch(cycle: &CombatCoreCycle, spec: CombatSpec) -> bool {
    spec.style == CombatStyleWitness::FireStrike
        && spec.extra == CombatExtra::WornShield
        && cycle.engagements >= 1
        && cycle.style_xp
        && cycle.autocast_armed
        && cycle.mind_rune_consumed
        && cycle.air_runes_consumed
        && cycle.shield_worn
        && !cycle.shield_continuity_break
        && !cycle.noted
        && !cycle.wrong_item
}

/// Bank-only dart branch: script withdrawal/equip/travel plus one ranged
/// engagement. Never CombatCoreCycle::qualified() (2/1/further_work).
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatDartBranchCycle {
    pub combat: CombatCoreCycle,
    pub withdrawn: bool,
    pub worn: bool,
    pub arrived: bool,
    pub wrong_ammo: bool,
}

impl CombatDartBranchCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        self.combat.observe(spec, baseline, now);
        let bank_withdraw = baseline.bank_open
            && baseline.bank_loaded
            && now.bank_open
            && now.bank_loaded
            && baseline.bank_item_id(BRONZE_DART_ID) > 0
            && now.bank_item_id(BRONZE_DART_ID) < baseline.bank_item_id(BRONZE_DART_ID);
        self.withdrawn |= now.item_id(BRONZE_DART_ID) > 0 || bank_withdraw;
        self.worn |= now.equipment_id(BRONZE_DART_ID) >= 1;
        self.arrived |= near(now.tile, MOSS_GIANT_SAFESPOT, MOSS_GIANT_DART_FIELD_RADIUS);
        self.wrong_ammo |= held_id(now, RUNE_ARROW_ID) > 0 || now.bank_item_id(RUNE_ARROW_ID) > 0;
    }

    pub fn qualified(&self) -> bool {
        self.withdrawn
            && self.worn
            && self.arrived
            && !self.wrong_ammo
            && self.combat.engagements >= 1
            && self.combat.style_xp
            && self.combat.combat_mode
            && self.combat.projectile_fired
            && !self.combat.noted
            && !self.combat.wrong_item
    }
}

pub fn combat_loot_id(id: i32, loot: CombatLoot) -> bool {
    match loot {
        CombatLoot::HerbLawNature => {
            unidentified_herb_id(id) || id == NATURE_RUNE_ID || id == LAW_RUNE_ID
        }
        CombatLoot::BigBones => id == BIG_BONES_ID,
        CombatLoot::BigBonesOrLimpwurt => id == BIG_BONES_ID || id == LIMPWURT_ROOT_ID,
        CombatLoot::DragonBonesOrHide | CombatLoot::DragonBonesAndHide => {
            id == DRAGON_BONES_ID || id == GREEN_DRAGONHIDE_ID
        }
        CombatLoot::GuardDrop => GUARD_DROP_IDS.contains(&id),
        CombatLoot::Bones => id == BONES_ID,
        CombatLoot::SapphireOrCasket => id == UNCUT_SAPPHIRE_ID || id == CASKET_ID,
        CombatLoot::None => false,
    }
}

/// Bank/return cycle for the five bank cells: the reviewed combat witness plus
/// the declared bank trip the card itself has to execute — pack stock moves to
/// the booth's bank, the card's restock line (and selected exact count) is met
/// from that bank's own stock, the modal closes on a later bank session, the
/// trip returns to the card's tile, and work resumes there. A booth opened away
/// from the card's stand, a deposit the card never made, a return without
/// further work, or a bank that only ever opened (seed/readiness) cannot
/// qualify it.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatBankCycle {
    /// The same reviewed combat witness the plain card core uses.
    pub combat: CombatCoreCycle,
    /// The observation that proved the trip's deposit: pack empty of the
    /// deposit class, bank's own stock above its pre-Start level.
    pub banked: Option<Observation>,
    /// Pack food count when the deposit was observed; the restock has to beat it.
    pub food_at_deposit: i32,
    /// The card's restock line was met from the bank's own stock.
    pub restocked: bool,
    /// The modal closed on a later bank session than the deposit.
    pub closed: bool,
    /// The trip returned to the card's own tile.
    pub returned: bool,
    /// Further work after that return.
    pub further: bool,
    /// Strength XP when the return landed; `further` must beat it.
    pub return_xp: Option<i32>,
    /// A loaded booth opened away from the card's declared stand.
    pub wrong_bank: bool,
    /// The optional via tile (Varrock land / barrel wash-up) was visited.
    pub via_seen: bool,
    /// Magic XP landed with the via tile when `via_magic` is set.
    pub via_magic: bool,
    /// A fresh post-Start player death line was observed.
    pub death_seen: bool,
}

impl CombatBankCycle {
    fn deposit_count(observation: &Observation, deposit: &[i32]) -> i32 {
        deposit.iter().map(|id| observation.item_id(*id)).sum()
    }

    fn bank_count(observation: &Observation, deposit: &[i32]) -> i32 {
        deposit.iter().map(|id| observation.bank_item_id(*id)).sum()
    }

    pub fn observe(
        &mut self,
        spec: CombatSpec,
        bank: CombatBankSpec,
        baseline: &Observation,
        now: &Observation,
    ) {
        self.combat.observe(spec, baseline, now);
        self.death_seen |= now.chat.iter().any(|line| {
            !baseline.chat.contains(line)
                && line.1.to_ascii_lowercase().contains("oh dear you are dead")
        });
        let open = now.bank_open && now.bank_loaded;
        let at_stand = near(now.tile, bank.stand, bank.stand_radius);
        if open && !at_stand {
            self.wrong_bank = true;
        }
        if let Some(via) = bank.via {
            if near(now.tile, via, bank.via_radius) {
                self.via_seen = true;
                if bank.via_magic {
                    self.via_magic |= now.skill_xp("magic") > baseline.skill_xp("magic");
                }
            }
        }
        let via_ready =
            bank.via.is_none() || (self.via_seen && (!bank.via_magic || self.via_magic));
        let deposited = if bank.deposit.is_empty() {
            true
        } else {
            Self::deposit_count(now, bank.deposit) == 0
                && Self::bank_count(now, bank.deposit) > Self::bank_count(baseline, bank.deposit)
        };
        if self.banked.is_none()
            && via_ready
            && open
            && at_stand
            // A fresh session: the pre-Start bank state cannot satisfy this.
            && now.bank_generation > baseline.bank_generation
            && deposited
        {
            self.food_at_deposit = bank.restock.map_or(0, |food| now.item_id(food));
            self.banked = Some(now.clone());
        }
        if let Some(banked) = &self.banked {
            if let Some(food) = bank.restock {
                self.restocked |= open
                    && now.bank_generation == banked.bank_generation
                    && now.item_id(food) > self.food_at_deposit
                    && now.bank_item_id(food) < banked.bank_item_id(food)
                    && bank
                        .restock_count
                        .is_none_or(|count| now.item_id(food) == count);
            }
            self.closed |=
                !now.bank_open && !now.bank_loaded && now.bank_generation > banked.bank_generation;
        }
        if self.closed {
            self.returned |= !now.bank_open && near(now.tile, bank.ret, bank.ret_radius);
            if self.returned {
                let xp = now.skill_xp("strength");
                let reference = *self.return_xp.get_or_insert(xp);
                self.further |= !now.bank_open && xp > reference;
            }
        }
    }

    pub fn qualified(&self, spec: CombatSpec, bank: CombatBankSpec) -> bool {
        let combat_ok = !bank.require_combat || self.combat.qualified(spec);
        combat_ok
            && (bank.via.is_none() || self.via_seen)
            && (!bank.via_magic || self.via_magic)
            && self.banked.is_some()
            && (bank.restock.is_none() || self.restocked)
            && self.closed
            && self.returned
            && self.further
            && !self.wrong_bank
            && (!bank.forbid_death || !self.death_seen)
    }
}

/// FireGiant `EnterDungeon` from the raft: Start is on the surface, the script
/// has to put the player at z>=9000, then Strength XP on a Fire giant. An
/// in-room core (already z>=9000 at Start) cannot qualify this cell.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CombatApproachCycle {
    pub entered: bool,
    pub style_xp: bool,
}

impl CombatApproachCycle {
    pub fn observe(&mut self, spec: CombatSpec, baseline: &Observation, now: &Observation) {
        let baseline_surface = baseline.tile.is_some_and(|tile| tile.1 < DUNGEON_MIN_Z);
        self.entered |= baseline_surface && now.tile.is_some_and(|tile| tile.1 >= DUNGEON_MIN_Z);
        self.style_xp |= now.skill_xp("strength") > baseline.skill_xp("strength")
            && spec.style == CombatStyleWitness::Strength;
    }

    pub fn qualified(&self) -> bool {
        self.entered && self.style_xp
    }
}
/// Combat and exact feather loot, then a fresh deposit, return r6, further work.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ChickenKillerBankCycle {
    pub combat_loot: bool,
    pub deposited: Option<Observation>,
    pub returned: bool,
    pub further: bool,
}

impl ChickenKillerBankCycle {
    pub fn observe(&mut self, baseline: &Observation, now: &Observation) {
        self.combat_loot |= now.skill_xp("strength") > baseline.skill_xp("strength")
            && now.item_id(FEATHER_ID) >= 1
            && baseline.item_id(FEATHER_ID) == 0;
        if self.combat_loot
            && self.deposited.is_none()
            && now.bank_open
            && now.bank_loaded
            && now.bank_generation > baseline.bank_generation
            && now.item_id(FEATHER_ID) == 0
            && now.bank_item_id(FEATHER_ID) >= 1
        {
            self.deposited = Some(now.clone());
        }
        if let Some(deposited) = &self.deposited {
            self.returned |= !now.bank_open
                && !now.bank_loaded
                // Closing the modal advances the bank session generation.
                && now.bank_generation > deposited.bank_generation
                && near(now.tile, FALADOR_CHICKENS, 6);
        }
        if self.returned {
            if let Some(deposited) = &self.deposited {
                self.further |= now.skill_xp("strength") > deposited.skill_xp("strength")
                    || now.item_id(FEATHER_ID) >= 1;
            }
        }
    }

    pub fn qualified(&self) -> bool {
        self.further
    }
}