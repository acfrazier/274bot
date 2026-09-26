use super::*;

/// Door loc ids from `content/scripts/doors/configs/*.loc` text: every
/// `[loc_N]` block that can open, i.e. has `op1=Open` or
/// `category=door_closed`/`category=gate_main_closed`/
/// `category=gate_outer_closed` (the fence-gate closed categories, same
/// openability as `door_closed`). Non-numeric blocks (e.g. `[membergatel]`)
/// are ignored, as are the `op1=Close`/`*_open` counterpart states.
pub fn parse_door_config(text: &str) -> HashSet<i32> {
    parse_door_config_ids(text, &HashMap::new())
}

/// The [`parse_door_config`] rule with the `scripts/areas/*/configs`
/// header style: `[loc_N]` blocks parse directly and `[name]` blocks
/// resolve through `ids` (the `pack/loc.pack` map), so e.g. the Al Kharid
/// toll gates (`border_gate.loc`'s `[border_gate_toll_left/_right]`,
/// `op1=Open`) join the door set under their own names. Numeric blocks
/// behave exactly as in [`parse_door_config`].
pub fn parse_door_config_ids(text: &str, ids: &HashMap<String, i32>) -> HashSet<i32> {
    let mut door_ids = HashSet::new();
    let mut cur: Option<i32> = None;
    let mut openable = false;
    for raw in text.lines() {
        let line = raw.trim();
        let header =
            loc_header(line).or_else(|| named_loc_header(line).and_then(|n| ids.get(n).copied()));
        if let Some(n) = header {
            if let Some(id) = cur {
                if openable {
                    door_ids.insert(id);
                }
            }
            cur = Some(n);
            openable = false;
        } else if cur.is_some()
            && (line == "op1=Open"
                || line == "category=door_closed"
                || line == "category=gate_main_closed"
                || line == "category=gate_outer_closed")
        {
            openable = true;
        }
    }
    if let Some(id) = cur {
        if openable {
            door_ids.insert(id);
        }
    }
    door_ids
}

/// Closed door loc id → its open leaf id: every `[loc_N]` (or `[name]`,
/// resolved through `ids`) block's `param=next_loc_stage,loc_M` (the id
/// the door changes into when opened).
/// Name-valued params (`param=next_loc_stage,<name>`) resolve through the
/// loc id map; unparseable values carry nothing.
pub fn parse_door_open_ids(text: &str, ids: &HashMap<String, i32>) -> HashMap<i32, i32> {
    let mut out = HashMap::new();
    let mut cur: Option<(i32, Option<i32>)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        let header =
            loc_header(line).or_else(|| named_loc_header(line).and_then(|n| ids.get(n).copied()));
        if let Some(n) = header {
            if let Some((id, Some(open))) = cur {
                out.insert(id, open);
            }
            cur = Some((n, None));
        } else if let Some((_, open)) = cur.as_mut() {
            if let Some(param) = line.strip_prefix("param=") {
                if let Some((key, value)) = param.split_once(',') {
                    if key.trim() == "next_loc_stage" {
                        *open = door_open_value(value.trim(), ids);
                    }
                }
            }
        }
    }
    if let Some((id, Some(open))) = cur {
        out.insert(id, open);
    }
    out
}

/// A `param=next_loc_stage` value → the open leaf id: `loc_N` parses
/// numerically; a bare name resolves through the loc id map.
pub(super) fn door_open_value(value: &str, ids: &HashMap<String, i32>) -> Option<i32> {
    if let Some(n) = value.strip_prefix("loc_") {
        n.parse().ok()
    } else if let Some(&id) = ids.get(value) {
        Some(id)
    } else {
        None
    }
}

/// Loc ids that do **not** block walk: `[loc_N]` blocks with `blockwalk=no`,
/// `category=door_opened`, or `op1=Close`. Absent `blockwalk` is the 274
/// default (block). Unknown loc ids are treated as blocking by the bake.
pub fn parse_passable_locs(text: &str) -> HashSet<i32> {
    let mut ids = HashSet::new();
    let mut cur: Option<i32> = None;
    let mut passable = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(n) = loc_header(line) {
            if let Some(id) = cur {
                if passable {
                    ids.insert(id);
                }
            }
            cur = Some(n);
            passable = false;
        } else if cur.is_some()
            && (line == "blockwalk=no" || line == "category=door_opened" || line == "op1=Close")
        {
            passable = true;
        }
    }
    if let Some(id) = cur {
        if passable {
            ids.insert(id);
        }
    }
    ids
}

/// `[loc_N]` block header -> `N`.
pub(super) fn loc_header(line: &str) -> Option<i32> {
    line.strip_prefix("[loc_")?.strip_suffix(']')?.parse().ok()
}

/// `[<name>]` block header -> the name (the `scripts/areas/*/configs`
/// style, e.g. `[border_gate_toll_left]`).
pub(super) fn named_loc_header(line: &str) -> Option<&str> {
    let name = line.strip_prefix('[')?.strip_suffix(']')?;
    if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return None;
    }
    Some(name)
}
