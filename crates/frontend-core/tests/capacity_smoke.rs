//! Live smoke: 20 headless bots on Engine A, CPU hog, warning then clear.
//!
//! Idle/20 ms park sleeps in the kernel, so a CFS hog does not make the
//! existing `next_wake` measurement late. Enabled `SlotInput` keeps the
//! 20 ms busy cadence; `per_frame` then holds the slot thread 700 ms past
//! that wake (the same overrun a loaded tick would show) while a `yes`
//! hog runs beside it.
//!
//! ```sh
//! LIVE=1 BOT_CPU=1 BOT_LIVE_NAME_PREFIX=cw BOT_NAV_BUILD=skip \
//!   WORLD_NAV_PACK=/Volumes/dev-scratch/274bot-evidence/NAV-SHANTAY-EDGE/nav/289/bake-r2-a.navpack \
//!   WORLD_ENGINE_DIR=/Users/acfrazier/experiments/lostcity-289/engine \
//!   isohome cargo test -p frontend-core --test capacity_smoke -- --ignored --nocapture --test-threads=1
//! ```

use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use frontend_core::{OperatorSession, RenderPolicyLabel};
use host::SlotInput;
use host_play::{
    mint_live_names, profile_password, run_with_template, InstancePermit, ProfileOptions,
    SharedClientTemplate,
};
use vault::{Profile, ProfileSettings, Vault};

const N: usize = 20;

fn hog(n: usize) -> Vec<Child> {
    (0..n)
        .map(|_| {
            Command::new("yes")
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

#[test]
#[ignore = "LIVE=1 Engine A smoke; start/stop a CPU hog owned by this process"]
fn live_host_capacity_warning_appears_and_clears() {
    assert_eq!(std::env::var("LIVE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("BOT_CPU").as_deref(), Ok("1"));
    let engine = std::env::var("WORLD_ENGINE_DIR").expect("WORLD_ENGINE_DIR");
    let pack = std::env::var("WORLD_NAV_PACK").expect("WORLD_NAV_PACK");
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
    let names = mint_live_names(N);
    eprintln!("smoke accounts: {names:?} pid {}", std::process::id());
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
    let overrun = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&overrun);
    let play = run_with_template(
        template,
        false,
        profiles,
        |_| {
            let input = SlotInput::new();
            input.set_enabled(true);
            (Some(input), None)
        },
        move |_, _, _| {
            if flag.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(700));
            }
        },
    )
    .expect("play");
    let mut session = OperatorSession::<()>::new(InstancePermit::SkipLock);
    session.set_capacity_render(RenderPolicyLabel::Headless);
    session.start(vault, play);
    let login_deadline = Instant::now() + Duration::from_secs(90);
    loop {
        session.poll();
        let ingame = session
            .play()
            .map(|play| {
                play.statuses
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|row| row.ingame)
                    .count()
            })
            .unwrap_or(0);
        if ingame >= 15 {
            eprintln!("ingame {ingame}/{N}");
            break;
        }
        assert!(
            Instant::now() < login_deadline,
            "login timed out with {ingame} ready"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    overrun.store(true, Ordering::Relaxed);
    let mut hogs = hog(8);
    let hog_pids: Vec<u32> = hogs.iter().map(|c| c.id()).collect();
    eprintln!("hog pids: {hog_pids:?}");
    let warn_deadline = Instant::now() + Duration::from_secs(30);
    let mut last_report = Instant::now();
    loop {
        session.poll();
        if let Some(notice) = session.capacity_notice() {
            eprintln!("WARN {}", notice.notice_text());
            break;
        }
        if last_report.elapsed() >= Duration::from_secs(2) {
            let eval = session.play().map(|play| {
                play.capacity()
                    .evaluate(Instant::now(), RenderPolicyLabel::Headless)
            });
            eprintln!("waiting for warn: {eval:?}");
            last_report = Instant::now();
        }
        if Instant::now() >= warn_deadline {
            overrun.store(false, Ordering::Relaxed);
            kill_hogs(&mut hogs);
            panic!("warning did not appear; hog pids were {hog_pids:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    overrun.store(false, Ordering::Relaxed);
    kill_hogs(&mut hogs);
    eprintln!("overrun off, hogs stopped");
    let clear_deadline = Instant::now() + Duration::from_secs(90);
    loop {
        session.poll();
        if session.capacity_notice().is_none() {
            eprintln!("CLEARED");
            break;
        }
        assert!(
            Instant::now() < clear_deadline,
            "warning did not clear after hog stop"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
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
