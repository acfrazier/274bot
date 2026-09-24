    use super::*;
    use crate::suite::manifest::SuiteManifest;

    fn manifest() -> SuiteManifest {
        SuiteManifest::parse(super::super::EMBEDDED_MANIFEST.as_bytes(), "embedded").unwrap()
    }

    fn core_case(id: &str) -> CaseEntry {
        manifest().case(id).expect("case").clone()
    }

    /// The core/pair entry point the scenario tests use: none of them launches the loader smoke,
    /// so no external source is bound.
    fn validate(
        case: &CaseEntry,
        receipts: &ParsedReceipts,
        exit_code: Option<i32>,
        captures: &[CaptureRecord],
    ) -> Verdict {
        super::validate(case, receipts, exit_code, captures, None)
    }

    fn external_case() -> CaseEntry {
        manifest()
            .case("external_loader")
            .expect("external loader row")
            .clone()
    }

    /// A real SHA-256 digest of `seed`, as the run and the producer write them. The validator
    /// refuses anything that is not a lowercase digest, so the tests bind real ones.
    fn digest(seed: &str) -> String {
        super::super::identity::sha256(seed.as_bytes())
    }

    /// The digest the producer's harmless whitespace reload of the bound input produces.
    fn whitespace_digest() -> String {
        digest("harmless whitespace transform")
    }

    fn bound_source(sha: &str) -> ExternalSource {
        ExternalSource {
            path: "/tmp/ExampleBot.ts".into(),
            sha256: sha.into(),
            bytes: 1,
            default_fixture: true,
            frozen_sha256: host_play::external_loader::FROZEN_SHA256.into(),
            frozen_match: sha == host_play::external_loader::FROZEN_SHA256,
            harmless_whitespace_sha256: whitespace_digest(),
        }
    }

    /// A real qualified external receipt: the producer's own state machine drives the record, so
    /// the test cannot drift from the receipt it will read. The bound raw source is the loaded
    /// card's origin cache key, and the harmless whitespace transform is what both post-reload
    /// identities move to.
    fn qualified_external_receipt(account: &str, source_sha: &str) -> Value {
        use host_play::external_loader::{
            ExternalWatch, Operation, BONES_COUNT, NOTHING_CHANGED, SCRIPT_NAME,
        };
        let watch = ExternalWatch::default();
        let now = std::time::Instant::now();
        let owned = Path::new("/tmp/274bot-external-1-ExampleBot.ts");
        let identity = "file:/tmp/274bot-external-1-ExampleBot.ts";
        watch.configure(account, owned.to_path_buf(), source_sha.to_string());
        watch.note_scene(true, 2);
        watch.note_inventory(now, account, BONES_COUNT, 0);
        watch.note_prereq_passed();
        watch.note_load(1, SCRIPT_NAME, owned, identity, source_sha, true, false);
        watch.begin_start(now).expect("Start");
        let burials: Vec<String> = (1..=10)
            .map(|i| format!("buried bones (#{i}, +{i} prayer xp total)"))
            .collect();
        watch.note_logs(now, account, &burials);
        watch.note_inventory(now, account, 12, 45);
        assert_eq!(
            watch.requested_operation(),
            Some(Operation::Stop),
            "the producer's own burials/XP/inventory gate must have advanced to Stop"
        );
        watch.note_logs(
            now,
            account,
            &["BoneBurier stopped — 10 buried, +45 prayer xp".into()],
        );
        watch.note_stop(now, true, false);
        watch.note_reload_unchanged(NOTHING_CHANGED);
        let after = whitespace_digest();
        watch.note_reload_changed(
            1, true, false, owned, identity, source_sha, &after, source_sha, &after, true, false,
        );
        watch.note_capture_requested();
        (*watch.qualify().expect("qualified")).clone()
    }

    /// The producer's real failure record: a prerequisite failure before the load.
    fn failed_external_receipt(account: &str) -> Value {
        use host_play::external_loader::ExternalWatch;
        let watch = ExternalWatch::default();
        watch.configure(
            account,
            Path::new("/tmp/274bot-external-1-ExampleBot.ts").to_path_buf(),
            host_play::external_loader::FROZEN_SHA256.into(),
        );
        watch.note_prereq_failed("fixture did not reach scene 2");
        watch.evidence()
    }

    /// A structurally complete terminal capture of the loaded actor, under the producer's own
    /// label and with the actor binding the panel writes into the sidecar.
    fn terminal_capture(actor: &str) -> CaptureRecord {
        let mut record = capture_record();
        record.label = format!(
            "2026-09-14T00-00-02_{}",
            scenario::shot::safe_label(host_play::external_loader::TERMINAL_SHOT)
        );
        record.sidecar_actor = Some(actor.into());
        record
    }

    /// The prerequisite capture is a different label and never the terminal evidence.
    fn prereq_capture() -> CaptureRecord {
        let mut record = capture_record();
        record.label = format!(
            "2026-09-14T00-00-01_{}",
            scenario::shot::safe_label(host_play::external_loader::PREREQ_SHOT)
        );
        record.sidecar_actor = Some("alice".into());
        record
    }

    fn external_output(receipt: &Value, terminal: &str) -> String {
        format!(
            "EXTERNAL_LOADER: script_external_loader {receipt}\n\
             {terminal}: live script_external_loader {receipt}\n"
        )
    }

    #[test]
    fn a_qualified_external_receipt_with_its_terminal_capture_is_pending_visual_review() {
        let case = external_case();
        let sha = host_play::external_loader::FROZEN_SHA256;
        let receipt = qualified_external_receipt("alice", sha);
        let receipts = parse(&external_output(&receipt, "PASS"));
        assert_eq!(
            receipts
                .external
                .as_ref()
                .map(|witness| witness.name.as_str()),
            Some("script_external_loader")
        );
        let verdict = super::validate(
            &case,
            &receipts,
            Some(0),
            &[terminal_capture("alice")],
            Some(&bound_source(sha)),
        );
        assert!(
            matches!(verdict, Verdict::PendingVisualReview { .. }),
            "{verdict:?}"
        );
        // Existence and structure are not visual approval: the verdict is never a bare pass.
        assert!(!matches!(verdict, Verdict::Passed));

        // The prerequisite capture is written too (the fixture's own proof) and is not the
        // contracted evidence; both together still resolve to the terminal capture.
        let verdict = super::validate(
            &case,
            &receipts,
            Some(0),
            &[prereq_capture(), terminal_capture("alice")],
            Some(&bound_source(sha)),
        );
        match verdict {
            Verdict::PendingVisualReview { captures } => {
                assert_eq!(captures.len(), 2, "both captures stay recorded as evidence");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_external_case_needs_its_own_witness_not_a_bare_pass() {
        let case = external_case();
        let sha = host_play::external_loader::FROZEN_SHA256;
        let receipt = qualified_external_receipt("alice", sha);
        let source = bound_source(sha);

        // A bare PASS string, and a PASS without the EXTERNAL_LOADER witness, are not a result.
        for output in [
            "PASS\n".to_string(),
            format!("PASS: live script_external_loader {receipt}\n"),
            format!("EXTERNAL_LOADER: script_thiever {receipt}\nFAIL: live script_external_loader {receipt}\n"),
        ] {
            let receipts = parse(&output);
            let verdict = super::validate(
                &case,
                &receipts,
                Some(1),
                &[terminal_capture("alice")],
                Some(&source),
            );
            assert!(
                matches!(verdict, Verdict::SharedFailure { .. }),
                "{output} -> {verdict:?}"
            );
        }

        // A witness that names another live name is rejected.
        let receipts = parse(&format!(
            "EXTERNAL_LOADER: script_bone_burier {receipt}\nPASS: live script_external_loader {receipt}\n"
        ));
        match super::validate(&case, &receipts, Some(0), &[], Some(&source)) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("expected \"script_external_loader\""),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn external_receipt_requires_real_counters() {
        let case = external_case();
        let source = bound_source(host_play::external_loader::FROZEN_SHA256);
        let receipt = qualified_external_receipt("alice", &source.sha256);
        let capture = terminal_capture("alice");
        for (section, key) in [
            ("initial", "bones"),
            ("final", "bones"),
            ("initial", "prayer_xp"),
            ("final", "prayer_xp"),
        ] {
            for value in [
                None,
                Some(Value::Null),
                Some(serde_json::json!("0")),
                Some(serde_json::json!(-1)),
            ] {
                let mut broken = receipt.clone();
                let fields = broken[section].as_object_mut().unwrap();
                match &value {
                    Some(value) => {
                        fields.insert(key.into(), value.clone());
                    }
                    None => {
                        fields.remove(key);
                    }
                }
                let parsed = parse(&external_output(&broken, "PASS"));
                assert!(
                    matches!(
                        super::validate(
                            &case,
                            &parsed,
                            Some(0),
                            std::slice::from_ref(&capture),
                            Some(&source)
                        ),
                        Verdict::SharedFailure {
                            kind: "receipt",
                            ..
                        }
                    ),
                    "accepted {section}.{key}={value:?}"
                );
            }
        }
    }

    #[test]
    fn external_receipt_requires_the_terminal_case_name() {
        let case = external_case();
        let source = bound_source(host_play::external_loader::FROZEN_SHA256);
        for (terminal, receipt, exit) in [
            (
                "PASS",
                qualified_external_receipt("alice", &source.sha256),
                0,
            ),
            ("FAIL", failed_external_receipt("alice"), 1),
        ] {
            let output = external_output(&receipt, terminal).replace(
                &format!("{terminal}: live script_external_loader"),
                &format!("{terminal}: live script_thiever"),
            );
            let parsed = parse(&output);
            assert!(
                matches!(
                    super::validate(
                        &case,
                        &parsed,
                        Some(exit),
                        &[terminal_capture("alice")],
                        Some(&source)
                    ),
                    Verdict::SharedFailure {
                        kind: "receipt",
                        ..
                    }
                ),
                "accepted wrong {terminal} name"
            );
        }
    }

    #[test]
    fn contradictory_or_incomplete_external_receipts_are_rejected_field_by_field() {
        let case = external_case();
        let sha = host_play::external_loader::FROZEN_SHA256;
        let receipt = qualified_external_receipt("alice", sha);
        let source = bound_source(sha);
        let capture = terminal_capture("alice");

        // The terminal line and the witness must be the same record.
        let mut other = receipt.clone();
        other["capture_requested"] = serde_json::json!(false);
        let receipts = parse(&format!(
            "EXTERNAL_LOADER: script_external_loader {receipt}\nPASS: live script_external_loader {other}\n"
        ));
        match super::validate(
            &case,
            &receipts,
            Some(0),
            std::slice::from_ref(&capture),
            Some(&source),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("disagree"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // A qualified record that does not confirm the capture request, is not the qualified
        // stage, or reports another source is refused field by field.
        for (mutate, needle) in [
            ("capture_requested", "capture request"),
            ("stage", "expected \"qualified\""),
            ("registration_count_after_reload", "after reload"),
            ("auto_start", "auto-started"),
            ("stop_elapsed_ms", "Stop took"),
            ("reload_unchanged", "unchanged reload"),
            ("initial_bones", "25 bone fixture"),
            ("card_key", "not the reloaded card key"),
            ("stale_card_key", "not the reloaded card key"),
            ("source_after", "harmless whitespace digest"),
            ("compiled_after", "hashes disagree"),
            ("burial_counters", "counters disagree"),
        ] {
            let mut broken = receipt.clone();
            match mutate {
                "capture_requested" => broken["capture_requested"] = serde_json::json!(false),
                "stage" => broken["stage"] = serde_json::json!("capture"),
                "registration_count_after_reload" => {
                    broken["registration_count_after_reload"] = serde_json::json!(2)
                }
                "auto_start" => broken["auto_start"] = serde_json::json!(true),
                "stop_elapsed_ms" => broken["stop_elapsed_ms"] = serde_json::json!(10_000),
                "reload_unchanged" => {
                    broken["reload_unchanged"] = serde_json::json!("something else")
                }
                "initial_bones" => broken["initial"]["bones"] = serde_json::json!(10),
                "card_key" => {
                    broken["script"]["compiled_sha"] = serde_json::json!(digest("another source"))
                }
                "stale_card_key" => {
                    // The load-time card key (the bound raw source digest) is not the loaded-card
                    // key of a qualified record: the changed reload moved it, so claiming the
                    // load-time digest means the reload's identity never moved.
                    broken["script"]["compiled_sha"] =
                        serde_json::json!(host_play::external_loader::FROZEN_SHA256)
                }
                "source_after" => {
                    broken["source_sha_after"] = serde_json::json!(digest("another source"))
                }
                "compiled_after" => {
                    broken["compiled_sha_after"] = serde_json::json!(digest("another source"))
                }
                "burial_counters" => broken["final"]["burial_logs"] = serde_json::json!(9),
                other => panic!("unhandled mutation {other}"),
            }
            let receipts = parse(&external_output(&broken, "PASS"));
            match super::validate(
                &case,
                &receipts,
                Some(0),
                std::slice::from_ref(&capture),
                Some(&source),
            ) {
                Verdict::SharedFailure { reason, .. } => {
                    assert!(reason.contains(needle), "{mutate}: {reason}")
                }
                other => panic!("{mutate} -> {other:?}"),
            }
        }

        // A hash field that is not a digest at all is refused, whatever it claims.
        let mut arbitrary = receipt.clone();
        arbitrary["source_sha_after"] = serde_json::json!("not-a-hash");
        let receipts = parse(&external_output(&arbitrary, "PASS"));
        match super::validate(
            &case,
            &receipts,
            Some(0),
            std::slice::from_ref(&capture),
            Some(&source),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("lowercase SHA-256 digest"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // The receipt's source hash is the *bound* input, not whatever the child claims.
        let mut unbound = receipt.clone();
        unbound["script"]["sha256"] = serde_json::json!("not-the-bound-source");
        let receipts = parse(&external_output(&unbound, "PASS"));
        match super::validate(
            &case,
            &receipts,
            Some(0),
            std::slice::from_ref(&capture),
            Some(&source),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("not the bound source sha"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // No bound source at all is a suite bug, and it is refused rather than skipped.
        let receipts = parse(&external_output(&receipt, "PASS"));
        match super::validate(&case, &receipts, Some(0), &[capture], None) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("bound no external loader source"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        // A PASS that exited nonzero, and a core witness on an external case, are shared
        // failures: the run must not read either as success.
        let receipts = parse(&external_output(&receipt, "PASS"));
        assert!(matches!(
            super::validate(&case, &receipts, Some(1), &[], Some(&source)),
            Verdict::SharedFailure { .. }
        ));
        let receipts = parse(&format!(
            "PASS: live script_external_loader {receipt}\n\
             EXTERNAL_LOADER: script_external_loader {receipt}\n\
             CATALOG_CORE: script_external_loader {{\"case\":\"bone_burier\"}}\n"
        ));
        match super::validate(&case, &receipts, Some(0), &[], Some(&source)) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("CATALOG_CORE"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_failed_external_receipt_preserves_its_stage_request_completion_and_cleanup() {
        let case = external_case();
        let receipt = failed_external_receipt("alice");
        let receipts = parse(&external_output(&receipt, "FAIL"));
        match super::validate(
            &case,
            &receipts,
            Some(1),
            &[],
            Some(&bound_source(host_play::external_loader::FROZEN_SHA256)),
        ) {
            Verdict::CaseFailure { reason } => {
                assert!(reason.contains("failed at stage failed"), "{reason}");
                assert!(reason.contains("fixture did not reach scene 2"), "{reason}");
                assert!(reason.contains("requested prepare_fixture"), "{reason}");
                assert!(reason.contains("completed -"), "{reason}");
                assert!(reason.contains("cleanup -"), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // A FAIL receipt with exit 0 is contradictory, and a FAIL without a reason is refused.
        let receipts = parse(&external_output(&receipt, "FAIL"));
        assert!(matches!(
            super::validate(
                &case,
                &receipts,
                Some(0),
                &[],
                Some(&bound_source(host_play::external_loader::FROZEN_SHA256))
            ),
            Verdict::SharedFailure { .. }
        ));
        let mut silent = receipt.clone();
        silent["failure_reason"] = serde_json::Value::Null;
        let receipts = parse(&external_output(&silent, "FAIL"));
        match super::validate(
            &case,
            &receipts,
            Some(1),
            &[],
            Some(&bound_source(host_play::external_loader::FROZEN_SHA256)),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("no failure reason"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_external_capture_must_be_the_terminal_shot_of_the_receipts_actor() {
        let case = external_case();
        let sha = host_play::external_loader::FROZEN_SHA256;
        let receipt = qualified_external_receipt("alice", sha);
        let receipts = parse(&external_output(&receipt, "PASS"));
        let source = bound_source(sha);

        // The prerequisite capture alone cannot discharge the contract.
        match super::validate(
            &case,
            &receipts,
            Some(0),
            &[prereq_capture()],
            Some(&source),
        ) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("external_loader terminal"), "{reason}");
                assert!(reason.contains("not written by this case"), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // A capture of another actor is not this case's terminal evidence.
        match super::validate(
            &case,
            &receipts,
            Some(0),
            &[terminal_capture("bob")],
            Some(&source),
        ) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("names actor \"bob\""), "{reason}");
                assert!(reason.contains("\"alice\""), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // A sidecar without the producer's actor binding is refused too.
        let mut unbound = terminal_capture("alice");
        unbound.sidecar_actor = None;
        match super::validate(&case, &receipts, Some(0), &[unbound], Some(&source)) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("no actor binding"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // A structurally broken PNG beside the right label is still not evidence.
        let mut torn = terminal_capture("alice");
        torn.png_magic = false;
        torn.decoded = false;
        torn.png_bytes = 4;
        match super::validate(&case, &receipts, Some(0), &[torn], Some(&source)) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("not a PNG"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // And no capture at all is the contracted-capture failure, never a pass.
        match super::validate(&case, &receipts, Some(0), &[], Some(&source)) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("was not written by this case"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_core_case_may_not_present_the_external_witness() {
        let case = core_case("thiever");
        let receipts = parse(&format!(
            "{}\
             EXTERNAL_LOADER: script_thiever {{\"stage\":\"qualified\"}}\n",
            panel_core_pass("script_thiever", "thiever")
        ));
        match validate(&case, &receipts, Some(0), &[capture_record()]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("EXTERNAL_LOADER"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    /// A real panel PASS line: the outer name is the live name (`script_*`), the inner
    /// evidence names the scenario, and the witness case is the snake_case wire form.
    fn panel_core_pass(live: &str, scenario: &str) -> String {
        let wire = wire_case(scenario, RunnerKind::Core).unwrap();
        format!(
            "PASS: live {live} {{\"scenario\":\"{scenario}\",\"outcome\":\"PASS\",\"scene\":2}}\n\
             CATALOG_CORE: {live} {{\"case\":\"{wire}\",\"post_start_observations\":2}}\n"
        )
    }

    #[test]
    fn wire_identities_come_from_the_host_enums() {
        assert_eq!(
            wire_case("thiever", RunnerKind::Core).unwrap(),
            "thiever",
            "CoreCase::Thiever serializes snake_case"
        );
        assert_eq!(
            wire_case("nature_crafter_air", RunnerKind::Pair).unwrap(),
            "air"
        );
        assert_eq!(
            wire_case("mule_crafter_air", RunnerKind::Pair).unwrap(),
            "mule"
        );
        assert!(wire_case("not_a_scenario", RunnerKind::Core).is_err());
        // The manifest's declared identities agree with the enum wire form.
        let manifest = manifest();
        for case in manifest.cases.iter().filter(|case| case.is_runnable()) {
            if case.runner() == RunnerKind::External {
                continue; // the loader smoke has no host-enum witness to derive
            }
            let scenario = case.scenario.as_deref().unwrap();
            assert_eq!(
                wire_case(scenario, case.runner()).unwrap(),
                match case.runner() {
                    RunnerKind::Core => case.core_case.clone().unwrap(),
                    RunnerKind::Pair => case.pair_case.clone().unwrap(),
                    // Filtered out above: the loader smoke has no host-enum witness identity.
                    RunnerKind::External => {
                        unreachable!("external rows continue before the witness identity check")
                    }
                },
                "{scenario}"
            );
        }
    }

    #[test]
    fn parses_terminal_receipts_without_a_permissive_pass_regex() {
        let output = "\
live script_thiever: running step 1/2
PASS: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"PASS\"}
CATALOG_CORE: script_thiever {\"case\":\"thiever\",\"post_start_observations\":2}
[panel] shot thiever paint -> /tmp/shots/run/thiever_paint.png
";
        let receipts = parse(output);
        assert!(receipts.has_scenario_terminal());
        let pass = receipts.scenario_pass.as_ref().unwrap();
        assert_eq!(pass.name, "script_thiever");
        assert_eq!(evidence_scenario(pass), Some("thiever"));
        assert_eq!(receipts.core.as_ref().unwrap().name, "script_thiever");
        assert_eq!(receipts.shot_lines.len(), 1);
        assert!(receipts.duplicates.is_empty());

        // "PASS" as a bare word is not a receipt.
        assert!(!parse("PASS\n").has_scenario_terminal());
        assert!(!parse("all good, PASS(ed) the case\n").has_scenario_terminal());
    }

    #[test]
    fn the_outer_live_name_and_the_inner_scenario_are_checked_separately() {
        let case = core_case("thiever");
        // The old invented shape (outer name = scenario, PascalCase witness) must fail.
        let invented = parse(
            "PASS: live thiever {\"scenario\":\"thiever\"}\nCATALOG_CORE: thiever {\"case\":\"Thiever\"}\n",
        );
        match validate(&case, &invented, Some(0), &[]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("expected \"script_thiever\""), "{reason}");
            }
            other => panic!("{other:?}"),
        }

        // A PascalCase witness under the right outer name fails on the wire form.
        let pascal = parse(
            &panel_core_pass("script_thiever", "thiever")
                .replace("\"case\":\"thiever\"", "\"case\":\"Thiever\""),
        );
        match validate(&case, &pascal, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"thiever\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // Inner evidence naming another scenario fails too.
        let wrong_inner = parse(
            &panel_core_pass("script_thiever", "thiever")
                .replace("\"scenario\":\"thiever\"", "\"scenario\":\"alcher\""),
        );
        match validate(&case, &wrong_inner, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("evidence names scenario"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_real_panel_core_pass_is_valid_and_keeps_the_shots_contract() {
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        // No capture attributed: thiever's scenario declares a terminal shot, so a PASS
        // without one cannot qualify.
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("thiever paint"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
        let shot = capture_record();
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[shot]),
            Verdict::PendingVisualReview { .. }
        ));
    }

    #[test]
    fn shared_signals_are_detected_from_child_output() {
        let receipts = parse("FATAL: engine unavailable\n");
        assert_eq!(receipts.shared_signals, vec!["engine unavailable"]);
    }

    #[test]
    fn zero_exit_without_a_receipt_is_a_shared_receipt_failure() {
        let case = core_case("thiever");
        let receipts = parse("live script_thiever: running step 1/2\n");
        let verdict = validate(&case, &receipts, Some(0), &[]);
        match verdict {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("no terminal scenario receipt"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn exit_zero_with_a_fail_receipt_is_contradictory() {
        let case = core_case("thiever");
        let receipts =
            parse("FAIL: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"FAIL\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));
    }

    #[test]
    fn a_dual_terminal_receipt_is_a_shared_failure() {
        let case = core_case("thiever");
        let receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"PASS\"}\n\
             FAIL: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"FAIL\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"thiever\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[capture_record()]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "receipt");
                assert!(reason.contains("both a PASS and a FAIL"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn assertion_failure_is_case_local_and_keeps_the_message() {
        let case = core_case("thiever");
        let receipts = parse(
            "FAIL: live script_thiever {\"scenario\":\"thiever\",\"message\":\"no Coins gained\"}\n",
        );
        match validate(&case, &receipts, Some(1), &[]) {
            Verdict::CaseFailure { reason } => {
                assert!(reason.contains("no Coins gained"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pair_failure_uses_qualification_not_prerequisite_outcome_pass() {
        let case = core_case("nature_crafter_air");
        let receipts = parse(
            "PAIRED_CORE: script_nature_crafter_air {\"phase\":\"running\",\"witness\":{\"case\":\"air\",\"master\":{\"settings\":{}},\"runner\":{\"settings\":{}}},\"qualification\":\"pair core did not qualify before the headed deadline: one-sided confirmation: both actors never observed the offer phase with the partner\"}\n\
             FAIL: live script_nature_crafter_air {\"scenario\":\"nature_crafter_air\",\"outcome\":\"PASS\",\"predicate\":\"stat(16)>=0\",\"ticks\":2}\n",
        );
        assert!(
            receipts.scenario_fail.is_some(),
            "scenario FAIL record is retained"
        );
        assert!(receipts.pair.is_some(), "PAIRED_CORE record is retained");
        match validate(&case, &receipts, Some(1), &[]) {
            Verdict::CaseFailure { reason } => {
                assert!(
                    reason.contains("one-sided confirmation"),
                    "pair qualification must be reported: {reason}"
                );
                assert!(
                    !reason.contains("outcome PASS"),
                    "prerequisite scenario PASS must not mask the pair error: {reason}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn native_core_failure_message_beats_prior_scenario_pass_evidence() {
        let case = core_case("thiever");
        let receipts = parse(
            "FAIL: live script_thiever {\"scenario\":\"thiever\",\"outcome\":\"FAIL\",\"message\":\"catalog core did not qualify before the headed deadline: rock_crab core post-Start delta incomplete\",\"prerequisite\":{\"scenario\":\"thiever\",\"outcome\":\"PASS\"}}\n",
        );
        match validate(&case, &receipts, Some(1), &[]) {
            Verdict::CaseFailure { reason } => {
                assert!(reason.contains("rock_crab core post-Start delta incomplete"));
                assert!(!reason.contains("outcome PASS"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn missing_or_wrong_witness_stops_the_run() {
        let case = core_case("thiever");
        let mut receipts = parse("PASS: live script_thiever {\"scenario\":\"thiever\"}\n");
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure {
                kind: "receipt",
                ..
            }
        ));

        receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"bank_fletcher\"}\n",
        );
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"thiever\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        // A core case must not present a pair witness.
        receipts = parse(
            "PASS: live script_thiever {\"scenario\":\"thiever\"}\n\
             CATALOG_CORE: script_thiever {\"case\":\"thiever\"}\n\
             PAIRED_CORE: script_thiever {\"case\":\"air\"}\n",
        );
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::SharedFailure { .. }
        ));
    }

    #[test]
    fn duplicate_terminal_receipts_are_rejected() {
        let case = core_case("thiever");
        let line = "PASS: live script_thiever {\"scenario\":\"thiever\"}\n";
        let receipts = parse(&format!("{line}{line}"));
        match validate(&case, &receipts, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("duplicate"), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn pair_cases_require_their_own_witness_under_the_live_name() {
        let case = core_case("nature_crafter_air");
        let receipts = parse(
            "PASS: live script_nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\n\
             PAIRED_CORE: script_nature_crafter_air {\"case\":\"air\"}\n",
        );
        assert!(matches!(
            validate(&case, &receipts, Some(0), &[pair_capture_record()]),
            Verdict::PendingVisualReview { .. }
        ));

        let wrong = parse(
            "PASS: live script_nature_crafter_air {\"scenario\":\"nature_crafter_air\"}\n\
             PAIRED_CORE: script_nature_crafter_air {\"case\":\"mule\"}\n",
        );
        match validate(&case, &wrong, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("expected \"air\""), "{reason}")
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn duel_pass_requires_the_native_claim() {
        use host_play::paired_core::{
            DuelObservation, DuelPairWitness, DuelSlotRecord, DUEL_CHALLENGE_ANCHOR,
        };

        let case = core_case("duel_arena");
        let first = duel_runtime_evidence(&duel_first_combat_pair(), false);
        let missing = parse(
            "PASS: live script_duel_arena {\"scenario\":\"duel_arena\"}\n\
             PAIRED_CORE: script_duel_arena {\"case\":\"duel\"}\n",
        );
        match validate(&case, &missing, Some(0), &[]) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("missing actor") || reason.contains("claim"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let mut queued_json = first.clone();
        queued_json["claim"] = serde_json::json!("queued-challenge");
        let queued = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {queued_json}\n"
        ));
        match validate(
            &case,
            &queued,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("queued-challenge"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        let receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {first}\n"
        ));
        assert!(matches!(
            validate(
                &case,
                &receipts,
                Some(0),
                &duel_actor_captures("alice", "bob")
            ),
            Verdict::PendingVisualReview { .. }
        ));

        match validate(&case, &receipts, Some(0), &[duel_actor_capture("alice")]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(
                    reason.contains("both actors") || reason.contains("found 1"),
                    "{reason}"
                );
            }
            other => panic!("{other:?}"),
        }

        let mut unbound = duel_actor_capture("alice");
        unbound.sidecar_actor = None;
        match validate(
            &case,
            &receipts,
            Some(0),
            &[unbound, duel_actor_capture("bob")],
        ) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(
                    reason.contains("no actor binding") || reason.contains("expected"),
                    "{reason}"
                );
            }
            other => panic!("{other:?}"),
        }

        match validate(
            &case,
            &receipts,
            Some(0),
            &[duel_actor_capture("alice"), duel_actor_capture("stranger")],
        ) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(
                    reason.contains("stranger") || reason.contains("bob"),
                    "{reason}"
                );
            }
            other => panic!("{other:?}"),
        }

        let mut forged = first.clone();
        forged["claim"] = serde_json::json!("first-combat");
        forged["a"]["saw_combat"] = serde_json::json!(false);
        forged["b"]["saw_combat"] = serde_json::json!(false);
        let unsupported = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {forged}\n"
        ));
        match validate(
            &case,
            &unsupported,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("offer/confirm/pen/combat")
                        || reason.contains("not backed")
                        || reason.contains("in-combat"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let mut seeded_offer = first.clone();
        seeded_offer["a"]["baseline"]["duel_offer_open"] = serde_json::json!(true);
        let seeded_offer_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {seeded_offer}\n"
        ));
        match validate(
            &case,
            &seeded_offer_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("seeded modal") || reason.contains("baseline"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let mut seeded_confirm = first.clone();
        seeded_confirm["a"]["baseline"]["duel_confirm_open"] = serde_json::json!(true);
        let seeded_confirm_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {seeded_confirm}\n"
        ));
        match validate(
            &case,
            &seeded_confirm_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("seeded modal") || reason.contains("baseline"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let mut missing_identity = first.clone();
        missing_identity["a"]
            .as_object_mut()
            .expect("actor a")
            .remove("mixed_identity");
        let missing_identity_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {missing_identity}\n"
        ));
        match validate(
            &case,
            &missing_identity_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("mixed_identity"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        let mut invalid_identity = first.clone();
        invalid_identity["a"]["mixed_identity"] = serde_json::json!("yes");
        let invalid_identity_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {invalid_identity}\n"
        ));
        match validate(
            &case,
            &invalid_identity_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("mixed_identity") || reason.contains("invalid type"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let mut missing_partner = first.clone();
        missing_partner["b"]
            .as_object_mut()
            .expect("actor b")
            .remove("saw_wrong_partner");
        let missing_partner_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {missing_partner}\n"
        ));
        match validate(
            &case,
            &missing_partner_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(reason.contains("saw_wrong_partner"), "{reason}")
            }
            other => panic!("{other:?}"),
        }

        let mut invalid_partner = first.clone();
        invalid_partner["b"]["saw_wrong_partner"] = serde_json::json!(1);
        let invalid_partner_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {invalid_partner}\n"
        ));
        match validate(
            &case,
            &invalid_partner_receipts,
            Some(0),
            &duel_actor_captures("alice", "bob"),
        ) {
            Verdict::SharedFailure { reason, .. } => {
                assert!(
                    reason.contains("saw_wrong_partner") || reason.contains("invalid type"),
                    "{reason}"
                )
            }
            other => panic!("{other:?}"),
        }

        let full = duel_runtime_evidence(&duel_reset_pair(), true);
        assert_eq!(full["claim"], "reset-and-further");
        assert_eq!(full["full"], "reset-and-further");
        assert_eq!(full["supported"], "first-combat");
        let full_receipts = parse(&format!(
            "PASS: live script_duel_arena {{\"scenario\":\"duel_arena\"}}\n\
             PAIRED_CORE: script_duel_arena {full}\n"
        ));
        assert!(matches!(
            validate(
                &case,
                &full_receipts,
                Some(0),
                &duel_actor_captures("alice", "bob")
            ),
            Verdict::PendingVisualReview { .. }
        ));

        fn duel_obs(player: &str) -> DuelObservation {
            DuelObservation {
                ingame: true,
                scene_state: 2,
                inventory_tab_available: true,
                player: Some(player.into()),
                tile: Some(DUEL_CHALLENGE_ANCHOR),
                tick: 0,
                attack_xp: 0,
                strength_xp: 0,
                defence_xp: 0,
                hitpoints_xp: 0,
                in_combat: false,
                in_challenge_area: true,
                in_fight_pen: false,
                main_modal: -1,
                duel_offer_open: false,
                duel_confirm_open: false,
                duel_win_open: false,
                duel_partner: None,
                waiting_for_other: false,
                weapon_equipped: true,
                peer_visible: true,
            }
        }

        fn duel_first_combat_pair() -> DuelPairWitness {
            let mut a = DuelSlotRecord::new(
                "alice".into(),
                "alice".into(),
                "bob".into(),
                serde_json::Map::new(),
                duel_obs("alice"),
            );
            let mut b = DuelSlotRecord::new(
                "bob".into(),
                "bob".into(),
                "alice".into(),
                serde_json::Map::new(),
                duel_obs("bob"),
            );
            a.post_start = 8;
            b.post_start = 8;
            a.saw_offer = true;
            b.saw_offer = true;
            a.saw_confirm = true;
            b.saw_confirm = true;
            a.saw_pen = true;
            b.saw_pen = true;
            a.saw_combat = true;
            b.saw_combat = true;
            a.melee_xp_from_script = 12;
            b.melee_xp_from_script = 8;
            DuelPairWitness { a, b }
        }

        fn duel_reset_pair() -> DuelPairWitness {
            let mut pair = duel_first_combat_pair();
            pair.a.saw_win_or_lobby_return = true;
            pair.a.further_combat = true;
            pair
        }

        fn duel_runtime_evidence(pair: &DuelPairWitness, full_cycle: bool) -> Value {
            assert!(pair.qualify_supported().is_ok());
            if full_cycle {
                assert!(pair.qualify_full_cycle().is_ok());
            } else {
                assert!(pair.qualify_full_cycle().is_err());
            }
            serde_json::json!({
                "case": "duel",
                "claim": pair.qualify_full_cycle().ok().or_else(|| pair.qualify_supported().ok()),
                "supported": pair.qualify_supported().ok(),
                "full": pair.qualify_full_cycle().ok(),
                "a": pair.a,
                "b": pair.b,
            })
        }
    }
    fn capture_record() -> CaptureRecord {
        CaptureRecord {
            label: "2026-09-14T00-00-01_thiever_paint".into(),
            png: "/tmp/a.png".into(),
            json: "/tmp/a.json".into(),
            png_bytes: 128,
            png_magic: true,
            png_sha256: "digest".into(),
            decoded: true,
            png_width: 2,
            png_height: 2,
            sidecar_ok: true,
            sidecar_ingame: Some(true),
            sidecar_scene_state: Some(2),
            sidecar_actor: None,
            modified_ms: 1,
        }
    }

    fn pair_capture_record() -> CaptureRecord {
        CaptureRecord {
            label: "2026-09-14T00-00-01_nature_crafter_air-alice".into(),
            ..capture_record()
        }
    }

    fn duel_actor_capture(actor: &str) -> CaptureRecord {
        CaptureRecord {
            label: format!("2026-09-14T00-00-01_duel_arena-{actor}"),
            sidecar_actor: Some(actor.into()),
            ..capture_record()
        }
    }

    fn duel_actor_captures(a: &str, b: &str) -> [CaptureRecord; 2] {
        [duel_actor_capture(a), duel_actor_capture(b)]
    }

    #[test]
    fn a_capture_that_is_not_the_terminal_shot_evidence_cannot_pass() {
        let mut broken = capture_record();
        broken.decoded = false;
        broken.png_magic = false;
        broken.png_bytes = 4;
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        match validate(&case, &receipts, Some(0), &[broken]) {
            Verdict::SharedFailure { kind, reason } => {
                assert_eq!(kind, "capture");
                assert!(reason.contains("not a PNG"), "{reason}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_capture_taken_outside_the_world_or_off_scene_two_is_not_evidence() {
        let case = core_case("thiever");
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        let off_world = {
            let mut record = capture_record();
            record.sidecar_ingame = Some(false);
            record
        };
        let off_scene = {
            let mut record = capture_record();
            record.sidecar_scene_state = Some(1);
            record
        };
        let no_scene = {
            let mut record = capture_record();
            record.sidecar_scene_state = None;
            record
        };
        let no_sidecar = {
            let mut record = capture_record();
            record.sidecar_ok = false;
            record
        };
        let empty_frame = {
            let mut record = capture_record();
            record.png_height = 0;
            record
        };
        for (name, needle, record) in [
            ("ingame=false", "not in game", off_world),
            ("scene_state=1", "scene_state 1", off_scene),
            ("scene_state absent", "no scene_state", no_scene),
            ("sidecar missing", "sidecar is missing", no_sidecar),
            ("empty frame", "empty frame", empty_frame),
        ] {
            match validate(&case, &receipts, Some(0), &[record]) {
                Verdict::SharedFailure { kind, reason } => {
                    assert_eq!(kind, "capture");
                    assert!(reason.contains(needle), "{name} not in: {reason}");
                }
                other => panic!("{name} -> {other:?}"),
            }
        }
    }

    #[test]
    fn captures_are_attributed_by_content_magic_and_sidecar_evidence() {
        let dir = std::env::temp_dir().join(format!("274bot-suite-caps-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let run = dir.join("2026-09-14T00-00-00_1234");
        std::fs::create_dir_all(&run).unwrap();
        let png = run.join("2026-09-14T00-00-01_thiever_paint.png");
        std::fs::write(&png, tiny_png()).unwrap();
        std::fs::write(
            run.join("2026-09-14T00-00-01_thiever_paint.json"),
            "{\"ingame\":true,\"scene_state\":2}",
        )
        .unwrap();
        std::fs::write(run.join("2026-09-14T00-00-01_other.png"), tiny_png()).unwrap();

        let found = find_captures(&dir, "thiever paint");
        assert_eq!(found.len(), 1);
        assert!(found[0].complete(), "{:?}", found[0].structural_reason());
        assert_eq!(found[0].sidecar_ingame, Some(true));
        assert_eq!(found[0].png_width, 1);
        assert_eq!(found[0].png_sha256.len(), 64);
        assert!(find_captures(&dir, "absent label").is_empty());

        // A torn PNG beside a valid sidecar is not a capture.
        std::fs::write(&png, b"\x89PNG\r\n\x1a\n truncated").unwrap();
        let broken = find_captures(&dir, "thiever paint");
        assert_eq!(broken.len(), 1);
        assert!(!broken[0].complete());
        assert!(
            broken[0]
                .structural_reason()
                .unwrap()
                .contains("did not decode"),
            "{:?}",
            broken[0].structural_reason()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn only_captures_written_during_the_case_are_attributed() {
        use std::collections::BTreeMap;
        let mut before = BTreeMap::new();
        let mut earlier = capture_record();
        earlier.label = "2026-09-14T00-00-01_thiever_paint".to_string();
        before.insert(earlier.label.clone(), earlier.clone());
        let mut after = before.clone();
        let mut later = capture_record();
        later.label = "2026-09-14T00-01-00_thiever_paint".to_string();
        after.insert(later.label.clone(), later.clone());
        let new = new_captures(&before, &after);
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].label, "2026-09-14T00-01-00_thiever_paint");
        assert!(
            new_captures(&after, &after).is_empty(),
            "a same-named capture from an earlier case is never reused"
        );
    }

    #[test]
    fn an_optional_capture_that_was_not_requested_leaves_the_receipt_passed() {
        let mut case = core_case("thiever");
        case.capture = Some(CaptureSpec {
            label: "thiever paint".into(),
            required: false,
        });
        let receipts = parse(&panel_core_pass("script_thiever", "thiever"));
        assert_eq!(
            validate(&case, &receipts, Some(0), &[]),
            Verdict::Passed,
            "a declared-but-optional capture does not turn a receipt into a failure"
        );
    }

    /// A real 1x1 RGBA PNG.
    fn tiny_png() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00,
            0x00, 0x1f, 0x15, 0xc4, 0x89, 0x00, 0x00, 0x00, 0x0a, 0x49, 0x44, 0x41, 0x54, 0x78,
            0x9c, 0x63, 0x00, 0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0d, 0x0a, 0x2d, 0xb4, 0x00,
            0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }