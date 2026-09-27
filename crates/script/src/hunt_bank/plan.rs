use super::*;

#[derive(Clone, Debug)]
pub(super) struct KeyItem {
    pub(super) id: i32,
    pub(super) name: String,
}

#[derive(Clone, Debug)]
pub(super) struct NamedCount {
    pub(super) name: String,
    pub(super) count: i32,
}

#[derive(Clone, Debug)]
pub(super) struct FlaskPlan {
    pub(super) name: String,
    pub(super) want: i32,
    pub(super) doses: Vec<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BankProj {
    pub(super) bank: Option<Tile>,
    pub(super) key_item: Option<KeyItem>,
    pub(super) area: Area,
    pub(super) coins: Option<i32>,
    pub(super) fire_at_range: bool,
    pub(super) withdraw_food: bool,
    pub(super) heal_to: f64,
    pub(super) wear: Vec<String>,
    pub(super) carry: Vec<String>,
    pub(super) has_pick_weapon: bool,
    pub(super) food_name: String,
    pub(super) food_want: i32,
    pub(super) style: String,
    pub(super) weapon: String,
    pub(super) ammo: String,
    pub(super) ammo_want: i32,
    pub(super) spell: String,
    pub(super) keep_extra: Vec<String>,
    /// Frozen `BankOpts.runeCasts` / `runeBuffer` / `escapeStock`.
    pub(super) rune_casts: i32,
    pub(super) rune_buffer: i32,
    pub(super) escape_stock: i32,
    pub(super) escape_id: String,
    pub(super) flasks: Vec<FlaskPlan>,
    pub(super) target: String,
    /// Frozen `acquireKey`'s bank stop: deposit, the key, the gear, food
    /// only for a Velrak fetch, close, wear. No pick, supplies, heal or
    /// trip count.
    pub(super) acquire: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyArm {
    Skip,
    Held,
    Bank,
    Fetch,
    Opaque,
    Nameless,
}

#[derive(Clone, Debug)]
pub(super) struct Plan {
    pub(super) key: KeyArm,
    pub(super) key_id: i32,
    pub(super) key_name: String,
    pub(super) shield_seen: bool,
    pub(super) needs_shield: bool,
    pub(super) food_seen: bool,
    pub(super) food_owed: bool,
    pub(super) junk: Vec<String>,
}
pub(super) fn opt_i32(input: &Value, key: &str) -> Option<i32> {
    input
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
}

/// Frozen `[...(opts.potions ?? []).map(asFlask), ...(opts.flasks ?? [])]`.
pub(super) fn parse_flasks(input: &Value) -> Vec<FlaskPlan> {
    let rows = |key: &str| {
        input
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
    };
    let potions = rows("potions").into_iter().map(|plan| FlaskPlan {
        name: plan
            .get("flask")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        want: opt_i32(&plan, "want").unwrap_or(0),
        doses: plan
            .get("potion")
            .map(|p| strings(p, "doses"))
            .unwrap_or_default(),
    });
    let flasks = rows("flasks").into_iter().map(|plan| FlaskPlan {
        name: plan
            .get("flask")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        want: opt_i32(&plan, "want").unwrap_or(0),
        doses: strings(&plan, "doses"),
    });
    potions
        .chain(flasks)
        .filter(|plan| !plan.name.is_empty())
        .collect()
}

pub(super) fn parse_boxes(input: &Value) -> Vec<SiteBox> {
    input
        .get("boxes")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|row| SiteBox {
                    min_x: i32_of(row.get("minX")),
                    max_x: i32_of(row.get("maxX")),
                    min_z: i32_of(row.get("minZ")),
                    max_z: i32_of(row.get("maxZ")),
                    level: i32_of(row.get("level")),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn parse_tile(v: Option<&Value>) -> Option<Tile> {
    let v = v?;
    if v.is_null() {
        return None;
    }
    Some(Tile {
        x: i32_of(v.get("x")),
        z: i32_of(v.get("z")),
        level: i32_of(v.get("level")),
    })
}

pub(super) fn parse_proj(input: &Value) -> BankProj {
    let key_item = match input.get("keyItem") {
        None | Some(Value::Null) => None,
        Some(item) => Some(KeyItem {
            id: i32_of(item.get("id")),
            name: item
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
    };
    let heal_to = input.get("healTo").and_then(Value::as_f64).unwrap_or(0.9);
    let coins = input
        .get("coins")
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok());
    BankProj {
        bank: parse_tile(input.get("bank")),
        key_item,
        area: Area::of(parse_boxes(input)),
        coins,
        fire_at_range: input.get("fireAtRange").and_then(Value::as_bool) == Some(true),
        withdraw_food: input.get("withdrawFood").and_then(Value::as_bool) == Some(true),
        heal_to,
        wear: strings(input, "wear"),
        carry: strings(input, "carry"),
        has_pick_weapon: input.get("hasPickWeapon").and_then(Value::as_bool) == Some(true),
        food_name: input
            .get("foodName")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        food_want: i32_of(input.get("foodWant")),
        style: input
            .get("style")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        weapon: input
            .get("weapon")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        ammo: input
            .get("ammo")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        // Frozen `BankOpts.ammo` is the withdraw count; the ammo's name is
        // the host's `ammoName()`.
        ammo_want: opt_i32(input, "ammo").unwrap_or(AMMO_WITHDRAW),
        spell: input
            .get("spell")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        keep_extra: strings(input, "keepExtra"),
        rune_casts: opt_i32(input, "runeCasts").unwrap_or(RUNE_CASTS),
        rune_buffer: opt_i32(input, "runeBuffer").unwrap_or(RUNE_BUFFER),
        escape_stock: opt_i32(input, "escapeStock").unwrap_or(ESCAPE_STOCK),
        escape_id: input
            .get("escapeTeleportId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        flasks: parse_flasks(input),
        target: input
            .get("target")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        acquire: input.get("acquire").and_then(Value::as_bool) == Some(true),
    }
}

/// Names of the worn items (frozen `wieldedNames()`).
pub(super) fn wielded(obs: &BankObservation) -> Vec<String> {
    obs.equipment
        .iter()
        .filter(|row| row.count > 0)
        .map(|row| row.name.clone())
        .collect()
}

/// The spell's remaining per-cast rune costs with the worn staves.
pub(super) fn cast_costs(proj: &BankProj, obs: &BankObservation) -> Option<Vec<NamedCount>> {
    let costs = crate::supply_v2::selected_data()?.runes_per_cast(&proj.spell, &wielded(obs))?;
    Some(
        costs
            .into_iter()
            .map(|cost| NamedCount {
                name: cost.rune,
                count: cost.count,
            })
            .collect(),
    )
}

/// Frozen `runeWithdrawList(spell, wielded, runeCasts)` each topped by the
/// buffer: the targets a mage trip withdraws to.
pub(super) fn rune_targets(proj: &BankProj, obs: &BankObservation) -> Vec<NamedCount> {
    if !proj.style.eq_ignore_ascii_case("mage") {
        return Vec::new();
    }
    cast_costs(proj, obs)
        .unwrap_or_default()
        .into_iter()
        .map(|cost| NamedCount {
            name: cost.name,
            count: cost.count.saturating_mul(proj.rune_casts) + proj.rune_buffer,
        })
        .collect()
}

/// Frozen `castsLeft(h)`.
pub(super) fn casts_left(proj: &BankProj, obs: &BankObservation) -> f64 {
    let Some(costs) = cast_costs(proj, obs) else {
        return 0.0;
    };
    let held = slotted(obs).cloned().collect::<Vec<_>>();
    costs
        .iter()
        .map(|cost| (name_count(&held, &cost.name) as f64 / cost.count as f64).floor())
        .fold(f64::INFINITY, f64::min)
}

/// The site's escape teleport: its label, level and per-cast runes.
pub(super) fn escape_fact(proj: &BankProj) -> Option<crate::escape_runes::EscapeRunesFact> {
    crate::escape_runes::escape_runes_for_optional(
        crate::supply_v2::selected_data().as_deref(),
        &proj.escape_id,
    )
    .ok()
}

/// Frozen `count * (escapeStock + 1)` per escape rune.
pub(super) fn escape_targets(proj: &BankProj) -> Vec<NamedCount> {
    escape_fact(proj)
        .map(|esc| {
            esc.runes
                .into_iter()
                .map(|rune| NamedCount {
                    name: rune.rune,
                    count: rune.count.saturating_mul(proj.escape_stock + 1),
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn classify_key(obs: &BankObservation, item: &KeyItem) -> KeyArm {
    let on_side = obs
        .bank_side
        .iter()
        .any(|row| row.id == item.id && row.count > 0);
    let on_inv = slotted(obs).any(|row| row.id == item.id);
    if on_side || on_inv {
        return KeyArm::Held;
    }
    let fallback = obs
        .inv
        .iter()
        .any(|row| row.id == item.id && row.count > 0 && !real_slot(row));
    if let Some(row) = obs
        .bank
        .iter()
        .find(|row| row.id == item.id && row.count > 0)
    {
        if row.name.trim().is_empty() {
            return KeyArm::Nameless;
        }
        return KeyArm::Bank;
    }
    if fallback {
        KeyArm::Opaque
    } else {
        KeyArm::Fetch
    }
}

pub(super) fn shield_seen(obs: &BankObservation) -> bool {
    equip_has_name(obs, SHIELD)
        || obs
            .bank_side
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        || obs
            .bank
            .iter()
            .any(|row| row.count > 0 && eq_name(&row.name, SHIELD))
        || slotted_has_name(obs, SHIELD)
}

pub(super) fn food_seen(obs: &BankObservation, forms: &[String]) -> bool {
    obs.bank
        .iter()
        .chain(obs.bank_side.iter())
        .chain(slotted(obs))
        .any(|row| row.count > 0 && is_food_name(&row.name, forms))
}

pub(super) fn keep_names(proj: &BankProj, obs: &BankObservation) -> Vec<String> {
    let mut keep = food_forms(&proj.food_name);
    if needs_shield(proj) {
        keep.push(SHIELD.to_string());
    }
    if let Some(item) = &proj.key_item {
        if !item.name.is_empty() {
            keep.push(item.name.clone());
        }
        for row in obs
            .bank
            .iter()
            .chain(obs.bank_side.iter())
            .chain(obs.inv.iter())
        {
            if row.id == item.id && !row.name.is_empty() {
                keep.push(row.name.clone());
            }
        }
    }
    if proj.coins.is_some() {
        keep.push("Coins".to_string());
    }
    if !proj.weapon.is_empty() {
        keep.push(proj.weapon.clone());
    }
    if proj.style.eq_ignore_ascii_case("range") && !proj.ammo.is_empty() {
        keep.push(proj.ammo.clone());
    }
    for rune in rune_targets(proj, obs) {
        keep.push(rune.name);
    }
    for rune in escape_targets(proj) {
        keep.push(rune.name);
    }
    keep.extend(proj.keep_extra.iter().cloned());
    for name in proj.wear.iter().chain(proj.carry.iter()) {
        if !name.is_empty() {
            keep.push(name.clone());
        }
    }
    for flask in &proj.flasks {
        keep.push(flask.name.clone());
        keep.extend(flask.doses.iter().cloned());
    }
    keep
}

pub(super) fn kept(keep: &[String], name: &str) -> bool {
    keep.iter().any(|item| eq_name(item, name))
}

pub(super) fn capture_plan(proj: &BankProj, obs: &BankObservation) -> Plan {
    let forms = food_forms(&proj.food_name);
    let key = match &proj.key_item {
        None => KeyArm::Skip,
        Some(item) => classify_key(obs, item),
    };
    let keep = keep_names(proj, obs);
    let mut junk = Vec::new();
    for row in &obs.bank_side {
        if row.name.is_empty() || row.count <= 0 {
            continue;
        }
        if !kept(&keep, &row.name) && !junk.iter().any(|name: &String| eq_name(name, &row.name)) {
            junk.push(row.name.clone());
        }
    }
    let seen_food = food_seen(obs, &forms);
    Plan {
        key,
        key_id: proj.key_item.as_ref().map(|item| item.id).unwrap_or(0),
        key_name: proj
            .key_item
            .as_ref()
            .map(|item| item.name.clone())
            .unwrap_or_default(),
        shield_seen: shield_seen(obs),
        needs_shield: needs_shield(proj),
        food_seen: seen_food,
        food_owed: proj.withdraw_food && seen_food,
        junk,
    }
}
