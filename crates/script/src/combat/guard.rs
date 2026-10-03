//! Host-side protected-walk driver. Hold-mode prayer only: no eat, no attack,
//! no flick. The host follow sites own one driver per armed route.
use super::arbiter;
use super::frame::Frame;
use super::schedule::reached;
use super::select;
use super::tables::{CombatTables, PotionKind, PrayerRole};
use super::threats::ThreatSet;
use crate::native::WalkRequest;
use api::game_data::PrayerFact;
use api::selected::ClientRevision;
use api::snapshot::SnapshotView;
use std::sync::{Arc, LazyLock};

const LOWEST_PROTECT: i32 = 37;
const PRAYER_STAT: i32 = 5;
const HITPOINTS_STAT: i32 = 3;

/// Protection prayer the driver wanted but could not raise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardProtect {
    Magic,
    Missiles,
    Melee,
}

/// One host-executed decision. A walk hop plus one of these is two user
/// events, never an exclusive combat batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardOp {
    IfButton { component: i32 },
    Drink { name: Arc<str> },
    Locked { until: u16 },
    Unprotectable { protect: GuardProtect },
}

/// Why [`WalkGuard::begin`] refused before the route was armed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuardRefusal {
    PrayerTooLow,
    PrayerDisallowed,
    Snapshot,
    Tables,
}

/// Per-followed-route protect driver. Threats plus a few clocks; tables live
/// behind an `Arc`.
pub struct WalkGuard {
    threats: ThreatSet,
    tables: Arc<CombatTables>,
    last_tick: u16,
    input_lock: u16,
    earliest_prayer: u16,
    earliest_drink: u16,
    drop_component: i32,
    unprotectable: u8,
    flags: u8,
}

const FLAG_LOCK: u8 = 1;
const FLAG_SEEN: u8 = 2;

const _: () = assert!(std::mem::size_of::<WalkGuard>() <= 256);

static TABLES: LazyLock<Option<Arc<CombatTables>>> = LazyLock::new(|| {
    api::game_data::for_revision(ClientRevision::R289)
        .ok()
        .and_then(|data| CombatTables::build(data).ok())
});

fn shared_tables() -> Result<Arc<CombatTables>, GuardRefusal> {
    TABLES.clone().ok_or(GuardRefusal::Tables)
}

fn prayer_base(snapshot: &SnapshotView<'_>) -> Option<i32> {
    snapshot
        .stats()?
        .value
        .iter()
        .find(|row| row.index == PRAYER_STAT)
        .map(|row| row.base)
}

fn protect_kind(tables: &CombatTables, fact: &PrayerFact) -> Option<GuardProtect> {
    (0..3).find_map(|tier| {
        tables
            .prayer(PrayerRole::Protect, tier)
            .filter(|row| row.varp == fact.varp)
            .map(|_| match tier {
                0 => GuardProtect::Magic,
                1 => GuardProtect::Missiles,
                _ => GuardProtect::Melee,
            })
    })
}

fn kind_bit(kind: GuardProtect) -> u8 {
    match kind {
        GuardProtect::Magic => 1,
        GuardProtect::Missiles => 2,
        GuardProtect::Melee => 3,
    }
}

impl WalkGuard {
    /// Documented begin. Refuses before the route is armed when no protect
    /// is reachable at all.
    pub fn begin(request: &WalkRequest, snapshot: &SnapshotView<'_>) -> Result<Self, GuardRefusal> {
        Self::begin_with(request, snapshot, shared_tables()?)
    }

    /// Host/tests supply the pin's tables explicitly.
    pub fn begin_with(
        _request: &WalkRequest,
        snapshot: &SnapshotView<'_>,
        tables: Arc<CombatTables>,
    ) -> Result<Self, GuardRefusal> {
        let base = prayer_base(snapshot).ok_or(GuardRefusal::Snapshot)?;
        if base < LOWEST_PROTECT {
            return Err(GuardRefusal::PrayerTooLow);
        }
        Ok(Self {
            threats: ThreatSet::default(),
            tables,
            last_tick: 0,
            input_lock: 0,
            earliest_prayer: 0,
            earliest_drink: 0,
            drop_component: 0,
            unprotectable: 0,
            flags: 0,
        })
    }

    /// Previous drink lock still covers `tick`: the route owner issues no
    /// follow click.
    pub fn blocks_follow(&self, tick: u16) -> bool {
        self.flags & FLAG_LOCK != 0 && !reached(tick, self.input_lock)
    }

    /// Observe threats and, when needed, one protect click or prayer dose.
    pub fn tick(&mut self, snapshot: &SnapshotView<'_>) -> Option<GuardOp> {
        let frame = Frame::borrow(*snapshot)?;
        let tick = frame.tick;
        if self.flags & FLAG_SEEN != 0 && tick == self.last_tick {
            return None;
        }
        self.flags |= FLAG_SEEN;
        self.last_tick = tick;
        if self.blocks_follow(tick) {
            return Some(GuardOp::Locked {
                until: self.input_lock,
            });
        }
        self.threats.observe(&frame, &self.tables, tick);
        let (points, base) = arbiter::stat(&frame, PRAYER_STAT);
        let (hp, hp_max) = arbiter::stat(&frame, HITPOINTS_STAT);
        let wanted =
            select::wanted_protect(&self.threats, &frame, &self.tables, tick, false, false);
        if let Some(fact) = select::active_protect(&frame, &self.tables)
            .and_then(|style| select::protect_fact(&self.tables, style))
        {
            self.drop_component = fact.button_com;
        }
        let wanted = wanted?;
        if base < wanted.level {
            let kind = protect_kind(&self.tables, wanted)?;
            let bit = kind_bit(kind);
            if self.unprotectable == bit {
                return None;
            }
            self.unprotectable = bit;
            return Some(GuardOp::Unprotectable { protect: kind });
        }
        self.unprotectable = 0;
        if select::prayer_on(&frame, wanted.varp) {
            self.drop_component = wanted.button_com;
            return self.maybe_floor_sip(&frame, tick, points, base, hp, hp_max);
        }
        if points == 0 {
            return self.drink(&frame, tick, hp, hp_max, true);
        }
        if self.prayer_ready(tick) {
            self.earliest_prayer = tick.wrapping_add(3);
            self.drop_component = wanted.button_com;
            return Some(GuardOp::IfButton {
                component: wanted.button_com,
            });
        }
        None
    }

    /// Always drops the protect this driver raised or last observed.
    pub fn end(self) -> GuardOp {
        GuardOp::IfButton {
            component: self.drop_component,
        }
    }

    fn prayer_ready(&self, tick: u16) -> bool {
        self.earliest_prayer == 0 || reached(tick, self.earliest_prayer)
    }

    fn drink_ready(&self, tick: u16) -> bool {
        self.earliest_drink == 0 || reached(tick, self.earliest_drink)
    }

    fn drink_gate(danger: Option<i32>, hp: i32, hp_max: i32) -> bool {
        match danger {
            Some(danger) => hp > danger.saturating_mul(3).saturating_add(1),
            None => hp > (hp_max + 1) / 2,
        }
    }

    fn maybe_floor_sip(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        points: i32,
        base: i32,
        hp: i32,
        hp_max: i32,
    ) -> Option<GuardOp> {
        let floor = (base - (7 + base / 4)).max(3);
        if points > floor {
            return None;
        }
        self.drink(frame, tick, hp, hp_max, true)
    }

    fn drink(
        &mut self,
        frame: &Frame<'_>,
        tick: u16,
        hp: i32,
        hp_max: i32,
        gated: bool,
    ) -> Option<GuardOp> {
        if !self.drink_ready(tick) {
            return None;
        }
        if gated {
            let danger = self.threats.danger(frame, &self.tables, tick, false, false);
            if !Self::drink_gate(danger, hp, hp_max) {
                return None;
            }
        }
        let id = arbiter::potion_id(frame, &self.tables, PotionKind::Prayer)?;
        let name = frame
            .inventory
            .iter()
            .find(|row| row.def.id == id)
            .and_then(|row| row.def.name.as_deref())
            .map(Arc::<str>::from)?;
        self.earliest_drink = tick.wrapping_add(3);
        self.input_lock = tick.wrapping_add(3);
        self.flags |= FLAG_LOCK;
        Some(GuardOp::Drink { name })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use api::quest_progress::EvidenceStamp;
    use api::selected::RunKey;
    use api::snapshot::{
        ActorKind, ActorTargetView, ActorView, GameSnapshot, HitmarkView, HitmarksView, ItemView,
        LocalPlayerView, NpcView, PlayerView, ProjectileView, StatView, VarpView, WorldStateView,
        WorldTile,
    };
    use api::{obj_names::ItemDefView, snapshot::ItemActionFamily, snapshot::ItemContainer};

    fn dummy_stamp(tick: u64) -> EvidenceStamp {
        EvidenceStamp {
            run: RunKey {
                slot: 0,
                run: 0,
                session: 0,
            },
            tick,
            sequence: 0,
        }
    }

    fn tile(x: i32, z: i32) -> WorldTile {
        WorldTile { x, z, level: 0 }
    }

    fn actor(at: WorldTile) -> ActorView {
        ActorView {
            name: None,
            actions: Vec::new(),
            tile: at,
            distance: 0,
            animation: -1,
            animation_frame: -1,
            pose_animation: -1,
            orientation: 0,
            target_orientation: 0,
            overhead_text: None,
            spot_animation: -1,
            spot_animation_stamp: -1,
            health: 40,
            total_health: 40,
            face_entity: -1,
            target: None,
            moving: false,
            running: false,
            in_combat: false,
        }
    }

    struct Scene {
        tables: Arc<CombatTables>,
        snapshot: GameSnapshot,
        local: LocalPlayerView,
        npcs: Vec<NpcView>,
        stats: Vec<StatView>,
        varps: Vec<VarpView>,
        inventory: Vec<ItemView>,
        projectiles: Vec<ProjectileView>,
    }

    impl Scene {
        fn new(prayer_base: i32) -> Self {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let tables = CombatTables::build(Arc::clone(&data)).unwrap();
            let row = data.npc_by_config("ardougne_archer").unwrap();
            let at = tile(2600, 3200);
            let npc = NpcView {
                index: 7,
                r#type: Some(row.id as usize),
                name: row.display.clone(),
                actions: vec![Some("Attack".into())],
                tile: tile(at.x + 1, at.z),
                distance: 1,
                animation: -1,
                animation_frame: -1,
                pose_animation: -1,
                orientation: 0,
                target_orientation: 0,
                overhead_text: None,
                spot_animation: -1,
                spot_animation_stamp: -1,
                health: 50,
                total_health: 50,
                face_entity: -1,
                target: None,
                moving: false,
                running: false,
                in_combat: false,
                level: 37,
                size: 1,
                network: tile(at.x + 1, at.z),
                x: 0,
                z: 0,
                yaw: 0,
            };
            let varps = data
                .prayers()
                .iter()
                .map(|row| VarpView {
                    index: row.varp,
                    value: 0,
                })
                .collect();
            let mut scene = Self {
                tables,
                snapshot: GameSnapshot::new(),
                npcs: vec![npc],
                local: LocalPlayerView {
                    player: PlayerView {
                        index: 1,
                        actor: actor(at),
                        combat_level: 60,
                        skill_level: 0,
                        weapon: None,
                    },
                    energy: 100,
                    weight: 0,
                },
                stats: (0..25)
                    .map(|index| StatView {
                        index,
                        name: String::new(),
                        effective: if index == 5 { prayer_base } else { 40 },
                        base: if index == 5 { prayer_base } else { 40 },
                        xp: 0,
                        used: api::snapshot::stat_used(index as usize),
                    })
                    .collect(),
                varps,
                inventory: Vec::new(),
                projectiles: Vec::new(),
            };
            scene.refresh();
            scene
        }

        fn refresh(&mut self) {
            self.snapshot.seed_ingame(2);
            self.snapshot.seed_world(WorldStateView {
                map_base_x: 2560,
                map_base_z: 3160,
                members: true,
                ..WorldStateView::default()
            });
            self.snapshot.seed_stats(self.stats.clone());
            self.snapshot.seed_varps(self.varps.clone());
            self.snapshot.seed_inventory(self.inventory.clone(), 28);
            self.snapshot.seed_equipment(Vec::new());
            self.snapshot.seed_npcs(self.npcs.clone());
            self.snapshot.seed_players(Vec::new());
            self.snapshot.seed_projectiles(self.projectiles.clone());
            self.snapshot.seed_local_player(self.local.clone());
            self.snapshot.seed_hitmarks(HitmarksView {
                marks: [HitmarkView {
                    value: 0,
                    kind: 0,
                    cycle: 0,
                }; 4],
                loop_cycle: 0,
            });
        }

        fn view(&self) -> SnapshotView<'_> {
            self.view_at(0)
        }

        fn view_at(&self, tick: u16) -> SnapshotView<'_> {
            SnapshotView::new(Some(&self.snapshot), dummy_stamp(u64::from(tick)))
        }

        fn request(&self) -> WalkRequest {
            WalkRequest {
                target: tile(2610, 3200),
                loc_id: None,
                radius: 1,
                options: crate::FindOptions::default(),
                required_after: dummy_stamp(0),
                evidence: None,
                cross: Box::default(),
                protect: true,
            }
        }

        fn begin(&self) -> Result<WalkGuard, GuardRefusal> {
            WalkGuard::begin_with(&self.request(), &self.view(), Arc::clone(&self.tables))
        }

        fn face_us(&mut self) {
            self.npcs[0].target = Some(ActorTargetView {
                kind: ActorKind::Player,
                index: 1,
            });
            self.npcs[0].in_combat = true;
            self.refresh();
        }

        fn launch_arrow(&mut self) {
            self.face_us();
            self.projectiles = vec![ProjectileView {
                spotanim: 9,
                level: 0,
                src: self.npcs[0].tile,
                target: Some(ActorTargetView {
                    kind: ActorKind::Player,
                    index: 1,
                }),
                t1: 0,
                t2: 30,
            }];
            self.refresh();
        }

        fn missiles(&self) -> &PrayerFact {
            self.tables.prayer(PrayerRole::Protect, 1).unwrap()
        }

        fn add_prayer_potion(&mut self) {
            let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
            let item = data.item_by_alias("4doseprayerrestore").unwrap();
            self.inventory.push(ItemView {
                def: ItemDefView {
                    id: item.id,
                    name: item.name.clone(),
                    stackable: false,
                    members: false,
                    base_value: 1,
                    noted: false,
                    certificate_link: -1,
                    certificate_template: -1,
                },
                container: ItemContainer::Inventory,
                action_family: ItemActionFamily::Held,
                slot: 0,
                count: 1,
                actions: vec![Some("Drink".into())],
                component_id: 3214,
            });
            self.refresh();
        }
    }

    #[test]
    fn driver_fits_the_per_route_budget() {
        assert!(std::mem::size_of::<WalkGuard>() <= 256);
    }

    #[test]
    fn begin_refuses_when_no_protect_is_reachable() {
        let scene = Scene::new(36);
        assert_eq!(scene.begin().err(), Some(GuardRefusal::PrayerTooLow));
    }

    #[test]
    fn begin_admits_the_lowest_protect() {
        let scene = Scene::new(37);
        assert!(scene.begin().is_ok());
    }

    #[test]
    fn missiles_above_prayer_base_are_unprotectable_and_send_no_click() {
        let mut scene = Scene::new(37);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        match guard.tick(&scene.view()) {
            Some(GuardOp::Unprotectable {
                protect: GuardProtect::Missiles,
            }) => {}
            other => panic!("wanted unprotectable missiles, got {other:?}"),
        }
        assert!(
            !matches!(guard.tick(&scene.view()), Some(GuardOp::IfButton { .. })),
            "unprotectable must not emit a prayer packet"
        );
    }

    #[test]
    fn missiles_protect_clicks_within_two_ticks_of_the_launch() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view());
        let missiles = scene.missiles();
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: missiles.button_com
            })
        );
        assert!(!guard.blocks_follow(0));
        let end = guard.end();
        assert_eq!(
            end,
            GuardOp::IfButton {
                component: missiles.button_com
            }
        );
    }

    #[test]
    fn a_prayer_dose_locks_the_follow_for_two_ticks() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.add_prayer_potion();
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let drink = guard.tick(&scene.view_at(10));
        assert!(
            matches!(drink, Some(GuardOp::Drink { .. })),
            "got {drink:?}"
        );
        assert!(guard.blocks_follow(11));
        assert_eq!(
            guard.tick(&scene.view_at(11)),
            Some(GuardOp::Locked { until: 13 })
        );
        assert!(guard.blocks_follow(12));
        assert!(!guard.blocks_follow(13));
    }
}
