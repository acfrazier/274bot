//! Persistence behaviour of the JS library's store (`js-scripts.json`) and of the
//! persisted catalog root (`rs2b0t-path`): what restore trusts, what it refuses,
//! and that a refused store is never overwritten.

use std::path::{Path, PathBuf};

use super::*;
use crate::rs2b0t_registry::{absolute_clean, persist_rs2b0t_root_at, rs2b0t_root_checked_at};

const BOT: &str = "export default class Main extends LoopingBot {\n    override loop() {}\n}\n";

fn scratch(name: &str) -> PathBuf {
    // Pin this thread's `~/.274bot` and `$RS2B0T` away from the operator's.
    crate::isolated_env::IsolatedEnv::ensure_thread();
    let dir = std::env::temp_dir().join(format!("274bot-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn library(dir: &Path) -> JsLibrary {
    JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache"))
}

fn bot_script(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, BOT).unwrap();
    path
}

/// A `js-scripts.json` written the way the app writes it (owner-only).
fn write_store(dir: &Path, entries: &[(&str, &str)]) -> PathBuf {
    let rows: Vec<_> = entries
        .iter()
        .map(|(name, path)| serde_json::json!({ "name": name, "path": path }))
        .collect();
    let store = dir.join("js-scripts.json");
    vault::write_private_file(&store, serde_json::to_string(&rows).unwrap().as_bytes()).unwrap();
    store
}

fn card_names(library: &JsLibrary) -> Vec<String> {
    library.cards().iter().map(|c| c.name.clone()).collect()
}

fn failure_names(library: &JsLibrary) -> Vec<String> {
    library
        .load_failures()
        .iter()
        .map(|f| f.name.clone())
        .collect()
}

#[test]
fn a_loaded_script_is_stored_as_an_absolute_clean_path_and_comes_back() {
    let dir = scratch("clean-path");
    let script = bot_script(&dir, "bot.ts");
    std::fs::create_dir(dir.join("sub")).unwrap();
    let roundabout = dir.join("sub").join("..").join("bot.ts");

    let mut first = library(&dir);
    let card = first.load(&roundabout).unwrap();

    assert_eq!(
        card.path, script,
        "`..` is folded before the path is stored"
    );
    let stored = std::fs::read_to_string(dir.join("js-scripts.json")).unwrap();
    assert!(!stored.contains(".."), "{stored}");
    let mut second = library(&dir);
    second.restore().unwrap();
    assert_eq!(second.cards().len(), 1);
    assert_eq!(second.cards()[0].path, script);
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

#[cfg(unix)]
mod unix {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn set_mode(path: &Path, mode: u32) {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    fn mode_of(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn a_store_others_can_write_is_refused_and_nothing_is_saved_over_it() {
        let dir = scratch("untrusted-store");
        let script = bot_script(&dir, "bot.ts");
        let store = write_store(&dir, &[("planted", script.to_str().unwrap())]);
        set_mode(&store, 0o666);
        let before = std::fs::read(&store).unwrap();

        let mut refused = library(&dir);
        let refusal = refused.restore().unwrap_err();
        assert!(refusal.contains("writable by other users"), "{refusal}");
        assert!(refused.cards().is_empty(), "nothing is restored from it");

        // A load while the store is refused is an error, not a silent overwrite.
        let saved = refused.load(&script).unwrap_err();
        assert!(saved.contains("nothing is saved"), "{saved}");
        assert_eq!(std::fs::read(&store).unwrap(), before, "left untouched");
        assert_eq!(mode_of(&store), 0o666);
        assert!(refused.cards().is_empty());

        // Once the operator has dealt with the file, the same store restores.
        set_mode(&store, 0o600);
        let mut again = library(&dir);
        again.restore().unwrap();
        assert_eq!(card_names(&again), ["planted"]);
    }

    #[test]
    fn a_store_written_before_owner_only_files_is_tightened_and_fully_restored() {
        let dir = scratch("legacy-store");
        let one = bot_script(&dir, "one.ts");
        let two = bot_script(&dir, "two.ts");
        let store = write_store(
            &dir,
            &[
                ("one", one.to_str().unwrap()),
                ("two", two.to_str().unwrap()),
            ],
        );
        set_mode(&store, 0o644);

        let mut restored = library(&dir);
        restored.restore().unwrap();

        assert_eq!(card_names(&restored), ["one", "two"], "no entry is lost");
        assert_eq!(mode_of(&store), 0o600);
    }

    #[test]
    fn a_restored_entry_must_name_a_regular_file_of_sane_size() {
        let dir = scratch("entry-kinds");
        let good = bot_script(&dir, "good.ts");
        let big = dir.join("big.ts");
        std::fs::File::create(&big)
            .unwrap()
            .set_len(MAX_SOURCE_BYTES + 1)
            .unwrap();
        let exact = dir.join("exact.ts");
        let padding = MAX_SOURCE_BYTES as usize - BOT.len();
        std::fs::write(&exact, format!("{BOT}{}", " ".repeat(padding))).unwrap();
        let fifo = dir.join("pipe.ts");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        assert!(made.unwrap().success(), "mkfifo");
        std::fs::create_dir(dir.join("adir.ts")).unwrap();
        let store = write_store(
            &dir,
            &[
                ("good", good.to_str().unwrap()),
                ("big", big.to_str().unwrap()),
                ("exact", exact.to_str().unwrap()),
                ("fifo", fifo.to_str().unwrap()),
                ("adir", dir.join("adir.ts").to_str().unwrap()),
                ("gone", dir.join("missing.ts").to_str().unwrap()),
            ],
        );

        let mut restored = library(&dir);
        restored.restore().unwrap();

        assert_eq!(
            card_names(&restored),
            ["good", "exact"],
            "exactly at the bound is fine"
        );
        let mut refused = failure_names(&restored);
        refused.sort();
        assert_eq!(
            refused,
            ["adir", "big", "fifo"],
            "each refusal is on record"
        );
        for failure in restored.load_failures() {
            assert!(failure.diagnostic.contains("not restored"), "{failure:?}");
        }
        assert!(
            std::fs::read_to_string(&store)
                .unwrap()
                .contains("missing.ts"),
            "restore never rewrites the store"
        );
    }

    #[test]
    fn a_refused_catalog_root_file_is_named_and_a_legacy_one_is_tightened() {
        let dir = scratch("root-file");
        let file = dir.join("rs2b0t-path");
        let root = dir.join("catalog");

        assert_eq!(rs2b0t_root_checked_at(&file), Ok(None), "first run");

        persist_rs2b0t_root_at(&root, &file).unwrap();
        assert_eq!(rs2b0t_root_checked_at(&file), Ok(Some(root.clone())));

        set_mode(&file, 0o666);
        let refusal = rs2b0t_root_checked_at(&file).unwrap_err();
        assert!(refusal.contains("writable by other users"), "{refusal}");

        set_mode(&file, 0o644);
        assert_eq!(rs2b0t_root_checked_at(&file), Ok(Some(root)));
        assert_eq!(mode_of(&file), 0o600);
    }
}

#[test]
fn a_catalog_root_must_be_an_absolute_path_without_dots() {
    let dir = scratch("root-shape");
    let file = dir.join("rs2b0t-path");
    for (content, needle) in [
        ("relative/catalog", "not an absolute path"),
        ("./catalog", "not an absolute path"),
    ] {
        vault::write_private_file(&file, content.as_bytes()).unwrap();
        let error = rs2b0t_root_checked_at(&file).unwrap_err();
        assert!(error.contains(needle), "{content}: {error}");
    }
    let climbing = format!("{}/../elsewhere", dir.display());
    vault::write_private_file(&file, climbing.as_bytes()).unwrap();
    let error = rs2b0t_root_checked_at(&file).unwrap_err();
    assert!(error.contains("`..`"), "{error}");

    let fine = dir.join("catalog");
    vault::write_private_file(&file, format!("{}\n", fine.display()).as_bytes()).unwrap();
    assert_eq!(rs2b0t_root_checked_at(&file), Ok(Some(fine)));
}

#[test]
fn persisting_a_relative_catalog_root_stores_it_absolute() {
    let dir = scratch("root-absolute");
    let file = dir.join("rs2b0t-path");

    persist_rs2b0t_root_at(Path::new("some/../catalog"), &file).unwrap();

    let stored = std::fs::read_to_string(&file).unwrap();
    let expected = std::env::current_dir().unwrap().join("catalog");
    assert_eq!(stored, expected.to_string_lossy());
    assert_eq!(rs2b0t_root_checked_at(&file), Ok(Some(expected)));
}

#[test]
fn runtime_load_start_failure_records_a_runtime_load_stage_and_refusals_do_not() {
    let dir = scratch("start-result-stage");
    let mut lib = library(&dir);
    let path = bot_script(&dir, "bot.ts");
    let card = lib.load(&path).expect("bot loads");
    // A runtime setup failure is recorded against the attempted card with
    // the RuntimeLoad stage, and the diagnostic reaches the caller.
    let err = lib
        .record_start_result(
            &card,
            Err(crate::StartLoadError::RuntimeLoad(
                "isolate thread: boom".into(),
            )),
        )
        .expect_err("runtime failure propagates");
    assert_eq!(err, "isolate thread: boom");
    let failures = lib.load_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].stage, LoadStage::RuntimeLoad);
    assert_eq!(failures[0].identity_key, card.identity_key());
    // A refusal carries its diagnostic back without touching the record.
    let err = lib
        .record_start_result(&card, Err(crate::StartLoadError::Refused("busy".into())))
        .expect_err("refusal propagates");
    assert_eq!(err, "busy");
    assert_eq!(
        lib.load_failures().len(),
        1,
        "refusals leave diagnostics untouched"
    );
    // Success clears the recorded failure.
    lib.record_start_result(&card, Ok(()))
        .expect("success clears");
    assert!(lib.load_failures().is_empty());
}
