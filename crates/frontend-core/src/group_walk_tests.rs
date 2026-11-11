//! Group walk through the real host seam: a real profile-bound
//! [`host_play::Play`], real slot arms and the real map model. Only the
//! game client is absent (statuses stand in for observed players).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use host_play as map_host;
use host_play::walk_map::{ActionError, MapContext, MapModel};
use host_play::{InstancePermit, SlotArm, SlotStatus, WalkArms};
use nav_planner::bank_fetch::BankRows;
use nav_planner::collision::WorldCollision;
use nav_planner::router::{AvoidRect, FindOptions};
use nav_planner::tile::Tile;
use nav_planner::transport::{TransportGraph, WildernessRules};
use nav_planner::world::NavWorld;
use nav_planner::zones::{Zone, ZoneKind, ZoneTable};
use nav_planner::WorldState;
use vault::{Profile, ProfileSettings, Vault};

use super::{walk_marked, MarkedWalk, WalkInputs};
use crate::bulk::{BulkOutcome, BulkReport};
use crate::selection::{MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;

#[path = "../../host-play/tests/support/map_fixture.rs"]
mod map_fixture;
use map_fixture::MapFixture;

fn t(x: i32, z: i32, level: i32) -> Tile {
    Tile { x, z, level }
}

/// A `size` square, four planes, open ground except the `blocked` tiles.
fn world(size: usize, blocked: &[(usize, usize)]) -> NavWorld {
    let mut flags = vec![0u32; 4 * size * size];
    for &(x, z) in blocked {
        flags[z * size + x] = client::dash3d::CollisionFlag::SQ_BLOCKED as u32;
    }
    let (walk, blocked) = nav_planner::collision::pack_walk(&flags);
    NavWorld::from_parts(
        WorldCollision {
            origin: api::snapshot::WorldTile {
                x: 0,
                z: 0,
                level: 0,
            },
            width: size,
            height: size,
            walk,
            blocked,
            flags: None,
        },
        TransportGraph::default(),
        Vec::new(),
    )
}

/// A slot the host sees: an arm plus an observed status row.
struct Slot {
    name: &'static str,
    at: Tile,
    connected: bool,
    ingame: bool,
}

const fn slot(name: &'static str, at: Tile) -> Slot {
    Slot {
        name,
        at,
        connected: true,
        ingame: true,
    }
}

struct Fixture {
    core: OperatorSession<()>,
    _map: MapFixture,
    dest_context: MapContext,
    model: MapModel,
    arms: WalkArms,
}

/// A vault of `names` (uid 1..), a real profile-bound play whose `alice`
/// is observed at `slots[0]`, and the other `slots` attached beside her.
fn fixture(names: &[&'static str], slots: &[Slot]) -> Fixture {
    // Dave (5, 5) is walled in on plane 0.
    let ring: Vec<(usize, usize)> = (4..=6)
        .flat_map(|x| (4..=6).map(move |z| (x, z)))
        .filter(|&tile| tile != (5, 5))
        .collect();
    fixture_with_world(names, slots, world(12, &ring))
}

fn fixture_with_world(names: &[&'static str], slots: &[Slot], world: NavWorld) -> Fixture {
    fixture_with_world_and_snapshots(names, slots, world, true)
}

fn fixture_without_observed_snapshot(
    names: &[&'static str],
    slots: &[Slot],
    world: NavWorld,
) -> Fixture {
    fixture_with_world_and_snapshots(names, slots, world, false)
}

fn fixture_with_world_and_snapshots(
    names: &[&'static str],
    slots: &[Slot],
    world: NavWorld,
    publish_snapshots: bool,
) -> Fixture {
    let map = MapFixture::new(&world, "local-289");
    let mut play = map.play(slots[0].at);
    for other in &slots[1..] {
        play.attach_arm(other.name, SlotArm::new(1, false));
        play.statuses.lock().unwrap().push(SlotStatus {
            username: other.name.into(),
            connected: other.connected,
            ingame: other.ingame,
            tile_x: other.at.x,
            tile_z: other.at.z,
            tile_level: other.at.level,
            ..Default::default()
        });
    }
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "274bot-group-walk-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut vault = Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    for (i, name) in names.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    let dest_context = MapContext {
        focus: play.map_focus("alice"),
        nav: play.server_profile().unwrap().nav_pack_id().unwrap(),
        overlay: None,
        generation: 1,
    };
    let mut core = OperatorSession::new(InstancePermit::SkipLock);
    core.set_spawn_workers(false);
    core.start(vault, play);
    if publish_snapshots {
        for observed in slots {
            core.frames().publish_test_snapshot(
                observed.name,
                api::snapshot::GameSnapshot::new(),
                WorldState::empty(),
            );
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    let mut model = MapModel::default();
    model.bind(dest_context);
    Fixture {
        core,
        _map: map,
        dest_context,
        model,
        arms: Arc::new(Mutex::new(HashMap::new())),
    }
}

impl Fixture {
    /// Select `dest` on the map, consume it as a front end's confirm does,
    /// and group walk the `selection` there.
    fn walk_to(&mut self, selection: &MarkedSelection, dest: Tile) -> BulkReport {
        let world = self.core.play().unwrap().world().unwrap();
        assert_eq!(self.model.select_tile(&world, dest), Some(dest));
        // These pre-admission fixtures exercise geometry, not incomplete risk
        // snapshots. Only their zone-less walks opt in explicitly.
        let options = FindOptions {
            zones: if world
                .graph
                .zones
                .as_ref()
                .is_none_or(|zones| zones.zones().is_empty())
            {
                nav_planner::zones::ZoneExempt::all()
            } else {
                nav_planner::zones::ZoneExempt::default()
            },
            ..FindOptions::default()
        };
        let walk = MarkedWalk {
            prepared: self
                .model
                .confirm_walk_plan(&self.dest_context, options)
                .map(|plan| (self.dest_context, plan)),
            destination: Some(dest),
            options,
            arms: &self.arms,
        };
        walk_marked(selection, &self.core, walk, empty_inputs)
    }

    fn queued(&self, name: &str) -> Option<Tile> {
        self.arms
            .lock()
            .unwrap()
            .get(name)
            .and_then(|arm| arm.lock().unwrap().queued_tile())
    }
}

fn marks(uids: &[i32]) -> MarkedSelection {
    let mut selection = MarkedSelection::default();
    selection.mark_all(uids.iter().copied().map(ProfileIdentity::uid));
    selection
}

fn empty_inputs(_: &str) -> WalkInputs {
    WalkInputs {
        state: WorldState::empty(),
        bank: BankRows::default(),
        risk_input: Default::default(),
    }
}

/// Four marked bots each walk to the one tile from their own start; the
/// unmarked fifth bot is never sent anywhere.
#[test]
fn every_marked_bot_walks_to_the_one_tile_and_unmarked_bots_stay() {
    let mut f = fixture(
        &["alice", "bob", "carol", "dave", "erin"],
        &[
            slot("alice", t(1, 1, 0)),
            slot("bob", t(4, 2, 0)),
            slot("carol", t(2, 8, 0)),
            slot("dave", t(9, 9, 0)),
            slot("erin", t(6, 6, 0)),
        ],
    );
    let dest = t(10, 3, 0);
    let report = f.walk_to(&marks(&[1, 2, 3, 4]), dest);

    assert_eq!(
        report.done().collect::<Vec<_>>(),
        ["alice", "bob", "carol", "dave"]
    );
    for name in ["alice", "bob", "carol", "dave"] {
        assert_eq!(f.queued(name), Some(dest), "{name}");
    }
    assert_eq!(f.queued("erin"), None, "an unmarked bot was routed");
}

#[test]
fn group_confirmation_without_observed_snapshot_keeps_ordinary_walk_available() {
    let mut fixture =
        fixture_without_observed_snapshot(&["alice"], &[slot("alice", t(1, 1, 0))], world(12, &[]));
    assert!(
        fixture
            .core
            .frames()
            .with_snapshot("alice", |_| ())
            .is_none(),
        "the live slot has not published a frame yet"
    );
    let destination = t(10, 3, 0);
    let report = fixture.walk_to(&marks(&[1]), destination);
    assert_eq!(report.failed_count(), 0);
    assert_eq!(report.done_count(), 1);
    assert_eq!(report.rows()[0].outcome(), &BulkOutcome::Done);
    assert_eq!(fixture.queued("alice"), Some(destination));
}

/// A zone refusal's report rows carry the host's short summary, never the
/// zone list; the list is in each bot's log ring, once (written by
/// `WalkRequest::run`). The shown line is the counts and the first bot.
#[test]
fn group_walk_report_shows_counts_and_the_first_bot_name() {
    let _ring = ring_guard();
    let mut world = world(12, &[]);
    world.graph.zones = Some(barrier_zone_table());
    // The fixture's first slot is `alice`. The first bot by name is another one,
    // and its name is longer than the 16 characters a line gives it.
    let first = "aardvark-with-a-very-long-name";
    let names: [&'static str; 6] = ["alice", first, "bravo", "charlie", "delta", "echo"];
    let slots: Vec<Slot> = names
        .iter()
        .enumerate()
        .map(|(i, name)| slot(name, t(1, 1 + i as i32, 0)))
        .collect();
    let mut f = fixture_with_world(&names, &slots, world);

    let base: Vec<usize> = names.iter().map(|name| slot_lines(name).len()).collect();
    let report = f.walk_to(&marks(&[1, 2, 3, 4, 5, 6]), t(10, 10, 0));
    assert_eq!(report.failed_count(), 6);
    for row in report.rows() {
        assert!(row.outcome().is_failed(), "{row:?}");
        let detail = row.detail().expect("a refused row keeps the summary");
        assert!(
            detail.chars().count() <= crate::SUMMARY_MAX && !detail.contains("Sentinel strip"),
            "{detail}"
        );
    }
    let line = report.brief().publish(&crate::LogTarget::walk(None));
    assert!(line.chars().count() <= crate::LINE_MAX, "{line}");
    assert!(line.contains("failed 6"), "{line}");
    assert!(
        line.contains("aardvark-with-a…"),
        "the first sorted name, cut to 16: {line}"
    );
    assert!(
        !line.contains("alice") && !line.contains("bravo"),
        "only the first bot is named: {line}"
    );
    assert!(
        !line.contains("Sentinel strip") && !line.contains("test-barrier@"),
        "{line}"
    );
    assert!(line.ends_with(crate::LOG_POINTER), "{line}");

    // The zone is named in each bot's ring exactly once.
    for (name, base) in names.into_iter().zip(base) {
        let ring: Vec<String> = slot_lines(name).into_iter().skip(base).collect();
        assert_eq!(
            ring.iter().filter(|l| l.contains("Sentinel strip")).count(),
            1,
            "{name}: {ring:#?}"
        );
    }
    // The summary keeps its older, fuller shape for callers that want it.
    assert!(report.summary().contains("bravo"));
}

/// The fixture's first bot is always `alice`, and the log store is shared by
/// every test in this binary: the tests that count `alice`'s ring lines take
/// this lock.
static RING_TESTS: Mutex<()> = Mutex::new(());

fn ring_guard() -> std::sync::MutexGuard<'static, ()> {
    RING_TESTS
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Every message in `slot`'s ring, oldest first.
fn slot_lines(slot: &str) -> Vec<String> {
    use crate::log::{global, LogScope, LogView};
    let mut view = LogView::new(LogScope::slot(slot));
    global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|row| row.message().to_owned())
        .collect()
}

/// 6 bots each refused for 60 zones in one confirm: each bot's ring holds one
/// packed group with every zone once, and no ring or category holds a second
/// copy (the host writes the detail; the report and the command book carry
/// the short line).
#[test]
fn a_group_walk_logs_each_bots_detail_once() {
    let _ring = ring_guard();
    // The router names at most 63 blocking zones; 60 is well inside that.
    let zones = 60;
    let mut world = world(12, &[]);
    world.graph.zones = Some(many_zone_table(zones));
    let names: [&'static str; 6] = [
        "alice",
        "gw-once-b",
        "gw-once-c",
        "gw-once-d",
        "gw-once-e",
        "gw-once-f",
    ];
    let slots: Vec<Slot> = names
        .iter()
        .enumerate()
        .map(|(i, name)| slot(name, t(0, 1 + i as i32, 0)))
        .collect();
    let mut f = fixture_with_world(&names, &slots, world);
    let base: Vec<usize> = names.iter().map(|name| slot_lines(name).len()).collect();
    let report = f.walk_to(&marks(&[1, 2, 3, 4, 5, 6]), t(11, 10, 0));
    assert_eq!(report.failed_count(), 6, "{}", report.summary());
    for (name, base) in names.into_iter().zip(base) {
        // Only what this confirm wrote: `alice` is shared with other tests.
        let ring: Vec<String> = slot_lines(name).into_iter().skip(base).collect();
        let packed: Vec<&String> = ring
            .iter()
            .filter(|line| line.contains("refused detail (RiskRefused)"))
            .collect();
        assert!(
            packed.len() > 1 && packed.len() <= 16,
            "{name}: {} detail lines: {ring:#?}",
            packed.len()
        );
        assert_eq!(
            packed.iter().filter(|line| line.contains("[1/")).count(),
            1,
            "{name}: one packed group: {ring:#?}"
        );
        for zone in 0..zones {
            let key = format!("strip{zone:02}");
            assert_eq!(
                ring.iter().filter(|line| line.contains(&key)).count(),
                1,
                "{name}: {key} appears once in the whole ring"
            );
        }
        for line in &ring {
            assert!(
                line.len() <= api::hostlog::LINE_BYTES,
                "{} bytes",
                line.len()
            );
        }
    }
    // No refusal line anywhere else holds a zone: the process ring (where a
    // command refusal would log) and the report carry the short text only.
    let mut process = crate::log::LogView::new(crate::log::LogScope::Process);
    crate::log::global().refresh(&mut process);
    assert!(
        process
            .rows()
            .iter()
            .all(|row| !row.message().contains("strip00")),
        "the process ring holds no zone"
    );
    assert!(!report.summary().contains("strip00"));
}

/// A whole-command refusal (here a stale confirmation naming 99 zones) logs
/// one packed group per bot too, and the report carries the summary.
#[test]
fn a_refused_group_walk_logs_one_group_per_bot() {
    let _ring = ring_guard();
    let names: [&'static str; 3] = ["alice", "gw-refuse-b", "gw-refuse-c"];
    let slots: Vec<Slot> = names
        .iter()
        .enumerate()
        .map(|(i, name)| slot(name, t(1, 1 + i as i32, 0)))
        .collect();
    let f = fixture_with_world(&names, &slots, world(12, &[]));
    let items: Vec<String> = (0..99)
        .map(|i| format!("Refuse zone {i:02} [refusezone{i:02}@1,{i},0]"))
        .collect();
    let error = ActionError::BlockedByZones {
        zones: Some(host_play::walk_map::BlockedZones::new(items)),
    };
    let walk = MarkedWalk {
        prepared: Err(error),
        destination: Some(t(10, 10, 0)),
        options: FindOptions::default(),
        arms: &f.arms,
    };
    let base: Vec<usize> = names.iter().map(|name| slot_lines(name).len()).collect();
    let report = walk_marked(&marks(&[1, 2, 3]), &f.core, walk, empty_inputs);
    assert_eq!(report.failed_count(), 3);
    let line = report.brief().publish(&crate::LogTarget::walk(None));
    assert!(line.chars().count() <= crate::LINE_MAX, "{line}");
    assert!(line.contains("Blocked by 99 danger zones"), "{line}");
    assert!(!line.contains("refusezone"), "{line}");
    for (name, base) in names.into_iter().zip(base) {
        let ring: Vec<String> = slot_lines(name).into_iter().skip(base).collect();
        assert_eq!(
            ring.iter()
                .filter(|l| l.contains("refused detail (BlockedByZones)") && l.contains("[1/"))
                .count(),
            1,
            "{name}: one group"
        );
        for i in 0..99 {
            let key = format!("refusezone{i:02}@");
            assert_eq!(
                ring.iter().filter(|l| l.contains(&key)).count(),
                1,
                "{name}: {key}"
            );
        }
    }
}

/// `zones` hazards over the same column of a 12 x 12 world, each its own kind
/// (`strip00` ...), so a walk across it is refused for all of them.
fn many_zone_table(zones: usize) -> ZoneTable {
    let rect = AvoidRect {
        min_x: 6,
        max_x: 6,
        min_z: 0,
        max_z: 11,
        level: Some(0),
    };
    ZoneTable::from_parts(
        (0..zones)
            .map(|kind| Zone::hazard(rect, 0, kind as u16))
            .collect(),
        (0..zones)
            .map(|kind| {
                ZoneKind::new(
                    format!("strip{kind:02}"),
                    format!("Strip {kind:02}"),
                    -1,
                    0,
                    false,
                    false,
                )
            })
            .collect(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

fn barrier_zone_table() -> ZoneTable {
    ZoneTable::from_parts(
        vec![Zone::hazard(
            AvoidRect {
                min_x: 6,
                max_x: 6,
                min_z: 0,
                max_z: 11,
                level: Some(0),
            },
            0,
            0,
        )],
        vec![ZoneKind::new(
            "test-barrier",
            "Sentinel strip",
            -1,
            0,
            false,
            false,
        )],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        WorldTile {
            x: 0,
            z: 0,
            level: 0,
        },
        12,
        12,
        &WildernessRules::default(),
    )
    .unwrap()
}

/// Ineligible and refused rows are named with their reasons, failures
/// first, and a marked profile that is gone is still counted once.
#[test]
fn mixed_outcomes_report_failures_first_and_count_each_marked_bot_once() {
    let mut f = fixture(
        &["alice", "bob", "carol", "dave", "erin"],
        &[
            slot("alice", t(1, 1, 0)),
            slot("bob", t(3, 3, 0)),
            // Logged out: no arm was ever attached for this one, but a
            // status row exists.
            Slot {
                name: "carol",
                at: t(2, 2, 0),
                connected: false,
                ingame: false,
            },
            // Walled in: eligible, but the router finds no path out.
            slot("dave", t(5, 5, 0)),
            // No observed position yet.
            slot("erin", t(0, 0, 0)),
        ],
    );
    // Uid 99 is not in the vault; marking it twice is one mark.
    let mut selection = marks(&[1, 2, 3, 4, 5, 99]);
    selection.set(ProfileIdentity::uid(99), true);
    let report = f.walk_to(&selection, t(10, 10, 0));

    let rows: Vec<(&str, &BulkOutcome)> = report
        .rows()
        .iter()
        .map(|row| (row.profile(), row.outcome()))
        .collect();
    assert_eq!(rows.len(), 6, "each marked row appears once: {rows:?}");
    assert_eq!(
        rows[0],
        ("dave", &BulkOutcome::failed("no path")),
        "failures sort first"
    );
    assert_eq!(
        (
            report.done_count(),
            report.skipped_count(),
            report.failed_count()
        ),
        (2, 3, 1)
    );
    assert_eq!(f.queued("alice"), Some(t(10, 10, 0)));
    assert_eq!(f.queued("bob"), Some(t(10, 10, 0)));
    for name in ["carol", "dave", "erin"] {
        assert_eq!(f.queued(name), None, "{name} must not be walking");
    }
}

/// A destination the map refuses fails every marked bot once, with the one
/// cause, and sends nobody anywhere.
#[test]
fn a_refused_destination_fails_each_marked_bot_with_one_cause() {
    let mut f = fixture(
        &["alice", "bob"],
        &[slot("alice", t(1, 1, 0)), slot("bob", t(2, 2, 0))],
    );
    // Nothing was selected on the map, so there is no plan to consume.
    let walk = MarkedWalk {
        prepared: f
            .model
            .confirm_walk_plan(&f.dest_context, FindOptions::default())
            .map(|plan| (f.dest_context, plan)),
        destination: None,
        options: FindOptions::default(),
        arms: &f.arms,
    };
    assert_eq!(
        walk.prepared.as_ref().err(),
        Some(&ActionError::NoSelection)
    );
    let report = walk_marked(&marks(&[1, 2]), &f.core, walk, empty_inputs);
    assert_eq!(report.failed_count(), 2);
    assert_eq!(report.done_count(), 0);
    assert!(f.arms.lock().unwrap().is_empty());
}

#[test]
fn frontend_owned_map_staging_arms_the_real_profile_bound_play() {
    let f = fixture_with_world(
        &["alice", "bob"],
        &[slot("alice", t(1, 1, 0)), slot("bob", t(2, 10, 0))],
        world(12, &[]),
    );
    let world = f.core.play().unwrap().world().unwrap();
    let context = f.core.walk_map_context(
        None,
        f.dest_context.nav,
        f.dest_context.overlay,
        f.dest_context.generation,
    );
    let mut map = crate::walk_map::WalkMap::default();
    map.bind(context);
    let destination = t(10, 10, 0);
    assert_eq!(map.select_tile(&world, destination), Some(destination));
    let staged = super::StagedMarkedWalk::new(
        &mut map,
        context,
        FindOptions {
            // This controlled fixture proves geometry, not missing risk facts.
            zones: nav_planner::zones::ZoneExempt::all(),
            ..FindOptions::default()
        },
    );
    let report = f.core.confirm_marked_walk(&marks(&[1, 2]), staged);
    let mut done = report.done();
    assert_eq!(done.next(), Some("alice"));
    assert_eq!(done.next(), Some("bob"));
    assert_eq!(done.next(), None);
    let arms = f.core.walk_arms();
    let armed = arms.lock().unwrap();
    for name in ["alice", "bob"] {
        assert_eq!(
            armed.get(name).unwrap().lock().unwrap().queued_tile(),
            Some(destination)
        );
    }
}
