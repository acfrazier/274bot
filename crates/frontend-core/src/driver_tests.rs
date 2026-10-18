//! Headless driver over real backends: a real vault file, a real
//! [`host_play::Play`] (spawns attach arms without worker threads, so no
//! server is contacted) and the real slot mailboxes.

use api::identity::Settlement;
use host_play::{InstancePermit, Play, PlayOptions};
use vault::{Profile, ProfileSettings, Vault};

use super::*;
use crate::command::{CommandFailure, Completion, Pending, SkipReason, Subject};

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

fn vault_with(test: &str, names: &[&str]) -> Vault {
    let path = std::env::temp_dir()
        .join(format!("274bot-t33-driver-{}-{test}", std::process::id()))
        .join("vault");
    std::fs::create_dir_all(path.parent().expect("vault dir")).expect("vault dir");
    let _ = std::fs::remove_file(&path);
    let mut vault = Vault::create(&path, "test-passphrase-01").expect("vault");
    for (uid, name) in names.iter().enumerate() {
        vault
            .upsert(Profile {
                username: (*name).to_string(),
                password: "pw".into(),
                uid: uid as i32 + 1,
                settings: ProfileSettings {
                    auto_login: false,
                    ..ProfileSettings::default()
                },
            })
            .expect("profile");
    }
    vault
}

fn driver(test: &str, names: &[&str]) -> HeadlessDriver {
    let mut session = OperatorSession::new(InstancePermit::SkipLock);
    session.set_spawn_workers(false);
    session.start(vault_with(test, names), empty_play());
    HeadlessDriver::new(session)
}

type Terminal = Settlement<Completion, CommandFailure>;

fn terminal(driver: &HeadlessDriver, request: RequestId, name: &str) -> Option<Terminal> {
    let subject = Subject::Profile(driver.session().profile_identity_or_synthetic(name));
    let member = driver.report(request)?.member(subject)?;
    member.terminal().cloned()
}

fn done() -> Terminal {
    Settlement::Completed(Completion::Done)
}

#[test]
fn load_settles_from_its_operation_and_login_on_the_slot_acknowledgement() {
    let mut d = driver("load-login", &["alice"]);
    let load = d
        .submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load admitted");
    assert_eq!(d.session().members(), &["alice".to_string()]);
    assert_eq!(terminal(&d, load.request(), "alice"), Some(done()));

    let login = d
        .submit(OperatorCommand::Login, Targets::Profile("alice"))
        .expect("login admitted");
    // The member was admitted on alice's slot, posted to its mailbox, and
    // the mailbox's IntentApplied settled it.
    let report = d.report(login.request()).expect("login report");
    let member = report.members().first().expect("one member");
    assert_eq!(
        member.scope(),
        api::identity::Scope::Slot(
            d.session()
                .play()
                .and_then(|play| play.arm("alice"))
                .expect("alice's arm")
                .slot()
        )
    );
    assert_eq!(terminal(&d, login.request(), "alice"), Some(done()));
    assert!(d
        .session()
        .play()
        .expect("play")
        .arm("alice")
        .expect("arm")
        .wants_login());
}

#[test]
fn logout_posts_to_the_member_slot_and_settles_on_its_acknowledgement() {
    let mut d = driver("logout", &["alice", "bob"]);
    d.submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    let names = ["alice".to_string(), "bob".to_string()];
    let logout = d
        .submit(OperatorCommand::Logout, Targets::Profiles(&names))
        .expect("logout admitted");
    for name in &names {
        assert_eq!(terminal(&d, logout.request(), name), Some(done()));
        let arm = d.session().play().and_then(|p| p.arm(name)).expect("arm");
        assert!(arm.wants_logout());
    }
}

#[test]
fn load_all_selects_the_focus_hint_when_nothing_is_selected() {
    let d = driver("load-all-hint", &["alice", "bob"]);
    let mut session = d.into_session();
    session.set_focus_hint(Some("bob"));
    let mut d = HeadlessDriver::new(session);
    let load = d
        .submit(OperatorCommand::Load, Targets::Everyone)
        .expect("load all");
    assert_eq!(load.added(), 2);
    assert_eq!(d.session().selected(), Some("bob"));
}

#[test]
fn select_and_render_policy_settle_at_dispatch() {
    let mut d = driver("select", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let select = d
        .submit(OperatorCommand::Select, Targets::Profile("alice"))
        .expect("select");
    assert_eq!(terminal(&d, select.request(), "alice"), Some(done()));
    assert_eq!(d.session().selected(), Some("alice"));
    let render = d
        .submit(OperatorCommand::SetRenderAll(true), Targets::Application)
        .expect("render all");
    let report = d.report(render.request()).expect("report");
    assert!(report.is_settled());
}

#[test]
fn pause_without_a_running_script_is_skipped_with_its_reason() {
    let mut d = driver("pause", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let pause = d
        .submit(OperatorCommand::PauseScript, Targets::Profile("alice"))
        .expect("pause");
    // The run owner reports the skip on the next poll.
    d.poll();
    assert_eq!(
        terminal(&d, pause.request(), "alice"),
        Some(Settlement::Completed(Completion::Skipped(
            SkipReason::NoRunningScript
        )))
    );
}

#[test]
fn targets_must_fit_the_row() {
    let mut d = driver("targets", &["alice"]);
    assert_eq!(
        d.submit(OperatorCommand::Select, Targets::Selected)
            .expect_err("nothing selected"),
        SubmitError::NoTarget(OperatorCommand::Select)
    );
    assert_eq!(
        d.submit(
            OperatorCommand::SetRenderAll(true),
            Targets::Profile("alice")
        )
        .expect_err("application row"),
        SubmitError::WrongTargets(OperatorCommand::SetRenderAll(true))
    );
    assert_eq!(
        d.submit(OperatorCommand::SwitchWorld(1), Targets::Profile("alice"))
            .expect_err("no entry point"),
        SubmitError::NoEntryPoint(OperatorCommand::SwitchWorld(1))
    );
}

#[test]
fn a_pending_owner_member_settles_on_a_later_poll() {
    let mut d = driver("owner-poll", &["alice"]);
    d.submit(OperatorCommand::Load, Targets::Profile("alice"))
        .expect("load");
    let remove = d
        .submit(OperatorCommand::Remove, Targets::Profile("alice"))
        .expect("remove");
    let subject = Subject::Profile(d.session().profile_identity_or_synthetic("alice"));
    let pending = d
        .report(remove.request())
        .and_then(|report| report.member(subject))
        .and_then(|member| member.pending());
    assert!(matches!(pending, None | Some(Pending::Accepted)));
    for _ in 0..200 {
        d.poll();
        if d.report(remove.request())
            .is_some_and(|report| report.is_settled())
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        d.report(remove.request())
            .is_some_and(|report| report.is_settled()),
        "remove settles once the slot retires"
    );
}
