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
    let refused = [
        (
            root.join("loadouts.json"),
            br#"[{"name":"old","worn":["Bronze sword"],"carry":["Coins"]}]"#.as_slice(),
        ),
        (
            root.join("worlds.json"),
            br#"{"schema_version":1,"worlds":[{"number":1,"host":"w1.rs2b2t.com","port":443,"node_id":10}]}"#.as_slice(),
        ),
        (
            root.join("bank-hints/local-289/unopened.json"),
            br#"{"schema_version":1,"profile":"local-289","account":"unopened","observed_at_unix":3,"rows":[[995,3]]}"#.as_slice(),
        ),
    ];
    for (path, bytes) in &refused {
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
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            if path.parent() == Some(root.as_path()) {
                4
            } else {
                1
            },
            "no move-aside backup is created for {}",
            path.display()
        );
    }
    assert_eq!(std::fs::read(preferences).unwrap(), preference_bytes);
    assert!(!root.join("servers.json").exists());
    assert!(!root.join("vault-289").exists());
}
