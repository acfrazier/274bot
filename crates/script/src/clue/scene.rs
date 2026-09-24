use super::*;

/// What this call's posted `here` says about the decoded tile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Arrival {
    /// No posted `here`: no arrival claim to make and no walk to measure.
    Unknown,
    /// Posted, and not this tile's level or not within `ARRIVE_RADIUS` of it.
    Walking,
    /// Posted on the tile's level and within `ARRIVE_RADIUS` of it.
    Arrived,
}

/// The landed arrival read over this call's posted `here`: the same level and
/// Chebyshev `ARRIVE_RADIUS` the search walk uses, so the guarded and unguarded
/// Dig arrive exactly the way their sibling search row does.
pub(super) fn arrival(tile: Tile, input: &Value) -> Arrival {
    let Some(here) = posted_here(input) else {
        return Arrival::Unknown;
    };
    if here.level == tile.level && chebyshev(here, tile) <= i64::from(ARRIVE_RADIUS) {
        Arrival::Arrived
    } else {
        Arrival::Walking
    }
}

/// The landed identify over the wrapper's page and the selected family. The
/// family token and `none-held` are the helper's own; only the missing pin is
/// named here.
pub(super) fn identify<'a>(
    selected: Option<&'a SelectedGameData>,
    input: &Value,
) -> Result<&'a TrailMembershipRow, &'static str> {
    let Some(data) = selected else {
        return Err(MISSING_SELECTED_DATA);
    };
    let held = posted_page(input);
    identify_step(&held, data.trails())
}

/// The posted `(id, count)` page in posted order, as the wrapper marshals it.
/// A missing, non-array, or malformed entry is skipped the same way a row
/// that is not an `i32` pair cannot be held — never a second page and never a
/// snapshot error. Only a positive count holds, and only a membership row
/// wins; that stays in the landed helper.
pub(super) fn posted_page(input: &Value) -> Vec<(i32, i32)> {
    let Some(rows) = input.get("held").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut held = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(pair) = row.as_array() else {
            continue;
        };
        let (Some(id), Some(count)) = (pair.first().and_then(i32_of), pair.get(1).and_then(i32_of))
        else {
            continue;
        };
        held.push((id, count));
    }
    held
}

/// The wrapper writes page numbers as JSON integers.
pub(super) fn i32_of(value: &Value) -> Option<i32> {
    i32::try_from(value.as_i64()?).ok()
}

/// One posted integer field of a marshalled page row — an npc row's health,
/// distance or target, and the payload's own `self_slot`, `hitpoints` and
/// overlay varp. A missing, null or non-integer field is `None`: the machine
/// never rounds a posted value into the number it wants.
pub(super) fn posted_i32(row: &Value, key: &str) -> Option<i32> {
    row.get(key).and_then(i32_of)
}

/// One packed-coord square: `SQUARE` tiles per map square on both axes.
/// Same packed contract as the landed `nav::canlight::unpack_packed_coord`;
/// the script crate does not take a `nav` runtime dependency, so the
/// arithmetic lives here.
pub(super) const SQUARE: i32 = 64;

/// Planes `0..=3`.
pub(super) const LEVELS: i32 = 4;

/// The frozen picker's `ARRIVE_RADIUS`: the decoded tile itself, and one step
/// off it on either axis.
pub(super) const ARRIVE_RADIUS: i32 = 1;

/// A world tile: the same three fields the posted `here` object and every
/// posted `SceneEntity` row carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Tile {
    pub(super) x: i32,
    pub(super) z: i32,
    pub(super) level: i32,
}

/// Chebyshev distance, widened: `max(|dx|, |dz|)` over the whole `i32` range
/// so a decoded tile and a posted tile can never overflow the subtraction.
/// Callers compare levels first, because a level mismatch is not a distance.
pub(super) fn chebyshev(a: Tile, b: Tile) -> i64 {
    let dx = (i64::from(a.x) - i64::from(b.x)).abs();
    let dz = (i64::from(a.z) - i64::from(b.z)).abs();
    dx.max(dz)
}

/// A selected `trail_coord` token → the tile it packs.
///
/// The landed `nav::canlight::unpack_packed_coord` contract, copied: five
/// `_`-separated integers `level_mapX_mapZ_localX_localZ`, level in `0..=3`,
/// both locals in `0..64`, and `x = mapX * 64 + localX` (`z` likewise). A
/// missing or extra part, a non-integer, an out-of-range level or local, and
/// a map that overflows the packed widening are all **not** a tile: the row
/// idles rather than walking to an invented coordinate.
pub(super) fn decode_trail_coord(token: &str) -> Option<Tile> {
    let mut parts = token.split('_');
    let mut next = || parts.next()?.parse::<i32>().ok();
    let level = next()?;
    let map_x = next()?;
    let map_z = next()?;
    let local_x = next()?;
    let local_z = next()?;
    if parts.next().is_some() {
        return None;
    }
    if !(0..LEVELS).contains(&level)
        || !(0..SQUARE).contains(&local_x)
        || !(0..SQUARE).contains(&local_z)
    {
        return None;
    }
    Some(Tile {
        x: map_x.checked_mul(SQUARE)?.checked_add(local_x)?,
        z: map_z.checked_mul(SQUARE)?.checked_add(local_z)?,
        level,
    })
}

/// The identified row's selected search membership: a selected
/// `trail_loc=^true` param **and** a decodable selected `trail_coord` on the
/// same row, in file order for the coord.
///
/// The pin is the membership; the coord alone is not. A coord-only row — the
/// easy maps, the frozen `keyFrom` riddles that carry only `trail_desc`, and
/// the bounded packed 3554 clue — is not a search step: the coord-bearing
/// ones are the sibling dig classify's, and a row the dig classify leaves out
/// idles. A pin with no coord, or an off-contract one, is the same idle:
/// nothing is invented.
pub(super) fn search_tile(row: &TrailMembershipRow) -> Option<Tile> {
    let mut located = false;
    let mut coord = None;
    for param in &row.params {
        if param.key == "trail_loc" && param.value == "^true" {
            located = true;
        } else if param.key == "trail_coord" && coord.is_none() {
            coord = Some(param.value.as_str());
        }
    }
    if !located {
        return None;
    }
    decode_trail_coord(coord?)
}

/// The one selected `access` value that is not playable here: the packed 3554
/// clue's own bound. Every other access, and a row that was posted with none
/// at all, is outside this machine's refusals.
pub(super) const CONSTRAINED: &str = "constrained";

/// The identified row's unguarded-dig membership: a decodable selected
/// `trail_coord` **and** no selected `trail_loc` **and** no selected
/// `trail_guardian` **and** an `access` that is not `"constrained"`.
///
/// The sibling of `search_tile`, not a fold into it: the loc pin is the search
/// membership and this is the selected-param classify that holds without
/// copying the frozen `type`. It reads no `trail_sextant` — that param stays
/// the guarded sibling's own pin — so the membership is the twenty medium
/// sextant rows and every coord-bearing row beside them: the map rows like
/// `2713`, the vague `3510` and the hard riddle-with-coord rows. Forty rows on
/// both pins, and the `trail_casket` param is never part of the classify. The
/// Sextant/Watch/Chart trio is never required, never waited for and never
/// acquired. A guarded row stays out: its first Dig is a spawn, and that row
/// belongs to the guarded encounter rather than this arm. The packed
/// constrained 3554 clue stays out with the desc-only rows that carry no coord
/// and the paramless 2722: identified, then idle rather than an invented
/// coordinate.
pub(super) fn dig_tile(row: &TrailMembershipRow) -> Option<Tile> {
    if row.access.as_deref() == Some(CONSTRAINED) {
        return None;
    }
    let mut located = false;
    let mut guarded = false;
    let mut coord = None;
    for param in &row.params {
        match param.key.as_str() {
            "trail_loc" => located = true,
            "trail_guardian" => guarded = true,
            "trail_coord" if coord.is_none() => coord = Some(param.value.as_str()),
            _ => {}
        }
    }
    if located || guarded {
        return None;
    }
    decode_trail_coord(coord?)
}

/// The identified row's guarded-dig membership: a decodable selected
/// `trail_coord` **and** no selected `trail_loc` **and** `trail_sextant=yes`
/// **and** a selected `trail_guardian` **and** an `access` that is not
/// `"constrained"`.
///
/// The sibling of `dig_tile`, not a fold into it: the guardian param is what
/// makes the first Dig a spawn, so the unguarded arm must never reach this
/// encounter and this arm must never Dig a row without one. The param value is
/// the family alias the thirty hard sextant rows carry; the wizard name it
/// stands for lives in `guardian_names` alone and is only ever a posted-name
/// filter, never a row field.
pub(super) fn guarded_tile(row: &TrailMembershipRow) -> Option<Tile> {
    if row.access.as_deref() == Some(CONSTRAINED) {
        return None;
    }
    let mut sextant = false;
    let mut located = false;
    let mut guarded = false;
    let mut coord = None;
    for param in &row.params {
        match param.key.as_str() {
            "trail_loc" => located = true,
            "trail_sextant" if param.value == "yes" => sextant = true,
            "trail_guardian" => guarded = true,
            "trail_coord" if coord.is_none() => coord = Some(param.value.as_str()),
            _ => {}
        }
    }
    if located || !sextant || !guarded {
        return None;
    }
    decode_trail_coord(coord?)
}

/// The cap-documented wizard names the row's own `trail_guardian` family alias
/// stands for: `trail_hard` is the Zamorak Wizard and `trail_hard2` the
/// Saradomin Wizard.
///
/// The alias is a family and not an npc debugname, so this list is only ever
/// compared against a posted npc page **after** the first Dig. It is never
/// written onto the row, never used to invent a scene entity, and an unpinned
/// family has no list at all — the encounter then waits rather than Attacking
/// the nearest anything.
pub(super) fn guardian_names(row: &TrailMembershipRow) -> &'static [&'static str] {
    let alias = row
        .params
        .iter()
        .find(|param| param.key == "trail_guardian")
        .map(|param| param.value.as_str());
    match alias {
        Some("trail_hard") => &["Zamorak Wizard"],
        Some("trail_hard2") => &["Saradomin Wizard"],
        _ => &[],
    }
}

/// The local player's own posted target pair: the posted `self_target_kind`
/// and `self_target_index`. Both are needed for the read, and a page that
/// posted neither is not a target at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SelfTarget {
    pub(super) kind: i32,
    pub(super) index: i32,
}

/// This call's posted local-player target, or `None` when the page did not
/// post the pair.
pub(super) fn self_target(input: &Value) -> Option<SelfTarget> {
    Some(SelfTarget {
        kind: posted_i32(input, "self_target_kind")?,
        index: posted_i32(input, "self_target_index")?,
    })
}

/// The frozen `targetsMe`: this posted row's own target is the local player.
/// A page that posted no `self_slot` is never a match — the machine does not
/// invent the zero slot.
pub(super) fn targets_me(row: &Value, self_slot: Option<i32>) -> bool {
    posted_i32(row, "target_kind") == Some(PLAYER_KIND)
        && self_slot.is_some_and(|slot| posted_i32(row, "target_index") == Some(slot))
}

/// The mirror read: the local player's own posted target is this posted row.
/// A page that posted no pair is never a match.
pub(super) fn we_target(row: &Value, target: Option<SelfTarget>) -> bool {
    target.is_some_and(|target| {
        target.kind == NPC_KIND && posted_i32(row, "index") == Some(target.index)
    })
}

/// The frozen `sawDeath` read: the owned row posted zero health beside a posted
/// maximum, and this call's page still shows this token's fight on it — the
/// row's own posted target is the player, or the player's own posted target is
/// that row. A row that merely died beside another player is not this token's
/// kill.
pub(super) fn died_owned(row: &Value, self_slot: Option<i32>, target: Option<SelfTarget>) -> bool {
    posted_i32(row, "health") == Some(0)
        && posted_i32(row, "max_health").is_some_and(|max| max > 0)
        && (targets_me(row, self_slot) || we_target(row, target))
}

/// The frozen spawn filter over this call's posted npc page: a row whose
/// posted display name is one of the row family's cap-documented wizards,
/// whose posted actions carry `Attack`, and whose distance from this call's
/// posted `here` is inside the frozen radius on that same level. The row that
/// targets the player wins, else the nearest, then posted order — the scan
/// only replaces its best on a strict improvement, exactly like the landed
/// loc picker.
///
/// A row without an index, without a posted name, without the action, or
/// without the marshalled tile that `npc_distance` needs matches nothing, and
/// a page with no match picks nothing: the encounter waits rather than
/// Attacking the nearest anything. The name compared is the posted one and the
/// name the verb carries is that same posted string — no frozen debugname is
/// ever substituted for it.
pub(super) fn pick_npc<'a>(
    names: &[&str],
    page: &'a [Value],
    self_slot: Option<i32>,
    here: Option<Tile>,
) -> Option<(i32, &'a str)> {
    let mut best: Option<(i32, &'a str, i64, bool)> = None;
    for row in page {
        let Some(index) = posted_i32(row, "index") else {
            continue;
        };
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !names.iter().any(|wanted| name.eq_ignore_ascii_case(wanted)) {
            continue;
        }
        if !posted_action(row, ATTACK) {
            continue;
        }
        let Some(distance) = npc_distance(row, here) else {
            continue;
        };
        if distance > i64::from(GUARDIAN_RADIUS) {
            continue;
        }
        let mine = targets_me(row, self_slot);
        let better = match &best {
            None => true,
            Some((_, _, best_distance, best_mine)) => {
                (mine && !*best_mine) || (mine == *best_mine && distance < *best_distance)
            }
        };
        if better {
            best = Some((index, name, distance, mine));
        }
    }
    best.map(|(index, name, _, _)| (index, name))
}

/// The distance the spawn filter reads for one posted npc row: the posted
/// `distance` when the row carried one, else the Chebyshev distance from this
/// call's posted `here` to the row's own posted `x`/`z` tile — the same
/// level-first `max(|dx|, |dz|)` measure the landed loc picker reads its own
/// rows with.
///
/// The same level is part of the membership whichever half the distance comes
/// from: a row on another level than the posted `here` is not the wizard this
/// Dig spawned. A call that posted no `here`, and a row that did not post the
/// marshalled `x`/`z`/`level` tile, have no distance here at all — `None`, so
/// the row matches nothing rather than being measured against an invented
/// base.
pub(super) fn npc_distance(row: &Value, here: Option<Tile>) -> Option<i64> {
    let here = here?;
    if posted_i32(row, "level") != Some(here.level) {
        return None;
    }
    match posted_i32(row, "distance") {
        Some(distance) => Some(i64::from(distance)),
        None => Some(chebyshev(posted_tile(row)?, here)),
    }
}

/// A posted `{ x, z, level }` value — the wrapper's `here` tile or one posted
/// loc row. A missing, null, or malformed object is not a tile.
pub(super) fn posted_tile(value: &Value) -> Option<Tile> {
    Some(Tile {
        x: i32_of(value.get("x")?)?,
        z: i32_of(value.get("z")?)?,
        level: i32_of(value.get("level")?)?,
    })
}

/// The picker's chosen row: the posted tile and id the verb is dispatched at,
/// and the canonical action.
pub(super) struct Pick {
    pub(super) tile: Tile,
    pub(super) id: i32,
    pub(super) action: &'static str,
}

/// The frozen `pickSearchLoc` rank: `Search` before `Open`, matched
/// case-insensitively on the posted action strings and emitted canonically.
pub(super) const SEARCH_OPS: [&str; 2] = ["Search", "Open"];

/// Whether a posted page row lists `wanted` among its actions, ignoring ASCII
/// case. The loc and ground pages are the one posted action shape. A missing
/// or non-array `actions` matches nothing.
pub(super) fn posted_action(row: &Value, wanted: &str) -> bool {
    let Some(actions) = row.get("actions").and_then(Value::as_array) else {
        return false;
    };
    actions.iter().any(|action| {
        action
            .as_str()
            .is_some_and(|text| text.eq_ignore_ascii_case(wanted))
    })
}

/// The frozen picker over this call's posted loc page: posted rows on the
/// decoded tile's level, within `ARRIVE_RADIUS` Chebyshev **of the decoded
/// tile** (not of `here`), whose actions carry `Search` then `Open`.
/// Nearest wins, then the action rank, then posted order — the scan only
/// replaces the best on a strict improvement. The verb keeps the row's own
/// tile and posted id, so the host matches the type identity it was posted
/// with; a row that is not the marshalled `{ id, x, z, level, actions }`
/// shape is skipped rather than guessed at.
pub(super) fn pick_loc(input: &Value, tile: Tile) -> Option<Pick> {
    let rows = input.get("locs")?.as_array()?;
    let mut best: Option<(Pick, i64, usize)> = None;
    for row in rows {
        let (Some(id), Some(row_tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if row_tile.level != tile.level {
            continue;
        }
        let distance = chebyshev(row_tile, tile);
        if distance > i64::from(ARRIVE_RADIUS) {
            continue;
        }
        let Some(rank) = SEARCH_OPS.iter().position(|op| posted_action(row, op)) else {
            continue;
        };
        let closer = match &best {
            None => true,
            Some((_, best_distance, best_rank)) => {
                distance < *best_distance || (distance == *best_distance && rank < *best_rank)
            }
        };
        if closer {
            best = Some((
                Pick {
                    tile: row_tile,
                    id,
                    action: SEARCH_OPS[rank],
                },
                distance,
                rank,
            ));
        }
    }
    best.map(|(pick, _, _)| pick)
}

/// One posted ground row a Take can be dispatched at: the posted id that tells
/// the row apart, the name the page posted beside it, and its own posted tile.
pub(super) struct Ground<'a> {
    pub(super) id: i32,
    pub(super) name: &'a str,
    pub(super) tile: Tile,
}

/// The frozen `collectReward` ground scan: the first posted row on the posted
/// `here` tile whose actions carry `Take` and whose id is neither the frozen
/// `SHARK_ID` nor an id this collect already Dropped.
///
/// A same-tile row is at Chebyshev zero from `here`, so nearest-then-posted is
/// posted order. A row that is not the marshalled
/// `{ id, name, x, z, level, actions }` shape, a row with no posted name, and
/// every row on another tile are skipped rather than guessed at: the verb is
/// the row's own tile and the name the host resolves by first name match.
pub(super) fn pick_ground<'a>(input: &'a Value, here: Tile, discarded: &[i32]) -> Option<Ground<'a>> {
    let rows = input.get("ground")?.as_array()?;
    for row in rows {
        let (Some(id), Some(tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if id == SHARK_ID || discarded.contains(&id) || tile != here {
            continue;
        }
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !posted_action(row, TAKE) {
            continue;
        }
        return Some(Ground { id, name, tile });
    }
    None
}

/// The posted ground row this key Take dispatches at: the posted row whose own
/// id is the key the key-keeper row names, whose actions carry the frozen
/// `Take`, and whose own posted tile is on the published spawn's level inside
/// `ARRIVE_RADIUS` of that tile. Posted order decides, so the first such row
/// wins, and the verb keeps that row's own tile and the name posted beside it.
///
/// The collect's own ground scan is not this read and is never reused: the
/// frozen `DROP_RADIUS` of twelve and the shark id are the jailer's and the
/// casket's, this row is identified by the key's own id rather than by the tile
/// the player stands on, and nothing about the pack's food is read. A row that
/// is not the marshalled shape, that posted no name and every row on another
/// tile or off the radius are skipped rather than guessed at.
pub(super) fn pick_key(input: &Value, key_id: i32, spawn: Tile) -> Option<Ground<'_>> {
    let rows = input.get("ground")?.as_array()?;
    for row in rows {
        let (Some(id), Some(tile)) = (row.get("id").and_then(i32_of), posted_tile(row)) else {
            continue;
        };
        if id != key_id || posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        if chebyshev(tile, spawn) > i64::from(ARRIVE_RADIUS) {
            continue;
        }
        let Some(name) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        if !posted_action(row, TAKE) {
            continue;
        }
        return Some(Ground { id, name, tile });
    }
    None
}

/// Whether this call's posted ground page still carries `id`: the settlement a
/// Take is read through, never an inventory count.
pub(super) fn ground_posted(input: &Value, id: i32) -> bool {
    input
        .get("ground")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter()
                .any(|row| row.get("id").and_then(i32_of) == Some(id))
        })
}

/// Whether this call's posted pack page holds `id`: the same `(id, count)`
/// page the identify reads, and only a positive count holds. The box is held
/// by its own id, never by a display name and never by a scan of the page.
pub(super) fn holds(input: &Value, id: i32) -> bool {
    posted_page(input)
        .iter()
        .any(|(row_id, count)| *row_id == id && *count > 0)
}

/// Whether this call's posted pack page carries the Dig verb's item: a row
/// with a positive count whose posted display name is the frozen `Spade`,
/// compared the way the landed collect compares a posted droppable name.
///
/// The page is the already-marshalled `{ id, name, count }` sequence the
/// collect arm reads from the same `snapshot.inv`, and it is read at call
/// time. A page that does not carry the name is a `wait` — never a refusal
/// token, never `abandon`, and never a reason to fetch from a bank or scan a
/// ground spawn.
pub(super) fn spade_posted(input: &Value) -> bool {
    input
        .get("inv")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("count")
                    .and_then(i32_of)
                    .is_some_and(|count| count > 0)
                    && row
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|name| name.eq_ignore_ascii_case(SPADE_NAME))
            })
        })
}

/// The landed `talk_op` over one posted row: the first posted action whose
/// first four bytes are `talk`, ignoring ASCII case, emitted as that posted
/// string. A row that posted no talk action is not a row this arm dispatches
/// at, and the action is never canonicalized — the host receives the page's own
/// `Talk-to` or `Talk`.
pub(super) fn talk_action(row: &Value) -> Option<&str> {
    let actions = row.get("actions")?.as_array()?;
    actions.iter().find_map(|action| {
        action.as_str().and_then(|text| {
            text.get(..TALK_PREFIX.len())
                .is_some_and(|head| head.eq_ignore_ascii_case(TALK_PREFIX))
                .then_some(text)
        })
    })
}

/// The identity one posted npc page row is joined to. The identity belongs to
/// the caller's own family — a landed talk step's npc, or one published
/// `trio_givers` row — and the *rule* around it (the posted scene index, the
/// posted display name the verb carries, the posted talk action) is this
/// machine's single read.
pub(super) enum NpcIdentity<'a> {
    /// A landed talk step's own npc: the selected packed type id against the
    /// posted `id`, or the selected display name against the posted `name`, the
    /// way that arm has always joined them.
    Talk { id: i32, name: &'a str },
    /// A published `trio_givers` row, whose packed id is the identity: the
    /// posted display name is only the fallback a page that posted no id at all
    /// leaves. The `observatory_professor2` lookalike posts this giver's own
    /// display name under another packed id and is never this family.
    Giver(&'a TrioGiverRow),
    /// The toll keeper this token shops at: the selected packed type and
    /// display name of the Shantay spawn. Its verb is the one this arm
    /// dispatches — a posted `Trade`, never a talk op — so the action it rides
    /// is read by [`Self::action`] rather than by the landed `talk_op`.
    Shantay,
}

impl NpcIdentity<'_> {
    /// Whether this posted row's own identity is this one, matched exactly the
    /// way the variant names it.
    pub(super) fn names(&self, row: &Value, posted: &str) -> bool {
        match self {
            Self::Talk { id, name } => {
                posted_i32(row, "id") == Some(*id) || posted.eq_ignore_ascii_case(name)
            }
            Self::Giver(giver) => match posted_i32(row, "id") {
                Some(id) => id == giver.id,
                None => posted.eq_ignore_ascii_case(&giver.name),
            },
            Self::Shantay => match posted_i32(row, "id") {
                Some(id) => id == SHANTAY_NPC_ID,
                None => posted.eq_ignore_ascii_case(SHANTAY_NAME),
            },
        }
    }

    /// The posted action this identity's own verb rides: the talk arms read the
    /// page's own talk action (the landed `talk_op`), and the toll keeper must
    /// list the frozen `Trade` — a posted row without it is not a keeper this
    /// arm clicks. Neither arm ever invents an action the page did not post.
    pub(super) fn action<'a>(&self, row: &'a Value) -> Option<&'a str> {
        match self {
            Self::Shantay => posted_action(row, TRADE).then_some(TRADE),
            Self::Talk { .. } | Self::Giver(_) => talk_action(row),
        }
    }
}

/// One posted npc row that names the npc this caller is looking for, as the
/// pickers read it: the posted scene index the host matches, the posted display
/// name and the posted action this identity's verb rides — the landed talk
/// action for a talk step or a giver, the frozen `Trade` for the toll keeper.
///
/// The identity and its action are `NpcIdentity`'s, and the script alias is
/// never compared to a posted string: the page carries no alias at all. A row
/// that posted no index, no name, or no action this identity dispatches at is
/// not a row this arm can use.
pub(super) fn named_row<'a>(row: &'a Value, identity: &NpcIdentity<'_>) -> Option<(i32, &'a str, &'a str)> {
    let index = posted_i32(row, "index")?;
    let posted = row
        .get("name")
        .and_then(Value::as_str)
        .filter(|posted| !posted.is_empty())?;
    if !identity.names(row, posted) {
        return None;
    }
    Some((index, posted, identity.action(row)?))
}

/// One posted npc row the talk arm has picked: the posted scene index the host
/// matches, the posted name and posted talk action the verb carries, the posted
/// tile a walk would go to, and the measure the pick was ranked by.
pub(super) struct TalkPick<'a> {
    pub(super) index: i32,
    pub(super) name: &'a str,
    pub(super) action: &'a str,
    pub(super) tile: Option<Tile>,
    pub(super) distance: i64,
}

/// The identity-only picker: the nearest posted row that names this step's npc
/// and lists a talk action, measured the landed way — the posted `distance`
/// when the row carried one, else the Chebyshev distance from this call's
/// posted `here` to the row's own posted tile, on that same level. The scan
/// only replaces its best on a strict improvement, so ties keep posted order.
///
/// The five steps whose jm2 spawn is not unique take this read, and it is the
/// posted page's own answer: no alias, no first-in-file row and no frozen
/// coordinate is ever picked, and a page with no match picks nothing.
pub(super) fn pick_talk<'a>(
    identity: &NpcIdentity<'_>,
    page: &'a [Value],
    here: Tile,
) -> Option<TalkPick<'a>> {
    let mut best: Option<TalkPick<'a>> = None;
    for row in page {
        let Some((index, posted, action)) = named_row(row, identity) else {
            continue;
        };
        let Some(distance) = npc_distance(row, Some(here)) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(best) => distance < best.distance,
        };
        if better {
            best = Some(TalkPick {
                index,
                name: posted,
                action,
                tile: posted_tile(row),
                distance,
            });
        }
    }
    best
}

/// The unique-spawn picker: the nearest posted row that names this npc, lists a
/// talk action, stands on the published spawn's own level, and is inside the
/// frozen `ARRIVE_RADIUS` of that spawn — by the row's own posted tile, or by
/// the posted `distance` the page carries to this player.
///
/// The radius is part of the membership and not only of the verb: a wanderer
/// outside it is not this step's npc, so the arm keeps the published tile and
/// waits instead of chasing a second target. Strict improvement only, so ties
/// keep posted order.
///
/// The talk arm's own unique-spawn steps and the trio acquire chain's givers are
/// both this read, each over its own family's `(id, name)`; neither ever
/// substitutes an alias for the posted name it dispatches.
pub(super) fn pick_at_spawn<'a>(
    identity: &NpcIdentity<'_>,
    page: &'a [Value],
    spawn: Tile,
) -> Option<TalkPick<'a>> {
    let mut best: Option<TalkPick<'a>> = None;
    for row in page {
        let Some((index, posted, action)) = named_row(row, identity) else {
            continue;
        };
        if posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        let tile = posted_tile(row);
        let distance = posted_i32(row, "distance").map(i64::from);
        let near = distance.is_some_and(|distance| distance <= i64::from(ARRIVE_RADIUS))
            || tile.is_some_and(|tile| chebyshev(tile, spawn) <= i64::from(ARRIVE_RADIUS));
        if !near {
            continue;
        }
        let Some(distance) = distance.or_else(|| tile.map(|tile| chebyshev(tile, spawn))) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some(best) => distance < best.distance,
        };
        if better {
            best = Some(TalkPick {
                index,
                name: posted,
                action,
                tile,
                distance,
            });
        }
    }
    best
}

/// The keeper pick over this call's posted npc page: the posted row whose own
/// packed id — or, failing that, whose posted display name — is the keeper this
/// key row names, that lists the posted `Attack`, and that stands on the
/// published spawn's own level inside the frozen `ARRIVE_RADIUS` of that tile.
///
/// The sibling of `pick_at_spawn`, and never a reuse of it: the identity here is
/// the keeper's packed type and the action is `Attack`, where the talk arm's own
/// pick reads a `talk_op` over its family's npc. The radius is part of the
/// membership and not only of the verb, so a keeper further off is not this
/// tile's and the hunt waits at the published tile instead of chasing it.
/// Nearest wins and ties keep posted order — the scan only replaces its best on
/// a strict improvement. A row with no posted index, no posted name, no
/// `Attack`, another level, no distance and no marshalled tile matches nothing.
pub(super) fn pick_keeper<'a>(id: i32, name: &str, page: &'a [Value], spawn: Tile) -> Option<(i32, &'a str)> {
    let mut best: Option<(i32, &'a str, i64)> = None;
    for row in page {
        let Some(index) = posted_i32(row, "index") else {
            continue;
        };
        let Some(posted) = row
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty())
        else {
            continue;
        };
        let named = posted_i32(row, "id") == Some(id) || posted.eq_ignore_ascii_case(name);
        if !named || !posted_action(row, ATTACK) {
            continue;
        }
        if posted_i32(row, "level") != Some(spawn.level) {
            continue;
        }
        let tile = posted_tile(row);
        let distance = posted_i32(row, "distance").map(i64::from);
        let near = distance.is_some_and(|distance| distance <= i64::from(ARRIVE_RADIUS))
            || tile.is_some_and(|tile| chebyshev(tile, spawn) <= i64::from(ARRIVE_RADIUS));
        if !near {
            continue;
        }
        let Some(distance) = distance.or_else(|| tile.map(|tile| chebyshev(tile, spawn))) else {
            continue;
        };
        let better = match &best {
            None => true,
            Some((_, _, best)) => distance < *best,
        };
        if better {
            best = Some((index, posted, distance));
        }
    }
    best.map(|(index, posted, _)| (index, posted))
}

/// The landed `dialog_ready` over this call's posted chat slots: a posted
/// `chat_modal_id` that is not the closed `-1`, or a posted `chat_continue`.
///
/// Only posted facts count. A page that posted neither slot has not said the
/// chat is open, so an unobserved slot is neither an open chat nor a close, and
/// the arm is free to Talk-to.
pub(super) fn dialog_ready(input: &Value) -> bool {
    posted_i32(input, "chat_modal_id").is_some_and(|id| id != -1)
        || input.get("chat_continue").and_then(Value::as_bool) == Some(true)
}

/// The posted `count_dialog_open` slot. Only a posted `true` blocks a Talk-to
/// and only a posted `true` is a dialog to answer: an omitted slot is
/// unobserved, not a close, and a posted `false` is a closed dialog.
pub(super) fn count_open(input: &Value) -> bool {
    input.get("count_dialog_open").and_then(Value::as_bool) == Some(true)
}

/// The posted `chat_continue` slot: only a posted `true` is the frozen continue
/// step this arm may send. An omitted slot is unobserved and a posted `false` is
/// not a continue.
pub(super) fn continue_posted(input: &Value) -> bool {
    input.get("chat_continue").and_then(Value::as_bool) == Some(true)
}

/// The posted pack page's occupied slots: the rows carrying a positive count.
/// A row with no posted count is not an occupied slot. Widened so the compare
/// against the posted slot count can never wrap.
pub(super) fn occupied(input: &Value) -> i64 {
    input
        .get("inv")
        .and_then(Value::as_array)
        .map_or(0, |rows| {
            rows.iter()
                .filter(|row| {
                    row.get("count")
                        .and_then(i32_of)
                        .is_some_and(|count| count > 0)
                })
                .count() as i64
        })
}

/// The isolate scene's last post, once this session has one. The adapter
/// still echoes these scalar pages from the same post (dropping the echo is
/// the adapter rework): a page the echo carries is read from it, and a page
/// the echo omits is read from the scene.
pub(super) fn scene_page<R>(read: impl FnOnce(observed::Lens<'_>) -> R) -> Option<R> {
    observed::with(|scene| scene.applied().then(|| read(scene.latest())))
}

/// The posted player tile. No posted tile is no arrival claim.
pub(super) fn posted_here(input: &Value) -> Option<Tile> {
    match input.get("here") {
        Some(echo) => posted_tile(echo),
        None => scene_page(|page| {
            page.here().map(|tile| Tile {
                x: tile.x,
                z: tile.z,
                level: tile.level,
            })
        })
        .flatten(),
    }
}

/// The posted inventory slot count. Never invented when unposted.
pub(super) fn posted_inv_size(input: &Value) -> Option<i32> {
    match input.get("inv_size") {
        Some(echo) => i32_of(echo),
        None => scene_page(|page| page.inv_size()).flatten(),
    }
}

/// The posted `hold || ours` cooperative interrupt.
pub(super) fn posted_hold(input: &Value) -> bool {
    match input.get("hold").and_then(Value::as_bool) {
        Some(echo) => echo,
        None => scene_page(|page| page.hold().unwrap_or(false) || page.ours().unwrap_or(false))
            .unwrap_or(false),
    }
}

pub(super) fn hydrate(input: &Value) -> Cow<'_, Value> {
    if input.get("held").is_some() {
        Cow::Borrowed(input)
    } else {
        Cow::Owned(fill_from_scene(input))
    }
}

pub(super) fn fill_from_scene(input: &Value) -> Value {
    let mut out = input.clone();
    let Some(obj) = out.as_object_mut() else {
        return out;
    };
    observed::with(|scene| {
        if !scene.applied() {
            return;
        }
        let page = scene.latest();
        if !obj.contains_key("held") {
            let held: Vec<Value> = match page.inv() {
                Some(rows) => rows.iter().map(|row| json!([row.id, row.count])).collect(),
                None => Vec::new(),
            };
            obj.insert("held".into(), json!(held));
        }
        if !obj.contains_key("hold") {
            obj.insert(
                "hold".into(),
                json!(page.hold().unwrap_or(false) || page.ours().unwrap_or(false)),
            );
        }
        if !obj.contains_key("here") {
            if let Some(tile) = page.here() {
                obj.insert(
                    "here".into(),
                    json!({ "x": tile.x, "z": tile.z, "level": tile.level }),
                );
            }
        }
        if !obj.contains_key("locs") {
            if let Some(rows) = page.locs() {
                obj.insert(
                    "locs".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "id": row.id,
                            "x": row.x,
                            "z": row.z,
                            "level": row.level,
                            "actions": observed::strings(&row.actions),
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("ground") {
            if let Some(rows) = page.ground() {
                obj.insert(
                    "ground".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "id": row.id,
                            "name": row.name.as_deref(),
                            "x": row.x,
                            "z": row.z,
                            "level": row.level,
                            "actions": observed::strings(&row.actions),
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("inv") {
            if let Some(rows) = page.inv() {
                obj.insert(
                    "inv".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "id": row.id,
                            "name": row.name.as_deref(),
                            "count": row.count,
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("npcs") {
            if let Some(rows) = page.npcs() {
                obj.insert(
                    "npcs".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "index": row.index,
                            "id": row.id,
                            "name": row.name.as_deref(),
                            "x": row.x,
                            "z": row.z,
                            "level": row.level,
                            "distance": row.distance,
                            "health": row.health,
                            "max_health": row.max_health,
                            "in_combat": row.in_combat,
                            "actions": observed::strings(&row.actions),
                            "target_kind": row.target_kind,
                            "target_index": row.target_index,
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("equipment") {
            if let Some(rows) = page.equipment() {
                obj.insert(
                    "equipment".into(),
                    json!(rows
                        .iter()
                        .map(|row| {
                            let mut item = serde_json::Map::new();
                            if let Some(name) = row.name.as_deref() {
                                item.insert("name".into(), json!(name));
                            }
                            item.insert("id".into(), json!(row.id));
                            item.insert("count".into(), json!(row.count));
                            if let Some(slot) = row.slot {
                                item.insert("slot".into(), json!(slot));
                            }
                            Value::Object(item)
                        })
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("nearest_booth") {
            if let Some(booth) = page.nearest_booth() {
                let mut booth_json = json!({
                    "x": booth.tile.x,
                    "z": booth.tile.z,
                    "level": booth.tile.level,
                    "id": booth.id,
                });
                if let Some(name) = booth.name.as_deref() {
                    booth_json["name"] = json!(name);
                }
                if let Some(op) = booth.op.as_deref() {
                    booth_json["op"] = json!(op);
                }
                obj.insert("nearest_booth".into(), booth_json);
            }
        }
        if !obj.contains_key("bank_open") {
            if let Some(open) = page.bank_open() {
                obj.insert("bank_open".into(), json!(open));
            }
        }
        if !obj.contains_key("main_modal_id") {
            if let Some(id) = page.main_modal_id() {
                obj.insert("main_modal_id".into(), json!(id));
            }
        }
        if !obj.contains_key("chat_modal_id") {
            if let Some(id) = page.chat_modal_id() {
                obj.insert("chat_modal_id".into(), json!(id));
            }
        }
        if !obj.contains_key("chat_continue") {
            if let Some(cont) = page.chat_continue() {
                obj.insert("chat_continue".into(), json!(cont));
            }
        }
        if !obj.contains_key("chat_options") {
            if let Some(rows) = page.chat_options() {
                obj.insert(
                    "chat_options".into(),
                    json!(rows
                        .iter()
                        .enumerate()
                        .map(|(i, text)| json!({ "text": text, "option": i as i32 + 1 }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("count_dialog_open") {
            if let Some(open) = page.count_dialog_open() {
                obj.insert("count_dialog_open".into(), json!(open));
            }
        }
        if !obj.contains_key("inv_size") {
            if let Some(size) = page.inv_size() {
                obj.insert("inv_size".into(), json!(size));
            }
        }
        if !obj.contains_key("self_slot") {
            if let Some(slot) = page.self_slot() {
                obj.insert("self_slot".into(), json!(slot));
            }
        }
        if !obj.contains_key("self_target_kind") {
            if let (Some(kind), Some(index)) = (page.self_target_kind(), page.self_target_index()) {
                obj.insert("self_target_kind".into(), json!(kind));
                obj.insert("self_target_index".into(), json!(index));
            }
        }
        if !obj.contains_key("hitpoints") {
            if let Some(hp) = page.stats().and_then(|stats| stats.hitpoints) {
                obj.insert("hitpoints".into(), json!(hp.effective));
            }
        }
        if !obj.contains_key("varp95") {
            if let Some(value) = page
                .varps()
                .and_then(|rows| rows.iter().find(|row| row.index == 95).map(|row| row.value))
            {
                obj.insert("varp95".into(), json!(value));
            }
        }
        if !obj.contains_key("puzzle_board") {
            if let Some(board) = page.puzzle_board() {
                obj.insert(
                    "puzzle_board".into(),
                    json!({
                        "component_id": board.component_id,
                        "size": board.size,
                        "items": board
                            .items
                            .iter()
                            .map(|row| json!({ "slot": row.slot, "id": row.id }))
                            .collect::<Vec<_>>(),
                    }),
                );
                if !obj.contains_key("puzzle_board_generation") {
                    obj.insert("puzzle_board_generation".into(), json!(board.generation));
                }
            }
        }
        if !obj.contains_key("walk_missing_carry") {
            if let Some(rows) = page.walk_missing_carry() {
                obj.insert(
                    "walk_missing_carry".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "id": row.id,
                            "count": row.count,
                            "name": row.name.as_deref(),
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
        if !obj.contains_key("shop_open") {
            if let Some(open) = page.shop_open() {
                obj.insert("shop_open".into(), json!(open));
            }
        }
        if !obj.contains_key("shop_stock") {
            if let Some(rows) = page.shop_stock() {
                obj.insert(
                    "shop_stock".into(),
                    json!(rows
                        .iter()
                        .map(|row| json!({
                            "id": row.id,
                            "name": row.name.as_deref(),
                            "count": row.count,
                            "slot": row.slot,
                            "component": row.component_id,
                        }))
                        .collect::<Vec<_>>()),
                );
            }
        }
    });
    out
}
