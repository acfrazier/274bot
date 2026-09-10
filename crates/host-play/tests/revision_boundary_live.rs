//! Controlled protocol proof using the production shared constructor, Rust
//! interaction driver and snapshot. This never starts Play slots or guardians.
//! Run one revision per process with LIVE=1 and BOUNDARY_REVISION=274|289.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use api::interact::{self, ActionSpec, Interactions, OpTarget, SendResult};
use api::snapshot::{GameSnapshot, WorldTile};
use client::client::Client;
use host::Pump;
use host_play::{ProfileOptions, SharedClientTemplate};
use serde_json::{json, Value};

fn observation(snapshot: &GameSnapshot) -> Value {
    json!({
        "ingame": snapshot.ingame(), "attached": snapshot.attached(),
        "scene_state": snapshot.scene_state(), "tile": snapshot.tile(),
        "tick": snapshot.tick(), "npc_count": snapshot.npcs().len(),
        "loc_count": snapshot.locs().len(), "inventory": snapshot.inv(),
        "modals": snapshot.modals(), "chat_texts": snapshot.chat_modal_texts(),
    })
}

fn record(phase: &str, snapshot: &GameSnapshot, detail: Value) {
    println!(
        "{}",
        json!({"phase": phase, "state": observation(snapshot), "detail": detail})
    );
}

fn wait_for(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    phase: &str,
    timeout: Duration,
    mut ready: impl FnMut(&GameSnapshot) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    loop {
        let start = Instant::now();
        client.mainloop();
        host::publish_snapshot(snapshot, client, pump.drain_client(client));
        if ready(snapshot) {
            return Ok(());
        }
        if client.error_loading || Instant::now() >= deadline {
            record(
                phase,
                snapshot,
                json!({"outcome": "FAIL", "timeout_seconds": timeout.as_secs()}),
            );
            return Err(format!("{phase}: readiness predicate failed"));
        }
        if let Some(rest) = Duration::from_millis(20).checked_sub(start.elapsed()) {
            std::thread::sleep(rest);
        }
    }
}

fn accepted(result: SendResult<'_>, phase: &str) -> Result<(), String> {
    match result {
        SendResult::Sent { .. } => Ok(()),
        SendResult::Refused { reason, .. } => Err(format!("{phase}: refused {reason:?}")),
    }
}

fn login_scene(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    username: &str,
) -> Result<(), String> {
    if !interact::login(client, username, "test", false) {
        return Err(format!(
            "login failed: {} {}",
            client.login_mes1, client.login_mes2
        ));
    }
    wait_for(
        client,
        snapshot,
        pump,
        "login-scene2",
        Duration::from_secs(90),
        |s| s.ingame() && s.attached() && s.scene_state() == 2 && s.tile().is_some(),
    )
}

fn logout(client: &mut Client, snapshot: &mut GameSnapshot, pump: &mut Pump) -> Result<(), String> {
    let ifaces = Arc::clone(&client.ifaces);
    if !interact::logout(client, &ifaces) {
        return Err("logout: selected cache has no logout control".into());
    }
    wait_for(
        client,
        snapshot,
        pump,
        "logout",
        Duration::from_secs(30),
        |s| !s.ingame() && !s.attached(),
    )?;
    if !snapshot.npcs().is_empty()
        || !snapshot.players().is_empty()
        || snapshot.local_player().is_some()
        || !snapshot.inventory().is_empty()
        || !snapshot.bank().is_empty()
    {
        return Err("logout retained live actors or inventory/bank state".into());
    }
    Ok(())
}

fn walk_leg(
    client: &mut Client,
    snapshot: &mut GameSnapshot,
    pump: &mut Pump,
    kind: &str,
    leg: usize,
    target: WorldTile,
) -> Result<(), String> {
    let before = snapshot.tile().ok_or("missing walk origin")?;
    if before == (target.x, target.z, target.level) {
        record(
            "walk-already-at-waypoint",
            snapshot,
            json!({"kind": kind, "leg": leg, "target": target}),
        );
        return Ok(());
    }
    record(
        "walk-request",
        snapshot,
        json!({"kind": kind, "leg": leg, "before": before, "target": target}),
    );
    accepted(Interactions::new(snapshot, client).walk(target), kind)?;
    wait_for(client, snapshot, pump, kind, Duration::from_secs(20), |s| {
        s.tile() == Some((target.x, target.z, target.level)) && s.tile() != Some(before)
    })?;
    record(
        "walk-applied",
        snapshot,
        json!({"kind": kind, "leg": leg, "before": before, "target": target}),
    );
    Ok(())
}

fn run() -> Result<(), String> {
    let revision =
        std::env::var("BOUNDARY_REVISION").map_err(|_| "BOUNDARY_REVISION=274|289 is required")?;
    if revision != "274" && revision != "289" {
        return Err("BOUNDARY_REVISION must be 274 or 289".into());
    }
    let options = ProfileOptions {
        profile: Some(format!("local-{revision}")),
        engine_dir: std::env::var_os("BOUNDARY_ENGINE_DIR").map(PathBuf::from),
        ..ProfileOptions::default()
    };
    let profile = options.resolve(None)?.bind()?;
    if profile.client().target() != client::BotTarget::Local
        || profile.client().game_host() != "127.0.0.1"
        || profile.client().asset_host() != "127.0.0.1"
    {
        return Err("boundary proof requires the isolated loopback local profile".into());
    }
    let template = SharedClientTemplate::load(Arc::clone(&profile))?;
    let serial = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let username = format!("h{revision}{:08}", serial % 100_000_000);
    let mut client = template.prepare_client((serial % 1_000_000_000) as i32, false)?;
    client.draw = false;
    client.maininit();
    if client.error_loading {
        return Err(format!(
            "asset initialization failed: {}",
            client.last_progress_message
        ));
    }
    let mut snapshot = GameSnapshot::new();
    let mut pump = Pump::new();
    println!(
        "{}",
        json!({
            "phase": "identity", "profile": profile.label(), "revision": client.revision().as_i32(),
            "cache_id": profile.cache_id(), "username": username,
            "shared_binding": Arc::ptr_eq(client.session_profile().ok_or("missing binding")?, profile.client()),
            "production_operation_gate": profile.require_bot_operation().err(),
            "navigation_used": false, "play_slots_started": false,
        })
    );

    let result = (|| {
        login_scene(&mut client, &mut snapshot, &mut pump, &username)?;
        record("initial-scene2", &snapshot, json!({}));

        // Fixture preparation only. Establish all action baselines after this
        // acknowledgement and the clean relog that releases tutorial controls.
        interact::seed_at(&mut client, 0, 3220, 3212);
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "mainland-seed",
            Duration::from_secs(30),
            |s| s.ingame() && s.scene_state() == 2 && s.tile() == Some((3220, 3212, 0)),
        )?;
        logout(&mut client, &mut snapshot, &mut pump)?;
        login_scene(&mut client, &mut snapshot, &mut pump, &username)?;
        if snapshot.tile() != Some((3220, 3212, 0)) {
            return Err("relog did not retain acknowledged mainland seed".into());
        }
        record("baseline-after-preparation", &snapshot, json!({}));

        // Walk around the courtyard and castle perimeter. The natural Hans
        // patrol extends west/north of the initial view. Each leg must land
        // before the next is sent; finding him never substitutes for movement.
        let mut observed_npc = None;
        for (leg, (x, z)) in [
            (3221, 3212),
            (3221, 3222),
            (3219, 3230),
            (3207, 3233),
            (3202, 3220),
            (3202, 3205),
            (3214, 3205),
            (3220, 3212),
        ]
        .into_iter()
        .enumerate()
        {
            let target = WorldTile { x, z, level: 0 };
            walk_leg(
                &mut client,
                &mut snapshot,
                &mut pump,
                "courtyard walk",
                leg,
                target,
            )?;
            record(
                "courtyard-npc-observation",
                &snapshot,
                json!({"npcs": snapshot.npcs().iter().map(|npc| json!({
                    "index": npc.index, "name": npc.name, "actions": npc.actions,
                    "tile": npc.tile, "distance": npc.distance,
                })).collect::<Vec<_>>()}),
            );
            if leg >= 2 {
                observed_npc = snapshot
                    .npcs()
                    .iter()
                    .find(|npc| {
                        npc.name.as_deref() == Some("Hans")
                            && npc
                                .actions
                                .iter()
                                .flatten()
                                .any(|op| op.eq_ignore_ascii_case("Talk-to"))
                    })
                    .map(|npc| npc.index);
                if observed_npc.is_some() {
                    break;
                }
            }
        }
        let observed_npc = observed_npc.ok_or("natural Hans absent after courtyard route")?;

        let npc = snapshot
            .npcs()
            .iter()
            .find(|npc| {
                npc.index == observed_npc
                    && npc.name.as_deref() == Some("Hans")
                    && npc
                        .actions
                        .iter()
                        .flatten()
                        .any(|action| action.eq_ignore_ascii_case("Talk-to"))
            })
            .cloned()
            .ok_or("fixture missing visible Hans with Talk-to")?;
        if snapshot.modals().chat != -1 || !snapshot.chat_modal_texts().is_empty() {
            return Err("NPC baseline already has a dialogue".into());
        }
        record("npc-request", &snapshot, json!({"npc": npc}));
        accepted(
            Interactions::new(&snapshot, &mut client)
                .interact(OpTarget::Npc(&npc), ActionSpec::Label("Talk-to".into())),
            "NPC Talk-to",
        )?;
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "NPC Talk-to",
            Duration::from_secs(30),
            |s| s.modals().chat != -1 && !s.chat_modal_texts().is_empty(),
        )?;
        record("npc-dialogue-applied", &snapshot, json!({"npc": npc}));
        accepted(
            Interactions::new(&snapshot, &mut client).close_modal(),
            "close-dialogue",
        )?;
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "close-dialogue",
            Duration::from_secs(10),
            |s| s.modals().chat == -1,
        )?;

        // The nearest door by distance can be inside the castle while Hans
        // and the player are outside its wall. Walk back along the perimeter
        // to the exterior house door that the original 274 cell exercised.
        let (x, z, _) = snapshot.tile().ok_or("missing door approach origin")?;
        let approach: &[(i32, i32)] = if x >= 3218 {
            &[(3225, 3214)]
        } else if z >= 3219 {
            &[(3207, 3233), (3219, 3230), (3221, 3222), (3225, 3214)]
        } else {
            &[(3202, 3205), (3214, 3205), (3220, 3212), (3225, 3214)]
        };
        for (leg, &(x, z)) in approach.iter().enumerate() {
            walk_leg(
                &mut client,
                &mut snapshot,
                &mut pump,
                "door approach",
                leg,
                WorldTile { x, z, level: 0 },
            )?;
        }
        let door_with = |s: &GameSnapshot, action: &str| {
            s.locs()
                .iter()
                .find(|loc| {
                    loc.name.as_deref() == Some("Door")
                        && loc.tile.level == 0
                        && loc.tile.z == 3214
                        && (3226..=3227).contains(&loc.tile.x)
                        && loc
                            .actions
                            .iter()
                            .flatten()
                            .any(|op| op.eq_ignore_ascii_case(action))
                })
                .cloned()
        };
        if door_with(&snapshot, "Open").is_none() {
            let open = door_with(&snapshot, "Close").ok_or("exterior fixture door absent")?;
            record("door-close-request", &snapshot, json!({"loc": open}));
            accepted(
                Interactions::new(&snapshot, &mut client)
                    .interact(OpTarget::Loc(&open), ActionSpec::Label("Close".into())),
                "loc Close",
            )?;
            wait_for(
                &mut client,
                &mut snapshot,
                &mut pump,
                "loc Close",
                Duration::from_secs(30),
                |s| {
                    door_with(s, "Open").is_some()
                        && !s.locs().iter().any(|current| {
                            current.tile == open.tile
                                && current.layer == open.layer
                                && current.typecode == open.typecode
                        })
                },
            )?;
            record("door-close-applied", &snapshot, json!({"before_loc": open}));
        }
        let loc = snapshot
            .locs()
            .iter()
            .filter(|loc| {
                loc.name.as_deref() == Some("Door")
                    && loc.tile.level == 0
                    && loc.tile.z == 3214
                    && (3226..=3227).contains(&loc.tile.x)
                    && loc
                        .actions
                        .iter()
                        .flatten()
                        .any(|action| action.eq_ignore_ascii_case("Open"))
            })
            .min_by_key(|loc| loc.distance)
            .cloned()
            .ok_or("fixture missing a nearby closed Door")?;
        record("loc-request", &snapshot, json!({"loc": loc}));
        let before_out = client.out.pos;
        accepted(
            Interactions::new(&snapshot, &mut client)
                .interact(OpTarget::Loc(&loc), ActionSpec::Label("Open".into())),
            "loc Open",
        )?;
        record(
            "loc-dispatched",
            &snapshot,
            json!({
                "outbound_bytes_added": client.out.pos - before_out,
                "path": client.try_move_path,
                "map_flag": [client.minimap_flag_x, client.minimap_flag_z],
            }),
        );
        wait_for(
            &mut client,
            &mut snapshot,
            &mut pump,
            "loc Open",
            Duration::from_secs(30),
            |s| {
                s.ingame()
                    && s.scene_state() == 2
                    && s.tile().is_some_and(|(x, z, level)| {
                        level == loc.tile.level
                            && x.abs_diff(loc.tile.x) <= 2
                            && z.abs_diff(loc.tile.z) <= 2
                    })
                    && !s.locs().iter().any(|current| {
                        current.tile == loc.tile
                            && current.layer == loc.layer
                            && current.typecode == loc.typecode
                    })
                    && s.locs().iter().any(|current| {
                        current.name.as_deref() == Some("Door")
                            && current.tile.level == loc.tile.level
                            && current.tile.x.abs_diff(loc.tile.x) <= 1
                            && current.tile.z.abs_diff(loc.tile.z) <= 1
                            && current
                                .actions
                                .iter()
                                .flatten()
                                .any(|action| action.eq_ignore_ascii_case("Close"))
                    })
            },
        )?;
        record(
            "loc-change-applied",
            &snapshot,
            json!({
                "before_loc": loc,
                "after_locs": snapshot.locs().iter().filter(|current| {
                    current.tile.level == loc.tile.level && current.tile.x.abs_diff(loc.tile.x) <= 1
                        && current.tile.z.abs_diff(loc.tile.z) <= 1
                }).collect::<Vec<_>>(),
            }),
        );
        logout(&mut client, &mut snapshot, &mut pump)?;
        record(
            "logout-reset-applied",
            &snapshot,
            json!({"outcome": "PASS"}),
        );
        Ok(())
    })();
    if client.ingame {
        // Failure cleanup is local teardown; it never satisfies the logout proof.
        client.logout();
    }
    result
}

#[test]
#[ignore = "requires LIVE=1, BOUNDARY_REVISION and its isolated local engine"]
fn revision_boundary_live() {
    if std::env::var("LIVE").as_deref() != Ok("1") {
        return;
    }
    if let Err(error) = run() {
        eprintln!("FAIL: revision_boundary_live: {error}");
        std::process::exit(1);
    }
}
