//! Random-event detection over one `GameSnapshot` (guardian spec
//! `2026-09-01-random-event-guardian-design.md`, the Detect / Ours / Kinds
//! locks). Detect is snapshot-only and stateless apart from the
//! caller-owned [`CooldownMap`]; act / hold / dialog, the wrong-talk
//! 45 s cooldown writes, the trapped-kind hold and the rising-edge
//! `on_random` knock live in [`Guardian`].

use std::collections::HashMap;

pub mod maze;

use api::interact::{
    op_loc, press, walk, ActionSpec, Driver, Interactions, OpTarget, SendResult, SCENE_READY,
};
use api::query::npc_by_index;
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, ItemView, NpcView, ReadContext};
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

/// The growing plant (pick kind): shown even when not ours yet.
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
const FISHING_GEAR: &[&str] = &[
    "small fishing net",
    "big fishing net",
    "fishing rod",
    "oily fishing rod",
    "fly fishing rod",
    "harpoon",
    "lobster pot",
    "fishing bait",
    "feather",
];

/// Detect the first random event the snapshot shows, in rs2b0t
/// `detectRaw` order: maze/mime by map square, then scene NPCs/locs,
/// then inv-held box/lamp, then lost-gear / lost-tool. Returns `None`
/// when nothing applies. NPC kinds are owner-gated (except `pick`, which
/// the TUI may show as not ours). The rs2b0t gear-loss 90 s window (the
/// caller-state half of `lost-gear`) is not needed here: gear on the
/// ground and out of the inventory detects. Box/lamp must beat lost-gear
/// so a trapped hold is not hidden by ground fishing gear.
pub fn detect(snap: &GameSnapshot, now_ms: u64, cooldown: &CooldownMap) -> Option<DetectedRandom> {
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
    if let Some(gear) = lost_gear(snap) {
        return Some(no_npc_event(RandomKind::LostGear, &gear));
    }
    if has_lost_tool(snap) {
        return Some(no_npc_event(RandomKind::LostTool, "lost tool"));
    }
    None
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
        let facing_self = npc.target
            == Some(ActorTargetView {
                kind: ActorKind::Player,
                index: self_slot.max(0) as usize,
            });
        // The drunken dwarf is dialog until it comes for us (`target ==
        // self`); then it is evade like the hostile guardians.
        if ours
            && (EVADE_NAMES.contains(&name.as_str()) || (name == "drunken dwarf" && facing_self))
        {
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
        // Growing plants give no client tell; the TUI can still name it.
        if name == PICK_NAME {
            return Some(DetectedRandom {
                kind: RandomKind::Pick,
                name,
                ours,
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

/// Whether a chat option answers the lamp's skill prompt for `want`
/// (the vault `lamp_skill`, default "strength").
fn skill_match(text: &str, want: &str) -> bool {
    text.trim().eq_ignore_ascii_case(want.trim())
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

/// A fishing-gear item on the ground near us that we are not holding
/// (rs2b0t `GearLossTracker` ground search, minus the 90 s window).
fn lost_gear(snap: &GameSnapshot) -> Option<String> {
    for gear in FISHING_GEAR {
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

/// Wrong-talk cooldown for an NPC slot (rs2b0t 45 s).
const WRONG_TALK_COOLDOWN_MS: u64 = 45_000;

/// Chat markers of a failed Talk-to: the NPC is not the event's owner.
const WRONG_TALK_MARKERS: &[&str] = &["trying to talk to", "It's not here for you."];

/// Trapped kinds: the player is stuck and the host must freeze the slot
/// (maze / mime / strange box). **Not** `lamp`: Genie Talk-to is the
/// solve, and a leftover lamp is inert XP until 0.1.5.
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
    /// Maze: the active solve state, None while trapped without a route.
    maze: Option<maze::MazeSolve>,
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
            maze: None,
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
        let ev = detect(snap, now_ms, &self.cooldown);
        let active = snap.ingame() && snap.scene_state() == SCENE_READY;

        if fresh && active {
            // Fresh chat only: a stale wrong-talk line must not re-bin a
            // later NPC the guardian talks to.
            let head = snap
                .chat_lines()
                .first()
                .map(|l| l.sequence)
                .unwrap_or(self.chat_seen);
            let mut new_lines = snap
                .chat_lines()
                .iter()
                .take_while(|l| l.sequence > self.chat_seen);
            if self.in_flight
                && new_lines.any(|l| WRONG_TALK_MARKERS.iter().any(|w| l.text.contains(w)))
            {
                if let Some(index) = self.in_flight_index {
                    self.cooldown.insert(index, now_ms + WRONG_TALK_COOLDOWN_MS);
                }
                self.clear_handle();
            }
            self.chat_seen = head;
            // The dialog ended: the chat is closed and the NPC is gone.
            if self.in_flight && self.dialog_done(snap) {
                self.clear_handle();
            }
            // Rising-edge knock: ask the running script once per detected
            // event. A vanished event resets the claim to Host (the host
            // owns whatever appears next). No knock supplied → Host.
            let sig = ev.as_ref().map(|e| format!("{:?}:{}", e.kind, e.name));
            if sig != self.sig {
                self.sig = sig;
                self.claim = match (&ev, knock) {
                    (Some(ev), Some(knock)) => knock(ev),
                    _ => RandomClaim::Host,
                };
            }
        }

        if fresh && active && settings.random_events && self.claim == RandomClaim::Host {
            self.act(driver, snap, ev.as_ref(), settings, now_ms);
        }
        self.last_tick = tick;

        let cooldown = ev
            .as_ref()
            .and_then(|e| e.npc_index)
            .is_some_and(|i| binned(i, now_ms, &self.cooldown));
        RandomStatus {
            kind: ev.as_ref().map(|e| e.kind),
            name: ev.as_ref().map(|e| e.name.clone()),
            ours: ev.as_ref().map(|e| e.ours).unwrap_or(false),
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
        let Some(ev) = ev else {
            if self.acting {
                self.resolve(driver, snap);
            }
            return;
        };
        if !ev.ours {
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
            RandomKind::Pick => self.step_pick(driver, snap, ev),
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
        self.maze = None;
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

    /// Pick the growing plant, gated on range like Talk-to.
    fn step_pick<D: Driver>(&mut self, driver: &mut D, snap: &GameSnapshot, ev: &DetectedRandom) {
        let Some(index) = ev.npc_index else {
            self.acting = false;
            return;
        };
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
        match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Pick".to_string())) {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.acting = false,
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
        }
        if snap.modals().main == CUBE_IF_ROOT {
            let ctx = ReadContext::new(snap);
            let question = ctx.component_text(CUBE_IF_QUESTION).unwrap_or("");
            let models = CUBE_IF_MODELS.map(|id| ctx.component_model_obj_id(id));
            let Some(answer) = solve_cube(question, models) else {
                self.acting = false;
                return;
            };
            self.box_answer_count = Some(count);
            press(driver, CUBE_IF_BUTTONS[answer]);
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
            SendResult::Sent { .. } => {}
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

    /// Lamp auto-use: Rub the held lamp, then answer the skill dialog
    /// with the vault `lamp_skill` button. `lamp_auto` off keeps the
    /// 0.1.2 behavior (detect, no op, no hold).
    fn step_lamp<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
    ) {
        if !settings.lamp_auto {
            self.acting = false;
            return;
        }
        let lamp_here = snap.inventory().iter().any(|i| i.def.id == LAMP_OBJ);
        if !lamp_here {
            self.acting = false;
            return;
        }
        // The skill dialog answers through its BUTTON_OK options; a
        // dialog page without the matching button continues. No dialog
        // yet → the Rub itself.
        if let Some(pos) = snap
            .chat_options()
            .iter()
            .position(|o| skill_match(&o.text, &settings.lamp_skill))
        {
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.answer_choice(pos as i32 + 1);
            return;
        }
        if chat_is_open(snap) {
            let mut ix = Interactions::new(snap, driver);
            let _ = ix.continue_dialog();
            return;
        }
        let Some(lamp) = snap.inventory().iter().find(|i| i.def.id == LAMP_OBJ) else {
            self.acting = false;
            return;
        };
        let mut ix = Interactions::new(snap, driver);
        match ix.interact(OpTarget::Item(lamp), ActionSpec::Label("Rub".to_string())) {
            SendResult::Sent { .. } => {}
            SendResult::Refused { .. } => self.acting = false,
        }
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
    walk(driver, target.0, target.1);
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
                // Route exhausted: the chamber door is next.
                st.phase = maze::MazePhase::ShrineDoor;
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
mod tests {
    use super::*;
    use api::prot::Out;
    use client::client::{Client, ClientConfig, ClientPlayer, MiniMenuAction};
    use client::config::if_type::{ButtonType, ComponentType, IfType, IfTypeMut};
    use client::config::{Cache, LocType, NpcType, ObjType};
    use client::dash3d::{ClientNpc, ClientObj};
    use client::datastruct::LinkList;
    use std::sync::Arc;

    fn cfg() -> ClientConfig {
        ClientConfig {
            host: "127.0.0.1".into(),
            port: 43594,
            cache_dir: "/tmp".into(),
            members: true,
            lowmem: true,
        }
    }

    /// An attached-free client at base (0, 0) with the local player slot 0.
    fn new_client() -> Client {
        let mut c = crate::prepare_client(
            cfg(),
            1,
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
        );
        c.map_build_base_x = 0;
        c.map_build_base_z = 0;
        c.self_slot = 0;
        c
    }

    /// Plant the local player with display name `name` on world tile
    /// `(x, z)` (base 0, level 0).
    fn plant_player(c: &mut Client, name: &str, x: i32, z: i32) {
        let mut lp = ClientPlayer::at(x, z);
        lp.entity.x = x * 128 + 64;
        lp.entity.z = z * 128 + 64;
        lp.name = Some(name.to_string());
        c.local_player = Some(lp);
    }

    /// An NPC of cache type `name` in client table slot `slot` (the
    /// `NpcView.index` / cooldown key) with the given `face_entity` and
    /// overhead text.
    fn plant_npc(
        c: &mut Client,
        slot: usize,
        name: &str,
        face_entity: i32,
        overhead: Option<&str>,
    ) {
        plant_npc_with_op(c, slot, name, face_entity, overhead, "Talk-to");
    }

    /// Like [`plant_npc`] but with a chosen first menu op (the growing
    /// plant's `Pick`).
    fn plant_npc_with_op(
        c: &mut Client,
        slot: usize,
        name: &str,
        face_entity: i32,
        overhead: Option<&str>,
        op: &str,
    ) {
        let type_id = 500 + slot;
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.npcs.len() <= type_id {
                cache.npcs.push(NpcType::default());
            }
            cache.npcs[type_id] = NpcType {
                id: type_id as i32,
                name: name.to_string(),
                op: vec![Some(op.to_string())],
                ..Default::default()
            };
        }
        let mut npc = ClientNpc::at(0, 0);
        npc.r#type = Some(type_id);
        npc.entity.face_entity = face_entity;
        npc.entity.chat_message = overhead.map(str::to_string);
        while c.npc.len() <= slot {
            c.npc.push(None);
        }
        c.npc[slot] = Some(Box::new(npc));
        // `npc_ids` is a fixed `vec![0; MAX_NPC_COUNT]`; write the slot at
        // the count index (like `handle_packet`) instead of pushing, so the
        // snapshot's view list is exactly the planted slots in order.
        c.npc_ids[c.npc_count as usize] = slot as i32;
        c.npc_count += 1;
    }

    /// A ground item of cache obj `name` on scene tile (x, z) (level 0).
    fn plant_ground_obj(c: &mut Client, x: i32, z: i32, obj_id: i32, name: Option<&str>) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= obj_id as usize {
                cache.objs.push(ObjType::default());
            }
            if let Some(name) = name {
                cache.objs[obj_id as usize] = ObjType {
                    id: obj_id,
                    name: name.to_string(),
                    ..Default::default()
                };
            }
        }
        let mut list = LinkList::new();
        list.push_front(ClientObj::new(obj_id, 1));
        c.ground_obj[0][x as usize][z as usize] = Some(Box::new(list));
    }

    /// Rebuild a fresh snapshot with the npc, player and scene families
    /// moved (scene moves the loc/ground-item views too).
    fn snap_at(c: &mut Client) -> GameSnapshot {
        c.gens.npc = 1;
        c.gens.player = 1;
        c.gens.scene = 1;
        let mut snap = GameSnapshot::new();
        snap.rebuild(c);
        snap
    }

    /// Advance every packet family and rebuild the **persistent**
    /// snapshot: like the host's drain, one call per game tick, so
    /// `snap.tick()` climbs 1, 2, … (a fresh `GameSnapshot` starts its
    /// tick at 0 again, which is why the guardian tests reuse one).
    fn tick_at(c: &mut Client, snap: &mut GameSnapshot) {
        c.gens.npc = c.gens.npc.wrapping_add(1);
        c.gens.player = c.gens.player.wrapping_add(1);
        c.gens.scene = c.gens.scene.wrapping_add(1);
        c.gens.iface = c.gens.iface.wrapping_add(1);
        c.gens.chat = c.gens.chat.wrapping_add(1);
        snap.rebuild(c);
    }

    fn no_cooldown() -> CooldownMap {
        HashMap::new()
    }

    #[test]
    fn overhead_greetings_with_local_name_is_dialog_ours() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("dialog detected");
        assert_eq!(ev.kind, RandomKind::Dialog);
        assert_eq!(ev.name, "genie");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, Some(0));
    }

    #[test]
    fn neighbour_genie_overhead_other_name_is_not_ours() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Bob!"));
        let snap = snap_at(&mut c);
        assert_eq!(detect(&snap, 0, &no_cooldown()), None);
    }

    #[test]
    fn swarm_targeting_self_is_evade_ours_untargeted_swarm_is_not() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        // `face_entity` >= 32768 decodes as Player kind; + self_slot (0).
        plant_npc(&mut c, 0, "Swarm", 32768, None);
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("evade detected");
        assert_eq!(ev.kind, RandomKind::Evade);
        assert_eq!(ev.name, "swarm");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, Some(0));

        // Same type without a target: not ours, so nothing to detect.
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Swarm", -1, None);
        let snap = snap_at(&mut c);
        assert_eq!(detect(&snap, 0, &no_cooldown()), None);
    }

    #[test]
    fn maze_square_is_maze_ours_with_no_npc() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 45 * 64, 71 * 64);
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("maze detected");
        assert_eq!(ev.kind, RandomKind::Maze);
        assert_eq!(ev.name, "maze");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }

    #[test]
    fn ground_axe_handle_is_lost_tool() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_ground_obj(&mut c, 0, 0, 500, Some("Axe handle"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("lost-tool detected");
        assert_eq!(ev.kind, RandomKind::LostTool);
        assert_eq!(ev.name, "lost tool");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }

    #[test]
    fn ground_fishing_gear_not_in_inv_is_lost_gear() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_ground_obj(&mut c, 0, 0, 501, Some("Fishing rod"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("lost-gear detected");
        assert_eq!(ev.kind, RandomKind::LostGear);
        assert_eq!(ev.name, "fishing rod");
        assert!(ev.ours);
        assert_eq!(ev.npc_index, None);
    }

    #[test]
    fn inv_box_beats_ground_fishing_net() {
        let mut c = new_client();
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_obj(&mut c, STRANGE_BOX_OBJ);
        plant_ground_obj(&mut c, 0, 0, 502, Some("Small fishing net"));
        let snap = snap_at(&mut c);
        let ev = detect(&snap, 0, &no_cooldown()).expect("box must beat lost-gear");
        assert_eq!(ev.kind, RandomKind::Box);
        assert_eq!(ev.name, "strange box");
    }

    // --- Guardian (Task 4): act + hold-while-handling, fake Driver ---

    /// No-op packet sink the recording driver hands out.
    struct NoopOut;
    impl Out for NoopOut {
        fn p1_enc(&mut self, _opcode: i32) {}
        fn p1(&mut self, _value: i32) {}
        fn p2(&mut self, _value: i32) {}
        fn p4(&mut self, _value: i32) {}
        fn pjstr(&mut self, _s: &str) {}
    }

    /// Recording driver: captures every `set_menu`/`do_action`/`try_move`
    /// instead of sending, so the guardian's sends are asserted directly.
    /// `route_origin` is `(0,0)` and `build_base` `(0,0)`, so absolute
    /// world tiles equal the recorded `try_move` target.
    struct FakeDriver {
        menus: Vec<(i32, i32, i32, i32, i32)>,
        actions: Vec<i32>,
        walks: Vec<(i32, i32)>,
        walk_ok: bool,
        route_origin: Option<(i32, i32)>,
        out: NoopOut,
    }

    impl Default for FakeDriver {
        fn default() -> Self {
            Self {
                menus: Vec::new(),
                actions: Vec::new(),
                walks: Vec::new(),
                walk_ok: true,
                route_origin: Some((0, 0)),
                out: NoopOut,
            }
        }
    }

    impl Driver for FakeDriver {
        fn set_menu(&mut self, slot: i32, action: i32, a: i32, b: i32, c: i32) {
            self.menus.push((slot, action, a, b, c));
        }
        fn do_action(&mut self, slot: i32) -> bool {
            self.actions.push(slot);
            true
        }
        fn try_move(
            &mut self,
            _src_x: i32,
            _src_z: i32,
            dx: i32,
            dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _t: i32,
        ) -> bool {
            self.walks.push((dx, dz));
            self.walk_ok
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            self.route_origin
        }
        fn build_base(&self) -> (i32, i32) {
            (0, 0)
        }
        fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
            None
        }
        fn out(&mut self) -> &mut dyn Out {
            &mut self.out
        }
        fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
            false
        }
    }

    /// Attach a socket and set ingame scene-2 so the `Interactions`
    /// preconditions (attached / ingame / scene ready) pass.
    fn ingame_scene(c: &mut Client) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("local addr");
        let stream = client::io::ClientStream::connect(&addr.ip().to_string(), addr.port())
            .expect("connect");
        std::mem::forget(listener);
        c.stream = Some(stream);
        c.ingame = true;
        c.scene_state = 2;
    }

    /// The chat modal root + its BUTTON_CONTINUE child (an open dialog).
    const CHAT_ROOT: i32 = 500;
    const CHAT_CONTINUE: i32 = 501;

    /// Open the chat modal with a continue button (the "dialog is open"
    /// state the guardian continues through).
    fn open_chat(c: &mut Client) {
        c.set_iface(
            CHAT_ROOT as usize,
            IfType {
                id: CHAT_ROOT,
                children: Some(vec![CHAT_CONTINUE]),
                ..Default::default()
            },
        );
        c.set_iface(
            CHAT_CONTINUE as usize,
            IfType {
                id: CHAT_CONTINUE,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            CHAT_CONTINUE as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_CONTINUE,
                ..Default::default()
            },
        );
        c.chat_modal_id = CHAT_ROOT;
        c.gens.iface += 1;
    }

    /// The lamp skill-choice component id.
    const CHAT_SKILL: i32 = 502;

    /// Open the chat modal with one BUTTON_OK choice (the lamp skill
    /// dialog shape).
    fn open_chat_choice(c: &mut Client, root: i32, component: i32, text: &str) {
        c.set_iface(
            root as usize,
            IfType {
                id: root,
                children: Some(vec![component]),
                ..Default::default()
            },
        );
        c.set_iface(
            component as usize,
            IfType {
                id: component,
                layer_id: root,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            component as usize,
            IfTypeMut {
                button_type: ButtonType::BUTTON_OK,
                text: text.to_string(),
                ..Default::default()
            },
        );
        c.chat_modal_id = root;
        c.gens.iface += 1;
    }

    #[test]
    fn guardian_talks_to_old_man_then_continues_open_chat() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Mysterious old man", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: our old man is a dialog event → Talk-to.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
        assert_eq!(status.kind, Some(RandomKind::Dialog));
        assert_eq!(status.name.as_deref(), Some("mysterious old man"));
        assert!(status.ours);
        assert!(status.handling);
        assert!(status.hold);
        assert!(status.toggle);
        assert!(!status.cooldown);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
            "Talk-to is the npc's first menu op"
        );
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the dialog is open → continue, not a second Talk-to.
        drv.menus.clear();
        drv.actions.clear();
        open_chat(&mut c);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
        assert!(status.handling);
        assert!(status.hold);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
            "an open chat continues"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    #[test]
    fn guardian_talks_to_a_sparse_npc_slot() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        // Only one live NPC, at client slot 7: the snapshot view list has
        // one entry (index 7), so a dense-vec lookup by `npc_index` would
        // miss it.
        plant_npc(&mut c, 7, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.handling);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_NPC1, 7, 0, 0)],
            "Talk-to must address the npc slot, not the dense vec position"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    #[test]
    fn talk_to_that_never_opens_chat_clears_on_continue_refuse() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: Talk-to arms the handle.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.handling, "Talk-to arms the in-flight handle");
        assert!(status.hold);

        // Tick 2: chat never opened → continue refuses; one-tick grace.
        drv.menus.clear();
        drv.actions.clear();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(
            status.handling,
            "first refuse after Talk-to keeps the handle for chat to open"
        );
        assert!(drv.actions.is_empty(), "refuse does not press");

        // Tick 3: still closed → clear (spec: continue refuses).
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(
            !status.handling,
            "second refuse with chat closed must lift the handle"
        );
        assert!(!status.hold, "no latch after continue refuse");
    }

    #[test]
    fn in_flight_dialog_continues_after_genie_despawns() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: talk-to the genie.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the chat opens → continue.
        drv.menus.clear();
        drv.actions.clear();
        open_chat(&mut c);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.actions, vec![0]);

        // Tick 3: the genie despawns while the chat is still open. detect
        // now finds nothing, but the in-flight dialog must keep continuing.
        drv.menus.clear();
        drv.actions.clear();
        c.npc[0] = None;
        c.npc_count = 0;
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.handling, "the open dialog keeps the handle");
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
            "a despawned genie must not stop the open chat"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    #[test]
    fn toggle_off_detects_but_never_sends() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings {
            random_events: false,
            ..ProfileSettings::default()
        };
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(drv.menus.is_empty(), "toggle off: no Talk-to");
        assert!(drv.actions.is_empty());
        assert_eq!(status.kind, Some(RandomKind::Dialog), "detect still fills");
        assert_eq!(status.name.as_deref(), Some("genie"));
        assert!(status.ours);
        assert!(!status.toggle);
        assert!(!status.handling);
        assert!(!status.hold);
    }

    #[test]
    fn wrong_talk_bins_npc_slot_and_other_slots_still_talk() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: talk-to the genie.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the chat rejects the talk → the slot is binned, and the
        // guardian neither continues nor re-talks it.
        drv.menus.clear();
        drv.actions.clear();
        c.add_chat(0, "It's not here for you.", "");
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 1_000, None);
        assert!(drv.actions.is_empty(), "wrong talk must not keep handling");
        assert!(status.cooldown, "the rejected slot is in the 45s bin");

        // Tick 3: the binned slot is skipped by detect — no send.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 2_000, None);
        assert!(drv.actions.is_empty());
        assert_eq!(status.kind, None);

        // A second genie on another slot is still talk-to-able.
        plant_npc(&mut c, 1, "Genie", -1, Some("Greetings Test!"));
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 3_000, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_NPC1, 1, 0, 0)],
            "an un-binned slot still gets Talk-to"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    // --- Task 5: trapped-kind hold, lamp, and the on_random knock ---

    #[test]
    fn maze_square_holds_without_any_send() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 45 * 64, 71 * 64);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Maze));
        assert!(status.hold, "a trapped kind holds the slot");
        assert!(drv.menus.is_empty(), "trapped kinds get no act");
        assert!(drv.actions.is_empty());

        // Toggle off: still detected, but never held.
        let settings = ProfileSettings {
            random_events: false,
            ..ProfileSettings::default()
        };
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Maze));
        assert!(!status.hold, "toggle off never holds");
    }

    #[test]
    fn lamp_auto_off_detects_but_never_rubs() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_obj(&mut c, LAMP_OBJ);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings {
            lamp_auto: false,
            ..ProfileSettings::default()
        };
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Lamp));
        assert_eq!(status.name.as_deref(), Some("lamp"));
        assert!(drv.menus.is_empty(), "lamp_auto off: no Rub");
        assert!(drv.actions.is_empty());
        assert!(!status.hold, "leftover lamp with auto off does not hold");
    }

    /// Plant obj `obj_id` into the inventory TYPE_INV iface (the shape
    /// `detect` reads the inv view from). Two slots with one empty, so a
    /// lost-gear take is not read as a full pack by default.
    fn plant_inv_obj(c: &mut Client, obj_id: i32) {
        plant_inv_named(c, obj_id, None);
    }

    /// Like [`plant_inv_obj`] with a cache name and a `Rub` held op (the
    /// lamp and lost-tool handle shapes).
    fn plant_inv_named(c: &mut Client, obj_id: i32, name: Option<&str>) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= obj_id as usize {
                cache.objs.push(client::config::ObjType::default());
            }
            cache.objs[obj_id as usize] = client::config::ObjType {
                id: obj_id,
                name: name.map(str::to_string).unwrap_or_default(),
                iop: [None, None, None, Some("Rub".into()), None],
                ..Default::default()
            };
        }
        c.side_icon[3] = 300;
        c.set_iface(
            300,
            IfType {
                id: 300,
                layer_id: 300,
                children: Some(vec![301]),
                ..Default::default()
            },
        );
        c.set_iface(
            301,
            IfType {
                id: 301,
                layer_id: 300,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![obj_id + 1, 0]),
                link_obj_number: Some(vec![1, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    }

    /// Empty the inventory TYPE_INV iface (the lamp-consumed shape).
    fn clear_inv(c: &mut Client) {
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![0, 0]),
                link_obj_number: Some(vec![0, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    }

    /// A wall loc of cache `name` at scene (`scene_x`, `scene_z`) (the
    /// hazard-loc shape).
    fn plant_loc(c: &mut Client, id: i32, name: &str, scene_x: i32, scene_z: i32) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.locs.len() <= id as usize {
                cache.locs.push(LocType::default());
            }
            cache.locs[id as usize] = LocType {
                id,
                name: name.to_string(),
                ..Default::default()
            };
        }
        let typecode = 0x4000_0000 + (id << 14) + scene_x + (scene_z << 7);
        c.world
            .set_wall(0, scene_x, scene_z, 0, 0, 0, typecode, 1 << 6, 0, 0, 0, 0);
    }

    #[test]
    fn on_random_knocks_once_per_event_edge() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();
        let knocks = std::sync::Arc::new(std::sync::Mutex::new(0usize));

        // Tick 1: the rising edge knocks the script.
        tick_at(&mut c, &mut snap);
        {
            let mut knock = |_: &DetectedRandom| {
                *knocks.lock().unwrap() += 1;
                RandomClaim::Handle
            };
            let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
            assert_eq!(*knocks.lock().unwrap(), 1, "one knock on the edge");
            assert_eq!(status.claim, RandomClaim::Handle);
            assert!(!status.hold, "a Handle claim never holds");
            assert!(drv.menus.is_empty(), "a Handle claim blocks Talk-to");
            assert!(drv.actions.is_empty());
        }

        // Tick 2: the same event persists → no second knock, the claim
        // sticks.
        tick_at(&mut c, &mut snap);
        {
            let mut knock = |_: &DetectedRandom| {
                *knocks.lock().unwrap() += 1;
                RandomClaim::Handle
            };
            let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
            assert_eq!(
                *knocks.lock().unwrap(),
                1,
                "a persisting event must not re-knock"
            );
            assert_eq!(status.claim, RandomClaim::Handle);
        }

        // Tick 3: the event vanishes → the claim resets to Host.
        c.npc[0] = None;
        c.npc_count = 0;
        tick_at(&mut c, &mut snap);
        {
            let mut knock = |_: &DetectedRandom| {
                *knocks.lock().unwrap() += 1;
                RandomClaim::Handle
            };
            let status = g.tick(&mut drv, &snap, &settings, 0, Some(&mut knock));
            assert_eq!(status.kind, None);
            assert_eq!(
                status.claim,
                RandomClaim::Host,
                "a vanished event hands the claim back to the host"
            );
            assert_eq!(*knocks.lock().unwrap(), 1, "no event → no knock");
        }
    }

    #[test]
    fn no_knock_source_keeps_host_claim_and_talks() {
        // No knock supplied (host-owned slot): the claim stays Host and a
        // dialog event still talks.
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.menus, vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)]);
        assert_eq!(drv.actions, vec![0]);
        assert!(status.handling);
        assert!(status.hold);
    }

    // --- Task 9: range WalkTo + the remaining act kinds ---

    #[test]
    fn flee_candidates_rings_are_farthest_first() {
        let tiles = flee_candidates((10, 20));
        assert_eq!(tiles.len(), 40, "8 compass points x 5 rings");
        assert_eq!(tiles[0], (10, 8), "north of the dist-12 ring first");
        assert_eq!(tiles[7], (-2, 8), "north-west closes the dist-12 ring");
        assert_eq!(tiles[8], (10, 10), "the dist-10 ring comes next");
        assert_eq!(tiles[39], (6, 16), "north-west of the dist-4 ring last");
        assert!(
            tiles[0..8]
                .iter()
                .all(|(x, z)| cheb((10, 20), (*x, *z)) == 12),
            "every first-ring tile is chebyshev 12 from the threat"
        );
    }

    #[test]
    fn out_of_range_dialog_walks_to_npc_before_talk_to() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Genie", -1, Some("Greetings Test!"));
        // Move the genie to tile (3, 0): out of Talk-to range.
        c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
        c.npc[0].as_mut().expect("planted").entity.z = 64;
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Dialog));
        assert_eq!(
            drv.walks,
            vec![(3, 0)],
            "an out-of-range Talk-to arms a walk to the npc tile"
        );
        assert!(drv.menus.is_empty(), "no Talk-to while out of range");
        assert!(drv.actions.is_empty());
        assert!(status.hold, "the range walk holds the slot");
    }

    #[test]
    fn evade_swarm_holds_flees_and_walks_back_after_despawn() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc(&mut c, 0, "Swarm", 32768, None);
        c.npc[0].as_mut().expect("planted").entity.x = 3 * 128 + 64;
        c.npc[0].as_mut().expect("planted").entity.z = 64;
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: the swarm targets us → flee away, hold the slot.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Evade));
        assert!(status.ours);
        assert!(status.hold, "an in-flight flee holds the slot");
        assert_eq!(
            drv.walks,
            vec![(3, -12)],
            "the first flee candidate is the farthest ring, north of the threat"
        );

        // Tick 2: the swarm despawned → the hold lifts and the guardian
        // walks back toward the pre-flee tile.
        drv.walks.clear();
        c.npc[0] = None;
        c.npc_count = 0;
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, None);
        assert!(!status.hold, "the hold lifts once the threat is gone");
        assert_eq!(drv.walks, vec![(0, 0)], "walk back to the pre-flee tile");
    }

    #[test]
    fn lamp_auto_rubs_then_answers_the_skill_button() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_obj(&mut c, LAMP_OBJ);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default(); // lamp_auto on, skill strength
        let mut snap = GameSnapshot::new();

        // Tick 1: rub the lamp and hold while the lamp iface is in flight.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Lamp));
        assert!(status.hold, "lamp auto-use holds while rubbing");
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_HELD4, LAMP_OBJ, 0, 301)],
            "Rub is the lamp's 4th held op"
        );

        // Tick 2: the skill dialog offers Strength → the auto skill is pressed.
        drv.menus.clear();
        drv.actions.clear();
        open_chat_choice(&mut c, CHAT_ROOT, CHAT_SKILL, "Strength");
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.hold);
        assert_eq!(drv.menus.len(), 1, "one skill button press");
        assert_eq!(drv.menus[0].1, MiniMenuAction::IF_BUTTON);
        assert_eq!(drv.menus[0].4, CHAT_SKILL);

        // Tick 3: the lamp is consumed → the hold lifts.
        drv.menus.clear();
        drv.actions.clear();
        clear_inv(&mut c);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, None);
        assert!(!status.hold);
    }

    #[test]
    fn ground_harpoon_not_in_inv_takes_within_reach() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_obj(&mut c, 999); // a free pack slot
        plant_ground_obj(&mut c, 3, 0, 502, Some("Harpoon"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::LostGear));
        assert_eq!(status.name.as_deref(), Some("harpoon"));
        assert!(status.hold, "a take in flight holds the slot");
        assert_eq!(drv.menus.len(), 1);
        assert_eq!(
            drv.menus[0].1,
            MiniMenuAction::OP_OBJ3,
            "Take is the ground item's 3rd op"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    #[test]
    fn lost_tool_handle_in_inv_uses_on_ground_head() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_ground_obj(&mut c, 0, 0, 503, Some("Bronze axe head"));
        plant_inv_named(&mut c, 504, Some("Bronze axe handle"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::LostTool));
        assert!(status.hold, "the reattach holds while in flight");
        assert_eq!(
            drv.menus.len(),
            2,
            "use-on arms select then the ground target"
        );
        assert_eq!(drv.menus[0].1, MiniMenuAction::USEHELD_START);
        assert_eq!(
            drv.menus[1].1,
            MiniMenuAction::USEHELD_ONOBJ,
            "the handle is used on the ground head"
        );
    }

    #[test]
    fn lost_tool_without_ground_head_sends_no_reattach() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_named(&mut c, 504, Some("Bronze axe handle"));
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::LostTool));
        assert!(drv.menus.is_empty(), "no head: no fake reattach");
        assert!(drv.actions.is_empty());
        assert!(!status.hold, "nothing in flight: no hold");
    }

    #[test]
    fn hazard_smoking_rock_underfoot_walks_off() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_loc(&mut c, 510, "Smoking rock", 0, 0);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Hazard));
        assert_eq!(status.name.as_deref(), Some("smoking rock"));
        assert!(status.hold, "stepping off holds the slot");
        assert_eq!(
            drv.walks,
            vec![(0, -12)],
            "flee ring from self, farthest first"
        );
    }

    #[test]
    fn our_strange_plant_gets_picked() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_npc_with_op(&mut c, 0, "Strange plant", -1, Some("Pick Test!"), "Pick");
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Pick));
        assert!(status.ours);
        assert!(status.hold, "the pick holds while the plant is still there");
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_NPC1, 0, 0, 0)],
            "Pick is the plant's 1st op"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    // --- Task 10: mime + strange-box solvers ---

    #[test]
    fn mime_answer_maps_seq_to_button_index() {
        assert_eq!(mime_answer(860), Some(0), "emote_cry");
        assert_eq!(mime_answer(857), Some(1), "emote_think");
        assert_eq!(mime_answer(861), Some(2), "emote_laugh");
        assert_eq!(mime_answer(866), Some(3), "emote_dance");
        assert_eq!(mime_answer(1130), Some(4), "emote_climbing_rope");
        assert_eq!(mime_answer(1129), Some(5), "emote_mime_lean");
        assert_eq!(mime_answer(1128), Some(6), "emote_glass_wall");
        assert_eq!(mime_answer(1131), Some(7), "emote_glass_box");
    }

    #[test]
    fn mime_answer_unknown_seq_is_none() {
        assert_eq!(mime_answer(858), None, "bow");
        assert_eq!(mime_answer(862), None, "cheer/idle");
        assert_eq!(mime_answer(0), None);
    }

    #[test]
    fn solve_cube_answers_colour_question_by_shape_position() {
        assert_eq!(
            solve_cube(
                "What colour is the Square?",
                [Some(3069), Some(3065), Some(3075)]
            ),
            Some(0),
            "square-red sits in model slot 0"
        );
        assert_eq!(
            solve_cube(
                "What colour is the Star?",
                [Some(3063), Some(3085), Some(3071)]
            ),
            Some(1),
            "the star is model slot 1"
        );
        assert_eq!(
            solve_cube(
                "What colour is the Half Moon?",
                [Some(3089), Some(3063), Some(3079)]
            ),
            Some(0),
            "two-word shape still matches"
        );
    }

    #[test]
    fn solve_cube_answers_shape_question_by_colour_position() {
        assert_eq!(
            solve_cube("Which shape is Blue?", [Some(3063), Some(3085), Some(3089)]),
            Some(2),
            "the blue part is model slot 2"
        );
    }

    #[test]
    fn solve_cube_unknown_question_or_missing_model_is_none() {
        assert_eq!(solve_cube("??", [Some(3063), Some(3071), Some(3079)]), None);
        assert_eq!(
            solve_cube("What colour is the Star?", [None, Some(3063), Some(3071)]),
            None,
            "a missing model obj id is unsolvable"
        );
        assert_eq!(
            solve_cube(
                "What colour is the Potato?",
                [Some(3063), Some(3071), Some(3079)]
            ),
            None,
            "a shape not on the cube is unsolvable"
        );
    }

    /// Plant `qty` Strange boxes (obj 3062, `Open` as held op 1) into the
    /// inv (one stacked row).
    fn plant_inv_box(c: &mut Client, qty: i32) {
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.objs.len() <= STRANGE_BOX_OBJ as usize {
                cache.objs.push(client::config::ObjType::default());
            }
            cache.objs[STRANGE_BOX_OBJ as usize] = client::config::ObjType {
                id: STRANGE_BOX_OBJ,
                name: "Strange box".to_string(),
                iop: [Some("Open".into()), None, None, None, None],
                ..Default::default()
            };
        }
        c.side_icon[3] = 300;
        c.set_iface(
            300,
            IfType {
                id: 300,
                layer_id: 300,
                children: Some(vec![301]),
                ..Default::default()
            },
        );
        c.set_iface(
            301,
            IfType {
                id: 301,
                layer_id: 300,
                r#type: ComponentType::TYPE_INV,
                obj_ops: true,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![STRANGE_BOX_OBJ + 1, 0]),
                link_obj_number: Some(vec![qty, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
    }

    /// Open the mysterious-cube main modal (macro_cube 6554) with three
    /// obj-model children, the question text and the three answer buttons.
    fn open_cube(c: &mut Client, question: &str, models: [i32; 3]) {
        c.set_iface(
            6554,
            IfType {
                id: 6554,
                layer_id: 6554,
                r#type: ComponentType::TYPE_LAYER,
                children: Some(vec![6555, 6557, 6559, 6561, 6562, 6563, 6564]),
                ..Default::default()
            },
        );
        for (com, obj) in [(6555, models[0]), (6557, models[1]), (6559, models[2])] {
            c.set_iface(
                com as usize,
                IfType {
                    id: com,
                    layer_id: 6554,
                    r#type: ComponentType::TYPE_MODEL,
                    ..Default::default()
                },
            );
            c.set_iface_mut(
                com as usize,
                IfTypeMut {
                    model1_type: 4,
                    model1_id: obj,
                    ..Default::default()
                },
            );
        }
        c.set_iface(
            6561,
            IfType {
                id: 6561,
                layer_id: 6554,
                r#type: ComponentType::TYPE_TEXT,
                ..Default::default()
            },
        );
        c.set_iface_mut(
            6561,
            IfTypeMut {
                text: question.to_string(),
                ..Default::default()
            },
        );
        for com in [6562, 6563, 6564] {
            c.set_iface(
                com as usize,
                IfType {
                    id: com,
                    layer_id: 6554,
                    r#type: ComponentType::TYPE_TEXT,
                    ..Default::default()
                },
            );
            c.set_iface_mut(
                com as usize,
                IfTypeMut {
                    button_type: ButtonType::BUTTON_OK,
                    ..Default::default()
                },
            );
        }
        c.main_modal_id = 6554;
        c.gens.iface += 1;
    }

    #[test]
    fn mime_square_answers_emote_and_holds_until_off_square() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 31 * 64, 74 * 64);
        plant_npc(&mut c, 0, "Mime", -1, None);
        c.npc[0].as_mut().expect("planted").entity.primary_anim = 860;
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: on the square, no emote chat yet → the guardian watches
        // the mime, sends nothing.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Mime));
        assert!(status.hold, "on the mime square the slot holds");
        assert!(drv.menus.is_empty(), "no press before the emote chat opens");
        assert!(drv.actions.is_empty());

        // Tick 2: the emote chat (6543) opens → IF_BUTTON on the button
        // for the last seen emote (cry → index 0 → 6546).
        drv.menus.clear();
        drv.actions.clear();
        c.chat_modal_id = 6543;
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.hold);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6546)],
            "emote index 0 maps to button 6546"
        );
        assert_eq!(drv.actions, vec![0]);

        // Tick 3: the chat stays open → one press per chat-open, no spam.
        drv.menus.clear();
        drv.actions.clear();
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.hold);
        assert!(
            drv.menus.is_empty(),
            "no repeat press while the chat stays up"
        );
        assert!(drv.actions.is_empty());

        // Tick 4: off the mime square → the hold lifts.
        drv.menus.clear();
        drv.actions.clear();
        plant_player(&mut c, "Test", 0, 0);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, None);
        assert!(!status.hold, "off the mime square the hold lifts");
        assert!(drv.menus.is_empty());
        assert!(drv.actions.is_empty());
    }

    #[test]
    fn strange_box_opens_solves_and_holds_until_consumed() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_box(&mut c, 1);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: no cube iface yet → Open the held box (held op 1).
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Box));
        assert_eq!(status.name.as_deref(), Some("strange box"));
        assert!(status.hold, "a held strange box holds the slot");
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
            "Open is the box's 1st held op"
        );
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the cube iface opens → the Square question answers the
        // square-red model (slot 0) via IF_BUTTON 6562.
        drv.menus.clear();
        drv.actions.clear();
        open_cube(&mut c, "What colour is the Square?", [3069, 3065, 3075]);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(status.hold);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6562)],
            "square-red is model slot 0 → answer button 1"
        );
        assert_eq!(drv.actions, vec![0]);

        // Tick 3: the box is consumed → the hold lifts.
        drv.menus.clear();
        drv.actions.clear();
        clear_inv(&mut c);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, None);
        assert!(!status.hold, "no box: no hold");
        assert!(drv.menus.is_empty());
    }

    #[test]
    fn strange_box_reopens_while_more_boxes_held() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_box(&mut c, 2);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: Open the first box.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
            "a held box opens"
        );

        // Tick 2: the cube iface answers the first box.
        drv.menus.clear();
        drv.actions.clear();
        open_cube(&mut c, "What colour is the Square?", [3069, 3065, 3075]);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.menus, vec![(0, MiniMenuAction::IF_BUTTON, 0, 0, 6562)]);

        // Tick 3: one box consumed (2→1) and the modal closed → the next
        // held box opens (rs2b0t repeats while the inv holds one).
        drv.menus.clear();
        drv.actions.clear();
        c.set_iface_mut(
            301,
            IfTypeMut {
                link_obj_type: Some(vec![STRANGE_BOX_OBJ + 1, 0]),
                link_obj_number: Some(vec![1, 0]),
                ..Default::default()
            },
        );
        c.bump_gens(client::io::ServerProt::UPDATE_INV_FULL);
        c.main_modal_id = -1;
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_HELD1, STRANGE_BOX_OBJ, 0, 301)],
            "a second held box reopens after the first was consumed"
        );
        assert_eq!(drv.actions, vec![0]);
    }

    #[test]
    fn strange_box_unknown_question_sends_no_click() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 0, 0);
        plant_inv_box(&mut c, 1);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: open the box.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Box));
        assert!(status.hold);
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: an unsolvable cube question → fail closed: no click,
        // the trapped hold stays while the box is held.
        drv.menus.clear();
        drv.actions.clear();
        open_cube(&mut c, "??", [3063, 3071, 3079]);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Box));
        assert!(status.hold, "an unsolvable cube keeps the trapped hold");
        assert!(drv.menus.is_empty(), "unknown question: no click");
        assert!(drv.actions.is_empty());
    }

    // --- Task 11: maze behavioural port ---

    /// The NW spawn's route (door 0 (2890,4592), door 1 (2888,4587)).
    fn nw_route() -> Vec<(i32, i32)> {
        maze::select_route(maze::graph(), maze::MAZE_SPAWNS[0]).expect("the NW spawn solves")
    }

    #[test]
    fn maze_spawn_walks_opens_and_advances_door_to_door() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 2891, 4597); // NW spawn
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Tick 1: the route solves from the observed tile (no send yet).
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Maze));
        assert!(status.hold, "on the maze square the slot holds");
        assert!(drv.menus.is_empty() && drv.walks.is_empty());

        // Tick 2: walk to door 0.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.walks, vec![(2890, 4592)], "walk toward door 0");

        // Adjacent to the door: oploc Open with the door's loc id.
        drv.walks.clear();
        plant_player(&mut c, "Test", 2890, 4593);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_LOC1, 3628, 2890, 4592)],
            "oploc Open is the door's OP_LOC1"
        );
        assert!(drv.walks.is_empty());

        // The open pushes the player through (>= 2 tiles): advance.
        drv.menus.clear();
        plant_player(&mut c, "Test", 2887, 4592);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(drv.menus.is_empty(), "the through tick just advances");
        assert!(drv.walks.is_empty());

        // Next tick: walk to door 1.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.walks, vec![(2888, 4587)], "walk toward door 1");
    }

    #[test]
    fn maze_wrong_door_mesbox_is_continued_then_the_route_advances() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 2891, 4597);
        let mut g = Guardian::new();
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Solve, walk to door 0, open it.
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        plant_player(&mut c, "Test", 2890, 4593);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_LOC1, 3628, 2890, 4592)]
        );

        // The door refuses: the wrong-door mesbox opens → continue drains
        // it (rs2b0t clearMesbox).
        drv.menus.clear();
        drv.walks.clear();
        open_chat(&mut c);
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::PAUSE_BUTTON, 0, 0, CHAT_CONTINUE)],
            "the wrong-door mesbox is continued through"
        );

        // Chat closed: the refused door advances the route → walk to door 1.
        drv.menus.clear();
        c.chat_modal_id = -1;
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert!(drv.menus.is_empty(), "the refusal tick just advances");
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(drv.walks, vec![(2888, 4587)], "advance after the refusal");
    }

    #[test]
    fn maze_walled_off_door_resyncs_then_gives_up_after_three() {
        let mut solve = maze::MazeSolve::new(nw_route());
        solve.next = 1; // door 0 already behind us
        let mut drv = FakeDriver::default();
        let snap = GameSnapshot::new(); // no chat

        // Door 1 never gets closer: WALK_LIMIT sends, then the walk is
        // stuck → one resync back through door 0.
        for _ in 0..maze::WALK_LIMIT {
            assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
        }
        drv.walks.clear();
        assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
        assert_eq!(solve.phase, maze::MazePhase::Resync);
        assert_eq!(solve.resyncs, 1);
        drv.walks.clear();
        assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
        assert_eq!(drv.walks, vec![(2890, 4592)], "resync walks to door 0");

        // Three resyncs are the ceiling: a fourth walled-off door gives
        // up the pass instead of stepping back again.
        solve.resyncs = maze::MAX_RESYNCS;
        solve.phase = maze::MazePhase::WalkDoor;
        solve.walk_from = None;
        solve.walk_sends = 0;
        drv.walks.clear();
        for _ in 0..maze::WALK_LIMIT {
            assert!(step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)));
        }
        assert!(
            !step_maze_phase(&mut solve, &mut drv, &snap, (2891, 4597)),
            "resyncs capped: the pass gives up"
        );
    }

    #[test]
    fn maze_touches_the_shrine_and_the_hold_lifts_off_square() {
        let mut c = new_client();
        ingame_scene(&mut c);
        plant_player(&mut c, "Test", 2911, 4576); // post-chamber-door tile
        let mut g = Guardian::new();
        let mut solve = maze::MazeSolve::new(vec![]);
        solve.phase = maze::MazePhase::Touch { pass: 0 };
        g.maze = Some(solve);
        let mut drv = FakeDriver::default();
        let settings = ProfileSettings::default();
        let mut snap = GameSnapshot::new();

        // Near the shrine on pass 0: Touch from where we stand.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, Some(RandomKind::Maze));
        assert!(status.hold, "the trapped hold stays while touching");
        assert_eq!(
            drv.menus,
            vec![(
                0,
                MiniMenuAction::OP_LOC1,
                maze::MAZE_SHRINE_LOC,
                2911,
                4575
            )],
            "Touch is the shrine's OP_LOC1"
        );
        assert!(drv.walks.is_empty(), "near the shrine: no stand walk");

        // The shrine teleports the player off the square → the hold lifts.
        drv.menus.clear();
        plant_player(&mut c, "Test", 0, 0);
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 0, None);
        assert_eq!(status.kind, None);
        assert!(!status.hold, "off the maze square the hold lifts");
    }
}
