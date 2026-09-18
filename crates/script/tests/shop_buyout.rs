// ShopBuyout host facts + in-isolate Rust buyout helper.
// Frozen ShopBuyout looks up rec from SHOP_DB and calls buyoutPlan; this
// fixture uses those same specifiers so an empty catalog fallback cannot pass.

use script::{LoadIsolate, LoadShape};
use serde_json::Value;

fn data() -> std::sync::Arc<api::game_data::SelectedGameData> {
    api::game_data::for_revision(client::io::ClientRevision::R289).unwrap()
}

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn_with_game_data(src.to_string(), LoadShape::CompatClass, vec![], data())
        .unwrap()
}

fn tick(iso: &LoadIsolate) {
    iso.on_game_tick(1);
    let _ = iso.probe("true");
}

const PLAN_SRC: &str = r#"
import { SHOP_DB } from '../../data/shopdb.js';
import { buyoutPlan } from '../../api/shop/BuyoutLogic.js';
export default class T extends LoopingBot {
    loop() {
        const rec = Object.values(SHOP_DB).find(r => r.keepers.includes('Aemad')) ?? null;
        const unknown = Object.values(SHOP_DB).find(r => r.keepers.includes('Shop keeper')) ?? null;
        let plan = null;
        let err = null;
        try {
            plan = rec
                ? buyoutPlan(
                    rec,
                    { vial_water: 500, bronze_arrow: 500, iron_axe: 2, papyrus: 50 },
                    200,
                    new Set(['vial of water']),
                )
                : null;
        } catch (e) {
            err = String(e.message || e);
        }
        let unsupported = null;
        try {
            buyoutPlan({ inv: 'generalshop1', keepers: ['Shop keeper'], items: [] }, { pot_empty: 3 }, 200, new Set(['Pot']));
        } catch (e) {
            unsupported = String(e.message || e);
        }
        globalThis.__probe = {
            recInv: rec && rec.inv,
            recKeepers: rec && rec.keepers,
            unknown,
            plan,
            err,
            unsupported,
            posted: Object.keys(SHOP_DB).sort(),
            shopJsonUnknown: (function() {
                const fn = globalThis.rustyscript && globalThis.rustyscript.functions
                    ? globalThis.rustyscript.functions.__rs2b0t_shop
                    : undefined;
                if (typeof fn !== 'function') return 'missing';
                return fn({ op: 'buyout-plan', rec: rec, stock: {}, coins: 200, chosen: [] });
            })(),
        };
    }
}
"#;

#[test]
fn frozen_shopbuyout_specifiers_get_rec_and_call_rust_planner() {
    let iso = spawn(PLAN_SRC);
    tick(&iso);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["recInv"], "adventurershop");
    assert_eq!(probe["recKeepers"], serde_json::json!(["Aemad", "Kortan"]));
    assert_eq!(
        probe["unknown"],
        Value::Null,
        "unpublished keepers are absent"
    );
    assert_eq!(
        probe["posted"],
        serde_json::json!(["adventurershop", "runeshop"])
    );
    assert_eq!(probe["err"], Value::Null);
    let plan = probe["plan"].as_array().expect("plan");
    assert_eq!(plan.len(), 1, "selected vial, not all-stock fallback");
    assert_eq!(plan[0]["obj"], "vial_water");
    assert_eq!(plan[0]["name"], "Vial of water");
    assert!(plan[0]["units"].as_i64().unwrap() > 0);
    assert!(plan[0]["estCost"].as_i64().unwrap() <= 200);
    let unsupported = probe["unsupported"].as_str().unwrap_or("");
    assert!(
        unsupported.contains("not impl"),
        "unsupported shop must fail closed, got {unsupported:?}"
    );
    let shop_json = &probe["shopJsonUnknown"];
    assert_eq!(
        shop_json["kind"], "notImpl",
        "buyout-plan must not ride the JSON shop binding, got {shop_json}"
    );
    iso.join();
}

#[test]
fn aubury_selection_ranks_by_base_cost() {
    let src = r#"
import { SHOP_DB } from '../../data/shopdb.js';
import { buyoutPlan } from '../../api/shop/BuyoutLogic.js';
export default class T extends LoopingBot {
    loop() {
        const rec = Object.values(SHOP_DB).find(r => r.keepers.includes('Aubury'));
        globalThis.__probe = buyoutPlan(
            rec,
            { firerune: 5, waterrune: 5, deathrune: 2, chaosrune: 2 },
            10_000,
            new Set(['fire rune', 'water rune', 'death rune', 'chaos rune']),
        );
    }
}
"#;
    let iso = spawn(src);
    tick(&iso);
    let probe = iso.probe("__probe").unwrap();
    let plan = probe.as_array().expect("plan");
    let objs: Vec<_> = plan
        .iter()
        .map(|row| row["obj"].as_str().unwrap_or(""))
        .collect();
    assert_eq!(objs, ["deathrune", "chaosrune", "firerune", "waterrune"]);
    iso.join();
}

#[test]
fn shopbuyout_folder_evaluates_the_same_rec_and_planner_imports() {
    let Some(_) = script::rs2b0t_root() else {
        return;
    };
    let root = script::rs2b0t_root().unwrap();
    let dir = std::env::temp_dir().join(format!(
        "274bot-shop-buyout-{}-{}",
        std::process::id(),
        "card"
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut lib = script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"));
    lib.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("catalog register");
    let card = lib
        .get(script::ScriptSource::Catalog, "ShopBuyout")
        .cloned()
        .expect("ShopBuyout listed");
    assert_eq!(card.unloadable, None, "ShopBuyout must load");
    let siblings = script::resolve_sibling_modules(
        &card.path,
        &card.origin,
        &script::JsCache::new(
            card.path
                .parent()
                .unwrap()
                .join(format!("sib-cache-{}", std::process::id())),
        ),
        script::CacheMeta {
            kind: script::ScriptKind::Compat,
            source: script::ScriptSource::Catalog,
            shape: Some("CompatClass".into()),
            api_family: None,
        },
    )
    .expect("canonical siblings resolve");
    let src = r#"
import { SHOP_DB } from '../../data/shopdb.js';
import { buyoutPlan } from '../../api/shop/BuyoutLogic.js';
export default class T extends LoopingBot {
    loop() {
        const rec = Object.values(SHOP_DB).find(r => r.keepers.includes('Aemad')) ?? null;
        globalThis.__probe = {
            recInv: rec && rec.inv,
            plan: rec
                ? buyoutPlan(rec, { vial_water: 500, bronze_arrow: 500 }, 200, new Set(['vial of water']))
                : null,
        };
    }
}
"#;
    let iso = LoadIsolate::spawn_with_game_data(
        src.to_string(),
        LoadShape::CompatClass,
        siblings,
        data(),
    )
    .expect("spawn ShopBuyout-folder isolate");
    tick(&iso);
    let probe = iso.probe("__probe").unwrap();
    assert_eq!(probe["recInv"], "adventurershop");
    assert_eq!(probe["plan"][0]["obj"], "vial_water");
    assert_eq!(probe["plan"].as_array().unwrap().len(), 1);
    iso.join();
}
