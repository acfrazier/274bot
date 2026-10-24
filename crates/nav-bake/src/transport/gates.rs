use super::*;

// ---------------------------------------------------------------------------
// Inherited closed fence gates (quest/area configs).
// ---------------------------------------------------------------------------

/// The two closed fence-gate categories and their generic category handlers
/// in `scripts/general_use/scripts/gates.rs2`:
/// `[oploc1,_gate_main_closed] ~open_gate;` (main) and
/// `[oploc1,_gate_outer_closed] ~open_outer_gate;` (outer). The engine
/// resolves a loc's open script as loc-specific `[oploc1,<name>]` >
/// category `[oploc1,<category>]` > global, so a member with no
/// loc-specific block inherits the category handler; `open_gate` opens the
/// main gate plus the adjacent paired outer (`get_pair_coord`), and
/// `open_outer_gate` resolves the adjacent main. Those two forwarding
/// bodies plus the two procs are the whole supported behavior.
pub(super) const GATE_MAIN_CLOSED: &str = "gate_main_closed";
pub(super) const GATE_OUTER_CLOSED: &str = "gate_outer_closed";
pub(super) const GATE_MAIN_OPEN: &str = "gate_main_open";
pub(super) const GATE_OUTER_OPEN: &str = "gate_outer_open";
pub(super) const GATE_MAIN_HANDLER: &str = "~open_gate;";
pub(super) const GATE_OUTER_HANDLER: &str = "~open_outer_gate;";
pub(super) const GATE_MAIN_PROC: &str = "open_gate";
pub(super) const GATE_OUTER_PROC: &str = "open_outer_gate";

/// The members check a members fence gate's own `[oploc1,<name>]` block
/// runs before the generic open call (Paterdomus `memberfencegate_l/_r`,
/// whitespace removed): on a free world it refuses, on a members world the
/// body is exactly the category handler.
pub(super) const GATE_MEMBERS_CHECK: &str =
    "if(map_members=^false){mes(^mes_members_gate);return;}";

/// Loc names whose every `[oploc1,<name>]` block is exactly
/// [`GATE_MEMBERS_CHECK`] followed by one generic gate call, `name → outer`
/// (the flavor the call opens). Such an override is the generic handler
/// behind `MAP_MEMBERS`, so its gate inherits with `members_req`; any other
/// override body is not this shape.
pub(super) fn members_gate_handlers(content_root: &Path) -> HashMap<String, bool> {
    let mut bodies: HashMap<String, Vec<Option<bool>>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, body) in script_blocks(text) {
            if op != "oploc1" {
                continue;
            }
            let flat = normalized_body(&body);
            let outer = flat
                .strip_prefix(GATE_MEMBERS_CHECK)
                .and_then(|call| match call {
                    GATE_MAIN_HANDLER => Some(false),
                    GATE_OUTER_HANDLER => Some(true),
                    _ => None,
                });
            bodies.entry(name).or_default().push(outer);
        }
    });
    bodies
        .into_iter()
        .filter_map(|(name, outers)| match outers.as_slice() {
            [Some(outer)] => Some((name, *outer)),
            _ => None,
        })
        .collect()
}

/// A closed-gate `category=` value → the flavor (`false` main, `true`
/// outer). Every other category is `None`: an unknown category has no
/// verified handler and is never inherited.
pub(super) fn closed_gate_category(value: &str) -> Option<bool> {
    match value.trim() {
        GATE_MAIN_CLOSED => Some(false),
        GATE_OUTER_CLOSED => Some(true),
        _ => None,
    }
}

/// The open leaf's category for a closed-gate flavor (`gate_main_open` /
/// `gate_outer_open`).
pub(super) fn open_gate_category(outer: bool) -> &'static str {
    if outer {
        GATE_OUTER_OPEN
    } else {
        GATE_MAIN_OPEN
    }
}

/// The `[oploc1,_gate_main_closed]` / `[oploc1,_gate_outer_closed]`
/// category handlers in a script text → `(outer, body)`, read by
/// [`oploc1_category_bodies`] (inline and next-line bodies alike).
pub(super) fn gate_category_handlers(text: &str) -> Vec<(bool, String)> {
    oploc1_category_bodies(text)
        .into_iter()
        .filter_map(|(category, body)| Some((closed_gate_category(&category)?, body)))
        .collect()
}

/// Loc names targeted by `[oploc1,<name>]` in a script text, including
/// same-line bodies (`[oploc1,foo] ~open_gate;`) that [`script_blocks`]
/// cannot see.
pub(super) fn oploc1_names(text: &str, into: &mut HashSet<String>) {
    for raw in text.lines() {
        let line = match raw.find("//") {
            Some(i) => &raw[..i],
            None => raw,
        };
        let line = line.trim();
        let Some(rest) = line.strip_prefix('[') else {
            continue;
        };
        let Some((header, _)) = rest.split_once(']') else {
            continue;
        };
        let Some((op, name)) = header.split_once(',') else {
            continue;
        };
        if op.trim() == "oploc1" {
            let name = name.trim();
            if !name.is_empty() {
                into.insert(name.to_string());
            }
        }
    }
}

/// The closed-gate categories this content actually supports, as the
/// verified generic open behavior: the `[oploc1,_gate_*_closed]` body must
/// be exactly the forwarding call (whitespace/comments ignored) and the
/// forwarded `[proc,…]` must be defined somewhere under `scripts`. A
/// category with no such block, a differently shaped body (any extra
/// statement proves nothing), a disagreeing duplicate, or a missing proc
/// is not supported — nothing inherits from it.
pub(super) fn generic_gate_handlers(content_root: &Path) -> HashSet<bool> {
    let mut bodies: HashMap<bool, bool> = HashMap::new();
    let mut procs: HashSet<&'static str> = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (outer, body) in gate_category_handlers(text) {
            let expected = if outer {
                GATE_OUTER_HANDLER
            } else {
                GATE_MAIN_HANDLER
            };
            let ok = normalized_body(&body) == expected;
            bodies
                .entry(outer)
                .and_modify(|prev| *prev &= ok)
                .or_insert(ok);
        }
        for proc in [GATE_MAIN_PROC, GATE_OUTER_PROC] {
            if !proc_bodies(text, proc).is_empty() {
                procs.insert(proc);
            }
        }
    });
    [false, true]
        .into_iter()
        .filter(|&outer| {
            let proc = if outer {
                GATE_OUTER_PROC
            } else {
                GATE_MAIN_PROC
            };
            bodies.get(&outer).copied().unwrap_or(false) && procs.contains(proc)
        })
        .collect()
}

/// One `.loc` block that declares a closed gate category, as read from the
/// quest/area config trees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct InheritedGate {
    /// The block's closed category (`false` main, `true` outer); `None`
    /// when the block declares neither.
    category: Option<bool>,
    /// The block's `op1=Open` line.
    op_open: bool,
    /// The `next_loc_stage` open leaf id, resolved through `pack/loc.pack`.
    open: Option<i32>,
}

impl InheritedGate {
    fn new() -> Self {
        InheritedGate {
            category: None,
            op_open: false,
            open: None,
        }
    }
}

/// Every closed-gate declaration in one `.loc` text: `(id, gate)` per block
/// that names one of the two closed gate categories, with the block's
/// `op1=Open` line and its resolved `next_loc_stage` open leaf. Numeric
/// `[loc_N]` aliases resolve through [`loc_pack_id`]; every syntactically
/// valid `[header]` ends the previous block even when the name does not
/// resolve. Unresolved headers and blocks whose category is `gate_*_open`
/// (the open leaves) yield nothing.
pub(super) fn closed_gate_blocks(
    text: &str,
    ids: &HashMap<String, i32>,
) -> Vec<(i32, InheritedGate)> {
    let mut out = Vec::new();
    let mut cur: Option<(i32, InheritedGate)> = None;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(name) = config_header(line) {
            if let Some(done) = cur.take() {
                out.push(done);
            }
            if let Some(id) = loc_pack_id(name, ids) {
                cur = Some((id, InheritedGate::new()));
            }
            continue;
        }
        let Some((_, gate)) = cur.as_mut() else {
            continue;
        };
        if line == "op1=Open" {
            gate.op_open = true;
        } else if let Some(value) = line.strip_prefix("category=") {
            if let Some(outer) = closed_gate_category(value) {
                gate.category = Some(outer);
            }
        } else if let Some(rest) = line.strip_prefix("param=") {
            if let Some((key, value)) = rest.split_once(',') {
                if key.trim() == "next_loc_stage" {
                    gate.open = stage_open_loc_id(value.trim(), ids);
                }
            }
        }
    }
    if let Some(done) = cur {
        out.push(done);
    }
    out
}

/// The tile of a closed gate's paired counterpart, from the
/// `[proc,get_pair_coord]` rule in `scripts/general_use/scripts/gates.rs2`:
/// the offset runs along the gate's wall (`west`→z+1, `north`→x+1, the
/// south/east cases mirrored) and `$outer` flips it. A gate member cannot
/// be read as the generic pair without it.
pub(super) fn gate_pair_tile(at: WorldTile, angle_dir: DoorDir, outer: bool) -> WorldTile {
    let dir = if outer { -1 } else { 1 };
    match angle_dir {
        DoorDir::W => WorldTile {
            x: at.x,
            z: at.z + dir,
            level: at.level,
        },
        DoorDir::N => WorldTile {
            x: at.x + dir,
            z: at.z,
            level: at.level,
        },
        DoorDir::E => WorldTile {
            x: at.x,
            z: at.z - dir,
            level: at.level,
        },
        DoorDir::S => WorldTile {
            x: at.x - dir,
            z: at.z,
            level: at.level,
        },
    }
}

/// Closed fence-gate members declared outside
/// `scripts/general_use/configs/gates.loc` (the `scripts/quests` and
/// `scripts/areas` config trees), `loc id → next_loc_stage open leaf id`.
/// A member is admitted only while its whole generic inheritance is
/// provable from the same canonical data:
///
/// - the block declares one of the two closed gate categories with
///   `op1=Open` (both, not either: a member without the open op is a shape
///   this derivation does not support);
/// - that category's generic handler is verified in the content
///   ([`generic_gate_handlers`], handler body plus proc);
/// - a `next_loc_stage` open leaf resolves through loc.pack, and that
///   leaf's own config is present with one unconflicted matching
///   `gate_*_open` category;
/// - no loc-specific `[oploc1,<name>]` block exists anywhere under
///   `scripts` (the resolver's first priority — a named override, gated or
///   denied, is never promoted), unless every override is the canonical
///   members check in front of this flavor's generic call (`members`,
///   [`members_gate_handlers`]): that member crosses with `members_req`;
/// - every level-0 placement resolves its paired counterpart
///   ([`gate_pair_tile`]) to a placement of the complementary closed
///   category, so the pair the generic handler walks is really there;
/// - the member is not defined twice with disagreeing data.
///
/// Anything unresolved, unsupported or malformed is counted and left out,
/// never promoted to an ungated crossing.
pub(super) fn inherited_closed_gates(
    content_root: &Path,
    ids: &HashMap<String, i32>,
    positions: &HashMap<i32, Vec<Placement>>,
    supported: &HashSet<bool>,
    members: &HashMap<String, bool>,
    skipped: &mut HashMap<&'static str, usize>,
) -> HashMap<i32, i32> {
    let mut defs: HashMap<i32, InheritedGate> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    let mut names: HashMap<i32, HashSet<String>> = HashMap::new();
    let scripts = content_root.join("scripts");
    let mut pending = vec![scripts.join("quests"), scripts.join("areas")];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let path = ent.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|s| s.to_str()) != Some("loc") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            for raw in text.lines() {
                let line = raw.trim();
                if let Some(name) = config_header(line) {
                    if let Some(id) = loc_pack_id(name, ids) {
                        names.entry(id).or_default().insert(name.to_string());
                    }
                }
            }
            for (id, gate) in closed_gate_blocks(&text, ids) {
                match defs.get(&id) {
                    Some(prev) if *prev != gate => {
                        defs.remove(&id);
                        conflicted.insert(id);
                    }
                    Some(_) => {}
                    None if conflicted.contains(&id) => {}
                    None => {
                        defs.insert(id, gate);
                    }
                }
            }
        }
    }
    // Every loc id any `.loc` under `scripts` names → its category, for the
    // open-leaf and pair checks (the leaf of a `gates.loc` member is
    // defined there, not in the member's own config).
    let mut categories: HashMap<i32, String> = HashMap::new();
    let mut category_conflicted: HashSet<i32> = HashSet::new();
    visit_loc_configs(&scripts, &mut |text| {
        let mut cur: Option<i32> = None;
        for raw in text.lines() {
            let line = raw.trim();
            if let Some(name) = config_header(line) {
                cur = loc_pack_id(name, ids);
                continue;
            }
            let Some(id) = cur else {
                continue;
            };
            if let Some(value) = line.strip_prefix("category=") {
                let value = value.trim();
                match categories.get(&id) {
                    Some(previous) if previous != value => {
                        categories.remove(&id);
                        category_conflicted.insert(id);
                    }
                    Some(_) => {}
                    None if category_conflicted.contains(&id) => {}
                    None => {
                        categories.insert(id, value.to_string());
                    }
                }
            }
        }
    });
    // Loc-specific `[oploc1,<name>]` blocks, the resolver's first priority.
    // Same-line bodies (`[oploc1,foo] ~open_gate;`) are included:
    // [`script_blocks`] cannot see a header that is not alone on the line.
    let mut overridden: HashSet<String> = HashSet::new();
    visit_rs2(&scripts, &mut |text| {
        oploc1_names(text, &mut overridden);
    });
    // Every placed loc id per tile, for the pair check.
    let mut placed: HashMap<(i32, i32, i32), Vec<i32>> = HashMap::new();
    for (&id, ps) in positions {
        for p in ps {
            placed.entry((p.x, p.z, p.level)).or_default().push(id);
        }
    }
    let mut out = HashMap::new();
    for (id, gate) in defs {
        let Some(outer) = gate.category else {
            continue;
        };
        if conflicted.contains(&id) {
            bump(skipped, SKIP_GATE_MEMBER_CONFLICT, 1);
            continue;
        }
        if !gate.op_open {
            bump(skipped, SKIP_GATE_MEMBER_SHAPE, 1);
            continue;
        }
        let Some(open) = gate.open else {
            bump(skipped, SKIP_GATE_MEMBER_STAGE, 1);
            continue;
        };
        if !supported.contains(&outer) {
            bump(skipped, SKIP_GATE_MEMBER_HANDLER, 1);
            continue;
        }
        if names.get(&id).is_some_and(|ns| {
            ns.iter()
                .any(|n| overridden.contains(n) && members.get(n) != Some(&outer))
        }) {
            bump(skipped, SKIP_GATE_MEMBER_OVERRIDE, 1);
            continue;
        }
        if category_conflicted.contains(&open)
            || categories.get(&open).map(String::as_str) != Some(open_gate_category(outer))
        {
            bump(skipped, SKIP_GATE_MEMBER_STAGE, 1);
            continue;
        }
        let paired = positions.get(&id).is_none_or(|ps| {
            ps.iter().filter(|p| p.level == 0).all(|p| {
                let Some(angle_dir) = door_dir(p.angle) else {
                    return false;
                };
                let at = WorldTile {
                    x: p.x,
                    z: p.z,
                    level: p.level,
                };
                let pair = gate_pair_tile(at, angle_dir, outer);
                placed
                    .get(&(pair.x, pair.z, pair.level))
                    .is_some_and(|ids| {
                        ids.iter().any(|lid| {
                            !category_conflicted.contains(lid)
                                && categories.get(lid).and_then(|c| closed_gate_category(c))
                                    == Some(!outer)
                        })
                    })
            })
        });
        if !paired {
            bump(skipped, SKIP_GATE_MEMBER_PAIR, 1);
            continue;
        }
        out.insert(id, open);
    }
    out
}
