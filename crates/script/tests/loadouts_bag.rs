// Task 6: settings.str('loadout') reads the posted operator bag.

use script::isolate_fb::ItemRowInput;
use script::{LoadIsolate, LoadShape, SettingDef};

#[path = "common/bank_slot.rs"]
mod bank_slot;
mod common;

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
    use api::loadout::Loadout;
    use script::{resolve_setting_options, LoadoutsStore};

    let store = LoadoutsStore::detached(vec![Loadout::new("a"), Loadout::new("b")]);
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
    use api::loadout::Loadout;

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

// The isolate keeps posted loadouts in Rust, resolves the saved setting there,
// and uses a re-posted row when the same name is selected again.
#[test]
fn selected_loadout_resolves_blank_and_unique_trimmed_case_match() {
    use api::loadout::Loadout;

    let src = r#"
import { selectedLoadout } from '../../api/loadout/loadoutSetting.js';
export default class T extends LoopingBot {
    loop() {
        const picked = selectedLoadout(this.settings);
        (globalThis.__picked ||= []).push(picked.name);
        globalThis.__worn = picked.worn;
    }
}
"#;
    let iso = LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn loadout selection");
    iso.post_loadouts(&[
        Loadout::new("Melee").with_slot("righthand", "Rune scimitar"),
        Loadout::new("Second").with_slot("righthand", "Oak shortbow"),
    ]);
    let mut bag = serde_json::Map::new();
    bag.insert("loadout".into(), serde_json::json!(""));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__picked").unwrap(), serde_json::json!(["Melee"]));

    bag.insert("loadout".into(), serde_json::json!(" SECOND "));
    iso.post_settings_bag(&bag);
    iso.on_game_tick(2);
    assert_eq!(
        iso.probe("__picked").unwrap(),
        serde_json::json!(["Melee", "Second"])
    );

    iso.post_loadouts(&[
        Loadout::new("Melee").with_slot("righthand", "Rune scimitar"),
        Loadout::new("Second").with_slot("righthand", "Crossbow"),
    ]);
    iso.on_game_tick(3);
    assert_eq!(
        iso.probe("__picked").unwrap(),
        serde_json::json!(["Melee", "Second", "Second"])
    );
    assert_eq!(
        iso.probe("__worn").unwrap(),
        serde_json::json!({ "righthand": "Crossbow" })
    );
    iso.join();
}

#[test]
fn isolate_provision_composes_fresh_bank_withdraw_and_wear() {
    use api::loadout::Loadout;

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

    let mut ready = bank_slot::BankSlot::new(
        src,
        LoadShape::CompatClass,
        None,
        std::sync::Arc::new(api::named_banks::NamedBankFacts::empty()),
    );
    ready.slot.post_loadouts(&[Loadout::new("melee")
        .with_slot("righthand", "Rune scimitar")
        .with_carry("Lobster", 10)]);
    ready.slot.post_settings_bag(&bag);
    // Rust reads the bank rows from the posted scene: post them, not a JS
    // snapshot object.
    let held_ops: [String; 0] = [];
    let bank_ops = ["Withdraw 10".to_string(), "Withdraw X".to_string()];
    let row = |name, count, id, ops, component_id| ItemRowInput {
        name: Some(name),
        count,
        id,
        ops,
        noted: false,
        cert: -1,
        component_id,
        slot: 0,
    };
    let inv = [row("Rune scimitar", 1, 1333, &held_ops[..], 3214)];
    let bank_rows = [row("Lobster", 40, 379, &bank_ops[..], 700)];
    let mut snap = common::ingame_snapshot();
    snap.bank_open = true;
    snap.bank_loaded = true;
    snap.bank_generation = 4;
    snap.inv = &inv;
    snap.bank = &bank_rows;
    let native = script::isolate_fb::NativeFactsInput {
        bank_snapshot_generation: Some(4),
        ..Default::default()
    };
    ready.post_snapshot(&snap, native);
    ready.tick_posted(1);
    assert_eq!(ready.probe("__queued").unwrap(), true);
    assert_eq!(ready.probe("__qty").unwrap(), 10);
    let (_, reqs, _) = ready.slot.drain_host_interacts();
    let mut actions = Vec::new();
    for tick in 2..=4 {
        snap.tick = tick;
        ready.tick_from_input(tick, &snap, native);
        actions.extend(
            ready
                .take_actions()
                .into_iter()
                .map(|action| match action.effect {
                    script::native::HostEffect::Interaction(request) => request,
                    _ => panic!("loadout provision emitted a non-interaction native effect"),
                }),
        );
        if !actions.is_empty() {
            break;
        }
    }
    assert!(
        actions.iter().any(|request| matches!(
            request,
            script::shim::InteractReq::WithdrawX { name, count, action, .. }
                if name == "Lobster" && *count == 10 && action == "Withdraw 10"
        )),
        "fresh bank must withdraw the loadout quantity (10): {actions:?}"
    );
    assert!(
        reqs.iter().any(|r| matches!(
            &r.req,
            script::shim::InteractReq::Wear { name } if name == "Rune scimitar"
        )),
        "wear must use righthand, not a queued success: {reqs:?}"
    );
    ready.stop();
}
