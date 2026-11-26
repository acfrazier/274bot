//! S9 qualification uses real SQL and private files, not a simulated store.

use super::*;
use crate::servers::{LaunchProfile, LaunchTransport, LaunchWorld, LoginKey, Servers};

fn fixture(profiles: usize, payload: usize) -> Servers {
    Servers {
        schema_version: 1,
        servers: (0..profiles)
            .map(|index| LaunchProfile {
                name: format!("Server{index:02}"),
                revision: 289,
                transport: LaunchTransport::Tcp,
                worlds: vec![LaunchWorld {
                    number: 1,
                    host: "127.0.0.1".into(),
                    port: 43594,
                    node_id: 1,
                    asset_host: "assets.example".into(),
                    asset_port: 80,
                }],
                login_key: LoginKey::Inline {
                    modulus: "1".repeat(payload),
                    exponent: "65537".into(),
                },
                vault: format!("vault-{index:02}"),
                members: Some(index % 2 == 0),
                allow_plaintext_offhost: false,
            })
            .collect(),
    }
}

fn small_fixture(profiles: usize) -> Servers {
    let mut servers = fixture(profiles, 0);
    for profile in &mut servers.servers {
        profile.login_key = LoginKey::EngineDir { engine_dir: None };
        profile.members = None;
        profile.worlds[0].host = "a".into();
        profile.worlds[0].asset_host = "b".into();
    }
    servers
}

fn world_fixture(count: usize) -> Servers {
    let mut servers = small_fixture(1);
    servers.servers[0].worlds = (1..=count)
        .map(|number| LaunchWorld {
            number: u16::try_from(number).unwrap(),
            host: "a".into(),
            port: 43594,
            node_id: i32::try_from(number).unwrap(),
            asset_host: "b".into(),
            asset_port: 80,
        })
        .collect();
    servers
}

// Counts retained allocation capacities, not transient parsing buffers or allocator metadata.
fn roster_cache_bytes(servers: &Servers) -> usize {
    let profiles = servers
        .servers
        .iter()
        .map(|profile| {
            let key = match &profile.login_key {
                LoginKey::Named(name) => name.capacity(),
                LoginKey::Inline { modulus, exponent } => modulus.capacity() + exponent.capacity(),
                LoginKey::EngineDir { engine_dir } => {
                    engine_dir.as_ref().map_or(0, |path| path.capacity())
                }
            };
            profile.name.capacity()
                + profile.vault.capacity()
                + key
                + profile.worlds.capacity() * std::mem::size_of::<LaunchWorld>()
                + profile
                    .worlds
                    .iter()
                    .map(|world| world.host.capacity() + world.asset_host.capacity())
                    .sum::<usize>()
        })
        .sum::<usize>();
    2 * std::mem::size_of::<usize>()
        + std::mem::size_of::<Servers>()
        + servers.servers.capacity() * std::mem::size_of::<LaunchProfile>()
        + profiles
}

fn root_at(base: &Path, name: &str) -> StateRoot {
    let root = StateRoot::new(base.join(name));
    vault::create_private_dir(root.path()).unwrap();
    root
}

fn seed(root: &StateRoot, bytes: &[u8]) {
    write_private(&root.path().join("servers.json"), bytes);
}

fn receipt(
    root: &StateRoot,
    version: StoreVersion,
    shape: &str,
    restart: bool,
) -> serde_json::Value {
    let conn = inspect(root);
    let (kind, evidence, path, digest): (String, String, String, Option<String>) = conn
        .query_row(
            "SELECT kind, evidence, path, NULLIF(lower(hex(digest)), '') FROM import_ledger
         WHERE domain='servers' AND source='servers.json'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    json!({"domain":"servers", "source":"servers.json", "shape":shape,
        "moment":"startup before profile selection and vault unlock", "committed_version":version.get(),
        "kind":kind,"evidence":evidence,"evidence_filename":Path::new(&path).file_name().unwrap().to_string_lossy(),
        "source_digest_hex":digest,
        "no_source_reason":if kind=="absent" {Some("confirmed absence of servers.json and worlds.json")} else {None},
        "restart_readback":restart,"receipt":"local_state::sql_tests::servers_budget::servers_release_budget"})
}

#[test]
#[ignore = "S9 qualification writes artifacts; run alone with S9_OUT set"]
fn servers_release_budget() {
    if let Some(root) = std::env::var_os("S9_CHILD_ROOT") {
        use std::io::Read;
        let mut go = [0];
        std::io::stdin().read_exact(&mut go).unwrap();
        let root = StateRoot::new(PathBuf::from(root));
        let started = Instant::now();
        let service = LocalStateService::open_process(root.clone());
        let roster = service.wait_server_roster(WAIT).unwrap();
        let heap = sqlite_heap();
        let result = json!({"version":roster.version().get(),"ready_ms":started.elapsed().as_secs_f64()*1000.0,
            "sqlite_heap_current":heap.0,"sqlite_heap_highwater":heap.1});
        let path = root
            .path()
            .join(format!("child-{}.json", std::process::id()));
        std::fs::write(path, serde_json::to_vec(&result).unwrap()).unwrap();
        return;
    }
    let out = PathBuf::from(std::env::var_os("S9_OUT").expect("S9_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let scratch = IsolatedEnv::enter("s9-budget");
    let base = scratch.home.join("qualification");
    vault::create_private_dir(&base).unwrap();
    let mut conversions = Vec::new();
    let mut samples = occupancy_tests::Samples::new();
    let mut low = 0;
    let mut high = usize::try_from(servers::MAX_SOURCE_BYTES).unwrap() / 17;
    // At most 20 candidates find this fixture family's exact storage boundary.
    for attempt in 0..20 {
        if low >= high {
            break;
        }
        let payload = low + (high - low).div_ceil(2);
        let expected = fixture(17, payload);
        let bytes = serde_json::to_vec(&expected).unwrap();
        let root = root_at(&base, &format!("capacity-{attempt}"));
        seed(&root, &bytes);
        let service = LocalStateService::open_process(root.clone());
        match service.wait_server_roster(WAIT) {
            Ok(roster) => {
                assert_eq!(roster.servers(), &expected);
                low = payload;
            }
            Err(ServerRosterError::Content { .. }) => {
                high = payload - 1;
            }
            Err(error) => panic!("unexpected capacity refusal: {error}"),
        }
        drop(service);
        std::fs::remove_dir_all(root.path()).unwrap();
    }
    let rejected = root_at(&base, "first-rejected");
    let rejected_bytes = serde_json::to_vec(&fixture(17, low + 1)).unwrap();
    seed(&rejected, &rejected_bytes);
    reset_sqlite_highwater();
    let rejected_service = LocalStateService::open_process(rejected.clone());
    let rejected_error = rejected_service.wait_server_roster(WAIT).unwrap_err();
    assert!(matches!(&rejected_error, ServerRosterError::Content { .. }));
    let rejected_heap = sqlite_heap();
    let upper_refusal = rejected_error.to_string();
    assert_eq!(
        std::fs::read(rejected.path().join("servers.json")).unwrap(),
        rejected_bytes
    );
    let conn = inspect(&rejected);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM domain_head WHERE domain='servers'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM import_ledger WHERE domain='servers'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    drop(rejected_service);
    conversions.push(json!({"domain":"servers","source":"servers.json","shape":"first size above page reservation",
        "committed_version":null,"source_intact":true,"head_count":0,"ledger_count":0,"refusal":upper_refusal}));
    assert_eq!(low, high);
    let expected = fixture(17, low);
    let bytes = serde_json::to_vec(&expected).unwrap();
    let root = root_at(&base, "maximum");
    seed(&root, &bytes);
    reset_sqlite_highwater();
    let started = Instant::now();
    let (service, timeline) = Timeline::open_timed(&root);
    let roster = service.wait_server_roster(WAIT).unwrap();
    occupancy_tests::record(
        &mut samples,
        "cold_import_selection_ready",
        started.elapsed(),
    );
    assert_eq!(roster.servers(), &expected);
    let heap = sqlite_heap();
    let conn = inspect(&root);
    let pages: u64 = conn
        .query_row("PRAGMA page_count", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    let wal = wal_bytes(&root);
    let profile_rows: u64 = conn
        .query_row("SELECT count(*) FROM server_profile", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    let world_rows: u64 = conn
        .query_row("SELECT count(*) FROM server_world", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    drop(conn);
    let version = roster.version();
    for (class, ms) in timeline.take() {
        samples.entry(class.into()).or_default().push(ms);
    }
    drop(service);
    let started = Instant::now();
    let (service, timeline) = Timeline::open_timed(&root);
    let warm = service.wait_server_roster(WAIT).unwrap();
    occupancy_tests::record(&mut samples, "warm_selection_ready", started.elapsed());
    assert_eq!(warm.version(), version);
    assert_eq!(warm.servers(), &expected);
    for _ in 0..100 {
        let started = Instant::now();
        assert_eq!(
            service
                .server_roster()
                .unwrap()
                .servers()
                .resolve("SERVER16")
                .unwrap(),
            expected.servers[16]
        );
        occupancy_tests::record(&mut samples, "durable_cached_selection", started.elapsed());
    }
    for (class, ms) in timeline.take() {
        samples.entry(class.into()).or_default().push(ms);
    }
    conversions.push(receipt(
        &root,
        version,
        "schema-1 maximum inline public login configuration",
        true,
    ));
    drop(service);
    let legacy = root.path().join("legacy-comparator.json");
    write_private(&legacy, &bytes);
    for _ in 0..10 {
        let started = Instant::now();
        let bytes = std::fs::read(&legacy).unwrap();
        let old: Servers = serde_json::from_slice(&bytes).unwrap();
        old.validate().unwrap();
        occupancy_tests::record(&mut samples, "legacy_server_load", started.elapsed());
    }
    let race = root_at(&base, "competing");
    seed(&race, &bytes);
    use std::io::Write;
    use std::process::{Command, Stdio};
    let workers: Vec<_> = (0..2)
        .map(|_| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "local_state::sql_tests::servers_budget::servers_release_budget",
                ])
                .env("S9_CHILD_ROOT", race.path())
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let mut workers = workers;
    for worker in &mut workers {
        worker.stdin.take().unwrap().write_all(b"G").unwrap();
    }
    let mut competing_heaps = Vec::new();
    let mut results = Vec::new();
    for mut worker in workers {
        let id = worker.id();
        assert!(worker.wait().unwrap().success());
        let bytes = std::fs::read(race.path().join(format!("child-{id}.json"))).unwrap();
        let result: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        samples
            .entry("competing_first_start".into())
            .or_default()
            .push(result["ready_ms"].as_f64().unwrap());
        results.push(result["version"].as_u64().unwrap());
        competing_heaps.push(result["sqlite_heap_highwater"].as_i64().unwrap());
    }
    assert_eq!(results[0], results[1]);
    let service = LocalStateService::open_process(race.clone());
    let restarted = service.wait_server_roster(WAIT).unwrap();
    assert_eq!(restarted.version().get(), results[0]);
    assert_eq!(restarted.servers(), &expected);
    conversions.push(receipt(
        &race,
        restarted.version(),
        "schema-1 maximum-roster competing first processes",
        true,
    ));
    drop(service);
    let absent = root_at(&base, "absent");
    let service = LocalStateService::open_process(absent.clone());
    let defaults = service.wait_server_roster(WAIT).unwrap();
    assert_eq!(defaults.servers(), &Servers::builtins());
    let version = defaults.version();
    drop(service);
    let service = LocalStateService::open_process(absent.clone());
    assert_eq!(service.wait_server_roster(WAIT).unwrap().version(), version);
    conversions.push(receipt(&absent, version, "absent", true));
    drop(service);
    let mut row_low = 1usize;
    let mut row_high = usize::try_from(servers::MAX_SOURCE_BYTES).unwrap()
        / serde_json::to_vec(&small_fixture(1)).unwrap().len();
    for attempt in 0..17 {
        if row_low >= row_high {
            break;
        }
        let count = row_low + (row_high - row_low).div_ceil(2);
        let candidate = small_fixture(count);
        let root = root_at(&base, &format!("row-capacity-{attempt}"));
        seed(&root, &serde_json::to_vec(&candidate).unwrap());
        let service = LocalStateService::open_process(root.clone());
        match service.wait_server_roster(WAIT) {
            Ok(roster) => {
                assert_eq!(roster.servers(), &candidate);
                row_low = count;
            }
            Err(ServerRosterError::Content { .. }) => row_high = count - 1,
            Err(error) => panic!("unexpected row capacity refusal: {error}"),
        }
        drop(service);
        std::fs::remove_dir_all(root.path()).unwrap();
    }
    assert_eq!(row_low, row_high);
    let rows_root = root_at(&base, "maximum-rows");
    let row_expected = small_fixture(row_low);
    let row_bytes = serde_json::to_vec(&row_expected).unwrap();
    seed(&rows_root, &row_bytes);
    reset_sqlite_highwater();
    let started = Instant::now();
    let (service, timeline) = Timeline::open_timed(&rows_root);
    let row_roster = service.wait_server_roster(WAIT).unwrap();
    occupancy_tests::record(
        &mut samples,
        "maximum_rows_selection_ready",
        started.elapsed(),
    );
    assert_eq!(row_roster.servers(), &row_expected);
    let row_heap = sqlite_heap();
    let row_pages: u64 = inspect(&rows_root)
        .query_row("PRAGMA page_count", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    let row_wal = wal_bytes(&rows_root);
    for (class, ms) in timeline.take() {
        samples.entry(class.into()).or_default().push(ms);
    }
    let row_version = row_roster.version();
    drop(service);
    let service = LocalStateService::open_process(rows_root.clone());
    assert_eq!(
        service.wait_server_roster(WAIT).unwrap().servers(),
        &row_expected
    );
    conversions.push(receipt(
        &rows_root,
        row_version,
        "maximum-count schema-1 engine-key profiles",
        true,
    ));
    drop(service);
    let row_rejected_root = root_at(&base, "first-rejected-count");
    let row_rejected_bytes = serde_json::to_vec(&small_fixture(row_low + 1)).unwrap();
    seed(&row_rejected_root, &row_rejected_bytes);
    let service = LocalStateService::open_process(row_rejected_root.clone());
    let row_refusal = service.wait_server_roster(WAIT).unwrap_err();
    assert!(matches!(&row_refusal, ServerRosterError::Content { .. }));
    assert_eq!(
        std::fs::read(row_rejected_root.path().join("servers.json")).unwrap(),
        row_rejected_bytes
    );
    let conn = inspect(&row_rejected_root);
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM domain_head WHERE domain='servers'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        conn.query_row(
            "SELECT count(*) FROM import_ledger WHERE domain='servers'",
            [],
            |row| row.get::<_, u32>(0)
        )
        .unwrap(),
        0
    );
    drop(conn);
    drop(service);
    conversions.push(json!({"domain":"servers","source":"servers.json","shape":"first profile count above page reservation",
        "committed_version":null,"source_intact":true,"head_count":0,"ledger_count":0,"refusal":row_refusal.to_string()}));
    let mut world_low = 1usize;
    let mut world_high = usize::from(u16::MAX);
    for attempt in 0..16 {
        if world_low >= world_high {
            break;
        }
        let count = world_low + (world_high - world_low).div_ceil(2);
        let candidate = world_fixture(count);
        let root = root_at(&base, &format!("world-capacity-{attempt}"));
        seed(&root, &serde_json::to_vec(&candidate).unwrap());
        let service = LocalStateService::open_process(root.clone());
        match service.wait_server_roster(WAIT) {
            Ok(roster) => {
                assert_eq!(roster.servers(), &candidate);
                world_low = count;
            }
            Err(ServerRosterError::Content { .. }) => world_high = count - 1,
            Err(error) => panic!("unexpected world capacity refusal: {error}"),
        }
        drop(service);
        std::fs::remove_dir_all(root.path()).unwrap();
    }
    assert_eq!(world_low, world_high);
    let world_root = root_at(&base, "maximum-worlds");
    let world_expected = world_fixture(world_low);
    let world_bytes = serde_json::to_vec(&world_expected).unwrap();
    seed(&world_root, &world_bytes);
    reset_sqlite_highwater();
    let started = Instant::now();
    let (service, timeline) = Timeline::open_timed(&world_root);
    let world_roster = service.wait_server_roster(WAIT).unwrap();
    occupancy_tests::record(
        &mut samples,
        "maximum_worlds_selection_ready",
        started.elapsed(),
    );
    assert_eq!(world_roster.servers(), &world_expected);
    let world_heap = sqlite_heap();
    let world_pages: u64 = inspect(&world_root)
        .query_row("PRAGMA page_count", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    let world_wal = wal_bytes(&world_root);
    for (class, ms) in timeline.take() {
        samples.entry(class.into()).or_default().push(ms);
    }
    let world_version = world_roster.version();
    drop(service);
    let service = LocalStateService::open_process(world_root.clone());
    assert_eq!(
        service.wait_server_roster(WAIT).unwrap().servers(),
        &world_expected
    );
    conversions.push(receipt(
        &world_root,
        world_version,
        "maximum-count worlds in one schema-1 engine-key profile",
        true,
    ));
    drop(service);
    let mut single_low = 1usize;
    let mut single_high = usize::try_from(sql::DOCUMENT_MAX).unwrap();
    for attempt in 0..20 {
        if single_low >= single_high {
            break;
        }
        let payload = single_low + (single_high - single_low).div_ceil(2);
        let candidate = fixture(1, payload);
        let root = root_at(&base, &format!("single-capacity-{attempt}"));
        seed(&root, &serde_json::to_vec(&candidate).unwrap());
        let service = LocalStateService::open_process(root.clone());
        match service.wait_server_roster(WAIT) {
            Ok(roster) => {
                assert_eq!(roster.servers(), &candidate);
                single_low = payload;
            }
            Err(ServerRosterError::Content { .. }) => single_high = payload - 1,
            Err(error) => panic!("unexpected single-row capacity refusal: {error}"),
        }
        drop(service);
        std::fs::remove_dir_all(root.path()).unwrap();
    }
    assert_eq!(single_low, single_high);
    let single_root = root_at(&base, "maximum-single-profile");
    let single_expected = fixture(1, single_low);
    let single_bytes = serde_json::to_vec(&single_expected).unwrap();
    seed(&single_root, &single_bytes);
    reset_sqlite_highwater();
    let service = LocalStateService::open_process(single_root.clone());
    assert_eq!(
        service.wait_server_roster(WAIT).unwrap().servers(),
        &single_expected
    );
    let single_heap = sqlite_heap();
    let single_pages: u64 = inspect(&single_root)
        .query_row("PRAGMA page_count", [], |row| {
            row.get::<_, u32>(0).map(u64::from)
        })
        .unwrap();
    let single_wal = wal_bytes(&single_root);
    drop(service);
    let single_rejected = root_at(&base, "first-rejected-single-profile");
    let single_rejected_bytes = serde_json::to_vec(&fixture(1, single_low + 1)).unwrap();
    seed(&single_rejected, &single_rejected_bytes);
    let service = LocalStateService::open_process(single_rejected.clone());
    let single_refusal = service.wait_server_roster(WAIT).unwrap_err();
    assert!(matches!(&single_refusal, ServerRosterError::Content { .. }));
    assert_eq!(
        std::fs::read(single_rejected.path().join("servers.json")).unwrap(),
        single_rejected_bytes
    );
    let conn = inspect(&single_rejected);
    let (single_heads, single_ledgers): (u32, u32) = conn
        .query_row(
            "SELECT (SELECT count(*) FROM domain_head WHERE domain='servers'),
                (SELECT count(*) FROM import_ledger WHERE domain='servers')",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!((single_heads, single_ledgers), (0, 0));
    drop(conn);
    drop(service);
    let timing_summary = occupancy_tests::summarize(&samples);
    let whole_job_over_limit = samples
        .get("servers-readback")
        .unwrap()
        .iter()
        .filter(|&&ms| ms > 250.0)
        .count();
    let budget = json!({
        "boundary":"existing SQLite capacity, no budget increase", "source_read_bound_bytes":servers::MAX_SOURCE_BYTES,
        "sqlite_row_length_limit_bytes":sql::DOCUMENT_MAX+4096, "db_max_pages":sql::DB_MAX_PAGES,
        "page_size":sql::PAGE_SIZE,"wal_max_bytes":sql::WAL_MAX_BYTES,"sqlite_heap_gate_bytes":sql::SOFT_HEAP_LIMIT,
        "transaction_page_reservation":sql::MAX_PAGES,"first_rejected_source_bytes":rejected_bytes.len(),
        "first_rejected_sqlite_heap_highwater":rejected_heap.1,
        "queued_roster_hydrations":1,"active_roster_hydrations":1,"retained_roster_snapshots":1,
        "key_bytes_upper_bound":sql::DOCUMENT_MAX+4096,"fixture_profiles":profile_rows,"fixture_worlds":world_rows,
        "profile_row_upper_bound":servers::MAX_SOURCE_BYTES/2,"world_row_upper_bound":servers::MAX_SOURCE_BYTES/2,
        "row_bound_derivation":"each admitted JSON row consumes at least two source bytes; SQL page reservation is the tighter dynamic bound",
        "retained_service_cache_encoded_bytes_bound":servers::MAX_SOURCE_BYTES,
        "retained_cache_allocation_bytes":{"cold":roster_cache_bytes(roster.servers()),
            "warm":roster_cache_bytes(warm.servers()),"maximum_profiles":roster_cache_bytes(row_roster.servers()),
            "maximum_worlds":roster_cache_bytes(world_roster.servers())},
        "retained_cache_measurement":"Arc header, values, vector and string allocation capacities; excludes allocator metadata",
        "largest_fixture_payload_per_profile":low,"source_bytes":bytes.len(),"canonical_bytes":serde_json::to_vec(&expected).unwrap().len(),
        "maximum_count_fixture":{"profiles":row_low,"worlds":row_low,"source_bytes":row_bytes.len(),
            "pages":row_pages,"wal_bytes":row_wal,"sqlite_heap_highwater":row_heap.1,
            "first_rejected_profiles":row_low+1,"first_rejected_source_bytes":row_rejected_bytes.len(),
            "first_rejected_refusal":row_refusal.to_string(),"first_rejected_source_intact":true,
            "first_rejected_head_count":0,"first_rejected_ledger_count":0},
        "staged_evidence_bytes":bytes.len(),"pages":pages,"wal_bytes":wal,"sqlite_heap_current":heap.0,"sqlite_heap_highwater":heap.1,
        "maximum_world_fixture":{"profiles":1,"worlds":world_low,"source_bytes":world_bytes.len(),
            "pages":world_pages,"wal_bytes":world_wal,"sqlite_heap_highwater":world_heap.1},
        "single_large_profile_fixture":{"source_bytes":single_bytes.len(),"key_payload_bytes":single_low,
            "pages":single_pages,"wal_bytes":single_wal,"sqlite_heap_highwater":single_heap.1,
            "first_rejected_source_bytes":single_rejected_bytes.len(),"first_rejected_source_intact":true,
            "first_rejected_head_count":single_heads,"first_rejected_ledger_count":single_ledgers,
            "first_rejected_refusal":single_refusal.to_string()},
        "next_larger_fixture_refusal":upper_refusal,"timings_ms":timing_summary,
        "whole_job_over_250ms":whole_job_over_limit,
        "competing_sqlite_heap_highwaters":competing_heaps,
        "largest_scope":"exact boundaries of key-volume, profile-count and world-count fixture families; one large key stresses the row shape",
    });
    std::fs::write(
        out.join("servers-budget.json"),
        serde_json::to_vec_pretty(&budget).unwrap(),
    )
    .unwrap();
    std::fs::write(
        out.join("conversion.json"),
        serde_json::to_vec_pretty(&json!({"sources":conversions})).unwrap(),
    )
    .unwrap();
    assert!(pages <= sql::DB_MAX_PAGES);
    assert!(wal <= sql::WAL_MAX_BYTES);
    assert!(
        heap.1 <= sql::SOFT_HEAP_LIMIT,
        "SQLite heap {} exceeds {}",
        heap.1,
        sql::SOFT_HEAP_LIMIT
    );
    assert!(row_pages <= sql::DB_MAX_PAGES && row_wal <= sql::WAL_MAX_BYTES);
    assert!(
        row_heap.1 <= sql::SOFT_HEAP_LIMIT,
        "maximum row-count heap exceeded"
    );
    assert!(single_pages <= sql::DB_MAX_PAGES && single_wal <= sql::WAL_MAX_BYTES);
    assert!(
        single_heap.1 <= sql::SOFT_HEAP_LIMIT,
        "single large row heap exceeded: {}",
        single_heap.1
    );
    assert_eq!(
        whole_job_over_limit, 0,
        "server hydration exceeded the whole-job gate"
    );
    assert!(competing_heaps
        .iter()
        .all(|&heap| heap <= sql::SOFT_HEAP_LIMIT));
    assert!(world_pages <= sql::DB_MAX_PAGES && world_wal <= sql::WAL_MAX_BYTES);
    assert!(
        world_heap.1 <= sql::SOFT_HEAP_LIMIT,
        "maximum world-count heap exceeded"
    );
}
