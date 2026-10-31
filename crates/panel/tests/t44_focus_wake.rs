//! Live evidence for the panel focus writer, actual control stream, and Play worker.

#[derive(Clone, Debug, serde::Serialize)]
struct FocusSlotReceipt {
    name: String,
    input_before: bool,
    input_after: bool,
    worker_before: Option<bool>,
    worker_after: Option<bool>,
    applications_before: u64,
    applications_after: u64,
    writes_before: u64,
    writes_after: u64,
    drained_before: u64,
    drained_after: u64,
    scenes_before: u64,
    scenes_after: u64,
    connected: bool,
    valid_scene: bool,
    render_fault: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
struct FocusReceipt {
    action: String,
    expected_before: Vec<bool>,
    expected_after: Vec<bool>,
    slots: Vec<FocusSlotReceipt>,
}

fn focus_verdict(receipt: &FocusReceipt) -> Result<(), String> {
    if receipt.slots.len() != 2
        || receipt.expected_before.len() != receipt.slots.len()
        || receipt.expected_after.len() != receipt.slots.len()
        || receipt.expected_before.iter().filter(|&&v| v).count() > 1
        || receipt.expected_after.iter().filter(|&&v| v).count() > 1
    {
        return Err("missing slots or multiple Game-pane owners".into());
    }
    for (index, slot) in receipt.slots.iter().enumerate() {
        if slot.name.is_empty()
            || receipt.slots[..index]
                .iter()
                .any(|other| other.name == slot.name)
        {
            return Err("missing or duplicate slot identity".into());
        }
        let before = receipt.expected_before[index];
        let after = receipt.expected_after[index];
        if slot.input_before != before || slot.input_after != after {
            return Err("panel wrote the wrong render focus bit".into());
        }
        if slot.worker_before != Some(before) || slot.worker_after != Some(after) {
            return Err("Play worker did not apply the panel focus bit".into());
        }
        let applications = slot
            .applications_after
            .checked_sub(slot.applications_before)
            .ok_or("focus application counter regressed")?;
        let writes = slot
            .writes_after
            .checked_sub(slot.writes_before)
            .ok_or("control write counter regressed")?;
        let drained = slot
            .drained_after
            .checked_sub(slot.drained_before)
            .ok_or("control drain counter regressed")?;
        if before != after {
            if applications != 1 {
                return Err("changed focus did not have exactly one worker application".into());
            }
            if writes == 0 || drained == 0 {
                return Err("changed focus lacks actual wake-byte traffic".into());
            }
        } else if applications != 0 {
            return Err("unchanged focus or cadence produced a focus application".into());
        }
        if !slot.connected || !slot.valid_scene || slot.render_fault {
            return Err("disconnected slot, invalid scene, or render fault".into());
        }
        if slot.scenes_after <= slot.scenes_before {
            return Err("Play worker scene progress stopped".into());
        }
    }
    Ok(())
}

fn focus_fixture(before: [bool; 2], after: [bool; 2]) -> FocusReceipt {
    FocusReceipt {
        action: "fixture".into(),
        expected_before: before.to_vec(),
        expected_after: after.to_vec(),
        slots: (0..2)
            .map(|index| FocusSlotReceipt {
                name: format!("fixture-{index}"),
                input_before: before[index],
                input_after: after[index],
                worker_before: Some(before[index]),
                worker_after: Some(after[index]),
                applications_before: 3,
                applications_after: 3 + u64::from(before[index] != after[index]),
                writes_before: 3,
                writes_after: 4,
                drained_before: 3,
                drained_after: 4,
                scenes_before: 10,
                scenes_after: 11,
                connected: true,
                valid_scene: true,
                render_fault: false,
            })
            .collect(),
    }
}

#[test]
fn focus_oracle_accepts_transfer_close_reopen_and_unchanged_cadence() {
    for (before, after) in [
        ([true, false], [false, true]),
        ([false, true], [true, false]),
        ([true, false], [false, false]),
        ([false, false], [false, true]),
        ([true, false], [true, false]),
        ([false, false], [false, false]),
    ] {
        assert!(focus_verdict(&focus_fixture(before, after)).is_ok());
    }
}

#[test]
fn focus_oracle_rejects_each_named_failure() {
    type Mutation = fn(&mut FocusReceipt);
    let cases: &[(Mutation, &str)] = &[
        (
            |r| {
                r.slots.pop().unwrap();
            },
            "missing slots",
        ),
        (
            |r| r.slots[0].name = r.slots[1].name.clone(),
            "duplicate slot",
        ),
        (|r| r.slots[0].input_after = true, "wrong render focus"),
        (|r| r.slots[0].worker_after = Some(true), "did not apply"),
        (|r| r.slots[1].worker_after = None, "did not apply"),
        (|r| r.slots[0].writes_after = 3, "wake-byte traffic"),
        (|r| r.slots[1].drained_after = 3, "wake-byte traffic"),
        (|r| r.slots[0].applications_after = 3, "exactly one"),
        (|r| r.slots[0].applications_after = 5, "exactly one"),
        (|r| r.slots[0].applications_after = 2, "counter regressed"),
        (|r| r.slots[0].writes_after = 2, "counter regressed"),
        (|r| r.slots[0].drained_after = 2, "counter regressed"),
        (|r| r.slots[0].connected = false, "disconnected slot"),
        (|r| r.slots[0].valid_scene = false, "invalid scene"),
        (|r| r.slots[0].render_fault = true, "render fault"),
        (|r| r.slots[1].scenes_after = 10, "progress stopped"),
    ];
    for (mutate, expected) in cases {
        let mut receipt = focus_fixture([true, false], [false, true]);
        mutate(&mut receipt);
        let error = focus_verdict(&receipt).unwrap_err();
        assert!(error.contains(expected), "{error}, expected {expected}");
    }
    let mut cadence = focus_fixture([true, false], [true, false]);
    cadence.slots[1].applications_after += 1;
    assert!(focus_verdict(&cadence)
        .unwrap_err()
        .contains("unchanged focus"));
}

use panel::session::Session;
use std::path::PathBuf;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct FocusSnapshot {
    input: bool,
    worker: Option<bool>,
    applications: u64,
    writes: u64,
    drained: u64,
    scenes: u64,
    connected: bool,
    valid: bool,
    fault: bool,
}

fn focus_snapshot(session: &Session, name: &str) -> Option<FocusSnapshot> {
    let slot = session.core.slots().get(name)?;
    let stats = slot.pixels.fair_stats()?;
    if slot.pixels.head() != Some(stats.identity()) {
        return None;
    }
    let wake = session.core.play_for_test()?.slot_wake_snapshot(name)?;
    let connected = session
        .statuses
        .live_rows()
        .iter()
        .any(|row| row.username == name && row.connected && row.ingame);
    Some(FocusSnapshot {
        input: slot.input.render_focused(),
        worker: stats.render_focused(),
        applications: stats.focus_applications(),
        writes: wake.successful_writes(),
        drained: wake.drained_bytes(),
        scenes: stats.scene_consumes(),
        connected,
        valid: stats.has_valid_scene(),
        fault: slot.pixels.refusal().is_some(),
    })
}

fn focus_pump(session: &mut Session) {
    session.pump_status();
    for slot in session.core.slots().values() {
        drop(slot.pixels.take());
    }
    host::render_head::pump(Instant::now());
}

fn focus_observe(session: &mut Session, duration: Duration) {
    let end = Instant::now() + duration;
    while Instant::now() < end {
        focus_pump(session);
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn record_focus_action(
    session: &mut Session,
    names: &[String],
    action: &str,
    expected_before: [bool; 2],
    expected_after: [bool; 2],
    change: impl FnOnce(&mut Session),
) -> FocusReceipt {
    let before: Vec<_> = names
        .iter()
        .map(|name| focus_snapshot(session, name).expect("attached live head before action"))
        .collect();
    change(session);
    // A fixed observation window includes repeated unchanged UI passes.
    // It cannot end early when the expected counter first appears.
    focus_observe(session, Duration::from_secs(3));
    let slots = names
        .iter()
        .zip(before)
        .map(|(name, before)| {
            let after = focus_snapshot(session, name).expect("attached live head after action");
            FocusSlotReceipt {
                name: name.clone(),
                input_before: before.input,
                input_after: after.input,
                worker_before: before.worker,
                worker_after: after.worker,
                applications_before: before.applications,
                applications_after: after.applications,
                writes_before: before.writes,
                writes_after: after.writes,
                drained_before: before.drained,
                drained_after: after.drained,
                scenes_before: before.scenes,
                scenes_after: after.scenes,
                connected: before.connected && after.connected,
                valid_scene: before.valid && after.valid,
                render_fault: before.fault || after.fault,
            }
        })
        .collect();
    FocusReceipt {
        action: action.into(),
        expected_before: expected_before.to_vec(),
        expected_after: expected_after.to_vec(),
        slots,
    }
}

#[test]
#[ignore = "LIVE=1 real panel Session focus and Play wake-byte proof"]
fn t44_panel_focus_wake_live() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let required =
        |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} is required"));
    let output = PathBuf::from(required("M3_OUT"));
    std::fs::create_dir_all(&output).unwrap();
    host::render_head::test_hooks::configure_process(
        host::render_head::test_hooks::ProcessFixture {
            grant_ttl: Duration::from_millis(required("M3_GRANT_TTL_MS").parse().unwrap()),
            quantum: Duration::from_millis(required("M3_SCENE_QUANTUM_MS").parse().unwrap()),
        },
    )
    .unwrap();
    let limits: serde_json::Value =
        serde_json::from_str(&required("T44_RENDER_BUDGET_LIMITS_JSON")).unwrap();
    let limit = |name: &str| {
        usize::try_from(
            limits[name]
                .as_u64()
                .unwrap_or_else(|| panic!("missing budget {name}")),
        )
        .unwrap()
    };
    host::render_head::test_hooks::configure_render_budgets(
        client::render::budget::RenderBudgetLimits::new(
            limit("capture"),
            limit("nav_vertices"),
            limit("nav_quads"),
            limit("nav_edges"),
            limit("nav_input"),
            limit("head_models"),
            limit("head_misc"),
            limit("temp_models"),
            limit("shared_models"),
            limit("shared_sprites"),
            limit("vertex"),
            limit("context_cpu"),
            limit("context_gpu"),
        ),
    )
    .unwrap();
    let profiles = frontend_core::live::LiveProfiles::mint(2);
    let names = profiles.names().to_vec();
    let vault = profiles.panel_vault(274_000_001, true);
    let mut session = Session::with_instance(frontend_core::launch::InstancePermit::skip());
    session
        .configure_profile(frontend_core::launch::ProfileArgs::from_host(
            host_play::ProfileOptions {
                profile: Some("local-289".into()),
                revision: Some("289".into()),
                host: Some("127.0.0.1".into()),
                asset_host: Some("127.0.0.1".into()),
                port: Some(required("M3_PORT").parse().unwrap()),
                http_port: Some(required("M3_HTTP_PORT").parse().unwrap()),
                engine_dir: Some(required("ENGINE_DIR").into()),
                nav_pack: Some(required("NAV_PACK").into()),
                ..host_play::ProfileOptions::default()
            },
        ))
        .unwrap();
    assert!(
        session.unlock_at(&vault, profiles.passphrase()),
        "{:?}",
        session.error
    );
    // Wait for the core startup policy before explicit profile selection.
    let deadline = Instant::now() + Duration::from_secs(120);
    while !session.core.has_play() || !session.core.prepare_startup() {
        focus_pump(&mut session);
        assert!(
            Instant::now() < deadline,
            "panel unlock deadline: {:?}",
            session.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    session.set_multibox(true);
    for name in &names {
        session.load(name);
    }
    session.confirm_render_all(false);
    session.select(&names[0]);
    session.set_renderer(true);
    session.set_game_pane_open(true);
    session.set_live_full_rate(true);
    session.login_all();
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        focus_pump(&mut session);
        if names.iter().enumerate().all(|(index, name)| {
            focus_snapshot(&session, name).is_some_and(|s| {
                s.connected
                    && s.valid
                    && !s.fault
                    && s.scenes > 0
                    && s.worker == Some(index == 0)
                    && s.input == (index == 0)
            })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "panel live head deadline: {:?}",
            session.error
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    focus_observe(&mut session, Duration::from_secs(10));
    let receipts = vec![
        record_focus_action(
            &mut session,
            &names,
            "sidecar-on",
            [true, false],
            [true, false],
            |s| s.set_sidecar_50(true),
        ),
        record_focus_action(
            &mut session,
            &names,
            "live-full-rate-off",
            [true, false],
            [true, false],
            |s| s.set_live_full_rate(false),
        ),
        record_focus_action(
            &mut session,
            &names,
            "select-second",
            [true, false],
            [false, true],
            |s| s.select(&names[1]),
        ),
        record_focus_action(
            &mut session,
            &names,
            "live-full-rate-on",
            [false, true],
            [false, true],
            |s| s.set_live_full_rate(true),
        ),
        record_focus_action(
            &mut session,
            &names,
            "sidecar-off",
            [false, true],
            [false, true],
            |s| s.set_sidecar_50(false),
        ),
        record_focus_action(
            &mut session,
            &names,
            "select-first",
            [false, true],
            [true, false],
            |s| s.select(&names[0]),
        ),
        record_focus_action(
            &mut session,
            &names,
            "select-same",
            [true, false],
            [true, false],
            |s| s.select(&names[0]),
        ),
    ];
    // Preserve failed evidence before the assertions and before teardown.
    let verdicts: Vec<_> = receipts.iter().map(focus_verdict).collect();
    std::fs::write(
        output.join("panel-focus-wake.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "host": required("T44_LAB_HOST"),
            "backend": "cpu",
            "wake_source": "Play.wakes paired with the actual SlotPark",
            "receipts": receipts,
            "verdicts": verdicts,
        }))
        .unwrap(),
    )
    .unwrap();
    session.logout_all();
    drop(session);
    for verdict in verdicts {
        verdict.unwrap();
    }
}
