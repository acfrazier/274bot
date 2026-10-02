//! A stack-only borrow of the observations needed by the combat core.
use api::snapshot::{
    CombatView, HitmarkView, ItemView, LocalPlayerView, NpcView, PlayerView, ProjectileView,
    SnapshotView, StatView, VarpView, WorldStateView, WorldTile,
};

/// Ready observations for one combat decision. Slices remain borrowed from the
/// snapshot; only compact scalar overlays and the four hitmarks are copied.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub here: WorldTile,
    pub combat_tab: Option<i32>,
    pub tick: u16,
    pub stats: &'a [StatView],
    pub inventory: &'a [ItemView],
    pub equipment: &'a [ItemView],
    pub npcs: &'a [NpcView],
    pub players: &'a [PlayerView],
    pub local: &'a LocalPlayerView,
    pub world: &'a WorldStateView,
    pub projectiles: &'a [ProjectileView],
    pub hitmarks: [HitmarkView; 4],
    pub loop_cycle: i32,
    pub prayers: [bool; 15],
    pub varps: &'a [VarpView],
    pub in_combat: CombatView,
}

impl<'a> Frame<'a> {
    /// Build a complete combat frame. An unavailable family is not treated as
    /// an empty observation, so any missing phase input keeps the caller Pending.
    pub fn borrow(snapshot: SnapshotView<'a>) -> Option<Self> {
        let here_observed = snapshot.here()?;
        let here = here_observed.value;
        let tick = here_observed.stamp.tick as u16;
        let stats = snapshot.stats()?.value;
        let inventory = snapshot.inventory()?.value;
        let equipment = snapshot.equipment()?.value;
        let npcs = snapshot.npcs()?.value;
        let players = snapshot.players()?.value;
        let local = snapshot.local_player()?.value;
        let world = snapshot.world()?.value;
        let projectiles = snapshot.projectiles()?.value;
        let hitmarks = snapshot.hitmarks()?.value;
        let prayers = snapshot.prayers_active()?.value;
        let varps = snapshot.varps()?.value;
        let combat_tab = snapshot.side_tabs().and_then(|tabs| {
            tabs.value
                .iter()
                .find(|tab| tab.index == 0 && tab.available)
                .map(|tab| tab.root_component_id)
        });
        let in_combat = snapshot.in_combat()?.value;
        Some(Self {
            here,
            combat_tab,
            tick,
            stats,
            inventory,
            equipment,
            npcs,
            players,
            local,
            world,
            projectiles,
            hitmarks: hitmarks.marks,
            loop_cycle: hitmarks.loop_cycle,
            prayers,
            varps,
            in_combat,
        })
    }

    #[inline]
    pub fn me(&self) -> usize {
        self.local.player.index
    }
}
