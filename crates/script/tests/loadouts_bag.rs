// Task 6: settings.str('loadout') reads the posted operator bag.

use script::{LoadIsolate, LoadShape, SettingDef};

const LOADOUT_PROBE: &str = r#"
export default class T extends LoopingBot {
    loop() {
        globalThis.__probe = this.settings.str('loadout', '');
    }
}
"#;

#[test]
fn settings_str_returns_selected_loadout_name() {
    let iso = LoadIsolate::spawn(LOADOUT_PROBE.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn loadout probe");
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!("melee"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    let value = iso.probe("__probe").expect("loadout probe readable");
    assert_eq!(
        value, "melee",
        "str reads posted loadout name, not fallback"
    );
    iso.join();
}

#[test]
fn loadout_combo_resolves_from_store_names_in_script_crate() {
    use script::{resolve_setting_options, Loadout, LoadoutsStore};

    let dir = std::env::temp_dir().join(format!("274bot-loadouts-combo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("loadouts.json");
    let mut store = LoadoutsStore::at(path);
    store.upsert(Loadout::new("a"));
    store.upsert(Loadout::new("b"));
    let def = SettingDef {
        id: "loadout".into(),
        ty: "string".into(),
        default: None,
        label: None,
        min: None,
        max: None,
        step: None,
        options: Vec::new(),
        option_labels: Vec::new(),
        group: None,
        show_if: None,
        options_from: Some("loadouts".into()),
        csv_toggle: None,
        help: None,
        item_option_spec: None,
    };
    assert_eq!(
        resolve_setting_options(&def, &store, None),
        vec!["a".to_string(), "b".to_string()]
    );
}

#[test]
fn isolate_accessors_read_slots_quantities_and_food() {
    use script::Loadout;

    let src = r#"
import { selectedLoadout } from '../../api/loadout/loadoutSetting.js';
import { gearOf, suppliesOf, weaponOf, scriptFood } from '../../api/loadout/loadoutPlan.js';
export default class T extends LoopingBot {
    loop() {
        const loadout = selectedLoadout(this.settings);
        globalThis.__weapon = weaponOf(loadout, 'Bronze sword');
        globalThis.__blank_weapon = weaponOf(null, 'Bronze sword');
        globalThis.__gear = gearOf(loadout);
        globalThis.__supplies = suppliesOf(loadout);
        globalThis.__food = scriptFood(this.settings, 'Trout');
    }
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .expect("spawn loadout accessors");
    iso.post_loadouts(&[Loadout::new("melee")
        .with_slot("righthand", "Rune scimitar")
        .with_slot("torso", "Rune chainbody")
        .with_carry("Coins", 1)
        .with_carry("Lobster", 10)]);
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!("melee"));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__weapon").unwrap(), "Rune scimitar");
    assert_eq!(iso.probe("__blank_weapon").unwrap(), "Bronze sword");
    assert_eq!(
        iso.probe("__gear").unwrap(),
        serde_json::json!(["Rune scimitar", "Rune chainbody"])
    );
    assert_eq!(
        iso.probe("__supplies").unwrap(),
        serde_json::json!([{"item":"Coins","qty":1},{"item":"Lobster","qty":10}])
    );
    assert_eq!(iso.probe("__food").unwrap(), "Lobster");
    iso.join();
}

#[test]
fn isolate_provision_composes_fresh_bank_withdraw_and_wear() {
    use script::Loadout;

    let src = r#"
import { Bank } from '../../api/bank/Bank.js';
import { Equipment } from '../../api/equipment/Equipment.js';
import { selectedLoadout } from '../../api/loadout/loadoutSetting.js';
import { suppliesOf, weaponOf } from '../../api/loadout/loadoutPlan.js';
export default class T extends LoopingBot {
    loop() {
        globalThis.__ran = true;
        if (globalThis.__did) return;
        globalThis.__did = true;
        try {
            const loadout = selectedLoadout(this.settings);
            const supplies = suppliesOf(loadout);
            const weapon = weaponOf(loadout, null);
            globalThis.__weapon = weapon;
            globalThis.__qty = supplies.length ? supplies[0].qty : 0;
            if (!Bank.ready()) {
                globalThis.__stale = true;
                return;
            }
            Bank.withdrawX(supplies[0].item, supplies[0].qty);
            Equipment.equip(weapon);
            globalThis.__queued = true;
        } catch (e) {
            globalThis.__err = String(e && e.message ? e.message : e);
        }
    }
}
"#;
    let stale = LoadIsolate::spawn_with_game_data(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .unwrap();
    stale.post_loadouts(&[Loadout::new("melee")
        .with_slot("righthand", "Rune scimitar")
        .with_carry("Lobster", 10)]);
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!("melee"));
    stale.post_settings_bag(&bag);
    stale
        .probe("globalThis.__rs2b0t_host.snapshot = {bank_open:false,bank_loaded:false,inv:[],equipment:[],bank:[]}; true")
        .unwrap();
    stale.on_game_tick(1);
    let ran = stale.probe("__ran");
    let err = stale.probe("__err");
    let stale_val = stale.probe("__stale");
    assert_eq!(
        stale_val.as_ref().ok(),
        Some(&serde_json::json!(true)),
        "stale bank must refuse ran={ran:?} err={err:?} stale={stale_val:?}"
    );
    assert!(stale.drain_interacts().is_empty());
    stale.join();

    let ready = LoadIsolate::spawn_with_game_data(
        src.to_string(),
        LoadShape::CompatClass,
        vec![],
        api::game_data::for_revision(client::io::ClientRevision::R274).unwrap(),
    )
    .unwrap();
    ready.post_loadouts(&[Loadout::new("melee")
        .with_slot("righthand", "Rune scimitar")
        .with_carry("Lobster", 10)]);
    ready.post_settings_bag(&bag);
    ready
        .probe(
            "globalThis.__rs2b0t_host.snapshot = {bank_open:true,bank_loaded:true,bank_generation:4,inv:[{name:'Rune scimitar',count:1,ops:[]}],equipment:[],bank:[{name:'Lobster',count:40,id:379,ops:['Withdraw 10','Withdraw X']}],inv_size:28}; true",
        )
        .unwrap();
    ready.on_game_tick(1);
    assert_eq!(ready.probe("__queued").unwrap(), true);
    let reqs = ready.drain_interacts();
    assert!(
        reqs.iter().any(|r| matches!(
            r,
            script::shim::InteractReq::WithdrawX { name, count, .. }
                if name == "Lobster" && *count == 10
        )),
        "fresh bank must withdraw the loadout quantity: {reqs:?}"
    );
    assert!(
        reqs.iter().any(|r| matches!(
            r,
            script::shim::InteractReq::Wear { name } if name == "Rune scimitar"
        )),
        "wear must use righthand, not a queued success: {reqs:?}"
    );
    ready.join();
}
