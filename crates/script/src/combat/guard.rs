//! Host-side protected-walk driver. Hold-mode prayer only: no eat, no attack,
//! no flick. The host follow sites own one driver per armed route.
use super::arbiter;
use super::frame::Frame;
use super::policy;
use super::schedule::{reached, InputEffect, OpKind, Schedule};
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
    schedule: Schedule,
    last_tick: u16,
    follow_until: u16,
    drop_component: i32,
    pending_com: i32,
    unprotectable: u8,
    flags: u8,
    drink_points: u8,
    drink_doses: u8,
}

const FLAG_FOLLOW: u8 = 1;
const FLAG_SEEN: u8 = 2;
const FLAG_ON: u8 = 4;

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
        request: &WalkRequest,
        snapshot: &SnapshotView<'_>,
        tables: Arc<CombatTables>,
    ) -> Result<Self, GuardRefusal> {
        if !request.allow.prayer {
            return Err(GuardRefusal::PrayerDisallowed);
        }
        let base = prayer_base(snapshot).ok_or(GuardRefusal::Snapshot)?;
        if base < LOWEST_PROTECT {
            return Err(GuardRefusal::PrayerTooLow);
        }
        Ok(Self {
            threats: ThreatSet::default(),
            tables,
            schedule: Schedule::default(),
            last_tick: 0,
            follow_until: 0,
            drop_component: 0,
            pending_com: 0,
            unprotectable: 0,
            flags: 0,
            drink_points: 0,
            drink_doses: 0,
        })
    }

    /// Previous drink lock or protect click still covers `tick`: the route
    /// owner issues no follow click.
    pub fn blocks_follow(&self, tick: u16) -> bool {
        (self.flags & FLAG_FOLLOW != 0 && !reached(tick, self.follow_until))
            || self.schedule.locked(tick)
    }

    /// Refresh observed-on state from the arrival snapshot before [`end`].
    pub fn observe_prayer(&mut self, snapshot: &SnapshotView<'_>) {
        let Some(frame) = Frame::borrow(*snapshot) else {
            return;
        };
        if let Some(fact) = select::active_protect(&frame, &self.tables)
            .and_then(|style| select::protect_fact(&self.tables, style))
        {
            self.drop_component = fact.button_com;
            self.flags |= FLAG_ON;
            self.schedule.settle(OpKind::Prayer);
        } else {
            self.flags &= !FLAG_ON;
        }
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
                until: if self.schedule.locked(tick) {
                    self.schedule.input_lock
                } else {
                    self.follow_until
                },
            });
        }
        self.threats.observe(&frame, &self.tables, tick);
        let (points, base) = arbiter::stat(&frame, PRAYER_STAT);
        let (hp, hp_max) = arbiter::stat(&frame, HITPOINTS_STAT);
        if let Some(fact) = select::active_protect(&frame, &self.tables)
            .and_then(|style| select::protect_fact(&self.tables, style))
        {
            self.drop_component = fact.button_com;
            self.flags |= FLAG_ON;
            self.schedule.settle(OpKind::Prayer);
        } else {
            self.flags &= !FLAG_ON;
        }
        self.settle_drink(&frame, tick, points);
        let wanted =
            policy::wanted_protect(&self.threats, &frame, &self.tables, tick, false, false)?;
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
            self.flags |= FLAG_ON;
            self.schedule.settle(OpKind::Prayer);
            return self.maybe_floor_sip(&frame, tick, points, base, hp, hp_max);
        }
        let same_toggle = self.pending_com == wanted.button_com;
        if self.schedule.pending(OpKind::Prayer) && same_toggle {
            if !self.schedule.ready(OpKind::Prayer, tick) {
                return None;
            }
            self.schedule.timeout(OpKind::Prayer);
        }
        if !self.schedule.ready(OpKind::Prayer, tick) && same_toggle {
            return None;
        }
        if points == 0 {
            return self.drink(&frame, tick, hp, hp_max, true);
        }
        // A walk hop on this tick would drop the prayer packet; lock follow
        // and do not click the same toggle again while it is unacknowledged.
        self.schedule
            .admitted(OpKind::Prayer, tick, 0, false, InputEffect::PrayerOn);
        self.pending_com = wanted.button_com;
        self.follow_until = tick.wrapping_add(1);
        self.flags |= FLAG_FOLLOW;
        Some(GuardOp::IfButton {
            component: wanted.button_com,
        })
    }

    /// Off-click only when the last observation had the protect on.
    pub fn end(self) -> GuardOp {
        GuardOp::IfButton {
            component: if self.flags & FLAG_ON != 0 {
                self.drop_component
            } else {
                0
            },
        }
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
        if !policy::prayer_sip_due(points, base) {
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
        if self.schedule.pending(OpKind::Drink) || !self.schedule.ready(OpKind::Drink, tick) {
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
        let (points, _) = arbiter::stat(frame, PRAYER_STAT);
        self.drink_points = u8::try_from(points.clamp(0, i32::from(u8::MAX))).unwrap_or(u8::MAX);
        self.drink_doses = u8::try_from(
            arbiter::doses(frame, &self.tables, PotionKind::Prayer).clamp(0, i16::from(u8::MAX)),
        )
        .unwrap_or(u8::MAX);
        self.schedule
            .admitted(OpKind::Drink, tick, 0, false, InputEffect::Standard);
        Some(GuardOp::Drink { name })
    }

    fn settle_drink(&mut self, frame: &Frame<'_>, tick: u16, points: i32) {
        if !self.schedule.pending(OpKind::Drink) {
            return;
        }
        let doses = arbiter::doses(frame, &self.tables, PotionKind::Prayer);
        let observed = points > i32::from(self.drink_points) || doses < i16::from(self.drink_doses);
        if observed {
            self.schedule.settle(OpKind::Drink);
        } else if self.schedule.ready(OpKind::Drink, tick) {
            self.schedule.timeout(OpKind::Drink);
        }
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
                allow: Default::default(),
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

        fn launch_style(&mut self, spotanim: i32) {
            self.face_us();
            self.projectiles = vec![ProjectileView {
                spotanim,
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

        fn launch_arrow(&mut self) {
            self.launch_style(9);
        }

        fn launch_spell(&mut self) {
            self.launch_style(88);
        }

        fn magic(&self) -> &PrayerFact {
            self.tables.prayer(PrayerRole::Protect, 0).unwrap()
        }

        fn missiles(&self) -> &PrayerFact {
            self.tables.prayer(PrayerRole::Protect, 1).unwrap()
        }

        fn set_missiles(&mut self, on: bool) {
            let varp = self.missiles().varp;
            if let Some(row) = self.varps.iter_mut().find(|row| row.index == varp) {
                row.value = i32::from(on);
            }
            self.refresh();
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
        assert!(
            guard.blocks_follow(0),
            "the protect click must not share its tick with a walk hop"
        );
        assert!(!guard.blocks_follow(1));
        assert_eq!(
            guard.end(),
            GuardOp::IfButton { component: 0 },
            "a proposed click is not an observed-on off click"
        );
    }

    #[test]
    fn begin_refuses_when_the_owner_disallows_prayer() {
        let scene = Scene::new(43);
        let mut request = scene.request();
        request.allow.prayer = false;
        assert_eq!(
            WalkGuard::begin_with(&request, &scene.view(), Arc::clone(&scene.tables)).err(),
            Some(GuardRefusal::PrayerDisallowed)
        );
    }

    #[test]
    fn pending_protect_click_is_not_repeated_before_ack() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(0));
        assert!(matches!(first, Some(GuardOp::IfButton { .. })), "{first:?}");
        assert!(
            !matches!(
                guard.tick(&scene.view_at(1)),
                Some(GuardOp::IfButton { .. })
            ),
            "unacknowledged protect must not toggle again"
        );
        assert!(
            !matches!(
                guard.tick(&scene.view_at(2)),
                Some(GuardOp::IfButton { .. })
            ),
            "three-tick prayer pacing must hold after the first click"
        );
    }

    #[test]
    fn end_does_not_click_when_protect_was_never_on() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let _ = guard.tick(&scene.view());
        assert_eq!(guard.end(), GuardOp::IfButton { component: 0 });
    }

    #[test]
    fn end_does_not_click_when_protect_was_turned_off() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let _ = guard.tick(&scene.view_at(0));
        scene.set_missiles(true);
        assert!(guard.tick(&scene.view_at(3)).is_none());
        scene.set_missiles(false);
        guard.observe_prayer(&scene.view_at(4));
        assert_eq!(
            guard.end(),
            GuardOp::IfButton { component: 0 },
            "a remembered button must not toggle an already-off prayer on"
        );
    }

    #[test]
    fn end_clicks_only_the_protect_last_observed_on() {
        let mut scene = Scene::new(43);
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let _ = guard.tick(&scene.view_at(0));
        scene.set_missiles(true);
        assert!(guard.tick(&scene.view_at(3)).is_none());
        let missiles = scene.missiles().button_com;
        assert_eq!(
            guard.end(),
            GuardOp::IfButton {
                component: missiles
            }
        );
    }

    #[test]
    fn prayer_sip_uses_the_shared_c5_floor() {
        for (points, due) in [(27, false), (26, true), (9, true), (8, true)] {
            let mut scene = Scene::new(43);
            scene.stats[5].effective = points;
            scene.add_prayer_potion();
            scene.launch_arrow();
            scene.set_missiles(true);
            let mut guard = scene.begin().unwrap();
            let op = guard.tick(&scene.view_at(10));
            assert_eq!(
                matches!(&op, Some(GuardOp::Drink { .. })),
                due,
                "Prayer points {points}, got {op:?}"
            );
        }
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

    #[test]
    fn a_troll_rock_launch_selects_missiles() {
        let mut scene = Scene::new(43);
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let row = data.npc_by_config("death_troll_thrower1").unwrap();
        scene.npcs[0].r#type = Some(row.id as usize);
        scene.npcs[0].name = row.display.clone();
        scene.npcs[0].animation = 1142;
        scene.npcs[0].tile = scene.local.player.actor.tile;
        scene.npcs[0].network = scene.local.player.actor.tile;
        scene.launch_style(276);
        scene.projectiles[0].t1 = 364;
        scene.projectiles[0].t2 = 381;
        scene.refresh();
        scene.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 364,
        });
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view());
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: scene.missiles().button_com
            }),
            "a classified rock projectile must select Missiles, got {op:?}"
        );
    }

    #[test]
    fn a_ranged_projectile_onset_does_not_select_melee() {
        let mut scene = Scene::new(43);
        let data = api::game_data::for_revision(ClientRevision::R289).unwrap();
        let row = data.npc_by_config("death_troll_thrower1").unwrap();
        scene.npcs[0].r#type = Some(row.id as usize);
        scene.npcs[0].name = row.display.clone();
        scene.npcs[0].animation = 1142;
        scene.npcs[0].tile = scene.local.player.actor.tile;
        scene.npcs[0].network = scene.local.player.actor.tile;
        scene.launch_style(276);
        scene.npcs[0].in_combat = false;
        scene.projectiles[0].src = tile(scene.npcs[0].tile.x, scene.npcs[0].tile.z + 1);
        scene.projectiles[0].t1 = 364;
        scene.projectiles[0].t2 = 381;
        scene.refresh();
        scene.snapshot.seed_hitmarks(HitmarksView {
            marks: [HitmarkView {
                value: 0,
                kind: 0,
                cycle: 0,
            }; 4],
            loop_cycle: 360,
        });
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view());
        let melee = scene.tables.prayer(PrayerRole::Protect, 2).unwrap();
        assert_ne!(
            op,
            Some(GuardOp::IfButton {
                component: melee.button_com
            }),
            "a ranged projectile onset must not select Melee, got {op:?}"
        );
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: scene.missiles().button_com
            }),
            "a ranged projectile onset must select Missiles, got {op:?}"
        );
    }

    #[test]
    fn incoming_projectile_does_not_override_selected_protect() {
        let mut scene = Scene::new(43);
        scene.launch_spell();
        let mut guard = scene.begin().unwrap();
        let op = guard.tick(&scene.view());
        let magic = scene.magic();
        let missiles = scene.missiles();
        assert_eq!(
            op,
            Some(GuardOp::IfButton {
                component: magic.button_com
            }),
            "a magic projectile must keep the shared selector's Protect from Magic, got {op:?}"
        );
        assert_ne!(
            op,
            Some(GuardOp::IfButton {
                component: missiles.button_com
            }),
            "an incoming projectile must not override the selected style with Missiles"
        );
    }

    #[test]
    fn first_dose_does_not_leave_drink_pending() {
        let mut scene = Scene::new(43);
        scene.stats[5].effective = 0;
        scene.add_prayer_potion();
        scene.launch_arrow();
        let mut guard = scene.begin().unwrap();
        let first = guard.tick(&scene.view_at(10));
        assert!(
            matches!(first, Some(GuardOp::Drink { .. })),
            "got {first:?}"
        );
        assert_eq!(
            guard.tick(&scene.view_at(11)),
            Some(GuardOp::Locked { until: 13 })
        );
        let _ = guard.tick(&scene.view_at(12));
        let second = guard.tick(&scene.view_at(13));
        assert!(
            matches!(second, Some(GuardOp::Drink { .. })),
            "a later dose on the same route must still be admitted after the first is refused, got {second:?}"
        );
    }
}
