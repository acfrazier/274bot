pub const CORE_SCENARIOS: &str = "bone_burier|chicken_killer|chicken_killer_bank|thiever|alcher|alcher_defaults|alcher_custom|alcher_custom_alias|alcher_custom_name|alcher_ordered|alcher_large_batch|alcher_low|alcher_fire_battlestaff|alcher_swarm_drain|bank_fletcher|bank_fletcher_shafts|bank_fletcher_headless|bank_fletcher_string|bank_fletcher_cut_string|dart_fletcher|dart_fletcher_iron|herb_cleaner|herb_cleaner_named|herb_cleaner_empty_bank|gem_cutter|gem_cutter_named|door_opener|door_opener_gate|gnome_course|gnome_course_radius|wildy_agility|brimhaven_agility|flax_picker|superheater|superheater_steel|superheater_fire_battlestaff|superheater_silver_low_natures|vial_filler|vial_filler_east|potion_maker|potion_maker_named|tanner_bot|tanner_bot_hard|rune_crafter|rune_crafter_earth|mule_crafter|ardy_cakes|ardy_cakes_fight|ardy_thiever|ardy_thiever_fight|ardy_thiever_knight|gnome_chop|gnome_fletch_short|gnome_fletch_long|coal_trucks|cook_bot|cook_bot_lobster|smelter_bot|smelter_bot_steel|flax_spinner|flax_aio|flax_aio_pick|flax_aio_spin|herblore_secondaries|herblore_secondaries_newt|chaos_druid|chaos_druid_tower|chaos_druid_yanille|moss_giant|moss_giant_prepared|moss_giant_dart|hill_giant|auto_fighter|auto_fighter_mage|auto_fighter_range|rock_crab|rock_crab_range|green_dragon|green_dragon_prepared|green_dragon_mage_prepared|green_dragon_special|green_dragon_special_prepared|green_dragon_potions|green_dragon_potions_prepared|fire_giant|fire_giant_prepared|ardy_fighter|auto_fighter_bank|moss_giant_bank|hill_giant_bank|hill_giant_bank_prepared|chaos_druid_bank|ardy_fighter_bank|rock_crab_bank|green_dragon_bank|green_dragon_bank_prepared|green_dragon_bank_default_prepared|green_dragon_tele|green_dragon_tele_prepared|fire_giant_approach|fire_giant_bank|fire_giant_bank_prepared|fire_giant_camelot_prepared|aio_teleport|aio_teleport_falador|aio_teleport_no_staff|shop_buyout|shop_buyout_aubury|shop_buyout_lowe|shop_buyout_hickton|shop_buyout_harry|shop_buyout_betty|shop_buyout_gerrant|smithing_bot|smithing_bot_platebody|leather_crafter|leather_crafter_hard_body|firemaker|firemaker_oak|climbing_boots|climbing_boots_teleport|ranging_guild_round|ranging_guild_redeem|ranging_guild_bank|ranging_guild_full|brimhaven_moss_inspect_v1|route_inspect_brimhaven_v2_ts|prayer_v2_ts|prayer_v1_ts|line_of_sight_v2_ts|actor_observation_v2_ts|fight_field_v2_ts|hold_spot_v2_ts|retreat_spot_v2_ts|walk_spot_v2_ts|enter_lair_v2_ts|leave_lair_v2_ts|acquire_key_v2_ts|cell_v2_ts|bank_v2_ts";
pub const CATALOG_COMMIT_A: &str = "100adccc037d9f6898080e1cad58fcfc43364775";
pub const CATALOG_COMMIT_B: &str = "8e7d965be2071d6ec65c3265e12af797082d720a";
pub const ADAMANT_SCIMITAR_ID: i32 = 1331;
pub const CERT_ADAMANT_SCIMITAR_ID: i32 = 1332;
pub const YEW_LONGBOW_ID: i32 = 855;
pub const CERT_YEW_LONGBOW_ID: i32 = 856;
pub const NATURE_RUNE_ID: i32 = 561;
pub const MIND_RUNE_ID: i32 = 558;
pub const COINS_ID: i32 = 995;
pub const RUNE_CHAINBODY_ID: i32 = 1113;
pub const CERT_RUNE_CHAINBODY_ID: i32 = 1114;
pub const RUNE_PLATELEGS_ID: i32 = 1079;
pub const RUNE_FULL_HELM_ID: i32 = 1163;
/// High Level Alchemy pays 60% of shop cost: floor(2560 * 0.6) = 1536.
pub const ADAMANT_SCIMITAR_ALCH_COINS: i32 = 1536;
pub const YEW_LONGBOW_ALCH_COINS: i32 = 768;
pub const HIGH_ALCH_MAGIC_XP: i32 = 65;
/// Canonical 289 `magic_spells.dbrow`: Low needs 21 Magic and 31 client-scale
/// XP per cast; High needs 55 and 65. Low pays 40% of shop cost, High 60%.
pub const LOW_ALCH_LEVEL: i32 = 21;
pub const HIGH_ALCH_LEVEL: i32 = 55;
pub const LOW_ALCH_MAGIC_XP: i32 = 31;
/// High Level Alchemy pays 60% of shop cost: floor(50000 * 0.6) = 30000.
pub const RUNE_CHAINBODY_HIGH_ALCH_COINS: i32 = 30_000;
/// Low Level Alchemy pays 40% of shop cost: floor(50000 * 0.4) = 20000.
pub const RUNE_CHAINBODY_LOW_ALCH_COINS: i32 = 20_000;
/// Both selected catalogs require Attack 30 to wield a Fire battlestaff.
pub const FIRE_BATTLESTAFF_WIELD_ATTACK: i32 = 30;
/// Fire-rune staves, the default first: a spelled row proves the one it selected.
pub const ALCHER_FIRE_STAFFS: [i32; 2] = [STAFF_OF_FIRE_ID, FIRE_BATTLESTAFF_ID];
pub const LOGS_ID: i32 = 1511;
pub const ARROW_SHAFT_ID: i32 = 52;
pub const HEADLESS_ARROW_ID: i32 = 53;
pub const BRONZE_DART_TIP_ID: i32 = 819;
pub const BRONZE_DART_ID: i32 = 806;
/// Stock289 `rune_arrow`. MossGiant dart SETTINGS leave this unused on purpose.
pub const RUNE_ARROW_ID: i32 = 892;
/// Pinned `e2e/mossgiant-dart-test.ts` bank-only dart stack and food.
pub const MOSS_GIANT_DART_SUPPLY: i32 = 80;
pub const MOSS_GIANT_DART_BANK_FOOD: i32 = 15;
pub const MOSS_GIANT_DART_RANGED: i32 = 50;
pub const MOSS_GIANT_DART_FIELD_RADIUS: i32 = 12;
pub const IRON_DART_TIP_ID: i32 = 820;
pub const IRON_DART_ID: i32 = 807;
pub const FEATHER_ID: i32 = 314;
pub const UNIDENTIFIED_GUAM_ID: i32 = 199;
pub const GUAM_LEAF_ID: i32 = 249;
pub const UNIDENTIFIED_MARENTILL_ID: i32 = 201;
pub const MARRENTILL_ID: i32 = 251;
pub const UNCUT_SAPPHIRE_ID: i32 = 1623;
pub const SAPPHIRE_ID: i32 = 1607;
pub const UNCUT_OPAL_ID: i32 = 1625;
pub const CHISEL_ID: i32 = 1755;
pub const CRUSHED_GEMSTONE_ID: i32 = 1633;
pub const WOODEN_DOOR_CLOSED_ID: i32 = 1530;
pub const WOODEN_DOOR_OPEN_ID: i32 = 1531;
pub const WOODEN_GATE_CLOSED_ID: i32 = 1551;
pub const WOODEN_GATE_OPEN_ID: i32 = 1552;
pub const FLAX_ID: i32 = 1779;
pub const COPPER_ORE_ID: i32 = 436;
pub const TIN_ORE_ID: i32 = 438;
pub const IRON_ORE_ID: i32 = 440;
/// Selected 289 `obj.pack`: silver_ore=442, silver_bar=2355.
pub const SILVER_ORE_ID: i32 = 442;
pub const COAL_ID: i32 = 453;
pub const STAFF_OF_FIRE_ID: i32 = 1387;
pub const FIRE_BATTLESTAFF_ID: i32 = 1393;
pub const BRONZE_BAR_ID: i32 = 2349;
pub const IRON_BAR_ID: i32 = 2351;
pub const STEEL_BAR_ID: i32 = 2353;
pub const SILVER_BAR_ID: i32 = 2355;
pub const LUMBRIDGE_DOOR: (i32, i32, i32) = (3208, 3211, 0);
pub const LUMBRIDGE_DOOR_STAND: (i32, i32, i32) = (3208, 3212, 0);
pub const LUMBRIDGE_GATE: (i32, i32, i32) = (3213, 3261, 0);
pub const LUMBRIDGE_GATE_STAND: (i32, i32, i32) = (3213, 3260, 0);
pub const GNOME_START: (i32, i32, i32) = (2474, 3436, 0);
pub const GNOME_AFTER_LOG: (i32, i32, i32) = (2474, 3429, 0);
pub const GNOME_GROUND_RETURN: (i32, i32, i32) = (2487, 3420, 0);
pub const GNOME_PIPE: (i32, i32, i32) = (2484, 3431, 0);
// Selected 274/289 content: 86.5 XP per full lap, then 7.5 for the next log.
pub const GNOME_SECOND_LOG_XP: i32 = 94;
pub const WILDY_START: (i32, i32, i32) = (2998, 3916, 0);
pub const WILDY_PIPE_DEST: (i32, i32, i32) = (3004, 3947, 0);
pub const WILDY_ROPE_DEST: (i32, i32, i32) = (3005, 3958, 0);
pub const WILDY_STONE_DEST: (i32, i32, i32) = (2996, 3960, 0);
// Loc 2297 is stored on raw map plane 1, but LinkBelow exposes player plane 0.
pub const WILDY_LOG_DEST: (i32, i32, i32) = (2994, 3945, 0);
pub const WILDY_ROCKS_DEST: (i32, i32, i32) = (2994, 3933, 0);
pub const WILDY_LAP_XP: i32 = 586;
pub const WILDY_FURTHER_XP: i32 = 598;
pub const BRIMHAVEN_START: (i32, i32, i32) = (2809, 3194, 0);
pub const BRIMHAVEN_ARENA_VARP: i32 = 309;
pub const BRIMHAVEN_TICKET_ID: i32 = 2996;
pub const FLAX_FIELD: (i32, i32, i32) = (2741, 3444, 0);
pub const FLAX_FIELD_SCOPE: i32 = 12;
pub const FALADOR_CHICKENS: (i32, i32, i32) = (3029, 3294, 0);
pub const EMPTY_VIAL_ID: i32 = 229;
pub const VIAL_OF_WATER_ID: i32 = 227;
pub const EYE_OF_NEWT_ID: i32 = 221;
pub const GUAM_UNF_ID: i32 = 91;
pub const ATTACK_POTION_3_ID: i32 = 121;
pub const RANARR_WEED_ID: i32 = 257;
pub const RANARR_UNF_ID: i32 = 99;
pub const SNAPE_GRASS_ID: i32 = 231;
pub const PRAYER_POTION_3_ID: i32 = 139;
pub const FALADOR_WEST_BANK: (i32, i32, i32) = (2946, 3369, 0);
pub const FALADOR_EAST_BANK: (i32, i32, i32) = (3013, 3355, 0);
pub const FALADOR_FOUNTAIN: (i32, i32, i32) = (2949, 3381, 0);
pub const COW_HIDE_ID: i32 = 1739;
pub const SOFT_LEATHER_ID: i32 = 1741;
pub const HARD_LEATHER_ID: i32 = 1743;
pub const TANNER_IF: i32 = 679;
pub const SOFT_TAN_ALL_COM: i32 = 8686;
pub const HARD_TAN_ALL_COM: i32 = 8690;
pub const SHOPMAIN: i32 = 3824;
pub const AL_KHARID_BANK: (i32, i32, i32) = (3269, 3167, 0);
pub const TANNER_STAND: (i32, i32, i32) = (3277, 3191, 0);
pub const DOMMIK_STAND: (i32, i32, i32) = (3316, 3192, 0);
pub const RUNE_ESSENCE_ID: i32 = 1436;
pub const NOTED_ESSENCE_ID: i32 = 1437;
pub const AIR_TALISMAN_ID: i32 = 1438;
pub const EARTH_TALISMAN_ID: i32 = 1440;
pub const AIR_RUNE_ID: i32 = 556;
pub const EARTH_RUNE_ID: i32 = 557;
pub const TEMPLE_Z: i32 = 4000;
pub const VARROCK_EAST_BANK: (i32, i32, i32) = (3253, 3420, 0);
pub const RUNECRAFTER_AIR_RUINS: (i32, i32, i32) = (2988, 3294, 0);
pub const RUNECRAFTER_EARTH_RUINS: (i32, i32, i32) = (3303, 3477, 0);
pub const MULECRAFTER_AIR_RUINS: (i32, i32, i32) = (2983, 3288, 0);
pub const AIR_ALTAR: (i32, i32, i32) = (2841, 4829, 0);
pub const EARTH_ALTAR: (i32, i32, i32) = (2655, 4830, 0);
pub const CAKE_ID: i32 = 1891;
pub const BREAD_ID: i32 = 2309;
pub const CHOCOLATE_SLICE_ID: i32 = 1901;
pub const CHOCOLATE_CAKE_ID: i32 = 1897;
pub const NOTED_CAKE_ID: i32 = 1892;
pub const NOTED_BREAD_ID: i32 = 2310;
pub const NOTED_CHOCOLATE_SLICE_ID: i32 = 1902;
pub const ARDY_CAKES_BALLAST_KNIVES: i32 = 22;
pub const ARDY_CAKES_STAND: (i32, i32, i32) = (2668, 3312, 0);
pub const ARDY_THIEVER_STAND: (i32, i32, i32) = (2661, 3306, 0);
pub const AUTO_FIGHTER_MAGE_LEVEL: i32 = 13;
pub const AUTO_FIGHTER_MAGE_CASTS: i32 = 150;
pub const AUTO_FIGHTER_MAGE_AIR_RUNES: i32 = AUTO_FIGHTER_MAGE_CASTS * 2;
pub const AUTOCAST_MAGIC_VARP: i32 = 108;
pub const AUTOCAST_ARMED_VALUE: i32 = 3;
pub const ARDY_BANK: (i32, i32, i32) = (2655, 3286, 0);
pub const MAGIC_LOGS_ID: i32 = 1513;
pub const NOTED_MAGIC_LOGS_ID: i32 = 1514;
pub const UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 72;
pub const UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 70;
pub const MAGIC_SHORTBOW_ID: i32 = 861;
pub const MAGIC_LONGBOW_ID: i32 = 859;
pub const NOTED_UNSTRUNG_MAGIC_SHORTBOW_ID: i32 = 73;
pub const NOTED_UNSTRUNG_MAGIC_LONGBOW_ID: i32 = 71;
pub const KNIFE_ID: i32 = 946;
pub const STEEL_AXE_ID: i32 = 1353;
pub const RUNE_AXE_ID: i32 = 1359;
pub const GNOME_BALLAST_KNIVES: i32 = 26;
pub const MAGIC_TREE_ID: i32 = 1306;
pub const RUNE_PICKAXE_ID: i32 = 1275;
pub const NOTED_COAL_ID: i32 = 454;
pub const COAL_BALLAST_KNIVES: i32 = 26;
pub const GNOME_SOUTH_BANK_MAGIC_STAND: (i32, i32, i32) = (2433, 3409, 0);
pub const GNOME_SOUTH_BANK_MAGIC_TREE: (i32, i32, i32) = (2432, 3410, 0);

pub const GNOME_BANK_STAND: (i32, i32, i32) = (2445, 3425, 1);
pub const GNOME_BANK_STAIR_SOUTH: (i32, i32, i32) = (2444, 3416, 0);
pub const COAL_MINE: (i32, i32, i32) = (2582, 3481, 0);
pub const COAL_MINE_TRUCK_STAND: (i32, i32, i32) = (2575, 3486, 0);
pub const SEERS_BANK: (i32, i32, i32) = (2725, 3491, 0);
pub const RAW_SALMON_ID: i32 = 331;
pub const SALMON_ID: i32 = 329;
pub const NOTED_RAW_SALMON_ID: i32 = 332;
pub const NOTED_SALMON_ID: i32 = 330;
pub const RAW_LOBSTER_ID: i32 = 377;
pub const LOBSTER_ID: i32 = 379;
pub const NOTED_RAW_LOBSTER_ID: i32 = 378;
pub const NOTED_LOBSTER_ID: i32 = 380;
pub const BURNT_FISH_1_ID: i32 = 323;
pub const BURNT_FISH_2_ID: i32 = 343;
pub const BURNT_LOBSTER_ID: i32 = 381;
pub const NOTED_COPPER_ORE_ID: i32 = 437;
pub const NOTED_TIN_ORE_ID: i32 = 439;
pub const NOTED_IRON_ORE_ID: i32 = 441;
pub const NOTED_BRONZE_BAR_ID: i32 = 2350;
pub const NOTED_STEEL_BAR_ID: i32 = 2354;
pub const BOW_STRING_ID: i32 = 1777;
pub const NOTED_FLAX_ID: i32 = 1780;
pub const NOTED_BOW_STRING_ID: i32 = 1778;
pub const BALL_OF_WOOL_ID: i32 = 1759;
pub const COOKING_FIXTURE_LEVEL: i32 = 80;
pub const CATHERBY_BANK: (i32, i32, i32) = (2809, 3441, 0);
pub const CATHERBY_RANGE_STAND: (i32, i32, i32) = (2817, 3443, 0);
pub const AL_KHARID_FURNACE: (i32, i32, i32) = (3275, 3185, 0);
pub const FLAX_SPINNER_BANK: (i32, i32, i32) = (2722, 3493, 0);
pub const FLAX_SPINNER_WHEEL: (i32, i32, i32) = (2711, 3471, 1);
pub const FLAX_AIO_BANK: (i32, i32, i32) = (2725, 3493, 0);
pub const RED_SPIDERS_EGGS_ID: i32 = 223;
pub const NOTED_RED_SPIDERS_EGGS_ID: i32 = 224;
pub const NOTED_EYE_OF_NEWT_ID: i32 = 222;
pub const EGG_FIELD: (i32, i32, i32) = (3120, 9952, 0);
pub const BETTY_SHOP: (i32, i32, i32) = (3012, 3259, 0);
pub const TROUT_ID: i32 = 333;
pub const BIG_BONES_ID: i32 = 532;
pub const NOTED_BIG_BONES_ID: i32 = 533;
pub const LIMPWURT_ROOT_ID: i32 = 225;
pub const NOTED_LIMPWURT_ROOT_ID: i32 = 226;
pub const LAW_RUNE_ID: i32 = 563;
pub const BONES_ID: i32 = 526;
pub const NOTED_BONES_ID: i32 = 527;
pub const NOTED_HERB_ID: i32 = 200;
pub const LANTADYME_HERB_ID: i32 = 2485;
pub const NOTED_LANTADYME_HERB_ID: i32 = 2486;
pub const CHAOS_DRUID_FIELD: (i32, i32, i32) = (3110, 9936, 0);
pub const CHAOS_DRUID_TOWER_FIELD: (i32, i32, i32) = (2562, 3356, 0);
pub const CHAOS_DRUID_YANILLE_FIELD: (i32, i32, i32) = (2580, 9501, 0);
/// ArdyCakes / ArdyThiever Flee kite tile. Fight mode must kill instead of landing here.
pub const ARDY_FLEE_TILE: (i32, i32, i32) = (2655, 3298, 0);
pub const MOSS_GIANT_SAFESPOT: (i32, i32, i32) = (2553, 3406, 0);
pub const HILL_GIANT_PIT: (i32, i32, i32) = (3110, 9832, 0);
pub const CHAOS_DRUID_FOOD: i32 = 12;
pub const MOSS_GIANT_FOOD: i32 = 10;
pub const HILL_GIANT_FOOD: i32 = 8;
pub const AUTO_FIGHTER_FOOD: i32 = 8;
/// Bank cells prepare the pack below the card's own restock line so the bank
/// trip itself has to withdraw: MossGiant only banks once its food is gone.
pub const MOSS_GIANT_BANK_FOOD: i32 = 2;
/// ChaosDruidKiller's `tripPrepared` needs `foodWithdraw` (12) carried in the
/// field; below that its own first trip end is `prepare-trip`, so the bank cell
/// prepares the shortfall instead of a full pack.
pub const CHAOS_DRUID_BANK_FOOD: i32 = 8;
/// AutoFighter's BankRun withdraws food up to its declared `foodWithdraw`
/// default of 10.
pub const AUTO_FIGHTER_BANK_RESTOCK: i32 = 10;
/// Restock lines the other bank cards withdraw to: MossGiant's declared
/// `foodWithdraw` default 20 and HillGiant's 12 (8 carried + 4 withdrawn).
pub const MOSS_GIANT_BANK_RESTOCK: i32 = 20;
pub const HILL_GIANT_BANK_RESTOCK: i32 = 4;
pub const HILL_GIANT_BANK_PREPARED_RESTOCK: i32 = 12;
pub const ROCK_CRAB_FOOD: i32 = 8;
/// Base GreenDragon's ordinary trip withdraws twenty Lobsters.
/// Special, potion, bank, and teleport cells retain their twelve-food inputs.
pub const GREEN_DRAGON_BASE_FOOD: i32 = 20;
pub const GREEN_DRAGON_FOOD: i32 = 12;
pub const GREEN_DRAGON_BANK_PREPARED_FOOD: i32 = 26;
pub const GREEN_DRAGON_BANK_PREPARED_RESTOCK: i32 = 27;
pub const FIRE_GIANT_FOOD: i32 = 12;
pub const FIRE_GIANT_BANK_PREPARED_INITIAL_FOOD: i32 = 1;
pub const FIRE_GIANT_BANK_PREPARED_RESTOCK: i32 = 25;
/// Camelot escape restock line; barrel prepared keeps `FIRE_GIANT_BANK_PREPARED_RESTOCK`.
pub const FIRE_GIANT_CAMELOT_PREPARED_RESTOCK: i32 = 24;
pub const COMBAT_ATTACK_LEVEL: i32 = 40;
pub const REMAINING_COMBAT_PREPARED_LEVEL: i32 = 70;
pub const BANK_PRESSURE_PREPARED_LEVEL: i32 = 99;
pub const GREEN_DRAGON_TELE_PREPARED_LEVEL: i32 = 99;
pub const GREEN_DRAGON_TELE_PREPARED_HP: i32 = 70;
pub const RUNE_SCIMITAR_ID: i32 = 1333;
pub const DRAGONFIRE_SHIELD_ID: i32 = 1540;
pub const NOTED_DRAGONFIRE_SHIELD_ID: i32 = 1541;
pub const BRASS_KEY_ID: i32 = 983;
pub const DRAGON_BONES_ID: i32 = 536;
pub const NOTED_DRAGON_BONES_ID: i32 = 537;
pub const GREEN_DRAGONHIDE_ID: i32 = 1753;
pub const NOTED_GREEN_DRAGONHIDE_ID: i32 = 1754;
pub const BLACK_DRAGONHIDE_ID: i32 = 1747;
pub const RED_DRAGONHIDE_ID: i32 = 1749;
pub const BLUE_DRAGONHIDE_ID: i32 = 1751;
pub const GLARIALS_AMULET_ID: i32 = 295;
pub const ROPE_ID: i32 = 954;
pub const ROCK_CRAB_SPOT: (i32, i32, i32) = (2704, 3726, 0);
/// The dormant field resource the RockCrab branches wake: the frozen 289
/// content's `Rocks` NPC, which the source's own AI turns into the
/// `Rock Crab` target.
pub const ROCK_CRAB_DORMANT_NAME: &str = "Rocks";
/// Chebyshev radius around [`ROCK_CRAB_SPOT`] the dormant Rocks and the scoped
/// stand both live inside. The source's own `DEFAULT_SPOTS` field is smaller;
/// this stays the audited outer bound.
pub const ROCK_CRAB_FIELD_RADIUS: i32 = 50;
/// Dormant Rocks carried in the bounded per-frame projection: the nearest few
/// to the player, the ones this player can actually wake. A live frame can
/// hold more; only their index/tile identity is retained.
pub const DORMANT_ROCK_FACTS_MAX: usize = 8;
/// Inside the native visibility window while remaining outside dormant
/// RockCrab wake range; the live witness observes Rocks before Start.
pub const ROCK_CRAB_SAFE_STAND: (i32, i32, i32) = (2712, 3707, 0);
pub const GREEN_DRAGON_FIELD: (i32, i32, i32) = (3096, 3814, 0);
pub const FIRE_GIANT_ROOM: (i32, i32, i32) = (2575, 9893, 0);
pub const WILDERNESS_MIN_Z: i32 = 3520;
pub const DUNGEON_MIN_Z: i32 = 9000;
/// `com_mode`: the combat-tab style varp the ranged branches set directly.
pub const COMBAT_MODE_VARP: i32 = 43;
/// `rapid`, the ranged style the range branches select through `setCombatMode`.
pub const RAPID_COMBAT_MODE: i32 = 1;
/// `%sa_energy`, the special-attack bar's 0-1000 pool.
pub const SA_ENERGY_VARP: i32 = 300;
/// `%sa_attack`, set while a special is armed and cleared when the hit lands.
pub const SA_ARMED_VARP: i32 = 301;
pub const SA_ARMED_VALUE: i32 = 1;
/// Dragon dagger special cost, per the frozen card's own weapon table.
pub const DRAGON_DAGGER_SPECIAL_COST: i32 = 250;
pub const MAPLE_SHORTBOW_ID: i32 = 853;
pub const BRONZE_ARROW_ID: i32 = 882;
pub const DRAGON_DAGGER_ID: i32 = 1215;
pub const SUPER_ATTACK_3_ID: i32 = 145;
pub const SUPER_ATTACK_2_ID: i32 = 147;
pub const SUPER_STRENGTH_3_ID: i32 = 157;
pub const SUPER_STRENGTH_2_ID: i32 = 159;
/// Ranged level both range branches need before Start.
pub const RANGED_LEVEL: i32 = 40;
/// The bank stands the five bank cells have to reach. These are the frozen
/// cards' own declared walk targets (MossGiant `bankTile`, HillGiant
/// WEST_BANK, ChaosDruidKiller Edgeville `bankStand`) or the nearest bank the
/// card's own `nearestBank(here)` ranking resolves from its anchor (the two
/// Ardougne cards: East Ardougne, 3 tiles from `ARDY_BANK`).
pub const MOSS_GIANT_BANK: (i32, i32, i32) = (2615, 3332, 0);
pub const HILL_GIANT_BANK: (i32, i32, i32) = (3185, 3440, 0);
pub const CHAOS_DRUID_BANK: (i32, i32, i32) = (3094, 3491, 0);
pub const ARDOUGNE_EAST_BANK: (i32, i32, i32) = (2655, 3283, 0);
/// ArdyFighter's own DEFAULT_LOOT names, minus `clue scroll` (the card keeps
/// clue items in its deposit matcher and one id per scroll tier would be a
/// guess). These are the Guard drop-table rows the cards' lists were written
/// for: ArdyFighter's own loot list and the AutoFighter bank cell's injected
/// one.
pub const STEEL_ARROW_ID: i32 = 886;
pub const BODY_TALISMAN_ID: i32 = 1446;
pub const BLOOD_RUNE_ID: i32 = 565;
pub const CHAOS_RUNE_ID: i32 = 562;
/// The six verifiable Guard drops ArdyFighter lists in `DEFAULT_LOOT`
/// (`iron ore, steel arrow, body talisman, blood/chaos/nature rune`, whose
/// item names resolve to exactly these ids) and the class its bank trip
/// deposits. A clue-only or junk drop is not one of these.
pub const GUARD_DROP_IDS: [i32; 6] = [
    IRON_ORE_ID,
    STEEL_ARROW_ID,
    BODY_TALISMAN_ID,
    BLOOD_RUNE_ID,
    CHAOS_RUNE_ID,
    NATURE_RUNE_ID,
];
/// RockCrab PeriodicBank listed loot the bank cell can verify (the card's
/// default list also has clues and other gems; these two are the audit's
/// reachable, id-stable pair).
pub const CASKET_ID: i32 = 405;
pub const NOTED_CASKET_ID: i32 = 406;
pub const NOTED_UNCUT_SAPPHIRE_ID: i32 = 1624;
pub const FIRE_RUNE_ID: i32 = 554;
/// Varrock teleport Magic level and the frozen GreenDragon rune recipe
/// (`Law×1 + Air×3 + Fire×1`). The cell carries three casts so one failed
/// click cannot starve the escape.
pub const VARROCK_TELE_MAGIC: i32 = 25;
pub const VARROCK_TELE_LAW: i32 = 3;
pub const VARROCK_TELE_AIR: i32 = 9;
pub const VARROCK_TELE_FIRE: i32 = 3;
pub const GREEN_DRAGON_BANK_RESTOCK: i32 = 20;
pub const FIRE_GIANT_BANK_RESTOCK: i32 = 20;
/// GreenDragon `bankTile` default (Edgeville), distinct from ChaosDruidKiller's
/// Edgeville stand at z=3491.
pub const GREEN_DRAGON_BANK: (i32, i32, i32) = (3094, 3493, 0);
/// FireGiantLogic `RAFT_STAND` / `WASHED_OUT` / `BARREL_BANK`.
pub const FIRE_GIANT_RAFT: (i32, i32, i32) = (2510, 3493, 0);
pub const FIRE_GIANT_WASH: (i32, i32, i32) = (2527, 3413, 0);
pub const FIRE_GIANT_BANK: (i32, i32, i32) = (2616, 3332, 0);
/// FireGiantLogic `ESCAPE_TELES.Camelot` land and Magic 45 / Air×5+Law×1.
pub const CAMELOT_TELE_LAND: (i32, i32, i32) = (2757, 3478, 0);
pub const CAMELOT_TELE_MAGIC: i32 = 45;
pub const CAMELOT_TELE_AIR: i32 = 5;
pub const CAMELOT_TELE_LAW: i32 = 1;
/// Source `teleStock` default 2 spare casts plus the one needed to leave.
pub const CAMELOT_TELE_STOCK: i32 = 2;
pub const CAMELOT_AIR_CARRY: i32 = CAMELOT_TELE_AIR * (CAMELOT_TELE_STOCK + 1);
pub const CAMELOT_LAW_CARRY: i32 = CAMELOT_TELE_LAW * (CAMELOT_TELE_STOCK + 1);
/// Frozen Varrock teleport land (the same tile FireGiant's Varrock escape uses).
pub const VARROCK_TELE_LAND: (i32, i32, i32) = (3213, 3424, 0);
/// Nearest RockCrab `DEFAULT_SPOTS` loc to the safe stand (2712,3707,0) — spot
/// 4 at (2710,3717,0). `onStart` picks that as `currentSpot()`, and PeriodicBank
/// returns there.
pub const ROCK_CRAB_BANK_RET: (i32, i32, i32) = (2710, 3717, 0);
/// AIO Teleport start: Lumbridge bank, r8, so Varrock/Falador arrival is a
/// real tile change. Generated tele_coord `0_46_52_21_50` is Falador land.
pub const LUMBRIDGE_BANK: (i32, i32, i32) = (3092, 3245, 0);
pub const FALADOR_TELE_LAND: (i32, i32, i32) = (2965, 3378, 0);
pub const FALADOR_TELE_MAGIC: i32 = 37;
pub const STAFF_OF_AIR_ID: i32 = 1381;
pub const STAFF_OF_WATER_ID: i32 = 1383;
pub const WATER_RUNE_ID: i32 = 555;
pub const AIO_LAW_PACK: i32 = 2;
pub const AIO_ELEMENT_PACK: i32 = 20;
pub const AIO_LAW_BANK: i32 = 200;
/// Frozen ShopBuyout presets: Aemad, Aubury, Lowe, Hickton, Harry, Betty, Gerrant.
pub const AEMAD_STAND: (i32, i32, i32) = (2613, 3294, 0);
pub const AUBURY_STAND: (i32, i32, i32) = (3253, 3401, 0);
pub const LOWE_STAND: (i32, i32, i32) = (3231, 3421, 0);
pub const HICKTON_STAND: (i32, i32, i32) = (2821, 3442, 0);
pub const HARRY_STAND: (i32, i32, i32) = (2833, 3443, 0);
/// Frozen shopPresets Betty stand (3012,3258) — not herblore BETTY_SHOP 3259.
pub const BETTY_STAND: (i32, i32, i32) = (3012, 3258, 0);
pub const GERRANT_STAND: (i32, i32, i32) = (3013, 3224, 0);
/// Frozen Gerrant bankStand / Draynor operable approach (not canonical 3093,3243).
pub const DRAYNOR_BANK: (i32, i32, i32) = (3092, 3243, 0);
pub const FISHING_BAIT_ID: i32 = 313;
pub const SHOP_COIN_BANK: i32 = 20_000;
/// Varrock West anvil used by SmithingBot. Bronze platebody is Smithing 18,
/// not the audit's copied Smithing 1 (dagger) seed.
pub const VARROCK_WEST_BANK: (i32, i32, i32) = (3185, 3440, 0);
pub const VARROCK_ANVIL: (i32, i32, i32) = (3188, 3425, 0);
pub const HAMMER_ID: i32 = 2347;
pub const BRONZE_DAGGER_ID: i32 = 1205;
pub const BRONZE_PLATEBODY_ID: i32 = 1117;
pub const BRONZE_PLATEBODY_SMITHING: i32 = 18;
pub const SMITH_BAR_BANK: i32 = 28;
pub const NEEDLE_ID: i32 = 1733;
pub const THREAD_ID: i32 = 1734;
pub const LEATHER_GLOVES_ID: i32 = 1059;
pub const HARDLEATHER_BODY_ID: i32 = 1131;
/// Selected 274 leather-interface identity. 289 mismatch is LIVE unavailable,
/// not a new native.
pub const LEATHER_IF: i32 = 2311;
pub const LEATHER_GLOVES_MAKE10: i32 = 8636;
pub const HARD_LEATHER_CRAFTING: i32 = 28;
pub const LEATHER_BANK: i32 = 28;
pub const THREAD_BANK: i32 = 100;
pub const OAK_LOGS_ID: i32 = 1521;
pub const TINDERBOX_ID: i32 = 590;
pub const OAK_FIREMAKING: i32 = 15;
pub const FIRE_LOG_BANK: i32 = 28;
/// Posted native `FIRE_PLOTS` Varrock East AABB (not frozen FIRE_SPOTS).
pub const FIRE_PLOT_VARROCK_EAST_X0: i32 = 3235;
pub const FIRE_PLOT_VARROCK_EAST_X1: i32 = 3275;
pub const FIRE_PLOT_VARROCK_EAST_Z0: i32 = 3418;
pub const FIRE_PLOT_VARROCK_EAST_Z1: i32 = 3432;
/// ClimbingBoots native witness (frozen `ClimbingBoots.ts` + clean 289 pack).
/// Tenzing's standing targets: Falador West bank stand, the hut door, and the
/// inside tile he is bought from. These are route targets, never PASS
/// predicates. Boots id 3105 and the 12-coin pair come from
/// `death_sherpa.rs2` (inv_del(coins,12) + inv_add(death_climbingboots,1)).
pub const CLIMBING_BOOTS_ID: i32 = 3105;
pub const CLIMBING_BOOTS_PAIR_COINS: i32 = 12;
pub const TENZING_HUT_DOOR: (i32, i32, i32) = (2823, 3555, 0);
pub const TENZING_INSIDE: (i32, i32, i32) = (2820, 3556, 0);
pub const TENZING_NAME: &str = "Tenzing";
/// `useTeleport`/`runeStock` bounds and the Falador cast cost. Magic below 37
/// walks even with useTeleport=true, so the teleport cell must prove a real
/// cast: magic XP plus a Law/Air/Water spend and a landing, not the option.
pub const CLIMBING_BOOTS_RUNE_STOCK_MIN: i32 = 1;
pub const CLIMBING_BOOTS_WALK_PACK_COINS: i32 = 28 * CLIMBING_BOOTS_PAIR_COINS;
pub const CLIMBING_BOOTS_TELE_PACK_COINS: i32 = 25 * CLIMBING_BOOTS_PAIR_COINS;
