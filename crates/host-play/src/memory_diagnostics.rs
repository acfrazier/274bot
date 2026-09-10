//! Bounded, opt-in diagnostics for memory runs. Never drains application logs.
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

#[derive(Default)]
struct Slot {
    snapshot: Value,
    logs: VecDeque<String>,
    requests: VecDeque<String>,
    navigation: VecDeque<String>,
    last_capture: Option<Instant>,
    sent_batches: u64,
    request_batches: u64,
}
static SLOTS: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();

pub(crate) fn enable(names: &[String]) {
    let _ = SLOTS.set(Mutex::new(
        names.iter().map(|n| (n.clone(), Slot::default())).collect(),
    ));
}
fn bounded_push(queue: &mut VecDeque<String>, text: &str) {
    if queue.len() == 32 {
        queue.pop_front();
    }
    queue.push_back(text.chars().take(512).collect());
}
pub(crate) fn logs(name: &str, lines: &[String]) {
    if lines.is_empty() {
        return;
    }
    let Some(all) = SLOTS.get() else { return };
    if let Some(slot) = all.lock().unwrap().get_mut(name) {
        for line in lines {
            bounded_push(&mut slot.logs, line)
        }
    }
}
pub(crate) fn requests(name: &str, reqs: &[script::shim::InteractReq]) {
    if reqs.is_empty() {
        return;
    }
    let Some(all) = SLOTS.get() else { return };
    if let Some(slot) = all.lock().unwrap().get_mut(name) {
        slot.request_batches += 1;
        for req in reqs {
            bounded_push(&mut slot.requests, &format!("{req:?}"))
        }
    }
}
pub(crate) fn enabled() -> bool {
    SLOTS.get().is_some()
}
pub(crate) fn navigation(name: &str, event: impl FnOnce() -> String) {
    let Some(all) = SLOTS.get() else { return };
    if let Some(slot) = all.lock().unwrap().get_mut(name) {
        bounded_push(&mut slot.navigation, &event());
    }
}
pub(crate) fn sent(name: &str, wrote: bool) {
    let Some(all) = SLOTS.get() else { return };
    if wrote {
        if let Some(slot) = all.lock().unwrap().get_mut(name) {
            slot.sent_batches += 1;
        }
    }
}
pub(crate) fn frame(c: &client::client::Client, name: &str, hold: bool) {
    let Some(all) = SLOTS.get() else { return };
    let mut all = all.lock().unwrap();
    let Some(slot) = all.get_mut(name) else {
        return;
    };
    if slot
        .last_capture
        .is_some_and(|t| t.elapsed().as_millis() < 1000)
    {
        return;
    }
    let guards: Vec<_>=c.npc_ids.iter().take(c.npc_count.max(0) as usize).filter_map(|index| {
        let npc=c.npc.get(*index as usize)?.as_ref()?;
        let kind=c.cache.npcs.get(npc.r#type?)?;
        if kind.name != "Guard" {return None}
        Some(json!({"index":index,"type":npc.r#type,"x":c.map_build_base_x+npc.route_x[0],"z":c.map_build_base_z+npc.route_z[0],"face_entity":npc.face_entity,"animation":npc.primary_anim}))
    }).take(16).collect();
    slot.snapshot = json!({"client_tick":c.gens.player,"loop_cycle":c.loop_cycle,
        "xp":c.stat_xp.get(17),"hp":c.stat_effective_level.get(3),
        "position":crate::player_here_tile(c),"ingame":c.ingame,"scene_state":c.scene_state,
        "drawing":c.draw,"hold":hold,"animation":c.local_player.as_ref().map(|p|p.primary_anim),
        "spot_animation":c.local_player.as_ref().map(|p|p.spotanim_id),
        "spot_animation_start_cycle":c.local_player.as_ref().map(|p|p.spotanim_last_cycle),
        "face_entity":c.local_player.as_ref().map(|p|p.face_entity),
        "chat":c.chat_text.first(),"guards":guards});
    slot.last_capture = Some(Instant::now());
}
pub(crate) fn sample(name: &str) -> Value {
    let Some(all) = SLOTS.get() else {
        return Value::Null;
    };
    let all = all.lock().unwrap();
    let Some(slot) = all.get(name) else {
        return Value::Null;
    };
    json!({"client":slot.snapshot,"snapshot_age_ms":slot.last_capture.map(|t|t.elapsed().as_millis()),"recent_logs":slot.logs,"recent_requests":slot.requests,"recent_navigation":slot.navigation,"request_batches":slot.request_batches,"sent_batches":slot.sent_batches})
}
#[cfg(test)]
mod tests {
    #[test]
    fn logs_remain_bounded_and_keep_latest_utf8_messages() {
        let mut queue = std::collections::VecDeque::new();
        for i in 0..100 {
            super::bounded_push(&mut queue, &format!("{i}:{}", "é".repeat(1024)));
        }
        assert_eq!(queue.len(), 32);
        assert!(queue.front().unwrap().starts_with("68:"));
        assert!(queue.back().unwrap().starts_with("99:"));
        assert!(queue.iter().all(|s| s.chars().count() == 512));
    }
}
