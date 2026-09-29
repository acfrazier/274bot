//! Script coordination through the real host seam: a real
//! [`host_play::Play`] (no server contact), real script slots and the real
//! profile writer over a temp vault.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{Notice, Scripts};
use crate::operations::Outcome;
use crate::selection::{start_marked, stop_marked, MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;
use crate::surface::HeadlessSurface;
use host_play::{InstancePermit, Play, PlayOptions};
use serde_json::{json, Map, Value};

use vault::{Profile, ProfileSettings, Vault};
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
fn browse_card_list_includes_compiled_cards() {
    let f = fixture("compiled-card-list", &["alice"]);
    let cards: Vec<_> = f.scripts.browse_cards().collect();
    let sherlock = cards
        .iter()
        .find(|card| card.name() == "Sherlock")
        .expect("load builds expose Sherlock to Browse");
    assert_eq!(sherlock.kind(), script::ScriptKind::Compiled);
    assert_eq!(sherlock.category(), "Treasure Trails");
    assert_eq!(
        sherlock.selection(),
        script::ScriptSel::Compiled(script::CompiledId("Sherlock"))
    );
}

#[test]
fn merged_profile_bag_carries_profile_global_clue_partner() {
    let mut f = fixture("clue-global", &["alice"]);
    let mut profile = f.core.vault().unwrap().get("alice").unwrap().clone();
    profile.settings.clue_duel_partner = "Helper".into();
    f.core
        .save_profile(profile, crate::ArmMirror::Remember(None), "clue-global")
        .unwrap();
    f.core.flush_writes();
    let card = f.card("clue.ts", LOOPING);

    let merged = f.scripts.merged_profile_bag(
        &mut f.core,
        "alice",
        card.source,
        &card.name,
        &card.path,
        &card.settings_schema,
    );
    assert_eq!(merged.get("clueDuelPartner"), Some(&json!("Helper")));

    f.scripts.inject = Some(bag(&[("clueDuelPartner", json!("Harness"))]));
    let injected = f.scripts.merged_profile_bag(
        &mut f.core,
        "alice",
        card.source,
        &card.name,
        &card.path,
        &card.settings_schema,
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

/// Apply to all posts the whole bag to each live target: its own
/// profile-global clue duel partner survives the card-parameter copy.
#[test]
fn apply_to_all_keeps_each_live_targets_global_clue_partner() {
    let mut f = fixture("sync-partner", &["alice", "bob", "dave"]);
    for (name, partner) in [("bob", "Helper"), ("dave", "Second")] {
        let mut profile = f.core.vault().unwrap().get(name).unwrap().clone();
        profile.settings.clue_duel_partner = partner.into();
        f.core
            .save_profile(profile, crate::ArmMirror::Remember(None), "partner")
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
        .save_profile(profile, crate::ArmMirror::Remember(live), "credentials")
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
            .save_profile(profile, crate::ArmMirror::Remember(live), "credentials")
            .unwrap()
    });
    drop(held);
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let outcome = |op| f.core.operation(op).unwrap().outcome("alice").cloned();
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
        f.core.set_auto_login("bob", true).unwrap();
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
        Some(Outcome::Completed)
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
    assert!(f.scripts.js.load_failure(&card.identity_key()).is_some());
    std::fs::write(f.dir.join("gate.ts"), "export const fail = false;").unwrap();
    f.scripts.take_notice();

    f.scripts.start_all(&mut f.core, None);
    let report = Notice::Show("Start all: started 1, skipped 0".into());
    assert_eq!(f.scripts.take_notice(), Some(report));
    f.settle();
    f.core.flush_writes();
    assert!(f.scripts.js.load_failure(&card.identity_key()).is_none());
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
        let deadline = Instant::now() + Duration::from_secs(10);
        while f.scripts.starts_pending() && Instant::now() < deadline {
            f.core.poll();
            f.scripts.poll(&mut f.core);
            if let Some(notice) = f.scripts.take_notice() {
                notice.apply(&mut banner);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!f.scripts.starts_pending());
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
    let sel = |card: &script::JsCard| script::ScriptSel::Loaded(card.source, card.identity_id());
    f.scripts.set_pending_browse("opid-alice", sel(&good));
    f.scripts.set_pending_browse("opid-bob", sel(&bad));
    let op_of = |f: &Fixture, name: &str| {
        f.core
            .fleet_view()
            .row(name)
            .and_then(|row| row.last_op.clone())
            .expect(name)
    };

    f.scripts
        .start_selected(&mut f.core, "opid-alice", None, None)
        .unwrap();
    f.scripts
        .start_selected(&mut f.core, "opid-bob", None, None)
        .unwrap();
    f.core.poll();
    let accepted = op_of(&f, "opid-alice");
    // Accepted (it may already have settled if the isolate was quick).
    assert_eq!(accepted.action, crate::ActionKind::ScriptStart);
    f.settle();
    f.core.poll();

    let alice = op_of(&f, "opid-alice");
    assert_eq!(
        (alice.id, &alice.outcome),
        (accepted.id, &Outcome::Completed)
    );
    let bob = op_of(&f, "opid-bob");
    assert!(
        matches!(&bob.outcome, Outcome::Failed(reason) if reason.contains("setup-fails")),
        "{bob:?}"
    );
    let failed = f.core.fleet_view().row("opid-bob").unwrap().has_failure();
    assert!(failed, "a failed Start is a visible row failure");

    let lines = |slot: &str| {
        let mut view = crate::log::LogView::new(crate::log::LogScope::Slot(slot.into()));
        crate::log::global().refresh(&mut view);
        view.rows()
            .iter()
            .map(|e| (e.level, e.message.to_string()))
            .collect::<Vec<_>>()
    };
    let alice_log = lines("opid-alice");
    let at = |text: String| alice_log.iter().position(|(_, line)| *line == text);
    let (start, done) = (
        at(format!("op#{} Start accepted", alice.id.0)),
        at(format!("op#{} Start completed", alice.id.0)),
    );
    assert!(start.is_some() && start < done, "{alice_log:?}");
    let prefix = format!("op#{} Start failed: ", bob.id.0);
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
    let card_sel = script::ScriptSel::Loaded(card.source, card.identity_id());
    let first = start_marked(
        &selected,
        &mut f.core,
        &mut f.scripts,
        Some(&card_sel),
        None,
    );
    assert_eq!(first.affected, 2);
    assert_eq!(first.skipped.len(), 1);
    assert_eq!(first.skipped[0].profile, "alice");
    assert!(first.skipped[0].reason.contains("active"));

    f.settle();
    f.wait_state("bob", script::RunState::Running);
    f.wait_state("carol", script::RunState::Running);

    let duplicate = start_marked(
        &selected,
        &mut f.core,
        &mut f.scripts,
        Some(&card_sel),
        None,
    );
    assert_eq!(duplicate.affected, 0);
    assert_eq!(duplicate.skipped.len(), 3);
    assert!(duplicate
        .skipped
        .iter()
        .all(|skip| skip.reason.contains("active")));

    let stop = stop_marked(&selected, &mut f.core);
    assert_eq!(stop.affected, 3);
    let duplicate_stop = stop_marked(&selected, &mut f.core);
    assert_eq!(duplicate_stop.affected, 0);
    assert!(duplicate_stop
        .skipped
        .iter()
        .any(|skip| skip.reason.contains("stopping")));
}
