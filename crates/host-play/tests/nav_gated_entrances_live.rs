//! Live Engine A proofs for the four newly routed, content-gated gathering entrances.
//! Every route uses the selected navigation pack, `find_with`, and `Traveller`.
use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::game_data::{self, GameItem, QuestFamilyIdentity};
use api::gather_methods::{AccessPolicy, GatherCatalog, GatherSkill, GatherSpot};
use api::identity::Deadline;
use api::interact::{self, Interactions, SendResult};
use api::quest_progress::{self, EvidenceStamp, QuestListEvidenceProvider};
use api::selected::{QuestGate, RunKey, Truth};
use api::snapshot::{GameSnapshot, QuestListStatus, ReadContext, SnapshotView, WorldTile};
use api::work::{PreparationPhase, SharedCancellation, SliceBudget, StepContext, SystemClock};
use client::client::Client;
use client::io::ClientRevision;
use host::Pump;
use host_play::{mint_live_entries, mint_live_names, ProfileOptions, SharedClientTemplate};
use nav::quest_gates::{QuestEvidence, QuestFamilyId};
use nav::router::{find_with, FindOptions, Leg, Route, RouteError};
use nav::transport::TransportEdge;
use nav::traveller::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav::world::NavWorld;
use nav::WorldState;
use serde::Serialize;
use serde_json::{json, Value};

const SETUP_TIMEOUT: Duration = Duration::from_secs(90);
const TRAVEL_TIMEOUT: Duration = Duration::from_secs(180);
const LOGOUT_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const ANA_BARREL_ID: i32 = 1842;
const DESERT_KEY_ID: i32 = 1839;
const DUSTY_KEY_ID: i32 = 1590;
const GRAND_TREE_TRAPDOOR_ID: i32 = 2446;
const DESERT_SURFACE_GATE_IDS: &[i32] = &[2673, 2674];
const DESERT_UNDERGROUND_GATE_IDS: &[i32] = &[2675, 2676];
const DUSTY_KEY_DOOR_ID: i32 = 2623;
const DESERT_KIT_IDS: &[i32] = &[1844, 1845, 1846];
const NEIGHBOR_OFFSETS: &[(i32, i32)] = &[
    (-1, -1),
    (-1, 0),
    (-1, 1),
    (0, -1),
    (0, 1),
    (1, -1),
    (1, 0),
    (1, 1),
];
const QUEST_TEST_RUN: RunKey = RunKey {
    slot: 1,
    run: 1,
    session: 1,
};

const GRAND_TREE_ANCHORS: &[WorldTile] = &[WorldTile {
    x: 2463,
    z: 3497,
    level: 0,
}];
const DESERT_SURFACE_ANCHORS: &[WorldTile] = &[
    WorldTile {
        x: 3273,
        z: 3028,
        level: 0,
    },
    WorldTile {
        x: 3273,
        z: 3029,
        level: 0,
    },
];
const DESERT_UNDERGROUND_ANCHORS: &[WorldTile] = &[
    WorldTile {
        x: 3301,
        z: 3036,
        level: 0,
    },
    WorldTile {
        x: 3302,
        z: 3036,
        level: 0,
    },
];
const DUSTY_KEY_ANCHORS: &[WorldTile] = &[WorldTile {
    x: 2924,
    z: 9803,
    level: 0,
}];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntranceCase {
    GrandTreeMine,
    DesertMiningCamp,
    DesertMiningCampUnderground,
    TaverleyLavaEels,
}

impl EntranceCase {
    fn name(self) -> &'static str {
        match self {
            Self::GrandTreeMine => "grand-tree-mine",
            Self::DesertMiningCamp => "desert-mining-camp",
            Self::DesertMiningCampUnderground => "desert-mining-camp-underground",
            Self::TaverleyLavaEels => "taverley-lava-eels",
        }
    }

    fn skill(self) -> &'static str {
        match self {
            Self::TaverleyLavaEels => "fishing",
            _ => "mining",
        }
    }

    fn site_id(self) -> &'static str {
        match self {
            Self::GrandTreeMine => "mining.grand_tree.underground",
            Self::DesertMiningCamp => "mining.desert_mining_camp",
            Self::DesertMiningCampUnderground => "mining.desert_mining_camp.underground",
            Self::TaverleyLavaEels => "fishing.dark_wizards_tower.underground.nw",
        }
    }

    fn loc_ids(self) -> &'static [i32] {
        match self {
            Self::GrandTreeMine => &[GRAND_TREE_TRAPDOOR_ID],
            Self::DesertMiningCamp => DESERT_SURFACE_GATE_IDS,
            Self::DesertMiningCampUnderground => DESERT_UNDERGROUND_GATE_IDS,
            Self::TaverleyLavaEels => &[DUSTY_KEY_DOOR_ID],
        }
    }

    fn anchors(self) -> &'static [WorldTile] {
        match self {
            Self::GrandTreeMine => GRAND_TREE_ANCHORS,
            Self::DesertMiningCamp => DESERT_SURFACE_ANCHORS,
            Self::DesertMiningCampUnderground => DESERT_UNDERGROUND_ANCHORS,
            Self::TaverleyLavaEels => DUSTY_KEY_ANCHORS,
        }
    }

    fn required_item(self) -> Option<i32> {
        match self {
            Self::DesertMiningCamp => Some(DESERT_KEY_ID),
            Self::TaverleyLavaEels => Some(DUSTY_KEY_ID),
            _ => None,
        }
    }

    fn required_worn(self) -> &'static [i32] {
        match self {
            Self::DesertMiningCampUnderground => DESERT_KIT_IDS,
            _ => &[],
        }
    }

    fn quest_varp(self) -> Option<(&'static str, i32)> {
        match self {
            Self::DesertMiningCamp | Self::DesertMiningCampUnderground => Some(("desertrescue", 30)),
            _ => None,
        }
    }

    fn matches_edge(self, edge: &TransportEdge) -> bool {
        self.loc_ids().contains(&edge.loc_id) && self.anchors().contains(&edge.at)
    }
}

#[derive(Debug, Clone, Serialize)]
struct TileProgress {
    tick: u32,
    tile: WorldTile,
}

#[derive(Debug, Clone, Serialize)]
struct TransportAttemptRecord {
    tick: u32,
    kind: String,
    expected_id: i32,
    actual_id: i32,
    target: WorldTile,
    option: i32,
    refusal: Option<String>,
    tile_at_attempt: Option<WorldTile>,
}

struct TravelRecord {
    outcome: Option<TravelOutcome>,
    start_tile: Option<WorldTile>,
    start_tick: u32,
    final_tile: Option<WorldTile>,
    progress: Vec<TileProgress>,
    attempts: Vec<TransportAttemptRecord>,
    events: Vec<String>,
    legs: Vec<String>,
}

#[derive(Debug, Clone)]
struct ResourceTarget {
    site_id: String,
    key: String,
    method: String,
    spot_id: u32,
    resource: WorldTile,
    stand: WorldTile,
    access: String,
    region: api::gather_methods::SceneRegionInput,
}

struct PlannedRoute {
    start: WorldTile,
    route: Route,
    edge: TransportEdge,
}

fn required_path(name: &str) -> PathBuf {
    let path = PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
    assert!(path.is_absolute(), "{name} must be an absolute path");
    path
}

fn live_profile_options() -> ProfileOptions {
    ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        cache_dir: Some(required_path("BOT_CACHE_DIR")),
        unpack_dir: Some(required_path("BOT_NAV_SNAPSHOT_ROOT")),
        engine_dir: Some(required_path("WORLD_ENGINE_DIR")),
        nav_pack: Some(required_path("WORLD_NAV_PACK")),
        ..ProfileOptions::default()
    }
}

fn pump_once(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) {
    client.mainloop();
    host::publish_snapshot(snapshot, client, pump.drain_client(client));
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    timeout: Duration,
    ready: impl Fn(&GameSnapshot) -> bool,
) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        pump_once(client, snapshot, pump);
        if ready(snapshot) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn inventory_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .inv()
        .iter()
        .filter(|&&(item_id, count)| item_id == id && count > 0)
        .map(|&(_, count)| count)
        .sum()
}

fn equipment_count(snapshot: &GameSnapshot, id: i32) -> i32 {
    snapshot
        .equipment()
        .iter()
        .filter(|item| item.def.id == id && item.count > 0)
        .map(|item| item.count)
        .sum()
}

fn equipment_rows(snapshot: &GameSnapshot) -> Vec<Value> {
    snapshot
        .equipment()
        .iter()
        .filter(|item| item.count > 0)
        .map(|item| {
            json!({
                "id": item.def.id,
                "name": item.def.name,
                "count": item.count,
                "slot": item.slot,
            })
        })
        .collect()
}

fn write_receipt(path: &Path, receipt: &Value) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create evidence directory");
    }
    fs::write(
        path,
        serde_json::to_vec_pretty(receipt).expect("serialize receipt"),
    )
    .expect("write receipt");
}

/// Save the actual CPU framebuffer as PNG with its matching snapshot.
fn save_frame(
    client: &mut Client,
    snapshot: &GameSnapshot,
    evidence: &Path,
    stem: &str,
    detail: Value,
) -> Result<(), String> {
    client.draw = true;
    let mut renderer = client::render::Renderer::new_prefer(client.config.lowmem, false);
    let output = renderer.mainredraw(client);
    let client::render::backend::FrameOutput::PixMap(pixels) = output else {
        client.draw = false;
        return Err("BOT_CPU=1 did not produce a CPU pixmap".into());
    };
    let mut rgb = Vec::with_capacity(pixels.pixels.len() * 3);
    for pixel in pixels.pixels {
        rgb.extend_from_slice(&[
            ((pixel >> 16) & 0xff) as u8,
            ((pixel >> 8) & 0xff) as u8,
            (pixel & 0xff) as u8,
        ]);
    }
    client.draw = false;
    let file = fs::File::create(evidence.join(format!("{stem}.png")))
        .map_err(|error| format!("create {stem}.png: {error}"))?;
    let mut encoder = png::Encoder::new(file, pixels.width as u32, pixels.height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
    writer
        .write_image_data(&rgb)
        .map_err(|error| error.to_string())?;
    let paired = json!({
        "ingame": snapshot.ingame(),
        "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(),
        "tick": snapshot.tick(),
        "tile": snapshot.tile(),
        "inventory": snapshot.inv(),
        "equipment": equipment_rows(snapshot),
        "stats": snapshot.stats(),
        "detail": detail,
    });
    fs::write(
        evidence.join(format!("{stem}.json")),
        serde_json::to_vec_pretty(&paired).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn logout_and_wait(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) -> bool {
    if !snapshot.ingame() {
        return true;
    }
    let ifaces = Arc::clone(&client.ifaces);
    interact::logout(client, &ifaces)
        && wait_for(client, snapshot, pump, LOGOUT_TIMEOUT, |s| !s.ingame())
}

fn evidence_stamp(snapshot: &GameSnapshot) -> EvidenceStamp {
    EvidenceStamp {
        run: QUEST_TEST_RUN,
        tick: u64::from(snapshot.tick()),
        sequence: 0,
    }
}

fn quest_evidence(
    quests: &api::quest_facts::QuestCatalog,
    catalog_identity: QuestFamilyIdentity,
    selected_identity: QuestFamilyIdentity,
    family: QuestFamilyId,
    snapshot: &GameSnapshot,
) -> QuestEvidence {
    let stamp = evidence_stamp(snapshot);
    let provider = Arc::new(QuestListEvidenceProvider::from_snapshot(
        quests,
        Some(catalog_identity),
        Some(selected_identity),
        SnapshotView::new(Some(snapshot), stamp),
    ));
    QuestEvidence::new(provider, family, stamp)
}

fn quest_status(
    quests: &api::quest_facts::QuestCatalog,
    quest_id: &str,
    snapshot: &GameSnapshot,
) -> Option<QuestListStatus> {
    let facts = quests.quest(quest_id).ok()?;
    quest_progress::quest_list_status(
        quests,
        &facts.id,
        SnapshotView::new(Some(snapshot), evidence_stamp(snapshot)),
    )
    .map(|observed| observed.value)
}

fn chat_choices(snapshot: &GameSnapshot) -> Vec<String> {
    snapshot
        .chat_options()
        .iter()
        .map(|choice| choice.text.clone())
        .collect()
}

fn choice_key(text: &str) -> String {
    text.trim()
        .trim_matches(|ch: char| matches!(ch, '.' | '!' | '?' | ':'))
        .to_ascii_lowercase()
}

/// Complete only the named quest through the account-local single-quest menu.
/// The helper never selects a choice that names all quests.
fn complete_quest_via_menu(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    quests: &api::quest_facts::QuestCatalog,
    quest_id: &str,
    display: &str,
) -> (bool, Vec<Value>) {
    let mut steps = Vec::new();
    if quest_status(quests, quest_id, snapshot) == Some(QuestListStatus::Complete) {
        return (true, steps);
    }
    if !matches!(
        quest_status(quests, quest_id, snapshot),
        Some(QuestListStatus::NotStarted | QuestListStatus::InProgress)
    ) {
        return (false, steps);
    }
    if !interact::cheat(client, "quest").is_sent() {
        steps.push(json!({"command":"quest", "sent":false}));
        return (false, steps);
    }
    steps.push(json!({"command":"quest", "sent":true}));

    let target = display.to_ascii_lowercase();
    let deadline = Instant::now() + SETUP_TIMEOUT;
    let mut selected_target = false;
    let mut waiting_for_change: Option<Vec<String>> = None;
    while Instant::now() < deadline {
        pump_once(client, snapshot, pump);
        if quest_status(quests, quest_id, snapshot) == Some(QuestListStatus::Complete) {
            return (true, steps);
        }
        let choices = chat_choices(snapshot);
        if waiting_for_change
            .as_ref()
            .is_some_and(|before| *before == choices)
        {
            std::thread::sleep(POLL_INTERVAL);
            continue;
        }
        waiting_for_change = None;

        let mut selected = None;
        for (index, text) in choices.iter().enumerate() {
            let key = choice_key(text);
            if key.contains("complete all") || key.contains("all quests") {
                continue;
            }
            if key.contains(&target) {
                selected = Some((index, text.as_str(), true));
                break;
            }
        }
        if selected.is_none() {
            for (index, text) in choices.iter().enumerate() {
                let key = choice_key(text);
                if key == "next" || key == "more" {
                    selected = Some((index, text.as_str(), false));
                    break;
                }
                if !selected_target
                    && (key.contains("select individual quest")
                        || key == "select quest"
                        || key == "choose quest")
                {
                    selected = Some((index, text.as_str(), false));
                    break;
                }
                if selected_target
                    && (key == "complete"
                        || key == "complete quest"
                        || key == "complete this quest"
                        || key == "yes")
                {
                    selected = Some((index, text.as_str(), false));
                    break;
                }
            }
        }
        let Some((index, text, target_selected_now)) = selected else {
            std::thread::sleep(POLL_INTERVAL);
            continue;
        };
        let before = choices;
        let sent = matches!(
            Interactions::new(snapshot, client).answer_choice(index as i32 + 1),
            SendResult::Sent { .. }
        );
        steps.push(json!({
            "choice": text,
            "option": index + 1,
            "sent": sent,
        }));
        if !sent {
            return (false, steps);
        }
        selected_target |= target_selected_now;
        waiting_for_change = Some(before);
    }
    (false, steps)
}

fn complete_entrance_quests(
    case: EntranceCase,
    edges: &[(usize, &TransportEdge)],
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    quests: &api::quest_facts::QuestCatalog,
) -> Result<Vec<Value>, String> {
    let mut quest_ids = HashSet::new();
    for (_, edge) in edges {
        if let Some(gates) = &edge.quest_gates {
            for gate in gates.gates() {
                if let QuestGate::Complete(quest) = gate {
                    quest_ids.insert(quest.0.to_string());
                }
            }
        }
    }
    let mut quest_ids: Vec<_> = quest_ids.into_iter().collect();
    quest_ids.sort();
    let mut completed = Vec::new();
    for quest_id in quest_ids {
        let facts = quests
            .quest(&quest_id)
            .map_err(|error| format!("{} names unknown quest {quest_id}: {error:?}", case.name()))?;
        let status_before = quest_status(quests, &quest_id, snapshot);
        let (success, steps) = complete_quest_via_menu(
            client,
            snapshot,
            pump,
            quests,
            &quest_id,
            &facts.display,
        );
        completed.push(json!({
            "id": quest_id,
            "display": facts.display,
            "status_before": format!("{status_before:?}"),
            "status_after": format!("{:?}", quest_status(quests, &facts.id.0, snapshot)),
            "steps": steps,
            "complete": success,
        }));
        if !success {
            return Err(format!("could not observe completion for quest {}", facts.display));
        }
    }
    Ok(completed)
}

fn selected_item(data: &game_data::SelectedGameData, id: i32) -> &GameItem {
    data.item_by_id(id)
        .unwrap_or_else(|| panic!("selected revision 289 data lacks object {id}"))
}

fn give_item(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    data: &game_data::SelectedGameData,
    id: i32,
    count: i32,
) {
    let item = selected_item(data, id);
    let alias = item
        .alias
        .as_deref()
        .unwrap_or_else(|| panic!("selected object {id} has no content alias"));
    assert!(
        interact::cheat(client, &format!("give {alias} {count}")).is_sent(),
        "give {alias} refused"
    );
    assert!(
        wait_for(client, snapshot, pump, SETUP_TIMEOUT, |s| {
            inventory_count(s, id) >= count
        }),
        "object {id} did not appear in inventory"
    );
}

fn wear_item(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump, id: i32) {
    assert!(
        matches!(
            Interactions::new(snapshot, client).wear(id),
            SendResult::Sent { .. }
        ),
        "normal Interactions::wear({id}) was refused"
    );
    assert!(
        wait_for(client, snapshot, pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.scene_state() == 2 && equipment_count(s, id) == 1
        }),
        "object {id} did not appear in equipment"
    );
}

fn set_quest_varp(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    name: &str,
    value: i32,
) -> bool {
    if !interact::cheat(client, &format!("setvar {name} {value}")).is_sent()
        || !interact::cheat(client, &format!("getvar {name}")).is_sent()
    {
        return false;
    }
    let expected = format!("get {name}: {value}").to_ascii_lowercase();
    wait_for(client, snapshot, pump, SETUP_TIMEOUT, |s| {
        s.chat_lines()
            .iter()
            .any(|line| line.text.to_ascii_lowercase().contains(&expected))
    })
}

fn seed_at(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    tile: WorldTile,
) -> bool {
    interact::seed_at(client, tile.level, tile.x, tile.z);
    wait_for(client, snapshot, pump, SETUP_TIMEOUT, |s| {
        s.ingame()
            && s.attached()
            && s.scene_state() == 2
            && ReadContext::new(s).world_tile() == Some(tile)
    })
}

fn center(region: api::gather_methods::SceneRegionInput) -> WorldTile {
    WorldTile {
        x: ((i64::from(region.min_x) + i64::from(region.max_x)) / 2) as i32,
        z: ((i64::from(region.min_z) + i64::from(region.max_z)) / 2) as i32,
        level: region.level,
    }
}

fn resource_stand(world: &NavWorld, spot: &GatherSpot) -> Option<WorldTile> {
    let width = i32::from(spot.width.max(1));
    let length = i32::from(spot.length.max(1));
    let mut best = world
        .collision
        .standable(spot.origin)
        .then_some((0, spot.origin));
    for dx in -1..=width {
        for dz in -1..=length {
            if dx != -1 && dx != width && dz != -1 && dz != length {
                continue;
            }
            let Some(x) = spot.origin.x.checked_add(dx) else {
                continue;
            };
            let Some(z) = spot.origin.z.checked_add(dz) else {
                continue;
            };
            let tile = WorldTile {
                x,
                z,
                level: spot.origin.level,
            };
            if !world.collision.standable(tile) {
                continue;
            }
            let score = (i64::from(dx) * i64::from(dx) + i64::from(dz) * i64::from(dz)) as u64;
            let improves = match best {
                Some((best_score, _)) => score < best_score,
                None => true,
            };
            if improves {
                best = Some((score, tile));
            }
        }
    }
    best.map(|(_, tile)| tile)
}

fn resource_target(
    data: &game_data::SelectedGameData,
    gathering: &GatherCatalog,
    world: &NavWorld,
    case: EntranceCase,
) -> Result<ResourceTarget, String> {
    let site = data
        .gather_site(case.skill(), case.site_id())
        .ok_or_else(|| format!("selected game data lacks site {}", case.site_id()))?;
    let from = center(site.region);
    for key in &site.keys {
        for method in gathering.methods_for_resource(&key.key) {
            let expected_skill = match case.skill() {
                "fishing" => GatherSkill::Fishing,
                _ => GatherSkill::Mining,
            };
            if method.skill != expected_skill {
                continue;
            }
            let nearest = gathering
                .nearest(method, from, Some(&site.region), AccessPolicy::Possible, 1)
                .map_err(|error| format!("nearest {} resource: {error:?}", key.key))?;
            let Some(nearest) = nearest.first() else {
                continue;
            };
            let stand = resource_stand(world, nearest.spot).ok_or_else(|| {
                format!("site {} resource {} has no adjacent standable tile", site.id, key.key)
            })?;
            return Ok(ResourceTarget {
                site_id: site.id.clone(),
                key: key.key.clone(),
                method: method.id.0.to_string(),
                spot_id: nearest.spot.id.0,
                resource: nearest.spot.origin,
                stand,
                access: format!("{:?}", nearest.access),
                region: site.region,
            });
        }
    }
    Err(format!("site {} has no selected resource placement", site.id))
}

fn matching_edges<'a>(world: &'a NavWorld, case: EntranceCase) -> Vec<(usize, &'a TransportEdge)> {
    world
        .graph
        .edges
        .iter()
        .enumerate()
        .filter(|(_, edge)| case.matches_edge(edge))
        .collect()
}

fn edge_value(edge: &TransportEdge) -> Value {
    let quest_gates = edge.quest_gates.as_ref().map(|gates| {
        json!({
            "family": gates.family().to_string(),
            "gates": gates.gates().iter().map(|gate| format!("{gate:?}")).collect::<Vec<_>>(),
        })
    });
    json!({
        "kind": format!("{:?}", edge.kind),
        "at": edge.at,
        "to": edge.to,
        "loc_id": edge.loc_id,
        "option": edge.option,
        "item_req": edge.item_req,
        "worn_req": edge.worn_req,
        "worn_all_req": edge.worn_all_req,
        "forbidden_held": edge.forbidden_held,
        "forbidden_worn": edge.forbidden_worn,
        "quest_req": edge.quest_req,
        "varp_req": edge.varp_req,
        "quest_gates": quest_gates,
    })
}

fn find_route(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with(
        &world.collision,
        &world.graph,
        from,
        to,
        FindOptions {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        state,
    )
}

fn planned_edge(route: &Route, case: EntranceCase) -> Option<TransportEdge> {
    route.legs.iter().find_map(|leg| match leg {
        Leg::Transport { edge } if case.matches_edge(edge) => Some(edge.as_ref().clone()),
        _ => None,
    })
}

fn candidate_starts(world: &NavWorld, case: EntranceCase) -> Vec<WorldTile> {
    let mut starts = Vec::new();
    let mut seen = HashSet::new();
    for (_, edge) in matching_edges(world, case) {
        if let Some(takeoff) = edge.takeoff {
            if world.collision.standable(takeoff) && seen.insert(takeoff) {
                starts.push(takeoff);
            }
        }
        for &(dx, dz) in NEIGHBOR_OFFSETS {
            let Some(x) = edge.at.x.checked_add(dx) else {
                continue;
            };
            let Some(z) = edge.at.z.checked_add(dz) else {
                continue;
            };
            let tile = WorldTile {
                x,
                z,
                level: edge.at.level,
            };
            if tile != edge.to && world.collision.standable(tile) && seen.insert(tile) {
                starts.push(tile);
            }
        }
    }
    starts
}

fn plan_route(
    world: &NavWorld,
    case: EntranceCase,
    target: WorldTile,
    state: &WorldState,
) -> Result<PlannedRoute, String> {
    let starts = candidate_starts(world, case);
    if starts.is_empty() {
        return Err(format!("{} has no standable entrance approach", case.name()));
    }
    let mut failures = Vec::new();
    for start in starts {
        match find_route(world, start, target, state) {
            Ok(route) => {
                if let Some(edge) = planned_edge(&route, case) {
                    return Ok(PlannedRoute { start, route, edge });
                }
                failures.push(format!("{start:?}: route omitted the declared entrance"));
            }
            Err(error) => failures.push(format!("{start:?}: {error:?}")),
        }
    }
    Err(format!("no route uses the declared entrance: {failures:?}"))
}

fn state_from_snapshot(
    snapshot: &GameSnapshot,
    map_members: bool,
    evidence: QuestEvidence,
) -> WorldState {
    WorldState::from_snapshot(snapshot)
        .with_map_members(map_members)
        .with_quest_evidence(evidence)
}

fn drive_route(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    world: &NavWorld,
    route: &Route,
    quest_evidence: Option<&QuestEvidence>,
) -> TravelRecord {
    let start_tile = ReadContext::new(snapshot).world_tile();
    let start_tick = snapshot.tick();
    let observed_tile = Cell::new(start_tile);
    let mut progress = Vec::new();
    let mut attempts = Vec::new();
    let mut events = Vec::new();
    let mut legs = Vec::new();
    if let Some(tile) = start_tile {
        progress.push(TileProgress {
            tick: start_tick,
            tile,
        });
    }
    let mut traveller = Traveller::new();
    let mut options = TravelOptions {
        close_enough: 0,
        edges: Some(&world.graph.edges),
        quest_evidence,
        on_event: Some(Box::new(|event| {
            if let TravelEvent::TransportAttempt {
                tick,
                kind,
                expected_id,
                actual_id,
                target,
                option,
                refusal,
            } = &event
            {
                attempts.push(TransportAttemptRecord {
                    tick: *tick,
                    kind: format!("{kind:?}"),
                    expected_id: *expected_id,
                    actual_id: *actual_id,
                    target: *target,
                    option: *option,
                    refusal: (*refusal).map(|reason| format!("{reason:?}")),
                    tile_at_attempt: observed_tile.get(),
                });
            }
            events.push(format!("{event:?}"));
        })),
        on_leg: Some(Box::new(|leg, phase| {
            legs.push(format!("{phase:?}: {leg:?}"));
        })),
        ..TravelOptions::default()
    };
    let deadline = Instant::now() + TRAVEL_TIMEOUT;
    let mut last_tick = None;
    let mut last_tile = start_tile;
    let mut outcome = None;
    while Instant::now() < deadline {
        pump_once(client, snapshot, pump);
        let here = ReadContext::new(snapshot).world_tile();
        observed_tile.set(here);
        if here != last_tile {
            if let Some(tile) = here {
                progress.push(TileProgress {
                    tick: snapshot.tick(),
                    tile,
                });
            }
            last_tile = here;
        }
        if last_tick != Some(snapshot.tick()) {
            last_tick = Some(snapshot.tick());
            if let Some(terminal) =
                traveller.follow(client, snapshot, route.clone(), &mut options)
            {
                outcome = Some(terminal);
                break;
            }
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    drop(options);
    TravelRecord {
        outcome,
        start_tile,
        start_tick,
        final_tile: ReadContext::new(snapshot).world_tile(),
        progress,
        attempts,
        events,
        legs,
    }
}

fn travel_value(record: &TravelRecord) -> Value {
    json!({
        "outcome": format!("{:?}", record.outcome),
        "start_tile": record.start_tile,
        "start_tick": record.start_tick,
        "final_tile": record.final_tile,
        "tile_progress": record.progress,
        "transport_attempts": record.attempts,
        "events": record.events,
        "legs": record.legs,
    })
}

fn entrance_leg_index(route: &Route, case: EntranceCase) -> Option<usize> {
    route.legs.iter().position(|leg| {
        matches!(leg, Leg::Transport { edge } if case.matches_edge(edge))
    })
}

fn route_moves_beyond_entrance(route: &Route, case: EntranceCase) -> bool {
    let Some(entrance_index) = entrance_leg_index(route, case) else {
        return false;
    };
    route
        .legs
        .iter()
        .enumerate()
        .skip(entrance_index + 1)
        .any(|(_, leg)| matches!(leg, Leg::Walk { tiles } if tiles.len() > 1))
}

fn run_case(case: EntranceCase) {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_NAV_BUILD").as_deref(), Ok("skip"));
    assert!(std::env::var_os("HOME").is_some(), "set a disposable HOME");
    let prefix = std::env::var("BOT_LIVE_NAME_PREFIX").expect("BOT_LIVE_NAME_PREFIX");
    assert_eq!(prefix, "ngte", "use this cell's account namespace");

    let evidence_root = required_path("NAV_GATED_ENTRANCES_EVIDENCE");
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock")
        .as_millis();
    let timestamp = format!("{serial}Z");
    let (account, password) = mint_live_entries(&mint_live_names(1))
        .into_iter()
        .next()
        .expect("mint one owned live account");
    let evidence = evidence_root.join(format!(
        "NAV-GATED-ENTRANCES_{}_{}_{}",
        case.name(),
        account,
        timestamp
    ));
    fs::create_dir_all(&evidence).expect("create per-account evidence directory");
    let receipt_path = evidence.join(format!("{timestamp}_receipt.json"));
    let mut receipt = json!({
        "scenario": case.name(),
        "site_id": case.site_id(),
        "account": account,
        "server": {"host":"127.0.0.1", "game_port":44594, "http_port":1080},
        "profile": "local-289",
        "pack_format": nav::pack::FORMAT_ID,
        "engine_loaded": false,
        "fresh_account_baseline": false,
        "entrance_edges": [],
        "selected_resource": null,
        "precondition_no_path": false,
        "planned_route": null,
        "start_capture": null,
        "entrance_refusals": [],
        "travel": null,
        "resource_arrival": false,
        "fresh_tile_progress": false,
        "arrival_scene_ready": false,
        "end_capture": null,
        "logout_observed": false,
    });
    write_receipt(&receipt_path, &receipt);

    let profile = live_profile_options()
        .resolve(None)
        .expect("resolve Engine A R289 profile")
        .bind()
        .expect("bind Engine A R289 profile");
    let template = SharedClientTemplate::load(Arc::clone(&profile)).expect("client template");
    let world = template.world().expect("explicit WORLD_NAV_PACK");
    assert_eq!(nav::pack::FORMAT_ID, "274V17", "acceptance requires pack v17");
    let identity = profile.nav_identity().expect("navigation identity");
    receipt["nav_identity"] = json!({
        "pack_format": nav::pack::FORMAT_ID,
        "nav_sha256": identity.nav_sha256,
    });

    let selected = game_data::for_revision(ClientRevision::R289).expect("selected R289 game data");
    let clock = SystemClock;
    let cancellation = SharedCancellation::new();
    let budget = SliceBudget::new(
        std::num::NonZeroU32::new(1).expect("nonzero preparation budget"),
        Deadline::at(Instant::now() + SETUP_TIMEOUT),
    );
    let mut preparation = StepContext::<PreparationPhase>::new(&clock, budget, &cancellation);
    let (quests, gathering) = api::selected::FamilyPreparation::within(&mut preparation, |worker| {
        (
            selected.prepare_quests(worker),
            selected.prepare_gathering(worker),
        )
    });
    let quests = quests.expect("verified R289 quest identity catalog");
    let gathering = gathering.expect("verified R289 gathering catalog");
    let family_identity = selected
        .quest_family_identity()
        .expect("verified quest manifest family identity");
    let family = QuestFamilyId::new(family_identity.sha256, family_identity.schema);
    world
        .graph
        .admit_quest_family(&family)
        .expect("nav pack quest gates match verified manifest family");
    let edges = matching_edges(&world, case);
    assert!(!edges.is_empty(), "pack lacks expected {} entrance edge", case.name());
    for (_, edge) in &edges {
        assert!(edge.quest_req.is_empty(), "new entrance must not use quest_req: {edge:?}");
        assert!(edge.varp_req.is_empty(), "new entrance must not use raw varps: {edge:?}");
        if let Some(gates) = &edge.quest_gates {
            assert_eq!(gates.family(), &family, "entrance gate family mismatch");
        }
    }
    receipt["engine_loaded"] = json!(true);
    receipt["nav_identity"]["quest_family"] = json!(family.to_string());
    receipt["entrance_edges"] = json!(edges
        .iter()
        .map(|(_, edge)| edge_value(edge))
        .collect::<Vec<_>>());
    write_receipt(&receipt_path, &receipt);

    let target = resource_target(&selected, &gathering, &world, case)
        .unwrap_or_else(|error| panic!("select resource destination: {error}"));
    assert!(
        (target.region.min_x..=target.region.max_x).contains(&target.resource.x)
            && (target.region.min_z..=target.region.max_z).contains(&target.resource.z)
            && target.region.level == target.resource.level,
        "selected resource is outside its published site region"
    );
    receipt["selected_resource"] = json!({
        "site_id": target.site_id,
        "key": target.key,
        "method": target.method,
        "spot_id": target.spot_id,
        "resource_tile": target.resource,
        "stand_tile": target.stand,
        "access": target.access,
        "region": target.region,
    });
    write_receipt(&receipt_path, &receipt);

    let mut client = template
        .prepare_client((serial % i32::MAX as u128) as i32, true)
        .expect("prepare CPU client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(interact::login(&mut client, &account, &password, false), "local login refused");
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }),
        "initial account scene did not become ready: {:?}",
        snapshot.tile()
    );

    interact::mainland_hop(&mut client);
    assert!(interact::cheat(&mut client, "getvar tutorial").is_sent());
    assert!(wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
        s.chat_lines()
            .iter()
            .any(|line| line.text.contains("get tutorial: 1000"))
    }), "tutorial skip was not acknowledged");
    assert!(logout_and_wait(&mut client, &mut snapshot, &mut pump), "tutorial relog-out failed");
    assert!(interact::login(&mut client, &account, &password, false), "relogin refused");
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.ingame() && s.attached() && s.scene_state() == 2
        }),
        "post-tutorial account scene did not become ready"
    );

    let initial_item_ids = [
        ANA_BARREL_ID,
        DESERT_KEY_ID,
        DUSTY_KEY_ID,
        DESERT_KIT_IDS[0],
        DESERT_KIT_IDS[1],
        DESERT_KIT_IDS[2],
    ];
    let fresh_account_baseline = initial_item_ids
        .iter()
        .all(|&id| inventory_count(&snapshot, id) == 0 && equipment_count(&snapshot, id) == 0);
    receipt["fresh_account_baseline"] = json!(fresh_account_baseline);
    receipt["baseline"] = json!({
        "tile": snapshot.tile(),
        "inventory": snapshot.inv(),
        "equipment": equipment_rows(&snapshot),
        "quest_statuses_available": snapshot.quest_statuses_available(),
        "scene_state": snapshot.scene_state(),
    });
    write_receipt(&receipt_path, &receipt);
    assert!(fresh_account_baseline, "account was not fresh before seeding: {receipt}");
    assert!(snapshot.quest_statuses_available(), "quest-list observation is unavailable");

    let pre_quest_evidence = quest_evidence(
        &quests,
        family_identity,
        selected.quest_family_identity().expect("selected quest family"),
        family,
        &snapshot,
    );
    if case == EntranceCase::GrandTreeMine {
        let grand_tree = quests
            .by_display("The Grand Tree")
            .expect("R289 quest catalog contains The Grand Tree");
        let gate = edges
            .iter()
            .flat_map(|(_, edge)| edge.quest_gates.iter())
            .flat_map(|gates| gates.gates())
            .find(|gate| matches!(gate, QuestGate::Complete(id) if id == &grand_tree.id))
            .expect("Grand Tree entrance has a typed completion gate");
        let verdict = pre_quest_evidence.test(gate);
        receipt["pre_completion_gate"] = json!({
            "quest": grand_tree.id.0,
            "display": grand_tree.display,
            "verdict": format!("{verdict:?}"),
        });
        write_receipt(&receipt_path, &receipt);
        assert_eq!(verdict, Truth::False, "fresh account did not disprove Grand Tree completion");
    }

    let quest_menu = complete_entrance_quests(
        case,
        &edges,
        &mut client,
        &mut snapshot,
        &mut pump,
        &quests,
    )
    .unwrap_or_else(|error| panic!("prepare entrance quest state: {error}; receipt={receipt}"));
    receipt["quest_menu"] = json!(quest_menu);
    write_receipt(&receipt_path, &receipt);

    let quest_varp = if let Some((name, value)) = case.quest_varp() {
        let observed = set_quest_varp(&mut client, &mut snapshot, &mut pump, name, value);
        receipt["quest_varp"] = json!({"name":name, "value":value, "observed":observed});
        write_receipt(&receipt_path, &receipt);
        assert!(observed, "account-local {name}={value} quest varp was not observed");
        Some(json!({"name":name,"value":value}))
    } else {
        None
    };

    if let Some(id) = case.required_item() {
        give_item(&mut client, &mut snapshot, &mut pump, &selected, id, 1);
    }
    for &id in case.required_worn() {
        let item = selected_item(&selected, id);
        assert!(item.worn_model && item.wear_position >= 0, "kit object {id} is not wearable");
        give_item(&mut client, &mut snapshot, &mut pump, &selected, id, 1);
        wear_item(&mut client, &mut snapshot, &mut pump, id);
    }
    receipt["seeded_items"] = json!({
        "required_item": case.required_item(),
        "required_worn": case.required_worn(),
        "quest_varp": quest_varp,
        "inventory": snapshot.inv(),
        "equipment": equipment_rows(&snapshot),
    });
    write_receipt(&receipt_path, &receipt);

    let positive_evidence = quest_evidence(
        &quests,
        family_identity,
        selected.quest_family_identity().expect("selected quest family"),
        family,
        &snapshot,
    );
    for (_, edge) in &edges {
        if let Some(gates) = &edge.quest_gates {
            for gate in gates.gates() {
                if matches!(gate, QuestGate::Window(_)) {
                    assert_eq!(
                        positive_evidence.test(gate),
                        Truth::Unknown,
                        "the identity-only quest-list provider must not prove stage windows"
                    );
                }
            }
        }
    }
    let state = state_from_snapshot(&snapshot, profile.map_members(), positive_evidence.clone());
    let planned = plan_route(&world, case, target.stand, &state)
        .unwrap_or_else(|error| panic!("route planning failed: {error}; receipt={receipt}"));
    let provisional_route = json!({
        "start": planned.start,
        "target_resource": target.resource,
        "target_stand": target.stand,
        "edge": edge_value(&planned.edge),
        "legs": planned.route.legs.iter().map(|leg| format!("{leg:?}")).collect::<Vec<_>>(),
    });
    receipt["planned_route"] = json!({"provisional":provisional_route});
    write_receipt(&receipt_path, &receipt);

    assert!(
        seed_at(&mut client, &mut snapshot, &mut pump, planned.start),
        "could not place account at the selected approach tile {:?}",
        planned.start
    );
    let final_evidence = quest_evidence(
        &quests,
        family_identity,
        selected.quest_family_identity().expect("selected quest family"),
        family,
        &snapshot,
    );
    let final_state = state_from_snapshot(&snapshot, profile.map_members(), final_evidence.clone());
    let route = find_route(&world, planned.start, target.stand, &final_state)
        .unwrap_or_else(|error| panic!("final live-state route failed: {error:?}; receipt={receipt}"));
    let edge = planned_edge(&route, case).expect("final route uses the declared entrance");
    assert!(edge.quest_req.is_empty(), "planned entrance contains raw quest requirement");
    assert!(edge.varp_req.is_empty(), "planned entrance contains raw varp requirement");
    if let Some(gates) = &edge.quest_gates {
        assert_eq!(gates.family(), &family, "planned entrance uses another quest family");
        for gate in gates.gates() {
            if matches!(gate, QuestGate::Window(_)) {
                assert_eq!(final_evidence.test(gate), Truth::Unknown);
            }
        }
    }

    if let Some(required_item) = case.required_item() {
        assert!(
            edge.item_req.iter().any(|&(id, count)| id == required_item && count > 0),
            "planned entrance does not require selected item {required_item}: {edge:?}"
        );
    }
    if case == EntranceCase::DesertMiningCampUnderground {
        let mut expected = DESERT_KIT_IDS.to_vec();
        expected.sort_unstable();
        let mut actual = edge.worn_all_req.clone();
        actual.sort_unstable();
        assert_eq!(actual, expected, "underground entrance must require all three worn kit pieces");
        assert!(route_moves_beyond_entrance(&route, case), "lower mine resource route must continue beyond the entrance");
    }
    if case == EntranceCase::TaverleyLavaEels {
        assert_eq!(edge.option, 0, "dusty-key door must use its content item-on-loc option");
    }
    if case == EntranceCase::DesertMiningCamp {
        assert!(edge.forbidden_held.contains(&ANA_BARREL_ID), "camp gate must exclude Ana's barrel");
        assert!(!edge.forbidden_worn.is_empty(), "camp gate must carry its equipment exclusions");
    }

    let missing_result = if case == EntranceCase::GrandTreeMine {
        let mut missing = final_state.clone().with_quest_evidence(pre_quest_evidence.clone());
        missing.quests.remove("The Grand Tree");
        find_route(&world, planned.start, target.stand, &missing)
    } else if case == EntranceCase::DesertMiningCampUnderground {
        let mut missing = final_state.clone();
        for id in DESERT_KIT_IDS {
            missing.worn.remove(id);
        }
        find_route(&world, planned.start, target.stand, &missing)
    } else if let Some(id) = case.required_item() {
        let mut missing = final_state.clone();
        missing.inv.remove(&id);
        find_route(&world, planned.start, target.stand, &missing)
    } else {
        Err(RouteError::NoPath)
    };
    let precondition_no_path = matches!(missing_result, Err(RouteError::NoPath));
    receipt["precondition_probe"] = json!({
        "missing": if case == EntranceCase::GrandTreeMine { "Grand Tree completion" }
            else if case == EntranceCase::DesertMiningCampUnderground { "all three worn kit pieces" }
            else { "required held key" },
        "result": format!("{missing_result:?}"),
        "no_path": precondition_no_path,
    });
    receipt["precondition_no_path"] = json!(precondition_no_path);
    write_receipt(&receipt_path, &receipt);
    assert!(precondition_no_path, "missing entrance prerequisite still produced a route: {receipt}");

    let start_stem = format!("{timestamp}_00-start");
    let end_stem = format!("{timestamp}_01-end");
    let failure_stem = format!("{timestamp}_FAIL-route-proof");
    let start_capture = save_frame(
        &mut client,
        &snapshot,
        &evidence,
        &start_stem,
        json!({"phase":"resource-route-start", "site_id":target.site_id, "resource":target.resource, "stand":target.stand}),
    );
    receipt["start_capture"] = json!(start_capture
        .as_ref()
        .map(|_| format!("{start_stem}.png"))
        .map_err(Clone::clone));
    write_receipt(&receipt_path, &receipt);

    let mut exclusion_runs = Vec::new();
    if case == EntranceCase::DesertMiningCamp {
        let held_item = selected_item(&selected, ANA_BARREL_ID);
        assert!(held_item.alias.is_some(), "Ana's barrel has no selected content alias");
        give_item(&mut client, &mut snapshot, &mut pump, &selected, ANA_BARREL_ID, 1);
        let held_run = drive_route(
            &mut client,
            &mut snapshot,
            &mut pump,
            &world,
            &route,
            Some(&final_evidence),
        );
        let held_expected = matches!(
            &held_run.outcome,
            Some(TravelOutcome::EntranceRefused {
                at,
                requirement: nav::world_state::EntranceRequirement::ForbiddenHeld { id, .. },
                ..
            }) if *at == edge.at && *id == ANA_BARREL_ID
        );
        exclusion_runs.push(json!({"kind":"forbidden-held", "expected":held_expected, "travel":travel_value(&held_run)}));
        receipt["entrance_refusals"] = json!(exclusion_runs);
        write_receipt(&receipt_path, &receipt);
        let held_capture = save_frame(
            &mut client,
            &snapshot,
            &evidence,
            &format!("{timestamp}_held-refusal"),
            json!({"phase":"held-exclusion-refused", "item_id":ANA_BARREL_ID, "outcome":format!("{:?}", held_run.outcome)}),
        );
        receipt["held_refusal_capture"] = json!(held_capture
            .as_ref()
            .map(|_| format!("{timestamp}_held-refusal.png"))
            .map_err(Clone::clone));
        assert!(held_expected, "Traveller did not return the typed held-item refusal: {receipt}");
        assert!(interact::cheat(&mut client, "clearinv").is_sent(), "clearinv refused");
        assert!(wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            inventory_count(s, ANA_BARREL_ID) == 0 && inventory_count(s, DESERT_KEY_ID) == 0
        }), "held exclusion item did not clear");
        give_item(&mut client, &mut snapshot, &mut pump, &selected, DESERT_KEY_ID, 1);
        assert!(seed_at(&mut client, &mut snapshot, &mut pump, planned.start), "could not reset to camp approach");

        let forbidden_worn = edge
            .forbidden_worn
            .iter()
            .filter_map(|&id| selected.item_by_id(id))
            .find(|item| item.worn_model && item.wear_position >= 0 && item.alias.is_some())
            .expect("camp edge has a selected wearable forbidden object");
        let forbidden_id = forbidden_worn.id;
        give_item(&mut client, &mut snapshot, &mut pump, &selected, forbidden_id, 1);
        wear_item(&mut client, &mut snapshot, &mut pump, forbidden_id);
        let worn_run = drive_route(
            &mut client,
            &mut snapshot,
            &mut pump,
            &world,
            &route,
            Some(&final_evidence),
        );
        let worn_expected = matches!(
            &worn_run.outcome,
            Some(TravelOutcome::EntranceRefused {
                at,
                requirement: nav::world_state::EntranceRequirement::ForbiddenWorn { id, .. },
                ..
            }) if *at == edge.at && *id == forbidden_id
        );
        exclusion_runs.push(json!({"kind":"forbidden-worn", "item_id":forbidden_id, "expected":worn_expected, "travel":travel_value(&worn_run)}));
        receipt["entrance_refusals"] = json!(exclusion_runs);
        write_receipt(&receipt_path, &receipt);
        let worn_capture = save_frame(
            &mut client,
            &snapshot,
            &evidence,
            &format!("{timestamp}_worn-refusal"),
            json!({"phase":"worn-exclusion-refused", "item_id":forbidden_id, "outcome":format!("{:?}", worn_run.outcome)}),
        );
        receipt["worn_refusal_capture"] = json!(worn_capture
            .as_ref()
            .map(|_| format!("{timestamp}_worn-refusal.png"))
            .map_err(Clone::clone));
        assert!(worn_expected, "Traveller did not return the typed worn-item refusal: {receipt}");
        assert!(
            matches!(
                Interactions::new(&snapshot, &mut client).unequip(forbidden_id),
                SendResult::Sent { .. }
            ),
            "normal unequip({forbidden_id}) was refused"
        );
        assert!(wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            equipment_count(s, forbidden_id) == 0
        }), "forbidden worn object did not leave equipment");
        assert!(interact::cheat(&mut client, "clearinv").is_sent(), "clearinv refused");
        assert!(wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            inventory_count(s, DESERT_KEY_ID) == 0
        }), "camp key did not clear");
        give_item(&mut client, &mut snapshot, &mut pump, &selected, DESERT_KEY_ID, 1);
        assert!(seed_at(&mut client, &mut snapshot, &mut pump, planned.start), "could not reset to camp approach");
    }

    let travel = drive_route(
        &mut client,
        &mut snapshot,
        &mut pump,
        &world,
        &route,
        Some(&final_evidence),
    );
    let resource_arrival = matches!(
        &travel.outcome,
        Some(TravelOutcome::Arrived { at }) if *at == target.stand
    );
    let fresh_tile_progress = travel
        .progress
        .iter()
        .any(|step| step.tick > travel.start_tick && Some(step.tile) != travel.start_tile);
    let entrance_attempt = travel.attempts.iter().any(|attempt| {
        attempt.expected_id == edge.loc_id
            && attempt.actual_id == edge.loc_id
            && attempt.target == edge.at
            && attempt.refusal.is_none()
    });
    let entrance_crossing = travel.attempts.iter().any(|attempt| {
        attempt.expected_id == edge.loc_id
            && attempt.actual_id == edge.loc_id
            && attempt.target == edge.at
            && attempt.refusal.is_none()
            && travel
                .progress
                .iter()
                .any(|step| step.tick > attempt.tick && step.tile == edge.to)
    });
    let arrival_scene_ready = snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2;
    receipt["planned_route"] = json!({
        "start": planned.start,
        "target_resource": target.resource,
        "target_stand": target.stand,
        "edge": edge_value(&edge),
        "legs": route.legs.iter().map(|leg| format!("{leg:?}")).collect::<Vec<_>>(),
        "moves_beyond_entrance": route_moves_beyond_entrance(&route, case),
    });
    receipt["entrance_refusals"] = json!(exclusion_runs);
    receipt["travel"] = travel_value(&travel);
    receipt["resource_arrival"] = json!(resource_arrival);
    receipt["entrance_attempt"] = json!(entrance_attempt);
    receipt["entrance_crossing"] = json!(entrance_crossing);
    receipt["fresh_tile_progress"] = json!(fresh_tile_progress);
    receipt["arrival_scene_ready"] = json!(arrival_scene_ready);
    write_receipt(&receipt_path, &receipt);

    let end_capture = save_frame(
        &mut client,
        &snapshot,
        &evidence,
        &end_stem,
        json!({
            "phase":"resource-route-end-before-logout",
            "site_id":target.site_id,
            "resource":target.resource,
            "stand":target.stand,
            "outcome":format!("{:?}", travel.outcome),
            "entrance_crossing":entrance_crossing,
        }),
    );
    receipt["end_capture"] = json!(end_capture
        .as_ref()
        .map(|_| format!("{end_stem}.png"))
        .map_err(Clone::clone));
    if !resource_arrival || !entrance_crossing || !fresh_tile_progress {
        let failure_capture = save_frame(
            &mut client,
            &snapshot,
            &evidence,
            &failure_stem,
            json!({
                "phase":"failed-resource-route-proof",
                "site_id":target.site_id,
                "resource":target.resource,
                "stand":target.stand,
                "outcome":format!("{:?}", travel.outcome),
            }),
        );
        receipt["failure_capture"] = json!(failure_capture
            .as_ref()
            .map(|_| format!("{failure_stem}.png"))
            .map_err(Clone::clone));
    }
    write_receipt(&receipt_path, &receipt);

    let logout_observed = logout_and_wait(&mut client, &mut snapshot, &mut pump);
    if !logout_observed {
        client.logout();
    }
    receipt["logout_observed"] = json!(logout_observed);
    receipt["post_logout"] = json!({
        "ingame":snapshot.ingame(),
        "scene_state":snapshot.scene_state(),
        "tile":snapshot.tile(),
    });
    write_receipt(&receipt_path, &receipt);
    println!("{}", serde_json::to_string_pretty(&receipt).expect("print receipt"));

    assert!(resource_arrival, "Traveller did not reach the selected resource stand: {receipt}");
    assert!(entrance_attempt, "Traveller did not send the selected entrance interaction: {receipt}");
    assert!(entrance_crossing, "no server-settled progress across the selected entrance: {receipt}");
    assert!(fresh_tile_progress, "Traveller produced no fresh tile progress: {receipt}");
    assert!(arrival_scene_ready, "resource arrival scene was not ready: {receipt}");
    assert!(start_capture.is_ok(), "start capture was not preserved: {receipt}");
    assert!(end_capture.is_ok(), "end capture was not preserved: {receipt}");
    assert!(logout_observed, "successful explicit logout was not observed: {receipt}");
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, BOT_LIVE_NAME_PREFIX=ngte, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_grand_tree_mine_requires_quest_completion_and_reaches_resources() {
    run_case(EntranceCase::GrandTreeMine);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, BOT_LIVE_NAME_PREFIX=ngte, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_desert_mining_camp_requires_metal_key_and_refuses_excluded_items() {
    run_case(EntranceCase::DesertMiningCamp);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, BOT_LIVE_NAME_PREFIX=ngte, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_desert_mining_camp_lower_mine_requires_worn_slave_kit() {
    run_case(EntranceCase::DesertMiningCampUnderground);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, BOT_LIVE_NAME_PREFIX=ngte, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_taverley_lava_eels_requires_dusty_key() {
    run_case(EntranceCase::TaverleyLavaEels);
}
