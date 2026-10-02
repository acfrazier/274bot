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
    capture.maze_injected = capture
        .random_events
        .iter()
        .any(|event| event["kind"] == json!("Maze") && event["error"].is_null());
    capture.maze_owner_live_before = value["maze_attack_owner_live_before"].as_bool();
    capture.maze_owner_live_after = value["maze_attack_owner_live_after"].as_bool();
    capture
}

#[test]
#[ignore = "offline replay requires retained evidence; COMBAT_RECEIPT_ROOTS and COMBAT_REPLAY_OUTPUT"]
fn replay_all_retained_combat_receipts() {
    let roots = std::env::var_os("COMBAT_RECEIPT_ROOTS").expect("receipt roots");
    let output = std::env::var_os("COMBAT_REPLAY_OUTPUT").expect("replay output");
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
                            name.starts_with('M') && name.ends_with("-receipt.json")
                        })
                }),
        );
    }
    files.sort();
    let mut results = Vec::new();
    for path in files {
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        let mut c = capture(&value);
        let case = match value["case"].as_str().unwrap() {
            "M1" => Case::M1,
            "M1-staged-hand-in" => Case::M1HandIn,
            "M2" => Case::M2,
            "M3" => Case::M3,
            "M4" => Case::M4,
            "M5" => Case::M5,
            "M6" => Case::M6,
            other => panic!("unknown case {other}"),
        };
        let ready = case_ready(case, &c);
        let preflight = c
            .start_baseline
            .as_ref()
            .and_then(|baseline| start_preflight(case, baseline));
        let invalid = case_invalid_reason(case, &c);
        let died = capture_has_death(&c);
        let verdict = if died {
            "FAIL"
        } else if preflight.is_some() || invalid.is_some() || value["invalid_reason"].is_string() {
            "INVALID"
        } else if ready {
            "PASS"
        } else {
            "FAIL"
        };
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
            _ => {}
        }
        results.push(json!({"path": path, "old_verdict": value["outcome"], "new_verdict": verdict,
            "ready": ready, "preflight": preflight, "current_invalid": invalid,
            "recorded_invalid": value["invalid_reason"], "recorded_error": value["error"], "leaves": leaves}));
    }
    assert!(!results.is_empty(), "no retained receipts");
    std::fs::write(output, serde_json::to_vec_pretty(&results).unwrap()).unwrap();
    println!("Replayed {} retained receipt files", results.len());
}
