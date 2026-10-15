use super::*;

fn ids(schema: &[SettingDef]) -> Vec<&str> {
    schema.iter().map(|s| s.id.as_str()).collect()
}

fn setting<'a>(schema: &'a [SettingDef], id: &str) -> &'a SettingDef {
    schema
        .iter()
        .find(|s| s.id == id)
        .unwrap_or_else(|| panic!("missing setting {id} in {:?}", ids(schema)))
}

#[test]
fn rockcrab_from_entries_keeps_bank_keys_and_trailing_solve_clues() {
    let src = r#"
export const SETTINGS = {
    combatStyle: { type: 'string', default: 'melee', options: ['melee', 'mage', 'range'] },
    loadout: LOADOUT_SETTING,
    ...Object.fromEntries(Object.entries(PERIODIC_BANK_SETTINGS).map(([key, def]) => [key, { ...def, group: 'Banking & loot' }])),
    solveClues: { type: 'boolean', default: true, label: 'Solve easy clues', group: 'Clues' }
};
"#;
    let schema = settings_schema_from_source(src);
    let found = ids(&schema);
    for id in [
        "combatStyle",
        "loadout",
        "bankStrategy",
        "bankEveryItems",
        "bankEveryMinutes",
        "bankCommonJunk",
        "solveClues",
    ] {
        assert!(found.contains(&id), "lost {id}: {found:?}");
    }
    assert!(
        !found.contains(&"group"),
        "fromEntries leftover must not invent group: {found:?}"
    );
    let bank = setting(&schema, "bankStrategy");
    assert_eq!(bank.options, ["Off", "Loot count", "Time", "Either"]);
    assert_eq!(
        setting(&schema, "solveClues").default.as_deref(),
        Some("true")
    );
    assert_eq!(
        setting(&schema, "loadout").options_from.as_deref(),
        Some("loadouts")
    );
}

#[test]
fn unknown_or_malformed_spread_skips_one_expression() {
    let src = r#"
export const SETTINGS = {
    before: { type: 'boolean', default: true },
    ...Unknown.fromEntries({ group: 'poison', later: { type: 'string' } }),
    ...NOT_A_REAL_SETTINGS,
    after: { type: 'boolean', default: false, label: 'survives' }
};
"#;
    let schema = settings_schema_from_source(src);
    let found = ids(&schema);
    assert_eq!(
        found,
        ["before", "after"],
        "spread must not swallow later fields or invent ids: {found:?}"
    );
    assert_eq!(setting(&schema, "after").default.as_deref(), Some("false"));
}

#[test]
fn cookbot_location_surface_and_log_options_are_usable() {
    let src = r#"
export const SURFACE_OPTIONS = ['Range', 'Fire'] as const;
export const SETTINGS = {
    location: { type: 'string', default: 'Catherby', options: [...COOK_LOCATION_OPTIONS] },
    surface: { type: 'string', default: 'Range', options: [...SURFACE_OPTIONS] },
    logType: { type: 'string', default: 'Logs', options: Object.keys(LOG_LEVELS) },
    fireSpot: { type: 'string', default: 'Varrock East', options: Object.keys(FIRE_SPOTS) }
};
"#;
    let schema = settings_schema_from_source(src);
    let location = setting(&schema, "location");
    assert!(location.options.contains(&"Auto".into()));
    assert!(location.options.contains(&"Catherby".into()));
    assert!(location.options.contains(&"Custom".into()));
    assert_eq!(setting(&schema, "surface").options, ["Range", "Fire"]);
    assert_eq!(
        setting(&schema, "logType").options,
        [
            "Logs",
            "Oak logs",
            "Willow logs",
            "Maple logs",
            "Yew logs",
            "Magic logs"
        ]
    );
    assert_eq!(
        setting(&schema, "fireSpot").options,
        ["Varrock East", "Varrock West", "Draynor", "Seers"]
    );
}

#[test]
fn source_backed_mapped_keys_spreads_and_labels() {
    let src = r#"
export const BEST_AVAILABLE = 'Best available';
export const CUSTOM = 'Custom';
export const PICK_TIERS = [
    { tier: 'Rune', item: 'Rune pickaxe', level: 41 },
    { tier: 'Bronze', item: 'Bronze pickaxe', level: 1 }
];
export const PICK_OPTIONS = [BEST_AVAILABLE, ...PICK_TIERS.map(t => t.tier)];
export const HERBS = [
    { key: 'guam', name: 'Guam leaf', id: 249 },
    { key: 'ranarr', name: 'Ranarr weed', id: 257 }
];
export const HERB_OPTIONS = HERBS.map(h => h.name);
export const SECONDARIES = [
    { id: 'eggs', name: "Red spiders' eggs" },
    { id: 'newt', name: 'Eye of newt' }
];
export const SECONDARY_OPTIONS = SECONDARIES.map(s => s.name);
export const RECIPES = [
    { bar: 'Bronze', level: 1 },
    { bar: 'Iron', level: 15 }
];
const BLURITE = { bar: 'Blurite', level: 13 };
export const BAR_OPTIONS = [...RECIPES.map(r => r.bar), BLURITE.bar];
export const GEMS = [
    { key: 'sapphire', name: 'Sapphire' },
    { key: 'ruby', name: 'Ruby' }
];
export const GEM_OPTIONS = GEMS.map(g => g.name);
export const RUNES = {
    'Nature runes': { rune: 'Nature rune' },
    'Air runes': { rune: 'Air rune' }
};
export const RUNE_OPTIONS = Object.keys(RUNES);
const LEATHERS = {
    Leather: { leatherId: 1741 },
    'Hard leather': { leatherId: 1743 }
};
export const SHOP_PRESETS = [
    { label: "Aemad's vials — East Ardougne (Ardougne East bank)", keeper: 'Aemad', shopStand: new Tile(2613, 3294, 0) },
    { label: 'Wizard Guild runes — Yanille (Yanille bank)', keeper: 'Magic Store owner' }
];
export const NEAREST_BANK = 'Nearest';
export const SETTINGS = {
    bank: { type: 'string', default: NEAREST_BANK, options: [NEAREST_BANK, ...BANK_LOCATIONS.map(b => b.name)] },
    pickaxe: { type: 'string', options: PICK_OPTIONS },
    herbs: { type: 'string[]', options: [...HERB_OPTIONS, CUSTOM] },
    secondary: { type: 'string', options: SECONDARY_OPTIONS },
    bar: { type: 'string', options: [...BAR_OPTIONS] },
    gems: { type: 'string[]', options: GEM_OPTIONS },
    rune: { type: 'string', options: RUNE_OPTIONS },
    leatherType: { type: 'string', options: Object.keys(LEATHERS) },
    shop: { type: 'string', options: SHOP_PRESETS.map(p => p.label) },
    jiveProduct: { type: 'string', options: PRODUCT_OPTIONS },
    staff: { type: 'string', default: 'Staff of air', options: STAFFS }
};
"#;
    let schema = settings_schema_from_source(src);
    let bank = setting(&schema, "bank");
    assert_eq!(bank.options[0], "Nearest");
    assert_eq!(
        setting(&schema, "pickaxe").options,
        ["Best available", "Rune", "Bronze"]
    );
    assert_eq!(
        setting(&schema, "herbs").options,
        ["Guam leaf", "Ranarr weed", "Custom"]
    );
    assert_eq!(
        setting(&schema, "secondary").options,
        ["Red spiders' eggs", "Eye of newt"]
    );
    assert_eq!(
        setting(&schema, "bar").options,
        ["Bronze", "Iron", "Blurite"]
    );
    assert_eq!(setting(&schema, "gems").options, ["Sapphire", "Ruby"]);
    assert_eq!(
        setting(&schema, "rune").options,
        ["Nature runes", "Air runes"]
    );
    assert_eq!(
        setting(&schema, "leatherType").options,
        ["Leather", "Hard leather"]
    );
    assert_eq!(
        setting(&schema, "shop").options,
        [
            "Aemad's vials — East Ardougne (Ardougne East bank)",
            "Wizard Guild runes — Yanille (Yanille bank)"
        ]
    );
    assert!(
        setting(&schema, "jiveProduct")
            .options
            .contains(&"Gold ring".into()),
        "constructor-built PRODUCT_OPTIONS uses frozen metadata"
    );
    let staff = setting(&schema, "staff");
    assert!(
        staff.options.is_empty(),
        "W1 equipment must stay unpublished at parse time: {:?}",
        staff.options
    );
    assert_eq!(staff.options_from.as_deref(), Some("STAFFS"));
}

#[test]
fn w1c_equipment_union_idents_match_frozen_ranged_ts() {
    assert_eq!(
        super::w1c_equipment_option_families("RANGED_WEAPONS"),
        Some(&["bows", "darts"][..])
    );
    assert_eq!(
        super::w1c_equipment_option_families("ROCK_CRAB_RANGED_WEAPONS"),
        Some(&["bows", "darts"][..])
    );
    assert!(super::w1c_equipment_option_families("AXES").is_none());
    assert!(super::w1c_equipment_option_families("DROP_DB").is_none());
}

#[test]
fn preset_buyable_names_dedupes_and_sorts() {
    let src = r#"
export const SHOP_PRESETS = [
    { label: 'A', keeper: 'Aemad' },
    { label: 'B', keeper: 'Lowe' }
];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"},{"name":"Feather"}]},
    "archeryshop": {"keepers":["Lowe"],"items":[{"name":"Bronze arrow"},{"name":"Vial of water"}]}
};
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
    let schema = settings_schema_from_source(src);
    let buy = setting(&schema, "buyItems");
    assert_eq!(
        buy.options,
        ["Bronze arrow", "Feather", "Vial of water"],
        "union sorted; duplicate Vial of water once"
    );
}

#[test]
fn preset_buyable_names_malformed_shop_db_stays_empty() {
    let src = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Nobody' }];
export const SHOP_DB = { broken: true };
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
    let schema = settings_schema_from_source(src);
    let buy = setting(&schema, "buyItems");
    assert!(
        buy.options.is_empty(),
        "unknown keeper must not fabricate names"
    );
}

#[test]
fn frozen_alch_sort_wrapper_item_option_spec() {
    let src = r#"
const ALCH_RATE = 0.6;
const FODDER = [{ obj: 'maple_longbow' }, { obj: 'yew_longbow', label: 'Yew longbow' }];
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = FODDER.flatMap(({ obj, label }) => {
    const rec = ITEM_DB.find(r => r.obj === obj);
    return rec ? [{ key: obj, id: rec.id, name: rec.name, label: label ?? rec.name, alchValue: Math.floor(rec.cost * ALCH_RATE) }] : [];
}).sort((a, b) => b.alchValue - a.alchValue);
export const ALCH_OPTIONS = [
    CUSTOM_ALCH_KEY,
    ...[...ALCH_ITEMS].sort((a, b) => a.label.localeCompare(b.label)).map(i => i.key)
];
export const SETTINGS = {
    items: { type: 'string[]', default: [], options: ALCH_OPTIONS }
};
"#;
    let schema = settings_schema_from_source(src);
    let items = setting(&schema, "items");
    assert!(items.options.is_empty());
    let spec = items.item_option_spec.as_ref().expect("sort-wrapped spec");
    assert_eq!(spec.prefix, vec!["custom".to_string()]);
    assert!(spec.sort_keys_by_label);
    assert_eq!(spec.candidates.len(), 2);
}

#[test]
fn computed_alch_map_key_stays_unresolved() {
    let src = r#"
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = [];
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...ALCH_ITEMS.map(i => i.key)];
export const SETTINGS = {
    items: { type: 'string[]', default: ['steel_platebody'], options: ALCH_OPTIONS }
};
"#;
    let schema = settings_schema_from_source(src);
    let items = setting(&schema, "items");
    assert!(
        items.options.is_empty(),
        "ALCH .key map must stay unresolved, got {:?}",
        items.options
    );
    assert!(items.item_option_spec.is_none());
}

#[test]
fn unsupported_alch_wrapper_shapes_stay_unresolved() {
    let fodder = r#"
const ALCH_RATE = 0.6;
const FODDER = [{ obj: 'maple_longbow' }, { obj: 'yew_longbow', label: 'Yew longbow' }];
export const CUSTOM_ALCH_KEY = 'custom';
export const ALCH_ITEMS = FODDER.flatMap(({ obj, label }) => {
    const rec = ITEM_DB.find(r => r.obj === obj);
    return rec ? [{ key: obj, id: rec.id, name: rec.name, label: label ?? rec.name, alchValue: Math.floor(rec.cost * ALCH_RATE) }] : [];
}).sort((a, b) => b.alchValue - a.alchValue);
"#;
    let filter = format!(
        r#"{fodder}
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...[...ALCH_ITEMS.filter(x => true)].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)];
export const SETTINGS = {{
    items: {{ type: 'string[]', default: [], options: ALCH_OPTIONS }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
    );
    let schema = settings_schema_from_source(&filter);
    let items = setting(&schema, "items");
    assert!(items.options.is_empty());
    assert!(
        items.item_option_spec.is_none(),
        "filter remainder must not parse as ALCH_ITEMS: {:?}",
        items.item_option_spec
    );
    assert_eq!(setting(&schema, "later").options, ["KeepMe"]);

    let concat = format!(
        r#"{fodder}
export const ALCH_OPTIONS = [CUSTOM_ALCH_KEY, ...[...ALCH_ITEMS].sort((a,b)=>a.label.localeCompare(b.label)).map(i=>i.key)].concat(evil);
export const SETTINGS = {{
    items: {{ type: 'string[]', default: [], options: ALCH_OPTIONS }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
    );
    let schema = settings_schema_from_source(&concat);
    let items = setting(&schema, "items");
    assert!(
        items.item_option_spec.is_none(),
        "trailing .concat must stay unresolved"
    );
    assert_eq!(setting(&schema, "later").options, ["KeepMe"]);
}

#[test]
fn preset_buyable_names_requires_empty_call_and_full_expression() {
    let db = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"}]}
};
"#;
    let args = format!(
        r#"{db}
export const SETTINGS = {{
    buyItems: {{ type: 'string[]', default: [], options: presetBuyableNames(x) }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
    );
    let schema = settings_schema_from_source(&args);
    assert!(
        setting(&schema, "buyItems").options.is_empty(),
        "presetBuyableNames(x) must stay unresolved"
    );
    assert_eq!(setting(&schema, "later").options, ["KeepMe"]);

    let concat = format!(
        r#"{db}
export const SETTINGS = {{
    buyItems: {{ type: 'string[]', default: [], options: presetBuyableNames().concat(x) }},
    later: {{ type: 'string', options: ['KeepMe'] }}
}};
"#
    );
    let schema = settings_schema_from_source(&concat);
    assert!(
        setting(&schema, "buyItems").options.is_empty(),
        "presetBuyableNames().concat must stay unresolved"
    );
    assert_eq!(setting(&schema, "later").options, ["KeepMe"]);
}

#[test]
fn incomplete_shop_db_parse_stays_empty() {
    let src = r#"
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Vial of water"}]},
    junk
};
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
    let schema = settings_schema_from_source(src);
    assert!(
        setting(&schema, "buyItems").options.is_empty(),
        "partial SHOP_DB must not publish a keeper union"
    );
}

#[test]
fn shop_db_sources_use_explicit_keys_and_never_ambient() {
    let iso = crate::IsolatedEnv::enter("shopdb-parser-ambient");
    let foreign = iso.dir.join("foreign-root");
    std::fs::create_dir_all(foreign.join("src/bot/data")).unwrap();
    std::fs::write(
        foreign.join("src/bot/data/shopdb.ts"),
        r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"ZZZ_FOREIGN_TONIC"}]}
};
"#,
    )
    .unwrap();
    iso.set_rs2b0t(&foreign);
    persist_rs2b0t_root(&foreign).expect("persist foreign root");

    let shop = r#"
import { presetBuyableNames } from './shopPresets.js';
export const SHOP_PRESETS = [{ label: 'A', keeper: 'Aemad' }];
export const SETTINGS = {
    buyItems: { type: 'string[]', default: [], options: presetBuyableNames() }
};
"#;
    let isolated = settings_schema_from_source(shop);
    assert!(
        setting(&isolated, "buyItems").options.is_empty(),
        "source-only parse must not read ambient shopdb: {:?}",
        setting(&isolated, "buyItems").options
    );

    let mut empty = HashMap::new();
    empty.insert("./ShopBuyout/ShopBuyout.js".into(), shop.to_string());
    let cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &empty,
        )
        .expect("partial sources parse");
    assert!(
        setting(&cards[0].settings_schema, "buyItems")
            .options
            .is_empty(),
        "Some(partial) must not borrow another catalog: {:?}",
        setting(&cards[0].settings_schema, "buyItems").options
    );

    let mut decoy = empty.clone();
    decoy.insert(
        "./injected/shopdb-extra.ts".into(),
        r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Wrong Name"}]}
};
"#
        .into(),
    );
    let decoy_cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &decoy,
        )
        .expect("decoy sources parse");
    assert!(
        setting(&decoy_cards[0].settings_schema, "buyItems")
            .options
            .is_empty(),
        "contains(shopdb) first-hit must not select a non-explicit key"
    );

    let mut exact = empty;
    exact.insert(
        "./src/bot/data/shopdb.ts".into(),
        r#"export const SHOP_DB = {
    "adventurershop": {"keepers":["Aemad"],"items":[{"name":"Right Name"}]}
};
"#
        .into(),
    );
    let exact_cards = parse_registry_with_sources(
            r#"
import ShopBuyout, { SETTINGS } from './ShopBuyout/ShopBuyout.js';
ScriptRegistry.register({ name: 'ShopBuyout', settingsSchema: SETTINGS, create: () => new ShopBuyout() });
"#,
            &exact,
        )
        .expect("exact shopdb key parses");
    assert_eq!(
        setting(&exact_cards[0].settings_schema, "buyItems").options,
        ["Right Name"]
    );
}

#[test]
fn synthetic_multi_card_settings_audit_has_recovered_options() {
    let index = r#"
import CookBot, { SETTINGS as COOK_SETTINGS } from './CookBot/CookBot.js';
import Herblore, { SETTINGS as HERB_SETTINGS } from './Herblore/Herblore.js';
ScriptRegistry.register({ name: 'CookBot', settingsSchema: COOK_SETTINGS, create: () => new CookBot() });
ScriptRegistry.register({ name: 'HerbloreSecondaries', settingsSchema: HERB_SETTINGS, create: () => new Herblore() });
"#;
    let cook = r#"
export const LOCATIONS = ['Lumbridge', 'Varrock'];
export const SETTINGS = { location: { type: 'string', options: LOCATIONS } };
export default class CookBot {}
"#;
    let herb = r#"
export const SECONDARY_OPTIONS = ['Eye of newt', 'Unicorn horn'];
export const SETTINGS = { secondary: { type: 'string', options: SECONDARY_OPTIONS } };
export default class Herblore {}
"#;
    let mut sources = std::collections::HashMap::new();
    sources.insert("./CookBot/CookBot.js".into(), cook.into());
    sources.insert("./Herblore/Herblore.js".into(), herb.into());
    let cards = parse_registry_with_sources(index, &sources).expect("parse");
    let by_name: std::collections::HashMap<_, _> =
        cards.iter().map(|c| (c.name.as_str(), c)).collect();
    assert_eq!(
        setting(&by_name["CookBot"].settings_schema, "location").options,
        ["Lumbridge", "Varrock"]
    );
    assert_eq!(
        setting(&by_name["HerbloreSecondaries"].settings_schema, "secondary").options,
        ["Eye of newt", "Unicorn horn"]
    );
}

#[test]
fn setting_def_spread_matches_bare_constant_plus_overridden_keys() {
    let bare_src = r#"
export const SETTINGS = {
    loadout: LOADOUT_SETTING,
};
"#;
    let group_spread_src = r#"
export const SETTINGS = {
    loadout: { ...LOADOUT_SETTING, group: 'Food & healing' },
};
"#;
    let help_spread_src = r#"
export const SETTINGS = {
    loadout: { ...LOADOUT_SETTING, help: 'gear to wear, defined in the Loadouts panel; its food is ignored, this bot eats what it steals from the Baker\'s stall' },
};
"#;
    let bare = setting(&settings_schema_from_source(bare_src), "loadout").clone();
    let group_spread = setting(&settings_schema_from_source(group_spread_src), "loadout").clone();
    let mut expected_group = bare.clone();
    expected_group.group = Some("Food & healing".into());
    assert_eq!(group_spread, expected_group);

    let help_spread = setting(&settings_schema_from_source(help_spread_src), "loadout").clone();
    let mut expected_help = bare;
    expected_help.help = Some(
        "gear to wear, defined in the Loadouts panel; its food is ignored, this bot eats what it steals from the Baker's stall"
            .into(),
    );
    assert_eq!(help_spread, expected_help);
}

#[test]
fn setting_def_spread_override_order_follows_js_object_spread_semantics() {
    let src = r#"
export const BASE_NUM = {
    type: 'number',
    default: 10,
    min: 1,
    max: 50,
    label: 'Base label',
    help: 'Base help',
};
export const SECOND_OVERRIDE = {
    default: 20,
    max: 100,
    group: 'From second spread',
};
export const SETTINGS = {
    spreadWinsOverEarlierKey: {
        label: 'Overwritten by spread',
        group: 'Survives because BASE_NUM has no group',
        ...BASE_NUM,
    },
    laterKeyWinsOverSpread: {
        ...BASE_NUM,
        label: 'Explicit label wins',
        default: 25,
        max: 99,
    },
    chainedSpreadsAndKeys: {
        min: 0,
        ...BASE_NUM,
        min: 5,
        ...SECOND_OVERRIDE,
        help: 'Final help wins',
    },
};
"#;
    let schema = try_settings_schema_from_source(src).expect("valid spreads parse");
    let first = setting(&schema, "spreadWinsOverEarlierKey");
    assert_eq!(first.ty, "number");
    assert_eq!(first.default.as_deref(), Some("10"));
    assert_eq!(first.min.as_deref(), Some("1"));
    assert_eq!(first.max.as_deref(), Some("50"));
    assert_eq!(first.label.as_deref(), Some("Base label"));
    assert_eq!(first.help.as_deref(), Some("Base help"));
    assert_eq!(
        first.group.as_deref(),
        Some("Survives because BASE_NUM has no group")
    );

    let second = setting(&schema, "laterKeyWinsOverSpread");
    assert_eq!(second.ty, "number");
    assert_eq!(second.default.as_deref(), Some("25"));
    assert_eq!(second.min.as_deref(), Some("1"));
    assert_eq!(second.max.as_deref(), Some("99"));
    assert_eq!(second.label.as_deref(), Some("Explicit label wins"));
    assert_eq!(second.help.as_deref(), Some("Base help"));

    let chained = setting(&schema, "chainedSpreadsAndKeys");
    assert_eq!(chained.ty, "number");
    assert_eq!(chained.default.as_deref(), Some("20"));
    assert_eq!(chained.min.as_deref(), Some("5"));
    assert_eq!(chained.max.as_deref(), Some("100"));
    assert_eq!(chained.label.as_deref(), Some("Base label"));
    assert_eq!(chained.group.as_deref(), Some("From second spread"));
    assert_eq!(chained.help.as_deref(), Some("Final help wins"));
}

#[test]
fn unresolvable_setting_def_spread_refuses_naming_the_setting_and_never_emits_untyped() {
    for (label, src) in [
        (
            "unknown ident",
            r#"
export const SETTINGS = {
    before: { type: 'boolean', default: true },
    loadout: { ...UNKNOWN_SETTING_CONST, group: 'Food & healing' },
    after: { type: 'boolean', default: false },
};
"#,
        ),
        (
            "call expression spread",
            r#"
export const SETTINGS = {
    loadout: { ...makeLoadoutSetting(), group: 'Food & healing' },
};
"#,
        ),
        (
            "cyclic setting spread",
            r#"
export const CYCLE_A = { ...CYCLE_B, group: 'A' };
export const CYCLE_B = { ...CYCLE_A, type: 'string' };
export const SETTINGS = {
    loadout: { ...CYCLE_A },
};
"#,
        ),
        (
            "schema-level spread inside setting def",
            r#"
export const SETTINGS = {
    loadout: { ...PERIODIC_BANK_SETTINGS, group: 'Banking' },
};
"#,
        ),
    ] {
        let err =
            try_settings_schema_from_source(src).expect_err(&format!("{label} must fail closed"));
        assert!(
            err.contains("loadout"),
            "{label}: refusal must name the setting, got {err:?}"
        );
        let fallback = settings_schema_from_source(src);
        assert!(
            fallback.iter().all(|def| !def.ty.is_empty()),
            "{label}: must never emit an untyped setting: {fallback:?}"
        );
        assert!(
            fallback.is_empty(),
            "{label}: failed schema must not publish partial settings: {fallback:?}"
        );
    }
}

#[test]
fn rock_crab_catalog_shaped_fixture_expands_loadout_spread_and_populates_bag() {
    let src = r#"
import { LOADOUT_SETTING } from '../../api/loadout/loadoutSetting.js';
import { PERIODIC_BANK_SETTINGS } from '../../api/bank/Banking.js';
import { CUSTOM_RANGED_SETTINGS } from '../../api/combat/rangedSettings.js';

const SHOW_MAGE = { key: 'combatStyle', anyOf: ['mage'] };
const SHOW_RANGE = { key: 'combatStyle', anyOf: ['range'] };
const AMMO_OPTIONS = ['Bronze arrow', 'Iron arrow'];

export const SETTINGS: SettingsSchema = {
    combatStyle: { type: 'string', default: 'melee', options: ['melee', 'mage', 'range'], label: 'Combat style' },
    staff: { type: 'string', default: 'Staff of air', options: STAFFS, label: 'Staff', group: 'Combat', showIf: SHOW_MAGE },
    bow: { type: 'string', default: 'Maple shortbow', options: [...ROCK_CRAB_RANGED_WEAPONS, 'Other'], label: 'Ranged weapon', group: 'Combat', showIf: SHOW_RANGE },
    ammo: { type: 'string', default: 'Bronze arrow', options: [...AMMO_OPTIONS, 'Other'], label: 'Bow ammo', group: 'Combat', showIf: SHOW_RANGE },
    ...CUSTOM_RANGED_SETTINGS,
    loadout: { ...LOADOUT_SETTING, group: 'Food & healing' },
    foodWithdraw: { type: 'number', default: 20, min: 1, max: 27, label: 'Food to withdraw per bank run', group: 'Food & healing' },
    ...Object.fromEntries(Object.entries(PERIODIC_BANK_SETTINGS).map(([key, def]) => [key, { ...def, group: 'Banking & loot' }])),
    solveClues: { type: 'boolean', default: true, label: 'Solve easy clues', group: 'Clues' }
};
"#;
    let schema = try_settings_schema_from_source(src).expect("RockCrab schema parses");
    let loadout = setting(&schema, "loadout");
    assert_eq!(loadout.ty, "string");
    assert_eq!(loadout.default.as_deref(), Some(""));
    assert_eq!(loadout.label.as_deref(), Some("Loadout"));
    assert_eq!(loadout.options_from.as_deref(), Some("loadouts"));
    assert_eq!(loadout.group.as_deref(), Some("Food & healing"));
    assert!(loadout.help.as_deref().unwrap_or("").contains("loadout"));

    let bag = crate::merge_bag(&schema, &serde_json::Map::new(), None);
    assert_eq!(
        bag.get("loadout"),
        Some(&serde_json::json!("")),
        "default bag must carry loadout='' so pre-Start loadout validation runs"
    );
    let empty_loadouts: [api::loadout::Loadout; 0] = [];
    let err = crate::loadouts_store::resolve_script_loadout_setting(
        &empty_loadouts,
        bag.get("loadout").and_then(|v| v.as_str()),
    )
    .expect_err("empty loadouts store must refuse before Start");
    assert!(
        err.to_string().contains("no loadouts exist"),
        "unexpected error: {err}"
    );
}
#[test]
fn nested_metadata_cannot_supply_outer_setting_fields() {
    let missing_type = "const BASE = { metadata: { type: 'string', default: '' } }; export const SETTINGS = { loadout: { ...BASE, group: 'Food' } };";
    let error = try_settings_schema_from_source(missing_type)
        .expect_err("nested type must not satisfy the setting's required outer type");
    assert!(
        error.contains("loadout") && error.contains("missing setting type"),
        "{error}"
    );

    let nested_fields = r#"
export const SETTINGS = {
    choice: {
        type: 'string',
        metadata: {
            default: '',
            label: 'Nested label',
            group: 'Nested group',
            help: 'Nested help',
            min: 1,
            options: ['nested'],
            optionsFrom: 'loadouts'
        }
    }
};
"#;
    let schema = try_settings_schema_from_source(nested_fields).expect("outer type is present");
    let choice = setting(&schema, "choice");
    assert_eq!(choice.default, None);
    assert_eq!(choice.label, None);
    assert_eq!(choice.group, None);
    assert_eq!(choice.help, None);
    assert_eq!(choice.min, None);
    assert!(choice.options.is_empty());
    assert_eq!(choice.options_from, None);
}

#[test]
fn nested_default_label_cannot_override_the_outer_label() {
    let src = "const BASE = { type: 'tile', default: { x: 1, z: 2, level: 0, label: 'Nested' } }; export const SETTINGS = { stand: { ...BASE, label: 'Actual stand' } };";
    let schema = try_settings_schema_from_source(src).expect("setting parses");
    let stand = setting(&schema, "stand");
    assert_eq!(stand.label.as_deref(), Some("Actual stand"));
    assert_eq!(
        stand.default, None,
        "an unsupported object default must not leak one of its nested strings"
    );
}

#[test]
fn a_later_spread_replaces_nested_setting_properties_shallowly() {
    let src = r#"
const BASE = { type: 'string', default: '', showIf: { key: 'mode', anyOf: ['x'] } };
const LATER = { showIf: { key: 'other', equals: 'y' } };
export const SETTINGS = { choice: { ...BASE, ...LATER } };
"#;
    let schema = try_settings_schema_from_source(src).expect("setting parses");
    assert_eq!(
        setting(&schema, "choice").show_if.as_deref(),
        Some("{ key: 'other', equals: 'y' }"),
        "the later nested object replaces the earlier value instead of merging"
    );
}
