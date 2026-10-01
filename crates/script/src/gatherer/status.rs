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

/// Value kind of one published status key. [`KEYS`] projects key/kind from
/// the single [`status_rows!`] table below, and [`publish`] builds its field
/// array from the same table; the `GatherStatus` JS declaration is rendered
/// from [`KEYS`] (never copied).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    Integer,
}

/// The single canonical status table: each row is
/// `(key, label, kind, value)`. The value expression reads `data` from the
/// enclosing scope. [`KEYS`] and [`publish`] both expand this table through
/// [`status_key!`] / [`status_field!`], so the published keys and the shared
/// key table cannot drift.
macro_rules! status_rows {
    ($emit:ident) => {
        $emit!("skill", "Skill", Text, data.skill),
        $emit!("method", "Method", Text, Arc::clone(&data.method)),
        $emit!("phase", "Phase", Text, data.phase),
        $emit!("area", "Area", Text, Arc::clone(&data.area)),
        $emit!("target", "Target", Text, Arc::clone(&data.target)),
        $emit!("tool", "Tool", Text, Arc::clone(&data.tool)),
        $emit!("bait", "Bait", Integer, data.bait),
        $emit!("food", "Food", Integer, data.food),
        $emit!("coins", "Coins", Integer, data.coins),
        $emit!("yielded", "Yielded", Integer, i64::from(data.yielded)),
        $emit!("dropped", "Dropped", Integer, i64::from(data.dropped)),
        $emit!("deposited", "Deposited", Integer, i64::from(data.deposited)),
        $emit!("trips", "Trips", Integer, i64::from(data.trips)),
        $emit!("xp", "XP", Integer, i64::from(data.xp)),
        $emit!(
            "xp_per_hour",
            "XP/hour",
            Integer,
            i64::from(data.xp_per_hour.unwrap_or(-1))
        ),
        $emit!("bank", "Bank", Text, Arc::clone(&data.bank)),
        $emit!(
            "last_progress",
            "Last progress",
            Integer,
            data.last_progress as i64
        ),
        $emit!("deaths", "Deaths", Integer, i64::from(data.deaths)),
        $emit!("absent", "Absent", Integer, i64::from(data.absent)),
        $emit!("zone_gated", "Zone gated", Integer, i64::from(data.zone_gated)),
        $emit!(
            "excluded_targets",
            "Excluded targets",
            Text,
            Arc::clone(&data.excluded_targets)
        ),
        $emit!(
            "last_event",
            "Last event",
            Text,
            Arc::clone(&data.last_event)
        )
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

/// The Gatherer's published status keys (22 entries), projected from the
/// single [`status_rows!`] table. The host JS `GatherStatus` declaration is
/// rendered from this table.
pub const KEYS: &[(&str, FieldKind)] = &[status_rows!(status_key)];

pub fn publish(
    output: &mut dyn NativeOutput,
    run: RunKey,
    revision: u64,
    pending_revision: Option<u64>,
    phase: NativePhase,
    failure: Option<ScriptFailure>,
    data: &StatusData,
) {
    let fields: Arc<[StatusField]> = Arc::from([status_rows!(status_field)]);
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

    /// `publish` emits the shared [`KEYS`] key set with the table's value
    /// kinds and the card's typed values. Order is not asserted: only the
    /// key set, the per-key kind, and representative typed public values.
    #[test]
    fn publish_emits_typed_values_for_every_shared_key() {
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
    }
}
