//! TaskBot `validate` for PeriodicBank and DeathRecovery as one native
//! call each. The shim passes the caller's options object; Rust calls its
//! functions in the frozen order through the one callback path and reads
//! the scene for the rest.
//!
//! - `__rs2b0t_periodic_bank_validate(opts)`: frozen `PeriodicBank.validate`
//!   (`strategy()`, combat, the failure backoff, then `strategy()`,
//!   `countLoot()`, `itemsThreshold()`, `minutesThreshold()` into
//!   `shouldBankNow`). An absent option keeps the shim's default (`'off'`,
//!   0, 15, 10).
//! - `__rs2b0t_death_recovery_validate(instance, opts)`: observe posted chat
//!   for this task instance; call `opts.onDeath()` on a new death; after a
//!   finished run, clear its latch and call `opts.onRecovered()` when the
//!   posted tile is on the anchor's level and within its normalized radius;
//!   answer due.
//! - `__rs2b0t_next_withdraw_chunk(need)`: frozen `nextWithdrawChunk`
//!   ([`crate::bank::ops::next_chunk`]), `null` or `{ kind, count }` /
//!   `{ kind, op }`.
//! - `__rs2b0t_withdraw_op(ops, amount)`: frozen `withdrawOp`
//!   ([`crate::bank::compat::withdraw_op`]), the matching op label or `null`.

use super::callback_v8::{self as cb, Callback, JsResult};
use crate::bank::ops::same_name;
use crate::periodic_bank::{
    minutes_since_last_bank, parse_bank_strategy, should_bank_now, suppressed,
};
use rustyscript::Runtime;

const BANK_SORT_CATEGORY_ORDER: [&str; 19] = [
    "coins",
    "runes",
    "staves",
    "ammunition",
    "weapons",
    "armour",
    "rangedArmour",
    "food",
    "potions",
    "jewellery",
    "herbs",
    "oresBarsGems",
    "logs",
    "supplies",
    "tools",
    "teleports",
    "questLive",
    "questObsolete",
    "junk",
];

pub(super) fn install(runtime: &mut Runtime) -> Result<(), String> {
    cb::install(
        runtime,
        "__rs2b0t_periodic_bank_validate",
        periodic_bank_validate,
    )?;
    cb::install(
        runtime,
        "__rs2b0t_death_recovery_validate",
        death_recovery_validate,
    )?;
    cb::install(runtime, "__rs2b0t_next_withdraw_chunk", next_withdraw_chunk)?;
    cb::install(runtime, "__rs2b0t_withdraw_op", withdraw_op)?;
    cb::install(runtime, "__rs2b0t_periodic_bank_rules", periodic_bank_rules)?;
    cb::install(
        runtime,
        "__rs2b0t_bank_sort_category_order",
        bank_sort_category_order,
    )?;
    cb::install(
        runtime,
        "__rs2b0t_bank_sort_category_of",
        bank_sort_category_of,
    )?;
    cb::install(
        runtime,
        "__rs2b0t_bank_sort_is_unmatched",
        bank_sort_is_unmatched,
    )?;
    cb::install(runtime, "__rs2b0t_bank_snapshot", bank_snapshot)?;
    cb::install(runtime, "__rs2b0t_deposit_matcher", deposit_matcher)?;
    cb::install(
        runtime,
        "__rs2b0t_deposit_all_except",
        deposit_all_except,
    )?;
    cb::install(
        runtime,
        "__rs2b0t_periodic_bank_settings",
        periodic_bank_settings,
    )
}

/// Frozen `withdrawOp(ops, amount)` (`api/bank/bankOps.ts:5-21`): the first
/// posted label matching the amount, `null` when none does, `undefined` for
/// an amount outside the frozen cases (the `switch` falls through).
fn withdraw_op<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    use crate::bank::compat::{withdraw_op, WithdrawAmount};
    let result = (|| {
        let amount = args.get(1);
        let amount = amount
            .is_string()
            .then(|| amount.to_rust_string_lossy(scope))
            .and_then(|amount| WithdrawAmount::parse(&amount));
        let ops = args.get(0);
        let Ok(list) = v8::Local::<v8::Array>::try_from(ops) else {
            return Err(cb::type_error(scope, "ops.filter is not a function"));
        };
        let Some(amount) = amount else {
            return Ok(v8::undefined(scope).into());
        };
        let mut labels = Vec::with_capacity(list.length() as usize);
        for index in 0..list.length() {
            let op = cb::get_index(scope, ops, index)?;
            if op.is_string() {
                labels.push(op.to_rust_string_lossy(scope));
            }
        }
        Ok(
            match withdraw_op(labels.iter().map(String::as_str), amount) {
                Some(found) => cb::string(scope, &labels[found]),
                None => v8::null(scope).into(),
            },
        )
    })();
    cb::finish(scope, rv, result);
}

fn next_withdraw_chunk<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    use crate::bank::ops::{next_chunk, Chunk};
    let result = cb::number(scope, args.get(0)).and_then(|need| match next_chunk(need) {
        None => Ok(v8::null(scope).into()),
        Some(Chunk::X(count)) => {
            let kind = cb::string(scope, "x");
            let count = cb::num(scope, count);
            cb::object(scope, &[("kind", kind), ("count", count)])
        }
        Some(Chunk::Op(op)) => {
            let kind = cb::string(scope, "op");
            let op = cb::string(scope, op);
            cb::object(scope, &[("kind", kind), ("op", op)])
        }
    });
    cb::finish(scope, rv, result);
}

/// `opts[key]()` when it is a function (receiver `opts`), else `None`.
fn call_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
    key: &'static str,
) -> JsResult<'s, Option<v8::Local<'s, v8::Value>>> {
    let func = cb::get(scope, opts, key)?;
    if !func.is_function() {
        return Ok(None);
    }
    Callback::method(func, opts, key).call(scope, &[]).map(Some)
}

fn number_option<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
    key: &'static str,
    absent: f64,
) -> JsResult<'s, f64> {
    match call_option(scope, opts, key)? {
        Some(value) => cb::number(scope, value),
        None => Ok(absent),
    }
}

/// The strategy string, or `None` for a non-string or an absent option.
fn strategy<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<String>> {
    Ok(call_option(scope, opts, "strategy")?
        .filter(|value| value.is_string())
        .map(|value| value.to_rust_string_lossy(scope)))
}

fn periodic_due<'s>(
    scope: &mut v8::HandleScope<'s>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    // The shim's absent strategy is 'off'.
    let first = match call_option(scope, opts, "strategy")? {
        None => return Ok(false),
        Some(value) => value,
    };
    let off = first.is_string() && first.to_rust_string_lossy(scope) == "off";
    let in_combat = crate::observed::with(|scene| scene.since_login().in_combat().unwrap_or(false));
    if off || in_combat || suppressed() {
        return Ok(false);
    }
    let strategy = parse_bank_strategy(strategy(scope, opts)?.as_deref().unwrap_or("off"));
    let loot = number_option(scope, opts, "countLoot", 0.0)?;
    let minutes = minutes_since_last_bank();
    let items_threshold = number_option(scope, opts, "itemsThreshold", 15.0)?;
    let minutes_threshold = number_option(scope, opts, "minutesThreshold", 10.0)?;
    Ok(should_bank_now(
        strategy,
        loot,
        items_threshold,
        minutes,
        minutes_threshold,
    ))
}

fn periodic_bank_rules<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| match args.get(0).to_rust_string_lossy(scope).as_str() {
        "parseBankStrategy" => {
            let label = args.get(1);
            let label = if cb::truthy(scope, label) {
                cb::to_string(scope, label)?
            } else {
                String::new()
            };
            let strategy = match parse_bank_strategy(&label) {
                crate::periodic_bank::BankStrategy::Off => "off",
                crate::periodic_bank::BankStrategy::Items => "items",
                crate::periodic_bank::BankStrategy::Time => "time",
                crate::periodic_bank::BankStrategy::Either => "either",
            };
            Ok(cb::string(scope, strategy))
        }
        _ => Err(cb::type_error(scope, "unknown periodic bank rule")),
    })();
    cb::finish(scope, rv, result);
}

fn periodic_bank_validate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = periodic_due(scope, args.get(0)).map(|due| v8::Boolean::new(scope, due).into());
    cb::finish(scope, rv, result);
}

fn death_due<'s>(
    scope: &mut v8::HandleScope<'s>,
    instance: v8::Local<'s, v8::Object>,
    opts: v8::Local<'s, v8::Value>,
) -> JsResult<'s, bool> {
    use crate::death_recovery as death;
    let observed = death::observe(scope, instance);
    if observed.died {
        call_option(scope, opts, "onDeath")?;
    }
    if let Some(home) = observed.check_from {
        // Frozen `near(home, this.opts.anchor, this.opts.radius ?? 6)`:
        // level must match, and both coordinates use the execute radius rule.
        let anchor = cb::get(scope, opts, "anchor")?;
        let radius = cb::get(scope, opts, "radius")?;
        let radius = if radius.is_null_or_undefined() || !radius.is_number() {
            None
        } else {
            radius.number_value(scope)
        };
        let radius = f64::from(death::normalized_radius(radius));
        let level = cb::get(scope, anchor, "level")?;
        let level = level
            .is_number()
            .then(|| level.number_value(scope).unwrap_or(f64::NAN));
        let near = if level == Some(f64::from(home.level)) {
            let x = cb::get(scope, anchor, "x")?;
            let x = cb::number(scope, x)?;
            if (f64::from(home.x) - x).abs() <= radius {
                let z = cb::get(scope, anchor, "z")?;
                let z = cb::number(scope, z)?;
                death::near(home, level, x, z, radius)
            } else {
                false
            }
        } else {
            false
        };
        if near {
            death::recover(scope, instance);
            call_option(scope, opts, "onRecovered")?;
        }
    }
    Ok(death::due(scope, instance))
}

fn death_recovery_validate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = match args.get(0).try_cast::<v8::Object>() {
        Ok(instance) => {
            death_due(scope, instance, args.get(1)).map(|due| v8::Boolean::new(scope, due).into())
        }
        Err(_) => Ok(v8::Boolean::new(scope, false).into()),
    };
    cb::finish(scope, rv, result);
}
fn bank_sort_category_order<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let values: [v8::Local<'s, v8::Value>; 19] =
        std::array::from_fn(|index| cb::string(scope, BANK_SORT_CATEGORY_ORDER[index]));
    let result = Ok(v8::Array::new_with_elements(scope, &values).into());
    cb::finish(scope, rv, result);
}

fn bank_sort_category_of<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let category = bank_sort_rule(scope, args.get(0))?.unwrap_or("junk");
        Ok(cb::string(scope, category))
    })();
    cb::finish(scope, rv, result);
}

fn bank_sort_is_unmatched<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| {
        let unmatched = bank_sort_rule(scope, args.get(0))?.is_none();
        Ok(v8::Boolean::new(scope, unmatched).into())
    })();
    cb::finish(scope, rv, result);
}

fn bank_sort_rule<'s>(
    scope: &mut v8::HandleScope<'s>,
    item: v8::Local<'s, v8::Value>,
) -> JsResult<'s, Option<&'static str>> {
    let name_value = cb::get(scope, item, "name")?;
    let name = if name_value.is_null_or_undefined() {
        String::new()
    } else {
        let lowered = cb::call_method(
            scope,
            name_value,
            "toLowerCase",
            &[],
            "item.name.toLowerCase",
        )?;
        if !lowered.is_string() {
            return Err(cb::type_error(
                scope,
                "item.name.toLowerCase did not return a string",
            ));
        }
        lowered.to_rust_string_lossy(scope)
    };

    let id_value = cb::get(scope, item, "id")?;
    let id = id_value
        .is_number()
        .then(|| id_value.number_value(scope))
        .flatten()
        .filter(|id| (0.0..=u32::MAX as f64).contains(id) && id.fract() == 0.0)
        .map(|id| id as u32)
        .unwrap_or(u32::MAX);

    Ok(bank_sort_rule_for(id, &name))
}

fn bank_sort_rule_for(id: u32, name: &str) -> Option<&'static str> {
    if id == 995 {
        return Some("coins");
    }
    if [
        554, 555, 556, 557, 558, 559, 560, 561, 562, 563, 564, 565, 566, 1436,
    ]
    .contains(&id)
    {
        return Some("runes");
    }
    if contains_any(
        name,
        &[
            "arrow",
            "bolt",
            "dart",
            "javelin",
            "knife",
            "knives",
            "cannonball",
        ],
    ) {
        return Some("ammunition");
    }
    if name.contains("teleport") {
        return Some("teleports");
    }
    if contains_any(
        name,
        &[
            "potion",
            "vial",
            "brew",
            "antipoison",
            "restore",
            "antifire",
            "super attack",
            "super strength",
            "super defence",
            "super energy",
        ],
    ) {
        return Some("potions");
    }
    if contains_any(name, &["amulet", "necklace", "bracelet"]) || contains_word(name, "ring") {
        return Some("jewellery");
    }
    if contains_any(
        name,
        &[
            "pickaxe",
            "hatchet",
            "hammer",
            "chisel",
            "tinderbox",
            "needle",
            "spade",
            "rope",
            "harpoon",
            "lobster pot",
            "fishing rod",
            "fishing net",
            "bucket",
            "jug",
            "tiara",
        ],
    ) || (ends_with_word(name, "axe") && !name.contains("battleaxe"))
    {
        return Some("tools");
    }
    if contains_any(name, &["bowstring", "bow string"]) {
        return Some("supplies");
    }
    if name.contains("staff") {
        return Some("staves");
    }
    if contains_any(
        name,
        &[
            "scimitar",
            "mace",
            "dagger",
            "battleaxe",
            "warhammer",
            "halberd",
            "spear",
            "crossbow",
            "wand",
            "claws",
            "whip",
        ],
    ) || contains_sword_not_fish(name)
        || contains_word_after(name, "bow")
    {
        return Some("weapons");
    }
    if contains_any(
        name,
        &[
            "dragonhide body",
            "dragonhide chaps",
            "dragon vambraces",
            "studded ",
            "hardleather",
            "leather ",
            "coif",
        ],
    ) {
        return Some("rangedArmour");
    }
    if contains_any(
        name,
        &[
            "platebody",
            "plateskirt",
            "platelegs",
            "chainbody",
            "helm",
            "shield",
            "boots",
            "gloves",
            "gauntlets",
            "vambraces",
            "chaps",
            "cape",
            "cloak",
            "robe",
            "hat",
            "body",
            "legs",
            "skirt",
        ],
    ) {
        return Some("armour");
    }
    if contains_any(
        name,
        &[
            "shark",
            "lobster",
            "swordfish",
            "tuna",
            "trout",
            "salmon",
            "bass",
            "pike",
            "shrimps",
            "anchovies",
            "herring",
            "sardine",
            "mackerel",
            "manta ray",
            "bread",
            "cake",
            "stew",
            "meat",
            "chicken",
            "kebab",
            "wine",
            "beer",
        ],
    ) || contains_word(name, "cod")
        || contains_word(name, "pie")
        || name.contains("strange fruit")
    {
        return Some("food");
    }
    if contains_any(
        name,
        &[
            "grimy",
            "clean ",
            "guam",
            "marrentill",
            "tarromin",
            "harralander",
            "ranarr",
            "irit",
            "avantoe",
            "kwuarm",
            "cadantine",
            "dwarf weed",
            "torstol",
            "lantadyme",
            "snapdragon",
            "snake weed",
            "ardrigal",
            "sito foil",
            "volencia moss",
            "rogues purse",
            "eye of newt",
            "unicorn horn",
            "limpwurt",
            "snape grass",
            "white berries",
            "red spiders",
            "chocolate dust",
            "toadflax",
        ],
    ) || name == "herb"
    {
        return Some("herbs");
    }
    if name == "half of a key" || name == "crystal key" {
        return Some("oresBarsGems");
    }
    if contains_any(
        name,
        &[
            "sapphire",
            "emerald",
            "ruby",
            "diamond",
            "dragonstone",
            "opal",
            "jade",
            "topaz",
        ],
    ) || ["ore", "bar", "bars", "coal", "gem"]
        .iter()
        .any(|word| contains_word(name, word))
    {
        return Some("oresBarsGems");
    }
    if contains_any(name, &["logs", "plank"]) {
        return Some("logs");
    }
    if contains_any(
        name,
        &[
            "feather",
            "leather",
            "hide",
            "thread",
            "bowstring",
            "bow string",
            "seed",
            "bait",
            "shaft",
            "wool",
            "flax",
            "molten glass",
            "soda ash",
        ],
    ) {
        return Some("supplies");
    }
    if id == 526 || id == 532 {
        return Some("junk");
    }
    None
}

fn contains_any(name: &str, fragments: &[&str]) -> bool {
    fragments.iter().any(|fragment| name.contains(fragment))
}

fn is_js_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn contains_word(name: &str, word: &str) -> bool {
    name.match_indices(word).any(|(start, matched)| {
        let end = start + matched.len();
        (start == 0 || !is_js_word_byte(name.as_bytes()[start - 1]))
            && (end == name.len() || !is_js_word_byte(name.as_bytes()[end]))
    })
}

fn ends_with_word(name: &str, suffix: &str) -> bool {
    name.ends_with(suffix) && {
        let start = name.len() - suffix.len();
        start == 0 || !is_js_word_byte(name.as_bytes()[start - 1])
    }
}

fn contains_word_after(name: &str, fragment: &str) -> bool {
    name.match_indices(fragment).any(|(start, matched)| {
        let end = start + matched.len();
        end == name.len() || !is_js_word_byte(name.as_bytes()[end])
    })
}

fn contains_sword_not_fish(name: &str) -> bool {
    name.match_indices("sword")
        .any(|(start, matched)| !name[start + matched.len()..].starts_with("fish"))
}
fn bank_snapshot<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let operation = args.get(0);
        if !operation.is_string() {
            return Err(cb::type_error(
                scope,
                "bank snapshot operation must be a string",
            ));
        }
        let operation = operation.to_rust_string_lossy(scope);
        let name = if operation == "count" {
            let name = args.get(1);
            if !name.is_string() {
                return Err(cb::type_error(scope, "bank count name must be a string"));
            }
            Some(name.to_rust_string_lossy(scope))
        } else {
            None
        };
        let wanted_id = if operation == "countById" {
            let id = args.get(1);
            if id.is_number() {
                id.number_value(scope).unwrap_or(f64::NAN)
            } else {
                f64::NAN
            }
        } else {
            f64::NAN
        };

        crate::observed::with(|scene| {
            let latest = scene.latest();
            let rows = latest.bank().map_or(&[][..], Vec::as_slice);
            match operation.as_str() {
                "isOpen" => Ok(v8::Boolean::new(scope, latest.bank_open().unwrap_or(false)).into()),
                "loaded" => Ok(v8::Boolean::new(scope, !rows.is_empty()).into()),
                "ready" => Ok(v8::Boolean::new(
                    scope,
                    latest.bank_open().unwrap_or(false)
                        && (latest.bank_loaded().unwrap_or(false) || !rows.is_empty()),
                )
                .into()),
                "snapshotReady" => {
                    Ok(v8::Boolean::new(scope, latest.bank_loaded().unwrap_or(false)).into())
                }
                "snapshotGeneration" => {
                    let generation = latest.bank_snapshot_generation().unwrap_or(-1);
                    Ok(cb::num(
                        scope,
                        if generation >= 0 {
                            generation as f64
                        } else {
                            -1.0
                        },
                    ))
                }
                "items" => bank_items(scope, rows),
                "count" => {
                    let name = name.as_deref().unwrap_or_default();
                    let total: f64 = rows
                        .iter()
                        .filter_map(|row| {
                            row.name
                                .as_deref()
                                .filter(|actual| same_name(actual, name))
                                .map(|_| f64::from(row.count))
                        })
                        .sum();
                    Ok(cb::num(scope, total))
                }
                "countById" => {
                    if rows.is_empty() {
                        return Ok(cb::num(scope, 0.0));
                    }
                    if !rows.iter().any(|row| row.id != 0) {
                        return Err(cb::not_impl(scope, "Bank.countById"));
                    }
                    let total: f64 = rows
                        .iter()
                        .filter(|row| f64::from(row.id) == wanted_id)
                        .map(|row| f64::from(row.count))
                        .sum();
                    Ok(cb::num(scope, total))
                }
                _ => Err(cb::type_error(scope, "unknown bank snapshot query")),
            }
        })
    })();
    cb::finish(scope, rv, result);
}

fn bank_items<'s>(
    scope: &mut v8::HandleScope<'s>,
    rows: &[crate::observed::ItemRow],
) -> JsResult<'s, v8::Local<'s, v8::Value>> {
    let items = v8::Array::new(scope, 0);
    for (index, row) in rows.iter().enumerate() {
        let index = u32::try_from(index)
            .map_err(|_| cb::type_error(scope, "bank items exceed the V8 array limit"))?;
        let ops = v8::Array::new(scope, 0);
        for (op_index, op) in row.ops.iter().enumerate() {
            let op_index = u32::try_from(op_index)
                .map_err(|_| cb::type_error(scope, "bank ops exceed the V8 array limit"))?;
            let label = cb::string(scope, op);
            if !cb::catching(scope, |s| ops.set_index(s, op_index, label))? {
                return Err(cb::type_error(scope, "could not materialize bank item ops"));
            }
        }
        let name = match row.name.as_deref() {
            Some(name) => cb::string(scope, name),
            None => v8::null(scope).into(),
        };
        let count = cb::num(scope, f64::from(row.count));
        let id = cb::num(scope, f64::from(row.id));
        let slot = cb::num(scope, f64::from(row.slot.unwrap_or(-1)));
        let component_id = cb::num(scope, f64::from(row.component_id.unwrap_or(-1)));
        let noted = v8::Boolean::new(scope, row.noted).into();
        let cert = cb::num(scope, f64::from(row.cert));
        let item = cb::object(
            scope,
            &[
                ("name", name),
                ("count", count),
                ("id", id),
                ("slot", slot),
                ("comId", component_id),
                ("ops", ops.into()),
                ("noted", noted),
                ("cert", cert),
            ],
        )?;
        if !cb::catching(scope, |s| items.set_index(s, index, item))? {
            return Err(cb::type_error(scope, "could not materialize bank items"));
        }
    }
    Ok(items.into())
}

fn deposit_matcher<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let own = args.get(0);
        if !own.is_function() {
            let message = v8::String::new(
                scope,
                "not impl: depositMatcher: requires a function",
            )
            .unwrap_or_else(|| v8::String::empty(scope));
            return Err(cb::Throw::Value(v8::Exception::error(scope, message)));
        }
        let include_common = cb::truthy(scope, args.get(1));
        let include_common = v8::Boolean::new(scope, include_common).into();
        let common = args.get(2);
        let data = cb::object(
            scope,
            &[
                ("own", own),
                ("includeCommon", include_common),
                ("common", common),
            ],
        )?;
        let Some(function) = v8::Function::builder(deposit_matcher_predicate)
            .data(data)
            .build(scope)
        else {
            return Err(cb::type_error(
                scope,
                "could not create bank deposit matcher",
            ));
        };
        Ok(function.into())
    })();
    cb::finish(scope, rv, result);
}

fn deposit_matcher_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let data = args.data();
        let own = cb::get(scope, data, "own")?;
        let own = Callback::plain(scope, own, "own");
        let name = args.get(0);
        let own_result = own.call(scope, &[name])?;
        if cb::truthy(scope, own_result) {
            return Ok(own_result);
        }

        let include_common = cb::get(scope, data, "includeCommon")?;
        if !cb::truthy(scope, include_common) {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let common = cb::get(scope, data, "common")?;
        Callback::plain(scope, common, "matchesCommonBankLoot")
            .call(scope, &[name, args.get(1)])
    })();
    cb::finish(scope, rv, result);
}

fn deposit_all_except<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let keep = args.get(0);
        let keep = if cb::truthy(scope, keep) {
            keep
        } else {
            v8::Array::new(scope, 0).into()
        };
        let global = scope.get_current_context().global(scope);
        let array = cb::get(scope, global.into(), "Array")?;
        let from = cb::get(scope, array, "from")?;
        let array = cb::call(
            scope,
            from,
            array,
            &[keep],
            "Array.from",
        )?;
        let mapper = v8::Function::new(scope, lower_string)
            .ok_or_else(|| cb::type_error(scope, "could not create bank matcher mapper"))?;
        let mapped = cb::call_method(
            scope,
            array,
            "map",
            &[mapper.into()],
            "Array.from(...).map",
        )?;
        let set_constructor = cb::get(scope, global.into(), "Set")?;
        let set = cb::construct(scope, set_constructor, &[mapped], "Set")?;
        let data = cb::object(scope, &[("set", set)])?;
        let Some(function) = v8::Function::builder(deposit_all_except_predicate)
            .data(data)
            .build(scope)
        else {
            return Err(cb::type_error(
                scope,
                "could not create bank deposit-except matcher",
            ));
        };
        Ok(function.into())
    })();
    cb::finish(scope, rv, result);
}

fn lower_string<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let global = scope.get_current_context().global(scope);
        let string = cb::get(scope, global.into(), "String")?;
        let undefined = v8::undefined(scope).into();
        let value = cb::call(
            scope,
            string,
            undefined,
            &[args.get(0)],
            "String",
        )?;
        cb::call_method(
            scope,
            value,
            "toLowerCase",
            &[],
            "String(...).toLowerCase",
        )
    })();
    cb::finish(scope, rv, result);
}

fn deposit_all_except_predicate<'s>(
    scope: &mut v8::HandleScope<'s>,
    args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let name = args.get(0);
        let length = cb::get(scope, name, "length")?;
        let zero = cb::num(scope, 0.0);
        if !cb::gt(scope, length, zero)? {
            return Ok(v8::Boolean::new(scope, false).into());
        }
        let set = cb::get(scope, args.data(), "set")?;
        let has = cb::get(scope, set, "has")?;
        let lowered = cb::call_method(scope, name, "toLowerCase", &[], "name.toLowerCase")?;
        let matched = cb::call(scope, has, set, &[lowered], "set.has")?;
        let matched = cb::truthy(scope, matched);
        Ok(v8::Boolean::new(scope, !matched).into())
    })();
    cb::finish(scope, rv, result);
}

const BANK_STRATEGY_OPTIONS: [&str; 4] = ["Off", "Loot count", "Time", "Either"];

fn periodic_bank_settings<'s>(
    scope: &mut v8::HandleScope<'s>,
    _args: v8::FunctionCallbackArguments<'s>,
    rv: v8::ReturnValue,
) {
    let result = (|| -> JsResult<'s, v8::Local<'s, v8::Value>> {
        let options = v8::Array::new(scope, 0);
        for (index, option) in BANK_STRATEGY_OPTIONS.iter().enumerate() {
            let option = cb::string(scope, option);
            let index = u32::try_from(index)
                .map_err(|_| cb::type_error(scope, "bank strategy options exceed V8 array limit"))?;
            if !cb::catching(scope, |s| options.set_index(s, index, option))? {
                return Err(cb::type_error(
                    scope,
                    "could not materialize bank strategy options",
                ));
            }
        }
        let bank_type = cb::string(scope, "string");
        let bank_default = cb::string(scope, "Off");
        let bank_label = cb::string(scope, "Periodic bank");
        let bank_help = cb::string(scope, "save accumulated loot so a death does not lose it all");
        let bank_strategy = cb::object(
            scope,
            &[
                ("type", bank_type),
                ("default", bank_default),
                ("options", options.into()),
                ("label", bank_label),
                ("help", bank_help),
            ],
        )?;
        let items_type = cb::string(scope, "number");
        let items_default = cb::num(scope, 15.0);
        let items_min = cb::num(scope, 1.0);
        let items_max = cb::num(scope, 27.0);
        let items_label = cb::string(scope, "Bank at N loot items");
        let items = cb::object(
            scope,
            &[
                ("type", items_type),
                ("default", items_default),
                ("min", items_min),
                ("max", items_max),
                ("label", items_label),
            ],
        )?;
        let minutes_type = cb::string(scope, "number");
        let minutes_default = cb::num(scope, 10.0);
        let minutes_min = cb::num(scope, 1.0);
        let minutes_max = cb::num(scope, 120.0);
        let minutes_label = cb::string(scope, "Bank every N minutes");
        let minutes = cb::object(
            scope,
            &[
                ("type", minutes_type),
                ("default", minutes_default),
                ("min", minutes_min),
                ("max", minutes_max),
                ("label", minutes_label),
            ],
        )?;
        let common_junk_type = cb::string(scope, "boolean");
        let common_junk_default = v8::Boolean::new(scope, true).into();
        let common_junk_label = cb::string(scope, "Also bank gems/fruit/beer/kebabs/caskets");
        let common_junk = cb::object(
            scope,
            &[
                ("type", common_junk_type),
                ("default", common_junk_default),
                ("label", common_junk_label),
            ],
        )?;
        cb::object(
            scope,
            &[
                ("bankStrategy", bank_strategy),
                ("bankEveryItems", items),
                ("bankEveryMinutes", minutes),
                ("bankCommonJunk", common_junk),
            ],
        )
    })();
    cb::finish(scope, rv, result);
}
