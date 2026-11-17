//! `check_private_path` is metadata only: it opens no descriptor. Its own
//! test file, so this process holds one test thread and no other test opens
//! or closes a descriptor while the table is counted. A caller that keeps a
//! SQLite connection open on the path relies on this: opening and closing a
//! second descriptor beside SQLite cancels its POSIX advisory locks.

#![cfg(target_os = "linux")]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use vault::{check_private_path, PrivateKind};

/// Every open descriptor as `number: target`, sorted. The listing's own
/// directory handle appears in each snapshot alike.
fn open_descriptors() -> Vec<String> {
    let mut targets: Vec<String> = fs::read_dir("/proc/self/fd")
        .expect("read the descriptor table")
        .map(|entry| {
            let entry = entry.expect("descriptor entry");
            let target = fs::read_link(entry.path()).unwrap_or_default();
            format!(
                "{}: {}",
                entry.file_name().to_string_lossy(),
                target.display()
            )
        })
        .collect();
    targets.sort();
    targets
}

fn write_with_mode(path: &Path, mode: u32) {
    fs::write(path, b"{}").expect("write the fixture");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("set the fixture mode");
}

#[test]
fn the_check_opens_and_leaves_open_no_descriptor() {
    let dir =
        std::env::temp_dir().join(format!("274bot-check-private-path-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create the scratch directory");
    let ok = dir.join("ok");
    write_with_mode(&ok, 0o600);
    let wide = dir.join("wide");
    write_with_mode(&wide, 0o664);
    let link = dir.join("link");
    std::os::unix::fs::symlink(&ok, &link).expect("create the symlink");
    let missing = dir.join("missing");

    let before = open_descriptors();
    check_private_path(&ok, PrivateKind::File).expect("an owner-only file is accepted");
    check_private_path(&dir, PrivateKind::Dir).expect("a private directory is accepted");
    for refused in [&wide, &link, &missing] {
        check_private_path(refused, PrivateKind::File)
            .expect_err("a wide file, a symlink and a missing path are not accepted");
    }
    let after = open_descriptors();

    assert_eq!(before, after, "no descriptor was opened or left open");
    let _ = fs::remove_dir_all(&dir);
}
