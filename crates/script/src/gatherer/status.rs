use crate::acquire::BankSeen;
pub use crate::native::tickcycle::CycleStats as TickStats;
use crate::native::{
    NativeOutput, NativePhase, ScriptFailure, ScriptStatus, StatusField, StatusValue,
};
use crate::paint::{host_control_buttons, ScriptPaint};
use crate::CompiledId;
use api::bank_memory::Origin;
use api::selected::RunKey;
use std::sync::{Arc, LazyLock};

#[derive(Debug, Clone)]
pub struct StatusData {
    pub skill: &'static str,
    pub method: Arc<str>,
    pub survival: crate::survival::SurvivalReport,
    pub phase: &'static str,
    pub area: Arc<str>,
    pub target: Arc<str>,
    pub tool: Arc<str>,
    pub bait: i64,
    pub food: i64,
    pub coins: i64,
    pub yielded: u32,
    pub dropped: u32,
    pub deposited: u32,
    pub trips: u32,
    pub xp: i32,
    pub xp_per_hour: Option<i32>,
    pub bank: Arc<str>,
    pub bank_seen: Option<BankSeen>,
    pub block_reason: Arc<str>,
    pub last_progress: u64,
    pub deaths: u8,
    pub recoveries: u8,
    pub recovery_step: u8,
    pub absent: u16,
    pub zone_gated: u16,
    pub excluded_targets: Arc<str>,
    pub last_event: Arc<str>,
    pub tick_method: Arc<str>,
    pub tick_cycle: i64,
    pub tick_stats: TickStats,
}

/// Value kind of one published status key. [`KEYS`] projects key/kind from
/// the single [`status_rows!`] table below, and [`publish`] builds its field
/// array from the same table; the `GatherStatus` JS declaration is rendered
/// from [`KEYS`] (never copied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Integer,
}

/// The canonical `(key, label, kind, value)` status table. Passing the
/// data identifier keeps value expressions hygienic; emitting the whole
/// array lets the same table project both keys and published fields.
macro_rules! status_rows {
    ($emit:ident, $data:ident) => {
        [
            $emit!("skill", "Skill", Text, $data.skill),
            $emit!("method", "Method", Text, Arc::clone(&$data.method)),
            $emit!("survival", "Survival", Text, format!("{}", $data.survival)),
            $emit!("phase", "Phase", Text, $data.phase),
            $emit!("area", "Area", Text, Arc::clone(&$data.area)),
            $emit!("target", "Target", Text, Arc::clone(&$data.target)),
            $emit!("tool", "Tool", Text, Arc::clone(&$data.tool)),
            $emit!(
                "tick_method",
                "Tick method",
                Text,
                Arc::clone(&$data.tick_method)
            ),
            $emit!("tick_cycle", "Tick cycle", Integer, $data.tick_cycle),
            $emit!(
                "tick_verified",
                "Tick verified",
                Integer,
                $data.tick_stats.verified
            ),
            $emit!(
                "tick_unobserved",
                "Tick unobserved",
                Integer,
                $data.tick_stats.unobserved
            ),
            $emit!(
                "tick_slips",
                "Tick slips",
                Integer,
                $data.tick_stats.slipped
            ),
            $emit!("tick_lost", "Tick lost", Integer, $data.tick_stats.lost),
            $emit!(
                "tick_delayed",
                "Tick delayed",
                Integer,
                $data.tick_stats.delayed
            ),
            $emit!(
                "tick_paused",
                "Tick paused",
                Integer,
                $data.tick_stats.paused
            ),
            $emit!(
                "tick_fallbacks",
                "Tick fallbacks",
                Integer,
                $data.tick_stats.fallback_clicks
            ),
            $emit!(
                "tick_fallback_failed",
                "Tick fallback failed",
                Integer,
                $data.tick_stats.fallback_failed
            ),
            $emit!(
                "tick_events",
                "Tick events",
                Integer,
                $data.tick_stats.wire_events
            ),
            $emit!("bait", "Bait", Integer, $data.bait),
            $emit!("food", "Food", Integer, $data.food),
            $emit!("coins", "Coins", Integer, $data.coins),
            $emit!("yielded", "Yielded", Integer, i64::from($data.yielded)),
            $emit!("dropped", "Dropped", Integer, i64::from($data.dropped)),
            $emit!(
                "deposited",
                "Deposited",
                Integer,
                i64::from($data.deposited)
            ),
            $emit!("trips", "Trips", Integer, i64::from($data.trips)),
            $emit!("xp", "XP", Integer, i64::from($data.xp)),
            $emit!(
                "xp_per_hour",
                "XP/hour",
                Integer,
                i64::from($data.xp_per_hour.unwrap_or(-1))
            ),
            $emit!("bank", "Bank", Text, Arc::clone(&$data.bank)),
            $emit!(
                "bank_seen",
                "Bank seen (at status update)",
                Text,
                bank_seen_text($data.bank_seen)
            ),
            $emit!(
                "block_reason",
                "Block reason (evidence when blocked)",
                Text,
                Arc::clone(&$data.block_reason)
            ),
            $emit!(
                "last_progress",
                "Last progress",
                Integer,
                $data.last_progress as i64
            ),
            $emit!("deaths", "Deaths", Integer, i64::from($data.deaths)),
            $emit!(
                "recoveries",
                "Recoveries",
                Integer,
                i64::from($data.recoveries)
            ),
            $emit!(
                "recovery_step",
                "Recovery step",
                Integer,
                i64::from($data.recovery_step)
            ),
            $emit!("absent", "Absent", Integer, i64::from($data.absent)),
            $emit!(
                "zone_gated",
                "Zone gated",
                Integer,
                i64::from($data.zone_gated)
            ),
            $emit!(
                "excluded_targets",
                "Excluded targets",
                Text,
                Arc::clone(&$data.excluded_targets)
            ),
            $emit!(
                "last_event",
                "Last event",
                Text,
                Arc::clone(&$data.last_event)
            ),
        ]
    };
}

macro_rules! status_key {
    ($key:literal, $_label:literal, $kind:ident, $_value:expr) => {
        ($key, FieldKind::$kind)
    };
}

macro_rules! status_field {
    ($key:literal, $label:literal, Text, $value:expr) => {
        StatusField {
            key: $key,
            label: $label,
            value: StatusValue::Text(($value).into()),
        }
    };
    ($key:literal, $label:literal, Integer, $value:expr) => {
        StatusField {
            key: $key,
            label: $label,
            value: StatusValue::Integer(($value).into()),
        }
    };
}

/// Published status keys, projected from the same table as [`publish`].
/// The host JS `GatherStatus` declaration is rendered from this table.
pub const KEYS: &[(&str, FieldKind)] = &status_rows!(status_key, data);

pub fn publish(
    output: &mut dyn NativeOutput,
    run: RunKey,
    revision: u64,
    pending_revision: Option<u64>,
    phase: NativePhase,
    failure: Option<ScriptFailure>,
    data: &StatusData,
) {
    let fields: Arc<[StatusField]> = Arc::from(status_rows!(status_field, data));
    output.status(ScriptStatus {
        run,
        card: CompiledId("Gatherer"),
        phase,
        active_settings: revision,
        pending_settings: pending_revision,
        fields,
        survival: data.survival,
        failure,
    });
}

pub fn paint(data: &StatusData) -> Arc<ScriptPaint> {
    Arc::new(ScriptPaint {
        title: Some(format!("Gatherer — {}", data.phase)),
        accent: Some("#c58b45".into()),
        lines: vec![
            format!("{} · {}", data.skill, data.method),
            format!("{}", data.area),
            format!("yielded {} · dropped {}", data.yielded, data.dropped),
            format!("tool {} · {}", data.tool, data.target),
            format!("last event: {}", data.last_event),
        ],
        // The host Pause and Stop controls, as a compat helper paint
        // advertises them.
        buttons: host_control_buttons(),
        ..ScriptPaint::default()
    })
}

pub(super) fn empty_block_reason() -> Arc<str> {
    static EMPTY: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from(""));
    Arc::clone(&EMPTY)
}

fn bank_seen_text(seen: Option<BankSeen>) -> Arc<str> {
    seen.map_or_else(
        || {
            static UNKNOWN: LazyLock<Arc<str>> = LazyLock::new(|| Arc::from("unknown"));
            Arc::clone(&UNKNOWN)
        },
        |seen| bank_seen_text_at(seen, std::time::SystemTime::now()),
    )
}

fn bank_seen_text_at(seen: BankSeen, now: std::time::SystemTime) -> Arc<str> {
    use std::fmt::Write;

    let origin = match seen.origin {
        Origin::Session => "Session",
        Origin::Hint => "Hint (saved)",
        Origin::Unknown => return Arc::from("unknown"),
    };
    // Match the core's relative-age rule, including future/overflow fallback.
    // This text is fixed at publication; G2 owns typed clock presentation.
    let age = std::time::UNIX_EPOCH
        .checked_add(std::time::Duration::from_secs(seen.observed_at_unix))
        .and_then(|observed| now.duration_since(observed).ok());
    let mut text = String::with_capacity(64);
    text.push_str(origin);
    if let Some(age) = age {
        let seconds = age.as_secs();
        if seconds < 60 {
            text.push_str(", less than a minute ago");
        } else if seconds < 3_600 {
            let _ = write!(text, ", {} min ago", seconds / 60);
        } else if seconds < 86_400 {
            let _ = write!(text, ", {} hr ago", seconds / 3_600);
        } else {
            let days = seconds / 86_400;
            let _ = write!(
                text,
                ", {days} {} ago",
                if days == 1 { "day" } else { "days" }
            );
        }
    }
    let _ = write!(text, ", {} rows", seen.rows);
    Arc::from(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native::NativeOutput;
    use api::hostlog::Level;

    struct Capture {
        status: Option<ScriptStatus>,
    }

    impl NativeOutput for Capture {
        fn status(&mut self, status: ScriptStatus) {
            self.status = Some(status);
        }

        fn paint(&mut self, _frame: Arc<ScriptPaint>) {}

        fn log(&mut self, _level: Level, _message: &str) {}

        fn settings_applied(&mut self, _revision: u64) {}
    }

    #[test]
    fn unchanged_status_text_uses_bounded_shared_storage() {
        let first = allocation_counter::measure(|| {
            drop(empty_block_reason());
            drop(bank_seen_text(None));
        });
        assert!(first.count_total <= 2);
        assert!(first.bytes_total <= 39);
        assert_eq!(std::mem::size_of::<LazyLock<Arc<str>>>(), 24);
        let warm = allocation_counter::measure(|| {
            for _ in 0..1_000 {
                assert!(empty_block_reason().is_empty());
                assert_eq!(bank_seen_text(None).as_ref(), "unknown");
            }
        });
        assert_eq!(warm.count_total, 0);
    }

    /// `publish` emits the shared [`KEYS`] key set with the table's value
    /// kinds and the card's typed values. Order is not asserted: only the
    /// key set, the per-key kind, and representative typed public values.
    #[test]
    fn publish_emits_typed_values_for_every_shared_key() {
        let data = StatusData {
            skill: "Woodcutting",
            method: Arc::from("normal"),
            survival: crate::survival::SurvivalReport::default(),
            phase: "gathering",
            area: Arc::from("Start"),
            target: Arc::from("Tree"),
            tool: Arc::from("Bronze axe"),
            bait: 0,
            food: 0,
            coins: 0,
            yielded: 3,
            dropped: 2,
            deposited: 0,
            trips: 0,
            xp: 75,
            xp_per_hour: None,
            bank: Arc::from(""),
            bank_seen: None,
            block_reason: Arc::from(""),
            last_progress: 7,
            deaths: 0,
            recoveries: 0,
            recovery_step: 0,
            absent: 0,
            zone_gated: 0,
            excluded_targets: Arc::from(""),
            last_event: Arc::from("gathered"),
            tick_method: Arc::from("Off"),
            tick_cycle: -1,
            tick_stats: Default::default(),
        };
        let mut out = Capture { status: None };
        publish(
            &mut out,
            RunKey {
                slot: 1,
                run: 1,
                session: 1,
            },
            1,
            None,
            NativePhase::Working,
            None,
            &data,
        );
        let status = out.status.expect("publish posts one status");
        let mut keys: Vec<&str> = status.fields.iter().map(|field| field.key).collect();
        keys.sort_unstable();
        let mut want: Vec<&str> = KEYS.iter().map(|(key, _)| *key).collect();
        want.sort_unstable();
        assert_eq!(keys, want, "published key set matches KEYS");
        let value = |key: &str| {
            status
                .fields
                .iter()
                .find(|field| field.key == key)
                .unwrap_or_else(|| panic!("status key {key}"))
                .value
                .clone()
        };
        for (key, kind) in KEYS {
            match kind {
                FieldKind::Text => assert!(
                    matches!(value(key), StatusValue::Text(_)),
                    "key {key} is text",
                ),
                FieldKind::Integer => assert!(
                    matches!(value(key), StatusValue::Integer(_)),
                    "key {key} is integer",
                ),
            }
        }
        assert_eq!(
            value("skill"),
            StatusValue::Text(Arc::from("Woodcutting")),
            "skill publishes its typed value",
        );
        assert_eq!(
            value("yielded"),
            StatusValue::Integer(3),
            "yielded publishes its typed value",
        );
        assert_eq!(
            value("xp_per_hour"),
            StatusValue::Integer(-1),
            "missing xp_per_hour publishes -1",
        );
        assert_eq!(value("bank_seen"), StatusValue::Text(Arc::from("unknown")),);
    }

    #[test]
    fn bank_seen_keeps_provenance_and_relative_age_without_a_clock_formatter() {
        assert_eq!(bank_seen_text(None).as_ref(), "unknown");
        let now_secs = 1_000_000;
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(now_secs);
        let mut seen = BankSeen {
            origin: Origin::Session,
            observed_at_unix: now_secs - 12 * 60,
            rows: 37,
            count: 0,
        };
        assert_eq!(
            bank_seen_text_at(seen, now).as_ref(),
            "Session, 12 min ago, 37 rows",
        );
        seen.origin = Origin::Hint;
        seen.observed_at_unix = now_secs - 3 * 86_400;
        assert_eq!(
            bank_seen_text_at(seen, now).as_ref(),
            "Hint (saved), 3 days ago, 37 rows",
        );
        for timestamp in [now_secs + 1, u64::MAX] {
            seen.observed_at_unix = timestamp;
            assert_eq!(
                bank_seen_text_at(seen, now).as_ref(),
                "Hint (saved), 37 rows"
            );
            seen.origin = Origin::Session;
            assert_eq!(bank_seen_text_at(seen, now).as_ref(), "Session, 37 rows");
            seen.origin = Origin::Hint;
        }
        seen.origin = Origin::Unknown;
        assert_eq!(bank_seen_text_at(seen, now).as_ref(), "unknown");
    }

    /// The native Gatherer paint advertises the host Pause and Stop
    /// controls, in the order a compat helper paint advertises them.
    #[test]
    fn paint_advertises_host_pause_and_stop() {
        let data = StatusData {
            skill: "Woodcutting",
            method: Arc::from("normal"),
            survival: crate::survival::SurvivalReport::default(),
            phase: "gathering",
            area: Arc::from("Start"),
            target: Arc::from("Tree"),
            tool: Arc::from("Bronze axe"),
            bait: 0,
            food: 0,
            coins: 0,
            yielded: 0,
            dropped: 0,
            deposited: 0,
            trips: 0,
            xp: 0,
            xp_per_hour: None,
            bank: Arc::from(""),
            bank_seen: None,
            block_reason: Arc::from(""),
            last_progress: 0,
            deaths: 0,
            recoveries: 0,
            recovery_step: 0,
            absent: 0,
            zone_gated: 0,
            excluded_targets: Arc::from(""),
            last_event: Arc::from(""),
            tick_method: Arc::from("Off"),
            tick_cycle: -1,
            tick_stats: Default::default(),
        };
        let frame = paint(&data);
        let ids: Vec<(&str, &str)> = frame
            .buttons
            .iter()
            .map(|button| (button.id.as_str(), button.label.as_str()))
            .collect();
        assert_eq!(
            ids,
            vec![
                (crate::paint::HOST_PAUSE_CONTROL_ID, "Pause"),
                (crate::paint::HOST_STOP_CONTROL_ID, "Stop"),
            ]
        );
    }
}
