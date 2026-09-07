//! Standalone diagnostic for std::thread::sleep excess; no game or host code.
//! Collect all samples in memory, print after the timed phase. Not a performance
//! baseline, scheduler policy experiment, or accepted campaign result.
use std::{time::{Duration, Instant}, thread};
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 3 { eprintln!("usage: sleep-probe REQUEST_MS SAMPLES"); std::process::exit(2); }
    let request: u64 = args[1].parse().expect("integer request milliseconds");
    let count: usize = args[2].parse().expect("integer sample count");
    assert!((1..=1000).contains(&request) && (1..=10000).contains(&count));
    let sleep = Duration::from_millis(request);
    for _ in 0..20 { thread::sleep(sleep); }
    let mut samples = Vec::with_capacity(count);
    let phase = Instant::now();
    for _ in 0..count {
        let start = Instant::now();
        thread::sleep(sleep);
        samples.push(start.elapsed().as_nanos());
    }
    let wall = phase.elapsed().as_nanos();
    println!("{{\"type\":\"meta\",\"request_ms\":{},\"samples\":{},\"phase_wall_ns\":{},\"warmup_calls\":20,\"output_after_phase\":true}}", request, count, wall);
    for (i, ns) in samples.iter().enumerate() {
        println!("{{\"type\":\"sample\",\"index\":{},\"actual_ns\":{},\"requested_ns\":{}}}", i, ns, sleep.as_nanos());
    }
}
