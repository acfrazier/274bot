//! Opt-in, bounded navigation diagnostic checkpoints. Full snapshots are
//! serialized only at checkpoints, never on ordinary reads or every tick.
use api::snapshot::GameSnapshot;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

#[derive(Debug)]
pub struct Checkpoint {
    pub name: String,
    pub label: String,
    pub evidence: Value,
}
#[derive(Default)]
struct State {
    watched: String,
    scene: bool,
    bank: bool,
    returning: bool,
    failed: bool,
    terminal: Option<String>,
    terminal_requested: bool,
    views: HashMap<String, Value>,
    ready: VecDeque<Checkpoint>,
    serial: u32,
}
static STATE: OnceLock<Mutex<State>> = OnceLock::new();
pub fn enable(names: &[String]) {
    if std::env::var("BOT_NAV_CAPTURES").as_deref() != Ok("1") {
        return;
    }
    let _ = STATE.set(Mutex::new(State {
        watched: names[0].clone(),
        views: names.iter().map(|n| (n.clone(), Value::Null)).collect(),
        ..Default::default()
    }));
}
pub fn enabled() -> bool {
    STATE.get().is_some()
}
impl State {
    fn push(&mut self, name: &str, label: &str, snapshot: &GameSnapshot, detail: Value) {
        self.serial += 1;
        let evidence = json!({"slot":name,"checkpoint":label,"event_tick":snapshot.tick(),"snapshot":snapshot,"detail":detail,"diagnostics":crate::memory_diagnostics::sample(name)});
        let item = Checkpoint {
            name: name.into(),
            label: format!("nav-{}-{label}-{name}", self.serial),
            evidence,
        };
        // At most three watched milestones plus the first nav failure and
        // one terminal script failure. Prefer failure evidence if full.
        if self.ready.len() == 5 {
            self.ready.pop_back();
        }
        if label.contains("failure") {
            self.ready.push_front(item);
        } else {
            self.ready.push_back(item);
        }
    }
}
pub(crate) fn observe(name: &str, snapshot: &GameSnapshot, drawing: bool, running: bool) {
    let Some(state) = STATE.get() else { return };
    let mut s = state.lock().unwrap();
    if let Some(view) = s.views.get_mut(name) {
        *view = json!({"tick":snapshot.tick(),"tile":snapshot.tile(),"scene_state":snapshot.scene_state(),"ingame":snapshot.ingame(),"drawing":drawing});
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
    let Some(state) = STATE.get() else { return };
    let mut s = state.lock().unwrap();
    if name == s.watched && s.bank && !s.returning && snapshot.bank_component_id() == -1 {
        s.returning = true;
        s.push(
            name,
            "return-route-start",
            snapshot,
            json!({"dest":dest,"generation":generation}),
        );
    }
}
pub(crate) fn failure(name: &str, snapshot: &GameSnapshot, detail: Value) {
    let Some(state) = STATE.get() else { return };
    let mut s = state.lock().unwrap();
    if !s.failed {
        s.failed = true;
        s.push(name, "nav-failure", snapshot, detail);
    }
}
pub fn request_terminal(error: &str) {
    let Some(state) = STATE.get() else { return };
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

#[cfg(test)]
mod tests {
    use super::*;
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
}
