use super::*;

fn line(store: &LogStore, slot: Option<&str>, source: Source, level: Level, message: &str) {
    store.push(&Record {
        slot,
        tick: Some(12),
        source,
        level,
        message,
    });
}

fn messages(view: &LogView) -> Vec<&str> {
    view.rows().iter().map(|e| &*e.message).collect()
}

#[test]
fn each_slot_ring_keeps_its_newest_500_and_counts_what_it_evicted() {
    let store = LogStore::new();
    for i in 0..1_200 {
        line(
            &store,
            Some("alice"),
            Source::Script,
            Level::Info,
            &format!("a{i}"),
        );
    }
    line(&store, Some("bob"), Source::Login, Level::Info, "b0");
    for i in 0..600 {
        line(&store, None, Source::Host, Level::Info, &format!("p{i}"));
    }
    assert_eq!(store.slot_len("alice"), SLOT_RING_CAP);
    assert_eq!(
        store.slot_len("bob"),
        1,
        "a flooding slot never evicts another"
    );

    let mut view = LogView::new(LogScope::Slot("alice".into()));
    store.refresh(&mut view);
    assert_eq!(view.len(), SLOT_RING_CAP);
    assert_eq!(view.rows().front().unwrap().message.as_ref(), "a700");
    assert_eq!(view.rows().back().unwrap().message.as_ref(), "a1199");
    assert_eq!(view.dropped(), 700);

    let mut process = LogView::new(LogScope::Process);
    store.refresh(&mut process);
    assert_eq!(process.len(), PROCESS_RING_CAP);
    assert_eq!(process.rows().front().unwrap().message.as_ref(), "p100");
}

#[test]
fn a_view_appends_new_lines_and_skips_work_when_nothing_changed() {
    let store = LogStore::new();
    let mut view = LogView::new(LogScope::Slot("alice".into()));
    line(&store, Some("alice"), Source::Script, Level::Info, "one");
    assert!(store.refresh(&mut view));
    assert!(!store.refresh(&mut view), "unchanged store: no work");
    line(
        &store,
        Some("bob"),
        Source::Script,
        Level::Info,
        "other slot",
    );
    assert!(
        !store.refresh(&mut view),
        "another slot's line changes no row"
    );
    line(&store, Some("alice"), Source::Script, Level::Info, "two");
    assert!(store.refresh(&mut view));
    assert_eq!(messages(&view), ["one", "two"]);
    let entry = &view.rows()[1];
    assert_eq!(entry.tick, Some(12));
    assert_eq!(entry.slot.as_deref(), Some("alice"));
    assert_eq!(entry.clock.as_str().len(), 12);
    assert_eq!(&entry.clock.as_str()[2..3], ":");
    assert_eq!(&entry.clock.as_str()[8..9], ".");
}

#[test]
fn filters_by_level_source_and_case_insensitive_text() {
    let store = LogStore::new();
    let s = Some("alice");
    line(&store, s, Source::Script, Level::Debug, "slow tick 9");
    line(&store, s, Source::Script, Level::Info, "Chopping Oak");
    line(
        &store,
        s,
        Source::Bank,
        Level::Warn,
        "withdraw lobster: not in bank",
    );
    line(
        &store,
        s,
        Source::Login,
        Level::Error,
        "login code 3: invalid",
    );
    line(
        &store,
        s,
        Source::Nav,
        Level::Info,
        "answered choice 2 for npc",
    );

    let mut view = LogView::new(LogScope::Slot("alice".into()));
    store.refresh(&mut view);
    assert_eq!(view.len(), 4, "debug is hidden by the default Info minimum");

    view.edit_filter(|f| f.min_level = Level::Warn);
    store.refresh(&mut view);
    assert_eq!(
        messages(&view),
        ["withdraw lobster: not in bank", "login code 3: invalid"]
    );

    view.edit_filter(|f| {
        f.min_level = Level::Debug;
        f.set_source(Some(Source::Script));
    });
    store.refresh(&mut view);
    assert_eq!(messages(&view), ["slow tick 9", "Chopping Oak"]);
    assert_eq!(view.filter().single_source(), Some(Source::Script));

    view.edit_filter(|f| {
        f.set_source(None);
        f.text = "OAK".into();
    });
    store.refresh(&mut view);
    assert_eq!(messages(&view), ["Chopping Oak"]);

    view.edit_filter(|f| f.text = "ALI".into());
    store.refresh(&mut view);
    assert_eq!(view.len(), 5, "text search also matches the slot name");

    view.edit_filter(|f| f.text = "no such text".into());
    store.refresh(&mut view);
    assert!(view.is_empty());
    line(&store, s, Source::Script, Level::Info, "still no match");
    store.refresh(&mut view);
    assert!(view.is_empty(), "new lines pass the same filter");
}

#[test]
fn scope_switches_between_slot_process_and_all_in_order() {
    let store = LogStore::new();
    line(&store, Some("alice"), Source::Script, Level::Info, "a1");
    line(
        &store,
        None,
        Source::Host,
        Level::Error,
        "vault: wrong passphrase",
    );
    line(&store, Some("bob"), Source::Login, Level::Info, "b1");
    line(&store, Some("alice"), Source::Script, Level::Info, "a2");

    let mut view = LogView::new(LogScope::Slot("alice".into()));
    store.refresh(&mut view);
    assert_eq!(messages(&view), ["a1", "a2"]);

    view.follow_slot(Some("bob"));
    store.refresh(&mut view);
    assert_eq!(messages(&view), ["b1"]);

    view.follow_slot(None);
    store.refresh(&mut view);
    assert_eq!(messages(&view), ["vault: wrong passphrase"]);

    view.set_scope(LogScope::All);
    store.refresh(&mut view);
    assert_eq!(
        messages(&view),
        ["a1", "vault: wrong passphrase", "b1", "a2"]
    );
    view.follow_slot(Some("alice"));
    assert_eq!(view.scope(), &LogScope::All, "All ignores focus changes");
    line(&store, Some("bob"), Source::Login, Level::Info, "b2");
    line(&store, Some("alice"), Source::Script, Level::Info, "a3");
    store.refresh(&mut view);
    assert_eq!(view.rows().back().unwrap().message.as_ref(), "a3");
    assert_eq!(view.len(), 6);
}

#[test]
fn registered_passwords_never_reach_a_row_a_copy_or_a_save() {
    let store = LogStore::new();
    store.register_secret("alice", "hunter22");
    store.register_secret("test", "test");
    store.register_secret("carol", "abc");
    let profile = vault::Profile {
        username: "alice".into(),
        password: "hunter22".into(),
        uid: 1,
        settings: vault::ProfileSettings::default(),
    };
    line(
        &store,
        Some("alice"),
        Source::Login,
        Level::Error,
        &format!("login failed for {profile:?} with hunter22"),
    );
    line(
        &store,
        Some("test"),
        Source::Login,
        Level::Info,
        "test ingame",
    );
    let mut view = LogView::new(LogScope::Slot("alice".into()));
    store.refresh(&mut view);
    let text = view.to_text();
    assert!(!text.contains("hunter22"), "{text}");
    assert!(text.contains("***"), "{text}");
    let mut harness = LogView::new(LogScope::Slot("test".into()));
    store.refresh(&mut harness);
    assert_eq!(
        messages(&harness),
        ["test ingame"],
        "a password equal to its username is not redacted"
    );
}

#[test]
fn long_and_multi_line_messages_are_flattened_and_capped() {
    let store = LogStore::new();
    let long = format!(
        "TypeError: boom\n    at loop\t(x.js:1)\n{}",
        "é".repeat(600)
    );
    line(&store, None, Source::Script, Level::Error, &long);
    let mut view = LogView::new(LogScope::Process);
    store.refresh(&mut view);
    let message = &view.rows()[0].message;
    assert!(message.len() <= MESSAGE_CAP);
    assert!(message.ends_with('…'));
    assert!(message.starts_with("TypeError: boom     at loop (x.js:1) "));
    assert!(!message.contains('\n'));
}

#[test]
fn script_lines_map_onto_sources_and_levels() {
    assert_eq!(
        classify_script_line("watchdog: hung loop (10s, no scheduler progress)"),
        (Source::Watchdog, Level::Warn)
    );
    assert_eq!(
        classify_script_line("script panic: boom"),
        (Source::Script, Level::Error)
    );
    assert_eq!(
        classify_script_line("TypeError: x is undefined"),
        (Source::Script, Level::Error)
    );
    assert_eq!(
        classify_script_line("slow tick 12: 80ms"),
        (Source::Script, Level::Debug)
    );
    assert_eq!(
        classify_script_line("Banking 27 logs"),
        (Source::Script, Level::Info)
    );
}

/// Bytes one bot's full ring costs, measured on realistic lines and at the
/// message cap. `cargo test -p frontend-core ring_memory -- --nocapture`
/// prints the numbers.
#[test]
fn ring_memory_per_bot_is_bounded() {
    let store = LogStore::new();
    for i in 0..SLOT_RING_CAP {
        line(
            &store,
            Some("alice"),
            Source::Script,
            Level::Info,
            &format!("Chopping willow at (3087, 3234), inventory {i}/28, xp 12345"),
        );
    }
    let typical = store.slot_bytes("alice");
    for _ in 0..SLOT_RING_CAP {
        line(
            &store,
            Some("worst"),
            Source::Script,
            Level::Info,
            &"x".repeat(MESSAGE_CAP * 2),
        );
    }
    let worst = store.slot_bytes("worst");
    eprintln!(
        "log ring: entry {} B; typical full ring {typical} B; worst case {worst} B",
        std::mem::size_of::<LogEntry>()
    );
    assert!(typical < 96 * 1024, "typical {typical}");
    assert!(
        worst <= SLOT_RING_CAP * (std::mem::size_of::<LogEntry>() + MESSAGE_CAP) + 64 * 1024,
        "worst {worst}"
    );
}
