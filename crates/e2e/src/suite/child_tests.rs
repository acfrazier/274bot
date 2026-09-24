    use super::*;

    const CATALOG_PATH: &str = if cfg!(windows) {
        r"C:\catalog"
    } else {
        "/catalog"
    };

    fn config() -> NativeConfig {
        NativeConfig {
            profile: "local-274".into(),
            revision: Some("274".into()),
            host: Some("127.0.0.1".into()),
            port: Some(43594),
            engine: None,
            cache: None,
            catalog: PathBuf::from(CATALOG_PATH),
            vault: None,
            lowmem: true,
            mainland: false,
            world_members: None,
            nav_paints: true,
            exec_core: None,
            exec_pair: None,
            exec_external: None,
            external_ts: None,
            cwd: None,
            extra_args: Vec::new(),
        }
    }

    fn case(id: &str) -> CaseEntry {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        manifest.case(id).expect("case").clone()
    }

    /// The flags must be exactly the ones the native executables accept: the panel family
    /// parses them with `host_play::parse_profile_args` and hands the remainder to its live
    /// parser, which rejects anything but `--live`/`--smoke`/`--prod`. Consuming every flag
    /// here proves the child sees exactly `--live <name>`.
    #[test]
    fn profile_args_are_exactly_what_the_native_executables_accept() {
        let args = config().profile_args();
        assert_eq!(
            args,
            vec![
                "--profile",
                "local-274",
                "--revision",
                "274",
                "--host",
                "127.0.0.1",
                "--port",
                "43594",
                "--catalog",
                CATALOG_PATH,
            ]
        );
        let (options, rest) =
            host_play::parse_profile_args(args.iter().map(String::as_str)).unwrap();
        assert!(
            rest.is_empty(),
            "the panel's live parser would reject these: {rest:?}"
        );
        assert_eq!(options.profile.as_deref(), Some("local-274"));
        assert_eq!(options.catalog_root, Some(PathBuf::from(CATALOG_PATH)));
        assert_eq!(options.port, Some(43594));
        assert!(
            config().validate().is_err(),
            "a missing catalog fails closed"
        );
    }

    /// Mainland is not a flag of these executables; it travels as `BOT_MAINLAND=1`. Memory
    /// mode is a typed panel argument and is kept out of the profile resolver arguments.
    #[test]
    fn mainland_travels_as_environment_and_highmem_fails_closed() {
        let mut config = config();
        config.mainland = true;
        let env = config.child_env(&case("thiever"), Path::new("/run/shots"), &BTreeMap::new());
        assert_eq!(env.get(super::MAINLAND_ENV).map(String::as_str), Some("1"));
        assert_eq!(
            env.get(scenario::shot::SHOT_ROOT_ENV).map(String::as_str),
            Some("/run/shots")
        );
        assert_eq!(
            config.env_keys(),
            vec![
                scenario::shot::SHOT_ROOT_ENV.to_string(),
                super::MAINLAND_ENV.to_string()
            ]
        );
        assert!(
            !config
                .profile_args()
                .iter()
                .any(|arg| arg == "--mainland" || arg == "--lowmem" || arg == "--highmem"),
            "front-end-only flags must never reach the child"
        );

        let mut high = config.clone();
        high.lowmem = false;
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let dir = test_dir("memory-mode");
        high.exec_core = Some(fake_executable(&dir, "fixture-core"));
        high.exec_pair = Some(fake_executable(&dir, "fixture-pair"));
        high.catalog = PathBuf::from(manifest.defaults.exec.core.program.clone());
        let binaries = high
            .binaries(
                [RunnerKind::Core, RunnerKind::Pair],
                &manifest,
                Some(Path::new("/repo")),
            )
            .unwrap();
        for selected in ["thiever", "nature_crafter_air"] {
            let runner = manifest.case(selected).unwrap().runner;
            let command = high
                .command(manifest.case(selected).unwrap(), &binaries)
                .unwrap();
            assert!(
                command.iter().any(|arg| arg == "--highmem"),
                "{runner:?}: {command:?}"
            );
        }

        for raw in ["--lowmem", "--highmem"] {
            let mut invalid = high.clone();
            invalid.extra_args.push(raw.into());
            let error = invalid.validate().unwrap_err();
            assert!(error.contains("typed"), "{raw}: {error}");
        }
    }

    #[test]
    fn relative_input_paths_are_refused() {
        let mut vaulted = config();
        vaulted.vault = Some(PathBuf::from("relative-vault"));
        let error = vaulted.validate().unwrap_err();
        assert!(
            error.contains("--vault relative-vault is relative"),
            "{error}"
        );
        assert!(error.contains("absolute"), "{error}");

        let mut catalog_config = config();
        catalog_config.catalog = PathBuf::from("relative-catalog");
        let error = catalog_config.validate().unwrap_err();
        assert!(
            error.contains("--catalog relative-catalog is relative"),
            "{error}"
        );

        let mut exec = config();
        exec.exec_core = Some(PathBuf::from("relative-bin"));
        let error = exec.validate().unwrap_err();
        assert!(
            error.contains("--exec-core relative-bin is relative"),
            "{error}"
        );
    }

    /// A real file standing in for a built native executable.
    fn fake_executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n# {name} fixture\n")).unwrap();
        path
    }

    fn test_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("274bot-suite-child-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_case_command_is_the_resolved_executable_profile_flags_and_its_live_name() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let dir = test_dir("command");
        let core_bin = fake_executable(&dir, "fixture-core");
        let pair_bin = fake_executable(&dir, "fixture-pair");
        let mut config = config();
        config.catalog = PathBuf::from(manifest.defaults.exec.core.program.clone());
        config.exec_core = Some(core_bin.clone());
        config.exec_pair = Some(pair_bin.clone());
        config.mainland = true;

        let binaries = config
            .binaries(
                [RunnerKind::Core, RunnerKind::Pair],
                &manifest,
                Some(Path::new("/repo")),
            )
            .unwrap();
        let core_canonical = std::fs::canonicalize(&core_bin).unwrap();
        assert_eq!(
            binaries["core"].program,
            core_canonical.display().to_string(),
            "a direct executable is the canonical identity"
        );
        assert!(
            binaries["core"].sha256.is_some(),
            "the launched executable is content-bound"
        );

        let core = config.command(&case("thiever"), &binaries).unwrap();
        assert_eq!(core[0], core_canonical.display().to_string());
        assert_eq!(core[core.len() - 2..], ["--live", "script_thiever"]);
        assert!(core.iter().any(|arg| arg == "--lowmem"));

        let pair = config
            .command(&case("nature_crafter_air"), &binaries)
            .unwrap();
        assert_eq!(
            pair[0],
            std::fs::canonicalize(&pair_bin)
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(
            pair[pair.len() - 2..],
            ["--live", "script_nature_crafter_air"]
        );
    }

    /// Without a direct executable the manifest's cargo template is only a *reference* to
    /// a built artifact: `command` launches the resolved, hashed path, never `cargo run`
    /// (which could rebuild different bytes under the same command after the identity was
    /// captured), and refuses when no executable was resolved at all.
    #[test]
    fn a_command_launches_the_resolved_artifact_never_the_cargo_template() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let template = &manifest.defaults.exec.core;
        let resolved = BinaryIdentity {
            kind: "resolved-cargo".into(),
            program: "/repo/target/debug/catalog_watch".into(),
            args: template.args.clone(),
            sha256: Some("0".repeat(64)),
            size: Some(1024),
            note: None,
        };
        assert!(!template.args.is_empty(), "{:?}", template.args);
        let binaries: BTreeMap<String, BinaryIdentity> =
            [("core".to_string(), resolved)].into_iter().collect();
        let config = config();

        let command = config.command(&case("thiever"), &binaries).unwrap();
        assert_eq!(
            command[0], "/repo/target/debug/catalog_watch",
            "the launched program is the resolved artifact"
        );
        assert!(
            !command
                .iter()
                .any(|arg| arg == "cargo" || arg == "run" || arg == &template.program),
            "the cargo template must never reach argv: {command:?}"
        );
        assert!(
            !command.windows(2).any(|pair| pair == ["run", "-p"]),
            "cargo's own arguments are identity metadata, not child arguments: {command:?}"
        );
        assert_eq!(command[command.len() - 2..], ["--live", "script_thiever"]);

        // An unresolved identity (a command string, no bytes) is refused.
        let unresolved: BTreeMap<String, BinaryIdentity> = [(
            "core".to_string(),
            BinaryIdentity {
                kind: "unresolved-cargo".into(),
                program: template.program.clone(),
                args: template.args.clone(),
                sha256: None,
                size: None,
                note: Some("cargo run is not an identity".into()),
            },
        )]
        .into_iter()
        .collect();
        let error = config.command(&case("thiever"), &unresolved).unwrap_err();
        assert!(error.contains("no content identity"), "{error}");
        assert!(error.contains("--exec-core"), "{error}");

        // No identity at all for the runner kind is refused too, with the flag that
        // resolves it.
        let error = config
            .command(&case("thiever"), &BTreeMap::new())
            .unwrap_err();
        assert!(error.contains("--exec-core"), "{error}");
        let error = config
            .command(&case("nature_crafter_air"), &BTreeMap::new())
            .unwrap_err();
        assert!(error.contains("--exec-pair"), "{error}");
    }

    /// `binaries` keeps one identity per runner kind and resolves nothing it was not
    /// asked for.
    #[test]
    fn binaries_binds_one_executable_per_runner_kind() {
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let mut config = config();
        config.exec_core = Some(fake_executable(&test_dir("binaries"), "fixture-core"));

        let binaries = config
            .binaries(
                [RunnerKind::Core, RunnerKind::Core],
                &manifest,
                Some(Path::new("/repo")),
            )
            .unwrap();
        assert_eq!(binaries.len(), 1);
        assert_eq!(
            binaries["core"].program,
            std::fs::canonicalize(config.exec_core.as_ref().unwrap())
                .unwrap()
                .display()
                .to_string()
        );
        assert_eq!(binaries["core"].kind, "direct");
    }

    #[test]
    fn tail_buffer_is_bounded_and_drops_partial_head() {
        let mut tail = TailBuffer::default();
        for i in 0..4 {
            tail.push(format!("line {i}").as_bytes());
        }
        assert_eq!(tail.text(), "line 0\nline 1\nline 2\nline 3\n");
        for i in 0..(TAIL_CAP_BYTES / 8 + 16) {
            tail.push(format!("{i:06} filler").as_bytes());
        }
        assert!(tail.bytes.len() <= TAIL_CAP_BYTES, "tail stays bounded");
        assert!(tail.text().len() <= TAIL_CAP_BYTES);
        assert!(tail.text().ends_with('\n'));
    }

    #[test]
    fn unsupported_reference_args_and_env_are_not_forwarded() {
        assert!(!supported_arg("--no-deploy"));
        assert!(!supported_arg("--minutes"));
        assert!(!typed_env_key("HEADED"));
        assert!(!typed_env_key("E2E_MINUTES"));
    }

    /// The refused inherited deadline controls are the *actually consumed* ones: the list is
    /// pinned so a later edit cannot grow it into a speculative blanket environment
    /// sanitization (or quietly drop the control that review found). The refusal itself,
    /// including its message, is exercised end to end in `tests/suite_offline.rs`, where the
    /// child's environment is controlled exactly.
    #[test]
    fn the_refused_deadline_environment_is_the_bounded_native_list() {
        assert_eq!(super::DEADLINE_ENV, ["BUDGET_S"]);
        assert!(super::inherited_deadline_env()
            .iter()
            .all(|name| super::DEADLINE_ENV.contains(&name.as_str())));
    }

    /// The loader smoke's argv is the resolved external executable, the typed source override and
    /// the case's own live name — and a raw `--child-arg` cannot re-bind either typed selection.
    #[test]
    fn the_external_argv_carries_the_typed_source_override_and_only_that() {
        let source = if cfg!(windows) {
            PathBuf::from(r"C:\ext\ExampleBot.ts")
        } else {
            PathBuf::from("/ext/ExampleBot.ts")
        };
        let mut config = config();
        config.external_ts = Some(source.clone());
        let mut binaries = BTreeMap::new();
        binaries.insert(
            "external".to_string(),
            BinaryIdentity {
                kind: "direct".into(),
                program: "/bin/external_watch".into(),
                args: Vec::new(),
                sha256: Some("digest".into()),
                size: Some(1),
                note: None,
            },
        );
        let command = config.command(&case("external_loader"), &binaries).unwrap();
        assert_eq!(command[0], "/bin/external_watch");
        assert_eq!(
            command[command.len() - 4..],
            [
                "--live",
                "script_external_loader",
                "--external-ts",
                source.to_str().unwrap()
            ],
            "{command:?}"
        );

        // Without the typed override the child reads its own tracked fixture, which the run
        // identity bound: no flag is invented for it.
        let mut default_config = config.clone();
        default_config.external_ts = None;
        let command = default_config
            .command(&case("external_loader"), &binaries)
            .unwrap();
        assert_eq!(
            command[command.len() - 2..],
            ["--live", "script_external_loader"],
            "{command:?}"
        );

        // A raw extra argument must not re-bind the typed source or the typed live name.
        let mut evasive = config.clone();
        evasive.extra_args = vec!["--external-ts".into(), "/other.ts".into()];
        let error = evasive.validate().unwrap_err();
        assert!(error.contains("--external-ts"), "{error}");
        let mut relive = config.clone();
        relive.extra_args = vec!["--live".into(), "script_thiever".into()];
        let error = relive.validate().unwrap_err();
        assert!(error.contains("--live"), "{error}");
        assert!(
            config.validate().is_err(),
            "the fixture catalog path still fails closed for its own reason"
        );
    }

    /// The external runner resolves its own executable identity: `--exec-external` when given,
    /// otherwise the manifest's own `external_watch` template — never the core one.
    #[test]
    fn the_external_runner_resolves_its_own_executable() {
        let dir = test_dir("external-binary");
        let mut config = config();
        config.exec_external = Some(fake_executable(&dir, "external-watch"));
        let manifest = SuiteManifest::parse(
            crate::suite::EMBEDDED_MANIFEST.as_bytes(),
            "embedded fixture",
        )
        .unwrap();
        let binaries = config
            .binaries([RunnerKind::External], &manifest, Some(Path::new("/repo")))
            .unwrap();
        assert_eq!(binaries.len(), 1);
        assert_eq!(
            binaries["external"].program,
            std::fs::canonicalize(config.exec_external.as_ref().unwrap())
                .unwrap()
                .display()
                .to_string()
        );
        assert!(binaries["external"].resolved());
        assert!(
            !binaries.contains_key("core"),
            "a loader-only selection resolves no catalog executable"
        );

        let error = config
            .binaries([RunnerKind::External], &manifest, None)
            .err();
        assert!(
            error.is_none(),
            "the direct executable resolves without a workspace root: {error:?}"
        );
    }