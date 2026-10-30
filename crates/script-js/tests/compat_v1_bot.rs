//! Compat v1 Bot census contracts and the native TreeBot traversal.

mod common;

use script_js::{LoadIsolate, LoadShape, Ready};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const INDEX_DTS: &str = include_str!("../compat-js/index.d.ts");
const ABI_CONTRACT: &str = include_str!("../compat-js/abi-contract.json");

fn run_source(source: String) -> Value {
    let isolate = LoadIsolate::spawn(source, LoadShape::CompatClass, vec![])
        .expect("compat Bot source links");
    let snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let probe = isolate.probe("globalThis.__probe");
    let logs = isolate.join();
    probe.unwrap_or_else(|error| panic!("Bot probe failed: {error}; logs: {logs:#?}"))
}

fn setup_result(isolate: &LoadIsolate) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match isolate.poll_ready() {
            Ready::Ready => return Ok(()),
            Ready::Failed(error) => return Err(error),
            Ready::Pending if Instant::now() < deadline => std::thread::yield_now(),
            Ready::Pending => return Err("isolate setup did not finish".into()),
        }
    }
}

#[test]
#[expect(non_snake_case, reason = "the census names this removal test")]
fn compat_v1_bot__resolveloopcadence__removed() {
    assert!(!INDEX_DTS.contains("resolveLoopCadence"));
    assert!(!ABI_CONTRACT.contains("resolveLoopCadence"));

    let source = r#"
import * as Bot from '../../api/bot/Bot.js';
export default class T extends LoopingBot {
    loop() {
        const key = 'resolveLoopCadence';
        globalThis.__probe = {
            own: Object.hasOwn(Bot, key),
            key: Object.keys(Bot).includes(key),
            descriptor: Object.getOwnPropertyDescriptor(Bot, key) === undefined,
            value: Bot[key] === undefined,
            sibling: typeof Bot.TreeBot === 'function' && typeof Bot.TaskBot === 'function',
        };
    }
}
"#;
    let result = run_source(source.into());
    assert_eq!(
        result,
        json!({ "own": false, "key": false, "descriptor": true, "value": true, "sibling": true })
    );

    let source = r#"
import { resolveLoopCadence } from '../../api/bot/Bot.js';
export default class T extends LoopingBot { loop() { globalThis.__probe = resolveLoopCadence; } }
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("missing-export probe isolate starts");
    let setup = setup_result(&isolate);
    let logs = isolate.join();
    let diagnostic = format!("{setup:?}\n{logs:#?}");
    assert!(
        setup.is_err() && diagnostic.contains("resolveLoopCadence"),
        "removed named export must fail module linking: {diagnostic}"
    );
}

fn hook_source(class_name: &str, hook: &str) -> String {
    let source = r#"
import * as BotModule from '../../api/bot/Bot.js';
import * as Api from '@rs2b0t/api';
const className = '__CLASS__';
const hook = '__HOOK__';
const check = (Base, Leaf) => {
    const descriptor = Object.getOwnPropertyDescriptor(Base.prototype, hook);
    const base = new Base();
    let baseError = 'no error';
    try { base[hook](); } catch (error) { baseError = error.name; }
    const Concrete = class extends Base {
        [hook]() {
            if (hook === 'validate') return true;
            if (hook === 'execute') { globalThis.__leafRuns = (globalThis.__leafRuns || 0) + 1; return; }
            return new Leaf();
        }
    };
    const child = new Concrete();
    const value = child[hook]();
    return {
        own: descriptor !== undefined,
        instanceOwn: Object.hasOwn(base, hook),
        baseType: typeof base[hook],
        baseError,
        override: hook === 'validate' ? value === true :
            hook === 'execute' ? value === undefined : value instanceof Leaf,
    };
};
export default class T extends LoopingBot {
    loop() {
        const moduleBase = BotModule[className];
        const barrelBase = Api[className];
        globalThis.__probe = {
            same: moduleBase === barrelBase,
            module: check(moduleBase, BotModule.LeafTask),
            barrel: check(barrelBase, Api.LeafTask),
            leafRuns: globalThis.__leafRuns || 0,
        };
    }
}
"#;
    source
        .replace("__CLASS__", class_name)
        .replace("__HOOK__", hook)
}

fn assert_hook_contract(class_name: &str, hook: &str) {
    let signature = match hook {
        "validate" => "abstract validate(): boolean;",
        "success" => "abstract success(): TreeNode;",
        "failure" => "abstract failure(): TreeNode;",
        "execute" => "abstract execute(): void | Promise<void>;",
        _ => unreachable!("known census hook"),
    };
    assert!(
        INDEX_DTS.contains(signature),
        "the published abstract declaration remains: {signature}"
    );
    let result = run_source(hook_source(class_name, hook));
    for route in ["module", "barrel"] {
        assert_eq!(
            result[route]["own"], false,
            "{route} base prototype owns {hook}"
        );
        assert_eq!(
            result[route]["instanceOwn"], false,
            "{route} instance owns {hook}"
        );
        assert_eq!(
            result[route]["baseType"], "undefined",
            "{route} base {hook}"
        );
        assert_eq!(
            result[route]["baseError"], "TypeError",
            "{route} base {hook}"
        );
        assert_eq!(result[route]["override"], true, "{route} override {hook}");
    }
    assert_eq!(
        result["same"], true,
        "module and barrel retain the same class"
    );
    assert_eq!(
        result["leafRuns"],
        if hook == "execute" { 2 } else { 0 },
        "caller override runs once on each route"
    );
}
#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__branchtask_validate__contract() {
    assert_hook_contract("BranchTask", "validate");
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__branchtask_success__contract() {
    assert_hook_contract("BranchTask", "success");
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__branchtask_failure__contract() {
    assert_hook_contract("BranchTask", "failure");
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__leaftask_execute__contract() {
    assert_hook_contract("LeafTask", "execute");
}

fn constructor_source(class_name: &str) -> String {
    let source = r#"
import * as BotModule from '../../api/bot/Bot.js';
import * as Api from '@rs2b0t/api';
const className = '__CLASS__';
const construct = (Base) => {
    const base = new Base();
    const Child = class extends Base { marker() { return 'caller'; } };
    const child = new Child();
    return {
        base: base instanceof Base,
        prototype: Object.getPrototypeOf(base) === Base.prototype,
        child: child instanceof Base,
        marker: child.marker(),
    };
};
export default class T extends LoopingBot {
    loop() {
        const moduleBase = BotModule[className];
        const barrelBase = Api[className];
        globalThis.__probe = {
            same: moduleBase === barrelBase,
            module: construct(moduleBase),
            barrel: construct(barrelBase),
        };
    }
}
"#;
    source.replace("__CLASS__", class_name)
}

fn assert_constructor_contract(class_name: &str) {
    let result = run_source(constructor_source(class_name));
    assert_eq!(result["same"], true);
    for route in ["module", "barrel"] {
        assert_eq!(result[route]["base"], true, "{route} base constructs");
        assert_eq!(
            result[route]["prototype"], true,
            "{route} prototype identity"
        );
        assert_eq!(result[route]["child"], true, "{route} subclass constructs");
        assert_eq!(result[route]["marker"], "caller", "{route} caller override");
    }
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__branchtask_constructor__contract() {
    assert_constructor_contract("BranchTask");
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__leaftask_constructor__contract() {
    assert_constructor_contract("LeafTask");
}
#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__registerscript__contract() {
    assert!(
        INDEX_DTS.contains("registerScript(manifest: import('*runtime/defineBot.js').BotManifestInput, origin?: string): void;"),
        "the published registerScript declaration retains its upstream signature"
    );
    assert!(
        INDEX_DTS.contains(
            "export type { BotManifest, BotManifestInput } from '*runtime/defineBot.js';"
        ),
        "the published manifest types remain available"
    );
    let source = r#"
import { registerScript } from '@rs2b0t/api';
import { ScriptRegistry } from '../../runtime/ScriptRegistry.js';
export default class T extends LoopingBot {
    loop() {
        const tags = ['compat', 'registry'];
        const settingsSchema = {};
        const create = () => new LoopingBot();
        const meta = registerScript({
            name: 'Compat census registration',
            description: 'registered by the API',
            version: '1.2.3',
            category: 'compat',
            tags,
            settingsSchema,
            create,
        }, 'compat://census');
        const stored = ScriptRegistry.get(meta.name);
        const tagged = Object.defineProperty({
            __rs2b0tManifest: 1,
            name: 'Compat census tagged',
            create,
        }, 'unused', {
            get() { throw new Error('tagged manifest must not be spread'); },
        });
        const taggedMeta = registerScript(tagged, 'compat://tagged');
        const defaultMeta = registerScript({
            name: 'Compat census defaults',
            create,
        });
        const taggedStored = ScriptRegistry.get(taggedMeta.name);
        globalThis.__probe = {
            meta: {
                name: meta.name,
                description: meta.description,
                version: meta.version,
                category: meta.category,
                tagsSame: meta.tags === tags,
                origin: meta.origin,
                schemaSame: meta.settingsSchema === settingsSchema,
                createSame: meta.create === create,
                hasMarker: Object.hasOwn(meta, '__rs2b0tManifest'),
            },
            stored: {
                same: stored === meta,
                listSame: ScriptRegistry.list().includes(meta),
                name: stored.name,
                description: stored.description,
                version: stored.version,
                category: stored.category,
                tagsSame: stored.tags === tags,
                origin: stored.origin,
                schemaSame: stored.settingsSchema === settingsSchema,
                createSame: stored.create === create,
            },
            defaults: {
                name: defaultMeta.name,
                description: defaultMeta.description,
                origin: defaultMeta.origin === undefined,
                createSame: defaultMeta.create === create,
            },
            tagged: {
                name: taggedMeta.name,
                description: taggedMeta.description,
                origin: taggedMeta.origin,
                createSame: taggedMeta.create === create,
                storedCreateSame: taggedStored.create === create,
                same: taggedStored === taggedMeta,
                listSame: ScriptRegistry.list().includes(taggedMeta),
            },
        };
    }
}
"#;
    let result = run_source(source.into());
    assert_eq!(
        result,
        json!({
            "meta": {
                "name": "Compat census registration",
                "description": "registered by the API",
                "version": "1.2.3",
                "category": "compat",
                "tagsSame": true,
                "origin": "compat://census",
                "schemaSame": true,
                "createSame": true,
                "hasMarker": false,
            },
            "stored": {
                "name": "Compat census registration",
                "same": true,
                "listSame": true,
                "description": "registered by the API",
                "version": "1.2.3",
                "category": "compat",
                "tagsSame": true,
                "origin": "compat://census",
                "schemaSame": true,
                "createSame": true,
            },
            "defaults": {
                "name": "Compat census defaults",
                "description": "",
                "origin": true,
                "createSame": true,
            },
            "tagged": {
                "name": "Compat census tagged",
                "description": "",
                "origin": "compat://tagged",
                "createSame": true,
                "storedCreateSame": true,
                "same": true,
                "listSame": true,
            },
        })
    );
}

#[test]
#[expect(non_snake_case, reason = "the census names this contract test")]
fn compat_v1_bot__isbotmanifest__contract() {
    let source = r#"
import * as Api from '@rs2b0t/api';
import { isBotManifest } from '../../runtime/defineBot.js';
export default class T extends LoopingBot {
    loop() {
        const error = new Error('manifest marker getter');
        const throwing = Object.defineProperty({}, '__rs2b0tManifest', {
            get() { throw error; },
        });
        let getterErrorPreserved = false;
        try { isBotManifest(throwing); } catch (caught) { getterErrorPreserved = caught === error; }
        const boxed = new String('manifest');
        boxed.__rs2b0tManifest = 1;
        const callable = Object.assign(function () {}, { __rs2b0tManifest: 1 });
        globalThis.__probe = {
            marked: isBotManifest({ __rs2b0tManifest: 1 }),
            array: isBotManifest(Object.assign([], { __rs2b0tManifest: 1 })),
            boxed: isBotManifest(boxed),
            functionMarker: isBotManifest(callable),
            stringMarker: isBotManifest({ __rs2b0tManifest: '1' }),
            plain: isBotManifest({}),
            nullValue: isBotManifest(null),
            undefinedValue: isBotManifest(undefined),
            primitive: isBotManifest(1),
            getterErrorPreserved,
            notPublished: !Object.hasOwn(Api, 'isBotManifest'),
        };
    }
}
"#;
    assert_eq!(
        run_source(source.into()),
        json!({
            "marked": true,
            "array": true,
            "boxed": true,
            "functionMarker": false,
            "stringMarker": false,
            "plain": false,
            "nullValue": false,
            "undefinedValue": false,
            "primitive": false,
            "getterErrorPreserved": true,
            "notPublished": true,
        })
    );
}

#[test]
fn native_treebot_traverses_branches_and_awaits_leaf_once() {
    let source = r#"
import { TreeBot, BranchTask, LeafTask } from '@rs2b0t/api';
const trace = globalThis.__treeTrace = [];
class First extends BranchTask {
    validate() { trace.push('first.validate'); return false; }
    success() { throw new Error('wrong first success'); }
    failure() { trace.push('first.failure'); return new Second(); }
}
class Second extends BranchTask {
    validate() { trace.push('second.validate'); return true; }
    success() { trace.push('second.success'); return new Done(); }
    failure() { throw new Error('wrong second failure'); }
}
class Done extends LeafTask {
    async execute() {
        trace.push('leaf.start');
        await Promise.resolve();
        trace.push('leaf.end');
    }
}
export default class T extends TreeBot {
    root() { trace.push('root'); return new First(); }
}
"#;
    let isolate = LoadIsolate::spawn(source.into(), LoadShape::CompatClass, vec![])
        .expect("published tree script links");
    let snapshot = common::ingame_snapshot();
    common::post_snapshot_input(&isolate, &snapshot);
    isolate.on_game_tick(1);
    let trace = isolate.probe("globalThis.__treeTrace");
    let logs = isolate.join();
    assert_eq!(
        trace.unwrap_or_else(|error| panic!("TreeBot probe failed: {error}; logs: {logs:#?}")),
        json!([
            "root",
            "first.validate",
            "first.failure",
            "second.validate",
            "second.success",
            "leaf.start",
            "leaf.end"
        ])
    );
}
