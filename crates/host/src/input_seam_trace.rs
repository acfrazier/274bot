//! Bounded Left/Right arrow input-delivery seam tracer.
//!
//! Gate: `BOT_DEBUG=1` (or host `set_debug(true)`). Independently removable:
//! delete this module and its call sites.
//!
//! Disabled path: one cached opt-in check only — no timestamps, allocations,
//! locks, IO, or evaluation-order changes beyond that check.
//!
//! Enabled path: strictly bounded per-stage stderr records (`[input-seam] …`),
//! with per-stage emit/suppress counters. Not a performance measurement.
//! Stages are independent counts; do not assume 1:1 event correlation across
//! stages unless a later controlled run proves it.
//!
//! Output contract (one line, whitespace-separated `k=v`):
//! `[input-seam] seq=<u64> mono_ns=<u64> stage=<tag> …`
//! Stage tags: `win_arrow`, `imgui_arrow`, `gate`, `stream`, `drain`,
//! `metric_start`, `gen_bind`, `present`, `saturate`.
//! Caps: arrow/action stages `STAGE_CAP` (48); gate `GATE_CAP` (24) with at most
//! `GATE_INIT_RESERVE` (8) non-transition initial samples; bind/present zero-work
//! heartbeats `HEARTBEAT_CAP` (4) so warmup cannot exhaust actionable capacity.

use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Instant;

/// Max emitted records per ordinary stage before further notes only suppress.
pub const STAGE_CAP: u32 = 48;
/// Max gate samples (initial + transitions).
pub const GATE_CAP: u32 = 24;
/// Max non-transition gate samples (remaining capacity reserved for transitions).
pub const GATE_INIT_RESERVE: u32 = 8;
/// Max bind/present lines with no pending work (warmup heartbeat).
pub const HEARTBEAT_CAP: u32 = 4;

const ENV_UNKNOWN: u8 = 0;
const ENV_OFF: u8 = 1;
const ENV_ON: u8 = 2;

static ENV_BOT_DEBUG: AtomicU8 = AtomicU8::new(ENV_UNKNOWN);
/// Test override: when set, forces enabled on/off without env.
#[cfg(test)]
static FORCE: AtomicU8 = AtomicU8::new(ENV_UNKNOWN);
static SEQ: AtomicU64 = AtomicU64::new(0);
static MONO_ORIGIN: OnceLock<Instant> = OnceLock::new();

macro_rules! stage_counters {
    ($($name:ident),* $(,)?) => {
        $(
            static $name: (AtomicU32, AtomicU32) = (AtomicU32::new(0), AtomicU32::new(0));
        )*
    };
}

stage_counters!(
    WIN_ARROW,
    IMGUI_ARROW,
    GATE,
    STREAM,
    DRAIN,
    METRIC_START,
    GEN_BIND,
    PRESENT,
    SATURATE,
    GEN_BIND_HB,
    PRESENT_HB,
);

/// Stage tags in stderr `stage=` field (stable parse contract).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    WinArrow,
    ImguiArrow,
    Gate,
    Stream,
    Drain,
    MetricStart,
    GenBind,
    Present,
    Saturate,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WinArrow => "win_arrow",
            Self::ImguiArrow => "imgui_arrow",
            Self::Gate => "gate",
            Self::Stream => "stream",
            Self::Drain => "drain",
            Self::MetricStart => "metric_start",
            Self::GenBind => "gen_bind",
            Self::Present => "present",
            Self::Saturate => "saturate",
        }
    }

    fn counters(self) -> &'static (AtomicU32, AtomicU32) {
        match self {
            Self::WinArrow => &WIN_ARROW,
            Self::ImguiArrow => &IMGUI_ARROW,
            Self::Gate => &GATE,
            Self::Stream => &STREAM,
            Self::Drain => &DRAIN,
            Self::MetricStart => &METRIC_START,
            Self::GenBind => &GEN_BIND,
            Self::Present => &PRESENT,
            Self::Saturate => &SATURATE,
        }
    }

    fn cap(self) -> u32 {
        match self {
            Self::Gate => GATE_CAP,
            Self::Saturate => STAGE_CAP,
            _ => STAGE_CAP,
        }
    }
}

/// Left/Right only (GameShell ch 1 / 2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArrowKey {
    Left,
    Right,
}

impl ArrowKey {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Left => "left",
            Self::Right => "right",
        }
    }

    pub fn from_ch(ch: i32) -> Option<Self> {
        match ch {
            1 => Some(Self::Left),
            2 => Some(Self::Right),
            _ => None,
        }
    }
}

/// Cached opt-in: `BOT_DEBUG=1` env (once) or host debug latch.
#[inline]
pub fn enabled() -> bool {
    #[cfg(test)]
    {
        match FORCE.load(Relaxed) {
            ENV_ON => return true,
            ENV_OFF => return false,
            _ => {}
        }
    }
    if crate::debug_flag() {
        return true;
    }
    match ENV_BOT_DEBUG.load(Relaxed) {
        ENV_ON => true,
        ENV_OFF => false,
        _ => {
            let on = std::env::var_os("BOT_DEBUG")
                .map(|v| v == "1")
                .unwrap_or(false);
            ENV_BOT_DEBUG.store(if on { ENV_ON } else { ENV_OFF }, Relaxed);
            on
        }
    }
}

/// Emit counts `(emitted, suppressed)` for a stage (test/report).
pub fn stage_counts(stage: Stage) -> (u32, u32) {
    let (e, s) = stage.counters();
    (e.load(Relaxed), s.load(Relaxed))
}

/// Shared test mutex so producer-path tests outside this module serialize
/// against the module's own unit tests (global FORCE/counters).
#[cfg(test)]
pub fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    tests::TEST_LOCK
        .lock()
        .unwrap_or_else(|e| e.into_inner())
}

/// Reset counters and force latch (unit tests only).
#[cfg(test)]
pub fn reset_for_test() {
    FORCE.store(ENV_UNKNOWN, Relaxed);
    SEQ.store(0, Relaxed);
    for st in [
        Stage::WinArrow,
        Stage::ImguiArrow,
        Stage::Gate,
        Stage::Stream,
        Stage::Drain,
        Stage::MetricStart,
        Stage::GenBind,
        Stage::Present,
        Stage::Saturate,
    ] {
        let (e, s) = st.counters();
        e.store(0, Relaxed);
        s.store(0, Relaxed);
    }
    GEN_BIND_HB.0.store(0, Relaxed);
    GEN_BIND_HB.1.store(0, Relaxed);
    PRESENT_HB.0.store(0, Relaxed);
    PRESENT_HB.1.store(0, Relaxed);
    LAST_GATE_BITS.store(0xFFFF_FFFF, Relaxed);
    GATE_INIT_LEFT.store(GATE_INIT_RESERVE, Relaxed);
}

/// Force enable/disable for unit tests (does not touch process env).
#[cfg(test)]
pub fn force_for_test(on: bool) {
    FORCE.store(if on { ENV_ON } else { ENV_OFF }, Relaxed);
}

fn mono_ns() -> u64 {
    let origin = MONO_ORIGIN.get_or_init(Instant::now);
    Instant::now()
        .saturating_duration_since(*origin)
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64
}

/// Take an emit slot; on saturation increment suppress and maybe one saturate line.
fn try_emit(stage: Stage) -> Option<u64> {
    let (emitted, suppressed) = stage.counters();
    let cap = stage.cap();
    let prev = emitted.fetch_add(1, Relaxed);
    if prev >= cap {
        emitted.fetch_sub(1, Relaxed);
        let sup = suppressed.fetch_add(1, Relaxed);
        if sup == 0 && stage != Stage::Saturate {
            note_saturate(stage, cap, 1);
        }
        return None;
    }
    Some(SEQ.fetch_add(1, Relaxed) + 1)
}

fn try_emit_hb(emitted: &AtomicU32, suppressed: &AtomicU32) -> Option<u64> {
    let prev = emitted.fetch_add(1, Relaxed);
    if prev >= HEARTBEAT_CAP {
        emitted.fetch_sub(1, Relaxed);
        suppressed.fetch_add(1, Relaxed);
        return None;
    }
    Some(SEQ.fetch_add(1, Relaxed) + 1)
}

fn note_saturate(of: Stage, emitted: u32, suppressed: u32) {
    let Some(seq) = try_emit(Stage::Saturate) else {
        return;
    };
    let mono = mono_ns();
    eprintln!(
        "[input-seam] seq={seq} mono_ns={mono} stage=saturate of={} emitted={emitted} suppressed={suppressed}",
        of.as_str()
    );
}

/// Emit only after a successful `try_emit` so saturation avoids body work.
fn emit_with(stage: Stage, body: impl FnOnce() -> String) {
    let Some(seq) = try_emit(stage) else {
        return;
    };
    let mono = mono_ns();
    let body = body();
    eprintln!(
        "[input-seam] seq={seq} mono_ns={mono} stage={} {body}",
        stage.as_str()
    );
}

// --- Stage notes (no-op when disabled) ---

/// `WindowEvent::KeyboardInput` Left/Right before platform forwarding.
pub fn note_window_arrow(key: ArrowKey, down: bool, repeat: bool) {
    if !enabled() {
        return;
    }
    emit_with(Stage::WinArrow, || {
        format!(
            "key={} down={} repeat={}",
            key.as_str(),
            u8::from(down),
            u8::from(repeat)
        )
    });
}

/// ImGui Left/Right edge (observed even when GameImage gate fails).
/// `edge`: press | release (hold optional).
pub fn note_imgui_arrow(key: ArrowKey, down: bool, edge: &str) {
    if !enabled() {
        return;
    }
    emit_with(Stage::ImguiArrow, || {
        format!(
            "key={} down={} edge={edge}",
            key.as_str(),
            u8::from(down)
        )
    });
}

/// Packed gate bits for transition detection.
/// Bits: draw, capture, hover, slot_focused, win_focused, pane, capture_tx.
static LAST_GATE_BITS: AtomicU32 = AtomicU32::new(0xFFFF_FFFF);
static GATE_INIT_LEFT: AtomicU32 = AtomicU32::new(GATE_INIT_RESERVE);

/// Bounded gate sample: first observation, up to [`GATE_INIT_RESERVE`] initials,
/// then transitions only (remaining [`GATE_CAP`] slots).
///
/// `slot_focused`: selected host slot for this GameImage path.
/// `win_focused`: ImGui window keyboard focus (separate; do not conflate).
pub fn note_gate(
    draw: bool,
    capture: bool,
    hover: bool,
    slot_focused: bool,
    win_focused: bool,
    pane_open: bool,
    capture_tx: bool,
) {
    if !enabled() {
        return;
    }
    let bits = u32::from(draw)
        | (u32::from(capture) << 1)
        | (u32::from(hover) << 2)
        | (u32::from(slot_focused) << 3)
        | (u32::from(win_focused) << 4)
        | (u32::from(pane_open) << 5)
        | (u32::from(capture_tx) << 6);
    let prev = LAST_GATE_BITS.swap(bits, Relaxed);
    let first = prev == 0xFFFF_FFFF;
    let transition = !first && prev != bits;
    let take_initial = if first || transition {
        false
    } else {
        // Consume an initial slot only when we will emit a same-bits sample.
        GATE_INIT_LEFT
            .fetch_update(
                Relaxed,
                Relaxed,
                |n| if n == 0 { None } else { Some(n - 1) },
            )
            .is_ok()
    };
    if !(first || take_initial || transition) {
        return;
    }
    emit_with(Stage::Gate, || {
        format!(
            "draw={} capture={} hover={} slot_focused={} win_focused={} pane={} capture_tx={}",
            u8::from(draw),
            u8::from(capture),
            u8::from(hover),
            u8::from(slot_focused),
            u8::from(win_focused),
            u8::from(pane_open),
            u8::from(capture_tx)
        )
    });
}

/// `stream_capture_for` entry for a Left/Right key edge.
pub fn note_stream(
    key: ArrowKey,
    down: bool,
    channel: bool,
    slot_opt: bool,
    slot_id: Option<u64>,
) {
    if !enabled() {
        return;
    }
    emit_with(Stage::Stream, || {
        let sid = match slot_id {
            Some(id) => format!("0x{id:x}"),
            None => "-".to_string(),
        };
        format!(
            "key={} down={} channel={} slot_opt={} slot_id={sid}",
            key.as_str(),
            u8::from(down),
            u8::from(channel),
            u8::from(slot_opt)
        )
    });
}

/// Host `SlotInput` drain applied a Left/Right `InputEv::Key`.
pub fn note_drain(key: ArrowKey, down: bool, enabled_slot: bool) {
    if !enabled() {
        return;
    }
    emit_with(Stage::Drain, || {
        format!(
            "key={} down={} enabled={}",
            key.as_str(),
            u8::from(down),
            u8::from(enabled_slot)
        )
    });
}

/// `note_input_start` outcome.
///
/// `admitted` (pending push) differs from `row_match` (any registry row) and
/// from `live_row_match` (non-ended row). `counter_bump` is whether
/// `input_start_n` was incremented on a matched row. `row_gen` is the matched
/// row's generation when present — never invented.
pub fn note_metric_start(
    slot_id: u64,
    admitted: bool,
    row_match: bool,
    live_row_match: bool,
    counter_bump: bool,
    row_gen: Option<u64>,
) {
    if !enabled() {
        return;
    }
    emit_with(Stage::MetricStart, || {
        let gen = match row_gen {
            Some(g) => g.to_string(),
            None => "-".to_string(),
        };
        format!(
            "slot_id=0x{slot_id:x} admitted={} row_match={} live_row_match={} counter_bump={} row_gen={gen}",
            u8::from(admitted),
            u8::from(row_match),
            u8::from(live_row_match),
            u8::from(counter_bump)
        )
    });
}

/// Mailbox generation bind for pending panel inputs.
///
/// Emits on `bound_n > 0` (actionable). Zero-work calls use a separate
/// [`HEARTBEAT_CAP`] so warmup stores cannot exhaust the stage budget.
pub fn note_gen_bind(slot_id: u64, gen: u64, bound_n: u32) {
    if !enabled() {
        return;
    }
    if bound_n == 0 {
        let Some(seq) = try_emit_hb(&GEN_BIND_HB.0, &GEN_BIND_HB.1) else {
            return;
        };
        let mono = mono_ns();
        eprintln!(
            "[input-seam] seq={seq} mono_ns={mono} stage=gen_bind slot_id=0x{slot_id:x} gen={gen} bound_n=0 heartbeat=1"
        );
        return;
    }
    emit_with(Stage::GenBind, || {
        format!("slot_id=0x{slot_id:x} gen={gen} bound_n={bound_n} heartbeat=0")
    });
}

/// Panel host-texture present (not display scanout).
///
/// `removed_n`: pending samples removed from the queue.
/// `published_n`: samples that actually bumped registry complete counters
/// (0 when no row matched). Heartbeat only when both are zero.
pub fn note_present(
    slot_id: u64,
    gen: u64,
    removed_n: u32,
    published_n: u32,
    row_match: bool,
    live_row_match: bool,
    row_gen: Option<u64>,
) {
    if !enabled() {
        return;
    }
    if removed_n == 0 && published_n == 0 {
        let Some(seq) = try_emit_hb(&PRESENT_HB.0, &PRESENT_HB.1) else {
            return;
        };
        let mono = mono_ns();
        let rg = match row_gen {
            Some(g) => g.to_string(),
            None => "-".to_string(),
        };
        eprintln!(
            "[input-seam] seq={seq} mono_ns={mono} stage=present slot_id=0x{slot_id:x} gen={gen} removed_n=0 published_n=0 row_match={} live_row_match={} row_gen={rg} heartbeat=1",
            u8::from(row_match),
            u8::from(live_row_match)
        );
        return;
    }
    emit_with(Stage::Present, || {
        let rg = match row_gen {
            Some(g) => g.to_string(),
            None => "-".to_string(),
        };
        format!(
            "slot_id=0x{slot_id:x} gen={gen} removed_n={removed_n} published_n={published_n} row_match={} live_row_match={} row_gen={rg} heartbeat=0",
            u8::from(row_match),
            u8::from(live_row_match)
        )
    });
}

/// Parse one `[input-seam]` stderr line into key/value map (stable contract).
pub fn parse_record(line: &str) -> Option<std::collections::BTreeMap<String, String>> {
    let rest = line.strip_prefix("[input-seam] ")?;
    let mut map = std::collections::BTreeMap::new();
    for tok in rest.split_whitespace() {
        let (k, v) = tok.split_once('=')?;
        map.insert(k.to_string(), v.to_string());
    }
    if map.contains_key("seq") && map.contains_key("stage") && map.contains_key("mono_ns") {
        Some(map)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    pub(super) static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn guard() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[test]
    fn disabled_is_silent_and_does_no_stage_work() {
        let _g = guard();
        reset_for_test();
        force_for_test(false);
        note_window_arrow(ArrowKey::Left, true, false);
        note_imgui_arrow(ArrowKey::Right, false, "release");
        note_gate(true, true, true, true, true, true, true);
        note_stream(ArrowKey::Left, true, true, true, Some(1));
        note_drain(ArrowKey::Left, true, true);
        note_metric_start(1, true, false, false, false, None);
        note_gen_bind(1, 9, 1);
        note_present(1, 9, 1, 1, true, true, Some(3));
        for st in [
            Stage::WinArrow,
            Stage::ImguiArrow,
            Stage::Gate,
            Stage::Stream,
            Stage::Drain,
            Stage::MetricStart,
            Stage::GenBind,
            Stage::Present,
            Stage::Saturate,
        ] {
            let (e, s) = stage_counts(st);
            assert_eq!((e, s), (0, 0), "disabled must not touch {:?}", st);
        }
        assert!(!enabled());
    }

    #[test]
    fn format_is_parseable_and_cap_saturates() {
        let _g = guard();
        reset_for_test();
        force_for_test(true);
        assert!(enabled());

        let sample = "[input-seam] seq=1 mono_ns=42 stage=win_arrow key=left down=1 repeat=0";
        let m = parse_record(sample).expect("parse");
        assert_eq!(m.get("stage").map(String::as_str), Some("win_arrow"));
        assert_eq!(m.get("key").map(String::as_str), Some("left"));
        assert_eq!(m.get("down").map(String::as_str), Some("1"));
        assert_eq!(m.get("seq").map(String::as_str), Some("1"));
        assert!(parse_record("not a record").is_none());
        assert!(parse_record("[input-seam] seq=1").is_none());

        for _ in 0..STAGE_CAP {
            note_window_arrow(ArrowKey::Left, true, false);
        }
        let (e, s) = stage_counts(Stage::WinArrow);
        assert_eq!(e, STAGE_CAP);
        assert_eq!(s, 0);

        note_window_arrow(ArrowKey::Right, false, false);
        let (e2, s2) = stage_counts(Stage::WinArrow);
        assert_eq!(e2, STAGE_CAP);
        assert_eq!(s2, 1);
        let (sat_e, _) = stage_counts(Stage::Saturate);
        assert!(sat_e >= 1);

        for _ in 0..5 {
            note_window_arrow(ArrowKey::Left, true, true);
        }
        let (e3, s3) = stage_counts(Stage::WinArrow);
        assert_eq!(e3, STAGE_CAP);
        assert_eq!(s3, 6);
    }

    #[test]
    fn gate_reserves_transition_capacity() {
        let _g = guard();
        reset_for_test();
        force_for_test(true);

        note_gate(true, true, false, true, false, true, true);
        assert_eq!(stage_counts(Stage::Gate).0, 1);
        // Same bits: only GATE_INIT_RESERVE-0 more initials after first? first
        // does not consume INIT_LEFT; subsequent same-bits consume up to 8.
        for _ in 0..20 {
            note_gate(true, true, false, true, false, true, true);
        }
        let after_init = stage_counts(Stage::Gate).0;
        // 1 first + up to GATE_INIT_RESERVE same-bit initials
        assert!(after_init <= 1 + GATE_INIT_RESERVE);
        assert!(after_init >= 1);
        let before = stage_counts(Stage::Gate).0;
        for _ in 0..10 {
            note_gate(true, true, false, true, false, true, true);
        }
        assert_eq!(stage_counts(Stage::Gate).0, before);
        // Transition still emits with reserved capacity.
        note_gate(true, true, true, true, false, true, true);
        assert_eq!(stage_counts(Stage::Gate).0, before + 1);
        assert!(stage_counts(Stage::Gate).0 <= GATE_CAP);
    }

    #[test]
    fn bind_present_heartbeat_does_not_exhaust_actionable_cap() {
        let _g = guard();
        reset_for_test();
        force_for_test(true);

        for _ in 0..20 {
            note_gen_bind(1, 3, 0);
            note_present(1, 3, 0, 0, false, false, None);
        }
        assert_eq!(GEN_BIND_HB.0.load(Relaxed), HEARTBEAT_CAP);
        assert_eq!(PRESENT_HB.0.load(Relaxed), HEARTBEAT_CAP);
        // Actionable counters unused by heartbeats.
        assert_eq!(stage_counts(Stage::GenBind).0, 0);
        assert_eq!(stage_counts(Stage::Present).0, 0);

        note_gen_bind(1, 9, 2);
        note_present(1, 9, 2, 2, true, true, Some(4));
        assert_eq!(stage_counts(Stage::GenBind).0, 1);
        assert_eq!(stage_counts(Stage::Present).0, 1);
    }

    #[test]
    fn metric_start_fields_distinguish_admission_from_row_match() {
        let _g = guard();
        reset_for_test();
        force_for_test(true);
        note_metric_start(0xabc, true, false, false, false, None);
        assert_eq!(stage_counts(Stage::MetricStart).0, 1);
        let line = "[input-seam] seq=9 mono_ns=1 stage=metric_start slot_id=0xabc admitted=1 row_match=0 live_row_match=0 counter_bump=0 row_gen=-";
        let m = parse_record(line).unwrap();
        assert_eq!(m.get("admitted").map(String::as_str), Some("1"));
        assert_eq!(m.get("row_match").map(String::as_str), Some("0"));
        assert_eq!(m.get("live_row_match").map(String::as_str), Some("0"));
        assert_eq!(m.get("counter_bump").map(String::as_str), Some("0"));
        assert_eq!(m.get("row_gen").map(String::as_str), Some("-"));
    }

    #[test]
    fn arrow_from_ch_only_left_right() {
        assert_eq!(ArrowKey::from_ch(1), Some(ArrowKey::Left));
        assert_eq!(ArrowKey::from_ch(2), Some(ArrowKey::Right));
        assert_eq!(ArrowKey::from_ch(3), None);
        assert_eq!(ArrowKey::from_ch(0), None);
    }
}
