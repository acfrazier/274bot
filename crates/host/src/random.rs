//! Random-event detection over one `GameSnapshot` (guardian spec
//! `2026-09-01-random-event-guardian-design.md`, the Detect / Ours / Kinds
//! locks). Detect is snapshot-only and stateless apart from the
//! caller-owned [`CooldownMap`]; act / hold / dialog and the wrong-talk
//! 45 s cooldown writes are Task 4.

use std::collections::HashMap;

use api::interact::{ActionSpec, Driver, Interactions, OpTarget, SendResult, SCENE_READY};
use api::snapshot::{ActorKind, ActorTargetView, GameSnapshot, NpcView};
use vault::ProfileSettings;

/// The kind of random event one detection found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomKind {
    Dialog,
    Pick,
    Evade,
    Maze,
    Mime,
    Box,
    Lamp,
    Hazard,
    LostTool,
    LostGear,
}

/// One detected random event on the current snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRandom {
    pub kind: RandomKind,
    pub name: String,
    pub ours: bool,
    pub npc_index: Option<usize>,
}

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

/// Ground-search radius for lost tool/gear (rs2b0t `.within(10)`).
const LOST_GEAR_RADIUS: i32 = 10;

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
/// then lost-gear on the ground, then inv-held box/lamp, then lost-tool.
/// Returns `None` when nothing applies. NPC kinds are owner-gated
/// (except `pick`, which the TUI may show as not ours). The rs2b0t
/// gear-loss 90 s window (the caller-state half of `lost-gear`) is not
/// needed here: gear on the ground and out of the inventory detects.
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
    if let Some(gear) = lost_gear(snap) {
        return Some(no_npc_event(RandomKind::LostGear, &gear));
    }
    if snap.inv().iter().any(|(id, _)| *id == STRANGE_BOX_OBJ) {
        return Some(no_npc_event(RandomKind::Box, "strange box"));
    }
    if snap.inv().iter().any(|(id, _)| *id == LAMP_OBJ) {
        return Some(no_npc_event(RandomKind::Lamp, "lamp"));
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
    let handle = |name: Option<&str>| {
        name.is_some_and(|n| {
            let l = n.to_lowercase();
            l.contains("axe handle") || l.contains("pickaxe handle")
        })
    };
    snap.inventory()
        .iter()
        .any(|i| handle(i.def.name.as_deref()))
        || snap
            .equipment()
            .iter()
            .any(|i| handle(i.def.name.as_deref()))
        || snap
            .ground_items()
            .iter()
            .any(|g| g.distance <= LOST_GEAR_RADIUS && handle(g.def.name.as_deref()))
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
// Guardian (the act/hold-while-handling half; skip-tick/follow freeze and
// the `on_random` knock are Task 5). One instance per slot.
// ---------------------------------------------------------------------------

/// The dialog-continue ceiling (rs2b0t `MAX_DIALOGUE_STEPS`).
const MAX_CONTINUES: u32 = 25;

/// Wrong-talk cooldown for an NPC slot (rs2b0t 45 s).
const WRONG_TALK_COOLDOWN_MS: u64 = 45_000;

/// Chat markers of a failed Talk-to: the NPC is not the event's owner.
const WRONG_TALK_MARKERS: &[&str] = &["trying to talk to", "It's not here for you."];

/// Who handles a detected random event. Task 5 wires `Script::on_random`;
/// nothing overrides `Host` this tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RandomClaim {
    Host,
    Handle,
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
    /// The slot must skip script tick / follow (Task 5 enforces the
    /// freeze). Task 4 fills the dialog-in-flight arm only.
    pub hold: bool,
    pub toggle: bool,
    pub claim: RandomClaim,
    /// The shown NPC slot is in the 45 s wrong-talk bin.
    pub cooldown: bool,
}

/// Per-slot random-event guardian state.
pub struct Guardian {
    /// A Talk-to went out and the dialog may still be open.
    pub in_flight: bool,
    /// The snapshot tick the last act ran on (one scan per game tick).
    pub last_tick: u64,
    /// NPC slot → wrong-talk cooldown expiry `now_ms`.
    pub cooldown: CooldownMap,
    /// Signature (kind+name) of the last handled event (the Task 5
    /// rising-edge `on_random` key).
    pub sig: Option<String>,
    /// Who handles the current event (always `Host` until Task 5).
    pub claim: RandomClaim,
    /// The NPC slot the in-flight dialog targets (the cooldown key).
    in_flight_index: Option<usize>,
    /// Continues sent for the in-flight dialog ([`MAX_CONTINUES`] cap).
    continues: u32,
    /// Newest chat sequence already scanned for wrong-talk lines, so a
    /// stale rejection line cannot re-bin a later NPC.
    chat_seen: i32,
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
        }
    }

    /// One guardian pass per caller frame: detect + publish the status
    /// every frame, but act at most once per snapshot `tick` (the
    /// PLAYER_INFO game-tick edge). Talk-to runs only for a dialog event
    /// that is ours on an un-binned slot; an open dialog then continues
    /// via `continue_dialog` / `answer_choice` (first option), max
    /// [`MAX_CONTINUES`]. A wrong-talk chat line bins that NPC slot for
    /// 45 s. Toggle off: never act, never hold, still detect+publish.
    pub fn tick<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        settings: &ProfileSettings,
        now_ms: u64,
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
        }

        if fresh && active && settings.random_events && self.claim == RandomClaim::Host {
            self.act(driver, snap, ev.as_ref(), now_ms);
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
            hold: self.in_flight && settings.random_events && self.claim == RandomClaim::Host,
            toggle: settings.random_events,
            claim: self.claim,
            cooldown,
        }
    }

    /// One send per game tick: Talk-to on the rising edge, then continue
    /// while the chat stays open. Refuses silently when the wire layer
    /// says no (not ingame, stale target, chat closed), so the machine is
    /// driven by the snapshot, not by error paths.
    fn act<D: Driver>(
        &mut self,
        driver: &mut D,
        snap: &GameSnapshot,
        ev: Option<&DetectedRandom>,
        now_ms: u64,
    ) {
        let Some(ev) = ev else { return };
        if ev.kind != RandomKind::Dialog || !ev.ours {
            return;
        }
        let Some(index) = ev.npc_index else { return };
        // A slot binned this pass (the wrong-talk above) or earlier is
        // not re-engaged; detect skips binned slots, this guards the
        // same-tick bin.
        if binned(index, now_ms, &self.cooldown) {
            return;
        }
        let Some(npc) = snap.npcs().get(index) else {
            return;
        };

        let mut ix = Interactions::new(snap, driver);
        if self.in_flight {
            // Keep talking the open dialog through.
            if self.continues >= MAX_CONTINUES {
                self.clear_handle();
                return;
            }
            let result = if snap.chat_options().is_empty() {
                ix.continue_dialog()
            } else {
                ix.answer_choice(1)
            };
            if let SendResult::Sent { .. } = result {
                self.continues += 1;
            }
        } else {
            match ix.interact(OpTarget::Npc(npc), ActionSpec::Label("Talk-to".to_string())) {
                SendResult::Sent { .. } => {
                    self.in_flight = true;
                    self.in_flight_index = Some(index);
                    self.continues = 0;
                    self.sig = Some(format!("{:?}:{}", ev.kind, ev.name));
                }
                SendResult::Refused { .. } => {}
            }
        }
    }

    /// The handle lifts when the chat is fully closed and the in-flight
    /// NPC has left the scene (the spec's "NPC gone and chat closed").
    fn dialog_done(&self, snap: &GameSnapshot) -> bool {
        let chat_open = snap.chat_continue_component_id() != -1
            || !snap.chat_options().is_empty()
            || snap.modals().chat != -1;
        let npc_here = self
            .in_flight_index
            .is_some_and(|i| snap.npcs().iter().any(|v| v.index == i));
        !chat_open && !npc_here
    }

    fn clear_handle(&mut self) {
        self.in_flight = false;
        self.in_flight_index = None;
        self.continues = 0;
    }
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
    use client::config::{Cache, NpcType, ObjType};
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
        let type_id = 500 + slot;
        {
            let cache = Arc::get_mut(&mut c.cache).expect("sole cache owner");
            while cache.npcs.len() <= type_id {
                cache.npcs.push(NpcType::default());
            }
            cache.npcs[type_id] = NpcType {
                id: type_id as i32,
                name: name.to_string(),
                op: vec![Some("Talk-to".to_string())],
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

    /// Recording driver: captures every `set_menu`/`do_action` instead of
    /// sending, so the guardian's sends are asserted directly.
    struct FakeDriver {
        menus: Vec<(i32, i32, i32, i32, i32)>,
        actions: Vec<i32>,
        out: NoopOut,
    }

    impl Default for FakeDriver {
        fn default() -> Self {
            Self {
                menus: Vec::new(),
                actions: Vec::new(),
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
            _dx: i32,
            _dz: i32,
            _try_nearest: bool,
            _loc_width: i32,
            _loc_length: i32,
            _loc_angle: i32,
            _loc_shape: i32,
            _forceapproach: i32,
            _t: i32,
        ) -> bool {
            false
        }
        fn local_route(&self) -> Option<(i32, i32)> {
            None
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
        let status = g.tick(&mut drv, &snap, &settings, 1_000);
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
        let status = g.tick(&mut drv, &snap, &settings, 1_000);
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
        let status = g.tick(&mut drv, &snap, &settings, 0);
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
        g.tick(&mut drv, &snap, &settings, 0);
        assert_eq!(drv.actions, vec![0]);

        // Tick 2: the chat rejects the talk → the slot is binned, and the
        // guardian neither continues nor re-talks it.
        drv.menus.clear();
        drv.actions.clear();
        c.add_chat(0, "It's not here for you.", "");
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 1_000);
        assert!(drv.actions.is_empty(), "wrong talk must not keep handling");
        assert!(status.cooldown, "the rejected slot is in the 45s bin");

        // Tick 3: the binned slot is skipped by detect — no send.
        tick_at(&mut c, &mut snap);
        let status = g.tick(&mut drv, &snap, &settings, 2_000);
        assert!(drv.actions.is_empty());
        assert_eq!(status.kind, None);

        // A second genie on another slot is still talk-to-able.
        plant_npc(&mut c, 1, "Genie", -1, Some("Greetings Test!"));
        tick_at(&mut c, &mut snap);
        g.tick(&mut drv, &snap, &settings, 3_000);
        assert_eq!(
            drv.menus,
            vec![(0, MiniMenuAction::OP_NPC1, 1, 0, 0)],
            "an un-binned slot still gets Talk-to"
        );
        assert_eq!(drv.actions, vec![0]);
    }
}
