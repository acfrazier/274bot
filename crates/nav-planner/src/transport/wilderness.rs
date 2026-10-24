//! Wilderness teleport legality derived from content at bake time.
//!
//! The level formula lives in `wilderness_levels.rs2` (`(coordz − south)/N + M`
//! over `wilderness_zones` pairs). Caps are the `~wilderness_level(coord) > N`
//! gates on spell teleports and jewellery rubs. Nothing here is hand-copied.

use super::*;

const LEVELS_RS2: &str = "scripts/areas/area_wilderness/scripts/wilderness_levels.rs2";
const LEVELS_CONSTANT: &str = "scripts/areas/area_wilderness/configs/wilderness_levels.constant";
const ZONES_DBROW: &str = "scripts/areas/area_wilderness/configs/wilderness_zones.dbrow";
const SPELL_TELEPORT_RS2: &str = "scripts/skill_magic/scripts/spells/teleport.rs2";
const JEWELLERY_DIR: &str = "scripts/general/scripts/enchanted_jewellry";
const RING_OF_DUELING: &str = "ring_of_dueling.rs2";
const GAMES_NECKLACE: &str = "necklace_of_minigames.rs2";
const AMULET_OF_GLORY: &str = "amulet_of_glory.rs2";

/// Inclusive AABB of one `wilderness_zones` coord pair, plus the script's
/// `$coord1` south-edge z used as the level origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WildernessZone {
    pub x1: i32,
    pub z1: i32,
    pub x2: i32,
    pub z2: i32,
    pub level1: i32,
    pub level2: i32,
    /// `coordz($coord1)` — the first listed pair coord, the formula origin.
    pub origin_z: i32,
}

impl WildernessZone {
    pub fn contains(self, t: WorldTile) -> bool {
        t.x >= self.x1
            && t.x <= self.x2
            && t.z >= self.z1
            && t.z <= self.z2
            && t.level >= self.level1
            && t.level <= self.level2
    }
}

/// Packed wilderness-level formula: zones plus `(z − origin_z) / divisor + offset`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WildernessRules {
    pub zones: Vec<WildernessZone>,
    pub divisor: i32,
    pub offset: i32,
}

impl WildernessRules {
    /// Content `~wilderness_level(coord)`: 0 outside every zone.
    pub fn level(&self, t: WorldTile) -> i32 {
        if self.divisor <= 0 {
            return 0;
        }
        for z in &self.zones {
            if z.contains(t) {
                return (t.z - z.origin_z) / self.divisor + self.offset;
            }
        }
        0
    }

    /// Inclusive `inzone` membership against packed `wilderness_zones`.
    /// A reversed bound in the listed pair is an empty zone, matching the
    /// engine (`x1..x2` / `level1..level2` as written, not min/max).
    pub fn contains(&self, t: WorldTile) -> bool {
        self.zones.iter().any(|z| z.contains(t))
    }
}

impl TransportGraph {
    /// Whether `edge` may be taken from `from` under the packed wilderness cap.
    /// Non-teleports and teleports with no derived cap stay unrestricted.
    pub fn teleport_legal_from(&self, from: WorldTile, edge: &TransportEdge) -> bool {
        Self::teleport_legal_at_level(self.wilderness.level(from), edge)
    }

    /// [`Self::teleport_legal_from`] with a precomputed wilderness level.
    pub fn teleport_legal_at_level(level: i32, edge: &TransportEdge) -> bool {
        match edge.wildy_cap {
            None => true,
            Some(cap) => level <= cap,
        }
    }
}

/// Load zones + formula. Missing sources yield the empty default (fixtures).
/// Present-but-unparseable sources are `Err` so the bake fails closed.
pub(super) fn load_wilderness_rules(content_root: &Path) -> Result<WildernessRules, String> {
    let rs2_path = content_root.join(LEVELS_RS2);
    let dbrow_path = content_root.join(ZONES_DBROW);
    let rs2_ok = rs2_path.is_file();
    let dbrow_ok = dbrow_path.is_file();
    if !rs2_ok && !dbrow_ok {
        return Ok(WildernessRules::default());
    }
    if !rs2_ok {
        return Err(format!("{} is missing", LEVELS_RS2));
    }
    if !dbrow_ok {
        return Err(format!("{} is missing", ZONES_DBROW));
    }
    let rs2 = fs::read_to_string(&rs2_path).map_err(|e| format!("{LEVELS_RS2}: {e}"))?;
    let dbrow = fs::read_to_string(&dbrow_path).map_err(|e| format!("{ZONES_DBROW}: {e}"))?;
    let (divisor, offset) = parse_wilderness_level_formula(&rs2)?;
    let zones = parse_wilderness_zones(&dbrow)?;
    if zones.is_empty() {
        return Err(format!("{ZONES_DBROW} lists no coord_pair rows"));
    }
    let constant_path = content_root.join(LEVELS_CONSTANT);
    if constant_path.is_file() {
        let constants =
            fs::read_to_string(&constant_path).map_err(|e| format!("{LEVELS_CONSTANT}: {e}"))?;
        validate_starting_z(&zones, &constants)?;
    }
    Ok(WildernessRules {
        zones,
        divisor,
        offset,
    })
}

/// Cap from `teleport.rs2`, or `None` when the file is absent.
pub(super) fn spell_teleport_cap(content_root: &Path) -> Result<Option<i32>, String> {
    let path = content_root.join(SPELL_TELEPORT_RS2);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path).map_err(|e| format!("{SPELL_TELEPORT_RS2}: {e}"))?;
    wilderness_level_cap(&text)
        .ok_or_else(|| {
            format!("{SPELL_TELEPORT_RS2} has no unique ~wilderness_level(coord) > N gate")
        })
        .map(Some)
}

/// Cap declared in one jewellery rub script, or `None` when absent / no gate.
pub(super) fn jewellery_file_cap(text: &str) -> Result<Option<i32>, String> {
    Ok(wilderness_level_cap(text))
}

/// Cap each jewellery obj id should carry, from the three gated rub files.
fn jewellery_caps_by_obj(
    content_root: &Path,
) -> Result<(HashMap<i32, i32>, i32, i32, i32), String> {
    let objs = obj_ids_by_name(content_root);
    let cats = jewellery_categories(content_root);
    let dir = content_root.join(JEWELLERY_DIR);
    let mut by_obj = HashMap::new();
    let mut dueling = None;
    let mut games = None;
    let mut glory = None;
    for name in [RING_OF_DUELING, GAMES_NECKLACE, AMULET_OF_GLORY] {
        let path = dir.join(name);
        if !path.is_file() {
            return Err(format!(
                "wilderness teleport legality: {name} is missing under {JEWELLERY_DIR}"
            ));
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("{name}: {e}"))?;
        let cap = jewellery_file_cap(&text)?
            .ok_or_else(|| format!("wilderness teleport legality: {name} cap cannot be derived"))?;
        match name {
            RING_OF_DUELING => dueling = Some(cap),
            GAMES_NECKLACE => games = Some(cap),
            AMULET_OF_GLORY => glory = Some(cap),
            _ => {}
        }
        for (op, block_name, body) in jewellery_blocks(&text) {
            if op != "opheld4" {
                continue;
            }
            if block_teleport_dests(&body, &text).is_empty() {
                continue;
            }
            let items: Vec<String> = match block_name.strip_prefix('_') {
                Some(cat) => cats.get(cat).cloned().unwrap_or_default(),
                None => vec![block_name],
            };
            for item in items {
                let Some(&id) = objs.get(&item) else {
                    continue;
                };
                if let Some(prev) = by_obj.insert(id, cap) {
                    if prev != cap {
                        return Err(format!(
                            "wilderness teleport legality: obj {id} has caps {prev} and {cap}"
                        ));
                    }
                }
            }
        }
    }
    Ok((
        by_obj,
        dueling.ok_or("wilderness teleport legality: dueling cap cannot be derived")?,
        games.ok_or("wilderness teleport legality: games necklace cap cannot be derived")?,
        glory.ok_or("wilderness teleport legality: glory cap cannot be derived")?,
    ))
}

/// Bake-time check: when the wilderness sources exist, caps and the formula
/// must parse and every packed teleport must carry the derived cap.
/// Exactly one of the formula/zones files, or teleport/jewellery gates
/// without those files, is an error so a custom bake cannot silently
/// ship no teleports.
pub(crate) fn require_wilderness_teleport_legality(
    content_root: &Path,
    graph: &TransportGraph,
) -> Result<(), String> {
    let rs2 = content_root.join(LEVELS_RS2).is_file();
    let dbrow = content_root.join(ZONES_DBROW).is_file();
    if rs2 != dbrow {
        return Err(if rs2 {
            format!("wilderness teleport legality: {ZONES_DBROW} is missing")
        } else {
            format!("wilderness teleport legality: {LEVELS_RS2} is missing")
        });
    }
    let jewellery_dir = content_root.join(JEWELLERY_DIR);
    let jewellery = [RING_OF_DUELING, GAMES_NECKLACE, AMULET_OF_GLORY]
        .iter()
        .any(|n| jewellery_dir.join(n).is_file());
    let spell = content_root.join(SPELL_TELEPORT_RS2).is_file();
    if !rs2 {
        if spell || jewellery {
            return Err(
                "wilderness teleport legality: teleport or jewellery gates present without wilderness_levels.rs2 / wilderness_zones.dbrow"
                    .into(),
            );
        }
        return Ok(());
    }
    let rules = load_wilderness_rules(content_root)?;
    if rules.zones.is_empty() || rules.divisor <= 0 {
        return Err(
            "wilderness teleport legality: level formula or zones cannot be derived".into(),
        );
    }
    let spell_cap = spell_teleport_cap(content_root)?
        .ok_or("wilderness teleport legality: spell cap cannot be derived")?;
    let (by_obj, dueling, games, _glory) = jewellery_caps_by_obj(content_root)?;
    if dueling != spell_cap {
        return Err(format!(
            "wilderness teleport legality: dueling cap {dueling} != spell cap {spell_cap}"
        ));
    }
    if games != spell_cap {
        return Err(format!(
            "wilderness teleport legality: games necklace cap {games} != spell cap {spell_cap}"
        ));
    }
    if graph.teleports.is_empty() {
        return Err(
            "wilderness teleport legality: content caps derived but no teleport edges were packed"
                .into(),
        );
    }
    for e in &graph.teleports {
        let Some(cap) = e.wildy_cap else {
            return Err(format!(
                "wilderness teleport legality: teleport to ({}, {}, {}) has no derived cap",
                e.to.x, e.to.z, e.to.level
            ));
        };
        if e.loc_id == 0 {
            if cap != spell_cap {
                return Err(format!(
                    "wilderness teleport legality: spell teleport cap {cap} != derived {spell_cap}"
                ));
            }
        } else {
            let Some(&expected) = by_obj.get(&e.loc_id) else {
                return Err(format!(
                    "wilderness teleport legality: jewellery teleport loc {} has no per-file cap",
                    e.loc_id
                ));
            };
            if cap != expected {
                return Err(format!(
                    "wilderness teleport legality: jewellery loc {} cap {cap} != file cap {expected}",
                    e.loc_id
                ));
            }
        }
    }
    Ok(())
}

/// Unique `~wilderness_level(coord) > N` in `text`. Several copies of the
/// same N are fine; disagreeing N or a missing gate is not a unique cap.
pub(super) fn wilderness_level_cap(text: &str) -> Option<i32> {
    let flat = normalized_body(text);
    let needle = "~wilderness_level(coord)>";
    let mut found: Option<i32> = None;
    let mut rest = flat.as_str();
    while let Some(at) = rest.find(needle) {
        let after = &rest[at + needle.len()..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            return None;
        }
        let n: i32 = digits.parse().ok()?;
        match found {
            None => found = Some(n),
            Some(prev) if prev != n => return None,
            Some(_) => {}
        }
        rest = &after[digits.len()..];
    }
    found
}

fn parse_wilderness_level_formula(rs2: &str) -> Result<(i32, i32), String> {
    let flat = normalized_body(rs2);
    if !flat.contains("[proc,wilderness_level](coord$coord)(int)") {
        return Err(format!(
            "{LEVELS_RS2} has no [proc,wilderness_level](coord $coord)(int)"
        ));
    }
    if !flat.contains("db_getfield(wilderness_zones,coord_pair_table:coord_pair") {
        return Err(format!(
            "{LEVELS_RS2} does not read wilderness_zones coord_pair rows"
        ));
    }
    if !flat.contains("inzone($coord1,$coord2,$coord)=true") {
        return Err(format!(
            "{LEVELS_RS2} does not test inzone($coord1, $coord2, $coord)"
        ));
    }
    let prefix = "return(calc((coordz($coord)-coordz($coord1))/";
    let Some(at) = flat.find(prefix) else {
        return Err(format!(
            "{LEVELS_RS2} does not return (coordz($coord) - coordz($coord1)) / N + M"
        ));
    };
    let after = &flat[at + prefix.len()..];
    let div_digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
    if div_digits.is_empty() {
        return Err(format!("{LEVELS_RS2} wilderness level divisor is missing"));
    }
    let divisor: i32 = div_digits
        .parse()
        .map_err(|_| format!("{LEVELS_RS2} wilderness level divisor does not parse"))?;
    if divisor <= 0 {
        return Err(format!("{LEVELS_RS2} wilderness level divisor must be > 0"));
    }
    let after_div = &after[div_digits.len()..];
    let (offset, rest) = if let Some(tail) = after_div.strip_prefix('+') {
        let off_digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
        if off_digits.is_empty() {
            return Err(format!("{LEVELS_RS2} wilderness level offset is missing"));
        }
        let offset: i32 = off_digits
            .parse()
            .map_err(|_| format!("{LEVELS_RS2} wilderness level offset does not parse"))?;
        (offset, &tail[off_digits.len()..])
    } else {
        (0, after_div)
    };
    if !rest.starts_with("));") {
        return Err(format!(
            "{LEVELS_RS2} wilderness level return is not calc((coordz - coordz)/N + M)"
        ));
    }
    if !flat.contains("return(0);") {
        return Err(format!(
            "{LEVELS_RS2} does not return 0 outside wilderness_zones"
        ));
    }
    Ok((divisor, offset))
}

fn parse_wilderness_zones(dbrow: &str) -> Result<Vec<WildernessZone>, String> {
    let mut in_table = false;
    let mut zones = Vec::new();
    for raw in dbrow.lines() {
        let line = match raw.find("//") {
            Some(i) => raw[..i].trim(),
            None => raw.trim(),
        };
        if line.is_empty() {
            continue;
        }
        if let Some(name) = dbrow_block(line) {
            in_table = name == "wilderness_zones";
            continue;
        }
        if !in_table {
            continue;
        }
        if let Some(rest) = line.strip_prefix("table=") {
            if rest.trim() != "coord_pair_table" {
                return Err(format!(
                    "{ZONES_DBROW} table is {}, not coord_pair_table",
                    rest.trim()
                ));
            }
            continue;
        }
        let Some(rest) = line.strip_prefix("data=coord_pair,") else {
            continue;
        };
        let mut parts = rest.split(',');
        let a = parts
            .next()
            .and_then(coord_literal)
            .ok_or_else(|| format!("{ZONES_DBROW} coord_pair first coord does not parse"))?;
        let b = parts
            .next()
            .and_then(coord_literal)
            .ok_or_else(|| format!("{ZONES_DBROW} coord_pair second coord does not parse"))?;
        if parts.next().is_some() {
            return Err(format!("{ZONES_DBROW} coord_pair has extra fields"));
        }
        // `$coord1` is the first listed coord; the formula origins on its z.
        // Engine `inzone` uses the pair as listed (a reversed x or level
        // bound is an empty zone). North-first z would invert the origin.
        if a.2 > b.2 {
            return Err(format!(
                "{ZONES_DBROW} coord_pair lists a north coord first (z {} > {})",
                a.2, b.2
            ));
        }
        zones.push(WildernessZone {
            x1: a.1,
            z1: a.2,
            x2: b.1,
            z2: b.2,
            level1: a.0,
            level2: b.0,
            origin_z: a.2,
        });
    }
    Ok(zones)
}

fn validate_starting_z(zones: &[WildernessZone], constants: &str) -> Result<(), String> {
    let flat = normalized_body(constants);
    let surface = constant_i32(&flat, "wilderness_starting_z")?;
    let underground = constant_i32(&flat, "wilderness_starting_underground_z")?;
    let origins: HashSet<i32> = zones.iter().map(|z| z.origin_z).collect();
    if !origins.contains(&surface) {
        return Err(format!(
            "{LEVELS_CONSTANT} ^wilderness_starting_z={surface} is not a zone origin {origins:?}"
        ));
    }
    if !origins.contains(&underground) {
        return Err(format!(
            "{LEVELS_CONSTANT} ^wilderness_starting_underground_z={underground} is not a zone origin {origins:?}"
        ));
    }
    Ok(())
}

fn constant_i32(flat: &str, name: &str) -> Result<i32, String> {
    let needle = format!("^{name}=");
    let Some(at) = flat.find(&needle) else {
        return Err(format!("{LEVELS_CONSTANT} does not define ^{name}"));
    };
    let after = &flat[at + needle.len()..];
    let digits: String = after
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits
        .parse()
        .map_err(|_| format!("{LEVELS_CONSTANT} ^{name} does not parse"))
}

#[cfg(test)]
pub(crate) const TEST_LEVELS_RS2: &str = LEVELS_RS2;
#[cfg(test)]
pub(crate) const TEST_ZONES_DBROW: &str = ZONES_DBROW;
#[cfg(test)]
pub(crate) const TEST_LEVELS_CONSTANT: &str = LEVELS_CONSTANT;
#[cfg(test)]
pub(crate) const TEST_SPELL_TELEPORT_RS2: &str = SPELL_TELEPORT_RS2;
