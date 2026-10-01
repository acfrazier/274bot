//! Group walk through the real host seam: a real profile-bound
//! [`host_play::Play`], real slot arms and the real map model. Only the
//! game client is absent (statuses stand in for observed players).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use api::snapshot::WorldTile;
use host_play as map_host;
use host_play::walk_map::{ActionError, MapContext, MapModel};
use host_play::{InstancePermit, SlotArm, SlotStatus, WalkArms};
use nav::collision::WorldCollision;
use nav::map::identity::Digest;
use nav::router::{AvoidRect, FindOptions};
use nav::tile::Tile;
use nav::transport::{TransportGraph, WildernessRules};
use nav::world::NavWorld;
use nav::zones::{Zone, ZoneKind, ZoneTable};
use nav::WorldState;
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
    let (walk, blocked) = nav::collision::pack_walk(&flags);
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
        nav: Digest::from_hex(
            &play
                .server_profile()
                .unwrap()
                .nav_identity()
                .unwrap()
                .nav_sha256,
        )
        .unwrap(),
        overlay: None,
        generation: 1,
    };
    let mut core = OperatorSession::new(InstancePermit::SkipLock);
    core.set_spawn_workers(false);
    core.start(vault, play);
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
        let options = FindOptions::default();
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
        bank: Vec::new(),
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
    assert_eq!(report.summary(), "Walk marked: walking 4, skipped 0");
    for name in ["alice", "bob", "carol", "dave"] {
        assert_eq!(f.queued(name), Some(dest), "{name}");
    }
    assert_eq!(f.queued("erin"), None, "an unmarked bot was routed");
}

#[test]
fn group_walk_reports_blocking_zone_names_for_each_failed_marked_bot() {
    let mut world = world(12, &[]);
    world.graph.zones = Some(barrier_zone_table());
    let mut f = fixture_with_world(
        &["alice", "bob"],
        &[slot("alice", t(1, 1, 0)), slot("bob", t(2, 10, 0))],
        world,
    );

    let report = f.walk_to(&marks(&[1, 2]), t(10, 10, 0));
    assert_eq!(report.failed_count(), 2);
    let failures: Vec<_> = report
        .rows()
        .iter()
        .filter(|row| {
            matches!(
                &row.outcome,
                BulkOutcome::Failed(reason) if reason.as_str() == "blocked by danger zones"
            )
        })
        .collect();
    assert_eq!(failures.len(), 2);
    for row in failures {
        assert!(
            row.detail
                .as_deref()
                .is_some_and(|detail| detail.contains("Sentinel strip")),
            "{row:?}"
        );
    }
    let summary = report.summary();
    assert_eq!(summary.matches("Sentinel strip").count(), 2, "{summary}");
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
        vec![ZoneKind::new("test-barrier", "Sentinel strip", -1, 0, false, false)],
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
        .map(|row| (row.profile.as_str(), &row.outcome))
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
    let gone = format!("profile#{}", ProfileIdentity::uid(99).raw());
    assert_eq!(
        report.summary(),
        format!(
            "Walk marked: walking 2, skipped 3, failed 1: dave: no path; \
             skipped carol: not logged in, erin: no position yet, {gone}: profile unavailable"
        )
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
    assert_eq!(report.refusal(), Some("Select a map destination first"));
    assert_eq!(report.failed_count(), 2);
    assert_eq!(report.done_count(), 0);
    assert_eq!(
        report.summary(),
        "Walk marked: Select a map destination first (2 marked bots not sent)"
    );
    assert!(f.arms.lock().unwrap().is_empty());
}
