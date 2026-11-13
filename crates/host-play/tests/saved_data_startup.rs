//! The command-line host must refuse old stores before normal startup work.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_HOME: AtomicUsize = AtomicUsize::new(0);

struct TestHome(PathBuf);

impl TestHome {
    fn new() -> Self {
        loop {
            let path = std::env::temp_dir().join(format!(
                "274bot-saved-startup-{}-{}",
                std::process::id(),
                NEXT_HOME.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create test home {}: {error}", path.display()),
            }
        }
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_collects_old_lazy_stores_and_quits_without_moving_or_initializing() {
    let home = TestHome::new();
    let root = home.0.join(".274bot");
    let preferences = root.join("panel-ui.json");
    let preference_bytes = br#"{"server_revision":289}"#;
    vault::write_private_file(&preferences, preference_bytes).unwrap();
    // Shapes older than anything 0.2.8 read are refused and named.
    let refused = [
        (
            root.join("worlds.json"),
            br#"{"schema_version":1,"worlds":[{"number":1,"host":"w1.rs2b2t.com","port":443,"node_id":10}]}"#.as_slice(),
        ),
        (
            root.join("bank-hints/local-289/unversioned.json"),
            br#"{"profile":"local-289","account":"unversioned","observed_at_unix":3,"rows":[[995,3]]}"#.as_slice(),
        ),
    ];
    // Shapes 0.2.8 read as its own (legacy loadout rows, an unscoped
    // schema-1 hint) load: the startup check neither names nor touches them.
    let supported = [
        (
            root.join("loadouts.json"),
            br#"[{"name":"old","worn":["Bronze sword"],"carry":["Coins"]}]"#.as_slice(),
        ),
        (
            root.join("bank-hints/local-289/unopened.json"),
            br#"{"schema_version":1,"profile":"local-289","account":"unopened","observed_at_unix":3,"rows":[[995,3]]}"#.as_slice(),
        ),
    ];
    for (path, bytes) in refused.iter().chain(&supported) {
        vault::write_private_file(path, bytes).unwrap();
    }

    let output = Command::new(env!("CARGO_BIN_EXE_host-play"))
        .args(["--profile", "local-289", "--vault-pass-stdin"])
        .env("HOME", &home.0)
        .env("USERPROFILE", &home.0)
        .stdin(Stdio::null())
        .env_remove("ENGINE_DIR")
        .output()
        .expect("run the real command-line host");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains("from before 0.2.8"), "{stderr}");
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains('?'),
        "the CLI must not ask a question"
    );
    assert!(
        !stderr.contains('?'),
        "the CLI must not ask a question: {stderr}"
    );
    for (path, bytes) in &refused {
        assert!(stderr.contains(path.to_string_lossy().as_ref()), "{stderr}");
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    for (path, bytes) in &supported {
        assert!(
            !stderr.contains(path.to_string_lossy().as_ref()),
            "a 0.2.8 shape must not be refused: {stderr}"
        );
        assert_eq!(std::fs::read(path).unwrap(), *bytes);
    }
    // No move-aside backup is created: the root holds panel-ui.json,
    // loadouts.json, worlds.json and bank-hints; the hint folder holds the two
    // hints.
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 4);
    assert_eq!(
        std::fs::read_dir(root.join("bank-hints/local-289"))
            .unwrap()
            .count(),
        2
    );
    assert_eq!(std::fs::read(preferences).unwrap(), preference_bytes);
    assert!(!root.join("servers.json").exists());
    assert!(!root.join("vault-289").exists());
}
