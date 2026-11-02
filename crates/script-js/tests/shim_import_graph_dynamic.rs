#![cfg(feature = "test-hooks")]

use std::time::{Duration, Instant};

use script_js::{LoadIsolate, LoadShape, Ready};

const REGIONS_URL: &str = "/rs2b0t/bot/api/map/regions.js";
const TAIL: &str = "\nexport default class P extends LoopingBot { loop() {} }";

const ESCAPED_IMPORTS: &[(&str, &str)] = &[
    (
        "indirect-eval-comma",
        "globalThis.__regionsKeys = Object.keys(await (0, eval)(\"import('URL')\")).sort();",
    ),
    (
        "globalThis-eval",
        "globalThis.__regionsKeys = Object.keys(await globalThis.eval(\"import('URL')\")).sort();",
    ),
    (
        "aliased-eval",
        "const e = eval;\nglobalThis.__regionsKeys = Object.keys(await e(\"import('URL')\")).sort();",
    ),
    (
        "optional-call-eval",
        "globalThis.__regionsKeys = Object.keys(await eval?.(\"import('URL')\")).sort();",
    ),
    (
        "tagged-template-Function",
        "globalThis.__regionsKeys = Object.keys(await Function`return import('URL')`()).sort();",
    ),
    (
        "new-globalThis-Function",
        "globalThis.__regionsKeys = Object.keys(await new globalThis.Function(\"return import('URL')\")()).sort();",
    ),
    (
        "Function-call-method",
        "globalThis.__regionsKeys = Object.keys(await Function.call(null, \"return import('URL')\")()).sort();",
    ),
    (
        "Reflect-construct-Function",
        "globalThis.__regionsKeys = Object.keys(await Reflect.construct(Function, [\"return import('URL')\"])()).sort();",
    ),
    (
        "AsyncFunction-constructor",
        "globalThis.__regionsKeys = Object.keys(await (async () => {}).constructor(\"return import('URL')\")()).sort();",
    ),
    (
        "optional-call-Function",
        "globalThis.__regionsKeys = Object.keys(await Function?.(\"return import('URL')\")()).sort();",
    ),
];

fn source(body: &str) -> String {
    format!("{}{}", body.replace("URL", REGIONS_URL), TAIL)
}

fn load_regions(source: String) -> (serde_json::Value, Duration) {
    let started = Instant::now();
    let isolate = LoadIsolate::spawn(source, LoadShape::CompatClass, Vec::new())
        .expect("spawn compat isolate");
    let _ = isolate.probe("true");
    let ready = isolate.poll_ready();
    let elapsed = started.elapsed();
    if ready != Ready::Ready {
        isolate.join();
        panic!("module load did not become ready: {ready:?}");
    }
    let keys = isolate.probe("globalThis.__regionsKeys");
    isolate.join();
    (keys.expect("regions namespace was read"), elapsed)
}

fn static_regions_source() -> String {
    source(&format!(
        "import * as regions from '{REGIONS_URL}';\nglobalThis.__regionsKeys = Object.keys(regions).sort();"
    ))
}

#[test]
fn escaped_dynamic_import_forms_load_regions_with_the_static_exports() {
    let (expected, _) = load_regions(static_regions_source());
    for (name, body) in ESCAPED_IMPORTS {
        let (actual, _) = load_regions(source(body));
        assert_eq!(actual, expected, "escaped import form {name}");
    }
}

#[test]
fn one_on_demand_import_cost_is_measured() {
    let dynamic = ESCAPED_IMPORTS
        .iter()
        .find(|(name, _)| *name == "indirect-eval-comma")
        .expect("indirect eval case");
    // Warm the process graph before measuring the two load paths.
    let _ = load_regions(static_regions_source());
    let (static_keys, static_time) = load_regions(static_regions_source());
    let (dynamic_keys, dynamic_time) = load_regions(source(dynamic.1));
    assert_eq!(dynamic_keys, static_keys);
    let incremental_us = dynamic_time.as_micros() as i128 - static_time.as_micros() as i128;
    eprintln!(
        "regions on-demand import: static_start={}us dynamic_start={}us incremental={}us source_bytes={}",
        static_time.as_micros(),
        dynamic_time.as_micros(),
        incremental_us,
        include_str!("../src/shim/regions.js").len(),
    );
}
#[test]
fn missing_shim_import_error_names_module_path() {
    const MISSING_PATH: &str = "/rs2b0t/bot/api/map/__cf029_missing_module.js";
    let isolate = LoadIsolate::spawn(
        format!(
            "import '{MISSING_PATH}';\nexport default class MissingShimModule extends LoopingBot {{}}"
        ),
        LoadShape::CompatClass,
        Vec::new(),
    )
    .expect("spawn missing-module fixture");

    let _ = isolate.probe("true");
    let error = match isolate.poll_ready() {
        Ready::Failed(error) => error,
        other => panic!("missing shim import unexpectedly loaded: {other:?}"),
    };
    assert!(
        error.contains("__cf029_missing_module.js"),
        "missing shim import error must name {MISSING_PATH}: {error}"
    );
    isolate.join();
}
