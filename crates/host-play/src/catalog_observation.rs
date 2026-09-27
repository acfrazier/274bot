use super::*;
use super::{
    actor::{choose_actor_observation_npc, packed_self_target, parse_actor_receipt_from_paint},
    fight_field::{choose_fight_field_npc, parse_fight_field_receipt_from_paint},
    los::collision_flag_at,
};
#[derive(Debug, Clone, Default, Serialize)]
pub struct Observation {
    pub ingame: bool,
    pub scene_state: i32,
    pub player: Option<String>,
    pub tile: Option<(i32, i32, i32)>,
    pub combat_level: i32,
    pub tick: u32,
    pub items: BTreeMap<String, i32>,
    pub item_ids: BTreeMap<i32, i32>,
    pub bank: BTreeMap<String, i32>,
    pub bank_ids: BTreeMap<i32, i32>,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    /// Bounded native receipt for ScriptRunner.stop from this Start. The
    /// panel log queue is not consumed to populate it.
    pub script_lifecycle: Option<script::ScriptLifecycleReceipt>,
    /// Native base skill levels from the snapshot stat table.
    pub levels: BTreeMap<String, i32>,
    /// Native effective skill levels, retained separately for fixture gates.
    pub effective_levels: BTreeMap<String, i32>,
    pub xp: BTreeMap<String, i32>,
    /// Only varps explicitly used by a core witness; do not clone the full table.
    pub varps: BTreeMap<i32, i32>,
    pub chat: Vec<(i32, String)>,
    pub loc_facts: Vec<BoundedLoc>,
    pub npc_facts: Vec<BoundedNpc>,
    pub magic_tree_ready: bool,
    pub dormant_rocks_seen: bool,
    pub ground_loot: Vec<BoundedGround>,
    pub local_in_combat: bool,
    /// Frozen snapshot positive-hit predicate. Zero/blocked/expired hits do not qualify.
    pub taking_damage: bool,
    pub local_target_npc: Option<usize>,
    pub local_health: i32,
    pub local_animation: i32,
    /// Prior-frame guardian publication. Snapshot observe runs before this
    /// frame's status-row copy of `RandomStatus`.
    pub guardian: BoundedGuardian,
    pub equipment_ids: BTreeMap<i32, i32>,
    pub main_modal: i32,
    pub widget_ids: BTreeSet<i32>,
    /// Compact open-shop stock (empty while the shop is down). Not a full
    /// shop clone.
    pub shop_open: bool,
    pub shop_stock: Vec<BoundedShopItem>,
    /// Posted anvil/main-skill-multi row ids this frame (empty if the panel
    /// was not decoded). Chat `make_products` does not fill this.
    pub main_make_ids: BTreeSet<i32>,
    /// Already-published inspect terminal, attached only for inspect Core
    /// cases. Default empty; `from_snapshot` does not copy hops.
    pub route_inspect_seq: u64,
    pub route_inspect_generation: u64,
    pub route_inspect_request_id: u64,
    pub route_inspect_ok: bool,
    pub route_inspect_reason: String,
    #[serde(rename = "route_inspect_hops")]
    pub route_inspect_hops: Vec<RouteInspectHopFact>,
    /// `InspectNav.generation` even when no terminal is published.
    /// Missing-terminal Observation defaults are 0 and are not this value.
    pub route_inspect_live_generation: u64,
    pub route_inspect_has_terminal: bool,
    /// Compact current-plane identity and selected pair cells. Empty unless
    /// the active Core case asked for collision. Never a whole-grid dump.
    pub los: LineOfSightObservation,
    /// Compact chosen NPC + LOS helper result. Empty unless the active Core
    /// case asked for actor observation. Never a world or NPC-table copy.
    pub actor: ActorObservation,
    /// Compact fight-field NPC + both LOS helper results. Empty unless the
    /// active Core case asked for fight field. Never a world or NPC-table copy.
    pub fight: FightFieldObservation,
    /// Compact hunt witness: the slot's act ledger and the cell's paint
    /// receipt. Empty unless the active Core case is a hunt cell.
    pub hunt: HuntObservation,
}

/// Compact hop projection for Core JSON. Only `locName` is copied from the
/// already-published host terminal.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RouteInspectHopFact {
    pub loc_name: String,
}

/// Identity/freshness/ok/hop names already published by the host inspect
/// terminal. Copied only when the active Core case asks for it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RouteInspectPublished {
    pub live_generation: u64,
    pub has_terminal: bool,
    pub seq: u64,
    pub generation: u64,
    pub request_id: u64,
    pub ok: bool,
    pub reason: String,
    pub hop_loc_names: Vec<String>,
}
/// One loc retained for these named cases. The live loc sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedLoc {
    pub id: i32,
    pub x: i32,
    pub z: i32,
    pub level: i32,
    pub name: Option<String>,
    pub open: bool,
}

/// Compact prior-frame guardian fact. Not a history buffer and not the chrome row.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct BoundedGuardian {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub ours: bool,
    pub hold: bool,
}

/// Compact NPC identity used by combat cores. The live NPC sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedNpc {
    pub index: usize,
    pub name: Option<String>,
    pub health: i32,
    pub total_health: i32,
    pub animation: i32,
    pub in_combat: bool,
    pub targeting_local: bool,
    pub tile: (i32, i32, i32),
    pub distance: i32,
}

/// Compact ground loot of combat-core item ids. The live ground sweep is not copied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedGround {
    pub id: i32,
    pub count: i32,
    pub tile: (i32, i32, i32),
    pub distance: i32,
}

/// One posted shop-stock row retained for shop-buyout cores.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct BoundedShopItem {
    pub id: i32,
    pub count: i32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlaxLocFailureFact {
    pub id: i32,
    pub tile: (i32, i32, i32),
    pub name: Option<String>,
    pub actions: Vec<String>,
    pub field_distance: i32,
    pub player_distance: i32,
    pub reachable: bool,
    pub reachable_adj: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FlaxAioPickFailureFacts {
    pub player_tile: Option<(i32, i32, i32)>,
    pub field_center: (i32, i32, i32),
    pub field_scope: i32,
    pub at_field: bool,
    pub bank_open: bool,
    pub bank_loaded: bool,
    pub bank_generation: u64,
    pub reachability_source: &'static str,
    pub adapter_query: &'static str,
    pub reachability_available: bool,
    pub relevant_locs: Vec<FlaxLocFailureFact>,
    pub nearest_reachable: Option<(i32, i32, i32)>,
}

pub fn flax_aio_pick_failure_facts(
    case: CoreCase,
    observation: &Observation,
    scene: &SceneView,
    locs: &[LocView],
) -> Option<FlaxAioPickFailureFacts> {
    if case != CoreCase::FlaxAioPick {
        return None;
    }
    let player_tile = observation.tile;
    let player_world = player_tile.map(|(x, z, level)| WorldTile { x, z, level });
    let flood = api::query::SceneQuery::new(scene, player_world).flood_reach();
    let field_world = WorldTile {
        x: FLAX_FIELD.0,
        z: FLAX_FIELD.1,
        level: FLAX_FIELD.2,
    };
    let distance = |a: WorldTile, b: WorldTile| {
        let xz = (a.x - b.x).abs().max((a.z - b.z).abs());
        if a.level == b.level {
            xz
        } else {
            1_000_000 + xz
        }
    };
    let mut relevant_locs = locs
        .iter()
        .filter(|loc| {
            loc.name
                .as_deref()
                .is_some_and(|name| name.trim().eq_ignore_ascii_case("Flax"))
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.eq_ignore_ascii_case("Pick"))
                && distance(loc.tile, field_world) <= FLAX_FIELD_SCOPE
        })
        .map(|loc| {
            let (reachable, reachable_adj) = flood
                .as_ref()
                .map(|flood| flood.at(&loc.tile))
                .unwrap_or((false, false));
            FlaxLocFailureFact {
                id: loc.id,
                tile: (loc.tile.x, loc.tile.z, loc.tile.level),
                name: loc.name.clone(),
                actions: loc.actions.iter().flatten().cloned().collect(),
                field_distance: distance(loc.tile, field_world),
                player_distance: player_world
                    .map(|player| distance(loc.tile, player))
                    .unwrap_or(1_000_000),
                reachable,
                reachable_adj,
            }
        })
        .collect::<Vec<_>>();
    relevant_locs.sort_by_key(|loc| loc.player_distance);
    let nearest_reachable = relevant_locs
        .iter()
        .find(|loc| loc.reachable_adj)
        .map(|loc| loc.tile);

    Some(FlaxAioPickFailureFacts {
        player_tile,
        field_center: FLAX_FIELD,
        field_scope: FLAX_FIELD_SCOPE,
        at_field: player_world
            .is_some_and(|player| distance(player, field_world) <= FLAX_FIELD_SCOPE),
        bank_open: observation.bank_open,
        bank_loaded: observation.bank_loaded,
        bank_generation: observation.bank_generation,
        reachability_source: "api::query::SceneQuery::flood_reach().at(tile)",
        adapter_query: "Reachability.canReach(tile,{adjacentOk:true,maxSteps:400})",
        reachability_available: flood.is_some(),
        relevant_locs,
        nearest_reachable,
    })
}

pub fn failure_diagnostic(case: CoreCase, snapshot: &GameSnapshot, names: &ObjNames) -> Value {
    let observation = Observation::from_snapshot(snapshot, names);
    json!({
        "observation": &observation,
        "flax_aio_pick": flax_aio_pick_failure_facts(
            case,
            &observation,
            snapshot.scene(),
            snapshot.locs(),
        ),
    })
}

impl Observation {
    pub fn from_snapshot(snapshot: &GameSnapshot, names: &ObjNames) -> Self {
        let mut items = BTreeMap::new();
        for (id, count) in snapshot.inv() {
            let name = names
                .name(*id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{id}"));
            *items.entry(name).or_insert(0) += *count;
        }
        let mut item_ids = BTreeMap::new();
        for (id, count) in snapshot.inv() {
            *item_ids.entry(*id).or_insert(0) += *count;
        }
        let xp = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.xp))
            .collect();
        let mut bank = BTreeMap::new();
        for row in snapshot.bank() {
            let name = names
                .name(row.def.id)
                .map(str::to_string)
                .unwrap_or_else(|| format!("obj#{}", row.def.id));
            *bank.entry(name).or_insert(0) += row.count;
        }
        let mut bank_ids = BTreeMap::new();
        for row in snapshot.bank() {
            *bank_ids.entry(row.def.id).or_insert(0) += row.count;
        }
        let levels = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.base))
            .collect();
        let effective_levels = snapshot
            .stats()
            .iter()
            .map(|stat| (stat.name.to_ascii_lowercase(), stat.effective))
            .collect();
        let varps = snapshot
            .varps()
            .iter()
            .filter(|varp| {
                matches!(
                    varp.index,
                    BRIMHAVEN_ARENA_VARP
                        | AUTOCAST_MAGIC_VARP
                        | COMBAT_MODE_VARP
                        | SA_ENERGY_VARP
                        | SA_ARMED_VARP
                        | VARP_TARGET_COUNT
                        | VARP_TARGET_SCORE
                        | VARP_TARGET_HIT
                )
            })
            .map(|varp| (varp.index, varp.value))
            .collect();
        let chat = snapshot
            .chat_lines()
            .iter()
            .map(|line| (line.sequence, line.text.clone()))
            .collect();
        let player = snapshot
            .local_player()
            .and_then(|local| local.player.actor.name.clone());
        let loc_facts = snapshot
            .locs()
            .iter()
            .filter(|loc| loc.distance <= 8 && keep_bounded_loc(loc.id, loc.name.as_deref()))
            .take(16)
            .map(|loc| BoundedLoc {
                id: loc.id,
                x: loc.tile.x,
                z: loc.tile.z,
                level: loc.tile.level,
                name: loc.name.clone(),
                open: loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|op| op.trim().to_ascii_lowercase().starts_with("open")),
            })
            .collect();
        let magic_tree_ready = snapshot.locs().iter().any(|loc| {
            loc.id == MAGIC_TREE_ID
                && (loc.tile.x, loc.tile.z, loc.tile.level) == GNOME_SOUTH_BANK_MAGIC_TREE
                && loc
                    .actions
                    .iter()
                    .flatten()
                    .any(|action| action.trim().eq_ignore_ascii_case("Chop down"))
        });
        let mut equipment_ids = BTreeMap::new();
        for item in snapshot.equipment() {
            if item.count > 0 && item.def.id >= 0 {
                *equipment_ids.entry(item.def.id).or_insert(0) += item.count;
            }
        }
        let self_slot = snapshot.self_slot();
        let local = snapshot.local_player();
        let local_in_combat = local.is_some_and(|player| player.player.actor.in_combat);
        let local_target_npc = local
            .and_then(|player| player.player.actor.target)
            .and_then(|target| (target.kind == ActorKind::Npc).then_some(target.index));
        let local_health = local.map(|player| player.player.actor.health).unwrap_or(0);
        let local_animation = local
            .map(|player| player.player.actor.animation)
            .unwrap_or(0);
        let npc_facts = snapshot
            .npcs()
            .iter()
            .filter(|npc| {
                npc.distance <= 8
                    && (npc.in_combat
                        || combat_npc_name(npc.name.as_deref())
                        || npc.target.is_some_and(|target| {
                            target.kind == ActorKind::Player
                                && self_slot >= 0
                                && target.index == self_slot as usize
                        })
                        || local_target_npc == Some(npc.index))
            })
            .take(8)
            .map(|npc| bounded_combat_npc(npc, self_slot))
            .collect::<Vec<_>>();
        // Dormant RockCrab `Rocks` are outside that combat window by
        // construction (no target, not in combat, parked far from the player),
        // yet their real index/tile identity is what the activation witness
        // matches: a `Rock Crab` on one of those witnesses is the source's own
        // Rocks->crab transition, not a separately spawned crab. Project the
        // nearest dormant Rocks inside the supported field, bounded, so the
        // cycle can see that identity without the world copy.
        let mut dormant_rocks = snapshot
            .npcs()
            .iter()
            .filter(|npc| dormant_rock_in_field(npc))
            .collect::<Vec<_>>();
        dormant_rocks.sort_by_key(|npc| (npc.distance, npc.index));
        dormant_rocks.truncate(DORMANT_ROCK_FACTS_MAX);
        // The field boolean is this projection, not a second sweep.
        let dormant_rocks_seen = !dormant_rocks.is_empty();
        let npc_facts = npc_facts
            .into_iter()
            .chain(
                dormant_rocks
                    .into_iter()
                    .map(|npc| bounded_combat_npc(npc, self_slot)),
            )
            .collect();
        let ground_loot = snapshot
            .ground_items()
            .iter()
            .filter(|item| item.distance <= 8 && combat_ground_id(item.def.id))
            .take(8)
            .map(|item| BoundedGround {
                id: item.def.id,
                count: item.count,
                tile: (item.tile.x, item.tile.z, item.tile.level),
                distance: item.distance,
            })
            .collect();
        Self {
            ingame: snapshot.ingame() && snapshot.attached(),
            scene_state: snapshot.scene_state(),
            player,
            tile: snapshot.tile(),
            combat_level: snapshot
                .local_player()
                .map(|local| local.player.combat_level)
                .unwrap_or(0),
            tick: snapshot.tick(),
            items,
            item_ids,
            bank,
            bank_ids,
            bank_open: snapshot.bank_component_id() >= 0,
            bank_loaded: snapshot.bank_loaded(),
            bank_generation: snapshot.bank_session_generation(),
            script_lifecycle: None,
            levels,
            effective_levels,
            xp,
            varps,
            chat,
            loc_facts,
            npc_facts,
            magic_tree_ready,
            dormant_rocks_seen,
            ground_loot,
            local_in_combat,
            taking_damage: snapshot.taking_damage(),
            local_target_npc,
            local_health,
            local_animation,
            guardian: BoundedGuardian::default(),
            equipment_ids,
            main_modal: snapshot.modals().main,
            widget_ids: snapshot
                .widgets()
                .iter()
                .map(|widget| widget.component_id)
                .collect(),
            shop_open: snapshot.shop().open,
            shop_stock: snapshot
                .shop()
                .stock
                .iter()
                .filter(|item| item.count > 0 && item.def.id >= 0)
                .take(16)
                .map(|item| BoundedShopItem {
                    id: item.def.id,
                    count: item.count,
                })
                .collect(),
            main_make_ids: snapshot
                .main_make()
                .iter()
                .filter(|item| item.def.id >= 0)
                .take(16)
                .map(|item| item.def.id)
                .collect(),
            route_inspect_seq: 0,
            route_inspect_generation: 0,
            route_inspect_request_id: 0,
            route_inspect_ok: false,
            route_inspect_reason: String::new(),
            route_inspect_hops: Vec::new(),
            route_inspect_live_generation: 0,
            route_inspect_has_terminal: false,
            los: LineOfSightObservation::default(),
            actor: ActorObservation::default(),
            fight: FightFieldObservation::default(),
            hunt: HuntObservation::default(),
        }
    }

    pub fn attach_route_inspect(&mut self, published: RouteInspectPublished) {
        self.route_inspect_live_generation = published.live_generation;
        self.route_inspect_has_terminal = published.has_terminal;
        self.route_inspect_seq = published.seq;
        self.route_inspect_generation = published.generation;
        self.route_inspect_request_id = published.request_id;
        self.route_inspect_ok = published.ok;
        self.route_inspect_reason = published.reason;
        self.route_inspect_hops = published
            .hop_loc_names
            .into_iter()
            .map(|loc_name| RouteInspectHopFact { loc_name })
            .collect();
    }

    pub fn item(&self, name: &str) -> i32 {
        self.items.get(name).copied().unwrap_or(0)
    }

    pub fn skill_xp(&self, name: &str) -> i32 {
        self.xp.get(name).copied().unwrap_or(0)
    }

    pub fn item_id(&self, id: i32) -> i32 {
        self.item_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn bank_item(&self, name: &str) -> i32 {
        self.bank.get(name).copied().unwrap_or(0)
    }

    pub fn bank_item_id(&self, id: i32) -> i32 {
        self.bank_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn level(&self, name: &str) -> i32 {
        self.levels.get(name).copied().unwrap_or(0)
    }

    pub fn effective_level(&self, name: &str) -> i32 {
        self.effective_levels.get(name).copied().unwrap_or(0)
    }

    pub fn varp(&self, index: i32) -> i32 {
        self.varps.get(&index).copied().unwrap_or(0)
    }

    /// Copy varps 83..=97 only. Absent snapshot rows stay absent (not 0).
    pub fn attach_prayer_varps(&mut self, snapshot: &GameSnapshot) {
        let first = api::prayer::PRAYER_VARP0;
        let last = first + api::prayer::PRAYER_COUNT as i32 - 1;
        for varp in snapshot.varps() {
            if (first..=last).contains(&varp.index) {
                self.varps.insert(varp.index, varp.value);
            }
        }
    }

    /// Compact current-plane identity, independently selected one-step pairs,
    /// and the script paint receipt. Never copies the collision grid.
    pub fn attach_line_of_sight(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let here_flag = here.and_then(|tile| collision_flag_at(scene, tile));
        let (host_open, host_blocked, fixture_failure) = match here {
            Some(tile) if scene.available => match select_line_of_sight_pairs(tile, |x, z| {
                collision_flag_at(
                    scene,
                    LineOfSightTile {
                        x,
                        z,
                        level: tile.level,
                    },
                )
            }) {
                Ok((open, blocked)) => (Some(open), Some(blocked), None),
                Err(msg) => (None, None, Some(msg)),
            },
            _ => (None, None, None),
        };
        self.los = LineOfSightObservation {
            available: scene.available,
            identity,
            here,
            here_flag,
            host_open,
            host_blocked,
            fixture_failure,
            receipt: paint.and_then(parse_los_receipt_from_paint),
        };
    }

    /// Compact host identity, one size>=1 NPC, existing LOS helper, and the
    /// script paint receipt. Never copies the NPC table or collision grid.
    pub fn attach_actor_observation(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let receipt = paint.and_then(parse_actor_receipt_from_paint);
        let npc = choose_actor_observation_npc(snapshot.npcs(), receipt.as_ref()).map(|row| {
            ActorObservationNpc {
                index: row.index as i32,
                name: row.name.clone(),
                size: row.size,
                tile_x: row.tile.x,
                tile_z: row.tile.z,
                nx: row.network.x,
                nz: row.network.z,
                level: row.tile.level,
            }
        });
        let (self_target_kind, self_target_index) = packed_self_target(snapshot);
        let host_los = match (here, npc.as_ref()) {
            (Some(from), Some(npc)) if scene.available => {
                let query = CollisionQuery {
                    available: scene.available,
                    base_x: scene.base_x,
                    base_z: scene.base_z,
                    level: scene.level,
                    width: scene.width,
                    height: scene.height,
                    flags: Arc::from(scene.collision_flags.as_slice()),
                };
                line_of_sight_v2(
                    Some(&query),
                    WorldTile {
                        x: from.x,
                        z: from.z,
                        level: from.level,
                    },
                    WorldTile {
                        x: npc.nx,
                        z: npc.nz,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok()
            }
            _ => None,
        };
        self.actor = ActorObservation {
            available: scene.available,
            identity,
            here,
            npc,
            host_los,
            self_target_kind,
            self_target_index,
            receipt,
        };
    }

    /// Compact host identity, one size>=1 NPC, both existing LOS helper
    /// results, and the script paint receipt. Never copies the NPC table.
    pub fn attach_fight_field(
        &mut self,
        snapshot: &GameSnapshot,
        paint: Option<&script::shim::ScriptPaint>,
    ) {
        let scene = snapshot.scene();
        let identity = LineOfSightIdentity {
            base_x: scene.base_x,
            base_z: scene.base_z,
            level: scene.level,
            width: scene.width,
            height: scene.height,
        };
        let here = self
            .tile
            .map(|(x, z, level)| LineOfSightTile { x, z, level });
        let receipt = paint.and_then(parse_fight_field_receipt_from_paint);
        let npc =
            choose_fight_field_npc(snapshot.npcs(), receipt.as_ref()).map(|row| FightFieldNpc {
                index: row.index as i32,
                size: row.size,
                tile_x: row.tile.x,
                tile_z: row.tile.z,
                nx: row.network.x,
                nz: row.network.z,
                level: row.tile.level,
            });
        let query = if scene.available {
            Some(CollisionQuery {
                available: scene.available,
                base_x: scene.base_x,
                base_z: scene.base_z,
                level: scene.level,
                width: scene.width,
                height: scene.height,
                flags: Arc::from(scene.collision_flags.as_slice()),
            })
        } else {
            None
        };
        let (host_los_network, host_los_tile) = match (here, npc.as_ref(), query.as_ref()) {
            (Some(from), Some(npc), Some(query)) => {
                let from_tile = WorldTile {
                    x: from.x,
                    z: from.z,
                    level: from.level,
                };
                let network = line_of_sight_v2(
                    Some(query),
                    from_tile,
                    WorldTile {
                        x: npc.nx,
                        z: npc.nz,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok();
                let tile = line_of_sight_v2(
                    Some(query),
                    from_tile,
                    WorldTile {
                        x: npc.tile_x,
                        z: npc.tile_z,
                        level: npc.level,
                    },
                    Some(npc.size),
                )
                .ok();
                (network, tile)
            }
            _ => (None, None),
        };
        self.fight = FightFieldObservation {
            available: scene.available,
            identity,
            here,
            npc,
            host_los_network,
            host_los_tile,
            receipt,
        };
    }

    pub fn equipment_id(&self, id: i32) -> i32 {
        self.equipment_ids.get(&id).copied().unwrap_or(0)
    }

    pub fn has_widget(&self, id: i32) -> bool {
        self.widget_ids.contains(&id)
    }

    pub fn shop_item_id(&self, id: i32) -> i32 {
        self.shop_stock
            .iter()
            .find(|row| row.id == id)
            .map(|row| row.count)
            .unwrap_or(0)
    }

    pub fn has_main_make(&self, id: i32) -> bool {
        self.main_make_ids.contains(&id)
    }
}

pub fn combat_npc_name(name: Option<&str>) -> bool {
    matches!(
        name.map(str::trim),
        Some("Chaos druid" | "Moss giant" | "Giant" | "Guard")
    )
}

/// One compact combat/fact record from the published npc family. The
/// `targeting_local` flag is recomputed here rather than copied, so a decoded
/// face target is the only source of that claim.
fn bounded_combat_npc(npc: &NpcView, self_slot: i32) -> BoundedNpc {
    BoundedNpc {
        index: npc.index,
        name: npc.name.clone(),
        health: npc.health,
        total_health: npc.total_health,
        animation: npc.animation,
        in_combat: npc.in_combat,
        targeting_local: npc.target.is_some_and(|target| {
            target.kind == ActorKind::Player && self_slot >= 0 && target.index == self_slot as usize
        }),
        tile: (npc.tile.x, npc.tile.z, npc.tile.level),
        distance: npc.distance,
    }
}

/// A dormant RockCrab `Rocks` parked inside the supported field. This is the
/// field predicate the scoped stand is audited against; it is identity
/// evidence for the activation witness, never combat evidence on its own.
fn dormant_rock_in_field(npc: &NpcView) -> bool {
    npc.name.as_deref() == Some(ROCK_CRAB_DORMANT_NAME)
        && npc.tile.level == ROCK_CRAB_SPOT.2
        && (npc.tile.x - ROCK_CRAB_SPOT.0)
            .abs()
            .max((npc.tile.z - ROCK_CRAB_SPOT.1).abs())
            <= ROCK_CRAB_FIELD_RADIUS
}

pub fn unidentified_herb_id(id: i32) -> bool {
    matches!(
        id,
        199 | 201 | 203 | 205 | 207 | 209 | 211 | 213 | 215 | 217 | 219 | LANTADYME_HERB_ID
    )
}

pub fn noted_herb_id(id: i32) -> bool {
    matches!(
        id,
        200 | 202 | 204 | 206 | 208 | 210 | 212 | 214 | 216 | 218 | 220 | NOTED_LANTADYME_HERB_ID
    )
}

pub fn combat_ground_id(id: i32) -> bool {
    unidentified_herb_id(id)
        || noted_herb_id(id)
        || matches!(
            id,
            NATURE_RUNE_ID
                | LAW_RUNE_ID
                | BIG_BONES_ID
                | NOTED_BIG_BONES_ID
                | LIMPWURT_ROOT_ID
                | NOTED_LIMPWURT_ROOT_ID
                | BONES_ID
                | NOTED_BONES_ID
        )
}

pub fn keep_bounded_loc(id: i32, name: Option<&str>) -> bool {
    matches!(
        id,
        WOODEN_DOOR_CLOSED_ID | WOODEN_DOOR_OPEN_ID | WOODEN_GATE_CLOSED_ID | WOODEN_GATE_OPEN_ID
    ) || name.is_some_and(|name| {
        let n = name.trim().to_ascii_lowercase();
        n == "door" || n.ends_with(" door") || n.contains("gate") || n == "fire"
    })
}

pub fn loc_name_matches(name: Option<&str>, gate: bool) -> bool {
    let n = name.unwrap_or("").trim().to_ascii_lowercase();
    if n.is_empty() {
        return false;
    }
    if gate {
        n.contains("gate")
    } else {
        n == "door" || n.ends_with(" door") || n.contains("gate")
    }
}

pub fn loc_at(
    observation: &Observation,
    id: i32,
    tile: (i32, i32, i32),
    radius: i32,
) -> Option<&BoundedLoc> {
    observation.loc_facts.iter().find(|loc| {
        loc.id == id
            && loc.level == tile.2
            && (loc.x - tile.0).abs().max((loc.z - tile.1).abs()) <= radius
    })
}

pub fn near(tile: Option<(i32, i32, i32)>, target: (i32, i32, i32), radius: i32) -> bool {
    tile.is_some_and(|tile| {
        tile.2 == target.2 && (tile.0 - target.0).abs().max((tile.1 - target.1).abs()) <= radius
    })
}

pub fn empty_pack(observation: &Observation) -> bool {
    observation.item_ids.values().copied().sum::<i32>() == 0
}
pub(super) fn empty_worn(observation: &Observation) -> bool {
    observation.equipment_ids.values().copied().sum::<i32>() == 0
}
pub fn held_id(observation: &Observation, id: i32) -> i32 {
    observation.item_id(id) + observation.equipment_id(id)
}
