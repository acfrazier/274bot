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
//!   ([`crate::bank_withdraw::next_chunk`]), `null` or `{ kind, count }` /
//!   `{ kind, op }`.
//! - `__rs2b0t_withdraw_op(ops, amount)`: frozen `withdrawOp`
//!   ([`crate::bank_op::withdraw_op`]), the matching op label or `null`.

use super::callback_v8::{self as cb, Callback, JsResult};
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
    use crate::bank_op::{withdraw_op, WithdrawAmount};
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
    use crate::bank_withdraw::{next_chunk, Chunk};
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
