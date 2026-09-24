use script::load::{LoadIsolate, LoadShape};

fn spawn_task_bot(source: &str) -> LoadIsolate {
    LoadIsolate::spawn(source.to_string(), LoadShape::CompatClass, vec![])
        .expect("spawn TaskBot isolate")
}

#[test]
fn async_false_validation_does_not_execute_or_starve_later_task() {
    let source = r#"
export default class T extends TaskBot {
    onStart() {
        this.events = [];
        this.add(
            {
                validate: async () => false,
                execute: async () => this.events.push('wrong'),
            },
            {
                validate: async () => true,
                execute: async () => this.events.push('right'),
            },
        );
    }
}
"#;
    let isolate = spawn_task_bot(source);

    isolate.on_game_tick(1);
    let events = isolate
        .probe("__rs_bot.events")
        .expect("task results are readable");

    assert_eq!(
        events,
        serde_json::json!(["right"]),
        "a Promise resolving false must not select or execute its task"
    );
    isolate.join();
}

#[test]
fn task_selection_waits_for_pending_validation() {
    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends TaskBot {
    onStart() {
        this.events = [];
        this.add(
            {
                validate: async () => {
                    this.events.push('first:start');
                    const valid = await Execution.delayUntilTicks(() => false, 2);
                    this.events.push(`first:${valid}`);
                    return valid;
                },
                execute: async () => this.events.push('wrong'),
            },
            {
                validate: async () => {
                    this.events.push('second:validate');
                    return true;
                },
                execute: async () => this.events.push('right'),
            },
        );
    }
}
"#;
    let isolate = spawn_task_bot(source);

    isolate.on_game_tick(1);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["first:start"]),
        "no later task may be selected while an earlier validation is pending"
    );
    isolate.on_game_tick(2);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["first:start"]),
        "pending validation remains the one selected operation"
    );
    isolate.on_game_tick(3);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["first:start", "first:false", "second:validate", "right"]),
        "selection continues in priority order only after validation settles"
    );
    isolate.join();
}

#[test]
fn validation_rejection_is_surfaced_without_execution() {
    let source = r#"
export default class T extends TaskBot {
    onStart() {
        this.executed = false;
        this.add({
            validate: async () => { throw new Error('validation boom'); },
            execute: async () => { this.executed = true; },
        });
    }
}
"#;
    let isolate = spawn_task_bot(source);

    isolate.on_game_tick(1);
    assert_eq!(isolate.probe("__rs_bot.executed").unwrap(), false);
    let logs = isolate.drain_logs();
    assert!(
        logs.iter().any(|line| line.contains("validation boom")),
        "validation rejection must reach the isolate log: {logs:?}"
    );
    isolate.join();
}

#[test]
fn pause_freezes_pending_validation_until_resume() {
    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends TaskBot {
    onStart() {
        this.events = [];
        this.add({
            validate: async () => {
                this.events.push('validate:start');
                await Execution.delayTicks(2);
                this.events.push('validate:done');
                return true;
            },
            execute: async () => this.events.push('execute'),
        });
    }
}
"#;
    let isolate = spawn_task_bot(source);

    isolate.on_game_tick(1);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["validate:start"])
    );
    isolate.pause();
    isolate.on_game_tick(2);
    isolate.on_game_tick(3);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["validate:start"]),
        "paused ticks must not settle validation or dispatch execution"
    );
    isolate.resume();
    isolate.on_game_tick(4);
    assert_eq!(
        isolate.probe("__rs_bot.events").unwrap(),
        serde_json::json!(["validate:start", "validate:done", "execute"]),
        "resume allows the pending validation to settle before execution"
    );
    isolate.join();
}

#[test]
fn stop_discards_pending_validation_without_gameplay_dispatch() {
    let source = r#"
import { Execution } from '../../api/execution/Execution.js';
export default class T extends TaskBot {
    onStart() {
        this.add({
            validate: async () => {
                globalThis.__validationStarted = true;
                await Execution.delayTicks(1000);
                return true;
            },
            execute: async () => {
                globalThis.__rs2b0t_host.interact = [{ kind: 'continue-dialog' }];
            },
        });
    }
}
"#;
    let mut slot = script::SlotScript::new();
    slot.start_load(source.to_string(), LoadShape::CompatClass, vec![])
        .expect("start TaskBot slot");
    // Start returns before V8 setup; ticks dispatch once the slot is Running.
    wait_slot_state(&mut slot, script::RunState::Running);
    let mut driver = NullDriver { out: NullOut };

    slot.on_game_tick(&mut script::ctx::ScriptCtx {
        driver: &mut driver,
        tick: 1,
        here: None,
        walk: None,
        walk_with: None,
        inv: None,
        snapshot: None,
        obj_names: None,
        compiled: script::CompiledTick::default(),
    });
    assert_eq!(slot.probe("__validationStarted").unwrap(), true);
    slot.stop();

    // Stop returns before the reap; the reap ends Idle.
    wait_slot_state(&mut slot, script::RunState::Idle);
    assert!(
        slot.drain_interacts().is_empty(),
        "Stop destroys the pending isolate instead of forwarding late gameplay"
    );
}

/// Pump the slot's lifecycle observe until it reaches `want` (bounded).
fn wait_slot_state(slot: &mut script::SlotScript, want: script::RunState) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while slot.state() != want && std::time::Instant::now() < deadline {
        slot.observe_lifecycle();
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert_eq!(slot.state(), want, "last_error={:?}", slot.last_error());
}

struct NullDriver {
    out: NullOut,
}

impl api::interact::Driver for NullDriver {
    fn set_menu(&mut self, _slot: i32, _action: i32, _a: i32, _b: i32, _c: i32) {}

    fn do_action(&mut self, _slot: i32) -> bool {
        true
    }

    fn try_move(
        &mut self,
        _src_x: i32,
        _src_z: i32,
        _dx: i32,
        _dz: i32,
        _try_nearest: bool,
        _loc_width: i32,
        _loc_length: i32,
        _loc_angle: i32,
        _loc_shape: i32,
        _forceapproach: i32,
        _type: i32,
    ) -> bool {
        true
    }

    fn local_route(&self) -> Option<(i32, i32)> {
        None
    }

    fn build_base(&self) -> (i32, i32) {
        (0, 0)
    }

    fn loc_typecode(&self, _scene_x: i32, _scene_z: i32) -> Option<i32> {
        None
    }

    fn out(&mut self) -> &mut dyn api::prot::Out {
        &mut self.out
    }

    fn login(&mut self, _username: &str, _password: &str, _reconnect: bool) -> bool {
        true
    }
}

struct NullOut;

impl api::prot::Out for NullOut {
    fn p1_enc(&mut self, _opcode: i32) {}
    fn p1(&mut self, _value: i32) {}
    fn p2(&mut self, _value: i32) {}
    fn p4(&mut self, _value: i32) {}
    fn pjstr(&mut self, _s: &str) {}
}
