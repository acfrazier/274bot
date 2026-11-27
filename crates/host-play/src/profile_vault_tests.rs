//! S8 vault split against real SQLite, real private files and, for every
//! interrupted two-write step, a real child process that aborts at the
//! named point (`S8_CRASH_AT`). The next unlock in the parent must settle
//! the interrupted step without losing a setting.

use super::*;
use crate::local_state::{LocalStateService, StateRoot};
use script::IsolatedEnv;
use serde_json::json;

#[path = "profile_vault_budget.rs"]
mod budget;

const PASS: &str = "s8-test-passphrase";

/// The 0.1.9 writer's own output (passphrase `bot`).
const LEGACY_0_1_9: &[u8] = include_bytes!("../../vault/tests/fixtures/legacy-0.1.9.vault");
/// The 0.1.8.1 writer's own output (passphrase `bot`).
const LEGACY_0_1_8_1: &[u8] =
    include_bytes!("../../frontend-core/tests/fixtures/vault-0.1.8.1.bin");
/// The 0.2.9.1 writer's own output (passphrase `v11c-legacy-fixture`).
const LEGACY_0_2_9: &[u8] =
    include_bytes!("../../vault/tests/fixtures/legacy-0.2.9-policies.vault");

struct Root {
    _env: IsolatedEnv,
    root: PathBuf,
    service: Option<Arc<LocalStateService>>,
}

impl Root {
    fn new(label: &str) -> Self {
        let env = IsolatedEnv::enter(label);
        let root = env.home.join(".274bot");
        vault::create_private_dir(&root).unwrap();
        Self::at(env, root)
    }

    fn at(env: IsolatedEnv, root: PathBuf) -> Self {
        let service = LocalStateService::open_process(StateRoot::new(root.clone()));
        Self {
            _env: env,
            root,
            service: Some(service),
        }
    }

    fn service(&self) -> &Arc<LocalStateService> {
        self.service.as_ref().expect("open")
    }

    /// Stop this process's service (its writer joins) and open a new one.
    fn restart(&mut self) {
        let service = self.service.take().expect("open");
        assert_eq!(
            Arc::strong_count(&service),
            1,
            "the test holds the only handle"
        );
        drop(service);
        self.service = Some(LocalStateService::open_process(StateRoot::new(
            self.root.clone(),
        )));
    }

    fn target(&self, server: &str, component: &str) -> VaultTarget {
        VaultTarget::new(server, &self.root.join(component)).unwrap()
    }

    fn unlock(&self, target: &VaultTarget, pass: &str) -> Result<Vault, JoinError> {
        unlock(
            self.service(),
            target,
            pass,
            &UnlockDecisions::default(),
            false,
        )
    }

    fn state(&self, target: &VaultTarget) -> ProfileState {
        self.service()
            .profile_state(target.server(), target.component())
            .unwrap()
    }

    /// Every byte of the database, its WAL and its shared-memory file.
    fn database_bytes(&self) -> Vec<u8> {
        let db = self.root.join("state").join("state.sqlite3");
        let mut bytes = Vec::new();
        for suffix in ["", "-wal", "-shm"] {
            let path = PathBuf::from(format!("{}{suffix}", db.display()));
            if let Ok(found) = std::fs::read(path) {
                bytes.extend(found);
            }
        }
        bytes
    }
}

fn contains(haystack: &[u8], needle: &str) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
}

fn profile(name: &str, password: &str, uid: i32, settings: ProfileSettings) -> Profile {
    Profile {
        username: name.into(),
        password: password.into(),
        uid,
        settings,
    }
}

/// A legacy (0.2.9-format) vault at `path` written by the old format's own
/// writer.
fn legacy_vault(path: &Path, pass: &str, profiles: &[Profile]) {
    let mut legacy = Vault::create(path, pass).unwrap();
    for profile in profiles {
        legacy.upsert(profile.clone()).unwrap();
    }
}

fn legacy_profiles(bytes: &[u8], pass: &str) -> BTreeMap<String, Profile> {
    match vault::authenticate(bytes, pass).unwrap().1 {
        Opened::Legacy(profiles) => profiles,
        Opened::Sealed { .. } => panic!("expected a legacy vault"),
    }
}

/// The credentials the vault file at `target` authenticates with `pass`,
/// with its binding.
fn sealed(target: &VaultTarget, pass: &str) -> (VaultBinding, BTreeMap<String, Credential>) {
    let (bytes, _) = vault::read_file(target.path()).unwrap();
    match vault::authenticate(&bytes, pass).unwrap().1 {
        Opened::Sealed {
            binding,
            credentials,
        } => (binding, credentials),
        Opened::Legacy(_) => panic!("expected a credential-only vault"),
    }
}

fn joined(vault: &Vault) -> BTreeMap<String, Profile> {
    vault
        .profiles()
        .map(|profile| (profile.username.clone(), profile.clone()))
        .collect()
}

/// `legacy` as the first conversion stores it (compiled conversions).
fn converted(mut legacy: BTreeMap<String, Profile>) -> BTreeMap<String, Profile> {
    for profile in legacy.values_mut() {
        convert_compiled(&mut profile.settings);
    }
    legacy
}

fn big_settings(seed: usize, bytes: usize) -> ProfileSettings {
    let mut settings = ProfileSettings {
        lowmem: seed.is_multiple_of(2),
        world: Some(300 + seed as u16),
        lamp_skill: format!("skill{seed}"),
        ..ProfileSettings::default()
    };
    let mut bag = Map::new();
    bag.insert("payload".into(), json!("x".repeat(bytes)));
    bag.insert("seed".into(), json!(seed));
    settings
        .script_settings
        .insert(format!("catalog:Big{seed}"), bag);
    settings
        .script_settings
        .insert("catalog:Empty".into(), Map::new());
    settings
}

/// The supplied fixtures convert with 0.2.9 defaults: every field survives
/// both a restart and a second unlock, the vault keeps only usernames and
/// passwords, and no password or passphrase reaches the database.
#[test]
fn shipped_fixtures_convert_and_both_stores_agree_after_restart() {
    let _lease = crate::test_process::state();
    for (label, bytes, pass) in [
        ("0.1.9", LEGACY_0_1_9, "bot"),
        ("0.1.8.1", LEGACY_0_1_8_1, "bot"),
        ("0.2.9", LEGACY_0_2_9, "v11c-legacy-fixture"),
    ] {
        let mut root = Root::new(&format!("s8-fixture-{label}"));
        let target = root.target("Local-274", "vault");
        std::fs::write(target.path(), bytes).unwrap();
        let expected = converted(legacy_profiles(bytes, pass));
        assert!(!expected.is_empty(), "{label}: fixture has profiles");

        let first = root.unlock(&target, pass).unwrap();
        assert_eq!(joined(&first), expected, "{label}: first join");
        drop(first);

        let (binding, credentials) = sealed(&target, pass);
        assert_eq!(&binding, target.binding());
        assert_eq!(
            credentials.keys().collect::<Vec<_>>(),
            expected.keys().collect::<Vec<_>>()
        );
        for (name, credential) in &credentials {
            assert_eq!(credential.password, expected[name].password);
        }

        root.restart();
        let state = root.state(&target);
        assert!(state.pending.is_empty(), "{label}: seal settled");
        let again = root.unlock(&target, pass).unwrap();
        assert_eq!(joined(&again), expected, "{label}: after restart");

        let db = root.database_bytes();
        for profile in expected.values() {
            assert!(
                !contains(&db, profile.password.as_str()),
                "{label}: a password reached the database"
            );
            assert!(
                contains(&db, &profile.username),
                "{label}: rows hold usernames"
            );
        }
        // `bot` is too short to search for: it is part of ordinary text
        // (`274bot`). The distinct fixture passphrase is checked.
        if pass.len() >= 8 {
            assert!(
                !contains(&db, pass),
                "{label}: the passphrase reached the database"
            );
        }
        if label == "0.2.9" {
            let alice = &again.get("alice").unwrap().settings;
            assert_eq!(
                alice.random_event_policy("river troll"),
                vault::RandomEventPolicy::Flee
            );
            assert_eq!(
                alice.random_event_policy("swarm"),
                vault::RandomEventPolicy::Fight
            );
            assert_eq!(alice.random_event_policies.len(), 5);
            assert!(!contains(&db, "\"ignore\""), "the new store writes flee");
        }
        if label == "0.1.8.1" {
            let alice = again.get("alice").unwrap();
            assert_eq!(alice.uid, 274_000_501);
            assert!(alice.settings.auto_login && !alice.settings.lowmem);
            assert_eq!(alice.settings.world, Some(2));
            assert_eq!(alice.settings.tutorial_skipped, Some(true));
            assert_eq!(alice.settings.raster, vault::RasterMode::Cpu);
            assert_eq!(alice.settings.lamp_skill, "prayer");
        }
    }
}

/// SD1 §5: an empty compiled entry becomes the schema-1 envelope without
/// promotion, Gatherer 4 becomes 5 with the same overrides, and accepted,
/// unsupported and malformed entries keep their representation.
#[test]
fn compiled_settings_convert_as_the_card_reads_them() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-compiled");
    let target = root.target("local-274", "vault");
    let mut settings = ProfileSettings::default();
    let bag = |value: Value| value.as_object().unwrap().clone();
    let bags = &mut settings.script_settings;
    bags.insert(
        "compiled:Gatherer".into(),
        bag(json!({"schema_version": 4, "values": {"site": "draynor"}})),
    );
    bags.insert("compiled:Quester".into(), Map::new());
    bags.insert(
        "compiled:Grinch".into(),
        bag(json!({"schema_version": 1, "values": {"mode": "x"}})),
    );
    bags.insert("compiled:WalkTo".into(), bag(json!({"bogus": 1})));
    bags.insert("catalog:Plain".into(), Map::new());
    let mut other = ProfileSettings::default();
    other.script_settings.insert(
        "compiled:Gatherer".into(),
        bag(json!({"schema_version": 3, "values": {"site": "old"}})),
    );
    legacy_vault(
        target.path(),
        PASS,
        &[
            profile("alice", "pa", 7, settings),
            profile("bob", "pb", 8, other),
        ],
    );
    let vault = root.unlock(&target, PASS).unwrap();
    let alice = &vault.get("alice").unwrap().settings.script_settings;
    assert_eq!(
        alice["compiled:Gatherer"],
        bag(json!({"schema_version": 5, "values": {"site": "draynor"}}))
    );
    assert_eq!(
        alice["compiled:Quester"],
        bag(json!({"schema_version": 1, "values": {}})),
        "not promoted: Quester does not read schema 1"
    );
    assert!(!script::compiled_card(script::CompiledId("Quester"))
        .unwrap()
        .reads_saved_schema(1));
    assert_eq!(
        alice["compiled:Grinch"],
        bag(json!({"schema_version": 1, "values": {"mode": "x"}}))
    );
    assert_eq!(
        alice["compiled:WalkTo"],
        bag(json!({"bogus": 1})),
        "malformed kept"
    );
    assert_eq!(alice["catalog:Plain"], Map::new(), "present empty bag kept");
    let bob = &vault.get("bob").unwrap().settings.script_settings;
    assert_eq!(
        bob["compiled:Gatherer"],
        bag(json!({"schema_version": 3, "values": {"site": "old"}})),
        "schema 3 stays unsupported for Gatherer"
    );
}

#[test]
fn a_wrong_passphrase_never_starts_the_conversion() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-wrong-pass");
    let target = root.target("local-274", "vault");
    legacy_vault(
        target.path(),
        PASS,
        &[profile("alice", "pa", 1, ProfileSettings::default())],
    );
    let before = std::fs::read(target.path()).unwrap();
    assert!(matches!(
        root.unlock(&target, "not-it"),
        Err(JoinError::Vault(VaultError::WrongPassphrase))
    ));
    assert_eq!(std::fs::read(target.path()).unwrap(), before);
    let state = root.state(&target);
    assert!(state.binding.is_none() && !state.unbound_rows && state.pending.is_empty());
}

/// A completed binding refuses a later legacy replacement, while another
/// server's untouched legacy vault still completes its own first import.
#[test]
fn a_completed_binding_refuses_a_later_legacy_vault() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-legacy-replacement");
    let target = root.target("local-274", "vault");
    legacy_vault(
        target.path(),
        PASS,
        &[profile("alice", "pa", 1, ProfileSettings::default())],
    );
    root.unlock(&target, PASS).unwrap();
    std::fs::remove_file(target.path()).unwrap();
    legacy_vault(
        target.path(),
        PASS,
        &[profile("mallory", "pm", 9, ProfileSettings::default())],
    );
    let before = std::fs::read(target.path()).unwrap();
    match root.unlock(&target, PASS) {
        Err(JoinError::Refused {
            phase: JoinPhase::Binding,
            detail,
        }) => {
            assert!(detail.contains("already converted"), "{detail}")
        }
        other => panic!("expected a binding refusal, got {:?}", other.err()),
    }
    assert_eq!(
        std::fs::read(target.path()).unwrap(),
        before,
        "not rewritten"
    );

    let other = root.target("local-289", "vault-289");
    legacy_vault(
        other.path(),
        PASS,
        &[profile("carol", "pc", 3, ProfileSettings::default())],
    );
    let joined = root.unlock(&other, PASS).unwrap();
    assert!(joined.get("carol").is_some());
}

/// Two servers with distinct components and an `alice` in each keep their
/// rows separate; swapped sealed files refuse even with identical
/// usernames; a server configured with another server's component refuses.
#[test]
fn two_servers_keep_their_alice_rows_and_swapped_files_refuse() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-two-servers");
    let a = root.target("local-274", "vault");
    let b = root.target("local-289", "vault-289");
    let settings = |world| ProfileSettings {
        world: Some(world),
        ..ProfileSettings::default()
    };
    legacy_vault(a.path(), PASS, &[profile("alice", "pa", 11, settings(301))]);
    legacy_vault(b.path(), PASS, &[profile("alice", "pb", 22, settings(302))]);
    assert_eq!(root.unlock(&a, PASS).unwrap().get("alice").unwrap().uid, 11);
    assert_eq!(root.unlock(&b, PASS).unwrap().get("alice").unwrap().uid, 22);
    assert_eq!(root.state(&a).rows[0].settings.world, Some(301));
    assert_eq!(root.state(&b).rows[0].settings.world, Some(302));

    let swap = root.root.join("swap");
    std::fs::rename(a.path(), &swap).unwrap();
    std::fs::rename(b.path(), a.path()).unwrap();
    std::fs::rename(&swap, b.path()).unwrap();
    for target in [&a, &b] {
        match root.unlock(target, PASS) {
            Err(JoinError::Refused {
                phase: JoinPhase::Binding,
                detail,
            }) => {
                assert!(detail.contains("was sealed for server"), "{detail}")
            }
            other => panic!("a swapped file must not join: {:?}", other.err()),
        }
    }
    std::fs::rename(a.path(), &swap).unwrap();
    std::fs::rename(b.path(), a.path()).unwrap();
    std::fs::rename(&swap, b.path()).unwrap();
    assert_eq!(
        root.unlock(&a, PASS)
            .unwrap()
            .get("alice")
            .unwrap()
            .password,
        "pa"
    );

    let moved = root.target("local-274", "vault-289");
    assert!(matches!(
        root.unlock(&moved, PASS),
        Err(JoinError::Refused {
            phase: JoinPhase::Binding,
            ..
        })
    ));
}

/// R1: a collision and an invalid name refuse non-interactively and ask
/// interactively, with names and reasons only; both stores stay unchanged
/// until every answer is given; then only the chosen and valid rows survive
/// with their original uid and settings, after a restart too.
#[test]
fn account_key_repair_asks_then_keeps_only_the_chosen_rows() {
    let _lease = crate::test_process::state();
    let mut root = Root::new("s8-repair");
    let target = root.target("local-274", "vault");
    let world = |world| ProfileSettings {
        world: Some(world),
        ..ProfileSettings::default()
    };
    legacy_vault(
        target.path(),
        PASS,
        &[
            profile("Alice", "secret-upper", 101, world(301)),
            profile("alice", "secret-lower", 102, world(302)),
            profile("bad name!!", "secret-bad", 103, world(303)),
            profile("bob", "secret-bob", 104, world(304)),
        ],
    );
    let before = std::fs::read(target.path()).unwrap();
    let items = match root.unlock(&target, PASS) {
        Err(JoinError::NonInteractive(items)) => items,
        other => panic!("expected a non-interactive refusal, got {:?}", other.err()),
    };
    assert_eq!(items.len(), 2);
    let asked = match unlock(
        root.service(),
        &target,
        PASS,
        &UnlockDecisions::default(),
        true,
    ) {
        Err(JoinError::NeedsDecisions(items)) => items,
        other => panic!("expected questions, got {:?}", other.err()),
    };
    assert_eq!(asked, items);
    assert!(asked.contains(&RepairItem::Collision {
        account: "alice".into(),
        usernames: vec!["Alice".into(), "alice".into()],
    }));
    assert!(asked.iter().any(|item| matches!(item,
        RepairItem::Invalid { username, .. } if username == "bad name!!")));
    for item in &asked {
        let text = item.to_string();
        assert!(
            !text.contains("secret-"),
            "no password in a question: {text}"
        );
        assert!(
            !text.contains("30"),
            "no setting value in a question: {text}"
        );
    }
    assert_eq!(std::fs::read(target.path()).unwrap(), before);
    assert!(root.state(&target).binding.is_none());

    let mut decisions = UnlockDecisions::default();
    decisions.keep.insert("alice".into(), "alice".into());
    decisions.remove_invalid.insert("bad name!!".into());
    let vault = unlock(root.service(), &target, PASS, &decisions, true).unwrap();
    assert_eq!(
        vault
            .profiles()
            .map(|p| p.username.as_str())
            .collect::<Vec<_>>(),
        ["alice", "bob"]
    );
    drop(vault);
    root.restart();
    let vault = root.unlock(&target, PASS).unwrap();
    let alice = vault.get("alice").unwrap();
    assert_eq!((alice.uid, alice.settings.world), (102, Some(302)));
    assert_eq!(alice.password, "secret-lower");
    let bob = vault.get("bob").unwrap();
    assert_eq!((bob.uid, bob.settings.world), (104, Some(304)));
    let (_, credentials) = sealed(&target, PASS);
    assert_eq!(credentials.keys().collect::<Vec<_>>(), ["alice", "bob"]);
}

/// The secret spill check: exact whole-value and whole-key matches against
/// another profile's password pause the conversion; substrings, excluded
/// identifiers and the passphrase never do; clear removes the entry and
/// confirm keeps it.
#[test]
fn secret_spills_pause_the_conversion_until_cleared_or_confirmed() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-spill");
    let target = root.target("local-274", "vault");
    // A short passphrase that equals a moved setting never blocks: the
    // check compares game passwords only.
    let passphrase = "ab";
    let bag = |value: Value| value.as_object().unwrap().clone();
    let mut alice = ProfileSettings {
        clue_duel_partner: "alice-pw-1".into(),
        lamp_skill: passphrase.into(),
        ..ProfileSettings::default()
    };
    alice.script_settings.insert(
        "catalog:Notes".into(),
        bag(json!({"note": "bob-pw-22", "keep": 1})),
    );
    alice.script_settings.insert(
        "catalog:Keys".into(),
        bag(json!({"nested": {"alice-pw-1": true}, "sub": "x alice-pw-1 x"})),
    );
    let carol = ProfileSettings {
        clue_duel_partner: "carol".into(),
        ..ProfileSettings::default()
    };
    legacy_vault(
        target.path(),
        passphrase,
        &[
            profile("alice", "alice-pw-1", 1, alice),
            profile("bob", "bob-pw-22", 2, ProfileSettings::default()),
            profile("carol", "carol", 3, carol),
        ],
    );
    let before = std::fs::read(target.path()).unwrap();
    assert!(matches!(
        root.unlock(&target, passphrase),
        Err(JoinError::NonInteractive(_))
    ));
    let items = match unlock(
        root.service(),
        &target,
        passphrase,
        &UnlockDecisions::default(),
        true,
    ) {
        Err(JoinError::NeedsDecisions(items)) => items,
        other => panic!("expected spill questions, got {:?}", other.err()),
    };
    assert_eq!(items.len(), 2, "{items:?}");
    for item in &items {
        let text = item.to_string();
        for secret in ["alice-pw-1", "bob-pw-22", "note", "nested"] {
            assert!(!text.contains(secret), "{text}");
        }
        assert!(matches!(
            item,
            RepairItem::Spill {
                category: "script setting",
                ..
            }
        ));
    }
    assert_eq!(std::fs::read(target.path()).unwrap(), before);
    assert!(root.state(&target).binding.is_none());

    let ids: Vec<String> = items
        .iter()
        .map(|item| match item {
            RepairItem::Spill { id, .. } => id.clone(),
            _ => unreachable!(),
        })
        .collect();
    let mut decisions = UnlockDecisions::default();
    // Bags sort as catalog:Keys (0), catalog:Notes (1).
    let keys = ids.iter().find(|id| id.contains("#bag0.")).unwrap();
    let notes = ids.iter().find(|id| id.contains("#bag1.")).unwrap();
    decisions.spill.insert(notes.clone(), SpillAction::Clear);
    decisions.spill.insert(keys.clone(), SpillAction::Confirm);
    let vault = unlock(root.service(), &target, passphrase, &decisions, true).unwrap();
    let settings = &vault.get("alice").unwrap().settings;
    assert_eq!(
        settings.script_settings["catalog:Notes"],
        bag(json!({"keep": 1}))
    );
    assert!(settings.script_settings["catalog:Keys"].contains_key("nested"));
    assert_eq!(
        settings.clue_duel_partner, "alice-pw-1",
        "excluded identifier"
    );
    let db = root.database_bytes();
    assert!(
        !contains(&db, "bob-pw-22"),
        "the cleared value never reached SQL"
    );
}

/// Grow a legacy vault large enough for several staging batches.
fn staged_fixture(root: &Root) -> (VaultTarget, BTreeMap<String, Profile>) {
    let target = root.target("local-274", "vault");
    let profiles: Vec<Profile> = (0..3)
        .map(|seed| {
            profile(
                &format!("user{seed}"),
                &format!("pw-{seed}-xyz"),
                274_000_010 + seed as i32,
                big_settings(seed, 700 * 1024),
            )
        })
        .collect();
    legacy_vault(target.path(), PASS, &profiles);
    let expected = converted(
        profiles
            .into_iter()
            .map(|profile| (profile.username.clone(), profile))
            .collect(),
    );
    (target, expected)
}

/// The child of the interrupted-conversion tests: unlock the vault at
/// `S8_ROOT` and abort at `S8_CRASH_AT`.
#[test]
#[ignore = "child process of the interrupted S8 tests"]
fn s8_crash_child() {
    let root = PathBuf::from(std::env::var("S8_ROOT").unwrap());
    let service = LocalStateService::open_process(StateRoot::new(root.clone()));
    let target = VaultTarget::new("local-274", &root.join("vault")).unwrap();
    match std::env::var("S8_ACTION").as_deref() {
        Ok("unlock") => {
            let _ = unlock(&service, &target, PASS, &UnlockDecisions::default(), false);
        }
        Ok("rename") => {
            let vault =
                unlock(&service, &target, PASS, &UnlockDecisions::default(), false).unwrap();
            let mut store = ProfileStore::joined(vault.store(), Arc::clone(&service));
            let mut renamed = vault.get("user0").unwrap().clone();
            renamed.username = std::env::var("S8_RENAME_TO").unwrap();
            renamed.password = "pw-renamed".into();
            renamed.settings.world = Some(399);
            let _ = store.commit_rename("user0", &renamed);
        }
        Ok("reset") => {
            let _ = reset(&service, &target);
        }
        other => panic!("unknown action {other:?}"),
    }
    // A child that reached no crash point exits normally.
}

fn crash(root: &Root, action: &str, at: &str, extra: &[(&str, &str)]) {
    let mut command = std::process::Command::new(crate::test_process::executable());
    command
        .args([
            "--ignored",
            "--exact",
            "profile_vault::tests::s8_crash_child",
            "--nocapture",
        ])
        .env("S8_ROOT", &root.root)
        .env("S8_ACTION", action)
        .env("S8_CRASH_AT", at)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    for (key, value) in extra {
        command.env(key, value);
    }
    let status = command.status().unwrap();
    if at != "none" {
        assert!(!status.success(), "the child aborted at {at}");
    }
}

/// A stop at every phase of the first conversion (after one staged batch,
/// inside the activation transaction, after the SQL commit, after the vault
/// replacement, after settlement) loses no setting: the next unlock in a
/// new process settles the step and joins exactly the converted profiles.
#[test]
fn a_stop_at_every_seal_phase_loses_no_setting() {
    let _lease = crate::test_process::state();
    for at in [
        "stage-batch-committed",
        "before-activation",
        "before-activation-commit",
        "after-sql-commit",
        "after-vault-replace",
        "none",
    ] {
        let mut root = Root::new(&format!("s8-seal-{at}"));
        let (target, expected) = staged_fixture(&root);
        root.restart();
        root.service = None;
        crash(&root, "unlock", at, &[]);
        root.service = Some(LocalStateService::open_process(StateRoot::new(
            root.root.clone(),
        )));
        let state = root.state(&target);
        match at {
            "stage-batch-committed" | "before-activation" | "before-activation-commit" => {
                assert!(state.binding.is_none(), "{at}: nothing is bound");
                assert!(
                    state.unbound_rows,
                    "{at}: staged rows exist but are not live"
                );
                assert!(state.rows.is_empty(), "{at}: staged rows are invisible");
                assert_eq!(
                    legacy_profiles(&std::fs::read(target.path()).unwrap(), PASS).len(),
                    3,
                    "{at}: the legacy vault stays authoritative"
                );
            }
            "after-sql-commit" => {
                assert!(
                    state.pending(PendingKind::Seal).is_some(),
                    "{at}: seal pending"
                );
                assert!(
                    matches!(
                        legacy_profiles(&std::fs::read(target.path()).unwrap(), PASS).len(),
                        3
                    ),
                    "{at}: the legacy vault is still in place"
                );
            }
            "after-vault-replace" => {
                assert!(
                    state.pending(PendingKind::Seal).is_some(),
                    "{at}: settle pending"
                );
                sealed(&target, PASS);
            }
            _ => assert!(state.pending.is_empty()),
        }
        let vault = root.unlock(&target, PASS).unwrap();
        assert_eq!(joined(&vault), expected, "{at}: no setting is lost");
        drop(vault);
        root.restart();
        let state = root.state(&target);
        assert!(
            state.pending.is_empty() && !state.unbound_rows,
            "{at}: settled"
        );
        assert_eq!(
            joined(&root.unlock(&target, PASS).unwrap()),
            expected,
            "{at}"
        );
        sealed(&target, PASS);
    }
}

/// Post-seal writes: settings-only edits leave the vault image alone;
/// credential-only edits leave the rows alone; create and delete keep both
/// stores in step; an occupied rename target changes neither store.
#[test]
fn joined_writes_keep_both_stores_in_step() {
    let _lease = crate::test_process::state();
    let mut root = Root::new("s8-writes");
    let target = root.target("local-274", "vault");
    legacy_vault(
        target.path(),
        PASS,
        &[
            profile("alice", "pa", 1, ProfileSettings::default()),
            profile("bob", "pb", 2, ProfileSettings::default()),
        ],
    );
    let vault = root.unlock(&target, PASS).unwrap();
    let mut store = ProfileStore::joined(vault.store(), Arc::clone(root.service()));

    let image = std::fs::read(target.path()).unwrap();
    let mut alice = vault.get("alice").unwrap().clone();
    alice.settings.world = Some(333);
    alice
        .settings
        .script_settings
        .insert("catalog:X".into(), Map::new());
    store.commit(&[VaultChange::Upsert(alice.clone())]).unwrap();
    assert_eq!(
        std::fs::read(target.path()).unwrap(),
        image,
        "settings only"
    );

    let rows = root.state(&target).rows;
    alice.password = "pa-2".into();
    store.commit(&[VaultChange::Upsert(alice.clone())]).unwrap();
    assert_eq!(root.state(&target).rows, rows, "credential only");
    assert_eq!(sealed(&target, PASS).1["alice"].password, "pa-2");

    let carol = profile("carol", "pc", 3, ProfileSettings::default());
    store.commit(&[VaultChange::Upsert(carol)]).unwrap();
    store.commit(&[VaultChange::Remove("bob".into())]).unwrap();

    // An occupied target (alice's canonical account, another spelling) is
    // refused and changes neither store.
    let image = std::fs::read(target.path()).unwrap();
    let rows = root.state(&target).rows;
    let mut bad = store.get("carol").unwrap().clone();
    bad.username = "Alice".into();
    assert!(store.commit_rename("carol", &bad).is_err());
    assert_eq!(std::fs::read(target.path()).unwrap(), image);
    assert_eq!(root.state(&target).rows, rows);
    assert!(root.state(&target).pending.is_empty());

    // A spelling-only rename keeps the canonical key, uid and settings.
    let mut spelled = store.get("alice").unwrap().clone();
    spelled.username = "Alice".into();
    store.commit_rename("alice", &spelled).unwrap();
    drop(store);
    drop(vault);
    root.restart();
    let vault = root.unlock(&target, PASS).unwrap();
    assert_eq!(
        vault
            .profiles()
            .map(|p| p.username.as_str())
            .collect::<Vec<_>>(),
        ["Alice", "carol"]
    );
    let alice = vault.get("Alice").unwrap();
    assert_eq!((alice.uid, alice.settings.world), (1, Some(333)));
    assert_eq!(alice.password, "pa-2");
    assert_eq!(vault.get("carol").unwrap().uid, 3);
    assert_eq!(root.state(&target).rows.len(), 2, "bob's row is retired");
}

/// A stop before, during and after each rename step leaves exactly one
/// credential that joins with all the old or all the new settings, for a
/// rename that changes the canonical key and for a spelling-only rename.
#[test]
fn a_stop_at_every_rename_phase_joins_one_whole_profile() {
    let _lease = crate::test_process::state();
    for to in ["newname", "User0"] {
        for at in [
            "rename-before-prepare",
            "rename-after-prepare",
            "rename-after-vault",
            "before-rename-finalize-commit",
            "rename-after-finalize",
        ] {
            let mut root = Root::new(&format!("s8-rename-{to}-{at}"));
            let (target, expected) = staged_fixture(&root);
            root.unlock(&target, PASS).unwrap();
            root.service = None;
            crash(&root, "rename", at, &[("S8_RENAME_TO", to)]);
            root.service = Some(LocalStateService::open_process(StateRoot::new(
                root.root.clone(),
            )));
            let vault = root.unlock(&target, PASS).unwrap();
            let names: Vec<&str> = vault.profiles().map(|p| p.username.as_str()).collect();
            let old = &expected["user0"];
            let renamed = matches!(
                at,
                "rename-after-vault" | "before-rename-finalize-commit" | "rename-after-finalize"
            );
            if renamed {
                assert!(
                    names.contains(&to) && !names.contains(&"user0"),
                    "{to} {at}: {names:?}"
                );
                let new = vault.get(to).unwrap();
                assert_eq!(new.uid, old.uid);
                assert_eq!(new.password, "pw-renamed");
                assert_eq!(new.settings.world, Some(399));
                assert_eq!(
                    new.settings.script_settings, old.settings.script_settings,
                    "{to} {at}: all the new settings"
                );
            } else {
                assert!(
                    names.contains(&"user0") && !names.contains(&to),
                    "{to} {at}"
                );
                assert_eq!(
                    vault.get("user0").unwrap(),
                    old,
                    "{to} {at}: all the old settings"
                );
            }
            assert_eq!(names.len(), 3);
            assert!(root.state(&target).pending.is_empty());
        }
    }
}

/// Reset retires the rows and keeps the binding; a legacy vault placed
/// afterwards is not imported; a new vault and a new `alice` get a fresh
/// uid and their own settings, and nothing retired returns.
#[test]
fn reset_then_create_alice_again_with_fresh_state() {
    let _lease = crate::test_process::state();
    let mut root = Root::new("s8-reset");
    let target = root.target("local-274", "vault");
    let old_settings = ProfileSettings {
        world: Some(377),
        lamp_skill: "magic".into(),
        ..ProfileSettings::default()
    };
    legacy_vault(
        target.path(),
        PASS,
        &[profile("alice", "old-pw", 50, old_settings)],
    );
    root.unlock(&target, PASS).unwrap();
    assert_eq!(
        reset(root.service(), &target).unwrap(),
        ResetOutcome::Deleted
    );
    assert!(!target.path().exists());
    let state = root.state(&target);
    assert!(state.rows.is_empty() && state.binding.as_ref().unwrap().absent_ok);
    assert!(matches!(
        root.unlock(&target, PASS),
        Err(JoinError::Vault(VaultError::NotFound(_)))
    ));

    legacy_vault(
        target.path(),
        PASS,
        &[profile("alice", "old-pw", 50, ProfileSettings::default())],
    );
    assert!(matches!(
        root.unlock(&target, PASS),
        Err(JoinError::Refused {
            phase: JoinPhase::Binding,
            ..
        })
    ));
    std::fs::remove_file(target.path()).unwrap();

    let vault = create(root.service(), &target, "new-pass").unwrap();
    let mut store = ProfileStore::joined(vault.store(), Arc::clone(root.service()));
    let fresh = ProfileSettings {
        world: Some(301),
        ..ProfileSettings::default()
    };
    store
        .commit(&[VaultChange::Upsert(profile(
            "alice",
            "new-pw",
            60,
            fresh.clone(),
        ))])
        .unwrap();
    drop(store);
    drop(vault);
    root.restart();
    let vault = root.unlock(&target, "new-pass").unwrap();
    let alice = vault.get("alice").unwrap();
    assert_eq!(
        (alice.uid, &alice.settings, alice.password.as_str()),
        (60, &fresh, "new-pw")
    );
    assert_eq!(root.state(&target).rows.len(), 1);
}

/// A stop inside a reset: before the file is deleted the reset is cancelled
/// and the vault opens unchanged; after it the reset settles and the vault
/// is absent. A reset during a pending seal cancels the conversion and keeps
/// the first import closed.
#[test]
fn an_interrupted_reset_settles_and_a_reset_cancels_a_pending_seal() {
    let _lease = crate::test_process::state();
    for at in ["reset-after-prepare", "reset-after-delete"] {
        let mut root = Root::new(&format!("s8-{at}"));
        let (target, expected) = staged_fixture(&root);
        root.unlock(&target, PASS).unwrap();
        root.service = None;
        crash(&root, "reset", at, &[]);
        root.service = Some(LocalStateService::open_process(StateRoot::new(
            root.root.clone(),
        )));
        match at {
            "reset-after-prepare" => {
                assert_eq!(joined(&root.unlock(&target, PASS).unwrap()), expected)
            }
            _ => assert!(matches!(
                root.unlock(&target, PASS),
                Err(JoinError::Vault(VaultError::NotFound(_)))
            )),
        }
        assert!(root.state(&target).pending.is_empty(), "{at}");
    }

    let mut root = Root::new("s8-reset-pending-seal");
    let (target, _) = staged_fixture(&root);
    root.restart();
    root.service = None;
    crash(&root, "unlock", "after-sql-commit", &[]);
    root.service = Some(LocalStateService::open_process(StateRoot::new(
        root.root.clone(),
    )));
    assert!(root.state(&target).pending(PendingKind::Seal).is_some());
    reset(root.service(), &target).unwrap();
    let state = root.state(&target);
    assert!(state.pending.is_empty() && state.rows.is_empty());
    staged_fixture(&root);
    assert!(matches!(
        root.unlock(&target, PASS),
        Err(JoinError::Refused {
            phase: JoinPhase::Binding,
            ..
        })
    ));
}

/// Missing rows refuse the join with the recovery hint; recovery under
/// confirmation creates only the missing rows with fresh uids and keeps
/// the surviving ones; it refuses a pending split.
#[test]
fn missing_rows_recover_with_defaults_only_when_confirmed() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-recover");
    let target = root.target("local-274", "vault");
    let kept = ProfileSettings {
        world: Some(321),
        ..ProfileSettings::default()
    };
    legacy_vault(
        target.path(),
        PASS,
        &[
            profile("alice", "pa", 274_000_100, kept.clone()),
            profile("bob", "pb", 274_000_200, ProfileSettings::default()),
        ],
    );
    root.unlock(&target, PASS).unwrap();
    let bob = root
        .state(&target)
        .rows
        .into_iter()
        .find(|row| row.username == "bob")
        .unwrap();
    // A restored older state backup without bob's row.
    root.service()
        .retire_profiles("local-274", vec![bob.id])
        .unwrap();
    let detail = match root.unlock(&target, PASS) {
        Err(JoinError::Refused {
            phase: JoinPhase::Join,
            detail,
        }) => detail,
        other => panic!("expected the missing-row refusal, got {:?}", other.err()),
    };
    assert!(
        detail.contains("state vault recover --defaults --profile local-274"),
        "{detail}"
    );

    let declined = recover_defaults(root.service(), &target, PASS, &|_| false).unwrap();
    assert!(declined.contains("not confirmed"));
    let asked = std::sync::Mutex::new(String::new());
    recover_defaults(root.service(), &target, PASS, &|question| {
        *asked.lock().unwrap() = question.to_string();
        true
    })
    .unwrap();
    assert!(asked.lock().unwrap().contains("1 credential(s)"));
    let vault = root.unlock(&target, PASS).unwrap();
    assert_eq!(
        vault.get("alice").unwrap().settings,
        kept,
        "surviving row kept"
    );
    let bob = vault.get("bob").unwrap();
    assert_eq!(bob.settings, ProfileSettings::default());
    assert_eq!(bob.uid, 274_000_101, "a fresh uid above the surviving rows");
}

/// No password and no passphrase reaches the database, the logs or the
/// audit lines across a whole session: conversion with a confirmed spill,
/// joined writes, a rename and a reset, run in a child whose output is
/// captured.
#[test]
fn no_secret_reaches_the_database_logs_or_audit() {
    let _lease = crate::test_process::state();
    let root = Root::new("s8-secrets");
    // The state lease is held, so the child is spawned directly (the
    // spawn helper takes the same lease).
    let output = std::process::Command::new(crate::test_process::executable())
        .args([
            "--ignored",
            "--exact",
            "profile_vault::tests::s8_secret_session_child",
            "--nocapture",
        ])
        .env("S8_ROOT", &root.root)
        .output()
        .unwrap();
    let text = [output.stdout.as_slice(), output.stderr.as_slice()].concat();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&text)
    );
    let log = String::from_utf8_lossy(&text);
    assert!(
        log.contains("audit domain=profiles"),
        "audit lines were written: {log}"
    );
    for secret in SECRET_SESSION_SECRETS {
        assert!(!contains(&text, secret), "{secret:?} reached the logs");
        assert!(
            !contains(&root.database_bytes(), secret),
            "{secret:?} reached the database"
        );
    }
}

const SECRET_SESSION_SECRETS: [&str; 4] = [
    "Pass-Phrase-S8",
    "alice-Secret-1",
    "bob-Secret-2",
    "renamed-Secret-3",
];

#[test]
#[ignore = "child process of no_secret_reaches_the_database_logs_or_audit"]
fn s8_secret_session_child() {
    let _drain = api::hostlog::stderr_drain();
    // Every lifecycle line, the audit lines included, reaches stderr.
    api::hostlog::set_debug(true);
    let root = PathBuf::from(std::env::var("S8_ROOT").unwrap());
    let service = LocalStateService::open_process(StateRoot::new(root.clone()));
    let target = VaultTarget::new("local-274", &root.join("vault")).unwrap();
    let [passphrase, alice_pw, bob_pw, renamed_pw] = SECRET_SESSION_SECRETS;
    let mut alice = ProfileSettings::default();
    alice.script_settings.insert(
        "catalog:Notes".into(),
        json!({"n": bob_pw}).as_object().unwrap().clone(),
    );
    legacy_vault(
        target.path(),
        passphrase,
        &[
            profile("alice", alice_pw, 1, alice),
            profile("bob", bob_pw, 2, ProfileSettings::default()),
        ],
    );
    let items = match unlock(
        &service,
        &target,
        passphrase,
        &UnlockDecisions::default(),
        true,
    ) {
        Err(JoinError::NeedsDecisions(items)) => items,
        other => panic!("{:?}", other.err()),
    };
    for item in &items {
        eprintln!("question: {item}");
    }
    let mut decisions = UnlockDecisions::default();
    for item in items {
        if let RepairItem::Spill { id, .. } = item {
            decisions.spill.insert(id, SpillAction::Clear);
        }
    }
    let vault = unlock(&service, &target, passphrase, &decisions, true).unwrap();
    let mut store = ProfileStore::joined(vault.store(), Arc::clone(&service));
    let mut renamed = vault.get("bob").unwrap().clone();
    renamed.username = "carol".into();
    renamed.password = renamed_pw.into();
    store.commit_rename("bob", &renamed).unwrap();
    let mut alice = vault.get("alice").unwrap().clone();
    alice.settings.world = Some(301);
    store.commit(&[VaultChange::Upsert(alice)]).unwrap();
    drop(store);
    reset(&service, &target).unwrap();
}
