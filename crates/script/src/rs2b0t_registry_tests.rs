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
