//! The Rust owner of the catalog hunting modules' tables and small rules
//! (`api/combat/hunting/{sites,logic,supply,guarded}.js` name-map here).
//!
//! - [`dragon_sites`]: the frozen dragon sites as a host coordinate table
//!   (content does not carry stands, safespots or gates).
//! - [`call`]: one typed local helper per frozen `logic.ts` / `guarded.ts`
//!   function and the supply reads (`escapeRunesFor`, `inCell`,
//!   `keyState`, `feePrepaid`). Bounded calculations over the caller's
//!   arguments and the isolate scene; nothing here sends an action.

use crate::hunt_fight::Tile;
use crate::observed;
use serde_json::{json, Value};

fn tile(x: i32, z: i32) -> Value {
    json!({ "x": x, "z": z, "level": 0 })
}

/// The frozen `DRAGON_SITES`, in its key order. `inArea` is each site's
/// `boxes` (their union); regexes are sources, matched case-insensitive.
pub(crate) fn dragon_sites() -> Value {
    let blue_box = area(2888, 2923, 9769, 9816);
    let taverley_blue = json!({
        "key": "taverley-blue",
        "label": "Taverley Dungeon blue dragons",
        "target": "Blue dragon",
        "bones": "Dragon bones",
        "keyItem": { "name": "Dusty key", "id": 1590 },
        "gate": { "locId": 2623, "op": "Open", "outside": tile(2924, 9803), "inside": tile(2923, 9803) },
        "approach": [tile(2911, 9809)],
        "safespots": [tile(2901, 9809), tile(2904, 9808), tile(2901, 9810)],
        "meleeAnchor": tile(2900, 9808),
        "bank": tile(2946, 3369),
        "escapeTeleportId": "falador",
        "walkOut": tile(2884, 3398),
        "boxes": [blue_box.clone()],
    });
    let mut taverley_black = taverley_blue.clone();
    merge(
        &mut taverley_black,
        json!({
            "key": "taverley-black",
            "label": "Taverley Dungeon black dragons",
            "target": "Black dragon",
            "lootSetting": "lootBlack",
            "food": "Shark",
            "antipoison": true,
            "approach": [tile(2893, 9790), tile(2882, 9768), tile(2860, 9803), tile(2845, 9815)],
            "safespots": [tile(2836, 9817), tile(2835, 9817), tile(2834, 9817)],
            "meleeAnchor": tile(2835, 9818),
            "boxes": [blue_box, area(2818, 2850, 9815, 9832), area(2835, 2896, 9762, 9820)],
        }),
    );
    let heroes_blue = json!({
        "key": "heroes-blue",
        "label": "Heroes' Guild blue dragon",
        "target": "Blue dragon",
        "bones": "Dragon bones",
        "keyItem": null,
        "gate": null,
        "approach": [tile(2892, 9908)],
        "safespots": [tile(2905, 9909), tile(2906, 9911), tile(2907, 9911)],
        "meleeAnchor": tile(2909, 9910),
        "bank": tile(2946, 3369),
        "escapeTeleportId": "falador",
        "walkOut": tile(2904, 3510),
        "boxes": [area(2886, 2942, 9883, 9917)],
    });
    let gutanoth_stands = json!([
        stand(
            "the north dragon at 2590,9461",
            [(2585, 9468), (2586, 9468), (2584, 9468)],
            (2588, 9468)
        ),
        stand(
            "the west dragon at 2568,9437",
            [(2573, 9429), (2572, 9429), (2574, 9429)],
            (2574, 9430)
        ),
        stand(
            "the north-west dragon at 2579,9445",
            [(2587, 9449), (2586, 9447), (2587, 9447)],
            (2586, 9449)
        ),
        stand(
            "the south dragon at 2592,9431",
            [(2591, 9423), (2590, 9423), (2591, 9422)],
            (2597, 9426)
        ),
        stand(
            "the east dragon at 2604,9443",
            [(2611, 9441), (2611, 9442), (2610, 9442)],
            (2610, 9441)
        ),
        stand(
            "the far east dragon at 2609,9459",
            [(2604, 9466), (2603, 9464), (2603, 9465)],
            (2606, 9466)
        ),
    ]);
    let gutanoth_blue = json!({
        "key": "gutanoth-blue",
        "label": "Gu'Tanoth Enclave blue dragons",
        "target": "Blue dragon",
        "bones": "Dragon bones",
        "keyItem": null,
        "gate": null,
        "talkGate": { "npc": "Enclave guard", "op": "Talk-to", "choose": "I want to go in there", "stand": tile(2508, 3038) },
        "approach": [tile(2588, 9432)],
        "safespots": gutanoth_stands[0]["tiles"].clone(),
        "meleeAnchor": gutanoth_stands[0]["anchor"].clone(),
        "stands": gutanoth_stands,
        "exit": { "locId": 2813, "op": "Enter", "stand": tile(2597, 9468) },
        "bank": tile(2612, 3092),
        "escapeTeleportId": "watchtower",
        "walkOut": tile(2540, 3054),
        "food": "Shark",
        "alsoHunt": ["Greater demon"],
        "lootSetting": "lootEnclave",
        "rangedThreat": true,
        "boxes": [area(2560, 2623, 9408, 9471)],
    });
    let iron_stands = json!([
        stand(
            "the north-east corner from 2744,9457",
            [(2744, 9457), (2743, 9457), (2744, 9458)],
            (2744, 9457)
        ),
        stand(
            "the south-west from 2704,9417",
            [(2704, 9417), (2703, 9417), (2705, 9417)],
            (2704, 9417)
        ),
    ]);
    let brimhaven_iron = json!({
        "key": "brimhaven-iron",
        "label": "Brimhaven Dungeon iron dragons",
        "target": "Iron dragon",
        "bones": "Dragon bones",
        "keyItem": null,
        "gate": null,
        "feeGate": {
            "npc": "Saniboch",
            "op": "Pay",
            "coins": 875,
            "stand": tile(2747, 3152),
            "entrance": { "locId": 5083, "op": "Enter" },
            "paidLine": "you pay saniboch 875 coins",
            "prepaidLine": "already given me lots of nice coins",
        },
        "approach": [tile(2691, 9564), tile(2649, 9562), tile(2672, 9499), tile(2682, 9506), tile(2698, 9500), tile(2698, 9492)],
        "safespots": iron_stands[0]["tiles"].clone(),
        "meleeAnchor": iron_stands[0]["anchor"].clone(),
        "stands": iron_stands,
        "exit": { "locId": 5084, "op": "leave", "stand": tile(2713, 9564) },
        "bank": tile(2655, 3283),
        "escapeTeleportId": "ardougne",
        "walkOut": tile(2745, 3152),
        "food": "Shark",
        "foodPerTrip": 8,
        "lootSetting": "lootIron",
        "alsoHunt": ["Steel dragon"],
        "fireAtRange": true,
        "rangedThreat": true,
        "antifire": true,
        "coins": 1000,
        "axe": true,
        "boxes": [area(2624, 2751, 9408, 9599)],
    });
    let steel_stands = json!([stand(
        "the west dragon from 2698,9440",
        [(2698, 9440), (2698, 9439), (2697, 9441)],
        (2698, 9440)
    )]);
    let mut brimhaven_steel = brimhaven_iron.clone();
    merge(
        &mut brimhaven_steel,
        json!({
            "key": "brimhaven-steel",
            "label": "Brimhaven Dungeon steel dragons",
            "target": "Steel dragon",
            "safespots": steel_stands[0]["tiles"].clone(),
            "meleeAnchor": steel_stands[0]["anchor"].clone(),
            "stands": steel_stands,
            "alsoHunt": ["Iron dragon"],
            "lootSetting": "lootSteel",
        }),
    );
    json!([
        taverley_blue,
        taverley_black,
        heroes_blue,
        gutanoth_blue,
        brimhaven_iron,
        brimhaven_steel
    ])
}

fn area(min_x: i32, max_x: i32, min_z: i32, max_z: i32) -> Value {
    json!({ "minX": min_x, "maxX": max_x, "minZ": min_z, "maxZ": max_z, "level": 0 })
}

fn stand(label: &str, tiles: [(i32, i32); 3], anchor: (i32, i32)) -> Value {
    json!({
        "label": label,
        "tiles": tiles.iter().map(|&(x, z)| tile(x, z)).collect::<Vec<_>>(),
        "anchor": tile(anchor.0, anchor.1),
    })
}

fn merge(base: &mut Value, over: Value) {
    if let (Value::Object(base), Value::Object(over)) = (base, over) {
        base.extend(over);
    }
}

const SAFESPOT_BLIND_MS: f64 = 20_000.0;
const ANTIFIRE_MARGIN_TICKS: f64 = 20.0;
const PRAYER_SIP_FLOOR: f64 = 8.0;
const PRAYER_SIP_FRACTION: f64 = 0.15;
const LOOT_GUARD: f64 = 4.0;
const PROTECT_FROM_MELEE: &str = "Protect from Melee";
const SHIELD_AT_RANGE: &str = "the metal dragons breathe fire at range, so every style here wears the Dragonfire shield, and there is none in the bank or worn. Duke Horacio in Lumbridge Castle hands one out free.";
const SHIELD_MELEE: &str = "melee needs the Dragonfire shield and there is none in the bank or worn. Duke Horacio in Lumbridge Castle hands one out free, or switch to mage or range, which fight from a fire-proof safespot.";
const RANGE_REFUSED: &str = "range cannot fight the metal dragons: every style here wears the Dragonfire shield against the far breath, and a bow needs both hands. Switch to mage or melee.";

/// The jail cell (frozen `CELL`), level 0.
const CELL: (i32, i32, i32, i32) = (2928, 2934, 9683, 9689);

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(f64::NAN)
}

fn flag(v: Option<&Value>) -> bool {
    crate::hunt::truthy(v.unwrap_or(&Value::Null))
}

fn field<'a>(v: Option<&'a Value>, key: &str) -> Option<&'a Value> {
    v.and_then(|v| v.get(key))
}

fn spots(v: Option<&Value>) -> Vec<(f64, f64)> {
    v.and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| (num(row.get("x")), num(row.get("z"))))
                .collect()
        })
        .unwrap_or_default()
}

fn spot(v: Option<&Value>) -> (f64, f64) {
    (num(field(v, "x")), num(field(v, "z")))
}

fn nearest(from: (f64, f64), spots: &[(f64, f64)]) -> usize {
    let mut best = 0;
    let mut best_dist = f64::INFINITY;
    for (i, s) in spots.iter().enumerate() {
        let dist = (s.0 - from.0).abs().max((s.1 - from.1).abs());
        if dist < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    best
}

fn body_origin(tile: (f64, f64), size: f64) -> (f64, f64) {
    let back = ((size as i64) >> 1) as f64;
    (tile.0 - back, tile.1 - back)
}

fn gap_to(from: (f64, f64), tile: (f64, f64), size: f64) -> f64 {
    let o = body_origin(tile, size);
    let dx = (o.0 - from.0).max(from.0 - (o.0 + size - 1.0)).max(0.0);
    let dz = (o.1 - from.1).max(from.1 - (o.1 + size - 1.0)).max(0.0);
    dx.max(dz)
}

fn chase(style: &str, fire_at_range: bool) -> bool {
    style == "melee" && fire_at_range
}

fn attack_range(style: &str) -> Value {
    match style {
        "melee" => json!(1),
        "range" => json!(7),
        "mage" => json!(10),
        _ => Value::Null,
    }
}

/// `CLUE_DB[id] !== undefined || CASKET_IDS[id] !== undefined`: the
/// selected trail facts are this revision's clue scrolls and caskets.
fn is_clue_obj(id: i32) -> bool {
    crate::supply_v2::selected_data()
        .and_then(|data| {
            data.trails().map(|facts| {
                facts.rows.iter().any(|row| row.id == id)
                    || facts.challenge_answers.iter().any(|row| row.id == id)
            })
        })
        .unwrap_or(false)
}

fn in_cell() -> bool {
    observed::with(|scene| {
        scene.since_login().here().is_some_and(|here| {
            let t = Tile::from(here);
            t.level == 0 && t.x >= CELL.0 && t.x <= CELL.1 && t.z >= CELL.2 && t.z <= CELL.3
        })
    })
}

/// Frozen `keyStatus(Inventory.countById(id), Bank.countById(id))` off the
/// posted pack and bank pages (a shut bank posts no rows).
fn key_state(id: i32) -> &'static str {
    let (held, banked) = observed::with(|scene| {
        let session = scene.since_login();
        let count = |rows: Option<&Vec<observed::ItemRow>>| -> i64 {
            rows.map(|rows| {
                rows.iter()
                    .filter(|row| row.id == id)
                    .map(|row| i64::from(row.count.max(0)))
                    .sum()
            })
            .unwrap_or(0)
        };
        (count(session.inv()), count(session.bank()))
    });
    key_status(held as f64, banked as f64)
}

fn key_status(held: f64, banked: f64) -> &'static str {
    if held > 0.0 {
        "held"
    } else if banked > 0.0 {
        "bank"
    } else {
        "fetch"
    }
}

/// One frozen hunting function over its JSON arguments.
pub(crate) fn call(name: &str, args: &[Value]) -> Result<Value, String> {
    let a = |i: usize| args.get(i);
    let text = |i: usize| a(i).and_then(Value::as_str).unwrap_or("").to_string();
    Ok(match name {
        "sites" => dragon_sites(),
        "nextSafespot" => {
            let s = a(0);
            let (index, spots) = (num(field(s, "index")), num(field(s, "spots")));
            if spots <= 1.0 {
                json!(0)
            } else if !flag(field(s, "hurt")) && num(field(s, "blindMs")) < SAFESPOT_BLIND_MS {
                json!(index)
            } else {
                json!((index + 1.0) % spots)
            }
        }
        "hurtOnSpot" => {
            let s = a(0);
            let (last, hp) = (num(field(s, "lastHp")), num(field(s, "hp")));
            json!(
                !flag(field(s, "rangedThreat"))
                    && flag(field(s, "onSpot"))
                    && last >= 0.0
                    && hp < last
            )
        }
        "retreatDue" => {
            let s = a(0);
            let retreat = num(field(s, "retreatHp"));
            if !flag(field(s, "inLair"))
                || num(field(s, "spots")) <= 0.0
                || flag(field(s, "onSafespot"))
                || retreat <= 0.0
            {
                json!(false)
            } else {
                json!(!flag(field(s, "hasFood")) || num(field(s, "hpFrac")) < retreat)
            }
        }
        "lootHalts" => {
            let s = a(0);
            let (hp, panic, retreat) = (
                num(field(s, "hpFrac")),
                num(field(s, "panicHp")),
                num(field(s, "retreatHp")),
            );
            json!(hp < panic || (retreat > 0.0 && hp < retreat))
        }
        "holdDue" => {
            let s = a(0);
            json!(flag(field(s, "hasFood")) || !flag(field(s, "onSafespot")))
        }
        "nearestSpot" => json!(nearest(spot(a(0)), &spots(a(1)))),
        "nextApproachIndex" => json!(nearest(spot(a(1)), &spots(a(0)))),
        "bodyOrigin" => {
            let (x, z) = body_origin(spot(a(0)), num(a(1)));
            json!({ "x": x, "z": z })
        }
        "noteSighting" => {
            let prev = a(0).filter(|v| v.is_object());
            let tile = spot(a(1));
            let now = a(2).cloned().unwrap_or(Value::Null);
            match prev {
                Some(prev) if spot(Some(prev)) == tile => {
                    let mut next = prev.clone();
                    next["at"] = now;
                    next
                }
                _ => json!({ "x": tile.0, "z": tile.1, "since": now, "at": now }),
            }
        }
        "settled" => {
            let s = a(0).filter(|v| v.is_object());
            json!(s.is_some_and(|s| num(a(1)) - num(s.get("since")) >= num(a(2))))
        }
        "retreatAim" => {
            let s = a(0);
            let spots = spots(field(s, "spots"));
            let index = field(s, "rotated")
                .and_then(Value::as_f64)
                .unwrap_or_else(|| nearest(spot(field(s, "from")), &spots) as f64);
            let next = if spots.is_empty() {
                index
            } else {
                (index + 1.0) % spots.len() as f64
            };
            json!({ "index": index, "next": next })
        }
        "chaseMode" => json!(chase(&text(0), flag(a(1)))),
        "prayerFor" => {
            if chase(&text(0), flag(a(1))) {
                json!(PROTECT_FROM_MELEE)
            } else {
                Value::Null
            }
        }
        "prayerSipDue" => {
            let (points, max) = (num(a(0)), num(a(1)));
            json!(max > 0.0 && points <= PRAYER_SIP_FLOOR.max((max * PRAYER_SIP_FRACTION).floor()))
        }
        "lootReach" => json!(if flag(a(0)) { 14 } else { 10 }),
        "attackRangeFor" => attack_range(&text(0)),
        "engageRangeFor" => {
            let style = text(0);
            match attack_range(&style).as_f64() {
                Some(range) if style != "melee" => json!(range - 1.0),
                Some(range) => json!(range),
                None => json!(f64::NAN),
            }
        }
        "gapTo" => json!(gap_to(spot(a(0)), spot(a(1)), num(a(2)))),
        "shieldGate" => {
            let (style, at_range) = (text(0), flag(a(1)));
            if flag(a(2)) || (style != "melee" && !at_range) {
                Value::Null
            } else if at_range {
                json!(SHIELD_AT_RANGE)
            } else {
                json!(SHIELD_MELEE)
            }
        }
        "styleGate" => {
            if text(0) != "range" || !flag(a(1)) {
                Value::Null
            } else {
                json!(RANGE_REFUSED)
            }
        }
        "antifireDue" => {
            let s = a(0);
            json!(
                flag(field(s, "inLair"))
                    && num(field(s, "tick")) >= num(field(s, "until")) - ANTIFIRE_MARGIN_TICKS
            )
        }
        "antifireLapsed" => json!(flag(a(0)) && !flag(a(1))),
        "isClueObj" => json!(a(0)
            .and_then(Value::as_i64)
            .is_some_and(|id| { i32::try_from(id).is_ok_and(is_clue_obj) })),
        "wantsDrop" => {
            let (item, f) = (a(0), a(1));
            let id = field(item, "id").and_then(Value::as_i64).unwrap_or(-1);
            let id = i32::try_from(id).unwrap_or(-1);
            if flag(field(f, "solveClues")) && is_clue_obj(id) {
                return Ok(json!(true));
            }
            let name = field(item, "name").and_then(Value::as_str).unwrap_or("");
            if name.is_empty() {
                return Ok(json!(false));
            }
            let lower = name.to_lowercase();
            let bone = field(f, "boneName").and_then(Value::as_str).unwrap_or("");
            if flag(field(f, "buryBones")) && lower == bone.to_lowercase() {
                return Ok(json!(true));
            }
            let listed = field(f, "loot")
                .and_then(Value::as_array)
                .is_some_and(|loot| loot.iter().any(|l| l.as_str() == Some(lower.as_str())));
            json!(
                listed
                    || (flag(field(f, "bankCommon"))
                        && api::content::matches_common_bank_loot(name, id))
            )
        }
        "keyStatus" => json!(key_status(num(a(0)), num(a(1)))),
        "guarded" => {
            let drop = spot(a(0));
            let radius = a(2).and_then(Value::as_f64).unwrap_or(LOOT_GUARD);
            let bodies = a(1).and_then(Value::as_array).cloned().unwrap_or_default();
            json!(bodies
                .iter()
                .any(|b| gap_to(drop, spot(b.get("tile")), num(b.get("size"))) <= radius))
        }
        "escapeRunesFor" => {
            let id = text(0);
            match crate::escape_runes::escape_runes_for_optional(
                crate::supply_v2::selected_data().as_deref(),
                &id,
            ) {
                Ok(fact) => json!({
                    "runes": fact.runes.iter().map(|r| json!({ "rune": r.rune, "count": r.count })).collect::<Vec<_>>(),
                    "level": fact.level,
                    "label": fact.label,
                }),
                // Frozen: an id the catalog does not name casts nothing.
                Err(_) => json!({ "runes": [], "level": 0, "label": id }),
            }
        }
        "inCell" => json!(in_cell()),
        "keyState" => match a(0)
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
        {
            Some(id) => json!(key_state(id)),
            None => json!("held"),
        },
        _ => return Err(format!("not impl: hunting {name}")),
    })
}
