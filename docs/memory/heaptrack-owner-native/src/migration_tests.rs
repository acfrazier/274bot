use super::*;
use std::collections::BTreeMap;
static NEXT_FIXTURE: AtomicUsize = AtomicUsize::new(0);
struct Fixture(std::path::PathBuf, Value);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "heaptrack-migration-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::SeqCst)
        ));
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
    let f = Fixture::new();
    OPEN_COUNTS.with(|counts| counts.set([0; 3]));
    let mut g = guard();
    crate::replay::analyze(&f.1, None, &mut g).unwrap();
    assert_eq!(OPEN_COUNTS.with(|counts| counts.get()), [1, 2, 1]);
}
#[test]
fn frozen_expected_hash_rejected() {
    let f = Fixture::new();
    let mut g = guard();
    let mut bad = f.1["raw"].clone();
    bad["sha256"] = json!("0".repeat(64));
    assert_eq!(
        crate::replay::raw_pass(&bad, &mut g).err(),
        Some("input hash mismatch")
    );
}
#[test]
fn midpass_metadata_change_rejected() {
    let f = Fixture::new();
    let mut g = guard();
    let mut stream = Input::open(&f.1["raw"], 2_000_000, MIB).unwrap();
    let mut line = Vec::new();
    stream.next(&mut line, &mut g).unwrap();
    let p = f.0.join("raw");
    let mut data = std::fs::read(&p).unwrap();
    assert_eq!(stream.reader.buffer(), &data[line.len()..]);
    data[0] = b'#';
    std::fs::write(&p, &data).unwrap();
    // Deliberately establish the tested precondition, independent of clock
    // granularity or elapsed time. No sleep and no assumption about ctime.
    let file = OpenOptions::new().write(true).open(&p).unwrap();
    file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1))
        .unwrap();
    let after = file.metadata().unwrap();
    assert_ne!(identity(&stream.before), identity(&after));
    assert_eq!(stream.before.len(), after.len());
    while stream.next(&mut line, &mut g).unwrap().is_some() {}
    assert_eq!(
        digest(stream.sha.clone()),
        f.1["raw"]["sha256"].as_str().unwrap()
    );
    assert_eq!(stream.finish(&mut g), Err("input changed during pass"));
}
#[test]
fn frozen_suffix_size_rejected() {
    let f = Fixture::new();
    let p = f.0.join("raw");
    let mut data = std::fs::read(&p).unwrap();
    data.truncate(data.len() - 4);
    std::fs::write(&p, &data).unwrap();
    assert_eq!(
        Input::open(&f.1["raw"], 2_000_000, MIB).err(),
        Some("input identity/size")
    );
}
#[test]
fn same_size_content_change_rejected_by_hash() {
    let f = Fixture::new();
    let p = f.0.join("raw");
    // Legal record content, unchanged length: no grammar/accounting failure
    // can masquerade as a content-identity rejection.
    let data = std::fs::read_to_string(&p)
        .unwrap()
        .replace("X fixture", "X altered");
    std::fs::write(&p, data).unwrap();
    let mut g = guard();
    assert_eq!(
        crate::replay::raw_pass(&f.1["raw"], &mut g).err(),
        Some("input hash mismatch")
    );
}
#[test]
fn buffered_write_with_simulated_equal_metadata_keeps_consumed_hash() {
    let f = Fixture::new();
    let mut g = guard();
    let mut stream = Input::open(&f.1["raw"], 2_000_000, MIB).unwrap();
    let mut line = Vec::new();
    stream.next(&mut line, &mut g).unwrap();
    let p = f.0.join("raw");
    let original = std::fs::read(&p).unwrap();
    assert_eq!(stream.reader.buffer(), &original[line.len()..]);
    let mut changed = original.clone();
    changed[0] = b'#';
    std::fs::write(&p, &changed).unwrap();
    // Model a filesystem timestamp collision at the private test seam. This
    // is not evidence that this machine's filesystem actually collided.
    stream.before = stream.reader.get_ref().metadata().unwrap();
    let mut consumed = line.clone();
    while stream.next(&mut line, &mut g).unwrap().is_some() {
        consumed.extend_from_slice(&line);
    }
    assert_eq!(consumed, original);
    assert_ne!(hash(&std::fs::read(&p).unwrap()), hash(&consumed));
    assert_eq!(stream.finish(&mut g).unwrap(), hash(&consumed));
}
#[test]
fn unread_write_with_simulated_equal_metadata_rejected_by_hash() {
    let f = Fixture::new();
    let p = f.0.join("raw");
    let mut data = b"# padding\n".repeat(7000);
    std::fs::write(&p, &data).unwrap();
    let entry = json!({"path":p,"bytes":data.len(),"sha256":hash(&data)});
    let mut stream = Input::open(&entry, 2_000_000, MIB).unwrap();
    let mut g = guard();
    let mut line = Vec::new();
    stream.next(&mut line, &mut g).unwrap();
    assert!(stream.offset as usize + stream.reader.buffer().len() < 69991);
    data[69991] = b'!';
    std::fs::write(&p, &data).unwrap();
    // Hold metadata comparison equal to isolate the independent hash gate.
    stream.before = stream.reader.get_ref().metadata().unwrap();
    while stream.next(&mut line, &mut g).unwrap().is_some() {}
    assert_eq!(stream.finish(&mut g), Err("input hash mismatch"));
}
#[test]
fn interpreted_mutation_between_passes() {
    let f = Fixture::new();
    let mut g = guard();
    let mut t = crate::replay::Tables::new();
    let (first, _) =
        crate::replay::interpreted(&f.1["interpreted"], &mut t, None, None, &mut g).unwrap();
    let checkpoints = BTreeMap::from([
        ("peak".into(), first["peak"]["line"].as_u64().unwrap()),
        (
            "last_mark".into(),
            first["last_mark"]["line"].as_u64().unwrap(),
        ),
    ]);
    let p = f.0.join("interpreted");
    let data = std::fs::read_to_string(&p)
        .unwrap()
        .replace("X fixture", "X altered");
    std::fs::write(p, data).unwrap();
    assert_eq!(
        crate::replay::interpreted(
            &f.1["interpreted"],
            &mut t,
            None,
            Some(&checkpoints),
            &mut g
        )
        .err(),
        Some("input hash mismatch")
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
