use super::*;

// ---------------------------------------------------------------------------
// Generic door-category members and in-place swap doors.
// ---------------------------------------------------------------------------

/// The generic closed-door categories of `scripts/doors/scripts/doors.rs2`,
/// `doubledoors.rs2`, `open_and_close_doors.rs2` and
/// `open_and_close_double_doors.rs2`: `(category, exact category handler
/// body with all whitespace removed, the proc it forwards to)`. The engine
/// resolves a loc's open script as loc-specific `[oploc1,<name>]` >
/// category `[oploc1,_<category>]`, so a member with no loc-specific block
/// runs this body. The open-and-close procs walk the player through either
/// way (`~check_axis` only picks the side) and shut the door behind them.
pub(super) const DOOR_CATEGORY_HANDLERS: [(&str, &str, &str); 9] = [
    ("door_closed", "~open_door(500);", "open_door"),
    (
        "reverse_door_closed",
        "~open_door_reversed(500);",
        "open_door_reversed",
    ),
    (
        "door_left_closed",
        "~open_double_doors_left(500,door_right_closed,loc_param(open_sound));",
        "open_double_doors_left",
    ),
    (
        "door_right_closed",
        "~open_double_doors_right(500,door_left_closed,loc_param(open_sound));",
        "open_double_doors_right",
    ),
    (
        "reverse_door_left_closed",
        "~open_reverse_double_doors_left(500,reverse_door_right_closed,loc_param(open_sound));",
        "open_reverse_double_doors_left",
    ),
    (
        "reverse_door_right_closed",
        "~open_reverse_double_doors_right(500,reverse_door_left_closed,loc_param(open_sound));",
        "open_reverse_double_doors_right",
    ),
    (
        "door_open_and_close",
        "if(string_length(loc_param(game_message))!0){mes(loc_param(game_message));}~open_and_close_door(loc_param(next_loc_stage),~check_axis(coord,loc_coord,loc_angle),false);",
        "open_and_close_door",
    ),
    (
        "double_door_open_and_close_left",
        "~open_and_close_double_door(~check_axis_locactive(coord),^left);",
        "open_and_close_double_door",
    ),
    (
        "double_door_open_and_close_right",
        "~open_and_close_double_door(~check_axis_locactive(coord),^right);",
        "open_and_close_double_door",
    ),
];

/// The only category whose open moves the leaf off the door's own tile:
/// `~open_door` `loc_del`s the door and `loc_add`s `next_loc_stage` at
/// `~door_open(loc_angle, loc_shape)` — for a `wall_diagonal` the next
/// cardinal tile clockwise of the angle's side — so a diagonal door's tile
/// frees. `~open_door_reversed` re-adds the leaf on the same tile, and the
/// double-door procs always add a `wall_straight`.
pub(super) const DOOR_CLOSED: &str = "door_closed";

/// The door categories whose generic handler is verified in this content:
/// every `[oploc1,_<category>]` block is exactly the forwarding call and the
/// forwarded `[proc,…]` exists. A category with a differently shaped or
/// disagreeing handler, or a missing proc, supports nothing.
pub(super) fn generic_door_handlers(content_root: &Path) -> HashSet<&'static str> {
    let mut bodies: HashMap<&'static str, bool> = HashMap::new();
    let mut procs: HashSet<&'static str> = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (category, body) in oploc1_category_bodies(text) {
            let Some(&(name, expected, _)) = DOOR_CATEGORY_HANDLERS
                .iter()
                .find(|(name, _, _)| *name == category)
            else {
                continue;
            };
            let ok = normalized_body(&body) == expected;
            bodies
                .entry(name)
                .and_modify(|prev| *prev &= ok)
                .or_insert(ok);
        }
        for (_, _, proc) in DOOR_CATEGORY_HANDLERS {
            if !proc_bodies(text, proc).is_empty() {
                procs.insert(proc);
            }
        }
    });
    DOOR_CATEGORY_HANDLERS
        .iter()
        .filter(|(name, _, proc)| {
            bodies.get(name).copied().unwrap_or(false) && procs.contains(proc)
        })
        .map(|(name, _, _)| *name)
        .collect()
}

/// One `.loc` block's door facts: its generic closed-door category (if it
/// declares one), its `op1=Open` line and its resolved `next_loc_stage`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DoorBlock {
    pub(super) category: Option<&'static str>,
    pub(super) op_open: bool,
    pub(super) open: Option<i32>,
}

/// Every `.loc` block of a text that resolves to a loc id, with its
/// [`DoorBlock`] facts. Numeric `[loc_N]` aliases resolve through
/// [`loc_pack_id`]; every valid `[header]` ends the previous block.
pub(super) fn door_blocks(text: &str, ids: &HashMap<String, i32>) -> Vec<(i32, DoorBlock)> {
    let mut out = Vec::new();
    let mut cur: Option<(i32, DoorBlock)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = config_header(line) {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            cur = loc_pack_id(name, ids).map(|id| {
                (
                    id,
                    DoorBlock {
                        category: None,
                        op_open: false,
                        open: None,
                    },
                )
            });
            continue;
        }
        let Some((_, block)) = cur.as_mut() else {
            continue;
        };
        if line == "op1=Open" {
            block.op_open = true;
        } else if let Some(value) = line.strip_prefix("category=") {
            block.category = DOOR_CATEGORY_HANDLERS
                .iter()
                .find(|(name, _, _)| *name == value.trim())
                .map(|(name, _, _)| *name);
        } else if let Some(rest) = line.strip_prefix("param=") {
            if let Some((key, value)) = rest.split_once(',') {
                if key.trim() == "next_loc_stage" {
                    block.open = stage_open_loc_id(value.trim(), ids);
                }
            }
        }
    }
    if let Some(done) = cur {
        out.push(done);
    }
    out
}

/// A generic closed door admitted from the content: its category and the
/// `next_loc_stage` open leaf its handler adds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GenericDoor {
    pub(super) category: &'static str,
    pub(super) open: i32,
}

/// Every loc (any `.loc` under `scripts`, the `_unpack` trees included —
/// the pack tool compiles them) whose open runs a verified generic door
/// category handler, `loc id → GenericDoor`. A member is admitted only while
/// the whole inheritance is provable:
///
/// - the block declares a generic closed-door category with `op1=Open`;
/// - that category's handler is verified ([`generic_door_handlers`]);
/// - its `next_loc_stage` resolves through `pack/loc.pack`;
/// - no loc-specific `[oploc1,<name>]` exists for any of its names (a named
///   override — gated, scripted or denied — wins and is never promoted);
/// - it is not declared twice with disagreeing data.
///
/// Anything else is counted and left out.
pub(super) fn generic_closed_doors(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    overridden: &HashSet<String>,
    skipped: &mut HashMap<&'static str, usize>,
) -> HashMap<i32, GenericDoor> {
    let supported = generic_door_handlers(content_root);
    let mut defs: HashMap<i32, DoorBlock> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    let mut names: HashMap<i32, HashSet<String>> = HashMap::new();
    visit_loc_configs(&content_root.join("scripts"), &mut |text| {
        for raw in text.lines() {
            if let Some(name) = config_header(raw.trim()) {
                if let Some(id) = loc_pack_id(name, ids) {
                    names.entry(id).or_default().insert(name.to_string());
                }
            }
        }
        for (id, block) in door_blocks(text, ids) {
            if block.category.is_none() {
                continue;
            }
            match defs.get(&id) {
                Some(prev) if *prev != block => {
                    defs.remove(&id);
                    conflicted.insert(id);
                }
                Some(_) => {}
                None if conflicted.contains(&id) => {}
                None => {
                    defs.insert(id, block);
                }
            }
        }
    });
    bump(skipped, SKIP_DOOR_MEMBER_CONFLICT, conflicted.len());
    let mut out = HashMap::new();
    for (id, block) in defs {
        let Some(category) = block.category else {
            continue;
        };
        if !block.op_open {
            bump(skipped, SKIP_DOOR_MEMBER_SHAPE, 1);
            continue;
        }
        if !supported.contains(category) {
            bump(skipped, SKIP_DOOR_MEMBER_HANDLER, 1);
            continue;
        }
        if names
            .get(&id)
            .is_some_and(|ns| ns.iter().any(|n| overridden.contains(n)))
        {
            bump(skipped, SKIP_DOOR_MEMBER_OVERRIDE, 1);
            continue;
        }
        let Some(open) = block.open else {
            bump(skipped, SKIP_DOOR_MEMBER_STAGE, 1);
            continue;
        };
        out.insert(id, GenericDoor { category, open });
    }
    out
}

/// Every loc name with a loc-specific `[oploc1,<name>]` block anywhere under
/// `scripts` (same-line bodies included, [`oploc1_names`]).
pub(super) fn oploc1_overrides(content_root: &Path) -> HashSet<String> {
    let mut overridden = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        oploc1_names(text, &mut overridden);
    });
    overridden
}

/// In-place swap doors, `closed loc id → open loc id`: a loc whose own
/// `[oploc1,<name>]` block is exactly `loc_change(<open>, <duration>);`
/// (optionally followed by one `sound_synth(…);`), whose config declares
/// `op1=Open`, and whose open loc does not block walk in the cache defs
/// (the Al Kharid curtains `loc_1528` → `loc_1529` and the Rellekka
/// `viking_fur_door` → `viking_fur_door_open`, both `blockwalk=no`).
/// `loc_change` keeps the tile, shape and angle, so the wall's passage is
/// free while the open loc stands; any other body (a gate, a check, a
/// teleport) is not this shape and is left out.
pub(super) fn swap_doors(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    loc_defs: &LocDefs,
    skipped: &mut HashMap<&'static str, usize>,
) -> HashMap<i32, i32> {
    let mut bodies: HashMap<String, Vec<String>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, body) in script_blocks(text) {
            if op == "oploc1" {
                bodies.entry(name).or_default().push(body);
            }
        }
    });
    let mut openable: HashSet<i32> = HashSet::new();
    visit_loc_configs(&content_root.join("scripts"), &mut |text| {
        for (id, block) in door_blocks(text, ids) {
            if block.op_open {
                openable.insert(id);
            }
        }
    });
    let mut out = HashMap::new();
    for (name, blocks) in bodies {
        let [body] = blocks.as_slice() else {
            continue;
        };
        let flat = normalized_body(body);
        let Some(open_name) = swap_body_target(&flat) else {
            continue;
        };
        let (Some(closed), Some(open)) = (loc_pack_id(&name, ids), loc_pack_id(open_name, ids))
        else {
            continue;
        };
        if !openable.contains(&closed) {
            continue;
        }
        if loc_defs.loc(open).is_none_or(|d| d.block_walk) {
            bump(skipped, SKIP_SWAP_DOOR_BLOCKS, 1);
            continue;
        }
        out.insert(closed, open);
    }
    out
}

/// `loc_change(<name>,<int>);` with an optional trailing
/// `sound_synth(…);`, whitespace already removed → `<name>`.
pub(super) fn swap_body_target(flat: &str) -> Option<&str> {
    let rest = flat.strip_prefix("loc_change(")?;
    let (args, tail) = rest.split_once(");")?;
    let (name, duration) = args.split_once(',')?;
    if name.is_empty()
        || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
        || duration.parse::<u32>().is_err()
    {
        return None;
    }
    let tail_ok = tail.is_empty()
        || tail
            .strip_prefix("sound_synth(")
            .and_then(|t| t.strip_suffix(");"))
            .is_some_and(|args| !args.contains(';') && !args.contains('('));
    tail_ok.then_some(name)
}
