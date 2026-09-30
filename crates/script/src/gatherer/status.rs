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
