use super::*;

/// Door loc id → its `(varp id, min value)` gate, read from the door's own
/// `[oploc1,<name>]` open script: a `switch_int(%<varp>)` whose opening
/// cases carry the open call, or an `if (%<varp> >= ^<const> [| …])` whose
/// arm opens. The generic `_door_closed` script opens freely and carries
/// nothing.
pub(super) fn quest_door_reqs(
    content_root: &Path,
    door_names: &HashSet<String>,
    ids: &HashMap<String, i32>,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> HashMap<i32, Vec<(i32, i32)>> {
    let mut out: HashMap<i32, Vec<(i32, i32)>> = HashMap::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, block) in script_blocks(text) {
            if op != "oploc1" || !door_names.contains(&name) {
                continue;
            }
            let Some(&id) = ids.get(&name) else {
                continue;
            };
            let Some(gates) = script_varp_gate(&block, text, constants) else {
                continue;
            };
            let reqs = out.entry(id).or_default();
            for (varp, min_value) in gates {
                if let Some(&varp_id) = varps.get(&varp) {
                    reqs.push((varp_id, min_value));
                }
            }
        }
    });
    out
}

/// A door whose `[oploc1,<name>]` block proves one crossing free: the block
/// opens on a `~check_axis` side test whose other disjunct is a
/// `~<proc> >= ^<const>` bitfield gate. The proven crossing is emitted with
/// empty requirements. The gated reverse is emitted only when
/// [`completed_quest_reverse`] can prove a conservative completed-quest
/// requirement for that loc; otherwise it stays out rather than open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FreeDoorArm {
    /// The `~check_axis(coord, loc_coord, loc_angle)` value the block opens
    /// on without any gate.
    free_when_check_axis: bool,
    /// Resolved `^const` minimum on the gated disjunct.
    gated_min: i32,
    /// Proven `getbit_range` varp id, lower bit, and upper bit. Completion
    /// is attached only when this identity is `%death_map` bits 0..3.
    bitfield_varp: i32,
    bitfield_lo: i32,
    bitfield_hi: i32,
}

impl FreeDoorArm {
    /// The crossing direction the free arm lands on, given the placement
    /// angle's own direction (the client's always-reached neighbour side —
    /// `test_wall` WALL_STRAIGHT EAST → destX+1, NORTH → destZ+1,
    /// WEST → destX−1, SOUTH → destZ−1). The native hop walks the player
    /// onto the door's own tile only for the crossing that starts on the
    /// far side and travels that way (`changeWallStraight`'s `canTravel`
    /// polarity lets the player step onto the loc along the angle, not
    /// against it), and that stand is the one `~check_axis` reads `true`
    /// for; the opposite crossing executes from the always-reached
    /// neighbour and reads `false`.
    pub(super) fn free_dir(&self, angle_dir: DoorDir) -> DoorDir {
        if self.free_when_check_axis {
            angle_dir
        } else {
            opposite(angle_dir)
        }
    }
}

/// Door loc id → the directional free arm its `[oploc1,<name>]` block
/// proves ([`free_door_arm`]). A door whose blocks disagree about the arm
/// proves neither; a door whose block is any other shape stays out of this
/// map (and out of the pack entirely when no gate is readable either), so
/// an unread door is never promoted to ungated.
pub(super) fn quest_door_free_arms(
    content_root: &Path,
    door_names: &HashSet<String>,
    ids: &HashMap<String, i32>,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> HashMap<i32, FreeDoorArm> {
    let mut out: HashMap<i32, FreeDoorArm> = HashMap::new();
    let mut conflicted: HashSet<i32> = HashSet::new();
    visit_rs2(&content_root.join("scripts"), &mut |text| {
        for (op, name, block) in script_blocks(text) {
            if op != "oploc1" || !door_names.contains(&name) {
                continue;
            }
            let Some(&id) = ids.get(&name) else {
                continue;
            };
            let Some(arm) = free_door_arm(&block, text, constants, varps) else {
                continue;
            };
            match out.get(&id) {
                Some(&prev) if prev != arm => {
                    out.remove(&id);
                    conflicted.insert(id);
                }
                Some(_) => {}
                None if conflicted.contains(&id) => {}
                None => {
                    out.insert(id, arm);
                }
            }
        }
    });
    out
}

/// The free arm a door's `[oploc1,<name>]` block proves, from exactly the
/// supported top-level sequence:
///
/// ```text
/// def_boolean $<b> = ~check_axis(coord, loc_coord, loc_angle);
/// if($<b> = <true|false> | ~<proc> >= ^<const>) { …opens the door directly… }
/// ```
///
/// The def must be the first top-level statement and the `if` the second;
/// extra statements before that pair (an outer guard, an earlier return, a
/// reassignment of `$<b>`) prove nothing. `$<b>` must be the boolean that
/// statement's own `check_axis` def introduces; the opening head must have
/// exactly two disjuncts — that boolean test and one `~<proc> >= ^<const>`
/// compare — with no nesting and no `&`; `<const>` must resolve in the
/// script constants; `<proc>` must be a `[proc,…]` block in the same script
/// text whose body is exactly
/// `return (getbit_range(%<varp>, ^<lo>, ^<hi>));` with the varp in
/// `pack/varp.pack` and both range constants resolving; and the arm must
/// open directly with the canonical `~open_and_close_door2(<loc>, $<b>,
/// door_open)` statement (no nested braces, no earlier return or quoted
/// text, no unrelated `~open_` name). Every other shape — a raw `%varp`
/// compare, a different comparator, a missing or differently-shaped proc,
/// an unresolved name, an arm that opens only inside a nested gate, an
/// `else` on the opening `if` — proves nothing at all, so the door keeps
/// no free arm and is never promoted to ungated. Knock and dialogue
/// branches after the opening `if` are never read.
pub(super) fn free_door_arm(
    block: &str,
    script_text: &str,
    constants: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> Option<FreeDoorArm> {
    let stmts = top_level_statements(block);
    let axis_bool = check_axis_def(stmts.first()?)?;
    let (head, arm) = if_head_and_arm(stmts.get(1)?)?;
    let (free_when_check_axis, proc, cname) = check_axis_or_proc(&head, &axis_bool)?;
    let gated_min = *constants.get(&cname)?;
    if !arm_opens_directly(&arm, &axis_bool) {
        return None;
    }
    let (bitfield_varp, bitfield_lo, bitfield_hi) =
        proc_bitfield_varp(script_text, &proc, constants, varps)?;
    Some(FreeDoorArm {
        free_when_check_axis,
        gated_min,
        bitfield_varp,
        bitfield_lo,
        bitfield_hi,
    })
}

/// Journal name of Death Plateau as the client stores a completed quest.
pub(super) const DEATH_PLATEAU_QUEST: &str = "Death Plateau";
/// Front hut door (`death_sherpa_door`): gated min is `^death_spoken_tenzing`,
/// free when `$leaving = true`.
pub(super) const DEATH_FRONT_DOOR: &str = "death_sherpa_door";
pub(super) const DEATH_FRONT_GATED_MIN: i32 = 2;
pub(super) const DEATH_FRONT_FREE_WHEN_CHECK_AXIS: bool = true;
/// Rear hut door (`death_sherpa_backdoor`): gated min is `^death_got_map`,
/// free when `$leaving = false`.
pub(super) const DEATH_BACK_DOOR: &str = "death_sherpa_backdoor";
pub(super) const DEATH_BACK_GATED_MIN: i32 = 7;
pub(super) const DEATH_BACK_FREE_WHEN_CHECK_AXIS: bool = false;
/// `%death_map` bits `^death_map_lower..^death_map_upper` (0..3).
pub(super) const DEATH_MAP_VARP: &str = "death_map";
pub(super) const DEATH_MAP_LO: i32 = 0;
pub(super) const DEATH_MAP_HI: i32 = 3;

/// Conservative completed-quest requirement for a directional door's gated
/// reverse, from an explicit source-backed mapping: only the two Death
/// Plateau hut doors whose proven free-arm shape still carries the current
/// thresholds, polarities, and `%death_map` bits 0..3 (front entry ≥ 2
/// leaving-true, garden exit ≥ 7 leaving-false). Completion is reachable
/// only after `death_get_map >= death_scouted_area` (8), so a completed
/// journal row implies both thresholds; in-progress map stages stay
/// unsupported. Loc ids resolve from the selected content's `loc.pack`.
/// An unrelated varp, a different bit range, the wrong polarity, or a
/// loc that is not the canonical sherpa door does not borrow completion.
/// This is not a general quest-implication engine and does not read varp
/// 315 as a raw gate.
pub(super) fn completed_quest_reverse(
    id: i32,
    arm: &FreeDoorArm,
    ids: &HashMap<String, i32>,
    varps: &HashMap<String, i32>,
) -> Option<&'static str> {
    let &death_map = varps.get(DEATH_MAP_VARP)?;
    if arm.bitfield_varp != death_map
        || arm.bitfield_lo != DEATH_MAP_LO
        || arm.bitfield_hi != DEATH_MAP_HI
    {
        return None;
    }
    if ids.get(DEATH_FRONT_DOOR) == Some(&id)
        && arm.gated_min == DEATH_FRONT_GATED_MIN
        && arm.free_when_check_axis == DEATH_FRONT_FREE_WHEN_CHECK_AXIS
    {
        return Some(DEATH_PLATEAU_QUEST);
    }
    if ids.get(DEATH_BACK_DOOR) == Some(&id)
        && arm.gated_min == DEATH_BACK_GATED_MIN
        && arm.free_when_check_axis == DEATH_BACK_FREE_WHEN_CHECK_AXIS
    {
        return Some(DEATH_PLATEAU_QUEST);
    }
    None
}
