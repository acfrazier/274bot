//! Process resource view shared by the panel rail, the main-panel resource
//! section, and the TUI status pane. One sampler; no second `sample_process`.

use std::io::{self, ErrorKind};
use std::path::PathBuf;
use std::time::Instant;

use crate::{sample_process, Play};

/// One resource row: still measuring, available with a compact label,
/// unavailable with the fixed reason, or a hard error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Metric {
    Measuring,
    Available(String),
    Unavailable(&'static str),
    Error(String),
}

/// Snapshot of host resources shown on the panel and TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceView {
    pub bots: usize,
    pub ingame: usize,
    /// Live slots that are not the focused profile.
    pub background: usize,
    pub cpu: Metric,
    pub ram: Metric,
    pub traffic: Metric,
}

impl Default for ResourceView {
    fn default() -> Self {
        Self {
            bots: 0,
            ingame: 0,
            background: 0,
            cpu: Metric::Measuring,
            ram: Metric::Measuring,
            traffic: Metric::Measuring,
        }
    }
}

/// One live worker lifetime at sample time. Terminal history rows are omitted
/// by the Play snapshot that produces these facts.
#[derive(Debug, Clone, Copy)]
pub struct LiveSlot<'a> {
    pub name: &'a str,
    pub ingame: bool,
    pub traffic_bytes: u64,
}

/// 1 Hz process + stream-byte sample. CPU needs a wall+CPU delta, so the
/// first sample is [`Metric::Measuring`]; RAM is available from the start.
/// Traffic needs two samples of summed `bytes_in+bytes_out`; zero slots stay
/// Measuring (never fake 0 B/s). A process-sampler failure flips CPU/RAM to
/// [`Metric::Error`] and re-baselines them, but traffic still samples from
/// live workers. The projected [`ResourceView`] is cached at the sample
/// boundary so UI frames can borrow it.
#[derive(Debug, Clone, Default)]
pub struct ResourceSampler {
    last_proc: Option<(Instant, f64)>,
    last_traffic: Option<(Instant, u64, usize)>,
    last_rss: u64,
    view: ResourceView,
}

impl ResourceSampler {
    /// Whether a new sample is due. Does not read slot statuses.
    pub fn due(&self, now: Instant) -> bool {
        match &self.last_proc {
            Some((t, _)) => now.duration_since(*t).as_secs_f64() >= 1.0,
            None => match &self.last_traffic {
                Some((t, ..)) => now.duration_since(*t).as_secs_f64() >= 1.0,
                None => true,
            },
        }
    }

    /// Project live workers into the cached view. Call only when [`Self::due`].
    pub fn sample<'a, I>(&mut self, now: Instant, focused: Option<&str>, live: I)
    where
        I: IntoIterator<Item = LiveSlot<'a>>,
    {
        let mut bots = 0usize;
        let mut ingame = 0usize;
        let mut background = 0usize;
        let mut sum = 0u64;
        for slot in live {
            bots += 1;
            if slot.ingame {
                ingame += 1;
            }
            if focused != Some(slot.name) {
                background += 1;
            }
            sum = sum.wrapping_add(slot.traffic_bytes);
        }
        self.finish(now, bots, ingame, background, sum);
    }

    /// Sample from Play's live worker lifetimes (arms), not status history.
    pub fn sample_play(&mut self, now: Instant, play: &Play, focused: Option<&str>) {
        let mut bots = 0usize;
        let mut ingame = 0usize;
        let mut background = 0usize;
        let mut sum = 0u64;
        play.for_each_live_slot(|slot| {
            bots += 1;
            if slot.ingame {
                ingame += 1;
            }
            if focused != Some(slot.name) {
                background += 1;
            }
            sum = sum.wrapping_add(slot.traffic_bytes);
        });
        self.finish(now, bots, ingame, background, sum);
    }

    fn finish(&mut self, now: Instant, bots: usize, ingame: usize, background: usize, sum: u64) {
        match self.last_traffic {
            Some((t0, sum0, n_prev)) => {
                let dt = now.duration_since(t0).as_secs_f64();
                self.view.traffic = traffic_from_samples(sum, sum0, dt, bots, n_prev);
            }
            None => self.view.traffic = Metric::Measuring,
        }
        self.last_traffic = Some((now, sum, bots));

        let (rss, cpu) = sample_process();
        self.last_rss = rss;
        if rss == 0 && cpu == 0.0 {
            self.view.cpu = Metric::Error("process sample failed".into());
            self.view.ram = Metric::Error("process sample failed".into());
            self.last_proc = None;
            self.view.bots = bots;
            self.view.ingame = ingame;
            self.view.background = background;
            return;
        }
        match self.last_proc {
            Some((t0, cpu0)) => {
                let wall = now.duration_since(t0).as_secs_f64();
                let ncpu = std::thread::available_parallelism()
                    .map(|n| n.get() as u32)
                    .unwrap_or(1)
                    .max(1);
                self.view.cpu = cpu_from_delta(cpu - cpu0, wall, ncpu);
            }
            None => self.view.cpu = Metric::Measuring,
        }
        self.view.ram = Metric::Available(format_rss_caption(rss));
        self.last_proc = Some((now, cpu));
        self.view.bots = bots;
        self.view.ingame = ingame;
        self.view.background = background;
    }

    pub fn view(&self) -> &ResourceView {
        &self.view
    }

    pub fn last_rss(&self) -> u64 {
        self.last_rss
    }
}

impl Play {
    /// Visit each worker lifetime that is still owned (`arm` present).
    /// Logged-out, queued, connecting, and disconnected workers count;
    /// terminal history rows and retired slots do not.
    pub fn for_each_live_slot(&self, mut visit: impl FnMut(LiveSlot<'_>)) {
        let statuses = crate::lock_statuses(&self.statuses);
        for name in self.arms.keys() {
            let row = statuses.iter().find(|status| &status.username == name);
            visit(LiveSlot {
                name: name.as_str(),
                ingame: row.is_some_and(|status| status.ingame),
                traffic_bytes: row
                    .map(|status| status.bytes_in.wrapping_add(status.bytes_out))
                    .unwrap_or(0),
            });
        }
    }

    pub fn background_bot_count(&self, focused: Option<&str>) -> usize {
        self.arms
            .keys()
            .filter(|name| focused != Some(name.as_str()))
            .count()
    }
}

/// Live slots that are not the focused profile.
pub fn background_bot_count<I, S>(live: I, focused: Option<&str>) -> usize
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    live.into_iter()
        .filter(|name| focused != Some(name.as_ref()))
        .count()
}

pub fn metric_text(metric: &Metric) -> &str {
    match metric {
        Metric::Measuring => "measuring…",
        Metric::Available(s) => s,
        Metric::Unavailable(r) => r,
        Metric::Error(e) => e,
    }
}

/// Rate from a byte-counter delta. No slots → Measuring (never fake 0 B/s).
/// Non-positive wall delta → Measuring.
pub fn traffic_from_delta(d_bytes: u64, dt_secs: f64, n_slots: usize) -> Metric {
    if n_slots == 0 || dt_secs <= 0.0 {
        return Metric::Measuring;
    }
    let bps = d_bytes as f64 / dt_secs;
    Metric::Available(format_bps(bps))
}

/// If `n_slots==0` or `dt<=0` → Measuring.
/// If `n_slots != n_prev` or `sum < sum0` → Measuring (re-baseline; do not wrapping_sub).
/// Else rate from `sum.wrapping_sub(sum0)` / dt.
pub fn traffic_from_samples(
    sum: u64,
    sum0: u64,
    dt_secs: f64,
    n_slots: usize,
    n_prev: usize,
) -> Metric {
    if n_slots == 0 || dt_secs <= 0.0 || n_slots != n_prev || sum < sum0 {
        return Metric::Measuring;
    }
    traffic_from_delta(sum.wrapping_sub(sum0), dt_secs, n_slots)
}

/// macos/windows: `format_rss(bytes) + " peak"` (sample_process first field is
/// lifetime peak WS / ru_maxrss); other: `format_rss(bytes)`.
pub fn format_rss_caption(bytes: u64) -> String {
    let base = format_rss(bytes);
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        format!("{base} peak")
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        base
    }
}

fn format_bps(bps: f64) -> String {
    let kb = 1024.0;
    let mb = kb * 1024.0;
    if bps < kb {
        format!("{bps:.0} B/s")
    } else if bps < mb {
        format!("{:.1} KB/s", bps / kb)
    } else {
        format!("{:.1} MB/s", bps / mb)
    }
}

/// `"{n} bots ({ingame} running)"`, singular `bot` when `n == 1`.
pub fn format_bots(n: usize, ingame: usize) -> String {
    let noun = if n == 1 { "bot" } else { "bots" };
    format!("{n} {noun} ({ingame} running)")
}

/// Background-bot count for the resource section.
pub fn format_background(n: usize) -> String {
    if n == 1 {
        "1 other".into()
    } else {
        format!("{n} others")
    }
}

/// CPU utilisation from CPU/wall delta times. `Measuring` when the wall
/// delta is not positive; else busy cores (`cpu / wall`) over `ncpu`.
pub fn cpu_from_delta(dt_cpu_secs: f64, dt_wall_secs: f64, ncpu: u32) -> Metric {
    if dt_wall_secs <= 0.0 {
        return Metric::Measuring;
    }
    let cores = dt_cpu_secs / dt_wall_secs;
    let pct = 100.0 * dt_cpu_secs / (dt_wall_secs * ncpu as f64);
    Metric::Available(format!("{cores:.1} cores ({pct:.0}% of {ncpu})"))
}

/// RSS in the nearest human unit: bytes under 1 KB, then KB, MB, GB.
pub fn format_rss(bytes: u64) -> String {
    let b = bytes as f64;
    let kb = 1024.0;
    let mb = kb * 1024.0;
    let gb = mb * 1024.0;
    if b < kb {
        format!("{b:.0} B")
    } else if b < mb {
        format!("{:.0} KB", b / kb)
    } else if b < gb {
        format!("{:.1} MB", b / mb)
    } else {
        format!("{:.2} GB", b / gb)
    }
}

/// One-line / dialog body: other profiles keep running, plus the live meter.
pub fn background_ack_text(others: usize, view: &ResourceView) -> String {
    format!(
        "Other profiles keep running in the background ({}). See the resource section for live cost: {} · cpu {} · ram {}.",
        format_background(others),
        format_bots(view.bots, view.ingame),
        metric_text(&view.cpu),
        metric_text(&view.ram),
    )
}

const BACKGROUND_BOTS_ACK_KEY: &str = "background_bots_ack";

/// `~/.274bot/panel-ui.json` — the shared panel/TUI prefs store.
pub fn panel_ui_path() -> PathBuf {
    script::bot_file("panel-ui.json")
}

pub fn background_bots_acked() -> bool {
    panel_ui_value(BACKGROUND_BOTS_ACK_KEY)
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

/// One top-level value of `panel-ui.json`; `None` when the file, the key or
/// a readable JSON object is absent.
pub fn panel_ui_value(key: &str) -> Option<serde_json::Value> {
    let data = std::fs::read(panel_ui_path()).ok()?;
    let mut value: serde_json::Value = serde_json::from_slice(&data).ok()?;
    value.as_object_mut()?.remove(key)
}

pub fn background_bots_ack_error(err: &io::Error) -> String {
    format!("background bots: {err}")
}

pub fn clear_background_bots_ack_error(error: &mut Option<String>) {
    if error
        .as_deref()
        .is_some_and(|msg| msg.starts_with("background bots:"))
    {
        *error = None;
    }
}

/// Set `background_bots_ack` in `panel-ui.json`, preserving other keys.
pub fn persist_background_bots_ack() -> io::Result<()> {
    persist_panel_ui_value(BACKGROUND_BOTS_ACK_KEY, serde_json::Value::Bool(true))
}

/// Set one top-level key of `panel-ui.json`, preserving every other key.
pub fn persist_panel_ui_value(key: &str, value: serde_json::Value) -> io::Result<()> {
    let path = panel_ui_path();
    let mut document = match std::fs::read(&path) {
        Ok(data) => serde_json::from_slice(&data).unwrap_or_else(|_| serde_json::json!({})),
        Err(e) if e.kind() == ErrorKind::NotFound => serde_json::json!({}),
        Err(e) => return Err(e),
    };
    let obj = document
        .as_object_mut()
        .ok_or_else(|| io::Error::new(ErrorKind::InvalidData, "panel-ui.json is not an object"))?;
    obj.insert(key.into(), value);
    let data = serde_json::to_vec_pretty(&document)
        .map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
    vault::write_private_file(&path, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{run_with_io, PlayOptions, SlotArm, SlotStatus, WorkerTerminal};
    use std::time::{Duration, Instant};

    fn empty_play() -> Play {
        run_with_io(
            &PlayOptions {
                host: "127.0.0.1".into(),
                port: 43594,
                cache_dir: "/tmp".into(),
                lowmem: true,
                mainland: false,
            },
            vec![],
            |_| (None, None),
            |_, _, _| {},
        )
    }

    #[test]
    fn background_count_skips_the_focused_name() {
        assert_eq!(background_bot_count(["alice", "bob"], Some("bob")), 1);
        assert_eq!(background_bot_count(["alice"], Some("alice")), 0);
        assert_eq!(background_bot_count(["alice", "bob"], None), 2);
    }

    #[test]
    fn due_does_not_require_statuses() {
        let sampler = ResourceSampler::default();
        assert!(sampler.due(Instant::now()));
    }

    #[test]
    fn sample_counts_only_the_live_iterator() {
        let mut sampler = ResourceSampler::default();
        let now = Instant::now();
        sampler.sample(
            now,
            Some("alice"),
            [
                LiveSlot {
                    name: "alice",
                    ingame: true,
                    traffic_bytes: 10,
                },
                LiveSlot {
                    name: "bob",
                    ingame: false,
                    traffic_bytes: 0,
                },
            ],
        );
        let view = sampler.view();
        assert_eq!(view.bots, 2);
        assert_eq!(view.ingame, 1);
        assert_eq!(view.background, 1);
        match &view.ram {
            Metric::Available(_) => assert_eq!(view.cpu, Metric::Measuring),
            Metric::Error(_) => {}
            other => panic!("ram should be sampled, got {other:?}"),
        }
        assert!(!sampler.due(now));
        sampler.sample(
            now + Duration::from_secs(1),
            Some("alice"),
            [LiveSlot {
                name: "alice",
                ingame: true,
                traffic_bytes: 10,
            }],
        );
        let later = sampler.view();
        assert_eq!(later.bots, 1);
        assert_eq!(later.background, 0);
    }

    #[test]
    fn play_live_slots_skip_terminal_and_retired_keep_logged_out() {
        let mut play = empty_play();
        play.attach_arm("alice", SlotArm::new(1, false));
        play.attach_arm("carol", SlotArm::new(3, false));
        play.statuses.lock().unwrap().extend([
            SlotStatus {
                username: "alice".into(),
                ingame: true,
                connected: true,
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "bob".into(),
                ingame: false,
                worker_terminal: Some(WorkerTerminal::Failed),
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "carol".into(),
                ingame: false,
                connected: false,
                login_latched: true,
                ..SlotStatus::default()
            },
            SlotStatus {
                username: "dave".into(),
                ingame: false,
                ..SlotStatus::default()
            },
        ]);
        let mut names = Vec::new();
        let mut ingame = 0;
        play.for_each_live_slot(|slot| {
            names.push(slot.name.to_string());
            if slot.ingame {
                ingame += 1;
            }
        });
        names.sort();
        assert_eq!(names, ["alice", "carol"]);
        assert_eq!(ingame, 1);
        assert_eq!(play.background_bot_count(Some("alice")), 1);
        let mut sampler = ResourceSampler::default();
        sampler.sample_play(Instant::now(), &play, Some("alice"));
        let view = sampler.view();
        assert_eq!(view.bots, 2);
        assert_eq!(view.ingame, 1);
        assert_eq!(view.background, 1);
    }

    #[test]
    fn persist_merges_ack_into_panel_ui_json() {
        let _iso = script::IsolatedEnv::enter("ack-json");
        let path = panel_ui_path();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, r#"{"last_focus":"alice"}"#).unwrap();
        assert!(!background_bots_acked());
        persist_background_bots_ack().unwrap();
        assert!(background_bots_acked());
        persist_background_bots_ack().unwrap();
        let v: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(v["last_focus"], "alice");
        assert_eq!(v["background_bots_ack"], true);
    }

    #[test]
    fn persist_fails_when_parent_is_a_file() {
        let _iso = script::IsolatedEnv::enter("ack-parent-file");
        let parent = panel_ui_path().parent().unwrap().to_path_buf();
        std::fs::write(&parent, b"not-a-dir").unwrap();
        assert!(persist_background_bots_ack().is_err());
        assert!(!background_bots_acked());
    }

    #[test]
    fn persist_fails_when_panel_ui_path_is_a_directory() {
        let _iso = script::IsolatedEnv::enter("ack-path-dir");
        let path = panel_ui_path();
        std::fs::create_dir_all(&path).unwrap();
        assert!(!background_bots_acked());
        assert!(persist_background_bots_ack().is_err());
        assert!(!background_bots_acked());
    }

    #[test]
    fn ack_text_names_the_meter() {
        let view = ResourceView {
            bots: 2,
            ingame: 1,
            background: 1,
            cpu: Metric::Measuring,
            ram: Metric::Available("12 MB peak".into()),
            traffic: Metric::Measuring,
        };
        let text = background_ack_text(1, &view);
        assert!(text.contains(&format_bots(2, 1)), "{text}");
        assert!(text.contains(metric_text(&view.ram)), "{text}");
        assert!(text.contains(&format_background(1)), "{text}");
    }
}
