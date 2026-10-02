//! Rust-native gathering for woodcutting, mining and fishing.
//!
//! Start, Custom and bounded Auto use observed resource identity and content
//! Power drops selected products; Bank mode deposits products and maintains
//! supplies from loaded bank observations. Combat and death recovery remain
//! later-stage capabilities.

mod area;
mod card;
mod drop;
mod gather;
mod oneop;
mod runner;
mod select;
pub mod settings;
pub mod status;
mod supply;
mod widen;

pub use area::{AreaMode, WorkArea};
pub use card::CARD;
pub use runner::Gatherer;
pub use settings::{GathererSettings, Skill};

#[cfg(test)]
pub(crate) fn test_full_pack_fixture(
    cx: &mut crate::native::PrepareContext<'_>,
) -> Result<
    (
        std::sync::Arc<crate::native::PreparedConfig>,
        api::snapshot::GameSnapshot,
    ),
    crate::native::StartError,
> {
    use api::gather_methods::{known_rows, SceneRegionInput};
    use api::selected::EntityId;
    use api::snapshot::{
        ActorView, ItemContainer, ItemView, LocLayer, LocView, LocalPlayerView, PlayerView,
        StatView, WorldStateView,
    };
    use api::ItemDefView;
    use std::sync::Arc;

    let mut settings = crate::native::SettingsBag::new();
    settings.insert(
        "disposition".into(),
        serde_json::Value::String("Power".into()),
    );
    let config = card::prepare(cx, 1, Arc::new(settings))?;
    let prepared = config
        .get::<Arc<card::Prepared>>()
        .expect("Gatherer card preparation returns its own payload");
    let region = SceneRegionInput {
        min_x: i32::MIN / 2,
        min_z: i32::MIN / 2,
        max_x: i32::MAX / 2,
        max_z: i32::MAX / 2,
        level: 0,
    };
    let mut chosen = None;
    for method in prepared.catalog.methods_for_resource("normal") {
        let Some(target_id) = known_rows(&method.targets).iter().find_map(|target| {
            (target.class == api::gather_methods::TargetClass::Resource)
                .then_some(target.entity)
                .and_then(|entity| match entity {
                    EntityId::Loc(id) => Some(id),
                    _ => None,
                })
        }) else {
            continue;
        };
        let mut spots = prepared
            .catalog
            .spots(method, &region)
            .map_err(crate::native::StartError::Facts)?;
        if let Some(spot) = spots.find(|spot| spot.entity == EntityId::Loc(target_id)) {
            chosen = Some((target_id, spot.origin));
            break;
        }
    }
    let Some((tree_id, tree_tile)) = chosen else {
        return Err(crate::native::StartError::Unavailable(
            "Gatherer fixture has no complete normal-tree placement".into(),
        ));
    };

    let item_def = |id: i32, name: &str| ItemDefView {
        id,
        name: Some(name.into()),
        stackable: false,
        members: false,
        base_value: 0,
        noted: false,
        certificate_link: -1,
        certificate_template: -1,
    };
    let logs = (0..28)
        .map(|slot| ItemView {
            def: item_def(1511, "Logs"),
            container: ItemContainer::Inventory,
            action_family: api::snapshot::ItemActionFamily::Held,
            slot,
            count: 1,
            actions: vec![Some("Drop".into())],
            component_id: -1,
        })
        .collect();
    let axe = ItemView {
        def: item_def(1351, "Bronze axe"),
        container: ItemContainer::Equipment,
        action_family: api::snapshot::ItemActionFamily::Held,
        slot: 0,
        count: 1,
        actions: vec![Some("Remove".into())],
        component_id: -1,
    };
    let actor = ActorView {
        name: Some("fixture".into()),
        actions: Vec::new(),
        tile: tree_tile,
        distance: 0,
        animation: -1,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        health: 10,
        total_health: 10,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
    };
    let mut snapshot = api::snapshot::GameSnapshot::new();
    // Loc arrival requires an observed live scene, not merely a seeded origin.
    let mut client = client::client::Client::new(client::client::ClientConfig {
        host: "127.0.0.1".into(),
        port: 1,
        cache_dir: String::new(),
        members: true,
        lowmem: true,
    });
    client.ingame = true;
    client.scene_state = 2;
    client.map_build_base_x = tree_tile.x.saturating_sub(52);
    client.map_build_base_z = tree_tile.z.saturating_sub(52);
    client.minusedlevel = tree_tile.level;
    client.bump_gens(client::io::ServerProt::REBUILD_NORMAL);
    snapshot.rebuild(&client);
    snapshot.seed_ingame(2);
    snapshot.seed_npcs(Vec::new());
    snapshot.seed_world(WorldStateView {
        map_base_x: tree_tile.x.saturating_sub(52),
        map_base_z: tree_tile.z.saturating_sub(52),
        level: tree_tile.level,
        members: true,
        multi_combat: false,
        player_count: 1,
        npc_count: 0,
        cycle: 1,
    });
    snapshot.seed_local_player(LocalPlayerView {
        player: PlayerView {
            index: 0,
            actor,
            combat_level: 3,
            skill_level: 1,
        },
        energy: 100,
        weight: 0,
    });
    snapshot.seed_stats(vec![StatView {
        index: 8,
        name: "woodcutting".into(),
        effective: 1,
        base: 1,
        xp: 0,
        used: true,
    }]);
    snapshot.seed_inventory(logs, 28);
    snapshot.seed_equipment(vec![axe]);
    snapshot.seed_locs(vec![LocView {
        typecode: 10,
        info: 0,
        id: tree_id,
        name: Some("Tree".into()),
        description: None,
        actions: vec![Some("Chop down".into())],
        tile: tree_tile,
        distance: 0,
        layer: LocLayer::Ground,
        shape: 10,
        angle: 0,
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
    }]);
    Ok((config, snapshot))
}

use api::snapshot::WorldTile;

/// Recovery is retained now so the slot cell has a stable shape for later
/// stages. G1 only ever uses `Idle`; a latched death is a non-retryable block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecoveryState {
    #[default]
    Idle,
    Pending {
        step: u8,
    },
    Proving,
}

/// Small slot-retained state that survives a watchdog recreation and a
/// reconnect, but is discarded when the operator stops the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatherRetained {
    pub anchor: Option<WorldTile>,
    pub start_tile: Option<WorldTile>,
    pub deaths: u8,
    pub recoveries: u8,
    pub death_seq: Option<i32>,
    pub recovery: RecoveryState,
    pub haul_since_death: bool,
    pub yielded: u32,
    pub deposited: u32,
    pub dropped: u32,
}

impl Default for GatherRetained {
    fn default() -> Self {
        Self {
            anchor: None,
            start_tile: None,
            deaths: 0,
            recoveries: 0,
            death_seq: None,
            recovery: RecoveryState::Idle,
            haul_since_death: false,
            yielded: 0,
            deposited: 0,
            dropped: 0,
        }
    }
}

impl GatherRetained {
    pub const fn is_fresh(&self) -> bool {
        self.anchor.is_none()
            && self.start_tile.is_none()
            && self.deaths == 0
            && self.recoveries == 0
            && self.death_seq.is_none()
            && matches!(self.recovery, RecoveryState::Idle)
            && !self.haul_since_death
            && self.yielded == 0
            && self.deposited == 0
            && self.dropped == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::size_of;

    #[test]
    fn retained_cell_stays_within_the_eighty_byte_budget() {
        assert!(size_of::<GatherRetained>() <= 80);
    }
}
