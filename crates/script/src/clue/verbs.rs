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

