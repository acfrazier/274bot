use super::*;
use std::cell::Cell;
use std::rc::Rc;

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A service at `at` with no time in the loop's own waits.
fn svc(at: Instant, overshoot: Duration, ticks: u64) -> Service {
    Service {
        at,
        end: at,
        slept: Duration::ZERO,
        overshoot,
        ticks,
    }
}

/// A monitor and a test origin 20 s after its epoch (room for older samples).
fn monitor() -> (Arc<CapacityMonitor>, Instant) {
    let mon = Arc::new(CapacityMonitor::new());
    let t0 = mon.epoch + Duration::from_secs(20);
    (mon, t0)
}

/// One slot thread's recorder with an injected run-queue clock.
struct Bot {
    rec: SlotRecorder,
    waited: Rc<Cell<Duration>>,
    ticks: u64,
}

impl Bot {
    /// Logged in and observed Active once (calm), so later services count.
    fn active(mon: &Arc<CapacityMonitor>, at: Instant) -> Self {
        let waited = Rc::new(Cell::new(Duration::ZERO));
        let mut rec = SlotRecorder::new(mon, Clock::InjectedWait(Rc::clone(&waited)));
        rec.begin(at, 0);
        rec.set_class(ServiceClass::Active);
        rec.record(svc(at, Duration::ZERO, 0));
        Self {
            rec,
            waited,
            ticks: 0,
        }
    }

    /// One service at `at` after `wait_ms` of run-queue wait.
    fn service(&mut self, at: Instant, wait_ms: u64) {
        self.waited.set(self.waited.get() + ms(wait_ms));
        self.rec.record(svc(at, Duration::ZERO, self.ticks));
    }

    /// `n` services 200 ms apart from `from`, each kept 700 ms off the CPU.
    fn late(&mut self, from: Instant, n: u32) {
        for i in 0..n {
            self.service(from + ms(u64::from(i) * 200), 700);
        }
    }

    fn class(&mut self, class: ServiceClass) {
        self.rec.set_class(class);
    }
}

fn late_bots(mon: &Arc<CapacityMonitor>, at: Instant, n: usize) -> Vec<Bot> {
    (0..n)
        .map(|_| {
            let mut bot = Bot::active(mon, at);
            bot.late(at + ms(10), 3);
            bot
        })
        .collect()
}

fn eval(mon: &CapacityMonitor, at: Instant) -> CapacityEval {
    mon.evaluate(at, RenderPolicyLabel::OnlySelected)
}

fn warning(eval: CapacityEval) -> CapacityWarning {
    match eval {
        CapacityEval::Warn(warning) => warning,
        other => panic!("expected a warning, got {other:?}"),
    }
}

fn is_warn(eval: CapacityEval) -> bool {
    matches!(eval, CapacityEval::Warn(_))
}

#[test]
fn a_bots_own_slow_work_on_an_idle_machine_is_not_late() {
    let (mon, t0) = monitor();
    // Three bots whose every service takes 2 s of their own work: the
    // run-queue clock does not move, so nothing is late.
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    for step in 1..=5u64 {
        for bot in &mut bots {
            bot.service(t0 + Duration::from_secs(2 * step), 0);
        }
    }
    assert_eq!(
        eval(&mon, t0 + Duration::from_secs(10)),
        CapacityEval::Clear
    );
    // Without a run-queue clock, a frame that overran its 20 ms has no
    // timed wake to miss: the loop passes zero overshoot.
    let mut rec = SlotRecorder::new(&mon, Clock::Overshoot);
    rec.begin(t0, 0);
    rec.set_class(ServiceClass::Active);
    for step in 0..5u64 {
        rec.record(svc(t0 + Duration::from_secs(2 * step), Duration::ZERO, 0));
    }
    assert!(rec
        .slot
        .samples
        .iter()
        .all(|cell| cell.load(Ordering::Relaxed) == 0));
}

#[test]
fn idle_bots_kept_off_a_busy_cpu_are_late() {
    let (mon, t0) = monitor();
    // Idle in-game bots wake once per park; each wake waits 700 ms for
    // a CPU.
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    for step in 1..=3u64 {
        for bot in &mut bots {
            bot.service(t0 + ms(600 * step), 700);
        }
    }
    let w = warning(eval(&mon, t0 + ms(2000)));
    assert_eq!((w.late_bots(), w.active_bots(), w.threshold()), (3, 3, 3));
}

#[test]
fn without_a_run_queue_clock_a_late_timed_wake_counts() {
    let (mon, t0) = monitor();
    let mut recs: Vec<SlotRecorder> = (0..3)
        .map(|_| {
            let mut rec = SlotRecorder::new(&mon, Clock::Overshoot);
            rec.begin(t0, 0);
            rec.set_class(ServiceClass::Active);
            rec.record(svc(t0, Duration::ZERO, 0));
            rec
        })
        .collect();
    for step in 1..=3u64 {
        for rec in &mut recs {
            rec.record(svc(t0 + ms(700 * step), ms(650), 0));
        }
    }
    assert_eq!(warning(eval(&mon, t0 + ms(2200))).late_bots(), 3);
}

/// A slot measuring time off the CPU through an injected thread CPU clock,
/// as on macOS and Windows.
struct CpuBot {
    rec: SlotRecorder,
    cpu: Rc<Cell<Duration>>,
    end: Instant,
}

impl CpuBot {
    /// Logged in and observed Active once, its previous service ending at
    /// `at`.
    fn active(mon: &Arc<CapacityMonitor>, at: Instant) -> Self {
        let cpu = Rc::new(Cell::new(Duration::ZERO));
        let mut rec = SlotRecorder::new(mon, Clock::InjectedCpu(Rc::clone(&cpu)));
        rec.begin(at, 0);
        rec.set_class(ServiceClass::Active);
        rec.record(svc(at, Duration::ZERO, 0));
        Self { rec, cpu, end: at }
    }

    /// The loop sleeps `slept` up to its deadline, resumes `late` after
    /// it, then runs `own` of its own work and is kept `preempted` off the
    /// CPU inside the tick. Returns the delay it measured.
    fn frame(&mut self, slept: u64, late: u64, own: u64, preempted: u64) -> Duration {
        let at = self.end + ms(slept + late);
        self.end = at + ms(own + preempted);
        // The sleep and the wake cost a little CPU of their own.
        self.cpu
            .set(self.cpu.get() + ms(own) + Duration::from_micros(30));
        self.rec.record(Service {
            at,
            end: self.end,
            slept: ms(slept),
            overshoot: ms(late),
            ticks: 0,
        })
    }
}

#[test]
fn time_off_the_cpu_is_wall_minus_cpu_minus_own_waits() {
    let (mon, t0) = monitor();
    let mut bot = CpuBot::active(&mon, t0);
    let close = |delay: Duration, want: u64| {
        assert!(
            delay.abs_diff(ms(want)) <= ms(1),
            "{delay:?} is not about {want} ms"
        );
    };
    // A 20 ms frame on an idle machine: 5 ms work, 15 ms sleep.
    close(bot.frame(15, 0, 5, 0), 0);
    // Its own 2 s of work (a slow compiled tick): not off the CPU.
    close(bot.frame(0, 0, 2000, 0), 0);
    // A 2 s idle park that ran to its bound: the bot's own wait.
    close(bot.frame(2000, 0, 1, 0), 0);
    // Preempted 700 ms in the middle of a busy frame.
    close(bot.frame(15, 0, 5, 700), 700);
    // The timer fired on time, the CPU came 650 ms later.
    close(bot.frame(15, 650, 5, 0), 650);
    // Both, on one service.
    close(bot.frame(15, 300, 5, 400), 700);
    // A CPU clock ahead of the wall clock never underflows.
    assert_eq!(off_cpu(ms(10), ms(20), ms(5)), Duration::ZERO);
}

#[test]
fn busy_bots_preempted_mid_frame_are_late_their_own_work_is_not() {
    let (mon, t0) = monitor();
    let mut starved: Vec<CpuBot> = (0..3).map(|_| CpuBot::active(&mon, t0)).collect();
    let mut working: Vec<CpuBot> = (0..3).map(|_| CpuBot::active(&mon, t0)).collect();
    for _ in 0..3 {
        for bot in &mut starved {
            // Frames that overran their 20 ms: no sleep, so no timed wake
            // to miss. Only time off the CPU sees the 650 ms preemption.
            assert!(bot.frame(0, 0, 25, 650) >= LATE);
        }
        for bot in &mut working {
            bot.frame(0, 0, 675, 0);
        }
    }
    let w = warning(eval(&mon, t0 + ms(3000)));
    assert_eq!((w.late_bots(), w.active_bots()), (3, 6));

    // The same overran frames measured by timed-wake overshoot alone
    // (R2's macOS/Windows measure) show nothing.
    let (mon, t0) = monitor();
    let mut recs: Vec<SlotRecorder> = (0..3)
        .map(|_| {
            let mut rec = SlotRecorder::new(&mon, Clock::Overshoot);
            rec.begin(t0, 0);
            rec.set_class(ServiceClass::Active);
            rec.record(svc(t0, Duration::ZERO, 0));
            rec
        })
        .collect();
    for step in 1..=3u64 {
        for rec in &mut recs {
            rec.record(svc(t0 + ms(675 * step), Duration::ZERO, 0));
        }
    }
    assert_eq!(eval(&mon, t0 + ms(3000)), CapacityEval::Clear);
}

#[test]
fn three_bots_trip_the_floor_two_do_not() {
    let (mon, t0) = monitor();
    let _two = late_bots(&mon, t0, 2);
    let at = t0 + ms(600);
    assert!(!is_warn(eval(&mon, at)), "2 late bots are below the floor");
    let _third = late_bots(&mon, t0, 1);
    assert!(is_warn(eval(&mon, at)), "3 late bots meet the floor");
}

#[test]
fn ten_percent_beats_the_floor_on_a_large_fleet() {
    let (mon, t0) = monitor();
    let _calm: Vec<Bot> = (0..40).map(|_| Bot::active(&mon, t0)).collect();
    let _late = late_bots(&mon, t0, 3);
    let at = t0 + ms(600);
    assert!(!is_warn(eval(&mon, at)), "3 of 43 is below 10% (5)");
    let _more = late_bots(&mon, t0, 2);
    let w = warning(eval(&mon, at));
    assert_eq!((w.late_bots(), w.active_bots(), w.threshold()), (5, 45, 5));
}

#[test]
fn two_late_services_do_not_count_a_bot_three_do() {
    let (mon, t0) = monitor();
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    for bot in &mut bots {
        bot.late(t0 + ms(10), 2);
        // A 500 ms wait keeps the bot unsettled but is not late.
        bot.service(t0 + ms(400), 500);
    }
    assert!(!is_warn(eval(&mon, t0 + ms(500))));
    for bot in &mut bots {
        bot.service(t0 + ms(450), 700);
    }
    assert!(is_warn(eval(&mon, t0 + ms(500))));
}

/// R2 review counterexample (M1): delays of 300–599 ms must not push
/// qualifying lates still inside the window out of the count. Each bot
/// waits 600 ms, ten times 300 ms, 600 ms, ten times 300 ms, 600 ms (7.8 s
/// of waiting, 20 ms of its own work per service), all inside one window.
#[test]
fn delays_under_late_do_not_push_out_late_ones_in_the_window() {
    let (mon, t0) = monitor();
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    let mut waits = vec![600u64];
    for _ in 0..2 {
        waits.extend([300; 10]);
        waits.push(600);
    }
    let (mut at, last) = (t0, waits.len() - 1);
    let mut next_eval = t0 + Duration::from_secs(1);
    for (i, wait) in waits.into_iter().enumerate() {
        at += ms(wait + 20);
        for bot in &mut bots {
            bot.service(at, wait);
        }
        // The evaluator runs once a second, never at the third late.
        if i < last && at >= next_eval {
            assert!(!is_warn(eval(&mon, at)), "fewer than 3 lates so far");
            next_eval += Duration::from_secs(1);
        }
    }
    assert!(at - t0 < WINDOW, "every service is inside one window");
    let w = warning(eval(&mon, at));
    assert_eq!((w.late_bots(), w.active_bots()), (3, 3));
}

/// Every delay of [`CALM`] or more inside the window feeds the p95. Back
/// to back, at most `WINDOW / CALM + 1` (34) of them fit in a window.
#[test]
fn p95_keeps_every_unsettled_delay_in_a_full_window() {
    let (mon, t0) = monitor();
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    // 3 × 900 ms then 25 × 300 ms, back to back: 28 per bot, the last 27
    // waited 9.3 s together.
    let waits: Vec<u64> = [900; 3].into_iter().chain([300; 25]).collect();
    let mut at = t0;
    for wait in waits {
        at += ms(wait);
        for bot in &mut bots {
            bot.service(at, wait);
        }
    }
    assert_eq!(mon.recorded_delays().len(), 3 * 28);
    // 84 delays: 75 of 300 ms, 9 of 900 ms; nearest rank 80 is 900 ms.
    assert_eq!(warning(eval(&mon, at)).p95_delay(), Some(ms(900)));
    // A window's worth of 300 ms waits back to back: all 34 are kept.
    let (mon, t0) = monitor();
    let mut bot = Bot::active(&mon, t0);
    for i in 1..=34u64 {
        bot.service(t0 + ms(300 * i), 300);
    }
    assert_eq!(mon.recorded_delays(), vec![300; 34]);
}

#[test]
fn waits_while_parked_paused_or_logging_in_do_not_count_once_active() {
    let (mon, t0) = monitor();
    let mut bots: Vec<Bot> = (0..3).map(|_| Bot::active(&mon, t0)).collect();
    for (bot, class) in bots.iter_mut().zip([
        ServiceClass::Parked,
        ServiceClass::Paused,
        ServiceClass::Suspended,
    ]) {
        bot.class(class);
        bot.late(t0 + ms(10), 3);
        bot.class(ServiceClass::Active);
        // The first Active service still carries a wait from before.
        bot.service(t0 + ms(800), 900);
    }
    assert_eq!(eval(&mon, t0 + ms(900)), CapacityEval::Clear);
    // A fresh slot loop (after login) re-baselines and skips its first
    // service too.
    let mut fresh = Bot::active(&mon, t0);
    fresh.rec.begin(t0 + ms(840), 0);
    fresh.waited.set(fresh.waited.get() + ms(5000));
    fresh.class(ServiceClass::Active);
    fresh.service(t0 + ms(850), 900);
    assert_eq!(eval(&mon, t0 + ms(900)), CapacityEval::Clear);
}

#[test]
fn leaving_active_drops_earlier_late_services() {
    let (mon, t0) = monitor();
    let mut bots = late_bots(&mon, t0, 3);
    for bot in &mut bots {
        bot.class(ServiceClass::Paused);
        bot.class(ServiceClass::Active);
        bot.service(t0 + ms(700), 0);
    }
    assert_eq!(eval(&mon, t0 + ms(800)), CapacityEval::Clear);
}

#[test]
fn late_services_leave_the_window_after_ten_seconds() {
    let (mon, t0) = monitor();
    let _old = late_bots(&mon, t0, 2);
    let later = t0 + Duration::from_secs(11);
    let _new = late_bots(&mon, later, 1);
    assert_eq!(
        eval(&mon, later + ms(700)),
        CapacityEval::Clear,
        "two of the three bots' lates are older than the window"
    );
    let (mon, t0) = monitor();
    let _all = late_bots(&mon, t0, 3);
    assert!(is_warn(eval(&mon, t0 + Duration::from_secs(10))));
}

#[test]
fn bots_joining_and_leaving_change_the_counts() {
    let (mon, t0) = monitor();
    let mut late = late_bots(&mon, t0, 3);
    assert_eq!(warning(eval(&mon, t0 + ms(700))).active_bots(), 3);
    // A calm bot joins mid-window: counted as active, not late.
    let mut joined = Bot::active(&mon, t0 + ms(800));
    let w = warning(eval(&mon, t0 + ms(900)));
    assert_eq!((w.late_bots(), w.active_bots()), (3, 4));
    // A late bot leaves: its slot is gone; the warning holds until calm.
    drop(late.pop());
    assert_eq!(mon.registered(), 3);
    let w = warning(eval(&mon, t0 + ms(1000)));
    assert_eq!((w.late_bots(), w.active_bots()), (2, 3));
    // The joined bot becomes late on its own services.
    joined.late(t0 + ms(1100), 3);
    let w = warning(eval(&mon, t0 + ms(1600)));
    assert_eq!((w.late_bots(), w.active_bots()), (3, 3));
    drop(late);
    drop(joined);
    assert_eq!(mon.registered(), 0);
    assert_eq!(eval(&mon, t0 + ms(1700)), CapacityEval::Clear);
}

#[test]
fn warning_clears_after_thirty_seconds_under_300ms() {
    let (mon, t0) = monitor();
    let mut bots = late_bots(&mon, t0, 3);
    let last_late = t0 + ms(410);
    assert!(is_warn(eval(&mon, t0 + ms(600))));
    for step in 1..=29u64 {
        for bot in &mut bots {
            bot.service(t0 + Duration::from_secs(step), 200);
        }
    }
    assert!(
        is_warn(eval(&mon, last_late + ms(29_900))),
        "under 30 s of calm"
    );
    assert_eq!(eval(&mon, last_late + ms(30_000)), CapacityEval::Clear);
}

#[test]
fn a_delay_of_400ms_extends_the_calm_timer() {
    let (mon, t0) = monitor();
    let mut bots = late_bots(&mon, t0, 3);
    assert!(is_warn(eval(&mon, t0 + ms(600))));
    let unrest = t0 + Duration::from_secs(5);
    bots[0].service(unrest, 400);
    assert!(
        is_warn(eval(&mon, unrest + ms(29_900))),
        "400 ms is not calm"
    );
    assert_eq!(eval(&mon, unrest + ms(30_000)), CapacityEval::Clear);
}

#[test]
fn quiet_bots_are_excluded_and_an_all_quiet_fleet_is_silence() {
    let (mon, t0) = monitor();
    let mut bots = late_bots(&mon, t0, 3);
    let calm: Vec<Bot> = (0..2).map(|_| Bot::active(&mon, t0)).collect();
    assert!(is_warn(eval(&mon, t0 + ms(600))));
    bots[0].class(ServiceClass::Quiet);
    let w = warning(eval(&mon, t0 + ms(700)));
    assert_eq!((w.late_bots(), w.active_bots()), (2, 4));
    for bot in &mut bots {
        bot.class(ServiceClass::Quiet);
    }
    drop(calm);
    assert_eq!(eval(&mon, t0 + ms(800)), CapacityEval::Silence);
    // Back from silence, the warning starts over.
    for bot in &mut bots {
        bot.class(ServiceClass::Active);
        bot.service(t0 + ms(900), 0);
    }
    assert_eq!(eval(&mon, t0 + ms(1000)), CapacityEval::Clear);
}

#[test]
fn p95_uses_every_delay_in_the_window() {
    let (mon, t0) = monitor();
    let mut bots: Vec<Bot> = (0..3)
        .map(|_| Bot::active(&mon, t0 - Duration::from_secs(12)))
        .collect();
    // An old 5 s wait outside the window must not count.
    bots[0].service(t0 - Duration::from_secs(11), 5000);
    for bot in &mut bots {
        bot.late(t0, 3);
    }
    bots[1].service(t0 + ms(700), 900);
    // The bot's latest service was on time; its delays still count.
    bots[1].service(t0 + ms(800), 0);
    let w = warning(eval(&mon, t0 + ms(900)));
    assert_eq!(w.p95_delay(), Some(ms(900)));
}

#[test]
fn tick_rate_counts_real_ticks_between_tick_instants() {
    let (mon, t0) = monitor();
    let mut bots = late_bots(&mon, t0, 3);
    let w = warning(eval(&mon, t0 + ms(500)));
    assert_eq!(w.tick_period(), None, "no ticks yet: no made-up rate");
    // 600 ms ticks for 20 s, then a 3 s stall drained as a 5-tick burst
    // in one service.
    let mut at = t0 + ms(600);
    let mut next_eval = at;
    for _ in 0..33 {
        for bot in &mut bots {
            bot.ticks += 1;
            bot.service(at, 0);
        }
        if at >= next_eval {
            eval(&mon, at);
            next_eval = at + Duration::from_secs(1);
        }
        at += ms(600);
    }
    let burst = at + ms(2400);
    for bot in &mut bots {
        bot.ticks += 5;
        bot.service(burst, 0);
    }
    let w = warning(eval(&mon, burst + ms(10)));
    assert_eq!(w.tick_period(), Some(ms(600)), "{w:?}");
}

#[test]
fn warning_carries_counts_and_render_policy() {
    let (mon, t0) = monitor();
    let _bots = late_bots(&mon, t0, 3);
    let w = warning(mon.evaluate(t0 + ms(700), RenderPolicyLabel::RenderAll));
    assert_eq!(w.render(), RenderPolicyLabel::RenderAll);
    assert_eq!(w.p95_delay(), Some(ms(700)));
    assert_eq!(
        w,
        CapacityWarning::new(3, 3, 3, Some(ms(704)), None, RenderPolicyLabel::RenderAll),
        "figures are rounded to 10 ms"
    );
}

#[test]
fn schedstat_line_parses_cpu_run_queue_wait_and_timeslices() {
    assert_eq!(
        parse_schedstat(b"123456 7890 42\n"),
        Some([123_456, 7_890, 42])
    );
    assert_eq!(parse_schedstat(b"12 x 3\n"), None);
    assert_eq!(parse_schedstat(b"12 3\n"), None);
    assert_eq!(parse_schedstat(b""), None);
}

/// Per-bot memory and the frame path's cost, for the report:
/// `cargo test -p host per_bot_memory -- --nocapture`.
#[test]
fn per_bot_memory_is_small() {
    let slot = std::mem::size_of::<SlotCapacity>();
    let entry = std::mem::size_of::<Entry>();
    let recorder = std::mem::size_of::<SlotRecorder>();
    // Arc header: strong + weak counts.
    let shared = slot + 2 * std::mem::size_of::<usize>();
    eprintln!(
        "SlotCapacity {slot} B (+Arc header = {shared} B), registry Entry {entry} B, \
         thread-local SlotRecorder {recorder} B, total {} B per bot",
        shared + entry + recorder
    );
    assert!(shared + entry + recorder <= 512);
}

/// Live receipts read every counted service's delay, short ones too.
#[cfg(feature = "test-support")]
#[test]
fn receipts_give_each_bots_max_and_p95_per_service() {
    let (mon, t0) = monitor();
    let mut bot = Bot::active(&mon, t0 - Duration::from_secs(12));
    bot.service(t0 - Duration::from_secs(11), 5000);
    for i in 1..=19u64 {
        bot.service(t0 + ms(20 * i), 10);
    }
    bot.service(t0 + ms(500), 700);
    let mut paused = Bot::active(&mon, t0);
    paused.service(t0 + ms(100), 700);
    paused.class(ServiceClass::Paused);
    assert_eq!(
        mon.service_delays(t0 + ms(600)),
        [ServiceDelays {
            services: 20,
            late: 1,
            p95: ms(10),
            max: ms(700),
        }],
        "the 5 s wait is older than the window; a paused bot is left out"
    );
}

/// Spin on this thread for `wall` behind four busy threads pinned to its
/// CPU, at the lowest priority, so it gets the CPU only now and then.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn spin_behind_hogs(wall: Duration) {
    use std::sync::atomic::AtomicBool;

    fn pin(cpu: usize) {
        // SAFETY: a zeroed cpu_set_t is valid; pid 0 is this thread.
        unsafe {
            let mut set: libc::cpu_set_t = std::mem::zeroed();
            libc::CPU_SET(cpu, &mut set);
            assert_eq!(
                libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set),
                0
            );
        }
    }

    // SAFETY: sched_getcpu has no preconditions.
    let cpu = usize::try_from(unsafe { libc::sched_getcpu() }).unwrap();
    pin(cpu);
    let stop = Arc::new(AtomicBool::new(false));
    let hogs: Vec<_> = (0..4)
        .map(|_| {
            let stop = Arc::clone(&stop);
            std::thread::spawn(move || {
                pin(cpu);
                while !stop.load(Ordering::Relaxed) {
                    std::hint::spin_loop();
                }
            })
        })
        .collect();
    // SAFETY: lowering this thread's own priority needs no privilege.
    unsafe {
        let tid = libc::gettid();
        assert_eq!(
            libc::setpriority(libc::PRIO_PROCESS, tid as libc::id_t, 19),
            0
        );
    }
    spin(wall);
    stop.store(true, Ordering::Relaxed);
    for hog in hogs {
        hog.join().unwrap();
    }
}

#[cfg(target_os = "linux")]
fn spin(wall: Duration) {
    let start = Instant::now();
    while start.elapsed() < wall {
        std::hint::spin_loop();
    }
}

/// The real Linux clock: blocking on other work (a slow script tick runs on
/// its isolate thread) is not run-queue wait; being kept off a busy CPU is.
#[cfg(target_os = "linux")]
#[test]
fn run_queue_wait_sees_starvation_and_not_own_work() {
    std::thread::spawn(|| {
        let stat = RunQueueStat::open().expect("schedstat");
        let before = stat.waited().unwrap();
        std::thread::sleep(Duration::from_millis(700));
        let own = stat.waited().unwrap().saturating_sub(before);
        assert!(own < CALM, "a 700 ms own wait counted {own:?}");

        let before = stat.waited().unwrap();
        spin_behind_hogs(Duration::from_secs(1));
        let starved = stat.waited().unwrap().saturating_sub(before);
        assert!(
            starved >= LATE,
            "1 s behind four hogs waited only {starved:?}"
        );
    })
    .join()
    .unwrap();
}

/// The real thread CPU clock (`CLOCK_THREAD_CPUTIME_ID`, the same call
/// macOS makes): the bot's own CPU work and its own reported waits are not
/// time off the CPU; being kept off a busy CPU mid-tick is.
#[cfg(target_os = "linux")]
#[test]
fn thread_cpu_clock_sees_starvation_and_not_own_work() {
    std::thread::spawn(|| {
        let mon = Arc::new(CapacityMonitor::new());
        let mut rec = SlotRecorder::new(&mon, Clock::ThreadCpu);
        let ended = |end: Instant, slept: Duration| Service {
            at: end,
            end,
            slept,
            overshoot: Duration::ZERO,
            ticks: 0,
        };
        rec.begin(Instant::now(), 0);
        spin(Duration::from_millis(700));
        let own = rec.record(ended(Instant::now(), Duration::ZERO));
        assert!(own < CALM, "700 ms of own work counted {own:?}");

        let parked = Instant::now();
        std::thread::sleep(Duration::from_millis(700));
        let woke = Instant::now();
        let slept = (woke - parked).min(Duration::from_millis(700));
        let own_wait = rec.record(ended(woke, slept));
        assert!(own_wait < CALM, "a 700 ms park counted {own_wait:?}");

        spin_behind_hogs(Duration::from_secs(1));
        let starved = rec.record(ended(Instant::now(), Duration::ZERO));
        assert!(
            starved >= LATE,
            "1 s behind four hogs was off the CPU only {starved:?}"
        );
    })
    .join()
    .unwrap();
}
