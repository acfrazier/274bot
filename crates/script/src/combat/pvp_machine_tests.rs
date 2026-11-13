//! PvP slice V1 (PVP-DESIGN-1026 §2 and §10.1 cases 1-6, 19, 20 and 22): the
//! opt-in to start a fight with a player, the candidate check, the zone gate,
//! the attack label, the typed refusals and the server-line table.
use super::*;
use crate::native::{PvpGlobals, WildernessLevels};
use api::snapshot::ChatLineView;
use select::PvpSource;

/// `areas/area_wilderness/configs/wilderness_zones.dbrow` (the main zone,
/// levels 0-3, and the Edgeville dungeon zone) with the level formula of
/// `wilderness_levels.rs2:4-16`: one level per 8 tiles from the zone's south edge.
fn content_wilderness() -> TestWilderness {
    TestWilderness {
        zones: vec![
            TestZone {
                x1: 2944,
                z1: 3520,
                x2: 3391,
                z2: 6399,
                level1: 0,
                level2: 3,
                origin_z: 3520,
            },
            TestZone {
                x1: 2944,
                z1: 9920,
                x2: 3391,
                z2: 12799,
                level1: 0,
                level2: 0,
                origin_z: 9920,
            },
        ],
    }
}

fn place_local(scene: &mut Scene, at: api::WorldTile) {
    scene.local.player.network = at;
    scene.local.player.actor.tile = at;
}

/// A scene with no npc, the content's Wilderness rules and the bot at `at`.
fn wilderness_scene(at: api::WorldTile) -> Scene {
    let mut scene = Scene::new("imp");
    scene.npcs.clear();
    scene.wilderness = Some(content_wilderness());
    place_local(&mut scene, at);
    scene.refresh();
    scene
}

/// The bot inside a duel pen (x 3333-3357, z 3244-3258).
pub(super) fn pen_scene() -> Scene {
    wilderness_scene(tile(3340, 3250))
}

pub(super) fn player(
    index: usize,
    name: &str,
    at: api::WorldTile,
    ops: &[Option<&str>],
    combat_level: i32,
) -> PlayerView {
    let mut actor = actor(at);
    actor.name = Some(name.into());
    actor.actions = ops.iter().map(|op| op.map(str::to_owned)).collect();
    PlayerView {
        index,
        network: at,
        actor,
        combat_level,
        skill_level: 0,
        headicons: 0,
        weapon: None,
    }
}

fn attack_ops() -> [Option<&'static str>; 2] {
    [None, Some("Attack")]
}

pub(super) fn fight_ops() -> [Option<&'static str>; 2] {
    [None, Some("Fight")]
}

fn player_request(name: &str, pvp: bool) -> CombatRequest {
    CombatRequest {
        target: Target::Player {
            name: Arc::from(name),
        },
        pvp,
        fallback: Fallback::Fight,
        lost_radius: 32,
        ..CombatRequest::default()
    }
}

fn self_defence_request() -> CombatRequest {
    CombatRequest {
        target: Target::Attacker {
            npcs: false,
            players: true,
        },
        fallback: Fallback::Fight,
        ..CombatRequest::default()
    }
}

/// An aggressor that faces us and swings: a player threat.
fn aggressor(scene: &Scene, index: usize, name: &str, at: api::WorldTile) -> PlayerView {
    let mut row = player(index, name, at, &attack_ops(), 60);
    row.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: scene.local.player.index,
    });
    row.actor.animation = scene.melee_seq();
    row.actor.in_combat = true;
    row
}

fn refused(result: Result<Harness, ActionError>) -> Option<PvpRefusal> {
    match result {
        Err(ActionError::PvpRefused(reason)) => Some(reason),
        Err(other) => panic!("expected a PvP refusal, got {other:?}"),
        Ok(_) => None,
    }
}

pub(super) fn player_attack(effect: Option<HostEffect>, who: &str, label: &str) {
    assert!(
        matches!(&effect, Some(HostEffect::Interaction(InteractReq::Player { name, action })) if name == who && action == label),
        "expected {label} on {who}, got a different effect (present: {})",
        effect.is_some()
    );
}

fn has_player_attack(batch: &DrainedBatch, label: &str) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::Player { action, .. })) if action == label
        )
    })
}

fn say(scene: &mut Scene, sequence: i32, text: &str) {
    scene.snapshot.seed_chat_lines(vec![ChatLineView {
        type_: 0,
        username: None,
        text: text.into(),
        sequence,
    }]);
}

/// Begin the machine against `bob` in a pen and run the first two polls so the
/// fight is installed.
fn installed_pen_fight(pvp: bool) -> (Scene, Harness) {
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, pvp));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    (scene, harness)
}

pub(super) const BOB: &str = "bob";

// --- Case 1: the opt-in ------------------------------------------------------

#[test]
fn begin_global_only_authorises_initiation() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    scene.globals = PvpGlobals { start_fights: true };
    let mut harness = Harness::try_new(&scene, player_request(BOB, false), 0)
        .unwrap_or_else(|error| panic!("the global setting alone must start the fight: {error:?}"));
    assert_eq!(harness.machine.pvp_source, Some(PvpSource::Global));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
}

#[test]
fn begin_names_the_source_that_authorised_the_fight() {
    if !api::test_support::game_data_available() {
        return;
    }
    for (request_pvp, global, expected) in [
        (true, false, PvpSource::Request),
        (false, true, PvpSource::Global),
        (true, true, PvpSource::Both),
    ] {
        let mut scene = pen_scene();
        scene
            .players
            .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
        scene.refresh();
        scene.globals = PvpGlobals {
            start_fights: global,
        };
        let harness = Harness::try_new(&scene, player_request(BOB, request_pvp), 0)
            .unwrap_or_else(|error| panic!("{request_pvp}/{global}: {error:?}"));
        assert_eq!(harness.machine.pvp_source, Some(expected));
    }
}

#[test]
fn begin_refuses_a_fight_with_a_player_without_either_source() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    // Neither the request field nor the global setting, and a cold store (the
    // fail-closed default) is the same as the setting off.
    for globals in [
        PvpGlobals::fail_closed(),
        PvpGlobals {
            start_fights: false,
        },
    ] {
        scene.globals = globals;
        assert_eq!(
            refused(Harness::try_new(&scene, player_request(BOB, false), 0)),
            Some(PvpRefusal::NotOptedIn)
        );
    }
}

/// §2.7: a refused start logs one `info` line that names both sources. The
/// process sink is shared by every test in this crate and other tests refuse
/// too, so this pins the line's level and text, not a global count.
#[test]
fn begin_logs_an_info_line_naming_both_sources_when_it_refuses() {
    if !api::test_support::game_data_available() {
        return;
    }
    crate::slot::compiled::tests::install_status_line_sink();
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    scene.globals = PvpGlobals {
        start_fights: false,
    };
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, false), 0)),
        Some(PvpRefusal::NotOptedIn)
    );
    let lines = crate::slot::compiled::tests::pvp_refusal_lines();
    let ours: Vec<_> = lines
        .iter()
        .filter(|(_, line)| line.contains("request.pvp = false, pvp.start_fights = false"))
        .collect();
    assert!(!ours.is_empty(), "no refusal line in {lines:?}");
    assert!(
        ours.iter()
            .all(|(level, _)| *level == api::hostlog::Level::Info),
        "{ours:?}"
    );
}

#[test]
fn a_cold_store_fails_closed_for_starting_but_never_for_self_defence() {
    if !api::test_support::game_data_available() {
        return;
    }
    // The request field still opts in on its own.
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    scene.globals = PvpGlobals::fail_closed();
    assert!(refused(Harness::try_new(&scene, player_request(BOB, true), 0)).is_none());

    // Self-defence begins with no source at all.
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene
        .players
        .push(aggressor(&scene, 8, "alice", tile(3101, 3600)));
    scene.refresh();
    scene.globals = PvpGlobals::fail_closed();
    assert!(refused(Harness::try_new(&scene, self_defence_request(), 0)).is_none());
}

#[test]
fn player_zone_gate_uses_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    let mut bob = player(8, BOB, tile(3101, 3600), &fight_ops(), 60);
    bob.network = tile(2700, 3600);
    scene.players.push(bob);
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::OpponentOutside)
    );
}

#[test]
fn named_player_search_bounds_use_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    let rendered = tile(3101, 3600);
    let mut bob = player(8, BOB, rendered, &fight_ops(), 60);
    bob.network = tile(3105, 3600);
    scene.players.push(bob);
    scene.refresh();
    let mut request = player_request(BOB, true);
    request.search_bounds = Some(Arc::from([api::gather_methods::SceneRegionInput {
        min_x: rendered.x,
        min_z: rendered.z,
        max_x: rendered.x,
        max_z: rendered.z,
        level: rendered.level,
    }]));
    let mut harness = Harness::new(&scene, request);
    let effect = harness.pending(&scene, 1);
    assert_eq!(harness.machine.engaged(), None);
    assert!(
        !matches!(
            effect,
            Some(HostEffect::Interaction(InteractReq::Player { .. }))
        ),
        "the rendered in-bounds pose cannot admit a player whose network tile is out of bounds"
    );
}

#[test]
fn attacker_player_search_bounds_use_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    let rendered = tile(3101, 3600);
    let mut alice = aggressor(&scene, 8, "alice", rendered);
    alice.network = tile(3105, 3600);
    scene.players.push(alice);
    scene.refresh();
    let mut request = self_defence_request();
    request.search_bounds = Some(Arc::from([api::gather_methods::SceneRegionInput {
        min_x: rendered.x,
        min_z: rendered.z,
        max_x: rendered.x,
        max_z: rendered.z,
        level: rendered.level,
    }]));
    let mut harness = Harness::new(&scene, request);
    let effect = harness.pending(&scene, 1);
    assert_eq!(harness.machine.engaged(), None);
    assert!(
        !matches!(
            effect,
            Some(HostEffect::Interaction(InteractReq::Player { .. }))
        ),
        "a rendered in-bounds aggressor is not selected when its network tile is outside bounds"
    );
}

#[test]
fn a_global_flip_during_a_fight_changes_the_next_begin_only() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    scene.globals = PvpGlobals { start_fights: true };
    let mut harness = Harness::new(&scene, player_request(BOB, false));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    // The setting goes off: the running fight is not asked again.
    scene.globals = PvpGlobals::fail_closed();
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    for tick in 2..6 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.take();
    }
    assert!(harness.machine.end.is_none(), "the fight continues");
    // The next begin is refused.
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, false), 6)),
        Some(PvpRefusal::NotOptedIn)
    );
}

#[test]
fn engage_self_defence_never_reads_opt_in() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene
        .players
        .push(aggressor(&scene, 8, "alice", tile(3101, 3600)));
    scene.refresh();
    scene.globals = PvpGlobals::fail_closed();
    let mut harness = Harness::new(&scene, self_defence_request());
    player_attack(harness.pending(&scene, 1), "alice", "Attack");
    assert!(harness.machine.engaged.is_some());
    assert_eq!(harness.machine.counters.pvp_blocked, 0);
    assert_eq!(harness.machine.pvp_source, None);
}

#[test]
fn case1b_intrusion_self_defence_engages_a_labelled_player_without_any_opt_in() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    let imp = Scene::new("imp");
    let mut npc = imp.npcs[0].clone();
    npc.tile = tile(3101, 3601);
    npc.network = tile(3101, 3601);
    scene.npcs.push(npc);
    scene
        .players
        .push(aggressor(&scene, 8, "alice", tile(3099, 3600)));
    scene.refresh();
    scene.globals = PvpGlobals::fail_closed();
    let mut request = scene.request();
    request.pvp = false;
    let mut harness = Harness::new(&scene, request);
    // The first poll engages the imp; the intruder is taken on the next.
    let _ = harness.poll(&scene.snapshot, 1);
    harness.take();
    let _ = harness.poll(&scene.snapshot, 2);
    harness.take();
    assert_eq!(harness.machine.counters.intruders, 1);
    assert_eq!(
        harness.machine.suspended.map(|actor| actor.kind),
        Some(ActorKind::Npc)
    );
    assert_eq!(
        harness.machine.engaged.map(|actor| actor.kind),
        Some(ActorKind::Player)
    );
}

#[test]
fn case1c_an_intruder_without_the_attack_label_stays_a_threat_and_is_not_engaged() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3600));
    let imp = Scene::new("imp");
    let mut npc = imp.npcs[0].clone();
    npc.tile = tile(3101, 3601);
    npc.network = tile(3101, 3601);
    scene.npcs.push(npc);
    let mut intruder = aggressor(&scene, 8, "alice", tile(3099, 3600));
    intruder.actor.actions = vec![None, None];
    scene.players.push(intruder);
    scene.refresh();
    let mut request = scene.request();
    request.pvp = false;
    let mut harness = Harness::new(&scene, request);
    // One poll: the blocked intruder is counted once and never engaged.
    let _ = harness.poll(&scene.snapshot, 1);
    harness.take();
    assert_eq!(harness.machine.counters.intruders, 0);
    assert_eq!(
        harness.machine.engaged.map(|actor| actor.kind),
        Some(ActorKind::Npc),
        "the npc fight continues"
    );
    assert_eq!(harness.machine.counters.pvp_blocked, 1);
}

// --- Case 3: the zone helper --------------------------------------------------

#[test]
fn case3_wilderness_levels_follow_the_dbrow_and_the_level_formula() {
    let rules = content_wilderness();
    let level = |x, z, plane| rules.level(api::WorldTile { x, z, level: plane });
    assert_eq!(level(3100, 3523, 0), 1);
    assert_eq!(level(3100, 3519, 0), 0);
    assert_eq!(level(3100, 9925, 0), 1);
    assert_eq!(level(3100, 3600, 1), 11);
    assert_eq!(level(2943, 3523, 0), 0);
}

// --- Cases 4 and 5: the zone gate and the attack label ------------------------

#[test]
fn case4_level_range_is_a_typed_refusal_with_its_payload_at_begin() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3523));
    scene.local.player.combat_level = 40;
    scene
        .players
        .push(player(8, BOB, tile(3100, 3524), &attack_ops(), 52));
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::LevelRange {
            wilderness: 1,
            difference: 12
        })
    );
    // Twelve levels deeper the same fight is allowed (wl 12 >= difference 12).
    let mut scene = wilderness_scene(tile(3100, 3608));
    scene.local.player.combat_level = 40;
    scene
        .players
        .push(player(8, BOB, tile(3100, 3609), &attack_ops(), 52));
    scene.refresh();
    assert!(refused(Harness::try_new(&scene, player_request(BOB, true), 0)).is_none());
}

#[test]
fn case4_an_opponent_stepping_out_of_the_wilderness_aborts_within_one_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3521));
    scene
        .players
        .push(player(8, BOB, tile(3100, 3520), &attack_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Attack");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    assert!(harness.pending(&scene, 2).is_none());
    // Bob steps south of the zone edge (wilderness level 0).
    scene.players[0].actor.tile = tile(3100, 3519);
    scene.players[0].network = tile(3100, 3519);
    scene.refresh();
    let report = harness.ready(&scene, 3);
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::PvpRefused(PvpRefusal::OpponentOutside))
    );
}

#[test]
fn case4_outside_every_pvp_zone_the_zone_reason_is_reported() {
    if !api::test_support::game_data_available() {
        return;
    }
    // A tile in Lumbridge: neither the Wilderness nor a pen.
    let mut scene = wilderness_scene(tile(3222, 3218));
    scene
        .players
        .push(player(8, BOB, tile(3223, 3218), &attack_ops(), 60));
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::OutsideWilderness)
    );
    // Real content: a Lumbridge player row carries no attack op at all, and
    // the refusal is still the zone, never "no attack op".
    let mut scene = wilderness_scene(tile(3222, 3218));
    scene
        .players
        .push(player(8, BOB, tile(3223, 3218), &[None, None], 60));
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::OutsideWilderness)
    );
    // The challenge area is its own refusal, never a pen.
    let mut scene = wilderness_scene(tile(3360, 3270));
    scene
        .players
        .push(player(8, BOB, tile(3361, 3270), &fight_ops(), 60));
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::ChallengeArea)
    );
    // Before the opponent is visible, only where we stand can refuse.
    let scene = wilderness_scene(tile(3222, 3218));
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::OutsideWilderness)
    );
    let scene = wilderness_scene(tile(3100, 3600));
    assert!(refused(Harness::try_new(&scene, player_request(BOB, true), 0)).is_none());
}

#[test]
fn case5_the_attack_verb_is_the_observed_label() {
    if !api::test_support::game_data_available() {
        return;
    }
    // A duel pen row carries "Fight": the verb is "Fight".
    let mut scene = pen_scene();
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");

    // A Wilderness row carries "Attack".
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene
        .players
        .push(player(8, BOB, tile(3101, 3600), &attack_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Attack");

    // The machine never invents a label: a row that offers "Fight" in the
    // Wilderness gets "Fight", not "Attack".
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene
        .players
        .push(player(8, BOB, tile(3101, 3600), &fight_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");

    // No attack op at all: refused at begin as a row that cannot be attacked.
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene
        .players
        .push(player(8, BOB, tile(3101, 3600), &[None, None], 60));
    scene.refresh();
    assert_eq!(
        refused(Harness::try_new(&scene, player_request(BOB, true), 0)),
        Some(PvpRefusal::NoAttackOp)
    );
    // A threat without the op is never engaged for self-defence either.
    let mut scene = wilderness_scene(tile(3100, 3600));
    let mut threat = aggressor(&scene, 8, "alice", tile(3101, 3600));
    threat.actor.actions = vec![None, None];
    scene.players.push(threat);
    scene.refresh();
    let mut harness = Harness::new(&scene, self_defence_request());
    let _ = harness.poll(&scene.snapshot, 1);
    assert!(harness.take().is_none());
    assert!(harness.machine.engaged.is_none());
}

// --- Case 6: the server lines -------------------------------------------------

fn abort_on_line(line: &str) -> CombatEnd {
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, line);
    match harness.poll(&scene.snapshot, 3) {
        Poll::Ready(Ok(report)) => report.end,
        other => panic!("{line:?} must end the fight, got {other:?}"),
    }
}

#[test]
fn case6_wilderness_and_arena_lines_end_the_fight_with_their_typed_reason() {
    if !api::test_support::game_data_available() {
        return;
    }
    let aborted = |reason| CombatEnd::Aborted(AbortReason::PvpRefused(reason));
    assert_eq!(
        abort_on_line("You can't attack players who aren't in the wilderness."),
        aborted(PvpRefusal::OpponentOutside)
    );
    assert_eq!(
        abort_on_line("You cannot attack players in the challenge area."),
        aborted(PvpRefusal::ChallengeArea)
    );
    assert_eq!(
        abort_on_line("You can only attack your opponent!"),
        aborted(PvpRefusal::NotOpponent)
    );
    assert!(matches!(
        abort_on_line("Your level difference is too great!"),
        CombatEnd::Aborted(AbortReason::PvpRefused(PvpRefusal::LevelRange { .. }))
    ));
    assert!(matches!(
        abort_on_line("You need to move deeper into the wilderness."),
        CombatEnd::Aborted(AbortReason::PvpRefused(PvpRefusal::LevelRange { .. }))
    ));
}

#[test]
fn case6_a_line_is_matched_exactly_never_by_substring() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(
        &mut scene,
        1,
        "You can only attack your opponent! (a player said this)",
    );
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    assert!(harness.machine.end.is_none());
}

#[test]
fn case6_out_of_scope_refusal_lines_abort_unattackable_on_the_third() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    for (tick, sequence) in [(3, 1), (4, 2)] {
        say(
            &mut scene,
            sequence,
            "Kolodion allows only magical combat within the arena.",
        );
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.take();
        assert!(harness.machine.end.is_none(), "line {sequence} is bounded");
    }
    say(
        &mut scene,
        3,
        "Kolodion allows only magical combat within the arena.",
    );
    let report = match harness.poll(&scene.snapshot, 5) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third line must end the fight, got {other:?}"),
    };
    assert_eq!(report.end, CombatEnd::Aborted(AbortReason::Unattackable));
}

#[test]
fn case6_the_single_way_line_holds_the_attack_for_exactly_eight_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, "I'm already under attack.");
    // The line is read at tick 3: no Attack row on ticks 3..=10.
    for tick in 3..=10 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        let batch = harness.drain(None);
        assert!(
            !has_player_attack(&batch, "Fight"),
            "an attack row inside the hold at tick {tick}"
        );
    }
    assert!(matches!(harness.poll(&scene.snapshot, 11), Poll::Pending));
    let batch = harness.drain(None);
    assert!(
        has_player_attack(&batch, "Fight"),
        "the attack is re-issued when the hold ends"
    );
    assert!(harness.machine.end.is_none(), "the engagement is kept");
}

/// A pen fight cast by hand (`Style::Mage` with an ordered spell list, so the
/// machine casts instead of attacking): the first cast is issued at tick 1.
fn installed_manual_cast_pen_fight() -> (Scene, Harness) {
    use super::magic_tests::{magic_scene, refresh_magic, spend};
    let mut scene = magic_scene(Some("staff_of_fire"), 60);
    scene.wilderness = Some(content_wilderness());
    place_local(&mut scene, tile(3340, 3250));
    scene
        .players
        .push(player(8, BOB, tile(3341, 3250), &fight_ops(), 60));
    refresh_magic(&mut scene, 3, false);
    let request = CombatRequest {
        style: Style::Mage,
        spells: Some(
            [SpellRef {
                alias: Arc::from("fire_bolt"),
            }]
            .into_iter()
            .collect::<Vec<_>>()
            .into(),
        ),
        retaliate: false,
        ..player_request(BOB, true)
    };
    let mut harness = Harness::new(&scene, request);
    assert!(
        is_player_cast(&harness.pending(&scene, 1)),
        "the first manual cast is issued at the player"
    );
    spend(&mut scene, "fire_bolt");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    refresh_magic(&mut scene, 3, false);
    assert!(harness.pending(&scene, 2).is_none());
    (scene, harness)
}

fn is_player_cast(effect: &Option<HostEffect>) -> bool {
    matches!(effect, Some(HostEffect::Interaction(InteractReq::UseWidgetOn { kind, .. })) if kind.as_ref() as &str == "player")
}

#[test]
fn manual_player_cast_uses_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    use super::magic_tests::{magic_scene, refresh_magic};
    let mut scene = magic_scene(Some("staff_of_fire"), 60);
    scene.wilderness = Some(content_wilderness());
    place_local(&mut scene, tile(3340, 3250));
    let rendered = tile(3341, 3250);
    let network = tile(3342, 3250);
    let mut bob = player(8, BOB, rendered, &fight_ops(), 60);
    bob.network = network;
    scene.players.push(bob);
    refresh_magic(&mut scene, 3, false);
    let request = CombatRequest {
        style: Style::Mage,
        spells: Some(
            [SpellRef {
                alias: Arc::from("fire_bolt"),
            }]
            .into_iter()
            .collect::<Vec<_>>()
            .into(),
        ),
        retaliate: false,
        ..player_request(BOB, true)
    };
    let mut harness = Harness::new(&scene, request);
    assert!(
        matches!(
            harness.pending(&scene, 1),
            Some(HostEffect::Interaction(InteractReq::UseWidgetOn {
                kind,
                x,
                z,
                level,
                index: Some(8),
                ..
            })) if kind == "player" && (x, z, level) == (network.x, network.z, network.level)
        ),
        "manual player casts must carry the network path-head coordinates"
    );
}

#[test]
fn case6_the_single_way_line_holds_a_manual_cast_for_exactly_eight_ticks() {
    if !api::test_support::game_data_available() {
        return;
    }
    use super::magic_tests::refresh_magic;
    let (mut scene, mut harness) = installed_manual_cast_pen_fight();
    say(&mut scene, 1, "I'm already under attack.");
    // The line is read at tick 3: no cast on ticks 3..=10.
    for tick in 3..=10 {
        let effect = harness.pending(&scene, tick);
        assert!(
            !is_player_cast(&effect),
            "a manual cast inside the hold at tick {tick}"
        );
        refresh_magic(&mut scene, 3, false);
    }
    assert!(
        is_player_cast(&harness.pending(&scene, 11)),
        "the cast is re-issued when the hold ends"
    );
    assert!(harness.machine.end.is_none(), "the engagement is kept");
}

#[test]
fn case6_the_duel_not_started_line_holds_a_manual_cast_for_one_tick() {
    if !api::test_support::game_data_available() {
        return;
    }
    use super::magic_tests::refresh_magic;
    let (mut scene, mut harness) = installed_manual_cast_pen_fight();
    // Cast spacing has passed by tick 8, so only the hold can stop a cast.
    refresh_magic(&mut scene, 3, false);
    assert!(harness.pending(&scene, 3).is_none());
    say(&mut scene, 1, "The duel has not started yet!");
    assert!(
        !is_player_cast(&harness.pending(&scene, 8)),
        "a manual cast on the tick that read the line"
    );
    refresh_magic(&mut scene, 3, false);
    assert!(
        is_player_cast(&harness.pending(&scene, 9)),
        "the cast is re-issued one tick later"
    );
}

#[test]
fn case6_someone_else_fighting_the_opponent_ends_the_run_after_three_lines() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    // The server refused every attack: we are not in combat with the opponent.
    scene.local.player.actor.target = None;
    scene.local.player.actor.in_combat = false;
    scene.refresh();
    for (tick, sequence) in [(3, 1), (12, 2)] {
        say(
            &mut scene,
            sequence,
            "Someone else is already fighting your opponent.",
        );
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.take();
        assert!(harness.machine.end.is_none());
    }
    say(
        &mut scene,
        3,
        "Someone else is already fighting your opponent.",
    );
    let report = match harness.poll(&scene.snapshot, 21) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third consecutive line must end the run, got {other:?}"),
    };
    assert_eq!(report.end, CombatEnd::TargetGone);
}

#[test]
fn case6_the_duel_not_started_line_holds_one_tick_and_ends_on_the_third() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, "The duel has not started yet!");
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    assert!(!has_player_attack(&harness.drain(None), "Fight"));
    assert!(matches!(harness.poll(&scene.snapshot, 4), Poll::Pending));
    harness.take();
    say(&mut scene, 2, "The duel has not started yet!");
    assert!(matches!(harness.poll(&scene.snapshot, 5), Poll::Pending));
    harness.take();
    say(&mut scene, 3, "The duel has not started yet!");
    let report = match harness.poll(&scene.snapshot, 6) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the third line must end the run, got {other:?}"),
    };
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::PvpRefused(PvpRefusal::DuelNotStarted))
    );
}

#[test]
fn case6_a_kill_line_is_recorded_and_never_ends_the_fight() {
    if !api::test_support::game_data_available() {
        return;
    }
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, "With a crushing blow you finish bob.");
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    harness.take();
    assert!(harness.machine.end.is_none());
    assert!(harness.machine.flags2 & PVP_KILL_LINE != 0);
    // A kill line about another name is not this fight's.
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, "With a crushing blow you finish carol.");
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    harness.take();
    assert!(harness.machine.flags2 & PVP_KILL_LINE == 0);
    // The report of a finished fight carries both facts.
    let (mut scene, mut harness) = installed_pen_fight(true);
    say(&mut scene, 1, "bob was no match for you.");
    assert!(matches!(harness.poll(&scene.snapshot, 3), Poll::Pending));
    harness.take();
    scene.players[0].actor.health = 0;
    scene.refresh();
    let report = harness.ready(&scene, 4);
    assert_eq!(report.end, CombatEnd::Killed);
    assert!(report.kill_line_seen);
    assert_eq!(report.pvp_source, Some(PvpSource::Request));
}

#[test]
fn case6_every_kill_statement_names_the_opponent() {
    for template in [
        "You have defeated {}.",
        "{} was no match for you.",
        "With a crushing blow you finish {}.",
        "Can anyone defeat you? Certainly not {}.",
        "{} regrets the day they met you in combat.",
        "It's all over for {}.",
        "{} didn't stand a chance against you.",
        "A humiliating defeat for {}.",
        "{} falls before your might.",
        "{} has won a free ticket to Lumbridge.",
        "You were clearly a better fighter than {}.",
    ] {
        assert!(select::is_kill_line(&template.replace("{}", BOB), BOB));
        assert!(!select::is_kill_line(&template.replace("{}", "carol"), BOB));
    }
}

// --- Case 19: death timing ------------------------------------------------------

#[test]
fn case19_the_zero_bar_latches_killed_and_no_vanish_is_required() {
    if !api::test_support::game_data_available() {
        return;
    }
    // The dying row stays present at zero health for several ticks; the kill
    // is latched on the first zero-bar frame, never TargetGone.
    for frames_present in [1u64, 6, 10] {
        let (mut scene, mut harness) = installed_pen_fight(true);
        scene.players[0].actor.health = 0;
        scene.refresh();
        let report = harness.ready(&scene, 3);
        assert_eq!(
            report.end,
            CombatEnd::Killed,
            "after {frames_present} frames"
        );
    }
}

// --- Case 22: lost radius in a pen ----------------------------------------------

#[test]
fn case22_a_pen_opponent_24_tiles_away_stays_engaged_with_lost_radius_32() {
    if !api::test_support::game_data_available() {
        return;
    }
    let at = |far: bool| {
        let mut scene = wilderness_scene(tile(3333, 3244));
        let target = if far {
            tile(3357, 3258)
        } else {
            tile(3334, 3244)
        };
        scene.players.push(player(8, BOB, target, &fight_ops(), 60));
        scene.refresh();
        scene
    };
    // 24 tiles apart (the pen's long axis), the trainer's lost radius of 32.
    let mut scene = at(true);
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    for tick in 2..8 {
        assert!(matches!(harness.poll(&scene.snapshot, tick), Poll::Pending));
        harness.take();
    }
    assert!(harness.machine.end.is_none(), "still engaged at 24 tiles");

    // The default lost radius of 20 loses the same opponent after three ticks.
    let mut request = player_request(BOB, true);
    request.lost_radius = 20;
    let mut scene = at(true);
    let mut harness = Harness::new(&scene, request);
    let _ = harness.poll(&scene.snapshot, 1);
    harness.take();
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    let mut ended = None;
    for tick in 2..8 {
        if let Poll::Ready(Ok(report)) = harness.poll(&scene.snapshot, tick) {
            ended = Some(report.end);
            break;
        }
        harness.take();
    }
    assert_eq!(ended, Some(CombatEnd::TargetGone));
}

#[test]
fn player_lost_radius_and_leash_cancel_use_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = pen_scene();
    let stand = scene.local.player.network;
    scene
        .players
        .push(player(8, BOB, tile(stand.x + 1, stand.z), &fight_ops(), 60));
    scene.refresh();
    let mut request = player_request(BOB, true);
    request.stand = Some(stand);
    request.lost_radius = 1;
    let mut harness = Harness::new(&scene, request);
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.players[0].actor.tile = tile(stand.x + 1, stand.z);
    scene.players[0].network = tile(stand.x + 7, stand.z);
    scene.refresh();
    assert!(matches!(
        harness.pending(&scene, 2),
        Some(HostEffect::Walk(walk)) if walk.target == tile(stand.x - 2, stand.z)
    ));
}

#[test]
fn level_refusal_uses_player_network_coords() {
    if !api::test_support::game_data_available() {
        return;
    }
    let mut scene = wilderness_scene(tile(3100, 3700));
    scene.local.player.combat_level = 40;
    scene
        .players
        .push(player(8, BOB, tile(3101, 3700), &fight_ops(), 60));
    scene.refresh();
    let mut harness = Harness::new(&scene, player_request(BOB, true));
    player_attack(harness.pending(&scene, 1), BOB, "Fight");
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.players[0].network = tile(3101, 3528);
    scene.refresh();
    say(
        &mut scene,
        1,
        "You need to move deeper into the wilderness.",
    );
    let report = match harness.poll(&scene.snapshot, 3) {
        Poll::Ready(Ok(report)) => report,
        other => panic!("the level refusal must end the fight: {other:?}"),
    };
    assert_eq!(
        report.end,
        CombatEnd::Aborted(AbortReason::PvpRefused(PvpRefusal::LevelRange {
            wilderness: 2,
            difference: 20,
        }))
    );
}

// --- Case 20: the protect follows the opponent's observed style -----------------

fn protect_button(scene: &Scene, name: &str) -> i32 {
    scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == name)
        .unwrap()
        .button_com
}

fn has_prayer_row(batch: &DrainedBatch, button: i32) -> bool {
    (0..batch.len()).any(|index| {
        matches!(
            batch.get(index),
            Some(HostEffect::Interaction(InteractReq::IfButton { component_id })) if *component_id == button
        )
    })
}

#[test]
fn protect_plan_on_first_frame() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Alice fights us bare-handed (melee), then wears a shortbow and shoots in
    // the same tick (frame N). The plan of frame N already carries the switch
    // to Protect from Missiles: the product reacts on the first frame that
    // shows the new weapon, never later (PVP-DESIGN-1026 §5.1).
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene.stat(5, 50, 50);
    scene
        .players
        .push(aggressor(&scene, 8, "alice", tile(3101, 3600)));
    scene.refresh();
    let mut harness = Harness::new(&scene, self_defence_request());
    let melee = protect_button(&scene, "Protect from Melee");
    let missiles = protect_button(&scene, "Protect from Missiles");
    let first = harness.pending_batch(&scene, 1);
    assert!(
        has_prayer_row(&first, melee),
        "melee protect on the first frame"
    );
    let protect_melee_varp = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .varp;
    scene.prayer(protect_melee_varp, true);
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    let _ = harness.pending_batch(&scene, 2);

    // Frame N: the new weapon and its first shot appear together.
    let bow = scene.data.item_by_alias("shortbow").unwrap().id;
    scene.players[0].weapon = Some(bow);
    scene.players[0].actor.animation = scene.ranged_seq();
    scene.refresh();
    let frame_n = harness.pending_batch(&scene, 3);
    assert!(
        has_prayer_row(&frame_n, missiles),
        "the plan of the first frame showing the bow switches the protect"
    );
}

#[test]
fn protect_follows_a_weapon_change_before_the_first_swing_with_it() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Case 20 (b): the opponent wears a bow with swing clock left: no attack
    // animation yet. The plan of the first frame that shows the bow already
    // carries Protect from Missiles.
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene.stat(5, 50, 50);
    scene
        .players
        .push(aggressor(&scene, 8, "alice", tile(3101, 3600)));
    scene.refresh();
    let mut harness = Harness::new(&scene, self_defence_request());
    let missiles = protect_button(&scene, "Protect from Missiles");
    let melee_protect = scene
        .data
        .prayers()
        .iter()
        .find(|row| row.name == "Protect from Melee")
        .unwrap()
        .varp;
    let first = harness.pending_batch(&scene, 1);
    assert!(!has_prayer_row(&first, missiles));
    scene.prayer(melee_protect, true);
    scene.local.player.actor.target = Some(ActorTargetView {
        kind: ActorKind::Player,
        index: 8,
    });
    scene.local.player.actor.in_combat = true;
    scene.refresh();
    let _ = harness.pending_batch(&scene, 2);
    // The bow appears while the melee swing is still mid-animation.
    scene.players[0].weapon = Some(scene.data.item_by_alias("shortbow").unwrap().id);
    scene.players[0].actor.animation_frame = 3;
    scene.refresh();
    let frame_n = harness.pending_batch(&scene, 3);
    assert!(
        has_prayer_row(&frame_n, missiles),
        "the protect follows the observed weapon on the first frame"
    );
}

#[test]
fn protect_follows_the_melee_event_of_a_staff_wielder() {
    if !api::test_support::game_data_available() {
        return;
    }
    // Case 20 (d): a staff wielder who meleed us: the melee event overrides the
    // staff's magic hint on the frame of the onset.
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene.stat(5, 50, 50);
    let mut alice = aggressor(&scene, 8, "alice", tile(3101, 3600));
    alice.weapon = Some(scene.data.item_by_alias("staff_of_fire").unwrap().id);
    // A sequence only melee attacks use (a shared one is an Unknown onset).
    alice.actor.animation = scene
        .data
        .style_seqs()
        .iter()
        .find(|row| {
            scene
                .tables
                .style_seq(row.seq_id)
                .is_some_and(|mask| mask.bits() == 1)
        })
        .unwrap()
        .seq_id;
    scene.players.push(alice);
    scene.refresh();
    let mut harness = Harness::new(&scene, self_defence_request());
    let first = harness.pending_batch(&scene, 1);
    assert!(
        has_prayer_row(&first, protect_button(&scene, "Protect from Melee")),
        "the melee event decides on the frame of the onset, not the staff"
    );
    assert!(!has_prayer_row(
        &first,
        protect_button(&scene, "Protect from Magic")
    ));

    // Control: an attack sequence that melee and magic share is an Unknown
    // onset, so the staff's hint stands.
    let mut scene = wilderness_scene(tile(3100, 3600));
    scene.stat(5, 50, 50);
    let mut alice = aggressor(&scene, 8, "alice", tile(3101, 3600));
    alice.weapon = Some(scene.data.item_by_alias("staff_of_fire").unwrap().id);
    scene.players.push(alice);
    scene.refresh();
    let mut harness = Harness::new(&scene, self_defence_request());
    let first = harness.pending_batch(&scene, 1);
    assert!(has_prayer_row(
        &first,
        protect_button(&scene, "Protect from Magic")
    ));
}
