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
            "Start all: started 1, skipped 1, failed 1: carol: no assignment; skipped bob: already active"
                .into(),
        ))
    );
    f.settle();
    f.wait_state("alice", script::RunState::Running);

    // A reload shape on bob: its run is reaping with a replacement queued.
    f.core.stop_script("bob");
    let sel = script::ScriptSel::Loaded(thiever.source, thiever.identity_id());
    f.scripts
        .start_sel(
            &mut f.core,
            "bob",
            sel,
            None,
            super::StartKind::Reload,
            false,
        )
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
    f.wait_state("bob", script::RunState::Running);
    f.wait_state("carol", script::RunState::Running);
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
    assert_eq!((stop.stopped, stop.cancelled), (3, 0));
    let duplicate_stop = stop_marked(&selected, &mut f.core, &mut f.scripts);
    assert_eq!(duplicate_stop.stopped, 0);
    assert!(duplicate_stop
        .skipped
        .iter()
        .any(|skip| skip.reason.contains("stopping")));
}

fn native_fixture(test: &str, members: &[&str]) -> Fixture {
    let mut fixture = fixture(test, members);
    let data = api::game_data::for_revision(api::selected::ClientRevision::R289).unwrap();
    fixture.core.play_mut().unwrap().bind_script_test_data(data);
    fixture
}

fn start_sherlock(f: &mut Fixture, name: &str) {
    f.scripts
        .start_sel(
            &mut f.core,
            name,
            script::ScriptSel::Compiled(script::CompiledId("Sherlock")),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state(name), script::RunState::Running);
}

fn save_native_partner(f: &mut Fixture, name: &str, partner: &str) {
    let mut row = f.core.profile_for_edit(name).unwrap().clone();
    row.settings.clue_duel_partner = partner.into();
    let live = f
        .scripts
        .profile_save_live(&f.core, name, &row.settings)
        .unwrap();
    f.core
        .save_profile(row, crate::ArmMirror::Remember(Some(live)), "partner")
        .unwrap();
}

#[test]
fn compiled_restore_distinguishes_known_empty_from_unknown_or_malformed_schema() {
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
            .save_profile(row, crate::ArmMirror::None, "restore")
            .unwrap();
        f.core.flush_writes();
        assert!(matches!(
            f.scripts.compiled_schema(&f.core, Some("alice"), id),
            super::SchemaView::Ready { fields: [], .. }
        ));
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
            .save_profile(row, crate::ArmMirror::None, "restore")
            .unwrap();
        f.core.flush_writes();
        assert!(matches!(
            f.scripts.compiled_schema(&f.core, Some("alice"), id),
            super::SchemaView::Unavailable(_)
        ));
        assert!(f.scripts.compiled_bag(&f.core, "alice", id).is_err());
        assert!(f
            .scripts
            .set_compiled_overrides(&mut f.core, "alice", id, Map::new())
            .is_err());
        assert_eq!(
            f.saved_bag("alice", &key),
            Some(entry.as_object().unwrap().clone())
        );
    }
    assert!(matches!(
        f.scripts
            .compiled_schema(&f.core, None, script::CompiledId("missing")),
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
        .set_compiled_overrides(&mut f.core, "alice", id, bag(&[("unknown", json!(1))]))
        .unwrap();
    f.core.flush_writes();
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite {
            result: super::SettingsResult::Failed(_),
            ..
        }]
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
            script::ScriptSel::Compiled(id),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state("alice"), script::RunState::Idle);
    assert_eq!(
        Scripts::assignment(&f.core, "alice"),
        Some(previous.assignment())
    );
    assert!(f.core.play().unwrap().script_last_error("alice").is_some());
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
            .compiled_bag(&f.core, "alice", script::CompiledId("Sherlock"))
            .unwrap()[script::CLUE_DUEL_PARTNER],
        "Bob"
    );
    assert_eq!(
        f.scripts
            .compiled_bag(&f.core, "bob", script::CompiledId("Sherlock"))
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
            script::CompiledId("Sherlock"),
            Map::new(),
        )
        .unwrap();
    f.core.vault_remove("alice").unwrap();
    f.core
        .create_profile(replacement, crate::ArmMirror::None, "recreate")
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
            .save_profile(row, crate::ArmMirror::Remember(Some(live)), "form")
            .unwrap();
        f.core.stop_script("alice");
        if restart {
            f.core
                .start_script(
                    "alice",
                    crate::ScriptStart::Compiled {
                        id: script::CompiledId("Sherlock"),
                        bag: Map::new(),
                    },
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
            op: saved_op, result: super::SettingsResult::Saved(super::LiveDelivery::Stale), ..
        }] if *saved_op == op)
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
                    script::ScriptSel::Compiled(id),
                    None,
                    super::StartKind::Start,
                    false,
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while f.state("alice") != script::RunState::Running {
                assert!(Instant::now() < deadline);
                f.core.poll();
                std::thread::yield_now();
            }
        }
        f.scripts
            .set_compiled_overrides(&mut f.core, "alice", id, bag(&[("unknown", json!(1))]))
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
            .any(|error| error.contains("unknown")));
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
                    script::ScriptSel::Compiled(id),
                    None,
                    super::StartKind::Start,
                    false,
                )
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while f.state("alice") != script::RunState::Running {
                assert!(Instant::now() < deadline);
                f.core.poll();
                std::thread::yield_now();
            }
        }
        let op = f
            .scripts
            .set_compiled_overrides(
                &mut f.core,
                "alice",
                id,
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
        assert!(
            f.core
                .take_settings_writes()
                .iter()
                .any(|write| write.op == op
                    && matches!(write.result, super::SettingsResult::Saved(_)))
        );
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
        .save_profile(row, crate::ArmMirror::Remember(Some(live)), "form")
        .unwrap();
    f.core.set_auto_login("alice", true).unwrap();
    f.core.flush_writes();
    assert_eq!(f.core.play().unwrap().script_native_run("alice"), run);
    assert!(f
        .core
        .take_settings_writes()
        .iter()
        .any(|write| write.op == op
            && matches!(
                write.result,
                super::SettingsResult::Saved(super::LiveDelivery::Applied)
            )));
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
        .save_profile(row, crate::ArmMirror::Remember(Some(live)), "form")
        .unwrap();
    f.core.flush_writes();
    let disk = Vault::unlock(&f.dir.join("vault"), "test-passphrase-01").unwrap();
    let saved = disk.get("alice").unwrap();
    assert_eq!(saved.password, "new-password");
    assert_eq!(saved.settings.clue_duel_partner, "Bob");
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite {
            result: super::SettingsResult::Saved(super::LiveDelivery::Rejected(_)),
            ..
        }]
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
            script::CompiledId("Sherlock"),
            bag(&[(script::CLUE_DUEL_PARTNER, json!("Changed"))]),
        )
        .unwrap();
    let key = script::compiled_identity_key(script::CompiledId("Sherlock"));
    let deadline = Instant::now() + Duration::from_secs(10);
    while f
        .core
        .vault()
        .unwrap()
        .get("alice")
        .unwrap()
        .settings
        .script_settings[&key]["values"]
        .get(script::CLUE_DUEL_PARTNER)
        .is_none()
    {
        assert!(Instant::now() < deadline, "validated edit was never staged");
        f.core.poll();
        std::thread::yield_now();
    }
    assert_eq!(f.saved_bag("alice", &key).unwrap()["values"], json!({}));
    f.core.stop_script("alice");
    f.scripts.start_profile(&mut f.core, "alice", None).unwrap();
    f.settle();
    assert_ne!(f.core.play().unwrap().script_native_run("alice"), Some(old));
    drop(held);
    f.core.flush_writes();
    assert!(matches!(
        f.core.take_settings_writes().as_slice(),
        [super::SettingsWrite {
            result: super::SettingsResult::Saved(super::LiveDelivery::Stale),
            ..
        }]
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
            script::ScriptSel::Compiled(script::CompiledId("Sherlock")),
            None,
            super::StartKind::Start,
            false,
        )
        .unwrap();
    f.core.vault_remove("alice").unwrap();
    f.settle();
    f.core.flush_writes();
    assert_eq!(f.state("alice"), script::RunState::Idle);
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
        .prepare_compiled_settings_sync(&f.core, "alice", id, Some(script::CLUE_DUEL_PARTNER))
        .is_err());
    let scope = f
        .scripts
        .prepare_compiled_settings_sync(&f.core, "alice", id, None)
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

fn bulk_names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("bot{i}")).collect()
}

fn start_accepted(f: &Fixture, name: &str) -> usize {
    (1..)
        .map_while(|id| f.core.operation(crate::OperationId(id)))
        .filter(|report| {
            report.action == crate::ActionKind::ScriptStart && report.outcome(name).is_some()
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
        .filter(|name| f.state(name) == script::RunState::Starting)
        .count()
}

fn idle_among<'a>(f: &Fixture, names: &'a [String]) -> Vec<&'a str> {
    names
        .iter()
        .filter(|name| f.state(name) == script::RunState::Idle)
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
    assert!(starting > 0, "Start all granted nobody in the click frame");
    assert_eq!(starting + idle.len(), names.len());
    if let Some(name) = idle.first() {
        let brief = f
            .core
            .fleet_view()
            .row(name)
            .map(|row| row.brief.clone())
            .unwrap_or_default();
        assert!(
            brief.starts_with("queued "),
            "{name} should reuse the login-queue brief, got {brief:?}"
        );
    }
    f.core.poll();
    f.scripts.poll(&mut f.core);
    let admitted = names
        .iter()
        .filter(|name| f.state(name) != script::RunState::Idle)
        .count();
    assert!(
        admitted > starting,
        "later frames must admit waiting members: click={starting} after_poll={admitted}"
    );
    assert!(
        admitted <= super::START_ADMIT_PER_FRAME * 2,
        "the second frame exceeded the permit budget: {admitted}"
    );
    let notice = f.scripts.take_notice();
    let text = match &notice {
        Some(Notice::Show(text)) => text.as_str(),
        other => panic!("{other:?}"),
    };
    assert!(
        text.contains("queued"),
        "the Start all report names the waiting members: {text}"
    );

    f.settle();
    for name in &names {
        f.wait_state(name, script::RunState::Running);
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
            script::RunState::Idle,
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
    let queued = idle_among(&f, &names)
        .first()
        .copied()
        .expect("need a queued member")
        .to_string();
    f.assign(&queued, &second);

    f.settle();
    assert_eq!(
        f.state(&queued),
        script::RunState::Idle,
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
    let queued = idle_among(&f, &names)
        .first()
        .copied()
        .expect("need a queued member")
        .to_string();
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
        script::RunState::Running,
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
    let queued = idle_among(&f, &names)
        .first()
        .copied()
        .expect("need a queued member")
        .to_string();
    f.core
        .play()
        .unwrap()
        .arm(&queued)
        .expect("loaded member")
        .request_logout();

    f.settle();
    assert_eq!(
        f.state(&queued),
        script::RunState::Idle,
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
    let queued = idle_among(&f, &names)
        .first()
        .copied()
        .expect("need a queued member")
        .to_string();
    f.scripts.start_profile(&mut f.core, &queued, None).unwrap();
    assert_eq!(
        f.state(&queued),
        script::RunState::Starting,
        "{queued} should start without waiting for later frames"
    );
    f.settle();
    f.wait_state(&queued, script::RunState::Running);
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
    let gone = script::ScriptSel::Loaded(script::ScriptSource::File, "gone.ts".into());
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
        assert_eq!(f.state(name), script::RunState::Idle, "{name}");
        assert_eq!(start_accepted(&f, name), 0, "{name} must not Start");
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
    let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&sel), None);
    let click_report = shown_bulk(&f);
    assert!(
        click_report.contains("queued") && !click_report.contains("started 3"),
        "click must not count waiting members as started: {click_report}"
    );
    let queued = ["alice", "bob", "carol"]
        .into_iter()
        .find(|name| f.state(name) == script::RunState::Idle)
        .expect("need a queued member")
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
    assert_ne!(f.state(&queued), script::RunState::Running);
    for name in ["alice", "bob", "carol"] {
        if name != queued {
            f.wait_state(name, script::RunState::Running);
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
    let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
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
        f.wait_state(name, script::RunState::Running);
        assert_eq!(start_accepted(&f, name), 1, "{name}");
        f.core.play().unwrap().script_stop(name);
    }
}

fn slot_lines(slot: &str) -> Vec<(api::hostlog::Level, String)> {
    let mut view = crate::log::LogView::new(crate::log::LogScope::Slot(slot.into()));
    crate::log::global().refresh(&mut view);
    view.rows()
        .iter()
        .map(|e| (e.level, e.message.to_string()))
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
        f.wait_state(name, script::RunState::Running);
        f.core.play().unwrap().script_stop(name);
    }
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
        f.wait_state(name, script::RunState::Running);
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
    let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
    let selected = mark_uids(&[1, 2, 3]);
    start_marked(&selected, &mut f.core, &mut f.scripts, Some(&sel), None);
    let stop = stop_marked(&selected, &mut f.core, &mut f.scripts);
    assert_eq!(
        stop.summary(),
        "Stop selected: stopped 1, cancelled 2, skipped 0"
    );
    assert_eq!(
        shown_bulk(&f),
        "Start selected: started 1, skipped 2: bob: cancelled, carol: cancelled"
    );
    f.settle();
    for name in ["bob", "carol"] {
        assert_eq!(start_accepted(&f, name), 0, "{name}");
        assert_eq!(f.state(name), script::RunState::Idle, "{name}");
    }
}
