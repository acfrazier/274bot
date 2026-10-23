//! Script coordination through the real host seam: a real
//! [`host_play::Play`] (no server contact), real script slots and the real
//! profile writer over a temp vault.

use std::num::NonZeroU64;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{
    parse_parameter_text, CardRuntime, CardSource, CatalogError, CompiledCardId, Notice,
    ParameterCommit, ParameterEditKey, SchemaRef, ScriptChoice, ScriptRunState,
    ScriptStartErrorKind, Scripts, SettingView,
};
use crate::command::{
    CardKind, CommandBook, CommandFailure, Completion, OperatorCommand, Pending, Stage, Subject,
};
use crate::marked::prepare_apply_settings_marked;
use crate::operations::{ActionKind, Outcome};
use crate::selection::{start_marked, stop_marked, MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;
use crate::surface::{HeadlessSurface, SlotAttach, SlotSurface, SpawnProfile};
use api::identity::{ProcessIncarnation, ProcessTime, RequestIdMinter, Scope, Settlement};
use api::process_condition::{
    ConditionCause, ConditionId, ProcessCapability, ProcessConditions, ProcessServiceCode,
    ProcessServiceCondition,
};
use host_play::{InstancePermit, Play, PlayOptions};
use serde_json::{json, Map, Value};

use vault::{Profile, ProfileSettings, Vault};
const LOOPING: &str = "export default class T extends LoopingBot { override loop() {} }\n";

/// Hang guard for every wait in this file, never a pass condition and never a
/// latency target. Each wait ends on the product's own settlement signal
/// (`starts_pending()`, `catalog_work_pending()`, the slot's run state): every
/// admitted job reaches a terminal and a Start refused as Full is resubmitted
/// on each poll. Card and catalogue preparation share one process-wide
/// background worker and one byte allowance (`script::work_admission`), so a
/// test waits behind the preparations of every other test in this binary; on a
/// loaded host that queue is far longer than one job. The guard only stops a
/// deadlocked test and is minutes, not the expected duration.
const HANG_GUARD: Duration = Duration::from_secs(600);

struct Fixture {
    core: OperatorSession<()>,
    scripts: Scripts,
    dir: PathBuf,
    _iso: script::IsolatedEnv,
}

fn empty_play() -> Play {
    host_play::run_with_io(
        &PlayOptions {
            host: "127.0.0.1".into(),
            transport: host_play::Transport::Tcp,
            port: 43594,
            cache_dir: "/tmp".into(),
            lowmem: true,
            mainland: false,
        },
        vec![],
        |_| (None, None),
        |_, _, _| {},
    )
}

/// Members loaded (arms attached, no worker threads) from a fresh vault.
fn fixture(test: &str, members: &[&str]) -> Fixture {
    let iso = script::IsolatedEnv::enter(&format!("frontend-core-scripts-{test}"));
    let dir = iso.dir.clone();
    let mut vault = Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    for (i, name) in members.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).into(),
                password: "pw".into(),
                uid: 1 + i as i32,
                settings: ProfileSettings::default(),
            })
            .unwrap();
    }
    let mut core = OperatorSession::new(InstancePermit::SkipLock);
    core.set_spawn_workers(false);
    core.start(vault, empty_play());
    let mut surface = HeadlessSurface::new();
    for name in members {
        core.load(name, &mut surface);
    }
    let scripts = Scripts::new(
        script::JsLibrary::with_cache(dir.join("js-scripts.json"), dir.join("js-cache")),
        script::ScriptSettingsStore::at(dir.join("script-settings.json")),
    );
    Fixture {
        core,
        scripts,
        dir,
        _iso: iso,
    }
}

struct ScriptSurface<'a>(&'a mut Scripts);

impl SlotSurface for ScriptSurface<'_> {
    type Io = ();

    fn attach(
        &mut self,
        name: &str,
        profile: &mut SpawnProfile<'_>,
        retained: Option<()>,
    ) -> SlotAttach<()> {
        HeadlessSurface::new().attach(name, profile, retained)
    }

    fn lifetime_reset(&mut self, _name: &str, _walk_was_armed: bool) {}
    fn released(&mut self, _name: &str) {}
    fn draw_changed(&mut self, _switch: crate::command::DrawSwitch, _on: bool) {}

    fn models(&mut self) -> crate::models::Models<'_> {
        crate::models::Models::none().with_scripts(self.0)
    }
}

impl Fixture {
    fn submit(&mut self, command: OperatorCommand, targets: crate::session::Targets<'_>) {
        self.core
            .submit(command, targets, &mut ScriptSurface(&mut self.scripts))
            .unwrap();
    }

    fn card(&mut self, file: &str, src: &str) -> script::JsCard {
        let path = self.dir.join(file);
        std::fs::write(&path, src).unwrap();
        self.scripts.js_mut().load(&path).unwrap()
    }

    fn assign(&mut self, name: &str, card: &script::JsCard) {
        assert!(self
            .scripts
            .persist_assignment(&mut self.core, name, card.assignment()));
        self.core.flush_writes();
    }

    /// Pump the core and the coordinator, as the front end does once per
    /// frame, until `done` holds. The pass condition is the product's own
    /// settlement signal; the only time bound is `HANG_GUARD`.
    fn pump_until(&mut self, what: &str, mut done: impl FnMut(&mut Self) -> bool) {
        let guard = Instant::now() + HANG_GUARD;
        while !done(self) {
            assert!(Instant::now() < guard, "{what}: hang guard expired");
            self.core.poll();
            self.scripts.poll(&mut self.core);
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Poll until no Start is pending (settled, failed or skipped).
    fn settle(&mut self) {
        self.pump_until("script Start did not settle", |f| {
            !f.scripts.starts_pending()
        });
    }

    fn settle_catalog_work(&mut self) {
        self.pump_until("catalog work did not settle", |f| {
            !f.scripts.catalog_work_pending()
        });
    }

    /// Poll until the slot reaches `want`. A slot that ends in `Error` when
    /// another state was wanted has failed: that fails the test now.
    fn wait_state(&mut self, name: &str, want: ScriptRunState) {
        self.pump_until(name, |f| {
            let state = f.state(name);
            assert!(
                state == want || state != ScriptRunState::Error,
                "{name} ended {state:?}, wanted {want:?}"
            );
            state == want
        });
    }

    fn state(&self, name: &str) -> ScriptRunState {
        ScriptRunState::from_script(self.core.play().unwrap().script_state(name))
    }

    fn start_running(&mut self, name: &str) {
        self.scripts
            .start_profile(&mut self.core, name, None)
            .unwrap();
        self.settle();
        self.wait_state(name, ScriptRunState::Running);
    }

    fn generation(&self, name: &str) -> u64 {
        self.core
            .play()
            .unwrap()
            .script_runtime_generation(name)
            .unwrap()
    }

    fn saved_bag(&self, name: &str, key: &str) -> Option<Map<String, Value>> {
        let disk = Vault::unlock(&self.dir.join("vault"), "test-passphrase-01").unwrap();
        disk.get(name)
            .unwrap()
            .settings
            .script_settings
            .get(key)
            .cloned()
    }

    fn set(&mut self, name: &str, card: &script::JsCard, id: &str, value: Value) {
        self.scripts
            .set_profile_setting(
                &mut self.core,
                name,
                CardSource::from_script(card.source),
                &card.name,
                &card.path,
                id,
                value,
            )
            .unwrap();
    }

    fn prepare(&mut self, source: &str, card: &script::JsCard) {
        self.scripts.prepare_settings_sync(
            &self.core,
            source,
            CardSource::from_script(card.source),
            &card.name,
            &card.path,
        );
    }

    /// Posting `bag` again is refused when the run already has it.
    fn run_has_bag(&self, name: &str, bag: &Map<String, Value>) -> bool {
        let play = self.core.play().unwrap();
        let identity = play.script_source_identity(name).unwrap();
        if let Some(run) = play.script_native_run(name) {
            let id = script::compiled_id(identity.strip_prefix("compiled:").unwrap()).unwrap();
            let revision = play.script_native_settings_revision(name).unwrap() + 1;
            let worker = play
                .script_prepare_config(id, revision, std::sync::Arc::new(bag.clone()))
                .unwrap();
            let work_identity = worker.identity();
            let config = worker
                .join()
                .unwrap()
                .unwrap()
                .commit(work_identity)
                .unwrap();
            return play.script_configure_compiled(name, config, run)
                == script::CompiledDelivery::Unchanged;
        }
        !play.script_post_settings_fenced(name, bag, &identity, self.generation(name))
    }
}

#[test]
fn newer_native_edit_keeps_the_copy_drafts_assignment() {
    let mut f = native_fixture("copy-draft-assignment", &["alice", "bob"]);
    let id = script::CompiledId("Gatherer");
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", script::compiled_assignment(id)));
    f.core.flush_writes();
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "targetPreference",
            json!("Nearest"),
        )
        .unwrap();
    f.core.flush_writes();
    prepare_apply_settings_marked(
        &mark_uids(&[2]),
        &f.core,
        &mut f.scripts,
        "alice",
        &ScriptChoice::from_script(script::ScriptSel::Compiled(id)),
    )
    .unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    // The editor composes over the accepted copy draft before preparation
    // settlement. Its newer same-card preparation supersedes the first one.
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "bob",
            CompiledCardId::from_script(id),
            "targetPreference",
            json!("Nearest"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let disk = Vault::unlock(&f.dir.join("vault"), "test-passphrase-01").unwrap();
    assert_eq!(
        disk.get("bob").unwrap().settings.script_assignment,
        Some(script::compiled_assignment(id))
    );
    let want = f
        .scripts
        .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
        .unwrap();
    assert_eq!(want["targetPreference"], "Nearest");
    f.start_running("bob");
    assert!(
        f.run_has_bag("bob", &want),
        "the composed draft's settings reach the assigned run"
    );
}

#[test]
fn parameter_text_is_deferred_until_commit_and_unchanged_is_a_noop() {
    let mut f = fixture("parameter-text-commit", &["alice"]);
    let card = f.card(
        "counter.ts",
        "export const SETTINGS = { count: { type: 'number', default: 1 } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("alice", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let key = ParameterEditKey::new(Some("alice"), &selection, "count");
    let def = card
        .settings_schema
        .iter()
        .find(|setting| setting.id == "count")
        .unwrap();
    let current = json!(1);
    f.scripts.merged_profile_bag(
        &mut f.core,
        "alice",
        CardSource::from_script(card.source),
        &card.name,
        &card.path,
        SchemaRef::new(&card.settings_schema),
    );
    f.core.flush_writes();
    let before = f.core.last_operation().map(|report| report.id());

    for text in ["1", "12", "123"] {
        let buffer = f.scripts.parameter_text_mut_for(
            Some("alice"),
            &selection,
            "count",
            "1",
            Some(current.clone()),
        );
        buffer.clear();
        buffer.push_str(text);
        assert_eq!(
            f.scripts.validate_parameter_buffer_for(
                Some("alice"),
                &selection,
                "count",
                ("1", Some(current.clone())),
                SettingView::new(def),
                &[],
            ),
            Ok(json!(text.parse::<u64>().unwrap()))
        );
        assert_eq!(f.core.last_operation().map(|report| report.id()), before);
    }

    let buffer = f.scripts.parameter_text_mut_for(
        Some("alice"),
        &selection,
        "count",
        "1",
        Some(current.clone()),
    );
    buffer.clear();
    buffer.push('-');
    assert!(f
        .scripts
        .validate_parameter_buffer_for(
            Some("alice"),
            &selection,
            "count",
            ("1", Some(current.clone())),
            SettingView::new(def),
            &[],
        )
        .is_err());
    assert!(f
        .scripts
        .parameter_text_error_for(Some("alice"), &selection, "count")
        .is_some());
    assert_eq!(
        f.core.last_operation().map(|report| report.id()),
        before,
        "an invalid partial number never creates a save operation"
    );
    let buffer = f.scripts.parameter_text_mut_for(
        Some("alice"),
        &selection,
        "count",
        "1",
        Some(current.clone()),
    );
    buffer.clear();
    buffer.push_str("123");
    assert_eq!(
        f.scripts.validate_parameter_buffer_for(
            Some("alice"),
            &selection,
            "count",
            ("1", Some(current.clone())),
            SettingView::new(def),
            &[],
        ),
        Ok(json!(123))
    );

    let start_error = f
        .scripts
        .start_profile(&mut f.core, "alice", None)
        .unwrap_err();
    match (start_error.kind(), start_error.to_string()) {
        (ScriptStartErrorKind::Waiting, message) => {
            assert!(message.contains("uncommitted"), "{message}")
        }
        other => panic!("expected a waiting start error, got {other:?}"),
    }
    assert_eq!(f.core.last_operation().map(|report| report.id()), before);
    assert!(
        f.scripts.parameter_edit_blocks_start("alice"),
        "a valid but uncommitted draft must hold Start"
    );
    let drafts: Vec<_> = f.scripts.parameter_draft_details("alice").collect();
    assert_eq!(
        drafts,
        vec![(card.name.as_str(), "count", "valid unsaved value")]
    );

    let value = parse_parameter_text(SettingView::new(def), "123", &[]).unwrap();
    let op = match f
        .scripts
        .commit_parameter_value(
            &mut f.core,
            &key,
            &selection,
            "count",
            value.clone(),
            Some(current),
        )
        .unwrap()
    {
        ParameterCommit::Submitted(op) => op,
        other => panic!("expected one queued profile save, got {other:?}"),
    };
    assert_eq!(
        f.core.last_operation().map(|report| report.id()),
        Some(op),
        "the explicit commit adds exactly one operation"
    );
    assert_eq!(
        f.core.operation(op).unwrap().action(),
        ActionKind::SaveProfile
    );
    assert_eq!(
        f.scripts
            .commit_parameter_value(
                &mut f.core,
                &key,
                &selection,
                "count",
                value.clone(),
                Some(json!(1)),
            )
            .unwrap(),
        ParameterCommit::Pending(op),
        "the same in-flight value must reuse its operation"
    );
    assert_eq!(f.core.last_operation().map(|report| report.id()), Some(op));

    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.core.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
    assert_eq!(
        f.scripts
            .commit_parameter_value(
                &mut f.core,
                &key,
                &selection,
                "count",
                value.clone(),
                Some(value),
            )
            .unwrap(),
        ParameterCommit::Unchanged
    );
    assert_eq!(f.core.last_operation().map(|report| report.id()), Some(op));
}

#[test]
fn failed_parameter_save_releases_start_and_allows_the_same_value_to_retry() {
    let mut f = fixture("parameter-save-retry", &["alice"]);
    let card = f.card(
        "cook-settings.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range' } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("alice", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let key = ParameterEditKey::new(Some("alice"), &selection, "surface");
    let value = json!("Fire");
    let vault_file = f.dir.join("vault");
    let aside = f.dir.join("vault-aside");
    std::fs::rename(&vault_file, &aside).unwrap();
    std::fs::create_dir(&vault_file).unwrap();
    let first = f
        .scripts
        .commit_parameter_value(
            &mut f.core,
            &key,
            &selection,
            "surface",
            value.clone(),
            Some(json!("Range")),
        )
        .unwrap();
    assert!(matches!(first, ParameterCommit::Submitted(_)));
    assert!(f.scripts.parameter_edit_blocks_start("alice"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(f.scripts.parameter_text_error(&key).is_some());
    assert!(f
        .scripts
        .parameter_text_error_for(Some("alice"), &selection, "surface")
        .is_some());
    assert!(
        !f.scripts.parameter_edit_blocks_start("alice"),
        "a failed save must keep the durable value and release Start"
    );
    std::fs::remove_dir(&vault_file).unwrap();
    std::fs::rename(&aside, &vault_file).unwrap();
    let retry = f
        .scripts
        .commit_parameter_value(
            &mut f.core,
            &key,
            &selection,
            "surface",
            value.clone(),
            Some(json!("Range")),
        )
        .expect("the same deliberate edit can retry after the save error");
    assert!(matches!(retry, ParameterCommit::Submitted(_)));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
    let saved = f.saved_bag("alice", &card.identity_key()).unwrap();
    assert_eq!(saved["surface"], value);
    f.start_running("alice");
}

#[test]
fn typed_parameter_save_failure_restores_saved_text_and_exposes_error() {
    let mut f = fixture("typed-parameter-save-failure", &["alice"]);
    let card = f.card(
        "cook-settings.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range', options: ['Range', 'Fire'] } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("alice", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    f.set("alice", &card, "surface", json!("Fire"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let vault_file = f.dir.join("vault");
    let aside = f.dir.join("vault-aside");
    std::fs::rename(&vault_file, &aside).unwrap();
    std::fs::create_dir(&vault_file).unwrap();
    f.scripts
        .stage_setting(Some("alice"), &selection, "surface", json!("Range"));
    f.submit(
        OperatorCommand::SaveScriptSettings(CardKind::Script),
        crate::session::Targets::Profile("alice"),
    );
    assert!(f.scripts.parameter_edit_blocks_start("alice"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    std::fs::remove_dir(&vault_file).unwrap();
    std::fs::rename(&aside, &vault_file).unwrap();

    assert_eq!(
        f.scripts
            .parameter_text_mut_for(
                Some("alice"),
                &selection,
                "surface",
                "Fire",
                Some(json!("Fire")),
            )
            .as_str(),
        "Fire"
    );
    assert!(f
        .scripts
        .parameter_text_error_for(Some("alice"), &selection, "surface")
        .is_some());
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
}

#[test]
fn cancelled_parameter_save_restores_durable_text_and_releases_start() {
    let mut f = fixture("parameter-save-cancelled", &["alice"]);
    let card = f.card(
        "cook-settings.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range' } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("alice", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let key = ParameterEditKey::new(Some("alice"), &selection, "surface");
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    let op = match f
        .scripts
        .commit_parameter_value(
            &mut f.core,
            &key,
            &selection,
            "surface",
            json!("Fire"),
            Some(json!("Range")),
        )
        .unwrap()
    {
        ParameterCommit::Submitted(op) => op,
        other => panic!("expected an accepted settings write, got {other:?}"),
    };
    assert!(f.scripts.parameter_edit_blocks_start("alice"));

    // A later write of the same field replaces the accepted edit in this
    // commit. Settle the actual writer outcome, not an injected test result.
    f.set("alice", &card, "surface", json!("Range"));
    drop(held);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.core.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Cancelled)
    );

    assert_eq!(
        f.scripts
            .parameter_text_mut(&key, "Range", Some(json!("Range")))
            .as_str(),
        "Range"
    );
    assert_eq!(
        f.scripts
            .parameter_text_error_for(Some("alice"), &selection, "surface"),
        Some("settings save was cancelled")
    );
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
    assert!(f.scripts.parameter_draft_details("alice").next().is_none());
    assert_eq!(
        f.saved_bag("alice", &card.identity_key()).unwrap()["surface"],
        json!("Range")
    );
    f.start_running("alice");
}

#[test]
fn typed_parameter_commands_hold_start_until_durable_and_log_each_saved_change() {
    let mut f = fixture("parameter-typed-command", &["prefs-script-save"]);
    let card = f.card(
        "cook-settings.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range', options: ['Range', 'Fire'] } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("prefs-script-save", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    for value in [json!("Fire"), json!("Range")] {
        f.scripts.stage_setting(
            Some("prefs-script-save"),
            &selection,
            "surface",
            value.clone(),
        );
        f.submit(
            OperatorCommand::SaveScriptSettings(CardKind::Script),
            crate::session::Targets::Profile("prefs-script-save"),
        );
        assert!(f.scripts.parameter_edit_blocks_start("prefs-script-save"));
        assert_eq!(
            f.scripts
                .start_profile(&mut f.core, "prefs-script-save", None)
                .unwrap_err()
                .kind(),
            ScriptStartErrorKind::Waiting
        );
        let op = f.core.last_operation().unwrap().id();
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);
        assert!(!f.scripts.parameter_edit_blocks_start("prefs-script-save"));
        assert_eq!(
            f.saved_bag("prefs-script-save", &card.identity_key())
                .unwrap()["surface"],
            value
        );
        let mut log = crate::log::LogView::new(crate::log::LogScope::slot("prefs-script-save"));
        crate::log::global().refresh(&mut log);
        let old = if value == json!("Fire") {
            "\"Range\""
        } else {
            "\"Fire\""
        };
        let expected = format!(
            "saved script settings; profile prefs-script-save; card {}; action parameter edit; field surface; old {old}; new {value}; op#{}",
            card.name, op.0
        );
        assert!(log
            .rows()
            .iter()
            .any(|row| { row.level() == api::hostlog::Level::Info && row.message() == expected }));
    }
    f.start_running("prefs-script-save");
    let mut credential = f.core.durable_profile("prefs-script-save").unwrap().clone();
    let live = f
        .scripts
        .profile_save_live(&f.core, "prefs-script-save", &credential.settings)
        .expect("the running script has live settings");
    credential.password = "updated-password".into();
    let op = f
        .core
        .save_profile(
            credential,
            crate::session::ArmMirror::remember(Some(live)),
            "password",
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let mut log = crate::log::LogView::new(crate::log::LogScope::slot("prefs-script-save"));
    crate::log::global().refresh(&mut log);
    assert!(!log.rows().iter().any(|row| {
        row.message().contains("saved script settings")
            && row.message().contains(&format!("op#{}", op.0))
    }));
}

#[test]
fn legacy_parameter_save_logs_card_field_and_both_values() {
    let mut f = fixture("legacy-parameter-log", &["alice"]);
    let card = f.card(
        "cook-settings.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range' } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let key = ParameterEditKey::new(None, &selection, "surface");
    let current = f
        .scripts
        .durable_setting_value(&f.core, None, &selection, "surface")
        .unwrap();
    assert_eq!(
        f.scripts
            .commit_parameter_value(
                &mut f.core,
                &key,
                &selection,
                "surface",
                json!("Fire"),
                Some(current)
            )
            .unwrap(),
        ParameterCommit::Saved
    );
    let op = f.core.last_operation().unwrap().id();
    let expected = format!(
        "saved script settings; profile legacy; card {}; action parameter edit; field surface; old \"Range\"; new \"Fire\"; op#{}",
        card.name, op.0
    );
    let mut log = crate::log::LogView::new(crate::log::LogScope::Process);
    crate::log::global().refresh(&mut log);
    assert!(log
        .rows()
        .iter()
        .any(|row| { row.level() == api::hostlog::Level::Info && row.message() == expected }));
}

#[test]
fn discard_recovers_hidden_card_drafts_without_cancelling_an_accepted_save() {
    let mut f = fixture("parameter-hidden-discard", &["alice", "bob"]);
    let card = f.card(
        "count.ts",
        "export const SETTINGS = { count: { type: 'int', default: '1' } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    f.assign("alice", &card);
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let def = SettingView::new(&card.settings_schema[0]);
    for profile in ["alice", "bob"] {
        let key = ParameterEditKey::new(Some(profile), &selection, "count");
        *f.scripts.parameter_text_mut(&key, "1", Some(json!(1))) = "-".into();
        assert!(f
            .scripts
            .validate_parameter_buffer(&key, "1", Some(json!(1)), def, &[])
            .is_err());
        let drafts: Vec<_> = f.scripts.parameter_draft_details(profile).collect();
        assert_eq!(drafts.len(), 1);
        assert_eq!(drafts[0].0, card.name.as_str());
        assert_eq!(drafts[0].1, "count");
        assert_eq!(drafts[0].2, f.scripts.parameter_text_error(&key).unwrap());
    }
    assert!(f.scripts.has_parameter_draft("alice"));
    f.scripts.stage_profile_parameter_discard("alice");
    f.submit(
        OperatorCommand::DiscardParameterText,
        crate::session::Targets::Application,
    );
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
    assert!(f.scripts.parameter_edit_blocks_start("bob"));
    let key = ParameterEditKey::new(Some("alice"), &selection, "count");
    f.scripts
        .commit_parameter_value(
            &mut f.core,
            &key,
            &selection,
            "count",
            json!(2),
            Some(json!(1)),
        )
        .unwrap();
    assert!(f.scripts.parameter_draft_details("alice").next().is_none());
    f.scripts.stage_profile_parameter_discard("alice");
    f.submit(
        OperatorCommand::DiscardParameterText,
        crate::session::Targets::Application,
    );
    assert!(f.scripts.parameter_edit_blocks_start("alice"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(!f.scripts.parameter_edit_blocks_start("alice"));
    assert_eq!(
        f.saved_bag("alice", &card.identity_key()).unwrap()["count"],
        json!(2)
    );
    f.start_running("alice");
}

#[test]
fn unknown_resource_save_survives_and_radius_remains_editable() {
    let mut f = native_fixture("unknown-resource-radius", &["alice"]);
    let id = script::CompiledId("Gatherer");
    let selection = ScriptChoice::from_script(script::ScriptSel::Compiled(id));
    let setup = f
        .scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "skill",
            json!("Mining"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.core.operation(setup).unwrap().outcome("alice"),
        Some(&Outcome::Completed)
    );
    let bag = f
        .scripts
        .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
        .unwrap();
    let current = bag.get("miningResources").cloned().unwrap();
    let schema = (script::compiled_card(id).unwrap().schema)();
    let def = schema
        .iter()
        .find(|setting| setting.id == "miningResources")
        .unwrap();
    let key = ParameterEditKey::new(Some("alice"), &selection, "miningResources");
    let initial_text = script::format_setting_value(&current);
    let invalid_text = "coppe";
    let buffer = f.scripts.parameter_text_mut_for(
        Some("alice"),
        &selection,
        "miningResources",
        &initial_text,
        Some(current.clone()),
    );
    buffer.clear();
    buffer.push_str(invalid_text);
    assert_eq!(
        f.scripts
            .validate_parameter_buffer_for(
                Some("alice"),
                &selection,
                "miningResources",
                (&initial_text, Some(current.clone())),
                SettingView::new(def),
                &[],
            )
            .unwrap(),
        json!(["coppe"]),
        "the typo is syntactically valid but not a known Mining resource"
    );
    let before = f.core.last_operation().map(|report| report.id());
    let result = f.scripts.commit_parameter_value(
        &mut f.core,
        &key,
        &selection,
        "miningResources",
        json!(["coppe"]),
        Some(current),
    );
    let op = match result {
        Ok(ParameterCommit::Submitted(op) | ParameterCommit::Pending(op)) => op,
        other => panic!("expected one accepted Save profile attempt, got {other:?}"),
    };
    assert_ne!(Some(op), before);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let report = f.core.operation(op).unwrap();
    assert_eq!(report.action(), ActionKind::SaveProfile);
    assert_eq!(
        report.outcome("alice"),
        Some(&Outcome::Completed),
        "an unknown Gatherer resource remains a persistable value: {report:?}"
    );
    let saved = f
        .scripts
        .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
        .unwrap();
    assert_eq!(saved.get("miningResources"), Some(&json!(["coppe"])));

    for _ in 0..4 {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert!(matches!(
            f.scripts.commit_parameter_value(
                &mut f.core,
                &key,
                &selection,
                "miningResources",
                json!(["coppe"]),
                Some(json!(["coppe"])),
            ),
            Ok(ParameterCommit::Unchanged)
        ));
        assert_eq!(f.core.last_operation().map(|report| report.id()), Some(op));
    }

    let current_bag = f
        .scripts
        .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
        .unwrap();
    let current_radius = current_bag.get("radius").cloned().unwrap();
    let radius_def = schema
        .iter()
        .find(|setting| setting.id == "radius")
        .unwrap();
    let radius_text = "13";
    let initial_radius_text = script::format_setting_value(&current_radius);
    let radius_buffer = f.scripts.parameter_text_mut_for(
        Some("alice"),
        &selection,
        "radius",
        &initial_radius_text,
        Some(current_radius.clone()),
    );
    radius_buffer.clear();
    radius_buffer.push_str(radius_text);
    let radius_value = f
        .scripts
        .validate_parameter_buffer_for(
            Some("alice"),
            &selection,
            "radius",
            (&initial_radius_text, Some(current_radius.clone())),
            SettingView::new(radius_def),
            &[],
        )
        .unwrap();
    assert_eq!(radius_value, json!(13));
    let radius_key = ParameterEditKey::new(Some("alice"), &selection, "radius");
    let radius_result = f.scripts.commit_parameter_value(
        &mut f.core,
        &radius_key,
        &selection,
        "radius",
        radius_value,
        Some(current_radius),
    );
    let radius_op = match radius_result {
        Ok(ParameterCommit::Submitted(op) | ParameterCommit::Pending(op)) => op,
        other => panic!("expected radius Save profile attempt, got {other:?}"),
    };
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let radius_report = f.core.operation(radius_op).unwrap();
    assert_eq!(radius_report.action(), ActionKind::SaveProfile);
    assert_eq!(radius_report.outcome("alice"), Some(&Outcome::Completed));
    let saved = f
        .scripts
        .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
        .unwrap();
    assert_eq!(saved.get("miningResources"), Some(&json!(["coppe"])));
    assert_eq!(saved.get("radius"), Some(&json!(13)));
}
fn bag(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
}

#[test]
fn browse_card_list_includes_compiled_cards() {
    let f = fixture("compiled-card-list", &["alice"]);
    let cards: Vec<_> = f.scripts.browse_cards().collect();
    if script::compiled_id("Sherlock").is_some() {
        let sherlock = cards
            .iter()
            .find(|card| card.name() == "Sherlock")
            .expect("compiled Sherlock is available to Browse when registered");
        assert_eq!(sherlock.kind(), CardRuntime::Compiled);
        assert_eq!(sherlock.category(), "Treasure Trails");
        assert_eq!(
            sherlock.selection(),
            ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Sherlock")))
        );
    } else {
        assert!(
            cards.iter().all(|card| card.name() != "Sherlock"),
            "Browse omits Sherlock when the load feature omits its registration"
        );
    }
    let grinch = cards
        .iter()
        .find(|card| card.name() == "Grinch")
        .expect("Grinch is available in Browse without a running instance");
    assert_eq!(grinch.kind(), CardRuntime::Compiled);
    assert_eq!(grinch.category(), "Gathering");
    assert_eq!(grinch.source(), super::CardSource::Builtin);
    assert_eq!(
        grinch.compiled_id(),
        Some(CompiledCardId::from_script(script::CompiledId("Grinch")))
    );
    assert_eq!(
        grinch.selection(),
        ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Grinch")))
    );
}

fn prepare_grinch(
    f: &Fixture,
    bag: Map<String, Value>,
) -> Result<std::sync::Arc<script::native::PreparedConfig>, script::native::StartError> {
    let id = script::CompiledId("Grinch");
    let worker = f
        .core
        .play()
        .unwrap()
        .script_prepare_config(id, 1, std::sync::Arc::new(bag))
        .expect("Grinch settings preparation is admitted");
    let identity = worker.identity();
    worker
        .join()
        .expect("Grinch settings worker does not panic")?
        .commit(identity)
}

#[test]
fn grinch_defaults_and_radius_use_the_real_compiled_schema_and_preparer() {
    let f = native_fixture("grinch-defaults", &["alice"]);
    let id = script::CompiledId("Grinch");
    let compiled_id = CompiledCardId::from_script(id);
    let super::SchemaView::Ready { version, fields } =
        f.scripts
            .compiled_schema(&f.core, Some("alice"), compiled_id)
    else {
        panic!("registered Grinch schema must be available");
    };
    assert_eq!(version, 1);

    let editor_bag = f
        .scripts
        .compiled_edit_bag(&f.core, Some("alice"), compiled_id)
        .unwrap();
    let expected_ids = [
        "1961", "981", "1989", "1959", "1053", "1055", "1057", "1050", "962", "1038", "1040",
        "1042", "1044", "1046", "1048", "1419", "1037", "4079",
    ];
    let selected: Vec<_> = editor_bag["items"]
        .as_array()
        .expect("items editor value is a JSON array")
        .iter()
        .map(|value| value.as_str().expect("item IDs are decimal strings"))
        .collect();
    assert_eq!(selected, expected_ids);
    assert_eq!(editor_bag["radius"].as_u64(), Some(12));
    assert_eq!(editor_bag["bank"], "Nearest");
    for permission in ["allow_wilderness", "allow_danger_zones", "allow_teleports"] {
        assert_eq!(editor_bag[permission], false, "{permission} defaults off");
    }

    let items = fields.get("items").expect("items editor field");
    let labels = items.option_labels();
    for (id, colour) in [("1053", "green"), ("1055", "blue"), ("1057", "red")] {
        assert!(
            labels
                .iter()
                .any(|label| { label.contains(id) && label.to_ascii_lowercase().contains(colour) }),
            "mask option label names {colour} and ID {id}: {labels:?}"
        );
    }
    for id in expected_ids {
        assert!(
            labels.iter().any(|label| label.contains(id)),
            "item option label contains ID {id}: {labels:?}"
        );
    }
    let bank = fields.get("bank").expect("bank editor field");
    assert_eq!(bank.options_from(), Some("named-banks"));
    let radius = fields.get("radius").expect("radius editor field");
    let (min, max, _) = radius.bounds();
    assert_eq!((min, max), (Some("1"), Some("16")));

    let prepared = prepare_grinch(&f, editor_bag.clone()).expect("default settings prepare");
    let config = prepared
        .get::<std::sync::Arc<script::grinch::Prepared>>()
        .expect("Grinch preparer returns typed settings");
    assert_eq!(
        config.ids.as_ref(),
        &expected_ids.map(|id| id.parse::<i32>().unwrap())
    );
    assert_eq!(config.radius, 12);
    assert_eq!(
        config.bank, None,
        "Nearest is represented by no explicit bank"
    );

    for radius in [1, 16] {
        let mut values = editor_bag.clone();
        values.insert("radius".into(), json!(radius));
        let prepared = prepare_grinch(&f, values).expect("inclusive radius bound prepares");
        let config = prepared
            .get::<std::sync::Arc<script::grinch::Prepared>>()
            .expect("Grinch preparer returns typed settings");
        assert_eq!(config.radius, radius);
    }
    for radius in [0, 17] {
        let mut values = editor_bag.clone();
        values.insert("radius".into(), json!(radius));
        assert!(
            prepare_grinch(&f, values).is_err(),
            "radius {radius} is outside the accepted 1..=16 range"
        );
    }
}

#[test]
fn grinch_mask_ids_survive_the_shared_panel_tui_settings_editor_path() {
    let mut f = native_fixture("grinch-mask-settings", &["alice"]);
    let id = script::CompiledId("Grinch");
    let compiled_id = CompiledCardId::from_script(id);
    f.scripts
        .start_sel(
            &mut f.core,
            "alice",
            &script::ScriptSel::Compiled(id),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.settle();
    f.wait_state("alice", ScriptRunState::Running);

    let mut values = f
        .scripts
        .compiled_edit_bag(&f.core, Some("alice"), compiled_id)
        .unwrap();
    let masks = vec![json!("1053"), json!("1055"), json!("1057")];
    values.insert("items".into(), Value::Array(masks.clone()));

    let operation = f
        .scripts
        .set_compiled_overrides(&mut f.core, "alice", compiled_id, values)
        .unwrap();
    f.core.flush_writes();
    assert_eq!(
        f.core.operation(operation).unwrap().outcome("alice"),
        Some(&Outcome::Completed),
        "the shared native settings editor write prepares and persists"
    );
    let writes = f.core.take_settings_writes();
    assert!(matches!(
        writes.as_slice(),
        [write]
            if write.op() == operation
                && write.result().delivery() == Some(&super::LiveDelivery::RestartRequired)
    ));
    let saved = f
        .saved_bag("alice", &script::compiled_identity_key(id))
        .expect("Grinch settings saved in the profile vault");
    let (_, saved_values) = vault::CompiledSettingsRecord::view(&saved).unwrap();
    assert_eq!(saved_values["items"], Value::Array(masks));
    let reloaded = f
        .scripts
        .compiled_bag(&f.core, "alice", compiled_id)
        .unwrap();
    assert_eq!(reloaded["items"], saved_values["items"]);
    let prepared = prepare_grinch(&f, reloaded).expect("reloaded editor settings prepare");
    let config = prepared
        .get::<std::sync::Arc<script::grinch::Prepared>>()
        .expect("Grinch preparer returns typed settings");
    assert_eq!(config.ids.as_ref(), &[1053, 1055, 1057]);
}

#[test]
fn merged_profile_bag_carries_profile_global_clue_partner() {
    let mut f = fixture("clue-global", &["alice"]);
    let mut profile = f.core.vault().unwrap().get("alice").unwrap().clone();
    profile.settings.clue_duel_partner = "Helper".into();
    f.core
        .save_profile(profile, crate::ArmMirror::remember(None), "clue-global")
        .unwrap();
    f.core.flush_writes();
    let card = f.card("clue.ts", LOOPING);

    let merged = f.scripts.merged_profile_bag(
        &mut f.core,
        "alice",
        CardSource::from_script(card.source),
        &card.name,
        &card.path,
        SchemaRef::new(&card.settings_schema),
    );
    assert_eq!(merged.get("clueDuelPartner"), Some(&json!("Helper")));

    f.scripts.inject = Some(bag(&[("clueDuelPartner", json!("Harness"))]));
    let injected = f.scripts.merged_profile_bag(
        &mut f.core,
        "alice",
        CardSource::from_script(card.source),
        &card.name,
        &card.path,
        SchemaRef::new(&card.settings_schema),
    );
    assert_eq!(
        injected.get("clueDuelPartner"),
        Some(&json!("Harness")),
        "scenario inject keeps final precedence"
    );
}

#[test]
fn apply_to_all_reaches_same_card_members_and_skips_other_cards() {
    let mut f = fixture("sync", &["alice", "bob", "carol", "dave"]);
    let thiever = f.card("thiever.ts", LOOPING);
    let miner = f.card("miner.ts", LOOPING);
    for name in ["alice", "bob", "dave"] {
        f.assign(name, &thiever);
    }
    f.assign("carol", &miner);
    f.start_running("bob");
    f.start_running("carol");
    let key = thiever.identity_key();
    f.set("carol", &miner, "target", json!("Man"));
    f.set("alice", &thiever, "target", json!("Guard"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    f.prepare("alice", &thiever);
    let scope = f.scripts.prepared_settings_sync().unwrap();
    assert_eq!(scope.targets, ["bob", "dave"]);
    assert_eq!(scope.skipped.len(), 1, "carol runs another card");
    let op = f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let want = bag(&[("target", json!("Guard"))]);
    assert_eq!(f.saved_bag("bob", &key), Some(want.clone()));
    assert_eq!(f.saved_bag("dave", &key), Some(want.clone()));
    assert_eq!(f.saved_bag("carol", &key), None, "other card untouched");
    assert_eq!(
        f.saved_bag("carol", &miner.identity_key()),
        Some(bag(&[("target", json!("Man"))]))
    );
    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(
        (report.saved, report.failed.len(), report.skipped().len()),
        (2, 0, 1)
    );
    assert_eq!((report.delivered, report.not_running), (1, 1));
    let operation = f.core.operation(op).unwrap();
    assert!(matches!(
        operation.outcome("carol"),
        Some(o) if o.is_skipped()
    ));
    assert_eq!(operation.outcome("bob"), Some(&Outcome::Completed));
    assert_eq!(operation.outcome("dave"), Some(&Outcome::Completed));
    assert!(f.run_has_bag("bob", &want), "bob's run received the bag");
    assert!(
        !f.run_has_bag("carol", &want),
        "carol's run of another card was never posted"
    );
    let summary = report.summary().to_string();
    assert_eq!(f.scripts.take_notice(), Some(Notice::show(summary)));
}
#[test]
fn apply_to_all_logs_each_profile_settings_write() {
    let mut f = fixture("sync-profile-settings-log", &["alice", "bob", "dave"]);
    let card = f.card(
        "settings-sync-log.ts",
        "export const SETTINGS = { surface: { type: 'string', default: 'Range' } };\nexport default class T extends LoopingBot { override loop() {} }\n",
    );
    for profile in ["alice", "bob", "dave"] {
        f.assign(profile, &card);
    }
    f.set("alice", &card, "surface", json!("Fire"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    f.prepare("alice", &card);
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    for profile in ["bob", "dave"] {
        let mut log = crate::log::LogView::new(crate::log::LogScope::slot(profile));
        crate::log::global().refresh(&mut log);
        let prefix = format!(
            "saved script settings; profile {profile}; card {}; action apply-to-all; field surface; old \"Range\"; new \"Fire\"; op#",
            card.name
        );
        let records: Vec<_> = log
            .rows()
            .iter()
            .filter(|row| {
                row.level() == api::hostlog::Level::Info && row.message().starts_with(&prefix)
            })
            .collect();
        assert_eq!(records.len(), 1, "{profile}: {:?}", log.rows());
        assert!(
            records[0]
                .message()
                .strip_prefix(&prefix)
                .is_some_and(|operation| operation.parse::<u64>().is_ok()),
            "{profile}: {}",
            records[0].message()
        );
    }
}

/// Wall-wide Apply to all copies same-card members only: unassigned members
/// are skipped and not assigned.
#[test]
fn apply_to_all_skips_unassigned_members() {
    let mut f = fixture("sync-unassigned-skip", &["alice", "bob", "carol"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.set("alice", &thiever, "target", json!("Knight of Ardougne"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    f.prepare("alice", &thiever);
    let scope = f.scripts.prepared_settings_sync().unwrap();
    assert!(
        scope.targets.is_empty(),
        "unassigned members are not wall-wide targets: {:?}",
        scope.targets
    );
    assert_eq!(
        scope.skipped,
        [
            ("bob".to_string(), "no assignment".to_string()),
            ("carol".to_string(), "no assignment".to_string()),
        ]
    );
    assert!(
        !scope.prompt().to_ascii_lowercase().contains("assign"),
        "{}",
        scope.prompt()
    );
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(Scripts::assignment(&f.core, "bob").is_none());
    assert!(Scripts::assignment(&f.core, "carol").is_none());
    assert_eq!(f.saved_bag("bob", &thiever.identity_key()), None);
}

/// Focused Start and per-profile Start refuse while a copy confirm is waiting,
/// the same as Fleet Start / Start all.
#[test]
fn focused_and_profile_start_wait_for_pending_copy() {
    let mut f = fixture("sync-start-pending", &["alice", "bob"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.set("alice", &thiever, "target", json!("Knight of Ardougne"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.prepare("alice", &thiever);

    let err = f
        .scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap_err();
    match (err.kind(), err.to_string()) {
        (ScriptStartErrorKind::Waiting, message) => assert!(
            message.contains("blocked") && message.contains("waiting for Apply or Cancel"),
            "{message}"
        ),
        other => panic!("expected a waiting start error, got {other:?}"),
    }
    let err = f
        .scripts
        .start_profile(&mut f.core, "bob", None)
        .unwrap_err();
    match (err.kind(), err.to_string()) {
        (ScriptStartErrorKind::Waiting, message) => assert!(
            message.contains("blocked") && message.contains("waiting for Apply or Cancel"),
            "{message}"
        ),
        other => panic!("expected a waiting start error, got {other:?}"),
    }
    assert_eq!(f.state("bob"), ScriptRunState::Idle);

    f.scripts.cancel_settings_sync();
    f.scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap();
    f.settle();
    f.wait_state("bob", ScriptRunState::Running);
}

#[test]
fn queued_start_waits_for_copy_confirmation() {
    for apply in [false, true] {
        let mut f = fixture(&format!("queued-copy-{apply}"), &["alice", "bob", "carol"]);
        let card = f.card("thiever.ts", "export const SETTINGS = { target: { type: 'string', default: 'Guard' } };\nexport default class T extends LoopingBot { override loop() {} }\n");
        for name in ["alice", "bob", "carol"] {
            f.assign(name, &card);
        }
        f.set("alice", &card, "target", json!("Knight of Ardougne"));
        f.core.flush_writes();
        f.scripts.start_all(&mut f.core, None);
        let place = f.scripts.start_queue_place("bob").expect("Bob queued");
        assert_eq!(f.state("bob"), ScriptRunState::Idle);
        f.prepare("alice", &card);
        for _ in 0..3 {
            f.core.poll();
            f.scripts.poll(&mut f.core);
            assert_eq!(f.state("bob"), ScriptRunState::Idle);
            assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
            assert_eq!(f.state("carol"), ScriptRunState::Idle);
        }
        assert!(f.scripts.prepared_settings_sync().is_some());
        assert!(shown_bulk(&f).contains("bob:") && shown_bulk(&f).contains("settings copy"));
        if apply {
            f.scripts.apply_settings_sync(&mut f.core).unwrap();
            f.core.flush_writes();
        } else {
            f.scripts.cancel_settings_sync();
        }
        f.settle();
        for name in ["bob", "carol"] {
            f.wait_state(name, ScriptRunState::Running);
            let want = f.scripts.merged_profile_bag(
                &mut f.core,
                name,
                CardSource::from_script(card.source),
                &card.name,
                &card.path,
                SchemaRef::new(&card.settings_schema),
            );
            assert_eq!(
                want["target"],
                json!(if apply { "Knight of Ardougne" } else { "Guard" })
            );
            assert!(
                f.run_has_bag(name, &want),
                "{name} must consume the resolved bag"
            );
        }
    }
}

#[test]
fn queued_js_copy_releases_start_after_staging_without_waiting_for_commit() {
    let mut f = fixture("queued-js-copy-staged", &["alice", "bob"]);
    let card = f.card("thiever.ts", "export const SETTINGS = { target: { type: 'string', default: 'Guard' } };\nexport default class T extends LoopingBot { override loop() {} }\n");
    for name in ["alice", "bob"] {
        f.assign(name, &card);
    }
    f.set("alice", &card, "target", json!("Knight of Ardougne"));
    f.core.flush_writes();
    f.scripts.start_all(&mut f.core, None);
    f.prepare("alice", &card);
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.poll();
    f.scripts.poll(&mut f.core);
    assert!(f.scripts.start_queue_place("bob").is_none());
    f.wait_state("bob", ScriptRunState::Running);
    let want = bag(&[("target", json!("Knight of Ardougne"))]);
    assert!(f.run_has_bag("bob", &want));
    drop(held);
    f.core.flush_writes();
}

#[test]
fn scenario_start_script_waits_for_copy_confirmation() {
    use host_play::live_start::{
        fire_pending_catalog_start, PendingCatalogStart, StartArming, StartScriptPump,
    };
    for loadouts in [false, true] {
        let mut f = fixture(&format!("scenario-copy-{loadouts}"), &["alice", "bob"]);
        let card = f.card("thiever.ts", "export const SETTINGS = { target: { type: 'string', default: 'Guard' } };\nexport default class T extends LoopingBot { override loop() {} }\n");
        f.assign("alice", &card);
        f.assign("bob", &card);
        f.set("alice", &card, "target", json!("Knight of Ardougne"));
        f.core.flush_writes();
        f.prepare("alice", &card);
        let handle = f.core.play().unwrap().script_start_handle();
        let arming = || StartArming {
            handle: Some(handle.clone()),
            ..Default::default()
        };
        let bag = bag(&[("target", json!("Guard"))]);
        let mut pending = vec![PendingCatalogStart::load(
            "bob",
            card.js.clone(),
            card.shape,
            Some(bag.clone()),
            Vec::new(),
            if loadouts {
                vec![api::loadout::Loadout::new("fixture")]
            } else {
                Vec::new()
            },
        )];
        for _ in 0..3 {
            assert_eq!(
                fire_pending_catalog_start(&mut pending, true, false, arming),
                StartScriptPump::Hold
            );
            assert!(!pending[0].started);
            assert_eq!(f.state("bob"), ScriptRunState::Idle);
        }
        assert!(pending[0].waiting_reason.as_ref().is_some_and(
            |reason| reason.contains("bob") && reason.contains("settings copy confirmation")
        ));
        assert!(f.scripts.prepared_settings_sync().is_some());
        f.scripts.cancel_settings_sync();
        assert_eq!(
            fire_pending_catalog_start(&mut pending, true, false, arming),
            StartScriptPump::Continue
        );
        f.wait_state("bob", ScriptRunState::Running);
        f.core
            .play()
            .unwrap()
            .script_attach_identity("bob", card.identity_key());
        assert!(f.run_has_bag("bob", &bag));
    }
}

#[test]
fn compiled_scenario_start_script_holds_without_failing_preparation() {
    use host_play::live_start::{
        fire_pending_catalog_start, PendingCatalogStart, StartArming, StartScriptPump,
    };
    let mut f = native_fixture("compiled-scenario-copy", &["alice", "bob"]);
    let id = script::CompiledId("Sherlock");
    for name in ["alice", "bob"] {
        start_sherlock(&mut f, name);
        f.core.stop_script(name);
        f.wait_state(name, ScriptRunState::Idle);
    }
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "alice", CompiledCardId::from_script(id), None)
        .unwrap();
    let handle = f.core.play().unwrap().script_start_handle();
    let arming = || StartArming {
        handle: Some(handle.clone()),
        ..Default::default()
    };
    let want = f
        .scripts
        .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
        .unwrap();
    let mut pending = vec![PendingCatalogStart::compiled("bob", id, want.clone())];
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Hold
    );
    assert!(!pending[0].started);
    assert_eq!(f.state("bob"), ScriptRunState::Idle);
    f.scripts.cancel_settings_sync();
    assert_eq!(
        fire_pending_catalog_start(&mut pending, true, false, arming),
        StartScriptPump::Continue
    );
    let guard = Instant::now() + HANG_GUARD;
    loop {
        match fire_pending_catalog_start(&mut pending, true, false, arming) {
            StartScriptPump::CompiledRunning => break,
            StartScriptPump::Failed(error) => panic!("{error}"),
            _ => assert!(
                Instant::now() < guard,
                "compiled scenario did not settle: hang guard expired"
            ),
        }
        std::thread::yield_now();
    }
    assert_eq!(f.state("bob"), ScriptRunState::Running);
    assert!(f.run_has_bag("bob", &want));
}

/// Apply to all shares the prepare path: freezing it for a newly assigned
/// card (no per-card key in the source profile yet) writes nothing, and the
/// bag it freezes is the migrated legacy overrides.
#[test]
fn apply_to_all_prepare_and_cancel_write_nothing_for_an_unclaimed_source() {
    let mut f = fixture("sync-readonly", &["alice", "bob"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.scripts
        .legacy
        .set_str(thiever.source, &thiever.name, "target", "Guard");
    let key = thiever.identity_key();
    let vault_file = f.dir.join("vault");
    let before = std::fs::read(&vault_file).unwrap();

    f.prepare("alice", &thiever);
    let scope = f.scripts.prepared_settings_sync().unwrap();
    assert_eq!(scope.targets, ["bob"]);
    assert_eq!(scope.overrides, bag(&[("target", json!("Guard"))]));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(std::fs::read(&vault_file).unwrap(), before, "prepare wrote");
    assert_eq!(f.saved_bag("alice", &key), None);

    f.scripts.cancel_settings_sync();
    f.core.flush_writes();
    assert_eq!(std::fs::read(&vault_file).unwrap(), before, "cancel wrote");

    f.prepare("alice", &thiever);
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.saved_bag("bob", &key),
        Some(bag(&[("target", json!("Guard"))]))
    );
}

/// Apply to all posts the whole bag to each live target: its own
/// profile-global clue duel partner survives the card-parameter copy.
#[test]
fn apply_to_all_keeps_each_live_targets_global_clue_partner() {
    let mut f = fixture("sync-partner", &["alice", "bob", "dave"]);
    for (name, partner) in [("bob", "Helper"), ("dave", "Second")] {
        let mut profile = f.core.vault().unwrap().get(name).unwrap().clone();
        profile.settings.clue_duel_partner = partner.into();
        f.core
            .save_profile(profile, crate::ArmMirror::remember(None), "partner")
            .unwrap();
    }
    f.core.flush_writes();
    let clue = f.card("clue.ts", LOOPING);
    for name in ["alice", "bob", "dave"] {
        f.assign(name, &clue);
    }
    f.start_running("bob");
    f.start_running("dave");
    f.set("alice", &clue, "target", json!("Guard"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    f.prepare("alice", &clue);
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(report.delivered, 2, "{report:?}");
    for (name, partner) in [("bob", "Helper"), ("dave", "Second")] {
        let whole = bag(&[
            ("target", json!("Guard")),
            ("clueDuelPartner", json!(partner)),
        ]);
        assert!(
            f.run_has_bag(name, &whole),
            "{name}'s run keeps its own global partner"
        );
    }
}

/// A profile save that changes the clue duel partner reaches the running
/// script once durable, as frozen reads the Global partner live.
#[test]
fn a_saved_partner_reaches_the_running_script() {
    let mut f = fixture("partner-live", &["alice"]);
    let clue = f.card("clue.ts", LOOPING);
    f.assign("alice", &clue);
    f.start_running("alice");
    let mut profile = f.core.vault().unwrap().get("alice").unwrap().clone();
    profile.settings.clue_duel_partner = " Helper ".into();
    let live = f
        .scripts
        .profile_save_live(&f.core, "alice", &profile.settings);
    f.core
        .save_profile(profile, crate::ArmMirror::remember(live), "credentials")
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(f.run_has_bag("alice", &bag(&[("clueDuelPartner", json!("Helper"))])));
}

/// Two partner saves of one profile committed together: the newer one
/// replaces the older, which is superseded rather than delivered, and the
/// running script ends on the newer partner.
#[test]
fn coalesced_partner_saves_settle_on_the_newest_partner() {
    let mut f = fixture("partner-coalesced", &["alice"]);
    let clue = f.card("clue.ts", LOOPING);
    f.assign("alice", &clue);
    f.start_running("alice");
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    let saves = ["First", "Second"].map(|partner| {
        let mut profile = f.core.vault().unwrap().get("alice").unwrap().clone();
        profile.settings.clue_duel_partner = partner.into();
        let live = f
            .scripts
            .profile_save_live(&f.core, "alice", &profile.settings);
        f.core
            .save_profile(profile, crate::ArmMirror::remember(live), "credentials")
            .unwrap()
    });
    drop(held);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let outcome = |op| {
        f.core
            .operation(op)
            .unwrap()
            .outcome("alice")
            .map(|o| o.duplicate())
    };
    assert_eq!(outcome(saves[0]), Some(Outcome::Cancelled));
    assert_eq!(outcome(saves[1]), Some(Outcome::Completed));
    assert!(f.run_has_bag("alice", &bag(&[("clueDuelPartner", json!("Second"))])));
}

#[test]
fn a_removed_catalog_card_marks_its_assignments_unavailable() {
    let mut f = fixture("catalog-removed", &["alice", "bob"]);
    let gone = vault::ScriptAssignment {
        source_kind: "catalog".into(),
        identity: "GoneBot".into(),
        display_name: "GoneBot".into(),
        unavailable: None,
    };
    assert!(f.scripts.persist_assignment(&mut f.core, "alice", gone));
    f.scripts
        .mark_removed_catalog_assignments(&mut f.core, "GoneBot");
    f.core.flush_writes();
    let asg = Scripts::assignment(&f.core, "alice").unwrap();
    assert_eq!(asg.identity, "GoneBot", "the assignment is kept");
    assert!(asg.unavailable.unwrap().contains("catalog removed"));
    assert_eq!(Scripts::assignment(&f.core, "bob"), None);
}

#[test]
fn a_failed_write_is_reported_and_never_reaches_the_run() {
    let mut f = fixture("sync-fail", &["alice", "bob"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.start_running("bob");
    f.set("alice", &thiever, "target", json!("Guard"));
    f.core.flush_writes();
    f.prepare("alice", &thiever);
    // The publish cannot replace a directory: keep the vault aside and put one
    // at its name.
    let vault_file = f.dir.join("vault");
    let aside = vault_file.with_extension("aside");
    std::fs::rename(&vault_file, &aside).unwrap();
    std::fs::create_dir(&vault_file).unwrap();

    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.set("bob", &thiever, "food", json!("Lobster"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    std::fs::remove_dir(&vault_file).unwrap();
    std::fs::rename(&aside, &vault_file).unwrap();

    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(report.saved, 0);
    assert_eq!(report.failed.len() + report.superseded, 1, "{report:?}");
    assert_eq!(report.delivered, 0);
    let key = thiever.identity_key();
    let target = |bag: Option<Map<String, Value>>| bag.and_then(|b| b.get("target").cloned());
    assert_eq!(
        target(f.saved_bag("bob", &key)),
        None,
        "nothing became durable"
    );
    assert!(
        !f.run_has_bag("bob", &bag(&[("target", json!("Guard"))])),
        "a failed sync is not pushed"
    );
    assert!(
        !f.run_has_bag(
            "bob",
            &bag(&[("target", json!("Guard")), ("food", json!("Lobster"))])
        ),
        "a failed single edit is not pushed"
    );
    let staged = f.core.vault().unwrap().get("bob").unwrap();
    assert_eq!(
        target(staged.settings.script_settings.get(&key).cloned()),
        None,
        "the staged edits roll back"
    );
}

#[test]
fn a_run_replaced_before_the_write_is_durable_is_not_posted() {
    let mut f = fixture("sync-stale", &["alice", "bob"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.start_running("bob");
    f.set("alice", &thiever, "target", json!("Guard"));
    f.core.flush_writes();
    f.prepare("alice", &thiever);
    let before = f.generation("bob");

    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    // Replace bob's run while the write is still queued.
    f.core.stop_script("bob");
    f.wait_state("bob", ScriptRunState::Idle);
    f.start_running("bob");
    assert_ne!(f.generation("bob"), before);
    drop(held);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!((report.saved, report.stale, report.delivered), (1, 1, 0));
    assert_eq!(
        f.saved_bag("bob", &thiever.identity_key()),
        Some(bag(&[("target", json!("Guard"))]))
    );
}

#[test]
fn an_assignment_is_saved_only_once_its_start_is_ready() {
    let mut f = fixture("assign-ready", &["alice", "bob"]);
    let good = f.card("good.ts", LOOPING);
    let bad = f.card(
        "bad.ts",
        "export const apiVersion = 2;\nthrow new Error('setup-fails');\nexport function tick(api) {}\n",
    );
    let sel = |card: &script::JsCard| {
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()))
    };
    f.scripts.set_pending_browse("alice", sel(&good));
    f.scripts.set_pending_browse("bob", sel(&bad));

    f.scripts
        .start_selected(&mut f.core, "alice", None, None)
        .unwrap();
    f.scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap();
    f.core.flush_writes();
    assert_eq!(Scripts::assignment(&f.core, "alice"), None, "not Ready yet");
    f.settle();
    f.core.flush_writes();

    assert_eq!(
        Scripts::assignment(&f.core, "alice"),
        Some(good.assignment())
    );
    assert_eq!(Scripts::assignment(&f.core, "bob"), None, "a failed Start");
    assert!(f.scripts.heading_is_pending(&f.core, "bob"));
    assert!(!f.scripts.heading_is_pending(&f.core, "alice"));
    let failures = f.scripts.take_start_failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "bob");
    assert!(failures[0].1.contains("setup-fails"), "{failures:?}");
}

#[test]
fn start_all_and_stop_all_cover_every_member() {
    let mut f = fixture("bulk", &["alice", "bob", "carol"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.start_running("bob");

    f.scripts.start_all(&mut f.core, None);
    assert_eq!(
        f.scripts.take_notice(),
        Some(Notice::show(
            "Start all: started 1, skipped 1, failed 1: carol: no assignment; skipped bob: already active",
        ))
    );
    f.settle();
    f.wait_state("alice", ScriptRunState::Running);

    // A reload shape on bob: its run is reaping with a replacement queued.
    f.core.stop_script("bob");
    let sel = script::ScriptSel::Loaded(thiever.source, thiever.identity_id());
    f.scripts
        .start_sel(
            &mut f.core,
            "bob",
            &sel,
            None,
            super::StartKind::Reload,
            false,
        )
        .unwrap();
    assert_eq!(f.state("bob"), ScriptRunState::Stopping);
    assert_eq!(f.scripts.stop_all(&mut f.core), 2);
    for name in ["alice", "bob"] {
        f.wait_state(name, ScriptRunState::Idle);
    }
    for _ in 0..20 {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        f.state("bob"),
        ScriptRunState::Idle,
        "the queued replacement never runs"
    );
}

#[test]
fn a_parameter_edit_reaches_the_run_once_durable() {
    let mut f = fixture("edit-live", &["alice"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.start_running("alice");
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.set("alice", &thiever, "target", json!("Guard"));
    drop(held);
    f.core.flush_writes();
    let writes = f.core.take_settings_writes();
    assert_eq!(writes.len(), 1);
    // Delivered, not Unchanged: the run had not received it while queued.
    assert_eq!(
        writes[0].result,
        super::SettingsResult::saved(super::LiveDelivery::Delivered)
    );
    assert!(f.run_has_bag("alice", &bag(&[("target", json!("Guard"))])));
}

/// An Apply-to-all write saved in one commit with a later edit of another
/// setting on the same member counts as saved, not superseded: its bag is
/// durable. A member running the card receives it; an idle one counts as
/// not running.
#[test]
fn a_sync_saved_together_with_another_edit_counts_as_saved_and_reaches_a_run() {
    for running in [true, false] {
        let mut f = fixture(&format!("sync-coalesced-{running}"), &["alice", "bob"]);
        let thiever = f.card("thiever.ts", LOOPING);
        f.assign("alice", &thiever);
        f.assign("bob", &thiever);
        if running {
            f.start_running("bob");
        }
        f.set("alice", &thiever, "target", json!("Guard"));
        f.core.flush_writes();
        f.prepare("alice", &thiever);

        let gate = f.core.write_gate();
        let held = gate.lock().unwrap();
        let op = f.scripts.apply_settings_sync(&mut f.core).unwrap();
        f.core
            .submit_detached(
                OperatorCommand::SetAutoLogin(true),
                crate::Targets::Profile("bob"),
            )
            .unwrap()
            .operation()
            .expect("the write opened an operation");
        drop(held);
        f.core.flush_writes();
        f.scripts.poll(&mut f.core);

        let want = bag(&[("target", json!("Guard"))]);
        assert_eq!(
            f.saved_bag("bob", &thiever.identity_key()),
            Some(want.clone())
        );
        let report = f.scripts.last_settings_sync().unwrap();
        let (delivered, not_running) = if running { (1, 0) } else { (0, 1) };
        assert_eq!(
            (
                report.saved,
                report.delivered,
                report.not_running,
                report.superseded
            ),
            (1, delivered, not_running, 0),
            "running {running}: {report:?}"
        );
        assert_eq!(
            f.core.operation(op).unwrap().outcome("bob"),
            Some(&Outcome::Completed)
        );
        if running {
            assert!(f.run_has_bag("bob", &want), "bob's run received the bag");
            f.core.play().unwrap().script_stop("bob");
        }
    }
}

/// Two Apply-to-all runs overlap (the first one's writes still queued when
/// the second applies): each operation settles on its own writes, and the
/// newest report is the one shown.
#[test]
fn overlapping_syncs_each_settle_their_own_operation() {
    let mut f = fixture("sync-overlap", &["alice", "bob"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    f.assign("bob", &thiever);
    f.set("alice", &thiever, "target", json!("Guard"));
    f.core.flush_writes();

    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.prepare("alice", &thiever);
    let first = f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.prepare("alice", &thiever);
    let second = f.scripts.apply_settings_sync(&mut f.core).unwrap();
    drop(held);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    for op in [first, second] {
        let report = f.core.operation(op).unwrap();
        assert!(report.is_settled(), "{op:?} left pending: {report:?}");
    }
    assert!(matches!(
        f.core.operation(second).unwrap().outcome("bob"),
        Some(&Outcome::Completed)
    ));
    assert_eq!(f.scripts.last_settings_sync().unwrap().op, second);
    assert_eq!(
        f.saved_bag("bob", &thiever.identity_key()),
        Some(bag(&[("target", json!("Guard"))]))
    );
}

/// A Start that clears a card's earlier load failure does not replace an
/// unrelated banner: the Start all report stays shown.
#[test]
fn a_ready_start_keeps_the_start_all_report_shown() {
    let mut f = fixture("ready-keeps-report", &["alice"]);
    std::fs::write(f.dir.join("gate.ts"), "export const fail = true;").unwrap();
    let card = f.card(
        "retry.ts",
        "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('first-load');\nexport function tick(api) {}\n",
    );
    f.assign("alice", &card);
    f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
    f.settle();
    assert!(f.scripts.js().load_failure(&card.identity_key()).is_some());
    std::fs::write(f.dir.join("gate.ts"), "export const fail = false;").unwrap();
    f.scripts.take_notice();

    f.scripts.start_all(&mut f.core, None);
    let report = Notice::show("Start all: started 1, skipped 0");
    assert_eq!(f.scripts.take_notice(), Some(report));
    f.settle();
    f.core.flush_writes();
    assert!(f.scripts.js().load_failure(&card.identity_key()).is_none());
    assert_eq!(f.scripts.take_notice(), None, "the report is not replaced");
    f.core.play().unwrap().script_stop("alice");
}

/// Start with a prior load failure: the banner lists the failure, then the
/// front end shows its own error; the Start settling Ready leaves that newer
/// error alone. Without the newer error the listing is cleared.
#[test]
fn a_ready_start_never_erases_a_newer_front_end_banner() {
    for newer in [Some("map: no route to 3200,3200"), None] {
        let mut f = fixture("ready-newer-banner", &["alice"]);
        std::fs::write(f.dir.join("gate.ts"), "export const fail = true;").unwrap();
        let card = f.card(
            "retry.ts",
            "import { fail } from './gate.js';\nexport const apiVersion = 2;\nif (fail) throw new Error('first-load');\nexport function tick(api) {}\n",
        );
        f.assign("alice", &card);
        f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
        f.settle();
        std::fs::write(f.dir.join("gate.ts"), "export const fail = false;").unwrap();

        // The front end's banner, as the panel and the TUI keep it.
        let mut banner: Option<String> = None;
        f.scripts.take_notice();
        f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
        f.scripts.show_load_failures();
        f.scripts.take_notice().unwrap().apply(&mut banner);
        assert!(banner.as_deref().unwrap_or("").contains("first-load"));
        if let Some(error) = newer {
            banner = Some(error.to_string());
        }
        // The banner folds every notice the coordinator raises while the
        // Start settles, so it pumps as `pump_until` does, plus that fold.
        let guard = Instant::now() + HANG_GUARD;
        while f.scripts.starts_pending() {
            assert!(
                Instant::now() < guard,
                "script Start did not settle: hang guard expired"
            );
            f.core.poll();
            f.scripts.poll(&mut f.core);
            if let Some(notice) = f.scripts.take_notice() {
                notice.apply(&mut banner);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(banner.as_deref(), newer, "newer banner {newer:?}");
        f.core.play().unwrap().script_stop("alice");
    }
}

/// Each Start's operation id reaches its member's fleet row and slot log
/// from acceptance to settlement; a failed setup stays on the row as that
/// operation's failure.
#[test]
fn a_start_carries_its_operation_to_the_row_and_the_log() {
    let mut f = fixture("start-op-ids", &["opid-alice", "opid-bob"]);
    let good = f.card("good.ts", LOOPING);
    let bad = f.card(
        "bad.ts",
        "export const apiVersion = 2;\nthrow new Error('setup-fails');\nexport function tick(api) {}\n",
    );
    let sel = |card: &script::JsCard| {
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()))
    };
    f.scripts.set_pending_browse("opid-alice", sel(&good));
    f.scripts.set_pending_browse("opid-bob", sel(&bad));
    let op_of = |f: &Fixture, name: &str| {
        f.core
            .fleet_view()
            .row(name)
            .and_then(|row| row.last_op().map(|op| op.duplicate()))
            .expect(name)
    };

    f.scripts
        .start_selected(&mut f.core, "opid-alice", None, None)
        .unwrap();
    f.scripts
        .start_selected(&mut f.core, "opid-bob", None, None)
        .unwrap();
    f.pump_until("Start preparation did not publish", |f| {
        f.core
            .fleet_view()
            .row("opid-alice")
            .and_then(|row| row.last_op())
            .is_some_and(|op| op.action() == crate::ActionKind::ScriptStart)
    });
    let accepted = op_of(&f, "opid-alice");
    assert_eq!(accepted.action(), crate::ActionKind::ScriptStart);
    f.settle();
    f.core.poll();

    let alice = op_of(&f, "opid-alice");
    assert_eq!(
        (alice.id(), alice.outcome()),
        (accepted.id(), &Outcome::Completed)
    );
    let bob = op_of(&f, "opid-bob");
    assert!(
        bob.outcome().is_failed()
            && bob
                .outcome()
                .reason()
                .is_some_and(|reason| reason.contains("setup-fails")),
        "{bob:?}"
    );
    let failed = f.core.fleet_view().row("opid-bob").unwrap().has_failure();
    assert!(failed, "a failed Start is a visible row failure");

    let lines = |slot: &str| {
        let mut view = crate::log::LogView::new(crate::log::LogScope::slot(slot));
        crate::log::global().refresh(&mut view);
        view.rows()
            .iter()
            .map(|e| (e.level(), e.message().to_string()))
            .collect::<Vec<_>>()
    };
    let alice_log = lines("opid-alice");
    let at = |text: String| alice_log.iter().position(|(_, line)| *line == text);
    let (start, done) = (
        at(format!("op#{} Start accepted", alice.id().0)),
        at(format!("op#{} Start completed", alice.id().0)),
    );
    assert!(start.is_some() && start < done, "{alice_log:?}");
    let prefix = format!("op#{} Start failed: ", bob.id().0);
    assert!(
        lines("opid-bob").iter().any(|(level, line)| {
            *level == api::hostlog::Level::Error
                && line.starts_with(&prefix)
                && line.contains("setup-fails")
        }),
        "{:?}",
        lines("opid-bob")
    );
    f.core.play().unwrap().script_stop("opid-alice");
}

#[test]
fn marked_start_skips_active_rows_and_duplicate_start_is_idempotent() {
    let mut f = fixture("bulk-selection", &["alice", "bob", "carol"]);
    let card = f.card("bulk.ts", LOOPING);
    f.assign("alice", &card);
    f.start_running("alice");

    let mut selected = MarkedSelection::default();
    selected.mark_all([
        ProfileIdentity::uid(1),
        ProfileIdentity::uid(2),
        ProfileIdentity::uid(3),
    ]);
    let card_sel =
        ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    start_marked(
        &selected,
        &mut f.core,
        &mut f.scripts,
        Some(&card_sel),
        None,
    );
    assert_eq!(
        shown_bulk(&f),
        "Start selected: started 1, queued 1, skipped 1: alice: already active"
    );

    f.settle();
    f.wait_state("bob", ScriptRunState::Running);
    f.wait_state("carol", ScriptRunState::Running);
    assert_eq!(
        shown_bulk(&f),
        "Start selected: started 2, skipped 1: alice: already active"
    );

    // The first report finished, so the repeat is a fresh report.
    start_marked(
        &selected,
        &mut f.core,
        &mut f.scripts,
        Some(&card_sel),
        None,
    );
    assert_eq!(
        shown_bulk(&f),
        "Start selected: started 0, skipped 3: alice: already active, \
         bob: already active, carol: already active"
    );
    for name in ["alice", "bob", "carol"] {
        assert_eq!(start_accepted(&f, name), 1, "{name}");
    }

    let stop = stop_marked(&selected, &mut f.core, &mut f.scripts);
    assert_eq!((stop.stopped(), stop.cancelled()), (3, 0));
    let duplicate_stop = stop_marked(&selected, &mut f.core, &mut f.scripts);
    assert_eq!(duplicate_stop.stopped(), 0);
    assert!(duplicate_stop
        .skipped()
        .iter()
        .any(|skip| skip.reason().contains("stopping")));
}

fn native_fixture(test: &str, members: &[&str]) -> Fixture {
    let mut fixture = fixture(test, members);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    fixture.core.play_mut().unwrap().bind_script_test_data(data);
    fixture
}

fn queued_gatherer_copy(test: &str) -> Fixture {
    let mut f = native_fixture(test, &["alice", "bob", "carol"]);
    let id = script::CompiledId("Gatherer");
    for name in ["alice", "bob", "carol"] {
        assert!(f
            .scripts
            .persist_assignment(&mut f.core, name, script::compiled_assignment(id)));
    }
    f.core.flush_writes();
    f.scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            bag(&[("targetPreference", json!("Nearest"))]),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.start_all(&mut f.core, None);
    assert!(f.scripts.start_queue_place("bob").is_some());
    assert_eq!(f.state("bob"), ScriptRunState::Idle);
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "alice", CompiledCardId::from_script(id), None)
        .unwrap();
    f
}

#[test]
fn queued_native_copy_waits_for_durable_apply() {
    let mut f = queued_gatherer_copy("queued-native-copy-durable");
    let id = script::CompiledId("Gatherer");
    let place = f.scripts.start_queue_place("bob").unwrap();
    for _ in 0..3 {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_eq!(f.state("bob"), ScriptRunState::Idle);
        assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
    }
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    // Exactly the next frame: no flush between Apply and admission.
    f.core.poll();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.state("bob"),
        ScriptRunState::Idle,
        "Bob must not start on his pre-copy bag while the native write is pending"
    );
    assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
    // Exercise the second asynchronous boundary too: preparation has staged
    // the copied row, but the writer still cannot commit it.
    let guard = Instant::now() + HANG_GUARD;
    let key = script::compiled_identity_key(id);
    loop {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        assert_eq!(f.state("bob"), ScriptRunState::Idle);
        assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
        let staged = f.core.vault().unwrap().get("bob").unwrap();
        if staged
            .settings
            .script_settings
            .get(&key)
            .and_then(|entry| vault::CompiledSettingsRecord::view(entry).ok())
            .is_some_and(|(_, values)| values.get("targetPreference") == Some(&json!("Nearest")))
        {
            break;
        }
        assert!(
            Instant::now() < guard,
            "native preparation did not settle: hang guard expired"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    let durable = f.core.durable_profile("bob").unwrap();
    assert!(!durable
        .settings
        .script_settings
        .get(&key)
        .and_then(|entry| vault::CompiledSettingsRecord::view(entry).ok())
        .is_some_and(|(_, values)| values.get("targetPreference") == Some(&json!("Nearest"))));
    drop(held);
    f.settle();
    for name in ["bob", "carol"] {
        f.wait_state(name, ScriptRunState::Running);
        let want = f
            .scripts
            .compiled_bag(&f.core, name, CompiledCardId::from_script(id))
            .unwrap();
        assert_eq!(want["targetPreference"], json!("Nearest"));
        assert!(
            f.run_has_bag(name, &want),
            "{name} started with the copied bag"
        );
        let entry = f
            .saved_bag(name, &script::compiled_identity_key(id))
            .unwrap();
        let (_, values) = vault::CompiledSettingsRecord::view(&entry).unwrap();
        assert_eq!(values["targetPreference"], json!("Nearest"));
    }
}

#[test]
fn failed_queued_native_copy_releases_start_on_pre_copy_bag() {
    let mut f = queued_gatherer_copy("queued-native-copy-failure");
    let id = script::CompiledId("Gatherer");
    let before = f
        .scripts
        .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
        .unwrap();
    let vault_file = f.dir.join("vault");
    let aside = vault_file.with_extension("aside");
    std::fs::rename(&vault_file, &aside).unwrap();
    std::fs::create_dir(&vault_file).unwrap();
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.poll();
    f.scripts.poll(&mut f.core);
    assert_eq!(f.state("bob"), ScriptRunState::Idle);
    assert!(f.scripts.start_queue_place("bob").is_some());
    drop(held);
    f.settle();
    f.wait_state("bob", ScriptRunState::Running);
    assert!(
        f.run_has_bag("bob", &before),
        "a refused copy leaves the running bag unchanged"
    );
    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(report.saved, 0);
    assert_eq!(report.failed.len() + report.superseded, 2, "{report:?}");
    std::fs::remove_dir(&vault_file).unwrap();
    std::fs::rename(&aside, &vault_file).unwrap();
}

#[test]
fn cancelling_a_new_prompt_does_not_release_an_accepted_native_copy() {
    let mut f = queued_gatherer_copy("queued-native-copy-overlap");
    let id = script::CompiledId("Gatherer");
    let place = f.scripts.start_queue_place("bob").unwrap();
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "alice", CompiledCardId::from_script(id), None)
        .unwrap();
    f.scripts.cancel_settings_sync();
    f.core.poll();
    f.scripts.poll(&mut f.core);
    assert_eq!(f.state("bob"), ScriptRunState::Idle);
    assert_eq!(f.scripts.start_queue_place("bob"), Some(place));
    drop(held);
    f.settle();
    f.wait_state("bob", ScriptRunState::Running);
    let want = f
        .scripts
        .compiled_bag(&f.core, "bob", CompiledCardId::from_script(id))
        .unwrap();
    assert_eq!(want["targetPreference"], json!("Nearest"));
    assert!(f.run_has_bag("bob", &want));
}

fn start_sherlock(f: &mut Fixture, name: &str) {
    f.scripts
        .start_sel(
            &mut f.core,
            name,
            &script::ScriptSel::Compiled(script::CompiledId("Sherlock")),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state(name), ScriptRunState::Running);
}

fn save_native_partner(f: &mut Fixture, name: &str, partner: &str) {
    let mut row = f.core.profile_for_edit(name).unwrap().clone();
    row.settings.clue_duel_partner = partner.into();
    let live = f
        .scripts
        .profile_save_live(&f.core, name, &row.settings)
        .unwrap();
    f.core
        .save_profile(row, crate::ArmMirror::remember(Some(live)), "partner")
        .unwrap();
}

#[test]
fn compiled_restore_keeps_legacy_sherlock_permissions_and_refuses_malformed_schema() {
    let mut f = fixture("native-restore", &["alice"]);
    let id = script::CompiledId("Sherlock");
    let key = script::compiled_identity_key(id);
    for entry in [
        None,
        Some(json!({})),
        Some(json!({"schema_version": 1, "values": {}})),
    ] {
        let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
        row.settings.script_settings.remove(&key);
        if let Some(entry) = entry {
            row.settings
                .script_settings
                .insert(key.clone(), entry.as_object().unwrap().clone());
        }
        f.core
            .save_profile(row, crate::ArmMirror::none(), "restore")
            .unwrap();
        f.core.flush_writes();
        let super::SchemaView::Ready { version, fields } =
            f.scripts
                .compiled_schema(&f.core, Some("alice"), CompiledCardId::from_script(id))
        else {
            panic!("legacy Sherlock settings stay usable without a schema bump");
        };
        assert_eq!(version, 1);
        let bag = f
            .scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .unwrap();
        for key in ["allow_teleports", "allow_wilderness", "allow_danger_zones"] {
            assert!(fields.iter().any(|field| field.id() == key));
            assert_eq!(bag[key], false, "missing legacy permission defaults off");
        }
    }
    for entry in [
        json!({"schema_version": 2, "values": {"future": 9}}),
        json!({"schema_version": 1, "values": null}),
    ] {
        let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
        row.settings
            .script_settings
            .insert(key.clone(), entry.as_object().unwrap().clone());
        f.core
            .save_profile(row, crate::ArmMirror::none(), "restore")
            .unwrap();
        f.core.flush_writes();
        assert!(matches!(
            f.scripts
                .compiled_schema(&f.core, Some("alice"), CompiledCardId::from_script(id)),
            super::SchemaView::Unavailable(_)
        ));
        assert!(f
            .scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .is_err());
        assert!(f
            .scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                Map::new()
            )
            .is_err());
        assert_eq!(
            f.saved_bag("alice", &key),
            Some(entry.as_object().unwrap().clone())
        );
    }
    assert!(matches!(
        f.scripts.compiled_schema(
            &f.core,
            None,
            CompiledCardId::from_script(script::CompiledId("missing"))
        ),
        super::SchemaView::Unavailable(_)
    ));
}

#[test]
fn native_invalid_preparation_keeps_assignment_and_durable_settings() {
    let mut f = native_fixture("native-invalid", &["alice"]);
    let previous = f.card("previous.ts", LOOPING);
    f.assign("alice", &previous);
    let id = script::CompiledId("Sherlock");
    f.scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            bag(&[("unknown", json!(1))]),
        )
        .unwrap();
    f.core.flush_writes();
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite { result, .. }] if result.error().is_some()
    ));
    assert_eq!(
        f.saved_bag("alice", &script::compiled_identity_key(id)),
        None
    );
    f.scripts.inject = Some(bag(&[("unknown", json!(1))]));
    f.scripts
        .start_sel(
            &mut f.core,
            "alice",
            &script::ScriptSel::Compiled(id),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state("alice"), ScriptRunState::Idle);
    assert_eq!(
        Scripts::assignment(&f.core, "alice"),
        Some(previous.assignment())
    );
    assert!(f.core.play().unwrap().script_last_error("alice").is_some());
}
#[test]
fn gatherer_radius_round_trips_through_profile_save_and_load() {
    let mut f = native_fixture("gatherer-radius-profile", &["alice"]);
    let id = script::CompiledId("Gatherer");
    let key = script::compiled_identity_key(id);
    let mut values = Map::new();
    values.insert("radius".into(), json!(12.0));

    let op = f
        .scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            values,
        )
        .unwrap();
    f.core.flush_writes();
    assert_eq!(
        f.core.operation(op).unwrap().outcome("alice"),
        Some(&Outcome::Completed),
        "saving the Gatherer profile must not fail while configuring its integer radius"
    );

    let saved = f.saved_bag("alice", &key).expect("Gatherer profile saved");
    let (_, saved_values) = vault::CompiledSettingsRecord::view(&saved).unwrap();
    assert_eq!(
        saved_values.get("radius").and_then(Value::as_u64),
        Some(12),
        "the durable profile stores the integral setting as an integer"
    );

    let bag = f
        .scripts
        .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
        .unwrap();
    assert_eq!(
        script::gatherer::GathererSettings::from_bag(&bag)
            .unwrap()
            .radius,
        12
    );
}

#[test]
fn gatherer_profile_loads_integral_legacy_float_and_rejects_fraction() {
    let mut f = native_fixture("gatherer-radius-legacy", &["alice"]);
    let id = script::CompiledId("Gatherer");
    let key = script::compiled_identity_key(id);

    for radius in [json!(12.0), json!(12.5)] {
        let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
        let mut values = Map::new();
        values.insert("radius".into(), radius);
        row.settings.script_settings.insert(
            key.clone(),
            vault::CompiledSettingsRecord {
                schema_version: script::compiled_card(id).unwrap().schema_version,
                values,
            }
            .into_entry(),
        );
        f.core
            .save_profile(row, crate::ArmMirror::none(), "legacy-radius")
            .unwrap();
        f.core.flush_writes();

        let bag = f
            .scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .unwrap();
        let parsed = script::gatherer::GathererSettings::from_bag(&bag);
        if bag["radius"].as_f64() == Some(12.5) {
            let error = parsed.expect_err("fractional radius must be rejected");
            assert_eq!(error.code.as_ref(), "invalid-settings");
            assert!(
                error
                    .message
                    .contains("floating point `12.5`, expected u16"),
                "{:?}",
                error.message
            );
        } else {
            assert_eq!(bag["radius"].as_u64(), Some(12));
            assert_eq!(parsed.unwrap().radius, 12);
        }
    }
}

#[test]
fn native_settings_wait_for_durability_and_remain_per_account() {
    let mut f = native_fixture("native-durable", &["alice", "bob"]);
    start_sherlock(&mut f, "alice");
    start_sherlock(&mut f, "bob");
    let revision = |f: &Fixture, name| {
        f.core
            .play()
            .unwrap()
            .script_native_settings_revision(name)
            .unwrap()
    };
    let original = revision(&f, "alice");
    // Writes stage under unique temp names, so block the publish itself: the
    // vault file kept aside while a directory sits at its name.
    let vault_path = f.dir.join("vault");
    let aside = vault_path.with_extension("aside");
    std::fs::rename(&vault_path, &aside).unwrap();
    std::fs::create_dir(&vault_path).unwrap();
    save_native_partner(&mut f, "alice", "Uncommitted");
    assert_eq!(
        f.core
            .durable_profile("alice")
            .unwrap()
            .settings
            .clue_duel_partner,
        ""
    );
    f.core.flush_writes();
    std::fs::remove_dir(&vault_path).unwrap();
    std::fs::rename(&aside, &vault_path).unwrap();
    assert_eq!(revision(&f, "alice"), original);
    assert_eq!(
        f.core
            .durable_profile("alice")
            .unwrap()
            .settings
            .clue_duel_partner,
        ""
    );
    assert!(!f.core.take_write_failures().is_empty());

    save_native_partner(&mut f, "alice", "Bob");
    save_native_partner(&mut f, "bob", "Alice");
    f.core.flush_writes();
    let alice_revision = revision(&f, "alice");
    let bob_revision = revision(&f, "bob");
    assert!(alice_revision > original);
    assert!(bob_revision > original);
    assert_eq!(
        f.scripts
            .compiled_bag(
                &f.core,
                "alice",
                CompiledCardId::from_script(script::CompiledId("Sherlock"))
            )
            .unwrap()[script::CLUE_DUEL_PARTNER],
        "Bob"
    );
    assert_eq!(
        f.scripts
            .compiled_bag(
                &f.core,
                "bob",
                CompiledCardId::from_script(script::CompiledId("Sherlock"))
            )
            .unwrap()[script::CLUE_DUEL_PARTNER],
        "Alice"
    );
    save_native_partner(&mut f, "alice", "Carol");
    f.core.flush_writes();
    assert!(revision(&f, "alice") > alice_revision);
    assert_eq!(revision(&f, "bob"), bob_revision);
}

#[test]
fn native_settings_reply_loses_to_profile_recreation() {
    let mut f = native_fixture("native-stale-preparation", &["alice"]);
    start_sherlock(&mut f, "alice");
    let mut replacement = f.core.vault().unwrap().get("alice").unwrap().clone();
    replacement.settings = ProfileSettings::default();
    f.scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(script::CompiledId("Sherlock")),
            Map::new(),
        )
        .unwrap();
    f.core
        .submit_detached(
            OperatorCommand::DeleteProfile,
            crate::Targets::Profile("alice"),
        )
        .unwrap()
        .operation()
        .expect("the delete opened an operation");
    f.core
        .create_profile(replacement, crate::ArmMirror::none(), "recreate")
        .unwrap();
    f.core.flush_writes();
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite {
            result: super::SettingsResult::Superseded,
            ..
        }]
    ));
    assert_eq!(
        f.core.durable_profile("alice").unwrap().settings,
        ProfileSettings::default()
    );
}

#[test]
fn profile_form_save_survives_stop_and_restart_with_stale_live_delivery() {
    for restart in [false, true] {
        let mut f = native_fixture("native-form-control-race", &["alice"]);
        start_sherlock(&mut f, "alice");
        let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
        row.password = "updated-password".into();
        row.settings.clue_duel_partner = "Bob".into();
        let live = f
            .scripts
            .profile_save_live(&f.core, "alice", &row.settings)
            .unwrap();
        let op = f
            .core
            .save_profile(row, crate::ArmMirror::remember(Some(live)), "form")
            .unwrap();
        f.core.stop_script("alice");
        if restart {
            f.core
                .start_script(
                    "alice",
                    crate::ScriptStart::compiled(
                        CompiledCardId::from_script(script::CompiledId("Sherlock")),
                        Map::new(),
                    ),
                    None,
                )
                .unwrap();
        }
        f.core.flush_writes();
        let saved = f.core.durable_profile("alice").unwrap();
        assert_eq!(saved.password, "updated-password");
        assert_eq!(saved.settings.clue_duel_partner, "Bob");
        assert!(
            matches!(f.core.take_settings_writes().as_slice(), [super::SettingsWrite {
            op: saved_op, result, ..
        }] if *saved_op == op && result.delivery() == Some(&super::LiveDelivery::Stale))
        );
    }
}

#[test]
fn unrelated_writes_never_persist_invalid_native_drafts_or_poison_start() {
    for writer in ["assignment", "ready", "loaded"] {
        let mut f = native_fixture("native-draft-isolation", &["alice"]);
        let id = script::CompiledId("Sherlock");
        if writer == "ready" {
            f.scripts
                .start_sel(
                    &mut f.core,
                    "alice",
                    &script::ScriptSel::Compiled(id),
                    None,
                    super::StartKind::Start,
                    false,
                )
                .unwrap();
            wait_core_state(&mut f, "alice", ScriptRunState::Running);
        }
        f.scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                bag(&[("unknown", json!(1))]),
            )
            .unwrap();
        if writer == "ready" {
            f.scripts.poll(&mut f.core);
        } else if writer == "loaded" {
            let card = f.card("other.ts", LOOPING);
            f.set("alice", &card, "target", json!("Guard"));
        } else {
            assert!(f.scripts.persist_assignment(
                &mut f.core,
                "alice",
                script::compiled_assignment(id)
            ));
        }
        f.core.flush_writes();
        let entry = f.saved_bag("alice", &script::compiled_identity_key(id));
        assert!(entry
            .as_ref()
            .is_none_or(|entry| entry["values"].get("unknown").is_none()));
        assert!(f
            .core
            .take_write_failures()
            .iter()
            .any(|failure| failure.error().contains("unknown")));
        f.core.stop_script("alice");
        start_sherlock(&mut f, "alice");
    }
}

#[test]
fn valid_native_draft_survives_unrelated_writes() {
    for writer in ["assignment", "ready", "loaded"] {
        let mut f = native_fixture("native-valid-draft", &["alice"]);
        let id = script::CompiledId("Sherlock");
        if writer == "ready" {
            f.scripts
                .start_sel(
                    &mut f.core,
                    "alice",
                    &script::ScriptSel::Compiled(id),
                    None,
                    super::StartKind::Start,
                    false,
                )
                .unwrap();
            wait_core_state(&mut f, "alice", ScriptRunState::Running);
        }
        let op = f
            .scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                CompiledCardId::from_script(id),
                bag(&[(script::CLUE_DUEL_PARTNER, json!("bob"))]),
            )
            .unwrap();
        if writer == "ready" {
            f.scripts.poll(&mut f.core);
        } else if writer == "loaded" {
            let card = f.card("other.ts", LOOPING);
            f.set("alice", &card, "target", json!("Guard"));
        } else {
            assert!(f.scripts.persist_assignment(
                &mut f.core,
                "alice",
                script::compiled_assignment(id),
            ));
        }
        let mut expected = f
            .core
            .vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .clone();
        f.core.flush_writes();
        let entry = f
            .saved_bag("alice", &script::compiled_identity_key(id))
            .unwrap();
        assert_eq!(
            entry["values"][script::CLUE_DUEL_PARTNER],
            "bob",
            "{writer}"
        );
        expected
            .script_settings
            .insert(script::compiled_identity_key(id), entry);
        assert_eq!(
            f.core.durable_profile("alice").unwrap().settings,
            expected,
            "{writer}"
        );
        assert!(f
            .core
            .take_settings_writes()
            .iter()
            .any(|write| write.op == op && write.result.is_saved()));
    }
}

#[test]
fn profile_partner_delivery_survives_unrelated_write() {
    let mut f = native_fixture("native-partner-unrelated", &["alice"]);
    start_sherlock(&mut f, "alice");
    let run = f.core.play().unwrap().script_native_run("alice");
    let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
    row.settings.clue_duel_partner = "bob".into();
    let live = f
        .scripts
        .profile_save_live(&f.core, "alice", &row.settings)
        .unwrap();
    let op = f
        .core
        .save_profile(row, crate::ArmMirror::remember(Some(live)), "form")
        .unwrap();
    f.core
        .submit_detached(
            OperatorCommand::SetAutoLogin(true),
            crate::Targets::Profile("alice"),
        )
        .unwrap()
        .operation()
        .expect("the write opened an operation");
    f.core.flush_writes();
    assert_eq!(f.core.play().unwrap().script_native_run("alice"), run);
    assert!(f
        .core
        .take_settings_writes()
        .iter()
        .any(|write| write.op == op
            && write.result.delivery() == Some(&super::LiveDelivery::Applied)));
}

#[test]
fn rejected_native_live_preparation_does_not_reject_profile_form_durability() {
    let mut f = native_fixture("native-form-preparation-failure", &["alice"]);
    start_sherlock(&mut f, "alice");
    let mut row = f.core.vault().unwrap().get("alice").unwrap().clone();
    row.password = "new-password".into();
    row.settings.clue_duel_partner = "Bob".into();
    let mut live = f
        .scripts
        .profile_save_live(&f.core, "alice", &row.settings)
        .unwrap();
    live.bag = std::sync::Arc::new(bag(&[("unknown", json!(1))]));
    f.core
        .save_profile(row, crate::ArmMirror::remember(Some(live)), "form")
        .unwrap();
    f.core.flush_writes();
    let disk = Vault::unlock(&f.dir.join("vault"), "test-passphrase-01").unwrap();
    let saved = disk.get("alice").unwrap();
    assert_eq!(saved.password, "new-password");
    assert_eq!(saved.settings.clue_duel_partner, "Bob");
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite { result, .. }]
            if result.delivery().and_then(|live| live.rejection()).is_some()
    ));
}

#[test]
fn durable_native_reply_never_configures_a_replacement_run() {
    let mut f = native_fixture("native-stale-durable", &["alice"]);
    start_sherlock(&mut f, "alice");
    let old = f.core.play().unwrap().script_native_run("alice").unwrap();
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(script::CompiledId("Sherlock")),
            bag(&[(script::CLUE_DUEL_PARTNER, json!("Changed"))]),
        )
        .unwrap();
    let key = script::compiled_identity_key(script::CompiledId("Sherlock"));
    pump_core_until(&mut f.core, "validated edit was never staged", |core| {
        core.vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .script_settings[&key]["values"]
            .get(script::CLUE_DUEL_PARTNER)
            .is_some()
    });
    assert_eq!(f.saved_bag("alice", &key).unwrap()["values"], json!({}));
    f.core.stop_script("alice");
    f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
    f.settle();
    assert_ne!(f.core.play().unwrap().script_native_run("alice"), Some(old));
    drop(held);
    f.core.flush_writes();
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite { result, .. }]
            if result.delivery() == Some(&super::LiveDelivery::Stale)
    ));
    assert_eq!(
        f.core
            .play()
            .unwrap()
            .script_native_settings_revision("alice"),
        Some(1)
    );
}

#[test]
fn deleting_a_profile_cancels_its_unsettled_native_start() {
    let mut f = native_fixture("native-start-removal", &["alice"]);
    f.scripts
        .start_sel(
            &mut f.core,
            "alice",
            &script::ScriptSel::Compiled(script::CompiledId("Sherlock")),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.core
        .submit_detached(
            OperatorCommand::DeleteProfile,
            crate::Targets::Profile("alice"),
        )
        .unwrap()
        .operation()
        .expect("the delete opened an operation");
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state("alice"), ScriptRunState::Idle);
    assert!(f.core.durable_profile("alice").is_none());
}

#[test]
fn native_bulk_reports_and_preserves_each_account_partner() {
    let mut f = native_fixture("native-bulk", &["alice", "bob"]);
    start_sherlock(&mut f, "alice");
    start_sherlock(&mut f, "bob");
    save_native_partner(&mut f, "alice", "Carol");
    save_native_partner(&mut f, "bob", "");
    f.core.flush_writes();
    let id = script::CompiledId("Sherlock");
    assert!(f
        .scripts
        .prepare_compiled_settings_sync(
            &f.core,
            "alice",
            CompiledCardId::from_script(id),
            Some(script::CLUE_DUEL_PARTNER)
        )
        .is_err());
    let scope = f
        .scripts
        .prepare_compiled_settings_sync(&f.core, "alice", CompiledCardId::from_script(id), None)
        .unwrap();
    assert_eq!(
        scope.excluded,
        vec![(
            "bob".to_string(),
            vec![script::CLUE_DUEL_PARTNER.to_string()]
        )]
    );
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let report = f.scripts.last_settings_sync().unwrap();
    assert!(report.is_settled());
    assert_eq!(report.saved, 1);
    assert!(report.failed.is_empty());
    assert_eq!(
        f.core
            .durable_profile("alice")
            .unwrap()
            .settings
            .clue_duel_partner,
        "Carol"
    );
    assert_eq!(
        f.core
            .durable_profile("bob")
            .unwrap()
            .settings
            .clue_duel_partner,
        ""
    );
}

/// A compiled copy that the target cannot accept must not persist an
/// assignment on an unassigned bot.
#[test]
fn failed_native_copy_does_not_assign_an_unassigned_bot() {
    let mut f = fixture("native-copy-no-assign", &["alice", "bob"]);
    let id = script::CompiledId("Sherlock");
    let key = script::compiled_identity_key(id);
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", script::compiled_assignment(id)));
    f.core.flush_writes();
    let mut row = f.core.vault().unwrap().get("bob").unwrap().clone();
    row.settings.script_settings.insert(
        key.clone(),
        json!({"schema_version": 999, "values": {}})
            .as_object()
            .unwrap()
            .clone(),
    );
    f.core
        .save_profile(row, crate::ArmMirror::none(), "plant")
        .unwrap();
    f.core.flush_writes();
    assert!(Scripts::assignment(&f.core, "bob").is_none());

    let sel = ScriptChoice::from_script(script::ScriptSel::Compiled(id));
    prepare_apply_settings_marked(&mark_uids(&[2]), &f.core, &mut f.scripts, "alice", &sel)
        .unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(report.saved, 0, "{report:?}");
    assert_eq!(report.failed.len(), 1, "{report:?}");
    assert!(
        Scripts::assignment(&f.core, "bob").is_none(),
        "failed copy assigned {:?}",
        Scripts::assignment(&f.core, "bob")
    );
    let disk = Vault::unlock(&f.dir.join("vault"), "test-passphrase-01").unwrap();
    assert!(disk
        .get("bob")
        .unwrap()
        .settings
        .script_assignment
        .is_none());
    assert_eq!(
        f.saved_bag("bob", &key),
        Some(
            json!({"schema_version": 999, "values": {}})
                .as_object()
                .unwrap()
                .clone()
        )
    );
}

fn gatherer_fishing_fleet(test: &str) -> Fixture {
    let mut f = native_fixture(test, &["source", "alice", "bob", "carol"]);
    let id = script::CompiledId("Gatherer");
    for name in ["source", "alice", "bob", "carol"] {
        assert!(f
            .scripts
            .persist_assignment(&mut f.core, name, script::compiled_assignment(id)));
    }
    f.core.flush_writes();
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "source",
            CompiledCardId::from_script(id),
            "skill",
            json!("Fishing"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "source", CompiledCardId::from_script(id), None)
        .unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(f.scripts.last_settings_sync().unwrap().saved, 3);
    assert!(f.scripts.pending_restart_prompt().is_none());
    for name in ["alice", "bob", "carol"] {
        assert_eq!(
            f.scripts
                .compiled_bag(&f.core, name, CompiledCardId::from_script(id))
                .unwrap()["skill"],
            "Fishing"
        );
        f.start_running(name);
    }
    f
}

fn sync_gatherer_skill(f: &mut Fixture, skill: &str) {
    let id = script::CompiledId("Gatherer");
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "source",
            CompiledCardId::from_script(id),
            "skill",
            json!(skill),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "source", CompiledCardId::from_script(id), None)
        .unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
}

fn single_gatherer_restart_prompt(test: &str) -> Fixture {
    let mut f = native_fixture(test, &["alice"]);
    let id = script::CompiledId("Gatherer");
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", script::compiled_assignment(id)));
    f.core.flush_writes();
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "skill",
            json!("Fishing"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.start_running("alice");
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "skill",
            json!("Mining"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(f.scripts.pending_restart_prompt().is_some());
    f
}

/// Poll the core (no coordinator) until `done` holds; only `HANG_GUARD` bounds it.
fn pump_core_until<Io>(
    core: &mut OperatorSession<Io>,
    what: &str,
    mut done: impl FnMut(&OperatorSession<Io>) -> bool,
) {
    let guard = Instant::now() + HANG_GUARD;
    while !done(core) {
        assert!(Instant::now() < guard, "{what}: hang guard expired");
        core.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn wait_core_state(f: &mut Fixture, name: &str, want: ScriptRunState) {
    pump_core_until(&mut f.core, name, |core| {
        let state = ScriptRunState::from_script(core.play().unwrap().script_state(name));
        assert!(
            state == want || state != ScriptRunState::Error,
            "{name} ended {state:?}, wanted {want:?}"
        );
        state == want
    });
}

#[test]
fn restart_prompt_skips_stopped_profile_at_click() {
    let mut f = single_gatherer_restart_prompt("restart-click-stopped");
    let starts_before = start_accepted(&f, "alice");
    f.core.play().unwrap().script_stop("alice");
    wait_core_state(&mut f, "alice", ScriptRunState::Idle);

    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();

    assert_eq!(f.state("alice"), ScriptRunState::Idle);
    assert_eq!(start_accepted(&f, "alice"), starts_before);
    assert!(f.scripts.pending_restart_prompt().is_none());
    assert!(matches!(
        f.scripts.take_notice(),
        Some(notice) if notice.text() == Some("Skipped alice: not running")
    ));
}

#[test]
fn restart_prompt_skips_run_that_ends_after_click_check() {
    let mut f = single_gatherer_restart_prompt("restart-click-run-ended");
    let starts_before = start_accepted(&f, "alice");

    f.scripts
        .restart_pending_settings_with_after_check(&mut f.core, None, |core, target| {
            if target.profile != "alice" {
                return;
            }
            core.play().unwrap().script_stop("alice");
            pump_core_until(core, "alice did not stop", |core| {
                ScriptRunState::from_script(core.play().unwrap().script_state("alice"))
                    == ScriptRunState::Idle
            });
            assert_eq!(
                ScriptRunState::from_script(core.play().unwrap().script_state("alice")),
                ScriptRunState::Idle
            );
        })
        .unwrap();
    f.settle();

    assert_eq!(f.state("alice"), ScriptRunState::Idle);
    assert_eq!(start_accepted(&f, "alice"), starts_before);
    assert!(matches!(
        f.scripts.take_notice(),
        Some(notice) if notice.text() == Some("Skipped alice: not running")
    ));
}

#[test]
fn restart_prompt_skips_logout_latch_at_click() {
    let mut f = single_gatherer_restart_prompt("restart-click-logout");
    let starts_before = start_accepted(&f, "alice");
    f.core.logout("alice");

    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();

    assert_eq!(start_accepted(&f, "alice"), starts_before);
    assert!(f.scripts.pending_restart_prompt().is_none());
    assert!(matches!(
        f.scripts.take_notice(),
        Some(notice) if notice.text().is_some_and(|text| text.starts_with("Skipped alice:"))
    ));
}

#[test]
fn restart_prompt_skips_reassigned_profile_without_reassigning_or_starting() {
    let mut f = single_gatherer_restart_prompt("restart-click-reassigned");
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Gatherer")));
    let run = f.generation("alice");
    let starts_before = start_accepted(&f, "alice");
    assert!(f.scripts.persist_assignment(
        &mut f.core,
        "alice",
        script::compiled_assignment(script::CompiledId("Sherlock")),
    ));
    f.core.flush_writes();

    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();

    assert_eq!(f.state("alice"), ScriptRunState::Running);
    assert_eq!(f.generation("alice"), run);
    assert_eq!(start_accepted(&f, "alice"), starts_before);
    assert_eq!(
        super::sel_from_assignment(&Scripts::assignment(&f.core, "alice").unwrap()),
        Some(script::ScriptSel::Compiled(script::CompiledId("Sherlock")))
    );
    assert!(!f.scripts.restart_required_for("alice", &selection));
    assert!(matches!(
        f.scripts.take_notice(),
        Some(notice) if notice.text() == Some("Skipped alice: reassigned")
    ));
}

#[test]
fn restart_badge_clears_after_stop() {
    let mut f = single_gatherer_restart_prompt("restart-badge-stop");
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Gatherer")));
    assert!(f.scripts.pending_restart_prompt().is_some());
    assert!(f.scripts.restart_required_for("alice", &selection));
    f.core.play().unwrap().script_stop("alice");
    wait_core_state(&mut f, "alice", ScriptRunState::Idle);

    f.scripts.poll(&mut f.core);

    assert!(!f.scripts.has_restart_badge("alice"));
    assert!(!f.scripts.restart_required_for("alice", &selection));
    assert!(f.scripts.pending_restart_prompt().is_none());
}

#[test]
fn restart_badge_clears_after_logout() {
    let mut f = single_gatherer_restart_prompt("restart-badge-logout");
    f.core.logout("alice");

    f.scripts.poll(&mut f.core);

    assert!(!f.scripts.has_restart_badge("alice"));
    assert!(f.scripts.pending_restart_prompt().is_none());
}

#[test]
fn restart_badge_clears_after_assignment_changes() {
    let mut f = single_gatherer_restart_prompt("restart-badge-reassigned");
    assert!(f.scripts.persist_assignment(
        &mut f.core,
        "alice",
        script::compiled_assignment(script::CompiledId("Sherlock")),
    ));
    f.core.flush_writes();

    f.scripts.poll(&mut f.core);

    assert!(!f.scripts.has_restart_badge("alice"));
    assert!(f.scripts.pending_restart_prompt().is_none());
}

#[test]
fn gatherer_sync_prompts_merge_and_later_suppresses_until_settings_change() {
    let mut f = gatherer_fishing_fleet("gatherer-sync-merge-later");
    sync_gatherer_skill(&mut f, "Mining");
    sync_gatherer_skill(&mut f, "Woodcutting");

    let prompt = f.scripts.pending_restart_prompt().unwrap();
    assert_eq!(prompt.profiles().len(), 3);
    assert_eq!(
        prompt.profiles().map(String::as_str).collect::<Vec<_>>(),
        vec!["alice", "bob", "carol"]
    );
    f.scripts.dismiss_restart_prompt();
    f.scripts.poll(&mut f.core);
    assert!(f.scripts.pending_restart_prompt().is_none());

    sync_gatherer_skill(&mut f, "Mining");
    assert_eq!(
        f.scripts.pending_restart_prompt().unwrap().profiles().len(),
        3
    );
}

#[test]
fn restarting_a_merged_prompt_twice_starts_each_profile_once() {
    let mut f = gatherer_fishing_fleet("gatherer-sync-double-restart");
    sync_gatherer_skill(&mut f, "Mining");
    sync_gatherer_skill(&mut f, "Woodcutting");
    let starts_before = ["alice", "bob", "carol"].map(|name| start_accepted(&f, name));

    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();
    assert!(f
        .scripts
        .restart_pending_settings(&mut f.core, None)
        .is_err());
    f.settle();

    for (name, starts) in ["alice", "bob", "carol"].into_iter().zip(starts_before) {
        f.wait_state(name, ScriptRunState::Running);
        assert_eq!(start_accepted(&f, name), starts + 1, "{name}");
        assert_eq!(
            f.scripts
                .compiled_bag(
                    &f.core,
                    name,
                    CompiledCardId::from_script(script::CompiledId("Gatherer"))
                )
                .unwrap()["skill"],
            "Woodcutting"
        );
    }
}

#[test]
fn single_restart_required_setting_prompts_and_restarts_its_profile() {
    let mut f = native_fixture("gatherer-single-restart", &["alice"]);
    let id = script::CompiledId("Gatherer");
    let selection = ScriptChoice::from_script(script::ScriptSel::Compiled(id));
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", script::compiled_assignment(id)));
    f.core.flush_writes();
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "skill",
            json!("Fishing"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        f.scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .unwrap()["skill"],
        "Fishing"
    );
    f.start_running("alice");
    let initial_generation = f.generation("alice");
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            "skill",
            json!("Mining"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(f.generation("alice"), initial_generation);

    let prompt = f.scripts.pending_restart_prompt().unwrap();
    assert_eq!(
        prompt.profiles().map(String::as_str).collect::<Vec<_>>(),
        vec!["alice"]
    );
    assert_eq!(
        prompt.summary(),
        "Restart Gatherer on alice to apply the new settings?"
    );
    assert!(f.scripts.restart_required_for("alice", &selection));
    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();
    f.settle();
    f.wait_state("alice", ScriptRunState::Running);
    assert_ne!(f.generation("alice"), initial_generation);
    assert_eq!(
        f.scripts
            .compiled_bag(&f.core, "alice", CompiledCardId::from_script(id))
            .unwrap()["skill"],
        "Mining"
    );
    assert!(!f.scripts.restart_required_for("alice", &selection));
}

#[test]
fn gatherer_sync_restart_confirms_only_the_three_changed_profiles() {
    let mut f = gatherer_fishing_fleet("gatherer-sync-restart");
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Gatherer")));
    let initial_generations = ["alice", "bob", "carol"].map(|name| f.generation(name));

    sync_gatherer_skill(&mut f, "Mining");
    for (name, generation) in ["alice", "bob", "carol"]
        .into_iter()
        .zip(initial_generations)
    {
        assert_eq!(f.generation(name), generation);
    }

    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!((report.saved, report.restart_required), (3, 3));
    let prompt = f.scripts.pending_restart_prompt().unwrap();
    assert_eq!(
        prompt.profiles().map(String::as_str).collect::<Vec<_>>(),
        vec!["alice", "bob", "carol"]
    );
    assert_eq!(
        prompt.summary(),
        "Restart Gatherer on alice, bob, carol to apply the new settings?"
    );

    f.scripts
        .restart_pending_settings(&mut f.core, None)
        .unwrap();
    f.settle();
    for (name, generation) in ["alice", "bob", "carol"]
        .into_iter()
        .zip(initial_generations)
    {
        f.wait_state(name, ScriptRunState::Running);
        assert_ne!(f.generation(name), generation);
        assert_eq!(
            f.scripts
                .compiled_bag(
                    &f.core,
                    name,
                    CompiledCardId::from_script(script::CompiledId("Gatherer"))
                )
                .unwrap()["skill"],
            "Mining"
        );
        assert!(!f.scripts.restart_required_for(name, &selection));
    }
    assert_eq!(f.state("source"), ScriptRunState::Idle);
    assert!(f.scripts.pending_restart_prompt().is_none());
}

#[test]
fn gatherer_sync_later_keeps_fishing_runs_and_restart_badges() {
    let mut f = gatherer_fishing_fleet("gatherer-sync-later");
    let selection =
        ScriptChoice::from_script(script::ScriptSel::Compiled(script::CompiledId("Gatherer")));
    let initial_generations = ["alice", "bob", "carol"].map(|name| f.generation(name));

    sync_gatherer_skill(&mut f, "Mining");

    let prompt = f.scripts.pending_restart_prompt().unwrap();
    assert_eq!(prompt.profiles().len(), 3);
    f.scripts.dismiss_restart_prompt();

    assert!(f.scripts.pending_restart_prompt().is_none());
    for (name, generation) in ["alice", "bob", "carol"]
        .into_iter()
        .zip(initial_generations)
    {
        assert_eq!(f.state(name), ScriptRunState::Running);
        assert_eq!(f.generation(name), generation);
        assert_eq!(
            f.scripts
                .compiled_bag(
                    &f.core,
                    name,
                    CompiledCardId::from_script(script::CompiledId("Gatherer"))
                )
                .unwrap()["skill"],
            "Mining"
        );
        assert!(f.scripts.restart_required_for(name, &selection));
    }
}

#[test]
fn live_applied_gatherer_sync_does_not_offer_a_restart() {
    let mut f = gatherer_fishing_fleet("gatherer-sync-live-applied");
    let id = script::CompiledId("Gatherer");
    let initial_generations = ["alice", "bob", "carol"].map(|name| f.generation(name));
    f.scripts
        .set_compiled_setting(
            &mut f.core,
            "source",
            CompiledCardId::from_script(id),
            "deathPolicy",
            json!("Stop"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.scripts
        .prepare_compiled_settings_sync(&f.core, "source", CompiledCardId::from_script(id), None)
        .unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    for (name, generation) in ["alice", "bob", "carol"]
        .into_iter()
        .zip(initial_generations)
    {
        assert_eq!(f.state(name), ScriptRunState::Running);
        assert_eq!(f.generation(name), generation);
    }

    let report = f.scripts.last_settings_sync().unwrap();
    assert_eq!(report.applied, 3);
    assert_eq!(report.restart_required, 0);
    assert!(f.scripts.pending_restart_prompt().is_none());
}

fn bulk_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("bot{i}")).collect()
}

fn start_accepted(f: &Fixture, name: &str) -> usize {
    (1..)
        .map_while(|id| f.core.operation(crate::OperationId(id)))
        .filter(|report| {
            report.action() == crate::ActionKind::ScriptStart && report.outcome(name).is_some()
        })
        .count()
}

fn mark_uids(uids: &[i32]) -> MarkedSelection {
    let mut selected = MarkedSelection::default();
    selected.mark_all(uids.iter().copied().map(ProfileIdentity::uid));
    selected
}

fn shown_bulk(f: &Fixture) -> String {
    f.scripts
        .last_bulk_report()
        .expect("expected a bulk Start report")
        .to_string()
}

fn starting_among(f: &Fixture, names: &[String]) -> usize {
    names
        .iter()
        .filter(|name| f.state(name) == ScriptRunState::Starting)
        .count()
}

fn idle_among<'a>(f: &Fixture, names: &'a [String]) -> Vec<&'a str> {
    names
        .iter()
        .filter(|name| f.state(name) == ScriptRunState::Idle)
        .map(String::as_str)
        .collect()
}

/// Start all over more members than one frame may admit must not dispatch
/// every isolate in the click. The rest wait for later [`Scripts::poll`]s.
#[test]
fn start_all_over_n_members_dispatches_at_most_the_permit_budget_in_the_click_frame() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-click", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }

    f.core.poll();
    f.scripts.start_all(&mut f.core, None);
    let starting = starting_among(&f, &names);
    let idle = idle_among(&f, &names);
    assert!(
        starting <= super::START_ADMIT_PER_FRAME,
        "Start all dispatched {starting} in the click frame (budget {})",
        super::START_ADMIT_PER_FRAME
    );
    assert!(f.scripts.starts_pending(), "Start all admitted no work");
    assert_eq!(shown_bulk(&f), "Start all: started 1, queued 4, skipped 0");
    assert_eq!(starting + idle.len(), names.len());
    let waiting = names
        .iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("a second member still waits for a permit");
    let brief = f
        .core
        .fleet_view()
        .row(waiting)
        .map(|row| row.brief())
        .unwrap_or("");
    assert!(
        brief.starts_with("queued "),
        "{waiting} should reuse the login-queue brief, got {brief:?}"
    );
    f.core.poll();
    f.scripts.poll(&mut f.core);
    let after_poll = shown_bulk(&f);
    assert!(
        after_poll.starts_with("Start all: started 2, queued 3, skipped 0"),
        "the later frame grants another waiting member: {after_poll}"
    );
    let notice = f.scripts.take_notice();
    let text = match notice.as_ref().and_then(Notice::text) {
        Some(text) => text,
        other => panic!("{other:?}"),
    };
    assert!(
        text.contains("queued"),
        "the Start all report names the waiting members: {text}"
    );

    f.settle();
    for name in &names {
        f.wait_state(name, ScriptRunState::Running);
        f.core.play().unwrap().script_stop(name);
    }
}

/// Stop all must drop queued-but-not-yet-admitted Starts so they never run.
#[test]
fn stop_all_cancels_queued_starts() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-stop", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }

    f.scripts.start_all(&mut f.core, None);
    let queued: Vec<String> = idle_among(&f, &names)
        .into_iter()
        .map(str::to_string)
        .collect();
    assert!(
        !queued.is_empty(),
        "need members still waiting for a permit"
    );

    f.scripts.stop_all(&mut f.core);
    for _ in 0..40 {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        std::thread::sleep(Duration::from_millis(5));
    }
    for name in &queued {
        assert_eq!(
            f.state(name),
            ScriptRunState::Idle,
            "{name} started after Stop all cancelled its place"
        );
    }
}

/// Re-assigning a member that still holds a Start-all place must not start
/// the card captured at the click.
#[test]
fn reassign_while_queued_does_not_start_stale_work() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-reassign", &name_refs);
    let first = f.card("first.ts", LOOPING);
    let second = f.card("second.ts", LOOPING);
    for name in &names {
        f.assign(name, &first);
    }

    f.scripts.start_all(&mut f.core, None);
    let queued = names
        .iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("need a member still waiting for a permit")
        .clone();
    f.assign(&queued, &second);

    f.settle();
    assert_eq!(
        f.state(&queued),
        ScriptRunState::Idle,
        "{queued} started after its assignment changed"
    );
    assert_ne!(
        f.core
            .play()
            .unwrap()
            .script_source_identity(&queued)
            .as_deref(),
        Some(first.identity_key().as_str())
    );
}

/// Once the permit has admitted a worker but before it publishes the Start,
/// a changed assignment or departing member must still fence the old card.
#[test]
fn prepared_start_rechecks_assignment_removal_and_logout_before_dispatch() {
    for (scenario, reason) in [
        ("reassigned", "reassigned"),
        ("removed", "removed"),
        ("logged-out", "logged out"),
    ] {
        let mut f = fixture(
            &format!("pace-inflight-{scenario}"),
            &["inflight-alice", "inflight-bob"],
        );
        let first = f.card("first.ts", LOOPING);
        let replacement = f.card("replacement.ts", LOOPING);
        for name in ["inflight-alice", "inflight-bob"] {
            f.assign(name, &first);
        }
        f.scripts.start_all(&mut f.core, None);
        assert!(
            f.scripts.start_queue_place("inflight-alice").is_none(),
            "first member already has the worker permit"
        );
        assert_eq!(f.state("inflight-alice"), ScriptRunState::Idle);
        match scenario {
            "reassigned" => f.assign("inflight-alice", &replacement),
            "removed" => {
                let mut surface = HeadlessSurface::new();
                f.core
                    .remove("inflight-alice", Instant::now(), &mut surface);
            }
            "logged-out" => f
                .core
                .play()
                .unwrap()
                .arm("inflight-alice")
                .unwrap()
                .request_logout(),
            _ => unreachable!(),
        }
        f.settle();
        let report = shown_bulk(&f);
        assert!(
            report.contains(&format!("inflight-alice: {reason}")),
            "{scenario}: {report}"
        );
        assert_eq!(start_accepted(&f, "inflight-alice"), 0, "{scenario}");
        assert_ne!(
            f.state("inflight-alice"),
            ScriptRunState::Running,
            "{scenario}: stale card was dispatched"
        );
        f.wait_state("inflight-bob", ScriptRunState::Running);
        f.core.play().unwrap().script_stop("inflight-bob");
    }
}

/// Removing a member that still holds a Start-all place must not start it.
#[test]
fn remove_while_queued_does_not_start_stale_work() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-remove", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }

    f.scripts.start_all(&mut f.core, None);
    let queued = names
        .iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("need a member still waiting for a permit")
        .clone();
    let mut surface = crate::surface::HeadlessSurface::new();
    f.core.remove(&queued, Instant::now(), &mut surface);

    f.settle();
    assert_eq!(
        start_accepted(&f, &queued),
        0,
        "frontend dispatched a Start for removed {queued}"
    );
    assert_ne!(
        f.state(&queued),
        ScriptRunState::Running,
        "{queued} started after it was removed from the wall"
    );
    let report = f
        .scripts
        .last_bulk_report()
        .expect("Start all report after a queued remove");
    assert!(
        report.contains(&format!("{queued}: removed")),
        "grant-time remove must name the member: {report}"
    );
    assert!(
        !report.contains("no slot"),
        "a wall-member refuse must not reach start_script: {report}"
    );
}

/// Operator logout while a member is still waiting for a Start-all permit
/// must not start that member.
#[test]
fn logout_while_queued_does_not_start_stale_work() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-logout", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }

    f.scripts.start_all(&mut f.core, None);
    let queued = names
        .iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("need a member still waiting for a permit")
        .clone();
    f.core
        .play()
        .unwrap()
        .arm(&queued)
        .expect("loaded member")
        .request_logout();

    f.settle();
    assert_eq!(
        f.state(&queued),
        ScriptRunState::Idle,
        "{queued} started after logout while queued"
    );
}

/// Single Start of a queued member bypasses the remaining permit wait.
#[test]
fn single_start_of_a_queued_member_is_immediate() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-single", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }

    f.scripts.start_all(&mut f.core, None);
    let queued = names
        .iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("need a member still waiting for a permit")
        .clone();
    f.scripts.start_profile(&mut f.core, &queued, None).unwrap();
    assert_eq!(
        f.state(&queued),
        ScriptRunState::Idle,
        "the single Start is accepted without waiting for another permit frame"
    );
    assert!(f.scripts.starts_pending());
    f.settle();
    f.wait_state(&queued, ScriptRunState::Running);
    assert_eq!(
        start_accepted(&f, &queued),
        1,
        "{queued} must receive exactly one Start"
    );
    let report = f
        .scripts
        .last_bulk_report()
        .expect("Start all report after a single-start handoff");
    assert!(
        report.starts_with("Start all: started 5, skipped 0"),
        "the handoff credits the member once: {report}"
    );
    f.core.play().unwrap().script_stop(&queued);
}

/// A marked Start with a missing card must not count the click as started,
/// and every later dispatch error must appear with its reason.
#[test]
fn marked_start_missing_card_reports_each_skip_as_grants_land() {
    let mut f = fixture("pace-marked-missing", &["alice", "bob", "carol"]);
    let gone = ScriptChoice::from_script(script::ScriptSel::Loaded(
        script::ScriptSource::File,
        "gone.ts".into(),
    ));
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&gone), None);
    let click_report = shown_bulk(&f);
    assert!(
        click_report.contains("queued") && !click_report.contains("started 3"),
        "click must not count waiting members as started: {click_report}"
    );
    f.settle();
    let report = shown_bulk(&f);
    assert!(
        report.starts_with("Start selected: started 0")
            && report.contains("alice: missing file: gone.ts")
            && report.contains("bob: missing file: gone.ts")
            && report.contains("carol: missing file: gone.ts"),
        "{report}"
    );
    for name in ["alice", "bob", "carol"] {
        assert_eq!(f.state(name), ScriptRunState::Idle, "{name}");
        assert_eq!(start_accepted(&f, name), 0, "{name} must not Start");
    }
}

#[test]
fn assigned_catalog_card_missing_from_loaded_catalog_names_scanned_root() {
    let mut f = fixture("assigned-catalog-missing", &["alice"]);
    let root = f.dir.join("catalog-root");
    let scripts = root.join("src/bot/scripts");
    let present_dir = scripts.join("PresentBot");
    std::fs::create_dir_all(&present_dir).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        "import PresentBot from './PresentBot/PresentBot.js';\n\
         ScriptRegistry.register({ name: 'PresentBot', create: () => new PresentBot() });\n",
    )
    .unwrap();
    std::fs::write(present_dir.join("PresentBot.ts"), LOOPING).unwrap();

    f.scripts.fill_catalog_once(Some(&root));
    f.settle_catalog_work();
    let present = f
        .scripts
        .js
        .get(script::ScriptSource::Catalog, "PresentBot")
        .expect("catalog root loaded its present card");
    let mut assignment = present.assignment();
    assignment.identity = "MissingBot".into();
    assignment.display_name = "MissingBot".into();
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", assignment));
    f.core.flush_writes();

    let error = f
        .scripts
        .start_profile(&mut f.core, "alice", Some(&root))
        .expect_err("a card absent from the loaded catalog must be refused");
    match (error.kind(), error.to_string()) {
        (ScriptStartErrorKind::Refused, message) => assert!(
            message.contains(&format!("catalog at {} was scanned", root.display()))
                && message.contains("MissingBot")
                && message.contains("missing"),
            "{message}"
        ),
        other => panic!("expected a refused start error, got {other:?}"),
    }
    assert_eq!(start_accepted(&f, "alice"), 0);
    assert_ne!(f.state("alice"), ScriptRunState::Running);
}

#[test]
fn failed_catalog_fill_does_not_latch_root_or_block_later_fill() {
    let mut f = fixture("catalog-fill-retry-after-registration-error", &["alice"]);
    let failed_root = f.dir.join("failed-catalog-root");
    let failed_scripts = failed_root.join("src/bot/scripts");
    std::fs::create_dir_all(&failed_scripts).unwrap();
    std::fs::write(failed_scripts.join("index.ts"), "const x = 1;\n").unwrap();

    f.scripts.fill_catalog_once(Some(&failed_root));
    f.settle_catalog_work();
    assert!(
        !f.scripts.catalog_filled,
        "a failed registry parse must not mark the catalog filled"
    );
    assert!(
        f.scripts.catalog_root.is_none(),
        "a failed registry parse must not be recorded as scanned"
    );

    let root = f.dir.join("catalog-root");
    let scripts = root.join("src/bot/scripts");
    let present_dir = scripts.join("PresentBot");
    std::fs::create_dir_all(&present_dir).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        "import PresentBot from './PresentBot/PresentBot.js';\n\
         ScriptRegistry.register({ name: 'PresentBot', create: () => new PresentBot() });\n",
    )
    .unwrap();
    std::fs::write(present_dir.join("PresentBot.ts"), LOOPING).unwrap();

    f.scripts.fill_catalog_once(Some(&root));
    f.settle_catalog_work();
    assert!(f.scripts.catalog_filled);
    assert_eq!(f.scripts.catalog_root.as_deref(), Some(root.as_path()));
    assert!(
        f.scripts
            .js
            .get(script::ScriptSource::Catalog, "PresentBot")
            .is_some(),
        "a later valid $RS2B0T root must still register its cards"
    );
}

#[test]
fn catalog_request_refusals_name_their_failure_kind() {
    let mut f = fixture("catalog-typed-errors", &["alice"]);
    let root = f.dir.join("catalog-root");
    let other = f.dir.join("other-catalog-root");
    let path_file = script::default_rs2b0t_path_file();
    f.scripts
        .request_catalog_import(&root, &path_file)
        .expect("first import queues");
    assert_eq!(
        f.scripts
            .request_catalog_import(&root, &path_file)
            .unwrap_err()
            .to_string(),
        "catalog preparation is already pending",
        "a second import while work is queued must report Busy"
    );
    assert_eq!(
        f.scripts
            .request_catalog_fill(&other, &path_file)
            .unwrap_err()
            .to_string(),
        "catalog preparation is already pending for another root",
        "a fill for another root while work is queued must report BusyForAnotherRoot"
    );
    f.settle_catalog_work();
    assert_eq!(
        f.scripts
            .request_catalog_fill(&other, &path_file)
            .unwrap_err()
            .to_string(),
        "the previous catalog import result has not been collected",
        "a fill must report UncollectedImportResult until the import receipt is taken"
    );
    assert!(
        f.scripts.take_catalog_import_result().is_some(),
        "the settled import receipt stays available as a typed result"
    );
}

#[test]
fn catalog_typed_failures_keep_their_byte_identical_display() {
    use script::load::{CatalogSnapshotError, CatalogWorkerError};

    let snapshots = [
        (
            CatalogSnapshotError::SnapshotTooLarge,
            "catalog snapshot is too large",
        ),
        (
            CatalogSnapshotError::RegistryIndexSizeOverflow,
            "catalog registry index size overflow",
        ),
        (
            CatalogSnapshotError::SourceEstimateTooLarge,
            "catalog source estimate is too large",
        ),
        (
            CatalogSnapshotError::CardMetadataTooLarge,
            "catalog card metadata is too large",
        ),
        (
            CatalogSnapshotError::CardMetadataExceedsSnapshotBound,
            "catalog card metadata exceeds the 8 MiB snapshot bound",
        ),
        (
            CatalogSnapshotError::SourceChargeOverflow,
            "catalog source charge overflow",
        ),
        (
            CatalogSnapshotError::SnapshotExceedsInputBound,
            "catalog snapshot exceeds the input charge bound",
        ),
        (
            CatalogSnapshotError::InputChargeOverflow,
            "catalog input charge overflow",
        ),
        (
            CatalogSnapshotError::ScratchChargeOverflow,
            "catalog scratch charge overflow",
        ),
        (
            CatalogSnapshotError::ResultChargeOverflow,
            "catalog result charge overflow",
        ),
    ];
    assert_eq!(snapshots.len(), 10, "every snapshot variant is covered");
    for (error, expected) in snapshots {
        let wrapped = CatalogError::from(error);
        assert_eq!(wrapped.to_string(), expected);
    }

    let workers = [
        (
            CatalogWorkerError::Registry {
                path: "/root/index.ts".into(),
                reason: "missing /root/index.ts: gone".into(),
            },
            "$RS2B0T registry /root/index.ts: missing /root/index.ts: gone",
        ),
        (
            CatalogWorkerError::TooManyRegistrations,
            "catalog has more than 4096 registrations",
        ),
        (
            CatalogWorkerError::RegistrySizeOverflow,
            "catalog registry size overflow",
        ),
        (
            CatalogWorkerError::RegistryMetadataExceedsInputBound,
            "catalog registry metadata exceeds its charged input bound",
        ),
        (
            CatalogWorkerError::InputsExceedInputBound,
            "catalog inputs exceed the charged input bound",
        ),
        (
            CatalogWorkerError::TooManySourceFiles,
            "catalog source graph exceeds 8192 files",
        ),
        (
            CatalogWorkerError::SourcesExceedInputBound,
            "catalog sources exceed the charged input bound",
        ),
        (
            CatalogWorkerError::SourcesExceedBoundedInputLimit,
            "catalog sources exceed the bounded input limit",
        ),
        (
            CatalogWorkerError::SourceFile {
                reason: "not a regular file".into(),
            },
            "not a regular file",
        ),
        (
            CatalogWorkerError::TooManySiblings {
                relative: "./src/bot/scripts/Big/Big.ts".into(),
            },
            "catalog source ./src/bot/scripts/Big/Big.ts has more than 128 sibling modules",
        ),
        (
            CatalogWorkerError::SiblingIndexSizeOverflow,
            "catalog sibling index size overflow",
        ),
        (
            CatalogWorkerError::ScratchExceedsBound,
            "catalog preparation scratch exceeds its charged scratch bound",
        ),
        (
            CatalogWorkerError::TooManySiblingModules,
            "catalog source graph exceeds 8192 sibling modules",
        ),
        (
            CatalogWorkerError::ParsedRegistrationMissing {
                name: "GoneBot".into(),
            },
            "catalog GoneBot: parsed registration is missing",
        ),
        (
            CatalogWorkerError::ExistingSnapshotMissing {
                name: "GoneBot".into(),
            },
            "catalog GoneBot: existing card snapshot is missing",
        ),
        (
            CatalogWorkerError::CardBounded {
                name: "GoneBot".into(),
                reason: "registry entry no longer resolves".into(),
            },
            "catalog GoneBot: registry entry no longer resolves",
        ),
        (
            CatalogWorkerError::FailureIndexSizeOverflow,
            "catalog failure index size overflow",
        ),
        (
            CatalogWorkerError::DiffCompletedMoreThanOnce,
            "catalog diff work completed more than once",
        ),
        (
            CatalogWorkerError::PreparationCompletedMoreThanOnce,
            "catalog preparation work completed more than once",
        ),
        (
            CatalogWorkerError::CandidateCountOverflow,
            "catalog candidate count overflow",
        ),
        (
            CatalogWorkerError::ResultSizeOverflow,
            "catalog result size overflow",
        ),
        (
            CatalogWorkerError::PreparedResultExceedsBound,
            "prepared catalog result exceeds its charged result bound",
        ),
        (
            CatalogWorkerError::PreparedCardSizeOverflow,
            "prepared catalog card size overflow",
        ),
        (
            CatalogWorkerError::FailureSizeOverflow,
            "catalog failure size overflow",
        ),
        (
            CatalogWorkerError::ResultSizeUnderflow,
            "catalog result size accounting underflow",
        ),
        (
            CatalogWorkerError::UnchangedCandidateIndexInvalid,
            "catalog unchanged candidate index is invalid",
        ),
        (
            CatalogWorkerError::ScratchSizeOverflow,
            "catalog scratch size overflow",
        ),
        (
            CatalogWorkerError::CardTaskAlreadyComplete,
            "card preparation task is already complete",
        ),
        (
            CatalogWorkerError::DiffSizeOverflow,
            "catalog diff size overflow",
        ),
        (
            CatalogWorkerError::DiffSizeUnderflow,
            "catalog diff size accounting underflow",
        ),
        (
            CatalogWorkerError::DiffExceedsResultBound,
            "catalog diff exceeds its charged result bound",
        ),
        (
            CatalogWorkerError::FingerprintInsertionDisappeared,
            "catalog fingerprint insertion disappeared",
        ),
        (
            CatalogWorkerError::IncomingInsertionDisappeared,
            "catalog incoming insertion disappeared",
        ),
        (
            CatalogWorkerError::DiffStateUnavailable,
            "catalog diff state is unavailable",
        ),
    ];
    assert_eq!(workers.len(), 34, "every worker variant is covered");
    for (error, expected) in workers {
        let wrapped = CatalogError::from(error);
        assert_eq!(wrapped.to_string(), expected);
    }
}

#[test]
fn loaded_start_preparation_bounds_pending_demands_and_preserves_accepted_work() {
    const LIMIT: usize = 64;
    let mut f = fixture("loaded-start-bound", &["alice"]);
    let root = f.dir.join("catalog-root");
    std::fs::create_dir_all(&root).unwrap();
    let selected = script::ScriptSel::Loaded(script::ScriptSource::Catalog, "BoneBurier".into());
    let accepted = (0..LIMIT)
        .map(|index| format!("profile-{index}"))
        .collect::<Vec<_>>();

    for profile in &accepted {
        f.scripts
            .queue_loaded_start(
                &f.core,
                profile,
                &selected,
                Some(&root),
                super::StartKind::Start,
                false,
            )
            .expect("demand within the pending bound");
    }

    f.scripts
        .queue_loaded_start(
            &f.core,
            &accepted[0],
            &selected,
            Some(&root),
            super::StartKind::Start,
            false,
        )
        .expect("a duplicate Start reuses its admitted preparation");
    assert!(matches!(
        f.scripts.queue_loaded_start(
            &f.core,
            "profile-over-limit",
            &selected,
            Some(&root),
            super::StartKind::Start,
            false,
        ),
        Err(script::StartLoadError::Refused(error))
            if error.contains("limit") && error.contains("64")
    ));
    for profile in &accepted {
        f.scripts
            .queue_loaded_start(
                &f.core,
                profile,
                &selected,
                Some(&root),
                super::StartKind::Start,
                false,
            )
            .expect("admitted Start demand is not replaced by a duplicate");
        assert!(
            f.scripts.has_start_preparation(profile),
            "accepted Start demand for {profile} must remain pending"
        );
    }

    for profile in accepted {
        f.scripts.cancel_start_preparation(&profile);
    }
    f.settle_catalog_work();
}

#[test]
fn marked_catalog_assignment_reports_pending_until_scan_settles() {
    let mut f = fixture("marked-catalog-assignment-pending", &["alice", "bob"]);
    let root = f.dir.join("catalog-root");
    let scripts = root.join("src/bot/scripts");
    let card_dir = scripts.join("BoneBurier");
    std::fs::create_dir_all(&card_dir).unwrap();
    std::fs::write(
        scripts.join("index.ts"),
        "import BoneBurier from './BoneBurier/BoneBurier.js';\n\
         ScriptRegistry.register({ name: 'BoneBurier', create: () => new BoneBurier() });\n",
    )
    .unwrap();
    std::fs::write(card_dir.join("BoneBurier.ts"), LOOPING).unwrap();
    let selection = mark_uids(&[1, 2]);
    let card = ScriptChoice::from_script(script::ScriptSel::Loaded(
        script::ScriptSource::Catalog,
        "BoneBurier".into(),
    ));

    let report =
        crate::marked::assign_marked(&selection, &mut f.core, &mut f.scripts, &card, Some(&root));
    assert_eq!(report.pending_count(), 2);
    assert_eq!(report.failed_count(), 0);
    assert!(
        report.summary().contains("pending 2")
            && report.summary().contains("alice: catalog scan pending"),
        "{}",
        report.summary()
    );
    assert!(report.rows().iter().all(|row| row.outcome().is_pending()));
    for profile in ["alice", "bob"] {
        assert_eq!(Scripts::assignment(&f.core, profile), None);
    }

    f.settle_catalog_work();
    for profile in ["alice", "bob"] {
        assert!(Scripts::assignment(&f.core, profile).is_some(), "{profile}");
    }
}

/// A member removed while waiting for a marked Start is named as skipped
/// and the frontend never dispatches a Start for it.
#[test]
fn marked_start_removed_member_is_named_and_never_dispatched() {
    let mut f = fixture("pace-marked-remove", &["alice", "bob", "carol"]);
    let card = f.card("loop.ts", LOOPING);
    for name in ["alice", "bob", "carol"] {
        f.assign(name, &card);
    }
    let sel = ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&sel), None);
    let click_report = shown_bulk(&f);
    assert!(
        click_report.contains("queued") && !click_report.contains("started 3"),
        "click must not count waiting members as started: {click_report}"
    );
    let queued = ["alice", "bob", "carol"]
        .into_iter()
        .find(|name| f.scripts.start_queue_place(name).is_some())
        .expect("need a member still waiting for a permit")
        .to_string();
    let mut surface = crate::surface::HeadlessSurface::new();
    f.core.remove(&queued, Instant::now(), &mut surface);
    f.settle();
    let report = shown_bulk(&f);
    assert!(
        report.contains(&format!("{queued}: removed")),
        "grant-time remove must name the member: {report}"
    );
    assert_eq!(start_accepted(&f, &queued), 0);
    assert_ne!(f.state(&queued), ScriptRunState::Running);
    for name in ["alice", "bob", "carol"] {
        if name != queued {
            f.wait_state(name, ScriptRunState::Running);
            f.core.play().unwrap().script_stop(name);
        }
    }
}

/// Start all while a marked batch is still waiting joins its report: the
/// waiting members are neither dropped from it nor counted twice.
#[test]
fn start_all_while_a_marked_batch_waits_joins_its_report() {
    let names = bulk_names(5);
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-overlap-marked", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }
    let sel = ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&sel), None);
    assert_eq!(
        shown_bulk(&f),
        "Start selected: started 1, queued 2, skipped 0"
    );
    f.scripts.start_all(&mut f.core, None);
    assert_eq!(
        shown_bulk(&f),
        "Start all: started 2, queued 3, skipped 0",
        "the marked rows still waiting stay counted, the started one is not skipped"
    );
    f.settle();
    assert_eq!(shown_bulk(&f), "Start all: started 5, skipped 0");
    for name in &names {
        f.wait_state(name, ScriptRunState::Running);
        assert_eq!(start_accepted(&f, name), 1, "{name}");
        f.core.play().unwrap().script_stop(name);
    }
}

fn slot_lines(slot: &str) -> Vec<(api::hostlog::Level, String)> {
    let mut view = crate::log::LogView::new(crate::log::LogScope::slot(slot));
    crate::log::global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|e| (e.level(), e.message().to_string()))
        .collect()
}

/// A second Start all while the first still waits must not lose the first
/// click's later failures: they stay on the report (failures before
/// skips), a setup failure reaches `take_start_failures`, and a dispatch
/// failure reaches the bot's log.
#[test]
fn start_all_twice_keeps_every_outcome_of_the_first_click() {
    let names: Vec<String> = (0..6).map(|i| format!("dbl-bot{i}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-overlap-twice", &name_refs);
    let good = f.card("loop.ts", LOOPING);
    let bad = f.card(
        "bad.ts",
        "export const apiVersion = 2;\nthrow new Error('setup-fails');\nexport function tick(api) {}\n",
    );
    let vanish = f.card("vanish.ts", LOOPING);
    for name in [&names[0], &names[1], &names[2], &names[5]] {
        f.assign(name, &good);
    }
    f.assign(&names[3], &bad);
    f.assign(&names[4], &vanish);
    f.start_running(&names[5]);
    std::fs::remove_file(f.dir.join("vanish.ts")).unwrap();

    f.scripts.start_all(&mut f.core, None);
    assert_eq!(
        shown_bulk(&f),
        "Start all: started 1, queued 4, skipped 1: dbl-bot5: already active"
    );
    f.scripts.start_all(&mut f.core, None);
    assert_eq!(
        shown_bulk(&f),
        "Start all: started 2, queued 3, skipped 1: dbl-bot5: already active",
        "a second click while the first still waits joins its report"
    );
    f.settle();
    let report = shown_bulk(&f);
    assert!(
        report.starts_with("Start all: started 3, skipped 1, failed 2: dbl-bot3: ")
            && report.contains("setup-fails")
            && report.contains("; dbl-bot4: missing file: ")
            && report.ends_with("; skipped dbl-bot5: already active"),
        "{report}"
    );
    let failures = f.scripts.take_start_failures();
    assert!(
        failures
            .iter()
            .any(|(name, message)| name == "dbl-bot3" && message.contains("setup-fails")),
        "{failures:?}"
    );
    let bot4_log = slot_lines("dbl-bot4");
    assert!(
        bot4_log.iter().any(|(level, line)| {
            *level == api::hostlog::Level::Error
                && line.starts_with("Start all failed: missing file: ")
        }),
        "{bot4_log:?}"
    );
    for (i, name) in names.iter().enumerate() {
        assert_eq!(start_accepted(&f, name), usize::from(i != 4), "{name}");
    }
    for name in [&names[0], &names[1], &names[2], &names[5]] {
        f.wait_state(name, ScriptRunState::Running);
        f.core.play().unwrap().script_stop(name);
    }
}

/// A file card whose file was deleted is refused as missing even when its
/// stored path still names it exactly (a canonical directory, as Linux temp
/// dirs are): Start never runs the copy loaded before the delete.
#[test]
fn a_deleted_file_card_is_refused_even_when_its_stored_path_matches() {
    let mut f = fixture("deleted-canonical", &["alice"]);
    let path = f.dir.canonicalize().unwrap().join("gone-later.ts");
    std::fs::write(&path, LOOPING).unwrap();
    let card = f.scripts.js_mut().load(&path).unwrap();
    assert_eq!(card.path, path, "the stored path is the canonical one");
    f.assign("alice", &card);
    std::fs::remove_file(&path).unwrap();

    let error = f
        .scripts
        .start_profile(&mut f.core, "alice", None)
        .unwrap_err();
    match (error.kind(), error.to_string()) {
        (ScriptStartErrorKind::Refused, message) => {
            assert!(message.starts_with("missing file: "), "{message}")
        }
        other => panic!("expected a refused start error, got {other:?}"),
    }
    assert_eq!(start_accepted(&f, "alice"), 0);
    assert_ne!(f.state("alice"), ScriptRunState::Running);
}

/// A bot logged out while it still waits after a second Start all is
/// named on the running report and in its own log, and never starts.
#[test]
fn logout_while_waiting_after_a_second_start_all_is_reported() {
    let names: Vec<String> = (0..4).map(|i| format!("lgo-bot{i}")).collect();
    let name_refs: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut f = fixture("pace-overlap-logout", &name_refs);
    let card = f.card("loop.ts", LOOPING);
    for name in &names {
        f.assign(name, &card);
    }
    f.scripts.start_all(&mut f.core, None);
    f.scripts.start_all(&mut f.core, None);
    let last = &names[3];
    assert!(f.scripts.cancel_queued_as(last, "logged out"));
    f.core.logout(last);
    f.settle();
    assert_eq!(
        shown_bulk(&f),
        "Start all: started 3, skipped 1: lgo-bot3: logged out"
    );
    assert!(
        slot_lines(last)
            .iter()
            .any(|(_, line)| line == "Start all skipped: logged out"),
        "{:?}",
        slot_lines(last)
    );
    assert_eq!(start_accepted(&f, last), 0);
    for name in &names[..3] {
        f.wait_state(name, ScriptRunState::Running);
        f.core.play().unwrap().script_stop(name);
    }
}

/// Stop on marked rows cancels the rows still waiting for their Start and
/// says so, instead of counting them as stopped.
#[test]
fn stop_on_marked_rows_reports_waiting_rows_as_cancelled() {
    let mut f = fixture("pace-stop-marked", &["alice", "bob", "carol"]);
    let card = f.card("loop.ts", LOOPING);
    for name in ["alice", "bob", "carol"] {
        f.assign(name, &card);
    }
    let sel = ScriptChoice::from_script(script::ScriptSel::Loaded(card.source, card.identity_id()));
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&sel), None);
    let stop = stop_marked(&selected, &mut f.core, &mut f.scripts);
    assert_eq!(
        stop.summary(),
        "Stop selected: stopped 0, cancelled 3, skipped 0"
    );
    let report = shown_bulk(&f);
    assert!(
        report.starts_with("Start selected: started 0, skipped 3"),
        "{report}"
    );
    for name in ["alice", "bob", "carol"] {
        assert!(report.contains(&format!("{name}: cancelled")), "{report}");
    }
    f.settle();
    for name in ["alice", "bob", "carol"] {
        assert_eq!(start_accepted(&f, name), 0, "{name}");
        assert_eq!(f.state(name), ScriptRunState::Idle, "{name}");
    }
}

/// A frozen command book with one latched process condition.
fn frozen_book(code: Option<ProcessServiceCode>) -> (CommandBook, ProcessConditions) {
    let incarnation = ProcessIncarnation::from_start_nonce(NonZeroU64::MIN);
    let mut conditions = ProcessConditions::new(incarnation);
    if let Some(code) = code {
        conditions
            .publish(ProcessServiceCondition::new(
                ConditionId::new(incarnation, code),
                ConditionCause::Panicked,
                ProcessTime::ZERO,
                None,
            ))
            .unwrap();
    }
    (CommandBook::new(), conditions)
}

/// R2-M1 against the real settings path (the review's fixture): a loaded
/// JS card's parameter save is staged at once and taken by the durable
/// writer, so its frozen row is admitted under a latched work-admission
/// outage and the writer's result settles it; the saved value is on disk.
#[test]
fn a_ready_js_settings_save_is_admitted_and_written_under_a_work_outage() {
    let mut f = fixture("frozen-js-save", &["alice"]);
    let thiever = f.card("thiever.ts", LOOPING);
    f.assign("alice", &thiever);
    let (mut book, conditions) = frozen_book(Some(ProcessServiceCode::WorkAdmissionUnavailable));
    let alice = Subject::Profile(ProfileIdentity::uid(1));
    let save = RequestIdMinter::new().mint().unwrap();
    book.open(
        save,
        OperatorCommand::SaveScriptSettings(CardKind::Script),
        &[(alice, Scope::Process)],
        &conditions,
    )
    .unwrap();
    assert_eq!(
        book.report(save).unwrap().member(alice).unwrap().pending(),
        Some(Pending::Accepted)
    );

    f.set("alice", &thiever, "target", json!("Guard"));
    f.core.flush_writes();
    let writes = f.core.take_settings_writes();
    assert_eq!(writes.len(), 1);
    assert_eq!(
        writes[0].result,
        super::SettingsResult::saved(super::LiveDelivery::NotRunning)
    );
    assert_eq!(
        f.saved_bag("alice", &thiever.identity_key()),
        Some(bag(&[("target", json!("Guard"))]))
    );
    // The outage never touched the member; the writer's result settles it.
    assert_eq!(book.fail_unavailable(&conditions), 0);
    assert_eq!(book.blocked_by(save, alice, &conditions), None);
    book.settle(save, alice, Settlement::Completed(Completion::Done), None)
        .unwrap();
    assert!(book.report(save).unwrap().is_settled());
}

/// A native card's parameter save is a preparation job first
/// (`queue_native_settings` → `script_prepare_config`), then the durable
/// write: its frozen row needs the background service until the
/// preparation staged the row, and only the writer afterwards.
#[test]
fn a_native_settings_save_needs_the_background_service_only_until_prepared() {
    let mut f = native_fixture("frozen-native-save", &["alice"]);
    let id = script::CompiledId("Gatherer");
    let key = script::compiled_identity_key(id);
    assert!(f
        .scripts
        .persist_assignment(&mut f.core, "alice", script::compiled_assignment(id)));
    f.core.flush_writes();
    let (mut book, mut conditions) = frozen_book(None);
    let alice = Subject::Profile(ProfileIdentity::uid(1));
    let requests = RequestIdMinter::new();
    let save = requests.mint().unwrap();
    book.open(
        save,
        OperatorCommand::SaveScriptSettings(CardKind::Native),
        &[(alice, Scope::Process)],
        &conditions,
    )
    .unwrap();
    let member = |book: &CommandBook| book.report(save).unwrap().member(alice).unwrap().stage();
    assert_eq!(member(&book), Stage::Admitted);

    // Hold the writer: the preparation stages the row, nothing is durable.
    let gate = f.core.write_gate();
    let held = gate.lock().unwrap();
    f.scripts
        .set_compiled_overrides(
            &mut f.core,
            "alice",
            CompiledCardId::from_script(id),
            bag(&[("targetPreference", json!("Nearest"))]),
        )
        .unwrap();
    let staged_value = |f: &Fixture| {
        f.core
            .vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .and_then(|entry| vault::CompiledSettingsRecord::view(entry).ok())
            .map(|(_, values)| values.get("targetPreference").cloned())
    };
    f.pump_until("native preparation did not settle", |f| {
        staged_value(f) == Some(Some(json!("Nearest")))
    });
    assert_eq!(f.saved_bag("alice", &key), None, "nothing durable yet");
    // The preparation owner reports; the member no longer needs the service.
    book.prepared(save, alice).unwrap();
    assert_eq!(member(&book), Stage::Prepared);
    let incarnation = ProcessIncarnation::from_start_nonce(NonZeroU64::MIN);
    let work = ConditionId::new(incarnation, ProcessServiceCode::WorkAdmissionUnavailable);
    conditions
        .publish(ProcessServiceCondition::new(
            work,
            ConditionCause::Panicked,
            ProcessTime::ZERO,
            None,
        ))
        .unwrap();
    assert_eq!(book.fail_unavailable(&conditions), 0);
    assert_eq!(book.blocked_by(save, alice, &conditions), None);
    // A second native save is refused now; the prepared one still writes.
    let refused = requests.mint().unwrap();
    book.open(
        refused,
        OperatorCommand::SaveScriptSettings(CardKind::Native),
        &[(alice, Scope::Process)],
        &conditions,
    )
    .unwrap();
    assert!(matches!(
        book.report(refused)
            .unwrap()
            .member(alice)
            .unwrap()
            .terminal(),
        Some(Settlement::Failed(CommandFailure::Refused {
            capability: ProcessCapability::BackgroundWork,
            ..
        }))
    ));
    drop(held);
    f.core.flush_writes();
    f.core.poll();
    let writes = f.core.take_settings_writes();
    assert_eq!(writes.len(), 1, "{writes:?}");
    assert!(writes[0].result.is_saved());
    let saved = f.saved_bag("alice", &key).unwrap();
    let (_, values) = vault::CompiledSettingsRecord::view(&saved).unwrap();
    assert_eq!(values["targetPreference"], json!("Nearest"));
    book.settle(save, alice, Settlement::Completed(Completion::Done), None)
        .unwrap();
    assert!(book.report(save).unwrap().is_settled());
}

#[test]
fn assignment_has_sel_agrees_with_sel_from_assignment() {
    for (source_kind, identity) in [
        ("compiled", "Gatherer"),
        ("compiled", "no-such-card"),
        ("file", "looper.ts"),
        ("catalog", "Thiever"),
        ("bogus", "looper.ts"),
        ("", ""),
    ] {
        let assignment = vault::ScriptAssignment {
            source_kind: source_kind.into(),
            identity: identity.into(),
            ..vault::ScriptAssignment::default()
        };
        assert_eq!(
            super::assignment_has_sel(&assignment),
            super::sel_from_assignment(&assignment).is_some(),
            "{source_kind}:{identity}"
        );
    }
}
