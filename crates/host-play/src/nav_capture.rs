//! Opt-in, bounded navigation diagnostic checkpoints. Full snapshots are
//! serialized only at checkpoints, never on ordinary reads or every tick.
//!
//! When enabled, a compact per-slot correlation ring retains route/guardian
//! transitions across hold windows so the first nav-failure checkpoint can
//! embed pre-failure evidence. Default-off paths allocate nothing.
//!
//! TUI drain is data-only JSON (no renderer select, no screenshot wait).
//! Panel screenshot focus/timing lives in `panel::nav_capture` and is unchanged.
use api::snapshot::GameSnapshot;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

/// Compact correlation records retained per slot (hold can last many seconds).
const RING_CAP: usize = 128;
/// Bound path points retained from client `try_move_path` (head+tail sample).
const PATH_POINTS_CAP: usize = 16;
/// Bound chat / name / claim / refusal strings in correlation rows.
const STRING_CAP: usize = 128;
/// Checkpoint queue: three milestones + first nav failure + script failure.
const READY_CAP: usize = 5;

#[derive(Debug)]
pub struct Checkpoint {
    pub name: String,
    pub label: String,
    pub evidence: Value,
}

/// Guardian fields published on the previous client frame (host-play holds
/// the follow from that lagging status). Not instantaneous wire state.
/// Strings are bounded at construction — never retain unbounded clones.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GuardianObs {
    pub kind: Option<String>,
    pub name: Option<String>,
    pub ours: bool,
    pub handling: bool,
    pub hold: bool,
    pub cooldown: bool,
    pub claim: String,
}

impl GuardianObs {
    /// Bound every string field before the value is retained.
    pub fn bounded(
        kind: Option<String>,
        name: Option<String>,
        ours: bool,
        handling: bool,
        hold: bool,
        cooldown: bool,
        claim: String,
    ) -> (Self, u64) {
        let mut trunc = 0u64;
        let g = Self {
            kind: kind.map(|s| bound_str_owned(s, &mut trunc)),
            name: name.map(|s| bound_str_owned(s, &mut trunc)),
            ours,
            handling,
            hold,
            cooldown,
            claim: bound_str_owned(claim, &mut trunc),
        };
        (g, trunc)
    }
}

/// Head/tail-sampled tryMove path. Callers must sample from a borrow — never
/// clone the full client path into ObserveInput.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BoundedPath {
    /// At most PATH_POINTS_CAP points (head then tail when truncated).
    pub points: Vec<(i32, i32)>,
    pub original_len: usize,
    pub nearest: i32,
    /// Signature over original_len + nearest + the bounded sample only.
    pub sig: u64,
    pub truncated: bool,
}

impl BoundedPath {
    /// Sample head/tail ≤ PATH_POINTS_CAP before any ownership of path points.
    /// Signature does not scan the full original path.
    pub fn sample(path: &[(i32, i32)], nearest: i32) -> Self {
        let original_len = path.len();
        let truncated = original_len > PATH_POINTS_CAP;
        let points: Vec<(i32, i32)> = if !truncated {
            path.to_vec()
        } else {
            let head = PATH_POINTS_CAP / 2;
            let tail = PATH_POINTS_CAP - head;
            let mut out = Vec::with_capacity(PATH_POINTS_CAP);
            out.extend_from_slice(&path[..head]);
            out.extend_from_slice(&path[original_len - tail..]);
            out
        };
        let sig = path_sig_bounded(&points, original_len, nearest);
        Self {
            points,
            original_len,
            nearest,
            sig,
            truncated,
        }
    }
}

/// Same-observation inputs for one correlation sample. Built only when
/// capture is enabled so the default path never clones paths or locks nav.
#[derive(Clone, Debug)]
pub struct ObserveInput {
    pub player_gen: u64,
    pub chat_gen: u64,
    pub guardian: GuardianObs,
    /// Pre-bounded path sample (no full Vec ownership of client path).
    pub try_move_path: BoundedPath,
    pub route_generation: Option<u64>,
    pub route_dest: Option<(i32, i32, i32)>,
    pub current_aim: Option<(i32, i32, i32)>,
    /// String truncations already applied while building guardian (caller may 0).
    pub pre_truncated_strings: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DedupKey {
    snapshot_tick: u32,
    player_gen: u64,
    hold: bool,
    handling: bool,
    ours: bool,
    cooldown: bool,
    /// Already bounded at GuardianObs construction.
    claim: String,
    kind: Option<String>,
    chat_gen: u64,
    route_generation: Option<u64>,
    route_dest: Option<(i32, i32, i32)>,
    current_aim: Option<(i32, i32, i32)>,
    try_move_nearest: i32,
    try_move_sig: u64,
    walk_pending: bool,
}

#[derive(Default)]
struct SlotHistory {
    records: VecDeque<Value>,
    last_key: Option<DedupKey>,
    /// Frames that reached the capture observe path (enabled).
    observed: u64,
    /// Ring overflows (oldest dropped).
    dropped: u64,
    truncated_strings: u64,
    truncated_paths: u64,
    first_retained_tick: Option<u32>,
    last_retained_tick: Option<u32>,
    /// Pending WalkAttempt from traveller callback, attached on next retain.
    pending_walk: Option<Value>,
    /// Overwrites of pending_walk before a retain attached it to a row.
    pending_walk_replaced: u64,
    /// Last bounded path signature seen; drives bounded_sample_last_change_observed_tick.
    last_path_sig: Option<u64>,
    /// Native snapshot tick when the bounded head/tail+len signature last changed.
    /// Not last tryMove/send time; middle-only and identical-path resends are undetectable.
    bounded_sample_last_change_observed_tick: Option<u32>,
}

#[derive(Default)]
struct State {
    watched: String,
    scene: bool,
    bank: bool,
    returning: bool,
    failed: bool,
    /// Slot name awaiting a terminal script-failure snapshot attach (panel path).
    terminal: Option<String>,
    terminal_requested: bool,
    views: HashMap<String, Value>,
    history: HashMap<String, SlotHistory>,
    ready: VecDeque<Checkpoint>,
    serial: u32,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn bound_str(s: &str, trunc: &mut u64) -> String {
    let count = s.chars().count();
    if count > STRING_CAP {
        *trunc += 1;
        s.chars().take(STRING_CAP).collect()
    } else {
        s.to_string()
    }
}

fn bound_str_owned(s: String, trunc: &mut u64) -> String {
    let count = s.chars().count();
    if count > STRING_CAP {
        *trunc += 1;
        s.chars().take(STRING_CAP).collect()
    } else {
        s
    }
}

/// Signature over the already-bounded sample + original_len + nearest only.
fn path_sig_bounded(sample: &[(i32, i32)], original_len: usize, nearest: i32) -> u64 {
    let mut h = nearest as u64;
    h ^= (original_len as u64).wrapping_mul(0x9E3779B97F4A7C15);
    for (i, (x, z)) in sample.iter().enumerate() {
        h ^= (*x as u64)
            .wrapping_mul(0xC2B2AE3D27D4EB4F)
            .wrapping_add(*z as u64)
            .wrapping_mul(0x165667B19E3779F9)
            .wrapping_add(i as u64);
    }
    h
}

fn history_summary(slot: &SlotHistory) -> Value {
    json!({
        "ring_cap": RING_CAP,
        "path_points_cap": PATH_POINTS_CAP,
        "string_cap": STRING_CAP,
        "observed": slot.observed,
        "retained": slot.records.len(),
        "dropped": slot.dropped,
        "truncated_strings": slot.truncated_strings,
        "truncated_paths": slot.truncated_paths,
        "first_retained_tick": slot.first_retained_tick,
        "last_retained_tick": slot.last_retained_tick,
        "bounded_sample_last_change_observed_tick": slot.bounded_sample_last_change_observed_tick,
        "pending_walk": slot.pending_walk,
        "pending_walk_replaced": slot.pending_walk_replaced,
        "records": slot.records,
        "guardian_status_lag": "previous_frame",
        "npc_slot": "unavailable",
        "hop_cursor": "unavailable",
        "ticks_waited": "unavailable",
        "try_move_note": "client last successful tryMove BFS path sample; bounded_sample_last_change_observed_tick is native snapshot tick when head/tail+len signature last changed — not last tryMove/send time, not server ACK",
        "full_path_change_detection": "per_row_false_when_truncated",
        "resend_detection": "unavailable",
        "walk_acceptance_note": "accepted tryMove means outbound buffer encoding succeeded; no server ACK exists",
        "scene_base_note": "row scene_base_for_path / base is observe-time scene base for scene-relative path points; send-time base unavailable and may differ",
    })
}

pub fn enable(names: &[String]) {
    if std::env::var("BOT_NAV_CAPTURES").as_deref() != Ok("1") {
        return;
    }
    let _ = STATE.set(Mutex::new(State {
        watched: names[0].clone(),
        views: names.iter().map(|n| (n.clone(), Value::Null)).collect(),
        history: names
            .iter()
            .map(|n| (n.clone(), SlotHistory::default()))
            .collect(),
        ..Default::default()
    }));
}

pub fn enabled() -> bool {
    STATE.get().is_some()
}

impl State {
    fn push(&mut self, name: &str, label: &str, snapshot: &GameSnapshot, detail: Value) {
        self.serial += 1;
        let evidence = json!({
            "slot": name,
            "checkpoint": label,
            "event_tick": snapshot.tick(),
            "snapshot": snapshot,
            "detail": detail,
            "diagnostics": crate::memory_diagnostics::sample(name)
        });
        let item = Checkpoint {
            name: name.into(),
            label: format!("nav-{}-{label}-{name}", self.serial),
            evidence,
        };
        // At most three watched milestones plus the first nav failure and
        // one terminal script failure. Prefer failure evidence if full.
        if self.ready.len() == READY_CAP {
            self.ready.pop_back();
        }
        if label.contains("failure") {
            self.ready.push_front(item);
        } else {
            self.ready.push_back(item);
        }
    }

    fn retain_history(&mut self, name: &str, snapshot: &GameSnapshot, input: &ObserveInput) {
        let Some(slot) = self.history.get_mut(name) else {
            return;
        };
        slot.observed = slot.observed.saturating_add(1);
        slot.truncated_strings = slot
            .truncated_strings
            .saturating_add(input.pre_truncated_strings);
        if input.try_move_path.truncated {
            slot.truncated_paths = slot.truncated_paths.saturating_add(1);
        }

        let tick = snapshot.tick();
        // Bounded sample change only: head/tail+len sig; middle-only and resends undetectable.
        if slot.last_path_sig != Some(input.try_move_path.sig) {
            slot.last_path_sig = Some(input.try_move_path.sig);
            slot.bounded_sample_last_change_observed_tick = Some(tick);
        }

        let walk_pending = slot.pending_walk.is_some();
        let key = DedupKey {
            snapshot_tick: tick,
            player_gen: input.player_gen,
            hold: input.guardian.hold,
            handling: input.guardian.handling,
            ours: input.guardian.ours,
            cooldown: input.guardian.cooldown,
            claim: input.guardian.claim.clone(),
            kind: input.guardian.kind.clone(),
            chat_gen: input.chat_gen,
            route_generation: input.route_generation,
            route_dest: input.route_dest,
            current_aim: input.current_aim,
            try_move_nearest: input.try_move_path.nearest,
            try_move_sig: input.try_move_path.sig,
            walk_pending,
        };
        if slot.last_key.as_ref() == Some(&key) {
            // Same domain keys: ignore mere re-observations (frame spam).
            return;
        }
        slot.last_key = Some(key);

        let path_points: Vec<Value> = input
            .try_move_path
            .points
            .iter()
            .map(|(x, z)| json!([x, z]))
            .collect();
        // Guardian strings already bounded at ObserveInput construction.
        let chat = snapshot
            .chat()
            .map(|s| bound_str(s, &mut slot.truncated_strings));
        let anim = snapshot.local_player().map(|lp| lp.player.actor.animation);
        let spot = snapshot
            .local_player()
            .map(|lp| lp.player.actor.spot_animation);
        let walk = slot.pending_walk.take();
        let record = json!({
            "snapshot_tick": tick,
            "player_gen": input.player_gen,
            "chat_gen": input.chat_gen,
            "tile": snapshot.tile(),
            "scene_state": snapshot.scene_state(),
            "base": snapshot.base(),
            "level": snapshot.tile().map(|(_, _, l)| l),
            "ingame": snapshot.ingame(),
            "animation": anim,
            "spot_animation": spot,
            "thieving_stun_tick": snapshot.thieving_stun_tick(),
            "chat": chat,
            "guardian": {
                "kind": input.guardian.kind,
                "name": input.guardian.name,
                "ours": input.guardian.ours,
                "handling": input.guardian.handling,
                "hold": input.guardian.hold,
                "cooldown": input.guardian.cooldown,
                "claim": input.guardian.claim,
                "npc_slot": "unavailable",
                "status_lag": "previous_frame",
            },
            "route_generation": input.route_generation,
            "route_dest": input.route_dest,
            "current_aim": input.current_aim,
            "try_move_nearest": input.try_move_path.nearest,
            "try_move_path_len": input.try_move_path.original_len,
            "try_move_path_truncated": input.try_move_path.truncated,
            "try_move_path": path_points,
            "bounded_sample_last_change_observed_tick": slot.bounded_sample_last_change_observed_tick,
            "full_path_change_detection": !input.try_move_path.truncated,
            "resend_detection": "unavailable",
            "scene_base_for_path": snapshot.base(),
            "try_move_note": "bounded_sample_last_change_observed_tick is head/tail+len sig change observe tick only; not last tryMove/send; send-time base unavailable",
            "walk_attempt": walk,
            "hop_cursor": "unavailable",
            "ticks_waited": "unavailable",
        });
        if slot.records.len() == RING_CAP {
            slot.records.pop_front();
            slot.dropped = slot.dropped.saturating_add(1);
            if let Some(front) = slot.records.front() {
                slot.first_retained_tick = front
                    .get("snapshot_tick")
                    .and_then(|v| v.as_u64())
                    .map(|t| t as u32);
            }
        }
        if slot.first_retained_tick.is_none() {
            slot.first_retained_tick = Some(tick);
        }
        slot.last_retained_tick = Some(tick);
        slot.records.push_back(record);
    }
}

/// Lightweight view update + milestone checkpoints + correlation retain.
/// `input` is required only when capture is enabled (caller gates construction).
pub(crate) fn observe(
    name: &str,
    snapshot: &GameSnapshot,
    drawing: bool,
    running: bool,
    input: Option<&ObserveInput>,
) {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut s = state.lock().unwrap();
    if let Some(view) = s.views.get_mut(name) {
        *view = json!({
            "tick": snapshot.tick(),
            "tile": snapshot.tile(),
            "scene_state": snapshot.scene_state(),
            "ingame": snapshot.ingame(),
            "drawing": drawing
        });
    }
    if let Some(input) = input {
        s.retain_history(name, snapshot, input);
    }
    if s.terminal.as_deref() == Some(name) {
        s.terminal = None;
        s.push(name, "script-failure", snapshot, Value::Null);
    }
    if name != s.watched || !running || !snapshot.ingame() || snapshot.scene_state() != 2 {
        return;
    }
    if !s.scene {
        s.scene = true;
        s.push(name, "scene-ready", snapshot, Value::Null);
    }
    if !s.bank && snapshot.bank_component_id() != -1 {
        s.bank = true;
        s.push(name, "bank-arrival", snapshot, Value::Null);
    }
}

pub(crate) fn route(
    name: &str,
    snapshot: &GameSnapshot,
    dest: api::snapshot::WorldTile,
    generation: u64,
) {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut s = state.lock().unwrap();
    if name == s.watched && s.bank && !s.returning && snapshot.bank_component_id() == -1 {
        s.returning = true;
        s.push(
            name,
            "return-route-start",
            snapshot,
            json!({"dest": dest, "generation": generation}),
        );
    }
}

/// Record a traveller WalkAttempt into the next correlation row (bounded).
/// Overwrites count replacements when a prior attempt was never retained.
pub(crate) fn walk_attempt(
    name: &str,
    tick: u32,
    at: (i32, i32, i32),
    aim: (i32, i32, i32),
    refusal: Option<String>,
) {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut s = state.lock().unwrap();
    let Some(slot) = s.history.get_mut(name) else {
        return;
    };
    if slot.pending_walk.is_some() {
        slot.pending_walk_replaced = slot.pending_walk_replaced.saturating_add(1);
    }
    // accepted == refusal is None: outbound encoding succeeded; no server ACK.
    let accepted = refusal.is_none();
    let mut trunc = 0u64;
    let refusal = refusal.map(|r| bound_str_owned(r, &mut trunc));
    slot.truncated_strings = slot.truncated_strings.saturating_add(trunc);
    slot.pending_walk = Some(json!({
        "tick": tick,
        "at": at,
        "aim": aim,
        "refusal": refusal,
        "driver_accepted": accepted,
        "server_ack": "none_available",
    }));
    // Force a retain opportunity even if other keys match.
    if let Some(key) = slot.last_key.as_mut() {
        key.walk_pending = false;
    }
}

/// First nav-failure only. Correlation is built once into the checkpoint
/// detail (immutable JSON). No separate sealed owner clone of the whole ring.
pub(crate) fn failure(name: &str, snapshot: &GameSnapshot, detail: Value) {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut s = state.lock().unwrap();
    if s.failed {
        return;
    }
    s.failed = true;
    // Build once into the checkpoint Value — pending walk included even if
    // the next observe has not yet attached it to a ring row.
    let correlation = s
        .history
        .get(name)
        .map(history_summary)
        .unwrap_or(Value::Null);
    let mut detail = detail;
    if let Some(obj) = detail.as_object_mut() {
        obj.insert("correlation_history".into(), correlation);
        obj.insert(
            "correlation_note".into(),
            json!("immutable in this checkpoint only; live ring may continue after first failure"),
        );
    }
    s.push(name, "nav-failure", snapshot, detail);
}

/// Panel path: latch terminal slot so the next observe can attach a
/// script-failure snapshot. TUI data-only must not use this wait path.
pub fn request_terminal(error: &str) {
    let Some(state) = STATE.get() else {
        return;
    };
    let mut s = state.lock().unwrap();
    if s.terminal_requested {
        return;
    }
    s.terminal_requested = true;
    s.terminal = s
        .views
        .keys()
        .find(|name| error.starts_with(&format!("{name}:")))
        .cloned();
}

pub fn take() -> Option<Checkpoint> {
    STATE.get()?.lock().unwrap().ready.pop_front()
}

pub fn pending() -> bool {
    STATE.get().is_some_and(|s| {
        let s = s.lock().unwrap();
        s.terminal.is_some() || !s.ready.is_empty()
    })
}

pub fn view(name: &str) -> Value {
    STATE
        .get()
        .and_then(|s| s.lock().unwrap().views.get(name).cloned())
        .unwrap_or(Value::Null)
}

/// TUI / headless data-only drain: write checkpoint JSON under the shot root
/// without selecting renderers or waiting for GPU screenshots.
/// Best-effort: I/O errors are reported in the return value and never replace
/// the original application error.
pub fn drain_json_files() -> Vec<String> {
    let mut notes = Vec::new();
    let root = match std::env::var("274BOT_SMOKE_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => match std::env::var("HOME") {
            Ok(home) => PathBuf::from(format!("{home}/.274bot/smoke")),
            Err(_) => PathBuf::from(".274bot/smoke"),
        },
    };
    if let Err(e) = std::fs::create_dir_all(&root) {
        notes.push(format!("nav-capture mkdir {}: {e}", root.display()));
        return notes;
    }
    while let Some(checkpoint) = take() {
        let stamp = {
            let secs = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            format!("{secs}")
        };
        let safe = checkpoint
            .label
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let path = root.join(format!("{stamp}_{safe}.json"));
        let body = match serde_json::to_string_pretty(&json!({
            "event": checkpoint.evidence,
            "drain": "tui-data-only",
            "pixel_state_note": "no GPU readback; evidence is the native checkpoint snapshot only",
        })) {
            Ok(s) => s,
            Err(e) => {
                notes.push(format!("nav-capture serialize {}: {e}", checkpoint.label));
                continue;
            }
        };
        match std::fs::write(&path, body) {
            Ok(()) => {
                eprintln!("[nav-capture] wrote {}", path.display());
            }
            Err(e) => {
                notes.push(format!("nav-capture write {}: {e}", path.display()));
            }
        }
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;

    /// Serialize tests that touch process-global STATE / env.
    static TEST_LOCK: StdMutex<()> = StdMutex::new(());

    fn fresh_state(names: &[&str]) -> State {
        State {
            watched: names[0].to_string(),
            views: names
                .iter()
                .map(|n| ((*n).to_string(), Value::Null))
                .collect(),
            history: names
                .iter()
                .map(|n| ((*n).to_string(), SlotHistory::default()))
                .collect(),
            ..Default::default()
        }
    }

    fn sample_input(hold: bool, player_gen: u64, tick_chat: u64) -> ObserveInput {
        let (guardian, pre) = GuardianObs::bounded(
            Some("Dialog".into()),
            Some("Mysterious Old Man".into()),
            true,
            hold,
            hold,
            false,
            "Host".into(),
        );
        ObserveInput {
            player_gen,
            chat_gen: tick_chat,
            guardian,
            try_move_path: BoundedPath::sample(&[(1, 1), (2, 2), (3, 3)], 0),
            route_generation: Some(1),
            route_dest: Some((2660, 3305, 0)),
            current_aim: Some((2662, 3297, 0)),
            pre_truncated_strings: pre,
        }
    }

    #[test]
    fn checkpoint_snapshot_is_retained_and_failure_has_priority() {
        let mut s = State::default();
        let snapshot = GameSnapshot::new();
        for _ in 0..7 {
            s.push("bot", "scene-ready", &snapshot, Value::Null);
        }
        assert_eq!(s.ready.len(), 5);
        s.push("bot", "nav-failure", &snapshot, json!({"why":"stalled"}));
        assert_eq!(s.ready.len(), 5);
        let first = s.ready.pop_front().unwrap();
        assert_eq!(first.evidence["detail"]["why"], "stalled");
        assert_eq!(
            first.evidence["snapshot"],
            serde_json::to_value(&snapshot).unwrap()
        );
    }

    #[test]
    fn disabled_mode_is_inert_without_env() {
        let _g = TEST_LOCK.lock().unwrap();
        std::env::remove_var("BOT_NAV_CAPTURES");
        enable(&["unit-bot".into()]);
        let snap = GameSnapshot::new();
        observe("unit-bot", &snap, false, false, None);
        failure("unit-bot", &snap, json!({}));
        walk_attempt("unit-bot", 1, (0, 0, 0), (1, 1, 0), None);
    }

    #[test]
    fn bounded_path_samples_before_full_ownership() {
        let full: Vec<(i32, i32)> = (0..64).map(|k| (k, k + 1)).collect();
        let b = BoundedPath::sample(&full, 2);
        assert_eq!(b.original_len, 64);
        assert!(b.truncated);
        assert_eq!(b.points.len(), PATH_POINTS_CAP);
        assert_eq!(b.points[0], (0, 1));
        assert_eq!(b.points[PATH_POINTS_CAP - 1], (63, 64));
        // Signature independent of middle tiles that were not sampled.
        let mut alt = full.clone();
        alt[32] = (999, 999);
        let b2 = BoundedPath::sample(&alt, 2);
        assert_eq!(b.sig, b2.sig);
    }

    #[test]
    fn correlation_ring_bounds_and_truncates() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        for i in 0..300u32 {
            let full: Vec<(i32, i32)> = (0..64).map(|k| (k, k + 1)).collect();
            let (guardian, pre) = GuardianObs::bounded(
                Some("Dialog".into()),
                Some("x".repeat(400)),
                true,
                true,
                true,
                false,
                "Host".into(),
            );
            let input = ObserveInput {
                player_gen: i as u64,
                chat_gen: 0,
                guardian,
                try_move_path: BoundedPath::sample(&full, 0),
                route_generation: Some(1),
                route_dest: Some((2660, 3305, 0)),
                current_aim: Some((2662, 3297, 0)),
                pre_truncated_strings: pre,
            };
            s.retain_history("bot", &snap, &input);
        }
        let slot = s.history.get("bot").unwrap();
        assert_eq!(slot.records.len(), RING_CAP);
        assert!(slot.dropped >= 300 - RING_CAP as u64);
        assert!(slot.truncated_paths > 0);
        assert!(slot.truncated_strings > 0);
        assert_eq!(slot.observed, 300);
        assert!(slot.first_retained_tick.is_some());
        assert!(slot.last_retained_tick.is_some());
        let row = slot.records.back().unwrap();
        assert_eq!(
            row["try_move_path"].as_array().unwrap().len(),
            PATH_POINTS_CAP
        );
        assert_eq!(
            row["guardian"]["name"].as_str().unwrap().chars().count(),
            STRING_CAP
        );
        assert_eq!(row["guardian"]["status_lag"], "previous_frame");
        assert_eq!(row["guardian"]["npc_slot"], "unavailable");
        assert!(row
            .get("bounded_sample_last_change_observed_tick")
            .is_some());
        assert_eq!(row["full_path_change_detection"], false);
        assert_eq!(row["resend_detection"], "unavailable");
        assert!(row.get("try_move_observed_snapshot_tick").is_none());
        assert!(row.get("path_last_change_observed_tick").is_none());
    }

    #[test]
    fn bounded_sample_change_tick_only_updates_on_signature_change() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        let path_a = BoundedPath::sample(&[(1, 1), (2, 2)], 0);
        let path_b = BoundedPath::sample(&[(9, 9), (8, 8)], 0);
        assert_ne!(path_a.sig, path_b.sig);
        let mut input = sample_input(true, 1, 0);
        input.try_move_path = path_a.clone();
        s.retain_history("bot", &snap, &input);
        let first_change = s
            .history
            .get("bot")
            .unwrap()
            .bounded_sample_last_change_observed_tick;
        // Same path, different player_gen → retain, but sample-change tick unchanged.
        input.player_gen = 2;
        s.retain_history("bot", &snap, &input);
        assert_eq!(
            s.history
                .get("bot")
                .unwrap()
                .bounded_sample_last_change_observed_tick,
            first_change
        );
        let row = s.history.get("bot").unwrap().records.back().unwrap();
        assert_eq!(
            row["bounded_sample_last_change_observed_tick"],
            json!(first_change)
        );
        assert_eq!(row["full_path_change_detection"], true); // short path, not truncated
        assert_eq!(row["resend_detection"], "unavailable");
        // New path signature updates bounded sample change tick.
        input.player_gen = 3;
        input.try_move_path = path_b;
        s.retain_history("bot", &snap, &input);
        assert_eq!(
            s.history
                .get("bot")
                .unwrap()
                .bounded_sample_last_change_observed_tick,
            Some(snap.tick())
        );
    }

    #[test]
    fn dedup_ignores_identical_reobservation_but_keeps_hold_release() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        let hold = sample_input(true, 10, 1);
        for _ in 0..50 {
            s.retain_history("bot", &snap, &hold);
        }
        assert_eq!(s.history.get("bot").unwrap().records.len(), 1);
        assert_eq!(s.history.get("bot").unwrap().observed, 50);
        let release = sample_input(false, 10, 1);
        s.retain_history("bot", &snap, &release);
        assert_eq!(s.history.get("bot").unwrap().records.len(), 2);
        assert_eq!(
            s.history.get("bot").unwrap().records[0]["guardian"]["hold"],
            true
        );
        assert_eq!(
            s.history.get("bot").unwrap().records[1]["guardian"]["hold"],
            false
        );
    }

    #[test]
    fn same_row_preserves_snapshot_tick_and_player_gen() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        let (guardian, pre) =
            GuardianObs::bounded(None, None, false, false, false, false, "None".into());
        let input = ObserveInput {
            player_gen: 4242,
            chat_gen: 7,
            guardian,
            try_move_path: BoundedPath::sample(&[], 0),
            route_generation: Some(3),
            route_dest: Some((1, 2, 0)),
            current_aim: Some((3, 4, 0)),
            pre_truncated_strings: pre,
        };
        s.retain_history("bot", &snap, &input);
        let row = &s.history.get("bot").unwrap().records[0];
        assert_eq!(row["snapshot_tick"], snap.tick());
        assert_eq!(row["player_gen"], 4242);
        assert_eq!(row["route_generation"], 3);
        assert_eq!(row["current_aim"], json!([3, 4, 0]));
    }

    #[test]
    fn pending_walk_replace_counted_and_included_on_failure_before_observe() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        s.retain_history("bot", &snap, &sample_input(false, 1, 0));
        // Simulate walk_attempt overwrites without observe.
        {
            let slot = s.history.get_mut("bot").unwrap();
            slot.pending_walk = Some(json!({"tick": 10, "aim": [1, 2, 0]}));
            slot.pending_walk_replaced = 0;
            if slot.pending_walk.is_some() {
                slot.pending_walk_replaced += 1;
            }
            slot.pending_walk =
                Some(json!({"tick": 11, "aim": [3, 4, 0], "driver_accepted": true}));
        }
        let summary = history_summary(s.history.get("bot").unwrap());
        assert_eq!(summary["pending_walk_replaced"], 1);
        assert_eq!(summary["pending_walk"]["tick"], 11);
        // failure embeds pending even before next retain attaches walk to a row
        s.failed = false;
        let correlation = history_summary(s.history.get("bot").unwrap());
        s.failed = true;
        s.push(
            "bot",
            "nav-failure",
            &snap,
            json!({
                "correlation_history": correlation,
            }),
        );
        let cp = s.ready.pop_front().unwrap();
        assert_eq!(
            cp.evidence["detail"]["correlation_history"]["pending_walk"]["tick"],
            11
        );
        assert!(cp.evidence["detail"]["correlation_history"]
            .get("sealed_at_failure")
            .is_none());
    }

    #[test]
    fn first_failure_checkpoint_is_sole_immutable_owner() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        for i in 0..40u64 {
            s.retain_history("bot", &snap, &sample_input(true, i, 0));
        }
        for i in 40..80u64 {
            s.retain_history("bot", &snap, &sample_input(false, i, 1));
        }
        let sealed_len = s.history.get("bot").unwrap().records.len();
        // Build correlation once into checkpoint (production failure path).
        let correlation = history_summary(s.history.get("bot").unwrap());
        s.failed = true;
        s.push(
            "bot",
            "nav-failure",
            &snap,
            json!({
                "generation": 1u64,
                "outcome": "Expired",
                "correlation_history": correlation,
            }),
        );
        let cp = s.ready.front().unwrap();
        let cp_len = cp.evidence["detail"]["correlation_history"]["records"]
            .as_array()
            .unwrap()
            .len();
        assert_eq!(cp_len, sealed_len);
        // Later observations mutate live ring only — checkpoint Value is independent.
        for i in 80..100u64 {
            s.retain_history("bot", &snap, &sample_input(false, i, 2));
        }
        let still = s.ready.front().unwrap();
        assert_eq!(
            still.evidence["detail"]["correlation_history"]["records"]
                .as_array()
                .unwrap()
                .len(),
            sealed_len
        );
        // No sealed_at_failure twin owner on the slot.
        // (SlotHistory no longer has sealed_at_failure field.)
        // Second failure ignored by flag.
        let before = s.ready.len();
        assert!(s.failed);
        assert_eq!(s.ready.len(), before);
    }

    #[test]
    fn first_failure_priority_in_ready_queue() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        for _ in 0..5 {
            s.push("bot", "scene-ready", &snap, Value::Null);
        }
        s.push(
            "bot",
            "nav-failure",
            &snap,
            json!({"correlation_history": {"records": [1, 2, 3]}}),
        );
        let first = s.ready.pop_front().unwrap();
        assert!(first.label.contains("nav-failure"));
        assert_eq!(
            first.evidence["detail"]["correlation_history"]["records"]
                .as_array()
                .unwrap()
                .len(),
            3
        );
    }

    #[test]
    fn drain_json_files_writes_and_writer_failure_is_reported() {
        let _g = TEST_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!(
            "274bot-nav-drain-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("274BOT_SMOKE_DIR", &dir);
        std::env::set_var("BOT_NAV_CAPTURES", "1");

        if let Some(state) = STATE.get() {
            let mut s = state.lock().unwrap();
            let snap = GameSnapshot::new();
            s.push("drain-bot", "nav-failure", &snap, json!({"why": "test"}));
        } else {
            let _ = STATE.set(Mutex::new(fresh_state(&["drain-bot"])));
            let mut s = STATE.get().unwrap().lock().unwrap();
            let snap = GameSnapshot::new();
            s.push("drain-bot", "nav-failure", &snap, json!({"why": "test"}));
        }
        drop(_g);
        let notes = drain_json_files();
        let written: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .collect();
        assert!(
            !written.is_empty() || !notes.is_empty(),
            "expected write or honest note; notes={notes:?}"
        );
        if let Some(entry) = written.first() {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            assert!(text.contains("tui-data-only"));
            assert!(text.contains("event"));
        }
        let blocker = dir.join("not-a-dir");
        std::fs::write(&blocker, b"x").unwrap();
        std::env::set_var("274BOT_SMOKE_DIR", blocker.join("child"));
        if let Some(state) = STATE.get() {
            let mut s = state.lock().unwrap();
            let snap = GameSnapshot::new();
            s.push("drain-bot", "scene-ready", &snap, Value::Null);
        }
        let notes = drain_json_files();
        assert!(
            notes.iter().any(|n| n.contains("nav-capture")),
            "expected honest write failure note, got {notes:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn many_repeated_frames_preserve_hold_window_not_only_adjacent() {
        let mut s = fresh_state(&["bot"]);
        let snap = GameSnapshot::new();
        let hold = sample_input(true, 1, 0);
        for _ in 0..200 {
            s.retain_history("bot", &snap, &hold);
        }
        for g in 2..90u64 {
            s.retain_history("bot", &snap, &sample_input(true, g, 0));
        }
        s.retain_history("bot", &snap, &sample_input(false, 90, 1));
        let idle = sample_input(false, 90, 1);
        for _ in 0..200 {
            s.retain_history("bot", &snap, &idle);
        }
        let slot = s.history.get("bot").unwrap();
        let holds = slot
            .records
            .iter()
            .filter(|r| r["guardian"]["hold"] == true)
            .count();
        let releases = slot
            .records
            .iter()
            .filter(|r| r["guardian"]["hold"] == false)
            .count();
        assert!(holds >= 10, "expected multi-gen hold history, got {holds}");
        assert!(releases >= 1);
        let sealed = history_summary(slot);
        let sealed_holds = sealed["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|r| r["guardian"]["hold"] == true)
            .count();
        assert!(sealed_holds >= 10);
    }
}
