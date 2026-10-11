//! Live smoke: headless bots on Engine A; real CPU load makes the host
//! capacity warning appear, and it clears once the load stops. Nothing in
//! the product is slowed on purpose: the load is `yes` hogs pinned with
//! `taskset` to the same CPUs this test process is pinned to.
//!
//! Build first, then run the test binary pinned (so cargo and rustc are
//! not):
//!
//! ```sh
//! isohome cargo test -p frontend-core --test capacity_smoke --no-run
//! isohome env LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=cw BOT_NAV_BUILD=skip \
//!   WORLD_NAV_PACK=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack \
//!   WORLD_ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine \
//!   CAPACITY_SMOKE_CPUS=31 CAPACITY_SMOKE_HOGS=32 CAPACITY_SMOKE_HOG_NICE=-10 \
//!   CAPACITY_SMOKE_BOTS=20 \
//!   taskset -c 31 target/debug/deps/capacity_smoke-<hash> \
//!   --ignored --nocapture --test-threads=1
//! ```
//!
//! `CAPACITY_SMOKE_BUSY=1` keeps every bot on the 20 ms frame path (as a
//! running script does); by default the bots are idle in game (one wake per
//! game tick). `CAPACITY_SMOKE_HOG_NICE` runs the hogs at that nice value
//! (a negative one needs `ulimit -e` above 20); the bots keep the default.
//! Linux's scheduler serves a waking thread quickly against equal-priority
//! hogs, so equal-priority hogs alone keep each wake's wait near 10 ms.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use frontend_core::{OperatorSession, RenderPolicyLabel};
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

/// `n` `yes` processes pinned to `cpus` (a `taskset -c` list) at `nice`.
fn hogs(n: usize, cpus: &str, nice: i32) -> Vec<Child> {
    let nice = nice.to_string();
    (0..n)
        .map(|_| {
            Command::new("taskset")
                .args(["-c", cpus, "nice", "-n", &nice, "yes"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("cpu hog")
        })
        .collect()
}

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
    session.play().map_or(0, |play| {
        play.statuses
            .lock()
            .unwrap()
            .iter()
            .filter(|row| row.ingame)
            .count()
    })
}

/// Run-queue wait of this process's slot threads (each is named after its
/// bot), read from the same `/proc` counters the product reads, for the
/// receipt: the average per wake since the previous report, and the worst
/// bot's.
#[derive(Default)]
struct SlotWaits {
    prev: std::collections::HashMap<String, (u64, u64)>,
}

impl SlotWaits {
    fn report(&mut self, prefix: &str) -> String {
        let (mut wait, mut wakes, mut worst) = (0u64, 0u64, 0f64);
        for task in std::fs::read_dir("/proc/self/task")
            .into_iter()
            .flatten()
            .flatten()
        {
            let Ok(comm) = std::fs::read_to_string(task.path().join("comm")) else {
                continue;
            };
            let comm = comm.trim();
            if !comm.starts_with(prefix) {
                continue;
            }
            let Ok(stat) = std::fs::read_to_string(task.path().join("schedstat")) else {
                continue;
            };
            let fields: Vec<u64> = stat
                .split_whitespace()
                .filter_map(|f| f.parse().ok())
                .collect();
            let [_, w, s] = fields[..] else { continue };
            let tid = task.file_name().to_string_lossy().into_owned();
            let (pw, ps) = self.prev.insert(tid, (w, s)).unwrap_or((w, s));
            let (dw, ds) = (w.saturating_sub(pw), s.saturating_sub(ps));
            wait += dw;
            wakes += ds;
            if ds > 0 {
                worst = worst.max(dw as f64 / ds as f64 / 1e6);
            }
        }
        let avg = if wakes == 0 {
            0.0
        } else {
            wait as f64 / wakes as f64 / 1e6
        };
        format!(
            "slot threads: {avg:.1} ms run-queue wait per wake, worst bot {worst:.1} ms, \
             {wakes} wakes"
        )
    }
}

#[test]
#[ignore = "LIVE=1 Engine A smoke; starts and stops CPU hogs owned by this process"]
fn live_host_capacity_warning_appears_under_real_load_and_clears() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let engine = std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR");
    let pack = std::env::var("WORLD_NAV_PACK").expect("WORLD_NAV_PACK");
    let cpus = std::env::var("CAPACITY_SMOKE_CPUS").expect("CAPACITY_SMOKE_CPUS");
    let bots: usize = env_or("CAPACITY_SMOKE_BOTS", 20);
    let hog_n: usize = env_or("CAPACITY_SMOKE_HOGS", 16);
    let hog_nice: i32 = env_or("CAPACITY_SMOKE_HOG_NICE", 0);
    let busy = std::env::var("CAPACITY_SMOKE_BUSY").as_deref() == Ok("1");
    eprintln!(
        "pid {} allowed cpus {} | hogs {hog_n} at nice {hog_nice} on cpus {cpus} | \
         bots {bots} busy {busy}",
        std::process::id(),
        allowed_cpus()
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
    session.set_capacity_render(RenderPolicyLabel::Headless);
    session.start(vault, play);

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

    let prefix = names[0].split('_').next().unwrap_or_default().to_string();
    let mut waits = SlotWaits::default();
    waits.report(&prefix);

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
        waits.report(&prefix)
    );

    let loaded_at = Instant::now();
    let mut load = hogs(hog_n, &cpus, hog_nice);
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
                "WARN after {:.1}s of load: {}",
                loaded_at.elapsed().as_secs_f64(),
                notice.notice_text()
            );
            break;
        }
        if report.elapsed() >= Duration::from_secs(5) {
            eprintln!(
                "waiting for warn ({:.0}s, ingame {}); {}",
                loaded_at.elapsed().as_secs_f64(),
                ingame(&session),
                waits.report(&prefix)
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
    eprintln!("hogs stopped; {}", waits.report(&prefix));
    let clear_deadline = stopped_at + Duration::from_secs(90);
    loop {
        session.poll();
        if session.capacity_notice().is_none() {
            eprintln!(
                "CLEARED {:.1}s after the hogs stopped",
                stopped_at.elapsed().as_secs_f64()
            );
            break;
        }
        assert!(
            Instant::now() < clear_deadline,
            "warning did not clear after the hogs stopped"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    // The session log carries one line into and one out of the warning.
    let mut view = frontend_core::log::LogView::new(frontend_core::log::LogScope::Process);
    frontend_core::log::global().refresh(&mut view);
    let lines: Vec<String> = view
        .rows()
        .iter()
        .filter(|row| row.message.starts_with("Host "))
        .map(|row| format!("{:?} {}", row.level, row.message))
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
