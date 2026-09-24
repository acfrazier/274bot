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
        bank.options.len(),
        crate::content::BANK_ALIASES.len() + 1,
        "the imported BANK_LOCATIONS spread must resolve every published alias: {:?}",
        bank.options
    );
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

fn load_frozen_catalog_index_and_sources(
    root: &std::path::Path,
) -> (String, HashMap<String, String>) {
    let index_path = registry_index_path(root);
    let index = std::fs::read_to_string(&index_path)
        .unwrap_or_else(|e| panic!("read {}: {e}", index_path.display()));
    let skeleton = parse_registry_with_sources(&index, &HashMap::new())
        .expect("frozen index parses without sources");
    let mut sources = HashMap::new();
    for card in &skeleton {
        let Some(path) = script_file_path(root, &card.rel_path) else {
            continue;
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            sources.insert(card.rel_path.clone(), text);
        }
        let Some(dir) = path.parent() else {
            continue;
        };
        let Some(card_dir) = rel_dir(&card.rel_path) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let name = ent.file_name();
            let name = name.to_string_lossy();
            if !(name.ends_with(".ts") || name.ends_with(".js")) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(ent.path()) else {
                continue;
            };
            let rel_native = format!("./{card_dir}/{name}");
            sources.entry(rel_native).or_insert_with(|| text.clone());
            if let Some(stem) = name.strip_suffix(".ts") {
                sources
                    .entry(format!("./{card_dir}/{stem}.js"))
                    .or_insert(text);
            }
        }
    }
    let shopdb_path = root.join("src/bot/data/shopdb.ts");
    if let Ok(text) = std::fs::read_to_string(&shopdb_path) {
        sources
            .entry("./src/bot/data/shopdb.ts".into())
            .or_insert_with(|| text.clone());
        sources
            .entry("./src/bot/data/shopdb.js".into())
            .or_insert(text);
    }
    (index, sources)
}

/// Parser → resolver proof on production FireGiant SETTINGS (`staff`/`bow`).
/// Not UI proof. Requires `$RS2B0T` (frozen pin root).
#[test]
#[ignore = "requires absolute RS2B0T frozen catalog"]
fn frozen_fire_giant_w1c_equipment_settings_resolve_from_parsed_catalog() {
    use crate::loadouts_store::{resolve_setting_options_with_labels, LoadoutsStore};
    use client::io::ClientRevision;

    const FROZEN_SETTINGS: &str = "src/bot/scripts/FireGiant/FireGiant.ts";
    const FROZEN_PIN: &str = "00d39a17e0";

    let root =
        PathBuf::from(std::env::var("RS2B0T").expect("RS2B0T must name the frozen catalog root"));
    assert!(root.is_absolute(), "RS2B0T must be absolute");
    let settings_path = root.join(FROZEN_SETTINGS);
    assert!(
        settings_path.is_file(),
        "expected frozen FireGiant SETTINGS at {}",
        settings_path.display()
    );

    let (index, sources) = load_frozen_catalog_index_and_sources(&root);
    let cards = parse_registry_with_sources(&index, &sources).expect("frozen catalog parses");
    let fire = cards
        .iter()
        .find(|c| c.name == "FireGiant")
        .expect("FireGiant card from frozen index.ts");
    assert!(
        fire.rel_path.contains("FireGiant"),
        "card path must reference FireGiant: {}",
        fire.rel_path
    );

    let staff = setting(&fire.settings_schema, "staff");
    let bow = setting(&fire.settings_schema, "bow");
    assert!(
        staff.options.is_empty(),
        "parse-time STAFFS must stay empty: {:?}",
        staff.options
    );
    assert_eq!(staff.options_from.as_deref(), Some("STAFFS"));
    assert_eq!(bow.options, vec!["Other"]);
    assert_eq!(bow.options_from.as_deref(), Some("RANGED_WEAPONS"));
    let ammo = setting(&fire.settings_schema, "ammo");
    assert_eq!(ammo.options.last().map(String::as_str), Some("Other"));
    for (id, parent) in [("customBow", "bow"), ("customAmmo", "ammo")] {
        let custom = setting(&fire.settings_schema, id);
        assert_eq!(custom.default.as_deref(), Some(""));
        assert!(custom
            .show_if
            .as_deref()
            .is_some_and(|v| v.contains(&format!("key: '{parent}'")) && v.contains("'Other'")));
    }

    let store = LoadoutsStore::at(std::env::temp_dir().join("274bot-w1c-firegiant-resolve"));
    let r274 = api::game_data::for_revision(ClientRevision::R274).unwrap();
    let r289 = api::game_data::for_revision(ClientRevision::R289).unwrap();

    let staff_274 = resolve_setting_options_with_labels(staff, &store, Some(r274.as_ref()));
    let staff_289 = resolve_setting_options_with_labels(staff, &store, Some(r289.as_ref()));
    assert_eq!(staff_274, staff_289);
    assert_eq!(staff_274.values.len(), 15);
    assert_eq!(staff_274.values[0], "Staff");
    assert!(staff_274.values.contains(&"Staff of air".to_string()));
    assert_eq!(staff_274.label_for("Staff of air"), "Staff of air");
    assert_eq!(staff_274.values.len(), staff_274.labels.len());

    let bow_274 = resolve_setting_options_with_labels(bow, &store, Some(r274.as_ref()));
    let bow_289 = resolve_setting_options_with_labels(bow, &store, Some(r289.as_ref()));
    assert_eq!(bow_274, bow_289);
    assert_eq!(bow_274.values.len(), 20, "12 bows + 7 darts + Other");
    assert_eq!(bow_274.values[0], "Shortbow");
    assert!(bow_274.values.contains(&"Maple shortbow".to_string()));
    assert_eq!(bow_274.values[12], "Bronze dart");
    assert_eq!(bow_274.values.last().map(String::as_str), Some("Other"));
    assert_eq!(bow_274.label_for("Maple shortbow"), "Maple shortbow");
    assert_eq!(bow_274.values.len(), bow_274.labels.len());
    for (card_name, family_count) in [
        ("MossGiant", 19),
        ("RockCrab", 19),
        ("BrimhavenMossGiants", 20),
    ] {
        let card = cards
            .iter()
            .find(|c| c.name == card_name)
            .unwrap_or_else(|| panic!("missing frozen {card_name}"));
        let bow = setting(&card.settings_schema, "bow");
        let resolved = resolve_setting_options_with_labels(bow, &store, Some(r289.as_ref()));
        assert_eq!(
            resolved.values.len(),
            family_count + 1,
            "{card_name} ranged choices"
        );
        assert_eq!(resolved.values.last().map(String::as_str), Some("Other"));
        let ammo = setting(&card.settings_schema, "ammo");
        let resolved_ammo = resolve_setting_options_with_labels(ammo, &store, Some(r289.as_ref()));
        assert_eq!(
            resolved_ammo.values.last().map(String::as_str),
            Some("Other")
        );
        for id in ["customBow", "customAmmo"] {
            assert!(
                card.settings_schema.iter().any(|def| def.id == id),
                "{card_name} missing {id}"
            );
        }
    }

    eprintln!(
        "frozen FireGiant pin={FROZEN_PIN} settings={FROZEN_SETTINGS} staff={} bow={}",
        staff_274.values.len(),
        bow_274.values.len()
    );
}

/// Bounded frozen same-dir walk. Not UI proof. Requires `$RS2B0T`.
#[test]
#[ignore = "requires absolute RS2B0T frozen catalog"]
fn frozen_catalog_settings_audit() {
    let root =
        PathBuf::from(std::env::var("RS2B0T").expect("RS2B0T must name the frozen catalog root"));
    assert!(root.is_absolute(), "RS2B0T must be absolute");
    let (index, sources) = load_frozen_catalog_index_and_sources(&root);
    let cards = parse_registry_with_sources(&index, &sources).expect("frozen catalog parses");
    let by_name: HashMap<&str, &RegistryCard> =
        cards.iter().map(|c| (c.name.as_str(), c)).collect();

    let want = [
        ("JiveCrafting", "product"),
        ("JiveEnchanter", "jewel"),
        ("JiveMarketDumper", "bank"),
        ("EssMiner", "pickaxe"),
        ("RuneCrafter", "rune"),
        ("NatureCrafter", "rune"),
        ("CookBot", "location"),
        ("CookBot", "surface"),
        ("CookBot", "logType"),
        ("DartFletcher", "tier"),
        ("HerbloreSecondaries", "secondary"),
        ("HerbCleaner", "herbs"),
        ("PotionMaker", "herb"),
        ("PotionMaker", "secondary"),
        ("SmelterBot", "bar"),
        ("Superheater", "bar"),
        ("Alcher", "items"),
        ("GemCutter", "gems"),
        ("MuleCrafter", "rune"),
        ("ShopBuyout", "shop"),
        ("ShopBuyout", "buyItems"),
        ("LeatherCrafter", "leatherType"),
        ("Firemaker", "logType"),
        ("Firemaker", "location"),
    ];
    let mut recovered = 0usize;
    let mut empty = 0usize;
    for (card_name, setting_id) in want {
        let card = by_name
            .get(card_name)
            .unwrap_or_else(|| panic!("missing catalog card {card_name}"));
        let def = card
            .settings_schema
            .iter()
            .find(|s| s.id == setting_id)
            .unwrap_or_else(|| panic!("{card_name}.{setting_id} missing from schema"));
        eprintln!(
            "AUDIT {card_name}.{setting_id} n={} spec={} from={:?} opts={:?}",
            def.options.len(),
            def.item_option_spec.is_some(),
            def.options_from,
            def.options
        );
        if def.options.is_empty() && def.item_option_spec.is_none() {
            empty += 1;
        } else {
            recovered += 1;
        }
    }

    let rock = by_name.get("RockCrab").expect("RockCrab card");
    let rock_ids: Vec<&str> = rock.settings_schema.iter().map(|s| s.id.as_str()).collect();
    eprintln!("AUDIT RockCrab ids={rock_ids:?}");
    for id in [
        "bankStrategy",
        "bankEveryItems",
        "bankEveryMinutes",
        "bankCommonJunk",
        "solveClues",
    ] {
        assert!(rock_ids.contains(&id), "RockCrab lost {id}: {rock_ids:?}");
    }
    assert!(
        !rock_ids.contains(&"group"),
        "RockCrab must not invent group: {rock_ids:?}"
    );

    let rune_crafter = setting(&by_name["RuneCrafter"].settings_schema, "rune");
    assert_eq!(
        rune_crafter.options,
        ["Air runes", "Earth runes"],
        "RuneCrafter local Object.keys must win over mule altar table: {:?}",
        rune_crafter.options
    );
    let nature = setting(&by_name["NatureCrafter"].settings_schema, "rune");
    assert_eq!(
        nature.options,
        ["Nature runes", "Air runes"],
        "NatureCrafter local keys must win: {:?}",
        nature.options
    );
    let mule = setting(&by_name["MuleCrafter"].settings_schema, "rune");
    assert_eq!(
        mule.options,
        catalog_option_values("RUNE_OPTIONS").unwrap(),
        "MuleCrafter re-export uses imported 11 altar names: {:?}",
        mule.options
    );

    let cook_loc = setting(&by_name["CookBot"].settings_schema, "location");
    assert!(cook_loc.options.contains(&"Catherby".into()));
    assert_eq!(
        setting(&by_name["CookBot"].settings_schema, "surface").options,
        ["Range", "Fire"]
    );
    assert!(!setting(&by_name["CookBot"].settings_schema, "logType")
        .options
        .is_empty());

    let potion_herbs = &setting(&by_name["PotionMaker"].settings_schema, "herb").options;
    assert_eq!(
        potion_herbs.len(),
        15,
        "PotionMaker HERBS is 14 through Torstol plus CUSTOM: {potion_herbs:?}"
    );
    assert_eq!(potion_herbs.last().map(String::as_str), Some("Custom"));

    let alcher = setting(&by_name["Alcher"].settings_schema, "items");
    assert!(
        alcher.options.is_empty(),
        "Alcher.items keys must not bake into options: {:?}",
        alcher.options
    );
    let spec = alcher
        .item_option_spec
        .as_ref()
        .expect("Alcher.items sort-wrapped item_option_spec");
    assert_eq!(spec.prefix, vec!["custom".to_string()]);
    assert!(
        spec.sort_keys_by_label,
        "frozen ALCH_OPTIONS uses label sort before .map(i => i.key)"
    );
    assert!(
        !spec.candidates.is_empty(),
        "Alcher FODDER chain must yield candidates"
    );
    let shop = setting(&by_name["ShopBuyout"].settings_schema, "shop");
    assert!(
        shop.options.iter().any(|s| s.contains("Aemad")),
        "ShopBuyout.shop SHOP_PRESETS.map(p => p.label) must emit frozen labels: {:?}",
        shop.options
    );
    let buy = setting(&by_name["ShopBuyout"].settings_schema, "buyItems");
    assert!(
        !buy.options.is_empty(),
        "ShopBuyout.buyItems presetBuyableNames join must emit options: {:?}",
        buy.options
    );
    assert!(buy.item_option_spec.is_none());
    let buy_sorted = buy.options.clone();
    let mut sorted = buy_sorted.clone();
    sorted.sort();
    assert_eq!(
        buy_sorted,
        sorted,
        "presetBuyableNames must be alphabetically sorted: first={:?} last={:?}",
        buy_sorted.first(),
        buy_sorted.last()
    );
    let buy_lower: Vec<String> = buy.options.iter().map(|s| s.to_ascii_lowercase()).collect();
    assert!(buy_lower.iter().any(|n| n.contains("rune")));
    assert!(buy_lower.iter().any(|n| n.contains("arrow")));
    assert!(buy_lower.contains(&"feather".to_string()));
    assert!(buy_lower.contains(&"vial of water".to_string()));

    eprintln!(
        "AUDIT summary recovered={recovered} empty={empty} of {}",
        want.len()
    );
    assert_eq!(
        recovered + empty,
        want.len(),
        "row accounting {recovered}+{empty}"
    );
    assert_eq!(empty, 0, "all 24 audited option rows must recover");
    assert_eq!(recovered, 24);
}

/// The bank dropdown names are the published aliases: the runtime
/// `BANK_LOCATIONS` resolves through `content.named_banks` (built from
/// `content::BANK_ALIASES`), so a frozen foreign bank list would offer
/// names the lookup has no row for.
#[test]
fn bank_location_options_are_the_published_aliases() {
    let published: Vec<String> = crate::content::BANK_ALIASES
        .iter()
        .map(|alias| alias.name.to_string())
        .collect();
    assert!(!published.is_empty());
    assert_eq!(catalog_option_values("BANK_LOCATIONS").unwrap(), published);
    let mut with_nearest = vec!["Nearest".to_string()];
    with_nearest.extend(published);
    assert_eq!(
        catalog_option_values("BANK_LOCATION_OPTIONS").unwrap(),
        with_nearest,
        "BANK_LOCATION_OPTIONS is the caller's Nearest head plus the aliases"
    );
}
