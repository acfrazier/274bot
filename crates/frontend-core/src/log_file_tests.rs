use super::*;
use crate::log::{LogStore, Record};
use std::time::{Duration, Instant};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fc-log-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Wait until the writer thread has exited (the file stops changing after
/// the handle is dropped and every expected byte landed).
fn wait_for(mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !done() {
        assert!(Instant::now() < deadline, "writer did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn the_session_file_is_off_unless_the_preference_says_on() {
    let dir = scratch("pref");
    let prefs = dir.join("panel-ui.json");
    assert!(!session_log_setting_at(&prefs), "no prefs file: off");
    std::fs::create_dir_all(&dir).unwrap();
    // A 0.1.8.1 prefs file has no such key.
    std::fs::write(
        &prefs,
        br#"{"last_focus":"alice","collapsed":{},"background_bots_ack":true}"#,
    )
    .unwrap();
    assert!(!session_log_setting_at(&prefs), "absent key: off");

    persist_session_log_setting_at(&prefs, true).unwrap();
    assert!(session_log_setting_at(&prefs));
    let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&prefs).unwrap()).unwrap();
    assert_eq!(value["last_focus"], "alice", "other keys survive");
    assert_eq!(value["background_bots_ack"], true);
    persist_session_log_setting_at(&prefs, false).unwrap();
    assert!(!session_log_setting_at(&prefs));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_store_writes_no_file() {
    let store = LogStore::new();
    assert_eq!(store.file_path(), None);
    assert!(!store.file_open());
}

#[test]
fn the_writer_rotates_by_size_and_keeps_only_its_segments() {
    let dir = scratch("rotate");
    let rotation = Rotation {
        max_bytes: 100,
        segments: 2,
        sessions: 10,
    };
    let file = SessionLogFile::start(dir.clone(), rotation);
    let path = file.path().to_path_buf();
    for i in 0..40 {
        file.send(&format!("line {i:03} ........................"));
    }
    drop(file);
    let last = segment(&path, 1);
    wait_for(|| {
        let live = read(&path);
        live.contains("line 039") || read(&last).contains("line 039")
    });
    assert!(segment(&path, 1).exists());
    assert!(segment(&path, 2).exists());
    assert!(!segment(&path, 3).exists(), "older segments are deleted");
    let total: u64 = [path.clone(), segment(&path, 1), segment(&path, 2)]
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok())
        .map(|m| m.len())
        .sum();
    assert!(total <= 3 * 140, "bounded to the kept segments: {total}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn starting_a_session_prunes_all_but_the_newest_sessions() {
    let dir = scratch("prune");
    std::fs::create_dir_all(&dir).unwrap();
    for day in 10..22 {
        let stem = format!("session-202609{day}-120000-1");
        std::fs::write(dir.join(format!("{stem}.log")), b"x").unwrap();
        std::fs::write(dir.join(format!("{stem}.log.1")), b"x").unwrap();
    }
    std::fs::write(dir.join("alice-20260901-000000.log"), b"saved").unwrap();
    let rotation = Rotation {
        max_bytes: 1 << 20,
        segments: 1,
        sessions: 3,
    };
    let file = SessionLogFile::start(dir.clone(), rotation);
    let path = file.path().to_path_buf();
    file.send("hello");
    drop(file);
    wait_for(|| read(&path).contains("hello"));
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let stems: std::collections::BTreeSet<&str> =
        names.iter().filter_map(|n| session_stem(n)).collect();
    assert_eq!(stems.len(), 3, "{names:?}");
    assert!(stems.contains("session-20260921-120000-1"));
    assert!(stems.contains("session-20260920-120000-1"));
    assert!(
        names.iter().any(|n| n == "alice-20260901-000000.log"),
        "saved logs are not session files"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_session_file_never_receives_a_password() {
    let dir = scratch("secret");
    let store = LogStore::new();
    api::hostlog::register_secret("hunter22");
    let file = SessionLogFile::start(dir.clone(), DEFAULT_ROTATION);
    let path = file.path().to_path_buf();
    store.set_file(Some(file));
    store.push(&Record {
        slot: Some("alice"),
        tick: None,
        source: Source::Login,
        level: Level::Error,
        message: "handshake with password hunter22 refused",
    });
    store.set_file(None);
    wait_for(|| read(&path).contains("refused"));
    let text = read(&path);
    assert!(!text.contains("hunter22"), "{text}");
    assert!(text.contains("alice"));
    assert!(text.contains("error"));
    let _ = std::fs::remove_dir_all(&dir);
}
