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
use nav_executor::{TravelEvent, TravelOptions, TravelOutcome, Traveller};
use nav_planner::quest_gates::{QuestEvidence, QuestFamilyId};
use nav_planner::router::{
    find_many_with, find_with, FindOptions, Leg, Route, RouteError, TargetError,
};
use nav_planner::transport::TransportEdge;
use nav_planner::world::NavWorld;
use nav_planner::zones::ZoneExempt;
use nav_planner::WorldState;
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
const TAVERLEY_CROSS_ZONES: &[&str] = &[
    "lesser_demon@2936,9793,0",
    "lesser_demon@2931,9798,0",
    "blue_dragon@2899,9802,0",
    "blue_dragon@2904,9802,0",
    "lesser_demon@2926,9802,0",
    "lesser_demon@2931,9807,0",
    "lesser_demon@2932,9810,0",
    "blue_dragon@2897,9797,0",
];
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

    fn cross_zones(self) -> &'static [&'static str] {
        match self {
            Self::TaverleyLavaEels => TAVERLEY_CROSS_ZONES,
            _ => &[],
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
            // general/configs/quest.constant:16,28. The account-local cheat
            // is followed by a relog and an actual quest-list observation.
            Self::GrandTreeMine => Some(("grandtree", 160)),
            Self::DesertMiningCamp | Self::DesertMiningCampUnderground => {
                Some(("desertrescue", 30))
            }
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

#[derive(Debug, PartialEq, Eq)]
struct RouteProof {
    resource_arrival: bool,
    fresh_tile_progress: bool,
    entrance_attempt: bool,
    entrance_crossing: bool,
}

fn route_proof(
    travel: &TravelRecord,
    stand: WorldTile,
    loc_id: i32,
    entrance_at: WorldTile,
    entrance_to: WorldTile,
) -> RouteProof {
    let accepted = |attempt: &TransportAttemptRecord| {
        attempt.expected_id == loc_id
            && attempt.actual_id == loc_id
            && attempt.target == entrance_at
            && attempt.refusal.is_none()
    };
    RouteProof {
        resource_arrival: matches!(
            &travel.outcome,
            Some(TravelOutcome::Arrived { at }) if *at == stand
        ),
        fresh_tile_progress: travel
            .progress
            .iter()
            .any(|step| step.tick > travel.start_tick && Some(step.tile) != travel.start_tile),
        entrance_attempt: travel.attempts.iter().any(&accepted),
        entrance_crossing: travel.attempts.iter().any(|attempt| {
            accepted(attempt)
                && travel
                    .progress
                    .iter()
                    .any(|step| step.tick > attempt.tick && step.tile == entrance_to)
        }),
    }
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
    let path =
        PathBuf::from(std::env::var_os(name).unwrap_or_else(|| panic!("{name} is required")));
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

/// Client logout does not prove that the engine released account ownership.
/// World.ts:795-804 retries persistence after 15 s. As in play_login.rs,
/// allow 30 s for the initial save and one retry, probing every 5 s.
/// Only response 5 (pending logout or active player, World.ts:813-867)
/// permits another attempt. Successful login is the readiness signal.
fn login_after_logout(
    client: &mut Client,
    credentials: (&str, &str),
    phase: &str,
    logout_tick: u32,
    receipt: &mut Value,
    receipt_path: &Path,
) {
    let started = Instant::now();
    let budget = Duration::from_secs(30);
    let cadence = Duration::from_secs(5);
    let mut attempts = Vec::new();
    loop {
        let result = client.login(credentials.0, credentials.1, false);
        attempts.push(json!({
            "attempt": attempts.len() + 1,
            "elapsed_ms": started.elapsed().as_millis(),
            "logout_tick": logout_tick,
            "accepted": result.is_ok(),
            "response_code": result.as_ref().err().map(|error| error.code),
            "message": format!("{} {}", client.login_mes1, client.login_mes2),
        }));
        receipt["relogin"][phase] = json!(attempts);
        write_receipt(receipt_path, receipt);
        match result {
            Ok(()) => return,
            Err(error) => {
                assert_eq!(error.code, 5, "{phase} relog refused: {error:?}");
                assert!(
                    started.elapsed() + cadence < budget,
                    "{phase} account ownership was not released within {budget:?}"
                );
                std::thread::sleep(cadence);
            }
        }
    }
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
    quest_progress::quest_list_status(
        quests,
        quest_id,
        SnapshotView::new(Some(snapshot), evidence_stamp(snapshot)),
    )
    .map(|observed| observed.value)
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

fn resource_stands(world: &NavWorld, spot: &GatherSpot) -> Vec<WorldTile> {
    let width = i32::from(spot.width.max(1));
    let length = i32::from(spot.length.max(1));
    let mut stands = Vec::new();
    if world.collision.standable(spot.origin) {
        stands.push((0, spot.origin));
    }
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
            if world.collision.standable(tile) {
                stands.push((i64::from(dx).pow(2) + i64::from(dz).pow(2), tile));
            }
        }
    }
    stands.sort_unstable_by_key(|(distance, tile)| (*distance, tile.x, tile.z));
    stands.into_iter().map(|(_, tile)| tile).collect()
}

fn resource_targets(
    data: &game_data::SelectedGameData,
    gathering: &GatherCatalog,
    world: &NavWorld,
    case: EntranceCase,
) -> Result<Vec<ResourceTarget>, String> {
    use api::gather_methods::TargetClass;
    use api::selected::Knowledge;

    let site = data
        .gather_site(case.skill(), case.site_id())
        .ok_or_else(|| format!("selected game data lacks site {}", case.site_id()))?;
    let from = center(site.region);
    let mut targets = Vec::new();
    for key in &site.keys {
        let option = data
            .gather_option(case.skill(), &key.key)
            .ok_or_else(|| format!("site {} names unknown option {}", site.id, key.key))?;
        for method_id in &option.methods {
            let method = gathering
                .method(method_id)
                .map_err(|error| format!("site {} method {method_id}: {error:?}", site.id))?;
            let expected_skill = match case.skill() {
                "fishing" => GatherSkill::Fishing,
                _ => GatherSkill::Mining,
            };
            if method.skill != expected_skill {
                continue;
            }
            let Knowledge::Known(spots) = &method.spots else {
                return Err(format!("method {method_id} placements are not complete"));
            };
            // We need a proved Resource member, not proof that this method
            // has no additional targets (for example incidental hazards).
            let method_targets = api::gather_methods::known_rows(&method.targets);
            let nearest = gathering
                .nearest(
                    method,
                    from,
                    Some(&site.region),
                    AccessPolicy::Possible,
                    spots.len(),
                )
                .map_err(|error| format!("nearest {} resource: {error:?}", key.key))?;
            for nearest in nearest {
                if !method_targets.iter().any(|target| {
                    target.entity == nearest.spot.entity && target.class == TargetClass::Resource
                }) {
                    continue;
                }
                for stand in resource_stands(world, nearest.spot) {
                    targets.push(ResourceTarget {
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
        }
    }
    if targets.is_empty() {
        return Err(format!(
            "site {} has no selected resource placement with a stand",
            site.id
        ));
    }
    Ok(targets)
}

fn matching_edges(world: &NavWorld, case: EntranceCase) -> Vec<(usize, &TransportEdge)> {
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

fn route_options(
    world: &NavWorld,
    case: EntranceCase,
    state: &WorldState,
    from: WorldTile,
) -> FindOptions {
    host_play::resolve_script_cross_zones_for_live_test(
        FindOptions {
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            ..FindOptions::default()
        },
        world,
        from,
        from,
        state,
        case.cross_zones(),
    )
    .expect("the product accepts the approved request-local crossing grants")
}

fn find_route(
    world: &NavWorld,
    from: WorldTile,
    to: WorldTile,
    options: FindOptions,
    state: &WorldState,
) -> Result<Route, RouteError> {
    find_with(&world.collision, &world.graph, from, to, options, state)
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
        let takeoff = edge.takeoff.unwrap_or(edge.at);
        if world.collision.standable(takeoff) && seen.insert(takeoff) {
            starts.push(takeoff);
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
    targets: &[ResourceTarget],
    options: FindOptions,
    state: &WorldState,
) -> Result<(usize, PlannedRoute), String> {
    let tiles: Vec<_> = targets.iter().map(|target| target.stand).collect();
    let mut approaches = Vec::new();
    // Decide all of this site's actual stands in one bounded search per
    // entrance approach, rather than repeating a world flood per rock.
    for start in candidate_starts(world, case) {
        let costs = find_many_with(
            &world.collision,
            &world.graph,
            start,
            &tiles,
            options,
            state,
        );
        assert!(
            costs.complete(),
            "resource-stand search from {start:?} did not finish"
        );
        let mut reachable = 0;
        for (index, result) in costs.results().iter().enumerate() {
            match result {
                Err(TargetError::NoPath) => continue,
                Err(error) => panic!("resource-stand search did not decide {start:?}: {error:?}"),
                Ok(_) => reachable += 1,
            }
            let route = find_route(world, start, tiles[index], options, state)
                .expect("the completed costs-only search proved this stand reachable");
            if let Some(edge) = planned_edge(&route, case) {
                return Ok((index, PlannedRoute { start, route, edge }));
            }
        }
        let blockers = if approaches.is_empty() {
            nav_planner::router::find_first_blocking_zones(
                &world.collision,
                &world.graph,
                start,
                &tiles,
                options,
                state,
                &[],
            )
            .map(|keys| {
                let table = world.graph.zones.as_ref().expect("diagnosed zone table");
                keys.into_iter()
                    .map(|key| table.name(key))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
        } else {
            Vec::new()
        };
        approaches.push(json!({
            "start": start,
            "reachable_stands": reachable,
            "blocking_zones": blockers,
        }));
    }
    Err(format!("no actual resource stand is reachable through the declared entrance; approaches={approaches:?}"))
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
        // The first lower-mine walk has 87 vertices. Keep this test's original
        // 180-second limit, expressed in Engine A's real 600-ms server ticks,
        // instead of expiring that valid leg at the default 60 ticks.
        budget_ticks_per_hop: u32::try_from(TRAVEL_TIMEOUT.as_millis() / 600)
            .expect("bounded acceptance window in server ticks"),
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
        if outcome.is_none() && last_tick != Some(snapshot.tick()) {
            last_tick = Some(snapshot.tick());
            outcome = traveller.follow(client, snapshot, route.clone(), &mut options);
        }
        // A region rebuild can still be pending at the terminal movement
        // tick. Keep pumping inside the original deadline, without another
        // Traveller call or send, until the arrival scene is ready.
        if outcome.as_ref().is_some_and(|terminal| {
            !matches!(terminal, TravelOutcome::Arrived { .. })
                || (snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2)
        }) {
            break;
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
    route
        .legs
        .iter()
        .position(|leg| matches!(leg, Leg::Transport { edge } if case.matches_edge(edge)))
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
    assert!(
        prefix.len() == 4 && prefix.bytes().all(|byte| byte.is_ascii_lowercase()),
        "use an owned four-character lowercase account namespace"
    );

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
        "pack_format": nav_planner::pack::FORMAT_ID,
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
    assert_eq!(
        nav_planner::pack::FORMAT_ID,
        "274V17",
        "acceptance requires pack v17"
    );
    let identity = profile.nav_identity().expect("navigation identity");
    receipt["nav_identity"] = json!({
        "pack_format": nav_planner::pack::FORMAT_ID,
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
    let (quests, gathering) =
        api::selected::FamilyPreparation::within(&mut preparation, |worker| {
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
    assert!(
        !edges.is_empty(),
        "pack lacks expected {} entrance edge",
        case.name()
    );
    for (_, edge) in &edges {
        assert!(
            edge.quest_req.is_empty(),
            "new entrance must not use quest_req: {edge:?}"
        );
        assert!(
            edge.varp_req.is_empty(),
            "new entrance must not use raw varps: {edge:?}"
        );
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

    let targets = resource_targets(&selected, &gathering, &world, case)
        .unwrap_or_else(|error| panic!("select resource destinations: {error}"));
    receipt["resource_candidates"] = json!(targets.len());
    write_receipt(&receipt_path, &receipt);

    let mut client = template
        .prepare_client((serial % i32::MAX as u128) as i32, true)
        .expect("prepare CPU client");
    client.draw = false;
    client.maininit();
    assert!(!client.error_loading, "client failed to load");
    assert!(
        interact::login(&mut client, &account, &password, false),
        "local login refused"
    );
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
    assert!(
        wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
            s.chat_lines()
                .iter()
                .any(|line| line.text.contains("get tutorial: 1000"))
        }),
        "tutorial skip was not acknowledged"
    );
    assert!(
        logout_and_wait(&mut client, &mut snapshot, &mut pump),
        "tutorial relog-out failed"
    );
    // The clean login binds tutorial-locked side icons and the quest journal.
    login_after_logout(
        &mut client,
        (&account, &password),
        "tutorial",
        snapshot.tick(),
        &mut receipt,
        &receipt_path,
    );
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
    assert!(
        fresh_account_baseline,
        "account was not fresh before seeding: {receipt}"
    );
    assert!(
        snapshot.quest_statuses_available(),
        "quest-list observation is unavailable"
    );
    if case != EntranceCase::GrandTreeMine {
        // Keep the real NPC danger policy. Seed only account combat stats,
        // as permitted account state, so it does not mask entrance access.
        // Engine ClientCheatHandler.ts:472-485 mutates only this player.
        for skill in [
            "attack",
            "defence",
            "strength",
            "hitpoints",
            "ranged",
            "prayer",
            "magic",
        ] {
            assert!(
                interact::cheat(&mut client, &format!("setstat {skill} 99")).is_sent(),
                "combat-stat account seed refused"
            );
        }
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                (0..7).all(|index| {
                    s.stats()
                        .iter()
                        .any(|stat| stat.index == index && stat.base == 99 && stat.effective == 99)
                })
            }),
            "actual combat-stat packets did not acknowledge the account seed"
        );
        let combat = WorldState::from_snapshot(&snapshot).combat_level;
        assert_eq!(combat, Some(126), "observed maximum combat account");
        receipt["combat_seed"] = json!({
            "combat_level": combat,
            "observed": snapshot.stats().iter().filter(|stat| stat.index < 7)
                .map(|stat| (stat.index, stat.base, stat.effective)).collect::<Vec<_>>(),
        });
        write_receipt(&receipt_path, &receipt);
    }

    let pre_quest_evidence = quest_evidence(
        &quests,
        family_identity,
        selected
            .quest_family_identity()
            .expect("selected quest family"),
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
        assert_eq!(
            verdict,
            Truth::False,
            "fresh account did not disprove Grand Tree completion"
        );
    }

    let quest_varp = if let Some((name, value)) = case.quest_varp() {
        let observed = set_quest_varp(&mut client, &mut snapshot, &mut pump, name, value);
        receipt["quest_varp"] = json!({"name":name, "value":value, "observed":observed});
        write_receipt(&receipt_path, &receipt);
        assert!(
            observed,
            "account-local {name}={value} quest varp was not observed"
        );
        assert!(
            logout_and_wait(&mut client, &mut snapshot, &mut pump),
            "quest-seed relog-out failed"
        );
        login_after_logout(
            &mut client,
            (&account, &password),
            "quest_seed",
            snapshot.tick(),
            &mut receipt,
            &receipt_path,
        );
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                s.ingame()
                    && s.attached()
                    && s.scene_state() == 2
                    && quest_status(&quests, name, s) == Some(QuestListStatus::Complete)
            }),
            "relog did not publish actual Complete quest-list evidence for {name}"
        );
        receipt["quest_varp"]["observed_status_after_relog"] =
            json!(format!("{:?}", quest_status(&quests, name, &snapshot)));
        write_receipt(&receipt_path, &receipt);
        Some(json!({"name":name,"value":value}))
    } else {
        None
    };

    if let Some(id) = case.required_item() {
        give_item(&mut client, &mut snapshot, &mut pump, &selected, id, 1);
    }
    for &id in case.required_worn() {
        let item = selected_item(&selected, id);
        assert!(
            item.worn_model && item.wear_position >= 0,
            "kit object {id} is not wearable"
        );
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
        selected
            .quest_family_identity()
            .expect("selected quest family"),
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
    receipt["route_state"] = json!({
        "inventory": state.inv,
        "inventory_ready": state.inventory_ready,
        "worn": state.worn,
        "equipment_ready": state.equipment_ready,
        "map_members": state.map_members,
        "entrance_allowed": edges.iter().map(|(_, edge)| state.allows(edge)).collect::<Vec<_>>(),
    });
    let from = ReadContext::new(&snapshot)
        .world_tile()
        .expect("the live account has an observed world tile");
    let options = route_options(&world, case, &state, from);
    receipt["route_options"] = json!({
        "crossZones": case.cross_zones(),
        "allow_teleports": options.allow_teleports,
        "allow_wilderness": options.allow_wilderness,
        "allow_bank_fetch": options.allow_bank_fetch,
    });
    write_receipt(&receipt_path, &receipt);
    let (target_index, planned) = plan_route(&world, case, &targets, options, &state)
        .unwrap_or_else(|error| panic!("route planning failed: {error}; receipt={receipt}"));
    let target = targets
        .into_iter()
        .nth(target_index)
        .expect("selected resource candidate");
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
        selected
            .quest_family_identity()
            .expect("selected quest family"),
        family,
        &snapshot,
    );
    let final_state = state_from_snapshot(&snapshot, profile.map_members(), final_evidence.clone());
    let route = find_route(&world, planned.start, target.stand, options, &final_state)
        .unwrap_or_else(|error| {
            panic!("final live-state route failed: {error:?}; receipt={receipt}")
        });
    let edge = planned_edge(&route, case).expect("final route uses the declared entrance");
    if case == EntranceCase::TaverleyLavaEels {
        let default_result = find_route(
            &world,
            planned.start,
            target.stand,
            FindOptions {
                zones: ZoneExempt::NONE,
                ..options
            },
            &final_state,
        );
        let default_refused = matches!(default_result, Err(RouteError::NoPath));
        receipt["default_danger_policy"] = json!({
            "combat_level": final_state.combat_level,
            "result": format!("{default_result:?}"),
            "no_path": default_refused,
        });
        write_receipt(&receipt_path, &receipt);
        assert!(
            default_refused,
            "the named crossing must not alter default danger admission"
        );
    }
    assert!(
        edge.quest_req.is_empty(),
        "planned entrance contains raw quest requirement"
    );
    assert!(
        edge.varp_req.is_empty(),
        "planned entrance contains raw varp requirement"
    );
    if let Some(gates) = &edge.quest_gates {
        assert_eq!(
            gates.family(),
            &family,
            "planned entrance uses another quest family"
        );
        for gate in gates.gates() {
            if matches!(gate, QuestGate::Window(_)) {
                assert_eq!(final_evidence.test(gate), Truth::Unknown);
            }
        }
    }

    if let Some(required_item) = case.required_item() {
        assert!(
            edge.item_req
                .iter()
                .any(|&(id, count)| id == required_item && count > 0),
            "planned entrance does not require selected item {required_item}: {edge:?}"
        );
    }
    if case == EntranceCase::DesertMiningCampUnderground {
        let mut expected = DESERT_KIT_IDS.to_vec();
        expected.sort_unstable();
        let mut actual = edge.worn_all_req.clone();
        actual.sort_unstable();
        assert_eq!(
            actual, expected,
            "underground entrance must require all three worn kit pieces"
        );
        assert!(
            route_moves_beyond_entrance(&route, case),
            "lower mine resource route must continue beyond the entrance"
        );
    }
    if case == EntranceCase::TaverleyLavaEels {
        assert_eq!(
            edge.option, 0,
            "dusty-key door must use its content item-on-loc option"
        );
    }
    if case == EntranceCase::DesertMiningCamp {
        assert!(
            edge.forbidden_held.contains(&ANA_BARREL_ID),
            "camp gate must exclude Ana's barrel"
        );
        assert!(
            !edge.forbidden_worn.is_empty(),
            "camp gate must carry its equipment exclusions"
        );
    }

    let missing_result = if case == EntranceCase::GrandTreeMine {
        let mut missing = final_state
            .clone()
            .with_quest_evidence(pre_quest_evidence.clone());
        missing.quests.remove("The Grand Tree");
        find_route(&world, planned.start, target.stand, options, &missing)
    } else if case == EntranceCase::DesertMiningCampUnderground {
        let mut missing = final_state.clone();
        for id in DESERT_KIT_IDS {
            missing.worn.remove(id);
        }
        find_route(&world, planned.start, target.stand, options, &missing)
    } else if let Some(id) = case.required_item() {
        let mut missing = final_state.clone();
        missing.inv.remove(&id);
        find_route(&world, planned.start, target.stand, options, &missing)
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
    assert!(
        precondition_no_path,
        "missing entrance prerequisite still produced a route: {receipt}"
    );

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
        assert!(
            held_item.alias.is_some(),
            "Ana's barrel has no selected content alias"
        );
        give_item(
            &mut client,
            &mut snapshot,
            &mut pump,
            &selected,
            ANA_BARREL_ID,
            1,
        );
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
                requirement: nav_planner::world_state::EntranceRequirement::ForbiddenHeld { id, .. },
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
        assert!(
            held_expected,
            "Traveller did not return the typed held-item refusal: {receipt}"
        );
        assert!(
            interact::cheat(&mut client, "~clearinv").is_sent(),
            "inventory debugproc refused"
        );
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                inventory_count(s, ANA_BARREL_ID) == 0 && inventory_count(s, DESERT_KEY_ID) == 0
            }),
            "held exclusion item did not clear"
        );
        give_item(
            &mut client,
            &mut snapshot,
            &mut pump,
            &selected,
            DESERT_KEY_ID,
            1,
        );
        assert!(
            seed_at(&mut client, &mut snapshot, &mut pump, planned.start),
            "could not reset to camp approach"
        );

        // A fresh account can wear the bronze sword. Selecting the first
        // forbidden object can choose Excalibur, whose wear requirement is
        // unrelated to this entrance proof.
        let forbidden_id = 1277;
        assert!(
            edge.forbidden_worn.contains(&forbidden_id),
            "the source-derived gate must forbid the bronze sword"
        );
        give_item(
            &mut client,
            &mut snapshot,
            &mut pump,
            &selected,
            forbidden_id,
            1,
        );
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
                requirement: nav_planner::world_state::EntranceRequirement::ForbiddenWorn { id, .. },
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
        assert!(
            worn_expected,
            "Traveller did not return the typed worn-item refusal: {receipt}"
        );
        assert!(
            matches!(
                Interactions::new(&snapshot, &mut client).unequip(forbidden_id),
                SendResult::Sent { .. }
            ),
            "normal unequip({forbidden_id}) was refused"
        );
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                equipment_count(s, forbidden_id) == 0
            }),
            "forbidden worn object did not leave equipment"
        );
        assert!(
            interact::cheat(&mut client, "~clearinv").is_sent(),
            "inventory debugproc refused"
        );
        assert!(
            wait_for(&mut client, &mut snapshot, &mut pump, SETUP_TIMEOUT, |s| {
                inventory_count(s, DESERT_KEY_ID) == 0
            }),
            "camp key did not clear"
        );
        give_item(
            &mut client,
            &mut snapshot,
            &mut pump,
            &selected,
            DESERT_KEY_ID,
            1,
        );
        assert!(
            seed_at(&mut client, &mut snapshot, &mut pump, planned.start),
            "could not reset to camp approach"
        );
    }

    let travel = drive_route(
        &mut client,
        &mut snapshot,
        &mut pump,
        &world,
        &route,
        Some(&final_evidence),
    );
    let RouteProof {
        resource_arrival,
        fresh_tile_progress,
        entrance_attempt,
        entrance_crossing,
    } = route_proof(&travel, target.stand, edge.loc_id, edge.at, edge.to);
    let arrival_scene_ready =
        snapshot.ingame() && snapshot.attached() && snapshot.scene_state() == 2;
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
    println!(
        "{}",
        serde_json::to_string_pretty(&receipt).expect("print receipt")
    );

    assert!(
        resource_arrival,
        "Traveller did not reach the selected resource stand: {receipt}"
    );
    assert!(
        entrance_attempt,
        "Traveller did not send the selected entrance interaction: {receipt}"
    );
    assert!(
        entrance_crossing,
        "no server-settled progress across the selected entrance: {receipt}"
    );
    assert!(
        fresh_tile_progress,
        "Traveller produced no fresh tile progress: {receipt}"
    );
    assert!(
        arrival_scene_ready,
        "resource arrival scene was not ready: {receipt}"
    );
    assert!(
        start_capture.is_ok(),
        "start capture was not preserved: {receipt}"
    );
    assert!(
        end_capture.is_ok(),
        "end capture was not preserved: {receipt}"
    );
    assert!(
        logout_observed,
        "successful explicit logout was not observed: {receipt}"
    );
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, an owned four-character BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_grand_tree_mine_requires_quest_completion_and_reaches_resources() {
    run_case(EntranceCase::GrandTreeMine);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, an owned four-character BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_desert_mining_camp_requires_metal_key_and_refuses_excluded_items() {
    run_case(EntranceCase::DesertMiningCamp);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, an owned four-character BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_desert_mining_camp_lower_mine_requires_worn_slave_kit() {
    run_case(EntranceCase::DesertMiningCampUnderground);
}

#[test]
#[ignore = "requires LIVE=1, BOT_CPU=1, disposable HOME, an owned four-character BOT_LIVE_NAME_PREFIX, BOT_CACHE_DIR, BOT_NAV_SNAPSHOT_ROOT, WORLD_ENGINE_DIR, WORLD_NAV_PACK, BOT_NAV_BUILD=skip, and NAV_GATED_ENTRANCES_EVIDENCE"]
fn live_taverley_lava_eels_requires_dusty_key() {
    run_case(EntranceCase::TaverleyLavaEels);
}

#[cfg(test)]
mod proof_tests {
    use super::*;

    fn tile(x: i32) -> WorldTile {
        WorldTile { x, z: 1, level: 0 }
    }

    fn valid_travel() -> TravelRecord {
        TravelRecord {
            outcome: Some(TravelOutcome::Arrived { at: tile(4) }),
            start_tile: Some(tile(1)),
            start_tick: 10,
            final_tile: Some(tile(4)),
            progress: vec![
                TileProgress {
                    tick: 11,
                    tile: tile(3),
                },
                TileProgress {
                    tick: 12,
                    tile: tile(4),
                },
            ],
            attempts: vec![TransportAttemptRecord {
                tick: 10,
                kind: "Door".into(),
                expected_id: 7,
                actual_id: 7,
                target: tile(2),
                option: 1,
                refusal: None,
                tile_at_attempt: Some(tile(1)),
            }],
            events: Vec::new(),
            legs: Vec::new(),
        }
    }

    fn passes(travel: &TravelRecord) -> bool {
        let proof = route_proof(travel, tile(4), 7, tile(2), tile(3));
        proof.resource_arrival
            && proof.fresh_tile_progress
            && proof.entrance_attempt
            && proof.entrance_crossing
    }

    #[test]
    fn actual_arrival_after_a_matching_entrance_crossing_passes() {
        assert!(passes(&valid_travel()));
    }

    #[test]
    fn no_op_stale_wrong_or_refused_transport_proofs_fail() {
        let mut missing_arrival = valid_travel();
        missing_arrival.outcome = None;
        assert!(!passes(&missing_arrival));

        let mut wrong_destination = valid_travel();
        wrong_destination.outcome = Some(TravelOutcome::Arrived { at: tile(5) });
        assert!(!passes(&wrong_destination));

        let mut no_move = valid_travel();
        no_move.progress.clear();
        assert!(!passes(&no_move));

        let mut stale_move = valid_travel();
        stale_move
            .progress
            .iter_mut()
            .for_each(|step| step.tick = 10);
        assert!(!passes(&stale_move));

        let mut missing_crossing = valid_travel();
        missing_crossing
            .progress
            .retain(|step| step.tile != tile(3));
        assert!(!passes(&missing_crossing));

        let mut wrong_expected_id = valid_travel();
        wrong_expected_id.attempts[0].expected_id = 8;
        assert!(!passes(&wrong_expected_id));

        let mut wrong_actual_id = valid_travel();
        wrong_actual_id.attempts[0].actual_id = 8;
        assert!(!passes(&wrong_actual_id));

        let mut wrong_target = valid_travel();
        wrong_target.attempts[0].target = tile(5);
        assert!(!passes(&wrong_target));

        let mut refused = valid_travel();
        refused.attempts[0].refusal = Some("refused".into());
        assert!(!passes(&refused));
    }
}
