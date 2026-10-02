//! Fleet-window commands through the real host seam: a real
//! [`host_play::Play`] (no server contact), real script slots and the real
//! profile writer over a temp vault.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use host_play::{InstancePermit, Play, PlayOptions, SlotStatus};
use vault::{Profile, ProfileSettings, Vault};

use super::{
    assign_and_restart_marked, assign_marked, login_marked, logout_marked,
    prepare_apply_settings_marked, restart_scope,
};
use crate::scripts::Scripts;
use crate::selection::{start_marked, MarkedSelection, ProfileIdentity};
use crate::session::OperatorSession;
use crate::surface::HeadlessSurface;

const LOOPING: &str = "export default class T extends LoopingBot { override loop() {} }\n";
const THIEVER: &str = "export const SETTINGS = { target: { type: 'string', default: 'Guard' } };\nexport default class T extends LoopingBot { override loop() {} }\n";

struct Fixture {
    core: OperatorSession<()>,
    scripts: Scripts,
    dir: PathBuf,
    surface: HeadlessSurface<fn(&str)>,
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

/// `profiles` (uids 1..) are in the vault; the first `loaded` are members.
fn fixture(test: &str, profiles: &[&str], loaded: usize) -> Fixture {
    let iso = script::IsolatedEnv::enter(&format!("frontend-core-marked-{test}"));
    let dir = iso.dir.clone();
    let mut vault = Vault::create(&dir.join("vault"), "test-passphrase-01").unwrap();
    for (i, name) in profiles.iter().enumerate() {
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
    for name in &profiles[..loaded] {
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
        surface,
        _iso: iso,
    }
}

impl Fixture {
    fn card(&mut self, file: &str) -> (script::JsCard, script::ScriptSel) {
        self.card_src(file, LOOPING)
    }

    fn card_src(&mut self, file: &str, src: &str) -> (script::JsCard, script::ScriptSel) {
        let path = self.dir.join(file);
        std::fs::write(&path, src).unwrap();
        let card = self.scripts.js.load(&path).unwrap();
        let sel = script::ScriptSel::Loaded(card.source, card.identity_id());
        (card, sel)
    }

    fn assign(&mut self, name: &str, card: &script::JsCard) {
        assert!(self
            .scripts
            .persist_assignment(&mut self.core, name, card.assignment()));
        self.core.flush_writes();
    }

    fn pump(&mut self) {
        self.core.poll();
        self.scripts.poll(&mut self.core);
        if let Some(play) = self.core.play() {
            play.pump_script_lifecycles();
        }
    }

    fn until(&mut self, what: &str, mut done: impl FnMut(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !done(self) && Instant::now() < deadline {
            self.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(done(self), "timed out waiting for {what}");
    }

    fn state(&self, name: &str) -> script::RunState {
        self.core.play().unwrap().script_state(name)
    }

    fn start_running(&mut self, name: &str) {
        self.scripts
            .start_profile(&mut self.core, name, None)
            .unwrap();
        self.until("start to settle", |f| !f.scripts.starts_pending());
        self.until("Running", |f| f.state(name) == script::RunState::Running);
    }

    fn generation(&self, name: &str) -> u64 {
        self.core
            .play()
            .unwrap()
            .script_runtime_generation(name)
            .unwrap()
    }

    fn assignment_key(&self, name: &str) -> Option<String> {
        Scripts::assignment(&self.core, name).map(|a| a.key())
    }

    fn run_has_bag(&self, name: &str, bag: &serde_json::Map<String, serde_json::Value>) -> bool {
        let play = self.core.play().unwrap();
        let identity = play.script_source_identity(name).unwrap();
        !play.script_post_settings_fenced(name, bag, &identity, self.generation(name))
    }
}

fn marks(uids: &[i32]) -> MarkedSelection {
    let mut selection = MarkedSelection::default();
    selection.mark_all(uids.iter().copied().map(ProfileIdentity::uid));
    selection
}

/// Assign saves the card on idle marked bots and nothing else: an active bot
/// keeps running its card, an unmarked bot is untouched, and a mark whose
/// profile is gone is named.
#[test]
fn assign_saves_the_card_on_idle_marked_bots_and_skips_active_ones() {
    let mut f = fixture("assign", &["alice", "bob", "carol"], 3);
    let (old, _) = f.card("old.ts");
    let (new, new_sel) = f.card("new.ts");
    f.assign("bob", &old);
    f.start_running("bob");
    let generation = f.generation("bob");
    let alice_settings = f
        .core
        .vault()
        .unwrap()
        .get("alice")
        .unwrap()
        .settings
        .script_settings
        .clone();

    let report = assign_marked(
        &marks(&[1, 2, 99]),
        &mut f.core,
        &mut f.scripts,
        &new_sel,
        None,
    );
    f.core.flush_writes();

    let gone = format!("profile#{}", ProfileIdentity::uid(99).raw());
    assert_eq!(
        report.summary(),
        format!(
            "Assign: assigned 1, skipped 2: bob: script active: use Assign & restart, \
             {gone}: profile unavailable"
        )
    );
    assert_eq!(f.assignment_key("alice"), Some(new.assignment().key()));
    assert_eq!(f.assignment_key("bob"), Some(old.assignment().key()));
    assert_eq!(f.assignment_key("carol"), None, "unmarked bot untouched");
    assert_eq!(f.state("bob"), script::RunState::Running);
    assert_eq!(f.generation("bob"), generation, "bob was not restarted");
    assert_eq!(
        f.core
            .vault()
            .unwrap()
            .get("alice")
            .unwrap()
            .settings
            .script_settings,
        alice_settings,
        "per-bot settings stay separate"
    );
    f.core.play().unwrap().script_stop("bob");
}

/// A card that cannot start is refused once for every marked bot, and
/// nothing is saved.
#[test]
fn assign_of_a_missing_card_fails_every_marked_bot_and_saves_nothing() {
    let mut f = fixture("assign-missing", &["alice", "bob"], 2);
    let gone = script::ScriptSel::Loaded(script::ScriptSource::File, "gone.ts".into());
    let report = assign_marked(&marks(&[1, 2]), &mut f.core, &mut f.scripts, &gone, None);
    assert_eq!(report.failed_count(), 2);
    assert_eq!(report.refusal(), Some("missing file: gone.ts"));
    assert_eq!(f.assignment_key("alice"), None);
    assert_eq!(f.assignment_key("bob"), None);
}

/// Assign & restart stops the active bots, then starts the new card on each
/// marked bot through the paced permit: the run is a new one on the new card,
/// no bot is counted twice and none fails for the stop still in flight.
#[test]
fn assign_and_restart_replaces_running_cards_and_starts_idle_ones() {
    let mut f = fixture("restart", &["alice", "bob", "carol", "dave"], 3);
    let (old, _) = f.card("old.ts");
    let (new, new_sel) = f.card("new.ts");
    f.assign("bob", &old);
    f.start_running("bob");
    let before = f.generation("bob");

    let selection = marks(&[1, 2, 3, 4]);
    let scope = restart_scope(&selection, &f.core);
    assert_eq!(
        scope.interrupted,
        ["bob"],
        "only the running bot is interrupted"
    );
    assert_eq!(scope.starting, ["alice", "carol"], "dave is not loaded");

    assign_and_restart_marked(&selection, &mut f.core, &mut f.scripts, &new_sel, None);
    f.until("every restart to settle", |f| !f.scripts.starts_pending());
    for name in ["alice", "bob", "carol"] {
        f.until(name, |f| f.state(name) == script::RunState::Running);
        f.core.flush_writes();
        assert_eq!(
            f.assignment_key(name),
            Some(new.assignment().key()),
            "{name}"
        );
    }
    assert!(f.generation("bob") > before, "bob runs a new incarnation");
    assert_eq!(
        f.scripts.last_bulk_report(),
        Some("Assign & restart: started 3, skipped 1: dave: not loaded")
    );
    for name in ["alice", "bob", "carol"] {
        f.core.play().unwrap().script_stop(name);
    }
}

/// A Logout while a restart still waits for the stop drops the restart.
#[test]
fn logout_while_a_restart_waits_for_the_stop_cancels_it() {
    let mut f = fixture("restart-logout", &["alice"], 1);
    let (old, _) = f.card("old.ts");
    let (_, new_sel) = f.card("new.ts");
    f.assign("alice", &old);
    f.start_running("alice");
    assign_and_restart_marked(&marks(&[1]), &mut f.core, &mut f.scripts, &new_sel, None);
    assert!(
        f.scripts.starts_pending(),
        "the restart is waiting on the stop or the permit"
    );
    assert!(f.scripts.cancel_queued_as("alice", "logged out"));
    assert!(!f.scripts.starts_pending());
    f.until("the stop to finish", |f| {
        f.state("alice") == script::RunState::Idle
    });
    f.pump();
    assert_eq!(f.state("alice"), script::RunState::Idle, "never restarted");
}

fn push_ready(f: &Fixture, name: &str) {
    f.core
        .play()
        .unwrap()
        .statuses
        .lock()
        .unwrap()
        .push(SlotStatus {
            username: name.into(),
            connected: true,
            ingame: true,
            scene_state: 2,
            ..SlotStatus::default()
        });
}

/// Log out reaches the bots that are logged in and skips the ones that are
/// not; Log in reaches the rest, loads a profile that was never loaded and
/// skips a bot that is already in game.
#[test]
fn login_and_logout_reach_only_the_bots_they_apply_to() {
    let mut f = fixture("login-logout", &["alice", "bob", "carol"], 2);
    push_ready(&f, "alice");
    f.core.poll();

    let out = logout_marked(&marks(&[1, 2]), &mut f.core, &mut f.scripts);
    assert_eq!(
        out.summary(),
        "Log out marked: logging out 1, skipped 1: bob: not logged in"
    );
    let arm = |f: &Fixture, name: &str| f.core.play().unwrap().arm(name).unwrap();
    assert!(arm(&f, "alice").login_latched() || arm(&f, "alice").wants_logout());
    assert!(!f.core.members().iter().any(|m| m == "carol"));

    // Alice is out again and carol was never loaded; bob is offline.
    f.core.play().unwrap().statuses.lock().unwrap().clear();
    f.core.poll();
    let login = login_marked(&marks(&[1, 3]), &mut f.core, &mut f.surface);
    assert_eq!(login.summary(), "Log in marked: logging in 2, skipped 0");
    assert!(
        f.core.members().iter().any(|m| m == "carol"),
        "carol was loaded"
    );
    assert!(arm(&f, "alice").wants_login());
    assert!(arm(&f, "carol").wants_login());

    push_ready(&f, "alice");
    f.core.poll();
    let again = login_marked(&marks(&[1]), &mut f.core, &mut f.surface);
    assert_eq!(
        again.summary(),
        "Log in marked: logging in 0, skipped 1: alice: already logged in"
    );
}

#[test]
fn marked_logout_cancels_a_memory_relog_login_half() {
    let mut f = fixture("marked-logout-relog", &["alice", "bob"], 2);
    for name in ["alice", "bob"] {
        push_ready(&f, name);
    }
    {
        let mut rows = f.core.play().unwrap().statuses.lock().unwrap();
        for row in rows.iter_mut() {
            row.login_lowmem = Some(true);
        }
    }
    f.core.poll();
    f.core.request_memory_relog("alice", &mut f.surface);
    {
        let mut rows = f.core.play().unwrap().statuses.lock().unwrap();
        let alice = rows.iter_mut().find(|row| row.username == "alice").unwrap();
        alice.connected = false;
        alice.ingame = false;
        alice.scene_state = 0;
        alice.login_latched = true;
    }
    f.core.poll();
    assert!(f.core.play().unwrap().arm("alice").unwrap().wants_login());
    assert!(f.core.memory_status("alice").unwrap().relog_pending);

    let out = logout_marked(&marks(&[1, 2]), &mut f.core, &mut f.scripts);

    assert_eq!(out.summary(), "Log out marked: logging out 2, skipped 0");
    for name in ["alice", "bob"] {
        let arm = f.core.play().unwrap().arm(name).unwrap();
        assert!(!arm.wants_login(), "{name} must stay logged out");
        assert!(arm.login_latched(), "{name} must keep the logout latch");
    }
    assert!(
        !f.core.memory_status("alice").unwrap().relog_pending,
        "marked Logout cancels the relog exactly like single Logout"
    );
}

/// Apply the focused bot's card settings to the marked bots only: an
/// unmarked same-card bot keeps its own and is only counted, and every
/// marked bot that cannot take the copy is named with its reason.
#[test]
fn apply_settings_to_marked_copies_only_to_marked_same_card_members() {
    let names = ["alice", "bob", "carol", "dave", "erin", "frank"];
    // frank is in the vault but not loaded.
    let mut f = fixture("apply-marked", &names, 5);
    let (card, sel) = f.card("thiever.ts");
    let (other, _) = f.card("other.ts");
    for name in ["alice", "bob", "dave", "erin"] {
        f.assign(name, &card);
    }
    f.assign("carol", &other);
    f.scripts
        .set_profile_setting(
            &mut f.core,
            "alice",
            card.source,
            &card.name,
            &card.path,
            "target",
            serde_json::json!("Guard"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    // bob and dave take the copy; carol is on another card and frank is not
    // loaded (both marked, so both are named); erin is unmarked.
    let marked = marks(&[2, 3, 4, 6]);
    let scope = prepare_apply_settings_marked(&marked, &f.core, &mut f.scripts, "alice", &sel)
        .unwrap()
        .clone();
    assert_eq!(scope.targets, ["bob", "dave"]);
    assert_eq!(
        scope.skipped,
        [
            ("carol".to_string(), "assigned another card".to_string()),
            ("frank".to_string(), "not loaded".to_string())
        ]
    );
    assert_eq!(scope.unmarked, Some(1));
    let prompt = scope.prompt();
    assert!(
        prompt.contains("to 2 marked same-card bot(s)")
            && prompt.contains("carol (assigned another card), frank (not loaded)")
            && prompt.contains("1 unmarked bot(s) left unchanged"),
        "{prompt}"
    );

    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let key = card.identity_key();
    let saved = |f: &Fixture, name: &str| {
        f.core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .cloned()
    };
    let want: serde_json::Map<String, serde_json::Value> =
        std::iter::once(("target".to_string(), serde_json::json!("Guard"))).collect();
    assert_eq!(saved(&f, "bob"), Some(want.clone()));
    assert_eq!(saved(&f, "dave"), Some(want));
    assert_eq!(saved(&f, "erin"), None, "an unmarked bot keeps its own");
    assert_eq!(saved(&f, "carol"), None);
    assert_eq!(saved(&f, "frank"), None);
    let summary = f.scripts.last_settings_sync().unwrap().summary();
    assert!(
        summary.starts_with("Apply to marked ")
            && summary.contains("saved 2")
            && summary.contains("skipped 2 (1 other card, 1 not loaded)")
            && summary.contains("carol: skipped, assigned another card")
            && summary.contains("frank: skipped, not loaded")
            && summary.contains("1 unmarked unchanged"),
        "{summary}"
    );
}

/// A mark set with nothing to copy to is refused with the reason for each
/// marked row, and leaves nothing prepared to apply.
#[test]
fn apply_settings_to_marked_refuses_when_no_marked_bot_can_take_it() {
    let mut f = fixture("apply-marked-refuse", &["alice", "bob", "carol"], 3);
    let (card, sel) = f.card("thiever.ts");
    let (other, _) = f.card("other.ts");
    f.assign("alice", &card);
    f.assign("bob", &other);
    f.assign("carol", &card);

    let refused = |f: &mut Fixture, marked: &MarkedSelection| {
        prepare_apply_settings_marked(marked, &f.core, &mut f.scripts, "alice", &sel)
            .map(|_| ())
            .unwrap_err()
    };
    assert_eq!(
        refused(&mut f, &marks(&[])),
        "Apply to marked: mark fleet rows first"
    );
    let only_other = refused(&mut f, &marks(&[2]));
    assert!(
        only_other.contains("bob: assigned another card"),
        "{only_other}"
    );
    let only_source = refused(&mut f, &marks(&[1]));
    assert!(
        only_source.contains("the only marked bot is the source"),
        "{only_source}"
    );
    assert!(
        f.scripts.prepared_settings_sync().is_none(),
        "a refusal leaves nothing to apply"
    );
}

/// A loaded card newly assigned to the focused bot has no per-card settings
/// key in its profile yet (only the legacy global overrides). Opening the
/// frozen confirmation and cancelling it writes nothing: the vault file is
/// byte-identical and the source profile stays unclaimed, while the frozen
/// bag still carries what the first read would have migrated. Confirming
/// copies that bag to the marked target and still leaves the source alone.
#[test]
fn apply_settings_to_marked_prepare_and_cancel_write_nothing() {
    let mut f = fixture("apply-marked-readonly", &["alice", "bob"], 2);
    let (card, sel) = f.card("thiever.ts");
    f.assign("alice", &card);
    f.assign("bob", &card);
    f.scripts
        .legacy
        .set_str(card.source, &card.name, "target", "Guard");
    let key = card.identity_key();
    let in_vault = |f: &Fixture, name: &str| {
        f.core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&key)
            .cloned()
    };
    assert_eq!(in_vault(&f, "alice"), None, "a newly assigned card");
    let vault_file = f.dir.join("vault");
    let before = std::fs::read(&vault_file).unwrap();

    let scope = prepare_apply_settings_marked(&marks(&[2]), &f.core, &mut f.scripts, "alice", &sel)
        .unwrap()
        .clone();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(scope.targets, ["bob"]);
    assert_eq!(
        scope.overrides.get("target"),
        Some(&serde_json::json!("Guard")),
        "the frozen bag is what the first read would migrate"
    );
    assert_eq!(
        std::fs::read(&vault_file).unwrap(),
        before,
        "preparing the confirmation wrote the vault"
    );
    assert_eq!(in_vault(&f, "alice"), None, "the source was claimed early");

    f.scripts.cancel_settings_sync();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(std::fs::read(&vault_file).unwrap(), before, "cancel wrote");
    assert_eq!(in_vault(&f, "bob"), None, "cancel copied nothing");

    prepare_apply_settings_marked(&marks(&[2]), &f.core, &mut f.scripts, "alice", &sel).unwrap();
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    let want: serde_json::Map<String, serde_json::Value> =
        std::iter::once(("target".to_string(), serde_json::json!("Guard"))).collect();
    assert_eq!(in_vault(&f, "bob"), Some(want), "confirm copies the bag");
}

/// The Fleet window's progress columns read the host: a started run reports
/// its runtime, a stopped or never-started slot reports none, and a new Start
/// begins a new run.
#[test]
fn a_started_run_reports_progress_and_a_stopped_one_does_not() {
    let mut f = fixture("progress", &["alice"], 1);
    let (card, _) = f.card("loop.ts");
    f.assign("alice", &card);
    let progress = |f: &Fixture| f.core.play().unwrap().script_progress("alice");
    assert!(progress(&f).is_none(), "never started");

    f.start_running("alice");
    let run = progress(&f).expect("a running slot has a run");
    assert!(run.running_for < Duration::from_secs(20));
    assert_eq!(run.levels_gained(), 0);

    f.core.play().unwrap().script_stop("alice");
    f.until("the stop to finish", |f| {
        f.state("alice") == script::RunState::Idle
    });
    assert!(progress(&f).is_none(), "a stopped slot shows no run");

    f.start_running("alice");
    assert!(progress(&f).is_some(), "a new Start begins a new run");
    f.core.play().unwrap().script_stop("alice");
}

/// Apply to marked is the only settings shortcut that treats an unassigned
/// bot as a target. The confirmation names the assignment; unmarked
/// unassigned wall members stay unassigned.
#[test]
fn apply_to_marked_assigns_unassigned_and_says_so() {
    let mut f = fixture("apply-marked-assign", &["alice", "bob", "carol", "dave"], 4);
    let (card, sel) = f.card_src("thiever.ts", THIEVER);
    f.assign("alice", &card);
    f.assign("dave", &card);
    f.scripts
        .set_profile_setting(
            &mut f.core,
            "alice",
            card.source,
            &card.name,
            &card.path,
            "target",
            serde_json::json!("Knight of Ardougne"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    let marked = marks(&[1, 2, 4]);
    let scope = prepare_apply_settings_marked(&marked, &f.core, &mut f.scripts, "alice", &sel)
        .unwrap()
        .clone();
    assert_eq!(scope.targets, ["bob", "dave"]);
    assert_eq!(scope.assigning, ["bob"]);
    let prompt = scope.prompt();
    assert!(
        prompt.contains("Assign")
            && prompt.contains("1 bot")
            && prompt.contains("copy its settings")
            && prompt.contains("alice")
            && prompt.contains("2 marked bot(s)")
            && prompt.contains("1 unmarked bot(s) left unchanged"),
        "{prompt}"
    );

    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert_eq!(
        Scripts::assignment(&f.core, "bob").unwrap().key(),
        card.identity_key()
    );
    assert!(
        Scripts::assignment(&f.core, "carol").is_none(),
        "unmarked unassigned carol is not a wall-wide assign"
    );
    let want: serde_json::Map<String, serde_json::Value> = std::iter::once((
        "target".to_string(),
        serde_json::json!("Knight of Ardougne"),
    ))
    .collect();
    let saved = |f: &Fixture, name: &str| {
        f.core
            .vault()
            .unwrap()
            .get(name)
            .unwrap()
            .settings
            .script_settings
            .get(&card.identity_key())
            .cloned()
    };
    assert_eq!(saved(&f, "bob"), Some(want.clone()));
    assert_eq!(saved(&f, "dave"), Some(want));
    assert_eq!(saved(&f, "carol"), None);
}

/// Fleet-window sequence from the 2026-10-02 operator session: wall-wide
/// Apply while Bob is unassigned (skipped, not assigned), Assign the card,
/// a pending copy confirm, ordinary Start, then Apply and Start. Start is
/// blocked until Apply or Cancel; after Apply it runs the copied Knight
/// bag, not the card default.
#[test]
fn operator_apply_assign_start_does_not_start_on_defaults() {
    let mut f = fixture("operator-apply", &["alice", "bob", "carol"], 3);
    let (card, sel) = f.card_src("thiever.ts", THIEVER);
    f.assign("alice", &card);
    f.scripts
        .set_profile_setting(
            &mut f.core,
            "alice",
            card.source,
            &card.name,
            &card.path,
            "target",
            serde_json::json!("Knight of Ardougne"),
        )
        .unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);

    f.scripts
        .prepare_settings_sync(&f.core, "alice", card.source, &card.name, &card.path);
    let wall = f.scripts.prepared_settings_sync().unwrap();
    assert!(
        wall.targets.is_empty(),
        "wall-wide Apply does not take unassigned bots: {:?}",
        wall.targets
    );
    assert!(
        wall.skipped
            .iter()
            .any(|(n, r)| n == "bob" && r == "no assignment")
            && wall
                .skipped
                .iter()
                .any(|(n, r)| n == "carol" && r == "no assignment"),
        "{:?}",
        wall.skipped
    );
    assert!(
        !wall.prompt().to_ascii_lowercase().contains("assign"),
        "{}",
        wall.prompt()
    );
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    assert!(Scripts::assignment(&f.core, "bob").is_none());
    assert!(Scripts::assignment(&f.core, "carol").is_none());

    let marked = marks(&[2]);
    let report = assign_marked(&marked, &mut f.core, &mut f.scripts, &sel, None);
    assert!(
        report.summary().contains("assigned"),
        "{}",
        report.summary()
    );
    assert_eq!(
        Scripts::assignment(&f.core, "bob").unwrap().key(),
        card.identity_key()
    );

    f.scripts
        .prepare_settings_sync(&f.core, "alice", card.source, &card.name, &card.path);
    assert_eq!(f.scripts.prepared_settings_sync().unwrap().targets, ["bob"]);

    let blocked = f
        .scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap_err();
    assert!(
        blocked.contains("blocked") && blocked.contains("waiting for Apply or Cancel"),
        "{blocked}"
    );
    let blocked = f
        .scripts
        .start_profile(&mut f.core, "bob", None)
        .unwrap_err();
    assert!(
        blocked.contains("blocked") && blocked.contains("waiting for Apply or Cancel"),
        "{blocked}"
    );
    start_marked(&marked, &mut f.core, &mut f.scripts, Some(&sel), None);
    let bulk = f.scripts.last_bulk_report().unwrap();
    assert!(bulk.contains("blocked"), "{bulk}");
    assert_eq!(f.state("bob"), script::RunState::Idle);

    f.scripts.cancel_settings_sync();
    f.scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap();
    f.until("starts to settle", |f| !f.scripts.starts_pending());
    f.until("bob running on default", |f| {
        f.state("bob") == script::RunState::Running
    });
    let guard: serde_json::Map<String, serde_json::Value> =
        std::iter::once(("target".to_string(), serde_json::json!("Guard"))).collect();
    assert!(
        f.run_has_bag("bob", &guard),
        "Cancel then Start uses the card default, not the unapplied copy"
    );
    f.core.play().unwrap().script_stop("bob");
    f.until("bob idle after cancel-start", |f| {
        f.state("bob") == script::RunState::Idle
    });

    f.scripts
        .prepare_settings_sync(&f.core, "alice", card.source, &card.name, &card.path);
    f.scripts.apply_settings_sync(&mut f.core).unwrap();
    f.core.flush_writes();
    f.scripts.poll(&mut f.core);
    f.scripts
        .start_selected(&mut f.core, "bob", None, None)
        .unwrap();
    f.until("starts to settle", |f| !f.scripts.starts_pending());
    f.until("bob running", |f| {
        f.state("bob") == script::RunState::Running
    });
    let knight: serde_json::Map<String, serde_json::Value> = std::iter::once((
        "target".to_string(),
        serde_json::json!("Knight of Ardougne"),
    ))
    .collect();
    assert!(
        f.run_has_bag("bob", &knight),
        "Start after Apply consumes the copied Knight bag"
    );
    assert!(Scripts::assignment(&f.core, "carol").is_none());
}
