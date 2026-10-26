//! Bounded native onStop + final Slot log delivery.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use script::shim::InteractReq;
use script_js::{LoadIsolate, LoadShape};

fn spawn_class(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn examplebot_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ExampleBot.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let pin = script_js::js_cache::JsCache::origin_sha(&bytes);
    assert_eq!(
        pin, "8e8e27cd9c2e57cd6478132e4bfaf910678664898db44915acc08ffc501c0973",
        "tracked ExampleBot fixture drifted from rs2b0t 96410ec5 script-template"
    );
    let src = String::from_utf8(bytes).expect("ExampleBot utf-8");
    script_js::transpile_ts(&src).expect("transpile ExampleBot")
}

fn contains_line(logs: &[String], needle: &str) -> bool {
    logs.iter().any(|l| l.contains(needle))
}

#[test]
fn join_returns_onstop_this_log() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() { this.log('tick-line'); }
            onStop() { this.log('stopped-ok'); }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let before = iso.drain_logs();
    assert!(
        contains_line(&before, "tick-line"),
        "tick log before join: {before:?}"
    );
    assert!(
        !contains_line(&before, "stopped-ok"),
        "onStop must not run on tick drain: {before:?}"
    );
    let logs = iso.join();
    assert!(
        contains_line(&logs, "stopped-ok"),
        "join must return onStop this.log: {logs:?}"
    );
}

#[test]
fn tracked_examplebot_stop_log() {
    let js = examplebot_source();
    let shape = script_js::detect_shape(&js);
    let iso = LoadIsolate::spawn(js, shape, vec![]).unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let logs = iso.join();
    assert!(
        contains_line(&logs, "BoneBurier stopped"),
        "unchanged ExampleBot onStop log: {logs:?}"
    );
}

#[test]
fn hostile_onstop_is_bounded_by_50ms_and_2s_join() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { while (true) {} }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let t0 = Instant::now();
    let logs = iso.join();
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "join ceiling is 2s, got {elapsed:?} logs={logs:?}"
    );
    assert!(
        contains_line(&logs, "onStop threw"),
        "the hostile hook must be interrupted by its deadline: {logs:?}"
    );
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::yield_now();
    }
    pred()
}

#[test]
fn ten_isolates_share_one_deadline_supervisor() {
    // Count only the ten isolates this test owns, not other default-parallel
    // tests sharing the parent process.
    const CHILD: &str = "SCRIPT_CENSUS_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "ten_isolates_share_one_deadline_supervisor",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(status.success(), "isolated census child failed: {status}");
        return;
    }
    let mut isolates = Vec::new();
    for _ in 0..10 {
        let isolate =
            spawn_class("export default class T extends LoopingBot { loop() {} onStop() {} }");
        assert_eq!(isolate.probe("1").unwrap(), 1);
        isolates.push(isolate);
    }
    for isolate in &isolates {
        assert_eq!(isolate.teardown_proof().supervisor_workers(), 1);
    }
    #[cfg(target_os = "linux")]
    {
        let names = std::fs::read_dir("/proc/self/task")
            .unwrap()
            .map(|task| {
                let task = task.unwrap();
                std::fs::read_to_string(task.path().join("comm")).unwrap()
            })
            .collect::<Vec<_>>();
        let count = |prefix: &str| names.iter().filter(|name| name.starts_with(prefix)).count();
        assert_eq!(count("js-deadline"), 1, "one OS deadline supervisor");
        assert_eq!(count("script-bridge"), 1, "exactly one async bridge worker");
        assert!(
            count("script-blk") <= 1,
            "at most one optional blocking bridge thread"
        );
        assert_eq!(
            count("js-isolate"),
            10,
            "one OS V8 owner per active isolate"
        );
    }
    for isolate in isolates {
        isolate.join();
    }
}

#[test]
fn self_stop_hostile_onstop_without_later_tick_or_probe() {
    let iso = spawn_class(
        r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() { ScriptRunner.stop('done'); }
    onStop() { while (true) {} }
}
"#,
    );
    let proof = iso.teardown_proof();
    assert_eq!(
        proof.supervisor_workers(),
        1,
        "the process supervisor is active before the hook"
    );
    iso.on_game_tick(1);
    let t0 = Instant::now();
    let finished = wait_until(Duration::from_secs(2), || proof.finished());
    let elapsed = t0.elapsed();
    assert!(
        finished,
        "self-stop must finish from the native deadline without join/tick/pause/probe: elapsed={elapsed:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "self-stop join ceiling is 2s: {elapsed:?}"
    );
    let mut logs = iso.drain_logs();
    let stopped = iso.stopped();
    logs.extend(iso.join());
    assert!(stopped, "self-stop must mark the isolate stopped: {logs:?}");
    assert!(
        contains_line(&logs, "script requested stop"),
        "autonomous ScriptRunner.stop must be the path: {logs:?}"
    );
    assert!(
        contains_line(&logs, "onStop threw"),
        "onStop must have entered and been interrupted before join: {logs:?}"
    );
    assert_eq!(
        proof.supervisor_workers(),
        1,
        "the process supervisor persists after self-stop"
    );
    assert!(proof.invoked(), "hostile self-stop entered onStop");
}

#[test]
fn drop_without_join_does_not_block_and_leaves_fresh_isolate() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }",
    );
    let proof = iso.teardown_proof();
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    assert_eq!(
        proof.supervisor_workers(),
        1,
        "the process supervisor is shared while the isolate runs"
    );
    assert!(!proof.finished(), "Running isolate has not consumed Stop");
    let t0 = Instant::now();
    drop(iso);
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "raw Drop must not join: {elapsed:?}"
    );
    assert!(
        wait_until(Duration::from_secs(5), || proof.finished()),
        "raw Drop must finish on the isolate thread before hook/resource checks"
    );
    assert!(
        !proof.invoked(),
        "raw Drop must not invoke onStop after the isolate consumed Stop"
    );
    assert_eq!(
        proof.supervisor_workers(),
        1,
        "the process supervisor persists after raw Drop"
    );
    let iso = spawn_class(
        "export default class T extends LoopingBot { loop() { globalThis.__fresh = 1; } }",
    );
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__fresh").unwrap(), 1);
    iso.join();
}

#[test]
fn delayed_hook_entry_keeps_deadline_interrupt() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { while (true) {} }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.delay_onstop_after_deadline_arm(Duration::from_millis(80));
    let t0 = Instant::now();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    iso.join_detached(done_tx);
    let logs = done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("delayed onStop join exceeded the generous hang guard");
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "join ceiling is 2s after delayed entry: {elapsed:?} logs={logs:?}"
    );
    assert!(
        contains_line(&logs, "onStop threw"),
        "deadline interrupt must survive delayed entry: {logs:?}"
    );
}

#[test]
fn deadline_registration_failure_skips_hook_and_stays_bounded() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }",
    );
    let proof = iso.teardown_proof();
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.fail_onstop_deadline_registration();
    let t0 = Instant::now();
    let logs = iso.join();
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "join ceiling is 2s when deadline-owner creation fails: {elapsed:?}"
    );
    assert!(
        contains_line(&logs, "onStop skipped: no deadline owner"),
        "native diagnostic when no deadline owner: {logs:?}"
    );
    assert!(
        !contains_line(&logs, "stopped-ok"),
        "getter/body/drain must not run without a deadline owner: {logs:?}"
    );
    assert!(
        wait_until(Duration::from_secs(5), || proof.finished()),
        "fail-closed isolate must still finish within the generous hang guard"
    );
    assert!(!proof.invoked(), "fail-closed must not invoke onStop");
    assert_eq!(proof.supervisor_workers(), 1);
}

#[test]
fn credited_transfer_refusal_and_actual_join_reopen_admission() {
    // A child process owns the four process-global credits exclusively; the
    // default-parallel parent suite cannot transiently refuse unrelated JS.
    const CHILD: &str = "SCRIPT_QUARANTINE_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "credited_transfer_refusal_and_actual_join_reopen_admission",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .status()
            .unwrap();
        assert!(
            status.success(),
            "isolated quarantine child failed: {status}"
        );
        return;
    }
    use script_js::{isolate_capacity_status, LoadIsolateError};
    let mut isolates = Vec::new();
    for _ in 0..5 {
        let isolate =
            spawn_class("export default class T extends LoopingBot { loop() {} onStop() {} }");
        isolate.on_game_tick(1);
        assert_eq!(isolate.probe("1").unwrap(), 1);
        assert_eq!(isolate.poll_ready(), script_js::Ready::Ready);
        isolate.delay_onstop_after_deadline_arm(Duration::from_millis(3_500));
        isolates.push(isolate);
    }
    let mut receivers = Vec::new();
    for isolate in isolates {
        let (tx, rx) = std::sync::mpsc::channel();
        isolate.join_detached(tx);
        receivers.push(rx);
    }
    assert!(
        wait_until(Duration::from_secs(4), || {
            isolate_capacity_status()
                .is_ok_and(|status| status.quarantined == 4 && status.retiring_gameplay == 1)
        }),
        "four A and one J must coexist; status={:?}",
        isolate_capacity_status()
    );
    let status = isolate_capacity_status().unwrap();
    assert_eq!(status.quarantined, 4, "{status:?}");
    assert_eq!(
        status.retiring_gameplay, 1,
        "no credit retains J: {status:?}"
    );
    // Registry publication precedes receipt delivery: seeing four credits
    // does not imply the supervisor has sent their receipts yet.
    let mut received = [false; 5];
    let mut credited = 0;
    assert!(
        wait_until(Duration::from_secs(2), || {
            for (index, rx) in receivers.iter().enumerate() {
                if !received[index] {
                    if let Ok(lines) = rx.try_recv() {
                        assert!(
                            lines
                                .iter()
                                .any(|line| line.contains("quarantined after join deadline")),
                            "{lines:?}"
                        );
                        received[index] = true;
                        credited += 1;
                    }
                }
            }
            credited == 4
        }),
        "only credited transfer releases a slot receipt; received={credited}"
    );
    assert!(matches!(
        LoadIsolate::spawn(
            "export function tick() {}".into(),
            LoadShape::NativeTick,
            vec![]
        ),
        Err(LoadIsolateError::QuarantineFull {
            quarantined: 4,
            retiring_gameplay: 1,
            ..
        })
    ));
    assert!(
        wait_until(Duration::from_secs(5), || {
            isolate_capacity_status().is_ok_and(|status| status.quarantined == 0)
        }),
        "real joins must release all credits"
    );
    let replacement =
        spawn_class("export default class T extends LoopingBot { loop() {} onStop() {} }");
    assert_eq!(replacement.probe("1").unwrap(), 1);
    replacement.join();
}

#[test]
fn missing_execution_deadline_cuts_with_the_actual_supervisor_cause() {
    for (stopped, cause) in [
        (false, "JS deadline supervisor is at capacity"),
        (true, "JS deadline supervisor worker stopped"),
    ] {
        let iso = spawn_class(
            "export default class T extends LoopingBot {
                loop() { while (true) {} }
                onStop() { this.log('clean-stop'); }
            }",
        );
        iso.fail_execution_deadline_registration(stopped);
        iso.on_game_tick(1);
        assert!(wait_until(Duration::from_secs(2), || iso.execution_active()));
        iso.pause();
        let mut logs = Vec::new();
        assert!(
            wait_until(Duration::from_secs(2), || {
                logs.extend(iso.drain_logs());
                logs.iter().any(|line| {
                    line.contains("tick 1: execution interrupted:") && line.contains(cause)
                })
            }),
            "tick error for {cause}: {logs:?}"
        );
        assert!(
            logs.iter().any(|line| line.contains("tick 1: execution interrupted:") && line.contains(cause)),
            "user-visible tick error must state the refusal: {logs:?}"
        );
        assert!(
            !logs
                .iter()
                .any(|line| line.contains("runaway execution interrupted")),
            "supervisor refusal is not a runaway execution: {logs:?}"
        );
        iso.join();
    }
}

#[test]
fn onstop_throw_is_logged_and_isolates_are_reusable() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { throw new Error('lookup-boom'); }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let logs = iso.join();
    assert!(
        contains_line(&logs, "onStop threw") && contains_line(&logs, "lookup-boom"),
        "throw diagnostic: {logs:?}"
    );
    let iso =
        spawn_class("export default class T extends LoopingBot { loop() { globalThis.__n = 1; } }");
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__n").unwrap(), 1);
    iso.join();
}

#[test]
fn onstop_getter_throw_is_logged() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            get onStop() { throw new Error('getter-boom'); }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let logs = iso.join();
    assert!(
        contains_line(&logs, "onStop threw") && contains_line(&logs, "getter-boom"),
        "lookup/getter must be inside the guarded invoke: {logs:?}"
    );
}

#[test]
fn onstop_promise_is_not_awaited() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() {
                this.log('stopped-ok');
                return new Promise(() => {});
            }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let t0 = Instant::now();
    let logs = iso.join();
    let elapsed = t0.elapsed();
    assert!(
        contains_line(&logs, "stopped-ok"),
        "sync this.log before returned promise: {logs:?}"
    );
    assert!(
        !contains_line(&logs, "onStop threw"),
        "returned promise is ignored, not a throw: {logs:?}"
    );
    assert!(
        elapsed < Duration::from_millis(200),
        "must not await the parked promise: {elapsed:?}"
    );
    let iso = spawn_class(
        "export default class T extends LoopingBot { loop() { globalThis.__fresh = 7; } }",
    );
    iso.on_game_tick(1);
    assert_eq!(iso.probe("__fresh").unwrap(), 7);
    iso.join();
}

#[test]
fn onstop_public_actions_are_not_forwarded() {
    let iso = spawn_class(
        r#"
import { Game } from '../../api/game/Game.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
export default class T extends LoopingBot {
    loop() { ScriptRunner.stop('done'); }
    onStop() {
        this.log('stopped-ok');
        Game.setCameraYaw(100);
    }
}
"#,
    );
    iso.on_game_tick(1);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut acts = Vec::new();
    while Instant::now() < deadline {
        acts.extend(iso.drain_interacts());
        if iso.stopped() {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    acts.extend(iso.drain_interacts());
    let logs = iso.drain_logs();
    assert!(
        contains_line(&logs, "stopped-ok"),
        "hook ran: {logs:?} acts={acts:?}"
    );
    assert!(
        !acts
            .iter()
            .any(|a| matches!(a, InteractReq::SetCameraYaw { .. })),
        "onStop public Game.setCameraYaw must not be forwarded: {acts:?}"
    );
    iso.join();
}

#[test]
fn parked_loop_stop_still_runs_onstop() {
    let iso = spawn_class(
        r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends LoopingBot {
    async loop() { await Execution.delayUntil(() => false, 60_000); }
    onStop() { this.log('stopped-ok'); }
}
"#,
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    let t0 = Instant::now();
    let logs = iso.join();
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "must not wait out delayUntil: {:?}",
        t0.elapsed()
    );
    assert!(
        contains_line(&logs, "stopped-ok"),
        "parked loop then Stop still runs onStop: {logs:?}"
    );
}

#[test]
fn interrupted_running_loop_then_normal_hook() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() { while (true) {} }
            onStop() { this.log('stopped-ok'); }
        }",
    );
    iso.on_game_tick(1);
    assert!(
        wait_until(Duration::from_secs(2), || iso.execution_active()),
        "runaway loop never entered its script call"
    );
    let t0 = Instant::now();
    let logs = iso.join();
    assert!(t0.elapsed() < Duration::from_secs(2));
    assert!(
        contains_line(&logs, "stopped-ok"),
        "unwound tick must still run onStop: {logs:?}"
    );
}

#[test]
fn peer_isolate_is_untouched() {
    let a = spawn_class(
        "export default class T extends LoopingBot {
            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
            onStop() { this.log('a-stopped'); }
        }",
    );
    let b = spawn_class(
        "export default class T extends LoopingBot {
            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
            onStop() { this.log('b-stopped'); }
        }",
    );
    a.on_game_tick(1);
    b.on_game_tick(1);
    let _ = a.probe("1");
    let _ = b.probe("1");
    let a_logs = a.join();
    assert!(contains_line(&a_logs, "a-stopped"), "{a_logs:?}");
    b.on_game_tick(2);
    assert_eq!(b.probe("__n").unwrap(), 2);
    let b_before = b.drain_logs();
    assert!(
        !contains_line(&b_before, "b-stopped"),
        "peer onStop must not run: {b_before:?}"
    );
    let b_logs = b.join();
    assert!(contains_line(&b_logs, "b-stopped"), "{b_logs:?}");
}

#[test]
fn native_tick_join_is_bounded_without_onstop() {
    let iso = LoadIsolate::spawn(
        "export function tick(api) { globalThis.__rs_n = (globalThis.__rs_n || 0) + 1; }".into(),
        LoadShape::NativeTick,
        vec![],
    )
    .unwrap();
    iso.on_game_tick(1);
    let _ = iso.probe("__rs_n");
    let t0 = Instant::now();
    let logs = iso.join();
    assert!(t0.elapsed() < Duration::from_secs(2));
    assert!(
        !contains_line(&logs, "onStop threw"),
        "NativeTick has no instance hook: {logs:?}"
    );
}

#[test]
fn reset_session_does_not_run_onstop() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() { globalThis.__n = (globalThis.__n || 0) + 1; }
            onStop() { this.log('stopped-ok'); }
        }",
    );
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.reset_session_work(None);
    iso.on_game_tick(2);
    assert_eq!(iso.probe("__n").unwrap(), 2);
    let mid = iso.drain_logs();
    assert!(
        !contains_line(&mid, "stopped-ok"),
        "logout/session reset is not Stop: {mid:?}"
    );
    let logs = iso.join();
    assert!(contains_line(&logs, "stopped-ok"), "{logs:?}");
}
