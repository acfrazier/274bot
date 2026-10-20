//! Live smoke: headless bots on Engine A; real CPU load makes the host
//! capacity warning appear, and it clears once the load stops. Nothing in
//! the product is slowed on purpose: the load is `yes` hogs. On Linux they
//! are pinned with `taskset` to the CPUs in `CAPACITY_SMOKE_CPUS`, the same
//! CPUs this test process is pinned to; elsewhere (macOS has no CPU
//! pinning) leave `CAPACITY_SMOKE_CPUS` unset and they run anywhere.
//!
//! Build first, then run the test binary pinned (so cargo and rustc are
//! not):
//!
//! ```sh
//! isohome cargo test -p frontend-core --test capacity_smoke --no-run
//! isohome env LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=cw BOT_NAV_BUILD=skip \
//!   WORLD_NAV_PACK=/path/to/289.navpack WORLD_ENGINE_DIR=/path/to/289/engine \
//!   CAPACITY_SMOKE_CPUS=31 CAPACITY_SMOKE_HOGS=64 CAPACITY_SMOKE_HOG_NICE=-11 \
//!   CAPACITY_SMOKE_BOTS=20 \
//!   taskset -c 31 target/debug/deps/capacity_smoke-<hash> \
//!   --ignored --nocapture --test-threads=1
//! ```
//!
//! On macOS, supply the same local pack and engine paths, omit
//! `CAPACITY_SMOKE_CPUS`, and run the test binary without `taskset`.
//!
//! `CAPACITY_SMOKE_BUSY=1` keeps every bot on the 20 ms frame path (as a
//! running script does); by default the bots are idle in game (one wake per
//! game tick). `CAPACITY_SMOKE_HOG_NICE` runs the hogs at that nice value
//! (a negative one needs `ulimit -e` above 20 on Linux, root on macOS); the
//! bots keep the default. Linux's scheduler serves a waking thread quickly
//! against equal-priority hogs, so equal-priority hogs alone keep each
//! wake's wait near 10 ms.
//!
//! On Linux, `CAPACITY_SMOKE_MEASURE=off-cpu` makes the bots measure time
//! off the CPU (wall − thread CPU time − the loop's own waits), the macOS
//! and Windows measure, instead of run-queue wait; on macOS and Windows
//! that is the measure anyway. The receipts are each bot's own per-service
//! delays as the warning reads them.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use frontend_core::OperatorSession;
use host::SlotInput;
use host_play::{
    mint_live_names, profile_password, run_with_template, InstancePermit, ProfileOptions,
    SharedClientTemplate,
};
use vault::{Profile, ProfileSettings, Vault};

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

/// `n` `yes` processes at `nice`, pinned to `cpus` (a `taskset -c` list)
/// when given. They are forked from a thread allowed on every CPU, so the
/// `taskset` and `nice` wrappers start at once instead of queueing behind
/// the hogs already on `cpus`; `taskset` pins only the `yes`.
fn hogs(n: usize, cpus: Option<&str>, nice: i32) -> Vec<Child> {
    let nice = nice.to_string();
    let hog = || {
        let mut command = match cpus {
            Some(cpus) => {
                let mut command = Command::new("taskset");
                command.args(["-c", cpus, "nice"]);
                command
            }
            None => Command::new("nice"),
        };
        command
            .args(["-n", &nice, "yes"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("cpu hog")
    };
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                allow_every_cpu();
                (0..n).map(|_| hog()).collect()
            })
            .join()
            .expect("hog spawner")
    })
}

/// Let this thread (and what it forks) run on any online CPU.
#[cfg(target_os = "linux")]
#[allow(unsafe_code)]
fn allow_every_cpu() {
    // SAFETY: a zeroed cpu_set_t is valid; the kernel keeps only online
    // CPUs of the mask; pid 0 is this thread.
    unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        for cpu in 0..libc::CPU_SETSIZE as usize {
            libc::CPU_SET(cpu, &mut set);
        }
        assert_eq!(
            libc::sched_setaffinity(0, std::mem::size_of::<libc::cpu_set_t>(), &set),
            0
        );
    }
}

/// Nothing pins threads elsewhere.
#[cfg(not(target_os = "linux"))]
fn allow_every_cpu() {}

fn kill_hogs(hogs: &mut [Child]) {
    for child in hogs {
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// This process's allowed CPUs, for the receipt.
fn allowed_cpus() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status
                .lines()
                .find_map(|line| line.strip_prefix("Cpus_allowed_list:"))
                .map(|list| list.trim().to_string())
        })
        .unwrap_or_else(|| "?".into())
}

fn ingame(session: &OperatorSession<()>) -> usize {
    session.play_for_test().map_or(0, |play| {
        play.statuses
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.ingame)
            .count()
    })
}

/// Each active bot's delay per service over the last 10 s, as the warning
/// reads them: per bot, the late services (600 ms or more), the max and
/// the p95, each list lowest to highest.
fn service_delays(session: &OperatorSession<()>) -> String {
    let Some(play) = session.play_for_test() else {
        return "no play".into();
    };
    let bots = play.capacity().service_delays(Instant::now());
    let services: usize = bots.iter().map(|bot| bot.services).sum();
    let sorted = |pick: fn(&host::ServiceDelays) -> u128| {
        let mut values: Vec<u128> = bots.iter().map(pick).collect();
        values.sort_unstable();
        values
            .iter()
            .map(u128::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    };
    format!(
        "per-service delay over 10 s, {} bots, {services} services: \
         per-bot late services [{}]; max ms [{}]; p95 ms [{}]",
        bots.len(),
        sorted(|bot| bot.late as u128),
        sorted(|bot| bot.max.as_millis()),
        sorted(|bot| bot.p95.as_millis()),
    )
}

#[test]
#[ignore = "LIVE=1 Engine A smoke; starts and stops CPU hogs owned by this process"]
fn live_host_capacity_warning_appears_under_real_load_and_clears() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    // The operator log is installed before any bot starts, as in the panel.
    frontend_core::log::global();
    let engine = std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR");
    let pack = std::env::var("WORLD_NAV_PACK").expect("WORLD_NAV_PACK");
    let cpus = std::env::var("CAPACITY_SMOKE_CPUS").ok();
    let bots: usize = env_or("CAPACITY_SMOKE_BOTS", 20);
    let hog_n: usize = env_or("CAPACITY_SMOKE_HOGS", 16);
    let hog_nice: i32 = env_or("CAPACITY_SMOKE_HOG_NICE", 0);
    let busy = std::env::var("CAPACITY_SMOKE_BUSY").as_deref() == Ok("1");
    let off_cpu = std::env::var("CAPACITY_SMOKE_MEASURE").as_deref() == Ok("off-cpu");
    if off_cpu {
        host::force_off_cpu_measure();
    }
    eprintln!(
        "pid {} allowed cpus {} | hogs {hog_n} at nice {hog_nice} on cpus {} | \
         bots {bots} busy {busy} | measure {}",
        std::process::id(),
        allowed_cpus(),
        cpus.as_deref().unwrap_or("any"),
        if off_cpu { "off-cpu" } else { "default" }
    );
    let options = ProfileOptions {
        profile: Some("local-289".into()),
        revision: Some("289".into()),
        host: Some("127.0.0.1".into()),
        asset_host: Some("127.0.0.1".into()),
        port: Some(44594),
        http_port: Some(1080),
        engine_dir: Some(engine.into()),
        nav_pack: Some(pack.into()),
        ..ProfileOptions::default()
    };
    let bound = options
        .resolve(None)
        .expect("resolve")
        .bind()
        .expect("bind");
    let names = mint_live_names(bots);
    eprintln!("smoke accounts: {names:?}");
    let dir = tempfile_vault();
    let mut vault = Vault::create(&dir.join("vault"), "smoke-pass-01").unwrap();
    let mut profiles = Vec::new();
    for (i, name) in names.iter().enumerate() {
        let settings = ProfileSettings {
            auto_login: true,
            lowmem: true,
            raster: vault::RasterMode::Cpu,
            ..ProfileSettings::default()
        };
        let profile = Profile {
            username: name.clone(),
            password: profile_password(name).into(),
            uid: 900_000 + i as i32,
            settings,
        };
        vault.upsert(profile.clone()).unwrap();
        profiles.push(profile);
    }
    let template = SharedClientTemplate::load(bound).expect("template");
    let play = run_with_template(
        template,
        false,
        profiles,
        move |_| {
            let input = SlotInput::new();
            input.set_enabled(busy);
            (Some(input), None)
        },
        |_, _, _| {},
    )
    .expect("play");
    let mut session = OperatorSession::<()>::new(InstancePermit::SkipLock);
    session.declare_headless_render();
    session.start_for_test(vault, play);

    let login_deadline = Instant::now() + Duration::from_secs(180);
    loop {
        session.poll();
        let ready = ingame(&session);
        if ready >= bots * 9 / 10 {
            eprintln!("ingame {ready}/{bots}");
            break;
        }
        assert!(
            Instant::now() < login_deadline,
            "login timed out with {ready} ready"
        );
        std::thread::sleep(Duration::from_millis(50));
    }

    // Unloaded: the same pinned CPUs, no hogs, must not warn.
    let calm_until = Instant::now() + Duration::from_secs(20);
    while Instant::now() < calm_until {
        session.poll();
        assert!(
            session.capacity_notice().is_none(),
            "false warning without load: {:?}",
            session
                .capacity_notice()
                .map(|n| n.notice_text().to_string())
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    eprintln!(
        "no warning for 20 s without load; ingame {}; {}",
        ingame(&session),
        service_delays(&session)
    );

    let loaded_at = Instant::now();
    let mut load = hogs(hog_n, cpus.as_deref(), hog_nice);
    let pids: Vec<u32> = load.iter().map(Child::id).collect();
    eprintln!(
        "hog pids ({:.1}s to start them all): {pids:?}",
        loaded_at.elapsed().as_secs_f64()
    );
    let warn_deadline = loaded_at + Duration::from_secs(90);
    let mut report = Instant::now();
    loop {
        session.poll();
        if let Some(notice) = session.capacity_notice() {
            eprintln!(
                "WARN after {:.1}s of load: {}; {}",
                loaded_at.elapsed().as_secs_f64(),
                notice.notice_text(),
                service_delays(&session)
            );
            break;
        }
        if report.elapsed() >= Duration::from_secs(5) {
            eprintln!(
                "waiting for warn ({:.0}s, ingame {}); {}",
                loaded_at.elapsed().as_secs_f64(),
                ingame(&session),
                service_delays(&session)
            );
            report = Instant::now();
        }
        if Instant::now() >= warn_deadline {
            kill_hogs(&mut load);
            panic!("no warning under load; hog pids were {pids:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    kill_hogs(&mut load);
    let stopped_at = Instant::now();
    eprintln!("hogs stopped; {}", service_delays(&session));
    let clear_deadline = stopped_at + Duration::from_secs(90);
    loop {
        session.poll();
        if session.capacity_notice().is_none() {
            eprintln!(
                "CLEARED {:.1}s after the hogs stopped; {}",
                stopped_at.elapsed().as_secs_f64(),
                service_delays(&session)
            );
            break;
        }
        assert!(
            Instant::now() < clear_deadline,
            "warning did not clear after the hogs stopped"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // The first slot thread to bind names the measure in its own log; the
    // session log carries one line into and one out of the warning.
    use frontend_core::log::{global, LogScope, LogView};
    let mut measure = Vec::new();
    for name in &names {
        let mut view = LogView::new(LogScope::slot(name.clone()));
        global().refresh(&mut view);
        measure.extend(
            view.rows()
                .iter()
                .filter(|row| row.message().starts_with("host capacity: "))
                .map(|row| format!("{name}: {}", row.message())),
        );
    }
    eprintln!("log: {measure:?}");
    let expected = if off_cpu {
        "time off the CPU (thread CPU clock; forced by a test)"
    } else if cfg!(target_os = "linux") {
        "run-queue wait (Linux schedstat)"
    } else {
        "time off the CPU (thread CPU clock)"
    };
    assert!(
        measure.len() == 1 && measure[0].ends_with(expected),
        "{measure:?}"
    );
    let mut view = LogView::new(LogScope::Process);
    global().refresh(&mut view);
    let lines: Vec<String> = view
        .rows()
        .iter()
        .filter(|row| row.message().starts_with("Host "))
        .map(|row| format!("{:?} {}", row.level(), row.message()))
        .collect();
    for line in &lines {
        eprintln!("log: {line}");
    }
    assert_eq!(lines.len(), 2, "{lines:?}");
}

fn tempfile_vault() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "capacity-smoke-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
