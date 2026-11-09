use super::*;

#[path = "npc_377.rs"]
mod npc_377;

impl NpcView {
    fn from_slot(
        index: usize,
        npc: &ClientNpc,
        base: (i32, i32),
        level: i32,
        distance: i32,
        loop_cycle: i32,
        client: &Client,
    ) -> Self {
        let entity = &npc.entity;
        let cache = &client.cache;
        let placed = npc.r#type.and_then(|id| cache.npcs.get(id));
        let is_377 = client.revision() == client::io::ClientRevision::R377;
        let form = if is_377 {
            npc.r#type.and_then(|id| npc_377::menu_form(client, id))
        } else {
            placed
        };
        let (name, actions) = match form {
            Some(form) => (Some(form.name.clone()), form.op.clone()),
            None => (None, Vec::new()),
        };
        let (npc_level, size) = if is_377 {
            (
                form.map(|form| form.vislevel.max(0)).unwrap_or(0),
                placed.map(|placed| placed.size).unwrap_or(entity.size),
            )
        } else {
            placed
                .map(|placed| (placed.vislevel.max(0), placed.size))
                .unwrap_or((0, entity.size))
        };
        NpcView {
            index,
            r#type: npc.r#type,
            name,
            actions,
            tile: entity_world_tile(entity, base, level),
            distance,
            animation: entity.primary_anim,
            animation_frame: entity.primary_anim_frame,
            pose_animation: entity.secondary_anim,
            orientation: entity.yaw,
            target_orientation: entity.dst_yaw,
            overhead_text: entity.chat_message.clone(),
            spot_animation: entity.spotanim_id,
            spot_animation_stamp: entity.spotanim_last_cycle,
            health: entity.health,
            total_health: entity.total_health,
            face_entity: entity.face_entity,
            target: decode_target(entity.face_entity),
            moving: entity.route_length > 0,
            running: entity.primary_anim == entity.runanim,
            in_combat: actor_in_combat(entity.combat_cycle, loop_cycle),
            level: npc_level,
            size,
            network: entity_network_tile(entity, base, level),
            x: entity.x,
            z: entity.z,
            yaw: entity.yaw,
        }
    }
}

impl GameSnapshot {
    pub(super) fn rebuild_stat(&mut self, client: &Client) -> bool {
        if !track(client.gens.stat, &mut self.gens.stat) {
            return false;
        }
        self.runenergy = client.runenergy;
        self.stats = (0..Skill::count)
            .map(|i| StatView {
                index: i as i32,
                name: Skill::names[i].to_string(),
                effective: client.stat_effective_level[i],
                base: client.stat_base_level[i],
                xp: client.stat_xp[i],
                used: Skill::used[i],
            })
            .collect();
        // The client publishes one generation for skill, energy, and weight
        // updates. Its packet history is shorter than the packets drained per
        // cycle, and skill values survive logout, so current values cannot
        // prove which skills received a stat update in this session.
        // Preserve the reader's documented no-readiness-signal fallback.
        self.stats_ready = false;
        true
    }

    /// Varp-family rebuild: the whole `Client.var` table, one view per
    /// definition (unset entries read 0), gated on the varp gen.
    pub(super) fn rebuild_varps(&mut self, client: &Client) -> bool {
        if !track(client.gens.varp, &mut self.gens.varp) {
            return false;
        }
        self.varps = (0..client.cache.varps.len())
            .map(|i| VarpView {
                index: i as i32,
                value: client.var.get(i).copied().unwrap_or(0),
            })
            .collect();
        true
    }

    /// Player-family rebuild: the scene origin, the local player's world
    /// tile (base + route head), the `LocalPlayerView`, and the remote
    /// `players` list. `REBUILD_NORMAL` bumps every gen, so a new world
    /// origin re-arms this too. Only `PLAYER_INFO` packet generations
    /// advance the game tick; other player-family invalidations only refresh
    /// the view.
    pub(super) fn rebuild_player(&mut self, client: &Client, player_info_ticks: u64) -> bool {
        if !self.family_observed(client, Family::Player) {
            return false;
        }
        if !track(client.gens.player, &mut self.gens.player) {
            return false;
        }
        self.players_available = true;
        if player_info_ticks != 0 {
            self.tick = self.tick.wrapping_add(player_info_ticks as u32);
        }
        if !client.ingame || client.local_player.is_none() {
            self.thieving_stun_tick = None;
            self.thieving_stun_stamp = None;
        } else if let Some(player) = client.local_player.as_ref() {
            // Packet decoder stamps every SPOTANIM, including a restarted
            // animation with the same ID. Visual expiry must not erase onset.
            if player.spotanim_id == 245
                && self.thieving_stun_stamp != Some(player.spotanim_last_cycle)
            {
                self.thieving_stun_tick = Some(self.tick);
                self.thieving_stun_stamp = Some(player.spotanim_last_cycle);
            }
        }
        self.self_slot = client.self_slot;
        let base = (client.map_build_base_x, client.map_build_base_z);
        if client.ingame && self.base.is_some_and(|previous| previous != base) {
            self.last_scene_rebuild_tick = Some(self.tick);
        }
        self.base = Some(base);
        let tile = client.local_player.as_ref().map(|lp| {
            (
                base.0 + lp.route_x[0],
                base.1 + lp.route_z[0],
                // The scene level (`minusedlevel`), like every actor view.
                client.minusedlevel,
            )
        });
        if !client.ingame || tile.is_none() {
            self.last_server_movement_tick = None;
        } else if player_info_ticks != 0 && self.tile.is_some() && self.tile != tile {
            self.last_server_movement_tick = Some(self.tick);
        }
        self.tile = tile;
        let level = client.minusedlevel;
        self.player = client.local_player.as_ref().map(|lp| LocalPlayerView {
            player: PlayerView {
                index: client.self_slot.max(0) as usize,
                network: entity_network_tile(&lp.entity, base, level),
                actor: actor_view(
                    &lp.entity,
                    base,
                    level,
                    0, // the local player's distance to itself
                    lp.name.clone(),
                    client.player_op.to_vec(),
                    client.loop_cycle,
                ),
                combat_level: lp.combat_level,
                skill_level: lp.skill_level,
                headicons: i32::from(lp.headicons),
                weapon: player_weapon(&lp.appearance),
            },
            energy: client.runenergy,
            weight: client.runweight,
            animation_update: client
                .local_animation_update()
                .map(|update| AnimationUpdateView {
                    serial: update.serial,
                    sequence: update.sequence,
                }),
        });
        self.players.clear();
        self.players.reserve(client.player_count as usize);
        let local_tile = local_world_tile(client);
        for i in 0..client.player_count as usize {
            let index = client.player_ids[i] as usize;
            if let Some(player) = client.players.get(index).and_then(|p| p.as_ref()) {
                let tile = entity_world_tile(&player.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.players.push(PlayerView {
                    index,
                    network: entity_network_tile(&player.entity, base, level),
                    actor: actor_view(
                        &player.entity,
                        base,
                        level,
                        distance,
                        player.name.clone(),
                        client.player_op.to_vec(),
                        client.loop_cycle,
                    ),
                    combat_level: player.combat_level,
                    skill_level: player.skill_level,
                    headicons: i32::from(player.headicons),
                    weapon: player_weapon(&player.appearance),
                });
            }
        }
        self.refresh_scene_distances(client);
        true
    }

    /// Refresh native observations that may change without a packet-family
    /// generation. Clone the overhead string only when its value changes.
    pub(super) fn refresh_native_facts(&mut self, client: &Client) {
        if !client.try_move_path.is_empty()
            && self.last_walk_path_local.as_slice() != client.try_move_path.as_slice()
        {
            self.last_walk_path_local.clone_from(&client.try_move_path);
        }
        let local_overhead_text = client
            .ingame
            .then(|| client.local_player.as_ref()?.entity.chat_message.as_deref())
            .flatten();
        if self.local_overhead_text.as_deref() != local_overhead_text {
            self.local_overhead_text = local_overhead_text.map(str::to_owned);
        }
        self.hint_tile = (client.ingame && client.hint_type == 2).then_some(HintTileView {
            x: client.hint_tile_x,
            z: client.hint_tile_z,
            level: client.minusedlevel,
        });
        // Hitmarks live on the local entity and expire against loop_cycle;
        // neither event bumps a packet family, so recompute every read.
        self.taking_damage = local_player_taking_damage(client);
        self.active_side_tab = client.active_icon;
        self.hitmarks = if client.ingame {
            client.local_player.as_ref().map(|player| HitmarksView {
                marks: std::array::from_fn(|index| HitmarkView {
                    value: player.entity.damage_values[index],
                    kind: player.entity.damage_types[index],
                    cycle: player.entity.damage_cycles[index],
                }),
                loop_cycle: client.loop_cycle,
            })
        } else {
            None
        };
        self.projectiles.clear();
        if client.ingame && client.self_slot >= 0 {
            let base = (client.map_build_base_x, client.map_build_base_z);
            let local_slot = client.self_slot as usize;
            let local_tile = local_world_tile(client);
            // Keep incoming threats first; outgoing candidates whose source tile is local
            // use remaining capacity and never displace them. ClientProj has no firing-player
            // index, so ranged-onset attribution must reject them when another player shares the tile.
            for incoming in [true, false] {
                if self.projectiles.len() >= MAX_PROJECTILES_PER_SNAPSHOT {
                    break;
                }
                client.projectiles.for_each(|projectile| {
                    if self.projectiles.len() >= MAX_PROJECTILES_PER_SNAPSHOT {
                        return;
                    }
                    let target = decode_projectile_target(projectile.target);
                    let targets_local = target.is_some_and(|target| {
                        target.kind == ActorKind::Player && target.index == local_slot
                    });
                    if targets_local != incoming {
                        return;
                    }
                    let src = projectile_source_tile(
                        projectile.src_x,
                        projectile.src_z,
                        projectile.level,
                        base,
                    );
                    if !incoming
                        && (target.is_none()
                            || Some((src.x, src.z)) != local_tile
                            || src.level != client.minusedlevel)
                    {
                        return;
                    }
                    self.projectiles.push(ProjectileView {
                        spotanim: projectile.spotanim,
                        level: projectile.level,
                        src,
                        target,
                        t1: projectile.t1,
                        t2: projectile.t2,
                    });
                });
            }
        }
    }

    /// Refresh cached spatial distances only when the local origin moves.
    /// Definitions, actions and vector allocations stay put.
    fn refresh_scene_distances(&mut self, client: &Client) {
        let origin = local_world_tile(client);
        if origin == self.scene_distance_tile {
            return;
        }
        self.scene_distance_tile = origin;
        let Some((lx, lz)) = origin else {
            return;
        };
        for loc in &mut self.loc {
            loc.distance = chebyshev(loc.tile.x, loc.tile.z, lx, lz);
        }
        for item in &mut self.ground_item {
            item.distance = chebyshev(item.tile.x, item.tile.z, lx, lz);
        }
    }

    /// Inv-family rebuild: zip the TYPE_INV iface's obj ids/counts. The
    /// iface stores `obj_id + 1` (0 = empty), so the view carries the
    /// real obj ids — the same convention as `ItemView.def.id`, the
    /// `ObjNames` table and the evidence/`Proof::Item` consumers.
    pub(super) fn rebuild_inv(&mut self, client: &Client) -> bool {
        if !track(client.gens.inv, &mut self.gens.inv) {
            return false;
        }
        self.inv_session_current = true;
        self.inv.clear();
        // The live inv is the side-tab-3 TYPE_INV container (the same the
        // `inventory()` family reads); a naive first-TYPE_INV scan can
        // pick an unrelated empty container (a shop/trade modal) and fail
        // the nav `WorldState` gate closed. Tests/stub clients without a
        // side tab fall back to the first TYPE_INV that actually holds
        // decoded slots, else the first TYPE_INV in the table. The bank's
        // withdraw grid is never the backpack: with no side tab bound it
        // would otherwise read the banked objs as carried.
        let inv = tab_inv_component(client, 3)
            .and_then(|id| client.if_(id as usize))
            .or_else(|| {
                let mut first = None;
                for com in client
                    .ifaces_merged()
                    .filter(|f| f.r#type == ComponentType::TYPE_INV && !is_bank_withdraw_grid(f))
                {
                    if com
                        .link_obj_type
                        .as_ref()
                        .is_some_and(|ids| ids.iter().any(|id| *id > 0))
                    {
                        return Some(com);
                    }
                    if first.is_none() {
                        first = Some(com);
                    }
                }
                first
            });
        if let Some(inv) = inv {
            if let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) {
                self.inv = ids
                    .iter()
                    .zip(counts)
                    .filter(|(id, _)| **id > 0)
                    .map(|(id, n)| (*id - 1, *n))
                    .collect();
            }
        }
        true
    }

    /// Chat-family rebuild: the ring head (`chat_text[0]`) is the most
    /// recent message, and the full ring becomes the `chat_lines` view
    /// (index 0 = newest). Each line's `sequence` is the client's own
    /// per-message counter (`chat_seq` bumps once per `add_chat`), so a
    /// burst in one gen bump still gets distinct sequences that only move
    /// forward.
    pub(super) fn rebuild_chat(&mut self, client: &Client) -> bool {
        if !track(client.gens.chat, &mut self.gens.chat) {
            return false;
        }
        let latest = client.chat_text[0].clone();
        self.chat = (!latest.is_empty()).then_some(latest);
        let head_seq = client.chat_seq as i32;
        self.chat_lines.clear();
        for i in 0..100 {
            let text = client.chat_text[i].clone();
            if text.is_empty() {
                break; // the ring is dense from the head (m8aq stops at the first hole)
            }
            self.chat_lines.push(ChatLineView {
                type_: client.chat_type[i],
                username: (!client.chat_username[i].is_empty())
                    .then(|| client.chat_username[i].clone()),
                text,
                sequence: head_seq - i as i32,
            });
        }
        true
    }

    /// Inventory rebuild: the inv tab's (side tab 3) TYPE_INV component,
    /// with held ops from the obj defs. Gated on the iface + inv gens.
    pub(super) fn rebuild_inventory(&mut self, client: &Client) -> bool {
        if !self.inventory_gate.moved(client, self.inv_session_current) {
            return false;
        }
        self.inventory.clear();
        self.inventory_packet_generation = None;
        self.inventory_ready = false;
        let Some(inv_id) = tab_inv_component(client, 3) else {
            self.inventory_size = 0;
            return true;
        };
        let state = client.inventory_packet_state(inv_id);
        self.inventory_packet_generation = state.map(|state| state.generation);
        self.inventory_ready =
            state.is_some_and(|state| state.transmitting && state.full_generation > 0);
        let Some(inv) = client.if_(inv_id as usize) else {
            self.inventory_size = 0;
            return true;
        };
        self.inventory_size = inv
            .link_obj_type
            .as_ref()
            .map(|ids| ids.len() as i32)
            .unwrap_or(0);
        let (Some(ids), Some(counts)) = (&inv.link_obj_type, &inv.link_obj_number) else {
            return true;
        };
        for (slot, stored) in ids
            .iter()
            .copied()
            .enumerate()
            .take(ids.len().min(counts.len()))
        {
            if stored <= 0 {
                continue;
            }
            let id = stored - 1;
            if let Some(view) = item_view(
                &client.cache,
                &inv,
                slot,
                ItemContainer::Inventory,
                ItemActionFamily::Held,
                cache_held_ops(&client.cache, id),
            ) {
                self.inventory.push(view);
            }
        }
        true
    }

    /// Equipment rebuild: the worn-items tab's (side tab 4) TYPE_INV
    /// component with its own interface ops.
    pub(super) fn rebuild_equipment(&mut self, client: &Client) -> bool {
        if !self.equipment_gate.moved(client, self.inv_session_current) {
            return false;
        }
        let equipment_component = tab_inv_component(client, 4);
        self.equipment_packet_generation = equipment_component
            .and_then(|id| client.inventory_packet_state(id))
            .map(|state| state.generation);
        let posted = equipment_component
            .and_then(|id| client.if_(id as usize))
            .is_some_and(|com| match (&com.link_obj_type, &com.link_obj_number) {
                (Some(ids), Some(counts)) => !ids.is_empty() && ids.len() == counts.len(),
                _ => false,
            });
        self.equipment_available = posted;
        self.equipment = if posted {
            tab_inv_component(client, 4)
                .and_then(|id| inv_items(client, id, ItemContainer::Equipment))
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        true
    }

    /// Bank rebuild: the open main modal's withdraw component (m8aq
    /// `bankItems`).
    pub(super) fn rebuild_bank(&mut self, client: &Client) -> bool {
        if !self.bank_gate.moved(client, self.inv_session_current) {
            return false;
        }
        let bank_component_id = if client.main_modal_id == -1 {
            -1
        } else {
            find_inv_component(client, client.main_modal_id, is_bank_withdraw_grid).unwrap_or(-1)
        };
        let modal = client.main_modal_packet_state();
        let modal_delta = modal
            .generation
            .wrapping_sub(self.bank_modal_generation_seen);
        if modal_delta != 0 {
            self.bank_session_generation = self.bank_session_generation.wrapping_add(modal_delta);
            self.bank_modal_generation_seen = modal.generation;
        } else if bank_component_id != self.bank_component_id {
            self.bank_session_generation = self.bank_session_generation.wrapping_add(1);
        }

        let track_com = if bank_component_id >= 0 {
            bank_component_id
        } else {
            self.bank_component_id
        };
        if track_com >= 0 {
            if let Some(state) = client.inventory_packet_state(track_com) {
                self.note_bank_inv_state(
                    track_com,
                    state.generation,
                    state.full_generation,
                    state.full_observation,
                    state.transmitting,
                );
            } else if self.bank_full_com != track_com {
                self.bank_full_generation = 0;
                self.bank_last_inv_com = -1;
                self.bank_last_inv_generation = 0;
                self.bank_last_full_observation = 0;
            }
        }

        if bank_component_id < 0 {
            if self.bank_inventory_session.is_some() || self.bank_component_id >= 0 {
                let com = if self.bank_component_id >= 0 {
                    self.bank_component_id
                } else {
                    self.bank_inventory_session
                        .map(|session| session.main_com_id)
                        .unwrap_or(-1)
                };
                let generation = if self.bank_last_inv_com == com {
                    self.bank_last_inv_generation
                } else {
                    0
                };
                self.close_bank_inv_session(com, generation);
            }
        } else {
            let same_com = bank_component_id == self.bank_component_id;
            let closed_then_opened = same_com
                && self.bank_component_id >= 0
                && modal_delta != 0
                && modal.closed_observation != 0
                && modal.closed_observation < modal.opened_observation;
            if closed_then_opened {
                let generation = if self.bank_last_inv_com == bank_component_id {
                    self.bank_last_inv_generation
                } else {
                    0
                };
                self.close_bank_inv_session(bank_component_id, generation);
            } else if !same_com {
                if let Some(session) = self.bank_inventory_session {
                    let generation = if self.bank_last_inv_com == session.main_com_id {
                        self.bank_last_inv_generation
                    } else {
                        0
                    };
                    self.close_bank_inv_session(session.main_com_id, generation);
                }
            }
            let needs_open = !matches!(self.bank_inventory_session, Some(session) if session.main_com_id == bank_component_id);
            if needs_open {
                self.open_bank_inv_session(bank_component_id);
            }
        }

        self.bank_component_id = bank_component_id;
        self.bank_loaded = match self.bank_inventory_session {
            Some(session) if session.main_com_id == bank_component_id => client
                .inventory_packet_state(bank_component_id)
                .is_some_and(|state| {
                    state.transmitting
                        && self.bank_full_generation > 0
                        && self.bank_full_generation >= session.main_opened_at
                }),
            _ => false,
        };
        self.bank = if self.bank_component_id == -1 {
            Vec::new()
        } else {
            inv_items(client, self.bank_component_id, ItemContainer::Bank).unwrap_or_default()
        };
        self.bank_size = if self.bank_component_id == -1 {
            0
        } else {
            client
                .if_(self.bank_component_id as usize)
                .and_then(|com| com.link_obj_type.as_ref().map(|ids| ids.len() as i32))
                .unwrap_or(0)
        };
        self.bank_note_controls = if client.main_modal_id == -1 {
            None
        } else {
            bank_note_controls(client, client.main_modal_id)
        };
        true
    }

    fn note_bank_inv_state(
        &mut self,
        com_id: i32,
        generation: u64,
        full_generation: u64,
        full_observation: u64,
        transmitting: bool,
    ) {
        if !transmitting {
            self.bank_full_generation = 0;
        } else if self.bank_last_inv_com != com_id
            || self.bank_last_full_observation != full_observation
        {
            self.bank_full_generation = if full_generation > 0 { generation } else { 0 };
        }
        self.bank_full_com = com_id;
        self.bank_last_inv_com = com_id;
        self.bank_last_inv_generation = generation;
        self.bank_last_full_observation = full_observation;
    }

    fn close_bank_inv_session(&mut self, com_id: i32, generation: u64) {
        if com_id >= 0 {
            self.bank_prev_inv_com = com_id;
            self.bank_prev_inv_generation = generation;
        }
        // The side component closes with the session (`bank.rs2:25-27`); a
        // later open counts only side updates newer than this generation.
        if self.bank_side_com >= 0 {
            self.bank_prev_side_com = self.bank_side_com;
            self.bank_prev_side_generation = self.bank_side_last_generation;
        }
        self.bank_side_ready = false;
        self.bank_inventory_session = None;
    }

    pub(super) fn open_bank_inv_session(&mut self, com_id: i32) {
        let main_opened_at = if self.bank_prev_inv_com == com_id {
            self.bank_prev_inv_generation
        } else {
            0
        };
        self.bank_inventory_session = Some(BankInvSession {
            main_com_id: com_id,
            main_opened_at,
        });
    }

    /// Bank-side rebuild: the open side modal's deposit component (m8aq
    /// `bankSideItems`).
    pub(super) fn rebuild_bank_side(&mut self, client: &Client) -> bool {
        if !self.bank_side_gate.moved(client, self.inv_session_current) {
            return false;
        }
        self.bank_side_packet_generation = None;
        self.bank_side_ready = false;
        if client.side_modal_id == -1 {
            self.bank_side = Vec::new();
        } else {
            let root = client.side_modal_id;
            let component = find_inv_component(client, root, |com| {
                com.iop[0]
                    .as_deref()
                    .is_some_and(|s| s.to_ascii_lowercase().contains("deposit"))
            });
            let state = component.and_then(|com_id| {
                client
                    .inventory_packet_state(com_id)
                    .map(|state| (com_id, state))
            });
            self.bank_side_packet_generation = state.map(|(_, state)| state.generation);
            if let Some((com_id, state)) = state {
                self.note_bank_side_state(com_id, state);
            }
            self.bank_side = component
                .and_then(|com_id| inv_items(client, com_id, ItemContainer::BankSide))
                .unwrap_or_default();
        }
        true
    }

    /// Side readiness mirrors the main table's session rule: the side
    /// component transmits, and its last full update is at least the side
    /// generation recorded when the previous bank session closed. The content
    /// sends that full update with every opening (`bank.rs2:20`).
    fn note_bank_side_state(
        &mut self,
        com_id: i32,
        state: client::client::client::InventoryPacketState,
    ) {
        if !state.transmitting {
            self.bank_side_full_generation = 0;
        } else if self.bank_side_com != com_id
            || self.bank_side_last_full_observation != state.full_observation
        {
            self.bank_side_full_generation = if state.full_generation > 0 {
                state.generation
            } else {
                0
            };
        }
        self.bank_side_com = com_id;
        self.bank_side_last_generation = state.generation;
        self.bank_side_last_full_observation = state.full_observation;
        let opened_at = if self.bank_prev_side_com == com_id {
            self.bank_prev_side_generation
        } else {
            0
        };
        self.bank_side_ready = self.bank_inventory_session.is_some()
            && state.transmitting
            && self.bank_side_full_generation > 0
            && self.bank_side_full_generation >= opened_at;
    }

    /// Trade rebuild: the 274 trade ifaces' state and containers. The
    /// component ids are baked by the packed interface table, so the
    /// reads work whether or not a trade is open (m8aq reads the same
    /// hardcoded ids).
    pub(super) fn rebuild_trade(&mut self, client: &Client) -> bool {
        if !self.trade_gate.moved(client, self.inv_session_current) {
            return false;
        }
        let my_offer = inv_items(client, TRADEMAIN_INV, ItemContainer::TradeMyOffer);
        let my_offer_packet_state = client.inventory_packet_state(TRADEMAIN_INV);
        let my_offer_packet_generation = my_offer_packet_state.map(|state| state.generation);
        let my_offer_available =
            my_offer.is_some() && my_offer_packet_state.is_some_and(|state| state.transmitting);
        let my_offer_stopped =
            my_offer.is_some() && my_offer_packet_state.is_some_and(|state| !state.transmitting);
        let my_offer = my_offer.unwrap_or_default();
        let their_offer = inv_items(client, TRADEMAIN_OTHER_INV, ItemContainer::TradeTheirOffer);
        let their_offer_packet_state = client.inventory_packet_state(TRADEMAIN_OTHER_INV);
        let their_offer_packet_generation = their_offer_packet_state.map(|state| state.generation);
        let their_offer_available = their_offer.is_some()
            && their_offer_packet_state.is_some_and(|state| state.transmitting);
        let their_offer_stopped = their_offer.is_some()
            && their_offer_packet_state.is_some_and(|state| !state.transmitting);
        let their_offer = their_offer
            .unwrap_or_default()
            .into_iter()
            .map(|mut item| {
                item.action_family = ItemActionFamily::None;
                item
            })
            .collect();
        let side_pack =
            inv_items(client, TRADESIDE_INV, ItemContainer::TradeSidePack).unwrap_or_default();
        let (accept_component_id, decline_component_id) = trade_controls(client);
        let confirm_open = client.main_modal_id == TRADECONFIRM;
        // The engine stops the offer containers at the mutual accept
        // (`trade.rs2:80-86`) and the client empties a stopped container, so
        // the confirm lists are the only record of the agreed trade.
        let confirm = confirm_open.then(|| {
            Box::new(TradeConfirmView {
                mine: confirm_rows(
                    client,
                    TRADECONFIRM_INV1,
                    TRADECONFIRM_INV2,
                    ItemContainer::TradeMyOffer,
                ),
                theirs: confirm_rows(
                    client,
                    TRADECONFIRM_OTHER_INV1,
                    TRADECONFIRM_OTHER_INV2,
                    ItemContainer::TradeTheirOffer,
                ),
            })
        });
        self.trade = TradeView {
            offer_open: client.main_modal_id == TRADEMAIN,
            confirm_open,
            my_offer,
            their_offer,
            side_pack,
            partner: trade_partner(client),
            accept_component_id,
            decline_component_id,
            interface_generation: client.gens.iface,
            inventory_generation: client.gens.inv,
            modal_generation: client.main_modal_packet_state().generation,
            session_generation: client.gens.session,
            my_offer_packet_generation,
            their_offer_packet_generation,
            my_offer_stopped,
            their_offer_stopped,
            my_offer_available,
            their_offer_available,
            confirm,
        };
        true
    }

    /// Shop rebuild: the packed shop main modal's stock TYPE_INV plus the
    /// shop side interface's player pack TYPE_INV. Both are empty while the
    /// matching root is not open — never the backpack.
    pub(super) fn rebuild_shop(&mut self, client: &Client) -> bool {
        if !self.shop_gate.moved(client, self.inv_session_current) {
            return false;
        }
        let open = client.main_modal_id == SHOPMAIN;
        let player = (open && client.side_modal_id == SHOP_SIDE)
            .then(|| inv_items(client, SHOP_SIDE_INV, ItemContainer::ShopPlayer))
            .flatten();
        self.shop = ShopView {
            open,
            server_closes: client.main_modal_packet_state().close_packets,
            stock: if open {
                inv_items(client, SHOP_STOCK_INV, ItemContainer::ShopStock).unwrap_or_default()
            } else {
                Vec::new()
            },
            // The player pack is the shop *side* interface (3822 → 3823),
            // opened with the main modal by `if_openmain_side`. Sell needs
            // this container, not the stock rows and not the backpack.
            player: player.clone().unwrap_or_default(),
            player_available: player.is_some(),
        };
        true
    }

    /// Widgets rebuild: walk every open root's tree into `WidgetView`s.
    /// Gated on the iface gen (tree/component state) and the inv gen
    /// (TYPE_INV slot contents).
    pub(super) fn rebuild_widgets(&mut self, client: &Client) -> bool {
        if !self.widgets_gate.moved(client, self.inv_session_current) {
            return false;
        }
        self.widgets.clear();
        let roots = widget_roots(client);
        let mut visited = vec![false; client.ifaces_len()];
        for (root_id, root) in roots {
            walk_widget_tree(client, root_id, root, &mut visited, &mut self.widgets);
        }
        true
    }

    /// Side-tabs rebuild: all 14 slots with the tab state and each
    /// available tab's widget tree.
    pub(super) fn rebuild_side_tabs(&mut self, client: &Client) -> bool {
        if !self.side_tabs_gate.moved(client, self.inv_session_current) {
            return false;
        }
        self.side_tabs.clear();
        let mut visited = vec![false; client.ifaces_len()];
        for index in 0..client.side_icon.len() {
            let root_component_id = client.side_icon[index];
            let available = root_component_id != -1;
            let active = client.active_icon == index as i32;
            let mut widgets = Vec::new();
            if available {
                walk_widget_tree(
                    client,
                    root_component_id,
                    WidgetRoot::Side,
                    &mut visited,
                    &mut widgets,
                );
            }
            self.side_tabs.push(SideTabView {
                index: index as i32,
                root_component_id,
                available,
                active,
                visible: active && client.side_modal_id == -1 && available,
                widgets,
            });
        }
        self.side_tabs_available = true;
        true
    }

    /// Chat-options rebuild: the chat modal's BUTTON_OK choices and its
    /// BUTTON_CONTINUE component (the m8aq `chatOptions`/continue).
    pub(super) fn rebuild_chat_options(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.chat_options_gate) {
            return false;
        }
        self.chat_options.clear();
        self.chat_continue_component_id = -1;
        if client.chat_modal_id == -1 {
            return true;
        }
        let root = client.chat_modal_id;
        // Walk the chat tree: level-up IFs nest `buttontype=pause` under a
        // layer, so a direct-child scan misses `advancestat`'s continue.
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.button_type == ButtonType::BUTTON_CONTINUE
                && self.chat_continue_component_id == -1
            {
                self.chat_continue_component_id = id;
            }
            if com.button_type == ButtonType::BUTTON_OK {
                let label = if !com.text.is_empty() {
                    Some(com.text.to_string())
                } else {
                    non_empty(&com.button_text)
                };
                if let Some(text) = label {
                    self.chat_options.push(ChatOptionView {
                        component_id: id,
                        text,
                    });
                }
            }
            queue.extend(children_of(&com));
        }
        if client.resumed_pause_button {
            self.chat_continue_component_id = -1;
        }
        true
    }

    /// Make-products rebuild: the chat (or main) modal's obj-model
    /// components as products with their make/smelt buttons grouped four
    /// per product (m8aq `makeProducts`).
    pub(super) fn rebuild_make_products(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.make_products_gate) {
            return false;
        }
        self.make_products.clear();
        let root = if client.chat_modal_id != -1 {
            client.chat_modal_id
        } else {
            client.main_modal_id
        };
        if root == -1 {
            return true;
        }
        let mut objs: Vec<i32> = Vec::new();
        let mut buttons: Vec<MakeButtonView> = Vec::new();
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.model1_type == 4 && com.model1_id > 0 {
                objs.push(com.model1_id);
            }
            if com.button_type == ButtonType::BUTTON_OK {
                if let Some(quantity) = make_quantity(&com.button_text) {
                    buttons.push(MakeButtonView {
                        quantity,
                        component_id: id,
                    });
                }
            }
            queue.extend(children_of(&com));
        }
        // Make-X groups four quantity buttons per obj-model. A modal can
        // carry TYPE_MODEL objs with no Make/Smelt buttons (the
        // mysterious-cube random event) — do not invent products, and never
        // slice `buttons[i*4..]` past the end.
        if buttons.is_empty() {
            return true;
        }
        for (i, obj) in objs.iter().enumerate() {
            let name = client
                .cache
                .objs
                .get(*obj as usize)
                .map(|o| o.name.clone())
                .unwrap_or_default();
            let start = i * 4;
            let chunk = if start >= buttons.len() {
                Vec::new()
            } else {
                let end = (start + 4).min(buttons.len());
                buttons[start..end].to_vec()
            };
            self.make_products.push(MakeProductView {
                object_id: *obj,
                name,
                buttons: chunk,
            });
        }
        true
    }

    /// Main-make rebuild: TYPE_INV children of the open main modal whose
    /// component ops start with Make (the anvil skill-multi panel). Chat
    /// make-products stay on `rebuild_make_products`; a shop/trade main
    /// modal has Buy/Sell/Remove ops and yields no rows.
    pub(super) fn rebuild_main_make(&mut self, client: &Client) -> bool {
        if !self.main_make_gate.moved(client, self.inv_session_current) {
            return false;
        }
        self.main_make.clear();
        let root = client.main_modal_id;
        if root == -1 {
            return true;
        }
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.r#type == ComponentType::TYPE_INV
                && com
                    .iop
                    .iter()
                    .flatten()
                    .any(|op| op.to_ascii_lowercase().starts_with("make"))
            {
                self.main_make.extend(read_inv_component(
                    &client.cache,
                    &com,
                    ItemContainer::MainMake,
                ));
            }
            queue.extend(children_of(&com));
        }
        true
    }

    /// Quest-statuses rebuild: the quest tab's (side tab 2) TYPE_TEXT
    /// rows with their colours (m8aq `questStatuses`).
    pub(super) fn rebuild_quest_statuses(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.quest_statuses_gate) {
            return false;
        }
        self.quest_statuses.clear();
        self.quest_statuses_available = false;
        let Some(root) = client.side_icon.get(2).copied() else {
            return true;
        };
        if root == -1 {
            return true;
        }
        if client.if_(root as usize).is_none() {
            return true;
        }
        self.quest_statuses_available = true;
        let mut queue = vec![root];
        let mut head = 0;
        while head < queue.len() {
            let id = queue[head];
            head += 1;
            let Some(com) = client.if_(id as usize) else {
                continue;
            };
            if com.r#type == ComponentType::TYPE_TEXT && !com.text.is_empty() {
                self.quest_statuses.push(QuestStatusView {
                    component_id: id,
                    name: com.text.to_string(),
                    colour: com.colour,
                });
            }
            queue.extend(children_of(&com));
        }
        true
    }

    /// Puzzle-board refresh: the open main modal's first depth-first
    /// TYPE_INV with `obj_ops` (`find_inv_component` — the same helper the
    /// bank's withdraw component uses; the hint panel is excluded because it
    /// has no `obj_ops`). The board's identity and its `link_obj_type` slot
    /// count are observed here, beside `main_modal_texts` (slot 248: the
    /// board is slot 250 of the same observation); the rows are read from
    /// the identified widget the widgets family already walked, never from
    /// a world copy.
    ///
    /// The session generation advances on a session open, a session close or
    /// a new board component — never on a piece move, so a stale generation
    /// stays a publishable observation rather than a packet verdict.
    fn refresh_puzzle_board(&mut self, client: &Client) {
        let open = client.main_modal_id != -1;
        let component_id = if open {
            find_inv_component(client, client.main_modal_id, |com| com.obj_ops).unwrap_or(-1)
        } else {
            -1
        };
        if open != self.puzzle_board_open {
            self.puzzle_board_open = open;
            self.puzzle_session_generation = self.puzzle_session_generation.wrapping_add(1);
        } else if component_id != self.puzzle_board_component_id {
            self.puzzle_session_generation = self.puzzle_session_generation.wrapping_add(1);
        }
        self.puzzle_board_component_id = component_id;
        self.puzzle_board_size = if component_id == -1 {
            0
        } else {
            client
                .if_(component_id as usize)
                .and_then(|com| com.link_obj_type.as_ref().map(|ids| ids.len() as i32))
                .unwrap_or(0)
        };
    }

    /// Controls rebuild: the run/retaliate toggle pairs from the
    /// player-controls overlay (a table scan — the overlay is a side tab,
    /// so its root is not among the widget roots).
    pub(super) fn rebuild_controls(&mut self, client: &Client) -> bool {
        if !track(client.gens.iface, &mut self.controls_gate) {
            return false;
        }
        self.run_controls = controls_pair(client, 5, 5, 4);
        self.retaliate_controls = controls_pair(client, 3, 2, 3);
        true
    }

    /// Modals rebuild: the four open roots plus the modal scalars. The
    /// scalar fields copy every rebuild (the count dialog and active tab
    /// flip locally with no packet), while the return value tracks the
    /// iface gen for the harness.
    pub(super) fn rebuild_modals(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.iface, &mut self.modals_gate);
        self.modals = ModalView {
            main: client.main_modal_id,
            side: client.side_modal_id,
            chat: client.chat_modal_id,
            tutorial: client.tut_com_id,
        };
        // The pending box is set by `add_chat` (chat gen) but cleared by a
        // left click that bumps no family, so copy it every rebuild like
        // the other scalars: a stale true would double-drain, a stale
        // false would swallow the next box. Bool copy, no allocation.
        self.tutorial_pending = client.tut_com_message.is_some();
        self.count_dialog_open = client.dialog_input_open;
        self.active_side_tab = client.active_icon;
        self.main_modal_texts = modal_texts(client, client.main_modal_id);
        self.chat_modal_texts = modal_texts(client, client.chat_modal_id);
        self.refresh_puzzle_board(client);
        moved
    }

    /// Menu rebuild: the minimenu entries and the login message. The menu
    /// is rebuilt locally every frame with no packet, so the entries copy
    /// every rebuild; the return value tracks the iface gen.
    pub(super) fn rebuild_menu(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.iface, &mut self.menu_gate);
        let n = client.menu_num_entries.max(0) as usize;
        self.menu_entries = client.menu_option.iter().take(n).cloned().collect();
        let mut login = String::new();
        if !client.login_mes1.is_empty() {
            login.push_str(&client.login_mes1);
        }
        if !client.login_mes2.is_empty() {
            if !login.is_empty() {
                login.push('\n');
            }
            login.push_str(&client.login_mes2);
        }
        self.login_message = login;
        moved
    }

    fn loc_bits(&self, client: &Client) -> (u64, u64) {
        self.loc_bits_prelude
            .unwrap_or_else(|| loc_dirty_bits(client))
    }

    fn publish_scene_from_client(&mut self, client: &Client) {
        self.projection_scene_wanted = self.projection_locs_wanted;
        if !client.ingame || client.scene_state != 2 {
            self.scene = SceneView::default();
            self.projection_scene = ProjectionSceneView::default();
            self.projection_locs_built = false;
            return;
        }
        // The height and link grids serve only the synchronous projection
        // readers; build them only for a run that asked for them.
        self.projection_scene = if self.projection_locs_wanted {
            Self::projection_scene_view(client).unwrap_or_default()
        } else {
            ProjectionSceneView::default()
        };
        self.projection_locs_built = false;
        let level = client.minusedlevel;
        match client.collision.get(level as usize) {
            Some(cmap) => {
                self.scene = SceneView {
                    available: true,
                    base_x: client.map_build_base_x,
                    base_z: client.map_build_base_z,
                    level,
                    width: cmap.size_x,
                    height: cmap.size_z,
                    collision_flags: cmap.flags.iter().flatten().copied().collect(),
                };
            }
            None => {
                self.scene = SceneView {
                    available: false,
                    base_x: client.map_build_base_x,
                    base_z: client.map_build_base_z,
                    level,
                    ..SceneView::default()
                };
            }
        }
    }

    fn projection_scene_view(client: &Client) -> Option<ProjectionSceneView> {
        use ::client::dash3d::MapFlag;

        let level = client.minusedlevel;
        if !(0..=3).contains(&level) {
            return None;
        }
        let mut heights = Vec::with_capacity(105 * 105);
        let ground = client.groundh.get(level as usize)?;
        for x in 0..=104 {
            let row = ground.get(x)?;
            for z in 0..=104 {
                heights.push(*row.get(z)?);
            }
        }
        let mut heights_above = Vec::new();
        if level < 3 {
            heights_above.reserve(105 * 105);
            let ground_above = client.groundh.get((level + 1) as usize)?;
            for x in 0..=104 {
                let row = ground_above.get(x)?;
                for z in 0..=104 {
                    heights_above.push(*row.get(z)?);
                }
            }
        }
        let link_map = client.mapl.get(1)?;
        let mut link_below = Vec::with_capacity(104 * 104);
        for x in 0..104 {
            let row = link_map.get(x)?;
            for z in 0..104 {
                let flags = *row.get(z)?;
                link_below.push(u8::from(flags & (MapFlag::LINK_BELOW as u8) != 0));
            }
        }
        Some(ProjectionSceneView {
            available: true,
            level,
            camera_x: client.cam_x,
            camera_y: client.cam_y,
            camera_z: client.cam_z,
            sin_pitch: client::graphics::Pix3D::sin_table()[(client.cam_pitch & 0x7ff) as usize],
            cos_pitch: client::graphics::Pix3D::cos_table()[(client.cam_pitch & 0x7ff) as usize],
            sin_yaw: client::graphics::Pix3D::sin_table()[(client.cam_yaw & 0x7ff) as usize],
            cos_yaw: client::graphics::Pix3D::cos_table()[(client.cam_yaw & 0x7ff) as usize],
            heights: heights.into(),
            heights_above: heights_above.into(),
            link_below: link_below.into(),
            locs: Vec::new().into(),
        })
    }

    /// Scene-family rebuild: `ingame` + `scene_state`, always fresh —
    /// these flip locally (`check_scene` sets `scene_state = 2` on the SIM
    /// loop with no gen bump), so a gen-gated copy would pin the snapshot
    /// in a stale "loading" state. The `SceneView` (build base, level and
    /// the collision grid) only changes on a world build, so it rebuilds
    /// when the scene gen moves. A session reset clears the host-owned grid,
    /// though response-15 reconnect retains the client's current map; once
    /// that client is scene-ready, materialize the missing host view exactly
    /// once without inventing a generation move. The return value still
    /// tracks the gen for the harness's dirty/tick semantics.
    pub(super) fn rebuild_scene(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.scene, &mut self.gens.scene);
        if !client.ingame {
            self.last_scene_rebuild_tick = None;
        } else if self.last_scene_rebuild_tick.is_none()
            || (client.scene_state == 1 && self.scene_state != 1)
            || (self.scene.available
                && (self.scene.base_x != client.map_build_base_x
                    || self.scene.base_z != client.map_build_base_z
                    || self.scene.level != client.minusedlevel))
        {
            self.last_scene_rebuild_tick = Some(self.tick);
        }
        self.ingame = client.ingame;
        self.scene_state = client.scene_state;
        self.attached = client.stream.is_some();
        let (stamp, static_gen) = self.loc_bits(client);
        let publishable = client.ingame && client.scene_state == 2;
        if !publishable {
            if self.scene.available || !self.scene.collision_flags.is_empty() {
                self.scene = SceneView::default();
            }
            return moved;
        }
        let loc_dirty = stamp != self.loc_model_stamp || static_gen != self.loc_static_gen;
        let identity_stale = !self.scene.available
            || self.scene.level != client.minusedlevel
            || self.scene.base_x != client.map_build_base_x
            || self.scene.base_z != client.map_build_base_z;
        let projection_stale = self.projection_locs_wanted != self.projection_scene_wanted;
        if moved || loc_dirty || identity_stale || projection_stale {
            self.publish_scene_from_client(client);
        }
        moved
    }

    /// Loc-family rebuild: sweep the sim world's four layers at
    /// `minusedlevel` (locs sit on scene tiles, so the world tile is
    /// `base + scene` with no pixel conversion). Gated on the scene gen,
    /// the summed tile loc-layer generations, and World's static-scenery
    /// mutation generation — typecodes can change after the observer
    /// already consumed that gen (map restamp, a door multiloc, or a
    /// queued LOC_DEL/add applied without another packet), so a gen-only
    /// gate leaves nav reading the previous build's door or flax.
    pub(super) fn rebuild_loc(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.scene, &mut self.loc_gen);
        let morph_changed = client.revision() == client::io::ClientRevision::R377
            && track(client.gens.varp, &mut self.loc_varp_gen);
        let standalone = self.loc_bits_prelude.is_none();
        let (stamp, static_gen) = self.loc_bits(client);
        let projection_state_dirty = self.projection_locs_wanted != self.projection_locs_built;
        if !moved
            && !morph_changed
            && stamp == self.loc_model_stamp
            && static_gen == self.loc_static_gen
            && !projection_state_dirty
        {
            return false;
        }
        let dirty = moved
            || morph_changed
            || stamp != self.loc_model_stamp
            || static_gen != self.loc_static_gen;
        self.loc_model_stamp = stamp;
        self.loc_static_gen = static_gen;
        if standalone && client.ingame && client.scene_state == 2 {
            self.publish_scene_from_client(client);
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.loc.clear();
        self.projection_scene.locs = Default::default();
        for sx in 0..104 {
            for sz in 0..104 {
                // Per-tile layer order matches the m8aq sweep: wall,
                // ground, ground decoration, wall decoration.
                if let Some(wall) = client.world.get_wall(level, sx, sz) {
                    self.loc.push(loc_view(
                        wall.typecode,
                        wall.typecode2 & 0xff,
                        LocLayer::Wall,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        client,
                    ));
                }
                if let Some(sprite) = client.world.get_scene(level, sx, sz) {
                    self.loc.push(loc_view(
                        sprite.typecode,
                        sprite.typecode2 & 0xff,
                        LocLayer::Ground,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        client,
                    ));
                }
                if let Some(gd) = client.world.get_gd(level, sx, sz) {
                    self.loc.push(loc_view(
                        gd.typecode,
                        gd.typecode2 & 0xff,
                        LocLayer::GroundDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        client,
                    ));
                }
                if let Some(decor) = client.world.get_decor(level, sx, sz) {
                    self.loc.push(loc_view(
                        decor.typecode,
                        decor.typecode2 & 0xff,
                        LocLayer::WallDecoration,
                        base,
                        level,
                        sx,
                        sz,
                        local_tile,
                        client,
                    ));
                }
            }
        }
        self.rebuild_projection_locs(client);
        dirty
    }

    fn projection_locs_should_build(scene_available: bool, demanded: bool) -> bool {
        scene_available && demanded
    }

    fn rebuild_projection_locs(&mut self, client: &Client) {
        if !Self::projection_locs_should_build(
            self.projection_scene.available,
            self.projection_locs_wanted,
        ) {
            self.projection_scene.locs = std::sync::Arc::from([]);
            // Settled for this state: an unwanted list stays empty, and a
            // wanted one waits for the next scene publish (which clears this).
            self.projection_locs_built = self.projection_locs_wanted;
            return;
        }
        let mut locs = Vec::new();
        let cache = &client.cache;
        for plane in 0..4 {
            let level = plane;
            for sx in 0..104 {
                for sz in 0..104 {
                    if let Some(wall) = client.world.get_wall(level, sx, sz) {
                        locs.push(projection_loc_view(
                            cache,
                            wall.typecode,
                            wall.typecode2 & 0xff,
                            LocLayer::Wall,
                            level,
                            wall.x,
                            wall.z,
                            (sx, sx, sz, sz),
                            Some(wall.angle1),
                            [wall.h_sw, wall.h_se, wall.h_ne, wall.h_nw],
                        ));
                    }
                    if let Some(sprite) = client.world.get_scene(level, sx, sz) {
                        locs.push(projection_loc_view(
                            cache,
                            sprite.typecode,
                            sprite.typecode2 & 0xff,
                            LocLayer::Ground,
                            level,
                            sprite.x,
                            sprite.z,
                            (
                                sprite.min_tile_x,
                                sprite.max_tile_x,
                                sprite.min_tile_z,
                                sprite.max_tile_z,
                            ),
                            None,
                            [sprite.h_sw, sprite.h_se, sprite.h_ne, sprite.h_nw],
                        ));
                    }
                    if let Some(gd) = client.world.get_gd(level, sx, sz) {
                        locs.push(projection_loc_view(
                            cache,
                            gd.typecode,
                            gd.typecode2 & 0xff,
                            LocLayer::GroundDecoration,
                            level,
                            gd.x,
                            gd.z,
                            (sx, sx, sz, sz),
                            None,
                            [gd.h_sw, gd.h_se, gd.h_ne, gd.h_nw],
                        ));
                    }
                    if let Some(decor) = client.world.get_decor(level, sx, sz) {
                        locs.push(projection_loc_view(
                            cache,
                            decor.typecode,
                            decor.typecode2 & 0xff,
                            LocLayer::WallDecoration,
                            level,
                            decor.x,
                            decor.z,
                            (sx, sx, sz, sz),
                            None,
                            [decor.h_sw, decor.h_se, decor.h_ne, decor.h_nw],
                        ));
                    }
                }
            }
        }
        self.projection_scene.locs = locs.into();
        self.projection_locs_built = self.projection_locs_wanted && self.projection_scene.available;
    }

    /// Ground-item rebuild: iterate each `ground_obj` list at
    /// `minusedlevel` into a `GroundItemView` (obj definition, stack
    /// count, ground ops). The `LinkList`'s shared `for_each` iterator
    /// needs no mutable cursor. Gated on the scene gen — object packets
    /// bump it.
    pub(super) fn rebuild_ground_items(&mut self, client: &Client) -> bool {
        if !track(client.gens.scene, &mut self.ground_item_gen) {
            return false;
        }
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        self.ground_item.clear();
        for x in 0..104 {
            for z in 0..104 {
                let Some(list) = &client.ground_obj[level as usize][x as usize][z as usize] else {
                    continue;
                };
                let tile = WorldTile {
                    x: base.0 + x,
                    z: base.1 + z,
                    level,
                };
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                list.for_each(|obj| {
                    self.ground_item.push(GroundItemView {
                        def: item_def_view(&client.cache, obj.id),
                        count: obj.count,
                        actions: ground_ops(&client.cache, obj.id),
                        tile,
                        distance,
                    });
                });
            }
        }
        true
    }

    /// World-state rebuild: the client's world scalars. Cheap reads copy
    /// every rebuild (like the scene status), so counts stay fresh between
    /// world-gen bumps; the return value tracks the gen for the harness.
    pub(super) fn rebuild_world(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.world, &mut self.gens.world);
        self.world = WorldStateView {
            map_base_x: client.map_build_base_x,
            map_base_z: client.map_build_base_z,
            level: client.minusedlevel,
            members: client.members_account != 0,
            multi_combat: client.in_multizone != 0,
            player_count: client.player_count,
            npc_count: client.npc_count,
            cycle: client.loop_cycle,
        };
        moved
    }

    /// Camera rebuild: the follow-camera eye, the orbit target and the
    /// cinematic flag. The follow camera eases every frame with no packet,
    /// so the cheap fields copy every rebuild; the return value tracks the
    /// gen.
    pub(super) fn rebuild_camera(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.camera, &mut self.gens.camera);
        self.camera = CameraView {
            x: client.cam_x,
            y: client.cam_y,
            z: client.cam_z,
            pitch: client.cam_pitch,
            yaw: client.cam_yaw,
            orbit_pitch: client.orbit_camera_pitch,
            orbit_yaw: client.orbit_camera_yaw,
            cinematic: client.cinema_cam,
        };
        if self.projection_scene.available {
            self.projection_scene.camera_x = client.cam_x;
            self.projection_scene.camera_y = client.cam_y;
            self.projection_scene.camera_z = client.cam_z;
            self.projection_scene.sin_pitch =
                ::client::graphics::Pix3D::sin_table()[(client.cam_pitch & 0x7ff) as usize];
            self.projection_scene.cos_pitch =
                ::client::graphics::Pix3D::cos_table()[(client.cam_pitch & 0x7ff) as usize];
            self.projection_scene.sin_yaw =
                ::client::graphics::Pix3D::sin_table()[(client.cam_yaw & 0x7ff) as usize];
            self.projection_scene.cos_yaw =
                ::client::graphics::Pix3D::cos_table()[(client.cam_yaw & 0x7ff) as usize];
        }
        moved
    }

    /// Map-flag rebuild: the minimap destination flag, `Some` only while
    /// it is set. The flag flips on minimap clicks with no packet, so the
    /// view copies every rebuild; the return value tracks the gen.
    pub(super) fn rebuild_map_flag(&mut self, client: &Client) -> bool {
        let moved = track(client.gens.map_flag, &mut self.gens.map_flag);
        self.map_flag = (client.minimap_flag_x != 0).then_some(MapFlagView {
            lx: client.minimap_flag_x,
            lz: client.minimap_flag_z,
        });
        moved
    }

    pub(super) fn rebuild_npcs(&mut self, client: &Client) -> bool {
        let morph_changed = client.revision() == client::io::ClientRevision::R377
            && track(client.gens.varp, &mut self.npc_varp_gen);
        let moved = track(client.gens.npc, &mut self.gens.npc);
        if !moved && !morph_changed {
            return false;
        }
        self.gens.npc_info = client.gens.npc_info;
        self.npc.clear();
        self.npc.reserve(client.npc_count as usize);
        let base = (client.map_build_base_x, client.map_build_base_z);
        let level = client.minusedlevel;
        let local_tile = local_world_tile(client);
        for i in 0..client.npc_count as usize {
            let index = client.npc_ids[i] as usize;
            if let Some(npc) = client.npc.get(index).and_then(|n| n.as_ref()) {
                let tile = entity_world_tile(&npc.entity, base, level);
                let distance = local_tile
                    .map(|(lx, lz)| chebyshev(tile.x, tile.z, lx, lz))
                    .unwrap_or(0);
                self.npc.push(NpcView::from_slot(
                    index,
                    npc,
                    base,
                    level,
                    distance,
                    client.loop_cycle,
                    client,
                ));
            }
        }
        true
    }
}
/// The local player's world tile `(x, z)` from the live client (build
/// base + route head); `None` before the first `PLAYER_INFO`.
fn local_world_tile(client: &Client) -> Option<(i32, i32)> {
    client.local_player.as_ref().map(|lp| {
        (
            client.map_build_base_x + lp.route_x[0],
            client.map_build_base_z + lp.route_z[0],
        )
    })
}

fn chebyshev(ax: i32, az: i32, bx: i32, bz: i32) -> i32 {
    (ax - bx).abs().max((az - bz).abs())
}

/// The absolute world tile of an entity: its scene-local pixel coords
/// (`route * 128 + size * 64`) un-scaled by 128 and offset by the build
/// base, so every actor view is in the same world-tile space as the local
/// player's tile.
fn entity_world_tile(entity: &ClientEntity, base: (i32, i32), level: i32) -> WorldTile {
    WorldTile {
        x: base.0 + (entity.x - entity.size * 64) / 128,
        z: base.1 + (entity.z - entity.size * 64) / 128,
        level,
    }
}

/// Path-head network SW: build base plus `route[0]`. Independent of rendered
/// pixel `x`/`z` (those stay mid-route until rest pose).
fn entity_network_tile(entity: &ClientEntity, base: (i32, i32), level: i32) -> WorldTile {
    WorldTile {
        x: base.0 + entity.route_x[0],
        z: base.1 + entity.route_z[0],
        level,
    }
}

/// Projectile sources are encoded as scene-local tile centers in pixel space.
fn projectile_source_tile(src_x: i32, src_z: i32, level: i32, base: (i32, i32)) -> WorldTile {
    WorldTile {
        x: base.0 + (src_x - 64) / 128,
        z: base.1 + (src_z - 64) / 128,
        level,
    }
}

fn player_weapon(appearance: &[u16; 12]) -> Option<i32> {
    let encoded = i32::from(appearance[3]);
    (encoded >= 0x200).then_some(encoded - 0x200)
}

/// The engine encodes a player face target as slot + 32768.
pub const PLAYER_FACE_BASE: i32 = 32768;

/// Local `Game.attackedByPlayer`: true only when `face_entity` is a player.
pub fn attacked_by_player(face_entity: i32) -> bool {
    face_entity >= PLAYER_FACE_BASE
}

/// Resolve `face_entity` with the client's own scheme (`entity_face` in
/// `client.rs`): slots below 32768 are NPC table indexes, at or above are
/// player slots offset by 32768. The player slot stays the raw server
/// slot (the client only rewrites `self_slot` → 2047 for its internal
/// turn lookup), so `targetingPlayer(self_slot)` matches the local player.
fn decode_target(face_entity: i32) -> Option<ActorTargetView> {
    if face_entity == -1 {
        return None;
    }
    if face_entity < PLAYER_FACE_BASE {
        Some(ActorTargetView {
            kind: ActorKind::Npc,
            index: face_entity as usize,
        })
    } else {
        Some(ActorTargetView {
            kind: ActorKind::Player,
            index: (face_entity - PLAYER_FACE_BASE) as usize,
        })
    }
}

/// Projectiles encode target slots by sign: positive NPC, negative player,
/// zero for untargeted. Keep the published view in the shared actor scheme.
fn decode_projectile_target(target: i32) -> Option<ActorTargetView> {
    if target == 0 {
        return None;
    }
    let (kind, index) = if target > 0 {
        (ActorKind::Npc, usize::try_from(target - 1).ok()?)
    } else {
        (
            ActorKind::Player,
            usize::try_from(target.unsigned_abs().checked_sub(1)?).ok()?,
        )
    };
    Some(ActorTargetView { kind, index })
}

/// Health-bar window: the client draws it while `combatCycle > loopCycle`.
/// `combat_cycle > 0` is wrong — after a hit the expiry tick stays a large
/// positive number, so `Game.inCombat()` would stick true forever.
fn actor_in_combat(combat_cycle: i32, loop_cycle: i32) -> bool {
    combat_cycle > loop_cycle
}

/// Frozen `takingDamage` / positive active combat hit: ingame + local player
/// required; at least one hitmark with value > 0, type == 1, and cycle still
/// ahead of the current loop cycle. Zero, blocked, poison, and expired hits
/// do not qualify; a later miss does not erase an earlier live positive hit.
fn local_player_taking_damage(client: &Client) -> bool {
    if !client.ingame {
        return false;
    }
    let Some(player) = client.local_player.as_ref() else {
        return false;
    };
    let entity = &player.entity;
    let loop_cycle = client.loop_cycle;
    (0..entity.damage_values.len()).any(|i| {
        entity.damage_values[i] > 0
            && entity.damage_types[i] == 1
            && entity.damage_cycles[i] > loop_cycle
    })
}

/// The shared actor fields from one entity, as the m8aq `ActorSnapshot`.
fn actor_view(
    entity: &ClientEntity,
    base: (i32, i32),
    level: i32,
    distance: i32,
    name: Option<String>,
    actions: Vec<Option<String>>,
    loop_cycle: i32,
) -> ActorView {
    ActorView {
        name,
        actions,
        tile: entity_world_tile(entity, base, level),
        distance,
        animation: entity.primary_anim,
        animation_frame: entity.primary_anim_frame,
        pose_animation: entity.secondary_anim,
        orientation: entity.yaw,
        target_orientation: entity.dst_yaw,
        overhead_text: entity.chat_message.clone(),
        spot_animation: entity.spotanim_id,
        spot_animation_stamp: entity.spotanim_last_cycle,
        health: entity.health,
        total_health: entity.total_health,
        face_entity: entity.face_entity,
        target: decode_target(entity.face_entity),
        moving: entity.route_length > 0,
        running: entity.primary_anim == entity.runanim,
        in_combat: actor_in_combat(entity.combat_cycle, loop_cycle),
    }
}

/// One `LocView` from a placed layer: decode the loc id and the
/// shape/angle info byte, then resolve the definition from the loc table.
/// An unloaded loc id reads the `LocType` defaults (the m8aq `LocType.list`
/// dummy type).
#[allow(clippy::too_many_arguments)]
fn loc_view(
    typecode: i32,
    info: i32,
    layer: LocLayer,
    base: (i32, i32),
    level: i32,
    sx: i32,
    sz: i32,
    local_tile: Option<(i32, i32)>,
    client: &Client,
) -> LocView {
    let id = (typecode >> 14) & 0x7fff;
    let shape = info & 0x1f;
    let angle = (info >> 6) & 0x3;
    let x = base.0 + sx;
    let z = base.1 + sz;
    let base_loc = client.cache.locs.get(id as usize);
    // Java 377 Client:4133-4176 resolves only menu text/options (including
    // the Examine target). Routing keeps the base size/forceapproach at
    // Client:6979-6993; World:795-858 keeps base placement/collision/anim.
    let display_loc = if client.revision() == client::io::ClientRevision::R377 {
        client.cache.resolve_loc(id as usize, &client.var)
    } else {
        base_loc
    };
    let (name, description, actions) = match display_loc {
        Some(loc) => (
            (!loc.name.is_empty()).then(|| loc.name.clone()),
            (!loc.desc.is_empty()).then(|| loc.desc.clone()),
            loc.op.clone(),
        ),
        None => (None, None, Vec::new()),
    };
    let (
        width,
        length,
        block_walk,
        block_range,
        active,
        animation,
        map_function,
        map_scene,
        force_approach,
    ) = match base_loc {
        Some(loc) => (
            loc.width,
            loc.length,
            loc.blockwalk,
            loc.blockrange,
            loc.active,
            loc.anim,
            loc.mapfunction,
            loc.mapscene,
            loc.forceapproach,
        ),
        None => (1, 1, true, true, false, -1, -1, -1, 0),
    };
    LocView {
        typecode,
        info,
        id,
        name,
        description,
        actions,
        tile: WorldTile { x, z, level },
        distance: local_tile
            .map(|(lx, lz)| chebyshev(x, z, lx, lz))
            .unwrap_or(0),
        layer,
        shape,
        angle,
        width,
        length,
        // A 90°/270° rotation swaps the footprint axes.
        footprint_width: if angle == 1 || angle == 3 {
            length
        } else {
            width
        },
        footprint_length: if angle == 1 || angle == 3 {
            width
        } else {
            length
        },
        block_walk,
        block_range,
        active,
        animation,
        map_function,
        map_scene,
        force_approach,
    }
}
#[allow(clippy::too_many_arguments)]
fn projection_loc_view(
    cache: &Cache,
    typecode: i32,
    info: i32,
    layer: LocLayer,
    level: i32,
    scene_x: i32,
    scene_z: i32,
    tiles: (i32, i32, i32, i32),
    wall_angle: Option<i32>,
    heights: [i32; 4],
) -> ProjectionLocView {
    use ::client::dash3d::LocShape;

    let id = (typecode >> 14) & 0x7fff;
    let shape = info & 0x1f;
    let angle = (info >> 6) & 0x3;
    let (min_tile_x, max_tile_x, min_tile_z, max_tile_z) = tiles;
    let def = cache.locs.get(id as usize);
    let name = def.and_then(|def| {
        (!def.name.is_empty()).then(|| std::sync::Arc::<str>::from(def.name.as_str()))
    });
    let width = def.map_or(1, |def| def.width);
    let length = def.map_or(1, |def| def.length);
    let used_scene_footprint = layer == LocLayer::Ground;
    let rank = match layer {
        LocLayer::Wall => 0,
        LocLayer::Ground => 1,
        LocLayer::WallDecoration => 2,
        LocLayer::GroundDecoration => 3,
    };
    let (mut half_width, mut half_length) = if used_scene_footprint {
        (
            max_tile_x
                .saturating_sub(min_tile_x)
                .saturating_add(1)
                .saturating_mul(64)
                .max(32),
            max_tile_z
                .saturating_sub(min_tile_z)
                .saturating_add(1)
                .saturating_mul(64)
                .max(32),
        )
    } else {
        (
            width.saturating_mul(64).max(32),
            length.saturating_mul(64).max(32),
        )
    };
    let mut top_height = 128;
    if let Some(def) = def {
        if let Some(angle) = wall_angle {
            let thin = if def.wallwidth == 0 {
                16
            } else {
                def.wallwidth
            }
            .clamp(8, 40);
            let angle = angle.rem_euclid(2048);
            if angle < 256 || (896..1152).contains(&angle) || angle >= 1792 {
                half_length = thin;
            } else {
                half_width = thin;
            }
        }
        let model_shape = match layer {
            LocLayer::Wall => shape,
            LocLayer::Ground if shape == LocShape::CENTREPIECE_DIAGONAL => {
                LocShape::CENTREPIECE_STRAIGHT
            }
            LocLayer::Ground => shape,
            LocLayer::GroundDecoration => LocShape::GROUND_DECOR,
            LocLayer::WallDecoration => LocShape::WALLDECOR_STRAIGHT_NOOFFSET,
        };
        let model_angle = match layer {
            LocLayer::Wall if shape == LocShape::WALL_L => angle + 4,
            LocLayer::Wall | LocLayer::Ground | LocLayer::GroundDecoration => angle,
            LocLayer::WallDecoration => 0,
        };
        let model = def
            .get_model_shared(
                cache,
                model_shape,
                model_angle,
                heights[0],
                heights[1],
                heights[2],
                heights[3],
                -1,
            )
            .or_else(|| {
                (layer == LocLayer::Wall && shape == LocShape::WALL_L).then(|| {
                    def.get_model_shared(
                        cache,
                        LocShape::WALL_L,
                        (angle + 1) & 0x3,
                        heights[0],
                        heights[1],
                        heights[2],
                        heights[3],
                        -1,
                    )
                })?
            });
        if let Some(model) = model {
            top_height = if model.min_y > 0 {
                model.min_y
            } else if model.max_y > 0 {
                model.max_y
            } else {
                128
            };
            if !used_scene_footprint
                && model.radius > 0
                && def.width <= 1
                && def.length <= 1
                && wall_angle.is_none()
            {
                let radius = model.radius.min(192);
                half_width = half_width.max(radius);
                half_length = half_length.max(radius);
            }
        }
    }
    ProjectionLocView {
        id,
        name,
        level,
        rank,
        scene_x,
        scene_z,
        min_tile_x,
        max_tile_x,
        min_tile_z,
        max_tile_z,
        half_width,
        half_length,
        top_height,
    }
}

/// The obj's definition view via Task 1's mapping. An unloaded obj id
/// reads the `ObjType` defaults with the requested id (the m8aq
/// `ObjType.list` dummy type).
fn item_def_view(cache: &Cache, id: i32) -> ItemDefView {
    let mut def = match cache.objs.get(id as usize) {
        Some(o) => ItemDefView::from_obj(o),
        None => ItemDefView::from_obj(&ObjType::default()),
    };
    def.id = id;
    def
}

/// The ground menu ops for an obj: the type's `op` table padded to five
/// slots with a `Take` default filling an empty third (m8aq `groundOps`).
fn ground_ops(cache: &Cache, id: i32) -> Vec<Option<String>> {
    let mut ops = cache
        .objs
        .get(id as usize)
        .map(|o| o.op.to_vec())
        .unwrap_or_else(|| vec![None; 5]);
    if ops.len() < 3 {
        ops.resize(3, None);
    }
    if ops[2].is_none() {
        ops[2] = Some("Take".into());
    }
    ops
}

pub(super) fn track(world: u64, tracked: &mut u64) -> bool {
    if *tracked == world {
        return false;
    }
    *tracked = world;
    true
}

/// Cheap dirty bit for loc rebuilds: aggregate every scene tile's loc-layer
/// generations (including door multilocs). Integer reads only — no loc
/// string clones. Scene sprites have a separate static-loc generation.
pub(super) fn loc_dirty_bits(client: &Client) -> (u64, u64) {
    (
        loc_model_stamp(client),
        client.world.static_loc_generation(),
    )
}

fn loc_model_stamp(client: &Client) -> u64 {
    let level = client.minusedlevel;
    let mut stamp = 0u64;
    for sx in 0..104 {
        for sz in 0..104 {
            let generation = client.world.square(level, sx, sz).map_or(i32::MIN, |tile| {
                tile.wall_model_stamp
                    .wrapping_add(tile.decor_model_stamp)
                    .wrapping_add(tile.gd_model_stamp)
            });
            stamp = stamp.wrapping_mul(31).wrapping_add(generation as u64);
        }
    }
    stamp
}

pub(super) const fn empty_loc_model_stamp() -> u64 {
    let mut stamp = 0u64;
    let mut i = 0usize;
    while i < 104 * 104 {
        stamp = stamp.wrapping_mul(31).wrapping_add(i32::MIN as u64);
        i += 1;
    }
    stamp
}

/// Session marker for bank snapshot readiness: last-full inv generation
/// must be at least the inv generation recorded when this component closed.
#[derive(Clone, Copy)]
pub(super) struct BankInvSession {
    main_com_id: i32,
    main_opened_at: u64,
}

/// The two-counter gate of the item-bearing iface families: rebuild when
/// either the iface or the inv gen moved (component ids on the iface gen,
/// TYPE_INV slot data on the inv gen).
#[derive(Default, Clone, Copy)]
pub(super) struct InvIfaceGate {
    pub(super) iface: u64,
    pub(super) inv: u64,
}

impl InvIfaceGate {
    fn moved(&mut self, client: &Client, inv_session_current: bool) -> bool {
        if !inv_session_current {
            return false;
        }
        let moved = client.gens.iface != self.iface || client.gens.inv != self.inv;
        self.iface = client.gens.iface;
        self.inv = client.gens.inv;
        moved
    }
}

/// `Some(s)` for a non-empty string (iface strings are empty when unset).
fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// A component's children list, empty when it has none.
fn children_of(com: &IfType) -> &[i32] {
    com.children.as_deref().unwrap_or_default()
}

/// The open widget roots: the main modal and overlay (both draw above the
/// game view — the main modal carries the trade/bank/dialog tree), the
/// side modal (else the active tab's interface), the chat modal and the
/// tutorial overlay. Roots that are not cleanly resolvable (id not in the
/// table) are skipped by the walk; every emitted widget keeps the root id
/// and tag it was walked under.
fn widget_roots(client: &Client) -> Vec<(i32, WidgetRoot)> {
    let mut roots = Vec::new();
    if client.main_modal_id != -1 {
        roots.push((client.main_modal_id, WidgetRoot::Main));
    }
    if client.main_overlay_id != -1 {
        roots.push((client.main_overlay_id, WidgetRoot::Main));
    }
    let side_root = if client.side_modal_id != -1 {
        client.side_modal_id
    } else {
        client
            .side_icon
            .get(client.active_icon.max(0) as usize)
            .copied()
            .unwrap_or(-1)
    };
    if side_root != -1 {
        roots.push((side_root, WidgetRoot::Side));
    }
    if client.chat_modal_id != -1 {
        roots.push((client.chat_modal_id, WidgetRoot::Chat));
    }
    if client.tut_com_id != -1 {
        roots.push((client.tut_com_id, WidgetRoot::Tutorial));
    }
    roots
}

/// Walk one widget root's tree into `out`, tagging every component with
/// `root`/`root_component_id`. `visited` is shared across roots so a
/// component reachable from two open roots belongs to the first that
/// walks it (its ancestor chain reaches that root first). Positions
/// accumulate `child_x`/`child_y` from the root (m8aq
/// `walkPositionedComponents`); the root itself has parent -1.
fn walk_widget_tree(
    client: &Client,
    root_id: i32,
    root: WidgetRoot,
    visited: &mut [bool],
    out: &mut Vec<WidgetView>,
) {
    let mut queue: Vec<(i32, i32, i32, i32)> = vec![(root_id, -1, 0, 0)];
    let mut head = 0;
    while head < queue.len() {
        let (id, parent_id, x, y) = queue[head];
        head += 1;
        if id < 0 || (id as usize) >= visited.len() || visited[id as usize] {
            continue;
        }
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        visited[id as usize] = true;
        out.push(widget_view(
            client, &com, id, parent_id, root_id, root, x, y,
        ));
        if let Some(children) = &com.children {
            for (i, child) in children.iter().enumerate() {
                let cx = com
                    .child_x
                    .as_ref()
                    .and_then(|xs| xs.get(i))
                    .copied()
                    .unwrap_or(0);
                let cy = com
                    .child_y
                    .as_ref()
                    .and_then(|ys| ys.get(i))
                    .copied()
                    .unwrap_or(0);
                queue.push((*child, id, x + cx, y + cy));
            }
        }
    }
}

/// One `WidgetView` from a component plus its walk context. `component_id`
/// is the table id the walk found the component under (matches `com.id`
/// for well-formed ifaces).
#[allow(clippy::too_many_arguments)]
fn widget_view(
    client: &Client,
    com: &IfTypeView,
    component_id: i32,
    parent_id: i32,
    root_component_id: i32,
    root: WidgetRoot,
    x: i32,
    y: i32,
) -> WidgetView {
    WidgetView {
        kind: WidgetKind::Widget,
        component_id,
        layer_id: com.layer_id,
        parent_id,
        root_component_id,
        root,
        type_: com.r#type,
        button_type: com.button_type,
        client_code: com.client_code,
        x,
        y,
        width: com.width,
        height: com.height,
        scroll_height: com.scroll_height,
        inventory_ready: client
            .inventory_packet_state(component_id)
            .is_some_and(|state| state.transmitting && state.full_generation > 0),
        scroll_position: com.scroll_pos,
        hidden: com.hide,
        text: non_empty(com.text),
        alternate_text: non_empty(&com.text2),
        button_text: non_empty(&com.button_text),
        target_verb: non_empty(&com.target_verb),
        target_base: non_empty(&com.target_base),
        target_mask: com.target_mask,
        model_type: com.model1_type,
        model_id: com.model1_id,
        alternate_model_type: com.model2_type,
        alternate_model_id: com.model2_id,
        scripts: com
            .scripts
            .clone()
            .map(|scripts| scripts.into_iter().map(Some).collect()),
        script_comparators: com.script_comparator.clone(),
        script_operands: com.script_operand.clone(),
        varp_bindings: varp_bindings(com),
        colour: com.colour,
        actions: com.iop.to_vec(),
        items: if com.r#type == ComponentType::TYPE_INV {
            read_inv_component(&client.cache, com, ItemContainer::Widget)
        } else {
            Vec::new()
        },
    }
}

/// The varp-bound scripts of a component: opcode-5 (`IF_VARP`) scripts
/// whose first operand is the varp, with the per-script comparator and
/// operand (m8aq `widgetVarpBindings`; the client's own toggle/select
/// arms decode `scripts[0][0] === 5` the same way).
fn varp_bindings(com: &IfType) -> Vec<WidgetVarpBindingView> {
    let mut out = Vec::new();
    let Some(scripts) = &com.scripts else {
        return out;
    };
    for (i, script) in scripts.iter().enumerate() {
        if script.len() >= 2 && script[0] == 5 {
            out.push(WidgetVarpBindingView {
                script_index: i as i32,
                varp: script[1],
                value: com.script_operand.as_ref().and_then(|o| o.get(i)).copied(),
                comparator: com
                    .script_comparator
                    .as_ref()
                    .and_then(|c| c.get(i))
                    .copied(),
            });
        }
    }
    out
}

/// One `ItemView` from a stored slot (`link_obj_type` holds `obj_id + 1`,
/// 0 = empty), with the given ops.
fn item_view(
    cache: &Cache,
    com: &IfTypeView,
    slot: usize,
    container: ItemContainer,
    action_family: ItemActionFamily,
    actions: Vec<Option<String>>,
) -> Option<ItemView> {
    let stored = com.link_obj_type.as_ref()?.get(slot).copied()?;
    if stored <= 0 {
        return None;
    }
    let id = stored - 1;
    Some(ItemView {
        def: item_def_view(cache, id),
        container,
        action_family,
        slot: slot as i32,
        count: com
            .link_obj_number
            .as_ref()
            .and_then(|n| n.get(slot))
            .copied()
            .unwrap_or(0),
        actions,
        component_id: com.id,
    })
}

/// The `ItemView`s of a TYPE_INV component's slots (m8aq
/// `readInvComponent`), with ops from the component's own `iop`.
fn read_inv_component(cache: &Cache, com: &IfTypeView, container: ItemContainer) -> Vec<ItemView> {
    let mut out = Vec::new();
    let Some(ids) = &com.link_obj_type else {
        return out;
    };
    let n = com.link_obj_number.as_ref().map(|n| n.len()).unwrap_or(0);
    for (slot, stored) in ids.iter().copied().enumerate().take(ids.len().min(n)) {
        if stored <= 0 {
            continue;
        }
        if let Some(view) = item_view(
            cache,
            com,
            slot,
            container,
            ItemActionFamily::Component,
            com.iop.to_vec(),
        ) {
            out.push(view);
        }
    }
    out
}

/// One confirm-screen list: the rows of the `small` component followed by
/// those of the `large` one. The engine fills exactly one of them
/// (`trade.rs2:111-145`), so the other adds nothing. The rows are display
/// only: no action family and no ops, and a list with no rows allocates
/// nothing.
fn confirm_rows(
    client: &Client,
    small: i32,
    large: i32,
    container: ItemContainer,
) -> Vec<ItemView> {
    let mut rows = Vec::new();
    for com_id in [small, large] {
        let Some(com) = client.if_(com_id as usize) else {
            continue;
        };
        let Some(ids) = &com.link_obj_type else {
            continue;
        };
        let n = com.link_obj_number.as_ref().map_or(0, |n| n.len());
        for slot in 0..ids.len().min(n) {
            rows.extend(item_view(
                &client.cache,
                &com,
                slot,
                container,
                ItemActionFamily::None,
                Vec::new(),
            ));
        }
    }
    rows
}

/// The items of the iface-table component `com_id` (component ops).
fn inv_items(client: &Client, com_id: i32, container: ItemContainer) -> Option<Vec<ItemView>> {
    if com_id < 0 {
        return None;
    }
    let com = client.if_(com_id as usize)?;
    Some(read_inv_component(&client.cache, &com, container))
}

/// The held-item ops for obj `id`: the type's `iop` padded to five slots
/// with a `Drop` default in the fifth (m8aq `heldOps`). Published for the
/// send-time Held-family view the puzzle-move dispatch builds: a caller
/// that cannot read the obj table sends nothing.
pub fn cache_held_ops(cache: &Cache, id: i32) -> Vec<Option<String>> {
    let mut ops = cache
        .objs
        .get(id as usize)
        .map(|o| o.iop.to_vec())
        .unwrap_or_else(|| vec![None; 5]);
    if ops.len() < 5 {
        ops.resize(5, None);
    }
    if ops[4].is_none() {
        ops[4] = Some("Drop".into());
    }
    ops
}

/// The TYPE_INV component of side tab `tab` (m8aq `findTabInvComponent`):
/// tab 4 (worn items) accepts any TYPE_INV, the other tabs need `obj_ops`.
/// Public so the script shim's inventory post reads the same component
/// the snapshot's inv view reads.
pub fn tab_inv_component(client: &Client, tab: usize) -> Option<i32> {
    let root = client.side_icon.get(tab).copied().unwrap_or(-1);
    if root == -1 {
        return None;
    }
    find_inv_component(client, root, |com| com.obj_ops || tab == 4)
}

/// Whether `com` is a bank's withdraw grid (m8aq `bankItems`): its first
/// interface op withdraws.
fn is_bank_withdraw_grid(com: &IfType) -> bool {
    com.iop[0]
        .as_deref()
        .is_some_and(|s| s.to_ascii_lowercase().contains("withdraw"))
}

/// Depth-first search for a TYPE_INV component satisfying `accept` under
/// `root_id` (m8aq `findInvComponentIn`).
fn find_inv_component<F>(client: &Client, root_id: i32, accept: F) -> Option<i32>
where
    F: Fn(&IfType) -> bool,
{
    let mut queue = vec![root_id];
    while let Some(id) = queue.pop() {
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        if com.r#type == ComponentType::TYPE_INV && accept(&com) {
            return Some(id);
        }
        queue.extend(children_of(&com));
    }
    None
}

/// The 274 shop iface ids (the packed `interface.order` allocation):
/// shop_template 3824 (main modal), shop_template:inv 3900 (stock),
/// shop_template_side 3822 (side modal) and shop_template_side:inv 3823
/// (the shop's player pack — the Sell 1/5/10 container).
const SHOPMAIN: i32 = 3824;
const SHOP_STOCK_INV: i32 = 3900;
const SHOP_SIDE: i32 = 3822;
const SHOP_SIDE_INV: i32 = 3823;

/// The 274 trade iface ids (the packed `interface.order` allocation):
/// trademain 3323 (offer screen), tradeconfirm 3443, trademain:inv 3415,
/// trademain:otherinv 3416, trademain:otherplayer 3417, tradeside:inv
/// 3322. The m8aq adapter reads the same hardcoded ids.
const TRADEMAIN: i32 = 3323;
const TRADECONFIRM: i32 = 3443;
const TRADEMAIN_INV: i32 = 3415;
const TRADEMAIN_OTHER_INV: i32 = 3416;
const TRADEMAIN_OTHER_PLAYER: i32 = 3417;
const TRADESIDE_INV: i32 = 3322;
/// The confirm screen's four lists (`interface.pack:3543, 3539, 3533, 3540`).
/// `trade.rs2:111-145` transmits each offer to `inv1`/`otherinv1` when it
/// holds fewer than 14 items and to `inv2`/`otherinv2` otherwise.
const TRADECONFIRM_INV1: i32 = 3542;
const TRADECONFIRM_INV2: i32 = 3538;
const TRADECONFIRM_OTHER_INV1: i32 = 3532;
const TRADECONFIRM_OTHER_INV2: i32 = 3539;

/// The trade partner's name: the `otherplayer` label ("Trading With: X")
/// with the prefix stripped and whitespace trimmed (m8aq
/// `normalizeTradePartner`); `None` for an empty label.
fn trade_partner(client: &Client) -> Option<String> {
    let text = client
        .if_(TRADEMAIN_OTHER_PLAYER as usize)
        .map(|c| c.text.to_string())
        .unwrap_or_default();
    let name = match text.find(':') {
        Some(colon) => text[colon + 1..].trim(),
        None => text.trim(),
    };
    (!name.is_empty()).then(|| name.to_string())
}

/// The Accept / Decline controls on the open trade root (`TRADEMAIN` or
/// `TRADECONFIRM`), discovered by walking the modal tree. The packed native
/// interfaces put the label in a non-clickable TYPE_TEXT sibling next to the
/// actual button, so the label itself must never be published as the target.
fn trade_controls(client: &Client) -> (i32, i32) {
    let (root, accept_id, decline_id) = match client.main_modal_id {
        TRADECONFIRM => (TRADECONFIRM, 3546, 3548),
        TRADEMAIN => (TRADEMAIN, 3420, 3422),
        _ => return (-1, -1),
    };
    (
        native_trade_control(client, root, accept_id, ButtonType::BUTTON_OK),
        native_trade_control(client, root, decline_id, ButtonType::BUTTON_CLOSE),
    )
}

/// Validate one canonical packed trade control. The numeric identity is part
/// of the native interface contract; labels are separate TYPE_TEXT siblings
/// and therefore cannot identify a replacement button. Walking the tree with
/// visibility state also rejects controls hidden by any ancestor.
fn native_trade_control(client: &Client, root: i32, wanted: i32, button_type: i32) -> i32 {
    let mut queue = vec![root];
    let mut head = 0;
    while head < queue.len() {
        let id = queue[head];
        head += 1;
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        if com.hide {
            continue;
        }
        let children = com.children.as_deref().unwrap_or(&[]);
        for child_id in children {
            let Some(candidate) = client.if_(*child_id as usize) else {
                continue;
            };
            if candidate.id == wanted
                && !candidate.hide
                && candidate.r#type == ComponentType::TYPE_RECT
                && candidate.button_type == button_type
            {
                return candidate.id;
            }
        }
        queue.extend(children.iter().copied());
    }
    -1
}

/// The TYPE_TEXT contents of a modal tree, in walk order (m8aq
/// `mainModalTexts`/`chatModalTexts`).
fn modal_texts(client: &Client, root: i32) -> Vec<String> {
    let mut out = Vec::new();
    if root == -1 {
        return out;
    }
    let mut queue = vec![root];
    let mut head = 0;
    while head < queue.len() {
        let id = queue[head];
        head += 1;
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        if com.r#type == ComponentType::TYPE_TEXT && !com.text.is_empty() {
            out.push(com.text.to_string());
        }
        queue.extend(children_of(&com));
    }
    out
}

/// The Note/Item toggle pair on the open bank main modal.
///
/// Two packed shapes:
/// - pressable TYPE_TEXT labeled "Note" / "Item" (unit fixtures)
/// - 274 `bank_main.if`: unpressable labels sitting on TYPE_GRAPHIC
///   SELECT boxes (`bankcert`); the snapshot maps those boxes.
fn bank_note_controls(client: &Client, main_root: i32) -> Option<ToggleControlsView> {
    if main_root == -1 {
        return None;
    }
    let mut note_id = -1;
    let mut item_id = -1;
    let mut note_label: Option<(i32, i32)> = None;
    let mut item_label: Option<(i32, i32)> = None;
    let mut selects: Vec<(i32, i32, i32, i32, i32)> = Vec::new();
    let mut queue: Vec<(i32, i32, i32)> = vec![(main_root, 0, 0)];
    let mut head = 0;
    while head < queue.len() {
        let (id, x, y) = queue[head];
        head += 1;
        let Some(com) = client.if_(id as usize) else {
            continue;
        };
        let label = if !com.text.trim().is_empty() {
            com.text.trim()
        } else {
            com.button_text.trim()
        };
        if label.eq_ignore_ascii_case("note") {
            if com.button_type != 0 {
                note_id = id;
            }
            note_label = Some((x + com.width / 2, y + com.height / 2));
        } else if label.eq_ignore_ascii_case("item") {
            if com.button_type != 0 {
                item_id = id;
            }
            item_label = Some((x + com.width / 2, y + com.height / 2));
        }
        if com.button_type == ButtonType::BUTTON_SELECT
            || com.button_type == ButtonType::BUTTON_TOGGLE
        {
            selects.push((id, x, y, com.width, com.height));
        }
        if let Some(children) = &com.children {
            for (i, child) in children.iter().enumerate() {
                let cx = com
                    .child_x
                    .as_ref()
                    .and_then(|xs| xs.get(i))
                    .copied()
                    .unwrap_or(0);
                let cy = com
                    .child_y
                    .as_ref()
                    .and_then(|ys| ys.get(i))
                    .copied()
                    .unwrap_or(0);
                queue.push((*child, x + cx, y + cy));
            }
        }
    }
    if note_id >= 0 && item_id >= 0 {
        return Some(ToggleControlsView {
            on_component_id: note_id,
            off_component_id: item_id,
        });
    }
    let contains = |cx: i32, cy: i32, x: i32, y: i32, w: i32, h: i32| -> bool {
        cx >= x && cy >= y && cx < x + w && cy < y + h
    };
    let pick = |center: (i32, i32)| -> Option<i32> {
        let (cx, cy) = center;
        selects
            .iter()
            .find(|(_, x, y, w, h)| contains(cx, cy, *x, *y, *w, *h))
            .map(|(id, ..)| *id)
    };
    match (note_label, item_label) {
        (Some(note), Some(item)) => {
            let on = pick(note)?;
            let off = pick(item)?;
            if on == off {
                return None;
            }
            Some(ToggleControlsView {
                on_component_id: on,
                off_component_id: off,
            })
        }
        _ => None,
    }
}

/// The toggle pair of the player-controls overlay: the root with an
/// "Auto retaliate" label among its children, reading `on_index`/
/// `off_index` from its children list (m8aq `runControls` reads 5/4 for
/// run, `readRetaliateControls` 2/3 for retaliate — the 274 `controls.if`
/// com_2/com_3 and com_4/com_5 buttons).
fn controls_pair(
    client: &Client,
    min_children: usize,
    on_index: usize,
    off_index: usize,
) -> Option<ToggleControlsView> {
    for com in client.ifaces_merged() {
        let Some(children) = &com.children else {
            continue;
        };
        let has_retaliate = children.iter().any(|id| {
            client
                .if_(*id as usize)
                .is_some_and(|c| c.text == "Auto retaliate")
        });
        if !has_retaliate || children.len() <= min_children {
            continue;
        }
        let on = children.get(on_index).copied().unwrap_or(-1);
        let off = children.get(off_index).copied().unwrap_or(-1);
        if on < 0
            || off < 0
            || client.if_(on as usize).is_none()
            || client.if_(off as usize).is_none()
        {
            return None;
        }
        return Some(ToggleControlsView {
            on_component_id: on,
            off_component_id: off,
        });
    }
    None
}

/// The make/smelt button quantity: `"Make X"`/`"Smelt X"` reads -1,
/// `"Make 10"` reads 10 (m8aq's button regex).
fn make_quantity(button_text: &str) -> Option<i32> {
    let lower = button_text.to_ascii_lowercase();
    let start = lower.find("make ").or_else(|| lower.find("smelt "))?;
    let rest = lower[start..].trim_start_matches(|c: char| c.is_ascii_alphabetic());
    let rest = rest.trim_start();
    if rest.is_empty() {
        return None;
    }
    let token: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    if token == "x" {
        Some(-1)
    } else {
        token.parse::<i32>().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use client::client::{Client, ClientConfig};
    use client::config::Cache;
    use client::io::ClientRevision;
    use std::sync::Arc;

    fn r289_client() -> Client {
        Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                members: true,
                lowmem: true,
            },
            Arc::new(Cache::default()),
            Arc::new(vec![]),
            Vec::new(),
            ClientRevision::R289,
        )
    }

    #[test]
    fn projection_loc_build_gate_requires_demand_even_when_scene_is_available() {
        assert!(!GameSnapshot::projection_locs_should_build(true, false));
        assert!(GameSnapshot::projection_locs_should_build(true, true));
        assert!(!GameSnapshot::projection_locs_should_build(false, true));
    }
    #[test]
    fn projection_locs_follow_demand_independently_of_loc_generation() {
        let mut client = r289_client();
        client.ingame = true;
        client.scene_state = 2;
        client.gens.scene = 1;
        let mut snapshot = GameSnapshot::new();
        snapshot.loc_bits_prelude = Some((0, 0));
        snapshot.projection_scene.available = true;

        assert!(snapshot.rebuild_loc(&client));
        assert!(snapshot.projection_scene().locs.is_empty());
        assert!(!snapshot.projection_locs_built);

        snapshot.set_loc_projection_wanted(true);
        assert!(!snapshot.rebuild_loc(&client));
        assert!(snapshot.projection_scene().locs.is_empty());
        assert!(snapshot.projection_locs_built);
        snapshot.projection_scene.locs = vec![ProjectionLocView::default()].into();

        snapshot.set_loc_projection_wanted(false);
        assert!(!snapshot.rebuild_loc(&client));
        assert!(snapshot.projection_scene().locs.is_empty());
        assert!(!snapshot.projection_locs_built);
    }

    #[test]
    fn projection_loc_list_is_built_only_when_a_ready_scene_is_demanded() {
        let mut client = r289_client();
        client.ingame = true;
        client.scene_state = 2;
        client.gens.scene = 1;
        client.map_build_base_x = 3200;
        client.map_build_base_z = 3200;
        client.world.set_wall(
            0,
            3,
            4,
            0,
            0,
            0,
            0x4000_0000 + (1 << 14) + 3 + (4 << 7),
            1 << 6,
            0,
            0,
            0,
            0,
        );

        let mut snapshot = GameSnapshot::new();
        // Keep this test on the same scene/loc rebuild seam as the full
        // snapshot path, where the scene bundle may remain published while a
        // run changes its projection-loc demand.
        snapshot.loc_bits_prelude = Some((0, 0));
        snapshot.set_loc_projection_wanted(true);
        snapshot.rebuild_scene(&client);
        assert!(snapshot.projection_scene().available);
        assert!(snapshot.rebuild_loc(&client));
        assert_eq!(snapshot.projection_scene().locs.len(), 1);

        snapshot.set_loc_projection_wanted(false);
        assert!(!snapshot.rebuild_loc(&client));
        assert!(
            snapshot.projection_scene().locs.is_empty(),
            "a ready scene without a run requesting projection locs must publish none"
        );

        snapshot.set_loc_projection_wanted(true);
        assert!(!snapshot.rebuild_loc(&client));
        assert_eq!(snapshot.projection_scene().locs.len(), 1);
    }

    /// The projection height and link grids (about 99 KB per scene) are built
    /// only while a reader asks for them, and a change of demand republishes
    /// the scene without a scene generation move.
    #[test]
    fn projection_scene_grids_follow_demand() {
        let mut client = r289_client();
        client.ingame = true;
        client.scene_state = 2;
        client.gens.scene = 1;
        let mut snapshot = GameSnapshot::new();
        snapshot.rebuild_scene(&client);
        assert!(!snapshot.projection_scene().available);
        assert!(snapshot.projection_scene().heights.is_empty());
        assert!(snapshot.projection_scene().link_below.is_empty());

        snapshot.set_loc_projection_wanted(true);
        snapshot.rebuild_scene(&client);
        assert!(snapshot.projection_scene().available);
        assert_eq!(snapshot.projection_scene().heights.len(), 105 * 105);
        assert_eq!(snapshot.projection_scene().link_below.len(), 104 * 104);

        snapshot.set_loc_projection_wanted(false);
        snapshot.rebuild_scene(&client);
        assert!(!snapshot.projection_scene().available);
        assert!(snapshot.projection_scene().heights.is_empty());
    }

    #[test]
    fn tutorial_pending_follows_the_pending_box() {
        let mut client = r289_client();
        let mut snapshot = GameSnapshot::new();
        // Overlay up, no box yet.
        client.tut_com_id = 1;
        snapshot.rebuild_modals(&client);
        assert!(!snapshot.tutorial_pending());
        // The probe reply captures into the box.
        client.add_chat(0, "get tutorial: 1", "");
        snapshot.rebuild_modals(&client);
        assert!(snapshot.tutorial_pending());
        // A later chat line leaves the box in place: key on the box, not
        // the ring head.
        client.add_chat(0, "Welcome to RuneScape", "");
        snapshot.rebuild_modals(&client);
        assert!(snapshot.tutorial_pending());
        // The click-ack clears the box with no packet family moving; the
        // copy is unconditional so the fact still refreshes.
        client.tut_com_message = None;
        snapshot.rebuild_modals(&client);
        assert!(
            !snapshot.tutorial_pending(),
            "the clear path bumps no gen — a gated copy would stick true"
        );
    }
    const VIEW_RESOLUTION_ITERATIONS: usize = 128;
    const ALLOCATION_ROUNDING_UNIT_BYTES: usize = 8;
    // R274, R289 and R377 share this Linux-measured 128-resolution baseline.
    const VIEW_RESOLUTION_BASELINE: [usize; 4] = [640, 21_248, 640, 19_712];

    fn snapshot_view_resolution_allocations(revision: ClientRevision) -> [usize; 4] {
        use client::config::runtime_377::{
            CacheFields377, InterfaceFields377, LocFields377, NpcFields377,
        };
        use client::config::{CacheRevision377, LocType, NpcType};

        let mut cache = Cache::default();
        let loc = LocType {
            name: "loc name with a heap-backed payload".to_owned(),
            desc: "location description with a heap-backed payload".to_owned(),
            op: vec![
                Some("Open the location".to_owned()),
                Some("Search the location".to_owned()),
            ],
            ..LocType::default()
        };
        cache.locs.push(loc);

        let npc_type = NpcType {
            name: "npc name with a heap-backed payload".to_owned(),
            op: vec![
                Some("Talk to the npc".to_owned()),
                Some("Attack the npc".to_owned()),
            ],
            ..NpcType::default()
        };
        cache.npcs.push(npc_type);
        if revision == ClientRevision::R377 {
            cache.revision_377 = Some(Box::new(CacheRevision377 {
                config: CacheFields377 {
                    locs: vec![LocFields377 { morph: None }],
                    npcs: vec![NpcFields377 {
                        field_1451: -1,
                        field_1430: -1,
                        field_1440: -1,
                        field_1434: true,
                        morph: None,
                    }],
                    ..Default::default()
                },
                interfaces: InterfaceFields377 {
                    client_code_layers: Default::default(),
                    type1_fields: Vec::new(),
                    null_string_masks: Vec::new(),
                    initial_overlays: Vec::new(),
                },
            }));
        }

        let client = Client::from_shared_with_revision(
            ClientConfig {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                members: true,
                lowmem: true,
            },
            Arc::new(cache),
            Arc::new(vec![]),
            Vec::new(),
            revision,
        );
        let mut npc = client::client::ClientNpc {
            r#type: Some(0),
            ..Default::default()
        };
        npc.entity.chat_message = Some("npc overhead text with heap-backed storage".to_owned());

        let resolve_loc = || {
            std::hint::black_box(loc_view(
                9,
                (2 << 6) | 10,
                LocLayer::Wall,
                (3_200, 3_400),
                1,
                12,
                18,
                Some((3_210, 3_405)),
                &client,
            ));
        };
        let resolve_npc = || {
            std::hint::black_box(NpcView::from_slot(
                17,
                &npc,
                (3_200, 3_400),
                1,
                6,
                100,
                &client,
            ));
        };

        for _ in 0..2 {
            resolve_loc();
            resolve_npc();
        }

        let loc = allocation_counter::measure(|| {
            for _ in 0..VIEW_RESOLUTION_ITERATIONS {
                resolve_loc();
            }
        });
        let npc = allocation_counter::measure(|| {
            for _ in 0..VIEW_RESOLUTION_ITERATIONS {
                resolve_npc();
            }
        });
        [
            loc.count_total as usize,
            loc.bytes_total as usize,
            npc.count_total as usize,
            npc.bytes_total as usize,
        ]
    }

    #[test]
    fn loc_and_npc_view_resolution_allocation_gate_for_each_revision() {
        for revision in [
            ClientRevision::R274,
            ClientRevision::R289,
            ClientRevision::R377,
        ] {
            let allocations = snapshot_view_resolution_allocations(revision);
            let expected = VIEW_RESOLUTION_BASELINE;
            assert_eq!(
                allocations[0], expected[0],
                "{revision:?} LocView allocation count changed across 128 unchanged resolutions"
            );
            assert!(
                allocations[1] <= expected[1] + ALLOCATION_ROUNDING_UNIT_BYTES,
                "{revision:?} LocView allocated {} bytes across 128 resolutions; baseline {} plus one allocator rounding unit",
                allocations[1],
                expected[1]
            );
            assert_eq!(
                allocations[2], expected[2],
                "{revision:?} NpcView allocation count changed across 128 unchanged resolutions"
            );
            assert!(
                allocations[3] <= expected[3] + ALLOCATION_ROUNDING_UNIT_BYTES,
                "{revision:?} NpcView allocated {} bytes across 128 resolutions; baseline {} plus one allocator rounding unit",
                allocations[3],
                expected[3]
            );
            eprintln!("snapshot view allocations {revision:?}: {allocations:?}");
        }
    }
}
