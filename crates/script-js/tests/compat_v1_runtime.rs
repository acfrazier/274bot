use api::identity::RequestId;
use api::local_state::{
    AccountKey, ContentScope, DomainEdit, DomainIdentity, DomainLease, ErrorCode, LocalStateDomain,
    LocalStatePort, StoreVersion, SubmitReceipt, WriteSettlement,
};
use script::local_state::{SlotState, StatePort};
use script::surface::{RuntimeControlRequest, RuntimeRunState, ScriptRuntimeContext};
use script_js::{LoadIsolate, LoadShape};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn spawn(src: &str) -> LoadIsolate {
    LoadIsolate::spawn(src.to_string(), LoadShape::CompatClass, vec![]).unwrap()
}

fn bot(imports: &str, body: &str) -> String {
    format!(
        r#"
{imports}
export default class T extends LoopingBot {{
    loop() {{
        if (globalThis.__done) return;
        globalThis.__done = true;
        {body}
    }}
}}
"#
    )
}
fn async_bot(imports: &str, body: &str) -> String {
    format!(
        r#"
{imports}
export default class T extends LoopingBot {{
    async loop() {{
        if (globalThis.__done) return;
        globalThis.__done = true;
        {body}
    }}
}}
"#
    )
}

fn run(src: &str, expr: &str) -> serde_json::Value {
    let iso = spawn(src);
    iso.on_game_tick(1);
    let value = iso.probe(expr).unwrap();
    iso.join();
    value
}
fn script_run_context(
    state: RuntimeRunState,
    source_identity: Option<&str>,
) -> Arc<ScriptRuntimeContext> {
    let context = Arc::new(ScriptRuntimeContext::new(29));
    context.set_state(state);
    context.set_source_identity(source_identity.map(str::to_owned));
    context
}

fn run_with_context(
    src: &str,
    expr: &str,
    context: Arc<ScriptRuntimeContext>,
) -> serde_json::Value {
    let iso = spawn(src);
    iso.bind_runtime_context(context);
    iso.on_game_tick(1);
    let value = iso.probe(expr).unwrap();
    iso.join();
    value
}

#[derive(Default)]
struct RuntimeSettingsPort {
    next_request: AtomicU64,
    values: Mutex<HashMap<(String, String), String>>,
}

impl LocalStatePort for RuntimeSettingsPort {
    fn lease(&self, identity: &DomainIdentity) -> Result<DomainLease, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }

    fn runtime_setting_raw(
        &self,
        identity: &DomainIdentity,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, ErrorCode> {
        if identity.domain() != LocalStateDomain::ScriptRuntimeSettings {
            return Err(ErrorCode::Validation {
                domain: identity.domain(),
                reason: api::local_state::ValidationReason::Identity,
            });
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(namespace.to_owned(), key.to_owned()))
            .cloned())
    }

    fn submit(&self, edit: DomainEdit) -> SubmitReceipt {
        let domain = edit.identity().domain();
        let DomainEdit::ScriptRuntimeSettings(edit) = edit else {
            return SubmitReceipt::Refused(ErrorCode::Busy { domain });
        };
        self.values.lock().unwrap().insert(
            (edit.namespace().to_owned(), edit.key().to_owned()),
            edit.raw().to_owned(),
        );
        let raw = self.next_request.fetch_add(1, Ordering::Relaxed) + 1;
        SubmitReceipt::Accepted {
            request: RequestId::isolate(raw).unwrap(),
        }
    }

    fn observe(&self, _request: RequestId) -> Option<WriteSettlement> {
        None
    }
}

impl StatePort for RuntimeSettingsPort {
    fn lease_version(&self, identity: &DomainIdentity) -> Result<StoreVersion, ErrorCode> {
        Err(ErrorCode::NotReady {
            domain: identity.domain(),
        })
    }
}

fn runtime_settings_state() -> Arc<SlotState> {
    let port = Arc::new(RuntimeSettingsPort::default());
    Arc::new(SlotState::new(
        port,
        AccountKey::try_new("local-289", "runtime").unwrap(),
        ContentScope::try_new(api::selected::ClientRevision::R289, "compat-runtime").unwrap(),
    ))
}

fn run_with_local_state(src: &str, expr: &str, state: Arc<SlotState>) -> serde_json::Value {
    let iso =
        LoadIsolate::spawn_with_local_state(src.to_string(), LoadShape::CompatClass, vec![], state)
            .unwrap();
    iso.on_game_tick(1);
    let value = iso.probe(expr).unwrap();
    iso.join();
    value
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__stepoffcandidates__contract() {
    let src = bot(
        "import { stepOffCandidates } from '../../runtime/randomevents/eventEvade.js';",
        r#"
const from = { x: 10, z: 20, level: 2 };
const before = JSON.stringify(from);
let candidates = null;
let error = null;
try {
    candidates = stepOffCandidates(from, { x: 6, z: 18 });
} catch (caught) {
    error = String(caught);
}
globalThis.__facts = {
    candidates,
    error,
    callbackType: typeof globalThis.__rs2b0t_step_off_candidates,
    inputUnchanged: JSON.stringify(from) === before,
};
"#,
    );
    let facts = run(&src, "globalThis.__facts");
    assert_eq!(facts["callbackType"], "function");
    assert_eq!(
        facts["error"],
        serde_json::Value::Null,
        "stepOffCandidates failed: {}",
        facts["error"]
    );
    assert_eq!(
        facts["candidates"],
        serde_json::json!([
            { "x": 11, "z": 20, "level": 2 },
            { "x": 11, "z": 21, "level": 2 },
            { "x": 11, "z": 19, "level": 2 },
            { "x": 10, "z": 21, "level": 2 },
            { "x": 10, "z": 19, "level": 2 },
            { "x": 9, "z": 21, "level": 2 },
            { "x": 9, "z": 20, "level": 2 },
            { "x": 9, "z": 19, "level": 2 },
        ]),
        "candidates are the eight east-first compass neighbors, sorted by descending Chebyshev distance with stable ties"
    );
    assert_eq!(facts["inputUnchanged"], true, "the input tile is preserved");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__settingsstore_save__contract() {
    let src = bot(
        "import { SettingsStore } from '../../runtime/Settings.js';",
        r#"
const seen = [];
SettingsStore.onChange((name, key, raw) => {
    seen.push([name, key, raw, SettingsStore.saved(name, key)]);
});
SettingsStore.save('WalkTo', 'destination', 'false');
globalThis.__settings = {
    seen,
    saved: SettingsStore.saved('WalkTo', 'destination'),
    missing: SettingsStore.saved('WalkTo', 'missing'),
};
"#,
    );
    let facts = run_with_local_state(&src, "__settings", runtime_settings_state());
    assert_eq!(
        facts,
        serde_json::json!({
            "seen": [["WalkTo", "destination", "false", "false"]],
            "saved": "false",
        }),
        "the accepted raw value is visible before synchronous listeners run and remains a string"
    );
}
#[test]
fn compat_v1_runtime_settingsstore_resolve_uses_scoped_raw_values() {
    let src = bot(
        "import { SettingsBag, SettingsStore } from '../../runtime/Settings.js';",
        r#"
globalThis.__rs2b0t_host = {
    settingsBag: {
        destination: 'Lumbridge',
        amount: 3,
        enabled: false,
        targets: ['Lumbridge'],
        tile: { x: 9, z: 9, level: 0 },
        fallback: 'host-value',
    },
};
const schema = {
    destination: {
        type: 'string',
        default: 'Draynor',
        options: ['Lumbridge', 'Map pick'],
    },
    amount: { type: 'number', default: 2, min: 0, max: 10 },
    enabled: { type: 'boolean', default: false },
    targets: {
        type: 'string[]',
        default: ['Lumbridge'],
        options: ['Lumbridge', 'Draynor'],
    },
    tile: { type: 'tile', default: { x: 1, z: 2, level: 0 } },
    fallback: { type: 'string', default: 'schema-value' },
};
SettingsStore.save('WalkTo', 'destination', ' Map PICK ');
SettingsStore.save('WalkTo', 'amount', '12');
SettingsStore.save('WalkTo', 'enabled', ' yes ');
SettingsStore.save('WalkTo', 'targets', 'Draynor, unknown, LUMBRIDGE,draynor');
SettingsStore.save('WalkTo', 'tile', '3221,3218,2');
const resolved = SettingsStore.resolve('WalkTo', schema);
const tile = new SettingsBag(resolved).tile('tile');
globalThis.__settings = {
    destination: resolved.destination,
    destinationRaw: SettingsStore.saved('WalkTo', 'destination'),
    amount: resolved.amount,
    enabled: resolved.enabled,
    targets: resolved.targets,
    tile: tile && { x: tile.x, z: tile.z, level: tile.level },
    fallback: resolved.fallback,
    fallbackRaw: SettingsStore.saved('WalkTo', 'fallback') ?? '<undefined>',
};
"#,
    );
    let facts = run_with_local_state(&src, "__settings", runtime_settings_state());
    assert_eq!(
        facts,
        serde_json::json!({
            "destination": "Map pick",
            "destinationRaw": " Map PICK ",
            "amount": 10,
            "enabled": true,
            "targets": ["Draynor", "Lumbridge", "Draynor"],
            "tile": { "x": 3221, "z": 3218, "level": 2 },
            "fallback": "schema-value",
            "fallbackRaw": "<undefined>",
        }),
        "resolve uses exact box-scoped raw values, parses each SettingDef type, and falls back to schema defaults instead of the per-card bag"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__settingsstore_onchange__contract() {
    let src = bot(
        "import { SettingsStore } from '../../runtime/Settings.js';",
        r#"
const seen = [];
const first = (name, key, raw) => seen.push(`first:${name}:${key}:${raw}`);
const throws = (name, key, raw) => {
    seen.push(`throws:${name}:${key}:${raw}`);
    throw new Error('listener failure');
};
const last = (name, key, raw) => seen.push(`last:${name}:${key}:${raw}`);
const unsubscribeFirst = SettingsStore.onChange(first);
SettingsStore.onChange(first);
SettingsStore.onChange(throws);
SettingsStore.onChange(last);
SettingsStore.save('one', 'key', 'raw-one');
const firstSave = seen.slice();
unsubscribeFirst();
SettingsStore.save('two', 'key', 'raw-two');
globalThis.__settings = { firstSave, secondSave: seen.slice(firstSave.length) };
"#,
    );
    let facts = run_with_local_state(&src, "__settings", runtime_settings_state());
    assert_eq!(
        facts,
        serde_json::json!({
            "firstSave": [
                "first:one:key:raw-one",
                "throws:one:key:raw-one",
                "last:one:key:raw-one",
            ],
            "secondSave": [
                "throws:two:key:raw-two",
                "last:two:key:raw-two",
            ],
        }),
        "the bounded Set preserves insertion order, ignores callback exceptions, and honors unsubscribe"
    );
}
#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptregistry__contract() {
    let src = bot(
        "import { ScriptRegistry } from '../../runtime/ScriptRegistry.js';",
        r#"
const firstCreate = () => ({ id: 1 });
const secondCreate = () => ({ id: 2 });
const first = { name: 'first', description: 'old', create: firstCreate };
const second = { name: 'second', create: secondCreate };
const replacement = { name: 'first', description: 'new', create: secondCreate };
ScriptRegistry.register(first);
ScriptRegistry.register(second);
ScriptRegistry.register(replacement);
const listed = ScriptRegistry.list();
globalThis.__registry = {
    names: listed.map((meta) => meta.name),
    description: ScriptRegistry.get('first').description,
    sameMetadata: ScriptRegistry.get('first') === replacement,
    sameCreate: ScriptRegistry.get('second').create === secondCreate,
    missing: ScriptRegistry.get('missing') === undefined,
};
"#,
    );
    assert_eq!(
        run(&src, "__registry"),
        serde_json::json!({
            "names": ["first", "second"],
            "description": "new",
            "sameMetadata": true,
            "sameCreate": true,
            "missing": true,
        }),
        "native registry metadata preserves callback identity, insertion order and replacement semantics"
    );
}

fn assert_export_removed(module: &str, name: &str) {
    for (path, contents) in [
        ("index.d.ts", include_str!("../compat-js/index.d.ts")),
        ("modules.json", include_str!("../compat-js/modules.json")),
        (
            "abi-contract.json",
            include_str!("../compat-js/abi-contract.json"),
        ),
        ("overlay.json", include_str!("../compat-js/overlay.json")),
        (
            "overlay-not-impl.json",
            include_str!("../compat-js/overlay-not-impl.json"),
        ),
        (
            "overlay-scene.json",
            include_str!("../compat-js/overlay-scene.json"),
        ),
        (
            "overlay-native.json",
            include_str!("../compat-js/overlay-native.json"),
        ),
    ] {
        assert!(
            !contents.contains(name),
            "{path} still promises removed export {name}"
        );
    }
    let src = bot(&format!("import {{ {name} }} from '{module}';"), "");
    let iso = LoadIsolate::spawn(src, LoadShape::CompatClass, vec![]).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    let error = loop {
        match iso.poll_ready() {
            script_js::Ready::Failed(error) => break error,
            script_js::Ready::Ready => {
                iso.join();
                panic!("importing removed {name} linked");
            }
            script_js::Ready::Pending if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            script_js::Ready::Pending => panic!("isolate setup timed out importing {name}"),
        }
    };
    iso.join();
    assert!(
        error.contains(name),
        "the link error names removed {name}: {error}"
    );
}

#[test]
fn compat_v1_runtime_cut_exports_preserve_supported_siblings() {
    let src = bot(
        r#"
import { RunManager } from '../../runtime/RunManager.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
import { SettingsBag, SettingsStore } from '../../runtime/Settings.js';
"#,
        r#"
const bag = SettingsStore.globalBag();
globalThis.__facts = {
    runOverride: typeof RunManager.override,
    stop: typeof ScriptRunner.stop,
    paintControls: typeof ScriptRunner.paintControls,
    save: typeof SettingsStore.save,
    onChange: typeof SettingsStore.onChange,
    saved: typeof SettingsStore.saved,
    displayString: typeof SettingsStore.displayString,
    resolve: typeof SettingsStore.resolve,
    globalBag: bag instanceof SettingsBag,
    bool: typeof bag.bool,
};
"#,
    );
    let facts = run(&src, "__facts");
    assert_eq!(facts["runOverride"], "function");
    assert_eq!(facts["stop"], "function");
    assert_eq!(facts["paintControls"], "function");
    assert_eq!(facts["save"], "function");
    assert_eq!(facts["onChange"], "function");
    assert_eq!(facts["saved"], "function");
    assert_eq!(facts["displayString"], "function");
    assert_eq!(facts["resolve"], "function");
    assert_eq!(facts["globalBag"], true);
    assert_eq!(facts["bool"], "function");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__resolverunpolicy__removed() {
    assert_export_removed("../../runtime/RunManager.js", "resolveRunPolicy");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__shouldenablerun__removed() {
    assert_export_removed("../../runtime/RunManager.js", "shouldEnableRun");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__loopreadyordetached__removed() {
    assert_export_removed("../../runtime/ScriptRunner.js", "loopReadyOrDetached");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__stopreasonof__removed() {
    assert_export_removed("../../runtime/ScriptRunner.js", "stopReasonOf");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__settingoptionlabel__removed() {
    assert_export_removed("../../runtime/Settings.js", "settingOptionLabel");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__global_settings_core__removed() {
    assert_export_removed("../../runtime/Settings.js", "GLOBAL_SETTINGS_CORE");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__nav_settings__removed() {
    assert_export_removed("../../runtime/Settings.js", "NAV_SETTINGS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__global_settings__removed() {
    assert_export_removed("../../runtime/Settings.js", "GLOBAL_SETTINGS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__map_picker_settings_ns__removed() {
    assert_export_removed("../../runtime/Settings.js", "MAP_PICKER_SETTINGS_NS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__map_picker_settings__removed() {
    assert_export_removed("../../runtime/Settings.js", "MAP_PICKER_SETTINGS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__loadout_settings_ns__removed() {
    assert_export_removed("../../runtime/Settings.js", "LOADOUT_SETTINGS_NS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__loadout_settings__removed() {
    assert_export_removed("../../runtime/Settings.js", "LOADOUT_SETTINGS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__price_book_settings_ns__removed() {
    assert_export_removed("../../runtime/Settings.js", "PRICE_BOOK_SETTINGS_NS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this removal test")]
fn compat_v1_runtime__price_book_settings__removed() {
    assert_export_removed("../../runtime/Settings.js", "PRICE_BOOK_SETTINGS");
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__bothost_addframelistener__contract() {
    let no_listener = bot(
        "import { BotHost } from '../../runtime/BotHost.js';",
        "globalThis.__frames = 0;",
    );
    let mut empty = spawn(&no_listener);
    empty.on_game_tick(1);
    assert!(
        !empty.test_client_frame(),
        "zero listeners do not queue frames"
    );
    assert_eq!(empty.probe("__frames").unwrap(), 0);
    empty.join();

    let src = bot(
        r#"
import { BotHost } from '../../runtime/BotHost.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
"#,
        r#"
globalThis.__frames = 0;
globalThis.__subscribe = () => BotHost.addFrameListener(() => ++globalThis.__frames);
globalThis.__unsubscribe = __subscribe();
globalThis.__stop = () => ScriptRunner.stop('frame census stop');
"#,
    );
    let mut iso = spawn(&src);
    iso.on_game_tick(1);
    assert_eq!(
        iso.probe("__frames").unwrap(),
        0,
        "the tick registers the listener before client frames arrive"
    );
    assert!(
        iso.test_client_frame(),
        "one listener receives a client frame"
    );
    assert!(
        iso.test_client_frame(),
        "frames are delivered independently"
    );
    assert_eq!(iso.probe("__frames").unwrap(), 2);

    iso.probe("__unsubscribe()").unwrap();
    assert!(
        !iso.test_client_frame(),
        "removal returns the producer to its zero-listener fast path"
    );
    assert_eq!(iso.probe("__frames").unwrap(), 2);

    iso.probe("__unsubscribe = __subscribe()").unwrap();
    iso.pause();
    assert!(
        iso.test_client_frame(),
        "operator pause parks ScriptRunner work, not BotHost client frames"
    );
    assert_eq!(iso.probe("__frames").unwrap(), 3);
    iso.resume();
    assert!(iso.test_client_frame());
    assert_eq!(iso.probe("__frames").unwrap(), 4);

    iso.probe("__stop()").unwrap();
    iso.on_game_tick(2);
    let deadline = Instant::now() + Duration::from_secs(5);
    while iso.script_stop_receipt().is_none() && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        iso.script_stop_receipt().is_some(),
        "stop receipt arrives before joining"
    );
    assert!(
        !iso.test_client_frame(),
        "stopped isolates receive no frames"
    );
    iso.join();
}
#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptrunner_state__contract() {
    let src = bot(
        "import { ScriptRunner } from '../../runtime/ScriptRunner.js';",
        r#"
globalThis.__rs2b0t_host.snapshot = { guardian_active_event: 'abyssal guardian' };
ScriptRunner.ctx.watchdogHold = 'scene-load';
globalThis.__stateFacts = {
    state: ScriptRunner.state,
    watchdogHold: ScriptRunner.ctx.watchdogHold,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__stateFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "state": "running", "watchdogHold": "scene-load" }),
        "a host hold does not masquerade as an operator pause"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptrunner_pause__contract() {
    let src = bot(
        "import { ScriptRunner } from '../../runtime/ScriptRunner.js';",
        r#"
ScriptRunner.pause();
globalThis.__pauseFacts = { state: ScriptRunner.state };
"#,
    );
    let context = script_run_context(RuntimeRunState::Running, None);
    let iso = spawn(&src);
    iso.bind_runtime_context(Arc::clone(&context));
    iso.on_game_tick(1);
    let facts = iso.probe("__pauseFacts").unwrap();
    assert_eq!(
        context.snapshot().control_request,
        Some(RuntimeControlRequest::Pause)
    );
    iso.join();
    assert_eq!(
        facts,
        serde_json::json!({ "state": "running" }),
        "pause queues a slot-owned request without claiming acknowledgement"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptrunner_resume__contract() {
    let src = bot(
        "import { ScriptRunner } from '../../runtime/ScriptRunner.js';",
        r#"
ScriptRunner.resume();
globalThis.__resumeFacts = { state: ScriptRunner.state };
"#,
    );
    let context = script_run_context(RuntimeRunState::Paused, None);
    let iso = spawn(&src);
    iso.bind_runtime_context(Arc::clone(&context));
    iso.on_game_tick(1);
    let facts = iso.probe("__resumeFacts").unwrap();
    assert_eq!(
        context.snapshot().control_request,
        Some(RuntimeControlRequest::Resume)
    );
    iso.join();
    assert_eq!(
        facts,
        serde_json::json!({ "state": "paused" }),
        "resume queues a slot-owned request without claiming acknowledgement"
    );
}

/// Upstream `this.log` writes at once. An operator pause parks ticks, so the
/// rows that `onPause` writes must reach the host without waiting for a tick
/// (the WalkTo live cell waited on them before it resumed).
#[test]
fn operator_pause_and_resume_hook_logs_reach_the_host_without_a_tick() {
    let src = r#"
export default class T extends LoopingBot {
    loop() {}
    onPause() { this.log('HOOK:PAUSE'); }
    onResume() { this.log('HOOK:RESUME'); }
}
"#;
    let iso = spawn(src);
    iso.on_game_tick(1);
    iso.probe("true").unwrap();
    let _ = iso.drain_logs();

    iso.pause();
    // Commands run in order, so the hook ran before this probe answers.
    iso.probe("true").unwrap();
    let paused = iso.drain_logs();
    assert_eq!(
        paused.iter().filter(|line| *line == "HOOK:PAUSE").count(),
        1,
        "the pause hook row is delivered once, with no tick: {paused:?}"
    );
    assert!(
        !paused.iter().any(|line| line == "HOOK:RESUME"),
        "{paused:?}"
    );

    iso.resume();
    iso.probe("true").unwrap();
    let resumed = iso.drain_logs();
    assert_eq!(
        resumed,
        vec!["HOOK:RESUME".to_string()],
        "the resume hook row is delivered once, before the next tick"
    );
    iso.join();
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptrunner_ctx__contract() {
    let src = bot(
        "import { ScriptRunner } from '../../runtime/ScriptRunner.js';",
        r#"
globalThis.__rs2b0t_host.snapshot = { guardian_active_event: 'abyssal guardian' };
const context = ScriptRunner.ctx;
context.watchdogHold = 'scene-load';
globalThis.__retainedContext = context;
globalThis.__contextFacts = {
    same: context === ScriptRunner.ctx,
    keys: Object.keys(context),
    state: context.state,
    aborted: context.aborted,
    stopReason: context.stopReason,
    activeEvent: context.activeEvent,
    watchdogHold: context.watchdogHold,
    abortWaitersType: typeof context.abortWaiters,
};
"#,
    );
    let iso = spawn(&src);
    iso.bind_runtime_context(script_run_context(RuntimeRunState::Running, None));
    iso.on_game_tick(1);
    let current = iso.probe("__contextFacts").unwrap();
    iso.bind_runtime_context(Arc::new(ScriptRuntimeContext::new(30)));
    let stale = iso
        .probe(
            r#"({
                state: __retainedContext.state,
                aborted: __retainedContext.aborted,
                stopReason: __retainedContext.stopReason,
                activeEvent: __retainedContext.activeEvent,
                watchdogHold: __retainedContext.watchdogHold,
                abortWaitersReturnsUndefined: __retainedContext.abortWaiters() === undefined,
            })"#,
        )
        .unwrap();
    iso.join();
    assert_eq!(
        current,
        serde_json::json!({
            "same": true,
            "keys": ["state", "aborted", "stopReason", "activeEvent", "watchdogHold", "abortWaiters"],
            "state": "running",
            "aborted": false,
            "stopReason": null,
            "activeEvent": "abyssal guardian",
            "watchdogHold": "scene-load",
            "abortWaitersType": "function",
        }),
        "the active context projects its current generation and owned event"
    );
    assert_eq!(
        stale,
        serde_json::json!({
            "state": "stopped",
            "aborted": true,
            "stopReason": null,
            "activeEvent": null,
            "watchdogHold": null,
            "abortWaitersReturnsUndefined": true,
        }),
        "a retained context cannot observe or mutate the next run"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptrunner_meta__contract() {
    let src = bot(
        r#"
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
import { ScriptRegistry } from '../../runtime/ScriptRegistry.js';
"#,
        r#"
const expected = { name: 'first', create: () => ({}) };
ScriptRegistry.register(expected);
globalThis.__readMeta = () => ScriptRunner.meta;
globalThis.__metaFacts = {
    sameMetadata: ScriptRunner.meta === expected,
    name: ScriptRunner.meta.name,
};
"#,
    );
    let context = script_run_context(RuntimeRunState::Running, Some("catalog:first"));
    let iso = spawn(&src);
    iso.bind_runtime_context(Arc::clone(&context));
    iso.on_game_tick(1);
    let facts = iso.probe("__metaFacts").unwrap();
    context.set_source_identity(Some("catalog:missing".into()));
    let missing = iso.probe("__readMeta() === null").unwrap();
    iso.join();
    assert_eq!(
        facts,
        serde_json::json!({ "sameMetadata": true, "name": "first" }),
        "meta is the exact registered metadata owned by the current catalog run"
    );
    assert_eq!(
        missing, true,
        "an unregistered run identity has no fabricated metadata"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scheduler__contract() {
    let src = bot(
        "import { Scheduler } from '../../runtime/Scheduler.js';",
        "globalThis.__schedulerAbsent = Scheduler.active === null;",
    );
    assert_eq!(
        run(&src, "__schedulerAbsent"),
        true,
        "Scheduler has no active context when no native run is bound"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scheduler_active__contract() {
    let src = bot(
        r#"
import { Scheduler } from '../../runtime/Scheduler.js';
import { ScriptRunner } from '../../runtime/ScriptRunner.js';
"#,
        r#"
globalThis.__schedulerActive = {
    present: Scheduler.active !== null,
    stable: Scheduler.active === Scheduler.active,
    sameAsRunnerContext: Scheduler.active === ScriptRunner.ctx,
    state: Scheduler.active.state,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__schedulerActive",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({
            "present": true,
            "stable": true,
            "sameAsRunnerContext": true,
            "state": "running",
        }),
        "active returns one stable facade for the bound native run"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scheduler_active_abortwaiters__contract() {
    let src = async_bot(
        r#"
import { ScriptActivity } from '../../runtime/ScriptActivity.js';
import { Scheduler } from '../../runtime/Scheduler.js';
import { Execution } from '../../api/execution/Execution.js';
"#,
        r#"
globalThis.__rootWaitOutcome = null;
globalThis.__childWaitOutcome = null;
Execution.delayTicks(99).catch((error) => {
    globalThis.__rootWaitOutcome = error.name;
});
const activity = new ScriptActivity(() => {});
const meta = {
    name: 'WaiterChild',
    settingsSchema: {},
    create: () => new class extends LoopingBot {
        onStart() {
            Execution.delayTicks(99).catch((error) => {
                globalThis.__childWaitOutcome = error.name;
            });
            Scheduler.active.abortWaiters();
        }
    },
};
await activity.start(meta, {});
globalThis.__waiterFacts = {
    pendingRootWaits: globalThis.__rs2b0t_wait_queue().filter((wait) => wait.activityId === null).length,
    pendingChildWaits: globalThis.__rs2b0t_wait_queue().filter((wait) => wait.activityId !== null).length,
    rootOutcome: globalThis.__rootWaitOutcome,
    childOutcome: globalThis.__childWaitOutcome,
    state: Scheduler.active.state,
    aborted: Scheduler.active.aborted,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__waiterFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({
            "pendingRootWaits": 1,
            "pendingChildWaits": 0,
            "rootOutcome": null,
            "childOutcome": "ScriptAborted",
            "state": "running",
            "aborted": false,
        }),
        "the activity-scoped waiter is cancelled exactly once while the parent run and wait survive"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scheduler_active_aborted__contract() {
    let src = bot(
        "import { Scheduler } from '../../runtime/Scheduler.js';",
        "globalThis.__activeAborted = { present: Scheduler.active !== null, aborted: Scheduler.active.aborted, state: Scheduler.active.state };",
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activeAborted",
            script_run_context(RuntimeRunState::Stopping, None),
        ),
        serde_json::json!({ "present": true, "aborted": true, "state": "stopping" }),
        "the active facade reflects native cancellation rather than a host hold"
    );
}
#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity__contract() {
    let src = bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
globalThis.__scriptActivitySurface = {
    constructor: typeof ScriptActivity,
    methods: ['start', 'loop', 'stop', 'interrupt'].every(
        (name) => typeof ScriptActivity.prototype[name] === 'function',
    ),
    projections: ['child', 'outcome', 'currentTask'].every((name) => {
        const descriptor = Object.getOwnPropertyDescriptor(ScriptActivity.prototype, name);
        return descriptor && typeof descriptor.get === 'function';
    }),
};
"#,
    );
    assert_eq!(
        run(&src, "__scriptActivitySurface"),
        serde_json::json!({ "constructor": "function", "methods": true, "projections": true }),
        "the exported ScriptActivity class exposes its lifecycle methods and read-only projections"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_constructor__contract() {
    let src = bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
const activity = new ScriptActivity(() => {});
globalThis.__activityConstructorFacts = {
    child: activity.child,
    outcome: activity.outcome,
    currentTask: activity.currentTask,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityConstructorFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "child": null, "outcome": null, "currentTask": null }),
        "constructing a run-fenced handle starts no child or worker"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_start__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
let created = 0;
let stopped = 0;
const logs = [];
const activity = new ScriptActivity((message) => logs.push(message));
const meta = {
    name: 'Worker',
    settingsSchema: { mode: { default: 'catalog' } },
    create: () => {
        created += 1;
        return new class extends LoopingBot {
            onStart() {
                globalThis.__startedMode = this.settings.str('mode');
                this.log(`ready ${this.settings.str('mode')}`);
            }
            onStop() { stopped += 1; }
        };
    },
};
await activity.start(meta, { mode: 'override' });
let secondStartError = null;
try {
    await activity.start(meta, {});
} catch (error) {
    secondStartError = error.message;
}
const started = {
    childPresent: activity.child !== null,
    mode: activity.child.settings.str('mode'),
    startedMode: globalThis.__startedMode,
    logs,
    created,
    secondStartError,
};
activity.stop();
globalThis.__activityStartFacts = {
    ...started,
    childAfterStop: activity.child,
    stopped,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityStartFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({
            "childPresent": true,
            "mode": "override",
            "startedMode": "override",
            "logs": ["[Worker] ready override"],
            "created": 1,
            "secondStartError": "An activity is already running",
            "childAfterStop": null,
            "stopped": 1,
        }),
        "start creates one child, applies caller overrides, retains its logger, and refuses a second child"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_loop__contract() {
    let src = async_bot(
        r#"
import { ScriptActivity } from '../../runtime/ScriptActivity.js';
import { BotHost } from '../../runtime/BotHost.js';
"#,
        r#"
globalThis.__rs2b0t_host.snapshot = { tick: 10 };
let loops = 0;
let stops = 0;
const activity = new ScriptActivity(() => {});
const meta = {
    name: 'Looper',
    settingsSchema: {},
    create: () => new class extends LoopingBot {
        loop() { loops += 1; }
        onStop() { stops += 1; }
    },
};
await activity.start(meta, {});
await activity.loop();
await activity.loop();
const loopCount = loops;
activity.stop();
globalThis.__activityLoopFacts = {
    loopCount,
    stopped: stops,
    child: activity.child,
    tick: BotHost.tickCount,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityLoopFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "loopCount": 1, "stopped": 1, "child": null, "tick": 10 }),
        "one eligible child turn advances its server-tick bound and prevents same-tick reentry"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_stop__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
let stopCount = 0;
const activity = new ScriptActivity(() => {});
await activity.start({
    name: 'StopOnce',
    settingsSchema: {},
    create: () => new class extends LoopingBot {
        onStop() { stopCount += 1; }
    },
}, {});
activity.stop();
activity.stop();
globalThis.__activityStopFacts = {
    stopCount,
    child: activity.child,
    outcome: activity.outcome,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityStopFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "stopCount": 1, "child": null, "outcome": null }),
        "stop cleans the owned child exactly once without stopping its parent"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_interrupt__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
let stopCount = 0;
const activity = new ScriptActivity(() => {});
await activity.start({
    name: 'Interruptible',
    settingsSchema: {},
    create: () => new class extends LoopingBot {
        onStop() { stopCount += 1; }
    },
}, {});
activity.interrupt('operator request');
globalThis.__activityInterruptFacts = {
    outcome: activity.outcome,
    child: activity.child,
    stopCount,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityInterruptFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({
            "outcome": { "kind": "interrupted", "reason": "operator request" },
            "child": null,
            "stopCount": 1,
        }),
        "interrupt publishes the exact child outcome and cleans only that child"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_outcome__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
const logs = [];
const activity = new ScriptActivity((message) => logs.push(message));
await activity.start({
    name: 'Failing',
    settingsSchema: {},
    create: () => new class extends LoopingBot {
        loop() { throw new Error('child failed'); }
    },
}, {});
await activity.loop();
globalThis.__activityOutcomeFacts = {
    outcome: activity.outcome,
    child: activity.child,
    logs,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityOutcomeFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({
            "outcome": { "kind": "failed", "reason": "child failed" },
            "child": null,
            "logs": ["failed: child failed"],
        }),
        "the actual child failure is retained with its reason and logged before cleanup"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_child__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
let created;
const activity = new ScriptActivity(() => {});
await activity.start({
    name: 'Identity',
    settingsSchema: {},
    create: () => (created = new class extends LoopingBot {}),
}, {});
const sameChild = activity.child === created;
activity.stop();
globalThis.__activityChildFacts = {
    sameChild,
    released: activity.child === null,
};
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityChildFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "sameChild": true, "released": true }),
        "child projects the exact factory object and releases its handle after cleanup"
    );
}

#[test]
#[expect(non_snake_case, reason = "the runtime census names this contract test")]
fn compat_v1_runtime__scriptactivity_currenttask__contract() {
    let src = async_bot(
        "import { ScriptActivity } from '../../runtime/ScriptActivity.js';",
        r#"
let entered;
const enteredTask = new Promise((resolve) => { entered = resolve; });
let releaseTask;
const activity = new ScriptActivity(() => {});
await activity.start({
    name: 'TaskRunner',
    settingsSchema: {},
    create: () => new class extends TaskBot {
        constructor() {
            super();
            this.loopDelay = 0;
            this.add({
                label: 'bank-item',
                validate: () => true,
                execute: () => {
                    entered();
                    return new Promise((resolve) => { releaseTask = resolve; });
                },
            });
        }
    },
}, {});
const loop = activity.loop();
await enteredTask;
const activeTask = activity.currentTask;
releaseTask();
await loop;
const idleTask = activity.currentTask;
activity.stop();
globalThis.__activityTaskFacts = { activeTask, idleTask };
"#,
    );
    assert_eq!(
        run_with_context(
            &src,
            "__activityTaskFacts",
            script_run_context(RuntimeRunState::Running, None),
        ),
        serde_json::json!({ "activeTask": "bank-item", "idleTask": null }),
        "currentTask reports the executing child task only during its actual execution"
    );
}

#[test]
fn client_cross_root_helpers_match_the_catalog_contract() {
    let src = bot(
        r#"
import { resolveWorldNumber } from '../../../client/config/worlds.js';
import Skill from '../../../client/shell/Skill.js';
"#,
        r#"
const params = values => ({
    getAll(key) {
        return key === 'world' ? values : [];
    },
});
const invalidWorld = values => {
    try {
        return resolveWorldNumber('w2.rs2b2t.com', params(values));
    } catch (error) {
        return String(error?.message ?? error);
    }
};
globalThis.__clientHelperFacts = {
    hosted: resolveWorldNumber('w2.rs2b2t.com:443', params([])),
    fallback: resolveWorldNumber('localhost:8081', params([])),
    explicit: resolveWorldNumber('w1.rs2b2t.com', params(['3'])),
    invalid: invalidWorld(['4']),
    duplicate: invalidWorld(['1', '2']),
    skillNames: Skill.names,
    skillUsed: Skill.used,
};
"#,
    );
    assert_eq!(
        run(&src, "globalThis.__clientHelperFacts"),
        serde_json::json!({
            "hosted": 2,
            "fallback": 1,
            "explicit": 3,
            "invalid": "world must be 1, 2 or 3",
            "duplicate": "world must be 1, 2 or 3",
            "skillNames": [
                "attack", "defence", "strength", "hitpoints", "ranged", "prayer", "magic",
                "cooking", "woodcutting", "fletching", "fishing", "firemaking", "crafting",
                "smithing", "mining", "herblore", "agility", "thieving", "slayer", "-unused-",
                "runecraft", "-unused-", "-unused-", "-unused-", "-unused-"
            ],
            "skillUsed": [
                true, true, true, true, true, true, true, true, true, true, true, true, true,
                true, true, true, true, true, false, false, true, false, false, false, false
            ],
        })
    );
}
