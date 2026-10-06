//! Regressions for the shared Quester target chooser (DIAG-COOK-HEADED):
//! reachable in-area targets ranked by walking distance, live re-resolution
//! while chasing, "I can't reach that!" re-picks, and opportunistic retargets.

use super::*;

/// A live reach view flooded from `player` over a `size`-square open scene at
/// `base`, with collision `flags` OR-ed onto tiles (pens, fence walls).
fn flood_view(
    base: WorldTile,
    size: i32,
    flags: &[(WorldTile, i32)],
    player: WorldTile,
) -> api::query::ReachQueryView {
    let mut scene = api::snapshot::SceneView {
        available: true,
        base_x: base.x,
        base_z: base.z,
        level: 0,
        width: size,
        height: size,
        collision_flags: vec![0; (size * size) as usize],
    };
    for (tile, flag) in flags {
        let index = ((tile.x - base.x) * size + tile.z - base.z) as usize;
        scene.collision_flags[index] |= flag;
    }
    let flood = api::query::SceneQuery::new(&scene, Some(player)).flood_reach();
    api::query::pack_reach_query(&scene, flood.as_ref())
}

/// The eight tiles around `center` blocked: a closed pen.
fn pen(center: WorldTile) -> Vec<(WorldTile, i32)> {
    (-1..=1)
        .flat_map(|dx| (-1..=1).map(move |dz| (dx, dz)))
        .filter(|&(dx, dz)| dx != 0 || dz != 0)
        .map(|(dx, dz)| {
            (
                tile(center.x + dx, center.z + dz),
                client::dash3d::CollisionFlag::SQ_BLOCKED,
            )
        })
        .collect()
}

/// A fence along the whole scene between columns `x - 1` and `x`.
fn fence_west_of(x: i32, z_range: std::ops::Range<i32>) -> Vec<(WorldTile, i32)> {
    use client::dash3d::CollisionFlag;
    z_range
        .flat_map(|z| {
            [
                (tile(x - 1, z), CollisionFlag::W_E),
                (tile(x, z), CollisionFlag::W_W),
            ]
        })
        .collect()
}

fn cow(id: i32, index: usize, at: WorldTile, from: WorldTile) -> api::snapshot::NpcView {
    api::snapshot::NpcView {
        index,
        r#type: Some(id as usize),
        name: Some("Cow".into()),
        actions: vec![Some("Attack".into())],
        tile: at,
        distance: (at.x - from.x).abs().max((at.z - from.z).abs()),
        animation: -1,
        animation_frame: 0,
        pose_animation: -1,
        orientation: 0,
        target_orientation: 0,
        overhead_text: None,
        spot_animation: -1,
        spot_animation_stamp: -1,
        health: 8,
        total_health: 8,
        face_entity: -1,
        target: None,
        moving: false,
        running: false,
        in_combat: false,
        level: 2,
        size: 2,
        network: at,
        x: 0,
        z: 0,
        yaw: 0,
    }
}

fn last_effect(ledger: &Option<Box<ledger::Ledger>>) -> &HostEffect {
    &ledger.as_ref().unwrap().outbox.last().unwrap().effect
}

fn describe(ledger: &Option<Box<ledger::Ledger>>) -> String {
    match last_effect(ledger) {
        HostEffect::Walk(request) => format!("walk to {:?} r{}", request.target, request.radius),
        HostEffect::Interaction(request) => format!("{request:?}"),
        HostEffect::BankPick(_) => "bank pick".into(),
        HostEffect::AssessWalk(request) => format!("assess walk to {:?}", request.target),
    }
}

/// Cook's milk step: an empty bucket used on any cow near the field anchor.
fn milk_fixture(cx: &CompileContext<'_>, player: WorldTile) -> (GameSnapshot, Arc<dyn StepPlan>) {
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(player));
    snapshot.seed_inventory(
        vec![ItemView {
            def: def(resolve_obj(cx, "bucket_empty").unwrap(), "Bucket"),
            container: ItemContainer::Inventory,
            action_family: ItemActionFamily::Held,
            slot: 0,
            count: 1,
            actions: vec![],
            component_id: 3214,
        }],
        28,
    );
    let plan = compile_use_on(
        test_args::<UseOnArgs>(serde_json::json!({
            "item": "bucket_empty",
            "target": {"npc": "cow"},
            "anchor": {"tile": [player.x, player.z, 0], "source": "unit fixture"},
            "radius": 4,
            "product": "bucket_milk"
        })),
        cx,
    )
    .unwrap();
    (snapshot, plan)
}

#[test]
fn reachable_egg_wins_over_a_nearer_egg_behind_the_coop_fence() {
    let player = tile(3222, 3300);
    let decoy_at = tile(3224, 3300);
    let reachable_at = tile(3222, 3305);
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(player));
    let mut decoy = ground();
    decoy.tile = decoy_at;
    decoy.distance = 2;
    let mut reachable = ground();
    reachable.tile = reachable_at;
    reachable.distance = 5;
    snapshot.seed_ground_items(vec![decoy, reachable]);
    let view = flood_view(tile(3212, 3290), 24, &pen(decoy_at), player);
    let mut ledger = None;
    let _handle = with_tick_reach(&snapshot, &view, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    assert!(
        matches!(
            last_effect(&ledger),
            HostEffect::Interaction(InteractReq::Obj {
                x: 3222,
                z: 3305,
                ..
            })
        ),
        "the egg inside the closed pen must not be clicked while a reachable egg exists, got {:?}",
        describe(&ledger)
    );
}

#[test]
fn cant_reach_egg_repicks_the_other_egg_instead_of_hunting_doors() {
    let mut snapshot = ready();
    let mut first = ground();
    first.tile = tile(3226, 3301);
    first.distance = 2;
    let mut second = ground();
    second.tile = tile(3229, 3299);
    second.distance = 4;
    snapshot.seed_ground_items(vec![first, second]);
    let mut ledger = None;
    let handle = with_tick(&snapshot, &mut ledger, 1, |t| {
        t.actions
            .begin::<reach::Reach>(egg(true), &mut t.cx)
            .unwrap()
    });
    assert!(matches!(
        last_effect(&ledger),
        HostEffect::Interaction(InteractReq::Obj {
            x: 3226,
            z: 3301,
            ..
        })
    ));
    snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
        sequence: 1,
        text: "I can't reach that!".into(),
        type_: 0,
        username: None,
    }]);
    assert!(with_tick(&snapshot, &mut ledger, 2, |t| t
        .actions
        .poll(&handle, &mut t.cx))
    .is_pending());
    assert!(
        matches!(
            last_effect(&ledger),
            HostEffect::Interaction(InteractReq::Obj {
                x: 3229,
                z: 3299,
                ..
            })
        ),
        "the unreachable egg is a failed attempt: take the other egg at once, got {:?}",
        describe(&ledger)
    );
}

#[test]
fn stale_cow_chase_repicks_the_live_reachable_cow() {
    compile_context_test(|cx| {
        let player = tile(3250, 3280);
        let cow_id = resolve_npc(cx, "cow").unwrap();
        let (mut snapshot, plan) = milk_fixture(cx, player);
        snapshot.seed_npcs(vec![cow(cow_id, 1, tile(3254, 3280), player)]);
        let view = flood_view(tile(3238, 3268), 24, &[], player);
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &view, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(last_effect(&ledger), HostEffect::Walk(request) if request.target == tile(3254, 3280)),
            "a cow three tiles away is chased, got {:?}",
            describe(&ledger)
        );

        // The chased cow wanders off; another cow is now beside the player.
        snapshot.seed_npcs(vec![cow(cow_id, 2, tile(3248, 3280), player)]);
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                last_effect(&ledger),
                HostEffect::Interaction(InteractReq::UseOn { index: Some(2), .. })
            ),
            "the stale chase must end and the adjacent live cow be milked, got {:?}",
            describe(&ledger)
        );
    });
}

#[test]
fn cow_behind_the_field_fence_is_not_milked_across_it() {
    compile_context_test(|cx| {
        let player = tile(3250, 3280);
        let cow_id = resolve_npc(cx, "cow").unwrap();
        let (mut snapshot, plan) = milk_fixture(cx, player);
        // One tile east (the old straight-line gate passes), but the field
        // fence runs between the player and the cow along the whole scene.
        snapshot.seed_npcs(vec![cow(cow_id, 1, tile(3251, 3280), player)]);
        let view = flood_view(
            tile(3238, 3268),
            24,
            &fence_west_of(3251, 3268..3292),
            player,
        );
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &view, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(last_effect(&ledger), HostEffect::Walk(_)),
            "a cow across the fence needs a nav approach, not a use, got {:?}",
            describe(&ledger)
        );
    });
}

#[test]
fn cant_reach_message_repicks_another_cow_immediately() {
    compile_context_test(|cx| {
        let player = tile(3250, 3280);
        let cow_id = resolve_npc(cx, "cow").unwrap();
        let (mut snapshot, plan) = milk_fixture(cx, player);
        snapshot.seed_npcs(vec![
            cow(cow_id, 1, tile(3251, 3280), player),
            cow(cow_id, 2, tile(3248, 3280), player),
        ]);
        let view = flood_view(tile(3238, 3268), 24, &[], player);
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &view, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(matches!(
            last_effect(&ledger),
            HostEffect::Interaction(InteractReq::UseOn { index: Some(1), .. })
        ));
        accept_last(&mut ledger, 3, true);
        snapshot.seed_chat_lines(vec![api::snapshot::ChatLineView {
            sequence: 1,
            text: "I can't reach that!".into(),
            type_: 0,
            username: None,
        }]);
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 3, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(
                last_effect(&ledger),
                HostEffect::Interaction(InteractReq::UseOn { index: Some(2), .. })
            ),
            "\"I can't reach that!\" must re-pick on the same tick, not wait out the settle, got {:?}",
            describe(&ledger)
        );
    });
}

#[test]
fn wheat_met_on_the_way_replaces_the_anchor_wheat_walk() {
    let anchor = tile(3155, 3300);
    let far = {
        let mut wheat = loc(313, "Wheat", "Pick");
        wheat.tile = anchor;
        wheat
    };
    let plan = InteractPlan {
        kind: reach::ReachKind::Loc {
            id: Some(313),
            name: Some(Arc::from("wheat")),
        },
        op: Arc::from("Pick"),
        tile: Some(anchor),
        radius: 1,
        wait_if_missing: false,
        settle_ms: None,
        ambiguous: false,
        default_dialogue: false,
        dialogue_options: None,
        until: None,
        target_tile: None,
        reachable_only: false,
    };
    // Approaching from the gate, only the anchor wheat is observed yet.
    let start = tile(3170, 3290);
    let mut snapshot = ready();
    snapshot.seed_local_player(local_player(start));
    let mut far_seen = far.clone();
    far_seen.distance = 15;
    snapshot.seed_locs(vec![far_seen]);
    let mut ledger = None;
    let mut run = with_tick(&snapshot, &mut ledger, 1, |tick| {
        with_step(tick, |cx| plan.begin(cx).unwrap())
    });
    assert!(with_tick(&snapshot, &mut ledger, 2, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(
        matches!(last_effect(&ledger), HostEffect::Walk(request) if request.target == anchor),
        "the walk heads for the anchor wheat, got {:?}",
        describe(&ledger)
    );

    // Inside the gate, wheat beside the path is reachable and much nearer.
    let here = tile(3158, 3290);
    let mut near = far.clone();
    near.tile = tile(3158, 3291);
    near.distance = 1;
    let mut far_now = far;
    far_now.distance = 10;
    snapshot.seed_local_player(local_player(here));
    snapshot.seed_locs(vec![far_now, near]);
    let view = flood_view(tile(3140, 3280), 32, &[], here);
    assert!(with_tick_reach(&snapshot, &view, &mut ledger, 3, |tick| {
        with_step(tick, |cx| run.poll(cx))
    })
    .is_pending());
    assert!(
        matches!(
            last_effect(&ledger),
            HostEffect::Interaction(InteractReq::Loc {
                x: 3158,
                z: 3291,
                ..
            })
        ),
        "the wheat passed on the way must be picked, got {:?}",
        describe(&ledger)
    );
}

#[test]
fn alternating_reachability_stops_after_the_bound_not_the_settle() {
    compile_context_test(|cx| {
        let player = tile(3250, 3280);
        let cow_id = resolve_npc(cx, "cow").unwrap();
        let (mut snapshot, plan) = milk_fixture(cx, player);
        let view = flood_view(
            tile(3238, 3268),
            24,
            &fence_west_of(3251, 3268..3292),
            player,
        );
        let west = tile(3246, 3280);
        let east = tile(3252, 3280);
        snapshot.seed_npcs(vec![
            cow(cow_id, 1, west, player),
            cow(cow_id, 2, east, player),
        ]);
        let mut ledger = None;
        let mut run = with_tick_reach(&snapshot, &view, &mut ledger, 1, |tick| {
            with_step(tick, |cx| plan.begin(cx).unwrap())
        });
        assert!(with_tick_reach(&snapshot, &view, &mut ledger, 2, |tick| {
            with_step(tick, |cx| run.poll(cx))
        })
        .is_pending());
        assert!(
            matches!(last_effect(&ledger), HostEffect::Walk(_)),
            "a cow four tiles away is chased, got {:?}",
            describe(&ledger)
        );
        // Two cows alternate which side of the fence they stand on. Every
        // swap would restart the walk; the bound must stop that long before
        // the 20 s settle. `NativeActions::begin` clears the outbox, so collect
        // each tick's walk request ids instead of counting the final outbox.
        let mut walk_requests = std::collections::BTreeSet::new();
        let mut note_walks = |ledger: &Option<Box<crate::native::ledger::Ledger>>| {
            for action in &ledger.as_ref().unwrap().outbox {
                if matches!(&action.effect, HostEffect::Walk(_)) {
                    walk_requests.insert(action.request_id);
                }
            }
        };
        note_walks(&ledger);
        for tick in 3..=14 {
            if tick % 2 == 0 {
                snapshot.seed_npcs(vec![
                    cow(cow_id, 1, west, player),
                    cow(cow_id, 2, east, player),
                ]);
            } else {
                snapshot.seed_npcs(vec![
                    cow(cow_id, 1, east, player),
                    cow(cow_id, 2, west, player),
                ]);
            }
            assert!(
                with_tick_reach(&snapshot, &view, &mut ledger, tick, |tick| {
                    with_step(tick, |cx| run.poll(cx))
                })
                .is_pending()
            );
            note_walks(&ledger);
        }
        let walks = walk_requests.len();
        assert!(
            walks <= 7,
            "alternating reachability must stop after the 6 re-picks, not run to the settle, got {walks} distinct walks: {:?}",
            describe(&ledger)
        );
    });
}
