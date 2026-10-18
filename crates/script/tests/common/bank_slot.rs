//! Test driver for compatibility scripts whose bank calls execute in the real
//! `SlotScript` bank seat. It posts the script-facing page and supplies a
//! native `GameSnapshot`; it synthesizes no bank operation or result.

#![allow(
    dead_code,
    reason = "shared across integration test binaries; each binary uses a subset, so expect would misfire"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use api::interact::Driver;
use api::named_banks::NamedBankFacts;
use api::prot::Out;
use api::snapshot::GameSnapshot;
use script::isolate_fb::{NativeFactsInput, SnapshotInput};
use script::native::HostAction;
use script::slot::{RunState, SlotScript, StartOutcome, StartPoll};
use script::{CompiledTick, LoadShape, ScriptCtx};
#[derive(Default)]
struct NullOut;

impl Out for NullOut {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _value: &str) {}
}

#[derive(Default)]
struct NullDriver {
    out: NullOut,
}

impl Driver for NullDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}
    fn do_action(&mut self, _slot: i32) -> bool {
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
        _force_approach: i32,
        _kind: i32,
    ) -> bool {
        true
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
        true
    }
}

pub struct BankSlot {
    pub slot: SlotScript,
    native_snapshot: GameSnapshot,
    native_reach: Option<api::query::ReachQueryView>,
    held: bool,
}

impl BankSlot {
    pub fn new(
        source: impl Into<String>,
        shape: LoadShape,
        settings: Option<&serde_json::Map<String, serde_json::Value>>,
        named_banks: Arc<NamedBankFacts>,
    ) -> Self {
        let game_data = api::game_data::for_revision(api::selected::ClientRevision::R289)
            .expect("R289 selected game data");
        let mut slot = SlotScript::new();
        slot.bind_incarnation(1);
        slot.start_load_with_settings_and_game_data(
            source.into(),
            shape,
            settings,
            vec![],
            &[],
            Some(game_data),
            named_banks,
        )
        .expect("start test SlotScript");

        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match slot.poll_start() {
                StartPoll::Settled(StartOutcome::Ready) => break,
                StartPoll::Settled(outcome) => panic!("Load start failed: {outcome:?}"),
                StartPoll::Pending => assert!(Instant::now() < deadline, "Load start stalled"),
                StartPoll::NotOwed => panic!("no pending Load start"),
            }
            std::thread::yield_now();
        }
        slot.on_is_up(true);
        assert_eq!(slot.state(), RunState::Running);
        Self {
            slot,
            native_snapshot: GameSnapshot::new(),
            native_reach: None,
            held: false,
        }
    }

    /// Publish a full/delta script page while updating the persistent native view.
    pub fn post_snapshot(&mut self, input: &SnapshotInput<'_>, native: NativeFactsInput<'_>) {
        let native = self.with_slot_bank_selection(native);
        let bytes = self
            .slot
            .encode_snapshot_delta_with_native(input, native, false);
        self.post_snapshot_bytes(bytes, input, native);
    }

    fn with_slot_bank_selection(
        &self,
        mut native: NativeFactsInput<'_>,
    ) -> NativeFactsInput<'_> {
        native.bank_selection = self.slot.bank_selection_input();
        native
    }

    /// Post a caller-encoded delta while keeping the native view independent.
    pub fn post_snapshot_bytes(
        &mut self,
        bytes: Vec<u8>,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) {
        let native = self.with_slot_bank_selection(native);
        assert!(
            self.slot.post_snapshot(bytes),
            "SlotScript rejected snapshot"
        );
        self.held = input.hold;
        let mut snapshot = std::mem::replace(&mut self.native_snapshot, GameSnapshot::new());
        update_native_snapshot(&mut snapshot, input, native, None);
        self.native_snapshot = snapshot;
        self.native_reach = reach_query_view(input.reach);
    }

    /// Tick against the persistent native view last posted by `post_snapshot`.
    pub fn tick_posted(&mut self, tick: u64) {
        let snapshot = std::mem::replace(&mut self.native_snapshot, GameSnapshot::new());
        self.run_game_tick(tick, &snapshot);
        self.native_snapshot = snapshot;
    }

    /// Publish the script-facing page, then run against the caller's native snapshot.
    pub fn tick(
        &mut self,
        tick: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        snapshot: &GameSnapshot,
    ) {
        self.post_snapshot(input, native);
        self.use_raw_reach(input, snapshot);
        self.run_game_tick(tick, snapshot);
        self.pump_snapshot_changes(tick, input, native, snapshot);
    }

    /// Build the native view from the posted fixture and retain session identity.
    pub fn tick_from_input(
        &mut self,
        tick: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) {
        self.post_snapshot(input, native);
        let snapshot = std::mem::replace(&mut self.native_snapshot, GameSnapshot::new());
        self.run_game_tick(tick, &snapshot);
        self.pump_snapshot_changes(tick, input, native, &snapshot);
        self.native_snapshot = snapshot;
    }
    /// Run snapshot-only work with the same native frame context as a game tick.
    pub fn snapshot_change(
        &mut self,
        tick: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        snapshot: &GameSnapshot,
    ) {
        self.post_snapshot(input, native);
        self.use_raw_reach(input, snapshot);
        self.run_snapshot_change(tick, snapshot);
        self.pump_snapshot_changes(tick, input, native, snapshot);
    }

    fn use_raw_reach(&mut self, input: &SnapshotInput<'_>, snapshot: &GameSnapshot) {
        if !input.reach.available {
            let here = snapshot
                .tile()
                .map(|(x, z, level)| api::snapshot::WorldTile { x, z, level });
            self.native_reach = api::query::SceneQuery::new(snapshot.scene(), here)
                .flood_reach()
                .map(|flood| api::query::pack_reach_query(snapshot.scene(), Some(&flood)));
        }
    }

    /// Carry admission, prompt decisions and terminal pages through ordinary
    /// host frames without advancing the observed game tick or its budget.
    fn pump_snapshot_changes(
        &mut self,
        tick: u64,
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
        snapshot: &GameSnapshot,
    ) {
        for _ in 0..8 {
            let native = self.with_slot_bank_selection(native);
            let bytes = self
                .slot
                .encode_snapshot_delta_with_native(input, native, false);
            assert!(self.slot.post_snapshot(bytes));
            self.run_snapshot_change(tick, snapshot);
        }
    }

    fn run_snapshot_change(&mut self, tick: u64, snapshot: &GameSnapshot) {
        self.slot.seat_guardian_hold(self.held);
        self.slot.sync_compiled_clue(self.held);
        self.drain_seat_starts();
        let mut driver = NullDriver::default();
        let mut ctx = ScriptCtx {
            driver: &mut driver,
            tick,
            here: snapshot.tile(),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: CompiledTick {
                hold: self.held,
                reach: self.native_reach.as_ref(),
                ..CompiledTick::default()
            },
        };
        self.slot.on_snapshot_change(&mut ctx);
        self.slot
            .probe("true")
            .expect("join the completed Load snapshot callback");
    }

    fn run_game_tick(&mut self, tick: u64, snapshot: &GameSnapshot) {
        self.slot.seat_guardian_hold(self.held);
        self.slot.sync_compiled_clue(self.held);
        self.drain_seat_starts();
        let mut driver = NullDriver::default();
        let mut ctx = ScriptCtx {
            driver: &mut driver,
            tick,
            here: snapshot.tile(),
            walk: None,
            walk_with: None,
            inv: None,
            snapshot: Some(snapshot),
            obj_names: None,
            compiled: CompiledTick {
                hold: self.held,
                reach: self.native_reach.as_ref(),
                ..CompiledTick::default()
            },
        };
        self.slot.on_game_tick(&mut ctx);
        self.slot
            .probe("true")
            .expect("join the completed Load tick");
    }

    fn drain_seat_starts(&mut self) {
        let (_, legacy, _) = self.slot.drain_host_interacts();
        assert!(
            legacy.is_empty(),
            "bank fixture emitted legacy effects instead of using the native seat"
        );
    }

    pub fn probe(&self, expression: &str) -> Result<serde_json::Value, String> {
        self.slot.probe(expression)
    }

    pub fn take_actions(&mut self) -> Vec<HostAction> {
        let mut actions = Vec::new();
        while let Some(action) = self.slot.take_native_action() {
            actions.push(action);
        }
        actions
    }
    pub fn reset_session_work(&mut self) {
        self.slot.reset_session_work();
        self.slot.on_is_up(true);
        self.native_snapshot = GameSnapshot::new();
        self.native_reach = None;
        self.held = false;
    }

    pub fn stop(&mut self) {
        self.slot.stop();
    }
}
pub(crate) fn update_native_snapshot(
    snapshot: &mut GameSnapshot,
    input: &SnapshotInput<'_>,
    native: NativeFactsInput<'_>,
    bank_component: Option<i32>,
) {
    use api::obj_names::ItemDefView;
    use api::snapshot::{
        ActorKind, ActorTargetView, ActorView, ItemActionFamily, ItemContainer, ItemView, LocLayer,
        LocView, LocalPlayerView, NpcView, PlayerView, QuestStatusView, StatView, WorldTile,
    };

    fn tile(x: i32, z: i32, level: i32) -> WorldTile {
        WorldTile { x, z, level }
    }

    fn item(row: &script::isolate_fb::ItemRowInput<'_>, container: ItemContainer) -> ItemView {
        let action_family = match container {
            ItemContainer::Bank | ItemContainer::BankSide => ItemActionFamily::Component,
            ItemContainer::Inventory | ItemContainer::Equipment => ItemActionFamily::Held,
            _ => ItemActionFamily::None,
        };
        ItemView {
            def: ItemDefView {
                id: row.id,
                name: row.name.map(str::to_owned),
                stackable: false,
                members: false,
                base_value: 0,
                noted: row.noted,
                certificate_link: row.cert,
                certificate_template: -1,
            },
            container,
            action_family,
            slot: row.slot,
            count: row.count,
            actions: row.ops.iter().cloned().map(Some).collect(),
            component_id: row.component_id,
        }
    }

    fn loc(row: &script::isolate_fb::SceneEntityInput<'_>) -> LocView {
        LocView {
            typecode: row.id,
            info: 0,
            id: row.id,
            name: row.name.map(str::to_owned),
            description: None,
            actions: row.actions.iter().cloned().map(Some).collect(),
            tile: tile(row.x, row.z, row.level),
            distance: row.distance,
            layer: LocLayer::GroundDecoration,
            shape: row.shape,
            angle: row.angle,
            width: 1,
            length: 1,
            footprint_width: 1,
            footprint_length: 1,
            block_walk: false,
            block_range: false,
            active: true,
            animation: -1,
            map_function: -1,
            map_scene: -1,
            force_approach: 0,
        }
    }

    fn npc(row: &script::isolate_fb::SceneEntityInput<'_>) -> NpcView {
        NpcView {
            index: usize::try_from(row.index).unwrap_or_default(),
            r#type: usize::try_from(row.id).ok(),
            name: row.name.map(str::to_owned),
            actions: row.actions.iter().cloned().map(Some).collect(),
            tile: tile(row.x, row.z, row.level),
            distance: row.distance,
            animation: if row.animating { 0 } else { -1 },
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
            health: row.health,
            total_health: row.max_health,
            face_entity: -1,
            target: match row.target_kind {
                1 => usize::try_from(row.target_index)
                    .ok()
                    .map(|index| ActorTargetView {
                        kind: ActorKind::Npc,
                        index,
                    }),
                2 => usize::try_from(row.target_index)
                    .ok()
                    .map(|index| ActorTargetView {
                        kind: ActorKind::Player,
                        index,
                    }),
                _ => None,
            },
            moving: false,
            running: false,
            in_combat: row.in_combat,
            level: row.combat_level,
            size: row.size.max(1),
            network: tile(row.nx, row.nz, row.level),
            x: row.x,
            z: row.z,
            yaw: row.angle,
        }
    }

    fn local_player(
        input: &SnapshotInput<'_>,
        native: NativeFactsInput<'_>,
    ) -> Option<LocalPlayerView> {
        let here = input.here?;
        let tile = tile(here.x, here.z, here.level);
        let actor = ActorView {
            name: input.my_name.map(str::to_owned),
            actions: Vec::new(),
            tile,
            distance: 0,
            animation: if input.animating { 0 } else { -1 },
            animation_frame: 0,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: 0,
            health: 0,
            total_health: 0,
            face_entity: -1,
            target: None,
            moving: native.local_player_moving.unwrap_or(false),
            running: input.run_enabled,
            in_combat: input.in_combat,
        };
        Some(LocalPlayerView {
            player: PlayerView {
                index: usize::try_from(input.self_slot).unwrap_or_default(),
                actor,
                network: tile,
                combat_level: input.combat_level,
                skill_level: 0,
                headicons: 0,
                weapon: None,
            },
            energy: input.run_energy,
            weight: input.weight,
            animation_update: None,
        })
    }

    if input.ingame {
        snapshot.seed_ingame(input.scene_state);
    }
    snapshot.seed_tick(input.tick as u32);
    if let Some(here) = input.here {
        snapshot.seed_tile(tile(here.x, here.z, here.level));
    }
    // These input fixtures model an open scene with the posted blocked tiles.
    // Wall-specific cases can pass their raw native snapshot to `tick`.
    let scene = if input.reach.available {
        let cells = usize::try_from(input.reach.width * input.reach.height)
            .expect("nonnegative fixture scene dimensions");
        api::snapshot::SceneView {
            available: true,
            base_x: input.reach.base_x,
            base_z: input.reach.base_z,
            level: input.reach.level,
            width: input.reach.width,
            height: input.reach.height,
            collision_flags: (0..cells)
                .map(|cell| {
                    if input.reach.walkable[cell / 32] & (1 << (cell % 32)) != 0 {
                        0
                    } else {
                        i32::MAX
                    }
                })
                .collect(),
        }
    } else {
        api::snapshot::SceneView::default()
    };
    snapshot.seed_scene(scene);
    if let Some(player) = local_player(input, native) {
        snapshot.seed_local_player(player);
    }
    snapshot.seed_runenergy(input.run_energy);
    snapshot.seed_inventory(
        input
            .inv
            .iter()
            .map(|row| item(row, ItemContainer::Inventory))
            .collect(),
        input.inv_size,
    );
    snapshot.seed_inventory_packet_generations(Some(input.tick), Some(input.tick));
    snapshot.seed_equipment(
        input
            .equipment
            .iter()
            .map(|row| item(row, ItemContainer::Equipment))
            .collect(),
    );
    let bank_rows = input
        .bank
        .iter()
        .map(|row| item(row, ItemContainer::Bank))
        .collect();
    let side_rows = input
        .bank_side
        .iter()
        .map(|row| item(row, ItemContainer::BankSide))
        .collect();
    let bank_component = bank_component.unwrap_or_else(|| {
        if input.bank_open {
            input
                .bank
                .first()
                .map(|row| row.component_id)
                .filter(|component| *component >= 0)
                .unwrap_or(700)
        } else {
            -1
        }
    });
    let packet_generation = match native.bank_snapshot_generation {
        Some(generation) => u64::try_from(generation).unwrap_or(0),
        None if input.bank_open => snapshot.bank_snapshot_generation().unwrap_or(0),
        None => 0,
    };
    snapshot.seed_bank_observation(
        bank_component,
        packet_generation,
        (input.bank_open && input.bank_loaded).then_some(bank_rows),
        side_rows,
    );
    if let Some(side_modal_id) = native.side_modal_id {
        snapshot.seed_side_modal(side_modal_id);
    }
    snapshot.seed_stats(
        input
            .stats
            .iter()
            .map(|row| StatView {
                index: row.index,
                name: row.name.to_owned(),
                effective: row.effective,
                base: row.base,
                xp: row.xp,
                used: api::snapshot::stat_used(row.index.max(0) as usize),
            })
            .collect(),
    );
    snapshot.seed_locs(input.locs.iter().map(loc).collect());
    snapshot.seed_npcs(input.npcs.iter().map(npc).collect());
    snapshot.seed_chat_options(
        input
            .chat_options
            .iter()
            .map(|option| api::snapshot::ChatOptionView {
                component_id: option.com_id,
                text: option.text.to_owned(),
            })
            .collect(),
        if input.chat_continue {
            input.chat_modal_id
        } else {
            -1
        },
    );
    snapshot.seed_chat_modal(
        input.chat_modal_id,
        input.chat_text.into_iter().map(str::to_owned).collect(),
    );

    if let Some(quests) = native.quest_statuses {
        snapshot.seed_quest_statuses(
            quests
                .iter()
                .map(|row| QuestStatusView {
                    component_id: row.component_id.unwrap_or(-1),
                    name: row.name.to_owned(),
                    colour: match row.status.to_ascii_lowercase().as_str() {
                        "complete" => 0x00f800,
                        "inprogress" => 0xf8f800,
                        "notstarted" => 0xf80000,
                        _ => 0,
                    },
                })
                .collect(),
            true,
        );
    }
}

fn reach_query_view(
    reach: script::isolate_fb::ReachViewInput<'_>,
) -> Option<api::query::ReachQueryView> {
    reach.available.then(|| api::query::ReachQueryView {
        available: true,
        base_x: reach.base_x,
        base_z: reach.base_z,
        level: reach.level,
        width: reach.width,
        height: reach.height,
        walkable: reach.walkable.to_vec(),
        reachable: reach.reachable.to_vec(),
        reachable_adj: reach.reachable_adj.to_vec(),
        exact_rank: reach.exact_rank.to_vec(),
        adjacent_rank: reach.adjacent_rank.to_vec(),
        step: reach.step.to_vec(),
        canlight: reach.canlight.to_vec(),
    })
}
