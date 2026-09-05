//! Bounded withdrawal sequencing owned by Rust. JS supplies observations and
//! executes the returned existing bank operation; it contains no retry policy.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Default, Deserialize, Serialize)]
struct State {
    start: f64,
    before: f64,
    rounds: u32,
    phase: String,
}

pub fn step(input: &Value) -> Value {
    let count = input["count"].as_f64().unwrap_or(0.0);
    let target = input["target"].as_f64().unwrap_or(0.0);
    let mut s: State = serde_json::from_value(input["state"].clone()).unwrap_or(State {
        start: count,
        ..Default::default()
    });
    let full = input["full"].as_bool().unwrap_or(false);
    let ok = input["ok"].as_bool().unwrap_or(false);
    if s.phase == "op" && !ok {
        return json!({"kind":"done", "value":count-s.start});
    }
    let fallback = s.phase == "x" && !(ok && count > s.before);
    if !fallback {
        if s.rounds >= 40 || count >= target || full {
            return json!({"kind":"done", "value":count-s.start});
        }
        s.rounds += 1;
        s.before = count;
    }
    let need = target - s.before;
    if need <= 0.0 {
        return json!({"kind":"done", "value":count-s.start});
    }
    if need > 10.0 && !fallback {
        s.phase = "x".into();
        return json!({"kind":"x", "count":need, "state":s});
    }
    s.phase = "op".into();
    let op = if need >= 10.0 {
        "Withdraw-10"
    } else if need >= 5.0 {
        "Withdraw-5"
    } else {
        "Withdraw-1"
    };
    json!({"kind":"op", "op":op, "before":s.before, "state":s})
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn x_then_fallback_then_exact_remainder() {
        let x = step(&json!({"count":2,"target":22}));
        assert_eq!(x["kind"], "x");
        assert_eq!(x["count"], 20.0);
        let op = step(&json!({"state":x["state"],"count":2,"target":22,"ok":false}));
        assert_eq!(op["op"], "Withdraw-10");
        let rest = step(&json!({"state":op["state"],"count":12,"target":22,"ok":true}));
        assert_eq!(rest["op"], "Withdraw-10");
        let done = step(&json!({"state":rest["state"],"count":22,"target":22,"ok":true}));
        assert_eq!(done["value"], 20.0);
    }
    #[test]
    fn no_progress_and_full_inventory_stop() {
        let op = step(&json!({"count":0,"target":5}));
        assert_eq!(op["op"], "Withdraw-5");
        assert_eq!(
            step(&json!({"state":op["state"],"count":0,"target":5,"ok":false}))["kind"],
            "done"
        );
        assert_eq!(
            step(&json!({"count":2,"target":22,"full":true}))["value"],
            0.0
        );
    }
}
