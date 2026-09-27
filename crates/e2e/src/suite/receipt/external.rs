use super::*;
/// The dedicated external loader smoke's terminal contract.
///
/// The producer prints its own machine-readable record instead of a scenario receipt, and the
/// terminal `PASS`/`FAIL` line carries the *same* payload as the `EXTERNAL_LOADER:` witness
/// (`live_line = record_ext()` in the panel), so the suite compares the two rather than trusting
/// either alone. Every field that establishes the native contract is derived from the production
/// constants and the bound input identity: an exit code, a bare `PASS` string, a fixture-only
/// counter or an earlier prerequisite capture is never success.
pub(super) fn validate_external(
    case: &CaseEntry,
    receipts: &ParsedReceipts,
    exit_code: Option<i32>,
    captures: &[CaptureRecord],
    external: Option<&ExternalSource>,
) -> Verdict {
    let shared = |reason: String| Verdict::SharedFailure {
        kind: "receipt",
        reason,
    };
    let live = case.live.as_deref().unwrap_or_default();
    if receipts.scenario_pass.is_some() && receipts.scenario_fail.is_some() {
        return shared(format!(
            "{live} printed both a PASS and a FAIL terminal receipt ({} / {})",
            receipts
                .scenario_pass
                .as_ref()
                .map(summarize)
                .unwrap_or_default(),
            receipts
                .scenario_fail
                .as_ref()
                .map(summarize)
                .unwrap_or_default(),
        ));
    }
    let Some(witness) = &receipts.external else {
        return shared(format!(
            "no EXTERNAL_LOADER witness for {live} (exit {})",
            exit_text(exit_code)
        ));
    };
    if witness.name != live {
        return shared(format!(
            "EXTERNAL_LOADER witness names {:?}, expected {live:?}",
            witness.name
        ));
    }
    let Some(payload) = witness
        .payload
        .as_ref()
        .filter(|payload| payload.is_object())
    else {
        return shared(format!(
            "{live} EXTERNAL_LOADER witness is not a JSON object"
        ));
    };
    if receipts.core.is_some() || receipts.pair.is_some() {
        return shared(format!(
            "{live} is the external loader smoke but printed a CATALOG_CORE/PAIRED_CORE witness"
        ));
    }
    match (
        receipts.scenario_pass.as_ref(),
        receipts.scenario_fail.as_ref(),
    ) {
        (None, None) => shared(format!(
            "no terminal external receipt for {live} (exit {})",
            exit_text(exit_code)
        )),
        (Some(pass), None) => {
            if pass.name != live {
                return shared(format!(
                    "PASS receipt names {:?}, expected {live:?}",
                    pass.name
                ));
            }
            if exit_code != Some(0) {
                return shared(format!(
                    "{live} printed a PASS receipt but exited {}",
                    exit_text(exit_code)
                ));
            }
            match pass.payload.as_ref().filter(|payload| payload.is_object()) {
                Some(line) if line == payload => {}
                Some(_) => {
                    return shared(format!(
                        "{live} PASS payload and EXTERNAL_LOADER witness disagree"
                    ))
                }
                None => return shared(format!("{live} PASS evidence is not a JSON object")),
            }
            let account = match qualified_external_account(payload, external) {
                Ok(account) => account,
                Err(reason) => return shared(format!("{live} {reason}")),
            };
            external_capture_verdict(case, captures, &account)
        }
        (None, Some(fail)) => {
            if fail.name != live {
                return shared(format!(
                    "FAIL receipt names {:?}, expected {live:?}",
                    fail.name
                ));
            }
            if exit_code == Some(0) {
                return shared(format!("{live} printed a FAIL receipt but exited 0"));
            }
            match fail.payload.as_ref().filter(|payload| payload.is_object()) {
                Some(line) if line == payload => {}
                Some(_) => {
                    return shared(format!(
                        "{live} FAIL payload and EXTERNAL_LOADER witness disagree"
                    ))
                }
                None => return shared(format!("{live} FAIL receipt is not a JSON object")),
            }
            match external_failure_detail(payload) {
                Ok(detail) => Verdict::CaseFailure { reason: detail },
                Err(reason) => shared(format!("{live} {reason}")),
            }
        }
        // The dual-terminal case returned above.
        (Some(_), Some(_)) => unreachable!("checked above"),
    }
}
/// The qualified external receipt, validated field by field against the production constants
/// and the bound source identity. Returns the run's account, which the capture must also name.
fn qualified_external_account(
    payload: &Value,
    external: Option<&ExternalSource>,
) -> Result<String, String> {
    use host_play::external_loader::{
        Operation, Stage, BONES_COUNT, MIN_DISTINCT_BURIALS, SCRIPT_NAME, START_DEADLINE,
        STOP_DEADLINE,
    };
    let Some(external) = external else {
        return Err(
            "bound no external loader source; the suite cannot validate the receipt's source identity"
                .into(),
        );
    };
    let expected_stage = wire(&Stage::Qualified);
    let stage = payload
        .get("stage")
        .and_then(Value::as_str)
        .unwrap_or_default();
    require(
        stage == expected_stage,
        format!("reports external stage {stage:?}, expected {expected_stage:?}"),
    )?;
    require(
        matches!(payload.get("failure_reason"), None | Some(Value::Null)),
        "qualified external receipt carries a failure reason",
    )?;
    require(
        matches!(payload.get("cleanup_outcome"), None | Some(Value::Null)),
        "qualified external receipt carries a cleanup outcome",
    )?;
    require(
        payload.get("capture_requested").and_then(Value::as_bool) == Some(true),
        "qualified external receipt does not confirm the terminal capture request",
    )?;
    let capture = wire(&Operation::Capture);
    require(
        payload.get("requested_operation").and_then(Value::as_str) == Some(capture.as_str()),
        format!(
            "qualified external receipt last requested {:?}, expected {capture:?}",
            payload.get("requested_operation")
        ),
    )?;
    require(
        payload.get("completed_operation").and_then(Value::as_str) == Some(capture.as_str()),
        format!(
            "qualified external receipt last completed {:?}, expected {capture:?}",
            payload.get("completed_operation")
        ),
    )?;

    let script = payload
        .get("script")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified external receipt carries no script record".to_string())?;
    require(
        script.get("name").and_then(Value::as_str) == Some(SCRIPT_NAME),
        format!(
            "external receipt registered script {:?}, expected {SCRIPT_NAME:?}",
            script.get("name")
        ),
    )?;
    require(
        script.get("version").and_then(Value::as_str)
            == Some(host_play::external_loader::SCRIPT_VERSION),
        format!(
            "external receipt script version {:?}, expected {:?}",
            script.get("version"),
            host_play::external_loader::SCRIPT_VERSION
        ),
    )?;
    require(
        script.get("sha256").and_then(Value::as_str) == Some(external.sha256.as_str()),
        format!(
            "external receipt script sha256 {:?} is not the bound source sha {}",
            script.get("sha256"),
            external.sha256
        ),
    )?;
    // `compiled_sha` is the loaded raw File card's *cache key* — the origin raw-source digest,
    // not a compiled-artifact hash — and the producer overwrites it with the reloaded card's key
    // when the changed reload lands (`note_reload_changed`). The surviving load-time identity in
    // a qualified record is therefore `script.sha256`: the raw source the watch was configured
    // with, checked against the bound input just above. The loaded-card key itself is checked
    // against the reloaded key once the changed-reload identities below are read.
    for key in ["sha256", "compiled_sha"] {
        let value = script.get(key).and_then(Value::as_str).unwrap_or_default();
        require(
            is_sha256(value),
            format!("external receipt {key} {value:?} is not a lowercase SHA-256 digest"),
        )?;
    }
    require(
        script
            .get("identity_key")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty()),
        "external receipt carries no card identity for the loaded script",
    )?;
    require(
        payload.get("registration_count").and_then(Value::as_u64) == Some(1),
        format!(
            "external receipt registration count {:?}, expected 1",
            payload.get("registration_count")
        ),
    )?;
    require(
        payload
            .get("registration_count_after_reload")
            .and_then(Value::as_u64)
            == Some(1),
        format!(
            "external receipt registration count after reload {:?}, expected 1",
            payload.get("registration_count_after_reload")
        ),
    )?;

    let gate = payload
        .get("scene_gate")
        .and_then(Value::as_object)
        .ok_or_else(|| "qualified external receipt carries no scene gate".to_string())?;
    require(
        gate.get("ingame").and_then(Value::as_bool) == Some(true)
            && gate.get("scene_state").and_then(Value::as_i64) == Some(INGAME_SCENE_STATE as i64),
        format!(
            "external receipt scene gate is {gate:?}, expected in game at scene {INGAME_SCENE_STATE}"
        ),
    )?;

    let counters = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_object)
            .ok_or_else(|| format!("qualified external receipt carries no {name} counters"))
    };
    let initial = counters("initial")?;
    let last = counters("final")?;
    let counter = |section: &str, fields: &serde_json::Map<String, Value>, key: &str| {
        fields
            .get(key)
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0)
            .ok_or_else(|| {
                format!("external receipt {section}.{key} must be a nonnegative integer")
            })
    };
    let initial_bones = counter("initial", initial, "bones")?;
    let final_bones = counter("final", last, "bones")?;
    // The prerequisite is the fixture's own 25 carried bones, not merely "some" starting count.
    require(
        initial_bones == BONES_COUNT as i64,
        format!(
            "external receipt started with {initial_bones} bones, expected the {BONES_COUNT} bone \
             fixture"
        ),
    )?;
    require(
        final_bones < initial_bones,
        format!("external receipt did not consume bones ({final_bones} of {initial_bones})"),
    )?;
    let initial_xp = counter("initial", initial, "prayer_xp")?;
    let final_xp = counter("final", last, "prayer_xp")?;
    require(
        final_xp > initial_xp,
        format!("external receipt gained no Prayer xp ({final_xp} of {initial_xp})"),
    )?;
    let burials = last
        .get("distinct_burial_logs")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    require(
        burials >= MIN_DISTINCT_BURIALS as u64,
        format!("external receipt observed {burials} distinct burials, expected {MIN_DISTINCT_BURIALS}+"),
    )?;
    // The producer keeps the raw and distinct burial counters on the same value; a receipt whose
    // counters disagree with each other is not a coherent record of that run.
    require(
        last.get("burial_logs").and_then(Value::as_u64) == Some(burials),
        format!(
            "external receipt burial counters disagree ({} raw, {burials} distinct)",
            last.get("burial_logs").unwrap_or(&Value::Null)
        ),
    )?;

    let start_deadline = START_DEADLINE.as_millis() as u64;
    let stop_deadline = STOP_DEADLINE.as_millis() as u64;
    require(
        payload.get("start_deadline_ms").and_then(Value::as_u64) == Some(start_deadline),
        format!(
            "external receipt start deadline {:?}, expected {start_deadline}",
            payload.get("start_deadline_ms")
        ),
    )?;
    require(
        payload.get("stop_deadline_ms").and_then(Value::as_u64) == Some(stop_deadline),
        format!(
            "external receipt Stop deadline {:?}, expected {stop_deadline}",
            payload.get("stop_deadline_ms")
        ),
    )?;
    let elapsed = payload
        .get("stop_elapsed_ms")
        .and_then(Value::as_u64)
        .ok_or_else(|| "qualified external receipt carries no Stop elapsed time".to_string())?;
    require(
        elapsed < stop_deadline,
        format!("external receipt Stop took {elapsed}ms, past the {stop_deadline}ms deadline"),
    )?;
    require(
        payload.get("reload_unchanged").and_then(Value::as_str)
            == Some(host_play::external_loader::NOTHING_CHANGED),
        format!(
            "external receipt unchanged reload reported {:?}, expected {:?}",
            payload.get("reload_unchanged"),
            host_play::external_loader::NOTHING_CHANGED
        ),
    )?;
    let source_after = payload
        .get("source_sha_after")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "qualified external receipt carries no changed-reload source hash".to_string()
        })?;
    let compiled_after = payload
        .get("compiled_sha_after")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            "qualified external receipt carries no changed-reload compiled hash".to_string()
        })?;
    for (name, value) in [
        ("source_sha_after", source_after),
        ("compiled_sha_after", compiled_after),
    ] {
        require(
            is_sha256(value),
            format!("external receipt {name} {value:?} is not a lowercase SHA-256 digest"),
        )?;
    }
    // The changed reload is the producer's harmless whitespace transform of the bound input
    // (`apply_harmless_whitespace` on its owned copy), so both post-reload identities must be the
    // digest the suite computed from exactly those bytes, and agree with each other.
    require(
        source_after == external.harmless_whitespace_sha256,
        format!(
            "external receipt changed-reload source hash {source_after:?} is not the harmless \
             whitespace digest {} of the bound input",
            external.harmless_whitespace_sha256
        ),
    )?;
    require(
        compiled_after == source_after,
        "external receipt changed-reload source and loaded-card hashes disagree",
    )?;
    // The key the producer carries under `compiled_sha` is the *reloaded* card's key (the changed
    // reload overwrites it), so it must be that same transform digest of the bound input — an
    // arbitrary, stale or merely non-empty value is refused. The load-time key the watch was
    // configured with is `script.sha256` (never overwritten), checked against the bound input
    // above; the producer does not serialize the before key separately.
    require(
        script.get("compiled_sha").and_then(Value::as_str) == Some(compiled_after),
        format!(
            "external receipt loaded-card cache key {:?} is not the reloaded card key \
             {compiled_after} of the harmless whitespace transform of the bound input",
            script.get("compiled_sha")
        ),
    )?;
    require(
        payload.get("auto_start").and_then(Value::as_bool) == Some(false),
        "external loader auto-started; load must select without Start",
    )?;
    payload
        .get("account")
        .and_then(Value::as_str)
        .filter(|account| !account.is_empty())
        .map(str::to_string)
        .ok_or_else(|| "qualified external receipt carries no account".to_string())
}
/// The stage/request/completion/reason/cleanup a failed external receipt preserves.
fn external_failure_detail(payload: &Value) -> Result<String, String> {
    use host_play::external_loader::Stage;
    let expected = wire(&Stage::Failed);
    let stage = payload
        .get("stage")
        .and_then(Value::as_str)
        .ok_or_else(|| "external FAIL receipt carries no stage".to_string())?;
    require(
        stage == expected,
        format!("external receipt stage {stage:?}, expected the failed stage {expected:?}"),
    )?;
    let reason = payload
        .get("failure_reason")
        .and_then(Value::as_str)
        .filter(|reason| !reason.trim().is_empty())
        .ok_or_else(|| "external FAIL receipt carries no failure reason".to_string())?;
    let field = |name: &str| {
        payload
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or("-")
            .to_string()
    };
    Ok(format!(
        "external loader failed at stage {stage}: {reason} (requested {}, completed {}, cleanup {})",
        field("requested_operation"),
        field("completed_operation"),
        field("cleanup_outcome"),
    ))
}
/// The external capture contract: the structural terminal-shot evidence every case requires,
/// bound to the account that owned the run.
///
/// The producer writes the terminal capture from the owned actor's own snapshot
/// (`actor_snapshot_json`), so its sidecar names that actor. A capture whose sidecar names
/// another actor, or none, is not this case's terminal evidence. A structurally complete,
/// actor-bound capture still stays `pending_visual_review`: a human reads it back.
fn external_capture_verdict(
    case: &CaseEntry,
    captures: &[CaptureRecord],
    account: &str,
) -> Verdict {
    let verdict = capture_verdict(case, captures);
    let Verdict::PendingVisualReview { captures } = verdict else {
        return verdict;
    };
    let Some(spec) = declared_capture(case) else {
        return Verdict::PendingVisualReview { captures };
    };
    for record in captures
        .iter()
        .filter(|record| record.matches_label(&spec.label))
    {
        match record.sidecar_actor.as_deref() {
            Some(actor) if actor == account => {}
            Some(actor) => {
                return Verdict::SharedFailure {
                    kind: "capture",
                    reason: format!(
                        "{}: the terminal capture names actor {actor:?}, expected the receipt's \
                         account {account:?}",
                        record.json
                    ),
                }
            }
            None => {
                return Verdict::SharedFailure {
                    kind: "capture",
                    reason: format!(
                        "{}: the terminal capture carries no actor binding",
                        record.json
                    ),
                }
            }
        }
    }
    Verdict::PendingVisualReview { captures }
}
