use super::*;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

const CHILD: &str = "local_state::root_use::process_tests::state_root_child";

#[test]
#[ignore = "state-root child process"]
fn state_root_child() {
    let root = StateRoot::new(PathBuf::from(std::env::var_os("STATE_ROOT_CHILD").unwrap()));
    let lock = match RootUse::acquire(&root) {
        Ok(lock) => lock,
        Err(error @ StateRootLockError::InUse { .. }) => {
            eprintln!("state-root-in-use: {error}");
            std::process::exit(1);
        }
        Err(error) => panic!("unexpected startup error: {error}"),
    };
    let service = crate::local_state::LocalStateService::open_process_with(
        root,
        crate::local_state::Frontend::Headless,
        Some(lock),
    );
    service.preferences().reload().unwrap();
    println!("\nstate-root-ready");
    let _ = std::io::stdin().read(&mut [0u8; 1]);
    drop(service);
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(crate::test_process::executable());
    command
        .args(["--ignored", "--exact", "--nocapture", CHILD])
        .env("STATE_ROOT_CHILD", root);
    command
}

struct Holder {
    child: Child,
    reader: Option<JoinHandle<()>>,
}

impl Holder {
    fn start(root: &Path) -> Self {
        let mut child = crate::test_process::spawn(
            command(root)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit()),
        )
        .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().trim() == "state-root-ready" {
                    let _ = tx.send(());
                }
            }
        });
        let holder = Self {
            child,
            reader: Some(reader),
        };
        rx.recv_timeout(Duration::from_secs(20))
            .expect("the child opens its real database before readiness");
        holder
    }

    fn stop(mut self, kill: bool) {
        if kill {
            self.child.kill().unwrap();
        } else {
            drop(self.child.stdin.take());
        }
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert_eq!(status.success(), !kill, "child exit: {status}");
                break;
            }
            assert!(Instant::now() < deadline, "child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for Holder {
    fn drop(&mut self) {
        if self.child.try_wait().unwrap().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        if let Some(reader) = self.reader.take() {
            reader.join().unwrap();
        }
    }
}

fn snapshot(root: &Path) -> BTreeMap<PathBuf, (Vec<u8>, SystemTime)> {
    fn visit(root: &Path, dir: &Path, files: &mut BTreeMap<PathBuf, (Vec<u8>, SystemTime)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, files);
            } else {
                let metadata = entry.metadata().unwrap();
                let bytes = if metadata.len() == 0 {
                    // Windows can deny reads even at EOF on a locked file.
                    // Its zero length already proves that it has no bytes.
                    Vec::new()
                } else {
                    std::fs::read(&path).unwrap_or_else(|error| {
                        panic!("Cannot snapshot {}: {error}", path.display())
                    })
                };
                files.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    (bytes, metadata.modified().unwrap()),
                );
            }
        }
    }
    let mut files = BTreeMap::new();
    visit(root, root, &mut files);
    files
}

#[test]
fn second_process_refuses_without_touching_store_and_exit_releases_lock() {
    let scratch = script::IsolatedEnv::enter("single-instance-process");
    let root = scratch.home.join(".274bot");
    for kill in [false, true] {
        let holder = Holder::start(&root);
        let before = snapshot(&root);
        assert!(before.contains_key(Path::new("state/state.sqlite3")));
        let output = crate::test_process::spawn(
            command(&root)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped()),
        )
        .unwrap()
        .wait_with_output()
        .unwrap();
        assert_eq!(output.status.code(), Some(1), "{output:?}");
        let error = String::from_utf8(output.stderr).unwrap();
        assert!(error.contains("state-root-in-use:"), "{error}");
        assert!(error.contains("Another 274bot process"), "{error}");
        assert!(error.contains(root.to_str().unwrap()), "{error}");
        assert_eq!(before, snapshot(&root), "refusal changed the store");
        holder.stop(kill);
        Holder::start(&root).stop(false);
    }
}

#[test]
fn different_roots_open_databases_concurrently() {
    let scratch = script::IsolatedEnv::enter("single-instance-independent");
    let first = Holder::start(&scratch.home.join("alice/.274bot"));
    let second = Holder::start(&scratch.home.join("bob/.274bot"));
    first.stop(false);
    second.stop(false);
}
