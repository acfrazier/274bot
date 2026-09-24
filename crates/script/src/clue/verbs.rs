use super::*;

pub(super) fn i32_field(step: &Value, key: &str) -> Option<i32> {
    i32::try_from(step.get(key)?.as_i64()?).ok()
}

pub(super) fn text_field<'a>(step: &'a Value, key: &str) -> Option<&'a str> {
    step.get(key).and_then(Value::as_str)
}

/// Map one machine verb onto an interact op. An unknown kind is not a loc.
pub(crate) fn verb_req(step: &Value) -> Option<InteractReq> {
    match step.get("kind").and_then(Value::as_str)? {
        "walk" => Some(InteractReq::Walk {
            x: i32_field(step, "x")?,
            z: i32_field(step, "z")?,
            level: i32_field(step, "level")?,
            allow_teleports: false,
            allow_wilderness: false,
            allow_bank_fetch: false,
            request_id: 0,
        }),
        "held" => Some(InteractReq::Held {
            name: text_field(step, "name")?.to_string(),
            action: text_field(step, "action")?.to_string(),
        }),
        "loc" => Some(InteractReq::Loc {
            x: i32_field(step, "x")?,
            z: i32_field(step, "z")?,
            level: i32_field(step, "level")?,
            action: text_field(step, "action")?.to_string(),
            id: Some(i32_field(step, "id")?),
        }),
        "npc" => Some(InteractReq::Npc {
            name: text_field(step, "name")?.to_string(),
            action: text_field(step, "action")?.to_string(),
            index: Some(i32_field(step, "index")?),
        }),
        "if-button" => Some(InteractReq::IfButton {
            component_id: i32_field(step, "component_id")?,
        }),
        "close-modal" => Some(InteractReq::CloseModal),
        "obj" => Some(InteractReq::Obj {
            x: i32_field(step, "x")?,
            z: i32_field(step, "z")?,
            level: i32_field(step, "level")?,
            name: Some(text_field(step, "name")?.to_string()),
            action: text_field(step, "action")?.to_string(),
        }),
        "puzzle-move" => Some(InteractReq::PuzzleMove {
            id: i32_field(step, "id")?,
            slot: i32_field(step, "slot")?,
            component: i32_field(step, "component")?,
            generation: step.get("generation").and_then(Value::as_u64)?,
        }),
        "continue" => Some(InteractReq::ContinueDialog),
        "answer" => Some(InteractReq::Answer {
            option: i32_field(step, "option")?,
        }),
        "answer-count" => Some(InteractReq::AnswerCount {
            value: i32_field(step, "value")?,
        }),
        "shop-button" => Some(InteractReq::ShopButton {
            kind: text_field(step, "shop")?.to_string(),
            name: text_field(step, "name")?.to_string(),
            id: i32_field(step, "id")?,
            slot: i32_field(step, "slot")?,
            component: i32_field(step, "component")?,
            chunk: i32_field(step, "chunk")?,
        }),
        "wear" => Some(InteractReq::Wear {
            name: text_field(step, "name")?.to_string(),
        }),
        "unequip" => Some(InteractReq::Unequip {
            name: text_field(step, "name")?.to_string(),
        }),
        "deposit" => Some(InteractReq::Deposit {
            name: text_field(step, "name")?.to_string(),
        }),
        "withdraw" => Some(InteractReq::Withdraw {
            name: text_field(step, "name")?.to_string(),
            action: text_field(step, "action")?.to_string(),
        }),
        "walk-nearest-bank" => Some(InteractReq::WalkNearestBank),
        "open-booth" => Some(InteractReq::OpenBooth {
            x: i32_field(step, "x")?,
            z: i32_field(step, "z")?,
            level: i32_field(step, "level")?,
            id: i32_field(step, "id")?,
            name: text_field(step, "name").map(str::to_string),
            action: text_field(step, "action").map(str::to_string),
        }),
        "close" => Some(InteractReq::Close),
        _ => None,
    }
}

pub(crate) fn verb_json(step: &Value) -> Value {
    let Some(req) = verb_req(step) else {
        return Value::Null;
    };
    match req {
        InteractReq::Walk { x, z, level, .. } => {
            json!({ "op": "walk", "x": x, "z": z, "level": level })
        }
        InteractReq::Held { name, action } => {
            json!({ "op": "held", "name": name, "action": action })
        }
        InteractReq::Loc {
            x,
            z,
            level,
            action,
            id,
        } => {
            json!({ "op": "loc", "x": x, "z": z, "level": level, "action": action, "id": id })
        }
        InteractReq::Npc {
            name,
            action,
            index,
        } => {
            json!({ "op": "npc", "name": name, "action": action, "index": index })
        }
        InteractReq::IfButton { component_id } => {
            json!({ "op": "if-button", "component_id": component_id })
        }
        InteractReq::CloseModal => json!({ "op": "close-modal" }),
        InteractReq::Obj {
            x,
            z,
            level,
            name,
            action,
        } => {
            json!({ "op": "obj", "x": x, "z": z, "level": level, "name": name, "action": action })
        }
        InteractReq::PuzzleMove {
            id,
            slot,
            component,
            generation,
        } => json!({
            "op": "puzzle-move",
            "id": id,
            "slot": slot,
            "component": component,
            "generation": generation,
        }),
        InteractReq::ContinueDialog => json!({ "op": "continue" }),
        InteractReq::Answer { option } => json!({ "op": "answer", "option": option }),
        InteractReq::AnswerCount { value } => json!({ "op": "answer-count", "value": value }),
        InteractReq::ShopButton {
            kind,
            name,
            id,
            slot,
            component,
            chunk,
        } => json!({
            "op": "shop-button",
            "kind": kind,
            "name": name,
            "id": id,
            "slot": slot,
            "component": component,
            "chunk": chunk,
        }),
        InteractReq::Wear { name } => json!({ "op": "wear", "name": name }),
        InteractReq::Unequip { name } => json!({ "op": "unequip", "name": name }),
        InteractReq::Deposit { name } => json!({ "op": "deposit", "name": name }),
        InteractReq::Withdraw { name, action } => {
            json!({ "op": "withdraw", "name": name, "action": action })
        }
        InteractReq::WalkNearestBank => json!({ "op": "walk-nearest-bank" }),
        InteractReq::OpenBooth {
            x,
            z,
            level,
            id,
            name,
            action,
        } => {
            let mut req = json!({ "op": "open-booth", "x": x, "z": z, "level": level, "id": id });
            if let Some(name) = name {
                req["name"] = json!(name);
            }
            if let Some(action) = action {
                req["action"] = json!(action);
            }
            req
        }
        InteractReq::Close => json!({ "op": "close" }),
        _ => Value::Null,
    }
}
