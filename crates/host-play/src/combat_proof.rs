use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use script::native::{HostAuthority, NativePhase, ScriptStatus, StatusValue};
use script::shim::InteractReq;
use serde_json::{json, Value};

static CAPTURES: LazyLock<Mutex<HashMap<String, Arc<Mutex<CombatCapture>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn captures() -> &'static Mutex<HashMap<String, Arc<Mutex<CombatCapture>>>> {
    &CAPTURES
}

#[derive(Default)]
pub(crate) struct CombatCapture {
    pub(crate) case: String,
    pub(crate) frames: Vec<Value>,
    pub(crate) statuses: Vec<Value>,
    pub(crate) actions: Vec<Value>,
    pub(crate) random_events: Vec<Value>,
    pub(crate) observations: Vec<Value>,
    pub(crate) prayer_facts: Vec<Value>,
    pub(crate) start_baseline: Option<Value>,
    pub(crate) invalid_reason: Option<String>,
    pub(crate) inject_maze_after_imp_attack: bool,
    pub(crate) maze_pending: bool,
    pub(crate) maze_injected: bool,
    pub(crate) started: bool,
    pub(crate) maze_owner_live_before: Option<bool>,
    pub(crate) maze_owner_live_after: Option<bool>,
    last_frame: Option<String>,
    last_status: Option<String>,
    m5_attack_owner: Option<HostAuthority>,
}

pub(crate) struct CaptureRegistration {
    account: String,
    capture: Arc<Mutex<CombatCapture>>,
}

impl CaptureRegistration {
    pub(crate) fn install(account: &str, capture: Arc<Mutex<CombatCapture>>) -> Self {
        let mut registered = captures().lock().expect("combat capture registry");
        assert!(
            registered
                .insert(account.to_owned(), Arc::clone(&capture))
                .is_none(),
            "duplicate combat proof account"
        );
        Self {
            account: account.to_owned(),
            capture,
        }
    }
}

impl Drop for CaptureRegistration {
    fn drop(&mut self) {
        let mut registered = captures().lock().unwrap_or_else(|e| e.into_inner());
        if registered
            .get(&self.account)
            .is_some_and(|current| Arc::ptr_eq(current, &self.capture))
        {
            registered.remove(&self.account);
        }
    }
}

fn capture_for(account: &str) -> Option<Arc<Mutex<CombatCapture>>> {
    captures()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(account)
        .cloned()
}

pub(crate) fn capture_enabled(account: &str) -> bool {
    capture_for(account).is_some()
}

pub(crate) fn record_frame(account: &str, snapshot: &GameSnapshot) {
    let Some(capture) = capture_for(account) else {
        return;
    };
    let facts = snapshot_facts(snapshot);
    let signature = serde_json::to_string(&facts).unwrap_or_default();
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    if capture.last_frame.as_deref() != Some(&signature) {
        capture.last_frame = Some(signature);
        capture.frames.push(facts);
    }
}

pub(crate) fn record_start_baseline(account: &str, snapshot: &GameSnapshot) {
    if let Some(capture) = capture_for(account) {
        capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .start_baseline = Some(snapshot_facts(snapshot));
    }
}

pub(crate) fn record_status(account: &str, status: &ScriptStatus) {
    let Some(capture) = capture_for(account) else {
        return;
    };
    let value = status_value(status);
    let signature = serde_json::to_string(&value).unwrap_or_default();
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    if capture.last_status.as_deref() != Some(&signature) {
        capture.last_status = Some(signature);
        capture.statuses.push(value);
    }
}

pub(crate) fn record_observation(account: &str, tick: u64, exclusive: bool) {
    if let Some(capture) = capture_for(account) {
        capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .observations
            .push(json!({
                "tick": tick,
                "exclusive": exclusive,
            }));
    }
}

pub(crate) fn mark_invalid(account: &str, reason: impl Into<String>) {
    if let Some(capture) = capture_for(account) {
        let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
        capture.invalid_reason.get_or_insert_with(|| reason.into());
    }
}

pub(crate) fn record_interaction(
    account: &str,
    tick: u64,
    run: impl std::fmt::Debug,
    request_id: u64,
    batch: u64,
    authority: &HostAuthority,
    request: &InteractReq,
    accepted: bool,
    decoded: bool,
    wire_opcodes: &[i32],
    snapshot: &GameSnapshot,
) {
    let Some(capture) = capture_for(account) else {
        return;
    };
    let request_value = interaction_value(request);
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    let sequence = capture.actions.len() as u64 + 1;
    capture.actions.push(json!({
        "sequence": sequence,
        "kind": "interaction",
        "tick": tick,
        "run": format!("{run:?}"),
        "request_id": request_id,
        "batch": batch,
        "request": request_value,
        "accepted": accepted,
        "wire_decoded": decoded,
        "wire_opcodes": wire_opcodes,
        "snapshot": snapshot_facts(snapshot),
    }));
    if capture.inject_maze_after_imp_attack
        && !capture.maze_pending
        && !capture.maze_injected
        && accepted
        && matches!(
            request,
            InteractReq::Npc { name, action, .. }
                if name.eq_ignore_ascii_case("imp") && action.eq_ignore_ascii_case("attack")
        )
    {
        capture.m5_attack_owner = Some(authority.clone());
        capture.maze_pending = true;
    }
}

pub(crate) fn record_shim_interactions(
    account: &str,
    tick: u64,
    requests: &[InteractReq],
    snapshot: &GameSnapshot,
) {
    if requests.is_empty() {
        return;
    }
    let Some(capture) = capture_for(account) else {
        return;
    };
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    for request in requests {
        let sequence = capture.actions.len() as u64 + 1;
        capture.actions.push(json!({
            "sequence": sequence,
            "kind": "other-interaction",
            "origin": "shim",
            "tick": tick,
            "batch": 0,
            "request_id": null,
            "request": interaction_value(request),
            "accepted": null,
            "wire_decoded": false,
            "wire_opcodes": [],
            "snapshot": snapshot_facts(snapshot),
        }));
    }
}

pub(crate) fn record_other_request(account: &str, tick: u64, origin: &str, request: &str) {
    let Some(capture) = capture_for(account) else {
        return;
    };
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    let sequence = capture.actions.len() as u64 + 1;
    capture.actions.push(json!({
        "sequence": sequence,
        "kind": "other-request",
        "origin": origin,
        "tick": tick,
        "batch": 0,
        "request_id": null,
        "request": request,
        "accepted": null,
        "wire_decoded": false,
        "wire_opcodes": [],
    }));
}

pub(crate) fn record_walk(
    account: &str,
    tick: u64,
    run: impl std::fmt::Debug,
    request_id: u64,
    request: &impl std::fmt::Debug,
    snapshot: &GameSnapshot,
) {
    let Some(capture) = capture_for(account) else {
        return;
    };
    let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    let sequence = capture.actions.len() as u64 + 1;
    capture.actions.push(json!({
        "sequence": sequence,
        "kind": "walk",
        "tick": tick,
        "run": format!("{run:?}"),
        "request_id": request_id,
        "request": format!("{request:?}"),
        "dispatch": "queued",
        "accepted": null,
        "wire_decoded": false,
        "wire_opcodes": [],
        "snapshot": snapshot_facts(snapshot),
    }));
}

pub(crate) fn maze_injection_pending(account: &str) -> bool {
    capture_for(account).is_some_and(|capture| {
        capture
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .maze_pending
    })
}

pub(crate) fn maze_attack_owner_live(account: &str) -> Option<bool> {
    let capture = capture_for(account)?;
    let capture = capture.lock().unwrap_or_else(|e| e.into_inner());
    capture.m5_attack_owner.as_ref().map(HostAuthority::live)
}

pub(crate) fn record_maze_injection(
    account: &str,
    observer_tick: u32,
    claim: impl std::fmt::Debug,
    owner_live_before: Option<bool>,
    owner_live_after: Option<bool>,
) {
    if let Some(capture) = capture_for(account) {
        let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
        capture.maze_pending = false;
        capture.maze_injected = true;
        capture.maze_owner_live_before = owner_live_before;
        capture.maze_owner_live_after = owner_live_after;
        let action_sequence_at_injection = capture.actions.len() as u64;
        let prayer_varps_at_injection = capture
            .frames
            .last()
            .map(|frame| frame["prayer_varps"].clone())
            .unwrap_or(Value::Null);
        capture.random_events.push(json!({
            "kind": "Maze",
            "name": "combat-live-proof synthetic Maze hold",
            "observer_tick": observer_tick,
            "claim": format!("{claim:?}"),
            "delivery": "Play.observe -> PlaySlotScript.on_random",
            "hold": true,
            "active_combat_owner_live_before": owner_live_before,
            "active_combat_owner_live_after": owner_live_after,
            "action_sequence_at_injection": action_sequence_at_injection,
            "prayer_varps_at_injection": prayer_varps_at_injection,
        }));
    }
}

pub(crate) fn record_maze_injection_failure(account: &str, reason: impl Into<String>) {
    if let Some(capture) = capture_for(account) {
        let mut capture = capture.lock().unwrap_or_else(|e| e.into_inner());
        let reason = reason.into();
        capture.maze_pending = false;
        capture.invalid_reason.get_or_insert_with(|| reason.clone());
        capture.random_events.push(json!({
            "kind": "Maze",
            "delivery": "Play.observe -> PlaySlotScript.on_random",
            "error": reason,
        }));
    }
}

fn interaction_value(request: &InteractReq) -> Value {
    match request {
        InteractReq::Npc {
            name,
            action,
            index,
        } => json!({"op": "npc", "name": name, "action": action, "index": index}),
        InteractReq::Held { name, action, slot } => {
            json!({"op": "held", "name": name, "action": action, "slot": slot})
        }
        InteractReq::IfButton { component_id } => {
            json!({"op": "if-button", "component_id": component_id})
        }
        InteractReq::Wear { name } => json!({"op": "wear", "name": name}),
        InteractReq::SetRetaliate { on } => json!({"op": "set-retaliate", "on": on}),
        _ => json!({"debug": format!("{request:?}")}),
    }
}

fn status_value(status: &ScriptStatus) -> Value {
    let fields = status
        .fields
        .iter()
        .map(|field| (field.key.to_owned(), status_field_value(&field.value)))
        .collect::<serde_json::Map<_, _>>();
    json!({
        "run": format!("{:?}", status.run),
        "card": format!("{:?}", status.card),
        "phase": phase_name(status.phase),
        "fields": fields,
        "failure": status.failure.as_ref().map(|failure| format!("{failure:?}")),
    })
}

fn phase_name(phase: NativePhase) -> &'static str {
    match phase {
        NativePhase::Preparing => "Preparing",
        NativePhase::Working => "Working",
        NativePhase::Waiting => "Waiting",
        NativePhase::Blocked => "Blocked",
        NativePhase::Complete => "Complete",
    }
}

fn status_field_value(value: &StatusValue) -> Value {
    match value {
        StatusValue::Text(value) => json!(value.as_ref()),
        StatusValue::Integer(value) => json!(value),
        StatusValue::Tile(value) => json!(value),
        StatusValue::Truth(value) => json!(format!("{value:?}")),
        StatusValue::Quest(value) => json!({"debug": format!("{value:?}")}),
    }
}

pub(crate) fn snapshot_facts(snapshot: &GameSnapshot) -> Value {
    let local_slot = snapshot.self_slot();
    let nearby_npcs = snapshot
        .npcs()
        .iter()
        .filter(|npc| {
            npc.distance <= 12
                || npc.in_combat
                || npc.target.is_some_and(|target| {
                    target.kind == ActorKind::Player && target.index == local_slot as usize
                })
        })
        .map(|npc| {
            json!({
                "index": npc.index,
                "type": npc.r#type,
                "name": npc.name,
                "tile": npc.tile,
                "distance": npc.distance,
                "health": npc.health,
                "total_health": npc.total_health,
                "face_entity": npc.face_entity,
                "target": npc.target.map(|target| {
                    json!({"kind": format!("{:?}", target.kind), "index": target.index})
                }),
                "in_combat": npc.in_combat,
                "animation": npc.animation,
                "spot_animation": npc.spot_animation,
                "actions": npc.actions,
            })
        })
        .collect::<Vec<_>>();
    let inventory = snapshot
        .inventory()
        .iter()
        .filter(|item| item.container == ItemContainer::Inventory)
        .map(|item| {
            json!({
                "id": item.def.id,
                "slot": item.slot,
                "count": item.count,
                "actions": item.actions,
            })
        })
        .collect::<Vec<_>>();
    let equipment = snapshot
        .equipment()
        .iter()
        .map(|item| {
            json!({
                "id": item.def.id,
                "slot": item.slot,
                "count": item.count,
                "actions": item.actions,
            })
        })
        .collect::<Vec<_>>();
    let stats = snapshot
        .stats()
        .iter()
        .map(|stat| {
            json!({
                "index": stat.index,
                "name": stat.name,
                "base": stat.base,
                "effective": stat.effective,
                "xp": stat.xp,
            })
        })
        .collect::<Vec<_>>();
    let prayer_varps = (83..=97)
        .map(|index| json!({"index": index, "value": snapshot.varp(index)}))
        .collect::<Vec<_>>();
    let local = snapshot.local_player().map(|player| {
        json!({
            "name": player.player.actor.name,
            "tile": player.player.actor.tile,
            "in_combat": player.player.actor.in_combat,
            "target": player.player.actor.target.map(|target| {
                json!({"kind": format!("{:?}", target.kind), "index": target.index})
            }),
            "animation": player.player.actor.animation,
            "spot_animation": player.player.actor.spot_animation,
        })
    });
    json!({
        "tick": snapshot.tick(),
        "ingame": snapshot.ingame(),
        "scene_state": snapshot.scene_state(),
        "tile": snapshot.tile(),
        "self_slot": local_slot,
        "local_player": local,
        "stats": stats,
        "prayer_varps": prayer_varps,
        "inventory": inventory,
        "equipment": equipment,
        "nearby_npcs": nearby_npcs,
    })
}
