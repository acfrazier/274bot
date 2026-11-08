//! TASK_VM_INFO/phys_footprint at a nominal 1 ms interval, with constant storage.
//! The long canonical corpus must not stop sampling at a retained-vector limit.
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde::Serialize;

const TASK_VM_INFO: u32 = 22;
const REV1_COUNT: u32 = 38;
const INTERVAL: Duration = Duration::from_millis(1);

// Darwin task_vm_info revision 1 has 36 integer_t words before phys_footprint.
// Mach's 4-byte ABI packing applies even though the last field is a u64.
#[repr(C, packed(4))]
struct TaskVmInfoRev1 {
    _prefix: [u32; 36],
    phys_footprint: u64,
}
const _: [(); 152] = [(); size_of::<TaskVmInfoRev1>()];

#[link(name = "System")]
unsafe extern "C" {
    static mach_task_self_: u32;
    fn task_info(task: u32, flavor: u32, info: *mut i32, count: *mut u32) -> i32;
}

pub fn bytes() -> Result<u64, String> {
    let mut info = TaskVmInfoRev1 {
        _prefix: [0; 36],
        phys_footprint: 0,
    };
    let mut count = REV1_COUNT;
    // SAFETY: The fixed, 4-byte-aligned REV1 buffer has exactly count i32 words.
    // The kernel writes only that buffer; mach_task_self_ is our own task port.
    let result = unsafe {
        task_info(
            mach_task_self_,
            TASK_VM_INFO,
            (&mut info as *mut TaskVmInfoRev1).cast(),
            &mut count,
        )
    };
    if result != 0 || count < REV1_COUNT {
        return Err(format!(
            "TASK_VM_INFO failed: result={result}, count={count}"
        ));
    }
    Ok(info.phys_footprint)
}

#[derive(Serialize)]
pub struct Measurement {
    api: &'static str,
    flavor: u32,
    metric: &'static str,
    retention: &'static str,
    nominal_interval_ns: u128,
    window_ns: u128,
    mean_sample_interval_ns: Option<u128>,
    sample_count: u64,
    peak_bytes: u64,
}

pub struct Sampler {
    start: Option<mpsc::SyncSender<()>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Result<Measurement, String>>>,
}

impl Sampler {
    pub fn start() -> Result<Self, String> {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let (start_tx, start_rx) = mpsc::sync_channel(0);
        let (ready_tx, ready_rx) = mpsc::sync_channel(0);
        let worker = thread::Builder::new()
            .name("npr-footprint".into())
            .spawn(move || {
                // Fault the native sampling path and worker stack before baseline.
                let initial = bytes();
                let supported = initial.is_ok();
                ready_tx.send(initial).map_err(|error| error.to_string())?;
                if !supported || start_rx.recv().is_err() {
                    return Err("footprint sampler did not start".into());
                }
                let started = Instant::now();
                let mut previous = None;
                let mut interval_total = Duration::ZERO;
                let mut count = 0_u64;
                let mut peak = 0;
                loop {
                    let at = Instant::now();
                    peak = peak.max(bytes()?);
                    count += 1;
                    if let Some(previous) = previous {
                        interval_total += at.saturating_duration_since(previous);
                    }
                    previous = Some(at);
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let wait = (at + INTERVAL).saturating_duration_since(Instant::now());
                    if !wait.is_zero() {
                        thread::park_timeout(wait);
                    }
                }
                Ok(Measurement {
                    api: "TASK_VM_INFO",
                    flavor: TASK_VM_INFO,
                    metric: "phys_footprint",
                    retention: "constant-memory peak and count; no retained per-sample vector",
                    nominal_interval_ns: INTERVAL.as_nanos(),
                    window_ns: started.elapsed().as_nanos(),
                    mean_sample_interval_ns: (count > 1)
                        .then(|| interval_total.as_nanos() / u128::from(count - 1)),
                    sample_count: count,
                    peak_bytes: peak,
                })
            })
            .map_err(|error| error.to_string())?;
        match ready_rx.recv() {
            Ok(Ok(_)) => Ok(Self {
                start: Some(start_tx),
                stop,
                worker: Some(worker),
            }),
            result => {
                drop(start_tx);
                let _ = worker.join();
                Err(format!("initialize footprint sampler: {result:?}"))
            }
        }
    }

    pub fn begin(&mut self) -> Result<(), String> {
        self.start
            .take()
            .ok_or("footprint sampler already started")?
            .send(())
            .map_err(|error| error.to_string())
    }

    pub fn finish(mut self) -> Result<Measurement, String> {
        if self.start.is_some() {
            return Err("footprint sampler never started".into());
        }
        self.stop.store(true, Ordering::Release);
        let worker = self.worker.take().ok_or("footprint worker is absent")?;
        worker.thread().unpark();
        worker.join().map_err(|_| "footprint worker panicked")?
    }
}

impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Dropping a not-yet-used sender also releases the pre-baseline receiver.
        self.start.take();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_one_abi_and_native_measurement_are_supported() {
        assert_eq!(size_of::<TaskVmInfoRev1>(), 152);
        assert!(bytes().expect("native TASK_VM_INFO") > 0);
    }

    #[test]
    fn sampler_joins_with_a_real_peak_and_no_sample_vector() {
        let mut sampler = Sampler::start().expect("native sampler");
        sampler.begin().expect("begin");
        let result = sampler.finish().expect("joined sampler");
        assert!(result.sample_count >= 1);
        assert!(result.peak_bytes > 0);
        assert_eq!(result.nominal_interval_ns, 1_000_000);
    }

    #[test]
    fn dropping_before_begin_releases_the_waiting_thread() {
        drop(Sampler::start().expect("native sampler"));
    }
}
