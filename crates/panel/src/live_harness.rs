//! Headed `--live` / `--smoke` watch state machine. Child of `app` so the
//! UI loop keeps process exit, presentation, focus, shot PNG write, and F12.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::resource::sample_process;
use crate::session::Session;
use crate::window::{ShotState, ShotStatus};

/// Headed live harness: null_raster (2 slots), stress50 / stress50_full
/// (50 slots), a shared scenario (`script_<name>`), or `--smoke` (one
/// whole-window shot at scene 2, then exit 0).
pub(super) enum LiveHarness {
    Null(LiveNull),
    Stress(LiveStress),
    Script(LiveScript),
    Smoke(LiveSmoke),
}

/// Headed `null_raster` harness state. `started` is the 120s login clock.
pub(super) struct LiveNull {
    pub(super) started: Instant,
    pub(super) saw_scene2: bool,
    pub(super) passed: bool,
}

/// Headed `stress50` / `stress50_full` harness. `started` is the 600s
/// login clock. `name` is the `--live` token used in PASS/FAIL lines.
pub(super) struct LiveStress {
    pub(super) started: Instant,
    pub(super) last_announced: u8,
    pub(super) passed: bool,
    pub(super) name: &'static str,
    pub(super) host: String,
    pub(super) port: u16,
}

/// Headed `script_<name>` watch. The shared `ScenarioRunner` lives on the
/// `Session` (the slot thread ticks it); this struct only mirrors the
/// last-reported step for progress lines and latches the terminal state.
/// PASS → the caller exits 0; FAIL → exit 1. A terminal shot holds either
/// exit until the capture writes (or [`NAV_FULL_SHOT_DRAIN`] lapses).
/// `BUDGET_S` soak: print PASS but do not latch `passed` until the budget
/// elapses, so the window stays up.
pub(super) struct LiveScript {
    pub(super) name: String,
    pub(super) passed: bool,
    pub(super) failed: Option<String>,
    pub(super) last_step: Option<(usize, usize)>,
    /// Terminal-shot drain: the instant the runner first reported a
    /// terminal status while a shot was armed. The watch keeps pumping
    /// (returning `None`) until the shot writes or [`NAV_FULL_SHOT_DRAIN`]
    /// lapses, so the screenshot lands before the process exits.
    pub(super) drain_started: Option<Instant>,
    /// `BUDGET_S` set: keep the window after proof PASS until `soak_until`.
    pub(super) soak: bool,
    pub(super) soak_until: Option<Instant>,
    pub(super) announced_pass: bool,
    /// A native-core terminal decision has issued its one current capture.
    /// This prevents the write frame from re-arming the same label again.
    pub(super) native_failure_capture_requested: bool,
    /// Headed clean-stop recapture has issued its one post-Idle request.
    /// Later frames must observe Requested → Written without re-enqueueing.
    pub(super) clean_stop_capture_requested: bool,
    /// Separate wall-clock ceiling when scenario PASS arrives before the
    /// full shared core qualifies. `None` for every ordinary panel run.
    pub(super) core_deadline: Option<Instant>,
    /// Bounded post-PASS evidence. Soak runs take one fresh checkpoint after
    /// the initial proof and one final readback at the budget deadline.
    pub(super) soak_capture: SoakCapture,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SoakCapture {
    NotNeeded,
    PostPass {
        label: String,
        started: Option<Instant>,
    },
    WaitingForFinal,
    Final {
        label: String,
        started: Option<Instant>,
    },
    Complete,
}

pub(super) const SOAK_POSTPASS_SUFFIX: &str = "-postpass";
pub(super) const SOAK_FINAL_SUFFIX: &str = "-soak-final";
pub(super) const SCRIPT_STOP_WAIT: Duration = Duration::from_secs(45);

#[derive(Debug, PartialEq)]
pub(super) enum CoreGate {
    Disabled,
    Pending,
    Qualified(Option<Arc<serde_json::Value>>),
    Failed(String),
}

pub(super) fn catalog_core_gate(
    watch: Option<&host_play::catalog_core::CoreWatch>,
    deadline: Option<Instant>,
    now: Instant,
) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    use host_play::catalog_core::CoreWatchStatus;
    match watch.status() {
        CoreWatchStatus::Disabled => CoreGate::Disabled,
        CoreWatchStatus::Qualified => CoreGate::Qualified(None),
        CoreWatchStatus::Failed => CoreGate::Failed(
            watch
                .failure()
                .unwrap_or_else(|| "catalog core failed".into()),
        ),
        CoreWatchStatus::Ready | CoreWatchStatus::Running
            if deadline.is_some_and(|deadline| now >= deadline) =>
        {
            match watch.qualify() {
                Ok(evidence) => CoreGate::Qualified(Some(evidence)),
                Err(error) => CoreGate::Failed(format!(
                    "catalog core did not qualify before the headed deadline: {error}"
                )),
            }
        }
        CoreWatchStatus::Ready | CoreWatchStatus::Running => CoreGate::Pending,
    }
}

pub(super) fn external_core_gate(
    watch: Option<&host_play::external_loader::ExternalWatch>,
) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    use host_play::external_loader::ExternalWatchStatus;
    match watch.status() {
        ExternalWatchStatus::Disabled => CoreGate::Disabled,
        ExternalWatchStatus::Qualified => CoreGate::Qualified(None),
        ExternalWatchStatus::Failed => CoreGate::Failed(
            watch
                .failure()
                .unwrap_or_else(|| "external loader failed".into()),
        ),
        // Inner 180s/10s live on the watch. BUDGET_S must not replace them.
        ExternalWatchStatus::Ready | ExternalWatchStatus::Running => CoreGate::Pending,
    }
}

pub(super) fn pair_core_gate(
    watch: Option<&host_play::paired_core::PairWatch>,
    deadline: Option<Instant>,
    now: Instant,
) -> CoreGate {
    let Some(watch) = watch else {
        return CoreGate::Disabled;
    };
    use host_play::paired_core::PairWatchStatus;
    match watch.status() {
        PairWatchStatus::Disabled => CoreGate::Disabled,
        PairWatchStatus::Qualified => CoreGate::Qualified(None),
        PairWatchStatus::Failed => {
            CoreGate::Failed(watch.failure().unwrap_or_else(|| "pair core failed".into()))
        }
        PairWatchStatus::Ready | PairWatchStatus::Running
            if deadline.is_some_and(|deadline| now >= deadline) =>
        {
            match watch.qualify() {
                Ok(evidence) => CoreGate::Qualified(Some(evidence)),
                Err(error) => CoreGate::Failed(format!(
                    "pair core did not qualify before the headed deadline: {error}"
                )),
            }
        }
        PairWatchStatus::Ready | PairWatchStatus::Running => CoreGate::Pending,
    }
}

/// Headed `--smoke` watch. The `render_smoke` scenario's shot sink fires
/// the tick the focused slot reaches scene 2, but the capture is held
/// [`SMOKE_SETTLE`] so the slot's 1 fps renderer can rasterize the world
/// first; this watch latches `passed` once `pump_shots` has written the
/// PNG (the caller exits 0), mirrors the shared runner's failures, and
/// FAILs when scene 2 never arrives within [`SMOKE_DEADLINE`].
pub(super) struct LiveSmoke {
    pub(super) started: Instant,
    pub(super) last_step: Option<(usize, usize)>,
    pub(super) failed: Option<String>,
    /// The instant the pure trigger first saw the focused slot at scene 2
    /// (`None` before). The settle gate holds the shot until
    /// [`SMOKE_SETTLE`] has elapsed; the deadline message distinguishes a
    /// stuck login from a missing write.
    pub(super) saw_scene2_at: Option<Instant>,
    /// The scene2 PNG landed (`pump_shots` wrote it): the caller exits 0.
    pub(super) passed: bool,
}

/// Which live harness the boot starts.
#[derive(Debug)]
pub(super) enum LiveBoot {
    NullRaster,
    Stress50,
    /// Same 50-head wall as [`LiveBoot::Stress50`], every member painting
    /// at 50 fps (Game + sidecar).
    Stress50Full,
    Script {
        name: String,
    },
    /// `--smoke`: the `render_smoke` scenario with the shot sink armed.
    /// The scenario's own settings carry the 300 s deadline and the off
    /// mainland-base gate; `live_smoke_tick` keeps its outer ceiling.
    Smoke,
}

impl LiveBoot {
    /// Prepare the harness session and return the installed variant. `Smoke`
    /// and `Script` arm the whole-window shot sink too.
    pub(super) fn start(
        self,
        session: &mut Session,
        shots: Arc<Mutex<ShotState>>,
        shot_dir: &mut Option<PathBuf>,
    ) -> Result<LiveHarness, String> {
        match self {
            LiveBoot::NullRaster => {
                session.live_prepare_null_raster()?;
                Ok(LiveHarness::Null(LiveNull {
                    started: Instant::now(),
                    saw_scene2: false,
                    passed: false,
                }))
            }
            LiveBoot::Stress50 => {
                session.live_prepare_stress50()?;
                Ok(LiveHarness::Stress(LiveStress {
                    started: Instant::now(),
                    last_announced: 0,
                    passed: false,
                    name: "stress50",
                    host: session.play_options().host.clone(),
                    port: session.play_options().port,
                }))
            }
            LiveBoot::Stress50Full => {
                session.live_prepare_stress50_full()?;
                Ok(LiveHarness::Stress(LiveStress {
                    started: Instant::now(),
                    last_announced: 0,
                    passed: false,
                    name: "stress50_full",
                    host: session.play_options().host.clone(),
                    port: session.play_options().port,
                }))
            }
            LiveBoot::Script { name } => {
                let scenario_name = name.strip_prefix("script_").unwrap_or(&name);
                let scenario = if session.external_core_enabled()
                    && scenario_name == host_play::external_loader::LIVE_SCENARIO
                {
                    crate::session::external_loader_fixture()
                } else {
                    scenario::get(scenario_name)
                        .ok_or_else(|| format!("unknown scenario {scenario_name}"))?
                };
                let scenario_deadline = scenario.settings.deadline;
                session.live_prepare_script(scenario)?;
                arm_scenario_shots(session, Arc::clone(&shots), shot_dir);
                let budget = scenario::budget_s_from_env();
                let core_deadline = {
                    let catalog = session
                        .catalog_core_watch()
                        .filter(|watch| watch.configured());
                    let pair = session
                        .paired_core_watch()
                        .filter(|watch| watch.configured());
                    (catalog.is_some() || pair.is_some())
                        .then_some(Instant::now() + budget.unwrap_or(scenario_deadline))
                };
                Ok(LiveHarness::Script(LiveScript {
                    name,
                    passed: false,
                    failed: None,
                    last_step: None,
                    drain_started: None,
                    soak: budget.is_some(),
                    soak_until: budget.map(|d| Instant::now() + d),
                    announced_pass: false,
                    native_failure_capture_requested: false,
                    clean_stop_capture_requested: false,
                    core_deadline,
                    soak_capture: if budget.is_some() {
                        SoakCapture::WaitingForFinal
                    } else {
                        SoakCapture::NotNeeded
                    },
                }))
            }
            LiveBoot::Smoke => {
                let scenario = scenario::get("render_smoke")
                    .ok_or_else(|| "unknown scenario render_smoke".to_string())?;
                session.live_prepare_script(scenario)?;
                arm_scenario_shots(session, Arc::clone(&shots), shot_dir);
                Ok(LiveHarness::Smoke(LiveSmoke {
                    started: Instant::now(),
                    last_step: None,
                    failed: None,
                    saw_scene2_at: None,
                    passed: false,
                }))
            }
        }
    }
}

/// `--smoke` login+world deadline: the focused slot must reach scene 2
/// (and the shot write must land) within this window or the run FAILs
/// with exit 1. Generous — scene 2 usually lands inside the first two
/// minutes.
pub(super) const SMOKE_DEADLINE: Duration = Duration::from_secs(300);

/// `nav_full` FAIL shot drain: after a `Failed` status the panel holds
/// the exit up to this long so the terminal whole-window capture — asked
/// for by the slot thread, written by the render readback — lands before
/// exit 1.
pub(super) const NAV_FULL_SHOT_DRAIN: Duration = Duration::from_secs(10);

/// `--smoke` render-settle window: after the focused slot reaches scene 2,
/// the whole-window shot request waits this long before reaching the render
/// readback — the slot's renderer rasterizes at 1 fps on the CPU backend
/// (slower in a debug build), so an immediate capture would still show the
/// title/loading screen.
pub(super) const SMOKE_SETTLE: Duration = Duration::from_secs(3);

/// `--smoke` trigger: fire exactly once, on the frame the focused slot
/// first reaches `ingame && scene_state == 2`. Pure so the trigger path
/// is a table test; the smoke watch calls it against the focused slot's
/// status every frame.
pub(super) fn smoke_should_fire(
    smoke_armed: bool,
    already_fired: bool,
    ingame: bool,
    scene_state: i32,
) -> bool {
    smoke_armed && !already_fired && ingame && scene_state == 2
}

/// `--smoke` render-settle predicate: the scene2 shot request is released
/// to the render readback only once the focused slot has held scene 2 for
/// [`SMOKE_SETTLE`] (wall-clock, so the slot's 1 fps renderer has rasterized
/// the world) and is still ingame. `saw_scene2_at` is `None` until the
/// trigger latches scene 2.
pub(super) fn smoke_settled(saw_scene2_at: Option<Instant>, now: Instant, ingame: bool) -> bool {
    match saw_scene2_at {
        Some(t) => ingame && now.saturating_duration_since(t) >= SMOKE_SETTLE,
        None => false,
    }
}

/// Headed watch: wait until both slots are scene 2, print RSS/counters, PASS.
/// Does **not** freeze-assert (operator may click the rail). Null freeze is
/// the headless `e2e` twin.
pub(super) fn live_null_tick(
    live: &mut LiveNull,
    statuses: &[host_play::SlotStatus],
) -> Option<String> {
    if live.passed {
        return None;
    }
    let ready = statuses
        .iter()
        .filter(|s| s.ingame && s.scene_state == 2)
        .count();
    if ready < 2 {
        if live.started.elapsed() >= Duration::from_secs(120) {
            return Some(format!(
                "live null_raster: {ready}/2 slot(s) ingame scene 2 after 120s"
            ));
        }
        return None;
    }
    let (rss, _) = sample_process();
    let Some(test2) = statuses.iter().find(|s| s.username == "test2") else {
        return Some("live null_raster: missing test2".into());
    };
    let Some(test) = statuses.iter().find(|s| s.username == "test") else {
        return Some("live null_raster: missing test".into());
    };
    println!("live null_raster: rss={rss}");
    println!(
        "live null_raster test2 bytes={}/{}",
        test2.bytes_in, test2.bytes_out
    );
    println!(
        "live null_raster test  bytes={}/{}",
        test.bytes_in, test.bytes_out
    );
    println!("PASS: live null_raster");
    live.saw_scene2 = true;
    live.passed = true;
    None
}

/// Headed watch: count Clients that are up — every slot is a full Client,
/// so "up" is `ingame && scene_state==2` for all of them. Announce 1, 10,
/// then 50. At 50 print PASS (with RSS) and stay open. Timeout 600s. Does
/// **not** freeze-assert (operator may click). Does **not** fail on RSS
/// magnitude — `stress50` is the release RAM check; `stress50_full` is
/// the same wall with every renderer at 50 fps.
pub(super) fn live_stress_tick(
    live: &mut LiveStress,
    statuses: &[host_play::SlotStatus],
) -> Option<String> {
    if live.passed {
        return None;
    }
    let name = live.name;
    let n = statuses.iter().filter(|s| s.is_up()).count();
    if n >= 1 && live.last_announced < 1 {
        println!("live {name}: 1/50 up");
        live.last_announced = 1;
    }
    if n >= 10 && live.last_announced < 10 {
        println!("live {name}: 10/50 up");
        live.last_announced = 10;
    }
    if n >= 50 {
        let (rss, _) = sample_process();
        let workers = client::io::OnDemand::live_workers_for(&live.host, live.port);
        let tcp = host_play::count_tcp_to(&live.host, live.port);
        let tcp_s = tcp.map(|c| c.to_string()).unwrap_or_else(|| "?".into());
        println!("PASS: live {name} rss={rss} up={n}/50 ondemand={workers} tcp={tcp_s}");
        live.last_announced = 50;
        live.passed = true;
        return None;
    }
    if live.started.elapsed() >= Duration::from_secs(600) {
        return Some(format!("live {name}: {n}/50 up after 600s"));
    }
    None
}

pub(super) fn pair_terminal_actor_names(session: &Session) -> Option<(String, String)> {
    let runner = session.scenario.lock().unwrap();
    let runner = runner.as_ref()?;
    let a = runner.profile_name().to_string();
    let b = runner.companion_profile_name(0).to_string();
    if a.is_empty() || b.is_empty() || a == b {
        None
    } else {
        Some((a, b))
    }
}

pub(super) fn pair_shot_labels(base: &str, a: &str, b: &str) -> [String; 2] {
    [format!("{base}-{a}"), format!("{base}-{b}")]
}

#[cfg(test)]
std::thread_local! {
    pub(super) static ACTOR_SNAPSHOT_SERIALIZATIONS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

pub(super) fn actor_snapshot_json(
    actor: &str,
    snapshot: &api::snapshot::GameSnapshot,
) -> Result<String, String> {
    #[cfg(test)]
    ACTOR_SNAPSHOT_SERIALIZATIONS.with(|count| count.set(count.get() + 1));
    let mut value = serde_json::to_value(snapshot).map_err(|error| error.to_string())?;
    if let Some(object) = value.as_object_mut() {
        object.insert("actor".into(), serde_json::Value::String(actor.to_string()));
    }
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

pub(super) fn pair_terminal_shot_status(shots: &ShotState, labels: &[String]) -> ShotStatus {
    if labels.is_empty() {
        return ShotStatus::Missing;
    }
    let mut pending = ShotStatus::Written;
    for label in labels {
        match shots.status(label) {
            ShotStatus::Failed(error) => return ShotStatus::Failed(error),
            ShotStatus::Missing => return ShotStatus::Missing,
            ShotStatus::Written => {}
            other => pending = other,
        }
    }
    pending
}

/// Request current scene2 snapshots for each distinct pair actor. Skip
/// title-screen / missing views so an early snapshot cannot stand in for
/// terminal world proof. Capture failure stays FAIL.
pub(super) fn enqueue_pair_terminal_shots(
    session: &Session,
    shots: &Mutex<ShotState>,
    base_label: &str,
) {
    let Some((a, b)) = pair_terminal_actor_names(session) else {
        return;
    };
    let labels = pair_shot_labels(base_label, &a, &b);
    let requests = {
        let states = session.nav_states.lock().unwrap();
        [&a, &b]
            .into_iter()
            .zip(labels.iter())
            .filter_map(|(name, label)| {
                let (snapshot, _) = states.get(name)?;
                if !snapshot.ingame() || snapshot.scene_state() != 2 {
                    return None;
                }
                Some((
                    label.clone(),
                    name.to_string(),
                    actor_snapshot_json(name, snapshot).ok()?,
                ))
            })
            .collect::<Vec<_>>()
    };
    let mut shots = shots.lock().unwrap();
    for (label, actor, json) in requests {
        if matches!(shots.status(&label), ShotStatus::Missing) {
            shots.enqueue_for_actor(label, json, actor);
        }
    }
}

/// Post-run (or failure) capture of the owned external actor at scene 2.
/// Prerequisite fixture writes use a distinct label and cannot discharge this.
pub(super) fn enqueue_external_terminal_shot(
    session: &Session,
    shots: &Mutex<ShotState>,
    label: &str,
) {
    let Some(watch) = session.external_core_watch() else {
        return;
    };
    if !watch.configured() {
        return;
    }
    let name = watch.account();
    let json = {
        let states = session.nav_states.lock().unwrap();
        let Some((snapshot, _)) = states.get(&name) else {
            return;
        };
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return;
        }
        actor_snapshot_json(&name, snapshot).ok()
    };
    let Some(json) = json else {
        return;
    };
    let mut shots = shots.lock().unwrap();
    if matches!(shots.status(label), ShotStatus::Missing) {
        shots.enqueue(label.to_string(), json);
    }
}

/// Re-arm a completed single-actor terminal capture for a later native-core
/// decision, pairing that failure with the current scene instead of the old image.
/// Re-request the headed terminal capture after Idle so the PNG can show
/// BoneBurier v2 paint, script Idle, and the clean self-stop reason.
/// Fail closed when the current focused view is not `ingame && scene_state == 2`
/// so an earlier pre-Idle PNG cannot discharge the recapture.
pub(super) fn enqueue_fresh_terminal_shot(
    session: &Session,
    shots: &Mutex<ShotState>,
    label: &str,
) -> Result<(), String> {
    let current = (|| {
        let actor = session
            .focused_name()
            .ok_or_else(|| "fresh terminal capture has no focused actor".to_string())?;
        let states = session.nav_states.lock().unwrap();
        let (snapshot, _) = states
            .get(&actor)
            .ok_or_else(|| "fresh terminal capture has no current snapshot".to_string())?;
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("fresh terminal capture requires a current ingame scene-2 snapshot".into());
        }
        actor_snapshot_json(&actor, snapshot)
    })();
    let mut shots = shots.lock().unwrap();
    match current {
        Ok(json) => {
            shots.enqueue(label.to_string(), json);
            Ok(())
        }
        Err(error) => {
            shots.fail_labels(&[label.to_string()], &error);
            Err(error)
        }
    }
}

/// One-shot post-Idle recapture. Wait for the original labeled shot to
/// reach Written, then re-arm exactly once from a current scene-2 snapshot
/// and reset only the capture drain window.
pub(super) fn request_clean_stop_capture(
    live: &mut LiveScript,
    session: &Session,
    shots: Option<&Mutex<ShotState>>,
    terminal_shot: Option<&str>,
) -> Result<(), String> {
    if live.clean_stop_capture_requested {
        return Ok(());
    }
    let (Some(shots), Some(label)) = (shots, terminal_shot) else {
        return Ok(());
    };
    if !matches!(shots.lock().unwrap().status(label), ShotStatus::Written) {
        return Ok(());
    }
    live.clean_stop_capture_requested = true;
    live.drain_started = None;
    enqueue_fresh_terminal_shot(session, shots, label)
}

pub(super) fn enqueue_current_terminal_shot(
    session: &Session,
    shots: &Mutex<ShotState>,
    label: &str,
) {
    let current = (|| {
        let actor = session
            .focused_name()
            .ok_or_else(|| "native failure capture has no focused actor".to_string())?;
        let states = session.nav_states.lock().unwrap();
        let (snapshot, _) = states
            .get(&actor)
            .ok_or_else(|| "native failure capture has no current snapshot".to_string())?;
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return Err("native failure capture requires a current ingame scene-2 snapshot".into());
        }
        actor_snapshot_json(&actor, snapshot)
    })();
    let mut shots = shots.lock().unwrap();
    match current {
        Ok(json) if matches!(shots.status(label), ShotStatus::Written) => {
            shots.enqueue(label.to_string(), json);
        }
        Ok(_) => {}
        Err(error) => {
            // The earlier image remains historical evidence on disk, but may
            // not discharge the current failure capture when its scene is gone.
            shots.fail_labels(&[label.to_string()], &error);
        }
    }
}

/// Enqueue one fresh bounded soak readback. Pair and external runs retain
/// their actor ownership rules; ordinary scenarios use the current focused
/// scene-2 snapshot. No request is made unless the label is still missing.
pub(super) fn enqueue_soak_capture(session: &Session, shots: &Mutex<ShotState>, label: &str) {
    if !matches!(shots.lock().unwrap().status(label), ShotStatus::Missing) {
        return;
    }
    if session
        .paired_core_watch()
        .is_some_and(|watch| watch.configured())
    {
        enqueue_pair_terminal_shots(session, shots, label);
        return;
    }
    if session
        .external_core_watch()
        .is_some_and(|watch| watch.configured())
    {
        enqueue_external_terminal_shot(session, shots, label);
        return;
    }
    let Some(actor) = session.focused_name() else {
        return;
    };
    let json = {
        let states = session.nav_states.lock().unwrap();
        let Some((snapshot, _)) = states.get(&actor) else {
            return;
        };
        if !snapshot.ingame() || snapshot.scene_state() != 2 {
            return;
        }
        actor_snapshot_json(&actor, snapshot).ok()
    };
    if let Some(json) = json {
        shots.lock().unwrap().enqueue(label.to_string(), json);
    }
}

pub(super) fn soak_capture_status(session: &Session, shots: &ShotState, label: &str) -> ShotStatus {
    if session
        .paired_core_watch()
        .is_some_and(|watch| watch.configured())
    {
        if let Some((a, b)) = pair_terminal_actor_names(session) {
            return pair_terminal_shot_status(shots, &pair_shot_labels(label, &a, &b));
        }
    }
    shots.status(label)
}

pub(super) fn request_native_failure_capture(
    live: &mut LiveScript,
    session: &Session,
    shots: Option<&Mutex<ShotState>>,
    terminal_shot: Option<&str>,
) {
    if live.native_failure_capture_requested {
        return;
    }
    // A prerequisite terminal shot may have consumed its drain while the
    // shared core was still pending. The native terminal decision gets a
    // fresh bounded window, without extending the gameplay deadline.
    live.native_failure_capture_requested = true;
    live.drain_started = None;
    if let (Some(shots), Some(label)) = (shots, terminal_shot) {
        enqueue_current_terminal_shot(session, shots, label);
    }
}

pub(super) fn hold_script_terminal_shot(
    live: &mut LiveScript,
    session: &Session,
    terminal_shot: Option<&str>,
    fallback: &ShotStatus,
    shots: Option<&Mutex<ShotState>>,
) -> Result<bool, String> {
    let owned;
    let mut pair_labels: Option<[String; 2]> = None;
    let status = match (
        shots,
        terminal_shot,
        session.paired_core_watch(),
        session.external_core_watch(),
    ) {
        (Some(shots), Some(label), Some(watch), _) if watch.configured() => {
            enqueue_pair_terminal_shots(session, shots, label);
            match pair_terminal_actor_names(session) {
                Some((a, b)) => {
                    let labels = pair_shot_labels(label, &a, &b);
                    owned = pair_terminal_shot_status(&shots.lock().unwrap(), &labels);
                    pair_labels = Some(labels);
                    &owned
                }
                None => fallback,
            }
        }
        (Some(shots), _, _, Some(watch)) if watch.configured() && watch.needs_terminal_hold() => {
            let label = host_play::external_loader::TERMINAL_SHOT;
            enqueue_external_terminal_shot(session, shots, label);
            owned = shots.lock().unwrap().status(label);
            if !matches!(owned, ShotStatus::Missing) {
                watch.note_capture_requested();
            }
            return hold_terminal_shot(live, Some(label), &owned);
        }
        (Some(shots), Some(label), _, _) => {
            // `ui_frame` snapshots the status before `live_script_tick`. A
            // native-core failure may re-arm a previously Written shot above,
            // so the caller's fallback can describe the old capture. Re-read
            // the ledger after the request to hold for the new capture.
            owned = shots.lock().unwrap().status(label);
            &owned
        }
        _ => fallback,
    };
    let result = hold_terminal_shot(live, terminal_shot, status);
    if let (Err(error), Some(shots), Some(labels)) = (&result, shots, pair_labels) {
        shots.lock().unwrap().fail_labels(&labels, error);
    }
    result
}

/// Hold a terminal-shot exit until `pump_shots` writes. The drain remains
/// bounded, but a terminal PASS may not turn a missing requested pair into
/// success when the bound lapses.
pub(super) fn hold_terminal_shot(
    live: &mut LiveScript,
    label: Option<&str>,
    status: &ShotStatus,
) -> Result<bool, String> {
    let Some(label) = label else {
        return Ok(false);
    };
    match status {
        ShotStatus::Written => return Ok(false),
        ShotStatus::Failed(error) => {
            return Err(format!("terminal shot {label} failed: {error}"));
        }
        ShotStatus::Missing
        | ShotStatus::Requested
        | ShotStatus::ReadbackPending
        | ShotStatus::WritePending => {}
    }
    if live
        .drain_started
        .is_some_and(|t0| t0.elapsed() >= NAV_FULL_SHOT_DRAIN)
    {
        return Err(format!(
            "terminal shot {label} was not written within {}s (capture stage: {status:?})",
            NAV_FULL_SHOT_DRAIN.as_secs(),
        ));
    }
    // The frame the terminal status is first seen: the request may have
    // landed after `pump_shots` ran, so the write needs another frame.
    live.drain_started.get_or_insert_with(Instant::now);
    Ok(true)
}

/// Advance the two extra captures required by `BUDGET_S`. The state machine
/// is driven once per UI frame, with the same bounded drain as terminal shots.
pub(super) fn soak_capture_tick(
    live: &mut LiveScript,
    session: &Session,
    shots: &Mutex<ShotState>,
    terminal_shot: Option<&str>,
) -> Result<bool, String> {
    let base = terminal_shot.unwrap_or(&live.name);
    match &mut live.soak_capture {
        SoakCapture::PostPass { label, started } => {
            enqueue_soak_capture(session, shots, label);
            let status = soak_capture_status(session, &*shots.lock().unwrap(), label);
            match status {
                ShotStatus::Written => {
                    println!("[panel] soak checkpoint {label} written");
                    live.soak_capture = SoakCapture::WaitingForFinal;
                    live.drain_started = None;
                    Ok(true)
                }
                ShotStatus::Failed(error) => {
                    Err(format!("soak checkpoint {label} failed: {error}"))
                }
                _ => {
                    let t0 = started.get_or_insert_with(Instant::now);
                    if t0.elapsed() >= NAV_FULL_SHOT_DRAIN {
                        Err(format!(
                            "soak checkpoint {label} was not written within {}s",
                            NAV_FULL_SHOT_DRAIN.as_secs()
                        ))
                    } else {
                        Ok(true)
                    }
                }
            }
        }
        SoakCapture::WaitingForFinal => {
            if live
                .soak_until
                .is_some_and(|deadline| Instant::now() < deadline)
            {
                return Ok(true);
            }
            let label = format!("{base}{SOAK_FINAL_SUFFIX}");
            enqueue_soak_capture(session, shots, &label);
            live.soak_capture = SoakCapture::Final {
                label,
                started: None,
            };
            Ok(true)
        }
        SoakCapture::Final { label, started } => {
            enqueue_soak_capture(session, shots, label);
            let status = soak_capture_status(session, &*shots.lock().unwrap(), label);
            match status {
                ShotStatus::Written => {
                    println!("[panel] soak final readback {label} written");
                    live.soak_capture = SoakCapture::Complete;
                    Ok(false)
                }
                ShotStatus::Failed(error) => {
                    Err(format!("soak final readback {label} failed: {error}"))
                }
                _ => {
                    let t0 = started.get_or_insert_with(Instant::now);
                    if t0.elapsed() >= NAV_FULL_SHOT_DRAIN {
                        Err(format!(
                            "soak final readback {label} was not written within {}s",
                            NAV_FULL_SHOT_DRAIN.as_secs()
                        ))
                    } else {
                        Ok(true)
                    }
                }
            }
        }
        SoakCapture::NotNeeded | SoakCapture::Complete => Ok(false),
    }
}

pub(super) fn script_failure_scenario(
    live_name: &str,
    evidence: Option<&scenario::Evidence>,
) -> String {
    evidence
        .map(|evidence| evidence.scenario.clone())
        .unwrap_or_else(|| {
            live_name
                .strip_prefix("script_")
                .unwrap_or(live_name)
                .to_string()
        })
}

/// Headed script watch: mirror the shared `ScenarioRunner` each frame.
/// PASS prints the JSON evidence record and latches `passed` (the caller
/// exits 0); FAIL prints the record and returns the message (exit 1).
/// When the runner armed a terminal shot, either outcome holds until the
/// capture writes (or [`NAV_FULL_SHOT_DRAIN`] lapses).
pub(super) fn live_script_tick(
    live: &mut LiveScript,
    session: &mut Session,
    terminal_shot_status: &ShotStatus,
    shots: Option<&Mutex<ShotState>>,
) -> Option<String> {
    if live.passed || live.failed.is_some() {
        return None;
    }
    let (status, evidence, terminal_shot) = {
        let guard = session.scenario.lock().unwrap();
        (
            guard.as_ref().map(|r| r.status()),
            guard.as_ref().and_then(|r| r.evidence().cloned()),
            guard.as_ref().and_then(|r| r.terminal_shot()),
        )
    };
    let core_watch = session.catalog_core_watch();
    let core_gate = catalog_core_gate(core_watch.as_ref(), live.core_deadline, Instant::now());
    let pair_watch = session.paired_core_watch();
    let pair_gate = pair_core_gate(pair_watch.as_ref(), live.core_deadline, Instant::now());
    if let Some(watch) = session.external_core_watch() {
        match &status {
            Some(scenario::RunnerStatus::Passed) => watch.note_prereq_passed(),
            Some(scenario::RunnerStatus::Failed(msg)) => watch.note_prereq_failed(msg),
            _ => {}
        }
    }
    session.pump_external_loader();
    // A failed proof still owns its execution. Stop must reach its bounded
    // outcome before a terminal capture or scenario failure can exit the host.
    if session
        .external_core_watch()
        .is_some_and(|watch| watch.cleanup_pending())
    {
        return None;
    }
    if let (Some(watch), Some(shots)) = (session.external_core_watch(), shots) {
        if watch.needs_terminal_hold() || watch.terminal_capture_due() {
            let label = host_play::external_loader::TERMINAL_SHOT;
            enqueue_external_terminal_shot(session, shots, label);
            let issued = !matches!(shots.lock().unwrap().status(label), ShotStatus::Missing);
            if issued {
                watch.note_capture_requested();
            } else if watch.terminal_capture_due() {
                watch.fail("external loader terminal capture without ingame && scene_state == 2");
            }
        }
    }
    let ext_watch = session.external_core_watch();
    let ext_gate = external_core_gate(ext_watch.as_ref());
    let record = |evidence: &Option<scenario::Evidence>| {
        evidence.as_ref().map(|ev| ev.to_json()).unwrap_or_default()
    };
    // Compact core evidence is additive: preserve the existing scenario JSON
    // receipt byte-for-byte and emit the shared witness only at a terminal
    // decision, off the gameplay observation thread.
    let record_core = || {
        core_watch
            .as_ref()
            .filter(|watch| watch.configured())
            .map(|watch| match &core_gate {
                CoreGate::Qualified(Some(evidence)) => evidence.to_string(),
                _ => watch
                    .qualify()
                    .map(|evidence| evidence.to_string())
                    .unwrap_or_else(|_| watch.evidence().to_string()),
            })
    };
    let record_pair = || {
        pair_watch
            .as_ref()
            .filter(|watch| watch.configured())
            .map(|watch| match &pair_gate {
                CoreGate::Qualified(Some(evidence)) => evidence.to_string(),
                _ => watch
                    .qualify()
                    .map(|evidence| evidence.to_string())
                    .unwrap_or_else(|_| watch.evidence().to_string()),
            })
    };
    let record_ext = || {
        ext_watch
            .as_ref()
            .filter(|watch| watch.configured())
            .map(|watch| match &ext_gate {
                CoreGate::Qualified(Some(evidence)) => evidence.to_string(),
                _ => watch
                    .qualify()
                    .map(|evidence| evidence.to_string())
                    .unwrap_or_else(|_| watch.evidence().to_string()),
            })
    };
    let live_line = || record_ext().unwrap_or_else(|| record(&evidence));
    let failure_name = script_failure_scenario(&live.name, evidence.as_ref());
    let failure_evidence = evidence.clone();
    let failure_line = |message: &str| {
        serde_json::json!({
            "scenario": failure_name,
            "outcome": "FAIL",
            "message": message,
            "prerequisite": failure_evidence,
        })
        .to_string()
    };
    let proof_name = live.name.clone();
    let emit_proof = |ok: bool| {
        if let Some(core) = record_core() {
            if ok {
                println!("CATALOG_CORE: {proof_name} {core}");
            } else {
                eprintln!("CATALOG_CORE: {proof_name} {core}");
            }
        }
        if let Some(pair) = record_pair() {
            if ok {
                println!("PAIRED_CORE: {proof_name} {pair}");
            } else {
                eprintln!("PAIRED_CORE: {proof_name} {pair}");
            }
        }
        if let Some(ext) = record_ext() {
            if ok {
                println!("EXTERNAL_LOADER: {proof_name} {ext}");
            } else {
                eprintln!("EXTERNAL_LOADER: {proof_name} {ext}");
            }
        }
    };
    // Preparation can fail before the scenario runner receives its first
    // client tick. Surface only the runner-owned slots' producer-marked,
    // terminal asset-init fact; ordinary retryable login errors stay pending.
    let terminal_startup_failure = {
        let guard = session.scenario.lock().unwrap();
        guard.as_ref().and_then(|runner| {
            let owned = runner.owned_profile_names();
            host_play::owned_terminal_startup_error(&session.statuses(), &owned)
        })
    };
    if let Some(message) = terminal_startup_failure {
        emit_proof(false);
        eprintln!("FAIL: live {} {}", live.name, failure_line(&message));
        live.failed = Some(message.clone());
        return Some(message);
    }
    if let CoreGate::Failed(message) = &core_gate {
        request_native_failure_capture(live, session, shots, terminal_shot);
        match hold_script_terminal_shot(live, session, terminal_shot, terminal_shot_status, shots) {
            Ok(true) => return None,
            Err(error) => eprintln!("[panel] {error}"),
            Ok(false) => {}
        }
        emit_proof(false);
        eprintln!("FAIL: live {} {}", live.name, failure_line(message));
        live.failed = Some(message.clone());
        return Some(message.clone());
    }
    if let CoreGate::Failed(message) = &pair_gate {
        match hold_script_terminal_shot(live, session, terminal_shot, terminal_shot_status, shots) {
            Ok(true) => return None,
            Err(error) => eprintln!("[panel] {error}"),
            Ok(false) => {}
        }
        emit_proof(false);
        eprintln!("FAIL: live {} {}", live.name, live_line());
        live.failed = Some(message.clone());
        return Some(message.clone());
    }
    if let CoreGate::Failed(message) = &ext_gate {
        match hold_script_terminal_shot(live, session, terminal_shot, terminal_shot_status, shots) {
            Ok(true) => return None,
            Err(error) => eprintln!("[panel] {error}"),
            Ok(false) => {}
        }
        emit_proof(false);
        eprintln!("FAIL: live {} {}", live.name, live_line());
        live.failed = Some(message.clone());
        return Some(message.clone());
    }
    let wait_script_stop = {
        let guard = session.scenario.lock().unwrap();
        guard.as_ref().and_then(|runner| runner.wait_script_stop())
    };
    match status {
        Some(scenario::RunnerStatus::Passed) => {
            if matches!(core_gate, CoreGate::Pending)
                || matches!(pair_gate, CoreGate::Pending)
                || matches!(ext_gate, CoreGate::Pending)
            {
                return None;
            }
            if let Some(needle) = wait_script_stop {
                if !session.script_self_stop_observed(needle) {
                    let started = session
                        .live_script_stop_wait_started
                        .get_or_insert_with(Instant::now);
                    if started.elapsed() >= SCRIPT_STOP_WAIT {
                        let message = format!(
                            "timed out waiting for script Idle and clean stop reason {needle:?}"
                        );
                        request_native_failure_capture(live, session, shots, terminal_shot);
                        match hold_script_terminal_shot(
                            live,
                            session,
                            terminal_shot,
                            terminal_shot_status,
                            shots,
                        ) {
                            Ok(true) => return None,
                            Err(error) => eprintln!("[panel] {error}"),
                            Ok(false) => {}
                        }
                        emit_proof(false);
                        eprintln!("FAIL: live {} {}", live.name, failure_line(&message));
                        live.failed = Some(message.clone());
                        return Some(message);
                    }
                    return None;
                }
                if let Err(error) = request_clean_stop_capture(live, session, shots, terminal_shot)
                {
                    live.failed = Some(error.clone());
                    return Some(error);
                }
            }
            if core_watch.as_ref().is_some_and(|watch| watch.configured())
                && matches!(core_gate, CoreGate::Qualified(_))
            {
                request_native_failure_capture(live, session, shots, terminal_shot);
            }
            match hold_script_terminal_shot(
                live,
                session,
                terminal_shot,
                terminal_shot_status,
                shots,
            ) {
                Ok(true) => return None,
                Err(error) => {
                    live.failed = Some(error.clone());
                    return Some(error);
                }
                Ok(false) => {}
            }
            let base = terminal_shot.unwrap_or(&live.name);
            if !live.announced_pass {
                emit_proof(true);
                println!("PASS: live {} {}", live.name, live_line());
                live.announced_pass = true;
                if live.soak {
                    let label = format!("{base}{SOAK_POSTPASS_SUFFIX}");
                    if let Some(shots) = shots {
                        enqueue_soak_capture(session, shots, &label);
                        live.soak_capture = SoakCapture::PostPass {
                            label,
                            started: None,
                        };
                        return None;
                    }
                }
            }
            if live.soak {
                if shots.is_none() {
                    if live
                        .soak_until
                        .is_some_and(|deadline| Instant::now() >= deadline)
                    {
                        live.passed = true;
                    }
                    return None;
                }
                match soak_capture_tick(live, session, shots.unwrap(), terminal_shot) {
                    Ok(true) => return None,
                    Ok(false) => live.passed = true,
                    Err(error) => {
                        live.failed = Some(error.clone());
                        return Some(error);
                    }
                }
                return None;
            }
            live.passed = true;
            None
        }
        Some(scenario::RunnerStatus::Failed(msg)) => {
            match hold_script_terminal_shot(
                live,
                session,
                terminal_shot,
                terminal_shot_status,
                shots,
            ) {
                Ok(true) => return None,
                Err(error) => eprintln!("[panel] {error}"),
                Ok(false) => {}
            }
            if let Some(core) = record_core() {
                eprintln!("CATALOG_CORE: {} {core}", live.name);
            }
            if let Some(pair) = record_pair() {
                eprintln!("PAIRED_CORE: {} {pair}", live.name);
            }
            if let Some(ext) = record_ext() {
                eprintln!("EXTERNAL_LOADER: {} {ext}", live.name);
            }
            eprintln!("FAIL: live {} {}", live.name, live_line());
            live.failed = Some(msg.clone());
            Some(msg)
        }
        Some(scenario::RunnerStatus::Seeding) => None,
        Some(scenario::RunnerStatus::Running { step, total }) => {
            if live.last_step != Some((step, total)) {
                live.last_step = Some((step, total));
                if step >= total {
                    println!("live {}: proving proof predicate", live.name);
                } else {
                    println!("live {}: running step {}/{}", live.name, step + 1, total);
                }
            }
            None
        }
        None => None,
    }
}

/// The focused slot's status row, `None` when nothing is focused or the
/// focused username has no row this pump.
pub(super) fn focused_slot<'a>(
    session: &Session,
    statuses: &'a [host_play::SlotStatus],
) -> Option<&'a host_play::SlotStatus> {
    let focused = session.focused_name();
    statuses
        .iter()
        .find(|s| focused.as_deref() == Some(s.username.as_str()))
}

/// Headed `--smoke` watch. `wrote_shots` is the count `pump_shots` wrote
/// this frame: the smoke's single scene2 shot landing passes the run (the
/// caller exits 0). Before that, the pure trigger latches scene 2 on the
/// focused slot for the render-settle gate and a precise deadline message,
/// and the shared runner's failures surface unchanged. The 300 s ceiling
/// is the run's own clock: it also catches a runner that passed but whose
/// PNG never got written (e.g. a failed render readback).
pub(super) fn live_smoke_tick(
    live: &mut LiveSmoke,
    session: &mut Session,
    statuses: &[host_play::SlotStatus],
    wrote_shots: usize,
) -> Option<String> {
    if live.passed || live.failed.is_some() {
        return None;
    }
    if wrote_shots > 0 {
        println!("PASS: panel-play --smoke (scene2 whole-window shot written)");
        live.passed = true;
        return None;
    }
    let slot = focused_slot(session, statuses);
    if smoke_should_fire(
        true,
        live.saw_scene2_at.is_some(),
        slot.is_some_and(|s| s.ingame),
        slot.map_or(0, |s| s.scene_state),
    ) {
        live.saw_scene2_at = Some(Instant::now());
        println!(
            "smoke: focused slot at scene 2; waiting for the render to settle before the capture"
        );
    }
    // Mirror the shared runner so a scenario failure is the failure.
    let status = session
        .scenario
        .lock()
        .unwrap()
        .as_ref()
        .map(|r| r.status());
    match status {
        Some(scenario::RunnerStatus::Failed(msg)) => {
            live.failed = Some(msg.clone());
            return Some(msg);
        }
        Some(scenario::RunnerStatus::Running { step, total })
            if live.last_step != Some((step, total)) =>
        {
            live.last_step = Some((step, total));
            println!("smoke: running step {step}/{total}");
        }
        _ => {}
    }
    if live.started.elapsed() >= SMOKE_DEADLINE {
        let msg = if live.saw_scene2_at.is_some() {
            "panel-play --smoke: scene2 shot never written within 300s".to_string()
        } else {
            "panel-play --smoke: focused slot never reached scene 2 within 300s".to_string()
        };
        live.failed = Some(msg.clone());
        return Some(msg);
    }
    None
}

/// Install the whole-window shot plumbing for a headed scenario run: the
/// per-run shot dir plus the sink bridging the slot-threaded runner to
/// the window readback — it enqueues `(label, snapshot JSON)`, and
/// `ui_frame` hands the requests to the render pass, then writes the PNG
/// and sidecar from the returned bytes. Shared by `--live script_*` and
/// `--smoke`.
pub(super) fn arm_scenario_shots(
    session: &mut Session,
    shots: Arc<Mutex<ShotState>>,
    shot_dir: &mut Option<PathBuf>,
) {
    *shot_dir = scenario::shot::create_run_dir()
        .map(Some)
        .unwrap_or_else(|e| {
            eprintln!("[panel] shot dir: {e}; shots will be skipped");
            None
        });
    if let Some(runner) = session.scenario.lock().unwrap().as_mut() {
        if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
            eprintln!(
                "[panel] scenario capture armed: {:?}",
                runner.terminal_shot()
            );
        }
        runner.set_shot_sink(Box::new(
            move |label: &str, snap: &api::snapshot::GameSnapshot| {
                if std::env::var("BOT_DEBUG").as_deref() == Ok("1") {
                    eprintln!("[panel] scenario capture requested: {label}");
                }
                match serde_json::to_string_pretty(snap) {
                    Ok(json) => shots.lock().unwrap().enqueue(label.to_string(), json),
                    Err(error) => {
                        eprintln!("[panel] shot {label}: snapshot serialization failed: {error}");
                        shots
                            .lock()
                            .unwrap()
                            .mark_failed(label, &format!("snapshot serialization failed: {error}"));
                    }
                }
            },
        ));
    }
}

impl LiveHarness {
    pub(super) fn tick(
        &mut self,
        session: &mut Session,
        statuses: &[host_play::SlotStatus],
        terminal_shot_status: &ShotStatus,
        shots: Option<&Mutex<ShotState>>,
        written_shots: usize,
    ) -> Option<String> {
        match self {
            Self::Null(n) => live_null_tick(n, statuses),
            Self::Stress(s) => live_stress_tick(s, statuses),
            Self::Script(ls) => live_script_tick(ls, session, terminal_shot_status, shots),
            Self::Smoke(s) => live_smoke_tick(s, session, statuses, written_shots),
        }
    }

    pub(super) fn holds_shot_promotion(&self, session: &Session, now: Instant) -> bool {
        match self {
            Self::Smoke(s) => !smoke_settled(
                s.saw_scene2_at,
                now,
                focused_slot(session, &session.statuses()).is_some_and(|slot| slot.ingame),
            ),
            _ => false,
        }
    }

    pub(super) fn exit_pass(&self) -> bool {
        matches!(self, Self::Smoke(s) if s.passed) || matches!(self, Self::Script(s) if s.passed)
    }
}
