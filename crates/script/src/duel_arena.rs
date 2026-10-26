/// Frozen `DUEL_FIGHT_ARENAS` as `(minX, maxX, minZ, maxZ)`.
pub const PENS: [(i32, i32, i32, i32); 6] = [
    (3333, 3357, 3244, 3258),
    (3364, 3388, 3225, 3239),
    (3333, 3357, 3206, 3220),
    (3364, 3388, 3244, 3258),
    (3333, 3357, 3225, 3239),
    (3364, 3388, 3206, 3220),
];

/// Frozen `clueDuelName`: underscores read as spaces, trimmed, lowercased.
/// The host's accept gate compares names the same way.
pub fn duel_name(name: &str) -> String {
    name.replace('_', " ").trim().to_lowercase()
}

/// Frozen `parseDuelPartnerHeader`: the display name after an optional
/// `Dueling with :` prefix (any case, optional spaces around the colon).
pub fn duel_partner_name(text: &str) -> Option<String> {
    let text = text.trim();
    let name = strip_dueling_with(text).unwrap_or(text).trim();
    (!name.is_empty()).then(|| name.to_string())
}

fn strip_dueling_with(text: &str) -> Option<&str> {
    const PREFIX: &str = "dueling with";
    let head = text.get(..PREFIX.len())?;
    if !head.eq_ignore_ascii_case(PREFIX) {
        return None;
    }
    text[PREFIX.len()..].trim_start().strip_prefix(':')
}

pub fn pen_at(x: i32, z: i32, level: i32) -> Option<usize> {
    (level == 0).then_some(())?;
    PENS.iter().position(|&(min_x, max_x, min_z, max_z)| {
        x >= min_x && x <= max_x && z >= min_z && z <= max_z
    })
}
