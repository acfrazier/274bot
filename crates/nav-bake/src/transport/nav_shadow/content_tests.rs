use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use api::test_support::local_asset_path;
use client::config::Cache;
use client::io::JagFile;
use rs2vm::machine::{
    Effect, End, Entry, Limits, Machine, MoveKind, PlayerPos, Value as VmValue, View,
};
use rs2vm::world::{Tri, World};
use rs2vm::{Coord, Ticks};
use serde_json::Value;

fn local_asset(revision: u16, kind: &str) -> Option<PathBuf> {
    let key = format!("RS2VM_{kind}_{revision}");
    local_asset_path(
        &key,
        "",
        &format!("revision {revision} {kind} for nav-shadow content tests"),
    )
}

fn bake(
    revision: u16,
    engine_dir: &Path,
    content_root: &Path,
    nav_shadow: bool,
) -> crate::bake::BakedNav {
    let inputs = crate::bake::content_inputs(content_root);
    let config_jag = crate::bake::config_jag_for(revision, &engine_dir.join("data/pack/client"))
        .expect("known revision config archive path");
    let game_data_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../api/data/game-data");
    crate::bake::bake_world(&crate::bake::BakeRequest {
        revision: Some(revision),
        game_data_dir: &game_data_dir,
        maps_dir: &inputs.maps_dir,
        doors_dir: &inputs.doors_dir,
        gates: &inputs.gates,
        config_jag: &config_jag,
        engine_dir,
        nav_shadow,
        server_loc_dat_377: None,
        require_all_door_configs: true,
    })
    .unwrap_or_else(|error| panic!("revision {revision} bake failed: {error}"))
}

#[test]
fn local_pack_shadow_paths_match_content_and_leave_bake_outputs_unchanged() {
    for revision in [289, 274] {
        let Some(engine_dir) = local_asset(revision, "ENGINE") else {
            continue;
        };
        let Some(content_root) = local_asset(revision, "CONTENT") else {
            continue;
        };

        let baseline = bake(revision, &engine_dir, &content_root, false);
        let shadow = bake(revision, &engine_dir, &content_root, true);
        assert_eq!(baseline.pack, shadow.pack, "revision {revision} pack bytes");
        assert_eq!(baseline.flags, shadow.flags, "revision {revision} flags");
        assert_eq!(baseline.reach, shadow.reach, "revision {revision} reach");
        assert_eq!(
            baseline.canlight, shadow.canlight,
            "revision {revision} canlight"
        );
        assert_eq!(baseline.pois, shadow.pois, "revision {revision} pois");
        assert_eq!(
            baseline.summary, shadow.summary,
            "revision {revision} summary"
        );
        assert_eq!(baseline.notes, shadow.notes, "revision {revision} notes");
        assert!(baseline.nav_shadow.is_none());

        let report: Value = serde_json::from_str(
            shadow
                .nav_shadow
                .as_deref()
                .expect("shadow bake includes its report"),
        )
        .expect("shadow report JSON");
        assert_eq!(report["revision"].as_u64(), Some(u64::from(revision)));
        assert!(report["machine_entries"].as_u64().unwrap_or_default() > 0);
        assert!(report["stage_elapsed_ns"].as_u64().unwrap_or_default() > 0);
        assert_report_limits(&report);
        assert_no_removed_edges(&report, revision);
        assert_changed_fields_exact(&report, revision);
        assert_no_open_loc_id_changes(&report, revision);
        assert_transfer_audit(&report, revision);
        assert_content_cases(&report, &content_root, revision);
    }
}

#[test]
fn local_valid_coordinate_climb_helper_finishes() {
    for revision in [289, 274] {
        let Some(engine) = local_asset(revision, "ENGINE") else {
            continue;
        };
        let Some(content) = local_asset(revision, "CONTENT") else {
            continue;
        };
        let config = crate::bake::config_jag_for(revision, &engine.join("data/pack/client"))
            .expect("selected config archive");
        let jag = JagFile::new(std::fs::read(config).expect("real selected config bytes"));
        let varbits = jag.read("varbit.dat").expect("real selected varbits");
        let locs = Cache::unpack(&jag).locs;
        let shadow = crate::transport::load_nav_shadow(
            &engine,
            &content,
            &content.join("maps"),
            &locs,
            &varbits,
            revision,
        )
        .expect("real selected program and world");
        let script = shadow
            .program
            .scripts
            .values()
            .find(|script| script.name.eq_ignore_ascii_case("[proc,climb_ladder]"))
            .expect("real climb_ladder procedure");
        let target = shadow
            .program
            .packs
            .locs
            .keys()
            .flat_map(|ty| shadow.world.placements(ty.0))
            .map(|placed| {
                let tile = &placed.placement;
                Coord::new(tile.x, tile.level, tile.z)
            })
            .find(|tile| shadow.world.zone_alloc(*tile).on_free == Tri::Yes)
            .expect("an actual placed tile allocated in both world modes");
        for up in [0, 1] {
            let mut entry = Entry::new(script.id);
            entry.ints = vec![VmValue::Known(target.0), VmValue::Known(up)];
            entry.stand = Some(target);
            let run = Machine::new(&shadow.program, shadow.profile, &shadow.world)
                .run(entry, View::ALL, Limits::default())
                .expect("bounded real helper run");
            assert_eq!(run.paths.len(), 1, "known inputs have one execution");
            let path = &run.paths[0];
            assert_eq!(path.end, End::Finished);
            assert_eq!(path.ticks, Ticks { min: 1, max: 1 });
            assert_eq!(path.player, PlayerPos::Tile(target));
            assert!(path.effects.iter().any(|record| {
                matches!(
                    record.effect,
                    Effect::Move {
                        kind: MoveKind::Telejump,
                        to: PlayerPos::Tile(tile)
                    } if tile == target
                )
            }));
            assert!(path
                .effects
                .iter()
                .any(|record| { matches!(record.effect, Effect::Delay { min: 1, max: 1 }) }));
        }
    }
}

fn assert_report_limits(report: &Value) {
    for name in ["paths", "instructions", "bytes"] {
        let summary = &report["limits"][name];
        let maximum = summary["maximum"].as_u64().expect("bound maximum");
        let percentile_99 = summary["percentile_99"].as_u64().expect("bound percentile");
        let expected_limit = maximum
            .saturating_mul(4)
            .checked_next_power_of_two()
            .unwrap_or(u64::MAX);
        assert!(
            percentile_99 <= maximum,
            "{name} percentile exceeds maximum"
        );
        assert_eq!(
            summary["recommended_limit"].as_u64(),
            Some(expected_limit),
            "{name} recommendation follows measured maximum"
        );
        let largest = summary["ten_largest"].as_array().expect("largest entries");
        assert!(largest.len() <= 10);
        assert!(largest
            .windows(2)
            .all(|pair| { pair[0]["value"].as_u64() >= pair[1]["value"].as_u64() }));
    }
}

fn assert_no_removed_edges(report: &Value, revision: u16) {
    let diffs = report["diffs"].as_array().expect("edge diff list");
    assert!(
        diffs.iter().all(|diff| diff["status"] != "removed"),
        "revision {revision} shadow removed a base edge"
    );
}

fn assert_changed_fields_exact(report: &Value, revision: u16) {
    let diffs = report["diffs"].as_array().expect("edge diff list");
    for diff in diffs.iter().filter(|diff| diff["status"] == "changed") {
        let baseline = diff["baseline"].as_object().expect("changed edge baseline");
        let candidate = diff["candidate"]
            .as_object()
            .expect("changed edge candidate");
        let baseline_fields: BTreeSet<_> = baseline.keys().map(String::as_str).collect();
        let candidate_fields: BTreeSet<_> = candidate.keys().map(String::as_str).collect();
        assert_eq!(
            baseline_fields, candidate_fields,
            "revision {revision} changed edge field inventory"
        );
        let expected: BTreeSet<_> = baseline
            .iter()
            .filter_map(|(field, value)| {
                (candidate.get(field) != Some(value)).then_some(field.as_str())
            })
            .collect();
        let actual: Vec<_> = diff["changed_fields"]
            .as_array()
            .expect("changed edge field list")
            .iter()
            .map(|field| field.as_str().expect("changed field name"))
            .collect();
        let actual_fields: BTreeSet<_> = actual.iter().copied().collect();
        assert_eq!(
            actual.len(),
            actual_fields.len(),
            "revision {revision} changed edge lists a field more than once"
        );
        assert!(
            !expected.is_empty(),
            "revision {revision} labels an unchanged edge as changed"
        );
        assert_eq!(
            actual_fields, expected,
            "revision {revision} changed edge field list is incomplete or inaccurate"
        );
    }
}

fn assert_no_open_loc_id_changes(report: &Value, revision: u16) {
    let open_loc_mismatches = report["diffs"]
        .as_array()
        .expect("edge diff list")
        .iter()
        .filter(|diff| {
            let baseline = diff["baseline"]
                .as_object()
                .and_then(|edge| edge.get("open_loc_id"));
            let candidate = diff["candidate"]
                .as_object()
                .and_then(|edge| edge.get("open_loc_id"));
            matches!((baseline, candidate), (Some(before), Some(after)) if before != after)
        })
        .count();
    assert_eq!(
        open_loc_mismatches, 0,
        "revision {revision} has mismatched projected open loc types"
    );
}

fn assert_transfer_audit(report: &Value, revision: u16) {
    for entry in entries(report) {
        let path_results = paths(entry);
        if path_results.is_empty() {
            continue;
        }
        assert_eq!(
            entry["execution"]["transfer_assertions"]["all_hold"], true,
            "revision {revision}, entry {:?} has invalid transfer assertions",
            entry["handler_name"]
        );
        for path in path_results {
            let audit = &path["transfer_audit"];
            let cancellations = audit["cancellations"].as_u64().expect("path cancellations");
            let automatic_closes = audit["automatic_closes"]
                .as_u64()
                .expect("path automatic closes");
            let suspensions = audit["suspensions"].as_u64().expect("path suspensions");
            let loop_summaries = audit["loop_summaries"]
                .as_u64()
                .expect("path loop summaries");
            let loop_allowance = audit["loop_allowance_before_exit"]
                .as_u64()
                .expect("path loop allowance");
            let unsupported_prefixes = audit["unsupported_prefixes"]
                .as_u64()
                .expect("path unsupported prefixes");
            let end = path["end"].as_str().expect("path end");
            assert_eq!(
                unsupported_prefixes > 0,
                end.starts_with("Unsupported"),
                "revision {revision}, entry {:?}, path {} unsupported prefix audit",
                entry["handler_name"],
                path["index"]
            );
            assert_eq!(
                cancellations > 0,
                end == "AwaitingInput",
                "revision {revision}, entry {:?}, path {} cancellation audit",
                entry["handler_name"],
                path["index"]
            );
            assert!(
                automatic_closes <= suspensions,
                "revision {revision}, entry {:?}, path {} closes exceed suspensions",
                entry["handler_name"],
                path["index"]
            );
            assert!(
                loop_allowance >= loop_summaries,
                "revision {revision}, entry {:?}, path {} loop summary lacks allowance",
                entry["handler_name"],
                path["index"]
            );
        }
    }
}

fn assert_content_cases(report: &Value, content_root: &Path, revision: u16) {
    let names = crate::transport::loc_ids_by_name(content_root);
    for name in super::MACHINE_ONLY_SCRIPTED_DOOR_NAMES {
        let id = loc_id(&names, name).unwrap_or_else(|| panic!("{name} loc id is absent"));
        let placed = entries_for_loc(report, id);
        assert!(
            !placed.is_empty(),
            "revision {revision} has no {name} entry"
        );
        assert!(
            placed.iter().all(|entry| entry["stage_owned"] == false),
            "{name} is machine-only and is not cut over"
        );
        assert!(
            placed.iter().any(|entry| !paths(entry).is_empty()),
            "{name} has no machine path results"
        );
    }

    assert_chefdoor(
        report,
        loc_id(&names, "chefdoor").expect("chefdoor loc id"),
        revision,
    );
    assert_crafting_guild(
        report,
        loc_id(&names, "crafting_guild_door").expect("crafting guild loc id"),
        revision,
    );
    assert_mourner_fence(
        report,
        loc_id(&names, "mournerstewfence").expect("mourner fence loc id"),
        revision,
    );
    assert_bkfortress_door(
        report,
        loc_id(&names, "bkfortressdoor3").expect("black knight door loc id"),
        revision,
    );
    assert_double_door(report, revision);
    if revision == 289 {
        assert_ikov_open_type(
            report,
            loc_id(&names, "ikov_mendedleverdoorl").expect("289 Ikov door loc id"),
            revision,
        );
    }
    assert_ikov_active_open_leaf(
        report,
        loc_id(&names, "ikov_mendedleverdoorr").expect("Ikov right door loc id"),
        revision,
    );
    assert_watch_ladder(
        report,
        loc_id(&names, "watchladderup").expect("watch ladder loc id"),
        revision,
    );
    assert_legends_gate(
        report,
        loc_id(&names, "legendsguildgatel").expect("Legends' Guild gate loc id"),
        revision,
    );
    if revision == 289 {
        assert_misc_ulby(
            report,
            loc_id(&names, "misc_ulby_throneroomdoor").expect("289 Ulby door"),
        );
    }
    assert_climb_ladder(report, revision);
    assert_mcannon_stage_window(report, revision);
}

fn loc_id(names: &std::collections::HashMap<String, i32>, wanted: &str) -> Option<i32> {
    names
        .iter()
        .find_map(|(name, id)| name.eq_ignore_ascii_case(wanted).then_some(*id))
}

fn entries(report: &Value) -> &[Value] {
    report["entries"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn entries_for_loc(report: &Value, loc_type: i32) -> Vec<&Value> {
    entries(report)
        .iter()
        .filter(|entry| entry["loc_type"].as_i64() == Some(i64::from(loc_type)))
        .collect()
}

fn paired_groups<'a>(entries: impl IntoIterator<Item = &'a Value>) -> Vec<Vec<&'a Value>> {
    let mut by_placed_id = BTreeMap::<u64, Vec<&Value>>::new();
    for entry in entries {
        if let Some(placed_id) = entry["placed_id"].as_u64() {
            by_placed_id.entry(placed_id).or_default().push(entry);
        }
    }
    by_placed_id
        .into_values()
        .filter(|group| {
            group
                .iter()
                .filter_map(|entry| tile_key(&entry["stand"]))
                .collect::<BTreeSet<_>>()
                .len()
                >= 2
        })
        .collect()
}

fn tile_key(tile: &Value) -> Option<(i64, i64, i64)> {
    Some((
        tile["x"].as_i64()?,
        tile["z"].as_i64()?,
        tile["level"].as_i64()?,
    ))
}

fn paths(entry: &Value) -> &[Value] {
    entry["execution"]["path_results"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn candidates(entry: &Value) -> &[Value] {
    entry["execution"]["candidates"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn candidates_for_path<'a>(entry: &'a Value, path: &Value) -> Vec<&'a Value> {
    let Some(index) = path["index"].as_u64() else {
        return Vec::new();
    };
    candidates(entry)
        .iter()
        .filter(|candidate| candidate["path"].as_u64() == Some(index))
        .collect()
}

fn path_has_move(path: &Value) -> bool {
    path["effects"].as_array().is_some_and(|effects| {
        effects.iter().any(|effect| {
            effect["effect"]
                .as_str()
                .is_some_and(|description| description.starts_with("Move {"))
        })
    })
}

fn condition_signature(path: &Value, kind: &str, constant: i32) -> Option<(String, bool)> {
    let expected = format!("Known({constant})");
    path["conditions"].as_array()?.iter().find_map(|condition| {
        let predicate = condition["predicate"].as_str()?;
        if predicate.contains(kind) && predicate.contains(&expected) {
            Some((predicate.to_owned(), condition["truth"].as_bool()?))
        } else {
            None
        }
    })
}

fn has_condition_truth(path: &Value, predicate: &str, truth: bool) -> bool {
    path["conditions"].as_array().is_some_and(|conditions| {
        conditions.iter().any(|condition| {
            condition["predicate"].as_str() == Some(predicate)
                && condition["truth"].as_bool() == Some(truth)
        })
    })
}

fn assert_chefdoor(report: &Value, loc_type: i32, revision: u16) {
    let groups = paired_groups(entries_for_loc(report, loc_type));
    assert!(
        !groups.is_empty(),
        "revision {revision} chefdoor has no paired stands"
    );
    let mut door_stand_finished = false;
    let mut far_stand_crossed = false;
    let mut stat_refusal = false;
    let mut worn_refusal = false;

    for group in groups {
        let door = group
            .iter()
            .copied()
            .find(|entry| tile_key(&entry["placement"]) == tile_key(&entry["stand"]));
        let far = group
            .iter()
            .copied()
            .find(|entry| tile_key(&entry["placement"]) != tile_key(&entry["stand"]));
        if let Some(entry) = door {
            let finished: Vec<_> = paths(entry)
                .iter()
                .filter(|path| {
                    path["end"] == "Finished"
                        && path["conditions"].as_array().is_some_and(Vec::is_empty)
                })
                .collect();
            if !finished.is_empty() {
                assert_eq!(finished.len(), 1, "door-tile chefdoor path count");
                door_stand_finished = true;
            }
        }
        if let Some(entry) = far {
            let crossing_paths: Vec<_> = paths(entry)
                .iter()
                .filter_map(|path| {
                    let stat = condition_signature(path, "Stat", 32)?;
                    let worn = condition_signature(path, "InvTotal", 1)?;
                    (path_has_move(path)).then_some((path, stat, worn))
                })
                .collect();
            if crossing_paths.is_empty() {
                continue;
            }
            assert_eq!(crossing_paths.len(), 1, "far-side chefdoor crossing count");
            let (crossing, stat, worn) = &crossing_paths[0];
            let has_stat_refusal = paths(entry)
                .iter()
                .any(|path| !path_has_move(path) && has_condition_truth(path, &stat.0, !stat.1));
            let has_worn_refusal = paths(entry)
                .iter()
                .any(|path| !path_has_move(path) && has_condition_truth(path, &worn.0, !worn.1));
            assert!(
                has_stat_refusal,
                "chefdoor lacks the cooking-threshold refusal"
            );
            assert!(has_worn_refusal, "chefdoor lacks the worn-hat refusal");
            let door_landing = door.and_then(|entry| tile_key(&entry["stand"]));
            assert!(
                candidates_for_path(entry, crossing)
                    .iter()
                    .any(|candidate| tile_key(&candidate["landing"]) == door_landing),
                "chefdoor crossing has no path to the other stand"
            );
            far_stand_crossed = true;
            stat_refusal = true;
            worn_refusal = true;
        }
    }

    assert!(
        door_stand_finished,
        "revision {revision} chefdoor door-tile path missing"
    );
    assert!(
        far_stand_crossed,
        "revision {revision} chefdoor far-tile crossing missing"
    );
    assert!(
        stat_refusal,
        "revision {revision} chefdoor stat refusal missing"
    );
    assert!(
        worn_refusal,
        "revision {revision} chefdoor worn refusal missing"
    );
}

fn assert_crafting_guild(report: &Value, loc_type: i32, revision: u16) {
    let mut tested = false;
    for entry in entries_for_loc(report, loc_type) {
        for path in paths(entry)
            .iter()
            .filter(|path| path["end"] == "AwaitingInput" && path_has_move(path))
        {
            let effects = path["effects"].as_array().map(Vec::as_slice).unwrap_or(&[]);
            let Some(open_index) = effects.iter().position(|effect| {
                effect["effect"]
                    .as_str()
                    .is_some_and(|text| text.contains("chat: Some("))
                    && site_names_npc_chat(&effect["site"])
            }) else {
                continue;
            };
            assert!(
                effects[open_index + 1..].iter().any(|effect| {
                    effect["effect"]
                        .as_str()
                        .is_some_and(|text| text.contains("chat: None"))
                }),
                "cancel transition leaves npcchat open"
            );
            assert!(
                path["modal"]
                    .as_str()
                    .is_some_and(|modal| modal.contains("chat: None")),
                "input pause path ends with chat still open"
            );
            assert!(
                path["transfer_audit"]["cancellations"]
                    .as_u64()
                    .unwrap_or_default()
                    > 0,
                "crafting guild pause has no path cancellation transfer"
            );
            tested = true;
        }
    }
    assert!(
        tested,
        "revision {revision} crafting guild input path missing"
    );
}

fn site_names_npc_chat(site: &Value) -> bool {
    ["script_name", "source"]
        .iter()
        .filter_map(|field| site[*field].as_str())
        .any(|value| value.to_ascii_lowercase().contains("chatnpc"))
}

fn assert_mourner_fence(report: &Value, loc_type: i32, revision: u16) {
    let groups = paired_groups(entries_for_loc(report, loc_type));
    assert!(
        !groups.is_empty(),
        "revision {revision} mourner fence has no paired stands"
    );
    for group in groups {
        for entry in &group {
            let stand = tile_key(&entry["stand"]).expect("wall stand tile");
            let other_stand = group
                .iter()
                .find(|other| tile_key(&other["stand"]).is_some_and(|tile| tile != stand))
                .and_then(|other| other["stand"].as_object())
                .expect("opposite stand")
                .clone();
            let paths_for_stand: Vec<_> = paths(entry)
                .iter()
                .filter(|path| {
                    path["end"] == "Finished"
                        && path["conditions"].as_array().is_some_and(Vec::is_empty)
                        && path["ticks"] == serde_json::json!([4, 4])
                        && path_has_move(path)
                })
                .collect();
            assert_eq!(paths_for_stand.len(), 1, "mourner path per stand");
            assert!(
                candidates_for_path(entry, paths_for_stand[0])
                    .iter()
                    .any(|candidate| candidate["landing"] == Value::Object(other_stand.clone())),
                "mourner path does not land on the opposite stand"
            );
        }
    }
}

fn assert_bkfortress_door(report: &Value, loc_type: i32, revision: u16) {
    let groups = paired_groups(entries_for_loc(report, loc_type));
    assert!(
        !groups.is_empty(),
        "revision {revision} black-knight door has no paired stands"
    );
    for group in groups {
        for entry in &group {
            assert!(
                paths(entry).iter().any(|path| {
                    path_has_move(path)
                        && candidates_for_path(entry, path)
                            .iter()
                            .any(|candidate| candidate["covered"] == true)
                }),
                "black-knight door stand has no covered crossing"
            );
        }
        let summarized_stands = group
            .iter()
            .filter(|entry| {
                paths(entry).iter().any(|path| {
                    path["transfer_audit"]["loop_summaries"]
                        .as_u64()
                        .unwrap_or_default()
                        > 0
                        && path_has_move(path)
                        && candidates_for_path(entry, path)
                            .iter()
                            .any(|candidate| candidate["covered"] == true)
                })
            })
            .count();
        assert_eq!(
            summarized_stands, 1,
            "only the inside branch runs the summarized black-knight aggro loop"
        );
    }
}

fn site_has_script(site: &Value, needle: &str) -> bool {
    site["script_name"]
        .as_str()
        .is_some_and(|name| name.to_ascii_lowercase().contains(needle))
}

fn path_has_script(path: &Value, needle: &str) -> bool {
    site_has_script(&path["at"], needle)
        || path["chain"]
            .as_array()
            .is_some_and(|sites| sites.iter().any(|site| site_has_script(site, needle)))
}

fn entries_with_script<'a>(report: &'a Value, wanted: &str) -> Vec<&'a Value> {
    let needle = wanted.to_ascii_lowercase();
    entries(report)
        .iter()
        .filter(|entry| {
            entry["handler_name"]
                .as_str()
                .is_some_and(|name| name.to_ascii_lowercase().contains(&needle))
                || paths(entry)
                    .iter()
                    .any(|path| path_has_script(path, &needle))
        })
        .collect()
}

fn assert_double_door(report: &Value, revision: u16) {
    let groups = paired_groups(entries_with_script(
        report,
        "_double_door_open_and_close_left",
    ));
    assert!(
        !groups.is_empty(),
        "revision {revision} double-door helper has no paired stands"
    );
    for group in groups {
        for entry in group.iter().copied() {
            let covered = paths(entry).iter().any(|path| {
                path_has_move(path)
                    && candidates_for_path(entry, path)
                        .iter()
                        .any(|candidate| candidate["covered"] == true)
            });
            assert!(covered, "double-door stand has no covered crossing");
        }
        let active_id = group[0]["placed_id"]
            .as_u64()
            .expect("double-door placement id");
        assert!(
            group.iter().any(|entry| {
                paths(entry).iter().any(|path| {
                    let effects = path["effects"].as_array().map(Vec::as_slice).unwrap_or(&[]);
                    let deleted_type = loc_type_for(effects, "Del", active_id);
                    let added_type = loc_type_for(effects, "Add", active_id);
                    let ids = placed_ids(path);
                    deleted_type.is_some()
                        && added_type.is_some()
                        && deleted_type != added_type
                        && ids.iter().any(|id| *id != active_id)
                })
            }),
            "double-door path does not restore the deleted placed loc and use the other placed leaf"
        );
    }
}

fn assert_ikov_open_type(report: &Value, loc_type: i32, revision: u16) {
    assert!(
        entries_for_loc(report, loc_type).iter().any(|entry| {
            entry["handler_name"]
                .as_str()
                .is_some_and(|name| name.contains("ikov_mendedleverdoorl"))
                && paths(entry).iter().any(|path| {
                    let covered_open_type =
                        candidates_for_path(entry, path).iter().any(|candidate| {
                            candidate["covered"] == true
                                && candidate["edge"]["open_loc_id"].as_i64() == Some(1_562)
                        });
                    let sourced_open_leaf = path["effects"].as_array().is_some_and(|effects| {
                        effects.iter().any(|effect| {
                            effect["effect"].as_str().is_some_and(|description| {
                                description.starts_with("Loc { kind: Add, obj: Created(")
                                    && description.contains("ty: LocTypeId(1562)")
                            }) && effect["site"]["source"].as_str().is_some_and(|source| {
                                source.contains("scripts/doors/scripts/doubledoors.rs2:135")
                            }) && effect["site"]["pc"].as_u64() == Some(34)
                        })
                    });
                    covered_open_type && sourced_open_leaf
                })
        }),
        "revision {revision} Ikov opened door lacks the sourced leaf type 1562"
    );
}

fn assert_ikov_active_open_leaf(report: &Value, loc_type: i32, revision: u16) {
    let entry = entries_for_loc(report, loc_type)
        .into_iter()
        .find(|entry| {
            entry["placement"] == serde_json::json!({"x": 2661, "z": 9803, "level": 0})
                && entry["handler_name"].as_str() == Some("[oploc1,ikov_mendedleverdoorr]")
                && entry["stand"] == serde_json::json!({"x": 2661, "z": 9802, "level": 0})
        })
        .unwrap_or_else(|| panic!("revision {revision} Ikov right-door stand is absent"));
    let (path, candidate) = paths(entry)
        .iter()
        .find_map(|path| {
            candidates_for_path(entry, path)
                .into_iter()
                .find(|candidate| candidate["covered"] == true)
                .map(|candidate| (path, candidate))
        })
        .unwrap_or_else(|| panic!("revision {revision} Ikov right-door path is not covered"));
    assert_eq!(
        candidate["edge"]["open_loc_id"].as_i64(),
        Some(1_563),
        "revision {revision} active door leaf lost its open loc type"
    );
    let effects = path["effects"].as_array().expect("Ikov path effects");
    for (loc_type, tile) in [
        (1_563, Coord::new(2_661, 0, 9_802)),
        (1_562, Coord::new(2_662, 0, 9_802)),
    ] {
        let expected_type = format!("ty: LocTypeId({loc_type})");
        let expected_tile = format!("tile: {tile:?}");
        assert!(
            effects.iter().any(|effect| {
                effect["effect"].as_str().is_some_and(|description| {
                    description.starts_with("Loc { kind: Add, obj: Created(")
                        && description.contains(&expected_type)
                        && description.contains(&expected_tile)
                })
            }),
            "revision {revision} Ikov path lacks type {loc_type} at {tile:?}"
        );
    }
}

fn loc_type_for(effects: &[Value], kind: &str, placed_id: u64) -> Option<String> {
    effects.iter().find_map(|effect| {
        let description = effect["effect"].as_str()?;
        let expected = format!("Loc {{ kind: {kind}, obj: Placed(PlacedId({placed_id})),");
        if !description.starts_with(&expected) {
            return None;
        }
        let value = description.split("ty: LocTypeId(").nth(1)?;
        Some(value.split(')').next()?.to_owned())
    })
}

fn placed_ids(path: &Value) -> BTreeSet<u64> {
    let mut ids = BTreeSet::new();
    for effect in path["effects"].as_array().into_iter().flatten() {
        let Some(description) = effect["effect"].as_str() else {
            continue;
        };
        let mut rest = description;
        while let Some(index) = rest.find("PlacedId(") {
            rest = &rest[index + "PlacedId(".len()..];
            let digits = rest
                .chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>();
            if let Ok(id) = digits.parse() {
                ids.insert(id);
            }
            rest = rest.get(digits.len()..).unwrap_or_default();
        }
    }
    ids
}

fn assert_watch_ladder(report: &Value, loc_type: i32, revision: u16) {
    let low = serde_json::json!({"x": 2549, "z": 3112, "level": 2});
    let high = serde_json::json!({"x": 2933, "z": 4712, "level": 2});
    let mut low_candidate = false;
    let mut high_candidate = false;
    for entry in entries_for_loc(report, loc_type) {
        for path in paths(entry) {
            let relation_low = path_has_relation(path, 13, "Lt");
            let relation_high = path_has_relation(path, 13, "Ge");
            for candidate in candidates_for_path(entry, path) {
                if candidate["landing"] == low && relation_low {
                    low_candidate = true;
                }
                if candidate["landing"] == high && relation_high {
                    high_candidate = true;
                }
            }
        }
    }
    assert!(
        low_candidate,
        "revision {revision} watchtower low landing lacks itwatchtower < 13"
    );
    assert!(
        high_candidate,
        "revision {revision} watchtower high landing lacks itwatchtower >= 13"
    );
}

fn path_has_relation(path: &Value, constant: i32, relation: &str) -> bool {
    let expected = format!("{relation} Known({constant})");
    path["conditions"].as_array().is_some_and(|conditions| {
        conditions.iter().any(|condition| {
            let predicate = condition["predicate"].as_str().unwrap_or_default();
            let truth = condition["truth"].as_bool().unwrap_or(false);
            if predicate.contains(&expected) {
                truth
            } else if (relation == "Lt" && predicate.contains(&format!("Ge Known({constant})")))
                || (relation == "Ge" && predicate.contains(&format!("Lt Known({constant})")))
            {
                !truth
            } else {
                false
            }
        })
    })
}

fn assert_legends_gate(report: &Value, loc_type: i32, revision: u16) {
    assert!(
        entries_for_loc(report, loc_type).iter().any(|entry| {
            paths(entry).iter().any(|path| {
                candidates_for_path(entry, path).iter().any(|candidate| {
                    candidate["covered"] == true && candidate["ticks"] == serde_json::json!([0, 2])
                })
            })
        }),
        "revision {revision} Legends' Guild gate lacks a covered 0..2 tick path"
    );
}

fn assert_misc_ulby(report: &Value, loc_type: i32) {
    assert!(
        entries_for_loc(report, loc_type).iter().any(|entry| {
            paths(entry).iter().any(|path| {
                path_has_move(path)
                    && candidates_for_path(entry, path).iter().any(|candidate| {
                        candidate["covered"] == true
                            && candidate["conditions"]
                                .as_array()
                                .is_some_and(|conditions| {
                                    conditions.len() == 1
                                        && conditions.first().is_some_and(|condition| {
                                            let predicate =
                                                condition["predicate"].as_str().unwrap_or_default();
                                            let truth =
                                                condition["truth"].as_bool().unwrap_or_default();
                                            predicate.contains("Varp")
                                                && predicate.contains("Known(0)")
                                                && ((predicate.contains("Gt") && truth)
                                                    || (predicate.contains("Eq") && !truth))
                                        })
                                })
                    })
            })
        }),
        "289 Ulby throne-room path does not have the single misc-quest gate"
    );
}

fn assert_climb_ladder(report: &Value, revision: u16) {
    let entry = entries(report)
        .iter()
        .find(|entry| {
            entry["handler_name"] == "[proc,climb_ladder]" && entry["role"] == "procedure"
        })
        .unwrap_or_else(|| {
            panic!("revision {revision} did not resolve the climb_ladder procedure")
        });
    let source = entry["handler_source"]
        .as_str()
        .filter(|source| !source.is_empty())
        .unwrap_or_else(|| panic!("revision {revision} climb_ladder has no program source"));
    assert_ne!(
        entry["execution"]["status"], "script not found",
        "revision {revision} silently skipped the climb_ladder procedure"
    );
    let prefixes = paths(entry);
    assert_eq!(
        prefixes.len(),
        2,
        "both unknown direction arms remain explicit"
    );
    for path in prefixes {
        assert!(
            path["end"].as_str().is_some_and(|end| {
                end.starts_with("Unsupported(")
                    && end.contains("coordinate validation needs a proof")
            }),
            "revision {revision} unknown coordinate must fail closed"
        );
        assert!(
            path["at"]["source"]
                .as_str()
                .is_some_and(|site| site.starts_with(source)),
            "unknown-coordinate stop has no real source"
        );
        assert_eq!(path["ticks"], serde_json::json!([1, 1]));
        assert!(!path_has_move(path), "unproved coordinate must not move");
        assert!(
            path["effects"].as_array().is_some_and(|effects| {
                effects.iter().any(|effect| {
                    effect["effect"]
                        .as_str()
                        .is_some_and(|text| text.starts_with("Delay {"))
                        && effect["ticks"] == serde_json::json!([1, 1])
                })
            }),
            "revision {revision} unknown helper lacks its one-tick prefix"
        );
    }
}

fn assert_mcannon_stage_window(report: &Value, revision: u16) {
    let candidate = entries(report).iter().find_map(|entry| {
        paths(entry).iter().find_map(|path| {
            let has_mcannon_condition = path["conditions"].as_array().is_some_and(|conditions| {
                conditions.iter().any(|condition| {
                    condition["site"]["source"]
                        .as_str()
                        .is_some_and(|source| source.contains("mcannon_ladders.rs2"))
                })
            });
            if !has_mcannon_condition {
                return None;
            }
            candidates_for_path(entry, path)
                .into_iter()
                .find(|candidate| {
                    candidate["covered"] == true
                        && candidate["edge"]["quest_gates"]
                            .as_str()
                            .is_some_and(|gates| {
                                gates.contains("FactKey(\"mcannon\")")
                                    && gates
                                        .contains("InclusiveRange { min: Some(2), max: Some(11) }")
                            })
                })
        })
    });
    assert!(
        candidate.is_some(),
        "revision {revision} McCannon ladder path lacks the narrowed stage window 2..11"
    );
}
