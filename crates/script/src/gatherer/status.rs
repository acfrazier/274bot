use crate::native::{
    NativeOutput, NativePhase, ScriptFailure, ScriptStatus, StatusField, StatusValue,
};
use crate::shim::ScriptPaint;
use crate::CompiledId;
use api::selected::RunKey;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct StatusData {
    pub skill: &'static str,
    pub method: Arc<str>,
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
    pub last_progress: u64,
    pub deaths: u8,
    pub absent: u16,
    pub zone_gated: u16,
    pub excluded_targets: Arc<str>,
    pub last_event: Arc<str>,
}

/// Value kind of one published status key. [`KEYS`] is the single key/type
/// table: [`publish`] emits exactly these keys with these value kinds, and
/// the `GatherStatus` JS declaration is rendered from it (never copied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Integer,
}

/// The Gatherer's published status keys in publish order (22 entries).
/// [`publish`] emits exactly this table; the host JS `GatherStatus`
/// declaration is rendered from it.
pub const KEYS: &[(&str, FieldKind)] = &[
    ("skill", FieldKind::Text),
    ("method", FieldKind::Text),
    ("phase", FieldKind::Text),
    ("area", FieldKind::Text),
    ("target", FieldKind::Text),
    ("tool", FieldKind::Text),
    ("bait", FieldKind::Integer),
    ("food", FieldKind::Integer),
    ("coins", FieldKind::Integer),
    ("yielded", FieldKind::Integer),
    ("dropped", FieldKind::Integer),
    ("deposited", FieldKind::Integer),
    ("trips", FieldKind::Integer),
    ("xp", FieldKind::Integer),
    ("xp_per_hour", FieldKind::Integer),
    ("bank", FieldKind::Text),
    ("last_progress", FieldKind::Integer),
    ("deaths", FieldKind::Integer),
    ("absent", FieldKind::Integer),
    ("zone_gated", FieldKind::Integer),
    ("excluded_targets", FieldKind::Text),
    ("last_event", FieldKind::Text),
];

pub fn publish(
    output: &mut dyn NativeOutput,
    run: RunKey,
    revision: u64,
    pending_revision: Option<u64>,
    phase: NativePhase,
    failure: Option<ScriptFailure>,
    data: &StatusData,
) {
    let fields: Arc<[StatusField]> = Arc::from([
        text("skill", "Skill", data.skill),
        text_arc("method", "Method", Arc::clone(&data.method)),
        text("phase", "Phase", data.phase),
        text_arc("area", "Area", Arc::clone(&data.area)),
        text_arc("target", "Target", Arc::clone(&data.target)),
        text_arc("tool", "Tool", Arc::clone(&data.tool)),
        integer("bait", "Bait", data.bait),
        integer("food", "Food", data.food),
        integer("coins", "Coins", data.coins),
        integer("yielded", "Yielded", i64::from(data.yielded)),
        integer("dropped", "Dropped", i64::from(data.dropped)),
        integer("deposited", "Deposited", i64::from(data.deposited)),
        integer("trips", "Trips", i64::from(data.trips)),
        integer("xp", "XP", i64::from(data.xp)),
        integer(
            "xp_per_hour",
            "XP/hour",
            i64::from(data.xp_per_hour.unwrap_or(-1)),
        ),
        text_arc("bank", "Bank", Arc::clone(&data.bank)),
        integer("last_progress", "Last progress", data.last_progress as i64),
        integer("deaths", "Deaths", i64::from(data.deaths)),
        integer("absent", "Absent", i64::from(data.absent)),
        integer("zone_gated", "Zone gated", i64::from(data.zone_gated)),
        text_arc(
            "excluded_targets",
            "Excluded targets",
            Arc::clone(&data.excluded_targets),
        ),
        text_arc("last_event", "Last event", Arc::clone(&data.last_event)),
    ]);
    debug_assert_eq!(
        fields.len(),
        KEYS.len(),
        "gatherer status KEYS drifted from publish",
    );
    for (field, (key, kind)) in fields.iter().zip(KEYS.iter()) {
        debug_assert_eq!(field.key, *key, "gatherer status key drift");
        debug_assert!(
            matches!(
                (&field.value, kind),
                (StatusValue::Text(_), FieldKind::Text)
                    | (StatusValue::Integer(_), FieldKind::Integer)
            ),
            "gatherer status kind drift for {key}",
        );
    }
    let fields: Arc<[StatusField]> = fields;
    output.status(ScriptStatus {
        run,
        card: CompiledId("Gatherer"),
        phase,
        active_settings: revision,
        pending_settings: pending_revision,
        fields,
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
        ..ScriptPaint::default()
    })
}

fn text(key: &'static str, label: &'static str, value: &str) -> StatusField {
    StatusField {
        key,
        label,
        value: StatusValue::Text(Arc::from(value)),
    }
}

fn text_arc(key: &'static str, label: &'static str, value: Arc<str>) -> StatusField {
    StatusField {
        key,
        label,
        value: StatusValue::Text(value),
    }
}

fn integer(key: &'static str, label: &'static str, value: i64) -> StatusField {
    StatusField {
        key,
        label,
        value: StatusValue::Integer(value),
    }
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

    /// `publish` emits exactly the shared [`KEYS`] table: same keys in the
    /// same order with the same value kinds. The JS `GatherStatus`
    /// declaration is rendered from that table, so drift here breaks the
    /// freshness gate, not just a comment.
    #[test]
    fn publish_emits_exactly_the_shared_keys_table() {
        let data = StatusData {
            skill: "Woodcutting",
            method: Arc::from("normal"),
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
            last_progress: 7,
            deaths: 0,
            absent: 0,
            zone_gated: 0,
            excluded_targets: Arc::from(""),
            last_event: Arc::from("gathered"),
        };
        let mut out = Capture { status: None };
        publish(
            &mut out,
            RunKey { slot: 1, run: 1, session: 1 },
            1,
            None,
            NativePhase::Working,
            None,
            &data,
        );
        let status = out.status.expect("publish posts one status");
        assert_eq!(status.fields.len(), KEYS.len(), "status KEYS length");
        assert_eq!(status.fields.len(), 22, "gatherer publishes 22 keys");
        for (field, (key, kind)) in status.fields.iter().zip(KEYS.iter()) {
            assert_eq!(field.key, *key, "status key");
            match kind {
                FieldKind::Text => assert!(
                    matches!(field.value, StatusValue::Text(_)),
                    "key {key} is text",
                ),
                FieldKind::Integer => assert!(
                    matches!(field.value, StatusValue::Integer(_)),
                    "key {key} is integer",
                ),
            }
        }
        let xp_per_hour = status
            .fields
            .iter()
            .find(|field| field.key == "xp_per_hour")
            .expect("xp_per_hour key");
        assert_eq!(
            xp_per_hour.value,
            StatusValue::Integer(-1),
            "missing xp_per_hour publishes -1",
        );
    }
}
