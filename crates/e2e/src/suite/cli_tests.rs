use super::*;

fn profile_identity_config(profile: &str, engine: &Path) -> NativeConfig {
    parse_args(&[
        "list".to_string(),
        "--profile".to_string(),
        profile.to_string(),
        "--engine".to_string(),
        engine.display().to_string(),
        "--catalog".to_string(),
        engine
            .parent()
            .unwrap()
            .join("missing-catalog")
            .display()
            .to_string(),
    ])
    .unwrap()
    .config
}

fn write_world_members(engine: &Path, revision: u16, port: u16, members: bool) -> PathBuf {
    let path = engine.join("data/config/world.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        &path,
        format!(
            r#"{{"engine":{{"revision":{revision}}},"node":{{"port":{port},"members":{members}}}}}"#
        ),
    )
    .unwrap();
    path
}

#[test]
fn profile_identity_records_effective_world_membership_and_provenance() {
    let root = std::env::temp_dir().join(format!(
        "274bot-e2e-members-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let engine = root.join("engine");
    let declaration = write_world_members(&engine, 274, 43594, true);

    let inherited = bind_profile(&profile_identity_config("local-274", &engine), &root).unwrap();
    let inherited_json = serde_json::to_value(&inherited).unwrap();
    assert_eq!(
        inherited.world_members, None,
        "legacy field remains the CLI override"
    );
    assert_eq!(inherited_json["effective_world_members"]["value"], true);
    assert_eq!(
        inherited_json["effective_world_members"]["source"],
        "local_world_json"
    );
    assert_eq!(
        inherited_json["effective_world_members"]["declaration"]["target"],
        declaration.display().to_string()
    );
    assert!(
        inherited_json["effective_world_members"]["declaration"]["sha256"]
            .as_str()
            .is_some_and(|digest| digest.len() == 64)
    );

    write_world_members(&engine, 274, 43594, false);
    let known_free = bind_profile(&profile_identity_config("local-274", &engine), &root).unwrap();
    let known_free_json = serde_json::to_value(&known_free).unwrap();
    assert_eq!(known_free_json["effective_world_members"]["value"], false);
    assert_eq!(
        known_free_json["effective_world_members"]["source"],
        "local_world_json"
    );

    let public = bind_profile(&profile_identity_config("public-289", &engine), &root).unwrap();
    let public_json = serde_json::to_value(&public).unwrap();
    assert!(public_json["effective_world_members"]["value"].is_null());
    assert_eq!(public_json["effective_world_members"]["source"], "unknown");
    assert!(public_json["effective_world_members"]["declaration"].is_null());

    let mut overridden = profile_identity_config("local-274", &engine);
    overridden.world_members = Some(true);
    overridden.extra_args = vec!["--world-members".into(), "false".into()];
    let overridden = bind_profile(&overridden, &root).unwrap();
    let overridden_json = serde_json::to_value(&overridden).unwrap();
    assert_eq!(overridden.world_members, Some(false));
    assert_eq!(overridden_json["effective_world_members"]["value"], false);
    assert_eq!(
        overridden_json["effective_world_members"]["source"],
        "explicit_override"
    );
    assert!(overridden_json["effective_world_members"]["declaration"].is_null());

    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn argument_parsing_rejects_unknown_flags_and_missing_values() {
    let error = parse_args(&["run".to_string(), "--nope".to_string()]).unwrap_err();
    assert!(error.contains("unknown flag"), "{error}");
    let error = parse_args(&["run".to_string(), "--profile".to_string()]).unwrap_err();
    assert!(error.contains("--profile needs a value"), "{error}");
    let error = parse_args(&["run".to_string()]).unwrap_err();
    assert!(error.contains("--catalog"), "{error}");
    let error = parse_args(&[]).unwrap_err();
    assert!(error.contains("no command"), "{error}");
    let error = parse_args(&[String::new()]).unwrap_err();
    assert!(error.contains("unknown argument"), "{error}");
    let args = parse_args(&[
        "run".to_string(),
        "--only".to_string(),
        "thiever,ardy".to_string(),
        "--catalog".to_string(),
        "/tmp".to_string(),
        "--profile".to_string(),
        "local-274".to_string(),
    ])
    .unwrap();
    assert_eq!(args.only, vec!["thiever", "ardy"]);
    assert_eq!(args.config.port, None);
    let error = parse_args(&[
        "run".to_string(),
        "--lowmem".to_string(),
        "--highmem".to_string(),
    ])
    .unwrap_err();
    assert!(error.contains("conflict"), "{error}");
}

/// A cleanup that could not reap the process tree is a shared failure on *every* exit
/// path, not only on a timeout: the suite cannot claim the child is gone.
#[test]
fn a_cleanup_that_could_not_reap_the_tree_stops_the_run() {
    let run = |reaped: bool, note: &str| child::ChildRun {
        pid: 1,
        exit_code: Some(0),
        signal: None,
        timed_out: false,
        interrupted: false,
        output_tail: String::new(),
        log_truncated: false,
        elapsed_ms: 0,
        cleanup: child::CleanupSummary {
            killed_signal: None,
            escalated_to_sigkill: false,
            reaped,
            note: note.into(),
        },
    };
    assert!(cleanup_failure(&run(true, "child exited on its own")).is_none());
    let reason = cleanup_failure(&run(false, "a descendant outlived the direct child"))
        .expect("a surviving tree is a harness failure");
    assert!(reason.contains("could not be reaped"), "{reason}");
    assert!(reason.contains("descendant"), "{reason}");
}

#[test]
fn list_and_dry_run_do_not_touch_a_run_directory() {
    let dir = std::env::temp_dir().join(format!("274bot-suite-cli-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for command in ["list", "dry-run"] {
        let code = main([
            command.to_string(),
            "--level".to_string(),
            "quick".to_string(),
            "--only".to_string(),
            "thiever".to_string(),
            "--run-dir".to_string(),
            dir.display().to_string(),
        ]);
        assert_eq!(code, EXIT_OK, "{command}");
        assert!(!dir.exists(), "{command} must not create the run directory");
    }
}

#[test]
fn run_without_a_run_dir_or_with_an_empty_selection_fails_before_launching() {
    let code = main([
        "run".to_string(),
        "--only".to_string(),
        "thiever".to_string(),
        "--catalog".to_string(),
        std::env::temp_dir().display().to_string(),
        "--profile".to_string(),
        "local-274".to_string(),
    ]);
    assert_eq!(code, EXIT_USAGE, "a run without --run-dir is a usage error");

    let dir = std::env::temp_dir().join(format!("274bot-suite-empty-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let code = main([
        "run".to_string(),
        "--only".to_string(),
        "nothing-matches-this".to_string(),
        "--catalog".to_string(),
        std::env::temp_dir().display().to_string(),
        "--profile".to_string(),
        "local-274".to_string(),
        "--run-dir".to_string(),
        dir.display().to_string(),
    ]);
    assert_eq!(code, EXIT_USAGE, "an empty selection is a request error");
}
