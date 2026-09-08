use super::*;
use std::collections::BTreeMap;
use std::sync::Mutex;
static SERIAL: Mutex<()> = Mutex::new(());
struct Fixture(std::path::PathBuf, Value);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("heaptrack-migration-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let mut entries = json!({});
        for (kind, data) in [
            (
                "raw",
                b"v 10500 3\nx 1 x\nX fixture\nI 1000 100\nt 10 0\n+ a 1 10\nc a\n".as_slice(),
            ),
            (
                "interpreted",
                b"v 10500 3\nX fixture\nI 1000 100\ns 1 f\ni 10 0 1\nt 1 0\na a 1\n+ 0\nc a\n"
                    .as_slice(),
            ),
            ("peak", b"f; 10\n".as_slice()),
        ] {
            let path = root.join(kind);
            std::fs::write(&path, data).unwrap();
            entries[kind] = json!({"path":path,"bytes":data.len(),"sha256":hash(data)});
        }
        Self(root, entries)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}
fn guard() -> Guard {
    Guard::new(Limits::new(&json!({})).unwrap(), false)
}
#[test]
fn actual_stream_open_counts() {
    let _lock = SERIAL.lock().unwrap();
    let f = Fixture::new();
    for count in &OPEN_COUNTS {
        count.store(0, Ordering::SeqCst);
    }
    let mut g = guard();
    crate::replay::analyze(&f.1, None, &mut g).unwrap();
    assert_eq!(
        OPEN_COUNTS.each_ref().map(|n| n.load(Ordering::SeqCst)),
        [1, 2, 1]
    );
}
#[test]
fn frozen_identity_hash_suffix_and_midpass_mutation() {
    let _lock = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let mut g = guard();
    let mut bad = f.1["raw"].clone();
    bad["sha256"] = json!("0".repeat(64));
    assert!(crate::replay::raw_pass(&bad, &mut g).is_err());
    let mut stream = Input::open(&f.1["raw"], 2_000_000, MIB).unwrap();
    let mut line = Vec::new();
    stream.next(&mut line, &mut g).unwrap();
    let p = f.0.join("raw");
    let mut data = std::fs::read(&p).unwrap();
    data[0] = b'#';
    std::fs::write(&p, &data).unwrap();
    while stream.next(&mut line, &mut g).unwrap().is_some() {}
    assert!(stream.finish(&mut g).is_err());
    data.truncate(data.len() - 4);
    std::fs::write(&p, &data).unwrap();
    assert!(Input::open(&f.1["raw"], 2_000_000, MIB).is_err());
}
#[test]
fn interpreted_mutation_between_passes() {
    let _lock = SERIAL.lock().unwrap();
    let f = Fixture::new();
    let mut g = guard();
    let mut t = crate::replay::Tables::new();
    crate::replay::interpreted(&f.1["interpreted"], &mut t, None, None, &mut g).unwrap();
    let p = f.0.join("interpreted");
    let data = std::fs::read_to_string(&p).unwrap().replace("+ 0", "- 0");
    std::fs::write(p, data).unwrap();
    assert!(
        crate::replay::interpreted(
            &f.1["interpreted"],
            &mut t,
            None,
            Some(&BTreeMap::new()),
            &mut g
        )
        .is_err()
    );
}
#[test]
fn normalized_batching_exact_bytes() {
    for size in [0, 1, 65535, 65536, 65537, 131073] {
        let bytes: Vec<u8> = (0..size).map(|n| (n % 251) as u8).collect();
        for chunk in [1, 17, 65536] {
            let mut h = BufferedDigest::new();
            for part in bytes.chunks(chunk) {
                h.update(part);
            }
            assert_eq!(digest(h), hash(&bytes));
        }
    }
}
