#![cfg(all(feature = "live-harness", feature = "test-support"))]

#[path = "common/quester_live.rs"]
mod quester_live;

use api::snapshot::{GameSnapshot, WorldTile};
use client::client::skill::Skill;
use quester_live::{Cell, Mode};
use script::quester::path::{PathDocument, PathKind, StepDocument};
use script::quester::registry;
use serde_json::{json, Map};
use std::path::PathBuf;

const QUEST: &str = "zanaris";
const DISPLAY: &str = "Lost City";
const STAGE: &str = "zanaris:1";
const NEXT_STAGE: &str = "zanaris:2";
// R289 content defines `^zanaris_started = 1` in `scripts/quests/quest_zanaris/configs/quest_zanaris.constant`.
const STAGE_VALUE: i32 = 1;
const BASE40_LEVEL: i32 = scenario::quester::TestProfile::Base40.combat_level() as i32;
const BASE40_STATS: [(u8, i32); 7] = [
    (0, BASE40_LEVEL),
    (1, BASE40_LEVEL),
    (2, BASE40_LEVEL),
    (3, BASE40_LEVEL),
    (4, BASE40_LEVEL),
    (5, 43),
    (6, BASE40_LEVEL),
];
// R289 content checks Woodcutting 36 at `scripts/quests/quest_zanaris/scripts/quest_zanaris.rs2:28-40`.
// It checks Crafting 31 at the same file's lines 77-87.
const ZANARIS_CONTENT_STATS: [(u8, i32); 2] = [
    (8, 36),  // `Skill::names[8]` is Woodcutting.
    (12, 31), // `Skill::names[12]` is Crafting.
];
// The R289 Path estimates these buys at base stock (`zanaris.json:196-198,289-291`).
// Shop purchases recalculate each unit at live stock (`shop.rs2:65-75`):
// axe cost 16, stock 10, multiplier 1000/delta 20
// (`axes.obj:35-38`, `lumbridge.inv:1-10`, `lumbridge.npc:129-132`);
// bread cost 12, stock 10, multiplier 1000/delta 20
// (`dough.obj:39-50`, `ardougne_east.inv:15-24`, `ardougne_east.npc:383-413`).
// `shop.rs2:131-147,182-187` gives a zero-stock factor of
// 1000 - ((0 - 10) * 20) = 1200, scaled by base cost.
// The live route to the Baker uses two Port Sarim/Karamja ferries;
// `sailors.rs2:10-31` charges 30 coins per trip.
const ZANARIS_SHOP_SELL_MULTIPLIER: i32 = 1000;
const ZANARIS_SHOP_DELTA: i32 = 20;
const ZANARIS_ROUTE_FARE_GP: i32 = 30;
const ZANARIS_ROUTE_FARE_COUNT: i32 = 2;
const EXTRA_ITEMS: [(&str, i32); 1] = [("knife", 1)];
type ZanarisFixtureItems = Vec<(&'static str, i32)>;

fn zanaris_max_unit_price_at_zero_stock(obj: &str) -> Option<i32> {
    let (base_cost, stock_base) = match obj {
        "bronze_axe" => (16, 10),
        "bread" => (12, 10),
        _ => return None,
    };
    let stock_adjustment = (-(stock_base) * ZANARIS_SHOP_DELTA).clamp(-5000, 1000);
    let price_multiplier = (ZANARIS_SHOP_SELL_MULTIPLIER - stock_adjustment).max(100);
    Some(price_multiplier * base_cost / 1000)
}

fn add_zanaris_buy_budget(
    estimated_total: &mut i32,
    funding_total: &mut i32,
    step: &StepDocument,
) -> Result<(), String> {
    if step.kind != "buy" {
        return Ok(());
    }
    let estimate = step
        .args
        .get("est_gp")
        .and_then(serde_json::Value::as_u64)
        .and_then(|amount| i32::try_from(amount).ok())
        .ok_or_else(|| format!("Zanaris Path buy step {} has invalid est_gp", step.id.0))?;
    *estimated_total = (*estimated_total)
        .checked_add(estimate)
        .ok_or_else(|| "Zanaris Path purchase estimates exceed i32".to_owned())?;
    let obj = step
        .args
        .get("obj")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("Zanaris Path buy step {} has invalid obj", step.id.0))?;
    let qty = step
        .args
        .get("qty")
        .and_then(serde_json::Value::as_u64)
        .and_then(|qty| i32::try_from(qty).ok())
        .filter(|qty| *qty > 0)
        .ok_or_else(|| format!("Zanaris Path buy step {} has invalid qty", step.id.0))?;
    let max_unit_price = zanaris_max_unit_price_at_zero_stock(obj).ok_or_else(|| {
        format!(
            "Zanaris Path buy step {} has no content-backed price for {obj}",
            step.id.0
        )
    })?;
    let worst_case = qty
        .checked_mul(max_unit_price)
        .ok_or_else(|| format!("Zanaris Path buy step {} price exceeds i32", step.id.0))?;
    *funding_total = (*funding_total)
        .checked_add(estimate.max(worst_case))
        .ok_or_else(|| "Zanaris Path purchase funding exceeds i32".to_owned())?;
    Ok(())
}

fn zanaris_purchase_budget(path: &PathDocument) -> Result<(i32, i32), String> {
    let mut estimated_total = 0;
    let mut funding_total = 0;
    if let Some(quest) = &path.quest {
        for steps in quest.acquire.values() {
            for step in steps {
                add_zanaris_buy_budget(&mut estimated_total, &mut funding_total, step)?;
            }
        }
    }
    for role in &path.roles {
        if let Some(step) = &role.progress_reader {
            add_zanaris_buy_budget(&mut estimated_total, &mut funding_total, step)?;
        }
        for step in &role.prelude {
            add_zanaris_buy_budget(&mut estimated_total, &mut funding_total, step)?;
        }
        for sequence in &role.sequences {
            for step in &sequence.steps {
                add_zanaris_buy_budget(&mut estimated_total, &mut funding_total, step)?;
            }
        }
    }
    Ok((estimated_total, funding_total))
}

fn zanaris_fixture_items(path: &PathDocument) -> Result<(ZanarisFixtureItems, i32), String> {
    let (_, purchase_seed) = zanaris_purchase_budget(path)?;
    let route_fares = ZANARIS_ROUTE_FARE_GP
        .checked_mul(ZANARIS_ROUTE_FARE_COUNT)
        .ok_or_else(|| "Zanaris route fares exceed i32".to_owned())?;
    let coin_seed = purchase_seed
        .checked_add(route_fares)
        .ok_or_else(|| "Zanaris fixture coin seed exceeds i32".to_owned())?;
    let mut items = EXTRA_ITEMS.to_vec();
    items.push(("coins", coin_seed));
    Ok((items, coin_seed))
}
const STAND: WorldTile = WorldTile {
    x: 3145,
    z: 3202,
    level: 0,
};

fn base40_seed_steps() -> Vec<scenario::Step> {
    let mut steps = Vec::with_capacity(BASE40_STATS.len() + ZANARIS_CONTENT_STATS.len() + 2);
    steps.push(scenario::Step {
        name: "reset stats before explicit Base40 seed",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| api::interact::cheat(client, "minme").is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::StatAtMost { id: 1, max: 1 },
            budget_ticks: 80,
        },
    });
    steps.push(scenario::Step {
        name: "clear worn equipment before Base40 seed",
        kind: scenario::StepKind::Perform {
            send: Box::new(|client, _| api::interact::cheat(client, "~clearinv worn").is_sent()),
        },
        wait: scenario::Wait {
            arm: scenario::Proof::SideTabAvailable { index: 3 },
            budget_ticks: 80,
        },
    });
    for &(skill, level) in BASE40_STATS.iter().chain(ZANARIS_CONTENT_STATS.iter()) {
        let name = Skill::names[usize::from(skill)];
        steps.push(scenario::Step {
            name: "seed explicit account stat",
            kind: scenario::StepKind::Perform {
                send: Box::new(move |client, _| {
                    api::interact::cheat(client, &format!("setstat {name} {level}")).is_sent()
                }),
            },
            wait: scenario::Wait {
                arm: scenario::Proof::Stat {
                    id: i32::from(skill),
                    min: level,
                },
                budget_ticks: 80,
            },
        });
    }
    steps
}

fn zanaris_navigation_cell() -> Result<Cell, String> {
    let (path, _) = scenario::quester::load_quester_path_document(QUEST)?;
    if path.id.0.as_ref() != QUEST
        || path.kind != PathKind::Quest
        || path.partner.is_some()
        || path.roles.len() != 1
    {
        return Err("Zanaris navigation cell needs one single-account quest Path".into());
    }
    let selected = api::game_data::for_revision(api::selected::ClientRevision::R289)
        .map_err(|error| error.to_string())?;
    let identity = selected
        .quest_identity()
        .and_then(|table| table.rows.iter().find(|row| row.id == QUEST))
        .ok_or_else(|| "selected R289 data has no Zanaris quest identity".to_owned())?;
    if identity.display != DISPLAY || identity.varp != QUEST {
        return Err(format!(
            "Zanaris identity changed: display {:?}, varp {:?}",
            identity.display, identity.varp
        ));
    }
    let progress = path.roles[0]
        .progress
        .as_ref()
        .ok_or("Zanaris Path has no progress program")?;
    let mut stage_found = false;
    for rule in &progress.rules {
        if rule.stage.0.as_ref() != STAGE {
            continue;
        }
        stage_found = true;
        if let Some(value) = rule.varp {
            if value != STAGE_VALUE {
                return Err("Zanaris stage has contradictory varp hints".into());
            }
        }
    }
    if !stage_found {
        return Err("Zanaris Path has no stage 1 progress rule".into());
    }
    let stage_value = STAGE_VALUE;
    let knife_id = selected
        .item_by_alias("knife")
        .map(|item| item.id)
        .ok_or("selected R289 data has no knife item")?;
    let coins_id = selected
        .item_by_alias("coins")
        .map(|item| item.id)
        .ok_or("selected R289 data has no coins item")?;
    let (extra_items, coin_seed) = zanaris_fixture_items(&path)?;

    let mut scenario = scenario::quester_stage_with_watch(
        "zanaris-entrana-dungeon-entry",
        DISPLAY,
        QUEST,
        stage_value,
        &extra_items,
        STAND,
        12_000,
    );
    scenario.settings.deadline = std::time::Duration::from_secs(3600);
    let seed_index = scenario
        .steps
        .iter()
        .position(|step| step.name == "reset quest stage")
        .ok_or("quest stage seed step is missing")?;
    scenario
        .steps
        .splice(seed_index..seed_index, base40_seed_steps());
    let start_settings = Map::from_iter([("quests".into(), json!([QUEST]))]);
    scenario.settings.script_settings_overrides = Some(start_settings.clone());

    // This cell proves one navigation leg, not Zanaris qualification.
    // Keep Zanaris out of the qualified-quest profile table.
    // Seed Base40 combat and the required R289 path skills locally.
    let observe_start = Box::new(move |snapshot: &GameSnapshot| {
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("account seed was not observed in an active game".into());
        }
        let mut observed_stats =
            Vec::with_capacity(BASE40_STATS.len() + ZANARIS_CONTENT_STATS.len());
        for &(skill, level) in BASE40_STATS.iter().chain(ZANARIS_CONTENT_STATS.iter()) {
            let stat = snapshot
                .stats()
                .iter()
                .find(|stat| stat.index == i32::from(skill))
                .ok_or_else(|| format!("seeded stat {skill} is unobserved"))?;
            if stat.base != level || stat.effective != level {
                return Err(format!(
                    "seeded stat {skill} expected {level}, observed base {} effective {}",
                    stat.base, stat.effective
                ));
            }
            observed_stats.push(json!({
                "skill": Skill::names[usize::from(skill)],
                "base": stat.base,
                "effective": stat.effective
            }));
        }
        if snapshot.tile() != Some((STAND.x, STAND.z, STAND.level)) {
            return Err(format!(
                "Base40 seed stand expected ({},{},{}), observed {:?}",
                STAND.x,
                STAND.z,
                STAND.level,
                snapshot.tile()
            ));
        }
        let knife_count: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == knife_id)
            .map(|item| item.count)
            .sum();
        if knife_count < 1 {
            return Err(format!(
                "Base40 fixture knife expected 1, observed {knife_count}"
            ));
        }
        let coin_count: i32 = snapshot
            .inventory()
            .iter()
            .filter(|item| item.def.id == coins_id)
            .map(|item| item.count)
            .sum();
        if coin_count != coin_seed {
            return Err(format!(
                "Base40 fixture coins expected {coin_seed}, observed {coin_count}"
            ));
        }
        Ok(json!({
            "profile": "Base40 combat + Zanaris content stats",
            "stats": observed_stats,
            "knife_id": knife_id,
            "knife_count": knife_count,
            "coins_id": coins_id,
            "coin_count": coin_count,
            "tile": {"x": STAND.x, "z": STAND.z, "level": STAND.level}
        }))
    });
    Ok(Cell {
        quest: QUEST,
        display: DISPLAY,
        label: "dungeon-entry".to_owned(),
        scenario,
        start_settings,
        mode: Mode::Stage {
            expect: vec![NEXT_STAGE.to_owned()],
        },
        observe_start: Some(observe_start),
    })
}

struct SourceGuard(Option<registry::FolderSource>);

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Some(source) = self.0.take() {
            registry::set_source(source);
        }
    }
}

#[test]
#[ignore = "requires LIVE=1 and the selected local 289 engine; uses a content-backed Path"]
fn live_zanaris_entrana_dungeon_entry() {
    if !api::test_support::game_data_available() {
        return;
    }
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    let path_dir = PathBuf::from(std::env::var_os("QUESTER_PATH_DIR").expect("QUESTER_PATH_DIR"));
    let prior_source = registry::source();
    let _source_guard = SourceGuard(Some(prior_source));
    let source = scenario::quester::reload_quester_path_source("zanaris", Some(&path_dir)).unwrap();
    assert!(
        matches!(source, "folder" | "draft"),
        "expected a folder-backed Path, got {source}"
    );

    let cell = zanaris_navigation_cell().unwrap();
    let start_settings = cell.start_settings.clone();
    let receipt = quester_live::run_family(
        cell,
        Box::new(move |handle, account, _| {
            handle.start_compiled(
                account,
                script::CompiledId("Quester"),
                start_settings.clone(),
            )
        }),
        Box::new(|snapshot, _, _| {
            if let Some((x, z, level)) = snapshot.tile() {
                if (2816..=2879).contains(&x) && (9728..=9791).contains(&z) && level == 0 {
                    return Ok(Some(json!({
                        "event": "entrana-dungeon-entry",
                        "tile": {"x": x, "z": z, "level": level},
                        "stop_condition": "first observed tile inside the dungeon region"
                    })));
                }
            }
            Ok(None)
        }),
    )
    .expect("Lost City Path must reach the Entrana dungeon on the selected nav pack");
    println!("LOST-CITY-DUNGEON-ENTRY {receipt}");
}
#[cfg(test)]
mod tests {
    use super::{
        zanaris_fixture_items, zanaris_max_unit_price_at_zero_stock, zanaris_purchase_budget,
        PathDocument, ZANARIS_ROUTE_FARE_COUNT, ZANARIS_ROUTE_FARE_GP,
    };
    use serde_json::Value;

    fn sum_buy_estimates(value: &Value) -> u64 {
        let current = if value.get("kind").and_then(Value::as_str) == Some("buy") {
            value
                .get("args")
                .and_then(|args| args.get("est_gp"))
                .and_then(Value::as_u64)
                .expect("every Path buy step has an unsigned est_gp")
        } else {
            0
        };
        let nested = match value {
            Value::Array(values) => values.iter().map(sum_buy_estimates).sum(),
            Value::Object(fields) => fields.values().map(sum_buy_estimates).sum(),
            _ => 0,
        };
        current + nested
    }

    fn sum_buy_worst_case(value: &Value) -> u64 {
        let current = if value.get("kind").and_then(Value::as_str) == Some("buy") {
            let args = value.get("args").expect("buy steps have arguments");
            let obj = args
                .get("obj")
                .and_then(Value::as_str)
                .expect("buy steps name an object");
            let qty = args
                .get("qty")
                .and_then(Value::as_u64)
                .expect("buy steps have an unsigned quantity");
            let estimate = args
                .get("est_gp")
                .and_then(Value::as_u64)
                .expect("every Path buy step has an unsigned est_gp");
            let max_unit_price = match obj {
                "bronze_axe" => 19,
                "bread" => 14,
                other => panic!("add the content-backed zero-stock price for {other}"),
            };
            estimate.max(
                qty.checked_mul(max_unit_price)
                    .expect("Path purchase price does not overflow"),
            )
        } else {
            0
        };
        let nested = match value {
            Value::Array(values) => values.iter().map(sum_buy_worst_case).sum(),
            Value::Object(fields) => fields.values().map(sum_buy_worst_case).sum(),
            _ => 0,
        };
        current + nested
    }

    #[test]
    fn zanaris_zero_stock_prices_match_shop_content() {
        assert_eq!(zanaris_max_unit_price_at_zero_stock("bronze_axe"), Some(19));
        assert_eq!(zanaris_max_unit_price_at_zero_stock("bread"), Some(14));
    }

    #[test]
    fn zanaris_fixture_coin_seed_covers_path_worst_case_prices_and_route_fares() {
        let path: PathDocument =
            serde_json::from_str(include_str!("../../script/paths/289/zanaris.json"))
                .expect("embedded Zanaris Path is valid JSON");
        let raw_path = serde_json::to_value(&path).expect("Zanaris Path serializes");
        let required_estimate = sum_buy_estimates(&raw_path);
        let required_worst_case = sum_buy_worst_case(&raw_path);
        let route_fares = u64::try_from(ZANARIS_ROUTE_FARE_GP * ZANARIS_ROUTE_FARE_COUNT)
            .expect("route fare budget is nonnegative");
        let required_with_fares = required_worst_case
            .checked_add(route_fares)
            .expect("Zanaris fixture coin budget does not overflow");
        let (path_estimate, _) =
            zanaris_purchase_budget(&path).expect("Path purchases have content-backed prices");
        let (items, coin_seed) = zanaris_fixture_items(&path).expect("Path purchases are priced");
        let seeded = items
            .iter()
            .find_map(|(alias, quantity)| (*alias == "coins").then_some(*quantity))
            .expect("fixture seeds coins");
        assert_eq!(seeded, coin_seed);
        let seeded = u64::try_from(seeded).expect("coin seed is nonnegative");
        let path_estimate = u64::try_from(path_estimate).expect("Path estimate is nonnegative");

        assert_eq!(path_estimate, required_estimate);
        assert!(
            seeded >= required_estimate,
            "Zanaris fixture seeds {seeded} coins but its Path estimates need {required_estimate}"
        );
        assert!(
            seeded >= required_worst_case,
            "Zanaris fixture seeds {seeded} coins but depleted-shop purchases need {required_worst_case}"
        );
        assert!(
            seeded >= required_with_fares,
            "Zanaris fixture seeds {seeded} coins but depleted-shop purchases and route fares need {required_with_fares}"
        );
    }
}
