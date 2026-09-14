//! Bounded native onStop + final Slot log delivery.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use script::shim::InteractReq;
use script::{LoadIsolate, LoadShape};

fn spawn_class(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn examplebot_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ExampleBot.ts");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let pin = script::js_cache::JsCache::origin_sha(&bytes);
    assert_eq!(
        pin, "8e8e27cd9c2e57cd6478132e4bfaf910678664898db44915acc08ffc501c0973",
        "tracked ExampleBot fixture drifted from rs2b0t 96410ec5 script-template"
    );
    let src = String::from_utf8(bytes).expect("ExampleBot utf-8");
    script::transpile_ts(&src).expect("transpile ExampleBot")
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
    let shape = script::load::detect_shape(&js);
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
        elapsed < Duration::from_millis(750),
        "hostile onStop must be the 50ms terminate, not the 2s abandon: {elapsed:?} logs={logs:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(20),
        "watchdog should consume the 50ms budget: {elapsed:?}"
    );
}

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if pred() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    pred()
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
        proof.deadline_workers(),
        0,
        "no idle deadline fleet while Running"
    );
    iso.on_game_tick(1);
    let t0 = Instant::now();
    let mut logs = Vec::new();
    let finished = wait_until(Duration::from_secs(2), || {
        logs.extend(iso.drain_logs());
        iso.stopped()
            && contains_line(&logs, "script requested stop")
            && contains_line(&logs, "onStop threw")
    });
    let elapsed = t0.elapsed();
    assert!(
        finished && iso.stopped(),
        "self-stop must finish from the native deadline without join/tick/pause/probe: logs={logs:?}"
    );
    assert!(
        contains_line(&logs, "script requested stop"),
        "autonomous ScriptRunner.stop must be the path: {logs:?}"
    );
    assert!(
        contains_line(&logs, "onStop threw"),
        "onStop must have entered and been interrupted before join: {logs:?}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "join ceiling is 2s: {elapsed:?} logs={logs:?}"
    );
    assert!(
        elapsed < Duration::from_millis(750),
        "self-stop hostile onStop must finish around 50ms without another tick/pause/probe: {elapsed:?} logs={logs:?}"
    );
    assert!(
        elapsed >= Duration::from_millis(20),
        "watchdog should consume the 50ms budget: {elapsed:?}"
    );
    let leftover = iso.join();
    logs.extend(leftover);
    assert!(
        wait_until(Duration::from_secs(1), || {
            proof.finished() && proof.deadline_workers() == 0
        }),
        "deadline worker must not remain after self-stop: workers={} finished={}",
        proof.deadline_workers(),
        proof.finished()
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
        proof.deadline_workers(),
        0,
        "no idle deadline fleet while Running"
    );
    assert!(!proof.finished(), "Running isolate has not consumed Stop");
    let t0 = Instant::now();
    drop(iso);
    assert!(
        t0.elapsed() < Duration::from_millis(500),
        "raw Drop must not join: {:?}",
        t0.elapsed()
    );
    assert!(
        wait_until(Duration::from_secs(2), || proof.finished()),
        "raw Drop must finish on the isolate thread before hook/resource checks"
    );
    assert!(
        !proof.invoked(),
        "raw Drop must not invoke onStop after the isolate consumed Stop"
    );
    assert_eq!(
        proof.deadline_workers(),
        0,
        "deadline workers must not remain after Drop"
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
    let logs = iso.join();
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "join ceiling is 2s after delayed entry: {elapsed:?} logs={logs:?}"
    );
    assert!(
        elapsed < Duration::from_millis(750),
        "deadline interrupt must survive delayed entry; unbounded hook would hit 2s: {elapsed:?} logs={logs:?}"
    );
}

#[test]
fn deadline_spawn_failure_skips_hook_and_stays_bounded() {
    let iso = spawn_class(
        "export default class T extends LoopingBot {
            loop() {}
            onStop() { this.log('stopped-ok'); }
        }",
    );
    let proof = iso.teardown_proof();
    iso.on_game_tick(1);
    let _ = iso.probe("1");
    iso.fail_onstop_deadline_spawn();
    let t0 = Instant::now();
    let logs = iso.join();
    let elapsed = t0.elapsed();
    assert!(
        elapsed < Duration::from_millis(500),
        "fail-closed teardown must not run an unbounded hook: {elapsed:?} logs={logs:?}"
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
        wait_until(Duration::from_secs(1), || proof.finished()),
        "fail-closed isolate must still finish"
    );
    assert!(!proof.invoked(), "fail-closed must not invoke onStop");
    assert_eq!(proof.deadline_workers(), 0);
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
    std::thread::sleep(Duration::from_millis(80));
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
    iso.reset_session_work();
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
