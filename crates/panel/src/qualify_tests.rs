use std::time::{Duration, Instant};

use super::*;
use crate::test_support::TestDir;

const TIMING: Timing = Timing {
    settle: Duration::from_secs(3),
    deadline: Duration::from_secs(300),
    shot_drain: Duration::from_secs(10),
};

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn adapter() -> AdapterReport {
    AdapterReport {
        name: "Test \"GPU\" 9000".into(),
        backend: "Vulkan".into(),
        device_type: "DiscreteGpu".into(),
        vendor: 0x10de,
        device: 0x2204,
        driver: "NVIDIA".into(),
        driver_info: "550.1".into(),
    }
}

fn scratch(name: &str) -> TestDir {
    TestDir::new(&format!("qualify-{name}"))
}

fn lines(out: Vec<u8>) -> Vec<String> {
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// A recorder armed at `t0` with `hold_s`, the receipt aimed at `dir`.
fn armed(hold_s: u64, dir: &std::path::Path, t0: Instant) -> Recorder {
    let recorder = Recorder::new();
    recorder.arm(
        QualifyConfig {
            hold_s,
            receipt: Some(dir.join("receipt.json")),
        },
        TIMING,
        t0,
    );
    recorder
}

fn read_receipt(dir: &std::path::Path) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(dir.join("receipt.json")).unwrap()).unwrap()
}

/// Everything the world needs before scene-ready, in the order a real
/// run produces it.
fn boot(recorder: &Recorder, t0: Instant, out: &mut Vec<u8>) {
    let info = wgpu::AdapterInfo {
        name: adapter().name,
        vendor: 0x10de,
        device: 0x2204,
        device_type: wgpu::DeviceType::DiscreteGpu,
        device_pci_bus_id: String::new(),
        driver: "NVIDIA".into(),
        driver_info: "550.1".into(),
        backend: wgpu::Backend::Vulkan,
        subgroup_min_size: 0,
        subgroup_max_size: 0,
        transient_saves_memory: false,
    };
    recorder.note_adapter(&info, out);
    recorder.note_frame_presented(t0 + Duration::from_millis(120), out);
}

#[test]
fn a_full_run_prints_each_marker_once_in_order_and_writes_a_passing_receipt() {
    let dir = scratch("full");
    let t0 = Instant::now();
    let recorder = armed(2, &dir, t0);
    let mut out = Vec::new();
    boot(&recorder, t0, &mut out);
    recorder.note_game_frame(GameBackend::Cpu, &mut out);
    // Repeated notes are idempotent: no second marker.
    recorder.note_frame_presented(t0 + secs(1), &mut out);
    recorder.note_game_frame(GameBackend::Gpu, &mut out);

    let fallback = || panic!("an explicit receipt path needs no fallback");
    // Scene 2 reached at +10s; the settle window has not elapsed.
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(10), &mut out, fallback),
        Outcome::Pending
    );
    assert!(!recorder.scene_settled(true, t0 + secs(12)));
    // A game frame lands after scene 2, and the settle window elapses.
    recorder.note_game_frame(GameBackend::Cpu, &mut out);
    assert!(recorder.scene_settled(true, t0 + secs(13)));
    recorder.note_scene2_png(&dir.join("scene2.png"), &mut out);
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(13), &mut out, fallback),
        Outcome::Pending
    );
    // The hold is 2 s from scene-ready.
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(14), &mut out, fallback),
        Outcome::Pending
    );
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(15), &mut out, fallback),
        Outcome::Passed
    );
    // Sealed: later ticks and failures change nothing.
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(16), &mut out, fallback),
        Outcome::Pending
    );
    recorder
        .finish(Some("late".into()), t0 + secs(17), &mut out, fallback)
        .unwrap();

    let printed = lines(out);
    let expected_prefixes = [
        "[qualify] adapter name=\"Test \\\"GPU\\\" 9000\" backend=Vulkan device_type=DiscreteGpu vendor=0x10de device=0x2204",
        "[qualify] first_frame_presented elapsed_ms=120",
        "[qualify] backend game=cpu surface=Vulkan",
        "[qualify] scene_ready elapsed_ms=13000 scene=2 game_backend=cpu hold_s=2",
        "[qualify] hold_complete elapsed_ms=15000 held_ms=2000",
    ];
    assert_eq!(printed.len(), expected_prefixes.len() + 1, "{printed:#?}");
    for (line, expected) in printed.iter().zip(expected_prefixes) {
        assert_eq!(line, expected);
    }
    let receipt_line = printed.last().unwrap();
    assert!(
        receipt_line.starts_with("[qualify] receipt path=\"") && receipt_line.ends_with(" ok=true"),
        "{receipt_line}"
    );

    let receipt = read_receipt(&dir);
    assert_eq!(receipt["schema"], 1);
    assert_eq!(receipt["ok"], true);
    assert_eq!(receipt["failure"], serde_json::Value::Null);
    assert_eq!(receipt["game_backend"], "cpu");
    assert_eq!(receipt["first_frame_presented_ms"], 120);
    assert_eq!(receipt["scene_ready_ms"], 13000);
    assert_eq!(receipt["hold_s"], 2);
    assert_eq!(receipt["held_ms"], 2000);
    assert_eq!(receipt["elapsed_ms"], 15000);
    assert_eq!(receipt["adapter"]["name"], "Test \"GPU\" 9000");
    assert_eq!(receipt["adapter"]["vendor"], 0x10de);
    assert_eq!(
        receipt["markers"],
        serde_json::json!([
            "adapter",
            "first_frame_presented",
            "backend",
            "scene_ready",
            "hold_complete"
        ])
    );
    assert!(receipt["scene2_png"]
        .as_str()
        .unwrap()
        .ends_with("scene2.png"));
}

#[test]
fn scene_ready_waits_for_the_first_present_a_post_scene_game_frame_and_the_settle() {
    let t0 = Instant::now();
    let dir = scratch("gates");
    let fallback = || panic!("no fallback expected");

    // A game frame from before scene 2 does not count.
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    recorder.note_frame_presented(t0 + secs(1), &mut out);
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(5), &mut out, fallback),
        Outcome::Pending
    );
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(9), &mut out, fallback),
        Outcome::Pending,
        "settled, but only a pre-scene game frame exists"
    );
    assert!(!lines(out).iter().any(|l| l.contains("scene_ready")));

    // No swapchain frame presented: not ready however long it settles.
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.tick(true, 2, t0 + secs(5), &mut out, fallback);
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(20), &mut out, fallback),
        Outcome::Pending
    );
    assert!(!lines(out).iter().any(|l| l.contains("scene_ready")));

    // Leaving the world resets nothing but blocks readiness (settle needs
    // ingame), and scene < 2 never latches scene 2.
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 1, t0 + secs(1), &mut out, fallback);
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    assert_eq!(
        recorder.tick(true, 1, t0 + secs(30), &mut out, fallback),
        Outcome::Pending
    );
    assert!(!recorder.scene_settled(true, t0 + secs(30)));
}

#[test]
fn the_hold_waits_for_the_scene_shot_up_to_the_drain() {
    let t0 = Instant::now();
    let dir = scratch("drain");
    let fallback = || panic!("no fallback expected");
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 2, t0 + secs(1), &mut out, fallback);
    recorder.note_game_frame(GameBackend::Cpu, &mut out);
    // Ready at +4s; a zero hold still waits for the missing shot ...
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(4), &mut out, fallback),
        Outcome::Pending
    );
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(13), &mut out, fallback),
        Outcome::Pending
    );
    // ... until the drain lapses, then passes without a PNG path.
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(14), &mut out, fallback),
        Outcome::Passed
    );
    let receipt = read_receipt(&dir);
    assert_eq!(receipt["ok"], true);
    assert_eq!(receipt["scene2_png"], serde_json::Value::Null);
    assert_eq!(receipt["held_ms"], 10000);
}

#[test]
fn each_stalled_stage_fails_at_the_deadline_with_its_own_reason() {
    let t0 = Instant::now();
    let dir = scratch("deadline");
    let fallback = || panic!("no fallback expected");
    let late = t0 + TIMING.deadline;

    // Nothing ever presented.
    let recorder = armed(1, &dir, t0);
    let mut out = Vec::new();
    assert_eq!(
        recorder.tick(false, 0, late, &mut out, fallback),
        Outcome::Failed("no frame was presented to the swapchain within 300s".into())
    );
    let printed = lines(out);
    assert_eq!(
        printed[0],
        "[qualify] failed reason=\"no frame was presented to the swapchain within 300s\""
    );
    assert!(printed[1].ends_with(" ok=false"), "{printed:#?}");
    let receipt = read_receipt(&dir);
    assert_eq!(receipt["ok"], false);
    assert_eq!(
        receipt["failure"],
        "no frame was presented to the swapchain within 300s"
    );
    assert_eq!(receipt["elapsed_ms"], 300_000);
    assert_eq!(receipt["markers"], serde_json::json!([]));

    // Presented, but scene 2 never came.
    let recorder = armed(1, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    assert_eq!(
        recorder.tick(true, 1, late, &mut out, fallback),
        Outcome::Failed("the focused slot never reached scene 2 within 300s".into())
    );

    // Scene 2 but no later game frame.
    let recorder = armed(1, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 2, t0 + secs(5), &mut out, fallback);
    assert_eq!(
        recorder.tick(true, 2, late, &mut out, fallback),
        Outcome::Failed("no game frame was presented after scene 2 within 300s".into())
    );
    let receipt = read_receipt(&dir);
    assert_eq!(receipt["first_frame_presented_ms"], 0);
    assert_eq!(receipt["scene_ready_ms"], serde_json::Value::Null);
}

#[test]
fn an_outside_failure_writes_a_failing_receipt_once_and_a_pass_wins_over_a_later_one() {
    let t0 = Instant::now();
    let dir = scratch("outside");
    let recorder = armed(1, &dir, t0);
    let mut out = Vec::new();
    recorder
        .finish(Some("boot failed".into()), t0 + secs(2), &mut out, || {
            panic!("no fallback expected")
        })
        .unwrap();
    assert!(recorder.has_verdict());
    recorder
        .finish(Some("second".into()), t0 + secs(3), &mut out, || {
            panic!("no fallback expected")
        })
        .unwrap();
    let printed = lines(out);
    assert_eq!(printed.len(), 2, "{printed:#?}");
    assert_eq!(printed[0], "[qualify] failed reason=\"boot failed\"");
    assert_eq!(read_receipt(&dir)["failure"], "boot failed");
}

#[test]
fn an_unarmed_recorder_prints_and_writes_nothing() {
    let recorder = Recorder::new();
    let mut out = Vec::new();
    assert!(!recorder.wants_present());
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    recorder.note_frame_presented(Instant::now(), &mut out);
    assert_eq!(
        recorder.tick(true, 2, Instant::now(), &mut out, || panic!("unused")),
        Outcome::Pending
    );
    recorder
        .finish(Some("x".into()), Instant::now(), &mut out, || {
            panic!("unused")
        })
        .unwrap();
    assert!(out.is_empty());
    assert!(!recorder.has_verdict());
}

#[test]
fn the_present_hook_turns_off_after_the_first_present() {
    let t0 = Instant::now();
    let dir = scratch("present");
    let recorder = armed(1, &dir, t0);
    let mut out = Vec::new();
    assert!(recorder.wants_present());
    recorder.note_frame_presented(t0, &mut out);
    assert!(!recorder.wants_present());
}

#[test]
fn the_default_receipt_lands_in_the_shot_dir_and_early_failures_use_the_fallback() {
    let t0 = Instant::now();
    let shots = scratch("shots");
    let fallback_dir = scratch("fallback");

    let recorder = Recorder::new();
    recorder.arm(QualifyConfig::default(), TIMING, t0);
    let mut out = Vec::new();
    recorder.note_shot_dir(&shots, &mut out);
    recorder
        .finish(Some("x".into()), t0, &mut out, || panic!("shot dir exists"))
        .unwrap();
    assert!(shots.join(RECEIPT_FILE).is_file());

    let recorder = Recorder::new();
    recorder.arm(QualifyConfig::default(), TIMING, t0);
    let mut out = Vec::new();
    let fallback = fallback_dir.to_path_buf();
    recorder
        .finish(Some("early".into()), t0, &mut out, move || fallback)
        .unwrap();
    assert!(fallback_dir.join(RECEIPT_FILE).is_file());
}

#[test]
fn a_receipt_write_failure_fails_an_otherwise_passing_run() {
    let t0 = Instant::now();
    let dir = scratch("unwritable");
    let recorder = Recorder::new();
    // The receipt's parent directory does not exist.
    recorder.arm(
        QualifyConfig {
            hold_s: 0,
            receipt: Some(dir.join("missing").join("receipt.json")),
        },
        TIMING,
        t0,
    );
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 2, t0 + secs(1), &mut out, || unreachable!());
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    recorder.note_scene2_png(&dir.join("scene2.png"), &mut out);
    let outcome = recorder.tick(true, 2, t0 + secs(4), &mut out, || unreachable!());
    assert!(matches!(&outcome, Outcome::Failed(error) if error.contains("receipt")));
    // The receipt file does not exist, so no `receipt` marker claims one;
    // the run ends with a `failed` marker carrying the write error.
    let printed = lines(out);
    assert!(
        !printed.iter().any(|line| line.contains("] receipt ")),
        "{printed:#?}"
    );
    let failed = printed.last().unwrap();
    assert!(
        failed.starts_with("[qualify] failed reason=\"receipt ") && failed.contains("receipt.json"),
        "{failed}"
    );
    assert!(!dir.join("missing").exists());
}

#[test]
fn leaving_scene_2_before_the_settle_resets_it_and_never_reports_a_non_world_scene_ready() {
    let t0 = Instant::now();
    let dir = scratch("reset");
    let fallback = || panic!("no fallback expected");
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 2, t0 + secs(1), &mut out, fallback);
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    // Scene 1 (and not-ingame) ticks past the old settle point: no ready.
    for (ingame, scene) in [(true, 1), (false, 2)] {
        assert_eq!(
            recorder.tick(ingame, scene, t0 + secs(5), &mut out, fallback),
            Outcome::Pending
        );
        assert!(!recorder.scene_settled(true, t0 + secs(5)));
    }
    assert!(!lines(out.clone()).iter().any(|l| l.contains("scene_ready")));
    // Back in the world: the settle window and a fresh game frame restart.
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(6), &mut out, fallback),
        Outcome::Pending
    );
    recorder.note_game_frame(GameBackend::Gpu, &mut out);
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(8), &mut out, fallback),
        Outcome::Pending
    );
    recorder.note_scene2_png(&dir.join("scene2.png"), &mut out);
    assert_eq!(
        recorder.tick(true, 2, t0 + secs(9), &mut out, fallback),
        Outcome::Passed
    );
    let ready: Vec<String> = lines(out)
        .into_iter()
        .filter(|l| l.contains("scene_ready"))
        .collect();
    assert_eq!(
        ready,
        ["[qualify] scene_ready elapsed_ms=9000 scene=2 game_backend=gpu hold_s=0"]
    );
}

#[test]
fn a_settled_tick_past_the_deadline_fails_instead_of_passing() {
    let t0 = Instant::now();
    let dir = scratch("late");
    let fallback = || panic!("no fallback expected");
    let recorder = armed(0, &dir, t0);
    let mut out = Vec::new();
    recorder.note_frame_presented(t0, &mut out);
    recorder.tick(true, 2, t0 + secs(1), &mut out, fallback);
    recorder.note_game_frame(GameBackend::Cpu, &mut out);
    recorder.note_scene2_png(&dir.join("scene2.png"), &mut out);
    // Everything else is satisfied, but the first settled tick is at 301 s.
    let outcome = recorder.tick(true, 2, t0 + secs(301), &mut out, fallback);
    assert!(matches!(&outcome, Outcome::Failed(reason) if reason.ends_with("within 300s")));
    let printed = lines(out);
    assert!(!printed.iter().any(|l| l.contains("scene_ready")));
    assert!(!printed.iter().any(|l| l.contains("hold_complete")));
    assert_eq!(read_receipt(&dir)["ok"], false);
}

#[test]
fn the_atomic_write_replaces_the_receipt_and_leaves_no_temp_file() {
    let dir = scratch("atomic");
    let path = dir.join("receipt.json");
    std::fs::write(&path, "stale").unwrap();
    let receipt = QualifyRun::new(QualifyConfig::default(), TIMING, Instant::now())
        .seal(Some("why".into()), Instant::now())
        .unwrap();
    write_receipt_atomic(&path, &receipt).unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(parsed["failure"], "why");
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(leftovers, [std::ffi::OsString::from("receipt.json")]);
    // A path with no file name is refused before anything is written.
    assert!(write_receipt_atomic(std::path::Path::new("/"), &receipt).is_err());
}
