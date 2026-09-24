//! Random-event detection over one `GameSnapshot` (guardian spec
//! `2026-09-01-random-event-guardian-design.md`, the Detect / Ours / Kinds
//! locks). Detect is snapshot-only and stateless apart from the
//! caller-owned [`CooldownMap`]; act / hold / dialog, the wrong-talk
//! 45 s cooldown writes, the trapped-kind hold and the rising-edge
//! `on_random` knock live in [`Guardian`].
mod guardian;
pub use guardian::{Guardian, RandomStatus};

use std::collections::{HashMap, HashSet};

pub mod maze;

use api::interact::{
    op_loc, press, walk, walk_nearest, ActionSpec, Driver, Interactions, OpTarget, SendResult,
    SCENE_READY,
};
use api::query::npc_by_index;
use api::snapshot::{
    ActorKind, ActorTargetView, GameSnapshot, ItemView, NpcView, ReadContext, StatView,
};
use vault::ProfileSettings;

// The detect/claim contracts live in `api::random` so `script` can answer
// the `on_random` knock without depending on `host` (the same types are
// `host::RandomClaim` / `host::DetectedRandom` / `host::RandomKind` here).
pub use api::random::{DetectedRandom, RandomClaim, RandomKind};

/// NPC slot index → cooldown expiry `now_ms`: the slot is skipped while
/// `now_ms < value`. Task 4 stores `now_ms + 45_000` on a wrong-talk.
pub type CooldownMap = HashMap<usize, u64>;

/// Maze square: `x>>6 == 45 && z>>6 == 71` at level 0.
const MAZE_X: i32 = 45;
const MAZE_Z: i32 = 71;
/// Mime square: `x>>6 == 31 && z>>6 == 74` at level 0.
const MIME_X: i32 = 31;
const MIME_Z: i32 = 74;

/// Dialog act-set names, lowercase (`NpcView.name`).
const DIALOG_NAMES: &[&str] = &[
    "genie",
    "drunken dwarf",
    "mysterious old man",
    "sandwich lady",
    "frog",
];

/// Hostile evade names: the 274 `antimacro.npc` macro guardians (their
/// npc ids 391..443 match rs2b0t, verified against the Lost City pack).
/// Detect is name-based per the guardian spec.
const EVADE_NAMES: &[&str] = &[
    "river troll",
    "swarm",
    "rock golem",
    "zombie",
    "shade",
    "watchman",
    "tree spirit",
];

/// The growing plant (pick kind): acted on only with current ownership
/// evidence — see [`owned_pickable_plant`].
const PICK_NAME: &str = "strange plant";

/// `Strange box` obj id (verified against the Lost City pack).
const STRANGE_BOX_OBJ: i32 = 3062;
/// `Lamp` (the genie lamp) obj id (verified against the Lost City pack).
const LAMP_OBJ: i32 = 2528;

/// Mime emote-chat root (rs2b0t `MIME_IF.root`; Lost City
/// `macro_mime_emotes` — the 274 interface jag verifies 6543 with
/// children 6544..6553, `com_2..com_9` the emote buttons).
const MIME_IF_ROOT: i32 = 6543;
/// The eight emote buttons, answer index → child id (`com_2..com_9`).
const MIME_IF_BUTTONS: [i32; 8] = [6546, 6547, 6548, 6549, 6550, 6551, 6552, 6553];

/// Genie-lamp skill IF root (Lost City `xplamp.if`; rs2b0t `LAMP_IF.root`).
const LAMP_IF_ROOT: i32 = 2808;
/// First skill button (`attack`); `strength` is 2813, `fletching` is 2830.
const LAMP_IF_FIRST: i32 = 2812;
/// Confirm button after the skill pick.
const LAMP_IF_CONFIRM: i32 = 2831;
/// Skill names in xplamp.if component order (2812..=2830). The 274 and
/// 289 `xplamp.if` declare the nineteen `option` buttons in exactly this
/// order (`attack` → `fletching`), which is also rs2b0t's `LAMP_IF.skills`
/// map. `runecraft` is the 12th button; the client's own skill table
/// spells it `runecraft` (`Skill.names`). `slayer` has no lamp button.
const LAMP_IF_SKILLS: [&str; 19] = [
    "attack",
    "strength",
    "ranged",
    "magic",
    "defence",
    "hitpoints",
    "prayer",
    "agility",
    "herblore",
    "thieving",
    "crafting",
    "runecraft",
    "mining",
    "smithing",
    "fishing",
    "cooking",
    "firemaking",
    "woodcutting",
    "fletching",
];

/// Strange-box cube root (rs2b0t `CUBE_IF.root`; Lost City `macro_cube`,
/// the 274 interface jag verifies 6554 with three TYPE_MODEL children,
/// the question text and the answer buttons).
const CUBE_IF_ROOT: i32 = 6554;
/// The three spinning obj models, in answer-button order.
const CUBE_IF_MODELS: [i32; 3] = [6555, 6557, 6559];
/// The cube's question TYPE_TEXT child.
const CUBE_IF_QUESTION: i32 = 6561;
/// The three answer buttons (center/side/top, rs2b0t `CUBE_IF.buttons`).
const CUBE_IF_BUTTONS: [i32; 3] = [6562, 6563, 6564];

/// Ground-search radius for lost tool/gear (rs2b0t `.within(10)`).
const LOST_GEAR_RADIUS: i32 = 10;

/// Compass offsets for the flee rings, N/NE/E/SE/S/SW/W/NW (rs2b0t
/// `FLEE_DIRECTIONS`).
const FLEE_COMPASS: [(i32, i32); 8] = [
    (0, -1),
    (1, -1),
    (1, 0),
    (1, 1),
    (0, 1),
    (-1, 1),
    (-1, 0),
    (-1, -1),
];

/// Pack-full sacrificial drop names, matched as substrings (the rs2b0t
/// `COMMON_BANK_LOOT` junk list; a name that is not a 274 obj never
/// matches a held row, so the take falls through).
const SACRIFICIAL_DROP: &[&str] = &[
    "uncut",
    "sapphire",
    "emerald",
    "ruby",
    "diamond",
    "opal",
    "jade",
    "topaz",
    "strange fruit",
    "beer",
    "kebab",
];

/// Fishing gear the macro randoms can knock off (rs2b0t `FISHING_GEAR`).
/// Consumables (bait/feather) are not stolen tools and are not tracked.
const FISHING_GEAR: &[&str] = &[
    "small fishing net",
    "big fishing net",
    "fishing rod",
    "oily fishing rod",
    "fly fishing rod",
    "harpoon",
    "lobster pot",
];

/// How long a proved fishing-gear loss stays recoverable (rs2b0t 90 s).
const GEAR_LOSS_WINDOW_MS: u64 = 90_000;

/// Macro whirlpool fishing-spot type ids (rs2b0t `WHIRLPOOL_NPC_IDS`).
const WHIRLPOOL_NPC_IDS: &[usize] = &[403, 404, 405, 406];

/// Detect the first random event the snapshot shows, in rs2b0t
/// `detectRaw` order: maze/mime by map square, then scene NPCs/locs,
/// then inv-held box/lamp, then lost-gear / lost-tool. Returns `None`
/// when nothing applies. NPC kinds are owner-gated (except `pick`, which
/// the TUI may show as not ours). Stateless `detect` cannot prove gear
/// ownership, so unowned ground fishing gear fails closed. Box/lamp must
/// beat lost-gear so a trapped hold is not hidden by ground fishing gear.
pub fn detect(snap: &GameSnapshot, now_ms: u64, cooldown: &CooldownMap) -> Option<DetectedRandom> {
    detect_ignoring_plants(snap, now_ms, cooldown, &[], None)
}

fn detect_ignoring_plants(
    snap: &GameSnapshot,
    now_ms: u64,
    cooldown: &CooldownMap,
    ignored_plants: &[PlantActor],
    gear_loss: Option<&GearLoss>,
) -> Option<DetectedRandom> {
    if let Some((x, z, level)) = snap.tile() {
        if level == 0 {
            if x >> 6 == MIME_X && z >> 6 == MIME_Z {
                return Some(no_npc_event(RandomKind::Mime, "mime"));
            }
            if x >> 6 == MAZE_X && z >> 6 == MAZE_Z {
                return Some(no_npc_event(RandomKind::Maze, "maze"));
            }
        }
    }
    if let Some(ev) = detect_scene(snap, now_ms, cooldown) {
        return Some(ev);
    }
    if snap.inv().iter().any(|(id, _)| *id == STRANGE_BOX_OBJ) {
        return Some(no_npc_event(RandomKind::Box, "strange box"));
    }
    if snap.inv().iter().any(|(id, _)| *id == LAMP_OBJ) {
        return Some(no_npc_event(RandomKind::Lamp, "lamp"));
    }
    if let Some(gear) = lost_gear(snap, now_ms, gear_loss) {
        return Some(no_npc_event(RandomKind::LostGear, &gear));
    }
    if has_lost_tool(snap) {
        return Some(no_npc_event(RandomKind::LostTool, "lost tool"));
    }
    detect_adjacent_featureless_plant(snap, now_ms, cooldown, ignored_plants)
}

/// A map-square or inventory-held event: ours by position/possession.
fn no_npc_event(kind: RandomKind, name: &str) -> DetectedRandom {
    DetectedRandom {
        kind,
        name: name.to_string(),
        ours: true,
        npc_index: None,
    }
}

/// Scene NPCs and locs, in rs2b0t `detectSceneEvents` order: dialog /
/// pick / evade over the NPC pass, then hazard locs. Cooled NPC slots
/// (the 45 s wrong-talk bin) are skipped.
fn detect_scene(
    snap: &GameSnapshot,
    now_ms: u64,
    cooldown: &CooldownMap,
) -> Option<DetectedRandom> {
    let self_slot = snap.self_slot();
    let display_name = snap
        .local_player()
        .and_then(|lp| lp.player.actor.name.clone());
    for npc in snap.npcs() {
        if cooldown
            .get(&npc.index)
            .is_some_and(|until| now_ms < *until)
        {
            continue;
        }
        let Some(raw) = npc.name.as_deref() else {
            continue;
        };
        let name = raw.to_lowercase();
        let ours = is_ours(npc, self_slot, display_name.as_deref());
        // FACEENTITY / playerfollow is the ours tell for the dialog five
        // (including drunken dwarf), not combat. Hostile names only here,
        // and only once a positive active type-1 hit has landed (frozen
        // takingDamage gate — ownership alone is not enough).
        if ours && EVADE_NAMES.contains(&name.as_str()) && snap.taking_damage() {
            return Some(DetectedRandom {
                kind: RandomKind::Evade,
                name,
                ours: true,
                npc_index: Some(npc.index),
            });
        }
        if ours && DIALOG_NAMES.contains(&name.as_str()) {
            return Some(DetectedRandom {
                kind: RandomKind::Dialog,
                name,
                ours: true,
                npc_index: Some(npc.index),
            });
        }
        // Growing plants give no client tell of their own, so the host
        // acts on one only with current ownership evidence (`is_ours`)
        // plus a pickable op. A foreign plant — and the aggressive type,
        // which offers only `Attack` — is not our event: no Pick, no
        // walk, no hold, and it must not shadow the box/lamp candidates
        // that follow this scene pass.
        if name == PICK_NAME && owned_pickable_plant(npc, self_slot, display_name.as_deref()) {
            return Some(DetectedRandom {
                kind: RandomKind::Pick,
                name,
                ours: true,
                npc_index: Some(npc.index),
            });
        }
    }
    for loc in snap.locs() {
        let Some(raw) = loc.name.as_deref() else {
            continue;
        };
        let name = raw.to_lowercase();
        // Hazard names first, per the spec's pack note: rs2b0t's
        // gas-chest / smoking-rock ids collide with unrelated 274 locs
        // ("Chest", "Rocks"), so this rev matches names only. Whirlpool
        // locs exist in the 274 pack; the other two are no-ops here.
        if name == "whirlpool" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "whirlpool".to_string(),
                ours: true,
                npc_index: None,
            });
        }
        if name == "gas chest" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "poisonous gas".to_string(),
                ours: true,
                npc_index: None,
            });
        }
        if name == "smoking rock" {
            return Some(DetectedRandom {
                kind: RandomKind::Hazard,
                name: "smoking rock".to_string(),
                ours: true,
                npc_index: None,
            });
        }
        // No `strange plant` loc branch: both supported packs define the
        // plant as an NPC only (`antimacro.npc`), and a loc carries no
        // ownership tell at all, so a loc-shaped plant can never satisfy
        // the pick evidence rule above.
    }
    None
}

/// The spec's hard owner: the NPC faces the local player, or its
/// overhead text contains the local display name. No distance grab.
fn is_ours(npc: &NpcView, self_slot: i32, display_name: Option<&str>) -> bool {
    if npc.target
        == Some(ActorTargetView {
            kind: ActorKind::Player,
            index: self_slot.max(0) as usize,
        })
    {
        return true;
    }
    match (display_name, npc.overhead_text.as_deref()) {
        (Some(name), Some(text)) => text.contains(name),
        _ => false,
    }
}

/// Whether this actor is a *currently* ours, pickable strange plant: the
/// name, the ownership evidence and the offered op are all re-read at the
/// call site, so a reused actor slot, a lost ownership tell or the
/// aggressive type (`antimacro.npc` `macro_triffidseed_angry` offers only
/// `Attack`) stops the pick instead of chasing another player's plant.
/// Aggression, proximity, the name alone and a cached slot index are all
/// insufficient on their own.
fn owned_pickable_plant(npc: &NpcView, self_slot: i32, display_name: Option<&str>) -> bool {
    npc.name
        .as_deref()
        .is_some_and(|n| n.eq_ignore_ascii_case(PICK_NAME))
        && is_ours(npc, self_slot, display_name)
        && plant_offers_pick(npc)
}

/// Whether the actor's cached ops offer the plant's `Pick` (the pick arm
/// of rs2b0t `plantStrategy`). The growing seed offers `Pick`/`Take`; the
/// aggressive type offers `Attack` only.
fn plant_offers_pick(npc: &NpcView) -> bool {
    npc.actions.iter().any(|action| {
        action.as_deref().is_some_and(|label| {
            let label = label.trim();
            label.eq_ignore_ascii_case("pick") || label.eq_ignore_ascii_case("take")
        })
    })
}

/// Last-priority ownership probe candidate. A server-authenticated probe is
/// safe only for a passive, featureless plant already within interaction
/// range; all established random candidates (especially held box/lamp) win.
fn detect_adjacent_featureless_plant(
    snap: &GameSnapshot,
    now_ms: u64,
    cooldown: &CooldownMap,
    ignored_plants: &[PlantActor],
) -> Option<DetectedRandom> {
    let (px, pz, _) = snap.tile()?;
    snap.npcs()
        .iter()
        .find(|npc| {
            !binned(npc.index, now_ms, cooldown)
                && featureless_pickable_plant(npc)
                && PlantActor::from_npc(npc).is_some_and(|actor| !ignored_plants.contains(&actor))
                && cheb((px, pz), (npc.tile.x, npc.tile.z)) <= 1
        })
        .map(|npc| DetectedRandom {
            kind: RandomKind::Pick,
            name: PICK_NAME.to_string(),
            ours: false,
            npc_index: Some(npc.index),
        })
}

fn featureless_pickable_plant(npc: &NpcView) -> bool {
    PlantActor::from_npc(npc).is_some()
        && npc.target.is_none()
        && npc.overhead_text.as_deref().is_none()
}

/// The random's axe/pickaxe handle sits in the inventory, worn, or on
/// the ground near us (rs2b0t `handleLocation` vs inv/ground).
fn has_lost_tool(snap: &GameSnapshot) -> bool {
    snap.inventory()
        .iter()
        .any(|i| is_tool_handle(i.def.name.as_deref()))
        || snap
            .equipment()
            .iter()
            .any(|i| is_tool_handle(i.def.name.as_deref()))
        || snap
            .ground_items()
            .iter()
            .any(|g| g.distance <= LOST_GEAR_RADIUS && is_tool_handle(g.def.name.as_deref()))
}

/// A tool handle name (`*axe handle` / `*pickaxe handle`).
fn is_tool_handle(name: Option<&str>) -> bool {
    name.is_some_and(|n| {
        let l = n.to_lowercase();
        l.contains("axe handle") || l.contains("pickaxe handle")
    })
}

/// A tool head name (`*axe head` / `*pickaxe head`): the ground half the
/// lost-tool random knocks off, reattached with the handle.
fn is_tool_head(name: Option<&str>) -> bool {
    name.is_some_and(|n| {
        let l = n.to_lowercase();
        (l.contains("axe") || l.contains("pickaxe")) && l.contains("head")
    })
}

/// A hazard loc name (the 0.1.2 detect list).
fn is_hazard_loc_name(name: Option<&str>) -> bool {
    name.is_some_and(|n| {
        matches!(
            n.trim().to_lowercase().as_str(),
            "whirlpool" | "gas chest" | "smoking rock"
        )
    })
}

/// Chebyshev distance between two world tiles.
fn cheb(a: (i32, i32), b: (i32, i32)) -> i32 {
    (a.0 - b.0).abs().max((a.1 - b.1).abs())
}

/// rs2b0t `fleeCandidates`: compass rings around `from` at Chebyshev
/// 12 → 4 stepping 2, farthest ring first (the first walkable candidate
/// is the flee target).
fn flee_candidates(from: (i32, i32)) -> Vec<(i32, i32)> {
    let mut tiles = Vec::with_capacity(40);
    for dist in [12, 10, 8, 6, 4] {
        for (dx, dz) in FLEE_COMPASS {
            tiles.push((from.0 + dx * dist, from.1 + dz * dist));
        }
    }
    tiles
}

/// xplamp.if button id for the vault `lamp_skill` (default `"strength"`
/// → 2813). Unknown skills → `None` (fail-closed: no click).
fn lamp_skill_button(skill: &str) -> Option<i32> {
    let want = skill.trim().to_lowercase();
    LAMP_IF_SKILLS
        .iter()
        .position(|s| *s == want.as_str())
        .map(|i| LAMP_IF_FIRST + i as i32)
}

/// Whether the pack holds the genie lamp (obj 2528).
fn lamp_held(snap: &GameSnapshot) -> bool {
    snap.inventory().iter().any(|i| i.def.id == LAMP_OBJ)
}

/// The client's stat slot named `skill` (the client's own skill names,
/// the same strings the xplamp buttons carry).
fn stat_by_name<'a>(snap: &'a GameSnapshot, skill: &str) -> Option<&'a StatView> {
    let want = skill.trim();
    snap.stats()
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(want))
}

/// Whether the redeemed skill's reward landed: the stat advanced past the
/// baseline captured when Confirm went out (the `xplamp_confirm`
/// `stat_advance`). The confirm packet, a closed interface and a consumed
/// lamp are all not the reward.
fn reward_landed(snap: &GameSnapshot, skill: &str, baseline: Option<(i32, i32)>) -> bool {
    let Some((xp, base)) = baseline else {
        return false;
    };
    stat_by_name(snap, skill).is_some_and(|s| s.xp > xp || s.base > base)
}

/// Mime anim seq → answer index (rs2b0t `MIME_EMOTE_BY_SEQ`; the Lost
/// City `macro_event_mime` `case` order: cry, think, laugh, dance,
/// climb-rope, lean, glass-wall, glass-box). Unknown → `None`.
fn mime_answer(seq: i32) -> Option<usize> {
    match seq {
        860 => Some(0),  // emote_cry
        857 => Some(1),  // emote_think
        861 => Some(2),  // emote_laugh
        866 => Some(3),  // emote_dance
        1130 => Some(4), // emote_climbing_rope
        1129 => Some(5), // emote_mime_lean
        1128 => Some(6), // emote_glass_wall
        1131 => Some(7), // emote_glass_box
        _ => None,
    }
}

/// Strange-box cube part model id → (shape, colour) (rs2b0t
/// `CUBE_PARTS`, all 15 shape×colour combos).
fn cube_part(id: i32) -> Option<(&'static str, &'static str)> {
    Some(match id {
        3063 => ("triangle", "red"),
        3065 => ("triangle", "blue"),
        3067 => ("triangle", "yellow"),
        3069 => ("square", "red"),
        3071 => ("square", "blue"),
        3073 => ("square", "yellow"),
        3075 => ("circle", "red"),
        3077 => ("circle", "blue"),
        3079 => ("circle", "yellow"),
        3081 => ("star", "red"),
        3083 => ("star", "blue"),
        3085 => ("star", "yellow"),
        3087 => ("half moon", "red"),
        3089 => ("half moon", "blue"),
        3091 => ("half moon", "yellow"),
        _ => return None,
    })
}

/// Which of the three cube models answers the question (rs2b0t
/// `solveCube`): "What colour is the X?" picks the model whose shape is
/// X, "Which shape is X?" the model whose colour is X, in answer-button
/// order. An unknown model or unrecognised question → `None` (no click).
fn solve_cube(question: &str, models: [Option<i32>; 3]) -> Option<usize> {
    let parts: [Option<(&str, &str)>; 3] = models.map(|id| id.and_then(cube_part));
    if parts.iter().any(Option::is_none) {
        return None;
    }
    let q = question.trim().to_lowercase();
    if let Some(shape) = q
        .strip_prefix("what colour is the ")
        .and_then(|r| r.strip_suffix('?'))
    {
        let shape = shape.trim();
        return parts
            .iter()
            .position(|p| p.expect("checked above").0 == shape);
    }
    if let Some(colour) = q
        .strip_prefix("which shape is ")
        .and_then(|r| r.strip_suffix('?'))
    {
        let colour = colour.trim();
        return parts
            .iter()
            .position(|p| p.expect("checked above").1 == colour);
    }
    None
}

/// Whether the local player stands on the mime stage square.
fn on_mime_square(snap: &GameSnapshot) -> bool {
    snap.tile()
        .is_some_and(|(x, z, level)| level == 0 && x >> 6 == MIME_X && z >> 6 == MIME_Z)
}

/// Whether the backpack has no free slot (the pack-full gate for the
/// lost gear/tool sacrificial drop).
fn pack_full(snap: &GameSnapshot) -> bool {
    snap.inventory_size() > 0 && snap.inventory().len() as i32 >= snap.inventory_size()
}

/// One held item we may sacrifice for a full pack, else `None` (the Take
/// then tries anyway).
fn sacrificial_item(snap: &GameSnapshot) -> Option<&ItemView> {
    snap.inventory().iter().find(|i| {
        i.def.name.as_deref().is_some_and(|n| {
            let l = n.to_lowercase();
            SACRIFICIAL_DROP.iter().any(|j| l.contains(j))
        })
    })
}

/// Per-slot fishing-gear ownership and recent-loss history. Stateless
/// public detect has none of this, so unowned ground gear fails closed.
#[derive(Clone, Debug)]
struct GearLoss {
    held: HashSet<String>,
    lost: HashMap<String, u64>,
    was_suppressed: bool,
    last_fishing_tick: Option<u32>,
    can_recover: bool,
}

impl GearLoss {
    fn new() -> Self {
        Self {
            held: HashSet::new(),
            lost: HashMap::new(),
            was_suppressed: false,
            last_fishing_tick: None,
            can_recover: false,
        }
    }

    fn observe(&mut self, snap: &GameSnapshot, now_ms: u64) {
        let fishing_nearby = snap.npcs().iter().any(|npc| {
            npc.distance <= LOST_GEAR_RADIUS
                && (item_named(npc.name.as_deref(), "fishing spot")
                    || npc.r#type.is_some_and(|id| WHIRLPOOL_NPC_IDS.contains(&id)))
        });
        let suppressed = snap.bank_component_id() >= 0 || snap.shop().open;
        let tick = snap.tick();
        if fishing_nearby {
            self.last_fishing_tick = Some(tick);
        }
        self.can_recover = self
            .last_fishing_tick
            .is_some_and(|seen| tick >= seen && tick - seen <= 1)
            && !suppressed
            && !self.was_suppressed;
        let now: HashSet<String> = snap
            .inventory()
            .iter()
            .filter_map(|item| {
                let name = item.def.name.as_deref()?.to_ascii_lowercase();
                FISHING_GEAR.contains(&name.as_str()).then_some(name)
            })
            .collect();
        for gear in &now {
            self.lost.remove(gear);
        }
        if self.can_recover {
            for gear in &self.held {
                if !now.contains(gear) {
                    self.lost.insert(gear.clone(), now_ms);
                }
            }
        }
        self.held = now;
        self.was_suppressed = suppressed;
    }

    fn recently_lost(&self, gear: &str, now_ms: u64) -> bool {
        let Some(at) = self.lost.get(&gear.to_ascii_lowercase()) else {
            return false;
        };
        self.can_recover && now_ms.saturating_sub(*at) <= GEAR_LOSS_WINDOW_MS
    }
}

/// A fishing-gear item on the ground near us that this slot recently held
/// and then lost while the fishing-nearby latch was live. Without that
/// per-slot history, detection fails closed.
fn lost_gear(snap: &GameSnapshot, now_ms: u64, loss: Option<&GearLoss>) -> Option<String> {
    let loss = loss?;
    for gear in FISHING_GEAR {
        if !loss.recently_lost(gear, now_ms) {
            continue;
        }
        let in_inv = snap
            .inventory()
            .iter()
            .any(|i| item_named(i.def.name.as_deref(), gear));
        if !in_inv
            && snap
                .ground_items()
                .iter()
                .any(|g| g.distance <= LOST_GEAR_RADIUS && item_named(g.def.name.as_deref(), gear))
        {
            return Some((*gear).to_string());
        }
    }
    None
}

fn item_named(name: Option<&str>, want: &str) -> bool {
    name.is_some_and(|n| n.eq_ignore_ascii_case(want))
}

// ---------------------------------------------------------------------------
// Guardian (the act/hold-while-handling half; the trapped-kind hold and
// the `on_random` knock fire here). One instance per slot.
// ---------------------------------------------------------------------------

/// The dialog-continue ceiling (rs2b0t `MAX_DIALOGUE_STEPS`).
const MAX_CONTINUES: u32 = 25;

/// Lamp: ticks one phase of the redemption may stall before the guardian
/// gives up (the Rub→IF wait, then the Confirm→reward wait). The server
/// settles the whole flow within a tick or two.
const MAX_LAMP_WAIT: u32 = 8;

/// Lamp: award-dialogue continues before the redemption is given up
/// (`xplamp_confirm`'s `mesbox` award is a single page).
const MAX_LAMP_DIALOGUE: u32 = 4;

/// Wrong-talk cooldown for an NPC slot (rs2b0t 45 s).
const WRONG_TALK_COOLDOWN_MS: u64 = 45_000;

/// Chat markers of a failed Talk-to: the NPC is not the event's owner.
const WRONG_TALK_MARKERS: &[&str] = &["trying to talk to", "It's not here for you."];

/// Canonical server response proving that this particular plant is foreign.
const PLANT_REJECTION_MARKER: &str = "It's not here for you.";
/// Canonical server response proving that the probed plant belongs to us.
const PLANT_GROWING_MARKER: &str = "The fruit isn't ready to be picked yet";
/// A featureless plant gets one bounded probe, never a speculative retry.
const PLANT_PROBE_TIMEOUT_MS: u64 = 5_000;
/// Authenticated retries are slow enough to avoid click spam.
const PLANT_RETRY_INTERVAL_MS: u64 = 3_000;
/// Authentication is temporary and belongs only to this actor instance.
const PLANT_AUTH_TIMEOUT_MS: u64 = 90_000;
/// Defensive cap for repeated growing-plant message pages.
const MAX_PLANT_CONTINUES: u32 = 25;

/// Trapped kinds: the player is stuck and the host must freeze the slot
/// (maze / mime / strange box). **Not** `lamp`: Genie Talk-to is the
/// solve; a leftover lamp is inert XP when `lamp_auto` is off.
fn is_trapped(kind: RandomKind) -> bool {
    matches!(kind, RandomKind::Maze | RandomKind::Mime | RandomKind::Box)
}


/// Client-visible identity used to pin a probe to one NPC actor. The client
/// exposes no owner or spawn generation, so every available structural field
/// that matters to this interaction is revalidated before each step.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PlantActor {
    slot: usize,
    type_id: Option<usize>,
    name: String,
    op_slot: usize,
    op: String,
}

impl PlantActor {
    fn from_npc(npc: &NpcView) -> Option<Self> {
        let name = npc.name.as_deref()?.trim().to_lowercase();
        if name != PICK_NAME {
            return None;
        }
        let (op_slot, op) = npc.actions.iter().enumerate().find_map(|(slot, action)| {
            let op = action.as_deref()?.trim();
            (op.eq_ignore_ascii_case("pick") || op.eq_ignore_ascii_case("take"))
                .then(|| (slot, op.to_lowercase()))
        })?;
        Some(Self {
            slot: npc.index,
            type_id: npc.r#type,
            name,
            op_slot,
            op,
        })
    }
}




// ---------------------------------------------------------------------------
// Maze act machine (rs2b0t `solveMaze` loop body, tick-driven).
// ---------------------------------------------------------------------------


/// Whether NPC slot `index` is in the 45 s wrong-talk bin at `now_ms`
/// (cooldown map values are expiry timestamps).
fn binned(index: usize, now_ms: u64, cooldown: &CooldownMap) -> bool {
    cooldown.get(&index).is_some_and(|until| now_ms < *until)
}

