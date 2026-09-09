// Diagnostic sibling of frozen production modules; no replacement router/decoder.
use crate::bank_fetch::{plan_bank_fetch, BankStep};
use crate::router::{self, CostModel, FindOptions, Leg, Route, RouteError};
use crate::router::{find_missing_item_reqs, find_with};
use crate::world::NavWorld;
use crate::WorldState;
use api::snapshot::WorldTile;
use std::{collections::VecDeque, hint::black_box, io::Write, sync::Arc, time::Instant};
include!("host-probe.rs");
include!("stage-facts.rs");
include!("stage-allocator.rs");

fn cpu() -> u64 {
    unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut r), 0);
        ((r.ru_utime.tv_sec + r.ru_stime.tv_sec) as u64) * 1_000_000_000
            + ((r.ru_utime.tv_usec + r.ru_stime.tv_usec) as u64) * 1000
    }
}
fn peak() -> u64 {
    unsafe {
        let mut r: libc::rusage = std::mem::zeroed();
        assert_eq!(libc::getrusage(libc::RUSAGE_SELF, &mut r), 0);
        (r.ru_maxrss as u64) * if cfg!(target_os = "linux") { 1024 } else { 1 }
    }
}
#[cfg(target_os = "linux")]
fn rss() -> u64 {
    let data = std::fs::read_to_string("/proc/self/statm").expect("current RSS required");
    let pages: u64 = data.split_whitespace().nth(1).unwrap().parse().unwrap();
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    assert!(page > 0);
    pages * page as u64
}
#[cfg(target_os = "macos")]
fn rss() -> u64 {
    unsafe {
        let mut info: libc::mach_task_basic_info = std::mem::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        #[allow(deprecated)]
        let rc = libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            &mut info as *mut _ as *mut i32,
            &mut count,
        );
        assert_eq!(rc, 0, "current RSS required");
        info.resident_size
    }
}
fn event(name: &str, elapsed: u128, cpu_ns: u64, begin: Instant) {
    let current = rss();
    let hwm = peak();
    let total_cpu = cpu();
    println!("{{\"phase\":\"{name}\",\"elapsed_ns\":{elapsed},\"cpu_ns\":{cpu_ns},\"process_cpu_ns\":{total_cpu},\"since_start_ns\":{},\"current_rss_bytes\":{current},\"process_peak_rss_bytes\":{hwm}}}",begin.elapsed().as_nanos());
    std::io::stdout().flush().unwrap();
    // Sampling dwell is outside all measured operation durations.
    std::thread::sleep(std::time::Duration::from_millis(25));
}
fn timed<T>(name: &str, begin: Instant, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let c = cpu();
    let result = f();
    let elapsed = t.elapsed().as_nanos();
    let used = cpu() - c;
    event(name, elapsed, used, begin);
    result
}
fn tile(v: &[i32], start: usize) -> WorldTile {
    WorldTile {
        x: v[start],
        z: v[start + 1],
        level: v[start + 2],
    }
}
fn selectors(path: &str) -> Vec<[i32; 10]> {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() <= 65536);
    let text = std::str::from_utf8(&bytes).unwrap();
    let rows: Vec<[i32; 10]> = text
        .lines()
        .map(|line| {
            let v: Vec<i32> = line
                .split_whitespace()
                .map(|s| s.parse().expect("integer selector"))
                .collect();
            assert_eq!(v.len(), 10, "ten selector fields required");
            assert!([0, 1, 3, 4]
                .iter()
                .all(|&i| (-16384..=32767).contains(&v[i])));
            assert!([2, 5].iter().all(|&i| (0..=3).contains(&v[i])));
            assert!(
                (0..=15).contains(&v[6])
                    && (0..=6).contains(&v[7])
                    && (0..=4).contains(&v[8])
                    && (0..=1).contains(&v[9])
            );
            v.try_into().unwrap()
        })
        .collect();
    assert!((1..=256).contains(&rows.len()));
    rows
}
fn options(bits: i32) -> FindOptions {
    FindOptions {
        allow_teleports: bits & 1 != 0,
        allow_wilderness: bits & 2 != 0,
        allow_bank_fetch: bits & 4 != 0,
        essence: if bits & 8 != 0 {
            Some(crate::essence::essence_session_for_wizard(553).expect("frozen wizard"))
        } else {
            None
        },
    }
}
#[derive(Default)]
struct Aggregate {
    calls: u64,
    no_path: u64,
    routes: u64,
    empty: u64,
    legs: u64,
    tiles: u64,
    transports: u64,
    banks: u64,
    ticks: f64,
    checksum: u64,
}
impl Aggregate {
    fn consume(&mut self, r: &Route) {
        black_box(r);
        self.routes += 1;
        self.ticks += r.ticks;
        self.legs += r.legs.len() as u64;
        if r.legs.is_empty() {
            self.empty += 1;
        }
        self.checksum = self.checksum.wrapping_add(r.ticks.to_bits());
        for leg in &r.legs {
            match leg {
                Leg::Walk { tiles } => {
                    self.tiles += tiles.len() as u64;
                    for t in tiles {
                        self.checksum = self.checksum.wrapping_add(
                            (black_box(t.x) as u64).wrapping_mul(31) ^ t.z as u64 ^ t.level as u64,
                        );
                    }
                }
                Leg::Transport { edge } => {
                    self.transports += 1;
                    self.checksum = self.checksum.wrapping_add(black_box(edge.loc_id) as u64);
                }
            }
        }
    }
    fn result(&mut self, r: Result<Route, RouteError>) {
        self.calls += 1;
        match r {
            Ok(r) => self.consume(&r),
            Err(_) => self.no_path += 1,
        }
    }
    fn outcome(&mut self, r: RouteOutcome) {
        self.calls += 1;
        match r {
            RouteOutcome::NoPath => self.no_path += 1,
            RouteOutcome::Routed(r) => self.consume(&r),
            RouteOutcome::BankSession { pending, route } => {
                self.banks += 1;
                black_box(&pending.steps);
                self.checksum = self.checksum.wrapping_add(pending.steps.len() as u64);
                self.consume(&pending.final_route);
                self.consume(&route);
            }
        }
    }
    fn json(&self) -> String {
        format!("{{\"calls\":{},\"no_path\":{},\"routes\":{},\"empty\":{},\"legs\":{},\"tiles\":{},\"transports\":{},\"banks\":{},\"ticks\":{},\"checksum\":{}}}",self.calls,self.no_path,self.routes,self.empty,self.legs,self.tiles,self.transports,self.banks,self.ticks,self.checksum)
    }
}
fn lookup(world: &NavWorld, repeats: usize) -> u64 {
    let (o, w, h, n) = crate::collision::stage_layout::geometry(&world.collision);
    let mut total = 0u64;
    // Fixed coprime stride visits every cell for generated 64x64x4 maps;
    // real lookup batch remains explicitly bounded to 65536 indices per pass.
    for pass in 0..repeats {
        for k in 0..n.min(65536) {
            let i = (k.wrapping_mul(8191) + pass) % n;
            let t = WorldTile {
                x: o.x + (i % w) as i32,
                z: o.z + ((i % (w * h)) / w) as i32,
                level: (i / (w * h)) as i32,
            };
            total = total
                .wrapping_add(black_box(world.collision.walkable_word(t.x, t.z, t.level)) as u64);
            total = total.wrapping_add(black_box(world.collision.standable(t)) as u64);
            total = total.wrapping_add(black_box(world.collision.walkable(t)) as u64);
        }
    }
    black_box(total)
}
fn self_test() {
    for preset in 0..=6 {
        let s = facts(preset);
        match preset {
            4 => {
                assert_eq!(s.inv.len(), 3);
                assert_eq!(s.inv.get(&563), Some(&50));
                assert_eq!(s.inv.get(&556), Some(&150));
                assert_eq!(s.inv.get(&554), Some(&50));
                assert_eq!(s.stats.len(), 1);
                assert_eq!(s.stats.get(&6), Some(&25));
            }
            5 => {
                assert_eq!(s.inv.len(), 1);
                assert_eq!(s.inv.get(&1712), Some(&1));
                assert!(s.stats.is_empty());
            }
            6 => {
                assert_eq!(s.inv.len(), 1);
                assert_eq!(s.inv.get(&995), Some(&5000));
                assert!(s.stats.is_empty());
            }
            _ => {
                assert_eq!(s.inv.len(), if preset & 2 != 0 { 1 } else { 0 });
                assert_eq!(s.worn.len(), if preset & 2 != 0 { 1 } else { 0 });
                assert_eq!(s.stats.len(), if preset & 1 != 0 { 2 } else { 0 });
            }
        }
        if preset >= 4 {
            assert!(s.worn.is_empty() && s.quests.is_empty() && s.varps.is_empty());
        }
    }
    let t = WorldTile {
        x: 100,
        z: 200,
        level: 0,
    };
    let mut a = Aggregate::default();
    a.consume(&Route {
        legs: vec![],
        dest: t,
        ticks: 0.0,
    });
    assert_eq!(a.empty, 1);
    a.consume(&Route {
        legs: vec![Leg::Walk { tiles: vec![t, t] }],
        dest: t,
        ticks: 0.5,
    });
    assert_eq!(a.tiles, 2);
    assert_eq!(a.ticks, 0.5);
    assert_ne!(a.checksum, 0);
    assert!(options(8).essence.is_some());
    let prior = counts();
    let allocated = black_box(vec![7u8; 4096]);
    black_box(&allocated);
    let during = counts();
    drop(allocated);
    if cfg!(feature = "stage-counting") {
        assert!(during[0] > prior[0] && during[2] >= prior[2] + 4096);
    } else {
        assert_eq!(during, prior);
    }
    // A released anonymous mapping demonstrates current resident semantics
    // independently of allocator retention. Peak must not decrease on unmap.
    let resident_before = rss();
    let peak_before = peak();
    let (resident_touched, resident_after, peak_after) = unsafe {
        let n = 32 * 1024 * 1024;
        let p = libc::mmap(
            std::ptr::null_mut(),
            n,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_PRIVATE | libc::MAP_ANON,
            -1,
            0,
        );
        assert_ne!(p, libc::MAP_FAILED);
        for i in (0..n).step_by(4096) {
            (p as *mut u8).add(i).write_volatile(1);
        }
        let touched = rss();
        assert_eq!(libc::munmap(p, n), 0);
        (touched, rss(), peak())
    };
    assert!(resident_touched > resident_before + 16 * 1024 * 1024);
    assert!(resident_after + 16 * 1024 * 1024 < resident_touched);
    assert!(peak_after >= peak_before);
    println!("{{\"self_test\":true,\"resident_before\":{resident_before},\"resident_touched\":{resident_touched},\"resident_after\":{resident_after},\"peak_before\":{peak_before},\"peak_after\":{peak_after}}}");
}
fn call_route(
    aggregate: &mut Aggregate,
    world: &NavWorld,
    v: &[i32; 10],
    request: &ScriptRouteRequest,
    lane: usize,
) {
    let model = if v[9] == 0 {
        CostModel::running()
    } else {
        CostModel {
            run_per_step: 1.0,
            walk_per_step: 1.0,
        }
    };
    match lane {
        0 => aggregate.result(router::find_with_model(
            &world.collision,
            &world.graph,
            request.from,
            request.to,
            model,
        )),
        1 => aggregate.result(router::find_allow_teleports_with_model(
            &world.collision,
            &world.graph,
            request.from,
            request.to,
            model,
            request.state.as_ref().unwrap(),
        )),
        _ => aggregate.outcome(request.calculate()),
    }
}
pub fn run() {
    let begin = Instant::now();
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 2 && args[1] == "--self-test" {
        self_test();
        return;
    }
    assert_eq!(args.len(), 4);
    assert!(matches!(args[3].as_str(), "generated" | "released"));
    let startup_peak = peak();
    event("startup", 0, 0, begin);
    let rows = selectors(&args[2]);
    let input = timed("retained_input", begin, || {
        std::fs::read(&args[1]).expect("admitted pack")
    });
    assert!(input.len() <= 128 * 1024 * 1024);
    let input_len = input.len();
    let world = timed("decoded_converted_retained_input", begin, || {
        let (c, g, b) = crate::pack::decode(&input).expect("original decoder rejected input");
        Arc::new(NavWorld::from_parts(c, g, b))
    });
    // Borrow ends after decode; no encode output exists in this process.
    timed("dropped_input_world_live", begin, || drop(input));
    let layout = crate::collision::stage_layout::layout(&world.collision);
    let (_, _, _, cells) = crate::collision::stage_layout::geometry(&world.collision);
    let requests: Vec<_> = rows
        .iter()
        .map(|v| ScriptRouteRequest {
            generation: 0,
            world: world.clone(),
            from: tile(v, 0),
            to: tile(v, 3),
            radius: v[8],
            opts: options(v[6]),
            state: Some(facts(v[7] as u8)),
            bank: vec![(995, 10), (1712, 1)],
        })
        .collect();
    let mut durations = Vec::with_capacity(rows.len() * 8 * 3);
    let mut aggregate = Aggregate::default();
    lookup(&world, 1); // Warm narrow reads before diagnostic counters and clean hot batch.
    let before = counts();
    let lookup_sum = timed("hot_lookup", begin, || lookup(&world, 8));
    let after = counts();
    // Counter window excludes event formatting/RSS instrumentation: repeat narrow scope independently.
    let narrow_before = counts();
    let narrow_sum = lookup(&world, 8);
    let narrow_after = counts();
    let mut lane_cpu = [0u64; 3];
    let mut warmed = Aggregate::default();
    for (v, request) in rows.iter().zip(&requests) {
        for lane in 0..3 {
            call_route(&mut warmed, &world, v, request, lane);
        }
    }
    black_box(&warmed);
    drop(warmed);
    timed("hot_routes", begin, || {
        // Exact ccff4bb fixed-selector semantics: three DISTINCT lanes. Model
        // lane ignores options/state by API definition; tele-model uses state;
        // host lane uses radius/options/state/bank and its original running model.
        for _ in 0..8 {
            for (v, request) in rows.iter().zip(&requests) {
                for lane in 0..3 {
                    let t = Instant::now();
                    let c = cpu();
                    call_route(&mut aggregate, &world, v, request, lane);
                    lane_cpu[lane] += cpu() - c;
                    durations.push(t.elapsed().as_nanos());
                }
            }
        }
    });
    drop(requests);
    // Preserve execution order outside all timing. Formatting and the copy are
    // post-route observer work; the original duration endpoint is unchanged.
    let raw_durations = format!("{:?}", durations);
    assert!(raw_durations.len() + 16384 <= 256 * 1024, "raw output bound");
    durations.sort_unstable();
    let p99 = durations[(durations.len() * 99).div_ceil(100) - 1];
    drop(durations);
    assert_eq!(
        Arc::strong_count(&world),
        1,
        "request world owners must be gone"
    );
    let weak = Arc::downgrade(&world);
    timed("world_drop", begin, || drop(world));
    assert!(weak.upgrade().is_none());
    drop(weak);
    event("post_drop", 0, 0, begin);
    let diagnostic = cfg!(feature = "stage-counting");
    println!("{{\"summary\":true,\"raw_schema\":\"stage-a-raw-v1\",\"raw_order\":\"sweep-row-lane/8/3\",\"raw_elapsed_ns\":{raw_durations},\"diagnostic\":{diagnostic},\"input_bytes\":{input_len},\"startup_peak_rss_bytes\":{startup_peak},\"logical_cells\":{cells},\"layout\":{:?},\"lookup_count\":{},\"lookup_checksum\":{lookup_sum},\"narrow_checksum\":{narrow_sum},\"narrow_allocations\":{},\"narrow_requested_bytes\":{},\"lookup_including_observer_allocations\":{},\"route_p99_ns\":{p99},\"lane_cpu_ns\":{:?},\"aggregate\":{},\"process_cpu_ns\":{},\"process_peak_rss_bytes\":{}}}",layout,cells.min(65536)*8*3,narrow_after[0]-narrow_before[0],narrow_after[2]-narrow_before[2],after[0]-before[0],lane_cpu,aggregate.json(),cpu(),peak());
}
