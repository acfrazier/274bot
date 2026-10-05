use super::*;

fn capture(value: &Value) -> CombatCapture {
    let rows = |key: &str| value[key].as_array().cloned().unwrap_or_default();
    let mut capture = CombatCapture::default();
    capture.frames = rows("frames");
    capture.actions = rows("actions");
    capture.statuses = rows("statuses");
    capture.random_events = rows("random_events");
    capture.observations = rows("observations");
    capture.prayer_facts = rows("prayer_facts");
    capture.start_baseline =
        (!value["start_baseline"].is_null()).then(|| value["start_baseline"].clone());
    capture.started = value["started"] == json!(true);
    capture.magic_setup = (!value["magic_setup"].is_null()).then(|| value["magic_setup"].clone());
    capture.magic_npc_events = value["magic"]["raw_npc_mask_and_landing_events"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    capture.maze_injected = capture
        .random_events
        .iter()
        .any(|event| event["kind"] == json!("Maze") && event["error"].is_null());
    capture.maze_owner_live_before = value["maze_attack_owner_live_before"].as_bool();
    capture.maze_owner_live_after = value["maze_attack_owner_live_after"].as_bool();
    capture
}

fn set_prayer_varp_off_through(value: &mut Value, varp: i64, through_tick: Option<i64>) {
    for frame in value["frames"].as_array_mut().unwrap() {
        if through_tick.is_some_and(|end| frame["tick"].as_i64().is_none_or(|tick| tick > end)) {
            continue;
        }
        for row in frame["prayer_varps"].as_array_mut().unwrap() {
            if row["index"] == json!(varp) {
                row["value"] = json!(0);
            }
        }
    }
}

fn replay_verdict(
    case: Case,
    receipt: &Value,
    capture: &CombatCapture,
) -> (&'static str, bool, Option<String>, Option<String>) {
    let preflight = capture
        .start_baseline
        .as_ref()
        .and_then(|baseline| start_preflight(case, baseline));
    let invalid = case_invalid_reason(case, capture);
    let ready = case_ready(case, capture);
    let verdict = if capture_has_death(capture) {
        "FAIL"
    } else if preflight.is_some() || invalid.is_some() || receipt["invalid_reason"].is_string() {
        "INVALID"
    } else if ready {
        "PASS"
    } else {
        "FAIL"
    };
    (verdict, ready, preflight, invalid)
}

fn retain_mutant(
    output: &std::path::Path,
    source: &std::path::Path,
    variant: &str,
    mutant: &Value,
) -> std::path::PathBuf {
    let root = output.parent().unwrap().join("mutations");
    std::fs::create_dir_all(&root).unwrap();
    let mutant_path = root.join(format!(
        "{}.{variant}.json",
        source.file_name().unwrap().to_string_lossy()
    ));
    std::fs::write(&mutant_path, serde_json::to_vec_pretty(mutant).unwrap()).unwrap();
    mutant_path
}

fn magic_mutant_row(
    source: &std::path::Path,
    mutant_path: std::path::PathBuf,
    original: &Value,
    mutant: &Value,
    case: Case,
) -> Value {
    let full_capture = capture(mutant);
    let full_tail_verdict = replay_verdict(case, mutant, &full_capture).0;
    let (ready_tick, capture) = causal_magic_capture(mutant, case);
    let (new_verdict, ready, preflight, invalid) = replay_verdict(case, mutant, &capture);
    let runes = magic_rune_evidence(&capture);
    json!({
        "receipt": source,
        "mutant_receipt": mutant_path,
        "old_verdict": original["outcome"],
        "full_tail_verdict": full_tail_verdict,
        "earliest_ready_tick": ready_tick,
        "new_verdict": new_verdict,
        "ready": ready,
        "preflight": preflight,
        "current_invalid": invalid,
        "protect_timing": protection_timing_receipt(&capture),
        "no_melee_offensive_prayers": no_melee_offensive_prayers(&capture),
        "protect_restoring_terminal": protect_plan_ends_with_terminal(&capture),
        "magic_protection_timing_ok": magic_protection_timing_ok(&capture),
        "corpse_for_every_killed": every_killed_report_has_corpse(&capture),
        "prayer_off_after_corpse": report_with_end(&capture, magic_expected_end(case))
            .is_some_and(|report| prayer_off_plan_after_corpse(&capture, report)),
        "input_after_report": report_with_end(&capture, magic_expected_end(case))
            .is_some_and(|report| magic_input_after_report(&capture, report)),
        "batch_contract": magic_batch_contract(case, &capture),
        "native_wire": native_interactions_wire_valid(&capture),
        "multiple_threats": has_multiple_local_threats(&capture),
        "autocast_contract": magic_autocast_contract(&capture, &runes.casts),
        "queue_contract": magic_queue_contract(&capture, &runes.casts),
        "attack_packet": has_real_attack_packet(&capture),
        "death": capture_has_death(&capture),
        "magic": magic_receipt(&capture, case),
    })
}

#[allow(clippy::too_many_arguments)]
fn push_magic_mutant(
    rows: &mut Vec<Value>,
    output: &std::path::Path,
    source_path: &std::path::Path,
    original: &Value,
    case: Case,
    control: &str,
    variant: &str,
    coverage_key: &str,
    description: &str,
    mutant: Value,
    expected_timing: Option<Value>,
    expected_no_melee_offensives: Option<bool>,
    expected_magic_protection: Option<bool>,
    expected_verdict: Option<&str>,
) {
    let mutant_path = retain_mutant(output, source_path, variant, &mutant);
    let mut row = magic_mutant_row(source_path, mutant_path, original, &mutant, case);
    if let Some(expected) = expected_timing.as_ref() {
        assert_eq!(
            &row["protect_timing"], expected,
            "control {control}/{variant} had unexpected timing result"
        );
    }
    if let Some(expected) = expected_no_melee_offensives {
        assert_eq!(
            row["no_melee_offensive_prayers"].as_bool(),
            Some(expected),
            "control {control}/{variant} had unexpected N1 result"
        );
    }
    if let Some(expected) = expected_magic_protection {
        assert_eq!(
            row["magic_protection_timing_ok"].as_bool(),
            Some(expected),
            "control {control}/{variant} had unexpected combined protection result"
        );
    }
    if let Some(expected) = expected_verdict {
        assert_eq!(
            row["new_verdict"].as_str(),
            Some(expected),
            "control {control}/{variant} had unexpected replay verdict: {row}"
        );
    }
    row["control"] = json!(control);
    row["variant"] = json!(variant);
    row["coverage_key"] = json!(coverage_key);
    row["description"] = json!(description);
    row["expected"] = json!({
        "protect_timing": expected_timing,
        "no_melee_offensive_prayers": expected_no_melee_offensives,
        "magic_protection_timing_ok": expected_magic_protection,
        "new_verdict": expected_verdict,
    });
    row["assertion"] = json!(if expected_verdict == Some("PASS") {
        "positive_pass"
    } else {
        "rejected"
    });
    rows.push(row);
}

fn engagement_frame_index(value: &Value, engaged_index: i64, tick: i64) -> usize {
    value["frames"]
        .as_array()
        .unwrap()
        .iter()
        .position(|frame| {
            frame["tick"] == json!(tick)
                && frame["nearby_npcs"]
                    .as_array()
                    .is_some_and(|npcs| npcs.iter().any(|npc| npc["index"] == json!(engaged_index)))
        })
        .unwrap_or_else(|| panic!("no retained engagement frame at tick {tick}"))
}

fn set_prayer_varp_at(value: &mut Value, tick: i64, varp: i64, state: i64) {
    let frame = value["frames"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|frame| frame["tick"] == json!(tick))
        .unwrap_or_else(|| panic!("no retained frame at tick {tick}"));
    let rows = frame["prayer_varps"].as_array_mut().unwrap();
    if let Some(row) = rows.iter_mut().find(|row| row["index"] == json!(varp)) {
        row["value"] = json!(state);
    } else {
        rows.push(json!({"index": varp, "value": state}));
    }
}

fn inject_warlord_onset(value: &mut Value, tick: i64, engaged_index: i64) {
    let frame = value["frames"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|frame| frame["tick"] == json!(tick))
        .unwrap_or_else(|| panic!("no retained frame at onset tick {tick}"));
    let self_slot = frame["self_slot"].as_i64().unwrap();
    let npc = frame["nearby_npcs"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|npc| npc["index"] == json!(engaged_index))
        .unwrap_or_else(|| panic!("engaged Warlord absent at onset tick {tick}"));
    npc["animation"] = json!(401);
    npc["in_combat"] = json!(true);
    npc["target"] = json!({"kind": "Player", "index": self_slot});
}

fn magic_review_mutants(
    value: &Value,
    capture: &CombatCapture,
    case: Case,
    source_path: &std::path::Path,
    output: &std::path::Path,
) -> Value {
    let filename = source_path.file_name().unwrap().to_string_lossy();
    let mut rows = Vec::new();
    if filename == "cm-G1-cml616e3nf_0-receipt.json" {
        assert_eq!(
            protection_timing_receipt(capture),
            json!(true),
            "the retained cml616e3nf onset receipt must still pass timing"
        );
        let mut masked_onsets = value.clone();
        let mut rewritten = 0;
        for frame in masked_onsets["frames"].as_array_mut().unwrap() {
            if let Some(npcs) = frame["nearby_npcs"].as_array_mut() {
                for npc in npcs {
                    if npc["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("khazard warlord"))
                        && npc["animation"] == json!(401)
                    {
                        npc["animation"] = json!(-1);
                        rewritten += 1;
                    }
                }
            }
        }
        assert!(
            rewritten > 0,
            "cml616e3nf has no retained Warlord 401 frames"
        );
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "c",
            "all-onsets-masked",
            "c-missed-onset-still-adjacent",
            "Every Warlord 401 becomes -1; tick 70 remains distance 1, so N/A is refused",
            masked_onsets,
            Some(json!(false)),
            Some(false),
            Some(false),
            Some("FAIL"),
        );
    }

    if filename == "cm-G1-cme63ggc0d_0-receipt.json" {
        assert_eq!(
            protection_timing_receipt(capture),
            json!("not_applicable_no_onset"),
            "the retained cme63ggc0d receipt is the no-onset control"
        );
        assert!(
            no_melee_offensive_prayers(capture),
            "the corrected cme63ggc0d receipt must pass N1"
        );
        let report = report_with_end(capture, "Killed").unwrap();
        let engaged_index = integer(report, "combat_engaged_index").unwrap();
        let engagement_frame = engagement_frame_index(value, engaged_index, 100);

        let mut missing_frame = value.clone();
        missing_frame["frames"]
            .as_array_mut()
            .unwrap()
            .remove(engagement_frame);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "d",
            "missing-engagement-frame",
            "d-missing-engagement-frame",
            "Delete one tick-100 frame from the no-onset engagement evidence",
            missing_frame,
            Some(json!(false)),
            Some(false),
            Some(false),
            Some("INVALID"),
        );

        let mut missing_npc = value.clone();
        missing_npc["frames"][engagement_frame]["nearby_npcs"]
            .as_array_mut()
            .unwrap()
            .retain(|npc| npc["index"] != json!(engaged_index));
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "d",
            "missing-engaged-npc",
            "d-missing-engaged-npc",
            "Remove the engaged Warlord from one tick-100 frame",
            missing_npc,
            Some(json!(false)),
            Some(true),
            Some(false),
            Some("INVALID"),
        );

        let mut adjacent = value.clone();
        let player_tile = adjacent["frames"][engagement_frame]["local_player"]["tile"].clone();
        let npc = adjacent["frames"][engagement_frame]["nearby_npcs"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|npc| npc["index"] == json!(engaged_index))
            .unwrap();
        npc["tile"] = player_tile;
        npc["distance"] = json!(0);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "e",
            "npc-adjacent",
            "e-npc-adjacent-without-onset",
            "Move the engaged Warlord onto the player at tick 100",
            adjacent,
            Some(json!(false)),
            Some(true),
            Some(false),
            Some("FAIL"),
        );

        let mut hp_drop = value.clone();
        let hitpoints = hp_drop["frames"][engagement_frame]["stats"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|stat| stat["name"] == json!("hitpoints"))
            .unwrap();
        let hp = hitpoints["effective"].as_i64().unwrap();
        assert!(hp > 0);
        hitpoints["effective"] = json!(hp - 1);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "f",
            "player-hp-drop",
            "f-player-hp-drop",
            "Drop player hitpoints by one during the no-onset engagement",
            hp_drop,
            Some(json!(false)),
            Some(true),
            Some(false),
            Some("FAIL"),
        );

        let mut early_onset = value.clone();
        inject_warlord_onset(&mut early_onset, 58, engaged_index);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "g",
            "onset-tick-58-before-protect",
            "g-onset-tick-58-fails",
            "Inject a targeted 401 onset at tick 58, before Protect at tick 62",
            early_onset,
            Some(json!(false)),
            Some(true),
            Some(false),
            Some("FAIL"),
        );

        let mut protected_onset = value.clone();
        inject_warlord_onset(&mut protected_onset, 100, engaged_index);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "g",
            "onset-tick-100-positive-pass",
            "g-onset-tick-100-positive-pass",
            "Inject a targeted 401 onset at tick 100 while Protect is already on",
            protected_onset,
            Some(json!(true)),
            Some(true),
            Some(true),
            Some("PASS"),
        );

        let mut offensive_button = value.clone();
        let mut extra_prayer = offensive_button["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| {
                action["batch"] == json!(8)
                    && prayer_action(
                        action,
                        prayer_component(capture, "Protect from Melee").unwrap(),
                    )
            })
            .unwrap()
            .clone();
        extra_prayer["request"]["component_id"] = json!(5619);

        let actions = offensive_button["actions"].as_array_mut().unwrap();
        let first_request = extra_prayer["request_id"].as_u64().unwrap();
        extra_prayer["request_id"] = json!(first_request + 1);
        let attack_position = actions
            .iter()
            .position(|action| action["batch"] == json!(8) && is_npc_attack(action))
            .expect("Protect batch 8 retains its terminal attack");
        actions[attack_position]["request_id"] = json!(first_request + 2);
        actions.insert(attack_position, extra_prayer);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "h",
            "melee-offensive-5619-in-batch-8",
            "h-5619-in-batch-8",
            "Add accepted melee offensive component 5619 to Protect batch 8",
            offensive_button,
            Some(json!("not_applicable_no_onset")),
            Some(false),
            Some(true),
            Some("FAIL"),
        );

        let mut attack_varp = value.clone();
        set_prayer_varp_at(&mut attack_varp, 100, 93, 1);
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "h",
            "attack-prayer-varp-93-on",
            "h-prayer-varp-93-on",
            "Set Attack prayer varp 93 to 1 during the engagement",
            attack_varp,
            Some(json!("not_applicable_no_onset")),
            Some(false),
            Some(true),
            Some("FAIL"),
        );

        let protect = capture
            .actions
            .iter()
            .find(|action| {
                prayer_action(
                    action,
                    prayer_component(capture, "Protect from Melee").unwrap(),
                )
            })
            .unwrap();
        let plan = plan_rows(capture, protect);
        let terminal = plan.last().unwrap();
        assert!(
            is_npc_attack(terminal),
            "G1 Protect terminal must be Attack"
        );
        let terminal_batch = terminal["batch"].clone();
        let terminal_sequence = terminal["sequence"].clone();
        let mut missing_terminal = value.clone();
        missing_terminal["actions"]
            .as_array_mut()
            .unwrap()
            .retain(|action| {
                !(action["batch"] == terminal_batch
                    && action["sequence"] == terminal_sequence
                    && is_npc_attack(action))
            });
        push_magic_mutant(
            &mut rows,
            output,
            source_path,
            value,
            case,
            "i",
            "protect-attack-terminal-removed",
            "i-protect-terminal-removed",
            "Remove the Attack terminal from the G1 Protect plan",
            missing_terminal,
            Some(json!("not_applicable_no_onset")),
            Some(true),
            Some(false),
            Some("FAIL"),
        );
    }
    json!(rows)
}

fn manual_protect_terminal_mutants(
    value: &Value,
    capture: &CombatCapture,
    case: Case,
    source_path: &std::path::Path,
    output: &std::path::Path,
) -> Value {
    assert_eq!(protection_timing_receipt(capture), json!(true));
    let component = prayer_component(capture, "Protect from Melee").unwrap();
    let protect = capture
        .actions
        .iter()
        .find(|row| prayer_action(row, component))
        .unwrap();
    let rows = plan_rows(capture, protect);
    let terminal = rows.last().unwrap();
    assert_eq!(
        terminal["request"]["op"],
        json!("use-widget-on"),
        "manual Protect plan should terminate in its targeted Cast"
    );
    let batch = terminal["batch"].clone();
    let source_name = source_path.file_name().unwrap().to_string_lossy();
    let source_identity = if source_name.contains("cmmy8z5ypd") {
        "cmmy8z5ypd"
    } else if source_name.contains("cmbkvsh23t") {
        "cmbkvsh23t"
    } else {
        "other"
    };
    let mut mutants = Vec::new();

    let mut non_npc_widget = value.clone();
    let widget = non_npc_widget["actions"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|row| row["batch"] == batch && row["request"]["op"] == json!("use-widget-on"))
        .unwrap();
    widget["request"]["kind"] = json!("obj");
    push_magic_mutant(
        &mut mutants,
        output,
        source_path,
        value,
        case,
        "manual-terminal",
        "non-npc-widget-on",
        &format!("manual-terminal-{source_identity}-non-npc-widget"),
        "Change the terminal widget-on target from NPC to object",
        non_npc_widget,
        Some(json!(false)),
        Some(false),
        Some(false),
        Some("FAIL"),
    );

    let mut protect_alone = value.clone();
    protect_alone["actions"]
        .as_array_mut()
        .unwrap()
        .retain(|row| !(row["batch"] == batch && row["request"]["op"] == json!("use-widget-on")));
    push_magic_mutant(
        &mut mutants,
        output,
        source_path,
        value,
        case,
        "manual-terminal",
        "protect-without-cast-terminal",
        &format!("manual-terminal-{source_identity}-protect-alone"),
        "Remove the manual Protect plan's Cast terminal",
        protect_alone,
        Some(json!(false)),
        Some(false),
        Some(false),
        Some("FAIL"),
    );
    json!(mutants)
}

// Recreate the harness's first full readiness boundary, rather than applying
// exact per-engagement arm counts to a timed-out run's later NoTarget retries.
// No oracle leaf is relaxed; every mutant follows this same stopping rule.
fn causal_magic_capture(receipt: &Value, case: Case) -> (Option<i64>, CombatCapture) {
    let full = capture(receipt);
    if !case.is_magic() {
        return (None, full);
    }
    match earliest_ready_prefix(receipt, case) {
        Some((tick, prefix)) => (Some(tick), prefix),
        None => (None, full),
    }
}

fn earliest_ready_prefix(receipt: &Value, case: Case) -> Option<(i64, CombatCapture)> {
    // Readiness cannot precede the required combat report.
    let first_report_tick = receipt["statuses"]
        .as_array()?
        .iter()
        .filter(|status| status["fields"]["combat_end"] == magic_expected_end(case))
        .filter_map(|status| integer(status, "combat_evidence_tick"))
        .min()?;
    let mut cutoffs = receipt["frames"]
        .as_array()?
        .iter()
        .filter_map(|frame| frame["tick"].as_i64())
        .collect::<Vec<_>>();
    cutoffs.extend(
        receipt["actions"]
            .as_array()?
            .iter()
            .filter_map(|action| action["tick"].as_i64()),
    );
    cutoffs.extend(
        receipt["observations"]
            .as_array()?
            .iter()
            .filter_map(|observation| observation["tick"].as_i64()),
    );
    cutoffs.extend(
        receipt["statuses"]
            .as_array()?
            .iter()
            .filter_map(|status| status["fields"]["combat_evidence_tick"].as_i64()),
    );
    cutoffs.sort_unstable();
    cutoffs.dedup();
    cutoffs.retain(|tick| *tick >= first_report_tick);

    for tick in cutoffs {
        let mut prefix = capture(receipt);
        prefix.frames.retain(|frame| {
            frame["tick"]
                .as_i64()
                .is_some_and(|frame_tick| frame_tick <= tick)
        });
        prefix.actions.retain(|action| {
            action["tick"]
                .as_i64()
                .is_some_and(|action_tick| action_tick <= tick)
        });
        prefix.observations.retain(|observation| {
            observation["tick"]
                .as_i64()
                .is_some_and(|observation_tick| observation_tick <= tick)
        });
        prefix.statuses.retain(|status| {
            status["fields"]["combat_evidence_tick"]
                .as_i64()
                .is_none_or(|evidence_tick| evidence_tick <= tick)
        });
        prefix.random_events.retain(|event| {
            event["tick"]
                .as_i64()
                .is_none_or(|event_tick| event_tick <= tick)
        });
        let latest_generation = prefix
            .frames
            .iter()
            .filter_map(|frame| frame["player_generation"].as_u64())
            .max()
            .unwrap_or_default();
        prefix.magic_npc_events.retain(|event| {
            event["player_generation"]
                .as_u64()
                .is_some_and(|generation| generation <= latest_generation)
        });
        if case_invalid_reason(case, &prefix).is_some() {
            return None;
        }
        if case_ready(case, &prefix) {
            return Some((tick, prefix));
        }
    }
    None
}

#[test]
#[ignore = "offline replay requires retained evidence; COMBAT_RECEIPT_ROOTS and COMBAT_REPLAY_OUTPUT"]
fn replay_all_retained_combat_receipts() {
    let roots = std::env::var_os("COMBAT_RECEIPT_ROOTS").expect("receipt roots");
    let output = std::env::var_os("COMBAT_REPLAY_OUTPUT").expect("replay output");
    let output_path = Path::new(&output);
    let mut files = Vec::new();
    for root in std::env::split_paths(&roots) {
        files.extend(
            std::fs::read_dir(root)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| {
                            (name.starts_with('M') || name.starts_with("cm-"))
                                && name.ends_with("-receipt.json")
                        })
                }),
        );
    }
    files.sort();
    let mut review_control_coverage = std::collections::HashSet::<String>::new();
    let mut results = Vec::new();
    for path in files {
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let case = match value["case"].as_str().unwrap() {
            "M1" => Case::M1,
            "M1-staged-hand-in" => Case::M1HandIn,
            "M2" => Case::M2,
            "M3" => Case::M3,
            "M4" => Case::M4,
            "M5" => Case::M5,
            "M6" => Case::M6,
            "cm-G1" => Case::MageAuto,
            "cm-G2-fallback" => Case::MageManualFallback,
            "cm-G2-no-fallback" => Case::MageManualNoFallback,
            other => panic!("unknown case {other}"),
        };
        let full_capture = capture(&value);
        let full_tail_verdict = replay_verdict(case, &value, &full_capture).0;
        let (earliest_ready, mut c) = causal_magic_capture(&value, case);
        let (verdict, ready, preflight, invalid) = replay_verdict(case, &value, &c);
        let died = capture_has_death(&c);
        let mut leaves = json!({
            "native_wire": native_interactions_wire_valid(&c),
            "batch_contract": batch_plan_contract(&c, &batch_plans(&c)),
            "corpse_for_every_killed": every_killed_report_has_corpse(&c),
            "death": died, "multiple_threats": has_multiple_local_threats(&c),
            "distinct_killed": combat_outcomes(&c).iter().filter(|fields| fields["combat_end"] == json!("Killed")).count(),
        });
        for row in &c.actions {
            if row["request"]["op"] == "open-stand"
                && row["wire_opcodes"][0] == 195
                && action_wire_valid(row)
            {
                let mut extra_event = row.clone();
                extra_event["wire_opcodes"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(86));
                assert!(!action_wire_valid(&extra_event));
                extra_event["wire_opcodes"] = json!([195, 195, 67, 45]);
                assert!(!action_wire_valid(&extra_event));
                leaves["bank_extra_event_and_double_anticheat_mutants_rejected"] = json!(true);
            }
            if row["request"]["action"] == "Talk-to" && action_wire_valid(row) {
                let mut wrong_packet = row.clone();
                wrong_packet["wire_opcodes"] = json!([67, 21]);
                assert!(!action_wire_valid(&wrong_packet));
                wrong_packet["wire_opcodes"] = row["wire_opcodes"].clone();
                wrong_packet["request"]["index"] = json!(-1);
                assert!(!action_wire_valid(&wrong_packet));
                leaves["dialogue_wrong_packet_and_actor_mutants_rejected"] = json!(true);
            }
            if row["request"]["debug"] == "ContinueDialog" && action_wire_valid(row) {
                let mut wrong_packet = row.clone();
                wrong_packet["wire_opcodes"] = json!([86]);
                assert!(!action_wire_valid(&wrong_packet));
                leaves["dialogue_continue_wrong_packet_mutant_rejected"] = json!(true);
            }
            if row["request"]["debug"] == "Answer { option: 1 }" && action_wire_valid(row) {
                let mut malformed = row.clone();
                malformed["request"]["debug"] = json!("Answer { option: 0 }");
                assert!(!action_wire_valid(&malformed));
                malformed["request"]["debug"] = json!("arbitrary Answer { option: 1 }");
                assert!(!action_wire_valid(&malformed));
                leaves["dialogue_malformed_answer_mutants_rejected"] = json!(true);
            }
        }
        match case {
            Case::M1HandIn if ready => {
                // Grok F5; wizard_mizgog.rs2:1-19,39-52 requires both talks.
                for missing in 0..2 {
                    let mut mutant = value.clone();
                    let mut talk = 0;
                    mutant["actions"].as_array_mut().unwrap().retain(|row| {
                        if row["request"]["action"] != "Talk-to" {
                            return true;
                        }
                        let keep = talk != missing;
                        talk += 1;
                        keep
                    });
                    assert!(!case_ready(case, &capture(&mutant)));
                    let root = Path::new(&output).parent().unwrap().join("mutations");
                    std::fs::create_dir_all(&root).unwrap();
                    let mutant_path = root.join(format!(
                        "{}.missing-talk-{missing}.json",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                    std::fs::write(&mutant_path, serde_json::to_vec_pretty(&mutant).unwrap())
                        .unwrap();
                    leaves[format!("missing_talk_{missing}_mutant_rejected")] = json!(mutant_path);
                }
                for op in ["Talk-to", "ContinueDialog", "Answer { option: 1 }"] {
                    let mut mutant = value.clone();
                    let row = mutant["actions"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|row| row["request"]["action"] == op || row["request"]["debug"] == op)
                        .unwrap();
                    row["wire_opcodes"] = json!([0]);
                    assert!(!case_ready(case, &capture(&mutant)));
                }
                leaves["hand_in_wrong_wire_mutants_rejected"] = json!(true);
            }
            Case::M1 => {
                leaves["every_imp_corpse_has_outcome"] = json!(every_imp_corpse_has_outcome(&c));
                leaves["four_beads"] = json!(c
                    .frames
                    .iter()
                    .any(|frame| IMP_BEADS.iter().all(|id| item_count(frame, *id) > 0)));
                leaves["stage_advanced"] = json!(c.statuses.iter().any(|status| matches!(
                    status["fields"]["stage"].as_str(),
                    Some("imp:1" | "imp:2")
                )));
                let classification = imp_corpse_classification(&c);
                if let Some(episode) = classification["episodes"].as_array().and_then(|rows| {
                    rows.iter()
                        .find(|row| row["class"] == json!("post_budget_kill"))
                }) {
                    let index = episode["index"].as_i64().unwrap();
                    let start = episode["first_corpse_tick"].as_i64().unwrap();
                    let end = episode["last_corpse_tick"].as_i64().unwrap();
                    let mut mutant = value.clone();
                    for frame in mutant["frames"].as_array_mut().unwrap() {
                        if frame["tick"]
                            .as_i64()
                            .is_some_and(|tick| (start..=end).contains(&tick))
                        {
                            for npc in frame["nearby_npcs"].as_array_mut().unwrap() {
                                if npc["index"] == json!(index) {
                                    npc["index"] = json!(100_000 + index);
                                }
                            }
                        }
                    }
                    let rejected = imp_corpse_classification(&capture(&mutant));
                    assert_eq!(
                        rejected["orphan"].as_u64(),
                        classification["orphan"].as_u64().map(|count| count + 1)
                    );
                    assert!(!every_imp_corpse_has_outcome(&capture(&mutant)));
                    let root = Path::new(&output).parent().unwrap().join("mutations");
                    std::fs::create_dir_all(&root).unwrap();
                    let mutant_path = root.join(format!(
                        "{}.wrong-post-budget-npc.json",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                    std::fs::write(&mutant_path, serde_json::to_vec_pretty(&mutant).unwrap())
                        .unwrap();
                    leaves["wrong_post_budget_npc_mutant_rejected"] = json!(mutant_path);
                }
                leaves["imp_corpse_classification"] = classification;
            }
            Case::M3 => leaves["food_timing"] = m3_timing_receipt(&c),
            Case::M6 => {
                if preflight.is_none() && c.start_baseline.is_some() {
                    let mut mutant = value.clone();
                    let inventory = mutant["start_baseline"]["inventory"]
                        .as_array_mut()
                        .unwrap();
                    let food = inventory
                        .iter()
                        .position(|row| row["id"] == LOBSTER_ID)
                        .unwrap();
                    inventory.remove(food);
                    assert!(start_preflight(case, &mutant["start_baseline"]).is_some());
                    let root = Path::new(&output).parent().unwrap().join("mutations");
                    std::fs::create_dir_all(&root).unwrap();
                    let mutant_path = root.join(format!(
                        "{}.wrong-food-quantity.json",
                        path.file_name().unwrap().to_string_lossy()
                    ));
                    std::fs::write(&mutant_path, serde_json::to_vec_pretty(&mutant).unwrap())
                        .unwrap();
                    leaves["wrong_food_quantity_mutant_rejected"] = json!(mutant_path);
                }
                let combo = m6_combo_receipt(&c);
                if let Some(eat) = combo["eats"].as_array().and_then(|rows| {
                    rows.iter()
                        .find(|row| row["both_counts_decremented_and_hp_capped"] == json!(true))
                }) {
                    let tick = eat["tick"].as_i64().unwrap();
                    c.frames
                        .retain(|frame| frame["tick"].as_i64() != Some(tick + 1));
                    let mutant = m6_combo_receipt(&c);
                    let row = mutant["eats"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|row| row["tick"].as_i64() == Some(tick))
                        .unwrap();
                    assert_eq!(row["both_counts_decremented_and_hp_capped"], json!(false));
                    leaves["missing_combo_output_mutant_rejected"] = json!(true);
                }
                leaves["combo"] = combo;
            }
            Case::M4 => {
                if let Some(report) = report_with_end(&c, "Aborted(Unattackable)") {
                    leaves["conditional_eat"] = m4_conditional_eat_receipt(&c, report);
                    if ready {
                        // The Start-window correction must not exempt real
                        // loaded low HP before the abort from its required Eat.
                        let mut mutant = value.clone();
                        let mut frame = value["start_baseline"].clone();
                        let end = integer(report, "combat_evidence_tick").unwrap();
                        frame["tick"] = json!(end - 1);
                        frame["host_tick"] = json!(end - 1);
                        let hp = frame["stats"]
                            .as_array_mut()
                            .unwrap()
                            .iter_mut()
                            .find(|stat| stat["name"] == "hitpoints")
                            .unwrap();
                        hp["effective"] = json!(7);
                        assert!(hp["base"].as_i64().unwrap() > 0);
                        mutant["frames"].as_array_mut().unwrap().insert(0, frame);
                        mutant["actions"]
                            .as_array_mut()
                            .unwrap()
                            .retain(|row| !is_eat(row));
                        assert!(!case_ready(case, &capture(&mutant)));
                        assert!(!m4_conditional_eat_ok(&capture(&mutant), report));
                        let root = Path::new(&output).parent().unwrap().join("mutations");
                        std::fs::create_dir_all(&root).unwrap();
                        let mutant_path = root.join(format!(
                            "{}.loaded-low-hp-without-eat.json",
                            path.file_name().unwrap().to_string_lossy()
                        ));
                        std::fs::write(&mutant_path, serde_json::to_vec_pretty(&mutant).unwrap())
                            .unwrap();
                        leaves["loaded_low_hp_without_eat_mutant_rejected"] = json!(mutant_path);
                    }
                }
            }
            Case::M5 => {
                if let Some(injection) = c
                    .random_events
                    .iter()
                    .find(|event| event["kind"] == json!("Maze"))
                {
                    leaves["owner_preempted"] = json!(m5_owner_preempted(&c, injection));
                    leaves["cleanup"] = json!(clear_prayers_before_next_operation(&c, injection));
                    leaves["prefix_contract"] = json!(m5_prefix_contract(&c, injection));
                }
                // Historical passes must not authorize measured-window cheats or duplicate Attack.
                if let Some(stage) = c
                    .actions
                    .iter_mut()
                    .find(|row| row["origin"] == json!("staging"))
                {
                    stage["request"] = json!("setvar prayer14 1");
                    assert!(!m5_ready(&c), "cheat mutant accepted: {}", path.display());
                    leaves["cheat_mutant_rejected"] = json!(true);
                }
                c = capture(&value);
                if let Some(attack) = c.actions.iter().find(|row| is_npc_attack(row)).cloned() {
                    c.actions.push(attack);
                    assert!(
                        !m5_ready(&c),
                        "duplicate mutant accepted: {}",
                        path.display()
                    );
                    leaves["duplicate_attack_mutant_rejected"] = json!(true);
                }
            }
            Case::MageAuto | Case::MageManualFallback | Case::MageManualNoFallback => {
                if path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("latekill60"))
                {
                    assert!(
                        !every_killed_report_has_corpse(&c),
                        "retained latekill60 mutant accepted"
                    );
                    leaves["latekill60_mutant_rejected"] = json!(true);
                }
                let evidence = magic_rune_evidence(&c);
                leaves["runes_coherent"] = json!(evidence.coherent);
                leaves["cast_cadence"] = json!(rune_cast_cadence_ok(&evidence.casts));
                leaves["fire_bolts"] = json!(rune_cast_count(&evidence.casts, MageSpell::FireBolt));
                leaves["fire_strikes"] =
                    json!(rune_cast_count(&evidence.casts, MageSpell::FireStrike));
                let protect_timing = protection_timing_receipt(&c);
                let no_melee_offensives = no_melee_offensive_prayers(&c);
                leaves["protect_timing"] = protect_timing.clone();
                leaves["no_melee_offensive_prayers"] = json!(no_melee_offensives);
                leaves["protect_restoring_terminal"] = json!(protect_plan_ends_with_terminal(&c));
                leaves["magic_protection_timing_ok"] = json!(magic_protection_timing_ok(&c));
                leaves["manual_cast_contract"] = json!(
                    case == Case::MageAuto || manual_magic_cast_contract(&c, &evidence.casts)
                );
                if case == Case::MageAuto {
                    leaves["queue_contract"] = json!(magic_queue_contract(&c, &evidence.casts));
                    leaves["splash_count"] = json!(magic_splashes(&c, &evidence.casts).len());
                }

                let filename = path.file_name().unwrap().to_string_lossy();
                if ["cml616e3nf", "cmmy8z5ypd", "cmbkvsh23t"]
                    .iter()
                    .any(|id| filename.contains(id))
                {
                    assert!(
                        !no_melee_offensives,
                        "retained onset receipt unexpectedly passed N1: {}",
                        path.display()
                    );
                    assert_ne!(
                        verdict,
                        "PASS",
                        "retained onset receipt passed overall after failing N1: {}",
                        path.display()
                    );
                }

                let mut review_mutants = magic_review_mutants(&value, &c, case, &path, output_path);
                if protect_timing == json!(true) {
                    let onset = first_warlord_attack_onset(&c).unwrap();
                    let component = prayer_component(&c, "Protect from Melee").unwrap();

                    let mut never_on = value.clone();
                    set_prayer_varp_off_through(&mut never_on, 97, None);
                    push_magic_mutant(
                        review_mutants.as_array_mut().unwrap(),
                        output_path,
                        &path,
                        &value,
                        case,
                        "b",
                        "never-on",
                        "b-never-on",
                        "Onset receipt with Protect from Melee never observed on",
                        never_on,
                        Some(json!(false)),
                        Some(no_melee_offensives),
                        Some(false),
                        Some("FAIL"),
                    );
                    leaves["never_on_mutant_rejected"] = json!(true);

                    let mut late = value.clone();
                    let action = late["actions"]
                        .as_array_mut()
                        .unwrap()
                        .iter_mut()
                        .find(|row| prayer_action(row, component))
                        .unwrap();
                    action["tick"] = json!(onset + 3);
                    set_prayer_varp_off_through(&mut late, 97, Some(onset + 2));
                    push_magic_mutant(
                        review_mutants.as_array_mut().unwrap(),
                        output_path,
                        &path,
                        &value,
                        case,
                        "a",
                        "late-onset-plus-3",
                        "a-late-onset-plus-3",
                        "Onset receipt with Protect from Melee delayed to onset+3",
                        late,
                        Some(json!(false)),
                        Some(no_melee_offensives),
                        Some(false),
                        Some("FAIL"),
                    );
                    leaves["late_on_mutant_rejected"] = json!(true);

                    if case.is_manual_magic() {
                        let manual_mutants =
                            manual_protect_terminal_mutants(&value, &c, case, &path, output_path);
                        for row in manual_mutants.as_array().unwrap() {
                            review_control_coverage
                                .insert(row["coverage_key"].as_str().unwrap().to_owned());
                            review_mutants.as_array_mut().unwrap().push(row.clone());
                        }
                        leaves["manual_terminal_mutants"] = manual_mutants;
                        leaves["non_npc_widget_terminal_mutant_rejected"] = json!(true);
                        leaves["manual_protect_alone_mutant_rejected"] = json!(true);
                    }
                }
                for row in review_mutants.as_array().unwrap() {
                    review_control_coverage
                        .insert(row["coverage_key"].as_str().unwrap().to_owned());
                }
                leaves["review_control_mutants"] = review_mutants;
            }
            _ => {}
        }
        let file_name = path.file_name().unwrap().to_string_lossy();
        if file_name == "cm-G1-cme63ggc0d_0-receipt.json" {
            assert!(
                earliest_ready.is_some(),
                "cme63ggc0d has no ready prefix before the retained tail"
            );
        }
        let protect_timing_only = [
            "cm-G1-cml616e3nf_0-receipt.json",
            "cm-G2-fallback-cmmy8z5ypd_0-receipt.json",
            "cm-G2-no-fallback-cmbkvsh23t_0-receipt.json",
        ]
        .iter()
        .any(|receipt_name| file_name == *receipt_name);
        let label = if protect_timing_only {
            Some("protect-timing-only")
        } else if file_name == "cm-G1-cme63ggc0d_0-receipt.json" && verdict == "PASS" {
            Some("offline replay PASS of a receipt whose live run timed out at 900 s")
        } else {
            None
        };
        results.push(json!({
            "path": path,
            "old_verdict": value["outcome"],
            "new_verdict": verdict,
            "full_tail_verdict": full_tail_verdict,
            "label": label,
            "earliest_ready_tick": earliest_ready,
            "ready": ready,
            "preflight": preflight,
            "current_invalid": invalid,
            "recorded_invalid": value["invalid_reason"],
            "recorded_error": value["error"],
            "leaves": leaves
        }));
    }
    assert!(!results.is_empty(), "no retained receipts");

    for required in [
        "a-late-onset-plus-3",
        "b-never-on",
        "c-missed-onset-still-adjacent",
        "d-missing-engagement-frame",
        "d-missing-engaged-npc",
        "e-npc-adjacent-without-onset",
        "f-player-hp-drop",
        "g-onset-tick-58-fails",
        "g-onset-tick-100-positive-pass",
        "h-5619-in-batch-8",
        "h-prayer-varp-93-on",
        "i-protect-terminal-removed",
        "manual-terminal-cmmy8z5ypd-non-npc-widget",
        "manual-terminal-cmmy8z5ypd-protect-alone",
        "manual-terminal-cmbkvsh23t-non-npc-widget",
        "manual-terminal-cmbkvsh23t-protect-alone",
    ] {
        assert!(
            review_control_coverage.contains(required),
            "retained receipt replay did not exercise control {required}"
        );
    }
    std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    println!("Replayed {} retained receipt files", results.len());
}
