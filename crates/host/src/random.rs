//! Random-event detection over one `GameSnapshot` (guardian spec
//! `2026-09-01-random-event-guardian-design.md`, the Detect / Ours / Kinds
//! locks). Detect is snapshot-only and stateless apart from the
//! caller-owned [`CooldownMap`]; act / hold / dialog, the wrong-talk
//! 45 s cooldown writes, the trapped-kind hold and the rising-edge
//! `on_random` knock live in [`Guardian`].

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

/// The chrome contract both views bind (guardian spec `RandomStatus`):
/// published every tick on the slot status row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RandomStatus {
    /// The detected event; `None` when the snapshot shows nothing.
    pub kind: Option<RandomKind>,
    pub name: Option<String>,
    pub ours: bool,
    /// A dialog handle is in flight (the host is talking it through).
    pub handling: bool,
    /// The slot must skip script tick / follow (host-play enforces the
    /// freeze). A dialog handle in flight or a trapped kind holds, while
    /// the claim is Host and the toggle is on.
    pub hold: bool,
    pub toggle: bool,
    pub claim: RandomClaim,
    /// The shown NPC slot is in the 45 s wrong-talk bin.
    pub cooldown: bool,
}

impl Default for RandomStatus {
    fn default() -> Self {
        Self {
            kind: None,
            name: None,
            ours: false,
            handling: false,
            hold: false,
            toggle: false,
            claim: RandomClaim::Host,
            cooldown: false,
        }
    }
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

/// One explicit ownership-probe machine. Identity-scoped rejected actors are
/// retained separately so they cannot shadow a different probe candidate.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum PlantProbe {
    #[default]
    Idle,
    AwaitingResponse {
        actor: PlantActor,
        deadline_ms: u64,
    },
    Authenticated {
        actor: PlantActor,
        retry_at_ms: u64,
        deadline_ms: u64,
        continues: u32,
    },
}

impl PlantProbe {
    fn actor(&self) -> Option<&PlantActor> {
        match self {
            Self::Idle => None,
            Self::AwaitingResponse { actor, .. } | Self::Authenticated { actor, .. } => Some(actor),
        }
    }
}

/// Per-slot random-event guardian state.
pub struct Guardian {
    /// A Talk-to went out and the dialog may still be open.
    pub in_flight: bool,
    /// The snapshot tick the last act ran on (one scan per game tick).
    pub last_tick: u64,
    /// NPC slot → wrong-talk cooldown expiry `now_ms`.
    pub cooldown: CooldownMap,
    /// Signature (kind+name) of the last detected event: the rising-edge
    /// key the `on_random` knock fires on once per new event.
    pub sig: Option<String>,
    /// Who handles the current event. Filled by the rising-edge knock
    /// (host-play's script); `Host` when no event, no script, or the
    /// script did not claim it.
    pub claim: RandomClaim,
    /// The NPC slot the in-flight dialog targets (the cooldown key).
    in_flight_index: Option<usize>,
    /// Continues sent for the in-flight dialog ([`MAX_CONTINUES`] cap).
    continues: u32,
    /// Newest chat sequence already scanned for wrong-talk lines, so a
    /// stale rejection line cannot re-bin a later NPC.
    chat_seen: i32,
    /// A non-dialog act is in flight: an op or walk was sent for the
    /// current event and it has not resolved (evade/plant/hazard/
    /// lamp-rub/lost-gear/lost-tool, plus the out-of-range walk before a
    /// Talk-to / Pick). Holds the slot like the dialog handle.
    acting: bool,
    /// The kind the in-flight non-dialog act belongs to.
    acting_kind: RandomKind,
    /// Evade: the tile the flee started from — the walk-back target once
    /// the threat despawns (rs2b0t).
    flee_from: Option<(i32, i32)>,
    /// Lost-tool: the handle was worn, so the reattached tool is re-wielded.
    tool_was_worn: bool,
    /// Lost-tool: the handle's base tool name (the handle name minus the
    /// "handle" suffix), the re-wield target after reattach.
    tool_handle_base: Option<String>,
    /// Mime: the last emote anim seq the mime NPC showed.
    mime_last_seen: Option<i32>,
    /// Mime: the emote button went out for the open chat — no repeat
    /// press until the chat closes.
    mime_answered: bool,
    /// Box: the held-box count when the answer went out; the solver
    /// waits for it to drop (the answer consumed a box) before handling
    /// the next held box (rs2b0t waits on the count drop the same way).
    box_answer_count: Option<i32>,
    /// Lamp: Rub went out — do not Rub again until the skill IF closes.
    lamp_rubbed: bool,
    /// Lamp: the skill button went out — the next tick on the IF presses
    /// confirm (2831).
    lamp_skill_sent: bool,
    /// Lamp: Confirm went out — the redemption is in flight (the server
    /// closes the IF, consumes the lamp, advances the skill and opens the
    /// award dialogue).
    lamp_confirmed: bool,
    /// Lamp: the configured skill's (xp, base) as the Confirm tick's
    /// snapshot read them — the reward is witnessed by the advance past
    /// this baseline, never by a later stale read.
    lamp_reward: Option<(i32, i32)>,
    /// Lamp: award-dialogue continues sent since Confirm
    /// ([`MAX_LAMP_DIALOGUE`] cap).
    lamp_dialog: u32,
    /// Lamp: ticks the current phase has waited without progress
    /// ([`MAX_LAMP_WAIT`] cap).
    lamp_wait: u32,
    /// Lamp: the redemption failed with the lamp still held — no act and
    /// no hold (the status row still detects it) until the lamp leaves the
    /// pack or `lamp_auto` is switched off.
    lamp_stalled: bool,
    /// Box: Open went out — do not Open again until the cube IF closes
    /// and the answer is consumed.
    box_opened: bool,
    /// Maze: the active solve state, None while trapped without a route.
    maze: Option<maze::MazeSolve>,
    /// Strange Plant's bounded server-authenticated ownership probe.
    plant: PlantProbe,
    /// Exact foreign/refused/timed-out identities still present in the scene.
    plant_ignored: Vec<PlantActor>,
    /// Per-slot fishing-gear held/lost history used to prove `LostGear`.
    gear_loss: GearLoss,
}

impl Default for Guardian {
    fn default() -> Self {
        Self::new()
    }
}

impl Guardian {
    pub fn new() -> Self {
        Self {
            in_flight: false,
            last_tick: 0,
            cooldown: HashMap::new(),
            sig: None,
            claim: RandomClaim::Host,
            in_flight_index: None,
            continues: 0,
            chat_seen: 0,
            acting: false,
            acting_kind: RandomKind::Dialog,
            flee_from: None,
            tool_was_worn: false,
            tool_handle_base: None,
            mime_last_seen: None,
            mime_answered: false,
            box_answer_count: None,
            lamp_rubbed: false,
            lamp_skill_sent: false,
            lamp_confirmed: false,
            lamp_reward: None,
            lamp_dialog: 0,
            lamp_wait: 0,
            lamp_stalled: false,
            box_opened: false,
            maze: None,
            plant: PlantProbe::Idle,
            plant_ignored: Vec::new(),
            gear_loss: GearLoss::new(),
        }
    }

    /// One guardian pass per caller frame: detect + publish the status
    /// every frame, but act at most once per snapshot `tick` (the
    /// PLAYER_INFO game-tick edge). Talk-to runs only for a dialog event
    /// that is ours on an un-binned slot; an open dialog then continues
    /// via `continue_dialog` / `answer_choice` (first option), max
    /// [`MAX_CONTINUES`] — the continue is keyed to the in-flight handle,
    /// not to a fresh detect, so a despawned genie cannot stall the chat.
    /// A wrong-talk chat line bins that NPC slot for 45 s. Toggle off:
    /// never act, never hold, still detect+publish. `knock` is the
    /// rising-edge `on_random` arm: once per detected event (kind+name
    /// signature), when the caller supplies it, the script's claim is
    /// recorded and gates act + hold (`Host` claims act and hold; a
    /// `Handle` claim lets the script run untouched).
    pub fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        now_ms: u64,
        knock: Option<&mut dyn FnMut(&DetectedRandom) -> RandomClaim>,
    ) -> RandomStatus {
        let tick = snap.tick() as u64;
        let fresh = self.last_tick != tick;
        let active = snap.ingame() && snap.scene_state() == SCENE_READY;
        if active {
            self.refresh_ignored_plants(snap);
            self.revalidate_plant(snap);
        } else {
            self.clear_plant();
            self.plant_ignored.clear();
        }
        self.gear_loss.observe(snap, now_ms);
        let mut ev = detect_ignoring_plants(
            snap,
            now_ms,
            &self.cooldown,
            &self.plant_ignored,
            Some(&self.gear_loss),
        );
        self.pin_plant_event(snap, &mut ev);

        if fresh && active {
            // Fresh chat only: a stale wrong-talk line must not re-bin a
            // later NPC the guardian talks to.
            let head = snap
                .chat_lines()
                .first()
                .map(|l| l.sequence)
                .unwrap_or(self.chat_seen);
            let wrong_talk = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| WRONG_TALK_MARKERS.iter().any(|w| l.text.contains(w)));
            let plant_growing = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| l.text.contains(PLANT_GROWING_MARKER));
            let plant_rejected = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen)
                .any(|l| l.text.contains(PLANT_REJECTION_MARKER));
            let plant_before_chat = self.plant.clone();
            if self.in_flight && wrong_talk {
                if let Some(index) = self.in_flight_index {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.clear_handle();
            } else if !matches!(self.plant, PlantProbe::Idle) && plant_rejected {
                // Plant rejection is pinned to the complete actor identity;
                // a reused slot must not inherit this bin.
                self.ignore_current_plant();
            } else if matches!(self.plant, PlantProbe::AwaitingResponse { .. }) && plant_growing {
                self.authenticate_plant(now_ms);
            } else if matches!(self.plant, PlantProbe::Authenticated { .. }) && plant_growing {
                self.note_plant_growing(now_ms);
            } else if self.acting && self.acting_kind == RandomKind::Pick && plant_rejected {
                if let Some(index) = ev.as_ref().and_then(|e| e.npc_index) {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.acting = false;
            }
            self.chat_seen = head;
            // The dialog ended: the chat is closed and the NPC is gone.
            if self.in_flight && self.dialog_done(snap) {
                self.clear_handle();
            }
            if self.plant != plant_before_chat {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
            self.pin_plant_event(snap, &mut ev);
            // Rising-edge knock: ask the running script once per detected
            // event. A vanished event resets the claim to Host (the host
            // owns whatever appears next). No knock supplied → Host.
            // An inert leftover lamp (`lamp_auto` off) does not knock —
            // it is XP in the pack, not a handler latch.
            let sig = ev.as_ref().map(|e| format!("{:?}:{}", e.kind, e.name));
            if sig != self.sig {
                self.sig = sig;
                self.claim = match (&ev, knock) {
                    (Some(ev), Some(knock))
                        if !(ev.kind == RandomKind::Lamp && !settings.lamp_auto) =>
                    {
                        knock(ev)
                    }
                    _ => RandomClaim::Host,
                };
            }
        }

        // A stalled redemption unlocks on the two operator-visible state
        // changes: the lamp left the pack, or lamp auto went off (the next
        // auto-on redemption starts clean).
        if self.lamp_stalled && (!lamp_held(snap) || !settings.lamp_auto) {
            self.clear_lamp();
        }
        // An inert lamp is detect-only: `lamp_auto` off, or a redemption
        // that already gave up. No act, no hold, no `ours` — the script
        // must not stay frozen behind a lamp the host will not redeem.
        let inert_lamp = ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp)
            && (!settings.lamp_auto || self.lamp_stalled);
        if inert_lamp {
            // Drop a previous auto-on latch the moment the operator
            // turns lamp auto off with the lamp still in inv, or the
            // redemption gave up with the lamp still held.
            self.acting = false;
        }

        if fresh
            && active
            && settings.random_events
            && self.claim == RandomClaim::Host
            && !inert_lamp
        {
            let plant_before_act = self.plant.clone();
            let ignored_before_act = self.plant_ignored.len();
            self.act(driver, snap, ev.as_ref(), settings, now_ms);
            if self.plant != plant_before_act || self.plant_ignored.len() != ignored_before_act {
                ev = detect_ignoring_plants(
                    snap,
                    now_ms,
                    &self.cooldown,
                    &self.plant_ignored,
                    Some(&self.gear_loss),
                );
            }
        }
        // `step_pick` may have timed out or seen a refused send. Reflect
        // that release in this same status frame instead of publishing a
        // stale authenticated `ours`.
        self.pin_plant_event(snap, &mut ev);
        self.last_tick = tick;

        // `act` may have stalled the redemption just above: a lamp the
        // host has given up on is inert in the same tick, so the script's
        // `EventSignal.pending` (hold OR ours) clears immediately.
        let inert_lamp = inert_lamp
            || (self.lamp_stalled && ev.as_ref().is_some_and(|e| e.kind == RandomKind::Lamp));

        let cooldown = ev
            .as_ref()
            .and_then(|e| e.npc_index)
            .is_some_and(|i| binned(i, now_ms, &self.cooldown));
        RandomStatus {
            kind: ev.as_ref().map(|e| e.kind),
            name: ev.as_ref().map(|e| e.name.clone()),
            // Inert leftover lamp still detects for the status row, but
            // must not publish ours — EventSignal.pending is hold OR ours.
            ours: !inert_lamp && ev.as_ref().map(|e| e.ours).unwrap_or(false),
            handling: self.in_flight,
            hold: settings.random_events
                && self.claim == RandomClaim::Host
                && (self.in_flight
                    || self.acting
                    || ev.as_ref().is_some_and(|e| is_trapped(e.kind))),
            toggle: settings.random_events,
            claim: self.claim,
            cooldown,
        }
    }

    fn clear_plant(&mut self) {
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    fn ignore_current_plant(&mut self) {
        if let Some(actor) = self.plant.actor().cloned() {
            self.ignore_plant(actor);
        }
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    fn ignore_plant(&mut self, actor: PlantActor) {
        if let Some(existing) = self
            .plant_ignored
            .iter_mut()
            .find(|existing| existing.slot == actor.slot)
        {
            *existing = actor;
        } else {
            self.plant_ignored.push(actor);
        }
    }

    /// Remove a bin only after its slot is observed empty. If a structurally
    /// different pickable actor appears in that slot without an observed gap,
    /// replace the bin so the ambiguous replacement inherits no auth.
    fn refresh_ignored_plants(&mut self, snap: &GameSnapshot) {
        self.plant_ignored.retain_mut(|ignored| {
            let Some(current) =
                npc_by_index(snap.npcs(), ignored.slot).and_then(PlantActor::from_npc)
            else {
                return false;
            };
            *ignored = current;
            true
        });
    }

    fn authenticate_plant(&mut self, now_ms: u64) {
        let PlantProbe::AwaitingResponse { actor, .. } = &self.plant else {
            return;
        };
        self.plant = PlantProbe::Authenticated {
            actor: actor.clone(),
            retry_at_ms: now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS),
            deadline_ms: now_ms.saturating_add(PLANT_AUTH_TIMEOUT_MS),
            continues: 0,
        };
    }

    fn note_plant_growing(&mut self, now_ms: u64) {
        if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
        }
    }

    /// Drop authentication as soon as any structural actor field changes.
    /// If a different pickable actor reused the slot, bind an ignore to the
    /// replacement so stale state can never turn into a retarget.
    fn revalidate_plant(&mut self, snap: &GameSnapshot) {
        let Some(expected) = self.plant.actor().cloned() else {
            return;
        };
        let current = npc_by_index(snap.npcs(), expected.slot).and_then(PlantActor::from_npc);
        if current.as_ref() == Some(&expected) {
            return;
        }

        if let Some(replacement) = current {
            self.ignore_plant(replacement);
        }
        self.plant = PlantProbe::Idle;
        if self.acting_kind == RandomKind::Pick {
            self.acting = false;
        }
    }

    /// Keep an in-flight probe pinned to its actor even after the player
    /// moves away and publish server authentication as `ours`. Established
    /// non-plant random kinds still preempt it.
    fn pin_plant_event(&mut self, snap: &GameSnapshot, ev: &mut Option<DetectedRandom>) {
        match self.plant.clone() {
            PlantProbe::AwaitingResponse { actor, .. }
            | PlantProbe::Authenticated { actor, .. } => {
                if ev.as_ref().is_some_and(|e| e.kind != RandomKind::Pick) {
                    return;
                }
                let Some(npc) = npc_by_index(snap.npcs(), actor.slot) else {
                    *ev = None;
                    return;
                };
                let display_name = snap
                    .local_player()
                    .and_then(|lp| lp.player.actor.name.clone());
                let authenticated = matches!(self.plant, PlantProbe::Authenticated { .. });
                *ev = Some(DetectedRandom {
                    kind: RandomKind::Pick,
                    name: PICK_NAME.to_string(),
                    ours: authenticated
                        || owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()),
                    npc_index: Some(actor.slot),
                });
            }
            PlantProbe::Idle => {}
        }
    }

    /// One send per game tick. The in-flight dialog continues on its own
    /// handle; everything else goes through the solver machine below.
    /// Refuses silently when the wire layer says no (not ingame, stale
    /// target, chat closed), so the machine is driven by the snapshot,
    /// not by error paths.
    fn act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        now_ms: u64,
    ) {
        // The in-flight dialog continues on its own handle, independent
        // of a fresh detect: the genie can despawn while the chat is
        // still open, and detect would then return None.
        if self.in_flight {
            if self.continues >= MAX_CONTINUES {
                self.clear_handle();
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            let result = if snap.chat_options().is_empty() {
                ix.continue_dialog()
            } else {
                ix.answer_choice(1)
            };
            match result {
                SendResult::Sent { .. } => {
                    self.continues += 1;
                }
                // Spec: stop when continue refuses. Keep one tick after
                // Talk-to for chat to open; clear once chat is still
                // closed and a continue/answer has refused.
                SendResult::Refused { .. } => {
                    if !chat_is_open(snap) {
                        if self.continues == 0 {
                            self.continues = 1;
                        } else {
                            self.clear_handle();
                        }
                    }
                }
            }
            return;
        }
        self.act_solver(driver, snap, ev, settings, now_ms);
    }

    /// The non-dialog act machine: drive the in-flight act to completion,
    /// or start one on a fresh event (the trapped kinds and the dialog
    /// handle never get here). `acting` latches the first send; the
    /// resolution rules are per kind — the walk-to-range kinds resolve
    /// when the event's signature changes (the NPC/plant/gear/tool left
    /// the scene), hazard resolves when the loc is no longer underfoot,
    /// lamp resolves when the lamp leaves the inventory.
    fn act_solver<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        settings: &ProfileSettings,
        now_ms: u64,
    ) {
        // A confirmed lamp redemption outlives its own detect: the server
        // consumes the lamp and opens the award dialogue in the same step,
        // so the drain must run while `detect` already shows nothing (or a
        // newly spawned event). It takes the slot, like the dialog handle,
        // until it completes or gives up.
        if self.acting && self.acting_kind == RandomKind::Lamp && self.lamp_confirmed {
            self.step_lamp(driver, snap, settings);
            return;
        }
        let Some(ev) = ev else {
            if self.acting {
                self.resolve(driver, snap);
            }
            return;
        };
        if !ev.ours && ev.kind != RandomKind::Pick {
            if self.acting {
                self.resolve(driver, snap);
            }
            return;
        }
        if self.acting {
            if ev.kind != self.acting_kind {
                self.resolve(driver, snap);
            } else {
                self.step_act(driver, snap, settings, ev, now_ms);
            }
            return;
        }
        self.acting = true;
        self.acting_kind = ev.kind;
        match ev.kind {
            RandomKind::Evade => {
                // The pre-flee tile: the walk-back target after despawn.
                self.flee_from = snap.tile().map(|(x, z, _)| (x, z));
            }
            RandomKind::LostTool => {
                // Remember whether the handle was worn so the reattached
                // tool is re-wielded.
                self.tool_was_worn = snap
                    .equipment()
                    .iter()
                    .any(|i| is_tool_handle(i.def.name.as_deref()));
                self.tool_handle_base = snap
                    .equipment()
                    .iter()
                    .chain(snap.inventory().iter())
                    .find(|i| is_tool_handle(i.def.name.as_deref()))
                    .and_then(|i| i.def.name.as_deref())
                    .map(|n| {
                        n.trim()
                            .trim_end_matches("handle")
                            .trim_end_matches("Handle")
                            .trim()
                            .to_lowercase()
                    });
            }
            _ => {}
        }
        self.step_act(driver, snap, settings, ev, now_ms);
    }

    /// One step of the in-flight (or freshly started) non-dialog act,
    /// per kind.
    fn step_act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        match self.acting_kind {
            RandomKind::Dialog => self.step_dialog(driver, snap, ev, now_ms),
            RandomKind::Pick => self.step_pick(driver, snap, ev, now_ms),
            RandomKind::Evade => self.step_evade(driver, snap, ev),
            RandomKind::Hazard => self.step_hazard(driver, snap),
            RandomKind::Lamp => self.step_lamp(driver, snap, settings),
            RandomKind::LostGear => self.step_lost_gear(driver, snap, ev),
            RandomKind::LostTool => self.step_lost_tool(driver, snap),
            RandomKind::Mime => self.step_mime(driver, snap),
            RandomKind::Box => self.step_box(driver, snap),
            RandomKind::Maze => self.step_maze(driver, snap),
        }
    }

    /// The event resolved (gone, changed kind, or no longer ours): clear
    /// the in-flight latch and finish the kind's tail work — the evade
    /// walk-back toward the pre-flee tile, and the lost-tool re-wield of
    /// a handle that was worn.
    fn resolve<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        if self.acting_kind == RandomKind::Evade {
            if let Some((fx, fz)) = self.flee_from.take() {
                walk(driver, fx, fz);
            }
        }
        // Re-wield a formerly worn handle after the reattach (best
        // effort): the combined tool carries the handle's base name.
        if self.acting_kind == RandomKind::LostTool && self.tool_was_worn {
            if let Some(base) = self.tool_handle_base.clone() {
                if let Some(tool) = snap.inventory().iter().find(|i| {
                    i.def
                        .name
                        .as_deref()
                        .is_some_and(|n| n.trim().to_lowercase() == base)
                }) {
                    let mut ix = Interactions::new(snap, driver);
                    let _ = ix.wear(tool.def.id);
                }
            }
        }
        self.acting = false;
        self.flee_from = None;
        self.tool_was_worn = false;
        self.tool_handle_base = None;
        self.mime_last_seen = None;
        self.mime_answered = false;
        self.box_answer_count = None;
        self.clear_lamp();
        self.box_opened = false;
        self.maze = None;
        self.plant = PlantProbe::Idle;
    }

    /// Talk-to, gated on range: an NPC further than Chebyshev 1 gets a
    /// ground walk to its tile first (the same `try_move` the nav bot
    /// uses); the Talk-to fires once the walk closes in.
    fn step_dialog<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
        // A slot binned this pass (the wrong-talk above) or earlier is
        // not re-engaged; detect skips binned slots, this guards the
        // same-tick bin.
        if binned(index, now_ms, &self.cooldown) {
            self.acting = false;
            return;
        }
        // `ev.npc_index` is the client NPC slot (`NpcView.index`), not
        // the dense view-vec position, so the lookup must scan by slot.
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.acting = false;
            return;
        };
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        if cheb((px, pz), (npc.tile.x, npc.tile.z)) > 1 {
            walk(driver, npc.tile.x, npc.tile.z);
            return;
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".to_string())) {
            SendResult::Sent { .. } => {
                self.in_flight = true;
                self.in_flight_index = Some(index);
                self.continues = 0;
                self.acting = false;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// Drive either the established hard-owner behavior or the bounded
    /// adjacent server probe. No featureless actor is walked to; only the
    /// exact actor authenticated by the growing response may later be chased.
    fn step_pick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
        now_ms: u64,
    ) {
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.acting = false;
            return;
        };
        let display_name = snap
            .local_player()
            .and_then(|lp| lp.player.actor.name.clone());
        let Some(actor) = PlantActor::from_npc(npc) else {
            self.clear_plant();
            return;
        };
        let distance = cheb((px, pz), (npc.tile.x, npc.tile.z));

        // Preserve the existing hard evidence path, including its allowed
        // walk to range. It does not need or inherit probe authentication.
        if owned_pickable_plant(npc, snap.self_slot(), display_name.as_deref()) {
            self.plant = PlantProbe::Idle;
            self.plant_ignored.retain(|ignored| ignored != &actor);
            if distance > 1 {
                walk(driver, npc.tile.x, npc.tile.z);
                return;
            }
            let mut ix = Interactions::new(snap, driver);
            if matches!(
                ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())),
                SendResult::Refused { .. }
            ) {
                self.acting = false;
            }
            return;
        }

        if !featureless_pickable_plant(npc) {
            self.ignore_current_plant();
            return;
        }

        match self.plant.clone() {
            PlantProbe::Idle => {
                if distance > 1 {
                    self.acting = false;
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        self.plant = PlantProbe::AwaitingResponse {
                            actor,
                            deadline_ms: now_ms.saturating_add(PLANT_PROBE_TIMEOUT_MS),
                        };
                    }
                    SendResult::Refused { .. } => {
                        self.ignore_plant(actor);
                        self.plant = PlantProbe::Idle;
                        self.acting = false;
                    }
                }
            }
            PlantProbe::AwaitingResponse {
                actor: expected,
                deadline_ms,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.plant = PlantProbe::Idle;
                    self.acting = false;
                } else if now_ms >= deadline_ms {
                    self.ignore_current_plant();
                }
            }
            PlantProbe::Authenticated {
                actor: expected,
                retry_at_ms,
                deadline_ms,
                continues,
            } => {
                if actor != expected {
                    self.ignore_plant(actor);
                    self.plant = PlantProbe::Idle;
                    self.acting = false;
                    return;
                }
                if now_ms >= deadline_ms || continues >= MAX_PLANT_CONTINUES {
                    self.ignore_current_plant();
                    return;
                }
                if chat_is_open(snap) {
                    let mut ix = Interactions::new(snap, driver);
                    match ix.continue_dialog() {
                        SendResult::Sent { .. } => {
                            if let PlantProbe::Authenticated { continues, .. } = &mut self.plant {
                                *continues += 1;
                            }
                        }
                        SendResult::Refused { .. } => self.ignore_current_plant(),
                    }
                    return;
                }
                if now_ms < retry_at_ms {
                    return;
                }
                if distance > 1 {
                    if !walk(driver, npc.tile.x, npc.tile.z) {
                        self.ignore_current_plant();
                    } else if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
                        *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                    }
                    return;
                }
                let mut ix = Interactions::new(snap, driver);
                match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
                    SendResult::Sent { .. } => {
                        if let PlantProbe::Authenticated { retry_at_ms, .. } = &mut self.plant {
                            *retry_at_ms = now_ms.saturating_add(PLANT_RETRY_INTERVAL_MS);
                        }
                    }
                    SendResult::Refused { .. } => self.ignore_current_plant(),
                }
            }
        }
    }

    /// Flee the hostile guardian: walk the first walkable `fleeCandidates`
    /// ring tile (farthest from the threat first) every tick the threat
    /// stays. The walk-back after despawn lives in [`Guardian::resolve`].
    fn step_evade<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, ev: &DetectedRandom) {
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
        let Some(npc) = npc_by_index(snap.npcs(), index) else {
            self.acting = false;
            return;
        };
        for (x, z) in flee_candidates((npc.tile.x, npc.tile.z)) {
            if walk(driver, x, z) {
                break;
            }
        }
    }

    /// Step off a hazard underfoot: flee rings from the player while the
    /// hazard loc is within Chebyshev 2, then stop (the event may still
    /// linger in the loaded scene — the danger is what matters).
    fn step_hazard<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, _)) = snap.tile() else {
            return;
        };
        let near = snap.locs().iter().any(|l| {
            is_hazard_loc_name(l.name.as_deref()) && cheb((px, pz), (l.tile.x, l.tile.z)) <= 2
        });
        if !near {
            self.acting = false;
            return;
        }
        for (x, z) in flee_candidates((px, pz)) {
            if walk(driver, x, z) {
                break;
            }
        }
    }

    /// Copy the mime's performance: watch the mime NPC's anim; when the
    /// emote chat (6543) opens, press the button for the last seen
    /// emote, once per chat-open (rs2b0t `performMimeStage`). The
    /// trapped hold is the square's; the act ends when the player leaves
    /// the stage.
    fn step_mime<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        if !on_mime_square(snap) {
            self.acting = false;
            return;
        }
        // Watch the mime NPC (rs2b0t watches every frame).
        for npc in snap.npcs() {
            if npc
                .name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case("mime"))
                && mime_answer(npc.animation).is_some()
            {
                self.mime_last_seen = Some(npc.animation);
            }
        }
        // Emote chat up: answer with the last seen emote, then wait for
        // the chat to close (a still-open chat must not re-press).
        if snap.modals().chat == MIME_IF_ROOT {
            if !self.mime_answered {
                if let Some(answer) = self.mime_last_seen.and_then(mime_answer) {
                    press(driver, MIME_IF_BUTTONS[answer]);
                    self.mime_answered = true;
                }
            }
            return;
        }
        self.mime_answered = false;
    }

    /// Solve a held Strange box (rs2b0t `solveAllBoxes`): Open the box,
    /// read the cube question + three obj models, press the matching
    /// answer button, wait for one box to be consumed, then repeat while
    /// the inventory holds a box. Unknown question / missing model →
    /// fail closed: no click, the trapped hold stays.
    fn step_box<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        // Total held quantity (rs2b0t `Inventory.count('Strange box')`:
        // the box can sit as one row of a multi-box stack).
        let count: i32 = snap
            .inventory()
            .iter()
            .filter(|i| i.def.id == STRANGE_BOX_OBJ)
            .map(|i| i.count.max(0))
            .sum();
        if count == 0 {
            self.acting = false;
            return;
        }
        // An answer went out: wait for one box to be consumed before
        // acting again (rs2b0t waits on the count drop).
        if let Some(before) = self.box_answer_count {
            if count >= before {
                return;
            }
            self.box_answer_count = None;
            self.box_opened = false;
        }
        if snap.modals().main == CUBE_IF_ROOT {
            let ctx = ReadContext::new(snap);
            let question = ctx.component_text(CUBE_IF_QUESTION).unwrap_or("");
            let models = CUBE_IF_MODELS.map(|id| ctx.component_model_obj_id(id));
            if models.iter().any(|m| m.is_none()) {
                return;
            }
            let Some(answer) = solve_cube(question, models) else {
                return;
            };
            self.box_answer_count = Some(count);
            press(driver, CUBE_IF_BUTTONS[answer]);
            return;
        }
        if self.box_opened {
            return;
        }
        let Some(held) = snap
            .inventory()
            .iter()
            .find(|i| i.def.id == STRANGE_BOX_OBJ)
        else {
            self.acting = false;
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(held), ActionSpec::Label("Open".to_string())) {
            SendResult::Sent { .. } => {
                self.box_opened = true;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// One maze solver step per tick (rs2b0t `solveMaze`): solve the
    /// route from the observed tile, then drive the door / shrine phase
    /// machine. No route → log and keep the trapped hold (never replay a
    /// different spawn's route). The hold lifts on its own once the
    /// player is no longer on the maze square.
    fn step_maze<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let Some((px, pz, _)) = snap.tile() else {
            self.acting = false;
            return;
        };
        if (px >> 6, pz >> 6) != maze::MAZE_SQUARE {
            self.acting = false;
            return;
        }
        if self.maze.is_none() {
            let me = (px, pz);
            match maze::select_route(maze::graph(), me) {
                Some(doors) => {
                    if crate::debug_enabled() {
                        eprintln!(
                            "[host] maze: spawn ({px},{pz}) -> {} doors, first ({},{})",
                            doors.len(),
                            doors[0].0,
                            doors[0].1
                        );
                    }
                    self.maze = Some(maze::MazeSolve::new(doors));
                }
                None => {
                    if crate::debug_enabled() {
                        eprintln!(
                            "[host] maze: no route solvable from ({px},{pz}); the layout does not reach the shrine from here"
                        );
                    }
                }
            }
            return;
        }
        let keep = step_maze_phase(
            self.maze.as_mut().expect("checked above"),
            driver,
            snap,
            (px, pz),
        );
        if !keep {
            if crate::debug_enabled() {
                eprintln!("[host] maze: pass gave up; restarting the route from ({px},{pz})");
            }
            self.maze = None;
        }
    }

    /// Lamp auto-use, driven to a real redemption: Rub (held op 1,
    /// `opheld1`), wait for the skill IF (2808), press the vault
    /// `lamp_skill` button, then Confirm (2831) — after which the server
    /// runs `xplamp_confirm`: it closes the IF, consumes the lamp,
    /// `stat_advance`s the chosen skill and opens the award `mesbox`. Only
    /// the observed reward with that dialogue drained releases the hold;
    /// a refused or stalled redemption gives up instead of replaying.
    /// `lamp_auto` off keeps the 0.1.2 behavior (detect, no op, no hold).
    fn step_lamp<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        if !settings.lamp_auto {
            // Detect-only: tick() already skipped act + ours. Keep the
            // latch down if we still land here (live toggle mid-step).
            self.clear_lamp();
            self.acting = false;
            return;
        }
        let lamp_here = lamp_held(snap);
        if self.lamp_confirmed {
            // Confirm is out: the redemption is in flight.
            self.step_lamp_redeem(driver, snap, settings, lamp_here);
            return;
        }
        if !lamp_here {
            // The lamp left the pack without a Confirm of ours (another
            // actor consumed it): nothing left to drive.
            self.clear_lamp();
            self.acting = false;
            return;
        }
        if snap.modals().main == LAMP_IF_ROOT {
            self.lamp_wait = 0;
            if !self.lamp_skill_sent {
                let Some(btn) = lamp_skill_button(&settings.lamp_skill) else {
                    // An unknown `lamp_skill` cannot be clicked at all.
                    // No click (fail closed) — and no endless hold behind
                    // a lamp the host will not redeem.
                    if crate::debug_enabled() {
                        eprintln!("[host] lamp: unknown skill {:?}", settings.lamp_skill);
                    }
                    self.stall_lamp();
                    return;
                };
                press(driver, btn);
                self.lamp_skill_sent = true;
                return;
            }
            // The skill press has settled: confirm, and take the reward
            // baseline from *this* tick's snapshot.
            self.lamp_reward = stat_by_name(snap, &settings.lamp_skill).map(|s| (s.xp, s.base));
            self.lamp_dialog = 0;
            self.lamp_wait = 0;
            self.lamp_confirmed = true;
            press(driver, LAMP_IF_CONFIRM);
            return;
        }
        self.lamp_skill_sent = false;
        if self.lamp_rubbed {
            // Waiting for the skill IF to open after the Rub.
            self.lamp_wait += 1;
            if self.lamp_wait > MAX_LAMP_WAIT {
                if crate::debug_enabled() {
                    eprintln!("[host] lamp: the skill interface never opened; giving up");
                }
                self.stall_lamp();
            }
            return;
        }
        let Some(lamp) = snap.inventory().iter().find(|i| i.def.id == LAMP_OBJ) else {
            self.acting = false;
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(lamp), ActionSpec::Label("Rub".to_string())) {
            SendResult::Sent { .. } => {
                self.lamp_rubbed = true;
                self.lamp_wait = 0;
            }
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// The post-Confirm half of [`Guardian::step_lamp`]. The reward is
    /// witnessed by the chosen skill's stat advance (`stat_advance`, read
    /// against the Confirm-tick baseline) and the award dialogue drained;
    /// the lamp leaving the pack alone is not completion. Nothing here
    /// replays a confirmation or drains a dialogue the Confirm did not
    /// open.
    fn step_lamp_redeem<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        lamp_here: bool,
    ) {
        let rewarded = reward_landed(snap, &settings.lamp_skill, self.lamp_reward);
        let chat = chat_is_open(snap);
        if !lamp_here {
            if rewarded && !chat {
                // Consumed, awarded and the award dialogue drained: the
                // redemption is real — release the hold.
                self.clear_lamp();
                self.acting = false;
                return;
            }
            if chat && self.lamp_dialog < MAX_LAMP_DIALOGUE {
                let mut ix = Interactions::new(snap, driver);
                if matches!(ix.continue_dialog(), SendResult::Sent { .. }) {
                    self.lamp_dialog += 1;
                    self.lamp_wait = 0;
                    return;
                }
            }
        }
        self.lamp_wait += 1;
        if self.lamp_wait > MAX_LAMP_WAIT {
            // The lamp is still held (Confirm did not take: the IF stayed
            // open / the selection was refused) or the reward never
            // arrived. Give up without replaying Confirm; the stall latch
            // keeps a fresh Rub from starting while the lamp is held.
            if crate::debug_enabled() {
                eprintln!("[host] lamp: redemption did not complete; giving up");
            }
            self.stall_lamp();
        }
    }

    /// Drop the lamp flow state (a resolved, consumed or abandoned lamp).
    fn clear_lamp(&mut self) {
        self.lamp_rubbed = false;
        self.lamp_skill_sent = false;
        self.lamp_confirmed = false;
        self.lamp_reward = None;
        self.lamp_dialog = 0;
        self.lamp_wait = 0;
        self.lamp_stalled = false;
    }

    /// Give up on this lamp: release the hold, keep the latch that stops
    /// `tick` from acting/holding again until the lamp leaves the pack or
    /// `lamp_auto` goes off.
    fn stall_lamp(&mut self) {
        self.clear_lamp();
        self.lamp_stalled = true;
        self.acting = false;
    }

    /// Take the named lost fishing gear from the ground (Chebyshev ≤ 10,
    /// the client walks the take). A full pack drops one sacrificial
    /// item first so the Take lands.
    fn step_lost_gear<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: &DetectedRandom,
    ) {
        let gear = ev.name.as_str();
        let Some(item) = snap
            .ground_items()
            .iter()
            .find(|g| g.distance <= LOST_GEAR_RADIUS && item_named(g.def.name.as_deref(), gear))
        else {
            self.acting = false;
            return;
        };
        if pack_full(snap) {
            if let Some(junk) = sacrificial_item(snap) {
                let mut ix = Interactions::new(snap, driver);
                let _ = ix.interact(OpTarget::Item(junk), ActionSpec::Label("Drop".to_string()));
                return;
            }
        }
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(
            OpTarget::GroundItem(item),
            ActionSpec::Label("Take".to_string()),
        ) {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// Reattach the lost tool: unequip a worn handle, then use the handle
    /// on the ground (or held) head. No head on the ground → fail closed,
    /// no fake use-on. The re-wield of a formerly worn handle happens in
    /// [`Guardian::resolve`].
    fn step_lost_tool<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot) {
        let head_ground = snap
            .ground_items()
            .iter()
            .find(|g| g.distance <= LOST_GEAR_RADIUS && is_tool_head(g.def.name.as_deref()));
        let head_inv = snap
            .inventory()
            .iter()
            .find(|i| is_tool_head(i.def.name.as_deref()));
        if head_ground.is_none() && head_inv.is_none() {
            self.acting = false;
            return;
        }
        let handle_inv = snap
            .inventory()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let handle_worn = snap
            .equipment()
            .iter()
            .find(|i| is_tool_handle(i.def.name.as_deref()));
        let Some(handle) = handle_inv.or(handle_worn) else {
            self.acting = false;
            return;
        };
        // A worn handle must come off before it can be used on the head.
        if handle_worn.is_some() {
            let mut ix = Interactions::new(snap, driver);
            match ix.interact(
                OpTarget::Item(handle),
                ActionSpec::Label("Remove".to_string()),
            ) {
                SendResult::Sent { .. } => {}
                SendResult::Refused { .. } => self.acting = false,
            }
            return;
        }
        let mut ix = Interactions::new(snap, driver);
        let result = if let Some(head) = head_ground {
            ix.use_item_on(handle, OpTarget::GroundItem(head))
        } else {
            ix.use_item_on(
                handle,
                OpTarget::Item(head_inv.expect("head_ground or head_inv above")),
            )
        };
        match result {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.acting = false,
        }
    }

    /// The handle lifts when the chat is fully closed and the in-flight
    /// NPC has left the scene (the spec's "NPC gone and chat closed").
    /// Continue/answer refuse clears via [`Guardian::act`] when chat is
    /// closed (after a one-tick grace post Talk-to).
    fn dialog_done(&self, snap: &GameSnapshot) -> bool {
        let npc_here = self
            .in_flight_index
            .is_some_and(|i| snap.npcs().iter().any(|v| v.index == i));
        !chat_is_open(snap) && !npc_here
    }

    fn clear_handle(&mut self) {
        self.in_flight = false;
        self.in_flight_index = None;
        self.continues = 0;
    }
}

// ---------------------------------------------------------------------------
// Maze act machine (rs2b0t `solveMaze` loop body, tick-driven).
// ---------------------------------------------------------------------------

/// One `try_move` toward `target`; false when the walk is stuck
/// ([`maze::WALK_LIMIT`] sends without a tile change — the door is
/// walled off).
fn maze_walk_step<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    me: (i32, i32),
    target: (i32, i32),
) -> bool {
    if st.walk_from != Some(me) {
        st.walk_from = Some(me);
        st.walk_sends = 0;
    }
    st.walk_sends += 1;
    if st.walk_sends > maze::WALK_LIMIT {
        return false;
    }
    walk_nearest(driver, target.0, target.1);
    true
}

/// `oploc` Open on a route door (ids 3628–3632 at that tile).
fn maze_send_open<D: Driver>(driver: &mut D, tile: (i32, i32)) {
    let loc_id = maze::graph()
        .door_id
        .get(&tile)
        .copied()
        .unwrap_or(maze::MAZE_DOOR_IDS[0]);
    op_loc(driver, tile.0, tile.1, loc_id);
}

/// `oploc` Touch on the shrine (loc 3634) and wait to leave the square.
fn maze_send_touch<D: Driver>(st: &mut maze::MazeSolve, driver: &mut D, pass: u32) {
    op_loc(
        driver,
        maze::MAZE_SHRINE.0,
        maze::MAZE_SHRINE.1,
        maze::MAZE_SHRINE_LOC,
    );
    st.phase = maze::MazePhase::TouchWait;
    st.touch_pass = pass;
    st.wait_ticks = 0;
}

/// One maze phase step (rs2b0t `solveMaze` loop body). Returns false
/// when the pass gives up and the route restarts.
fn step_maze_phase<D: Driver>(
    st: &mut maze::MazeSolve,
    driver: &mut D,
    snap: &GameSnapshot,
    me: (i32, i32),
) -> bool {
    // A mesbox/briefing chat is drained first; while it is up nothing
    // else happens. A chat during an in-flight open is the wrong-door
    // refusal mesbox (rs2b0t clears it, then continues the route).
    if chat_is_open(snap) {
        if matches!(
            st.phase,
            maze::MazePhase::OpenDoor { .. }
                | maze::MazePhase::OpenResync { .. }
                | maze::MazePhase::OpenShrine { .. }
        ) {
            st.refused = true;
        }
        if st.continues < maze::MESBOX_LIMIT {
            st.continues += 1;
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.continue_dialog();
        }
        return true;
    }
    st.continues = 0;

    match st.phase {
        maze::MazePhase::WalkDoor => {
            let Some(door) = st.target() else {
                // A route ending at the chamber door already opened it.
                // Regenerated routes that missed it retain the fallback.
                st.phase = if st.doors.last() == Some(&maze::MAZE_SHRINE_DOOR) {
                    maze::MazePhase::Touch { pass: 0 }
                } else {
                    maze::MazePhase::ShrineDoor
                };
                st.touch_pass = 0;
                return true;
            };
            if cheb(me, door) <= 1 {
                maze_send_open(driver, door);
                st.phase = maze::MazePhase::OpenDoor { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, door) {
                // Walled off: step back through the previous door.
                if st.next == 0 || st.resyncs >= maze::MAX_RESYNCS {
                    return false;
                }
                st.resyncs += 1;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                st.phase = maze::MazePhase::Resync;
            }
            true
        }
        maze::MazePhase::OpenDoor { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.next += 1;
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Resync => {
            let Some(prev_door) = st.target() else {
                return false;
            };
            if cheb(me, prev_door) <= 1 {
                maze_send_open(driver, prev_door);
                st.phase = maze::MazePhase::OpenResync { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, prev_door) {
                // The previous door is walled off too: retry the route
                // door, which re-counts a resync (rs2b0t the same way).
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::OpenResync { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                // Back on the route: retry the walled-off door.
                st.phase = maze::MazePhase::WalkDoor;
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::ShrineDoor => {
            if cheb(me, maze::MAZE_SHRINE_DOOR) <= 1 {
                maze_send_open(driver, maze::MAZE_SHRINE_DOOR);
                st.phase = maze::MazePhase::OpenShrine { from: me };
                return true;
            }
            if !maze_walk_step(st, driver, me, maze::MAZE_SHRINE_DOOR) {
                // The chamber door is unreachable: give up this pass.
                return false;
            }
            true
        }
        maze::MazePhase::OpenShrine { from } => {
            if cheb(me, from) >= 2 || st.refused || st.wait_ticks >= maze::OPEN_WAIT {
                st.refused = false;
                st.phase = maze::MazePhase::Touch {
                    pass: st.touch_pass,
                };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
                return true;
            }
            st.wait_ticks += 1;
            true
        }
        maze::MazePhase::Touch { pass } => {
            if pass >= maze::TOUCH_LIMIT {
                // Still inside after all passes: restart the route.
                return false;
            }
            // Pass 0 near the shrine (the post-door tile): touch now.
            if pass == 0 && cheb(me, maze::MAZE_SHRINE) <= 2 {
                maze_send_touch(st, driver, pass);
                return true;
            }
            let stand = maze::TOUCH_STANDS[pass as usize % maze::TOUCH_STANDS.len()];
            let onto = pass % 2 == 0;
            let reached = if onto {
                me == stand
            } else {
                cheb(me, stand) <= 1
            };
            if reached {
                maze_send_touch(st, driver, pass);
                return true;
            }
            if !maze_walk_step(st, driver, me, stand) {
                // A walled-off stand: the next pass.
                st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                st.walk_from = None;
                st.walk_sends = 0;
                st.wait_ticks = 0;
            }
            true
        }
        maze::MazePhase::TouchWait => {
            st.wait_ticks += 1;
            if st.wait_ticks >= maze::TOUCH_WAIT {
                st.wait_ticks = 0;
                let pass = st.touch_pass;
                st.touch_pass = pass + 1;
                // rs2b0t re-opens the chamber door on odd passes.
                if pass % 2 == 1 {
                    st.phase = maze::MazePhase::ShrineDoor;
                } else {
                    st.phase = maze::MazePhase::Touch { pass: pass + 1 };
                }
            }
            true
        }
    }
}

/// Whether the NPC chat modal (continue button, choice buttons, or chat
/// root) is up — the same open check dialog_done / refuse-clear share.
fn chat_is_open(snap: &GameSnapshot) -> bool {
    snap.chat_continue_component_id() != -1
        || !snap.chat_options().is_empty()
        || snap.modals().chat != -1
}

/// Whether NPC slot `index` is in the 45 s wrong-talk bin at `now_ms`
/// (cooldown map values are expiry timestamps).
fn binned(index: usize, now_ms: u64, cooldown: &CooldownMap) -> bool {
    cooldown.get(&index).is_some_and(|until| now_ms < *until)
}

#[cfg(test)]
#[path = "random_tests.rs"]
mod tests;
