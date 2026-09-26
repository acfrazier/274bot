//! Script coordination through the real host seam: a real
//! [`host_play::Play`] (no server contact), real script slots and the real
//! profile writer over a temp vault.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, PlayOptions};
use serde_json::{json, Map, Value};
use vault::{Profile, ProfileSettings, Vault};

use super::{Notice, Scripts};
use crate::operations::Outcome;
use crate::session::OperatorSession;
use crate::surface::HeadlessSurface;

const LOOPING: &str = "export default class T extends LoopingBot { override loop() {} }\n";

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
    let mut vault = Vault::create(&dir.join("vault"), "bot").unwrap();
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

impl Fixture {
    fn card(&mut self, file: &str, src: &str) -> script::JsCard {
        let path = self.dir.join(file);
        std::fs::write(&path, src).unwrap();
        self.scripts.js.load(&path).unwrap()
    }

    fn assign(&mut self, name: &str, card: &script::JsCard) {
        assert!(self
            .scripts
            .persist_assignment(&mut self.core, name, card.assignment()));
        self.core.flush_writes();
    }

    /// Poll the core and the coordinator until no Start is pending.
    fn settle(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.scripts.starts_pending() && Instant::now() < deadline {
            self.core.poll();
            self.scripts.poll(&mut self.core);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(
            !self.scripts.starts_pending(),
            "script Start did not settle"
        );
    }

    fn wait_state(&mut self, name: &str, want: script::RunState) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.state(name) != want && Instant::now() < deadline {
            self.core.poll();
            self.scripts.poll(&mut self.core);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(self.state(name), want, "{name}");
    }

    fn state(&self, name: &str) -> script::RunState {
        self.core.play().unwrap().script_state(name)
    }

    fn start_running(&mut self, name: &str) {
        self.scripts
            .start_profile(&mut self.core, name, None)
            .unwrap();
        self.settle();
        self.wait_state(name, script::RunState::Running);
    }

    fn generation(&self, name: &str) -> u64 {
        self.core
            .play()
            .unwrap()
            .script_runtime_generation(name)
            .unwrap()
    }

    fn saved_bag(&self, name: &str, key: &str) -> Option<Map<String, Value>> {
        let disk = Vault::unlock(&self.dir.join("vault"), "bot").unwrap();
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
                card.source,
                &card.name,
                &card.path,
                id,
                value,
            )
            .unwrap();
    }

    fn prepare(&mut self, source: &str, card: &script::JsCard) {
        self.scripts.prepare_settings_sync(
            &mut self.core,
            source,
            card.source,
            &card.name,
            &card.path,
        );
    }

    /// Posting `bag` again is refused when the run already has it.
    fn run_has_bag(&self, name: &str, bag: &Map<String, Value>) -> bool {
        let play = self.core.play().unwrap();
        let identity = play.script_source_identity(name).unwrap();
        !play.script_post_settings_fenced(name, bag, &identity, self.generation(name))
    }
}

fn bag(pairs: &[(&str, Value)]) -> Map<String, Value> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), v.clone()))
        .collect()
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
        (report.saved, report.failed.len(), report.skipped.len()),
        (2, 0, 1)
    );
    assert_eq!((report.delivered, report.not_running), (1, 1));
    let operation = f.core.operation(op).unwrap();
    assert!(matches!(
        operation.outcome("carol"),
        Some(Outcome::Skipped(_))
    ));
    assert_eq!(operation.outcome("bob"), Some(&Outcome::Completed));
    assert_eq!(operation.outcome("dave"), Some(&Outcome::Completed));
    assert!(f.run_has_bag("bob", &want), "bob's run received the bag");
    assert!(
        !f.run_has_bag("carol", &want),
        "carol's run of another card was never posted"
    );
    let summary = report.summary().to_string();
    assert_eq!(f.scripts.take_notice(), Some(Notice::Show(summary)));
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
    // The writer's temp file cannot be created where a directory sits.
    let blocker = f.dir.join("vault").with_extension("tmp");
    std::fs::create_dir_all(&blocker).unwrap();

    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.set("bob", &thiever, "food", json!("Lobster"));
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    std::fs::remove_dir_all(&blocker).unwrap();

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
    f.wait_state("bob", script::RunState::Idle);
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
    let sel = |card: &script::JsCard| script::ScriptSel::Loaded(card.source, card.identity_id());
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
        Some(Notice::Show(
            "Start all: started 1, skipped 1, failed 1: carol: no assignment".into()
        ))
    );
    f.settle();
    f.wait_state("alice", script::RunState::Running);

    // A reload shape on bob: its run is reaping with a replacement queued.
    f.core.stop_script("bob");
    let sel = script::ScriptSel::Loaded(thiever.source, thiever.identity_id());
    f.scripts
        .start_sel(&mut f.core, "bob", sel, None, super::StartKind::Reload)
        .unwrap();
    assert_eq!(f.state("bob"), script::RunState::Stopping);
    assert_eq!(f.scripts.stop_all(&mut f.core), 2);
    for name in ["alice", "bob"] {
        f.wait_state(name, script::RunState::Idle);
    }
    for _ in 0..20 {
        f.core.poll();
        f.scripts.poll(&mut f.core);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        f.state("bob"),
        script::RunState::Idle,
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
        super::SettingsResult::Saved(super::LiveDelivery::Delivered)
    );
    assert!(f.run_has_bag("alice", &bag(&[("target", json!("Guard"))])));
}
