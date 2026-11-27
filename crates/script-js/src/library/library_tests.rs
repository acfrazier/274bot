//! Persistence behavior of the JS library's catalogue-reference port.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::*;
use api::local_state::{CatalogueReferenceStore, FileCardReference};
use host_play::local_state::{LocalStateService, StateRoot};
use crate::library::rs2b0t_registry::absolute_clean;

const BOT: &str = "export default class Main extends LoopingBot {\n    override loop() {}\n}\n";

fn scratch(name: &str) -> PathBuf {
    script::isolated_env::IsolatedEnv::ensure_thread();
    let dir = std::env::temp_dir().join(format!("274bot-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

struct TestState {
    _service: Arc<LocalStateService>,
    references: Arc<dyn CatalogueReferenceStore>,
}

impl TestState {
    fn new(dir: &Path) -> Self {
        let service = LocalStateService::open_process(StateRoot::new(dir.join("local-state")));
        let references = service.catalogue_references();
        Self {
            _service: service,
            references,
        }
    }

    fn references(&self) -> Arc<dyn CatalogueReferenceStore> {
        Arc::clone(&self.references)
    }

    fn library(&self, dir: &Path) -> JsLibrary {
        JsLibrary::with_cache(self.references(), dir.join("js-cache"))
    }
}

fn library(dir: &Path) -> JsLibrary {
    TestState::new(dir).library(dir)
}

fn bot_script(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, BOT).unwrap();
    path
}

#[test]
fn a_loaded_script_is_stored_as_an_absolute_clean_path_and_comes_back() {
    let dir = scratch("clean-path");
    let script = bot_script(&dir, "bot.ts");
    std::fs::create_dir(dir.join("sub")).unwrap();
    let roundabout = dir.join("sub").join("..").join("bot.ts");

    let state = TestState::new(&dir);
    let mut first = state.library(&dir);
    let card = first.load(&roundabout).unwrap();
    assert_eq!(card.path, script, "`..` is folded before the path is stored");

    let stored = state.references().file_cards().unwrap().value;
    assert_eq!(stored.len(), 1);
    assert_eq!(stored[0].name, "bot");
    assert_eq!(
        stored[0].path,
        card.path.to_string_lossy().as_ref(),
        "the port stores the absolute, cleaned source path"
    );

    let mut second = state.library(&dir);
    second.restore().unwrap();
    assert_eq!(second.cards().len(), 1);
    assert_eq!(second.cards()[0].path, script);
}

#[test]
fn restore_rejects_invalid_paths_before_open_and_preserves_unresolved_refs() {
    let dir = scratch("unresolved-refs");
    let outside = bot_script(&dir, "outside.ts");
    std::fs::create_dir(dir.join("sub")).unwrap();
    let missing = dir.join("missing.ts");

    let state = TestState::new(&dir);
    let references = state.references();
    let current = references.file_cards().unwrap();
    let rows: Arc<[FileCardReference]> = vec![
        FileCardReference {
            name: "escape".into(),
            path: dir
                .join("sub")
                .join("..")
                .join("outside.ts")
                .to_string_lossy()
                .into_owned(),
        },
        FileCardReference {
            name: "relative".into(),
            path: "outside.ts".into(),
        },
        FileCardReference {
            name: "missing".into(),
            path: missing.to_string_lossy().into_owned(),
        },
    ]
    .into();
    references
        .save_file_cards(current.version, rows)
        .expect("seed refs through SQL port");

    let mut library = state.library(&dir);
    library.restore().unwrap();
    assert!(library.cards().is_empty());
    let failures = library.load_failures();
    assert_eq!(failures.len(), 2, "missing files remain silently omitted");
    assert!(failures.iter().any(|failure| {
        failure.name == "escape"
            && failure.diagnostic.contains("not restored")
            && failure.diagnostic.contains("..")
    }));
    assert!(failures.iter().any(|failure| {
        failure.name == "relative" && failure.diagnostic.contains("not an absolute path")
    }));

    let added = bot_script(&dir, "added.ts");
    library.load(&added).expect("load a new file card");
    let stored = references.file_cards().unwrap().value;
    assert_eq!(stored.len(), 4, "unresolved refs survive later file saves");
    assert!(stored.iter().any(|row| row.name == "escape"));
    assert!(stored.iter().any(|row| row.name == "relative"));
    assert!(stored.iter().any(|row| row.name == "missing"));
    assert!(stored
        .iter()
        .any(|row| row.name == "added" && row.path == added.to_string_lossy().as_ref()));
    assert!(outside.is_file());
}

#[test]
fn a_stale_file_reference_save_does_not_change_in_memory_cards() {
    let dir = scratch("stale-file-save");
    let state = TestState::new(&dir);
    let references = state.references();
    let mut library = state.library(&dir);
    let first = bot_script(&dir, "first.ts");
    library.load(&first).expect("first file card saves");

    let current = references.file_cards().unwrap();
    let mut concurrent = current.value.to_vec();
    concurrent.push(FileCardReference {
        name: "other-writer".into(),
        path: dir.join("other-writer.ts").to_string_lossy().into_owned(),
    });
    references
        .save_file_cards(current.version, concurrent.into())
        .expect("simulate a concurrent SQL writer");

    let second = bot_script(&dir, "second.ts");
    let error = library
        .load(&second)
        .expect_err("the stale component version must be refused");
    assert!(!error.is_empty(), "the store refusal detail is preserved");
    assert_eq!(library.cards().len(), 1, "failed persistence commits no card");
    assert_eq!(library.cards()[0].path, first);

    let stored = references.file_cards().unwrap().value;
    assert!(stored.iter().any(|row| row.name == "other-writer"));
    assert!(!stored.iter().any(|row| row.name == "second"));
}

#[test]
fn a_relative_path_is_made_absolute_against_the_current_directory() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(
        absolute_clean(Path::new("rel/./x/../y.ts")).unwrap(),
        cwd.join("rel").join("y.ts")
    );
    let absolute = std::env::temp_dir().join("a").join("b.ts");
    assert_eq!(absolute_clean(&absolute).unwrap(), absolute);
}

#[test]
fn persisting_a_relative_catalog_root_stores_it_absolute() {
    let dir = scratch("root-absolute");
    let state = TestState::new(&dir);
    let library = state.library(&dir);
    library
        .persist_catalog_root(Path::new("some/../catalog"))
        .unwrap();

    let stored = state.references().saved_root().unwrap().value;
    let expected = absolute_clean(Path::new("some/../catalog")).unwrap();
    assert_eq!(stored, Some(expected));
}

fn failed_surface_start() -> script::StartLoadError {
    use script::surface::test_support::{StubSurfaceCard, StubSurfaceFactory};
    use std::sync::Arc;

    let factory = Arc::new(StubSurfaceFactory::new());
    factory.fail_next_spawn("failed to spawn isolate thread: injected spawn failure");
    let mut slot = script::SlotScript::new();
    slot.bind_surface_factory(factory);
    slot.start_surface(script::surface::SurfaceStart {
        card: Arc::new(StubSurfaceCard),
        settings_bag: None,
        game_data: None,
        named_banks: Arc::default(),
        loadouts: Arc::default(),
        wilderness_rules: script::surface::no_wilderness(),
    })
    .expect_err("injected spawn failure must err")
}

#[test]
fn runtime_load_start_failure_records_a_runtime_load_stage_and_refusals_do_not() {
    let dir = scratch("start-result-stage");
    let mut lib = library(&dir);
    let path = bot_script(&dir, "bot.ts");
    let card = lib.load(&path).expect("bot loads");
    // Drive the real Start producer with a deterministic spawn failure,
    // then assert it retains the typed surface spawn cause (not Refused).
    let producer_err = failed_surface_start();
    let diagnostic = match &producer_err {
        script::StartLoadError::Surface(script::surface::SurfaceError::Spawn(diagnostic)) => {
            diagnostic.clone()
        }
        other => panic!("producer must retain the surface spawn cause, got {other:?}"),
    };
    assert!(
        diagnostic.contains("isolate thread"),
        "spawn diagnostic reaches the caller: {diagnostic}"
    );
    // A runtime setup failure is recorded against the attempted card with
    // the RuntimeLoad stage, and the diagnostic reaches the caller.
    let err = lib
        .record_start_result(&card, Err(producer_err))
        .expect_err("runtime failure propagates");
    assert!(err.contains("isolate thread"), "{err}");
    let failures = lib.load_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].stage, LoadStage::RuntimeLoad);
    assert_eq!(failures[0].identity_key, card.identity_key());
    // A refusal carries its diagnostic back without touching the record.
    let err = lib
        .record_start_result(&card, Err(script::StartLoadError::Refused("busy".into())))
        .expect_err("refusal propagates");
    assert_eq!(err, "busy");
    assert_eq!(
        lib.load_failures().len(),
        1,
        "refusals leave diagnostics untouched"
    );
    for error in [
        script::surface::SurfaceError::NoFactory(script::surface::SurfaceKind::Js),
        script::surface::SurfaceError::ForeignCard(script::surface::SurfaceKind::Js),
    ] {
        let diagnostic = error.to_string();
        assert_eq!(
            lib.record_start_result(&card, Err(script::StartLoadError::Surface(error))),
            Err(diagnostic)
        );
        assert_eq!(lib.load_failures().len(), 1, "refusals retain diagnostics");
    }
    // Success clears the recorded failure.
    lib.record_start_result(&card, Ok(()))
        .expect("success clears");
    assert!(lib.load_failures().is_empty());
}

#[test]
fn start_load_reports_runtime_spawn_failure_not_refusal() {
    let err = failed_surface_start();
    match err {
        script::StartLoadError::Surface(script::surface::SurfaceError::Spawn(diagnostic)) => {
            assert!(diagnostic.contains("isolate thread"), "{diagnostic}")
        }
        other => panic!("spawn failure must retain the surface cause, got {other:?}"),
    }
}

#[test]
fn catalog_missing_compat_api_export_gets_actionable_start_message() {
    let dir = scratch("catalog-missing-export");
    let path = bot_script(&dir, "bot.ts");
    let mut lib = library(&dir);
    let mut card = lib.load(&path).expect("bot loads");
    card.source = script::card::ScriptSource::Catalog;

    let raw = "load: Uncaught SyntaxError: The requested module '../../api/duel/Duel.js' does not provide an export named 'inDuelChallengeArea' at file:///rs2b0t/bot/scripts/bot/DuelArenaLogic.js:1:24";
    let error = lib
        .record_start_result(&card, Err(script::StartLoadError::RuntimeLoad(raw.into())))
        .expect_err("missing compat API export must refuse Start");
    assert_eq!(
        error,
        "This script imports compatibility API member 'inDuelChallengeArea', which this build does not provide."
    );

    let failure = lib
        .load_failure(&card.identity_key())
        .expect("raw runtime diagnostic is retained");
    assert_eq!(failure.diagnostic, raw);
    assert!(lib.named_failure_output().contains(raw));

    let mut file_card = card.clone();
    file_card.source = script::card::ScriptSource::File;
    let file_error = lib
        .record_start_result(
            &file_card,
            Err(script::StartLoadError::RuntimeLoad(raw.into())),
        )
        .expect_err("file-card diagnostics are not a catalog API mismatch");
    assert_eq!(file_error, raw);
}

#[test]
fn catalog_missing_compat_api_default_export_keeps_raw_start_message() {
    let dir = scratch("catalog-missing-default-export");
    let path = bot_script(&dir, "bot.ts");
    let mut lib = library(&dir);
    let mut card = lib.load(&path).expect("bot loads");
    card.source = script::card::ScriptSource::Catalog;

    let raw = "load: Uncaught SyntaxError: The requested module '../../api/duel/Duel.js' does not provide an export named 'default' at file:///rs2b0t/bot/scripts/bot/bot.js:1:1";
    let error = lib
        .record_start_result(&card, Err(script::StartLoadError::RuntimeLoad(raw.into())))
        .expect_err("missing default export is the script's error, not an API version mismatch");
    assert_eq!(error, raw);
}

#[test]
fn catalog_missing_export_from_scripts_sibling_api_keeps_raw_start_message() {
    let dir = scratch("catalog-sibling-api-missing-export");
    let path = bot_script(&dir, "bot.ts");
    let mut lib = library(&dir);
    let mut card = lib.load(&path).expect("bot loads");
    card.source = script::card::ScriptSource::Catalog;

    let raw = "load: Uncaught SyntaxError: The requested module '../api/Helper.js' does not provide an export named 'helper' at file:///rs2b0t/bot/scripts/bot/bot.js:1:1";
    let error = lib
        .record_start_result(&card, Err(script::StartLoadError::RuntimeLoad(raw.into())))
        .expect_err("a scripts-folder sibling is not a compat API module");
    assert_eq!(error, raw);
}

#[test]
fn unrelated_syntax_and_missing_relative_module_errors_stay_raw() {
    let dir = scratch("catalog-unrelated-load-errors");
    let path = bot_script(&dir, "bot.ts");
    let mut lib = library(&dir);
    let mut card = lib.load(&path).expect("bot loads");
    card.source = script::card::ScriptSource::Catalog;

    for raw in [
        "load: Uncaught SyntaxError: Unexpected token '}' at file:///rs2b0t/bot/scripts/bot/bot.js:1:1",
        "load: Uncaught Error: Module not found: '../api/Missing.js' at file:///rs2b0t/bot/scripts/bot/bot.js:1:1",
    ] {
        let error = lib
            .record_start_result(&card, Err(script::StartLoadError::RuntimeLoad(raw.into())))
            .expect_err("unrelated load errors are never rewritten");
        assert_eq!(error, raw);
    }
}
#[test]
fn refresh_and_ensure_js_refuse_unresolvable_setting_spreads_without_mutating_card() {
    let dir = scratch("refresh-unresolvable-settings");
    let path = dir.join("SettingsBot.ts");
    let good = "export const SETTINGS = { loadout: { ...LOADOUT_SETTING, group: 'Food' } }; export default class SettingsBot extends TaskBot {}";
    let bad = "export const SETTINGS = { loadout: { ...UNKNOWN_SETTING, group: 'Food' } }; export default class SettingsBot extends TaskBot {}";
    std::fs::write(&path, good).unwrap();
    let mut library = library(&dir);
    let loaded = library.load(&path).expect("valid schema loads");
    let prior_schema = loaded.settings_schema.clone();
    let prior_hash = loaded.sha256;

    std::fs::write(&path, bad).unwrap();
    let refresh_error = library
        .refresh(script::card::ScriptSource::File, "SettingsBot")
        .expect_err("refresh must reject a new unresolved setting spread");
    assert!(
        refresh_error.contains("SettingsBot")
            && refresh_error.contains("loadout")
            && refresh_error.contains("UNKNOWN_SETTING"),
        "{refresh_error}"
    );
    let retained = library
        .get(script::card::ScriptSource::File, "SettingsBot")
        .expect("the old accepted card remains registered");
    assert_eq!(retained.origin, good);
    assert_eq!(retained.sha256, prior_hash);
    assert_eq!(retained.settings_schema, prior_schema);
    assert!(
        library.named_failure_output().contains("UNKNOWN_SETTING"),
        "refresh refusal is visible in named failures: {}",
        library.named_failure_output()
    );

    library
        .cards
        .iter_mut()
        .find(|card| card.name == "SettingsBot")
        .expect("registered card")
        .js
        .clear();
    let ensure_error = library
        .ensure_js(script::card::ScriptSource::File, "SettingsBot")
        .expect_err("ensure_js must reject the same unresolved source");
    assert!(ensure_error.contains("UNKNOWN_SETTING"), "{ensure_error}");
    let retained = library
        .get(script::card::ScriptSource::File, "SettingsBot")
        .expect("failed ensure_js retains the old accepted card");
    assert_eq!(retained.origin, good);
    assert_eq!(retained.settings_schema, prior_schema);
    drop(library);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn refresh_and_pending_preparation_follow_the_file_identity_not_the_stem() {
    use crate::library::prepare::NativeFixture;
    use script::card::ScriptSource::File;

    let dir = scratch("refresh-by-identity");
    std::fs::create_dir_all(dir.join("one")).unwrap();
    std::fs::create_dir_all(dir.join("two")).unwrap();
    let first = dir.join("one").join("miner.ts");
    let second = dir.join("two").join("miner.ts");
    let source = |marker: &str| {
        NativeFixture::new("Miner")
            .source(&format!("export function tick(api) {{ /* {marker} */ }}"))
    };
    std::fs::write(&first, source("first")).unwrap();
    std::fs::write(&second, source("second")).unwrap();
    let mut library = library(&dir);
    let a = library.load(&first).expect("first loads");
    let b = library.load(&second).expect("second loads");
    assert_eq!(a.name, b.name, "one stem");
    assert_ne!(a.identity_id(), b.identity_id());

    // Refreshing by the file identity updates that card only.
    std::fs::write(&second, source("second, edited")).unwrap();
    library
        .refresh(File, &b.identity_id())
        .expect("refresh by the file identity");
    assert!(library
        .get(File, &b.identity_id())
        .unwrap()
        .origin
        .contains("second, edited"));
    assert!(library
        .get(File, &a.identity_id())
        .unwrap()
        .origin
        .contains("first"));

    // A refused refresh of the first file is recorded against its own
    // identity and leaves the second card and its failure state alone.
    std::fs::write(&first, "export function tick(api) {}").unwrap();
    let error = library
        .refresh(File, &a.identity_id())
        .expect_err("a file without a manifest is refused");
    assert!(error.contains("manifest-required"), "{error}");
    let failures = library.load_failures();
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].identity_key, a.identity_key());
    assert!(library.load_failure(&b.identity_key()).is_none());
    assert!(library
        .get(File, &a.identity_id())
        .unwrap()
        .origin
        .contains("first"));

    // Cards waiting for preparation are listed by identity, so a File card
    // is prepared by its path and two files with one stem are both reached.
    std::fs::remove_dir_all(dir.join("js-cache")).unwrap();
    for card in library.cards.iter_mut() {
        card.js.clear();
    }
    let waiting = library.cards_needing_transpile();
    assert_eq!(waiting, [(File, a.identity_id()), (File, b.identity_id())]);
    for (source, identity) in waiting {
        let result = library.ensure_js(source, &identity);
        if identity == b.identity_id() {
            result.expect("the valid card prepares by identity");
        }
    }
    assert!(!library.get(File, &b.identity_id()).unwrap().js.is_empty());
    drop(library);
    std::fs::remove_dir_all(dir).unwrap();
}
