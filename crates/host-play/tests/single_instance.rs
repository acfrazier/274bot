use host_play::local_state::{RootUse, StateRoot};
use std::process::Command;

#[test]
fn host_binary_refuses_a_held_root_before_loading_resources() {
    let scratch = script::IsolatedEnv::enter("host-binary-instance");
    let root = StateRoot::new(scratch.home.join(".274bot"));
    let _lock = RootUse::acquire(&root).unwrap();
    let legacy = root.path().join("panel-ui.json");
    std::fs::write(&legacy, b"not JSON").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_host-play"))
        .env("HOME", &scratch.home)
        .env("USERPROFILE", &scratch.home)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("Another 274bot process"), "{error}");
    assert!(error.contains(root.path().to_str().unwrap()), "{error}");
    assert_eq!(std::fs::read(&legacy).unwrap(), b"not JSON");
    assert!(!root.database_file().exists());
}
