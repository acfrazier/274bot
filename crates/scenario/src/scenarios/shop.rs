use super::{combat::*, production::*};
use crate::*;

const LUMBRIDGE_BANK: WorldTile = WorldTile {
    x: 3092,
    z: 3245,
    level: 0,
};
pub(crate) const FALADOR_TELE_LAND: WorldTile = WorldTile {
    x: 2965,
    z: 3378,
    level: 0,
};
pub(crate) const AEMAD_STAND: WorldTile = WorldTile {
    x: 2613,
    z: 3294,
    level: 0,
};
pub(crate) const AUBURY_STAND: WorldTile = WorldTile {
    x: 3253,
    z: 3401,
    level: 0,
};
/// Frozen shopPresets Lowe stand (Varrock Archery Emporium).
pub(crate) const LOWE_STAND: WorldTile = WorldTile {
    x: 3231,
    z: 3421,
    level: 0,
};
/// Frozen shopPresets Hickton stand (Catherby Archery Emporium).
pub(crate) const HICKTON_STAND: WorldTile = WorldTile {
    x: 2821,
    z: 3442,
    level: 0,
};
/// Frozen shopPresets Harry stand (Catherby Fishing Shop).
pub(crate) const HARRY_STAND: WorldTile = WorldTile {
    x: 2833,
    z: 3443,
    level: 0,
};
/// Frozen shopPresets Betty stand (Port Sarim Magic Emporium). Distinct from
/// herblore `BETTY_SHOP` 3012,3259 — ShopBuyout pin is shopPresets 3012,3258.
pub(crate) const BETTY_STAND: WorldTile = WorldTile {
    x: 3012,
    z: 3258,
    level: 0,
};
/// Frozen shopPresets Gerrant stand (Port Sarim Fishy Business).
pub(crate) const GERRANT_STAND: WorldTile = WorldTile {
    x: 3013,
    z: 3224,
    level: 0,
};
/// Frozen shopPresets Bob stand (Lumbridge Axe Shop).
pub(crate) const BOB_STAND: WorldTile = WorldTile {
    x: 3231,
    z: 3203,
    level: 0,
};
/// Frozen shopPresets Nurmof stand (Dwarven Mine pickaxe shop).
pub(crate) const NURMOF_STAND: WorldTile = WorldTile {
    x: 2997,
    z: 9844,
    level: 0,
};
/// Frozen shopPresets Magic Store owner stand (Yanille Wizards' Guild, floor 1).
pub(crate) const MAGIC_STORE_STAND: WorldTile = WorldTile {
    x: 2594,
    z: 3090,
    level: 1,
};
/// Frozen shopPresets Lundail stand (Mage Arena cellar rune shop).
pub(crate) const LUNDAIL_STAND: WorldTile = WorldTile {
    x: 2535,
    z: 4719,
    level: 0,
};
/// Frozen shopPresets Gundai bankStand (Mage Arena cellar). No booth;
/// `magearena_banker` publishes Talk-to only (`mage_arena.npc`).
pub(crate) const GUNDAI_BANK_APPROACH: WorldTile = WorldTile {
    x: 2533,
    z: 4714,
    level: 0,
};
/// Frozen shopPresets Fernahei stand (Shilo fishing hut).
pub(crate) const FERNAHEI_STAND: WorldTile = WorldTile {
    x: 2870,
    z: 2971,
    level: 0,
};
/// Frozen shopPresets Shilo bankStand. No booth; `shilobanker` display
/// name is `Banker` (`banker.npc`).
pub(crate) const SHILO_BANK_APPROACH: WorldTile = WorldTile {
    x: 2852,
    z: 2954,
    level: 0,
};
/// Stock-facing walkable adjacent for Yanille open booth 2213@2614,3092
/// (map m40_48 local 54,20). Preset bankStand 2613,3092 is Chebyshev 1 west.
pub(crate) const YANILLE_BANK_APPROACH: WorldTile = WorldTile {
    x: 2613,
    z: 3092,
    level: 0,
};
pub(crate) const YANILLE_BANK_BOOTH: WorldTile = WorldTile {
    x: 2614,
    z: 3092,
    level: 0,
};
pub(crate) const YANILLE_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Ardougne East open booth 2213@2656,3283
/// (map m41_51 local 32,19). Preset bankStand 2655,3283 is Chebyshev 1 west;
/// closed 2215@2656,3280 is not operable. Keep the seed stand off the booth.
pub(crate) const AEMAD_BANK_APPROACH: WorldTile = WorldTile {
    x: 2655,
    z: 3283,
    level: 0,
};
pub(crate) const AEMAD_BANK_BOOTH: WorldTile = WorldTile {
    x: 2656,
    z: 3283,
    level: 0,
};
pub(crate) const AEMAD_BANK_BOOTH_ID: i32 = 2213;
/// Catalog Varrock East stand (content.rs) north of open booth 2213@3253,3419
/// (m50_53 local 53,27). Closed 2215@3251/3255,3419 are not operable.
pub(crate) const VARROCK_EAST_BANK_APPROACH: WorldTile = WorldTile {
    x: 3253,
    z: 3420,
    level: 0,
};
pub(crate) const VARROCK_EAST_BANK_BOOTH: WorldTile = WorldTile {
    x: 3253,
    z: 3419,
    level: 0,
};
pub(crate) const VARROCK_EAST_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Catherby open booth 2213@2809,3442
/// (map m43_53 local 57,50 shape 10 angle 2). Preset bankStand 2809,3441 is
/// Chebyshev 1 south; closed 2215@2806/2808/2812,3442 are not operable.
/// Reuses the existing cook CATHERBY_BANK stand identity.
pub(crate) const CATHERBY_BANK_APPROACH: WorldTile = CATHERBY_BANK;
pub(crate) const CATHERBY_BANK_BOOTH: WorldTile = WorldTile {
    x: 2809,
    z: 3442,
    level: 0,
};
pub(crate) const CATHERBY_BANK_BOOTH_ID: i32 = 2213;
/// Stock-facing walkable adjacent for Falador West open booth 2213@2946,3367
/// (vial_filler/climbing_boots booth identity). Preset bankStand 2946,3369 is
/// Chebyshev 2 north of the booth; approach reuses FALADOR_WEST_BANK 2946,3368
/// (Chebyshev 1). Keep the seed stand off the booth tile.
pub(crate) const FALADOR_WEST_BANK_APPROACH: WorldTile = FALADOR_WEST_BANK;
pub(crate) const FALADOR_WEST_BANK_BOOTH: WorldTile = FALADOR_WEST_BOOTH;
pub(crate) const FALADOR_WEST_BANK_BOOTH_ID: i32 = 2213;
pub(super) const VARROCK_ANVIL: WorldTile = WorldTile {
    x: 3188,
    z: 3425,
    level: 0,
};

/// ShopBuyout coin seed: enough banked gp for two+ trips under the injected
/// budgets without product/ballast seed. Deposited through the ordinary booth.
pub(crate) const SHOP_BUYOUT_COIN_SEED: i32 = 20_000;
/// Aemad nonstackable representative: shopdb adventurershop `vial_water`
/// baseline 500 cost 2 sell 1300 → ~2gp/unit at stock; 28-slot pack fill is
/// the natural bank trigger (needSpaceFor). perTrip covers one pack + margin;
/// budget > perTrip leaves sessionSpent headroom for a resumed buy after
/// deposit (budget==perTrip can Stop on budget spent and kill resume).
pub(crate) const SHOP_BUYOUT_AEMAD_PER_TRIP_GP: f64 = 200.0;
pub(crate) const SHOP_BUYOUT_AEMAD_BUDGET_GP: f64 = 600.0;
/// Aubury stackable representative: shopdb runeshop `firerune` baseline 2000
/// cost 4 sell 1000 → 4gp/unit. Pack fill does not bank stackables that
/// already hold a stack; natural bank is coins<100 after a real buy (or
/// budget spent). perTrip 500 spends down under 100; budget 1500 keeps
/// remaining budget for a second withdraw+buy.
pub(crate) const SHOP_BUYOUT_AUBURY_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_AUBURY_BUDGET_GP: f64 = 1500.0;
/// Lowe/Hickton stackable bronze arrows: cost 1 sell 1000 → 1gp/unit at
/// baseline (Lowe stock 2000, Hickton 1000). Same stackable bank trigger as
/// Aubury (coins<100); perTrip 500 exhausts under 100 with stock left.
pub(crate) const SHOP_BUYOUT_LOWE_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_LOWE_BUDGET_GP: f64 = 1500.0;
pub(crate) const SHOP_BUYOUT_HICKTON_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_HICKTON_BUDGET_GP: f64 = 1500.0;
/// Harry stackable fishing bait: cost 3 sell 1000 → 3gp/unit; baseline 1200.
/// perTrip 500 buys ~166 units (498gp) and leaves coins under 100.
pub(crate) const SHOP_BUYOUT_HARRY_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_HARRY_BUDGET_GP: f64 = 1500.0;
/// Betty stackable fire rune: magicshop firerune baseline 1000 cost 4 sell 1000
/// → 4gp/unit (same unit math as Aubury). perTrip 500 / budget 1500 for coins<100
/// bank + resume headroom. Stock 1000 leaves remainder after one trip.
pub(crate) const SHOP_BUYOUT_BETTY_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_BETTY_BUDGET_GP: f64 = 1500.0;
/// Gerrant stackable feather: fishingshop feather baseline 1000 cost 2 sell 1000
/// → 2gp/unit. perTrip 500 buys ~250 units and leaves coins under 100.
pub(crate) const SHOP_BUYOUT_GERRANT_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_GERRANT_BUDGET_GP: f64 = 1500.0;
/// Bob nonstackable steel axe: axeshop sell 1000 delta 20, baseline 3 cost 200.
/// Frozen ShopBuyout banks when `coins < 100` after a real buy (BuyoutPass.ts).
/// Stock-sensitive unit_price at full shelf: stock3→200, stock2→204 (208 unaffordable on 3rd).
/// perTrip 500 → buyoutPlan 200+204=404 spent → 96 coins (<100 bank); 1 steel axe
/// remains on shelf for resumed purchase after deposit. Flat 3×200 ignores haggle.
pub(crate) const SHOP_BUYOUT_BOB_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_BOB_BUDGET_GP: f64 = 1500.0;
/// Nurmof nonstackable iron pickaxe: pickaxeshop sell 1000 delta 20, baseline 5 cost 140.
/// perTrip 500 → stock-sensitive 140+142+145=427 spent → 73 coins (<100 bank).
/// Stock 5 at start; no restock wait on first pass. Long mine↔Falador East route.
pub(crate) const SHOP_BUYOUT_NURMOF_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_NURMOF_BUDGET_GP: f64 = 1500.0;
/// Magic Guild stackable blood rune: magicguildshop baseline 1000 cost 50 members.
/// Stackable coins<100 bank trigger like Aubury; perTrip 500 / budget 1500.
pub(crate) const SHOP_BUYOUT_MAGIC_PER_TRIP_GP: f64 = 500.0;
pub(crate) const SHOP_BUYOUT_MAGIC_BUDGET_GP: f64 = 1500.0;
/// Lundail stackable fire rune: magearena_runeshop sell 1000 delta 30, baseline
/// 200 cost 4. Stock-sensitive unit_price at shelf 200: first units 4gp, rising
/// with sold count. perTrip 500 → 66 fire runes / 490gp → 10 coins (<100 bank);
/// 134 remain for resumed purchase. Cosmic baseline 20 stocks out with 124
/// leftover (restock wait) — not selected. Cellar shop↔Gundai is Chebyshev 5.
const SHOP_BUYOUT_LUNDAIL_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_LUNDAIL_BUDGET_GP: f64 = 1500.0;
/// Fernahei stackable feather: shilofishingshop sell 1000 delta 20, baseline
/// 800 cost 2. perTrip 500 → 125 feathers / 500gp → 0 coins (<100 bank);
/// 675 remain for resume. Rods baseline 5 stock out with 475 leftover — not
/// selected. Hut↔teller is Chebyshev 18.
const SHOP_BUYOUT_FERNAHEI_PER_TRIP_GP: f64 = 500.0;
const SHOP_BUYOUT_FERNAHEI_BUDGET_GP: f64 = 1500.0;
/// Betty's frozen Falador West preset requires four travel legs before a
/// resumed purchase: shop→bank→shop for the initial withdrawal, then
/// shop→bank→shop for deposit and return. The 150-dirty first-purchase
/// watch expired while the player was still progressing back with 500 coins.
///
/// These are bounded trial estimates, not measured completion times.
/// Dirty-snapshot increments are distinct from engine ticks and wall time.
/// The failed trace had 102 engine ticks through initial withdrawal and
/// 28 on the return. Its remaining 108-tile Chebyshev distance requires at
/// least 54 ticks at two tiles/tick; later energy depletion and detours
/// can increase that. It does not establish a 136-tick return lower bound.
///
/// First purchase gets 320 dirties; later deposit/return get 280 each to
/// allow for depleted energy. At the observed roughly 1.67 dirties/second,
/// a 320-dirty first purchase is about 190s. Assuming later legs complete
/// near 120 dirties each, plus setup/actions, gives roughly 365s; 420s is
/// the wall trial bound. Those future durations remain unmeasured.
/// Empty/close/further-purchase watches, other presets, runtime timeouts,
/// and global guards retain their existing limits. Falador West is preserved.
pub(crate) const SHOP_BUYOUT_BETTY_FIRST_PURCHASE_WATCH_TICKS: u32 = 320;
pub(crate) const SHOP_BUYOUT_BETTY_DEPOSIT_WATCH_TICKS: u32 = 280;
pub(crate) const SHOP_BUYOUT_BETTY_RETURN_WATCH_TICKS: u32 = 280;
pub(crate) const SHOP_BUYOUT_BETTY_DEADLINE: Duration = Duration::from_secs(420);

/// Gerrant's frozen Draynor preset needs the same four travel legs as other
/// shop buyouts (initial shop→bank→shop withdrawal, post-buy shop→bank deposit,
/// bank→shop return, resumed buy).
///
/// **Units:** per-arm `budget_ticks` are runner dirty-snapshot increments
/// (`SCRIPT_GOLD_WATCH_TICKS` docs); whole-cycle `deadline` is wall seconds.
///
/// **Route (geometry, not measured completion):** Chebyshev Gerrant stand
/// 3013,3224 ↔ Draynor approach 3092,3243 is 79 tiles → 40 engine ticks at
/// two tiles/tick on a straight leg. Nav traces use north-loop detours (7–8
/// hops per leg) and one-tile/tick once run energy drops.
///
/// **Four legs vs arms:**
/// - First-purchase watch: shop→bank (leg 1), withdraw/open, bank→shop (leg 2),
///   Trade + buy until pack holds feather — two travel legs plus banking/shop UI.
/// - Deposit watch: shop→bank (leg 3) until product in bank — one travel leg.
/// - Return watch: bank→shop keeper radius (leg 4) — one travel leg, often
///   depleted energy (same geometry as leg 2/3).
/// - Empty/close/further-purchase arms stay ordinary 150-dirty gold watches.
///
/// **Measured live7zpi0g_0 (180s wall, host 0a80e7535):** first purchase
/// 160 feathers / 488gp, deposit, empty, close, 488gp withdraw under 150-dirty
/// deposit arm; whole scenario deadline 180s exceeded on step 21 return at
/// 3075,3264 run energy 6 (315 runner increments; capture
/// `2026-09-18T11-53-30_shop_buyout_gerrant`). Establishes mid-return fail and
/// deposit pass, not a first-purchase upper bound under hitch variance.
///
/// **Measured livexjav5j_0 (105.046s, host 0c14892f5):** step 17 first-purchase
/// watch exhausted 150 dirty at 168 runner increments with 500 coins, no
/// feather, run energy 20; nav arrived 3015,3222 engine tick 165 then Trade
/// Gerrant before timeout (capture `2026-09-18T12-19-09_shop_buyout_gerrant`).
/// Proves 150 insufficient for the two-leg + withdraw + buy arm under scene
/// reload/hitch variance — not a completed first-purchase duration.
///
/// **First-purchase dirty 240:** observed arm exhausted 150 dirty while the
/// whole runner reported 168 increments (including setup). The additional
/// 90 arm increments are a bounded trial margin for remaining Trade/buy work
/// and route variation; they are not a measured UI or hitch duration.
///
/// **Deposit dirty 150:** live7zpi0g measured pass for leg 3; one depleted
/// leg is shorter than the first-purchase composite — no widening without a
/// deposit-arm fail.
///
/// **Return dirty 240:** bounded trial above depleted leg-4 lower bound plus
/// detour margin from live7zpi0g mid-return geometry (~62 Chebyshev tiles
/// remaining at 180s wall); not Betty's 280.
///
/// **Whole-cycle wall 390s:** live7zpi0g 180s through mid-return plus ~210s
/// trial remainder for depleted return and resumed buy; retained unless LIVE
/// shows deadline fail with widened first-purchase dirty budget.
pub(crate) const SHOP_BUYOUT_GERRANT_FIRST_PURCHASE_WATCH_TICKS: u32 = 240;
pub(crate) const SHOP_BUYOUT_GERRANT_RETURN_WATCH_TICKS: u32 = 240;
pub(crate) const SHOP_BUYOUT_GERRANT_DEADLINE: Duration = Duration::from_secs(390);

/// Per-case observation budgets for [`shop_buyout_variant`]. Default matches
/// ordinary gold watches; Betty widens only the long bank-travel arms.
struct ShopBuyoutTiming {
    first_purchase_ticks: u32,
    deposit_ticks: u32,
    return_ticks: u32,
    deadline: Duration,
}

const SHOP_BUYOUT_DEFAULT_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SCRIPT_GOLD_WATCH_TICKS,
    deposit_ticks: SCRIPT_GOLD_WATCH_TICKS,
    return_ticks: SCRIPT_GOLD_WATCH_TICKS,
    deadline: SCRIPT_GOLD_DEADLINE,
};

const SHOP_BUYOUT_BETTY_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SHOP_BUYOUT_BETTY_FIRST_PURCHASE_WATCH_TICKS,
    deposit_ticks: SHOP_BUYOUT_BETTY_DEPOSIT_WATCH_TICKS,
    return_ticks: SHOP_BUYOUT_BETTY_RETURN_WATCH_TICKS,
    deadline: SHOP_BUYOUT_BETTY_DEADLINE,
};

const SHOP_BUYOUT_GERRANT_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    first_purchase_ticks: SHOP_BUYOUT_GERRANT_FIRST_PURCHASE_WATCH_TICKS,
    deposit_ticks: SCRIPT_GOLD_WATCH_TICKS,
    return_ticks: SHOP_BUYOUT_GERRANT_RETURN_WATCH_TICKS,
    deadline: SHOP_BUYOUT_GERRANT_DEADLINE,
};

/// Bob Lumbridge ↔ Draynor: Chebyshev stand 3231,3203 ↔ Draynor approach
/// 3092,3243 is 139 tiles (estimate, not measured) — wider than Gerrant's 79.
/// Reuses Betty's four-leg trial dirty budgets and 420s wall until LIVE
/// measures Bob-specific variance.
const SHOP_BUYOUT_BOB_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_BETTY_TIMING;

/// Nurmof livewfnjs8_0 reached the return Trade but hit the 420s deadline
/// before a fresh purchase. Summed host windows put those Trade sends at
/// 406.586/410.124/413.621s and termination at 415.941s (approximate process
/// timing, not an atomic shop-open witness). Native open waits 3s per attempt.
/// Allow one bounded 30s observation margin for the remaining open/buy work;
/// this is a trial allowance, not proof that timing is the only defect.
/// Keep every dirty budget and the fresh post-bank purchase proof unchanged.
const SHOP_BUYOUT_NURMOF_TIMING: ShopBuyoutTiming = ShopBuyoutTiming {
    deadline: Duration::from_secs(450),
    ..SHOP_BUYOUT_BETTY_TIMING
};

/// Lundail cellar shop 2535,4719 ↔ Gundai stand 2533,4714 is Chebyshev 5
/// (estimate, not measured). Four legs stay inside the Mage Arena cellar —
/// not the wilderness lever/web approach. Ordinary gold 150/180s, same
/// family as Aemad/Aubury short booth routes.
const SHOP_BUYOUT_LUNDAIL_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_DEFAULT_TIMING;

/// Fernahei hut 2870,2971 ↔ Shilo teller 2852,2954 is Chebyshev 18
/// (estimate, not measured), shorter than Aemad's ~42-tile East Ardougne
/// booth route. Ordinary gold 150/180s. Village wooden gates are not on
/// this interior shop↔bank geometry.
const SHOP_BUYOUT_FERNAHEI_TIMING: ShopBuyoutTiming = SHOP_BUYOUT_DEFAULT_TIMING;

pub(crate) const SHOP_BUYOUT_AEMAD_LABEL: &str =
    "Aemad's vials — East Ardougne (Ardougne East bank)";
pub(crate) const SHOP_BUYOUT_AUBURY_LABEL: &str = "Aubury's runes — Varrock (Varrock East bank)";
pub(crate) const SHOP_BUYOUT_LOWE_LABEL: &str = "Lowe's arrows — Varrock (Varrock East bank)";
pub(crate) const SHOP_BUYOUT_HICKTON_LABEL: &str = "Hickton's arrows — Catherby (Catherby bank)";
pub(crate) const SHOP_BUYOUT_HARRY_LABEL: &str = "Harry's fishing — Catherby (Catherby bank)";
pub(crate) const SHOP_BUYOUT_BETTY_LABEL: &str = "Betty's runes — Port Sarim (Falador West bank)";
pub(crate) const SHOP_BUYOUT_GERRANT_LABEL: &str = "Gerrant's feathers — Port Sarim (Draynor bank)";
pub(crate) const SHOP_BUYOUT_BOB_LABEL: &str = "Bob's axes — Lumbridge (Draynor bank)";
pub(crate) const SHOP_BUYOUT_NURMOF_LABEL: &str =
    "Nurmof's pickaxes — Dwarven Mine (Falador East bank)";
pub(crate) const SHOP_BUYOUT_MAGIC_LABEL: &str = "Wizard Guild runes — Yanille (Yanille bank)";
pub(crate) const SHOP_BUYOUT_LUNDAIL_LABEL: &str = "Mage Arena runes — Lundail (Gundai bank)";
pub(crate) const SHOP_BUYOUT_FERNAHEI_LABEL: &str =
    "Fernahei's fishing — Shilo Village (Shilo bank)";
pub(crate) const SHOP_BUYOUT_AEMAD_ITEM: &str = "Vial of water";
pub(crate) const SHOP_BUYOUT_AUBURY_ITEM: &str = "Fire rune";
pub(crate) const SHOP_BUYOUT_LOWE_ITEM: &str = "Bronze arrow";
pub(crate) const SHOP_BUYOUT_HICKTON_ITEM: &str = "Bronze arrow";
pub(crate) const SHOP_BUYOUT_HARRY_ITEM: &str = "Fishing bait";
pub(crate) const SHOP_BUYOUT_BETTY_ITEM: &str = "Fire rune";
pub(crate) const SHOP_BUYOUT_GERRANT_ITEM: &str = "Feather";
pub(crate) const SHOP_BUYOUT_BOB_ITEM: &str = "Steel axe";
pub(crate) const SHOP_BUYOUT_NURMOF_ITEM: &str = "Iron pickaxe";
pub(crate) const SHOP_BUYOUT_MAGIC_ITEM: &str = "Blood rune";
pub(crate) const SHOP_BUYOUT_LUNDAIL_ITEM: &str = "Fire rune";
pub(crate) const SHOP_BUYOUT_FERNAHEI_ITEM: &str = "Feather";
pub(crate) const FISHING_BAIT_ID: i32 = 313;
pub(crate) const STEEL_AXE_ID: i32 = 1353;
pub(crate) const IRON_PICKAXE_ID: i32 = 1267;

const AIO_TELEPORT_INJECT: &[ScriptSettingInject] = &[];
const AIO_TELEPORT_FALADOR_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "teleportName",
    value: ScriptInjectValue::Str("falador"),
}];
const AIO_TELEPORT_NO_STAFF_INJECT: &[ScriptSettingInject] = &[ScriptSettingInject {
    id: "useStaffRunes",
    value: ScriptInjectValue::Bool(false),
}];
const SHOP_BUYOUT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_AEMAD_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AEMAD_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AEMAD_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_AEMAD_ITEM]),
    },
];
const SHOP_BUYOUT_AUBURY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_AUBURY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AUBURY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_AUBURY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_AUBURY_ITEM]),
    },
];
const SHOP_BUYOUT_LOWE_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_LOWE_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LOWE_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LOWE_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_LOWE_ITEM]),
    },
];
const SHOP_BUYOUT_HICKTON_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_HICKTON_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HICKTON_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HICKTON_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_HICKTON_ITEM]),
    },
];
const SHOP_BUYOUT_HARRY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_HARRY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HARRY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_HARRY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_HARRY_ITEM]),
    },
];
const SHOP_BUYOUT_BETTY_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_BETTY_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BETTY_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BETTY_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_BETTY_ITEM]),
    },
];
const SHOP_BUYOUT_GERRANT_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_GERRANT_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_GERRANT_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_GERRANT_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_GERRANT_ITEM]),
    },
];
const SHOP_BUYOUT_BOB_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_BOB_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BOB_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_BOB_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_BOB_ITEM]),
    },
];
const SHOP_BUYOUT_NURMOF_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_NURMOF_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_NURMOF_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_NURMOF_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_NURMOF_ITEM]),
    },
];
const SHOP_BUYOUT_MAGIC_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_MAGIC_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_MAGIC_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_MAGIC_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_MAGIC_ITEM]),
    },
];
const SHOP_BUYOUT_LUNDAIL_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_LUNDAIL_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LUNDAIL_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_LUNDAIL_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_LUNDAIL_ITEM]),
    },
];
const SHOP_BUYOUT_FERNAHEI_INJECT: &[ScriptSettingInject] = &[
    ScriptSettingInject {
        id: "shop",
        value: ScriptInjectValue::Str(SHOP_BUYOUT_FERNAHEI_LABEL),
    },
    ScriptSettingInject {
        id: "budgetGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_FERNAHEI_BUDGET_GP),
    },
    ScriptSettingInject {
        id: "perTripGp",
        value: ScriptInjectValue::Num(SHOP_BUYOUT_FERNAHEI_PER_TRIP_GP),
    },
    ScriptSettingInject {
        id: "stopFloorGp",
        value: ScriptInjectValue::Num(0.0),
    },
    ScriptSettingInject {
        id: "buyItems",
        value: ScriptInjectValue::StrList(&[SHOP_BUYOUT_FERNAHEI_ITEM]),
    },
];
struct AioTeleportPlan {
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    magic: i32,
    staff_id: Option<i32>,
    staff_alias: Option<&'static str>,
    pack_air: bool,
    pack_fire: bool,
    landing: WorldTile,
    restock: WorldTile,
}

pub(crate) fn aio_teleport_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport",
        inject: AIO_TELEPORT_INJECT,
        magic: 25,
        staff_id: Some(STAFF_OF_AIR_ID),
        staff_alias: Some("staff_of_air"),
        pack_air: false,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

pub(crate) fn aio_teleport_falador_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_falador",
        inject: AIO_TELEPORT_FALADOR_INJECT,
        magic: 37,
        staff_id: Some(STAFF_OF_WATER_ID),
        staff_alias: Some("staff_of_water"),
        pack_air: true,
        pack_fire: false,
        landing: FALADOR_TELE_LAND,
        restock: FALADOR_WEST_BANK,
    })
}

pub(crate) fn aio_teleport_no_staff_scenario() -> Scenario {
    aio_teleport_variant(AioTeleportPlan {
        name: "aio_teleport_no_staff",
        inject: AIO_TELEPORT_NO_STAFF_INJECT,
        magic: 25,
        staff_id: None,
        staff_alias: None,
        pack_air: true,
        pack_fire: true,
        landing: VARROCK_TELE_LAND,
        restock: VARROCK_EAST_BANK,
    })
}

/// Pack two laws so the default 1000-law withdraw never runs inside 180s.
/// Falador also packs Air: water staff covers Water, not Air.
fn aio_teleport_variant(plan: AioTeleportPlan) -> Scenario {
    let AioTeleportPlan {
        name,
        inject,
        magic,
        staff_id,
        staff_alias,
        pack_air,
        pack_fire,
        landing,
        restock,
    } = plan;
    let first_xp = Proof::StatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let further_xp = Proof::FreshStatXpGain {
        id: MAGIC_STAT,
        min: 1,
    };
    let land = Proof::ArrivedNear {
        x: landing.x,
        z: landing.z,
        level: landing.level,
        radius: 8,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed Magic, packed laws, and Lumbridge bank before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("setstat magic {magic}"));
                cheat(c, "give lawrune 2");
                if pack_air {
                    cheat(c, "give airrune 20");
                }
                if pack_fire {
                    cheat(c, "give firerune 20");
                }
                if let Some(alias) = staff_alias {
                    cheat(c, &format!("give {alias} 1"));
                }
                cheat(c, "givebank lawrune 200");
                cheat(
                    c,
                    &tele_args(LUMBRIDGE_BANK.level, LUMBRIDGE_BANK.x, LUMBRIDGE_BANK.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: LUMBRIDGE_BANK.x,
                z: LUMBRIDGE_BANK.z,
                level: LUMBRIDGE_BANK.level,
                radius: 8,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "confirm Magic before Start",
        Proof::Stat {
            id: MAGIC_STAT,
            min: magic,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm packed laws before Start",
        Proof::ItemId {
            id: LAW_RUNE_ID,
            count: 2,
        },
    ));
    if let Some(id) = staff_id {
        steps.push(wear_combat_item_step(
            "wield and acknowledge the covering staff before Start",
            id,
        ));
    } else {
        steps.push(bank_fletcher_watch(
            "confirm no covering air staff before Start",
            Proof::ItemIdAtMost {
                id: STAFF_OF_AIR_ID,
                count: 0,
            },
        ));
    }
    steps.push(tanner_open_seed_bank(
        "open and acknowledge the banked law restock",
        Proof::BankItemId {
            id: LAW_RUNE_ID,
            count: 200,
        },
    ));
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(start_catalog_step());
    for (step_name, arm) in [
        ("watch Magic XP from Game.teleport after Start", first_xp),
        (
            "watch arrival at the selected teleport land after Start",
            land,
        ),
        (
            "watch a packed law consumed after Start",
            Proof::ItemIdAtMost {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        (
            "watch law restock at the destination bank after Start",
            Proof::BankItemId {
                id: LAW_RUNE_ID,
                count: 1,
            },
        ),
        ("watch the teleport bank close", Proof::BankClosed),
        ("watch a further Magic XP after restock", further_xp),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    let _ = restock;
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: further_xp,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: SCRIPT_GOLD_DEADLINE,
            start_script: Some("AIO Teleport"),
            script_settings_inject: if inject.is_empty() {
                None
            } else {
                Some(inject)
            },
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}

/// Aemad nonstackable ShopBuyout: selected `Vial of water` (obj 227) only.
/// Coin seed via ordinary booth deposit; product unseeded. Natural bank on
/// pack-full needSpaceFor; budget>perTrip permits resumed purchase.
pub(crate) fn shop_buyout_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout",
        SHOP_BUYOUT_INJECT,
        AEMAD_STAND,
        AEMAD_BANK_APPROACH,
        AEMAD_BANK_BOOTH,
        AEMAD_BANK_BOOTH_ID,
        "Aemad",
        VIAL_OF_WATER_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Aubury stackable ShopBuyout: selected `Fire rune` (obj 554) only. Pack
/// fill does not bank an existing rune stack; natural bank is coins<100
/// after a real buy with remaining budget for withdraw+resume.
pub(crate) fn shop_buyout_aubury_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_aubury",
        SHOP_BUYOUT_AUBURY_INJECT,
        AUBURY_STAND,
        VARROCK_EAST_BANK_APPROACH,
        VARROCK_EAST_BANK_BOOTH,
        VARROCK_EAST_BANK_BOOTH_ID,
        "Aubury",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Lowe stackable ShopBuyout: selected `Bronze arrow` (obj 882) only. Same
/// Varrock East bank geometry as Aubury; coins<100 after real buy drives bank.
pub(crate) fn shop_buyout_lowe_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_lowe",
        SHOP_BUYOUT_LOWE_INJECT,
        LOWE_STAND,
        VARROCK_EAST_BANK_APPROACH,
        VARROCK_EAST_BANK_BOOTH,
        VARROCK_EAST_BANK_BOOTH_ID,
        "Lowe",
        BRONZE_ARROW_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Hickton stackable ShopBuyout: selected `Bronze arrow` (obj 882) only.
/// Catherby open booth 2213@2809,3442 with south approach 2809,3441.
pub(crate) fn shop_buyout_hickton_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_hickton",
        SHOP_BUYOUT_HICKTON_INJECT,
        HICKTON_STAND,
        CATHERBY_BANK_APPROACH,
        CATHERBY_BANK_BOOTH,
        CATHERBY_BANK_BOOTH_ID,
        "Hickton",
        BRONZE_ARROW_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Harry stackable ShopBuyout: selected `Fishing bait` (obj 313) only. Same
/// Catherby bank geometry as Hickton.
pub(crate) fn shop_buyout_harry_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_harry",
        SHOP_BUYOUT_HARRY_INJECT,
        HARRY_STAND,
        CATHERBY_BANK_APPROACH,
        CATHERBY_BANK_BOOTH,
        CATHERBY_BANK_BOOTH_ID,
        "Harry",
        FISHING_BAIT_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    )
}

/// Betty stackable ShopBuyout: selected `Fire rune` (obj 554) only. Falador
/// West open booth 2213@2946,3367 with approach 2946,3368 (preset bankStand
/// 2946,3369 is Chebyshev 2). coins<100 after real buy drives bank.
/// Long Port Sarim↔Falador West travel uses [`SHOP_BUYOUT_BETTY_TIMING`].
pub(crate) fn shop_buyout_betty_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_betty",
        SHOP_BUYOUT_BETTY_INJECT,
        BETTY_STAND,
        FALADOR_WEST_BANK_APPROACH,
        FALADOR_WEST_BANK_BOOTH,
        FALADOR_WEST_BANK_BOOTH_ID,
        "Betty",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_BETTY_TIMING,
    )
}

/// Gerrant stackable ShopBuyout: selected `Feather` (obj 314) only. Reuses
/// Draynor open booth 2213@3091,3243 with approach 3092,3243 (frozen bankStand).
pub(crate) fn shop_buyout_gerrant_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_gerrant",
        SHOP_BUYOUT_GERRANT_INJECT,
        GERRANT_STAND,
        DRAYNOR_BANK_APPROACH,
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
        "Gerrant",
        FEATHER_ID,
        SHOP_BUYOUT_GERRANT_TIMING,
    )
}

/// Bob nonstackable ShopBuyout: selected `Steel axe` (obj 1353) only.
/// Long Lumbridge ↔ Draynor route uses [`SHOP_BUYOUT_BOB_TIMING`].
pub(crate) fn shop_buyout_bob_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_bob",
        SHOP_BUYOUT_BOB_INJECT,
        BOB_STAND,
        DRAYNOR_BANK_APPROACH,
        DRAYNOR_BANK_BOOTH,
        DRAYNOR_BANK_BOOTH_ID,
        "Bob",
        STEEL_AXE_ID,
        SHOP_BUYOUT_BOB_TIMING,
    )
}

/// Nurmof nonstackable ShopBuyout: selected `Iron pickaxe` (obj 1267) only.
/// Dwarven Mine ↔ Falador East uses [`SHOP_BUYOUT_NURMOF_TIMING`].
pub(crate) fn shop_buyout_nurmof_scenario() -> Scenario {
    shop_buyout_variant(
        "shop_buyout_nurmof",
        SHOP_BUYOUT_NURMOF_INJECT,
        NURMOF_STAND,
        FALADOR_EAST_BANK,
        FALADOR_EAST_BOOTH,
        2213,
        "Nurmof",
        IRON_PICKAXE_ID,
        SHOP_BUYOUT_NURMOF_TIMING,
    )
}

/// Magic Store owner stackable ShopBuyout: selected `Blood rune` (obj 565) only.
/// Guild floor-1 shop with Yanille booth seed; pre-Start magic 66 for return
/// legs through the guild door (`magic_guild.rs2`).
pub(crate) fn shop_buyout_magic_scenario() -> Scenario {
    let mut scenario = shop_buyout_variant(
        "shop_buyout_magic",
        SHOP_BUYOUT_MAGIC_INJECT,
        MAGIC_STORE_STAND,
        YANILLE_BANK_APPROACH,
        YANILLE_BANK_BOOTH,
        YANILLE_BANK_BOOTH_ID,
        "Magic Store owner",
        BLOOD_RUNE_ID,
        SHOP_BUYOUT_DEFAULT_TIMING,
    );
    let tele_shop = scenario
        .steps
        .iter()
        .position(|step| step.name == "tele to the original shop keeper before Start")
        .expect("shop buyout seed tele to shop");
    scenario.steps.insert(
        tele_shop,
        Step {
            name: "seed magic 66 for Wizard Guild door on post-bank return legs",
            kind: StepKind::Perform {
                send: Box::new(|c, _| {
                    cheat(c, "setstat magic 66");
                    true
                }),
            },
            wait: Wait {
                arm: Proof::Stat {
                    id: MAGIC_STAT,
                    min: 66,
                },
                budget_ticks: 200,
            },
        },
    );
    scenario.steps.insert(tele_shop + 1, drain_advancestat());
    scenario
}

/// Lundail stackable ShopBuyout: selected `Fire rune` (obj 554) only.
/// Coin seed through Gundai Talk-to + exact cellar bank choice. Shop↔bank
/// stays in the Mage Arena cellar (Chebyshev 5); wilderness lever/webs are
/// not on the proof cycle.
pub(crate) fn shop_buyout_lundail_scenario() -> Scenario {
    shop_buyout_with_bank(
        "shop_buyout_lundail",
        SHOP_BUYOUT_LUNDAIL_INJECT,
        LUNDAIL_STAND,
        ShopBuyoutBankAccess::Npc {
            approach: GUNDAI_BANK_APPROACH,
            banker: "Gundai",
            action: "Talk-to",
            choose: "Cool, I'd like to access my bank account please.",
        },
        "Lundail",
        FIRE_RUNE_ID,
        SHOP_BUYOUT_LUNDAIL_TIMING,
    )
}

/// Fernahei stackable ShopBuyout: selected `Feather` (obj 314) only.
/// Coin seed through the Shilo `Banker` Talk-to + exact teller choice.
/// Village membership is the native Shilo Village journal acknowledgement
/// before the coin seed; shop↔bank is interior Chebyshev 18.
pub(crate) fn shop_buyout_fernahei_scenario() -> Scenario {
    let mut scenario = shop_buyout_with_bank(
        "shop_buyout_fernahei",
        SHOP_BUYOUT_FERNAHEI_INJECT,
        FERNAHEI_STAND,
        ShopBuyoutBankAccess::Npc {
            approach: SHILO_BANK_APPROACH,
            banker: "Banker",
            action: "Talk-to",
            choose: "I'd like to access my bank account, please.",
        },
        "Fernahei",
        FEATHER_ID,
        SHOP_BUYOUT_FERNAHEI_TIMING,
    );
    let coin = scenario
        .steps
        .iter()
        .position(|step| {
            step.name
                == "seed stackable coins and stand at the operable shop bank approach before Start"
        })
        .expect("shop buyout coin seed");
    for step in quest_prereq_steps(SHILO_VILLAGE_PREREQ).into_iter().rev() {
        scenario.steps.insert(coin, step);
    }
    scenario
}

/// Seed-bank identity for [`shop_buyout_with_bank`]. Booth fixtures keep the
/// exact Use-quickly loc path; NPC fixtures Talk-to a named teller and
/// press that teller's authentic bank choice. Deposit/close stay shared.
enum ShopBuyoutBankAccess {
    Booth {
        approach: WorldTile,
        booth: WorldTile,
        booth_id: i32,
    },
    Npc {
        approach: WorldTile,
        banker: &'static str,
        action: &'static str,
        choose: &'static str,
    },
}

#[allow(clippy::too_many_arguments)] // scenario factory bundles inject/stand/shop timing
fn shop_buyout_variant(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    stand: WorldTile,
    bank_approach: WorldTile,
    booth: WorldTile,
    booth_id: i32,
    keeper_name: &'static str,
    product_id: i32,
    timing: ShopBuyoutTiming,
) -> Scenario {
    shop_buyout_with_bank(
        name,
        inject,
        stand,
        ShopBuyoutBankAccess::Booth {
            approach: bank_approach,
            booth,
            booth_id,
        },
        keeper_name,
        product_id,
        timing,
    )
}

#[derive(Default)]
struct ShopBuyoutNpcOpenState {
    talked: bool,
    saw_chat: bool,
    continued: Option<(i32, i32, String)>,
    answered: Option<(i32, String)>,
}

pub(crate) fn shop_buyout_open_npc_bank(
    name: &'static str,
    arm: Proof,
    banker: &'static str,
    approach: WorldTile,
    action: &'static str,
    choose: &'static str,
) -> Step {
    let state = std::sync::Mutex::new(ShopBuyoutNpcOpenState::default());
    Step {
        name,
        kind: StepKind::Repeat {
            send: Box::new(move |c, snapshot| {
                let Ok(mut state) = state.lock() else {
                    return false;
                };
                if snapshot.bank_component_id() >= 0 {
                    return true;
                }
                let chat_open = snapshot.modals().chat != -1;
                if chat_open {
                    state.saw_chat = true;
                }
                if snapshot.chat_continue_component_id() != -1 {
                    let identity = (
                        snapshot.modals().chat,
                        snapshot.chat_continue_component_id(),
                        snapshot.chat_modal_texts().join("\n"),
                    );
                    if state.continued.as_ref() == Some(&identity) {
                        return true;
                    }
                    let mut ix = Interactions::new(snapshot, c);
                    return match ix.continue_dialog() {
                        SendResult::Sent { .. } => {
                            state.continued = Some(identity);
                            true
                        }
                        SendResult::Refused { .. } => true,
                    };
                }
                if !snapshot.chat_options().is_empty() {
                    let identity = (
                        snapshot.modals().chat,
                        snapshot
                            .chat_options()
                            .iter()
                            .map(|option| option.text.as_str())
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    if state.answered.as_ref() == Some(&identity) {
                        return true;
                    }
                    let Some(choice) = snapshot
                        .chat_options()
                        .iter()
                        .position(|option| option.text == choose)
                    else {
                        return false;
                    };
                    let mut ix = Interactions::new(snapshot, c);
                    return match ix.answer_choice(choice as i32 + 1) {
                        SendResult::Sent { .. } => {
                            state.answered = Some(identity);
                            true
                        }
                        SendResult::Refused { .. } => true,
                    };
                }
                if chat_open || (state.talked && !state.saw_chat) || state.answered.is_some() {
                    return true;
                }
                if state.talked && state.saw_chat {
                    *state = ShopBuyoutNpcOpenState::default();
                }
                let Some(npc) = snapshot.npcs().iter().find(|npc| {
                    npc.name.as_deref() == Some(banker)
                        && npc.tile.level == approach.level
                        && (npc.tile.x - approach.x)
                            .abs()
                            .max((npc.tile.z - approach.z).abs())
                            <= 12
                        && npc.actions.iter().any(|published| {
                            published
                                .as_deref()
                                .is_some_and(|label| label.eq_ignore_ascii_case(action))
                        })
                }) else {
                    return true;
                };
                let Some((px, pz, level)) = snapshot.tile() else {
                    return true;
                };
                if level != npc.tile.level
                    || (px - npc.tile.x).abs().max((pz - npc.tile.z).abs()) > 1
                {
                    let mut ix = Interactions::new(snapshot, c);
                    return matches!(
                        ix.walk(npc.tile),
                        SendResult::Sent { .. } | SendResult::Refused { .. }
                    );
                }
                let mut ix = Interactions::new(snapshot, c);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label(action.to_string())) {
                    SendResult::Sent { .. } => {
                        state.talked = true;
                        true
                    }
                    SendResult::Refused {
                        reason:
                            SendReason::SceneUnavailable
                            | SendReason::OffScene
                            | SendReason::StaleTarget,
                        ..
                    } => true,
                    SendResult::Refused { reason, .. } => {
                        eprintln!("[scenario] npc bank {banker} {action} send refused: {reason:?}");
                        false
                    }
                }
            }),
        },
        wait: Wait {
            arm,
            budget_ticks: SCRIPT_GOLD_WATCH_TICKS,
        },
    }
}

fn shop_buyout_with_bank(
    name: &'static str,
    inject: &'static [ScriptSettingInject],
    stand: WorldTile,
    bank: ShopBuyoutBankAccess,
    keeper_name: &'static str,
    product_id: i32,
    timing: ShopBuyoutTiming,
) -> Scenario {
    let product = Proof::ItemId {
        id: product_id,
        count: 1,
    };
    let coin_seed = SHOP_BUYOUT_COIN_SEED;
    let bank_approach = match bank {
        ShopBuyoutBankAccess::Booth { approach, .. }
        | ShopBuyoutBankAccess::Npc { approach, .. } => approach,
    };
    let mut steps = script_live_seed_steps();
    steps.push(Step {
        name: "seed stackable coins and stand at the operable shop bank approach before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, "~clearinv");
                cheat(c, &format!("give coins {coin_seed}"));
                cheat(
                    c,
                    &tele_args(bank_approach.level, bank_approach.x, bank_approach.z),
                );
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: bank_approach.x,
                z: bank_approach.z,
                level: bank_approach.level,
                radius: 4,
            },
            budget_ticks: 200,
        },
    });
    match bank {
        ShopBuyoutBankAccess::Booth {
            booth, booth_id, ..
        } => {
            steps.push(bank_fletcher_watch(
                "acknowledge exact shop bank booth identity and Use-quickly action before bank send",
                Proof::LocActionNear {
                    id: booth_id,
                    x: booth.x,
                    z: booth.z,
                    level: booth.level,
                    radius: 0,
                    action: "Use-quickly",
                    present: true,
                },
            ));
            // Same exact-booth open pattern as Herblore readiness (do not retarget
            // closed booths via open_nearest). Reuses the generic helper only.
            steps.push(herblore_open_seed_bank_at(
                "open the exact named shop bank booth for the coin seed deposit",
                Proof::BankItemIdAtMost {
                    id: COINS_ID,
                    count: 0,
                },
                booth,
                booth_id,
            ));
        }
        ShopBuyoutBankAccess::Npc {
            banker,
            action,
            choose,
            ..
        } => {
            steps.push(bank_fletcher_watch(
                "acknowledge the exact named shop banker and Talk-to action before bank send",
                Proof::NpcNameNear {
                    name: banker,
                    x: bank_approach.x,
                    z: bank_approach.z,
                    level: bank_approach.level,
                    radius: 12,
                },
            ));
            steps.push(shop_buyout_open_npc_bank(
                "open the exact named shop banker for the coin seed deposit",
                Proof::BankItemIdAtMost {
                    id: COINS_ID,
                    count: 0,
                },
                banker,
                bank_approach,
                action,
                choose,
            ));
        }
    }
    steps.extend(native_bank_deposit(
        "deposit the coin seed through the bank window",
        vec![NativeSeed {
            unnoted_id: COINS_ID,
            debug_alias: "coins",
            note_alias: None,
            quantity: coin_seed,
            note_id: None,
        }],
    ));
    for (step_name, arm) in [
        (
            "confirm the named bank holds the exact coin seed",
            Proof::BankItemId {
                id: COINS_ID,
                count: coin_seed,
            },
        ),
        (
            "confirm the named bank has no excess coin seed",
            Proof::BankItemIdAtMost {
                id: COINS_ID,
                count: coin_seed,
            },
        ),
        (
            "confirm no seeded product in bank before Start",
            Proof::BankItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "confirm the coin seed left the pack",
            Proof::ItemIdAtMost {
                id: COINS_ID,
                count: 0,
            },
        ),
        (
            "confirm no seeded product in pack before Start",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
    ] {
        steps.push(bank_fletcher_watch(step_name, arm));
    }
    steps.push(bank_fletcher_close_seed_bank());
    steps.push(Step {
        name: "tele to the original shop keeper before Start",
        kind: StepKind::Perform {
            send: Box::new(move |c, _| {
                cheat(c, &tele_args(stand.level, stand.x, stand.z));
                true
            }),
        },
        wait: Wait {
            arm: Proof::ArrivedNear {
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 6,
            },
            budget_ticks: 200,
        },
    });
    steps.push(bank_fletcher_watch(
        "acknowledge the original shop keeper before Start",
        Proof::NpcNameNear {
            name: keeper_name,
            x: stand.x,
            z: stand.z,
            level: stand.level,
            radius: 12,
        },
    ));
    steps.push(bank_fletcher_watch(
        "confirm pack still empty of product at the shop before Start",
        Proof::ItemIdAtMost {
            id: product_id,
            count: 0,
        },
    ));
    steps.push(start_catalog_step());
    // Contract: unseeded product → fresh bank product → empty pack →
    // BankClosed → return named shop → new product. Coins secondary only.
    // Travel-leg dirty budgets come from `timing` (Betty long West route);
    // empty/close/further stay ordinary gold watches. See
    // SHOP_BUYOUT_BETTY_* constant docs (dirty increments, not 150×600ms).
    for (step_name, arm) in [
        (
            "watch unseeded purchased product in pack after Start",
            product,
        ),
        (
            "watch purchased product enter a fresh bank",
            Proof::BankItemId {
                id: product_id,
                count: 1,
            },
        ),
        (
            "watch the pack empty of product after deposit",
            Proof::ItemIdAtMost {
                id: product_id,
                count: 0,
            },
        ),
        (
            "watch the buyout bank close after deposit",
            Proof::BankClosed,
        ),
        (
            "watch return to the named shop after banking",
            Proof::NpcNameNear {
                name: keeper_name,
                x: stand.x,
                z: stand.z,
                level: stand.level,
                radius: 12,
            },
        ),
        ("watch further purchased product after return", product),
    ] {
        let budget_ticks = match step_name {
            "watch unseeded purchased product in pack after Start" => timing.first_purchase_ticks,
            "watch purchased product enter a fresh bank" => timing.deposit_ticks,
            "watch return to the named shop after banking" => timing.return_ticks,
            _ => SCRIPT_GOLD_WATCH_TICKS,
        };
        steps.push(Step {
            name: step_name,
            kind: StepKind::Perform {
                send: Box::new(|_, _| true),
            },
            wait: Wait { arm, budget_ticks },
        });
    }
    Scenario {
        name,
        seed: Seed {
            profiles: vec![("test", "test")],
            mainland: true,
        },
        steps,
        proof: product,
        companions: vec![],
        settings: ScenarioSettings {
            full_rate: true,
            require_mainland_base: true,
            deadline: timing.deadline,
            start_script: Some("ShopBuyout"),
            script_settings_inject: Some(inject),
            terminal_shot: Some(name),
            nav: gold_script_nav(),
            ..Default::default()
        },
    }
}
