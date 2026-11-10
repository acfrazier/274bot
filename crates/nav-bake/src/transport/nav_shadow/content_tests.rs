use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use api::test_support::local_asset_path;
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
        assert_transfer_audit(&report, revision);
        assert_content_cases(&report, &content_root, revision);
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

fn assert_transfer_audit(report: &Value, revision: u16) {
    for entry in entries(report) {
        let candidates = entry["execution"]["candidates"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if candidates
            .iter()
            .any(|candidate| candidate["covered"] == true)
        {
            assert_eq!(
                entry["execution"]["transfer_assertions"]["all_hold"], true,
                "revision {revision}, entry {:?} has covered candidates",
                entry["handler_name"]
            );
        }
    }
}

fn assert_content_cases(report: &Value, content_root: &Path, revision: u16) {
    let names = crate::transport::loc_ids_by_name(content_root);
    let required_machine_only = [
        "chefdoor",
        "crafting_guild_door",
        "loc_2025",
        "mournerstewfence",
    ];
    for name in required_machine_only {
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
            let (crossing, stat, worn) = crossing_paths[0];
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
                    && site_names_npcchat(&effect["site"])
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
                entry["execution"]["transfer_audit"]["cancellations"]
                    .as_u64()
                    .unwrap_or_default()
                    > 0,
                "crafting guild pause has no cancellation transfer"
            );
            tested = true;
        }
    }
    assert!(
        tested,
        "revision {revision} crafting guild input path missing"
    );
}

fn site_names_npcchat(site: &Value) -> bool {
    ["script_name", "source"]
        .iter()
        .filter_map(|field| site[*field].as_str())
        .any(|value| value.to_ascii_lowercase().contains("npcchat"))
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
        for entry in group {
            assert!(
                entry["execution"]["transfer_audit"]["loop_summaries"]
                    .as_u64()
                    .unwrap_or_default()
                    > 0,
                "black-knight aggro loop was not summarized"
            );
            assert!(
                paths(entry).iter().any(|path| {
                    path_has_move(path)
                        && candidates_for_path(entry, path)
                            .iter()
                            .any(|candidate| candidate["covered"] == true)
                }),
                "black-knight stand has no covered crossing"
            );
        }
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

fn entries_with_script(report: &Value, wanted: &str) -> Vec<&Value> {
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
            } else if relation == "Lt" && predicate.contains(&format!("Ge Known({constant})")) {
                !truth
            } else if relation == "Ge" && predicate.contains(&format!("Lt Known({constant})")) {
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
                path["ticks"] == serde_json::json!([0, 2])
                    && candidates_for_path(entry, path)
                        .iter()
                        .any(|candidate| candidate["covered"] == true)
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
                    && path["conditions"].as_array().is_some_and(|conditions| {
                        conditions.len() == 1
                            && conditions.first().is_some_and(|condition| {
                                let predicate = condition["predicate"].as_str().unwrap_or_default();
                                let truth = condition["truth"].as_bool().unwrap_or_default();
                                predicate.contains("Varp")
                                    && predicate.contains("Known(0)")
                                    && ((predicate.contains("Gt") && truth)
                                        || (predicate.contains("Eq") && !truth))
                            })
                    })
            })
        }),
        "289 Ulby throne-room path does not have the single misc-quest gate"
    );
}

fn assert_climb_ladder(report: &Value, revision: u16) {
    assert!(
        entries(report).iter().any(|entry| {
            entry["handler_name"] == "climb_ladder"
                && entry["role"] == "procedure"
                && paths(entry).iter().any(|path| {
                    path["end"] == "Finished"
                        && path["effects"].as_array().is_some_and(|effects| {
                            effects.iter().any(|effect| {
                                effect["effect"]
                                    .as_str()
                                    .is_some_and(|text| text.starts_with("Delay {"))
                                    && effect["ticks"] == serde_json::json!([1, 1])
                            })
                        })
                })
        }),
        "revision {revision} climb_ladder lacks a finished one-tick path"
    );
}
