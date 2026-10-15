//! Retained per-source load/transpile/import/runtime-load failures.
//!
//! Display name is never identity. One success must not clear another
//! card's error. A failed replacement keeps the working registration.

use std::path::{Path, PathBuf};
use std::sync::mpsc::TryRecvError;
use std::time::{Duration, Instant};

use script::load::{CardPreparationOutcome, JsLibrary, LoadStage, PreparedCard};
use script::{ApiFamily, LoadIsolate, ScriptSource};

const GOOD: &str = "export default class T extends LoopingBot { override loop() {} }\n";
const GOOD_V2: &str = "export const apiVersion = 2;\nexport function tick(api) { api._n = 1; }\n";
const BAD_PARSE: &str =
    "export default class T extends LoopingBot { override loop() { const x = \"unterminated } }\n";
const BAD_IMPORT: &str = concat!(
    "import x from '../../event/webwalk/Something.js';\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);
const BAD_RUNTIME: &str = concat!(
    "throw new Error('runtime boom');\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);
const V1_ON_NATIVE: &str =
    "export const apiVersion = 1;\nexport function tick(api) { api._n = 1; }\n";
const V2_ON_COMPAT: &str = concat!(
    "export const apiVersion = 2;\n",
    "export default class T extends LoopingBot { override loop() {} }\n"
);

static VALIDATION_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
static COOPERATIVE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "274bot-load-diagnostics-{}-{name}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn library(dir: &Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

/// `rel` uses `/`; the path is joined part by part so it is spelled with the
/// platform separator, as the library stores and prints it.
fn write(dir: &Path, rel: &str, src: &str) -> PathBuf {
    let path = rel
        .split('/')
        .fold(dir.to_path_buf(), |path, part| path.join(part));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, src).unwrap();
    path
}

fn prepare_snapshot(js: &JsLibrary, source: ScriptSource, name: &str) -> CardPreparationOutcome {
    js.card_preparation_snapshot(source, name, None)
        .unwrap_or_else(|error| panic!("{name}: snapshot failed: {error}"))
        .prepare()
        .unwrap_or_else(|error| panic!("{name}: prepare failed: {error}"))
}

#[test]
fn cooperative_card_preparation_matches_the_changed_real_source() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("cooperative-card");
    let mut js = library(&dir);
    let path = write(&dir, "changed.ts", GOOD);
    let previous = js
        .load(&path)
        .expect("load the original card")
        .sha256
        .clone();
    std::fs::write(&path, GOOD_V2).unwrap();
    let snapshot = std::sync::Arc::new(
        js.card_preparation_snapshot(ScriptSource::File, &path.to_string_lossy(), None)
            .expect("capture a changed card"),
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let candidate = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Card,
        api::identity::Scope::Process,
        input,
        std::sync::Arc::clone(&snapshot).cooperative_prepare(),
    )
    .expect("admit the real card")
    .join()
    .expect("worker completes")
    .expect("bounded preparation completes");
    let reference = snapshot.prepare().expect("current card preparation");
    let (CardPreparationOutcome::Prepared(candidate), CardPreparationOutcome::Prepared(reference)) =
        (candidate, reference)
    else {
        panic!("both preparations must produce the changed card")
    };
    assert_ne!(candidate.card.sha256, previous);
    assert_eq!(candidate.card, reference.card);
    assert_eq!(candidate.siblings, reference.siblings);
    assert_eq!(candidate.fingerprint, reference.fingerprint);
}

#[test]
fn cooperative_force_prepare_warms_an_unchanged_card() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("cooperative-force-card");
    let path = write(&dir, "unchanged.ts", GOOD);
    let mut js = library(&dir);
    js.load(&path).expect("load the original card");
    let snapshot = js
        .card_preparation_snapshot(ScriptSource::File, &path.to_string_lossy(), None)
        .expect("capture an unchanged card");
    let bounds = (
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    );
    let snapshot = std::sync::Arc::new(snapshot.force_prepare());
    assert_eq!(
        bounds,
        (
            snapshot.input_bytes(),
            snapshot.scratch_bytes(),
            snapshot.result_bytes(),
        ),
        "forcing preparation must not change its bounded charge"
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let outcome = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Card,
        api::identity::Scope::Process,
        input,
        snapshot.cooperative_prepare(),
    )
    .expect("admit the forced card preparation")
    .join()
    .expect("worker completes")
    .expect("forced preparation completes");
    let CardPreparationOutcome::Prepared(prepared) = outcome else {
        panic!("a forced preparation must warm the unchanged card");
    };
    assert!(!prepared.card.js.is_empty());
}

fn fake_catalog(root: &Path, bots: &[(&str, &str)]) {
    let scripts = root.join("src/bot/scripts");
    std::fs::create_dir_all(&scripts).unwrap();
    let mut index =
        String::from("import { ScriptRegistry } from '../runtime/ScriptRegistry.js';\n");
    for (name, src) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!("import {folder} from './{folder}/{folder}.js';\n"));
        let d = scripts.join(&folder);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join(format!("{folder}.ts")), src).unwrap();
    }
    for (name, _) in bots {
        let folder = name.replace(' ', "");
        index.push_str(&format!(
            "ScriptRegistry.register({{ name: '{name}', description: '{name}', category: 'Test', create: () => new {folder}() }});\n"
        ));
    }
    std::fs::write(scripts.join("index.ts"), index).unwrap();
}

/// Drive the same cancellable validation transaction that product reload
/// uses, rather than bypassing its bounded admission with a synchronous card.
fn validate_cards(cards: Vec<PreparedCard>) -> Vec<(PreparedCard, Result<(), String>)> {
    let _guard = VALIDATION_TEST_LOCK
        .lock()
        .expect("validation test lock poisoned");
    let batch = LoadIsolate::validate_prepared_batch(cards).unwrap();
    let until = Instant::now() + Duration::from_secs(45);
    loop {
        match batch.try_recv() {
            Ok(rows) => return rows,
            Err(TryRecvError::Empty) => {
                assert!(Instant::now() < until, "validation batch did not settle");
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(TryRecvError::Disconnected) => panic!("validation coordinator disconnected"),
        }
    }
}

#[test]
fn mixed_file_batch_keeps_both_failures_after_success() {
    let dir = scratch("mixed-file");
    let mut js = library(&dir);
    let a = write(&dir, "a/bot.ts", BAD_PARSE);
    let b = write(&dir, "b/bot.ts", BAD_PARSE);
    let c = write(&dir, "ok.ts", GOOD);

    assert!(js.load(&a).is_err());
    assert!(js.load(&b).is_err());
    assert!(js.load(&c).is_ok());

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures.iter().any(|f| f.path == a && f.name == "bot"));
    assert!(failures.iter().any(|f| f.path == b && f.name == "bot"));
    assert!(!failures.iter().any(|f| f.name == "ok"));
    assert!(failures
        .iter()
        .all(|f| f.stage == LoadStage::ParseTranspile));
    assert!(failures
        .iter()
        .all(|f| f.line.is_some() || f.diagnostic.contains("ts parse")));
    let named = js.named_failure_output();
    assert!(named.contains("bot") && named.contains(&a.display().to_string()));
    assert!(named.contains(&b.display().to_string()));
}

#[test]
fn duplicate_display_names_are_keyed_by_path() {
    let dir = scratch("dup-name");
    let mut js = library(&dir);
    let one = write(&dir, "one/bot.ts", BAD_PARSE);
    let two = write(&dir, "two/bot.ts", BAD_IMPORT);

    assert!(js.load(&one).is_err());
    let imported = js.load(&two).expect("unloadable file still registers");
    assert_eq!(
        imported.unloadable.as_deref(),
        Some("../../event/webwalk/Something.js")
    );

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert_ne!(failures[0].identity_key, failures[1].identity_key);
    assert!(failures
        .iter()
        .any(|f| { f.path == one && f.stage == LoadStage::ParseTranspile && f.name == "bot" }));
    assert!(failures.iter().any(|f| {
        f.path == two
            && f.stage == LoadStage::ImportResolution
            && f.sibling.as_deref() == Some("../../event/webwalk/Something.js")
    }));
}

#[test]
fn corrected_retry_clears_only_the_affected_identity() {
    let dir = scratch("retry");
    let mut js = library(&dir);
    let keep = write(&dir, "keep.ts", BAD_PARSE);
    let fix = write(&dir, "fix.ts", BAD_PARSE);
    assert!(js.load(&keep).is_err());
    assert!(js.load(&fix).is_err());
    assert_eq!(js.load_failures().len(), 2);

    std::fs::write(&fix, GOOD).unwrap();
    assert!(js.load(&fix).is_ok());

    let failures = js.load_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].path, keep);
    assert_eq!(failures[0].name, "keep");
}

#[test]
fn failed_replace_preserves_working_card_and_records_attempt() {
    let dir = scratch("replace");
    let mut js = library(&dir);
    let path = write(&dir, "live.ts", GOOD);
    let card = js.load(&path).unwrap();
    let old_js = card.js.clone();
    let old_fp = js
        .stored_fingerprint(&card.identity_key())
        .unwrap()
        .to_string();
    assert!(js.load_failures().is_empty());

    std::fs::write(&path, BAD_RUNTIME).unwrap();
    let prepared = match prepare_snapshot(&js, ScriptSource::File, &path.to_string_lossy()) {
        CardPreparationOutcome::Prepared(prepared) => prepared,
        CardPreparationOutcome::Failed { error, .. } => panic!("prepare failed: {error}"),
        CardPreparationOutcome::Unchanged | CardPreparationOutcome::Confirmed => {
            panic!("changed candidate was not prepared")
        }
    };
    let (prepared, result) = validate_cards(vec![prepared])
        .pop()
        .expect("one validation result");
    let err = result.unwrap_err();
    js.record_prepared_failure(&prepared, &err);
    assert!(err.contains("runtime boom"), "{err}");

    let live = js.get(ScriptSource::File, &path.to_string_lossy()).unwrap();
    assert_eq!(live.js, old_js);
    assert_eq!(live.origin, GOOD);

    let failures = js.load_failures();
    assert_eq!(failures.len(), 1);
    let f = &failures[0];
    assert_eq!(f.identity_key, card.identity_key());
    assert_eq!(f.stage, LoadStage::RuntimeLoad);
    assert_ne!(f.fingerprint, old_fp);
    assert!(!f.fingerprint.is_empty());
}

#[test]
fn catalog_mixed_batch_retains_named_failures() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("catalog-mixed");
    let root = dir.join("catalog");
    fake_catalog(
        &root,
        &[("GoodBot", GOOD), ("BadParse", GOOD), ("BadImport", GOOD)],
    );
    let mut js = library(&dir);
    js.register_rs2b0t(&root, &dir.join("rs2b0t-path")).unwrap();
    js.ensure_js(ScriptSource::Catalog, "GoodBot").unwrap();
    js.ensure_js(ScriptSource::Catalog, "BadParse").unwrap();
    js.ensure_js(ScriptSource::Catalog, "BadImport").unwrap();
    let good_js = js.get(ScriptSource::Catalog, "GoodBot").unwrap().js.clone();
    let bad_parse_js = js
        .get(ScriptSource::Catalog, "BadParse")
        .unwrap()
        .js
        .clone();
    let bad_import_origin = js
        .get(ScriptSource::Catalog, "BadImport")
        .unwrap()
        .origin
        .clone();

    std::fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    std::fs::write(root.join("src/bot/scripts/BadParse/BadParse.ts"), BAD_PARSE).unwrap();
    std::fs::write(
        root.join("src/bot/scripts/BadImport/BadImport.ts"),
        BAD_IMPORT,
    )
    .unwrap();

    let snapshot = std::sync::Arc::new(
        js.catalog_preparation_snapshot(&root)
            .expect("capture the catalogue metadata"),
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let result = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        std::sync::Arc::clone(&snapshot).cooperative_prepare(),
    )
    .expect("admit catalogue preparation")
    .join()
    .expect("worker completes")
    .expect("bounded catalogue preparation completes");
    assert!(result.diff.changed.iter().any(|name| name == "GoodBot"));
    for failure in result.prepare_failed {
        js.record_failure(failure);
    }
    let mut prepared_ok = Vec::new();
    for candidate in result.prepared.into_iter().chain(result.warmed) {
        let (candidate, result) = validate_cards(vec![candidate])
            .pop()
            .expect("one validation result");
        match result {
            Ok(()) => prepared_ok.push(candidate),
            Err(error) => js.record_prepared_failure(&candidate, &error),
        }
    }
    assert_eq!(prepared_ok.len(), 1);
    assert_eq!(prepared_ok[0].card.name, "GoodBot");
    for item in prepared_ok {
        js.commit_prepared(item).unwrap();
    }
    let report = js.apply_catalog_diff(result.diff);
    let good = js.get(ScriptSource::Catalog, "GoodBot").unwrap();
    assert_ne!(good.js, good_js);
    let bad_parse = js.get(ScriptSource::Catalog, "BadParse").unwrap();
    assert_eq!(bad_parse.js, bad_parse_js);
    let bad_import = js.get(ScriptSource::Catalog, "BadImport").unwrap();
    assert_eq!(bad_import.origin, bad_import_origin);
    assert!(
        js.load_failures().len() >= 2,
        "both catalog failures stay: {:?}",
        js.load_failures()
    );
    assert!(js
        .load_failures()
        .iter()
        .any(|f| f.name == "BadParse" && f.stage == LoadStage::ParseTranspile));
    assert!(js.load_failures().iter().any(|f| f.name == "BadImport"
        && (f.stage == LoadStage::ImportResolution || f.diagnostic.contains("unloadable"))));
    assert!(!js.load_failures().iter().any(|f| f.name == "GoodBot"));
    let named = report.named_failure_output();
    assert!(
        named.contains("BadParse") || js.named_failure_output().contains("BadParse"),
        "{named} / {}",
        js.named_failure_output()
    );
}

#[test]
fn cooperative_catalogue_matches_the_changed_real_source() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("cooperative-catalogue");
    let root = dir.join("catalog");
    fake_catalog(&root, &[("GoodBot", GOOD)]);
    let mut js = library(&dir);
    js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("load current registry");
    std::fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    let snapshot = std::sync::Arc::new(
        js.catalog_preparation_snapshot(&root)
            .expect("capture the catalogue metadata"),
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let diff = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        std::sync::Arc::clone(&snapshot).cooperative_diff(),
    )
    .expect("admit the real catalogue")
    .join()
    .expect("worker completes")
    .expect("bounded catalogue diff completes");
    assert!(diff.added.is_empty());
    assert_eq!(diff.changed, ["GoodBot"]);
    assert!(diff.removed.is_empty());
    assert!(diff.failed.is_empty());

    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let prepared = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        snapshot.cooperative_prepare(),
    )
    .expect("admit catalogue preparation")
    .join()
    .expect("worker completes")
    .expect("bounded catalogue preparation completes");
    assert_eq!(prepared.diff.changed, diff.changed);
    assert!(
        prepared.prepare_failed.is_empty(),
        "{:?}",
        prepared.prepare_failed
    );
    assert_eq!(prepared.prepared.len(), 1);
    assert_eq!(prepared.prepared[0].card.name, "GoodBot");
}

#[test]
fn small_catalogue_refresh_reserves_a_measured_charge() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("small-catalog-charge");
    let root = dir.join("catalog");
    fake_catalog(&root, &[("GoodBot", GOOD)]);
    let mut js = library(&dir);
    js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("load current registry");
    std::fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        "export default class T extends LoopingBot { override loop() { return; } }\n",
    )
    .unwrap();
    let snapshot = js
        .catalog_preparation_snapshot(&root)
        .expect("capture the catalogue metadata");
    let charge = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap()
    .charge();
    let total = charge.total_bytes().expect("checked catalogue charge");
    let accounted = charge
        .input_bytes()
        .checked_add(charge.scratch_bytes())
        .and_then(|bytes| bytes.checked_add(charge.result_bytes()))
        .expect("catalogue charge sum is representable");
    assert_eq!(total, accounted);
    assert!(
        total < 64 * 1024 * 1024,
        "small catalogue charged {total} bytes"
    );
}

#[test]
fn catalogue_growth_past_its_measured_input_charge_is_refused() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("catalog-input-over-budget");
    let root = dir.join("catalog");
    fake_catalog(&root, &[("GoodBot", GOOD), ("OtherBot", GOOD)]);
    let mut js = library(&dir);
    js.register_rs2b0t(&root, &dir.join("rs2b0t-path"))
        .expect("load current registry");
    let snapshot = std::sync::Arc::new(
        js.catalog_preparation_snapshot(&root)
            .expect("capture the bounded catalogue"),
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let oversized = format!("{}\n/*{}*/\n", GOOD, "x".repeat(5 * 1024 * 1024));
    std::fs::write(
        root.join("src/bot/scripts/GoodBot/GoodBot.ts"),
        oversized.as_bytes(),
    )
    .unwrap();
    std::fs::write(
        root.join("src/bot/scripts/OtherBot/OtherBot.ts"),
        oversized.as_bytes(),
    )
    .unwrap();
    let error = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        snapshot.cooperative_prepare(),
    )
    .expect("admit the bounded catalogue demand")
    .join()
    .expect("worker completes")
    .expect_err("source growth beyond the captured charge must be refused");
    assert!(
        error.contains("charged input bound"),
        "unexpected over-budget refusal: {error}"
    );
}

#[test]
fn v1_and_v2_family_errors_keep_stable_identities() {
    let dir = scratch("family");
    let mut js = library(&dir);
    let v1 = write(&dir, "native-v1.ts", V1_ON_NATIVE);
    let v2 = write(&dir, "compat-v2.ts", V2_ON_COMPAT);
    let ok = write(&dir, "native-v2.ts", GOOD_V2);

    assert!(js.load(&v1).is_err());
    assert!(js.load(&v2).is_err());
    let good = js.load(&ok).unwrap();
    assert_eq!(good.api_family, ApiFamily::V2);

    let failures = js.load_failures();
    assert_eq!(failures.len(), 2, "{failures:?}");
    assert!(failures
        .iter()
        .any(|f| { f.identity_key.contains("native-v1") && f.diagnostic.contains("api-version") }));
    assert!(failures
        .iter()
        .any(|f| { f.identity_key.contains("compat-v2") && f.diagnostic.contains("api-version") }));
    assert!(failures.iter().all(|f| !f.identity_key.is_empty()));
    assert!(!failures
        .iter()
        .any(|f| f.identity_key == good.identity_key()));
}

#[test]
#[ignore = "release-only timing probe"]
fn release_probe_bounded_card_transpile_8mib() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("release-card-probe");
    let path = write(&dir, "probe.ts", GOOD);
    let mut js = library(&dir);
    js.load(&path).expect("load probe baseline");
    let snapshot = js
        .card_preparation_snapshot(ScriptSource::File, &path.to_string_lossy(), None)
        .expect("capture probe card");
    let source_limit = snapshot.source_limit().expect("bounded source limit");
    // Whitespace exercises the full source-read/parse limit without making the
    // emitted JavaScript alone exceed the separate 16 MiB result budget.
    let filler_bytes = source_limit
        .checked_sub(GOOD.len())
        .expect("source limit fits card");
    let source = format!("{GOOD}{}", " ".repeat(filler_bytes));
    assert_eq!(source.len(), source_limit);
    let source_bytes = source.len();
    std::fs::write(&path, source).unwrap();

    let snapshot = std::sync::Arc::new(snapshot);
    let input_bytes = snapshot.input_bytes();
    let scratch_bytes = snapshot.scratch_bytes();
    let result_bytes = snapshot.result_bytes();
    let input =
        script::preparation::PreparationInput::new(1, input_bytes, scratch_bytes, result_bytes)
            .unwrap();
    let timing = std::sync::Arc::new(std::sync::Mutex::new((0_usize, Duration::ZERO)));
    let worker_timing = std::sync::Arc::clone(&timing);
    let mut work = std::sync::Arc::clone(&snapshot).cooperative_prepare();
    let started = Instant::now();
    let outcome = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Card,
        api::identity::Scope::Process,
        input,
        move |context| {
            let action_started = Instant::now();
            let step = work(context);
            let elapsed = action_started.elapsed();
            let mut timing = worker_timing.lock().expect("probe timing lock");
            timing.0 += 1;
            timing.1 = timing.1.max(elapsed);
            step
        },
    )
    .expect("admit near-limit card")
    .join()
    .expect("worker completes")
    .expect("near-limit card transpiles");
    let wall_time = started.elapsed();
    let (actions, max_action) = *timing.lock().expect("probe timing lock");
    let CardPreparationOutcome::Prepared(prepared) = outcome else {
        match outcome {
            CardPreparationOutcome::Failed { error, .. } => {
                panic!("near-limit card failed to prepare: {error}")
            }
            other => panic!("near-limit card was not prepared: {other:?}"),
        }
    };
    assert!(!prepared.card.js.is_empty());
    eprintln!(
        "scenario=bounded_card_transpile source_bytes={source_bytes} input_charge={} scratch_charge={} result_charge={} actions={actions} max_action_us={} wall_us={}",
        input_bytes,
        scratch_bytes,
        result_bytes,
        max_action.as_micros(),
        wall_time.as_micros(),
    );
}

#[test]
#[ignore = "release-only timing probe"]
fn release_probe_catalogue_cooperative_action_max() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    let dir = scratch("release-catalogue-probe");
    let root = dir.join("catalog");
    let names = [
        "Probe0", "Probe1", "Probe2", "Probe3", "Probe4", "Probe5", "Probe6", "Probe7",
    ];
    let bots: Vec<(&str, &str)> = names.iter().copied().map(|name| (name, GOOD)).collect();
    fake_catalog(&root, &bots);
    let js = library(&dir);
    let snapshot = std::sync::Arc::new(
        js.catalog_preparation_snapshot(&root)
            .expect("capture probe catalogue"),
    );
    let input_bytes = snapshot.input_bytes();
    let scratch_bytes = snapshot.scratch_bytes();
    let result_bytes = snapshot.result_bytes();
    let input =
        script::preparation::PreparationInput::new(1, input_bytes, scratch_bytes, result_bytes)
            .unwrap();
    let timing = std::sync::Arc::new(std::sync::Mutex::new((0_usize, Duration::ZERO)));
    let worker_timing = std::sync::Arc::clone(&timing);
    let mut work = std::sync::Arc::clone(&snapshot).cooperative_prepare();
    let started = Instant::now();
    let result = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        move |context| {
            let action_started = Instant::now();
            let step = work(context);
            let elapsed = action_started.elapsed();
            let mut timing = worker_timing.lock().expect("probe timing lock");
            timing.0 += 1;
            timing.1 = timing.1.max(elapsed);
            step
        },
    )
    .expect("admit probe catalogue")
    .join()
    .expect("worker completes")
    .expect("catalogue preparation completes");
    let wall_time = started.elapsed();
    let (actions, max_action) = *timing.lock().expect("probe timing lock");
    assert_eq!(result.warmed.len(), bots.len());
    assert!(result.prepare_failed.is_empty());
    eprintln!(
        "scenario=catalogue_initial_fill cards={} input_charge={} scratch_charge={} result_charge={} actions={actions} max_action_us={} wall_us={}",
        bots.len(),
        input_bytes,
        scratch_bytes,
        result_bytes,
        max_action.as_micros(),
        wall_time.as_micros(),
    );
}

#[test]
#[ignore = "release-only declared-maxima catalogue probe"]
fn release_probe_catalogue_at_4096_registrations_and_8192_files() {
    let _guard = COOPERATIVE_TEST_LOCK.lock().unwrap();
    const WITH_SIBLING: &str = concat!(
        "import { helper } from './helper.js';\n",
        "export default class T extends LoopingBot { override loop() { void helper; } }\n",
    );
    let dir = scratch("catalogue-declared-maxima");
    let root = dir.join("catalog");
    let names: Vec<_> = (0..4096).map(|index| format!("Probe{index}")).collect();
    let bots: Vec<_> = names
        .iter()
        .map(|name| (name.as_str(), WITH_SIBLING))
        .collect();
    fake_catalog(&root, &bots);
    for name in &names {
        let sibling = root.join("src/bot/scripts").join(name).join("helper.ts");
        std::fs::write(sibling, "export const helper = 1;\n").unwrap();
    }
    let js = library(&dir);
    let snapshot = std::sync::Arc::new(
        js.catalog_preparation_snapshot(&root)
            .expect("capture declared-maxima catalogue"),
    );
    let input = script::preparation::PreparationInput::new(
        1,
        snapshot.input_bytes(),
        snapshot.scratch_bytes(),
        snapshot.result_bytes(),
    )
    .unwrap();
    let service = script::work_admission::WorkService::shared().unwrap();
    let drops_before = service.lock().dropped_progress_reports();
    let actions = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = std::sync::Arc::clone(&actions);
    let mut work = snapshot.cooperative_prepare();
    let started = Instant::now();
    let task = script::preparation::submit_cooperative(
        script::preparation::PreparationKind::Catalogue,
        api::identity::Scope::Process,
        input,
        move |context| {
            observed.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            work(context)
        },
    )
    .expect("admit declared-maxima catalogue");
    let identity = task.identity();
    let prepared = task
        .join()
        .expect("worker completes declared-maxima catalogue")
        .expect("bounded catalogue completes");
    assert_eq!(prepared.warmed.len(), 4096, "{:?}", prepared.prepare_failed);
    assert!(prepared.prepare_failed.is_empty());
    let service = service.lock();
    assert_eq!(service.dropped_progress_reports(), drops_before);
    let mut sequence = 0;
    service.publications().progress().read(|_, snapshot| {
        let latest = snapshot.as_ref().expect("sealed catalogue progress");
        assert_eq!(latest.identity(), identity);
        sequence = latest.sequence();
    });
    eprintln!(
        "scenario=catalogue-declared-maxima registrations=4096 source_files=8192 actions={} progress_snapshots={sequence} input_charge={} scratch_charge={} result_charge={} wall_ms={}",
        actions.load(std::sync::atomic::Ordering::Relaxed),
        input.charge().input_bytes(),
        input.charge().scratch_bytes(),
        input.charge().result_bytes(),
        started.elapsed().as_millis(),
    );
    assert!(sequence <= 1027);
    let _ = std::fs::remove_dir_all(dir);
}
